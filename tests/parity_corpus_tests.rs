//! Executable backend-cutover parity corpus (#987).
//!
//! Turns the #646 behavior inventory (`workspaces/docs-site/docs/contributing/reference/
//! backend_behavior_inventory.md`) into a runnable corpus with stable case IDs and an explicit disposition per
//! case, per #987's scope. See `tests/support/parity_corpus.rs` for the schema; implementation status lives on
//! issue #652, not in permanent contributor docs, since it describes an in-flight migration state rather than a
//! durable 0.6 end-state contract.
//!
//! Run with: `cargo test --test parity_corpus_tests`
//!
//! ## Why these eleven cases
//!
//! Per #987's own plan step 3 ("add a narrow source-only seed corpus before public package or Rust-interop
//! rows"), every seed case here uses a direct-parser/typechecker, generated-project-run, or codegen-snapshot
//! lane. Package/import, Rust-interop, vocab, and downstream lanes are deferred until the required compiler/ABI
//! decisions are available (plan step 4). Issue #987 records the expansion criteria; a seed case must never imply
//! that an unavailable lane has been proven.
//!
//! Each case's `evaluate` function probes the *current* compiler directly (not a fixture snapshot of past output),
//! so a behavior change shows up as [`parity_corpus::ComparisonOutcome::Mismatch`] the next time this test runs.

use incan::backend::IrCodegen;
use incan::backend::replacement::ReplacementValue;
use incan::frontend::diagnostics::CompileError;
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

/// Lex, parse, and typecheck `src`, returning the typechecker's non-fatal warnings, or a reason the probe could
/// not run at all.
///
/// Kept separate from [`typecheck_err_messages`] because warnings ride their own channel: `check_program` reports
/// only hard errors, so a case asserting a *warning* that read the error channel would pass for the wrong reason —
/// a silently accepted program and a correctly warned one look identical from there.
fn typecheck_warnings(src: &str) -> Result<Vec<CompileError>, String> {
    let tokens = lexer::lex(src).map_err(|errs| format!("lex failed: {:?}", messages(errs)))?;
    let ast = parser::parse(&tokens).map_err(|errs| format!("parse failed: {:?}", messages(errs)))?;
    let mut tc = typechecker::TypeChecker::new();
    tc.check_program(&ast)
        .map_err(|errs| format!("typecheck failed: {:?}", messages(errs)))?;
    Ok(tc.take_warnings())
}

/// Reduce diagnostics to their message text for probe-failure reporting.
fn messages(errors: Vec<CompileError>) -> Vec<String> {
    errors.into_iter().map(|error| error.message).collect()
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

const CASE_1_SRC: &str = r#"
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

fn case_supported_match_exhaustiveness() -> ComparisonOutcome {
    outcome_from_typecheck(
        CASE_1_SRC,
        |errs| errs.iter().any(|e| e.to_lowercase().contains("exhaustive")),
        "a non-exhaustive-match diagnostic naming the missing `Blue` arm",
    )
}

// ============================================================================
// Case 2 — Diagnostic behavior: chained comparisons are rejected, not silently re-parsed
// ============================================================================

// Incan does not support Python-style chained comparisons (`a < b < c` as `a < b and b < c`). Verified by direct
// probe: today it type-errors because `(a < b) < c` compares a `bool` to an `int`. The corpus records that this
// stays a rejection, not a silent reinterpretation as chained boolean logic — a real semantic decision, not just
// token shape.
const CASE_2_SRC: &str = r#"
def main() -> None:
    a = 1
    b = 2
    c = 3
    if a < b < c:
        println("chained")
"#;

fn case_diagnostic_chained_comparison_rejected() -> ComparisonOutcome {
    outcome_from_typecheck(
        CASE_2_SRC,
        |errs| !errs.is_empty(),
        "a type-mismatch diagnostic rejecting the chained comparison",
    )
}

// ============================================================================
// Case 3 — Stdlib/runtime behavior: string membership (`in`) matches the runtime helper
// ============================================================================

const CASE_3_SRC: &str = r#"
def f() -> bool:
    return "a" in "abc"
"#;

fn case_stdlib_runtime_string_membership() -> ComparisonOutcome {
    use incan_core::strings::str_contains;

    if !str_contains("hello", "hell") || str_contains("hello", "xyz") {
        return ComparisonOutcome::Mismatch {
            detail: "incan_core::strings::str_contains no longer matches its documented substring policy".to_string(),
        };
    }

    outcome_from_typecheck(
        CASE_3_SRC,
        |errs| errs.is_empty(),
        "`\"a\" in \"abc\"` to typecheck as bool with no errors, matching the runtime membership helper",
    )
}

// ============================================================================
// Case 4 — Generated-artifact behavior: codegen stays inspectable, not semantically authoritative
// ============================================================================

const CASE_4_SRC: &str = r#"
def add(a: int, b: int) -> int:
    return a + b
"#;

fn case_generated_artifact_valid_rust_shape() -> ComparisonOutcome {
    let Ok(tokens) = lexer::lex(CASE_4_SRC) else {
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
// Case 5 — Supported language contract: lexical bindings shadow ambient builtins
// ============================================================================

// #1116 adopts this as a language contract: a direct module declaration or explicit import is a real lexical
// binding and wins over an ambient core builtin function for unqualified calls. `std.builtins.<name>` remains the
// explicit route to the builtin when the local spelling is shadowed. The corresponding typechecker, codegen, and
// runtime coverage lives alongside this corpus row; #653 must reproduce the same precedence deliberately.
const CASE_5_SRC: &str = r#"
def len(x: int) -> int:
    return x + 1

def main() -> None:
    y = len(5)
    println(y)
"#;

fn case_supported_builtin_len_shadowing() -> ComparisonOutcome {
    outcome_from_typecheck(
        CASE_5_SRC,
        |errs| errs.is_empty(),
        "a module `len` binding to shadow the ambient builtin without a diagnostic",
    )
}

// ============================================================================
// Case 6 — Diagnostic behavior: dead code after `return` warns (migrated from a silent accept)
// ============================================================================

// This row entered the corpus as bug-compatible behavior: statements after an unconditional `return` typechecked
// with zero diagnostics, so the gap was invisible unless a user read the generated Rust. #1117 migrated it
// deliberately — the typechecker now emits `INCAN-T0101` — so the case asserts the diagnostic contract instead of
// the old silence, and its disposition records the migration rather than freezing either behavior.
//
// The assertion reads the *stable code*, not message prose, so rewording the diagnostic does not silently break
// the corpus while a real change of contract still does.
const CASE_6_SRC: &str = r#"
def f() -> int:
    return 1
    println("dead code")
    return 2
"#;

fn case_diagnostic_unreachable_code_after_return() -> ComparisonOutcome {
    match typecheck_warnings(CASE_6_SRC) {
        Err(reason) => ComparisonOutcome::Incompatible { reason },
        Ok(warnings) => {
            if warnings
                .iter()
                .any(|warning| warning.stable_code() == Some("INCAN-T0101"))
            {
                ComparisonOutcome::Match
            } else {
                ComparisonOutcome::Mismatch {
                    detail: format!(
                        "expected an INCAN-T0101 unreachable-code warning, got warnings: {:?}",
                        warnings.iter().map(|warning| &warning.message).collect::<Vec<_>>()
                    ),
                }
            }
        }
    }
}

// ============================================================================
// #988 replacement execution corpus — stable receipt-bound source cases
// ============================================================================

const REPLACEMENT_BODY_V0_001_SRC: &str = r#"
def add(x: int, y: int) -> int:
    return x + y
"#;

const REPLACEMENT_BODY_V0_002_SRC: &str = r#"
def greet(name: str) -> str:
    return "hello, " + name
"#;

const REPLACEMENT_BODY_V0_003_SRC: &str = r#"
def return_owned() -> str:
    value = "owned"
    return value
"#;

const REPLACEMENT_BODY_V0_004_SRC: &str = r#"
def control_flow() -> int:
    for value in range(1, 5):
        if value % 2 == 0:
            continue
    while false:
        return 0
    return 10
"#;

const REPLACEMENT_BODY_V0_005_SRC: &str = r#"
def guarded_floor_div(a: int, b: int) -> int:
    assert b != 0
    return a // b
"#;

fn replacement_body_v0_001_arguments() -> Vec<ReplacementValue> {
    vec![ReplacementValue::Int(40), ReplacementValue::Int(2)]
}

fn replacement_body_v0_001_expected() -> ReplacementValue {
    ReplacementValue::Int(42)
}

fn replacement_body_v0_002_arguments() -> Vec<ReplacementValue> {
    vec![ReplacementValue::Str("Ada".to_string())]
}

fn replacement_body_v0_002_expected() -> ReplacementValue {
    ReplacementValue::Str("hello, Ada".to_string())
}

fn replacement_body_v0_003_arguments() -> Vec<ReplacementValue> {
    vec![]
}

fn replacement_body_v0_003_expected() -> ReplacementValue {
    ReplacementValue::Str("owned".to_string())
}

fn replacement_body_v0_004_arguments() -> Vec<ReplacementValue> {
    vec![]
}

fn replacement_body_v0_004_expected() -> ReplacementValue {
    ReplacementValue::Int(10)
}

fn replacement_body_v0_005_arguments() -> Vec<ReplacementValue> {
    vec![ReplacementValue::Int(84), ReplacementValue::Int(2)]
}

fn replacement_body_v0_005_expected() -> ReplacementValue {
    ReplacementValue::Int(42)
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
            source: CASE_1_SRC,
            evaluate: Some(case_supported_match_exhaustiveness),
            replacement_execution: None,
        },
        ParityCase {
            id: "parity-987-0002",
            title: "Chained comparisons are rejected with a type-mismatch diagnostic",
            category: BehaviorCategory::DiagnosticBehavior,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs::case_diagnostic_chained_comparison_rejected",
            disposition: Disposition::Preserved,
            source: CASE_2_SRC,
            evaluate: Some(case_diagnostic_chained_comparison_rejected),
            replacement_execution: None,
        },
        ParityCase {
            id: "parity-987-0003",
            title: "String membership (`in`) matches the runtime helper's substring policy",
            category: BehaviorCategory::StdlibRuntimeBehavior,
            lane: EvidenceLane::GeneratedProjectRun,
            evidence: "tests/parity_corpus_tests.rs::case_stdlib_runtime_string_membership",
            disposition: Disposition::Preserved,
            source: CASE_3_SRC,
            evaluate: Some(case_stdlib_runtime_string_membership),
            replacement_execution: None,
        },
        ParityCase {
            id: "parity-987-0004",
            title: "Generated Rust stays syntactically valid and inspectable, not the semantic contract",
            category: BehaviorCategory::GeneratedArtifactBehavior,
            lane: EvidenceLane::CodegenSnapshot,
            evidence: "tests/parity_corpus_tests.rs::case_generated_artifact_valid_rust_shape",
            disposition: Disposition::Preserved,
            source: CASE_4_SRC,
            evaluate: Some(case_generated_artifact_valid_rust_shape),
            replacement_execution: None,
        },
        ParityCase {
            id: "parity-987-0005",
            title: "A lexical builtin-name collision (`len`) preserves the module binding",
            category: BehaviorCategory::SupportedLanguageContract,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs::case_supported_builtin_len_shadowing",
            disposition: Disposition::Preserved,
            source: CASE_5_SRC,
            evaluate: Some(case_supported_builtin_len_shadowing),
            replacement_execution: None,
        },
        ParityCase {
            id: "parity-987-0006",
            title: "Dead code after `return` reports an unreachable-code warning (INCAN-T0101)",
            category: BehaviorCategory::DiagnosticBehavior,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs::case_diagnostic_unreachable_code_after_return",
            disposition: Disposition::IntentionalMigration {
                owning_issue: 1117,
                migration_note: "Migrated by #1117 before 0.6 cutover: statements after a `return` in the same \
                                  block now raise the non-fatal `INCAN-T0101` warning instead of typechecking \
                                  silently. Migration guidance for the replacement backend: the contract is the \
                                  frontend diagnostic, not generated Rust's own `unreachable_code` lint, so a \
                                  replacement backend must not be relied on to reproduce it. The rule is \
                                  deliberately block-local — it does not model divergence through `if`/`else`, \
                                  `match`, or loops — and existing user code with dead code still compiles, \
                                  because the diagnostic is a warning and never an error.",
            },
            source: CASE_6_SRC,
            evaluate: Some(case_diagnostic_unreachable_code_after_return),
            replacement_execution: None,
        },
        ParityCase {
            id: "replacement-body-v0-001",
            title: "Parameterized integer addition executes through Body IR",
            category: BehaviorCategory::SupportedLanguageContract,
            lane: EvidenceLane::DirectReplacementBodyIr,
            evidence: "#988 replacement-body-v0-001; tests/parity_corpus_tests.rs::seed_corpus",
            disposition: Disposition::Preserved,
            source: REPLACEMENT_BODY_V0_001_SRC,
            evaluate: None,
            replacement_execution: Some(parity_corpus::ReplacementExecutionPlan {
                function: "add",
                arguments: replacement_body_v0_001_arguments,
                expected: replacement_body_v0_001_expected,
            }),
        },
        ParityCase {
            id: "replacement-body-v0-002",
            title: "Parameterized string concatenation executes through Body IR",
            category: BehaviorCategory::SupportedLanguageContract,
            lane: EvidenceLane::DirectReplacementBodyIr,
            evidence: "#988 replacement-body-v0-002; tests/parity_corpus_tests.rs::seed_corpus",
            disposition: Disposition::Preserved,
            source: REPLACEMENT_BODY_V0_002_SRC,
            evaluate: None,
            replacement_execution: Some(parity_corpus::ReplacementExecutionPlan {
                function: "greet",
                arguments: replacement_body_v0_002_arguments,
                expected: replacement_body_v0_002_expected,
            }),
        },
        ParityCase {
            id: "replacement-body-v0-003",
            title: "Owned local return preserves move evidence through Body IR",
            category: BehaviorCategory::SupportedLanguageContract,
            lane: EvidenceLane::DirectReplacementBodyIr,
            evidence: "#988 replacement-body-v0-003; tests/parity_corpus_tests.rs::seed_corpus",
            disposition: Disposition::Preserved,
            source: REPLACEMENT_BODY_V0_003_SRC,
            evaluate: None,
            replacement_execution: Some(parity_corpus::ReplacementExecutionPlan {
                function: "return_owned",
                arguments: replacement_body_v0_003_arguments,
                expected: replacement_body_v0_003_expected,
            }),
        },
        ParityCase {
            id: "replacement-body-v0-004",
            title: "Normalized range, branch, and while control flow execute through Body IR",
            category: BehaviorCategory::SupportedLanguageContract,
            lane: EvidenceLane::DirectReplacementBodyIr,
            evidence: "#988 replacement-body-v0-004; tests/parity_corpus_tests.rs::seed_corpus",
            disposition: Disposition::Preserved,
            source: REPLACEMENT_BODY_V0_004_SRC,
            evaluate: None,
            replacement_execution: Some(parity_corpus::ReplacementExecutionPlan {
                function: "control_flow",
                arguments: replacement_body_v0_004_arguments,
                expected: replacement_body_v0_004_expected,
            }),
        },
        ParityCase {
            id: "replacement-body-v0-005",
            title: "Assertion and floor division execute through Body IR",
            category: BehaviorCategory::SupportedLanguageContract,
            lane: EvidenceLane::DirectReplacementBodyIr,
            evidence: "#988 replacement-body-v0-005; tests/parity_corpus_tests.rs::seed_corpus",
            disposition: Disposition::Preserved,
            source: REPLACEMENT_BODY_V0_005_SRC,
            evaluate: None,
            replacement_execution: Some(parity_corpus::ReplacementExecutionPlan {
                function: "guarded_floor_div",
                arguments: replacement_body_v0_005_arguments,
                expected: replacement_body_v0_005_expected,
            }),
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
            // Never reaches `evaluate_case` — `red_state_validate_corpus_...` calls
            // `validate_corpus` directly, so this placeholder source is never hashed into a real receipt.
            source: "",
            evaluate: Some(|| ComparisonOutcome::Match),
            replacement_execution: None,
        },
        ParityCase {
            id: "parity-987-dup",
            title: "Second of a duplicate pair (same id — must be flagged)",
            category: BehaviorCategory::SupportedLanguageContract,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs (red-state fixture)",
            disposition: Disposition::Preserved,
            // Never reaches `evaluate_case` — `red_state_validate_corpus_...` calls
            // `validate_corpus` directly, so this placeholder source is never hashed into a real receipt.
            source: "",
            evaluate: Some(|| ComparisonOutcome::Match),
            replacement_execution: None,
        },
        ParityCase {
            id: "parity-987-empty-title",
            title: "",
            category: BehaviorCategory::DiagnosticBehavior,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs (red-state fixture)",
            disposition: Disposition::Preserved,
            // Never reaches `evaluate_case` — `red_state_validate_corpus_...` calls
            // `validate_corpus` directly, so this placeholder source is never hashed into a real receipt.
            source: "",
            evaluate: Some(|| ComparisonOutcome::Match),
            replacement_execution: None,
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
            // Never reaches `evaluate_case` — `red_state_validate_corpus_...` calls
            // `validate_corpus` directly, so this placeholder source is never hashed into a real receipt.
            source: "",
            evaluate: Some(|| ComparisonOutcome::Match),
            replacement_execution: None,
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
            id.starts_with("parity-987-") || id.starts_with("replacement-body-v0-"),
            "case id {id} must carry a stable #987 or #988 replacement-body namespace prefix"
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
fn seed_cases_and_direct_replacement_cases_remain_non_green_without_a_legacy_runtime_comparator() {
    // This is the corpus's core promise: direct replacement execution does not become green parity merely because
    // it has a receipt. The older seed rows and the new #988 rows both stay explicitly shadow-unavailable until a
    // source-observable legacy comparison is available for the same source profile.
    let summary = parity_corpus::summarize(&seed_corpus());
    assert!(
        summary.receipt_schema_available,
        "the summary must say the #986 receipt schema is available now that PR #1120 landed it"
    );
    let falsely_green: Vec<&parity_corpus::CaseReport> = summary
        .cases
        .iter()
        .filter(|c| c.overall_state == OverallState::Green)
        .collect();
    assert!(
        falsely_green.is_empty(),
        "no existing seed case should reach OverallState::Green without its own replacement execution: {falsely_green:#?}"
    );
    assert_eq!(summary.green, 0);
    assert_eq!(
        summary.non_green_shadow_unavailable, summary.total_cases,
        "every corpus row must report shadow-unavailable, not green, until it has a source-observable legacy comparison"
    );
    assert_eq!(summary.non_green_behavior, 0);
}

/// Bind each selected #988 source case to its own replacement receipt and complete Body-IR proof evidence.
#[test]
fn replacement_body_v0_cases_have_receipt_bound_non_green_execution_evidence() -> Result<(), Box<dyn std::error::Error>>
{
    let summary = parity_corpus::summarize(&seed_corpus());
    let replacement_rows: Vec<&parity_corpus::CaseReport> = summary
        .cases
        .iter()
        .filter(|case| case.id.starts_with("replacement-body-v0-"))
        .collect();
    assert_eq!(
        replacement_rows.len(),
        5,
        "the five agreed #988 cases must stay stable in #987"
    );

    for row in replacement_rows {
        assert_eq!(row.lane, EvidenceLane::DirectReplacementBodyIr);
        assert_eq!(row.overall_state, OverallState::NonGreenShadowUnavailable);
        match &row.receipt {
            ReceiptRef::ReplacementExecuted {
                selection_identity,
                receipt_identity,
                output_identity,
                body_snapshot,
                ownership_reads,
                runtime_requirements,
                comparison_reason,
            } => {
                assert!(selection_identity.starts_with("sha256:"));
                assert!(receipt_identity.starts_with("sha256:"));
                assert!(output_identity.starts_with("sha256:"));
                assert!(body_snapshot.contains("body "));
                assert!(
                    ownership_reads
                        .iter()
                        .all(|read| read.span_end >= read.span_start && !read.fact.is_empty()),
                    "{} lost canonical ownership evidence: {ownership_reads:?}",
                    row.id
                );
                assert!(
                    runtime_requirements
                        .iter()
                        .all(|requirement| !requirement.requirement.is_empty()),
                    "{} emitted an invalid runtime-requirement projection: {runtime_requirements:?}",
                    row.id
                );
                assert_eq!(
                    comparison_reason,
                    incan::backend::selection::SHADOW_COMPARISON_UNAVAILABLE_REASON,
                    "{} must report the actual missing legacy comparator rather than generated-Rust evidence",
                    row.id
                );
            }
            receipt => {
                return Err(format!(
                    "{} needs its own replacement execution receipt, got {receipt:?}",
                    row.id
                )
                .into());
            }
        }
    }
    Ok(())
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
        "non_green_shadow_unavailable",
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
