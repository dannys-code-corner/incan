//! Native test inventory and exact execution for Oven Alpha's direct-rustc consumers.
//!
//! Oven executes the libtest binary it built itself. It obtains a real inventory
//! first and rejects a requested exact test absent from that inventory, so a
//! zero-match filter can never become a misleading success. Neither collection
//! nor execution launches Cargo or inherits Cargo process state.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use super::rustc::clear_inherited_cargo_environment;

/// Inventory returned by one exact native libtest binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OvenNativeTestInventory {
    /// Complete deterministic set of test names reported by the binary.
    pub names: Vec<String>,
}

/// Exact native test execution request.
#[derive(Debug, Clone)]
pub struct OvenNativeTestRequest {
    /// Caller-owned direct-rustc libtest binary.
    pub executable: PathBuf,
    /// Names that must occur in the real binary inventory before execution begins.
    pub exact_names: Vec<String>,
    /// Compiler-owned environment replacements for the test process.
    ///
    /// These are applied after inherited Cargo variables are removed. They let a receipt-bound suite pin paths such
    /// as its source checkout without making ambient shell configuration part of test correctness.
    pub environment: BTreeMap<String, String>,
}

/// Successful native-test execution record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OvenNativeTestReport {
    /// Complete inventory consulted before exact selection.
    pub inventory: OvenNativeTestInventory,
    /// Exact test names successfully executed by the native binary.
    pub passed: Vec<String>,
}

/// One verified all-in-one native libtest execution used when fixture scope requires a shared process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OvenNativeTestBatchReport {
    /// Complete inventory consulted before execution.
    pub inventory: OvenNativeTestInventory,
    /// Whether libtest reported an all-green result.
    pub success: bool,
    /// Combined libtest transcript retained for per-test result mapping by the caller.
    pub output: String,
}

/// Error while obtaining native test inventory or executing an exact test.
#[derive(Debug, thiserror::Error)]
pub enum OvenNativeTestError {
    /// The caller supplied an invalid executable path or duplicate/empty exact test name.
    #[error("invalid Oven native-test {field}: {message}")]
    InvalidInput { field: &'static str, message: String },
    /// Starting or reading a native-test process failed.
    #[error("Oven native-test I/O failed at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    /// The native binary could not produce a valid libtest inventory.
    #[error("Oven native-test inventory failed: {output}")]
    InventoryFailed { output: String },
    /// The requested exact test did not occur in the binary's verified inventory.
    #[error("Oven native-test exact selection `{name}` is absent from the binary inventory")]
    MissingExactTest { name: String },
    /// An exact test ran but reported a failure.
    #[error("Oven native-test `{name}` failed: {output}")]
    TestFailed { name: String, output: String },
}

/// Obtain a complete deterministic native libtest inventory without a Cargo process.
pub fn inventory_native_tests(executable: &Path) -> Result<OvenNativeTestInventory, OvenNativeTestError> {
    inventory_native_tests_with_environment(executable, &BTreeMap::new(), None, false)
}

/// Inventory a native libtest binary after applying its explicit, Cargo-free process environment.
///
/// This is necessary for test roots such as proc-macro crates, whose direct-rustc binary links the receipt-selected
/// toolchain dynamic standard library. The environment is never inherited from Cargo.
fn inventory_native_tests_with_environment(
    executable: &Path,
    environment: &BTreeMap<String, String>,
    working_directory: Option<&Path>,
    allow_empty: bool,
) -> Result<OvenNativeTestInventory, OvenNativeTestError> {
    let executable = verified_executable(executable)?;
    let mut command = Command::new(&executable);
    command.args(["--list", "--format", "terse"]);
    if let Some(working_directory) = working_directory {
        command.current_dir(working_directory);
    }
    clear_inherited_cargo_environment(&mut command);
    command.envs(environment);
    let output = command.output().map_err(|source| OvenNativeTestError::Io {
        path: executable.clone(),
        source,
    })?;
    let transcript = combined_output(&output.stdout, &output.stderr);
    if !output.status.success() {
        return Err(OvenNativeTestError::InventoryFailed { output: transcript });
    }
    let names = parse_inventory(&output.stdout, allow_empty)?;
    Ok(OvenNativeTestInventory { names })
}

/// Run exact native tests only after every requested name occurs in the verified binary inventory.
pub fn run_native_tests(request: &OvenNativeTestRequest) -> Result<OvenNativeTestReport, OvenNativeTestError> {
    let inventory = inventory_native_tests_with_environment(&request.executable, &request.environment, None, false)?;
    let requested = normalized_exact_names(&request.exact_names)?;
    let available = inventory.names.iter().collect::<BTreeSet<_>>();
    for name in &requested {
        if !available.contains(name) {
            return Err(OvenNativeTestError::MissingExactTest { name: name.clone() });
        }
    }

    for name in &requested {
        let mut command = Command::new(&request.executable);
        command.args(["--exact", name, "--nocapture"]);
        clear_inherited_cargo_environment(&mut command);
        command.envs(&request.environment);
        let output = command.output().map_err(|source| OvenNativeTestError::Io {
            path: request.executable.clone(),
            source,
        })?;
        if !output.status.success() {
            return Err(OvenNativeTestError::TestFailed {
                name: name.clone(),
                output: combined_output(&output.stdout, &output.stderr),
            });
        }
    }
    Ok(OvenNativeTestReport {
        inventory,
        passed: requested,
    })
}

/// Run one generated batch in a single native libtest process after verifying its exact expected inventory.
///
/// This preserves session-scoped fixture behaviour. Generated Incan file batches can share registration and fixture
/// initialization between their native Rust `#[test]` functions, so the batch itself runs one test at a time while
/// the outer scheduler remains free to run independent files in parallel. The caller may parse the returned libtest
/// transcript into its own richer test-reporting format; a test assertion failure is represented as `success: false`,
/// not as a transport error that would hide results for later tests in the same batch.
pub fn run_native_test_batch(
    request: &OvenNativeTestRequest,
) -> Result<OvenNativeTestBatchReport, OvenNativeTestError> {
    let inventory = inventory_native_tests_with_environment(&request.executable, &request.environment, None, false)?;
    let requested = normalized_exact_names(&request.exact_names)?;
    let available = inventory.names.iter().collect::<BTreeSet<_>>();
    for name in &requested {
        if !available.contains(name) {
            return Err(OvenNativeTestError::MissingExactTest { name: name.clone() });
        }
    }
    let executable = verified_executable(&request.executable)?;
    let mut command = Command::new(&executable);
    command.args(["--test-threads=1", "--nocapture"]);
    clear_inherited_cargo_environment(&mut command);
    command.envs(&request.environment);
    let output = command.output().map_err(|source| OvenNativeTestError::Io {
        path: executable,
        source,
    })?;
    Ok(OvenNativeTestBatchReport {
        inventory,
        success: output.status.success(),
        output: combined_output(&output.stdout, &output.stderr),
    })
}

/// Inventory and execute every test in one native libtest binary, accepting a valid zero-test target.
///
/// Cargo accepts a compiled test root with no `#[test]` functions; Oven must do the same for workspace proc-macro
/// roots. The binary is still inventoried and launched with Cargo state removed, so an empty inventory is not treated
/// as an unverified success.
pub fn run_native_test_batch_all(
    executable: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<OvenNativeTestBatchReport, OvenNativeTestError> {
    run_native_test_batch_all_in_directory(executable, environment, None)
}

/// Inventory and execute every test from one verified caller-selected package directory.
///
/// Cargo launches each test target from its package manifest directory. Stored direct-rustc test binaries must retain
/// that authored working-directory contract: snapshot and fixture tests commonly use paths relative to the package
/// root, while Oven's executable output remains caller-owned and must not become an implicit source root.
pub fn run_native_test_batch_all_in_directory(
    executable: &Path,
    environment: &BTreeMap<String, String>,
    working_directory: Option<&Path>,
) -> Result<OvenNativeTestBatchReport, OvenNativeTestError> {
    let inventory = inventory_native_tests_with_environment(executable, environment, working_directory, true)?;
    let executable = verified_executable(executable)?;
    let mut command = Command::new(&executable);
    command.arg("--nocapture");
    if let Some(working_directory) = working_directory {
        command.current_dir(working_directory);
    }
    clear_inherited_cargo_environment(&mut command);
    command.envs(environment);
    let output = command.output().map_err(|source| OvenNativeTestError::Io {
        path: executable,
        source,
    })?;
    Ok(OvenNativeTestBatchReport {
        inventory,
        success: output.status.success(),
        output: combined_output(&output.stdout, &output.stderr),
    })
}

/// Reject symlink and non-file execution paths before creating a child process.
fn verified_executable(executable: &Path) -> Result<PathBuf, OvenNativeTestError> {
    if executable.as_os_str().is_empty() {
        return Err(OvenNativeTestError::InvalidInput {
            field: "executable",
            message: "must not be empty".to_string(),
        });
    }
    let metadata = fs::symlink_metadata(executable).map_err(|source| OvenNativeTestError::Io {
        path: executable.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(OvenNativeTestError::InvalidInput {
            field: "executable",
            message: "must be a non-symlink regular file".to_string(),
        });
    }
    Ok(executable.to_path_buf())
}

/// Normalize the exact test selection and make duplicate execution an input error.
fn normalized_exact_names(names: &[String]) -> Result<Vec<String>, OvenNativeTestError> {
    if names.is_empty() {
        return Err(OvenNativeTestError::InvalidInput {
            field: "exact test selection",
            message: "must name at least one collected test".to_string(),
        });
    }
    let mut unique = BTreeSet::new();
    for name in names {
        let normalized = name.trim();
        if normalized.is_empty() {
            return Err(OvenNativeTestError::InvalidInput {
                field: "exact test selection",
                message: "must not contain an empty name".to_string(),
            });
        }
        if !unique.insert(normalized.to_string()) {
            return Err(OvenNativeTestError::InvalidInput {
                field: "exact test selection",
                message: format!("contains duplicate `{normalized}`"),
            });
        }
    }
    Ok(unique.into_iter().collect())
}

/// Parse the stable `<name>: test` libtest terse inventory lines and reject unexplained non-empty output.
fn parse_inventory(stdout: &[u8], allow_empty: bool) -> Result<Vec<String>, OvenNativeTestError> {
    let text = String::from_utf8_lossy(stdout);
    let mut names = BTreeSet::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let Some(name) = line.strip_suffix(": test") else {
            return Err(OvenNativeTestError::InventoryFailed {
                output: format!("unexpected libtest inventory line `{line}`"),
            });
        };
        if name.is_empty() || !names.insert(name.to_string()) {
            return Err(OvenNativeTestError::InventoryFailed {
                output: format!("invalid or duplicate libtest test name `{name}`"),
            });
        }
    }
    if names.is_empty() && !allow_empty {
        return Err(OvenNativeTestError::InventoryFailed {
            output: "libtest inventory contained no test cases".to_string(),
        });
    }
    Ok(names.into_iter().collect())
}

/// Preserve both child streams in deterministic diagnostic order.
fn combined_output(stdout: &[u8], stderr: &[u8]) -> String {
    format!("{}{}", String::from_utf8_lossy(stdout), String::from_utf8_lossy(stderr))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    use super::{
        OvenNativeTestError, OvenNativeTestRequest, run_native_test_batch, run_native_test_batch_all, run_native_tests,
    };
    use crate::oven::rustc::{
        OVEN_RUSTC_ARTIFACT_MANIFEST_SCHEMA_VERSION, OvenRustcArtifactManifest, OvenStoredDirectRustcTestRequest,
        bake_stored_direct_rustc_test,
    };
    use crate::oven::store::{OvenArtifactKind, OvenArtifactPublishRequest, OvenStore, OvenStoreLimits};
    use crate::oven::{OvenImportRequest, digest_bytes, import_frozen_project};

    #[test]
    fn native_runner_rejects_missing_exact_test_and_runs_verified_test_without_cargo()
    -> Result<(), Box<dyn std::error::Error>> {
        let project = tempfile::tempdir()?;
        let output = tempfile::tempdir()?;
        let store_root = tempfile::tempdir()?;
        write_project(project.path())?;
        let source = output.path().join("native-tests.rs");
        fs::write(
            &source,
            "#[test]\nfn selected() { assert!(std::env::var_os(\"CARGO\").is_none()); assert!(std::env::var_os(\"CARGO_PKG_NAME\").is_none()); }\n#[test]\nfn other() {}\n",
        )?;
        let rustc = rustc_path()?;
        let receipt = import_frozen_project(
            &OvenImportRequest::new(
                project.path(),
                "aarch64-apple-darwin",
                rustc_identity(&rustc)?,
                "release",
                Vec::new(),
            )
            .with_supplemental_source_digest("direct-rustc-source", digest_bytes(&fs::read(&source)?)),
        )?;
        let plan = OvenRustcArtifactManifest {
            schema_version: OVEN_RUSTC_ARTIFACT_MANIFEST_SCHEMA_VERSION,
            intent: receipt.intent.clone(),
            dependency_search_paths: Vec::new(),
            native_search_paths: Vec::new(),
            externs: Vec::new(),
            entrypoint_externs: BTreeMap::new(),
            compile_environment: BTreeMap::new(),
            supporting_artifacts: Vec::new(),
        };
        let store = OvenStore::new(
            store_root.path(),
            OvenStoreLimits::new(128 * 1024, 128 * 1024, 64 * 1024),
        );
        let stored = store.publish(&OvenArtifactPublishRequest {
            receipt: receipt.clone(),
            domain: "native-tests".to_string(),
            kind: OvenArtifactKind::DirectRustcPlan,
            payload: serde_json::to_vec(&plan)?,
            materialized_files: Vec::new(),
        })?;
        let bake = bake_stored_direct_rustc_test(&OvenStoredDirectRustcTestRequest {
            store: &store,
            plan_identity: stored.identity,
            receipt,
            rustc,
            source,
            output: output.path().join("native-tests"),
            crate_name: "oven_native_tests".to_string(),
            edition: "2024".to_string(),
            source_evidence_key: "direct-rustc-source".to_string(),
        })?;

        let missing = run_native_tests(&OvenNativeTestRequest {
            executable: bake.output.clone(),
            exact_names: vec!["absent".to_string()],
            environment: BTreeMap::new(),
        });
        assert!(matches!(missing, Err(OvenNativeTestError::MissingExactTest { .. })));
        let report = run_native_tests(&OvenNativeTestRequest {
            executable: bake.output,
            exact_names: vec!["selected".to_string()],
            environment: BTreeMap::new(),
        })?;
        assert_eq!(report.inventory.names, ["other", "selected"]);
        assert_eq!(report.passed, ["selected"]);
        Ok(())
    }

    #[test]
    fn all_batch_executes_a_valid_zero_test_target() -> Result<(), Box<dyn std::error::Error>> {
        let output = tempfile::tempdir()?;
        let source = output.path().join("zero-tests.rs");
        let executable = output.path().join("zero-tests");
        fs::write(&source, "fn helper() {}\n")?;
        let rustc = rustc_path()?;
        let status = Command::new(rustc)
            .arg("--test")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()?;
        assert!(status.success());

        let report = run_native_test_batch_all(&executable, &BTreeMap::new())?;
        assert!(report.success);
        assert!(report.inventory.names.is_empty());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn generated_batch_forces_single_inner_libtest_thread() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt;

        let output = tempfile::tempdir()?;
        let executable = output.path().join("native-test-argument-check");
        fs::write(
            &executable,
            "#!/bin/sh\n\
             if [ \"$1\" = \"--list\" ] && [ \"$2\" = \"--format\" ] && [ \"$3\" = \"terse\" ]; then\n\
               printf '%s\\n' 'generated::case: test'\n\
               exit 0\n\
             fi\n\
             if [ \"$1\" = \"--test-threads=1\" ] && [ \"$2\" = \"--nocapture\" ] && [ \"$#\" -eq 2 ]; then\n\
               exit 0\n\
             fi\n\
             printf 'unexpected native test arguments: %s\\n' \"$*\" >&2\n\
             exit 62\n",
        )?;
        let mut permissions = fs::metadata(&executable)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions)?;

        let report = run_native_test_batch(&OvenNativeTestRequest {
            executable,
            exact_names: vec!["generated::case".to_string()],
            environment: BTreeMap::new(),
        })?;
        assert!(report.success, "{report:#?}");
        assert_eq!(report.inventory.names, ["generated::case"]);
        Ok(())
    }

    fn write_project(path: &std::path::Path) -> Result<(), std::io::Error> {
        fs::write(
            path.join("Cargo.toml"),
            "[package]\nname = \"oven-native-tests\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        fs::write(
            path.join("Cargo.lock"),
            "# This file is automatically @generated by Cargo.\nversion = 4\n",
        )
    }

    fn rustc_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let output = Command::new("rustup").args(["which", "rustc"]).output()?;
        if !output.status.success() {
            return Err("rustup could not locate rustc".into());
        }
        let path = PathBuf::from(String::from_utf8(output.stdout)?.trim());
        if !path.is_file() {
            return Err(format!("rustup returned a non-file rustc path: {}", path.display()).into());
        }
        Ok(path)
    }

    fn rustc_identity(rustc: &std::path::Path) -> Result<String, Box<dyn std::error::Error>> {
        let output = Command::new(rustc).arg("--version").output()?;
        if !output.status.success() {
            return Err(format!("rustc could not report its version: {}", rustc.display()).into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }
}
