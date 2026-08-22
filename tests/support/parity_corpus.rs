//! Schema, validation, and CI-summary emission for the #987 backend-cutover parity corpus.
//!
//! This module turns the #646 backend behavior inventory (`workspaces/docs-site/docs/contributing/reference/
//! backend_behavior_inventory.md`) into an executable shape. Each [`ParityCase`] is a stable, identified claim
//! about current compiler behavior, tagged with the inventory category and evidence lane that justifies it, plus
//! an explicit disposition for the 0.6 backend cutover (`#652`): preserved, intentionally migrated, or unsupported
//! with an owning issue.
//!
//! Two things this module deliberately refuses to do:
//!
//! - Treat generated Rust token shape as semantic authority. A case's [`ParityCase::evaluate`] function must compare
//!   frontend-observable, user-visible outcomes (typecheck acceptance/rejection, diagnostic presence, runtime helper
//!   results) even when its evidence lane is [`EvidenceLane::CodegenSnapshot`]; that lane may only assert that
//!   generation succeeds and stays syntactically valid Rust, never that a particular token layout is the contract.
//! - Default anything to green. [`ComparisonOutcome`] and [`ReceiptRef`] both carry explicit non-`Match`/ non-available
//!   states so that unavailable, skipped, or incompatible comparisons are visible in the emitted summary rather than
//!   silently counted as parity.
//!
//! ## Receipt-awareness (#986, landed via PR #1120)
//!
//! #987's own scope calls for "receipt-aware reference/replacement or shadow comparisons where both paths are
//! available." #986 landed [`incan::backend::selection`], so [`evaluate_case`] declares a real
//! [`BackendSelection`], resolves it, and finalizes a real [`BackendExecutionReceipt`] for every case's source —
//! the same three-call sequence `src/cli/commands/build.rs` uses for an actual build. The first #988 Body-IR cases
//! additionally execute the replacement backend and carry their own selection/execution receipt, Body-IR snapshot,
//! ownership evidence, and runtime requirements. Their requested shadow comparison remains genuinely
//! [`ShadowComparisonState::Unavailable`] because no source-observable legacy comparator exists; generated Rust is
//! never substituted as semantic proof.

use incan::backend::replacement::{
    OwnershipReadProjection, ReplacementValue, RuntimeRequirementProjection, execute_prevalidated_free_function,
    prepare_free_function_execution,
};
use incan::backend::selection::{
    BackendExecutionReceipt, BackendKind, BackendSelection, BackendSelectionError, FallbackPolicy,
    ShadowComparisonState, digest_output, finalize_receipt, resolve_execution, select_backend,
    unavailable_shadow_comparison,
};
use incan::frontend::body_ir::build_body_ir_module_v0;
use incan::frontend::diagnostics::DIAGNOSTIC_SCHEMA_VERSION;
use incan::frontend::typechecker::TypeChecker;
use incan::frontend::{lexer, parser};
use incan_semantics_core::body_ir::BodyIrModule;
use std::collections::BTreeSet;

// ============================================================================
// Case schema
// ============================================================================

/// One of the seven behavior categories from the #646 inventory's `Categories` table.
///
/// The category names and meanings are the inventory's, not invented here — keep this enum in sync with
/// `workspaces/docs-site/docs/contributing/reference/backend_behavior_inventory.md` if that table changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BehaviorCategory {
    /// Documented or intentionally exposed source-level Incan semantics.
    SupportedLanguageContract,
    /// Behavior owned by `crates/incan_stdlib`, `crates/incan_core`, or `.incn` stdlib source.
    StdlibRuntimeBehavior,
    /// Behavior crossing `rust::` imports, rust-inspect metadata, or generated Cargo projects.
    ///
    /// No seed case uses this category yet: #987's own plan step 3 asks for a narrow source-only seed corpus
    /// before Rust-interop rows (step 4). Kept referenced here so the category taxonomy stays complete ahead of
    /// that growth — it requires an executable interop boundary and receipt-aware comparison.
    #[allow(dead_code)]
    RustInteropBehavior,
    /// Behavior visible mainly through generated Rust shape, manifests, or `target/incan/**` layout.
    GeneratedArtifactBehavior,
    /// Error/warning codes, spans, JSON schema facts, and text diagnostics.
    DiagnosticBehavior,
    /// Accepted only because a parser/typechecker/lowering path happens to allow it without a documented contract.
    AccidentalAcceptedBehavior,
    /// Preserved only because current users may rely on a workaround, or fixing it needs a larger migration.
    BugCompatibleBehavior,
}

/// One of the six evidence lanes from the #646 inventory's `Evidence lanes` table.
///
/// A behavior can be proven from more than one lane; [`ParityCase`] records the primary lane the case's
/// [`ParityCase::evaluate`] function actually exercises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceLane {
    /// `src/frontend/**` unit tests, diagnostics tests, parser snapshots — source acceptance/rejection.
    DirectParserTypechecker,
    /// `tests/codegen_snapshot_tests.rs`, `tests/snapshots/**` — current generated Rust shape.
    CodegenSnapshot,
    /// Integration tests, stdlib runtime tests, smoke tests — compiled/runtime behavior.
    GeneratedProjectRun,
    /// Typed source lowered to Body IR and executed directly by the bounded replacement profile.
    DirectReplacementBodyIr,
    /// Package consumer fixtures, facade/reexport tests, checked API metadata tests.
    ///
    /// No seed case uses this lane yet — deferred to plan step 4 alongside `RustInteropBehavior`.
    #[allow(dead_code)]
    PackageImportBoundary,
    /// Vocab desugarer tests, formatter/test-runner activation paths.
    ///
    /// No seed case uses this lane yet — deferred to plan step 4.
    #[allow(dead_code)]
    VocabTestBatch,
    /// IncQL or Hees.ai acceptance runs when the surface is exercised there.
    ///
    /// No seed case uses this lane yet — deferred to plan step 4.
    #[allow(dead_code)]
    DownstreamProof,
}

/// The cutover disposition for one case, per #987's own "Done when" contract.
///
/// This is deliberately restricted to the three states #987 names explicitly: preserved, intentional migration, or
/// unsupported. There is no fourth "undecided" variant — a case that has not been triaged yet must still pick
/// [`Disposition::Unsupported`] or [`Disposition::IntentionalMigration`] with a real owning issue and a migration
/// note that says what triage is still needed, rather than avoiding the decision.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Disposition {
    /// The behavior is a documented or evidenced contract that the 0.6 backend cutover must keep working.
    Preserved,
    /// The behavior will change deliberately during cutover; a real issue tracks the migration.
    ///
    /// No seed case uses this variant yet — the seed corpus only needed `Preserved` and `Unsupported` so far.
    /// Kept referenced so the three-state disposition contract (#987's own "Done when") stays visible in the type
    /// even before a migration-flagged case is added.
    #[allow(dead_code)]
    IntentionalMigration {
        owning_issue: u32,
        migration_note: &'static str,
    },
    /// The behavior is not guaranteed to survive cutover as-is; a real issue tracks the decision.
    Unsupported {
        owning_issue: u32,
        migration_note: &'static str,
    },
}

impl Disposition {
    /// Return the disposition's serialized tag without allocating, for compact summary rows.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Disposition::Preserved => "preserved",
            Disposition::IntentionalMigration { .. } => "intentional_migration",
            Disposition::Unsupported { .. } => "unsupported",
        }
    }
}

/// The result of actually consulting the #986 backend-selection receipt for one case's source.
///
/// Every variant carries the real [`BackendExecutionReceipt::identity`] this case produced — never a guessed or
/// presence-checked value — so a consumer can independently re-verify which receipt a case's `overall_state` is
/// derived from.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum ReceiptRef {
    /// A real legacy receipt was produced, but its requested source-observable comparison is unavailable. This is
    /// the honest state for pre-#988 seed rows that do not yet own a direct replacement execution. See
    /// [`OverallState::NonGreenShadowUnavailable`].
    ShadowUnavailable { receipt_identity: String, reason: String },
    /// A #988 replacement execution with its own #986 selection/execution receipt and Body-IR evidence.
    ///
    /// The comparison remains non-green because the requested source-observable legacy comparator is unavailable;
    /// this variant proves replacement execution without promoting it to parity.
    ReplacementExecuted {
        /// Identity of the pre-execution replacement selection.
        selection_identity: String,
        /// Identity of the finalized replacement execution receipt.
        receipt_identity: String,
        /// Identity of the direct Body-IR output bound into that receipt.
        output_identity: String,
        /// Deterministic snapshot of the Body IR the replacement executor consumed.
        body_snapshot: String,
        /// Canonical ownership facts observed during direct execution.
        ownership_reads: Vec<OwnershipReadProjection>,
        /// Canonical Body-IR runtime requirements observed by direct execution.
        runtime_requirements: Vec<RuntimeRequirementProjection>,
        /// Concrete reason the intentionally requested semantic comparison is non-green.
        comparison_reason: String,
    },
    /// A real receipt's source-observable shadow comparison against the replacement backend matched. Not reachable
    /// until a paired legacy runtime comparator is implemented; kept so the schema does not need another shape
    /// change when it does.
    #[allow(dead_code)]
    ShadowMatched { receipt_identity: String },
    /// A real receipt's shadow comparison against the replacement backend diverged — a genuine regression signal
    /// on the backend-selection axis itself, not a schema gap. Not reachable until a paired legacy runtime
    /// comparator exists.
    #[allow(dead_code)]
    ShadowDiverged { receipt_identity: String, detail: String },
    /// The backend-selection API itself returned an error while declaring or resolving a selection for this
    /// case's source. Defensive: with today's fixed `(Legacy, FallbackPolicy::Refuse)` inputs this cannot actually
    /// happen, but the corpus must not silently treat an API error as an available, green-eligible receipt.
    #[allow(dead_code)]
    SelectionError { detail: String },
}

/// Declare, resolve, and finalize the legacy-side #986 receipt retained by the pre-#988 seed rows.
fn compute_legacy_receipt(source: &str) -> ReceiptRef {
    let source_identity = digest_output(&[source]);
    let selection = select_backend(
        BackendKind::Legacy,
        false,
        true,
        source_identity,
        FallbackPolicy::Refuse,
    );
    let executed = match resolve_execution(&selection, BackendKind::Legacy.is_implemented()) {
        Ok(backend) => backend,
        Err(error) => return receipt_ref_from_error(&error),
    };
    let shadow_comparison = shadow_comparison_for(&selection);
    let output_identity = digest_output(&[source]);
    let receipt: Result<BackendExecutionReceipt, BackendSelectionError> = finalize_receipt(
        &selection,
        executed,
        output_identity,
        shadow_comparison,
        DIAGNOSTIC_SCHEMA_VERSION,
    );
    match receipt {
        Ok(receipt) => receipt_ref_from_receipt(receipt),
        Err(error) => receipt_ref_from_error(&error),
    }
}

/// The behavior result and #986 receipt produced by one direct replacement execution.
///
/// The two values are inseparable evidence: the value comes from the same selected, validated Body-IR execution
/// whose output identity is finalized into `receipt`.
struct ReplacementPlanEvidence {
    behavior_outcome: ComparisonOutcome,
    receipt: ReceiptRef,
}

/// Execute one #988 plan once, through #986 selection, and bind the observed Body-IR result to its receipt.
fn execute_replacement_plan(source: &str, plan: ReplacementExecutionPlan) -> ReplacementPlanEvidence {
    let arguments = (plan.arguments)();
    let expected = (plan.expected)();
    let selection = select_backend(
        BackendKind::Replacement,
        true,
        true,
        digest_output(&[source]),
        FallbackPolicy::Refuse,
    );
    let body_ir = match lower_replacement_case(source) {
        Ok(body_ir) => body_ir,
        Err(detail) => return replacement_profile_refusal(&selection, detail),
    };
    let execution_plan = match prepare_free_function_execution(&body_ir, plan.function, &arguments) {
        Ok(execution_plan) => execution_plan,
        Err(error) => return replacement_profile_refusal(&selection, error.to_string()),
    };
    let executed = match resolve_execution(&selection, true) {
        Ok(backend) => backend,
        Err(error) => {
            return ReplacementPlanEvidence {
                behavior_outcome: ComparisonOutcome::Incompatible {
                    reason: format!("replacement corpus selection failure: {error}"),
                },
                receipt: receipt_ref_from_error(&error),
            };
        }
    };
    let execution = match execute_prevalidated_free_function(execution_plan) {
        Ok(execution) => execution,
        Err(error) => {
            return ReplacementPlanEvidence {
                behavior_outcome: ComparisonOutcome::Mismatch {
                    detail: format!("replacement corpus execution failure: {error}"),
                },
                receipt: ReceiptRef::SelectionError {
                    detail: format!("replacement corpus execution failure: {error}"),
                },
            };
        }
    };
    let behavior_outcome = if execution.value == expected {
        ComparisonOutcome::Match
    } else {
        ComparisonOutcome::Mismatch {
            detail: format!(
                "replacement `{}` returned {:?}, expected {:?}",
                plan.function, execution.value, expected
            ),
        }
    };
    let shadow_comparison = unavailable_shadow_comparison(selection.shadow_requested);
    let comparison_reason = match &shadow_comparison {
        ShadowComparisonState::Unavailable { reason } => reason.clone(),
        state => {
            return ReplacementPlanEvidence {
                behavior_outcome,
                receipt: ReceiptRef::SelectionError {
                    detail: format!("replacement corpus expected unavailable shadow comparison, got {state:?}"),
                },
            };
        }
    };
    let receipt = match finalize_receipt(
        &selection,
        executed,
        execution.output_identity.clone(),
        shadow_comparison,
        DIAGNOSTIC_SCHEMA_VERSION,
    ) {
        Ok(receipt) => receipt,
        Err(error) => {
            return ReplacementPlanEvidence {
                behavior_outcome,
                receipt: receipt_ref_from_error(&error),
            };
        }
    };
    if let Err(error) = receipt.verify_identity() {
        return ReplacementPlanEvidence {
            behavior_outcome,
            receipt: receipt_ref_from_error(&error),
        };
    }
    let output_identity = execution.output_identity.clone();
    let body_snapshot = execution.body_snapshot.clone();
    let ownership_reads = execution.ownership_evidence();
    let runtime_requirements = execution.runtime_requirement_evidence();
    ReplacementPlanEvidence {
        behavior_outcome,
        receipt: ReceiptRef::ReplacementExecuted {
            selection_identity: receipt.selection.identity,
            receipt_identity: receipt.identity,
            output_identity,
            body_snapshot,
            ownership_reads,
            runtime_requirements,
            comparison_reason,
        },
    }
}

/// Refuse a replacement corpus profile through the canonical #986 selection boundary.
///
/// A Body-IR lowering or profile error must not execute directly, and must not silently turn into legacy behavior.
/// Resolving the declared selection with availability set to `false` preserves that refusal as receipt evidence.
fn replacement_profile_refusal(selection: &BackendSelection, detail: String) -> ReplacementPlanEvidence {
    match resolve_execution(selection, false) {
        Ok(executed) => ReplacementPlanEvidence {
            behavior_outcome: ComparisonOutcome::Incompatible {
                reason: format!("replacement corpus profile refusal: {detail}"),
            },
            receipt: ReceiptRef::SelectionError {
                detail: format!(
                    "replacement corpus profile refusal was incorrectly resolved to `{executed:?}`: {detail}"
                ),
            },
        },
        Err(error) => ReplacementPlanEvidence {
            behavior_outcome: ComparisonOutcome::Incompatible {
                reason: format!("replacement corpus profile refusal: {detail}"),
            },
            receipt: ReceiptRef::SelectionError {
                detail: format!("replacement corpus profile refusal: {detail}; selection refusal: {error}"),
            },
        },
    }
}

/// Decide the shadow-comparison state for a selection, matching `build.rs`'s own `backend_shadow_comparison`
/// helper exactly (same condition, same reason string) so the two do not drift into disagreeing explanations for
/// the same unavailability.
fn shadow_comparison_for(selection: &BackendSelection) -> ShadowComparisonState {
    unavailable_shadow_comparison(selection.shadow_requested)
}

/// Fold a finalized receipt's shadow-comparison outcome into the [`ReceiptRef`] a [`CaseReport`] carries.
fn receipt_ref_from_receipt(receipt: BackendExecutionReceipt) -> ReceiptRef {
    match receipt.shadow_comparison {
        ShadowComparisonState::Matched => ReceiptRef::ShadowMatched {
            receipt_identity: receipt.identity,
        },
        ShadowComparisonState::Diverged { detail } => ReceiptRef::ShadowDiverged {
            receipt_identity: receipt.identity,
            detail,
        },
        ShadowComparisonState::Unavailable { reason } => ReceiptRef::ShadowUnavailable {
            receipt_identity: receipt.identity,
            reason,
        },
        ShadowComparisonState::NotRequested => ReceiptRef::ShadowUnavailable {
            receipt_identity: receipt.identity,
            reason: "no shadow comparison was requested for this case".to_string(),
        },
    }
}

/// Fold a backend-selection API error into a [`ReceiptRef`] rather than panicking or silently treating it as
/// available.
fn receipt_ref_from_error(error: &BackendSelectionError) -> ReceiptRef {
    ReceiptRef::SelectionError {
        detail: error.to_string(),
    }
}

/// The outcome of actually running a case's [`ParityCase::evaluate`] function.
///
/// [`ComparisonOutcome::Match`] is the only state that counts as green. Every other variant is an explicit
/// non-green state so that a missing, skipped, or incompatible comparison is visible in the emitted summary
/// instead of being silently rolled up as passing parity.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum ComparisonOutcome {
    /// The observed behavior matched the case's documented expectation.
    Match,
    /// The observed behavior diverged from the case's documented expectation — a real regression signal.
    Mismatch { detail: String },
    /// The comparison was not run for a stated reason (for example, the required backend path does not exist yet).
    ///
    /// No seed case produces this yet — every seed `evaluate` function can run today. Reserved for a future source
    /// profile whose required execution boundary is unavailable, so that case can report "not run" honestly instead
    /// of being omitted from the corpus entirely.
    #[allow(dead_code)]
    Skipped { reason: String },
    /// The two sides of the comparison are not comparable (for example, mismatched build profiles).
    Incompatible { reason: String },
}

impl ComparisonOutcome {
    /// Whether this outcome counts as green for the behavior-verification axis.
    ///
    /// This only covers whether the case's own `evaluate` function observed the expected behavior; it says
    /// nothing about receipt-aware reference/replacement comparison, which is tracked separately via
    /// [`ReceiptRef`] and folded into [`CaseReport::overall_state`].
    pub(crate) fn is_green(&self) -> bool {
        matches!(self, ComparisonOutcome::Match)
    }
}

/// One executable parity corpus case.
///
/// `evaluate` is a function pointer rather than a pre-computed result: the corpus must be executable, not just
/// metadata, so each case proves its own claim by actually lexing/parsing/typechecking/generating against the
/// current compiler at test-run time.
pub(crate) struct ParityCase {
    /// Stable case identity. Once assigned, an ID must never be reused for a different case — renumbering breaks
    /// the "stable case ID" contract #987 and #655 both depend on. Delete and re-add rather than renumber.
    pub(crate) id: &'static str,
    /// Short human-readable title for CI summaries and review.
    pub(crate) title: &'static str,
    pub(crate) category: BehaviorCategory,
    pub(crate) lane: EvidenceLane,
    /// Repo-relative pointer to the primary evidence for this case (fixture path, test name, or doc anchor).
    pub(crate) evidence: &'static str,
    pub(crate) disposition: Disposition,
    /// The case's own Incan source, used to derive a real #986 backend-selection receipt via [`evaluate_case`].
    /// Red-state fixtures that never reach [`evaluate_case`] may use a trivial placeholder since it is unused.
    pub(crate) source: &'static str,
    /// Executes the legacy/non-replacement comparison for this case. Direct replacement plans instead derive their
    /// outcome from the single selected execution that also produces their receipt. Must not panic on an expected
    /// non-green result — return [`ComparisonOutcome::Mismatch`]/`Skipped`/`Incompatible` instead.
    pub(crate) evaluate: Option<fn() -> ComparisonOutcome>,
    /// Optional direct replacement execution that owns this case's #988 proof bundle.
    pub(crate) replacement_execution: Option<ReplacementExecutionPlan>,
}

/// A parameterized direct replacement execution bound to one stable #987 corpus case.
///
/// This intentionally names a function plus concrete values rather than a generated-Rust entrypoint. The source is
/// typechecked and lowered to Body IR in-process, then the replacement executor consumes that Body IR directly.
#[derive(Clone, Copy)]
pub(crate) struct ReplacementExecutionPlan {
    /// Source-level free function executed by the replacement profile.
    pub(crate) function: &'static str,
    /// Concrete typed values passed to `function` in source parameter order.
    pub(crate) arguments: fn() -> Vec<ReplacementValue>,
    /// The source-observable value the direct execution must produce.
    pub(crate) expected: fn() -> ReplacementValue,
}

/// Typecheck and lower source into the Body IR that replacement selection validates before execution.
fn lower_replacement_case(source: &str) -> Result<BodyIrModule, String> {
    let tokens = lexer::lex(source).map_err(|errors| format!("replacement corpus lex failure: {errors:?}"))?;
    let program = parser::parse(&tokens).map_err(|errors| format!("replacement corpus parse failure: {errors:?}"))?;
    let module_path = vec!["parity_987_replacement".to_string()];
    let mut checker = TypeChecker::new();
    checker.set_current_module_path(Some(module_path.clone()));
    checker
        .check_program(&program)
        .map_err(|errors| format!("replacement corpus typecheck failure: {errors:?}"))?;
    Ok(build_body_ir_module_v0(&program, &module_path, checker.type_info()))
}

// ============================================================================
// Validation (proves the schema surfaces gaps rather than defaulting to green)
// ============================================================================

/// One structural problem found in a corpus by [`validate_corpus`].
///
/// This is separate from [`ComparisonOutcome`]: a validation violation means the case itself is malformed (missing
/// metadata, duplicate ID, a disposition without an owning issue), not that its evaluated behavior diverged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CorpusViolation {
    pub(crate) case_id: String,
    pub(crate) problem: String,
}

/// Validate structural invariants of a candidate corpus without evaluating any case.
///
/// Checks stable-ID uniqueness/non-emptiness, required text fields, and that every non-`Preserved` disposition
/// carries a real (non-zero) owning issue and a non-empty migration note. This is what proves the schema surfaces
/// gaps instead of defaulting to green — see the red-state tests in `tests/parity_corpus_tests.rs`.
pub(crate) fn validate_corpus(cases: &[ParityCase]) -> Vec<CorpusViolation> {
    let mut violations = Vec::new();
    let mut seen_ids: BTreeSet<&'static str> = BTreeSet::new();

    for case in cases {
        if case.id.trim().is_empty() {
            violations.push(CorpusViolation {
                case_id: "<empty>".to_string(),
                problem: "case id must not be empty".to_string(),
            });
        } else if !seen_ids.insert(case.id) {
            violations.push(CorpusViolation {
                case_id: case.id.to_string(),
                problem: "duplicate case id".to_string(),
            });
        }

        if case.title.trim().is_empty() {
            violations.push(CorpusViolation {
                case_id: case.id.to_string(),
                problem: "title must not be empty".to_string(),
            });
        }

        if case.evidence.trim().is_empty() {
            violations.push(CorpusViolation {
                case_id: case.id.to_string(),
                problem: "evidence pointer must not be empty".to_string(),
            });
        }

        match &case.disposition {
            Disposition::Preserved => {}
            Disposition::IntentionalMigration {
                owning_issue,
                migration_note,
            }
            | Disposition::Unsupported {
                owning_issue,
                migration_note,
            } => {
                if *owning_issue == 0 {
                    violations.push(CorpusViolation {
                        case_id: case.id.to_string(),
                        problem: "non-preserved disposition must name a real (non-zero) owning issue".to_string(),
                    });
                }
                if migration_note.trim().is_empty() {
                    violations.push(CorpusViolation {
                        case_id: case.id.to_string(),
                        problem: "non-preserved disposition must carry a migration note".to_string(),
                    });
                }
            }
        }
    }

    violations
}

// ============================================================================
// Evaluation and CI-readable summary
// ============================================================================

/// The combined result of running one case and folding in its receipt-availability state.
///
/// `overall_state` is the field a CI consumer should read first: it is the only field that already accounts for
/// both axes (observed behavior and receipt availability) so a consumer cannot accidentally read `behavior_outcome`
/// alone and report green while the receipt-aware comparison this corpus promises is still unavailable.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct CaseReport {
    pub(crate) id: &'static str,
    pub(crate) title: &'static str,
    pub(crate) category: BehaviorCategory,
    pub(crate) lane: EvidenceLane,
    pub(crate) evidence: &'static str,
    pub(crate) disposition_kind: &'static str,
    pub(crate) behavior_outcome: ComparisonOutcome,
    pub(crate) receipt: ReceiptRef,
    pub(crate) overall_state: OverallState,
}

/// The final, honest per-case state a CI consumer or #655 should read.
///
/// There is deliberately no plain `Green` reachable today: reaching it requires both a [`ComparisonOutcome::Match`]
/// behavior outcome *and* a matched source-observable shadow comparison against the replacement backend. #988's
/// direct executions remain non-green until the same source profile has a paired legacy runtime comparator, even
/// though every case already consults a real receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OverallState {
    /// Behavior matched and a real source-observable shadow comparison against the replacement backend also
    /// matched. Not reachable until a paired legacy runtime comparator exists.
    Green,
    /// Behavior matched and a real receipt was produced, but its shadow comparison against the replacement
    /// backend is unavailable because the active source profile has no paired legacy runtime comparator.
    NonGreenShadowUnavailable,
    /// The case's own behavior evaluation did not match (mismatch, skip, or incompatible) — a real signal.
    NonGreenBehavior,
}

/// Evaluate one case: run its behavior probe, consult a real #986 receipt for its source, and fold both into a
/// [`CaseReport`].
pub(crate) fn evaluate_case(case: &ParityCase) -> CaseReport {
    let (behavior_outcome, receipt) = match case.replacement_execution {
        Some(plan) => {
            let evidence = execute_replacement_plan(case.source, plan);
            (evidence.behavior_outcome, evidence.receipt)
        }
        None => match case.evaluate {
            Some(evaluate) => (evaluate(), compute_legacy_receipt(case.source)),
            None => (
                ComparisonOutcome::Incompatible {
                    reason: "corpus case has neither a legacy behavior probe nor a replacement execution plan"
                        .to_string(),
                },
                ReceiptRef::SelectionError {
                    detail: "corpus case has neither a legacy behavior probe nor a replacement execution plan"
                        .to_string(),
                },
            ),
        },
    };
    // `shadow_matched` is its own match (rather than folded into one match on `(outcome, receipt)`) so that adding
    // a future reachable `ReceiptRef` variant only requires updating this one arm.
    let shadow_matched = matches!(receipt, ReceiptRef::ShadowMatched { .. });
    let overall_state = if !behavior_outcome.is_green() {
        OverallState::NonGreenBehavior
    } else if shadow_matched {
        OverallState::Green
    } else {
        OverallState::NonGreenShadowUnavailable
    };
    CaseReport {
        id: case.id,
        title: case.title,
        category: case.category,
        lane: case.lane,
        evidence: case.evidence,
        disposition_kind: case.disposition.kind(),
        behavior_outcome,
        receipt,
        overall_state,
    }
}

/// A CI-readable summary of one corpus run, shaped for #655 (compatibility report) to consume.
///
/// Serializes to a stable-keyed JSON object. `receipt_schema_available` is a top-level flag (not just inferable
/// from per-case state) so a consumer can immediately see whether #986 has landed without scanning every case.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct CorpusSummary {
    pub(crate) schema_version: u32,
    pub(crate) total_cases: usize,
    pub(crate) green: usize,
    pub(crate) non_green_shadow_unavailable: usize,
    pub(crate) non_green_behavior: usize,
    pub(crate) receipt_schema_available: bool,
    pub(crate) cases: Vec<CaseReport>,
}

/// The current CI-summary schema version. Version `3` adds `ReceiptRef::ReplacementExecuted`, which binds the five
/// #988 Body-IR cases to their own selection/execution identities and canonical body, ownership, and runtime
/// evidence while retaining an explicit non-green unavailable comparison. Bump again whenever `CorpusSummary`'s or
/// `CaseReport`'s field shape changes in a way a consumer (including #655) would need to notice.
pub(crate) const SCHEMA_VERSION: u32 = 3;

/// Evaluate every case in the corpus and assemble the CI-readable summary.
///
/// This does not itself assert anything; callers in `tests/parity_corpus_tests.rs` are responsible for turning
/// `non_green_behavior > 0` (an unexpected regression, as opposed to a case whose disposition already expects a
/// non-green mismatch) into a test failure.
pub(crate) fn summarize(cases: &[ParityCase]) -> CorpusSummary {
    let reports: Vec<CaseReport> = cases.iter().map(evaluate_case).collect();
    let green = reports
        .iter()
        .filter(|r| r.overall_state == OverallState::Green)
        .count();
    let non_green_shadow_unavailable = reports
        .iter()
        .filter(|r| r.overall_state == OverallState::NonGreenShadowUnavailable)
        .count();
    let non_green_behavior = reports
        .iter()
        .filter(|r| r.overall_state == OverallState::NonGreenBehavior)
        .count();
    CorpusSummary {
        schema_version: SCHEMA_VERSION,
        total_cases: reports.len(),
        green,
        non_green_shadow_unavailable,
        non_green_behavior,
        // #986 landed: every case above consulted a real `BackendExecutionReceipt`, not a placeholder. This is
        // `true` regardless of how many cases reach `Green`, because it reports whether the receipt contract
        // itself is available, not whether every comparison succeeded.
        receipt_schema_available: true,
        cases: reports,
    }
}
