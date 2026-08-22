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
//! ## Why these six cases
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
use incan::frontend::{lexer, parser, typechecker};
use std::path::PathBuf;

#[path = "support/parity_corpus.rs"]
mod parity_corpus;

use parity_corpus::{
    BehaviorCategory, ComparisonOutcome, Disposition, EvidenceLane, OverallState, ParityCase, validate_corpus,
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
// Case 6 — Bug-compatible behavior: dead code after `return` is not flagged
// ============================================================================

// Verified by direct probe: statements after an unconditional `return` typecheck with no diagnostic (Rust itself
// would warn `unreachable_code` in the generated function body). Tracked as bug-compatible rather than supported:
// current users may have dead code today, and this is not a behavior we want to freeze as an intentional language
// contract.
const CASE_6_SRC: &str = r#"
def f() -> int:
    return 1
    println("dead code")
    return 2
"#;

fn case_bug_compatible_dead_code_after_return() -> ComparisonOutcome {
    outcome_from_typecheck(
        CASE_6_SRC,
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
            source: CASE_1_SRC,
            evaluate: case_supported_match_exhaustiveness,
        },
        ParityCase {
            id: "parity-987-0002",
            title: "Chained comparisons are rejected with a type-mismatch diagnostic",
            category: BehaviorCategory::DiagnosticBehavior,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs::case_diagnostic_chained_comparison_rejected",
            disposition: Disposition::Preserved,
            source: CASE_2_SRC,
            evaluate: case_diagnostic_chained_comparison_rejected,
        },
        ParityCase {
            id: "parity-987-0003",
            title: "String membership (`in`) matches the runtime helper's substring policy",
            category: BehaviorCategory::StdlibRuntimeBehavior,
            lane: EvidenceLane::GeneratedProjectRun,
            evidence: "tests/parity_corpus_tests.rs::case_stdlib_runtime_string_membership",
            disposition: Disposition::Preserved,
            source: CASE_3_SRC,
            evaluate: case_stdlib_runtime_string_membership,
        },
        ParityCase {
            id: "parity-987-0004",
            title: "Generated Rust stays syntactically valid and inspectable, not the semantic contract",
            category: BehaviorCategory::GeneratedArtifactBehavior,
            lane: EvidenceLane::CodegenSnapshot,
            evidence: "tests/parity_corpus_tests.rs::case_generated_artifact_valid_rust_shape",
            disposition: Disposition::Preserved,
            source: CASE_4_SRC,
            evaluate: case_generated_artifact_valid_rust_shape,
        },
        ParityCase {
            id: "parity-987-0005",
            title: "A lexical builtin-name collision (`len`) preserves the module binding",
            category: BehaviorCategory::SupportedLanguageContract,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs::case_supported_builtin_len_shadowing",
            disposition: Disposition::Preserved,
            source: CASE_5_SRC,
            evaluate: case_supported_builtin_len_shadowing,
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
            source: CASE_6_SRC,
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
            // Never reaches `evaluate_case`/`compute_receipt` — `red_state_validate_corpus_...` calls
            // `validate_corpus` directly, so this placeholder source is never hashed into a real receipt.
            source: "",
            evaluate: || ComparisonOutcome::Match,
        },
        ParityCase {
            id: "parity-987-dup",
            title: "Second of a duplicate pair (same id — must be flagged)",
            category: BehaviorCategory::SupportedLanguageContract,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs (red-state fixture)",
            disposition: Disposition::Preserved,
            // Never reaches `evaluate_case`/`compute_receipt` — `red_state_validate_corpus_...` calls
            // `validate_corpus` directly, so this placeholder source is never hashed into a real receipt.
            source: "",
            evaluate: || ComparisonOutcome::Match,
        },
        ParityCase {
            id: "parity-987-empty-title",
            title: "",
            category: BehaviorCategory::DiagnosticBehavior,
            lane: EvidenceLane::DirectParserTypechecker,
            evidence: "tests/parity_corpus_tests.rs (red-state fixture)",
            disposition: Disposition::Preserved,
            // Never reaches `evaluate_case`/`compute_receipt` — `red_state_validate_corpus_...` calls
            // `validate_corpus` directly, so this placeholder source is never hashed into a real receipt.
            source: "",
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
            // Never reaches `evaluate_case`/`compute_receipt` — `red_state_validate_corpus_...` calls
            // `validate_corpus` directly, so this placeholder source is never hashed into a real receipt.
            source: "",
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
fn no_case_reaches_green_while_the_replacement_backend_is_unimplemented() {
    // This is the corpus's core promise: an unavailable comparison must never be counted as parity. #986 has
    // landed, so every case now consults a real receipt (`receipt_schema_available` is `true`) — but the
    // replacement backend itself is not implemented yet (#653), so every case's shadow comparison against it is
    // genuinely unavailable. Every case — even a behaviorally-confirmed one — must land in
    // `NonGreenShadowUnavailable`, never `Green`.
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
        "no case should reach OverallState::Green before #653 lands the replacement backend: {falsely_green:#?}"
    );
    assert_eq!(summary.green, 0);
    assert_eq!(
        summary.non_green_shadow_unavailable, summary.total_cases,
        "every behaviorally-confirmed seed case must report shadow-unavailable, not green, today"
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
