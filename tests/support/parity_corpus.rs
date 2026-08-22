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
//! ## Receipt-awareness (coordination with #986)
//!
//! #987's own scope calls for "receipt-aware reference/replacement or shadow comparisons where both paths are
//! available." The backend-selection receipt contract (#986) has not landed in `0.6-dev` yet.
//! [`ReceiptRef::PendingSchema`] is a documented placeholder for that dependency: every seed case in
//! `tests/parity_corpus_tests.rs` currently carries it, and the summary's `receipt_schema_available` field is `false`
//! so downstream consumers (including #655) can see this is an open item rather than an implemented comparison.

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

/// A placeholder reference to the #986 backend-selection receipt shape.
///
/// #986 (`backend-selection-receipt`) has not landed a stable repository receipt contract in `0.6-dev`. The corpus
/// cannot depend on a pre-merge API without risking drift, so the schema and source-only seed proceed without it;
/// every case carries an explicit placeholder rather than a guessed or silently omitted receipt field.
///
/// This is a genuine external dependency, not a TODO this slice can close on its own. It is **intentionally
/// deferred**, not partially implemented: no case in `tests/parity_corpus_tests.rs` performs a real
/// reference/replacement or shadow comparison against a live receipt today.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum ReceiptRef {
    /// #986's receipt type is not yet available. `blocking_issue` names the issue that owns landing it; the case
    /// cannot perform a receipt-aware reference/replacement comparison yet and must not be scored as green for
    /// that axis. See [`OverallState::NonGreenPendingReceipt`], which every case with this receipt state and a
    /// matching `behavior_outcome` lands on instead of [`OverallState::Green`].
    ///
    /// What unblocks this: once #986 lands a stable receipt type, add a new `ReceiptRef`
    /// variant (for example `Recorded(SelectionReceiptSummary)`) that wraps it, keep `PendingSchema` for any case
    /// that still predates receipt wiring, and update [`evaluate_case`] to treat the new variant as
    /// `receipt_available: true` after actually performing a reference/replacement comparison against it (not
    /// just presence-checking the type). At least one seed case in `tests/parity_corpus_tests.rs` should then be
    /// migrated off `PendingSchema` to exercise the newly reachable [`OverallState::Green`] path, which is
    /// currently provably unreachable — see the
    /// `no_case_is_silently_reported_as_fully_green_while_the_986_receipt_schema_is_pending` test in
    /// `tests/parity_corpus_tests.rs`.
    PendingSchema { blocking_issue: u32 },
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
    /// No seed case produces this yet — every seed `evaluate` function can run today. Reserved for cases that
    /// depend on a boundary that does not exist yet (for example Body IR execution, #653), so a future case can
    /// report "not run" honestly instead of being omitted from the corpus entirely.
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
    pub(crate) receipt: ReceiptRef,
    /// Executes the actual comparison and returns its outcome. Must not panic on an expected-non-green result —
    /// return [`ComparisonOutcome::Mismatch`]/`Skipped`/`Incompatible` instead so the corpus test can assert on
    /// the report rather than on a panicking case function.
    pub(crate) evaluate: fn() -> ComparisonOutcome,
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
/// behavior outcome *and* an available receipt-aware comparison, and #986's receipt schema has not landed yet. This
/// keeps the summary from ever silently claiming full backend-cutover parity before that dependency exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OverallState {
    /// Behavior matched and a receipt-aware reference/replacement comparison also confirmed it. Not reachable
    /// until #986 lands — kept as a variant so the summary schema does not need to change when it does.
    Green,
    /// Behavior matched, but receipt-aware comparison is unavailable (today: always this, for a `Match` case).
    NonGreenPendingReceipt,
    /// The case's own behavior evaluation did not match (mismatch, skip, or incompatible) — a real signal.
    NonGreenBehavior,
}

/// Evaluate one case and fold its behavior outcome and receipt availability into a [`CaseReport`].
pub(crate) fn evaluate_case(case: &ParityCase) -> CaseReport {
    let behavior_outcome = (case.evaluate)();
    // `receipt_available` is its own match (rather than folded into one match on `(outcome, receipt)`) so that
    // adding a future non-placeholder `ReceiptRef` variant only requires updating this one arm, instead of
    // producing an unreachable-pattern warning against today's single-variant enum.
    let receipt_available = match &case.receipt {
        ReceiptRef::PendingSchema { .. } => false,
    };
    let overall_state = if !behavior_outcome.is_green() {
        OverallState::NonGreenBehavior
    } else if receipt_available {
        OverallState::Green
    } else {
        OverallState::NonGreenPendingReceipt
    };
    CaseReport {
        id: case.id,
        title: case.title,
        category: case.category,
        lane: case.lane,
        evidence: case.evidence,
        disposition_kind: case.disposition.kind(),
        behavior_outcome,
        receipt: case.receipt.clone(),
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
    pub(crate) non_green_pending_receipt: usize,
    pub(crate) non_green_behavior: usize,
    pub(crate) receipt_schema_available: bool,
    pub(crate) cases: Vec<CaseReport>,
}

/// The current CI-summary schema version. Bump this whenever `CorpusSummary`'s or `CaseReport`'s field shape
/// changes in a way a consumer (including #655) would need to notice.
pub(crate) const SCHEMA_VERSION: u32 = 1;

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
    let non_green_pending_receipt = reports
        .iter()
        .filter(|r| r.overall_state == OverallState::NonGreenPendingReceipt)
        .count();
    let non_green_behavior = reports
        .iter()
        .filter(|r| r.overall_state == OverallState::NonGreenBehavior)
        .count();
    CorpusSummary {
        schema_version: SCHEMA_VERSION,
        total_cases: reports.len(),
        green,
        non_green_pending_receipt,
        non_green_behavior,
        receipt_schema_available: false,
        cases: reports,
    }
}
