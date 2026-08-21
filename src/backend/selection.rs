//! Backend-selection identity and execution receipt for the #652 replacement-backend cutover.
//!
//! The v0.6 programme tracked by #652 introduces a second compiler backend (the Body IR
//! "replacement" backend, tracked by #653) alongside the current Rust-source-emission backend
//! (`IrCodegen`, `src/backend/ir/`, referred to here as "legacy"). Until the replacement backend
//! actually lands, a build must still be able to declare, record, and report which backend it
//! intended to use and which backend it actually ran — so that a legacy result is never mistaken
//! for a replacement-backend result, and so that a caller who explicitly asks for the replacement
//! backend gets a visible outcome instead of a silent legacy execution.
//!
//! This module owns two boundary types:
//!
//! - [`BackendSelection`]: a versioned, content-identified record of what was decided *before* execution — which
//!   backend, at what implementation revision, for what compatibility profile and source input, for what reason, and
//!   under what fallback policy.
//! - [`BackendExecutionReceipt`]: a versioned, content-identified record of what actually happened *after* execution —
//!   which backend actually ran, whether a declared fallback occurred, the optional shadow-comparison outcome, the
//!   diagnostic-contract version in force, and the produced output's identity.
//!
//! Both types are plain, IO-free data: this module does not invoke `IrCodegen` or any other
//! execution machinery. Callers (the CLI build path today) perform the actual dispatch and use
//! [`resolve_execution`] and [`finalize_receipt`] to turn real outcomes into these records. This
//! keeps the boundary testable without a full compilation pipeline and keeps it reusable by other
//! clients (Oven, `incan inspect`) that only need to read a versioned receipt.
//!
//! This is a different axis from Oven's legacy-Cargo-vs-direct-rustc *build* boundary
//! (`src/oven.rs`, `OvenCompatibilityKind`), which selects how an already-generated artifact is
//! compiled and never influences which compiler backend produced it.

use serde::{Deserialize, Serialize};

/// Current wire format for [`BackendSelection`] and [`BackendExecutionReceipt`].
pub const BACKEND_SELECTION_SCHEMA_VERSION: u32 = 1;

/// Implementation revision of the current Rust-emission ("legacy") backend.
///
/// Independent of [`crate::version::INCAN_VERSION`]: increase it only when a change to the
/// Rust-emission pipeline can change generated output for a previously accepted program, so a
/// consumer keying reuse on this revision knows to invalidate.
pub const LEGACY_BACKEND_REVISION: u32 = 1;

/// Implementation revision of the not-yet-implemented replacement backend.
///
/// #653 owns actual Body IR execution. This constant exists now so that landing #653 only needs
/// to bump a revision and flip [`BackendKind::is_implemented`], not invent a new schema field.
pub const REPLACEMENT_BACKEND_REVISION: u32 = 0;

/// Compiler backend that can produce or execute a build unit.
///
/// This is the codegen-execution axis tracked by #652/#986. See the module docs for how it
/// differs from Oven's build-compilation boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    /// The current Rust-source-emission pipeline (`IrCodegen`, `src/backend/ir/`).
    Legacy,
    /// The Body IR replacement backend tracked by #653. Not yet implemented: selecting it always
    /// produces an explicit, visible outcome (a refusal or a declared fallback), never a silent
    /// legacy execution.
    Replacement,
}

impl BackendKind {
    /// Whether this backend can actually execute a compilation today.
    ///
    /// Only [`BackendKind::Legacy`] is implemented until #653 lands. Callers must consult this
    /// (or pass the equivalent fact into [`resolve_execution`]) instead of assuming a requested
    /// backend can run.
    #[must_use]
    pub fn is_implemented(self) -> bool {
        matches!(self, BackendKind::Legacy)
    }
}

/// Coverage a backend declares for the compilation it was selected for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityProfile {
    /// The backend declares complete coverage of the active release's language/runtime contract.
    /// The legacy backend is always `Full` today.
    Full,
    /// The backend declares only partial coverage. The replacement backend is `Partial` until
    /// #988 proves and expands its execution to the complete v0.6 contract.
    Partial,
}

/// Why a particular backend was selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionReason {
    /// No explicit backend was requested; the compiler-owned default was declared. This is the
    /// "declared legacy capability selection" case: even the default path produces an explicit,
    /// recorded selection rather than an implicit, unrecorded one.
    Default,
    /// The caller explicitly requested this backend, for example `incan build --backend <kind>`.
    ExplicitRequest,
}

/// Declared policy for what happens if the selected backend cannot execute.
///
/// The policy is decided and recorded as part of [`BackendSelection`], before execution is
/// attempted, so a later fallback (or refusal) is always a declared outcome rather than a
/// runtime improvisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackPolicy {
    /// Never silently substitute another backend; an unavailable selection must fail visibly via
    /// [`BackendSelectionError::Refused`].
    Refuse,
    /// If the selected backend cannot execute, fall back to the named backend and record the
    /// substitution explicitly in the execution receipt's `fallback_outcome`.
    AllowTo(BackendKind),
}

/// Optional shadow-comparison outcome between the executed backend and the replacement backend.
///
/// Recorded explicitly whenever a shadow comparison was requested, even when the comparison
/// could not run, so an unavailable comparison is a visible non-green state rather than a
/// silently skipped one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowComparisonState {
    /// No shadow comparison was requested for this compilation.
    NotRequested,
    /// A shadow comparison was requested but could not run, with the concrete reason (for
    /// example, the replacement backend is not implemented yet).
    Unavailable { reason: String },
    /// A shadow comparison ran and the replacement backend's output matched the executed
    /// backend's output under the compilation's comparison rules.
    Matched,
    /// A shadow comparison ran and the outputs diverged, with a human-readable summary of the
    /// difference.
    Diverged { detail: String },
}

/// Whether the backend that actually executed differed from the one declared in the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackOutcome {
    /// The executed backend matches the selected backend; no fallback occurred.
    NotNeeded,
    /// The selected backend could not execute and [`FallbackPolicy::AllowTo`] authorized this
    /// substitution, which is recorded explicitly rather than left implicit.
    Declared { from: BackendKind, to: BackendKind },
}

/// Pre-execution declaration of which backend a compilation will use.
///
/// Built by [`select_backend`] before any codegen or execution starts. `identity` is a content
/// hash over every other field, so a later stage that only holds a serialized copy can call
/// [`BackendSelection::verify_identity`] instead of re-deriving trust from scratch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendSelection {
    /// Selection wire-schema version.
    pub schema_version: u32,
    /// Content-derived `sha256:` identity over every other field.
    pub identity: String,
    /// The backend declared for this compilation.
    pub selected_backend: BackendKind,
    /// Implementation revision of `selected_backend` at selection time.
    pub implementation_revision: u32,
    /// Compatibility coverage `selected_backend` declares for this compilation.
    pub compatibility_profile: CompatibilityProfile,
    /// Content-derived identity of the semantic/source input being compiled.
    pub source_identity: String,
    /// Why `selected_backend` was chosen.
    pub selection_reason: SelectionReason,
    /// What happens if `selected_backend` cannot execute.
    pub fallback_policy: FallbackPolicy,
    /// Whether a shadow comparison against the replacement backend was requested alongside
    /// `selected_backend`'s normal execution.
    pub shadow_requested: bool,
}

/// Post-execution record of how a declared [`BackendSelection`] was actually carried out.
///
/// Binds the backend that actually ran, any declared fallback, the shadow-comparison outcome,
/// the diagnostic-contract version in force, and the produced output's identity into one
/// versioned, content-identified record that Oven and other clients can consume without reading
/// private HIR or Body IR structures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendExecutionReceipt {
    /// Receipt wire-schema version.
    pub schema_version: u32,
    /// Content-derived `sha256:` identity over every other field, including the bound
    /// selection's own identity.
    pub identity: String,
    /// Compiler version that produced this receipt.
    pub compiler_version: String,
    /// The selection this receipt is bound to.
    pub selection: BackendSelection,
    /// The backend that actually executed. May differ from `selection.selected_backend` only
    /// when `fallback_outcome` is [`FallbackOutcome::Declared`].
    pub executed_backend: BackendKind,
    /// Shadow-comparison outcome, always present and explicit even when unavailable.
    pub shadow_comparison: ShadowComparisonState,
    /// Whether a declared fallback occurred.
    pub fallback_outcome: FallbackOutcome,
    /// Diagnostic-contract version in force when this receipt was produced.
    pub diagnostic_contract_version: u32,
    /// Content-derived identity of the produced output or artifact.
    pub output_identity: String,
}

/// Typed failure while selecting a backend or verifying a selection/receipt's content identity.
#[derive(Debug, thiserror::Error)]
pub enum BackendSelectionError {
    /// A persisted selection uses an unsupported schema version.
    #[error("unsupported backend-selection schema version {found}; expected {expected}")]
    UnsupportedSelectionSchema { found: u32, expected: u32 },
    /// A selection's claimed content identity did not match its immutable fields.
    #[error("backend-selection identity mismatch: expected {expected}, got {actual}")]
    SelectionIdentityMismatch { expected: String, actual: String },
    /// A persisted receipt uses an unsupported schema version.
    #[error("unsupported backend-execution-receipt schema version {found}; expected {expected}")]
    UnsupportedReceiptSchema { found: u32, expected: u32 },
    /// A receipt's claimed content identity did not match its immutable fields.
    #[error("backend-execution-receipt identity mismatch: expected {expected}, got {actual}")]
    ReceiptIdentityMismatch { expected: String, actual: String },
    /// The selected backend cannot execute and [`FallbackPolicy::Refuse`] applies, so the build
    /// must stop instead of silently substituting another backend.
    #[error(
        "backend `{backend:?}` was selected but cannot execute yet ({reason}); fallback policy is `refuse`, so the \
         build stops instead of silently running another backend"
    )]
    Refused {
        /// The backend that was selected but could not execute.
        backend: BackendKind,
        /// Concrete reason it could not execute.
        reason: String,
    },
}

/// Declare a backend selection for one compilation, before anything executes.
///
/// `explicit` distinguishes an operator-provided request (for example `incan build --backend
/// <kind>`) from the compiler-owned default, and is the sole input to `selection_reason`.
/// `shadow_requested` records whether a shadow comparison against the replacement backend was
/// also asked for. This function never fails: it only declares intent. Availability is checked
/// separately by [`resolve_execution`], right before dispatch.
#[must_use]
pub fn select_backend(
    requested: BackendKind,
    explicit: bool,
    shadow_requested: bool,
    source_identity: impl Into<String>,
    fallback_policy: FallbackPolicy,
) -> BackendSelection {
    let implementation_revision = match requested {
        BackendKind::Legacy => LEGACY_BACKEND_REVISION,
        BackendKind::Replacement => REPLACEMENT_BACKEND_REVISION,
    };
    let compatibility_profile = match requested {
        BackendKind::Legacy => CompatibilityProfile::Full,
        BackendKind::Replacement => CompatibilityProfile::Partial,
    };
    let selection_reason = if explicit {
        SelectionReason::ExplicitRequest
    } else {
        SelectionReason::Default
    };
    let source_identity = source_identity.into();
    let identity = selection_identity(
        requested,
        implementation_revision,
        compatibility_profile,
        &source_identity,
        selection_reason,
        fallback_policy,
        shadow_requested,
    );
    BackendSelection {
        schema_version: BACKEND_SELECTION_SCHEMA_VERSION,
        identity,
        selected_backend: requested,
        implementation_revision,
        compatibility_profile,
        source_identity,
        selection_reason,
        fallback_policy,
        shadow_requested,
    }
}

/// Decide which backend a caller must actually invoke for a declared selection.
///
/// `selected_available` reports whether `selection.selected_backend` can execute right now (in
/// practice, [`BackendKind::is_implemented`] on that backend). Returns the backend to invoke, or
/// [`BackendSelectionError::Refused`] when the selection is unavailable and
/// [`FallbackPolicy::Refuse`] applies, or when it is unavailable and the declared
/// [`FallbackPolicy::AllowTo`] target is *itself* unimplemented — a fallback to a backend that
/// cannot execute either must still refuse, not silently report the unavailable target as
/// executed. No receipt is produced on refusal: an unavailable selection must stop the build
/// rather than emit a receipt that could be mistaken for a successful replacement-backend result.
pub fn resolve_execution(
    selection: &BackendSelection,
    selected_available: bool,
) -> Result<BackendKind, BackendSelectionError> {
    if selected_available {
        return Ok(selection.selected_backend);
    }
    match selection.fallback_policy {
        FallbackPolicy::Refuse => Err(BackendSelectionError::Refused {
            backend: selection.selected_backend,
            reason: format!(
                "{:?} backend is not available for this compilation",
                selection.selected_backend
            ),
        }),
        FallbackPolicy::AllowTo(target) if target.is_implemented() => Ok(target),
        FallbackPolicy::AllowTo(target) => Err(BackendSelectionError::Refused {
            backend: target,
            reason: format!(
                "declared fallback target {target:?} is not available either; {:?} backend is also unavailable",
                selection.selected_backend
            ),
        }),
    }
}

/// Bind a real execution outcome to its declared selection, producing a versioned receipt.
///
/// `executed_backend` must be the value [`resolve_execution`] returned (or
/// `selection.selected_backend` itself, when it executed directly); this function does not
/// re-validate the fallback policy, it only records what happened. A mismatch between
/// `executed_backend` and `selection.selected_backend` is recorded as
/// [`FallbackOutcome::Declared`].
#[must_use]
pub fn finalize_receipt(
    selection: &BackendSelection,
    executed_backend: BackendKind,
    output_identity: impl Into<String>,
    shadow_comparison: ShadowComparisonState,
    diagnostic_contract_version: u32,
) -> BackendExecutionReceipt {
    let output_identity = output_identity.into();
    let fallback_outcome = if executed_backend == selection.selected_backend {
        FallbackOutcome::NotNeeded
    } else {
        FallbackOutcome::Declared {
            from: selection.selected_backend,
            to: executed_backend,
        }
    };
    let compiler_version = crate::version::INCAN_VERSION.to_string();
    let identity = receipt_identity(
        &selection.identity,
        &compiler_version,
        executed_backend,
        &shadow_comparison,
        fallback_outcome,
        diagnostic_contract_version,
        &output_identity,
    );
    BackendExecutionReceipt {
        schema_version: BACKEND_SELECTION_SCHEMA_VERSION,
        identity,
        compiler_version,
        selection: selection.clone(),
        executed_backend,
        shadow_comparison,
        fallback_outcome,
        diagnostic_contract_version,
        output_identity,
    }
}

impl BackendSelection {
    /// Recompute this selection's content identity and confirm it matches `self.identity`.
    ///
    /// A later stage that only holds a serialized `BackendSelection` (for example, one persisted
    /// alongside an Oven cache entry) must call this before trusting it, the same way
    /// [`crate::oven::OvenReceipt::verify_identity`] guards a persisted Oven receipt.
    pub fn verify_identity(&self) -> Result<(), BackendSelectionError> {
        if self.schema_version != BACKEND_SELECTION_SCHEMA_VERSION {
            return Err(BackendSelectionError::UnsupportedSelectionSchema {
                found: self.schema_version,
                expected: BACKEND_SELECTION_SCHEMA_VERSION,
            });
        }
        let actual = selection_identity(
            self.selected_backend,
            self.implementation_revision,
            self.compatibility_profile,
            &self.source_identity,
            self.selection_reason,
            self.fallback_policy,
            self.shadow_requested,
        );
        if actual != self.identity {
            return Err(BackendSelectionError::SelectionIdentityMismatch {
                expected: self.identity.clone(),
                actual,
            });
        }
        Ok(())
    }
}

impl BackendExecutionReceipt {
    /// Recompute this receipt's content identity (and its bound selection's identity) and
    /// confirm both match their recorded values.
    pub fn verify_identity(&self) -> Result<(), BackendSelectionError> {
        self.selection.verify_identity()?;
        if self.schema_version != BACKEND_SELECTION_SCHEMA_VERSION {
            return Err(BackendSelectionError::UnsupportedReceiptSchema {
                found: self.schema_version,
                expected: BACKEND_SELECTION_SCHEMA_VERSION,
            });
        }
        let actual = receipt_identity(
            &self.selection.identity,
            &self.compiler_version,
            self.executed_backend,
            &self.shadow_comparison,
            self.fallback_outcome,
            self.diagnostic_contract_version,
            &self.output_identity,
        );
        if actual != self.identity {
            return Err(BackendSelectionError::ReceiptIdentityMismatch {
                expected: self.identity.clone(),
                actual,
            });
        }
        Ok(())
    }
}

/// Digest the fields that make up a [`BackendSelection`]'s content identity.
fn selection_identity(
    selected_backend: BackendKind,
    implementation_revision: u32,
    compatibility_profile: CompatibilityProfile,
    source_identity: &str,
    selection_reason: SelectionReason,
    fallback_policy: FallbackPolicy,
    shadow_requested: bool,
) -> String {
    digest_content(&format!(
        "{selected_backend:?}\n{implementation_revision}\n{compatibility_profile:?}\n{source_identity}\n\
         {selection_reason:?}\n{fallback_policy:?}\n{shadow_requested}\n"
    ))
}

/// Digest the fields that make up a [`BackendExecutionReceipt`]'s content identity.
///
/// Takes the bound selection's own `identity` rather than its full field set, so tampering with
/// any selection field is caught by [`BackendSelection::verify_identity`] and any tampering with
/// the receipt's own fields (including swapping in a different, validly-identified selection) is
/// caught here.
fn receipt_identity(
    selection_identity: &str,
    compiler_version: &str,
    executed_backend: BackendKind,
    shadow_comparison: &ShadowComparisonState,
    fallback_outcome: FallbackOutcome,
    diagnostic_contract_version: u32,
    output_identity: &str,
) -> String {
    digest_content(&format!(
        "{selection_identity}\n{compiler_version}\n{executed_backend:?}\n{shadow_comparison:?}\n\
         {fallback_outcome:?}\n{diagnostic_contract_version}\n{output_identity}\n"
    ))
}

/// Render a `sha256:`-prefixed hex digest of `content`.
///
/// Delegates to [`crate::oven::digest_bytes`] rather than hashing independently, so
/// `BackendSelection`/`BackendExecutionReceipt` identities share exactly one hashing
/// implementation with `OvenReceipt` identities instead of two that could drift apart.
fn digest_content(content: &str) -> String {
    crate::oven::digest_bytes(content.as_bytes())
}

/// Digest one or more content fragments into a single content-derived identity.
///
/// Exposed for callers that need to turn real source or generated-output text into a
/// `source_identity` or `output_identity` without duplicating the hashing scheme this module
/// uses internally. `parts` must be presented in a stable, caller-chosen order (for example,
/// sorted by module path) so the identity does not depend on incidental iteration order, such as
/// `HashMap` iteration over multi-file codegen output.
#[must_use]
pub fn digest_output(parts: &[&str]) -> String {
    digest_content(&parts.join("\u{1}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refuse_selection(requested: BackendKind, explicit: bool) -> BackendSelection {
        select_backend(requested, explicit, false, "sha256:source", FallbackPolicy::Refuse)
    }

    #[test]
    fn declared_legacy_capability_selection_is_explicit() -> Result<(), BackendSelectionError> {
        let selection = refuse_selection(BackendKind::Legacy, false);
        assert_eq!(selection.selected_backend, BackendKind::Legacy);
        assert_eq!(selection.selection_reason, SelectionReason::Default);
        assert_eq!(selection.compatibility_profile, CompatibilityProfile::Full);
        assert_eq!(selection.implementation_revision, LEGACY_BACKEND_REVISION);
        selection.verify_identity()?;

        let executed = resolve_execution(&selection, true)?;
        assert_eq!(executed, BackendKind::Legacy);

        let receipt = finalize_receipt(
            &selection,
            executed,
            "sha256:output",
            ShadowComparisonState::NotRequested,
            1,
        );
        assert_eq!(receipt.fallback_outcome, FallbackOutcome::NotNeeded);
        assert_eq!(receipt.executed_backend, BackendKind::Legacy);
        receipt.verify_identity()?;
        Ok(())
    }

    #[test]
    fn replacement_selection_refuses_visibly_when_unavailable() {
        let selection = refuse_selection(BackendKind::Replacement, true);
        assert_eq!(selection.selection_reason, SelectionReason::ExplicitRequest);
        assert_eq!(selection.compatibility_profile, CompatibilityProfile::Partial);

        let Err(error) = resolve_execution(&selection, false) else {
            panic!("replacement backend is not implemented yet; selection must be refused");
        };
        match error {
            BackendSelectionError::Refused { backend, .. } => assert_eq!(backend, BackendKind::Replacement),
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn fallback_refusal_never_produces_a_receipt() {
        // `resolve_execution` returning `Err` is the only outcome for a refused fallback: there is
        // no code path in this module that can turn a refusal into a `BackendExecutionReceipt`,
        // so a refused build cannot be mistaken for a green replacement-backend result.
        let selection = refuse_selection(BackendKind::Replacement, true);
        assert!(resolve_execution(&selection, false).is_err());
    }

    #[test]
    fn declared_fallback_is_recorded_explicitly() -> Result<(), BackendSelectionError> {
        let selection = select_backend(
            BackendKind::Replacement,
            true,
            false,
            "sha256:source",
            FallbackPolicy::AllowTo(BackendKind::Legacy),
        );
        let executed = resolve_execution(&selection, false)?;
        assert_eq!(executed, BackendKind::Legacy);

        let receipt = finalize_receipt(
            &selection,
            executed,
            "sha256:output",
            ShadowComparisonState::NotRequested,
            1,
        );
        assert_eq!(
            receipt.fallback_outcome,
            FallbackOutcome::Declared {
                from: BackendKind::Replacement,
                to: BackendKind::Legacy,
            }
        );
        receipt.verify_identity()?;
        Ok(())
    }

    #[test]
    fn declared_fallback_to_an_unavailable_target_still_refuses() {
        // A fallback target that is itself unimplemented must never be reported as "executed":
        // otherwise a build that actually ran legacy could claim the replacement backend
        // executed cleanly with no fallback (`executed_backend == selected_backend`).
        let selection = select_backend(
            BackendKind::Replacement,
            true,
            false,
            "sha256:source",
            FallbackPolicy::AllowTo(BackendKind::Replacement),
        );
        let Err(error) = resolve_execution(&selection, false) else {
            panic!("a fallback target that is itself unavailable must still be refused");
        };
        match error {
            BackendSelectionError::Refused { backend, .. } => assert_eq!(backend, BackendKind::Replacement),
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn shadow_comparison_unavailability_is_recorded_not_skipped() -> Result<(), BackendSelectionError> {
        let selection = select_backend(BackendKind::Legacy, true, true, "sha256:source", FallbackPolicy::Refuse);
        assert!(selection.shadow_requested);

        let executed = resolve_execution(&selection, true)?;
        let shadow_comparison = ShadowComparisonState::Unavailable {
            reason: "replacement backend not implemented yet (#653)".to_string(),
        };
        let receipt = finalize_receipt(&selection, executed, "sha256:output", shadow_comparison.clone(), 1);
        assert_eq!(receipt.shadow_comparison, shadow_comparison);
        receipt.verify_identity()?;
        Ok(())
    }

    #[test]
    fn mismatched_receipt_identity_is_detected() -> Result<(), BackendSelectionError> {
        let selection = refuse_selection(BackendKind::Legacy, false);
        let mut receipt = finalize_receipt(
            &selection,
            BackendKind::Legacy,
            "sha256:output",
            ShadowComparisonState::NotRequested,
            1,
        );
        receipt.verify_identity()?;

        // Tamper with the recorded output identity without recomputing `receipt.identity`, the
        // same class of divergence a stale or hand-edited receipt on disk would exhibit.
        receipt.output_identity = "sha256:tampered".to_string();
        let Err(error) = receipt.verify_identity() else {
            panic!("tampered output identity must be detected");
        };
        assert!(matches!(error, BackendSelectionError::ReceiptIdentityMismatch { .. }));
        Ok(())
    }

    #[test]
    fn mismatched_selection_identity_is_detected() {
        let mut selection = refuse_selection(BackendKind::Legacy, false);
        selection.source_identity = "sha256:different-source".to_string();
        let Err(error) = selection.verify_identity() else {
            panic!("tampered source identity must be detected");
        };
        assert!(matches!(error, BackendSelectionError::SelectionIdentityMismatch { .. }));
    }
}
