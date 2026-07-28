Warning: truncated output (original token count: 67990)
Total output lines: 6761

//! Shared utilities used across multiple CLI command pipelines.
//!
//! This module contains functions for source file reading, module collection, project root resolution,
//! dependency helpers, and Cargo flag construction.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};

#[cfg(feature = "rust_inspect")]
use crate::backend::ProjectGenerator;
use crate::backend::ir::detect_serde_non_import_usage;
use crate::backend::project::generator::GENERATED_CARGO_TARGET_DIR_ENV;
use crate::backend::project::{GENERATED_TOOLCHAIN_SUPPORT_CRATES, INCAN_STDLIB_CRATE_NAME};
use crate::cli::prelude::ParsedModule;
use crate::cli::{CliError, CliResult};
use crate::dependency_resolver::ResolvedDependencies;
use crate::dependency_resolver::{DependencyError, InlineRustImport};
use crate::frontend::ast::{ImportKind, ImportPath, Program, Span};
use crate::frontend::contract_metadata::{
    CanonicalModelBundle, materialize_contract_models, read_project_model_bundles,
};
use crate::frontend::hir::build_semantic_module_snapshot_v0;
use crate::frontend::library_manifest_index::{
    LibraryArtifactMetadata, LibraryManifestFailureKind, LibraryManifestIndex, LibraryManifestIndexEntry,
};
use crate::frontend::module::{
    SourceModuleImportResolution, canonicalize_source_module_segments, logical_module_segments_from_file,
    logical_source_import_candidates, resolve_program_source_imports,
};
use crate::frontend::testing_markers::{
    TestingMarkerSemantics, load_testing_marker_semantics, testing_marker_semantics_from_manifest,
};
use crate::frontend::typechecker::TypeCheckInfo;
use crate::frontend::typechecker::stdlib_loader::StdlibAstCache;
use crate::frontend::{ast_walk, diagnostics, lexer, parser, typechecker, vocab_desugar_pass};
use crate::library_manifest::{
    LibraryManifest, ProviderCargoDependency, ProviderCargoDependencySource, ProviderModuleClaim,
    digest_provider_artifact,
};
use crate::lockfile::CargoFeatureSelection;
use crate::manifest::{DependencySource, DependencySpec};
use crate::manifest::{
    INTERNAL_MANIFEST_OVERRIDE_ENV, INTERNAL_PROJECT_ROOT_OVERRIDE_ENV, MANIFEST_FILENAME, ProjectManifest,
};
use crate::project_lifecycle::toolchain::ToolchainConstraintSet;
use crate::provider::{
    BackendImplementationRequirement, FeatureSelection, PackageFeatureGraph, PackageFeaturePlan,
    ProviderModuleResolution, ProviderPlan, ProviderProvenance, ResolvedSdkComponents, SDK_INVENTORY_FILE,
    SDK_PROVIDER_BUILD_ENV, SDK_SOURCE_CATALOG_FILE, SdkArtifactProjection, SdkComponent, SdkComponentSelection,
    SdkDependencyRebinding, SdkInventory, SdkProviderDescriptor, SdkResolutionError, SdkSourceCatalog,
};
#[cfg(feature = "rust_inspect")]
use crate::rust_inspect::{Inspector, InspectorConfig};
use crate::workspace::WorkspaceGraph;
use incan_core::lang::{
    stdlib::{self, StdlibExtraCrateDep, StdlibExtraCrateSource},
    surface::result_methods,
};
use sha2::{Digest, Sha256};

use super::vocab_extraction::collect_library_vocab_metadata_for_parser;

/// Maximum source file size (100 MB)
///
/// Files larger than this are rejected to prevent out-of-memory conditions during compilation.
const MAX_SOURCE_SIZE: u64 = 100 * 1024 * 1024;
static PREPARED_LIBRARY_DEPENDENCIES: LazyLock<Mutex<HashMap<PathBuf, BTreeSet<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static SDK_PROVIDER_COMPILER_DIGESTS: LazyLock<Mutex<HashMap<PathBuf, [u8; 32]>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
pub(crate) const INTERNAL_LIBRARY_ARTIFACT_ONLY_ENV: &str = "INCAN_INTERNAL_LIBRARY_ARTIFACT_ONLY";
/// Internal provider-store override used by isolated compiler and packaging tests.
const INTERNAL_SDK_PROVIDER_STORE_ENV: &str = "INCAN_INTERNAL_SDK_PROVIDER_STORE";
/// Internal file through which release packaging receives the exact immutable SDK provider root.
const INTERNAL_SDK_PROVIDER_PATH_FILE_ENV: &str = "INCAN_INTERNAL_SDK_PROVIDER_PATH_FILE";
/// Internal SDK distribution profile used by release packaging to omit component payloads physically.
const INTERNAL_SDK_DISTRIBUTION_PROFILE_ENV: &str = "INCAN_INTERNAL_SDK_DISTRIBUTION_PROFILE";
/// Internal path override for the Cargo.lock payload used while producing a compiler-owned artifact.
pub(crate) const INTERNAL_CARGO_LOCK_PAYLOAD_PATH_ENV: &str = "INCAN_INTERNAL_CARGO_LOCK_PAYLOAD_PATH";
/// Explicit active SDK inventory override used by toolchain selection and SDK publication.
pub(crate) const SDK_INVENTORY_OVERRIDE_ENV: &str = "INCAN_SDK_INVENTORY";

/// One compiler diagnostic with enough source context for either human or machine-readable rendering.
#[derive(Debug, Clone)]
pub(crate) struct CliDiagnostic {
    pub file_path: String,
    pub source: String,
    pub error: diagnostics::CompileError,
    pub phase: diagnostics::DiagnosticPhase,
}

/// Structured failure produced by shared CLI collection/typechecking helpers.
#[derive(Debug, Clone)]
pub(crate) struct CliDiagnosticFailure {
    pub diagnostics: Vec<CliDiagnostic>,
}

impl CliDiagnosticFailure {
    /// Build one structured diagnostic failure while preserving the source text needed for JSON span projection.
    pub(crate) fn single(
        file_path: impl Into<String>,
        source: impl Into<String>,
        error: diagnostics::CompileError,
        phase: diagnostics::DiagnosticPhase,
    ) -> Self {
        Self {
            diagnostics: vec![CliDiagnostic {
                file_path: file_path.into(),
                source: source.into(),
                error,
                phase,
            }],
        }
    }

    /// Build one structured failure from parser or typechecker errors that all belong to the same source file.
    pub(crate) fn from_errors(
        file_path: impl Into<String>,
        source: impl Into<String>,
        errors: Vec<diagnostics::CompileError>,
        phase: diagnostics::DiagnosticPhase,
    ) -> Self {
        let file_path = file_path.into();
        let source = source.into();
        Self {
            diagnostics: errors
                .into_iter()
                .map(|error| CliDiagnostic {
                    file_path: file_path.clone(),
                    source: source.clone(),
                    error,
                    phase,
                })
                .collect(),
        }
    }

    /// Render the structured diagnostics through the existing source-highlighted human diagnostic formatter.
    pub(crate) fn render_human(&self) -> String {
        let mut rendered = String::new();
        for diagnostic in &self.diagnostics {
            rendered.push_str(&diagnostics::format_error(
                &diagnostic.file_path,
                &diagnostic.source,
                &diagnostic.error,
            ));
            rendered.push('\n');
        }
        rendered.trim_end().to_string()
    }
}

impl From<CliError> for CliDiagnosticFailure {
    fn from(error: CliError) -> Self {
        Self::single(
            "<command>",
            "",
            diagnostics::CompileError::new(error.message, Span::default()),
            diagnostics::DiagnosticPhase::Tooling,
        )
    }
}

#[derive(Debug, Clone)]
struct SourceReadFailure {
    message: String,
}

/// Unified project requirements collected from parsed modules and loaded provider manifests.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProjectRequirements {
    /// Required stdlib feature flags, such as `json`, `async`, and `web`.
    pub stdlib_features: Vec<String>,
    /// Required Cargo dependencies contributed by stdlib namespaces and provider manifests.
    pub dependencies: Vec<DependencySpec>,
    /// Immutable compiled-library projections that replace obsolete physical SDK cache coordinates.
    pub sdk_dependency_rebindings: Vec<SdkDependencyRebinding>,
    /// Path dependencies proven to be owned by the active SDK/toolchain rather than an ordinary project source.
    pub sdk_path_dependencies: Vec<DependencySpec>,
    /// Complete compiled-artifact closure whose transitive coordinates must be projected together.
    pub sdk_artifact_projections: Vec<SdkArtifactProjection>,
}

/// Select the Incan CLI executable that prepares SDK provider artifacts.
///
/// Cargo integration tests and development utilities do not run inside the `incan` CLI. Tests receive the real binary
/// through `CARGO_BIN_EXE_incan`; utility binaries use the sibling CLI built in the same target directory. Returning an
/// error is important: executing a generator with CLI arguments can exit successfully without publishing an artifact.
fn sdk_provider_builder_executable(
    cargo_test_binary: Option<PathBuf>,
    current_executable: PathBuf,
) -> CliResult<PathBuf> {
    if let Some(executable) = cargo_test_binary.filter(|path| path.is_file()) {
        return Ok(executable);
    }

    let binary_dir = current_executable.parent().unwrap_or_else(|| Path::new("."));
    let mut sibling = binary_dir.join("incan");
    sibling.set_extension(std::env::consts::EXE_EXTENSION);
    if sibling.is_file() {
        return Ok(sibling);
    }

    let mut parent_sibling = binary_dir.parent().unwrap_or_else(|| Path::new(".")).join("incan");
    parent_sibling.set_extension(std::env::consts::EXE_EXTENSION);
    if parent_sibling.is_file() {
        return Ok(parent_sibling);
    }

    Err(CliError::failure(format!(
        "SDK provider publication requires the incan CLI executable at {} or {}; build that binary before running compiler-backed utilities",
        sibling.display(),
        parent_sibling.display()
    )))
}

/// Find the verified workspace Cargo.lock available to a development SDK provider build.
///
/// A standalone artifact crate otherwise resolves its own newest compatible versions, which can differ from the
/// compiler workspace's verified offline cache. Installed SDK layouts need not contain a workspace lockfile, so they
/// deliberately retain normal Cargo resolution.
fn sdk_provider_workspace_lock(stdlib_root: &Path) -> Option<PathBuf> {
    stdlib_root
        .ancestors()
        .skip(1)
        .map(|parent| parent.join("Cargo.lock"))
        .find(|path| path.is_file())
        .map(|path| fs::canonicalize(&path).unwrap_or(path))
}

/// Seed a development SDK provider build from the verified enclosing workspace lockfile.
fn seed_sdk_provider_workspace_lock(workspace_lock: Option<&Path>, artifact_root: &Path) -> CliResult<()> {
    let Some(workspace_lock) = workspace_lock else {
        return Ok(());
    };
    fs::create_dir_all(artifact_root).map_err(|error| {
        CliError::failure(format!(
            "failed to create SDK provider artifact directory {}: {error}",
            artifact_root.display()
        ))
    })?;
    fs::copy(workspace_lock, artifact_root.join("Cargo.lock")).map_err(|error| {
        CliError::failure(format!(
            "failed to seed SDK provider artifact lock from {}: {error}",
            workspace_lock.display()
        ))
    })?;
    Ok(())
}

/// Keep the bootstrap artifact lock alive for the whole preparation/publish transaction.
///
/// The compiler cannot call its own Incan `std.fs` artifact before that artifact exists. This is therefore a
/// deliberately narrow native bootstrap boundary, mirroring RFC 112's advisory-lock contract while the compiler
/// produces the first Incan-owned stdlib artifact.
struct SdkProviderStoreLock {
    _file: fs::File,
}

/// Acquire the artifact-store lock that serializes all bootstrap builds and publications.
fn acquire_sdk_provider_store_lock(store_root: &Path) -> CliResult<SdkProviderStoreLock> {
    fs::create_dir_all(store_root).map_err(|error| {
        CliError::failure(format!(
            "failed to create SDK provider store {}: {error}",
            store_root.display()
        ))
    })?;
    let lock_path = store_root.join(".incan.lock");
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| CliError::failure(format!("failed to open artifact lock {}: {error}", lock_path.display())))?;
    file.lock().map_err(|error| {
        CliError::failure(format!(
            "failed to acquire artifact lock {}: {error}",
            lock_path.display()
        ))
    })?;
    Ok(SdkProviderStoreLock { _file: file })
}

/// Hash one sorted provider source subtree while excluding generated build output.
fn hash_sdk_provider_source_tree(root: &Path, current: &Path, hasher: &mut Sha256) -> CliResult<()> {
    let mut entries = fs::read_dir(current)
        .map_err(|error| {
            CliError::failure(format!(
                "failed to read stdlib source directory {}: {error}",
                current.display()
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            CliError::failure(format!(
                "failed to enumerate stdlib source directory {}: {error}",
                current.display()
            ))
        })?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(|error| {
            CliError::failure(format!(
                "failed to make stdlib source path {} relative: {error}",
                path.display()
            ))
        })?;
        if relative.components().any(|component| component.as_os_str() == "target") {
            continue;
        }
        let file_type = entry.file_type().map_err(|error| {
            CliError::failure(format!(
                "failed to inspect stdlib source path {}: {error}",
                path.display()
            ))
        })?;
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        if file_type.is_dir() {
            hasher.update(b"directory\0");
            hash_sdk_provider_source_tree(root, &path, hasher)?;
        } else if file_type.is_file() {
            hasher.update(b"file\0");
            let bytes = fs::read(&path).map_err(|error| {
                CliError::failure(format!("failed to read stdlib source file {}: {error}", path.display()))
            })?;
            hasher.update(bytes);
        } else if file_type.is_symlink() {
            hasher.update(b"symlink\0");
            let target = fs::read_link(&path).map_err(|error| {
                CliError::failure(format!(
                    "failed to read stdlib source symlink {}: {error}",
                    path.display()
                ))
            })?;
            hasher.update(target.to_string_lossy().as_bytes());
        }
        hasher.update([0xff]);
    }
    Ok(())
}

/// Derive the immutable provider-store identity from every input that can change generated Rust or its dependency
/// closure. The identity is content based, so a stale provider set is never accepted because a directory exists.
fn sdk_provider_store_identity(
    source_root: &Path,
    executable: &Path,
    workspace_lock: Option<&Path>,
    distribution_profile: &str,
) -> CliResult<String> {
    let mut hasher = Sha256::new();
    hasher.update(b"incan-sdk-provider-store-v2\0");
    hash_sdk_provider_source_tree(source_root, source_root, &mut hasher)?;
    hasher.update(b"compiler-version\0");
    hasher.update(crate::version::INCAN_VERSION.as_bytes());
    hasher.update(b"distribution-profile\0");
    hasher.update(distribution_profile.as_bytes());

    hasher.update(b"compiler-executable-content\0");
    let executable = fs::canonicalize(executable).unwrap_or_else(|_| executable.to_path_buf());
    hasher.update(sdk_provider_compiler_digest(&executable)?);

    hasher.update(b"workspace-lock\0");
    if let Some(workspace_lock) = workspace_lock {
        hasher.update(fs::read(workspace_lock).map_err(|error| {
            CliError::failure(format!(
                "failed to read workspace lock {}: {error}",
                workspace_lock.display()
            ))
        })?);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Hash the running compiler once per process with BLAKE3's optimized implementation, independent of its path.
fn sdk_provider_compiler_digest(executable: &Path) -> CliResult<[u8; 32]> {
    if let Some(digest) = SDK_PROVIDER_COMPILER_DIGESTS
        .lock()
        .map_err(|_| CliError::failure("failed to lock the compiler-content digest cache"))?
        .get(executable)
        .copied()
    {
        return Ok(digest);
    }

    let mut executable_file = fs::File::open(executable).map_err(|error| {
        CliError::failure(format!(
            "failed to read compiler executable {}: {error}",
            executable.display()
        ))
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = executable_file.read(&mut buffer).map_err(|error| {
            CliError::failure(format!(
                "failed to read compiler executable {}: {error}",
                executable.display()
            ))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = *hasher.finalize().as_bytes();
    SDK_PROVIDER_COMPILER_DIGESTS
        .lock()
        .map_err(|_| CliError::failure("failed to lock the compiler-content digest cache"))?
        .insert(executable.to_path_buf(), digest);
    Ok(digest)
}

/// Select one user-shared development cache instead of duplicating identical provider artifacts in every checkout.
fn default_sdk_provider_store(
    stdlib_root: &Path,
    incan_home: Option<std::ffi::OsString>,
    user_home: Option<std::ffi::OsString>,
) -> PathBuf {
    incan_home
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            user_home
                .filter(|path| !path.is_empty())
                .map(|path| PathBuf::from(path).join(".incan"))
        })
        .map(|root| root.join("cache").join("providers").join("sdk-v2"))
        .unwrap_or_else(|| stdlib_root.join("target").join("incan_sdk_components"))
}

/// Flush every staged artifact file and directory before atomic publication.
fn sync_sdk_provider_tree(path: &Path) -> CliResult<()> {
    let mut entries = fs::read_dir(path)
        .map_err(|error| {
            CliError::failure(format!(
                "failed to read staged artifact directory {}: {error}",
                path.display()
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            CliError::failure(format!(
                "failed to enumerate staged artifact directory {}: {error}",
                path.display()
            ))
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let entry_path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            CliError::failure(format!(
                "failed to inspect staged artifact path {}: {error}",
                entry_path.display()
            ))
        })?;
        if file_type.is_dir() {
            sync_sdk_provider_tree(&entry_path)?;
        } else if file_type.is_file() {
            fs::File::open(&entry_path)
                .and_then(|file| file.sync_all())
                .map_err(|error| {
                    CliError::failure(format!(
                        "failed to synchronize staged artifact file {}: {error}",
                        entry_path.display()
                    ))
                })?;
        }
    }
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            CliError::failure(format!(
                "failed to synchronize staged artifact directory {}: {error}",
                path.display()
            ))
        })
}

/// Flush the artifact store after publishing a new immutable artifact directory.
fn sync_sdk_provider_store(store_root: &Path) -> CliResult<()> {
    fs::File::open(store_root)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            CliError::failure(format!(
                "failed to synchronize artifact store {}: {error}",
                store_root.display()
            ))
        })
}

/// Allocate a unique private staging directory for one artifact identity.
fn staged_sdk_provider_root(store_root: &Path, identity: &str) -> CliResult<PathBuf> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| CliError::failure(format!("system clock predates Unix epoch: {error}")))?;
    Ok(store_root.join(format!(
        ".staging-{identity}-{}-{}",
        std::process::id(),
        elapsed.as_nanos()
    )))
}

/// Build and atomically publish every SDK component provider from the source catalog.
fn prepare_sdk_provider_inventory() -> CliResult<Arc<SdkInventory>> {
    let stdlib_root = crate::cli::prelude::find_stdlib_dir().ok_or_else(|| {
        CliError::failure("cannot locate built-in stdlib sources needed to prepare SDK component providers")
    })?;
    let stdlib_root = fs::canonicalize(&stdlib_root).map_err(|error| {
        CliError::failure(format!(
            "failed to canonicalize built-in stdlib source directory {}: {error}",
            stdlib_root.display()
        ))
    })?;
    let catalog = SdkSourceCatalog::read_from_path(&stdlib_root.join(SDK_SOURCE_CATALOG_FILE))
        .map_err(|error| CliError::failure(error.to_string()))?;
    catalog
        .validate_compiler_version(crate::version::INCAN_VERSION)
        .map_err(|error| CliError::failure(error.to_string()))?;
    let current_exe = env::current_exe()
        .map_err(|error| CliError::failure(format!("failed to resolve current incan executable: {error}")))?;
    let cargo_test_binary = env::var_os("CARGO_BIN_EXE_incan")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    let executable = sdk_provider_builder_executable(cargo_test_binary, current_exe)?;
    let workspace_lock = sdk_provider_workspace_lock(&stdlib_root);
    let distribution_profile = env::var(INTERNAL_SDK_DISTRIBUTION_PROFILE_ENV)
        .ok()
        .filter(|profile| !profile.is_empty())
        .unwrap_or_else(|| "full".to_string());
    let store_root = env::var_os(INTERNAL_SDK_PROVIDER_STORE_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(test) {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/incan_test_sdk_provider_store")
            } else {
                default_sdk_provider_store(
                    &stdlib_root,
                    env::var_os("INCAN_HOME"),
                    env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")),
                )
            }
        });
    let identity = sdk_provider_store_identity(
        &stdlib_root,
        &executable,
        workspace_lock.as_deref(),
        &distribution_profile,
    )?;
    let _lock = acquire_sdk_provider_store_lock(&store_root)?;
    let artifact_root = store_root.join(&identity);
    let inventory_path = artifact_root.join(SDK_INVENTORY_FILE);
    if inventory_path.is_file() {
        let inventory =
            SdkInventory::read_from_path(&inventory_path).map_err(|error| CliError::failure(error.to_string()))?;
        inventory
            .validate_compiler_compatibility(
                crate::version::INCAN_VERSION,
                crate::version::SDK_PROVIDER_CODEGEN_REVISION,
            )
            .map_err(|error| CliError::failure(error.to_string()))?;
        record_sdk_provider_root(&artifact_root)?;
        return Ok(Arc::new(inventory));
    }
    if artifact_root.exists() {
        return Err(CliError::failure(format!(
            "compiled SDK component artifact at {} is incomplete; refusing to overwrite an already published identity",
            artifact_root.display()
        )));
    }

    let staging_root = staged_sdk_provider_root(&store_root, &identity)?;
    let staged_inventory = match build_sdk_components_into_staging(
        &catalog,
        &executable,
        workspace_lock.as_deref(),
        &staging_root,
        &distribution_profile,
    ) {
        Ok(inventory) => inventory,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging_root);
            return Err(error);
        }
    };
    sync_sdk_provider_tree(&staging_root)?;
    fs::rename(&staging_root, &artifact_root).map_err(|error| {
        CliError::failure(format!(
            "failed to publish compiled SDK components from {} to {}: {error}",
            staging_root.display(),
            artifact_root.display()
        ))
    })?;
    sync_sdk_provider_store(&store_root)?;
    let published_inventory_path = artifact_root.join(SDK_INVENTORY_FILE);
    let published = SdkInventory::read_from_path(&published_inventory_path).map_err(|error| {
        CliError::failure(format!(
            "failed to load published SDK component inventory for {}: {error}",
            staged_inventory.identity()
        ))
    })?;
    record_sdk_provider_root(&artifact_root)?;
    Ok(Arc::new(published))
}

/// Report the exact immutable provider root to release packaging when requested.
fn record_sdk_provider_root(artifact_root: &Path) -> CliResult<()> {
    let Some(path_file) = env::var_os(INTERNAL_SDK_PROVIDER_PATH_FILE_ENV).filter(|path| !path.is_empty()) else {
        return Ok(());
    };
    fs::write(&path_file, format!("{}\n", artifact_root.display())).map_err(|error| {
        CliError::failure(format!(
            "failed to record SDK provider root in {}: {error}",
            PathBuf::from(path_file).display()
        ))
    })
}

/// Build source components in dependency order while exposing only already-published providers to each producer.
fn build_sdk_components_into_staging(
    catalog: &SdkSourceCatalog,
    executable: &Path,
    workspace_lock: Option<&Path>,
    staging_root: &Path,
    distribution_profile: &str,
) -> CliResult<SdkInventory> {
    fs::create_dir_all(staging_root).map_err(|error| {
        CliError::failure(format!(
            "failed to create SDK component staging directory {}: {error}",
            staging_root.display()
        ))
    })?;
    if let Some(workspace_lock) = workspace_lock {
        fs::copy(workspace_lock, staging_root.join("Cargo.lock")).map_err(|error| {
            CliError::failure(format!(
                "failed to publish shared SDK provider lock from {}: {error}",
                workspace_lock.display()
            ))
        })?;
    }
    let mut inventory = source_catalog_inventory(catalog, staging_root);
    let inventory_path = staging_root.join(SDK_INVENTORY_FILE);
    let cargo_target_dir = staging_root.join(".cargo-target");
    let caller_cargo_target = env::var_os(GENERATED_CARGO_TARGET_DIR_ENV).filter(|path| !path.is_empty());
    let mut built_any = false;

    for component in catalog.publication_order() {
        let output_root = staging_root.join("components").join(&component.id);
        seed_sdk_provider_workspace_lock(workspace_lock, &output_root)?;
        let manifest = ProjectManifest::discover(&component.project_root)
            .map_err(|error| CliError::failure(error.to_string()))?
            .ok_or_else(|| {
                CliError::failure(format!(
                    "SDK component `{}` has no incan.toml at {}",
                    component.id,
                    component.project_root.display()
                ))
            })?;
        let provider_name = manifest
            .project
            .as_ref()
            .and_then(|project| project.name.clone())
            .ok_or_else(|| CliError::failure(format!("SDK component `{}` has no project name", component.id)))?;
        eprintln!(
            "Preparing SDK component `{}` with `incan build --lib` in {}",
            component.id,
            component.project_root.display()
        );
        let mut command = Command::new(executable);
        command
            .current_dir(&component.project_root)
            .args(["build", "--lib", "."])
            .arg(&output_root)
            .arg("--all-features");
        configure_sdk_provider_build_environment(
            &mut command,
            &component.id,
            &cargo_target_dir,
            caller_cargo_target.as_deref(),
        );
        if built_any {
            inventory
                .write_to_path(&inventory_path)
                .map_err(|error| CliError::failure(error.to_string()))?;
            command.env(SDK_INVENTORY_OVERRIDE_ENV, &inventory_path);
        } else {
            command.env_remove(SDK_INVENTORY_OVERRIDE_ENV);
        }
        if let Some(workspace_lock) = workspace_lock {
            command.env(INTERNAL_CARGO_LOCK_PAYLOAD_PATH_ENV, workspace_lock);
        }
        let output = command.output().map_err(|error| {
            CliError::failure(format!(
                "failed to run SDK component build for `{}` at {}: {error}",
                component.id,
                component.project_root.display()
            ))
        })?;
        if !output.status.success() {
            return Err(nested_sdk_component_build_error(
                component.id.as_str(),
                &component.project_root,
                &output,
            ));
        }
        let manifest_path = output_root.join(format!("{provider_name}.incnlib"));
        let provider_manifest = LibraryManifest::read_from_path(&manifest_path).map_err(|error| {
            CliError::failure(format!(
                "failed to read SDK component `{}` manifest {}: {error}",
                component.id,
                manifest_path.display()
            ))
        })?;
        let component_lock = output_root.join("Cargo.lock");
        if component_lock.is_file() {
            fs::remove_file(&component_lock).map_err(|error| {
                CliError::failure(format!(
                    "failed to remove duplicated SDK component lock {}: {error}",
                    component_lock.display()
                ))
            })?;
        }
        let namespace_claims = sdk_component_namespace_claims(
            &component.id,
            &component.namespace_roots,
            &provider_manifest.contract_metadata.provider.namespace_claims,
        )?;
        let digest = digest_provider_artifact(&output_root).map_err(|error| {
            CliError::failure(format!(
                "failed to hash SDK component `{}` artifact {}: {error}",
                component.id,
                output_root.display()
            ))
        })?;
        let inventory_component = inventory.components.get_mut(&component.id).ok_or_else(|| {
            CliError::failure(format!(
                "SDK source catalog lost component `{}` while publishing",
                component.id
            ))
        })?;
        inventory_component.available = true;
        inventory_component.providers = vec![SdkProviderDescriptor {
            name: provider_manifest.name,
            version: provider_manifest.version,
            digest,
            namespace_claims,
            manifest_path: Some(manifest_path),
            crate_root: Some(output_root),
        }];
        built_any = true;
    }
    if cargo_target_dir.exists() {
        fs::remove_dir_all(&cargo_target_dir).map_err(|error| {
            CliError::failure(format!(
                "failed to remove transient SDK provider Cargo target {}: {error}",
                cargo_target_dir.display()
            ))
        })?;
    }
    restrict_staged_sdk_profile(catalog, distribution_profile, staging_root, &mut inventory)?;
    inventory
        .write_to_path(&inventory_path)
        .map_err(|error| CliError::failure(error.to_string()))?;
    Ok(inventory)
}

/// Preserve a caller-owned Cargo target while keeping the ordinary provider-publication fallback transaction-local.
fn configure_sdk_provider_build_environment(
    command: &mut Command,
    component_id: &str,
    transaction_cargo_target: &Path,
    caller_cargo_target: Option<&std::ffi::OsStr>,
) {
    let cargo_target_dir = caller_cargo_target.map(Path::new).unwrap_or(transaction_cargo_target);
    command
        .env_remove(INTERNAL_MANIFEST_OVERRIDE_ENV)
        .env_remove(INTERNAL_PROJECT_ROOT_OVERRIDE_ENV)
        .env(SDK_PROVIDER_BUILD_ENV, component_id)
        .env(INTERNAL_LIBRARY_ARTIFACT_ONLY_ENV, "1")
        // Share transient Cargo artifacts across components, then remove them before immutable provider publication.
        .env(GENERATED_CARGO_TARGET_DIR_ENV, cargo_target_dir);
}

/// Validate producer claims against the namespace grant before publishing them into the SDK inventory.
fn sdk_component_namespace_claims(
    component_id: &str,
    namespace_roots: &BTreeSet<String>,
    claims: &[ProviderModuleClaim],
) -> CliResult<BTreeSet<Vec<String>>> {
    let unauthorized = claims
        .iter()
        .filter(|claim| {
            claim
                .module_path
                .first()
                .is_none_or(|root| !namespace_roots.contains(root))
        })
        .map(|claim| claim.module_path.join("."))
        .collect::<Vec<_>>();
    if !unauthorized.is_empty() {
        return Err(CliError::failure(format!(
            "SDK component `{component_id}` claims module(s) {} outside its granted namespace roots [{}]",
            unauthorized.join(", "),
            namespace_roots.iter().cloned().collect::<Vec<_>>().join(", ")
        )));
    }

    Ok(claims
        .iter()
        .map(|claim| {
            let mut path = vec![stdlib::STDLIB_ROOT.to_string()];
            path.extend(claim.module_path.iter().cloned());
            path
        })
        .collect())
}

/// Remove provider payloads outside one release distribution profile while retaining their catalog records.
fn restrict_staged_sdk_profile(
    catalog: &SdkSourceCatalog,
    distribution_profile: &str,
    staging_root: &Path,
    inventory: &mut SdkInventory,
) -> CliResult<()> {
    if !catalog.profiles.contains_key(distribution_profile) {
        return Err(CliError::failure(format!(
            "unknown SDK distribution profile `{distribution_profile}`"
        )));
    }
    let resolved = inventory
        .resolve_catalog(&SdkComponentSelection {
            profile: distribution_profile.to_string(),
            components: BTreeSet::new(),
            exclude_components: BTreeSet::new(),
        })
        .map_err(|error| CliError::failure(error.to_string()))?;
    for component in inventory.components.values_mut() {
        if resolved.enabled.contains(&component.id) {
            continue;
        }
        component.available = false;
        for provider in &mut component.providers {
            provider.manifest_path = None;
            provider.crate_root = None;
        }
        let component_root = staging_root.join("components").join(&component.id);
        if component_root.exists() {
            fs::remove_dir_all(&component_root).map_err(|error| {
                CliError::failure(format!(
                    "failed to exclude SDK component payload {}: {error}",
                    component_root.display()
                ))
            })?;
        }
    }
    Ok(())
}

/// Create the unavailable installation catalog before component publication begins.
fn source_catalog_inventory(catalog: &SdkSourceCatalog, root: &Path) -> SdkInventory {
    let components = catalog
        .components
        .iter()
        .map(|(id, component)| {
            (
                id.clone(),
                SdkComponent {
                    id: id.clone(),
                    version: catalog.sdk_version.clone(),
                    mandatory: component.mandatory,
                    available: false,
                    dependencies: component.dependencies.clone(),
                    providers: Vec::new(),
                },
            )
        })
        .collect();
    SdkInventory {
        root: root.to_path_buf(),
        sdk_id: catalog.sdk_id.clone(),
        sdk_version: catalog.sdk_version.clone(),
        compiler_requirement: catalog.compiler_requirement.clone(),
        provider_codegen_revision: crate::version::SDK_PROVIDER_CODEGEN_REVISION,
        components,
        profiles: catalog.profiles.clone(),
    }
}

/// Preserve nested compiler stdout and stderr when one component publication fails.
fn nested_sdk_component_build_error(component: &str, project_root: &Path, output: &std::process::Output) -> CliError {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let diagnostics = [stderr.trim(), stdout.trim()]
        .into_iter()
        .filter(|message| !message.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    CliError::failure(format!(
        "failed to prepare SDK component `{component}` at {}{}",
        project_root.display(),
        if diagnostics.is_empty() {
            String::new()
        } else {
            format!("\n{diagnostics}")
        }
    ))
}

/// Cargo execution policy resolved from CLI inputs and environment defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CargoPolicy {
    pub(crate) offline: bool,
    pub(crate) locked: bool,
    pub(crate) frozen: bool,
    pub(crate) extra_args: Vec<String>,
}

/// CLI policy flags, including explicit disables for environment defaults.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CargoPolicyCliFlags {
    pub offline: bool,
    pub no_offline: bool,
    pub locked: bool,
    pub no_locked: bool,
    pub frozen: bool,
    pub no_frozen: bool,
}

impl CargoPolicy {
    /// Resolve policy for a user-facing build/run/test command.
    pub(crate) fn from_cli_and_env(
        cli_flags: CargoPolicyCliFlags,
        cli_cargo_args: Vec<String>,
        cli_passthrough_args: Vec<String>,
    ) -> Self {
        Self::from_sources(cli_flags, cli_cargo_args, cli_passthrough_args, |name| {
            env::var(name).ok()
        })
    }

    /// Build an explicit policy for internal Cargo invocations that should not read RFC 020 env defaults.
    pub(crate) fn explicit(offline: bool, locked: bool, frozen: bool, extra_args: Vec<String>) -> Self {
        let mut policy = Self {
            offline,
            locked,
            frozen,
            extra_args,
        };
        policy.normalize();
        policy
    }

    /// Resolve policy from injected sources; used by tests to avoid mutating process env.
    fn from_sources<F>(
        cli_flags: CargoPolicyCliFlags,
        mut cli_cargo_args: Vec<String>,
        cli_passthrough_args: Vec<String>,
        env_value: F,
    ) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let env_frozen = env_flag_value(env_value("INCAN_FROZEN").as_deref());
        let env_offline = env_flag_value(env_value("INCAN_OFFLINE").as_deref());
        let env_locked = env_flag_value(env_value("INCAN_LOCKED").as_deref());

        cli_cargo_args.extend(cli_passthrough_args);
        let extra_args = if cli_cargo_args.is_empty() {
            split_env_cargo_args(env_value("INCAN_CARGO_ARGS").as_deref())
        } else {
            cli_cargo_args
        };

        Self::explicit(
            resolve_cli_env_flag(env_offline, cli_flags.offline, cli_flags.no_offline),
            resolve_cli_env_flag(env_locked, cli_flags.locked, cli_flags.no_locked),
            resolve_cli_env_flag(env_frozen, cli_flags.frozen, cli_flags.no_frozen),
            extra_args,
        )
    }

    /// Apply derived policy semantics after raw source resolution.
    fn normalize(&mut self) {
        if self.frozen {
            self.offline = true;
            self.locked = true;
        }
    }
}

/// Enforce the project-level `requires-incan` constraint for a…47990 tokens truncated…      )?;

        let modules = collect_modules(entry.to_string_lossy().as_ref())?;
        let module_idx_by_key = module_key_index(&modules);
        for (idx, module) in modules.iter().enumerate() {
            let deps = imported_module_deps_for_with_index(&modules, idx, &module_idx_by_key);
            let mut checker = typechecker::TypeChecker::new();
            if let Err(errs) = checker.check_with_imports(&module.ast, &deps) {
                return Err(format!(
                    "typecheck failed for module {}: {:?}",
                    module.file_path.display(),
                    errs.iter().map(|e| e.message.clone()).collect::<Vec<_>>()
                )
                .into());
            }
        }
        Ok(())
    }

    #[test]
    fn collect_modules_supports_directory_module_cycles_from_example_entry() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let project_root = tmp.path();
        std::fs::write(
            project_root.join("incan.toml"),
            r#"[project]
name = "example_directory_cycle_demo"
version = "0.1.0"
"#,
        )?;
        let src_dir = project_root.join("src");
        let dataset_dir = src_dir.join("dataset");
        let examples_dir = project_root.join("examples");
        std::fs::create_dir_all(&dataset_dir)?;
        std::fs::create_dir_all(&examples_dir)?;
        std::fs::write(
            src_dir.join("session.incn"),
            r#"from dataset import DataFrame, LazyFrame

pub model SessionError:
    pub message: str

pub class Session:
    @staticmethod
    def default() -> Session:
        return Session()

    def read_csv[T with Clone](self, _logical_name: str, _uri: str) -> Result[LazyFrame[T], SessionError]:
        return Err(SessionError(message=str("not implemented")))

pub def collect_with_active_session[T with Clone](data: LazyFrame[T]) -> Result[DataFrame[T], SessionError]:
    return Err(SessionError(message=str("not implemented")))
"#,
        )?;
        std::fs::write(
            dataset_dir.join("mod.incn"),
            r#"from session import SessionError, collect_with_active_session

pub trait DataSet[T with Clone]:
    pass

pub class DataFrame[T with Clone] with DataSet:
    def clone(self) -> Self:
        return self

pub class LazyFrame[T with Clone] with DataSet:
    def clone(self) -> Self:
        return self

    def collect(self) -> Result[DataFrame[T], SessionError]:
        return collect_with_active_session[T](self.clone())
"#,
        )?;
        let entry = examples_dir.join("main.incn");
        std::fs::write(
            &entry,
            r#"from session import Session, SessionError

@derive(Clone)
pub model OrderLine:
    pub sku: str

def main() -> Result[None, SessionError]:
    session = Session.default()
    lines = session.read_csv[OrderLine](str("orders"), str("input.csv"))?
    df = lines.clone().collect()?
    df.clone()
    return Ok(None)
"#,
        )?;

        let modules = collect_modules(entry.to_string_lossy().as_ref())?;
        let module_idx_by_key = module_key_index(&modules);
        for (idx, module) in modules.iter().enumerate() {
            let deps = imported_module_deps_for_with_index(&modules, idx, &module_idx_by_key);
            let mut checker = typechecker::TypeChecker::new();
            if let Err(errs) = checker.check_with_imports(&module.ast, &deps) {
                return Err(format!(
                    "typecheck failed for module {}: {:?}",
                    module.file_path.display(),
                    errs.iter().map(|e| e.message.clone()).collect::<Vec<_>>()
                )
                .into());
            }
        }
        Ok(())
    }

    #[test]
    fn collect_modules_cycle_falls_back_to_deterministic_order() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let project_root = tmp.path();
        std::fs::write(
            project_root.join("incan.toml"),
            r#"[project]
name = "cycle_demo"
version = "0.1.0"
"#,
        )?;
        let src_dir = project_root.join("src");
        std::fs::create_dir_all(&src_dir)?;

        std::fs::write(
            src_dir.join("a.incn"),
            r#"from b import pong

pub def ping() -> int:
    return pong()
"#,
        )?;
        std::fs::write(
            src_dir.join("b.incn"),
            r#"from a import ping

pub def pong() -> int:
    return 1
"#,
        )?;
        let entry = src_dir.join("main.incn");
        std::fs::write(
            &entry,
            r#"from a import ping

pub def main() -> int:
    return ping()
"#,
        )?;

        let modules = collect_modules(entry.to_string_lossy().as_ref())?;
        assert_eq!(modules.len(), 3, "expected all modules to be collected even with cycle");
        assert!(modules[0].file_path.ends_with("src/b.incn"));
        assert!(modules[1].file_path.ends_with("src/a.incn"));
        assert!(modules[2].file_path.ends_with("src/main.incn"));
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn rust_inspect_workspace_fingerprint_is_deterministic() {
        let requirements = ProjectRequirements::default();
        let resolved = ResolvedDependencies {
            dependencies: vec![DependencySpec {
                crate_name: "serde".to_string(),
                version: Some("1".to_string()),
                features: vec!["derive".to_string()],
                default_features: true,
                source: DependencySource::Registry,
                optional: false,
                package: None,
            }],
            dev_dependencies: Vec::new(),
        };
        let fp_a = super::rust_inspect_workspace_fingerprint(
            "probe",
            "probe",
            Some("2021"),
            &resolved,
            &requirements.stdlib_features,
            &requirements.sdk_dependency_rebindings,
            &requirements.sdk_path_dependencies,
            &requirements.sdk_artifact_projections,
            Some("lock-bytes"),
            None,
            false,
            Path::new("/cache/target"),
        );
        let fp_b = super::rust_inspect_workspace_fingerprint(
            "probe",
            "probe",
            Some("2021"),
            &resolved,
            &requirements.stdlib_features,
            &requirements.sdk_dependency_rebindings,
            &requirements.sdk_path_dependencies,
            &requirements.sdk_artifact_projections,
            Some("lock-bytes"),
            None,
            false,
            Path::new("/cache/target"),
        );
        let workspace_fp = super::rust_inspect_workspace_fingerprint(
            "probe",
            "incan_workspace",
            Some("2021"),
            &resolved,
            &requirements.stdlib_features,
            &requirements.sdk_dependency_rebindings,
            &requirements.sdk_path_dependencies,
            &requirements.sdk_artifact_projections,
            Some("lock-bytes"),
            None,
            false,
            Path::new("/cache/target"),
        );
        let target_fp = super::rust_inspect_workspace_fingerprint(
            "probe",
            "probe",
            Some("2021"),
            &resolved,
            &requirements.stdlib_features,
            &requirements.sdk_dependency_rebindings,
            &requirements.sdk_path_dependencies,
            &requirements.sdk_artifact_projections,
            Some("lock-bytes"),
            None,
            false,
            Path::new("/cache/other-target"),
        );
        assert_eq!(fp_a, fp_b);
        assert_ne!(fp_a, workspace_fp);
        assert_ne!(fp_a, target_fp);
        assert!(fp_a.starts_with(super::RUST_INSPECT_WORKSPACE_FINGERPRINT_PREFIX));
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn rust_inspect_workspace_fingerprint_changes_when_lock_payload_changes() {
        let requirements = ProjectRequirements::default();
        let resolved = ResolvedDependencies {
            dependencies: Vec::new(),
            dev_dependencies: Vec::new(),
        };
        let fp_one = super::rust_inspect_workspace_fingerprint(
            "p",
            "p",
            None,
            &resolved,
            &requirements.stdlib_features,
            &requirements.sdk_dependency_rebindings,
            &requirements.sdk_path_dependencies,
            &requirements.sdk_artifact_projections,
            Some("lock-a"),
            None,
            false,
            Path::new("/cache/target"),
        );
        let fp_two = super::rust_inspect_workspace_fingerprint(
            "p",
            "p",
            None,
            &resolved,
            &requirements.stdlib_features,
            &requirements.sdk_dependency_rebindings,
            &requirements.sdk_path_dependencies,
            &requirements.sdk_artifact_projections,
            Some("lock-b"),
            None,
            false,
            Path::new("/cache/target"),
        );
        assert_ne!(fp_one, fp_two);
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn rust_inspect_projection_receives_frozen_cargo_policy() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let requirements = ProjectRequirements::default();
        let resolved = ResolvedDependencies {
            dependencies: Vec::new(),
            dev_dependencies: Vec::new(),
        };
        let canonical = format!(
            "version = 4\n\n[[package]]\nname = \"incan_workspace\"\nversion = \"{}\"\n",
            crate::version::INCAN_VERSION
        );
        let fingerprint = super::rust_inspect_workspace_fingerprint(
            "policy_probe",
            "caller",
            None,
            &resolved,
            &requirements.stdlib_features,
            &requirements.sdk_dependency_rebindings,
            &requirements.sdk_path_dependencies,
            &requirements.sdk_artifact_projections,
            Some(&canonical),
            Some("incan_workspace"),
            false,
            &tmp.path().join("cargo-target"),
        );
        let output_dir = super::rust_inspect_workspace_dir(tmp.path(), "policy_probe", &fingerprint);
        let flags = vec!["--frozen".to_string()];

        let result = ensure_rust_inspect_workspace_with_cargo_package_name(
            tmp.path(),
            "policy_probe",
            "caller",
            None,
            &resolved,
            &requirements,
            Some(canonical),
            Some("incan_workspace"),
            false,
            &tmp.path().join("cargo-target"),
            &flags,
        );
        assert!(
            result.is_err(),
            "the deliberately incomplete canonical fixture must fail closed"
        );
        assert_eq!(
            crate::backend::project::runner::test_projection_cargo_policy(&output_dir),
            Some(flags),
            "rust-inspect must set frozen policy before attempting Cargo lock projection"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn rust_inspect_fingerprint_tracks_same_path_projection_rebuild_issue911() -> Result<(), Box<dyn std::error::Error>>
    {
        let workspace = tempfile::tempdir()?;
        let artifact = workspace.path().join("compiled");
        fs::create_dir_all(artifact.join("src"))?;
        fs::write(
            artifact.join("Cargo.toml"),
            "[package]\nname = \"issue911_compiled\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        fs::write(artifact.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n")?;
        let requirements = ProjectRequirements {
            sdk_artifact_projections: vec![SdkArtifactProjection {
                artifact: LibraryArtifactMetadata::from_crate_root("issue911_compiled", "issue911_compiled", &artifact),
            }],
            ..ProjectRequirements::default()
        };
        let resolved = ResolvedDependencies {
            dependencies: Vec::new(),
            dev_dependencies: Vec::new(),
        };
        let before = super::rust_inspect_workspace_fingerprint(
            "probe",
            "probe",
            None,
            &resolved,
            &requirements.stdlib_features,
            &requirements.sdk_dependency_rebindings,
            &requirements.sdk_path_dependencies,
            &requirements.sdk_artifact_projections,
            None,
            None,
            false,
            &workspace.path().join("cargo-target"),
        );
        fs::write(artifact.join("src/lib.rs"), "pub fn value() -> u8 { 2 }\n")?;
        let after = super::rust_inspect_workspace_fingerprint(
            "probe",
            "probe",
            None,
            &resolved,
            &requirements.stdlib_features,
            &requirements.sdk_dependency_rebindings,
            &requirements.sdk_path_dependencies,
            &requirements.sdk_artifact_projections,
            None,
            None,
            false,
            &workspace.path().join("cargo-target"),
        );

        assert_ne!(before, after);
        Ok(())
    }

    #[test]
    fn helper_requirements_keep_unused_active_sdk_path_targets_issue911() -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempfile::tempdir()?;
        let artifact = workspace.path().join("unused-sdk-provider");
        let record = crate::provider::ProviderRecord {
            identity: crate::provider::ProviderIdentity {
                name: "incan_issue911_unused_sdk".to_string(),
                version: "0.5.0".to_string(),
                digest: "sha256:issue911-unused".to_string(),
                feature_projection: BTreeSet::new(),
            },
            provenance: crate::provider::ProviderProvenance::Sdk {
                sdk_identity: "incan@0.5.0".to_string(),
                component_id: "issue911-unused".to_string(),
                inventory_path: None,
            },
            authority: crate::provider::NamespaceAuthority::SdkReserved,
            namespace_claims: BTreeSet::from([vec!["std".to_string(), "issue911_unused".to_string()]]),
            available: true,
            enabled: true,
            manifest: Some(Arc::new(LibraryManifest::new("incan_issue911_unused_sdk", "0.5.0"))),
            artifact: Some(LibraryArtifactMetadata::from_crate_root(
                "incan_issue911_unused_sdk",
                "incan_issue911_unused_sdk",
                &artifact,
            )),
            implementation_facets: Vec::new(),
        };
        let plan = ProviderPlan::new(
            crate::frontend::library_manifest_index::LibraryManifestIndex::default(),
            vec![record.clone()],
            std::iter::empty(),
        )?;
        assert!(
            plan.sdk_link_roots().is_empty(),
            "unused SDK provider must not become a direct link root"
        );

        let mut requirements = ProjectRequirements::default();
        extend_requirements_with_provider_plan(&mut requirements, &plan)?;

        assert!(
            requirements.dependencies.is_empty(),
            "unused provider must not be linked directly"
        );
        assert_eq!(requirements.sdk_path_dependencies.len(), 1);
        assert!(matches!(
            &requirements.sdk_path_dependencies[0].source,
            DependencySource::Path { path } if path == &artifact
        ));

        let used_plan = ProviderPlan::new(
            crate::frontend::library_manifest_index::LibraryManifestIndex::default(),
            vec![record],
            [vec!["std".to_string(), "issue911_unused".to_string()]],
        )?;
        let mut used_requirements = ProjectRequirements::default();
        extend_requirements_with_provider_plan(&mut used_requirements, &used_plan)?;
        assert_eq!(used_requirements.dependencies.len(), 1);
        assert!(
            !used_requirements.dependencies[0].default_features,
            "new direct SDK edges must render explicit default-features = false"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn rust_inspect_helper_materializes_sdk_projection_issue911() -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempfile::tempdir()?;
        let artifact = workspace.path().join("compiled");
        let absent_sdk = workspace.path().join("sdk-cache-a/runtime");
        let active_sdk = workspace.path().join("sdk-cache-b/runtime");
        for root in [&artifact, &active_sdk] {
            fs::create_dir_all(root.join("src"))?;
        }
        fs::write(
            artifact.join("Cargo.toml"),
            format!(
                "[package]\nname = \"issue911_compiled\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n\n[dependencies.issue911_runtime]\npath = {:?}\ndefault-features = false\n",
                absent_sdk.to_string_lossy()
            ),
        )?;
        fs::write(
            artifact.join("src/lib.rs"),
            "pub fn value() -> u8 { issue911_runtime::value() }\n",
        )?;
        fs::write(
            active_sdk.join("Cargo.toml"),
            "[package]\nname = \"issue911_runtime\"\nversion = \"0.5.0\"\nedition = \"2024\"\n\n[workspace]\n",
        )?;
        fs::write(active_sdk.join("src/lib.rs"), "pub fn value() -> u8 { 3 }\n")?;
        let mut manifest = LibraryManifest::new("issue911_compiled", "0.1.0");
        manifest.contract_metadata.provider.provider_dependencies.push(
            crate::library_manifest::ProviderDependencyMetadata {
                kind: crate::library_manifest::ProviderDependencyKind::PrivateImplementation,
                dependency_key: "issue911_runtime".to_string(),
                provider_name: "issue911_runtime".to_string(),
                provider_version: "0.5.0".to_string(),
                artifact_digest: digest_provider_artifact(&active_sdk)?,
                relative_artifact_path: "../sdk-cache-a/runtime".to_string(),
                requested_features: BTreeSet::new(),
                default_features: false,
                optional: false,
            },
        );
        let manifest_path = artifact.join("issue911_compiled.incnlib");
        manifest.write_to_path(&manifest_path)?;
        let metadata = LibraryArtifactMetadata::from_manifest_path(
            "issue911_compiled",
            "issue911_compiled",
            manifest_path,
            artifact.clone(),
        );
        let requirements = ProjectRequirements {
            sdk_dependency_rebindings: vec![SdkDependencyRebinding {
                containing_artifact: metadata.clone(),
                source_crate_root: absent_sdk.clone(),
                provider_name: "issue911_runtime".to_string(),
                dependency_key: "issue911_runtime".to_string(),
                active_crate_root: active_sdk,
            }],
            sdk_artifact_projections: vec![SdkArtifactProjection { artifact: metadata }],
            ..ProjectRequirements::default()
        };
        let resolved = ResolvedDependencies {
            dependencies: vec![DependencySpec {
                crate_name: "issue911_compiled".to_string(),
                version: None,
                features: Vec::new(),
                default_features: true,
                source: DependencySource::Path { path: artifact },
                optional: false,
                package: None,
            }],
            dev_dependencies: Vec::new(),
        };

        let generated = ensure_rust_inspect_workspace_with_cargo_package_name(
            workspace.path(),
            "issue911_probe",
            "issue911_probe",
            Some("2024".to_string()),
            &resolved,
            &requirements,
            None,
            None,
            false,
            &workspace.path().join("cargo-target"),
            &[],
        )?;

        let cargo_manifest = fs::read_to_string(generated.join("Cargo.toml"))?;
        assert!(cargo_manifest.contains(".incan-sdk-rebound"));
        assert!(!cargo_manifest.contains(absent_sdk.to_string_lossy().as_ref()));
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let output = Command::new(cargo)
            .arg("check")
            .arg("--offline")
            .arg("--manifest-path")
            .arg(generated.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", workspace.path().join("cargo-target"))
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "rust-inspect projected Cargo graph failed:\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        let projection_parent = generated
            .parent()
            .ok_or("rust-inspect workspace has no parent")?
            .join(".incan-sdk-rebound");
        let shadow_root = fs::read_dir(&projection_parent)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.is_dir())
            .ok_or("missing rust-inspect projected artifact")?;
        fs::write(shadow_root.join("src/lib.rs"), "pub fn corrupt() {}\n")?;
        let regenerated = ensure_rust_inspect_workspace_with_cargo_package_name(
            workspace.path(),
            "issue911_probe",
            "issue911_probe",
            Some("2024".to_string()),
            &resolved,
            &requirements,
            None,
            None,
            false,
            &workspace.path().join("cargo-target"),
            &[],
        )?;
        assert_eq!(generated, regenerated);
        assert!(fs::read_to_string(shadow_root.join("src/lib.rs"))?.contains("value"));
        assert!(!absent_sdk.exists());
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn rust_inspect_workspace_dir_is_namespaced_by_input_fingerprint() {
        let root = Path::new("/workspace");
        let first = super::rust_inspect_workspace_dir(root, "demo", "v1:aaaaaaaaaaaaaaaaaaaaaaaa");
        let second = super::rust_inspect_workspace_dir(root, "demo", "v1:bbbbbbbbbbbbbbbbbbbbbbbb");

        assert_ne!(first, second);
        assert!(first.ends_with(Path::new("target/incan_lock/rust_inspect/demo-aaaaaaaaaaaaaaaa")));
        assert!(second.ends_with(Path::new("target/incan_lock/rust_inspect/demo-bbbbbbbbbbbbbbbb")));
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn rust_inspect_out_dirs_fingerprint_tracks_query_surface() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let manifest_dir = tmp.path();
        fs::create_dir_all(manifest_dir.join("src"))?;
        fs::write(
            manifest_dir.join("Cargo.toml"),
            "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )?;
        fs::write(manifest_dir.join("src").join("main.rs"), "fn main() {}\n")?;
        let target_dir = manifest_dir.join("target");

        let one = super::rust_inspect_out_dirs_fingerprint(manifest_dir, &target_dir, &["demo::One".to_string()])?;
        let two = super::rust_inspect_out_dirs_fingerprint(manifest_dir, &target_dir, &["demo::Two".to_string()])?;

        assert_ne!(
            one, two,
            "rust-inspect out-dir prewarm must rerun when the inspected ABI query surface changes"
        );
        assert!(one.starts_with(super::RUST_INSPECT_OUT_DIRS_FINGERPRINT_FILE));
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn rust_inspect_locked_prewarm_detects_stale_generated_lockfile() {
        let cannot_update = "error: cannot update the lock file /tmp/target/incan_lock/rust_inspect/demo/Cargo.lock because --locked was passed to prevent this";
        assert!(super::rust_inspect_locked_prewarm_needs_lock_update(cannot_update));

        let needs_update = "error: the lock file /tmp/target/incan_lock/rust_inspect/demo/Cargo.lock needs to be updated but --locked was passed to prevent this";
        assert!(super::rust_inspect_locked_prewarm_needs_lock_update(needs_update));

        assert!(!super::rust_inspect_locked_prewarm_needs_lock_update(
            "error: failed to select a version for `demo`"
        ));
        assert!(!super::rust_inspect_locked_prewarm_needs_lock_update(
            "error: package selected but no lock file policy was involved"
        ));
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn ensure_rust_inspect_workspace_uses_rust_safe_dependency_keys() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let requirements = ProjectRequirements::default();
        let resolved = ResolvedDependencies {
            dependencies: vec![DependencySpec {
                crate_name: "datafusion-substrait".to_string(),
                version: Some("53".to_string()),
                features: vec!["protoc".to_string()],
                default_features: true,
                source: DependencySource::Registry,
                optional: false,
                package: None,
            }],
            dev_dependencies: Vec::new(),
        };

        let out_dir = ensure_rust_inspect_workspace(
            tmp.path(),
            "metadata_probe",
            Some("2021".to_string()),
            &resolved,
            &requirements,
            Some("[[package]]\nname = \"metadata_probe\"\n".to_string()),
            &tmp.path().join("cargo-target"),
            &[],
        )?;
        assert_eq!(
            super::test_rust_inspect_workspace_generations(&out_dir),
            1,
            "expected one rust-inspect workspace generation"
        );

        let cargo_toml = fs::read_to_string(out_dir.join("Cargo.toml"))?;
        let cargo_lock = fs::read_to_string(out_dir.join("Cargo.lock"))?;
        let main_rs = fs::read_to_string(out_dir.join("src").join("main.rs"))?;

        assert!(
            cargo_toml.contains("[dependencies.datafusion_substrait]"),
            "expected rust-safe dependency key in generated rust-inspect workspace, got:\n{cargo_toml}"
        );
        assert!(
            cargo_toml.contains("package = \"datafusion-substrait\""),
            "expected original package name preserved in generated rust-inspect workspace, got:\n{cargo_toml}"
        );
        assert!(
            cargo_lock.contains("metadata_probe"),
            "expected rust-inspect workspace to write the provided Cargo.lock payload"
        );
        assert!(
            main_rs.contains("use datafusion_substrait as _;"),
            "expected rust-inspect workspace stub to reference the aliased dependency crate, got:\n{main_rs}"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn ensure_rust_inspect_workspace_skips_regeneration_when_unchanged() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let requirements = ProjectRequirements::default();
        let resolved = ResolvedDependencies {
            dependencies: vec![DependencySpec {
                crate_name: "serde".to_string(),
                version: Some("1".to_string()),
                features: Vec::new(),
                default_features: true,
                source: DependencySource::Registry,
                optional: false,
                package: None,
            }],
            dev_dependencies: Vec::new(),
        };
        let lock = Some("[[package]]\nname = \"skip_probe\"\n".to_string());

        let out_dir = ensure_rust_inspect_workspace(
            tmp.path(),
            "skip_probe",
            Some("2021".to_string()),
            &resolved,
            &requirements,
            lock.clone(),
            &tmp.path().join("cargo-target"),
            &[],
        )?;
        assert_eq!(
            super::test_rust_inspect_workspace_generations(&out_dir),
            1,
            "first call should generate the workspace"
        );

        ensure_rust_inspect_workspace(
            tmp.path(),
            "skip_probe",
            Some("2021".to_string()),
            &resolved,
            &requirements,
            lock,
            &tmp.path().join("cargo-target"),
            &[],
        )?;
        assert_eq!(
            super::test_rust_inspect_workspace_generations(&out_dir),
            1,
            "second call with identical inputs should skip regeneration"
        );

        Ok(())
    }

    #[test]
    fn typecheck_modules_with_import_graph_accepts_valid_program() -> Result<(), Box<dyn std::error::Error>> {
        let module = parsed_module_for_test(
            r#"
def main() -> None:
    pass
"#,
        )?;

        typecheck_modules_with_import_graph(
            &[module],
            None,
            &Arc::new(ProviderPlan::default()),
            #[cfg(feature = "rust_inspect")]
            None,
        )?;

        Ok(())
    }

    #[test]
    fn typecheck_modules_with_import_graph_reports_errors() -> Result<(), Box<dyn std::error::Error>> {
        let module = parsed_module_for_test(
            r#"
def main() -> None:
    missing_symbol()
"#,
        )?;

        let result = typecheck_modules_with_import_graph(
            &[module],
            None,
            &Arc::new(ProviderPlan::default()),
            #[cfg(feature = "rust_inspect")]
            None,
        );
        assert!(result.is_err(), "expected unresolved symbol to fail typecheck");

        Ok(())
    }

    #[test]
    fn compilation_session_projects_declared_package_features() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let source_root = tmp.path().join("src");
        std::fs::create_dir_all(&source_root)?;
        std::fs::write(
            tmp.path().join("incan.toml"),
            "[project]\nname = \"feature_projection\"\n\n[project.features]\ndefault = [\"json\"]\njson = []\n",
        )?;
        let source_path = source_root.join("main.incn");
        let source = "when feature(\"json\"):\n    const JSON_ENABLED = true\n\nconst ALWAYS = true\n";
        std::fs::write(&source_path, source)?;

        let default_session =
            CompilationSession::discover_with_feature_selection(&source_path, &FeatureSelection::default())?;
        let default_program = default_session
            .parse_source(&source_path, source, false)
            .map_err(|errors| std::io::Error::other(format!("default feature parse failed: {errors:?}")))?;
        assert_eq!(default_program.declarations.len(), 2);

        let selection = FeatureSelection {
            no_default_features: true,
            ..FeatureSelection::default()
        };
        let minimal_session = CompilationSession::discover_with_feature_selection(&source_path, &selection)?;
        let tooling_program = minimal_session
            .parse_source_unprojected(&source_path, source, false)
            .map_err(|errors| std::io::Error::other(format!("tooling feature parse failed: {errors:?}")))?;
        assert_eq!(
            tooling_program.declarations.len(),
            2,
            "tooling must retain inactive declarations before semantic projection"
        );
        let minimal_program = minimal_session
            .parse_source(&source_path, source, false)
            .map_err(|errors| std::io::Error::other(format!("minimal feature parse failed: {errors:?}")))?;
        assert_eq!(minimal_program.declarations.len(), 1);

        let unknown_source = "when feature(\"missing\"):\n    const VALUE = true\n";
        let errors = minimal_session
            .parse_source(&source_path, unknown_source, false)
            .err()
            .ok_or("unknown feature should fail source projection")?;
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("Unknown package feature `missing`"))
        );
        Ok(())
    }

    #[test]
    fn library_source_seeds_deduplicate_imports_across_noncanonical_source_roots_issue948()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let source_root = tmp.path().join("stdlib");
        let project_root = source_root.join("components/core");
        let entrypoint = project_root.join("src/lib.incn");
        let operations = source_root.join("traits/ops.incn");
        std::fs::create_dir_all(entrypoint.parent().ok_or("entrypoint must have a parent")?)?;
        std::fs::create_dir_all(operations.parent().ok_or("operations must have a parent")?)?;
        std::fs::write(
            project_root.join("incan.toml"),
            "[project]\nname = \"core\"\n\n[build]\nsource-root = \"../..\"\n",
        )?;
        std::fs::write(&entrypoint, "import traits.ops\n")?;
        std::fs::write(
            &operations,
            "pub def add(left: int, right: int) -> int:\n  return left + right\n",
        )?;

        let session = CompilationSession::discover_with_feature_selection(&entrypoint, &FeatureSelection::default())?;
        let modules = collect_library_modules_detailed_with_session(entrypoint, &session)
            .map_err(|failure| failure.render_human())?;
        let operations_modules = modules
            .iter()
            .filter(|module| module.path_segments == ["traits".to_string(), "ops".to_string()])
            .collect::<Vec<_>>();

        assert_eq!(
            operations_modules.len(),
            1,
            "all-source discovery and an authored import must share one canonical source identity"
        );
        assert_eq!(operations_modules[0].file_path, operations.canonicalize()?);
        Ok(())
    }

    #[test]
    fn library_source_seeds_exclude_the_unselected_root_entrypoint_issue948() -> Result<(), Box<dyn std::error::Error>>
    {
        let tmp = tempfile::tempdir()?;
        let source_root = tmp.path().join("src");
        std::fs::create_dir_all(&source_root)?;
        let lib = source_root.join("lib.incn");
        let main = source_root.join("main.incn");
        std::fs::write(&lib, "pub def exported() -> int:\n  return 1\n")?;
        std::fs::write(&main, "def main() -> None:\n  pass\n")?;

        assert!(!is_unselected_package_entrypoint(
            &source_root,
            &lib,
            &lib.canonicalize()?
        ));
        assert!(is_unselected_package_entrypoint(
            &source_root,
            &main,
            &lib.canonicalize()?
        ));
        Ok(())
    }

    #[test]
    fn artifact_only_dependency_rejects_a_stale_feature_projection() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let dependency_root = tmp.path().join("feature_library");
        let artifact_root = dependency_root.join("target/lib");
        fs::create_dir_all(artifact_root.join("src"))?;
        fs::write(
            artifact_root.join("Cargo.toml"),
            "[package]\nname = \"feature_library\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        fs::write(artifact_root.join("src/lib.rs"), "pub fn alpha() {}\n")?;
        let mut artifact = LibraryManifest::new("feature_library", "0.1.0");
        artifact.contract_metadata.provider.public_features = BTreeMap::from([
            ("alpha".to_string(), ProviderFeatureMetadata::default()),
            ("beta".to_string(), ProviderFeatureMetadata::default()),
        ]);
        artifact.contract_metadata.provider.active_features = BTreeSet::from(["alpha".to_string()]);
        artifact.write_to_path(&artifact_root.join("feature_library.incnlib"))?;

        let consumer_root = tmp.path().join("consumer");
        fs::create_dir_all(&consumer_root)?;
        fs::write(
            consumer_root.join(MANIFEST_FILENAME),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nfeature_library = { path = \"../feature_library\", features = [\"beta\"], default-features = false }\n",
        )?;
        let consumer = ProjectManifest::discover(&consumer_root)?.ok_or("missing consumer manifest")?;
        let error = PackageFeaturePlan::resolve(&consumer, &FeatureSelection::default())
            .err()
            .ok_or("stale artifact-only feature projection should fail")?;
        let message = error.to_string();

        assert!(message.contains("was built with package features [alpha]"));
        assert!(message.contains("requires [beta]"));
        Ok(())
    }

    #[test]
    fn compilation_session_analysis_bundles_lowering_inputs_with_semantic_facts()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let project_root = tmp.path();
        let source_root = project_root.join("src");
        std::fs::create_dir_all(&source_root)?;
        std::fs::write(
            project_root.join("incan.toml"),
            "[project]\nname = \"analysis_consumer\"\n",
        )?;
        let main_path = source_root.join("main.incn");
        std::fs::write(
            &main_path,
            "from std.testing import assert_eq\n\ndef helper() -> int:\n  return 1\n\ndef main() -> int:\n  assert_eq(helper(), 1)\n  return helper()\n",
        )?;

        let session = CompilationSession::discover_with_feature_selection(&main_path, &FeatureSelection::default())?;
        let modules = collect_modules_detailed_with_session(main_path.clone(), &session)
            .map_err(|failure| failure.render_human())?;
        let analysis = session
            .analyze_modules(
                &modules,
                #[cfg(feature = "rust_inspect")]
                None,
            )
            .map_err(|failure| failure.render_human())?;
        let snapshot = analysis
            .semantic_snapshots()
            .get(&main_path)
            .ok_or("expected a session semantic snapshot for the entry module")?;

        assert!(analysis.type_info_for_path(&main_path).is_some());
        assert!(snapshot.render_snapshot().contains("decl:main::helper type=() -> int"));
        assert!(
            snapshot
                .render_snapshot()
                .contains("symbol_target=function:main::helper")
        );
        let mut stdlib_cache = analysis.stdlib_cache().clone();
        assert!(
            stdlib_cache
                .lookup_function_symbol(&["std".to_string(), "testing".to_string()], "assert_eq")
                .is_some(),
            "session analysis must retain source-backed stdlib metadata for lowering"
        );
        Ok(())
    }

    #[test]
    fn compilation_session_analysis_preserves_same_file_module_identities() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let source_root = tmp.path().join("src");
        std::fs::create_dir_all(&source_root)?;
        std::fs::write(
            tmp.path().join("incan.toml"),
            "[project]\nname = \"identity_consumer\"\n",
        )?;
        let shared_path = source_root.join("shared.incn");
        std::fs::write(&shared_path, "def value() -> int:\n  return 1\n")?;

        let mut first = parsed_module_for_test("def first() -> int:\n  return 1\n")?;
        first.name = "first".to_string();
        first.path_segments = vec!["first".to_string()];
        first.file_path = shared_path.clone();
        let mut second = parsed_module_for_test("def second() -> int:\n  return 2\n")?;
        second.name = "second".to_string();
        second.path_segments = vec!["second".to_string()];
        second.file_path = shared_path.clone();

        let analysis = CompilationSession::discover_with_feature_selection(&shared_path, &FeatureSelection::default())?
            .analyze_modules(
                &[first, second],
                #[cfg(feature = "rust_inspect")]
                None,
            )
            .map_err(|failure| failure.render_human())?;

        assert!(analysis.type_info_for_module_path(&["first".to_string()]).is_some());
        assert!(analysis.type_info_for_module_path(&["second".to_string()]).is_some());
        Ok(())
    }
}
