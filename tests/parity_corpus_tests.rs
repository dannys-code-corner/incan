//! Executable backend-cutover parity corpus (#987).
//!
//! Turns the #646 behavior inventory (`workspaces/docs-site/docs/contributing/reference/
//! backend_behavior_inventory.md`) into a runnable corpus with stable case IDs and an explicit disposition per
//! case, per #987's scope. See `tests/support/parity_corpus.rs` for the schema and
//! `workspaces/docs-site/docs/contributing/reference/parity_corpus.md` for the consumer-facing shape doc (#655).
//!
//! Run with: `cargo test --test parity_corpus_tests`
//!
//! ## Why these six cases
//!
//! Per #987's own plan step 3 ("add a narrow source-only seed corpus before public package or Rust-interop
//! rows"), every seed case here uses a direct-parser/typechecker, generated-project-run, or codegen-snapshot
//! lane. Package/import, Rust-interop, vocab, and downstream lanes are deferred until the required compiler/ABI
//! decisions are available (plan step 4). The public parity-corpus reference and issue #987 record the expansion
//! criteria; a seed case must never imply that an unavailable lane has been proven.
//!
//! Each case's `evaluate` function probes the *current* compiler directly (not a fixture snapshot of past output),
//! so a behavior change shows up as [`parity_corpus::ComparisonOutcome::Mismatch`] the next time this test runs.

use incan::backend::IrCodegen;
use incan::frontend::{lexer, parser, typechecker};
use std::path::PathBuf;

#[path = "support/parity_corpus.rs"]
mod parity_corpus;

use parity_corpus::{
    BehaviorCategory, ComparisonOutcome, Disposition, EvidenceLane, OverallState, ParityCase, ReceiptRef,
    validate_corpus,
};

// ============================================================================
// Shared frontend probes
// ============================================================================

/// Lex, parse, and typecheck `src`, returning the typechecker's error messages (empty on success).
///
/// Mirrors the helper already used by `tests/construction_diagnostics_tests.rs` and
/// `tests/semantic_core_parity.rs` — kept local rather than shared because each corpus case wants a plain
/// `ComparisonOutcome`, not a `Result` a caller must unwrap.
fn typecheck_err_messages(src: &str) -> Result<Vec<String>, Vec<String>> {
    let tokens = lexer::lex(src).map_err(|errs| errs.into_iter().map(|e| e.message).collect::<Vec<_>>())?;
    let ast = parser::parse(&tokens).map_err(|errs| errs.into_iter().map(|e| e.message).collect::<Vec<_>>())?;
    let mut tc = typechecker::TypeChecker::new();
    match tc.check_program(&ast) {
        Ok(()) => Ok(vec![]),
        Err(errs) => Ok(errs.into_iter().map(|e| e.message).collect()),
    }
}

/// Fold a `typecheck_err_messages` result (lex/parse failure or typecheck errors) into a `ComparisonOutcome`,
/// given a predicate over the typechecker error messages that decides whether the observed shape still matches
/// the case's documented expectation.
fn outcome_from_typecheck(src: &str, expect: impl FnOnce(&[String]) -> bool, expect_desc: &str) -> ComparisonOutcome {
    match typecheck_err_messages(src) {
        Err(errs) => ComparisonOutcome::Incompatible {
            reason: format!("lex/parse failed before typecheck could run: {errs:?}"),
        },
        Ok(errs) => {
            if expect(&errs) {
                ComparisonOutcome::Match
            } else {
                ComparisonOutcome::Mismatch {
                    detail: format!("expected {expect_desc}, got typechecker messages: {errs:?}"),
                }
            }
        }
    }
}

// ============================================================================
// Case 1 — Supported language contract: match exhaustiveness is enforced
// ============================================================================

fn case_supported_match_exhaustiveness() -> ComparisonOutcome {
    let src = r#"
enum Color:
    Red
    Green
    Blue

def name(c: Color) -> str:
    match c:
        case Color.Red:
            return "red"
        case Color.Green:
            return "green"
"#;
    outcome_from_typecheck(
        src,
        |errs| errs.iter().any(|e| e.to_lowercase().contains("exhaustive")),
        "a non-exhaustive-match diagnostic naming the missing `Blue` arm",
    )
}

// ============================================================================
// Case 2 — Diagnostic behavior: chained comparisons are rejected, not silently re-parsed
// ============================================================================

fn case_diagnostic_chained_comparison_rejected() -> ComparisonOutcome {
    // Incan does not support Python-style chained comparisons (`a < b < c` as `a < b and b < c`). Verified by
    // direct probe: today it type-errors because `(a < b) < c` compares a `bool` to an `int`. The corpus records
    // that this stays a rejection, not a silent reinterpretation as chained boolean logic — a real semantic
    // decision, not just token shape.
    let src = r#"
def main() -> None:
    a = 1
    b = 2
    c = 3
    if a < b < c:
        println("chained")
"#;
    outcome_from_typecheck(
        src,
        |errs| !errs.is_empty(),
        "a type-mismatch diagnostic rejecting the chained comparison",
    )
}

// ============================================================================
// Case 3 — Stdlib/runtime behavior: string membership (`in`) matches the runtime helper
// ============================================================================

fn case_stdlib_runtime_string_membership() -> ComparisonOutcome {
    use incan_core::strings::str_contains;

    if !str_contains("hello", "hell") || str_contains("hello", "xyz") {
        return ComparisonOutcome::Mismatch {
            detail: "incan_core::strings::str_contains no longer matches its documented substring policy".to_string(),
        };
    }

    let src = r#"
def f() -> bool:
    return "a" in "abc"
"#;
    outcome_from_typecheck(
        src,
        |errs| errs.is_empty(),
        "`\"a\" in \"abc\"` to typecheck as bool with no errors, matching the runtime membership helper",
    )
}

// ============================================================================
// Case 4 — Generated-artifact behavior: codegen stays inspectable, not semantically authoritative
// ============================================================================

fn case_generated_artifact_valid_rust_shape() -> ComparisonOutcome {
    let src = r#"
def add(a: int, b: int) -> int:
    return a + b
"#;
    let Ok(tokens) = lexer::lex(src) else {
        return ComparisonOutcome::Incompatible {
            reason: "lexer failed on a fixture that must lex cleanly".to_string(),
        };
    };
    let Ok(ast) = parser::parse(&tokens) else {
        return ComparisonOutcome::Incompatible {
            reason: "parser failed on a fixture that must parse cleanly".to_string(),
        };
    };
    let rust_code = match IrCodegen::new().try_generate(&ast) {
        Ok(code) => code,
        Err(e) => {
            return ComparisonOutcome::Mismatch {
                detail: format!("codegen failed on a fixture that must generate cleanly: {e:?}"),
            };
        }
    };
    // This deliberately only checks that the output is syntactically valid Rust (still inspectable), never that
    // it matches a specific token layout — a byte-exact snapshot would make generated-Rust shape the semantic
    // contract, which the #646 inventory and the rust-source-backend deprecation policy both reject.
    match syn::parse_file(&rust_code) {
        Ok(_) => ComparisonOutcome::Match,
        Err(e) => ComparisonOutcome::Mismatch {
            detail: format!("generated Rust is not syntactically valid: {e}"),
        },
    }
}

// ============================================================================
// Case 5 — Accidental accepted behavior: shadowing a builtin name is accepted silently
// ============================================================================

fn case_accidental_shadowing_builtin_len() -> ComparisonOutcome {
    // Verified by direct probe: a user-defined top-level `def len(...)` shadows the `len` builtin with no
    // diagnostic, and the shadowed definition wins at the call site. There is no documented contract for builtin
    // shadowing in the language reference. Per the #646 inventory's own migration guidance for this category
    // ("either document and test it as supported or reject it with a clear diagnostic"), this is exactly the kind
    // of gap #987 exists to surface, not resolve — the disposition below tracks that decision under the cutover
    // umbrella issue pending the dedicated #1116 disposition decision.
    let src = r#"
def len(x: int) -> int:
    return x + 1

def main() -> None:
    y = len(5)
    println(y)
"#;
    outcome_from_typecheck(
        src,
        |errs| errs.is_empty(),
        "shadowing the `len` builtin to keep typechecking silently (today's accidental-acceptance baseline)",
    )
}

// ============================================================================
// Case 6 — Bug-compatible behavior: dead code after `return` is not flagged
// ============================================================================

fn case_bug_compatible_dead_code_after_return() -> ComparisonOutcome {
    // Verified by direct probe: statements after an unconditional `return` typecheck with no diagnostic (Rust
    // itself would warn `unreachable_code` in the generated function body). Tracked as bug-compatible rather than
    // supported: current users may have dead code today, and this is not a behavior we want to freeze as an
    // intentional language contract.
    let src = r#"
def f() -> int:
    return 1
    println("dead code")
    return 2
"#;
    outcome_from_typecheck(
        src,
        |errs| errs.is_empty(),
        "unreachable code after `return` to keep typechecking silently (today's bug-compatible baseline)",
    )
}

// ============================================================================
// Seed corpus
// ============================================================================

/// The narrow, source-only seed corpus (#987 plan step 3). Package/import, Rust-interop, vocab, and downstream
/// rows are deferred to plan step 4 until their compiler/ABI contracts are available; the public parity-corpus
/// reference and #987 record that expansion boundary.
fn seed_corpus() -> Vec<ParityCase> {
    vec![
        ParityCase {
            id: "parity-987-0001",
            title: "Match expressions over enums must be exhaustive",
            category: BehaviorCategory::SupportedLanguageContract,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs::case_supported_match_exhaustiveness",
            disposition: Disposition::Preserved,
            receipt: ReceiptRef::PendingSchema { blocking_issue: 986 },
            evaluate: case_supported_match_exhaustiveness,
        },
        ParityCase {
            id: "parity-987-0002",
            title: "Chained comparisons are rejected with a type-mismatch diagnostic",
            category: BehaviorCategory::DiagnosticBehavior,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs::case_diagnostic_chained_comparison_rejected",
            disposition: Disposition::Preserved,
            receipt: ReceiptRef::PendingSchema { blocking_issue: 986 },
            evaluate: case_diagnostic_chained_comparison_rejected,
        },
        ParityCase {
            id: "parity-987-0003",
            title: "String membership (`in`) matches the runtime helper's substring policy",
            category: BehaviorCategory::StdlibRuntimeBehavior,
            lane: EvidenceLane::GeneratedProjectRun,
            evidence: "tests/parity_corpus_tests.rs::case_stdlib_runtime_string_membership",
            disposition: Disposition::Preserved,
            receipt: ReceiptRef::PendingSchema { blocking_issue: 986 },
            evaluate: case_stdlib_runtime_string_membership,
        },
        ParityCase {
            id: "parity-987-0004",
            title: "Generated Rust stays syntactically valid and inspectable, not the semantic contract",
            category: BehaviorCategory::GeneratedArtifactBehavior,
            lane: EvidenceLane::CodegenSnapshot,
            evidence: "tests/parity_corpus_tests.rs::case_generated_artifact_valid_rust_shape",
            disposition: Disposition::Preserved,
            receipt: ReceiptRef::PendingSchema { blocking_issue: 986 },
            evaluate: case_generated_artifact_valid_rust_shape,
        },
        ParityCase {
            id: "parity-987-0005",
            title: "Shadowing a builtin name (`len`) typechecks with no diagnostic",
            category: BehaviorCategory::AccidentalAcceptedBehavior,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs::case_accidental_shadowing_builtin_len",
            disposition: Disposition::Unsupported {
                owning_issue: 1116,
                migration_note: "No documented contract for builtin shadowing exists today. Tracked in #1116: \
                                  before 0.6 cutover, decide (a) document and test builtin shadowing as \
                                  supported, matching Python's own scoping rules, or (b) add a typechecker \
                                  diagnostic rejecting redefinition of builtin names. Do not let the replacement \
                                  backend silently inherit whichever behavior falls out of its symbol resolution \
                                  order.",
            },
            receipt: ReceiptRef::PendingSchema { blocking_issue: 986 },
            evaluate: case_accidental_shadowing_builtin_len,
        },
        ParityCase {
            id: "parity-987-0006",
            title: "Dead code after `return` typechecks with no diagnostic",
            category: BehaviorCategory::BugCompatibleBehavior,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs::case_bug_compatible_dead_code_after_return",
            disposition: Disposition::Unsupported {
                owning_issue: 1117,
                migration_note: "Current users may have unreachable code today, so this must not be frozen as \
                                  supported. Tracked in #1117: before 0.6 cutover, add an unreachable-code \
                                  diagnostic (mirroring Rust's own `unreachable_code` lint on the generated \
                                  function body) rather than carrying the silent gap into the replacement \
                                  backend.",
            },
            receipt: ReceiptRef::PendingSchema { blocking_issue: 986 },
            evaluate: case_bug_compatible_dead_code_after_return,
        },
    ]
}

// ============================================================================
// Red-state proof: the schema must surface gaps, not default to green
// ============================================================================

/// Build cases that are individually malformed in a distinct way, to prove [`validate_corpus`] catches each
/// problem rather than letting a broken case pass silently. These are never added to [`seed_corpus`].
fn malformed_cases_for_red_state_proof() -> Vec<ParityCase> {
    vec![
        ParityCase {
            id: "parity-987-dup",
            title: "First of a duplicate pair",
            category: BehaviorCategory::SupportedLanguageContract,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs (red-state fixture)",
            disposition: Disposition::Preserved,
            receipt: ReceiptRef::PendingSchema { blocking_issue: 986 },
            evaluate: || ComparisonOutcome::Match,
        },
        ParityCase {
            id: "parity-987-dup",
            title: "Second of a duplicate pair (same id — must be flagged)",
            category: BehaviorCategory::SupportedLanguageContract,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs (red-state fixture)",
            disposition: Disposition::Preserved,
            receipt: ReceiptRef::PendingSchema { blocking_issue: 986 },
            evaluate: || ComparisonOutcome::Match,
        },
        ParityCase {
            id: "parity-987-empty-title",
            title: "",
            category: BehaviorCategory::DiagnosticBehavior,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs (red-state fixture)",
            disposition: Disposition::Preserved,
            receipt: ReceiptRef::PendingSchema { blocking_issue: 986 },
            evaluate: || ComparisonOutcome::Match,
        },
        ParityCase {
            id: "parity-987-unsupported-no-issue",
            title: "Unsupported disposition missing an owning issue and note",
            category: BehaviorCategory::AccidentalAcceptedBehavior,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs (red-state fixture)",
            disposition: Disposition::Unsupported {
                owning_issue: 0,
                migration_note: "",
            },
            receipt: ReceiptRef::PendingSchema { blocking_issue: 986 },
            evaluate: || ComparisonOutcome::Match,
        },
    ]
}

#[test]
fn red_state_validate_corpus_catches_duplicate_ids_missing_titles_and_unowned_dispositions() {
    let violations = validate_corpus(&malformed_cases_for_red_state_proof());

    let has_violation_matching = |case_id: &str, needle: &str| {
        violations
            .iter()
            .any(|v| v.case_id == case_id && v.problem.contains(needle))
    };

    assert!(
        has_violation_matching("parity-987-dup", "duplicate"),
        "expected a duplicate-id violation, got: {violations:?}"
    );
    assert!(
        has_violation_matching("parity-987-empty-title", "title"),
        "expected an empty-title violation, got: {violations:?}"
    );
    assert!(
        has_violation_matching("parity-987-unsupported-no-issue", "owning issue"),
        "expected a missing-owning-issue violation, got: {violations:?}"
    );
    assert!(
        has_violation_matching("parity-987-unsupported-no-issue", "migration note"),
        "expected a missing-migration-note violation, got: {violations:?}"
    );
    // Four distinct cases, at least one violation each (duplicate id reports on the second occurrence only).
    assert!(
        violations.len() >= 4,
        "expected at least 4 violations across the malformed fixtures, got {}: {violations:?}",
        violations.len()
    );
}

// ============================================================================
// Green-state proof: the real seed corpus is structurally sound and behaviorally confirmed
// ============================================================================

#[test]
fn seed_corpus_has_no_structural_violations() {
    let violations = validate_corpus(&seed_corpus());
    assert!(
        violations.is_empty(),
        "seed corpus has structural violations: {violations:?}"
    );
}

#[test]
fn seed_corpus_ids_are_stable_and_globally_unique() {
    let ids: Vec<&str> = seed_corpus().iter().map(|c| c.id).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "seed corpus case ids must be globally unique");
    for id in &ids {
        assert!(
            id.starts_with("parity-987-"),
            "case id {id} should carry the parity-987- namespace prefix for stability across future slices"
        );
    }
}

#[test]
fn seed_corpus_every_case_confirms_its_documented_current_behavior() {
    let summary = parity_corpus::summarize(&seed_corpus());
    let regressions: Vec<&parity_corpus::CaseReport> = summary
        .cases
        .iter()
        .filter(|c| !c.behavior_outcome.is_green())
        .collect();
    assert!(
        regressions.is_empty(),
        "seed corpus cases whose evaluate() no longer confirms the documented current behavior (this means the \
         compiler's actual behavior drifted from what this corpus recorded — update the case or investigate the \
         regression, do not silently accept it): {regressions:#?}"
    );
}

#[test]
fn no_case_is_silently_reported_as_fully_green_while_the_986_receipt_schema_is_pending() {
    // This is the corpus's core promise: an unavailable receipt-aware comparison must never be counted as
    // parity. As long as #986 has not landed a receipt type, every case — even a behaviorally-confirmed one —
    // must land in `NonGreenPendingReceipt`, never `Green`.
    let summary = parity_corpus::summarize(&seed_corpus());
    assert!(
        !summary.receipt_schema_available,
        "the summary must say the #986 receipt schema is unavailable until that slice actually lands"
    );
    let falsely_green: Vec<&parity_corpus::CaseReport> = summary
        .cases
        .iter()
        .filter(|c| c.overall_state == OverallState::Green)
        .collect();
    assert!(
        falsely_green.is_empty(),
        "no case should reach OverallState::Green before #986's receipt schema lands: {falsely_green:#?}"
    );
    assert_eq!(summary.green, 0);
    assert_eq!(
        summary.non_green_pending_receipt, summary.total_cases,
        "every behaviorally-confirmed seed case must be pending-receipt, not green, today"
    );
    assert_eq!(summary.non_green_behavior, 0);
}

// ============================================================================
// CI-readable summary emission (#655 consumer contract)
// ============================================================================

/// Where the CI-readable summary is written, honoring a harness-selected `CARGO_TARGET_DIR` when set (matching
/// `tests/support/mod.rs`'s convention for other generated test artifacts) and falling back to the crate-local
/// `target/` directory otherwise.
fn summary_output_path() -> PathBuf {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
    target_dir.join("parity-corpus").join("summary.json")
}

#[test]
fn ci_summary_serializes_with_the_fields_655_needs_and_is_written_to_a_stable_path()
-> Result<(), Box<dyn std::error::Error>> {
    let summary = parity_corpus::summarize(&seed_corpus());
    let json = serde_json::to_string_pretty(&summary)?;
    let value: serde_json::Value = serde_json::from_str(&json)?;

    for field in [
        "schema_version",
        "total_cases",
        "green",
        "non_green_pending_receipt",
        "non_green_behavior",
        "receipt_schema_available",
        "cases",
    ] {
        assert!(
            value.get(field).is_some(),
            "CI summary is missing required top-level field `{field}`: {value}"
        );
    }

    let cases = value
        .get("cases")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("CI summary `cases` must be a JSON array: {value}"))?;
    assert_eq!(cases.len(), seed_corpus().len());
    for case in cases {
        for field in [
            "id",
            "title",
            "category",
            "lane",
            "evidence",
            "disposition_kind",
            "behavior_outcome",
            "receipt",
            "overall_state",
        ] {
            assert!(
                case.get(field).is_some(),
                "CI summary case row is missing required field `{field}`: {case}"
            );
        }
    }

    let output_path = summary_output_path();
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output_path, &json)?;
    Ok(())
}
