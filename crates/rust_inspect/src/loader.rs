//! Load a Cargo tree into rust-analyzer's `RootDatabase`.
//!
//! This module is intentionally behind the rust-inspect preparation/cache boundary. It owns the unstable rust-analyzer
//! embedding details so parser/typechecker/codegen code does not load Cargo workspaces directly.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ra_ap_hir::Crate;
use ra_ap_ide_db::RootDatabase;
use ra_ap_load_cargo::{LoadCargoConfig, ProcMacroServerChoice, load_workspace_at};
use ra_ap_project_model::CargoConfig;
use ra_ap_vfs::Vfs;

use super::error::RustMetadataError;

/// A loaded Cargo workspace suitable for `hir` queries.
///
/// The `Vfs` handle is retained so file-backed state remains consistent with the database for the lifetime of this
/// value.
pub struct RustWorkspace {
    pub(crate) db: RootDatabase,
    crate_index: HashMap<String, Crate>,
    #[allow(dead_code)]
    vfs: Vfs,
}

/// A sequence scoped to this process keeps generated direct-project descriptions independent when libtests run in
/// parallel. The descriptions live under the caller-managed inspection output, never beside an inspected source
/// tree or in Cargo's cache.
static OVEN_PROJECT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// One local source crate in the direct rust-analyzer graph used by a sealed Oven consumer.
struct OvenProjectCrate {
    display_name: String,
    root_module: PathBuf,
    edition: String,
    dependencies: Vec<(String, PathBuf)>,
}

impl RustWorkspace {
    fn normalize_crate_name(name: &str) -> String {
        name.replace('-', "_")
    }

    fn build_crate_index(db: &RootDatabase) -> HashMap<String, Crate> {
        let mut index = HashMap::new();
        for krate in Crate::all(db) {
            if let Some(display_name) = krate.display_name(db) {
                index
                    .entry(Self::normalize_crate_name(display_name.to_string().as_str()))
                    .or_insert(krate);
                index
                    .entry(Self::normalize_crate_name(display_name.crate_name().as_str()))
                    .or_insert(krate);
                index
                    .entry(Self::normalize_crate_name(display_name.canonical_name().as_str()))
                    .or_insert(krate);
            }
        }
        index
    }

    /// Build Cargo configuration for one Rust metadata workspace.
    ///
    /// rust-analyzer may run `cargo check` to discover build-script output. Keep those nested Cargo artifacts inside
    /// the generated workspace target selected by Incan instead of inheriting a caller-level target or unstable
    /// Cargo `build-dir` override.
    fn metadata_cargo_config(target_dir: &Path) -> CargoConfig {
        let target_dir = target_dir.to_string_lossy().into_owned();
        let mut config = CargoConfig::default();
        config
            .extra_env
            .insert("CARGO_TARGET_DIR".to_string(), Some(target_dir.clone()));
        config
            .extra_env
            .insert("CARGO_BUILD_BUILD_DIR".to_string(), Some(target_dir));
        config
    }

    /// Whether this process is a receipt-bound Oven compiler-suite consumer.
    ///
    /// The normal compiler has no reason to ask rust-analyzer to rediscover a Cargo graph: Oven either supplies a
    /// sealed provider ABI or rejects the unsupported dynamic request. The legacy publisher retains the historical
    /// Cargo loader. The compiler-suite consumer exercises the same non-Cargo boundary against small source fixtures.
    fn oven_compiler_suite_active() -> bool {
        std::env::var_os("INCAN_OVEN_COMPILER_SUITE_RUSTC").is_some()
    }

    /// Materialize a minimal rust-analyzer project description for one compiler-authored manifest without invoking
    /// Cargo. `rust-project.json` is rust-analyzer's documented build-system interface; absolute source paths keep
    /// the descriptor independent from the caller's working directory.
    fn oven_project_json_payload(manifest_dir: &Path) -> Result<Vec<u8>, RustMetadataError> {
        fn local_path_dependencies(manifest: &toml::Value, manifest_dir: &Path) -> Vec<(String, PathBuf)> {
            fn collect(table: Option<&toml::Value>, manifest_dir: &Path, dependencies: &mut Vec<(String, PathBuf)>) {
                let Some(table) = table.and_then(toml::Value::as_table) else {
                    return;
                };
                for (dependency_name, declaration) in table {
                    let Some(path) = declaration.get("path").and_then(toml::Value::as_str) else {
                        continue;
                    };
                    let candidate = manifest_dir.join(path);
                    if candidate.join("Cargo.toml").is_file() {
                        dependencies.push((dependency_name.replace('-', "_"), candidate));
                    }
                }
            }

            let mut dependencies = Vec::new();
            for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                collect(manifest.get(section), manifest_dir, &mut dependencies);
            }
            if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
                for target in targets.values() {
                    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                        collect(target.get(section), manifest_dir, &mut dependencies);
                    }
                }
            }
            dependencies.sort();
            dependencies.dedup();
            dependencies
        }

        fn crate_from_manifest(manifest_dir: &Path) -> Result<OvenProjectCrate, RustMetadataError> {
            let manifest_path = manifest_dir.join("Cargo.toml");
            let manifest = toml::from_str::<toml::Value>(&fs::read_to_string(&manifest_path)?).map_err(|error| {
                RustMetadataError::LoadWorkspace {
                    path: manifest_path.clone(),
                    message: format!("failed to parse compiler-authored manifest for direct Oven inspection: {error}"),
                }
            })?;
            let package = manifest.get("package").and_then(toml::Value::as_table).ok_or_else(|| {
                RustMetadataError::LoadWorkspace {
                    path: manifest_path.clone(),
                    message: "direct Oven inspection requires a package manifest".to_string(),
                }
            })?;
            let package_name =
                package
                    .get("name")
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| RustMetadataError::LoadWorkspace {
                        path: manifest_path.clone(),
                        message: "direct Oven inspection requires package.name".to_string(),
                    })?;
            let display_name = manifest
                .get("lib")
                .and_then(|library| library.get("name"))
                .and_then(toml::Value::as_str)
                .unwrap_or(package_name)
                .replace('-', "_");
            let edition = package
                .get("edition")
                .and_then(toml::Value::as_str)
                .filter(|edition| matches!(*edition, "2015" | "2018" | "2021" | "2024"))
                .unwrap_or("2021")
                .to_string();
            let root = manifest
                .get("lib")
                .and_then(|library| library.get("path"))
                .and_then(toml::Value::as_str)
                .map(|path| manifest_dir.join(path))
                .unwrap_or_else(|| manifest_dir.join("src/lib.rs"));
            let root_module = root.canonicalize().map_err(|error| RustMetadataError::LoadWorkspace {
                path: root,
                message: format!("direct Oven inspection requires a readable library root: {error}"),
            })?;
            Ok(OvenProjectCrate {
                display_name,
                root_module,
                edition,
                dependencies: local_path_dependencies(&manifest, manifest_dir),
            })
        }

        fn visit(
            manifest_dir: &Path,
            crates: &mut Vec<OvenProjectCrate>,
            indices: &mut HashMap<PathBuf, usize>,
        ) -> Result<usize, RustMetadataError> {
            let manifest_dir = manifest_dir.canonicalize()?;
            if let Some(index) = indices.get(&manifest_dir) {
                return Ok(*index);
            }
            let index = crates.len();
            indices.insert(manifest_dir.clone(), index);
            let direct_crate = crate_from_manifest(&manifest_dir)?;
            let dependencies = direct_crate.dependencies.clone();
            crates.push(direct_crate);
            for (_, dependency_dir) in dependencies {
                visit(&dependency_dir, crates, indices)?;
            }
            Ok(index)
        }

        let mut crates = Vec::new();
        let mut indices = HashMap::new();
        visit(manifest_dir, &mut crates, &mut indices)?;
        let crates = crates
            .into_iter()
            .map(|direct_crate| {
                let dependencies = direct_crate
                    .dependencies
                    .into_iter()
                    .filter_map(|(name, manifest_dir)| {
                        indices
                            .get(&manifest_dir.canonicalize().ok()?)
                            .map(|index| serde_json::json!({ "crate": index, "name": name }))
                    })
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "display_name": direct_crate.display_name,
                    "root_module": direct_crate.root_module,
                    "edition": direct_crate.edition,
                    "deps": dependencies,
                    "cfg": [],
                    "env": {},
                    "is_workspace_member": true,
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_vec(&serde_json::json!({
            "crates": crates,
        }))
        .map_err(|error| RustMetadataError::LoadWorkspace {
            path: manifest_dir.to_path_buf(),
            message: format!("failed to encode direct Oven rust-project graph: {error}"),
        })
    }

    /// Load the sealed compiler-suite source graph with rust-analyzer's build-system-neutral interface.
    fn load_oven_project(
        manifest_dir: &Path,
        target_dir: &Path,
        progress: &(dyn Fn(String) + Sync),
        _load_out_dirs_from_check: bool,
    ) -> Result<Self, RustMetadataError> {
        let manifest_dir = manifest_dir.canonicalize()?;
        let payload = Self::oven_project_json_payload(&manifest_dir)?;
        let sequence = OVEN_PROJECT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let project_dir = target_dir
            .join("incan-oven-rust-projects")
            .join(format!("{}-{sequence}", std::process::id()));
        fs::create_dir_all(&project_dir)?;
        // rust-analyzer recognizes only this exact filename when discovering a build-system-neutral graph. A suffix
        // such as `*.rust-project.json` makes it climb to an ancestor Cargo.toml and silently reintroduce Cargo.
        let project_path = project_dir.join("rust-project.json");
        fs::write(&project_path, payload)?;
        let load_config = LoadCargoConfig {
            load_out_dirs_from_check: false,
            with_proc_macro_server: ProcMacroServerChoice::None,
            prefill_caches: false,
            num_worker_threads: 1,
            proc_macro_processes: 1,
        };
        let result =
            load_workspace_at(&project_path, &CargoConfig::default(), &load_config, progress).map_err(|error| {
                RustMetadataError::LoadWorkspace {
                    path: manifest_dir.clone(),
                    message: error.to_string(),
                }
            });
        let _ = fs::remove_file(&project_path);
        let _ = fs::remove_dir(&project_dir);
        let (db, vfs, _pm) = result?;
        let crate_index = Self::build_crate_index(&db);
        Ok(RustWorkspace { db, crate_index, vfs })
    }

    /// Load the Cargo project rooted at `manifest_dir` (directory containing `Cargo.toml`).
    ///
    /// `progress` is forwarded to rust-analyzer while discovering workspace members. Call this only from explicit
    /// inspection preparation paths, not from ordinary semantic lookups.
    pub fn load(manifest_dir: &Path, progress: &(dyn Fn(String) + Sync)) -> Result<Self, RustMetadataError> {
        Self::load_with_options(manifest_dir, progress, false)
    }

    /// Load the Cargo project rooted at `manifest_dir` with optional build-script OUT_DIR support.
    pub fn load_with_options(
        manifest_dir: &Path,
        progress: &(dyn Fn(String) + Sync),
        load_out_dirs_from_check: bool,
    ) -> Result<Self, RustMetadataError> {
        let target_dir = crate::cache::cargo_configured_target_dir(manifest_dir);
        Self::load_with_options_and_target(manifest_dir, &target_dir, progress, load_out_dirs_from_check)
    }

    /// Load a Cargo project while keeping any nested build-script discovery in the owner workspace's target.
    pub(crate) fn load_with_options_and_target(
        manifest_dir: &Path,
        target_dir: &Path,
        progress: &(dyn Fn(String) + Sync),
        load_out_dirs_from_check: bool,
    ) -> Result<Self, RustMetadataError> {
        if Self::oven_compiler_suite_active() {
            return Self::load_oven_project(manifest_dir, target_dir, progress, load_out_dirs_from_check);
        }
        let manifest_dir = manifest_dir.canonicalize()?;
        let cargo_config = Self::metadata_cargo_config(target_dir);
        let load_config = LoadCargoConfig {
            load_out_dirs_from_check,
            // Proc macros are optional for many crates; `None` keeps CI fast.
            with_proc_macro_server: ProcMacroServerChoice::None,
            prefill_caches: false,
            num_worker_threads: 1,
            proc_macro_processes: 1,
        };
        let (db, vfs, _pm) = load_workspace_at(&manifest_dir, &cargo_config, &load_config, progress).map_err(|e| {
            RustMetadataError::LoadWorkspace {
                path: manifest_dir.clone(),
                message: e.to_string(),
            }
        })?;
        let crate_index = Self::build_crate_index(&db);
        Ok(RustWorkspace { db, crate_index, vfs })
    }

    /// Shared read-only access to the underlying database.
    pub fn db(&self) -> &RootDatabase {
        &self.db
    }

    pub fn crate_by_name(&self, crate_name: &str) -> Option<Crate> {
        self.crate_index
            .get(Self::normalize_crate_name(crate_name).as_str())
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::RustWorkspace;

    use tempfile::tempdir;

    #[test]
    fn metadata_loader_allows_cargo_to_resolve_uncached_dependencies() -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempdir()?;
        let cargo_config = RustWorkspace::metadata_cargo_config(&workspace.path().join("target"));
        assert!(
            !cargo_config.extra_args.iter().any(|arg| arg == "--offline"),
            "rust-inspect workspace loads must not force offline metadata resolution"
        );
        assert_eq!(
            cargo_config.extra_env.get("CARGO_NET_OFFLINE"),
            None,
            "rust-inspect workspace loads must not force Cargo into offline mode"
        );
        Ok(())
    }

    #[test]
    fn metadata_loader_contains_nested_cargo_output_in_configured_target() -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempdir()?;
        let configured_target = workspace.path().join("managed-target");
        fs::create_dir_all(workspace.path().join(".cargo"))?;
        fs::write(
            workspace.path().join(".cargo/config.toml"),
            format!("[build]\ntarget-dir = {:?}\n", configured_target),
        )?;

        let resolved_target = crate::cache::cargo_configured_target_dir(workspace.path());
        assert_eq!(resolved_target, configured_target);
        let cargo_config = RustWorkspace::metadata_cargo_config(&resolved_target);
        let expected = Some(configured_target.to_string_lossy().into_owned());
        assert_eq!(cargo_config.extra_env.get("CARGO_TARGET_DIR"), Some(&expected));
        assert_eq!(cargo_config.extra_env.get("CARGO_BUILD_BUILD_DIR"), Some(&expected));
        Ok(())
    }
}
