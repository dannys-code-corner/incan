//! Unit coverage for the parts of the comparison that do not need a staged Oven capability.
//!
//! Everything here is about the *rules*: how a result is framed and recovered, how failures are classified, when
//! two observations may be compared at all, and what survives a comparison that could not run. The end-to-end
//! proof that two real executions agree lives in `tests/shadow_comparison_tests.rs`, because only a real Oven
//! build can supply it.

use super::*;

fn profile() -> ShadowComparisonProfile {
    ShadowComparisonProfile::new(
        "def add(x: int, y: int) -> int:\n    return x + y\n",
        "add",
        vec![ReplacementValue::Int(40), ReplacementValue::Int(2)],
    )
}

fn authority() -> LegacyExecutionAuthority {
    LegacyExecutionAuthority {
        oven_receipt_identity: "sha256:oven-receipt".to_string(),
        oven_build_unit_identity: "sha256:build-unit".to_string(),
        direct_rustc_plan_identity: "sha256:plan".to_string(),
        output_digest: "sha256:output".to_string(),
        cargo_process_started: false,
    }
}

fn observation(profile_identity: &str, observable: SourceObservable, detail: &str) -> RouteObservation {
    RouteObservation {
        profile_kind: SHADOW_COMPARISON_PROFILE_ID.to_string(),
        profile_identity: profile_identity.to_string(),
        output_identity: digest_output(&["test", detail]),
        observable,
        detail: detail.to_string(),
    }
}

fn completed(result: &str) -> SourceObservable {
    SourceObservable::Completed {
        result: result.to_string(),
    }
}

fn framed(payload: &str) -> Vec<u8> {
    format!("{RESULT_BEGIN_MARKER}\n{payload}\n{RESULT_END_MARKER}\n").into_bytes()
}

// ============================================================================
// Lossless transport
// ============================================================================

#[test]
fn a_framed_result_round_trips_exactly() -> Result<(), ShadowUnavailable> {
    assert_eq!(decode_framed_result(&framed("42"))?, "42");
    assert_eq!(decode_framed_result(&framed(""))?, "");
    assert_eq!(decode_framed_result(&framed("  padded  "))?, "  padded  ");
    Ok(())
}

#[test]
fn trailing_newlines_in_a_result_survive_transport() -> Result<(), ShadowUnavailable> {
    // The whole point of framing: `"x"`, `"x\n"`, and `"\n"` must stay three distinct observations. Trimming the
    // process output would collapse them and report a real divergence as a match.
    assert_eq!(decode_framed_result(&framed("x"))?, "x");
    assert_eq!(decode_framed_result(&framed("x\n"))?, "x\n");
    assert_eq!(decode_framed_result(&framed("\n"))?, "\n");
    assert_ne!(
        completed(&decode_framed_result(&framed("x"))?),
        completed(&decode_framed_result(&framed("x\n"))?)
    );
    Ok(())
}

#[test]
fn a_result_containing_the_end_marker_is_still_recovered_exactly() -> Result<(), ShadowUnavailable> {
    // The frame is positional, so a payload that happens to contain the marker text cannot truncate the result.
    let payload = format!("before\n{RESULT_END_MARKER}\nafter");
    assert_eq!(decode_framed_result(&framed(&payload))?, payload);
    Ok(())
}

#[test]
fn unframed_output_is_unavailable_rather_than_guessed() {
    assert!(decode_framed_result(b"42\n").is_err());
    assert!(decode_framed_result(b"").is_err());
    assert!(decode_framed_result(format!("{RESULT_BEGIN_MARKER}\n42\n").as_bytes()).is_err());
    let mut leading_noise = b"unexpected\n".to_vec();
    leading_noise.extend_from_slice(&framed("42"));
    assert!(decode_framed_result(&leading_noise).is_err());
}

#[test]
fn a_non_utf8_result_is_unavailable_rather_than_lossily_converted() {
    let mut output = format!("{RESULT_BEGIN_MARKER}\n").into_bytes();
    output.push(0xFF);
    output.extend_from_slice(format!("\n{RESULT_END_MARKER}\n").as_bytes());
    assert!(decode_framed_result(&output).is_err());
}

#[test]
fn the_generated_entrypoint_frames_the_observed_call() -> Result<(), ShadowUnavailable> {
    let program = profile().legacy_program_source()?;
    assert!(program.contains("def add(x: int, y: int) -> int:"), "{program}");
    assert!(
        program.contains(&format!("println(\"{RESULT_BEGIN_MARKER}\")")),
        "{program}"
    );
    assert!(program.contains("println(add(40, 2))"), "{program}");
    assert!(
        program.contains(&format!("println(\"{RESULT_END_MARKER}\")")),
        "{program}"
    );
    Ok(())
}

// ============================================================================
// Classification
// ============================================================================

#[test]
fn agreeing_observations_record_the_profile_and_the_compared_value() {
    let state = classify_observations(
        &observation("sha256:profile", completed("42"), "legacy"),
        &observation("sha256:profile", completed("42"), "replacement"),
    );
    assert_eq!(
        state,
        ShadowComparisonState::Matched {
            profile_kind: SHADOW_COMPARISON_PROFILE_ID.to_string(),
            profile_identity: "sha256:profile".to_string(),
            observable: "completed(\"42\")".to_string(),
        }
    );
}

#[test]
fn differing_results_diverge_and_name_both_sides() {
    let state = classify_observations(
        &observation("sha256:profile", completed("42"), "legacy detail"),
        &observation("sha256:profile", completed("43"), "replacement detail"),
    );
    let ShadowComparisonState::Diverged {
        profile_kind,
        profile_identity,
        detail,
    } = state
    else {
        panic!("differing observables must diverge");
    };
    assert_eq!(profile_kind, SHADOW_COMPARISON_PROFILE_ID);
    assert_eq!(profile_identity, "sha256:profile");
    assert!(detail.contains("completed(\"42\")"), "{detail}");
    assert!(detail.contains("completed(\"43\")"), "{detail}");
}

#[test]
fn a_whitespace_only_difference_diverges_and_stays_visible_in_the_detail() {
    let state = classify_observations(
        &observation("sha256:profile", completed("x"), "legacy"),
        &observation("sha256:profile", completed("x\n"), "replacement"),
    );
    let ShadowComparisonState::Diverged { detail, .. } = state else {
        panic!("`x` and `x\\n` are different results and must diverge");
    };
    assert!(detail.contains(r#"completed("x")"#), "{detail}");
    assert!(detail.contains(r#"completed("x\n")"#), "{detail}");
}

#[test]
fn observations_of_different_profiles_are_never_compared() {
    // Pairing two unrelated profile instances would manufacture a verdict about a comparison nobody ran.
    let state = classify_observations(
        &observation("sha256:profile-a", completed("42"), "legacy"),
        &observation("sha256:profile-b", completed("43"), "replacement"),
    );
    let ShadowComparisonState::Unavailable { reason } = state else {
        panic!("cross-profile observations must not produce a comparison verdict");
    };
    assert!(reason.contains("different comparison profiles"), "{reason}");
}

#[test]
fn a_completed_result_never_matches_a_runtime_failure() {
    let state = classify_observations(
        &observation(
            "sha256:profile",
            SourceObservable::Failed {
                failure: RuntimeFailureClass::Assertion,
            },
            "legacy",
        ),
        &observation("sha256:profile", completed("0"), "replacement"),
    );
    assert!(matches!(state, ShadowComparisonState::Diverged { .. }));
}

#[test]
fn different_failure_classes_diverge_rather_than_agreeing_that_something_broke() {
    let state = classify_observations(
        &observation(
            "sha256:profile",
            SourceObservable::Failed {
                failure: RuntimeFailureClass::ArithmeticOverflow,
            },
            "legacy",
        ),
        &observation(
            "sha256:profile",
            SourceObservable::Failed {
                failure: RuntimeFailureClass::DivisionByZero,
            },
            "replacement",
        ),
    );
    assert!(matches!(state, ShadowComparisonState::Diverged { .. }));
}

#[test]
fn an_overflow_is_not_classified_as_a_division_by_zero() -> Result<(), ShadowUnavailable> {
    // The executor spells an unrepresentable quotient as an "integer division overflow". Reading the word
    // "division" and filing it as a division by zero would make two different behaviors compare equal.
    assert_eq!(
        classify_replacement_failure("integer division overflow")?,
        RuntimeFailureClass::ArithmeticOverflow
    );
    assert_eq!(
        classify_replacement_failure("division or modulo by zero")?,
        RuntimeFailureClass::DivisionByZero
    );
    assert_eq!(
        classify_legacy_failure("attempt to divide with overflow")?,
        RuntimeFailureClass::ArithmeticOverflow
    );
    assert_eq!(
        classify_legacy_failure("ZeroDivisionError: division by zero")?,
        RuntimeFailureClass::DivisionByZero
    );
    Ok(())
}

#[test]
fn an_unclassifiable_failure_stays_unavailable_on_both_routes() {
    assert!(classify_legacy_failure("Segmentation fault").is_err());
    assert!(classify_replacement_failure("something went wrong").is_err());
}

#[test]
fn a_failing_legacy_exit_is_never_read_as_a_result() {
    // A non-zero exit must be classified as a failure, not decoded as output that happens to be framed.
    let observed = observe_legacy_process(
        SHADOW_COMPARISON_PROFILE_ID,
        "sha256:profile",
        &authority(),
        Some(101),
        &framed("42"),
        "",
    );
    assert!(
        observed.is_err(),
        "an unclassifiable failing exit must not yield a result"
    );
}

#[test]
fn the_legacy_output_identity_covers_its_oven_authority() -> Result<(), ShadowUnavailable> {
    let baseline = observe_legacy_process(
        SHADOW_COMPARISON_PROFILE_ID,
        "sha256:profile",
        &authority(),
        Some(0),
        &framed("42"),
        "",
    )?;
    let mut other_plan = authority();
    other_plan.direct_rustc_plan_identity = "sha256:different-plan".to_string();
    let under_other_plan = observe_legacy_process(
        SHADOW_COMPARISON_PROFILE_ID,
        "sha256:profile",
        &other_plan,
        Some(0),
        &framed("42"),
        "",
    )?;
    assert_eq!(baseline.observable, under_other_plan.observable);
    assert_ne!(
        baseline.output_identity, under_other_plan.output_identity,
        "the same observed result under a different Oven authority is different evidence"
    );
    Ok(())
}

// ============================================================================
// Partial evidence
// ============================================================================

#[test]
fn an_executed_replacement_route_survives_an_unavailable_legacy_route() -> Result<(), Box<dyn std::error::Error>> {
    let profile = profile();
    let replacement = observe_replacement_route(&profile)?;
    let comparison = assemble_comparison(
        &profile,
        Ok(replacement),
        Err(ShadowUnavailable::new("no Oven plan is staged")),
    );

    let reason = comparison
        .unavailable_reason()
        .ok_or("a missing legacy route must record an unavailable comparison")?;
    assert!(reason.contains("no Oven plan is staged"), "{reason}");
    assert!(!comparison.matched());
    assert!(comparison.legacy.is_none(), "the legacy route did not execute");

    // The replacement route really ran, so its receipt and Body-IR evidence must not be thrown away.
    let replacement_evidence = comparison
        .replacement
        .as_ref()
        .ok_or("an executed replacement route must keep its receipt")?;
    let replacement_receipt = replacement_evidence.receipt()?;
    replacement_receipt.verify_identity()?;
    assert_eq!(replacement_receipt.shadow_comparison, comparison.state);
    assert_eq!(
        replacement_evidence.observation.observable,
        completed("42"),
        "the retained observation must be the one that was really executed"
    );
    let execution = comparison
        .replacement_execution
        .as_ref()
        .ok_or("an executed replacement route must keep its Body-IR execution")?;
    assert_eq!(execution.value, ReplacementValue::Int(42));
    Ok(())
}

#[test]
fn an_executed_route_without_a_receipt_reports_that_rather_than_vanishing() {
    // Receipt finalization cannot fail for this module's fixed inputs, so this covers the contract rather than a
    // reachable path: if it ever does fail, the route's execution stays visible and the missing receipt reads as
    // the explicit failure it is.
    let evidence = RouteEvidence {
        receipt: None,
        observation: observation("sha256:profile", completed("42"), "legacy"),
    };
    let Err(reason) = evidence.receipt() else {
        panic!("an evidence entry with no receipt must not report one");
    };
    assert!(reason.contains("could not be finalized"), "{reason}");
    assert_eq!(
        evidence.observation.observable,
        completed("42"),
        "the observation must survive a missing receipt"
    );
}

#[test]
fn a_comparison_with_no_executed_route_keeps_no_receipts() {
    let comparison = assemble_comparison(
        &profile(),
        Err(ShadowUnavailable::new("replacement refused")),
        Err(ShadowUnavailable::new("legacy unstaged")),
    );
    let Some(reason) = comparison.unavailable_reason() else {
        panic!("two failed routes must record an unavailable comparison");
    };
    assert!(reason.contains("replacement refused"), "{reason}");
    assert!(reason.contains("legacy unstaged"), "{reason}");
    assert!(comparison.legacy.is_none());
    assert!(comparison.replacement.is_none());
    assert!(comparison.legacy_authority.is_none());
}

// ============================================================================
// Profile boundaries
// ============================================================================

#[test]
fn observing_a_program_entrypoint_is_outside_the_profile() {
    let profile = ShadowComparisonProfile::new("def main() -> int:\n    return 42\n", "main", vec![]);
    let Err(unavailable) = profile.legacy_program_source() else {
        panic!("a `main` observation must be refused");
    };
    assert_eq!(unavailable.reason, PROGRAM_ENTRYPOINT_UNAVAILABLE_REASON);
}

#[test]
fn a_non_scalar_argument_is_refused_rather_than_guessed() {
    let profile = ShadowComparisonProfile::new(
        "def echo(value: int) -> int:\n    return value\n",
        "echo",
        vec![ReplacementValue::Tuple(vec![ReplacementValue::Int(1)])],
    );
    let Err(unavailable) = profile.legacy_program_source() else {
        panic!("a tuple argument has no source literal and must be refused");
    };
    assert!(unavailable.reason.contains("source literals"), "{}", unavailable.reason);
}

#[test]
fn string_arguments_are_escaped_into_incan_literals() -> Result<(), ShadowUnavailable> {
    let profile = ShadowComparisonProfile::new(
        "def greet(name: str) -> str:\n    return \"hello, \" + name\n",
        "greet",
        vec![ReplacementValue::Str("A\"da\\".to_string())],
    );
    let program = profile.legacy_program_source()?;
    assert!(program.contains(r#"println(greet("A\"da\\"))"#), "{program}");
    Ok(())
}

#[test]
fn arguments_are_part_of_the_profile_identity() {
    let source = "def add(x: int, y: int) -> int:\n    return x + y\n";
    let first = ShadowComparisonProfile::new(source, "add", vec![ReplacementValue::Int(40), ReplacementValue::Int(2)]);
    let second = ShadowComparisonProfile::new(source, "add", vec![ReplacementValue::Int(40), ReplacementValue::Int(3)]);
    assert_ne!(first.profile_identity(), second.profile_identity());
    assert_eq!(
        first.source_identity(),
        second.source_identity(),
        "the same module must keep one source identity across argument lists"
    );
}
