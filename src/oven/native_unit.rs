//! Compiler-owned native-unit seeds for the bounded Oven Alpha envelope.
//!
//! A seed is an immutable direct-`rustc` closure shipped with the active Incan toolchain. It is deliberately
//! separate from a generated-project receipt: one seed can satisfy compatible clean worktrees, while each generated
//! source tree keeps its own receipt and final output. Normal commands may copy a verified seed into the bounded Oven
//! store, but never inspect a Cargo target or accept a project-selected native-artifact directory.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::legacy_cargo::{
    OvenLegacyCargoError, OvenLegacyCargoPrepareRequest, OvenLegacyCargoPublicationKind,
    direct_rustc_compile_environment, prepare_direct_rustc_plan,
};
use super::rustc::{
    OvenRegistryLeafAuthority, OvenRustcArtifactExtern, OvenRustcArtifactManifest, OvenRustcArtifactPlan,
    OvenRustcAuxiliaryTarget, OvenRustcError, OvenRustcRegistryLeaf, OvenRustcSupportingArtifact,
    clear_inherited_cargo_environment, validate_sealed_registry_leaf,
};
use super::store::{
    OvenArtifactKind, OvenArtifactMaterializedFile, OvenArtifactPublishRequest, OvenStore, OvenStoreError,
};
use super::{OvenReceipt, digest_bytes};
use crate::manifest::{DependencySource, DependencySpec};
use crate::version::{INCAN_VERSION, SDK_PROVIDER_CODEGEN_REVISION};

/// Current wire format for one compiler-shipped Oven native-unit seed.
pub const OVEN_NATIVE_UNIT_SEED_SCHEMA_VERSION: u32 = 10;
/// Internal marker enabled only while the named legacy publisher creates a compiler-owned native-unit seed.
///
/// This is deliberately distinct from normal Oven command selection: it grants compiler source emission the same
/// trusted standard-provider identity as the SDK publisher, but it never authorizes Cargo for a caller command.
pub(crate) const OVEN_NATIVE_UNIT_SEED_ENV: &str = "INCAN_OVEN_NATIVE_UNIT_SEED";
const TOOLCHAIN_SEED_RELATIVE_ROOT: &str = "share/incan/oven/native-units";
const BASE_UNIT_MAX_PHYSICAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const BASE_UNIT_MAX_DOMAIN_PHYSICAL_BYTES: u64 = 1024 * 1024 * 1024;
const BASE_UNIT_MAX_DOMAIN_LOGICAL_BYTES: u64 = 768 * 1024 * 1024;
static NATIVE_UNIT_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Owner-scoped staging directory that is removed unless a verified seed is atomically published from it.
struct NativeUnitTemporaryDirectory {
    path: PathBuf,
    keep: bool,
}

impl NativeUnitTemporaryDirectory {
    /// Create a unique owner-scoped native-unit staging directory below `parent`.
    fn create(parent: &Path, prefix: &str) -> io::Result<Self> {
        for _ in 0..128 {
            let sequence = NATIVE_UNIT_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!("{prefix}{}-{sequence}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path, keep: false }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "could not allocate unique Oven native-unit staging directory below {}",
                parent.display()
            ),
        ))
    }

    /// Return the staging directory path while this owner retains cleanup responsibility.
    fn path(&self) -> &Path {
        &self.path
    }

    /// Retain the staging directory after its caller has atomically published it.
    fn persist(mut self) -> PathBuf {
        self.keep = true;
        self.path.clone()
    }
}

impl Drop for NativeUnitTemporaryDirectory {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Immutable direct-`rustc` closure shipped with one compiler/toolchain distribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OvenNativeUnitSeed {
    /// Seed wire-schema version.
    pub schema_version: u32,
    /// Source-independent identity of the provider/dependency unit this seed can materialize.
    pub build_unit_identity: String,
    /// Explicit compiler-owned runtime capabilities that may authorize a narrower standard-provider request.
    ///
    /// This is deliberately more restrictive than a generic dependency solver: every runtime input other than
    /// provider selection stays exact. The provider-subset policy is callable only after normal-command routing has
    /// rejected every caller-owned external `rust::` import. Rust's own `rust::std` is compiler-supplied, while a
    /// selected standard provider may contribute its checked transitive Rust closure; only standard-provider modules
    /// and facets may otherwise be subsets of this seed.
    #[serde(default)]
    pub compatibility: OvenNativeUnitCompatibility,
    /// Exact registry package artifacts emitted by the named native-unit publisher.
    ///
    /// Normal consumers may select only these records; this is deliberately not a Cargo cache, package index, or
    /// source resolver.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry_leaves: Vec<OvenRustcRegistryLeaf>,
    /// Direct-`rustc` compiler input closure relative to the seed file's parent directory.
    pub plan: OvenRustcArtifactManifest,
}

/// A receipt-authorized compiler-native closure resolved from immutable toolchain data.
///
/// The compiler-suite scheduler copies this data from its leased store partition into read-only caller output before
/// spawning nested normal commands. Those commands must not copy the same large closure once per fixture into their
/// small mutable home: doing so would turn bounded store pruning into a selection/execution race. This value is
/// therefore available only through the scheduler's internal handoff, while ordinary commands continue to publish
/// the selected seed into their policy-bounded Oven store.
#[derive(Debug, Clone)]
pub struct OvenToolchainNativeUnit {
    /// Stable identity of the compiler-shipped seed selected for this receipt.
    pub seed_build_unit_identity: String,
    /// Receipt-compatible direct-Rustc manifest retained by the seed.
    pub artifacts: OvenRustcArtifactManifest,
    /// Exact registry leaves sealed with this native unit.
    pub registry_leaves: Vec<OvenRustcRegistryLeaf>,
    /// Immutable compiler-data directory containing the manifest's declared files.
    pub artifact_root: PathBuf,
    /// Trusted direct-Rustc invocation inputs resolved from that immutable compiler data.
    pub artifact_plan: OvenRustcArtifactPlan,
}

impl OvenToolchainNativeUnit {
    #[must_use]
    /// Expose this unit's registry leaves with only its verified transitive metadata directories.
    pub(crate) fn registry_leaf_authority(&self) -> OvenRegistryLeafAuthority {
        OvenRegistryLeafAuthority::new_with_trusted_dependency_search_paths(
            self.artifact_root.clone(),
            self.registry_leaves.clone(),
            self.artifact_plan.dependency_search_paths.clone(),
        )
    }
}

/// Explicit runtime capability envelope for a compiler-owned native-unit seed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OvenNativeUnitCompatibility {
    /// Every receipt build-unit input other than provider selection and its derived feature set.
    ///
    /// Values such as the runtime source digests and lockfile must compare exactly before a seed can satisfy another
    /// receipt. The resolved Rust-dependency digest remains in the receipt identity, but is intentionally excluded
    /// here: this policy runs only after caller-owned external Rust imports are refused. The standard-library feature
    /// digest is likewise represented by the selected provider modules and facets, which permits a verified provider
    /// superset to serve a narrower compiler-owned request without becoming a dependency resolver.
    pub runtime_inputs: BTreeMap<String, String>,
    /// Standard-provider modules, implementation facets, and direct rlib links compiled into the shipped closure.
    pub providers: Vec<OvenNativeUnitProviderCapability>,
}

/// One provider capability compiled into a native-unit seed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OvenNativeUnitProviderCapability {
    /// Stable provider identity supplied by the checked compiler provider plan.
    pub identity: String,
    /// Canonical standard-provider module paths covered by this closure.
    pub modules: Vec<String>,
    /// Exact implementation facets selected while the closure was published.
    pub facets: Vec<String>,
    /// Whether this provider's rlib is a required direct Rust link root even without a source-module import.
    #[serde(default)]
    pub direct_link: bool,
}

/// Authorization policy for compiler-shipped native-unit selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OvenNativeUnitSelection {
    /// Require the seed's complete build-unit identity to equal the caller receipt.
    Exact,
    /// Permit a compiler-owned standard-provider closure to satisfy a narrower request.
    ///
    /// Callers use this only after rejecting inline `rust::` imports. The relation still requires exact runtime
    /// inputs and seed-superset provider modules/facets; it is not a dependency resolver or a Cargo fallback.
    CompilerOwnedProviderSuperset,
}

/// How much compiler-owned provider capability a seed contributes beyond one requested receipt.
///
/// Multiple immutable seeds can safely authorize the same request. Prefer the narrowest one so adding a new
/// provider-family seed does not make an otherwise valid core request ambiguous. The values are derived only after
/// the exact runtime-input check, so this is a deterministic efficiency choice, never dependency resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct OvenNativeUnitProviderExcess {
    providers: usize,
    modules: usize,
    facets: usize,
    direct_links: usize,
}

/// A native-unit seed that has passed the narrow compiler-owned provider-subset authorization rule.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CompatibleNativeUnitSeed {
    path: PathBuf,
    excess: OvenNativeUnitProviderExcess,
}

impl OvenNativeUnitCompatibility {
    /// Derive the explicit, portable compatibility envelope from one verified generated-project receipt.
    fn from_receipt(receipt: &OvenReceipt) -> Result<Self, OvenNativeUnitError> {
        let mut runtime_inputs = receipt.sources.build_unit_inputs.clone();
        let provider_records = runtime_inputs.remove("providers").unwrap_or_default();
        let _ = runtime_inputs.remove("rust-dependencies");
        let _ = runtime_inputs.remove("stdlib-features");
        let provider_plan = runtime_inputs
            .remove("provider-plan")
            .ok_or_else(|| OvenNativeUnitError::Preparation {
                message: "native-unit receipt is missing its provider-plan input".to_string(),
            })?;
        let expected_provider_plan = digest_bytes(provider_records.as_bytes());
        if provider_plan != expected_provider_plan {
            return Err(OvenNativeUnitError::Preparation {
                message: "native-unit receipt provider-plan digest does not match its provider records".to_string(),
            });
        }
        let providers = parse_provider_capabilities(&provider_records)?;
        Ok(Self {
            runtime_inputs,
            providers,
        })
    }

    /// Return the extra capability retained by this compatible seed, or `None` when it cannot serve `receipt`.
    fn provider_subset_excess(
        &self,
        receipt: &OvenReceipt,
    ) -> Result<Option<OvenNativeUnitProviderExcess>, OvenNativeUnitError> {
        let requested = Self::from_receipt(receipt)?;
        if self.runtime_inputs != requested.runtime_inputs {
            return Ok(None);
        }
        let mut available = BTreeMap::new();
        for provider in &self.providers {
            if available.insert(provider.identity.as_str(), provider).is_some() {
                return Err(OvenNativeUnitError::InvalidSeed {
                    path: PathBuf::from("seed compatibility"),
                    message: format!("declares provider `{}` more than once", provider.identity),
                });
            }
        }
        let requested_by_identity = requested
            .providers
            .iter()
            .map(|provider| (provider.identity.as_str(), provider))
            .collect::<BTreeMap<_, _>>();
        for required in &requested.providers {
            let Some(candidate) = available.get(required.identity.as_str()) else {
                return Ok(None);
            };
            if !required
                .modules
                .iter()
                .all(|module| candidate.modules.binary_search(module).is_ok())
                || !required
                    .facets
                    .iter()
                    .all(|facet| candidate.facets.binary_search(facet).is_ok())
                || (required.direct_link && !candidate.direct_link)
            {
                return Ok(None);
            }
        }
        let mut excess = OvenNativeUnitProviderExcess {
            providers: 0,
            modules: 0,
            facets: 0,
            direct_links: 0,
        };
        for candidate in &self.providers {
            let Some(required) = requested_by_identity.get(candidate.identity.as_str()) else {
                excess.providers += 1;
                excess.modules += candidate.modules.len();
                excess.facets += candidate.facets.len();
                excess.direct_links += usize::from(candidate.direct_link);
                continue;
            };
            excess.modules += candidate
                .modules
                .iter()
                .filter(|module| required.modules.binary_search(module).is_err())
                .count();
            excess.facets += candidate
                .facets
                .iter()
                .filter(|facet| required.facets.binary_search(facet).is_err())
                .count();
            excess.direct_links += usize::from(candidate.direct_link && !required.direct_link);
        }
        Ok(Some(excess))
    }

    /// Return whether this shipped runtime closure can safely satisfy `receipt` under the narrow provider-subset rule.
    fn authorizes_provider_subset(&self, receipt: &OvenReceipt) -> Result<bool, OvenNativeUnitError> {
        Ok(self.provider_subset_excess(receipt)?.is_some())
    }
}

/// Parse the canonical provider-capability records sealed into a native-unit receipt.
fn parse_provider_capabilities(records: &str) -> Result<Vec<OvenNativeUnitProviderCapability>, OvenNativeUnitError> {
    let mut providers = Vec::new();
    for record in records.lines().filter(|record| !record.is_empty()) {
        let mut parts = record.split('|');
        let identity = parts.next().unwrap_or_default().trim();
        let modules = parts.next().unwrap_or_default();
        let facets = parts.next().unwrap_or_default();
        let direct_link = match parts.next() {
            None => false,
            Some("none") => false,
            Some("link") => true,
            Some(_) => {
                return Err(OvenNativeUnitError::Preparation {
                    message: format!("native-unit provider record is not canonical: {record}"),
                });
            }
        };
        if identity.is_empty() || parts.next().is_some() {
            return Err(OvenNativeUnitError::Preparation {
                message: format!("native-unit provider record is not canonical: {record}"),
            });
        }
        let mut modules = modules
            .split(',')
            .filter(|module| !module.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let mut facets = facets
            .split(',')
            .filter(|facet| !facet.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        modules.sort();
        modules.dedup();
        facets.sort();
        facets.dedup();
        providers.push(OvenNativeUnitProviderCapability {
            identity: identity.to_string(),
            modules,
            facets,
            direct_link,
        });
    }
    providers.sort_by(|left, right| left.identity.cmp(&right.identity));
    if providers.windows(2).any(|pair| pair[0].identity == pair[1].identity) {
        return Err(OvenNativeUnitError::Preparation {
            message: "native-unit provider records repeat one provider identity".to_string(),
        });
    }
    Ok(providers)
}

/// Result from a release-stage base-runtime seed preparation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OvenNativeUnitSeedPreparation {
    /// Reusable native compatibility identity represented by the seed.
    pub build_unit_identity: String,
    /// The direct-rustc plan identity in the temporary publisher store.
    pub plan_identity: String,
    /// Logical bytes in the final compiler-shipped seed directory, including its verified plan.
    pub logical_bytes: u64,
    /// Measured allocation in the final compiler-shipped seed directory.
    pub physical_bytes: u64,
}

/// Construct the portable runtime portion of a normal generated project's native build-unit identity.
///
/// The caller contributes normalized provider records, selected stdlib features, and the digest of resolved Rust
/// dependencies. Compiler-owned sources and the lockfile are resolved from the active toolchain layout so a packaged
/// compiler never depends on the checkout from which its binary happened to be built.
pub fn runtime_build_unit_inputs(
    provider_records: Vec<String>,
    stdlib_features: &[String],
    rust_dependencies_digest: String,
) -> Result<BTreeMap<String, String>, String> {
    let mut inputs = BTreeMap::new();
    inputs.insert("compiler-version".to_string(), INCAN_VERSION.to_string());
    inputs.insert(
        "sdk-provider-codegen-revision".to_string(),
        SDK_PROVIDER_CODEGEN_REVISION.to_string(),
    );
    for (name, crate_name) in [
        ("runtime-source-incan-core", "incan_core"),
        ("runtime-source-incan-derive", "incan_derive"),
        ("runtime-source-incan-stdlib", "incan_stdlib"),
    ] {
        let path = crate::toolchain_layout::resolve_toolchain_crate_path(crate_name);
        let digest = digest_runtime_crate_source(&path)?;
        inputs.insert(name.to_string(), digest);
    }
    let lock_path = crate::toolchain_layout::resolve_toolchain_runtime_lockfile();
    let lock = fs::read(&lock_path)
        .map_err(|error| format!("failed to read Oven runtime lock {}: {error}", lock_path.display()))?;
    inputs.insert("runtime-lock".to_string(), digest_bytes(&lock));
    inputs.insert(
        "provider-plan".to_string(),
        digest_bytes(provider_records.join("\n").as_bytes()),
    );
    if !provider_records.is_empty() {
        inputs.insert("providers".to_string(), provider_records.join("\n"));
    }
    inputs.insert(
        "stdlib-features".to_string(),
        digest_bytes(stdlib_features.join(",").as_bytes()),
    );
    inputs.insert("rust-dependencies".to_string(), rust_dependencies_digest);
    Ok(inputs)
}

/// Digest exactly the compiler runtime source closure retained by a suite publisher.
///
/// Runtime compatibility is determined by the package manifest and Rust sources that a generated provider can
/// compile against. Test fixtures, documentation, and nested build output are not runtime inputs and the suite
/// publisher deliberately does not retain them. Hashing the whole checkout crate here would make a native seed
/// incompatible with the publisher's smaller immutable closure even when the compiled runtime is identical.
pub(crate) fn digest_runtime_crate_source(root: &Path) -> Result<String, String> {
    let root_metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("failed to read runtime crate root {}: {error}", root.display()))?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(format!(
            "runtime crate root {} must be a directory without symlink indirection",
            root.display()
        ));
    }
    let manifest = root.join("Cargo.toml");
    let manifest_metadata = fs::symlink_metadata(&manifest)
        .map_err(|error| format!("failed to read runtime manifest {}: {error}", manifest.display()))?;
    if !manifest_metadata.is_file() || manifest_metadata.file_type().is_symlink() {
        return Err(format!(
            "runtime manifest {} must be a regular file without symlink indirection",
            manifest.display()
        ));
    }
    let source_root = root.join("src");
    let source_metadata = fs::symlink_metadata(&source_root).map_err(|error| {
        format!(
            "failed to read runtime source directory {}: {error}",
            source_root.display()
        )
    })?;
    if !source_metadata.is_dir() || source_metadata.file_type().is_symlink() {
        return Err(format!(
            "runtime source directory {} must be a directory without symlink indirection",
            source_root.display()
        ));
    }

    let mut records = BTreeMap::new();
    records.insert(
        "Cargo.toml".to_string(),
        digest_bytes(
            &fs::read(&manifest)
                .map_err(|error| format!("failed to read runtime manifest {}: {error}", manifest.display()))?,
        ),
    );
    collect_runtime_source_records(&source_root, &source_root, &mut records)?;
    serde_json::to_vec(&records)
        .map(|payload| digest_bytes(&payload))
        .map_err(|error| {
            format!(
                "failed to serialize runtime source digest for {}: {error}",
                root.display()
            )
        })
}

/// Add the regular files below one runtime crate's `src/` tree to its portable source digest.
fn collect_runtime_source_records(
    source_root: &Path,
    current: &Path,
    records: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(current)
        .map_err(|error| format!("failed to read runtime source directory {}: {error}", current.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to read runtime source directory {}: {error}", current.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect runtime source {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("runtime source {} must not contain symlinks", path.display()));
        }
        if metadata.is_dir() {
            collect_runtime_source_records(source_root, &path, records)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(format!(
                "runtime source {} must contain only regular files",
                path.display()
            ));
        }
        let relative = path
            .strip_prefix(source_root)
            .map_err(|_| format!("runtime source {} escaped {}", path.display(), source_root.display()))?
            .to_string_lossy()
            .replace('\\', "/");
        let key = format!("src/{relative}");
        let digest = digest_bytes(
            &fs::read(&path).map_err(|error| format!("failed to read runtime source {}: {error}", path.display()))?,
        );
        if records.insert(key.clone(), digest).is_some() {
            return Err(format!("runtime source contains duplicate portable path {key}"));
        }
    }
    Ok(())
}

/// Export one compiler-owned native-unit seed from an already receipted generated Incan project.
///
/// Release packaging first drives the compiler's ordinary Oven analysis for a small in-package Incan program. That
/// produces the same provider, SDK, feature, dependency, target, and toolchain identity that an everyday command
/// would use. This explicit publisher then converts only that exact generated project into an immutable direct-rustc
/// seed; its temporary store is dropped before the release archive is created.
pub fn prepare_native_unit_seed_from_generated_project(
    seed_root: &Path,
    cargo: &Path,
    rustc: &Path,
    receipt: OvenReceipt,
    generated_project: &Path,
) -> Result<OvenNativeUnitSeedPreparation, OvenNativeUnitError> {
    if seed_root.exists() && !seed_root.is_dir() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!("seed root is not a directory: {}", seed_root.display()),
        });
    }
    fs::create_dir_all(seed_root).map_err(|source| OvenNativeUnitError::Io {
        path: seed_root.to_path_buf(),
        source,
    })?;
    let store_root =
        NativeUnitTemporaryDirectory::create(seed_root, ".incan-oven-base-unit-store-").map_err(|source| {
            OvenNativeUnitError::Io {
                path: seed_root.to_path_buf(),
                source,
            }
        })?;
    let store = OvenStore::new(
        store_root.path(),
        super::store::OvenStoreLimits::new(
            BASE_UNIT_MAX_PHYSICAL_BYTES,
            BASE_UNIT_MAX_DOMAIN_PHYSICAL_BYTES,
            BASE_UNIT_MAX_DOMAIN_LOGICAL_BYTES,
        ),
    );
    let generated_source = generated_project.join("src/main.rs");
    let compile_environment = direct_rustc_compile_environment(generated_project, &generated_source)?;
    let publication = prepare_direct_rustc_plan(&OvenLegacyCargoPrepareRequest {
        store: &store,
        receipt: receipt.clone(),
        generated_project: generated_project.to_path_buf(),
        cargo: cargo.to_path_buf(),
        rustc: rustc.to_path_buf(),
        domain: format!("toolchain-base-{}", receipt.intent.profile),
        publication_kind: OvenLegacyCargoPublicationKind::Executable,
        source_evidence_key: "generated-root".to_string(),
        compile_environment,
        compact_debug_info: true,
    })?;
    let output_directory = seed_root.join(
        receipt
            .build_unit_identity
            .strip_prefix("sha256:")
            .unwrap_or(receipt.build_unit_identity.as_str()),
    );
    if output_directory.exists() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!("seed destination already exists: {}", output_directory.display()),
        });
    }
    export_native_unit_seed(
        &store,
        &publication.plan_identity,
        &receipt,
        cargo,
        rustc,
        publication.registry_leaves,
        &output_directory,
    )
}

/// Native-unit seed loading, validation, or store-publication failure.
#[derive(Debug, Error)]
pub enum OvenNativeUnitError {
    /// A compiler-owned seed file could not be read.
    #[error("failed to read Oven native-unit seed {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    /// A seed payload is malformed or belongs to an unsupported schema.
    #[error("invalid Oven native-unit seed {path}: {message}")]
    InvalidSeed { path: PathBuf, message: String },
    /// The declared closure is not a valid direct-rustc artifact plan.
    #[error(transparent)]
    Plan(#[from] OvenRustcError),
    /// Bounded publication refused the requested immutable closure.
    #[error(transparent)]
    Store(#[from] OvenStoreError),
    /// The release-stage publisher could not prepare its temporary direct-rustc closure.
    #[error(transparent)]
    Publisher(#[from] OvenLegacyCargoError),
    /// A release-stage native-unit seed could not be assembled safely.
    #[error("failed to prepare Oven native-unit seed: {message}")]
    Preparation { message: String },
}

/// Copy a fully verified temporary store entry into the compiler-owned seed layout and report its accounting.
fn export_native_unit_seed(
    store: &OvenStore,
    plan_identity: &str,
    receipt: &OvenReceipt,
    cargo: &Path,
    rustc: &Path,
    registry_leaves: Vec<OvenRustcRegistryLeaf>,
    output_directory: &Path,
) -> Result<OvenNativeUnitSeedPreparation, OvenNativeUnitError> {
    let inspection = store.inspect()?;
    let entry = inspection
        .entries
        .iter()
        .find(|entry| entry.manifest.identity == plan_identity)
        .ok_or_else(|| OvenNativeUnitError::Preparation {
            message: format!("temporary native-unit plan {plan_identity} is absent after publication"),
        })?;
    if entry.manifest.kind != OvenArtifactKind::DirectRustcPlan
        || entry.manifest.build_unit_identity != receipt.build_unit_identity
        || entry.manifest.intent != receipt.intent
    {
        return Err(OvenNativeUnitError::Preparation {
            message: "temporary native-unit plan does not match its base runtime receipt".to_string(),
        });
    }
    let (_manifest, artifact_root, payload, _lease) = store.select_payload_for_execution(plan_identity)?;
    let mut plan = serde_json::from_slice::<OvenRustcArtifactManifest>(&payload).map_err(|error| {
        OvenNativeUnitError::Preparation {
            message: format!("temporary native-unit payload is not a direct-rustc plan: {error}"),
        }
    })?;
    discard_seed_metadata_sidecars(&mut plan);
    record_generated_root_externs(&mut plan);
    promote_compiler_runtime_externs(&mut plan)?;
    plan.registry_leaves = registry_leaves.clone();
    let materialized_files = plan.materialized_artifacts(&artifact_root, &receipt.intent)?;
    let parent = output_directory
        .parent()
        .ok_or_else(|| OvenNativeUnitError::Preparation {
            message: format!("seed destination has no parent: {}", output_directory.display()),
        })?;
    let staging = NativeUnitTemporaryDirectory::create(parent, ".incan-oven-native-seed-").map_err(|source| {
        OvenNativeUnitError::Io {
            path: parent.to_path_buf(),
            source,
        }
    })?;
    for file in materialized_files {
        let destination = staging.path().join(&file.relative_path);
        let destination_parent = destination.parent().ok_or_else(|| OvenNativeUnitError::Preparation {
            message: format!("seed artifact has no parent: {}", file.relative_path),
        })?;
        fs::create_dir_all(destination_parent).map_err(|source_error| OvenNativeUnitError::Io {
            path: destination_parent.to_path_buf(),
            source: source_error,
        })?;
        fs::copy(&file.source_path, &destination).map_err(|source_error| OvenNativeUnitError::Io {
            path: file.source_path,
            source: source_error,
        })?;
    }
    bake_compiler_vocab_support(&mut plan, staging.path(), cargo, rustc)?;
    let seed = OvenNativeUnitSeed {
        schema_version: OVEN_NATIVE_UNIT_SEED_SCHEMA_VERSION,
        build_unit_identity: receipt.build_unit_identity.clone(),
        compatibility: OvenNativeUnitCompatibility::from_receipt(receipt)?,
        registry_leaves,
        plan,
    };
    let seed_path = staging.path().join("seed.json");
    fs::write(
        &seed_path,
        serde_json::to_vec_pretty(&seed).map_err(|error| OvenNativeUnitError::Preparation {
            message: format!("could not encode native-unit seed: {error}"),
        })?,
    )
    .map_err(|source| OvenNativeUnitError::Io {
        path: seed_path,
        source,
    })?;
    fs::rename(staging.path(), output_directory).map_err(|source| OvenNativeUnitError::Io {
        path: output_directory.to_path_buf(),
        source,
    })?;
    let _ = staging.persist();
    let (logical_bytes, physical_bytes) = seed_directory_byte_counts(output_directory)?;
    Ok(OvenNativeUnitSeedPreparation {
        build_unit_identity: receipt.build_unit_identity.clone(),
        plan_identity: plan_identity.to_string(),
        logical_bytes,
        physical_bytes,
    })
}

/// Preserve the publisher-selected generated-root dependency set before the seed adds compiler-only helpers.
///
/// The ordinary native unit is published from a minimal generated program, so its original direct externs are the
/// roots that generated caller code may receive. Later preparation adds compiler runtime and vocabulary capabilities
/// to the same immutable closure. Runtime roots are promoted into every declared entrypoint below, but the vocabulary
/// helper roots must remain private to vocabulary extraction: passing their independently built `serde` closure to a
/// generated library would make Rustc see two incompatible `serde` identities.
fn record_generated_root_externs(plan: &mut OvenRustcArtifactManifest) {
    plan.entrypoint_externs
        .entry("generated-root".to_string())
        .or_insert_with(|| {
            let mut crate_names = plan
                .externs
                .iter()
                .map(|artifact| artifact.crate_name.clone())
                .collect::<Vec<_>>();
            crate_names.sort();
            crate_names.dedup();
            crate_names
        });
}

/// Promote compiler runtime artifacts required by generated provider libraries to direct externs.
///
/// The minimal seed program need not use models or provider metadata, while generated caller-owned libraries do.
/// `incan_derive` and `incan_core` are therefore promoted from the verified support closure. Leaving either only on
/// `-L dependency` relies on Cargo's implicit extern selection and makes a normal direct-Rustc consumer recompile
/// compiler source instead of linking the selected immutable plan.
fn promote_compiler_runtime_externs(plan: &mut OvenRustcArtifactManifest) -> Result<(), OvenNativeUnitError> {
    promote_compiler_runtime_extern(plan, "incan_derive", is_incan_derive_artifact)?;
    promote_compiler_runtime_extern(plan, "incan_core", |relative_path| {
        is_named_rlib(relative_path, "incan_core")
    })
}

/// Promote one exact compiler-owned support artifact after confirming the seed is unambiguous.
fn promote_compiler_runtime_extern(
    plan: &mut OvenRustcArtifactManifest,
    crate_name: &str,
    matches_artifact: impl Fn(&str) -> bool,
) -> Result<(), OvenNativeUnitError> {
    if plan.externs.iter().any(|artifact| artifact.crate_name == crate_name) {
        return Ok(());
    }
    let candidates = plan
        .supporting_artifacts
        .iter()
        .enumerate()
        .filter(|(_, artifact)| matches_artifact(&artifact.relative_path))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let [index] = candidates.as_slice() else {
        return Err(OvenNativeUnitError::Preparation {
            message: format!(
                "native foundation must declare exactly one compiler `{crate_name}` direct-Rustc artifact; found {}",
                candidates.len()
            ),
        });
    };
    let artifact = plan.supporting_artifacts.remove(*index);
    plan.externs.push(OvenRustcArtifactExtern {
        crate_name: crate_name.to_string(),
        relative_path: artifact.relative_path,
        digest: artifact.digest,
    });
    plan.externs
        .sort_by(|left, right| left.crate_name.cmp(&right.crate_name));
    for crate_names in plan.entrypoint_externs.values_mut() {
        if !crate_names
            .iter()
            .any(|entrypoint_extern| entrypoint_extern == crate_name)
        {
            crate_names.push(crate_name.to_string());
            crate_names.sort();
        }
    }
    Ok(())
}

/// Build and seal the compiler-owned vocab registration closure into a native seed.
///
/// A generated Incan program need not use JSON, while the compiler's vocab contract always serializes metadata.
/// Consequently, this compiler-owned closure cannot be inferred from a caller program's provider features. The
/// explicit `legacy_cargo` publisher builds only `incan_vocab` against the repository lockfile, copies its small
/// target-specific Rust closure into the immutable seed, and records the two helper roots explicitly. Vocabulary
/// extraction receives those roots from the selected full plan; generated roots do not, because their entrypoint
/// projection is fixed before this compiler-only closure is added. No normal command can re-run this Cargo operation.
fn bake_compiler_vocab_support(
    plan: &mut OvenRustcArtifactManifest,
    seed_staging: &Path,
    cargo: &Path,
    rustc: &Path,
) -> Result<(), OvenNativeUnitError> {
    const INCAN_VOCAB: &str = "incan_vocab";
    const SERDE_JSON: &str = "serde_json";
    const VOCAB_DESUGARER_TARGET: &str = "wasm32-wasip1";
    let required_externs = [INCAN_VOCAB, SERDE_JSON];
    if plan
        .externs
        .iter()
        .any(|artifact| required_externs.contains(&artifact.crate_name.as_str()))
    {
        return Err(OvenNativeUnitError::Preparation {
            message: "native foundation unexpectedly declares a compiler vocab support extern".to_string(),
        });
    }
    if !cargo.is_file() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!("native vocabulary publisher Cargo is not a file: {}", cargo.display()),
        });
    }
    if !rustc.is_file() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!(
                "native vocabulary publisher Rust compiler is not a file: {}",
                rustc.display()
            ),
        });
    }
    let source_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let crate_root = source_root.join("crates/incan_vocab");
    let manifest = crate_root.join("Cargo.toml");
    if !manifest.is_file() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!(
                "native vocabulary publisher manifest is unavailable: {}",
                manifest.display()
            ),
        });
    }

    let support_root = seed_staging.join("compiler-support");
    let cargo_target = support_root.join("target");
    let mut command = Command::new(cargo);
    command
        .current_dir(&crate_root)
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--message-format=json-render-diagnostics")
        .arg("--target")
        .arg(&plan.intent.target)
        .arg("--target-dir")
        .arg(&cargo_target)
        .arg("--locked")
        .arg("--offline");
    if plan.intent.profile == "release" {
        command.arg("--release");
    }
    clear_inherited_cargo_environment(&mut command);
    command.env("RUSTC", rustc).env("CARGO_NET_OFFLINE", "true");
    if plan.intent.profile == "debug" {
        command.env("CARGO_PROFILE_DEV_DEBUG", "0");
    }
    let compile = command.output().map_err(|source| OvenNativeUnitError::Io {
        path: cargo.to_path_buf(),
        source,
    })?;
    if !compile.status.success() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!(
                "native vocabulary support publisher failed:\n{}",
                String::from_utf8_lossy(&compile.stderr).trim()
            ),
        });
    }

    // Vocabulary companions may ship a Wasm desugarer. Build its compiler-owned dependency closure here at the
    // explicit publisher boundary; a normal `incan build --lib` later invokes only direct Rustc against the copied,
    // digest-verified files. Keep this as a separate target directory so host artifacts can never be selected for
    // a Wasm command by accident.
    let mut wasm_command = Command::new(cargo);
    wasm_command
        .current_dir(&crate_root)
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--message-format=json-render-diagnostics")
        .arg("--target")
        .arg(VOCAB_DESUGARER_TARGET)
        .arg("--target-dir")
        .arg(&cargo_target)
        .arg("--locked")
        .arg("--offline");
    if plan.intent.profile == "release" {
        wasm_command.arg("--release");
    }
    clear_inherited_cargo_environment(&mut wasm_command);
    wasm_command.env("RUSTC", rustc).env("CARGO_NET_OFFLINE", "true");
    if plan.intent.profile == "debug" {
        wasm_command.env("CARGO_PROFILE_DEV_DEBUG", "0");
    }
    let wasm_compile = wasm_command.output().map_err(|source| OvenNativeUnitError::Io {
        path: cargo.to_path_buf(),
        source,
    })?;
    if !wasm_compile.status.success() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!(
                "native vocabulary Wasm support publisher failed:\n{}",
                String::from_utf8_lossy(&wasm_compile.stderr).trim()
            ),
        });
    }

    let profile = if plan.intent.profile == "release" {
        "release"
    } else {
        "debug"
    };
    let artifact_directory = cargo_target.join(&plan.intent.target).join(profile).join("deps");
    // Cargo places procedural macros for the host compiler under the profile-only directory even when the native
    // build target equals the host. The target closure alone therefore cannot link a crate whose metadata names a
    // derive macro such as `serde_derive`.
    let host_artifact_directory = cargo_target.join(profile).join("deps");
    let seed_directory = support_root.join("deps");
    let host_artifacts = compiler_artifact_paths_from_cargo_output(
        &compile.stdout,
        &cargo_target,
        &[&artifact_directory, &host_artifact_directory],
        INCAN_VOCAB,
        &cargo_target.join(&plan.intent.target).join(profile),
        "native vocabulary support",
    )?;
    copy_compiler_vocab_support_artifacts(
        &host_artifacts,
        &artifact_directory,
        &cargo_target.join(&plan.intent.target).join(profile),
        &host_artifact_directory,
        &seed_directory,
        plan,
    )?;
    let wasm_artifact_directory = cargo_target.join(VOCAB_DESUGARER_TARGET).join(profile).join("deps");
    let wasm_artifact_directory_canonical =
        fs::canonicalize(&wasm_artifact_directory).map_err(|source| OvenNativeUnitError::Io {
            path: wasm_artifact_directory.clone(),
            source,
        })?;
    let wasm_primary_artifact_directory = cargo_target.join(VOCAB_DESUGARER_TARGET).join(profile);
    let wasm_primary_artifact_directory_canonical =
        fs::canonicalize(&wasm_primary_artifact_directory).map_err(|source| OvenNativeUnitError::Io {
            path: wasm_primary_artifact_directory.clone(),
            source,
        })?;
    let wasm_artifacts = compiler_artifact_paths_from_cargo_output(
        &wasm_compile.stdout,
        &cargo_target,
        &[&wasm_artifact_directory, &host_artifact_directory],
        INCAN_VOCAB,
        &wasm_primary_artifact_directory,
        "native vocabulary Wasm support",
    )?
    .into_iter()
    .filter(|artifact| {
        artifact.starts_with(&wasm_artifact_directory_canonical)
            || artifact.parent() == Some(wasm_primary_artifact_directory_canonical.as_path())
    })
    .collect::<Vec<_>>();
    copy_compiler_vocab_auxiliary_target_artifacts(
        &wasm_artifacts,
        &support_root.join(VOCAB_DESUGARER_TARGET).join("deps"),
        VOCAB_DESUGARER_TARGET,
        plan,
    )?;
    discard_compiler_vocab_build_state(&cargo_target)?;
    plan.externs
        .sort_by(|left, right| left.crate_name.cmp(&right.crate_name));
    plan.validate_shape(&plan.intent)?;
    Ok(())
}

/// Remove Cargo-private publisher state after the direct-Rustc closure has been copied into the seed.
///
/// `compiler-support/deps` is the complete declared closure. The adjacent Cargo target tree contains incremental
/// objects, fingerprint records, and metadata sidecars that are neither selected by the sealed plan nor safe to ship
/// as immutable Oven input. Keeping it would inflate every profile seed and can make a policy-compliant suite appear
/// oversized without adding any direct-Rustc capability.
fn discard_compiler_vocab_build_state(cargo_target: &Path) -> Result<(), OvenNativeUnitError> {
    fs::remove_dir_all(cargo_target).map_err(|source| OvenNativeUnitError::Io {
        path: cargo_target.to_path_buf(),
        source,
    })
}

/// Copy the sealed `incan_vocab` direct-Rustc support set produced by the compiler-owned package build.
///
/// The named publisher starts from an empty target directory, builds only `incan_vocab` against the checked lockfile,
/// and retains only the exact Rust-library paths in Cargo's `compiler-artifact` records for that invocation. The two
/// roots selected by vocabulary extraction (`incan_vocab` and `serde_json`) are explicit externs; the remaining
/// digested artifacts are their declared direct-Rustc support closure, including host procedural macros. A stale or
/// unrelated Cargo `deps` file is neither scanned nor admitted. The normal guarded library-vocab regression exercises
/// that sealed set and fails if a consumer attempts to launch Cargo.
fn copy_compiler_vocab_support_artifacts(
    source_artifacts: &[PathBuf],
    target_artifact_directory: &Path,
    primary_artifact_directory: &Path,
    host_artifact_directory: &Path,
    seed_directory: &Path,
    plan: &mut OvenRustcArtifactManifest,
) -> Result<(), OvenNativeUnitError> {
    const INCAN_VOCAB: &str = "incan_vocab";
    const SERDE_JSON: &str = "serde_json";
    if !target_artifact_directory.is_dir() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!(
                "native vocabulary publisher produced no target dependency directory: {}",
                target_artifact_directory.display()
            ),
        });
    }
    if !primary_artifact_directory.is_dir() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!(
                "native vocabulary publisher produced no primary artifact directory: {}",
                primary_artifact_directory.display()
            ),
        });
    }
    if !host_artifact_directory.is_dir() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!(
                "native vocabulary publisher produced no host dependency directory: {}",
                host_artifact_directory.display()
            ),
        });
    }
    fs::create_dir_all(seed_directory).map_err(|source| OvenNativeUnitError::Io {
        path: seed_directory.to_path_buf(),
        source,
    })?;
    let target_artifact_directory =
        fs::canonicalize(target_artifact_directory).map_err(|source| OvenNativeUnitError::Io {
            path: target_artifact_directory.to_path_buf(),
            source,
        })?;
    let primary_artifact_directory =
        fs::canonicalize(primary_artifact_directory).map_err(|source| OvenNativeUnitError::Io {
            path: primary_artifact_directory.to_path_buf(),
            source,
        })?;
    let host_artifact_directory =
        fs::canonicalize(host_artifact_directory).map_err(|source| OvenNativeUnitError::Io {
            path: host_artifact_directory.to_path_buf(),
            source,
        })?;
    let mut copied = BTreeMap::new();
    let mut target_copied = BTreeMap::new();
    for source in source_artifacts {
        let target_artifacts = source.starts_with(&target_artifact_directory)
            || source.parent() == Some(primary_artifact_directory.as_path());
        if !target_artifacts && !source.starts_with(&host_artifact_directory) {
            return Err(OvenNativeUnitError::Preparation {
                message: format!(
                    "native vocabulary compiler-artifact escaped its declared target or host dependency directory: {}",
                    source.display()
                ),
            });
        }
        let metadata = fs::symlink_metadata(source).map_err(|source_error| OvenNativeUnitError::Io {
            path: source.clone(),
            source: source_error,
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || !is_rust_library_artifact(source) {
            return Err(OvenNativeUnitError::Preparation {
                message: format!(
                    "native vocabulary compiler-artifact is not a regular direct-Rustc library: {}",
                    source.display()
                ),
            });
        }
        let file_name = source
            .file_name()
            .ok_or_else(|| OvenNativeUnitError::Preparation {
                message: format!("native vocabulary artifact has no filename: {}", source.display()),
            })?
            .to_string_lossy()
            .to_string();
        let bytes = fs::read(source).map_err(|source_error| OvenNativeUnitError::Io {
            path: source.clone(),
            source: source_error,
        })?;
        let digest = digest_bytes(&bytes);
        if let Some(existing) = copied.insert(file_name.clone(), digest.clone())
            && existing != digest
        {
            return Err(OvenNativeUnitError::Preparation {
                message: format!("native vocabulary target and host closures conflict on artifact `{file_name}`"),
            });
        }
        if target_artifacts {
            target_copied.insert(file_name.clone(), digest.clone());
        }
        let destination = seed_directory.join(&file_name);
        fs::write(&destination, bytes).map_err(|source_error| OvenNativeUnitError::Io {
            path: destination,
            source: source_error,
        })?;
    }
    if copied.is_empty() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!(
                "native vocabulary publisher retained no Rust artifacts from {}",
                target_artifact_directory.display()
            ),
        });
    }

    let relative_directory = "compiler-support/deps".to_string();
    plan.dependency_search_paths.push(relative_directory.clone());
    plan.dependency_search_paths.sort();
    plan.dependency_search_paths.dedup();
    for (file_name, digest) in &copied {
        let relative_path = format!("{relative_directory}/{file_name}");
        plan.supporting_artifacts.push(OvenRustcSupportingArtifact {
            relative_path: relative_path.clone(),
            digest: digest.clone(),
        });
    }
    for crate_name in [INCAN_VOCAB, SERDE_JSON] {
        let matches = target_copied
            .iter()
            .filter(|(file_name, _)| is_named_rlib(file_name, crate_name))
            .collect::<Vec<_>>();
        let [(file_name, digest)] = matches.as_slice() else {
            return Err(OvenNativeUnitError::Preparation {
                message: format!(
                    "native vocabulary publisher must retain exactly one `{crate_name}` rlib; found {}",
                    matches.len()
                ),
            });
        };
        let relative_path = format!("{relative_directory}/{file_name}");
        plan.supporting_artifacts
            .retain(|artifact| artifact.relative_path != relative_path);
        plan.externs.push(OvenRustcArtifactExtern {
            crate_name: crate_name.to_string(),
            relative_path,
            digest: digest.to_string(),
        });
    }
    plan.supporting_artifacts
        .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(())
}

/// Return only Rust-library files explicitly emitted by one named publisher Cargo invocation.
///
/// Cargo's dependency directory is staging state, not an Oven input contract. The publisher requests structured
/// `compiler-artifact` output and this helper admits only listed regular files beneath the exact target/host
/// dependency directories supplied by the caller. Cargo reports the primary package's rlib only at the profile root
/// (with a dependency-directory rmeta): that one exact, named rlib is admitted as an explicit Rustc extern. Every
/// other Rust-library file outside the dependency directories is refused. A path outside the one publisher target
/// root is likewise refused. This keeps an unrelated retained `deps` artifact from becoming a silent immutable seed
/// dependency while still retaining the publisher's real primary artifact.
fn compiler_artifact_paths_from_cargo_output(
    cargo_stdout: &[u8],
    publisher_target_root: &Path,
    allowed_directories: &[&Path],
    primary_crate_name: &str,
    primary_artifact_directory: &Path,
    publisher: &str,
) -> Result<Vec<PathBuf>, OvenNativeUnitError> {
    let publisher_target_root = fs::canonicalize(publisher_target_root).map_err(|source| OvenNativeUnitError::Io {
        path: publisher_target_root.to_path_buf(),
        source,
    })?;
    let allowed_directories = allowed_directories
        .iter()
        .map(|directory| {
            fs::canonicalize(directory).map_err(|source| OvenNativeUnitError::Io {
                path: (*directory).to_path_buf(),
                source,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if allowed_directories.is_empty() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!("{publisher} publisher declared no Cargo artifact directories"),
        });
    }
    let primary_artifact_directory =
        fs::canonicalize(primary_artifact_directory).map_err(|source| OvenNativeUnitError::Io {
            path: primary_artifact_directory.to_path_buf(),
            source,
        })?;
    let primary_artifact_filename = format!("lib{primary_crate_name}.rlib");
    let cargo_stdout = std::str::from_utf8(cargo_stdout).map_err(|error| OvenNativeUnitError::Preparation {
        message: format!("{publisher} publisher emitted non-UTF-8 Cargo JSON: {error}"),
    })?;
    let mut artifacts = BTreeSet::new();
    for (line_number, line) in cargo_stdout.lines().enumerate() {
        let value =
            serde_json::from_str::<serde_json::Value>(line).map_err(|error| OvenNativeUnitError::Preparation {
                message: format!(
                    "{publisher} publisher emitted invalid Cargo JSON on line {}: {error}",
                    line_number + 1
                ),
            })?;
        if value.get("reason").and_then(serde_json::Value::as_str) != Some("compiler-artifact") {
            continue;
        }
        let filenames = value
            .get("filenames")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| OvenNativeUnitError::Preparation {
                message: format!(
                    "{publisher} publisher compiler-artifact on line {} has no filenames",
                    line_number + 1
                ),
            })?;
        let artifact_target_name = value
            .get("target")
            .and_then(|target| target.get("name"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| OvenNativeUnitError::Preparation {
                message: format!(
                    "{publisher} publisher compiler-artifact on line {} has no target name",
                    line_number + 1
                ),
            })?;
        for filename in filenames {
            let filename = filename.as_str().ok_or_else(|| OvenNativeUnitError::Preparation {
                message: format!(
                    "{publisher} publisher compiler-artifact on line {} has a non-string filename",
                    line_number + 1
                ),
            })?;
            let source = PathBuf::from(filename);
            if !is_rust_library_artifact(&source) {
                continue;
            }
            let metadata = fs::symlink_metadata(&source).map_err(|source_error| OvenNativeUnitError::Io {
                path: source.clone(),
                source: source_error,
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(OvenNativeUnitError::Preparation {
                    message: format!(
                        "{publisher} publisher compiler-artifact is not a regular file: {}",
                        source.display()
                    ),
                });
            }
            let source = fs::canonicalize(&source).map_err(|source_error| OvenNativeUnitError::Io {
                path: source.clone(),
                source: source_error,
            })?;
            let in_dependency_directory = allowed_directories
                .iter()
                .any(|directory| source.starts_with(directory));
            let is_primary_profile_rlib = artifact_target_name == primary_crate_name
                && source.parent() == Some(primary_artifact_directory.as_path())
                && source.file_name().and_then(|name| name.to_str()) == Some(primary_artifact_filename.as_str());
            if in_dependency_directory || is_primary_profile_rlib {
                artifacts.insert(source);
            } else if !source.starts_with(&publisher_target_root) {
                return Err(OvenNativeUnitError::Preparation {
                    message: format!(
                        "{publisher} publisher compiler-artifact escaped its target root: {}",
                        source.display()
                    ),
                });
            } else {
                return Err(OvenNativeUnitError::Preparation {
                    message: format!(
                        "{publisher} publisher compiler-artifact escaped its declared dependency directories: {}",
                        source.display()
                    ),
                });
            }
        }
    }
    if artifacts.is_empty() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!("{publisher} publisher emitted no Rust compiler-artifact files"),
        });
    }
    Ok(artifacts.into_iter().collect())
}

/// Copy the target-only vocabulary support closure used to produce Wasm desugarers without Cargo.
///
/// The host closure remains an auxiliary search path because Rustc may need host procedural macros while compiling
/// target code. The target rlibs are retained separately and named explicitly, which prevents host and Wasm copies
/// of the same crate from occupying one ambiguous direct-Rustc search directory.
fn copy_compiler_vocab_auxiliary_target_artifacts(
    source_artifacts: &[PathBuf],
    seed_directory: &Path,
    target: &str,
    plan: &mut OvenRustcArtifactManifest,
) -> Result<(), OvenNativeUnitError> {
    const INCAN_VOCAB: &str = "incan_vocab";
    const SERDE_JSON: &str = "serde_json";
    fs::create_dir_all(seed_directory).map_err(|source| OvenNativeUnitError::Io {
        path: seed_directory.to_path_buf(),
        source,
    })?;
    let mut artifacts = BTreeMap::new();
    for source in source_artifacts {
        let metadata = fs::symlink_metadata(source).map_err(|source_error| OvenNativeUnitError::Io {
            path: source.clone(),
            source: source_error,
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || !is_rust_library_artifact(source) {
            return Err(OvenNativeUnitError::Preparation {
                message: format!(
                    "native vocabulary {target} compiler-artifact is not a regular direct-Rustc library: {}",
                    source.display()
                ),
            });
        }
        let file_name = source
            .file_name()
            .ok_or_else(|| OvenNativeUnitError::Preparation {
                message: format!("native vocabulary artifact has no filename: {}", source.display()),
            })?
            .to_string_lossy()
            .to_string();
        let bytes = fs::read(source).map_err(|source_error| OvenNativeUnitError::Io {
            path: source.clone(),
            source: source_error,
        })?;
        let digest = digest_bytes(&bytes);
        if artifacts.insert(file_name.clone(), digest.clone()).is_some() {
            return Err(OvenNativeUnitError::Preparation {
                message: format!("native vocabulary {target} closure duplicates artifact `{file_name}`"),
            });
        }
        let destination = seed_directory.join(&file_name);
        fs::write(&destination, bytes).map_err(|source_error| OvenNativeUnitError::Io {
            path: destination,
            source: source_error,
        })?;
    }
    if artifacts.is_empty() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!("native vocabulary publisher retained no declared {target} Rust artifacts"),
        });
    }
    let relative_directory = format!("compiler-support/{target}/deps");
    let mut externs = Vec::new();
    for crate_name in [INCAN_VOCAB, SERDE_JSON] {
        let matches = artifacts
            .iter()
            .filter(|(file_name, _)| is_named_rlib(file_name, crate_name))
            .collect::<Vec<_>>();
        let [(file_name, digest)] = matches.as_slice() else {
            return Err(OvenNativeUnitError::Preparation {
                message: format!(
                    "native vocabulary {target} publisher must retain exactly one `{crate_name}` rlib; found {}",
                    matches.len()
                ),
            });
        };
        externs.push(OvenRustcArtifactExtern {
            crate_name: crate_name.to_string(),
            relative_path: format!("{relative_directory}/{file_name}"),
            digest: digest.to_string(),
        });
    }
    for (file_name, digest) in artifacts {
        let relative_path = format!("{relative_directory}/{file_name}");
        if externs.iter().any(|artifact| artifact.relative_path == relative_path) {
            continue;
        }
        plan.supporting_artifacts
            .push(OvenRustcSupportingArtifact { relative_path, digest });
    }
    // `compiler-support/deps` holds host proc macros that Rustc may load while expanding the target closure.
    let mut dependency_search_paths = vec![relative_directory, "compiler-support/deps".to_string()];
    dependency_search_paths.sort();
    plan.vocab_auxiliary_targets.push(OvenRustcAuxiliaryTarget {
        target: target.to_string(),
        dependency_search_paths,
        externs,
    });
    plan.vocab_auxiliary_targets
        .sort_by(|left, right| left.target.cmp(&right.target));
    Ok(())
}

/// Return whether a file can participate in a direct Rustc dependency closure.
fn is_rust_library_artifact(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("rlib" | "dylib" | "so" | "dll")
    )
}

/// Return whether a retained artifact is the exact rlib for one compiler-owned crate.
fn is_named_rlib(relative_path: &str, crate_name: &str) -> bool {
    let Some(name) = Path::new(relative_path).file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == format!("lib{crate_name}.rlib")
        || (name.starts_with(&format!("lib{crate_name}-")) && name.ends_with(".rlib"))
}

/// Return whether a manifest path is the dynamic compiler-owned `incan_derive` procedural macro.
fn is_incan_derive_artifact(relative_path: &str) -> bool {
    let Some(name) = Path::new(relative_path).file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.starts_with("libincan_derive-")
        && matches!(
            Path::new(name).extension().and_then(|extension| extension.to_str()),
            Some("dylib" | "so" | "dll")
        )
}

/// Measure the exact directory that will be copied into a toolchain archive.
///
/// This intentionally includes `seed.json`: while the plan is control metadata rather than a link input, it is a
/// retained physical file and therefore belongs in the release accounting. Publisher source files are copied rather
/// than linked, so summing regular-file allocation gives a conservative, portable report for this final closure.
fn seed_directory_byte_counts(root: &Path) -> Result<(u64, u64), OvenNativeUnitError> {
    let metadata = fs::symlink_metadata(root).map_err(|source| OvenNativeUnitError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!("native-unit seed may not contain a symlink: {}", root.display()),
        });
    }
    if metadata.is_file() {
        return Ok((metadata.len(), seed_file_physical_bytes(&metadata)));
    }
    if !metadata.is_dir() {
        return Err(OvenNativeUnitError::Preparation {
            message: format!(
                "native-unit seed may contain only regular files and directories: {}",
                root.display()
            ),
        });
    }

    let mut logical_bytes = 0_u64;
    let mut physical_bytes = 0_u64;
    for child in fs::read_dir(root).map_err(|source| OvenNativeUnitError::Io {
        path: root.to_path_buf(),
        source,
    })? {
        let child = child.map_err(|source| OvenNativeUnitError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let (child_logical_bytes, child_physical_bytes) = seed_directory_byte_counts(&child.path())?;
        logical_bytes = logical_bytes.saturating_add(child_logical_bytes);
        physical_bytes = physical_bytes.saturating_add(child_physical_bytes);
    }
    Ok((logical_bytes, physical_bytes))
}

/// Return physical allocation for one compiler-shipped seed file, preserving a portable fallback outside Unix.
#[cfg(unix)]
fn seed_file_physical_bytes(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;

    metadata.blocks().saturating_mul(512)
}

/// Return logical bytes where the host cannot expose allocated Unix block counts.
#[cfg(not(unix))]
fn seed_file_physical_bytes(metadata: &fs::Metadata) -> u64 {
    metadata.len()
}

/// Remove Cargo's redundant Rust metadata sidecars from one compiler-shipped seed.
///
/// The direct-rustc plan names its usable roots as `--extern` rlibs and retains all other required link inputs. A
/// standalone `.rmeta` sidecar is Cargo metadata for a companion rlib, not a direct-rustc input. Removing it here is
/// safe only because the reduced plan is immediately revalidated and copied from that plan; stored compiler-suite
/// closures retain their original artifact inventories.
fn discard_seed_metadata_sidecars(plan: &mut OvenRustcArtifactManifest) {
    plan.supporting_artifacts
        .retain(|artifact| !artifact.relative_path.ends_with(".rmeta"));
}

/// Return the compiler-owned seed path for a receipt's source-independent native build unit.
///
/// Installed toolchains place this beneath `share/incan/oven/native-units`; a development checkout may provide the
/// same layout for an integration fixture. The caller cannot select an arbitrary seed path.
#[must_use]
pub fn toolchain_native_unit_seed_path(receipt: &OvenReceipt) -> PathBuf {
    let identity = receipt
        .build_unit_identity
        .strip_prefix("sha256:")
        .unwrap_or(receipt.build_unit_identity.as_str());
    crate::toolchain_layout::resolve_toolchain_data_path(
        &Path::new(TOOLCHAIN_SEED_RELATIVE_ROOT).join(identity).join("seed.json"),
    )
}

/// Materialize a compiler-owned seed only when the active toolchain ships one that the selected policy authorizes.
///
/// `Ok(None)` means the requested unit is outside the installed Oven Alpha envelope. It is intentionally distinct
/// from a corrupt seed, which is a fail-closed error rather than permission to use a generated-Cargo fallback.
pub fn materialize_toolchain_native_unit(
    store: &OvenStore,
    receipt: &OvenReceipt,
    selection: OvenNativeUnitSelection,
) -> Result<Option<String>, OvenNativeUnitError> {
    let seed_path = toolchain_native_unit_seed_path(receipt);
    if seed_path.is_file() {
        return materialize_native_unit_from_seed(store, receipt, &seed_path).map(Some);
    }
    if selection == OvenNativeUnitSelection::Exact {
        return Ok(None);
    }

    let seed_root = crate::toolchain_layout::resolve_toolchain_data_path(Path::new(TOOLCHAIN_SEED_RELATIVE_ROOT));
    let candidates = compatible_seed_paths(&seed_root, receipt)?;
    let Some(candidate) = select_most_specific_compatible_seed(candidates) else {
        return Ok(None);
    };
    materialize_native_unit_from_seed_with_selection(
        store,
        receipt,
        &candidate.path,
        OvenNativeUnitSelection::CompilerOwnedProviderSuperset,
    )
    .map(Some)
}

/// Materialize the narrowest receipt-compatible seed whose own registry catalog satisfies every caller-selected
/// registry dependency.
///
/// Direct Rust metadata is a feature-unified graph, not a set of interchangeable package filenames. When a caller
/// imports a registry leaf absent from the narrowest provider-only seed, selecting that seed and borrowing an
/// arbitrary compatible catalog can combine incompatible `serde`/`rand` instances. This selector stays within the
/// compiler-shipped seeds, but chooses one coherent closure before any normal direct-rustc process starts.
pub fn materialize_toolchain_native_unit_for_registry_dependencies(
    store: &OvenStore,
    receipt: &OvenReceipt,
    selection: OvenNativeUnitSelection,
    dependencies: &[DependencySpec],
) -> Result<Option<String>, OvenNativeUnitError> {
    let registry_dependencies = dependencies
        .iter()
        .filter(|dependency| matches!(dependency.source, DependencySource::Registry))
        .collect::<Vec<_>>();
    if registry_dependencies.is_empty() {
        return materialize_toolchain_native_unit(store, receipt, selection);
    }

    let seed_path = toolchain_native_unit_seed_path(receipt);
    if seed_path.is_file() {
        let native = native_unit_from_seed(receipt, &seed_path, selection)?;
        return registry_dependencies_supported_by_native_unit(
            &native,
            &registry_dependencies,
            &receipt.intent.profile,
        )
        .then(|| materialize_native_unit_from_seed_with_selection(store, receipt, &seed_path, selection))
        .transpose();
    }
    if selection == OvenNativeUnitSelection::Exact {
        return Ok(None);
    }

    let seed_root = crate::toolchain_layout::resolve_toolchain_data_path(Path::new(TOOLCHAIN_SEED_RELATIVE_ROOT));
    let candidates = compatible_seed_paths(&seed_root, receipt)?;
    let mut supported = Vec::new();
    for candidate in candidates {
        let native = native_unit_from_seed(
            receipt,
            &candidate.path,
            OvenNativeUnitSelection::CompilerOwnedProviderSuperset,
        )?;
        if registry_dependencies_supported_by_native_unit(&native, &registry_dependencies, &receipt.intent.profile) {
            supported.push(candidate);
        }
    }
    let Some(candidate) = select_most_specific_compatible_seed(supported) else {
        return Ok(None);
    };
    materialize_native_unit_from_seed_with_selection(
        store,
        receipt,
        &candidate.path,
        OvenNativeUnitSelection::CompilerOwnedProviderSuperset,
    )
    .map(Some)
}

/// Check whether one already validated compiler-native seed can supply all caller-visible registry imports from its
/// own exact catalog. A missing or incompatible leaf disqualifies this seed; it never widens the caller to Cargo.
fn registry_dependencies_supported_by_native_unit(
    native: &OvenToolchainNativeUnit,
    dependencies: &[&DependencySpec],
    profile: &str,
) -> bool {
    let authority = native.registry_leaf_authority();
    dependencies
        .iter()
        .all(|dependency| validate_sealed_registry_leaf(dependency, Some(&authority), profile).is_ok())
}

/// Resolve a compiler-native seed for scheduler-owned direct execution without copying it into a mutable Oven store.
///
/// This is deliberately narrower than [`materialize_toolchain_native_unit`]. Callers must use it only for immutable
/// compiler-suite toolchain data already selected and leased by the parent scheduler; it is not a general project
/// artifact-path override or a dependency resolver.
pub fn resolve_toolchain_native_unit(
    receipt: &OvenReceipt,
    selection: OvenNativeUnitSelection,
) -> Result<Option<OvenToolchainNativeUnit>, OvenNativeUnitError> {
    let seed_path = toolchain_native_unit_seed_path(receipt);
    if seed_path.is_file() {
        return native_unit_from_seed(receipt, &seed_path, OvenNativeUnitSelection::Exact).map(Some);
    }
    if selection == OvenNativeUnitSelection::Exact {
        return Ok(None);
    }

    let seed_root = crate::toolchain_layout::resolve_toolchain_data_path(Path::new(TOOLCHAIN_SEED_RELATIVE_ROOT));
    let candidates = compatible_seed_paths(&seed_root, receipt)?;
    let Some(candidate) = select_most_specific_compatible_seed(candidates) else {
        return Ok(None);
    };
    native_unit_from_seed(
        receipt,
        &candidate.path,
        OvenNativeUnitSelection::CompilerOwnedProviderSuperset,
    )
    .map(Some)
}

/// Resolve a scheduler-held compiler-native seed whose own catalog satisfies every caller-selected registry root.
///
/// This is the immutable-suite counterpart to
/// [`materialize_toolchain_native_unit_for_registry_dependencies`]. It keeps nested suite children on the parent
/// lease while preserving the same single compatibility-domain rule as ordinary bounded-store consumers.
pub fn resolve_toolchain_native_unit_for_registry_dependencies(
    receipt: &OvenReceipt,
    selection: OvenNativeUnitSelection,
    dependencies: &[DependencySpec],
) -> Result<Option<OvenToolchainNativeUnit>, OvenNativeUnitError> {
    let registry_dependencies = dependencies
        .iter()
        .filter(|dependency| matches!(dependency.source, DependencySource::Registry))
        .collect::<Vec<_>>();
    if registry_dependencies.is_empty() {
        return resolve_toolchain_native_unit(receipt, selection);
    }

    let seed_path = toolchain_native_unit_seed_path(receipt);
    if seed_path.is_file() {
        let native = native_unit_from_seed(receipt, &seed_path, selection)?;
        return Ok(registry_dependencies_supported_by_native_unit(
            &native,
            &registry_dependencies,
            &receipt.intent.profile,
        )
        .then_some(native));
    }
    if selection == OvenNativeUnitSelection::Exact {
        return Ok(None);
    }

    let seed_root = crate::toolchain_layout::resolve_toolchain_data_path(Path::new(TOOLCHAIN_SEED_RELATIVE_ROOT));
    let candidates = compatible_seed_paths(&seed_root, receipt)?;
    let mut supported = Vec::new();
    for candidate in candidates {
        let native = native_unit_from_seed(
            receipt,
            &candidate.path,
            OvenNativeUnitSelection::CompilerOwnedProviderSuperset,
        )?;
        if registry_dependencies_supported_by_native_unit(&native, &registry_dependencies, &receipt.intent.profile) {
            supported.push(candidate);
        }
    }
    let Some(candidate) = select_most_specific_compatible_seed(supported) else {
        return Ok(None);
    };
    native_unit_from_seed(
        receipt,
        &candidate.path,
        OvenNativeUnitSelection::CompilerOwnedProviderSuperset,
    )
    .map(Some)
}

/// Return every compiler-owned seed that authorizes the narrow runtime-provider subset rule.
fn compatible_seed_paths(
    seed_root: &Path,
    receipt: &OvenReceipt,
) -> Result<Vec<CompatibleNativeUnitSeed>, OvenNativeUnitError> {
    let entries = match fs::read_dir(seed_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(OvenNativeUnitError::Io {
                path: seed_root.to_path_buf(),
                source,
            });
        }
    };
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| OvenNativeUnitError::Io {
            path: seed_root.to_path_buf(),
            source,
        })?;
        let file_type = entry.file_type().map_err(|source| OvenNativeUnitError::Io {
            path: entry.path(),
            source,
        })?;
        if !file_type.is_dir() {
            continue;
        }
        let seed_path = entry.path().join("seed.json");
        if !seed_path.is_file() {
            continue;
        }
        let seed = read_native_unit_seed(&seed_path)?;
        if seed.schema_version != OVEN_NATIVE_UNIT_SEED_SCHEMA_VERSION
            || seed.build_unit_identity == receipt.build_unit_identity
        {
            continue;
        }
        if seed.plan.intent == receipt.intent
            && let Some(excess) = seed.compatibility.provider_subset_excess(receipt)?
        {
            candidates.push(CompatibleNativeUnitSeed {
                path: seed_path,
                excess,
            });
        }
    }
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(candidates)
}

/// Select the narrowest compatible compiler-owned seed, with a path tie-breaker for reproducibility.
///
/// Every candidate has already matched all runtime inputs and contains every requested provider module/facet. The
/// tie-breaker therefore cannot widen the authority of the request; it only prevents independent shipped provider
/// families from making a no-provider request fail arbitrarily.
fn select_most_specific_compatible_seed(
    mut candidates: Vec<CompatibleNativeUnitSeed>,
) -> Option<CompatibleNativeUnitSeed> {
    candidates.sort_by(|left, right| left.excess.cmp(&right.excess).then_with(|| left.path.cmp(&right.path)));
    candidates.into_iter().next()
}

/// Validate and copy one explicitly located compiler-owned seed into the bounded store.
///
/// This lower-level entry point exists for packaging and focused tests. Normal CLI commands use
/// [`materialize_toolchain_native_unit`], which derives the only accepted location from the active toolchain.
pub fn materialize_native_unit_from_seed(
    store: &OvenStore,
    receipt: &OvenReceipt,
    seed_path: &Path,
) -> Result<String, OvenNativeUnitError> {
    materialize_native_unit_from_seed_with_selection(store, receipt, seed_path, OvenNativeUnitSelection::Exact)
}

/// Validate and copy one seed whose exact or explicit provider-subset authorization was selected internally.
fn materialize_native_unit_from_seed_with_selection(
    store: &OvenStore,
    receipt: &OvenReceipt,
    seed_path: &Path,
    selection: OvenNativeUnitSelection,
) -> Result<String, OvenNativeUnitError> {
    let native_unit = native_unit_from_seed(receipt, seed_path, selection)?;
    let materialized_files = native_unit
        .artifacts
        .materialized_artifacts(&native_unit.artifact_root, &receipt.intent)?
        .into_iter()
        .map(|artifact| OvenArtifactMaterializedFile {
            source_path: artifact.source_path,
            relative_path: artifact.relative_path,
        })
        .collect::<Vec<_>>();
    let payload = serde_json::to_vec(&native_unit.artifacts).map_err(|error| OvenNativeUnitError::InvalidSeed {
        path: seed_path.to_path_buf(),
        message: format!("could not serialize direct-rustc plan: {error}"),
    })?;
    let domain = native_unit_domain(&receipt.build_unit_identity);
    let artifact = store.publish(&OvenArtifactPublishRequest {
        receipt: receipt.clone(),
        domain,
        kind: OvenArtifactKind::DirectRustcPlan,
        payload,
        materialized_files,
    })?;
    Ok(artifact.identity)
}

/// Verify that one seed authorizes `receipt` and resolve its compiler-owned direct-Rustc closure.
fn native_unit_from_seed(
    receipt: &OvenReceipt,
    seed_path: &Path,
    selection: OvenNativeUnitSelection,
) -> Result<OvenToolchainNativeUnit, OvenNativeUnitError> {
    receipt
        .verify_identity()
        .map_err(|error| OvenNativeUnitError::InvalidSeed {
            path: seed_path.to_path_buf(),
            message: format!("requested receipt is invalid: {error}"),
        })?;
    let seed = read_native_unit_seed(seed_path)?;
    if seed.schema_version != OVEN_NATIVE_UNIT_SEED_SCHEMA_VERSION {
        return Err(OvenNativeUnitError::InvalidSeed {
            path: seed_path.to_path_buf(),
            message: format!(
                "schema version {} is unsupported (expected {})",
                seed.schema_version, OVEN_NATIVE_UNIT_SEED_SCHEMA_VERSION
            ),
        });
    }
    let exact_identity = seed.build_unit_identity == receipt.build_unit_identity;
    let compatible_provider_subset = selection == OvenNativeUnitSelection::CompilerOwnedProviderSuperset
        && seed.compatibility.authorizes_provider_subset(receipt)?;
    if !exact_identity && !compatible_provider_subset {
        return Err(OvenNativeUnitError::InvalidSeed {
            path: seed_path.to_path_buf(),
            message: "build-unit identity does not authorize the requested receipt or provider-subset runtime"
                .to_string(),
        });
    }
    if seed.plan.intent != receipt.intent {
        return Err(OvenNativeUnitError::InvalidSeed {
            path: seed_path.to_path_buf(),
            message: "direct-rustc intent does not authorize the requested receipt".to_string(),
        });
    }
    if seed.plan.registry_leaves != seed.registry_leaves {
        return Err(OvenNativeUnitError::InvalidSeed {
            path: seed_path.to_path_buf(),
            message: "seed registry catalog does not match its copied direct-rustc plan".to_string(),
        });
    }
    validate_registry_leaf_catalog(&seed, seed_path)?;
    let artifact_root = seed_path.parent().ok_or_else(|| OvenNativeUnitError::InvalidSeed {
        path: seed_path.to_path_buf(),
        message: "seed file has no parent directory".to_string(),
    })?;
    let artifact_plan = seed.plan.materialize_trusted_store(artifact_root, &receipt.intent)?;
    Ok(OvenToolchainNativeUnit {
        seed_build_unit_identity: seed.build_unit_identity,
        artifacts: seed.plan,
        registry_leaves: seed.registry_leaves,
        artifact_root: artifact_root.to_path_buf(),
        artifact_plan,
    })
}

/// Reject registry catalog records that do not describe an artifact already sealed by the native-unit plan.
///
/// The direct-Rustc resolver may select a catalog leaf by package requirement. Binding every leaf to the plan keeps
/// that selection from becoming a second, less constrained artifact channel beside the receipt-owned closure.
fn validate_registry_leaf_catalog(seed: &OvenNativeUnitSeed, seed_path: &Path) -> Result<(), OvenNativeUnitError> {
    let mut plan_artifacts = BTreeMap::new();
    for artifact in seed
        .plan
        .externs
        .iter()
        .map(|artifact| (&artifact.relative_path, &artifact.digest))
        .chain(
            seed.plan
                .supporting_artifacts
                .iter()
                .map(|artifact| (&artifact.relative_path, &artifact.digest)),
        )
    {
        if plan_artifacts
            .insert(artifact.0.as_str(), artifact.1.as_str())
            .is_some()
        {
            return Err(OvenNativeUnitError::InvalidSeed {
                path: seed_path.to_path_buf(),
                message: format!("direct-rustc plan declares artifact `{}` more than once", artifact.0),
            });
        }
    }

    let mut package_versions = BTreeSet::new();
    for leaf in &seed.registry_leaves {
        if leaf.package.trim().is_empty() || leaf.version.trim().is_empty() {
            return Err(OvenNativeUnitError::InvalidSeed {
                path: seed_path.to_path_buf(),
                message: "registry leaf package and version must not be empty".to_string(),
            });
        }
        if leaf.crate_name.trim().is_empty() || leaf.crate_name != leaf.artifact.crate_name {
            return Err(OvenNativeUnitError::InvalidSeed {
                path: seed_path.to_path_buf(),
                message: format!(
                    "registry leaf `{}` `{}` has inconsistent crate identity",
                    leaf.package, leaf.version
                ),
            });
        }
        if !package_versions.insert((leaf.package.as_str(), leaf.version.as_str())) {
            return Err(OvenNativeUnitError::InvalidSeed {
                path: seed_path.to_path_buf(),
                message: format!(
                    "registry catalog declares package `{}` version `{}` more than once",
                    leaf.package, leaf.version
                ),
            });
        }
        let mut features = BTreeSet::new();
        for feature in &leaf.features {
            if feature.trim().is_empty() || !features.insert(feature.as_str()) {
                return Err(OvenNativeUnitError::InvalidSeed {
                    path: seed_path.to_path_buf(),
                    message: format!(
                        "registry leaf `{}` `{}` declares an empty or duplicate feature",
                        leaf.package, leaf.version
                    ),
                });
            }
        }
        if Path::new(&leaf.artifact.relative_path)
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("rlib")
        {
            return Err(OvenNativeUnitError::InvalidSeed {
                path: seed_path.to_path_buf(),
                message: format!(
                    "registry leaf `{}` `{}` must reference an rlib",
                    leaf.package, leaf.version
                ),
            });
        }
        match plan_artifacts.get(leaf.artifact.relative_path.as_str()) {
            Some(digest) if *digest == leaf.artifact.digest.as_str() => {}
            Some(_) => {
                return Err(OvenNativeUnitError::InvalidSeed {
                    path: seed_path.to_path_buf(),
                    message: format!(
                        "registry leaf `{}` `{}` has a digest that disagrees with its sealed direct-rustc plan artifact",
                        leaf.package, leaf.version
                    ),
                });
            }
            None => {
                return Err(OvenNativeUnitError::InvalidSeed {
                    path: seed_path.to_path_buf(),
                    message: format!(
                        "registry leaf `{}` `{}` references `{}`, which the sealed direct-rustc plan does not declare",
                        leaf.package, leaf.version, leaf.artifact.relative_path
                    ),
                });
            }
        }
    }
    Ok(())
}

/// Read one native-unit seed and attach the source path to any decoding failure.
fn read_native_unit_seed(seed_path: &Path) -> Result<OvenNativeUnitSeed, OvenNativeUnitError> {
    let bytes = fs::read(seed_path).map_err(|source| OvenNativeUnitError::Io {
        path: seed_path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice::<OvenNativeUnitSeed>(&bytes).map_err(|error| OvenNativeUnitError::InvalidSeed {
        path: seed_path.to_path_buf(),
        message: format!("must be valid JSON: {error}"),
    })
}

/// Keep one native compatibility identity in a stable, path-safe capacity domain.
fn native_unit_domain(build_unit_identity: &str) -> String {
    let identity = build_unit_identity
        .strip_prefix("sha256:")
        .unwrap_or(build_unit_identity);
    format!("native-unit-{identity}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{
        CompatibleNativeUnitSeed, OVEN_NATIVE_UNIT_SEED_SCHEMA_VERSION, OvenNativeUnitCompatibility,
        OvenNativeUnitError, OvenNativeUnitSeed, OvenNativeUnitSelection, digest_runtime_crate_source,
        materialize_native_unit_from_seed, materialize_native_unit_from_seed_with_selection, native_unit_from_seed,
        select_most_specific_compatible_seed,
    };
    use crate::oven::rustc::{
        OVEN_RUSTC_ARTIFACT_MANIFEST_SCHEMA_VERSION, OvenRustcArtifactExtern, OvenRustcArtifactManifest,
        OvenRustcRegistryLeaf, OvenRustcSupportingArtifact, select_direct_rustc_plan_identity,
    };
    use crate::oven::store::{OvenStore, OvenStoreLimits};
    use crate::oven::{OvenGeneratedProjectRequest, digest_bytes, digest_source_tree, receipt_generated_project};

    #[test]
    fn runtime_source_digest_matches_the_staged_minimal_runtime_closure() -> Result<(), Box<dyn std::error::Error>> {
        let source = tempfile::tempdir()?;
        fs::create_dir_all(source.path().join("src/nested"))?;
        fs::create_dir_all(source.path().join("target/temporary"))?;
        fs::write(
            source.path().join("Cargo.toml"),
            "[package]\nname = \"runtime\"\nversion = \"0.1.0\"\n",
        )?;
        fs::write(source.path().join("src/lib.rs"), "pub mod nested;\n")?;
        fs::write(source.path().join("src/nested/mod.rs"), "pub fn value() {}\n")?;
        fs::write(source.path().join("README.md"), "not a runtime input\n")?;
        fs::write(source.path().join("target/temporary/artifact"), "not a runtime input\n")?;

        let staged = tempfile::tempdir()?;
        fs::create_dir_all(staged.path().join("src/nested"))?;
        for relative in ["Cargo.toml", "src/lib.rs", "src/nested/mod.rs"] {
            fs::copy(source.path().join(relative), staged.path().join(relative))?;
        }

        let digest = digest_runtime_crate_source(source.path())?;
        assert_eq!(digest, digest_source_tree(staged.path())?);

        fs::write(source.path().join("README.md"), "still not a runtime input\n")?;
        assert_eq!(digest_runtime_crate_source(source.path())?, digest);
        fs::write(source.path().join("src/nested/mod.rs"), "pub fn changed() {}\n")?;
        assert_ne!(digest_runtime_crate_source(source.path())?, digest);
        Ok(())
    }

    #[test]
    fn a_toolchain_seed_materializes_once_and_reuses_across_clean_project_receipts()
    -> Result<(), Box<dyn std::error::Error>> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let seed = tempfile::tempdir()?;
        let store_root = tempfile::tempdir()?;
        fs::write(first.path().join("main.rs"), "fn main() {}\n")?;
        fs::write(second.path().join("main.rs"), "fn main() { println!(\"second\"); }\n")?;
        let receipt_for = |root: &Path| {
            receipt_generated_project(
                &OvenGeneratedProjectRequest::new(
                    root,
                    "seeded_fixture",
                    "0.1.0",
                    "aarch64-apple-darwin",
                    "rustc seeded-test",
                    "debug",
                    Vec::new(),
                )
                .with_generated_source("generated-root", root.join("main.rs")),
            )
        };
        let first_receipt = receipt_for(first.path())?;
        let second_receipt = receipt_for(second.path())?;
        assert_ne!(first_receipt.identity, second_receipt.identity);
        assert_eq!(first_receipt.build_unit_identity, second_receipt.build_unit_identity);
        let seed_payload = OvenNativeUnitSeed {
            schema_version: OVEN_NATIVE_UNIT_SEED_SCHEMA_VERSION,
            build_unit_identity: first_receipt.build_unit_identity.clone(),
            compatibility: OvenNativeUnitCompatibility {
                runtime_inputs: BTreeMap::new(),
                providers: Vec::new(),
            },
            registry_leaves: Vec::new(),
            plan: OvenRustcArtifactManifest {
                schema_version: OVEN_RUSTC_ARTIFACT_MANIFEST_SCHEMA_VERSION,
                intent: first_receipt.intent.clone(),
                dependency_search_paths: Vec::new(),
                native_search_paths: Vec::new(),
                externs: Vec::new(),
                entrypoint_externs: BTreeMap::new(),
                registry_leaves: Vec::new(),
                compile_environment: BTreeMap::new(),
                vocab_auxiliary_targets: Vec::new(),
                supporting_artifacts: Vec::new(),
            },
        };
        let seed_path = seed.path().join("seed.json");
        fs::write(&seed_path, serde_json::to_vec(&seed_payload)?)?;
        let store = OvenStore::new(
            store_root.path(),
            OvenStoreLimits::new(128 * 1024, 128 * 1024, 64 * 1024),
        );

        let resolved = native_unit_from_seed(&second_receipt, &seed_path, OvenNativeUnitSelection::Exact)?;
        assert_eq!(resolved.seed_build_unit_identity, first_receipt.build_unit_identity);
        assert_eq!(resolved.artifact_root, seed.path());
        assert!(resolved.artifact_plan.externs.is_empty());

        let identity = materialize_native_unit_from_seed(&store, &first_receipt, &seed_path)?;
        assert_eq!(select_direct_rustc_plan_identity(&store, &second_receipt)?, identity);
        Ok(())
    }

    #[test]
    fn a_seed_for_another_build_unit_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
        let project = tempfile::tempdir()?;
        let seed = tempfile::tempdir()?;
        let store_root = tempfile::tempdir()?;
        let source = project.path().join("main.rs");
        fs::write(&source, "fn main() {}\n")?;
        let receipt = receipt_generated_project(
            &OvenGeneratedProjectRequest::new(
                project.path(),
                "mismatch_fixture",
                "0.1.0",
                "aarch64-apple-darwin",
                "rustc seeded-test",
                "debug",
                Vec::new(),
            )
            .with_generated_source("generated-root", &source),
        )?;
        let seed_payload = OvenNativeUnitSeed {
            schema_version: OVEN_NATIVE_UNIT_SEED_SCHEMA_VERSION,
            build_unit_identity: digest_bytes(b"another-unit"),
            compatibility: OvenNativeUnitCompatibility {
                runtime_inputs: BTreeMap::new(),
                providers: Vec::new(),
            },
            registry_leaves: Vec::new(),
            plan: OvenRustcArtifactManifest {
                schema_version: OVEN_RUSTC_ARTIFACT_MANIFEST_SCHEMA_VERSION,
                intent: receipt.intent.clone(),
                dependency_search_paths: Vec::new(),
                native_search_paths: Vec::new(),
                externs: Vec::new(),
                entrypoint_externs: BTreeMap::new(),
                registry_leaves: Vec::new(),
                compile_environment: BTreeMap::new(),
                vocab_auxiliary_targets: Vec::new(),
                supporting_artifacts: Vec::new(),
            },
        };
        let seed_path = seed.path().join("seed.json");
        fs::write(&seed_path, serde_json::to_vec(&seed_payload)?)?;
        let store = OvenStore::new(
            store_root.path(),
            OvenStoreLimits::new(128 * 1024, 128 * 1024, 64 * 1024),
        );

        assert!(matches!(
            materialize_native_unit_from_seed(&store, &receipt, &seed_path),
            Err(OvenNativeUnitError::InvalidSeed { .. })
        ));
        Ok(())
    }

    #[test]
    fn native_seed_rejects_registry_leaves_outside_its_declared_plan() -> Result<(), Box<dyn std::error::Error>> {
        let project = tempfile::tempdir()?;
        let seed_root = tempfile::tempdir()?;
        let source = project.path().join("main.rs");
        fs::write(&source, "fn main() {}\n")?;
        let receipt = runtime_receipt(&source, "", "fixture-registry", "fixture-stdlib")?;
        let artifact_relative_path = "deps/libfixture.rlib".to_string();
        let artifact_path = seed_root.path().join(&artifact_relative_path);
        fs::create_dir_all(artifact_path.parent().ok_or("registry artifact parent")?)?;
        let artifact_bytes = b"sealed fixture registry artifact";
        fs::write(&artifact_path, artifact_bytes)?;
        let artifact_digest = digest_bytes(artifact_bytes);
        let mut plan = empty_manifest(&receipt);
        plan.dependency_search_paths = vec!["deps".to_string()];
        plan.supporting_artifacts.push(OvenRustcSupportingArtifact {
            relative_path: artifact_relative_path.clone(),
            digest: artifact_digest.clone(),
        });
        let registry_leaf = OvenRustcRegistryLeaf {
            package: "fixture-registry".to_string(),
            version: "1.0.0".to_string(),
            crate_name: "fixture_registry".to_string(),
            features: vec!["std".to_string()],
            artifact: OvenRustcArtifactExtern {
                crate_name: "fixture_registry".to_string(),
                relative_path: artifact_relative_path,
                digest: artifact_digest,
            },
        };
        plan.registry_leaves = vec![registry_leaf.clone()];
        let mut seed = OvenNativeUnitSeed {
            schema_version: OVEN_NATIVE_UNIT_SEED_SCHEMA_VERSION,
            build_unit_identity: receipt.build_unit_identity.clone(),
            compatibility: OvenNativeUnitCompatibility::default(),
            registry_leaves: vec![registry_leaf],
            plan,
        };
        let seed_path = seed_root.path().join("seed.json");
        fs::write(&seed_path, serde_json::to_vec(&seed)?)?;
        let resolved = native_unit_from_seed(&receipt, &seed_path, OvenNativeUnitSelection::Exact)?;
        assert_eq!(resolved.registry_leaves.len(), 1);

        seed.registry_leaves[0].artifact.relative_path = "deps/libunsealed.rlib".to_string();
        seed.plan.registry_leaves = seed.registry_leaves.clone();
        fs::write(&seed_path, serde_json::to_vec(&seed)?)?;
        let error = native_unit_from_seed(&receipt, &seed_path, OvenNativeUnitSelection::Exact)
            .expect_err("a registry leaf outside the sealed plan must fail");
        assert!(matches!(error, OvenNativeUnitError::InvalidSeed { .. }));
        assert!(error.to_string().contains("sealed direct-rustc plan does not declare"));
        Ok(())
    }

    #[test]
    fn a_standard_testing_seed_authorizes_the_core_provider_subset() -> Result<(), Box<dyn std::error::Error>> {
        let project = tempfile::tempdir()?;
        let seed_root = tempfile::tempdir()?;
        let store_root = tempfile::tempdir()?;
        let source = project.path().join("main.rs");
        fs::write(&source, "fn main() {}\n")?;
        let core = runtime_receipt(&source, "", "empty-rust-dependencies", "empty-stdlib-features")?;
        let testing = runtime_receipt(
            &source,
            "incan-stdlib|std.testing|testing",
            "empty-rust-dependencies",
            "fs,json,testing",
        )?;
        let unsupported_facet = runtime_receipt(
            &source,
            "incan-stdlib|std.testing|unsupported",
            "empty-rust-dependencies",
            "empty-stdlib-features",
        )?;
        assert_ne!(core.build_unit_identity, testing.build_unit_identity);

        let compatibility = OvenNativeUnitCompatibility::from_receipt(&testing)?;
        assert!(compatibility.authorizes_provider_subset(&core)?);
        assert!(!OvenNativeUnitCompatibility::from_receipt(&core)?.authorizes_provider_subset(&testing)?);
        assert!(!compatibility.authorizes_provider_subset(&unsupported_facet)?);

        let seed_payload = OvenNativeUnitSeed {
            schema_version: OVEN_NATIVE_UNIT_SEED_SCHEMA_VERSION,
            build_unit_identity: testing.build_unit_identity.clone(),
            compatibility,
            registry_leaves: Vec::new(),
            plan: empty_manifest(&testing),
        };
        let seed_path = seed_root.path().join("seed.json");
        fs::write(&seed_path, serde_json::to_vec(&seed_payload)?)?;
        let store = OvenStore::new(
            store_root.path(),
            OvenStoreLimits::new(128 * 1024, 128 * 1024, 64 * 1024),
        );

        let identity = materialize_native_unit_from_seed_with_selection(
            &store,
            &core,
            &seed_path,
            OvenNativeUnitSelection::CompilerOwnedProviderSuperset,
        )?;
        assert_eq!(select_direct_rustc_plan_identity(&store, &core)?, identity);
        Ok(())
    }

    #[test]
    fn a_private_sdk_direct_link_requires_a_seed_extern_capability() -> Result<(), Box<dyn std::error::Error>> {
        let project = tempfile::tempdir()?;
        let source = project.path().join("main.rs");
        fs::write(&source, "fn main() {}\n")?;
        let ordinary = runtime_receipt(&source, "incan_stdlib_data|||none", "direct-link", "no-stdlib-features")?;
        let private_sdk_link =
            runtime_receipt(&source, "incan_stdlib_data|||link", "direct-link", "no-stdlib-features")?;

        let ordinary_compatibility = OvenNativeUnitCompatibility::from_receipt(&ordinary)?;
        let linked_compatibility = OvenNativeUnitCompatibility::from_receipt(&private_sdk_link)?;
        assert!(linked_compatibility.authorizes_provider_subset(&ordinary)?);
        assert!(
            !ordinary_compatibility.authorizes_provider_subset(&private_sdk_link)?,
            "a seed without the direct SDK rlib cannot authorize a provider's private link root"
        );
        Ok(())
    }

    #[test]
    fn native_unit_selection_prefers_the_narrowest_compatible_provider_seed() -> Result<(), Box<dyn std::error::Error>>
    {
        let project = tempfile::tempdir()?;
        let source = project.path().join("main.rs");
        fs::write(&source, "fn main() {}\n")?;
        let core = runtime_receipt(&source, "", "empty-rust-dependencies", "empty-stdlib-features")?;
        let encoding = runtime_receipt(
            &source,
            "incan-stdlib|std.encoding.base64|codecs",
            "empty-rust-dependencies",
            "codecs",
        )?;
        let broad = runtime_receipt(
            &source,
            "incan-stdlib|std.async,std.testing|async,testing",
            "empty-rust-dependencies",
            "async,testing",
        )?;

        let encoding_excess = OvenNativeUnitCompatibility::from_receipt(&encoding)?
            .provider_subset_excess(&core)?
            .ok_or("encoding seed must authorize the core subset")?;
        let broad_excess = OvenNativeUnitCompatibility::from_receipt(&broad)?
            .provider_subset_excess(&core)?
            .ok_or("broad seed must authorize the core subset")?;
        assert!(encoding_excess < broad_excess);

        let selected = select_most_specific_compatible_seed(vec![
            CompatibleNativeUnitSeed {
                path: PathBuf::from("/toolchain/native-units/broad/seed.json"),
                excess: broad_excess,
            },
            CompatibleNativeUnitSeed {
                path: PathBuf::from("/toolchain/native-units/encoding/seed.json"),
                excess: encoding_excess,
            },
        ])
        .ok_or("a compatible seed must be selected")?;
        assert_eq!(
            selected.path,
            PathBuf::from("/toolchain/native-units/encoding/seed.json")
        );
        Ok(())
    }

    #[test]
    fn a_native_seed_drops_only_redundant_rmeta_sidecars() -> Result<(), Box<dyn std::error::Error>> {
        let receipt = runtime_receipt_for_plan()?;
        let mut plan = empty_manifest(&receipt);
        plan.supporting_artifacts = vec![
            crate::oven::rustc::OvenRustcSupportingArtifact {
                relative_path: "deps/libruntime.rlib".to_string(),
                digest: digest_bytes(b"runtime"),
            },
            crate::oven::rustc::OvenRustcSupportingArtifact {
                relative_path: "deps/libruntime.rmeta".to_string(),
                digest: digest_bytes(b"metadata"),
            },
            crate::oven::rustc::OvenRustcSupportingArtifact {
                relative_path: "provenance/legacy-cargo.json".to_string(),
                digest: digest_bytes(b"provenance"),
            },
        ];

        super::discard_seed_metadata_sidecars(&mut plan);

        assert_eq!(
            plan.supporting_artifacts
                .iter()
                .map(|artifact| artifact.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["deps/libruntime.rlib", "provenance/legacy-cargo.json"]
        );
        Ok(())
    }

    #[test]
    fn native_seed_promotes_compiler_runtime_externs_for_compatible_callers() -> Result<(), Box<dyn std::error::Error>>
    {
        let receipt = runtime_receipt_for_plan()?;
        let mut plan = empty_manifest(&receipt);
        plan.entrypoint_externs.insert("generated-root".to_string(), Vec::new());
        plan.supporting_artifacts = vec![
            crate::oven::rustc::OvenRustcSupportingArtifact {
                relative_path: "host/deps/libincan_derive-verified.dylib".to_string(),
                digest: digest_bytes(b"derive macro"),
            },
            crate::oven::rustc::OvenRustcSupportingArtifact {
                relative_path: "target/deps/libincan_core-verified.rlib".to_string(),
                digest: digest_bytes(b"compiler runtime"),
            },
            crate::oven::rustc::OvenRustcSupportingArtifact {
                relative_path: "target/deps/libruntime.rlib".to_string(),
                digest: digest_bytes(b"runtime"),
            },
        ];

        super::promote_compiler_runtime_externs(&mut plan)?;

        assert_eq!(
            plan.externs
                .iter()
                .map(|artifact| artifact.crate_name.as_str())
                .collect::<Vec<_>>(),
            vec!["incan_core", "incan_derive"]
        );
        assert!(plan.externs.iter().any(|artifact| {
            artifact.crate_name == "incan_derive"
                && artifact.relative_path == "host/deps/libincan_derive-verified.dylib"
        }));
        assert!(plan.externs.iter().any(|artifact| {
            artifact.crate_name == "incan_core" && artifact.relative_path == "target/deps/libincan_core-verified.rlib"
        }));
        assert_eq!(
            plan.supporting_artifacts
                .iter()
                .map(|artifact| artifact.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["target/deps/libruntime.rlib"]
        );
        assert_eq!(
            plan.entrypoint_externs.get("generated-root"),
            Some(&vec!["incan_core".to_string(), "incan_derive".to_string()])
        );
        Ok(())
    }

    #[test]
    fn native_seed_keeps_compiler_vocab_helpers_off_generated_root() -> Result<(), Box<dyn std::error::Error>> {
        let receipt = runtime_receipt_for_plan()?;
        let mut plan = empty_manifest(&receipt);
        plan.externs.push(crate::oven::rustc::OvenRustcArtifactExtern {
            crate_name: "incan_stdlib".to_string(),
            relative_path: "target/deps/libincan_stdlib-verified.rlib".to_string(),
            digest: digest_bytes(b"stdlib runtime"),
        });
        plan.supporting_artifacts = vec![
            crate::oven::rustc::OvenRustcSupportingArtifact {
                relative_path: "host/deps/libincan_derive-verified.dylib".to_string(),
                digest: digest_bytes(b"derive macro"),
            },
            crate::oven::rustc::OvenRustcSupportingArtifact {
                relative_path: "target/deps/libincan_core-verified.rlib".to_string(),
                digest: digest_bytes(b"compiler runtime"),
            },
        ];

        super::record_generated_root_externs(&mut plan);
        super::promote_compiler_runtime_externs(&mut plan)?;
        plan.externs.extend([
            crate::oven::rustc::OvenRustcArtifactExtern {
                crate_name: "incan_vocab".to_string(),
                relative_path: "compiler-support/deps/libincan_vocab-verified.rlib".to_string(),
                digest: digest_bytes(b"compiler vocabulary"),
            },
            crate::oven::rustc::OvenRustcArtifactExtern {
                crate_name: "serde_json".to_string(),
                relative_path: "compiler-support/deps/libserde_json-verified.rlib".to_string(),
                digest: digest_bytes(b"compiler json"),
            },
        ]);
        plan.externs
            .sort_by(|left, right| left.crate_name.cmp(&right.crate_name));
        plan.validate_shape(&receipt.intent)?;

        assert_eq!(
            plan.entrypoint_externs.get("generated-root"),
            Some(&vec![
                "incan_core".to_string(),
                "incan_derive".to_string(),
                "incan_stdlib".to_string(),
            ])
        );
        Ok(())
    }

    #[test]
    fn native_seed_accounting_measures_the_final_seed_directory() -> Result<(), Box<dyn std::error::Error>> {
        let seed = tempfile::tempdir()?;
        fs::write(seed.path().join("seed.json"), b"plan")?;
        fs::create_dir(seed.path().join("artifacts"))?;
        fs::write(seed.path().join("artifacts/runtime.rlib"), b"runtime")?;

        let (logical_bytes, physical_bytes) = super::seed_directory_byte_counts(seed.path())?;

        assert_eq!(logical_bytes, 11);
        assert!(physical_bytes >= logical_bytes);
        Ok(())
    }

    #[test]
    fn native_seed_discards_cargo_private_vocab_build_state() -> Result<(), Box<dyn std::error::Error>> {
        let support = tempfile::tempdir()?;
        let cargo_target = support.path().join("target");
        fs::create_dir_all(cargo_target.join("incremental/private"))?;
        fs::write(
            cargo_target.join("incremental/private/dep-graph.bin"),
            b"private build state",
        )?;
        let direct_closure = support.path().join("deps/libincan_vocab.rlib");
        fs::create_dir_all(direct_closure.parent().ok_or("direct closure parent missing")?)?;
        fs::write(&direct_closure, b"declared direct-rustc input")?;

        super::discard_compiler_vocab_build_state(&cargo_target)?;

        assert!(!cargo_target.exists());
        assert_eq!(fs::read(direct_closure)?, b"declared direct-rustc input");
        Ok(())
    }

    #[test]
    fn native_vocab_seed_copies_only_cargo_reported_artifact_closure() -> Result<(), Box<dyn std::error::Error>> {
        let publisher = tempfile::tempdir()?;
        let target_deps = publisher.path().join("target/deps");
        let host_deps = publisher.path().join("host/deps");
        fs::create_dir_all(&target_deps)?;
        fs::create_dir_all(&host_deps)?;
        let reported = [
            ("serde_json", "libserde_json-reported.rlib", b"json".as_slice()),
            (
                "required_transitive",
                "librequired_transitive-reported.rlib",
                b"transitive".as_slice(),
            ),
        ];
        for (_, name, contents) in reported {
            fs::write(target_deps.join(name), contents)?;
        }
        let unreported = target_deps.join("libunrelated_cargo_residue.rlib");
        fs::write(&unreported, b"unreported")?;
        // Cargo reports both the hashed `deps` input and an unhashed convenience
        // copy at the profile root. The latter is publisher output, not a
        // direct-rustc input, and must not expand the sealed seed closure.
        let profile_copy = publisher.path().join("target/libincan_vocab.rlib");
        fs::write(&profile_copy, b"profile copy")?;
        let profile_copy_canonical = fs::canonicalize(&profile_copy)?;
        let mut cargo_output = reported
            .iter()
            .map(|(crate_name, name, _)| {
                serde_json::json!({
                    "reason": "compiler-artifact",
                    "target": { "name": crate_name },
                    "filenames": [target_deps.join(name).display().to_string()],
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        cargo_output.push('\n');
        cargo_output.push_str(
            &serde_json::json!({
                "reason": "compiler-artifact",
                "target": { "name": "incan_vocab" },
                "filenames": [profile_copy.display().to_string()],
            })
            .to_string(),
        );
        let artifacts = super::compiler_artifact_paths_from_cargo_output(
            cargo_output.as_bytes(),
            publisher.path(),
            &[target_deps.as_path(), host_deps.as_path()],
            "incan_vocab",
            publisher.path().join("target").as_path(),
            "native vocabulary fixture",
        )?;
        let seed = tempfile::tempdir()?;
        let receipt = runtime_receipt_for_plan()?;
        let mut plan = empty_manifest(&receipt);

        super::copy_compiler_vocab_support_artifacts(
            &artifacts,
            &target_deps,
            &publisher.path().join("target"),
            &host_deps,
            &seed.path().join("deps"),
            &mut plan,
        )?;

        assert!(!seed.path().join("deps/libunrelated_cargo_residue.rlib").exists());
        assert!(
            artifacts.iter().any(|artifact| artifact == &profile_copy_canonical),
            "the named publisher's reported profile-root rlib must enter the direct-rustc closure"
        );
        assert!(plan.externs.iter().any(|artifact| artifact.crate_name == "incan_vocab"));
        assert!(plan.externs.iter().any(|artifact| artifact.crate_name == "serde_json"));
        assert!(
            plan.supporting_artifacts
                .iter()
                .any(|artifact| artifact.relative_path.ends_with("librequired_transitive-reported.rlib"))
        );
        assert!(
            !plan
                .supporting_artifacts
                .iter()
                .any(|artifact| artifact.relative_path.ends_with("libunrelated_cargo_residue.rlib"))
        );
        Ok(())
    }

    #[test]
    fn native_vocab_seed_rejects_unexpected_profile_root_compiler_artifact() -> Result<(), Box<dyn std::error::Error>> {
        let publisher = tempfile::tempdir()?;
        let target_deps = publisher.path().join("target/deps");
        let host_deps = publisher.path().join("host/deps");
        fs::create_dir_all(&target_deps)?;
        fs::create_dir_all(&host_deps)?;
        let unexpected = publisher.path().join("target/libunrelated.rlib");
        fs::write(&unexpected, b"must not become a sealed input")?;
        let cargo_output = serde_json::json!({
            "reason": "compiler-artifact",
            "target": { "name": "unrelated" },
            "filenames": [unexpected.display().to_string()],
        })
        .to_string();

        let error = super::compiler_artifact_paths_from_cargo_output(
            cargo_output.as_bytes(),
            publisher.path(),
            &[target_deps.as_path(), host_deps.as_path()],
            "incan_vocab",
            publisher.path().join("target").as_path(),
            "native vocabulary fixture",
        )
        .expect_err("unexpected profile-root artifact must fail closed");

        assert!(
            error
                .to_string()
                .contains("escaped its declared dependency directories")
        );
        Ok(())
    }

    fn runtime_receipt(
        source: &Path,
        providers: &str,
        rust_dependencies: &str,
        stdlib_features: &str,
    ) -> Result<crate::oven::OvenReceipt, Box<dyn std::error::Error>> {
        let provider_plan = digest_bytes(providers.as_bytes());
        let mut request = OvenGeneratedProjectRequest::new(
            source.parent().ok_or("source has no parent")?,
            "runtime_fixture",
            "0.1.0",
            "aarch64-apple-darwin",
            "rustc seeded-test",
            "debug",
            Vec::new(),
        )
        .with_generated_source("generated-root", source)
        .with_build_unit_input("runtime-lock", "runtime-lock")
        .with_build_unit_input("rust-dependencies", rust_dependencies)
        .with_build_unit_input("stdlib-features", stdlib_features)
        .with_build_unit_input("provider-plan", provider_plan);
        if !providers.is_empty() {
            request = request.with_build_unit_input("providers", providers);
        }
        Ok(receipt_generated_project(&request)?)
    }

    fn runtime_receipt_for_plan() -> Result<crate::oven::OvenReceipt, Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("main.rs");
        fs::write(&source, "fn main() {}\n")?;
        // The receipt owns no filesystem path, so retaining only its value is valid after this helper drops the
        // temporary source tree.
        runtime_receipt(&source, "", "empty-rust-dependencies", "empty-stdlib-features")
    }

    fn empty_manifest(receipt: &crate::oven::OvenReceipt) -> OvenRustcArtifactManifest {
        OvenRustcArtifactManifest {
            schema_version: OVEN_RUSTC_ARTIFACT_MANIFEST_SCHEMA_VERSION,
            intent: receipt.intent.clone(),
            dependency_search_paths: Vec::new(),
            native_search_paths: Vec::new(),
            externs: Vec::new(),
            entrypoint_externs: BTreeMap::new(),
            registry_leaves: Vec::new(),
            compile_environment: BTreeMap::new(),
            vocab_auxiliary_targets: Vec::new(),
            supporting_artifacts: Vec::new(),
        }
    }
}
