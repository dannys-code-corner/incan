//! Oven-owned legacy execution for the bounded #1146 shadow comparison.
//!
//! Parity evidence must come from the adopted build and execution boundary, not from an ad-hoc compiler
//! invocation. The legacy route therefore runs the same way an ordinary Incan build does: the emitted Rust is
//! authorized by an [`OvenReceipt`], an immutable direct-`rustc` plan is selected from the bounded
//! [`OvenStore`] against that receipt's reusable build unit, Oven compiles the caller-owned source with no Cargo
//! process, and the produced binary is executed. Every one of those authority facts is carried back in a
//! [`LegacyExecutionAuthority`] and folded into the legacy receipt's output identity.
//!
//! ## Where the build unit comes from
//!
//! An Oven receipt's `build_unit_identity` is derived from build intent, compatibility envelope, and build-unit
//! inputs — never from the generated source. A comparison therefore cannot invent its own build unit and expect a
//! stored plan to match it; it must adopt the intent and build-unit inputs of a project whose plan was already
//! published by an explicit `incan oven bake`. [`LegacyOvenCapability::adopt_baked_project`] does exactly that:
//! it reads and verifies a real Oven receipt, keeps its intent and build-unit inputs, and replaces only the
//! generated-source evidence with this comparison's own program. The source bytes stay caller-owned and are
//! re-authorized every run; the native closure stays store-owned and immutable.
//!
//! Where no such capability is staged, the legacy route is honestly unavailable. It is never approximated by
//! calling a compiler directly, because an unauthorized build would produce a result no receipt can account for.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::oven::rustc::{
    OvenStoredDirectRustcRunRequest, bake_stored_direct_rustc_run, select_direct_rustc_plan_for_execution,
};
use crate::oven::store::{OvenStore, OvenStoreLimits};
use crate::oven::{
    DEFAULT_OVEN_MAX_DOMAIN_LOGICAL_BYTES, DEFAULT_OVEN_MAX_DOMAIN_PHYSICAL_BYTES, DEFAULT_OVEN_MAX_PHYSICAL_BYTES,
    OvenGeneratedProjectRequest, OvenReceipt, receipt_generated_project,
};

use super::{
    LegacyExecutionAuthority, LegacyRouteResult, ShadowComparisonProfile, ShadowUnavailable, emit_legacy_rust,
    observe_legacy_process,
};

/// Receipt evidence key under which the comparison's emitted Rust is authorized.
///
/// Matches the key a normal generated executable build uses, so the stored plan authorizes the same kind of
/// caller-owned root source rather than a comparison-specific shape.
const SOURCE_EVIDENCE_KEY: &str = "generated-root";

/// Project name recorded in the comparison's Oven receipt.
const LEGACY_PROJECT_NAME: &str = "incan-shadow-comparison";

/// Rust crate name for the produced legacy program.
const LEGACY_CRATE_NAME: &str = "incan_shadow_comparison";

/// Everything needed to run one legacy route under Oven authority.
///
/// The intent and build-unit inputs are adopted from an already-baked project so a stored direct-`rustc` plan can
/// be selected; only the generated source is this comparison's own. The compiler is explicit, never resolved from
/// `PATH`, matching Oven's rule that a hidden selector must not decide which compiler produced evidence.
#[derive(Debug, Clone)]
pub struct LegacyOvenCapability {
    store_root: PathBuf,
    rustc: PathBuf,
    baked_receipt: OvenReceipt,
}

impl LegacyOvenCapability {
    /// Adopt an already-baked project's build unit as the authority for comparison builds.
    ///
    /// `baked_receipt_path` is a persisted Oven receipt — in practice `.incan/oven/receipt.json` from a project
    /// that has been through `incan oven bake`. It is parsed and identity-verified before it can authorize
    /// anything; a receipt that does not verify is refused rather than trusted, exactly as
    /// `incan oven run` treats its own input.
    pub fn adopt_baked_project(
        store_root: impl Into<PathBuf>,
        rustc: impl Into<PathBuf>,
        baked_receipt_path: &Path,
    ) -> Result<Self, ShadowUnavailable> {
        let bytes = std::fs::read(baked_receipt_path).map_err(|error| {
            ShadowUnavailable::new(format!(
                "the legacy route has no Oven authority: cannot read {}: {error}",
                baked_receipt_path.display()
            ))
        })?;
        let baked_receipt: OvenReceipt = serde_json::from_slice(&bytes).map_err(|error| {
            ShadowUnavailable::new(format!(
                "the legacy route's Oven receipt {} could not be parsed: {error}",
                baked_receipt_path.display()
            ))
        })?;
        baked_receipt.verify_identity().map_err(|error| {
            ShadowUnavailable::new(format!(
                "the legacy route's Oven receipt {} failed identity verification: {error}",
                baked_receipt_path.display()
            ))
        })?;
        Ok(Self {
            store_root: store_root.into(),
            rustc: rustc.into(),
            baked_receipt,
        })
    }

    /// Build intent adopted for comparison builds.
    #[must_use]
    pub fn intent(&self) -> &crate::oven::OvenBuildIntent {
        &self.baked_receipt.intent
    }

    /// The verified Oven receipt whose build unit this capability adopted.
    ///
    /// Exposed so a report can name the authority a comparison ran under, and so a caller can prove the
    /// capability refuses a receipt that does not verify.
    #[must_use]
    pub fn adopted_receipt(&self) -> &OvenReceipt {
        &self.baked_receipt
    }

    /// Derive the comparison's own Oven receipt over one emitted Rust file.
    ///
    /// The adopted intent and build-unit inputs are preserved so the reusable build unit — and therefore the
    /// stored plan — stays the baked project's. Only the source evidence is this comparison's, so the receipt
    /// authorizes exactly the bytes that will be compiled.
    fn receipt_for_generated_source(
        &self,
        project_root: &Path,
        generated_source: &Path,
    ) -> Result<OvenReceipt, ShadowUnavailable> {
        let intent = &self.baked_receipt.intent;
        let mut request = OvenGeneratedProjectRequest::new(
            project_root,
            LEGACY_PROJECT_NAME,
            self.baked_receipt.project.version.clone(),
            intent.target.clone(),
            intent.toolchain.clone(),
            intent.profile.clone(),
            intent.features.clone(),
        )
        .with_generated_source(SOURCE_EVIDENCE_KEY, generated_source);
        for (name, value) in &self.baked_receipt.sources.build_unit_inputs {
            request = request.with_build_unit_input(name.clone(), value.clone());
        }
        receipt_generated_project(&request).map_err(|error| {
            ShadowUnavailable::new(format!(
                "the legacy route could not receipt its generated source through Oven: {error}"
            ))
        })
    }

    /// Open the bounded Oven store this capability selects plans from.
    fn open_store(&self) -> OvenStore {
        OvenStore::new(
            &self.store_root,
            OvenStoreLimits::new(
                DEFAULT_OVEN_MAX_PHYSICAL_BYTES,
                DEFAULT_OVEN_MAX_DOMAIN_PHYSICAL_BYTES,
                DEFAULT_OVEN_MAX_DOMAIN_LOGICAL_BYTES,
            ),
        )
    }
}

/// Run the legacy route for one profile through Oven and observe what the produced program did.
///
/// The steps mirror `incan oven run`: receipt the caller-owned source, select an immutable plan authorized by
/// that receipt's build unit, compile without a Cargo process, then execute. A failure at any step before
/// execution is [`ShadowUnavailable`] — the program was never observed, so there is nothing to compare, and a
/// build failure must never be promoted into a claim about program meaning.
pub(crate) fn observe_legacy_route(
    profile: &ShadowComparisonProfile,
    capability: &LegacyOvenCapability,
    workspace: &Path,
) -> Result<LegacyRouteResult, ShadowUnavailable> {
    let program = profile.legacy_program_source()?;
    let rust_source = emit_legacy_rust(&program)?;

    let project_root = workspace.join("oven-project");
    let source_path = project_root.join("src").join("main.rs");
    let output_path = project_root.join(LEGACY_CRATE_NAME);
    write_generated_source(&source_path, &rust_source)?;

    let receipt = capability.receipt_for_generated_source(&project_root, &source_path)?;
    let store = capability.open_store();
    let selected = select_direct_rustc_plan_for_execution(&store, &receipt)
        .map_err(|error| {
            ShadowUnavailable::new(format!(
                "the legacy route could not select an Oven direct-rustc plan: {error}"
            ))
        })?
        .ok_or_else(|| {
            ShadowUnavailable::new(format!(
                "no immutable Oven direct-rustc plan is staged for build unit {}; bake the adopted project once \
                 with `incan oven bake` before a legacy comparison route can run",
                receipt.build_unit_identity
            ))
        })?;
    let plan_identity = selected.manifest.identity.clone();

    let bake = bake_stored_direct_rustc_run(&OvenStoredDirectRustcRunRequest {
        store: &store,
        plan_identity: plan_identity.clone(),
        receipt: receipt.clone(),
        rustc: capability.rustc.clone(),
        source: source_path.clone(),
        output: output_path.clone(),
        crate_name: LEGACY_CRATE_NAME.to_string(),
        edition: crate::backend::project::cargo_toml::DEFAULT_GENERATED_RUST_EDITION.to_string(),
        source_evidence_key: SOURCE_EVIDENCE_KEY.to_string(),
    })
    .map_err(|error| {
        ShadowUnavailable::new(format!(
            "Oven did not build the legacy program, so it was never observed: {error}"
        ))
    })?;

    let authority = LegacyExecutionAuthority {
        oven_receipt_identity: receipt.identity.clone(),
        oven_build_unit_identity: receipt.build_unit_identity.clone(),
        direct_rustc_plan_identity: plan_identity,
        output_digest: bake.output_digest.clone(),
        cargo_process_started: bake.cargo_process_started,
    };
    if authority.cargo_process_started {
        return Err(ShadowUnavailable::new(
            "a Cargo process participated in the legacy build, so the result is not Oven-owned execution evidence"
                .to_string(),
        ));
    }

    let mut command = Command::new(&bake.output);
    clear_inherited_cargo_environment(&mut command);
    let run = command.output().map_err(|error| {
        ShadowUnavailable::new(format!(
            "the legacy route could not run the Oven-produced program {}: {error}",
            bake.output.display()
        ))
    })?;
    let stderr = String::from_utf8_lossy(&run.stderr).trim().to_string();
    let observation = observe_legacy_process(
        profile.profile_kind(),
        &profile.profile_identity(),
        &authority,
        run.status.code(),
        &run.stdout,
        &stderr,
    )?;
    Ok(LegacyRouteResult { observation, authority })
}

/// Write the emitted Rust into the comparison's caller-owned project tree.
fn write_generated_source(source_path: &Path, rust_source: &str) -> Result<(), ShadowUnavailable> {
    let parent = source_path.parent().ok_or_else(|| {
        ShadowUnavailable::new(format!(
            "the legacy route's generated source path {} has no parent directory",
            source_path.display()
        ))
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        ShadowUnavailable::new(format!(
            "the legacy route could not create {}: {error}",
            parent.display()
        ))
    })?;
    std::fs::write(source_path, rust_source).map_err(|error| {
        ShadowUnavailable::new(format!(
            "the legacy route could not write {}: {error}",
            source_path.display()
        ))
    })
}

/// Remove inherited Cargo process variables before running the Oven-produced program.
///
/// Mirrors what `incan oven run` does: an Oven-owned execution must not observe a surrounding Cargo invocation's
/// environment, or its behavior could depend on how the comparison happened to be launched.
fn clear_inherited_cargo_environment(command: &mut Command) {
    let inherited: Vec<String> = std::env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .filter(|name| name == "CARGO" || name.starts_with("CARGO_"))
        .collect();
    for name in inherited {
        command.env_remove(name);
    }
}

/// Named environment variables that stage a legacy comparison capability.
///
/// Kept as one table so the contract is stated once, and so an operator can see exactly what must be provided
/// before a comparison can run.
pub const CAPABILITY_ENVIRONMENT: &[(&str, &str)] = &[
    (
        INCAN_HOME_ENV,
        "Incan home whose bounded Oven store holds a published direct-rustc plan",
    ),
    (
        RECEIPT_ENV,
        "path to a verified Oven receipt from a project already baked with `incan oven bake`",
    ),
    (RUSTC_ENV, "explicit Rust compiler executable Oven must use"),
];

/// Incan home whose bounded Oven store the comparison selects plans from.
const INCAN_HOME_ENV: &str = "INCAN_SHADOW_OVEN_HOME";

/// Persisted Oven receipt whose build unit the comparison adopts.
const RECEIPT_ENV: &str = "INCAN_SHADOW_OVEN_RECEIPT";

/// Explicit compiler Oven must use for the comparison build.
const RUSTC_ENV: &str = "INCAN_SHADOW_RUSTC";

impl LegacyOvenCapability {
    /// Resolve a capability from the environment variables listed in [`CAPABILITY_ENVIRONMENT`].
    ///
    /// Returns [`ShadowUnavailable`] naming the first missing variable, so an unstaged environment produces an
    /// actionable non-green reason instead of a silent skip.
    pub fn from_environment() -> Result<Self, ShadowUnavailable> {
        let read = |name: &str| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| {
                    ShadowUnavailable::new(format!(
                        "the legacy comparison route is not staged: {name} is unset; it must name the {}",
                        CAPABILITY_ENVIRONMENT
                            .iter()
                            .find(|(variable, _)| *variable == name)
                            .map_or("required input", |(_, description)| *description)
                    ))
                })
        };
        let incan_home = read(INCAN_HOME_ENV)?;
        let receipt_path = read(RECEIPT_ENV)?;
        let rustc = read(RUSTC_ENV)?;
        Self::adopt_baked_project(
            crate::oven::store::store_root_for_home(&incan_home),
            rustc,
            &receipt_path,
        )
    }
}
