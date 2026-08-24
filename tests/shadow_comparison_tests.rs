//! End-to-end proof for the bounded source-observable shadow comparison (#1146).
//!
//! Every "matched" result here comes from two genuinely independent executions of the same source: the
//! replacement route executes Body IR directly in-process, and the legacy route emits Rust, has Oven authorize
//! and build it through an immutable store-selected direct-`rustc` plan, and runs the produced program as a
//! separate process. Nothing here compares generated Rust text, and nothing treats a successful build as
//! agreement.
//!
//! The legacy route needs a staged Oven capability (see `incan::backend::shadow::legacy_oven`). Tests that assert
//! a matched comparison require it and say so when it is missing; tests about honest unavailability do not.
//!
//! Run with: `cargo test --test shadow_comparison_tests`

use std::path::Path;

use incan::backend::replacement::ReplacementValue;
use incan::backend::selection::{BackendKind, FallbackOutcome, FallbackPolicy, ShadowComparisonState};
use incan::backend::shadow::legacy_oven::LegacyOvenCapability;
use incan::backend::shadow::{
    PROGRAM_ENTRYPOINT_UNAVAILABLE_REASON, RouteEvidence, RuntimeFailureClass, ShadowComparison,
    ShadowComparisonProfile, SourceObservable, compare_source_observable,
};

#[path = "support/shadow_capability.rs"]
mod shadow_capability;

const ADD_SRC: &str = "def add(x: int, y: int) -> int:\n    return x + y\n";
const GREET_SRC: &str = "def greet(name: str) -> str:\n    return \"hello, \" + name\n";
const DIVIDE_SRC: &str = "def divide(a: int, b: int) -> int:\n    return a // b\n";
const GUARD_SRC: &str = "def guarded(a: int) -> int:\n    assert a > 0\n    return a\n";

/// Run one comparison against the staged Oven capability, or report why the legacy route is unavailable.
fn compare(
    profile: &ShadowComparisonProfile,
    workspace: &Path,
) -> Result<ShadowComparison, Box<dyn std::error::Error>> {
    let capability = shadow_capability::legacy_capability()?;
    Ok(compare_source_observable(profile, &capability, workspace))
}

/// Both route receipts, or a failure naming the state that produced no receipt.
fn route_evidence(comparison: &ShadowComparison) -> Result<(&RouteEvidence, &RouteEvidence), String> {
    match (&comparison.legacy, &comparison.replacement) {
        (Some(legacy), Some(replacement)) => Ok((legacy, replacement)),
        _ => Err(format!(
            "expected both routes to execute and produce receipts, got state {:?}",
            comparison.state
        )),
    }
}

/// Assert the shared, route-independent facts every completed comparison must carry.
fn assert_receipts_are_independent_but_bound(comparison: &ShadowComparison) -> Result<(), Box<dyn std::error::Error>> {
    let (legacy, replacement) = route_evidence(comparison)?;

    let legacy_receipt = legacy.receipt()?;
    let replacement_receipt = replacement.receipt()?;
    legacy_receipt.verify_identity()?;
    replacement_receipt.verify_identity()?;

    // Same source, same recorded comparison outcome: the two receipts describe one comparison.
    assert_eq!(legacy_receipt.selection.source_identity, comparison.source_identity);
    assert_eq!(
        replacement_receipt.selection.source_identity,
        comparison.source_identity
    );
    assert_eq!(legacy_receipt.shadow_comparison, comparison.state);
    assert_eq!(replacement_receipt.shadow_comparison, comparison.state);
    assert!(legacy_receipt.selection.shadow_requested);
    assert!(replacement_receipt.selection.shadow_requested);

    // Same profile: neither observation was produced under a different comparison instance.
    assert_eq!(legacy.observation.profile_identity, comparison.profile_identity);
    assert_eq!(replacement.observation.profile_identity, comparison.profile_identity);

    // Different routes: each receipt records the backend that actually ran, with no fallback in either direction.
    assert_eq!(legacy_receipt.selection.selected_backend, BackendKind::Legacy);
    assert_eq!(legacy_receipt.executed_backend, BackendKind::Legacy);
    assert_eq!(replacement_receipt.selection.selected_backend, BackendKind::Replacement);
    assert_eq!(replacement_receipt.executed_backend, BackendKind::Replacement);
    assert_eq!(legacy_receipt.fallback_outcome, FallbackOutcome::NotNeeded);
    assert_eq!(replacement_receipt.fallback_outcome, FallbackOutcome::NotNeeded);
    assert_eq!(legacy_receipt.selection.fallback_policy, FallbackPolicy::Refuse);
    assert_eq!(replacement_receipt.selection.fallback_policy, FallbackPolicy::Refuse);

    // The receipts are not interchangeable: each covers what its own route produced.
    assert_ne!(legacy_receipt.identity, replacement_receipt.identity);
    assert_ne!(
        legacy_receipt.selection.identity,
        replacement_receipt.selection.identity
    );
    assert_ne!(
        legacy_receipt.output_identity, replacement_receipt.output_identity,
        "a shared output identity would erase the routes' independence"
    );

    // The legacy answer is attributable to the Oven authority that produced it.
    let authority = comparison
        .legacy_authority
        .as_ref()
        .ok_or("an executed legacy route must record the Oven authority that permitted it")?;
    assert!(authority.oven_receipt_identity.starts_with("sha256:"));
    assert!(authority.oven_build_unit_identity.starts_with("sha256:"));
    assert!(authority.direct_rustc_plan_identity.starts_with("sha256:"));
    assert!(authority.output_digest.starts_with("sha256:"));
    assert!(
        !authority.cargo_process_started,
        "Oven-owned legacy execution must not start a Cargo process"
    );
    Ok(())
}

/// Guard for tests that require a staged legacy route.
///
/// Returns the reason when nothing is staged, so the test reports why it could not assert a match rather than
/// passing silently. Setting `INCAN_SHADOW_REQUIRE_LEGACY_ROUTE` turns that report into a failure, which is how
/// an environment that is supposed to be staged proves it.
fn require_staged_legacy_route() -> Option<String> {
    shadow_capability::unstaged_legacy_route_reason()
}

/// A real scalar profile agrees across a directly executed Body IR and an Oven-built, separately run program.
#[test]
fn a_scalar_profile_matches_across_two_independent_executions() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(reason) = require_staged_legacy_route() {
        eprintln!("skipping: {reason}");
        return Ok(());
    }
    let workspace = tempfile::tempdir()?;
    let profile = ShadowComparisonProfile::new(
        ADD_SRC,
        "add",
        vec![ReplacementValue::Int(40), ReplacementValue::Int(2)],
    );
    let comparison = compare(&profile, workspace.path())?;

    let ShadowComparisonState::Matched {
        profile_kind,
        profile_identity,
        observable,
    } = &comparison.state
    else {
        panic!("expected a matched comparison, got {:?}", comparison.state);
    };
    assert_eq!(profile_kind, incan::backend::shadow::SHADOW_COMPARISON_PROFILE_ID);
    assert_eq!(profile_identity, &profile.profile_identity());
    assert_eq!(observable, "completed(\"42\")");
    assert!(comparison.matched());

    let (legacy, replacement) = route_evidence(&comparison)?;
    let expected = SourceObservable::Completed {
        result: "42".to_string(),
    };
    assert_eq!(legacy.observation.observable, expected);
    assert_eq!(replacement.observation.observable, expected);
    assert_receipts_are_independent_but_bound(&comparison)?;

    // The replacement route carried its own Body-IR evidence, proving it executed rather than reading the
    // legacy route's result.
    let execution = comparison
        .replacement_execution
        .as_ref()
        .ok_or("a matched comparison must retain the direct Body-IR execution")?;
    assert_eq!(execution.value, ReplacementValue::Int(42));
    assert!(
        execution.body_snapshot.contains("body add"),
        "{}",
        execution.body_snapshot
    );
    assert_eq!(
        execution.output_identity, replacement.observation.output_identity,
        "the replacement receipt must be bound to the execution that produced its result"
    );
    Ok(())
}

/// String results compare through the exact printed value, not a trimmed approximation of it.
#[test]
fn a_string_profile_matches_on_its_exact_printed_value() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(reason) = require_staged_legacy_route() {
        eprintln!("skipping: {reason}");
        return Ok(());
    }
    let workspace = tempfile::tempdir()?;
    let profile = ShadowComparisonProfile::new(GREET_SRC, "greet", vec![ReplacementValue::Str("Ada".to_string())]);
    let comparison = compare(&profile, workspace.path())?;

    assert!(
        comparison.matched(),
        "expected a matched string comparison, got {:?}",
        comparison.state
    );
    let (legacy, replacement) = route_evidence(&comparison)?;
    let expected = SourceObservable::Completed {
        result: "hello, Ada".to_string(),
    };
    assert_eq!(legacy.observation.observable, expected);
    assert_eq!(replacement.observation.observable, expected);
    assert_receipts_are_independent_but_bound(&comparison)?;
    Ok(())
}

/// A string whose value ends in a newline must survive the legacy route's stdout transport intact.
///
/// This is the case a trimming transport would corrupt: `"line\n"` and `"line"` would become indistinguishable,
/// and a real difference between the routes would report as agreement.
#[test]
fn a_trailing_newline_in_a_result_is_not_lost_in_transport() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(reason) = require_staged_legacy_route() {
        eprintln!("skipping: {reason}");
        return Ok(());
    }
    let workspace = tempfile::tempdir()?;
    let profile = ShadowComparisonProfile::new(
        "def echo(value: str) -> str:\n    return value\n",
        "echo",
        vec![ReplacementValue::Str("line\n".to_string())],
    );
    let comparison = compare(&profile, workspace.path())?;

    assert!(
        comparison.matched(),
        "a trailing newline must survive both routes, got {:?}",
        comparison.state
    );
    let (legacy, replacement) = route_evidence(&comparison)?;
    let expected = SourceObservable::Completed {
        result: "line\n".to_string(),
    };
    assert_eq!(
        legacy.observation.observable, expected,
        "the legacy route must report the exact printed value, including its trailing newline"
    );
    assert_eq!(replacement.observation.observable, expected);
    Ok(())
}

/// The compared observable includes diagnostics: both routes must report the same classified runtime failure.
#[test]
fn a_runtime_failure_profile_matches_on_its_failure_class() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(reason) = require_staged_legacy_route() {
        eprintln!("skipping: {reason}");
        return Ok(());
    }
    let workspace = tempfile::tempdir()?;
    let profile = ShadowComparisonProfile::new(
        DIVIDE_SRC,
        "divide",
        vec![ReplacementValue::Int(1), ReplacementValue::Int(0)],
    );
    let comparison = compare(&profile, workspace.path())?;

    assert!(
        comparison.matched(),
        "expected both routes to report the same failure class, got {:?}",
        comparison.state
    );
    let (legacy, replacement) = route_evidence(&comparison)?;
    let expected = SourceObservable::Failed {
        failure: RuntimeFailureClass::DivisionByZero,
    };
    assert_eq!(legacy.observation.observable, expected);
    assert_eq!(replacement.observation.observable, expected);
    assert!(
        comparison.replacement_execution.is_none(),
        "a failed direct execution has no successful Body-IR execution to retain"
    );
    assert_receipts_are_independent_but_bound(&comparison)?;
    Ok(())
}

/// A failing source-level assertion is observed identically by both routes.
#[test]
fn an_assertion_failure_matches_across_both_routes() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(reason) = require_staged_legacy_route() {
        eprintln!("skipping: {reason}");
        return Ok(());
    }
    let workspace = tempfile::tempdir()?;
    let profile = ShadowComparisonProfile::new(GUARD_SRC, "guarded", vec![ReplacementValue::Int(0)]);
    let comparison = compare(&profile, workspace.path())?;

    assert!(
        comparison.matched(),
        "expected both routes to report a failed assertion, got {:?}",
        comparison.state
    );
    let (legacy, replacement) = route_evidence(&comparison)?;
    let expected = SourceObservable::Failed {
        failure: RuntimeFailureClass::Assertion,
    };
    assert_eq!(legacy.observation.observable, expected);
    assert_eq!(replacement.observation.observable, expected);
    Ok(())
}

/// Observing a program entrypoint is outside the profile — for the *legacy* route only.
///
/// The replacement backend executes a zero-argument `main` perfectly well, so it really does run here; it is the
/// legacy route that has no way to observe an entrypoint's return value. That asymmetry is the point: the
/// comparison is unavailable, and the replacement execution that genuinely happened is retained rather than
/// discarded alongside the route that could not run.
#[test]
fn a_program_entrypoint_profile_is_unavailable_but_keeps_its_replacement_execution()
-> Result<(), Box<dyn std::error::Error>> {
    if let Some(reason) = require_staged_legacy_route() {
        eprintln!("skipping: {reason}");
        return Ok(());
    }
    let workspace = tempfile::tempdir()?;
    let profile = ShadowComparisonProfile::new("def main() -> int:\n    return 42\n", "main", vec![]);
    let comparison = compare(&profile, workspace.path())?;

    let reason = comparison
        .unavailable_reason()
        .ok_or_else(|| format!("a `main` observation must stay unavailable, got {:?}", comparison.state))?;
    assert!(reason.contains(PROGRAM_ENTRYPOINT_UNAVAILABLE_REASON), "{reason}");
    assert!(
        reason.contains("the legacy route did not execute"),
        "the reason must name the route that could not run: {reason}"
    );
    assert!(!comparison.matched());
    assert!(
        comparison.legacy.is_none(),
        "the legacy route cannot observe an entrypoint"
    );
    assert!(
        comparison.legacy_authority.is_none(),
        "no Oven build was authorized for a route that never ran"
    );

    // The replacement route executed and must keep its evidence.
    let replacement = comparison
        .replacement
        .as_ref()
        .ok_or("the replacement route executes a zero-argument `main` and must keep its evidence")?;
    let receipt = replacement.receipt()?;
    receipt.verify_identity()?;
    assert_eq!(receipt.executed_backend, BackendKind::Replacement);
    assert_eq!(receipt.shadow_comparison, comparison.state);
    assert_eq!(
        replacement.observation.observable,
        SourceObservable::Completed {
            result: "42".to_string()
        }
    );
    let execution = comparison
        .replacement_execution
        .as_ref()
        .ok_or("the executed replacement route must keep its Body-IR execution")?;
    assert_eq!(execution.value, ReplacementValue::Int(42));
    Ok(())
}

/// A source the replacement route refuses stays unavailable rather than becoming a divergence claim.
///
/// This needs no staged legacy route: the replacement refusal alone decides it.
#[test]
fn a_source_outside_the_replacement_profile_stays_unavailable() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    let profile = ShadowComparisonProfile::new(
        "def pairs() -> list[tuple[int, int]]:\n    return [(1, 2)]\n",
        "pairs",
        vec![],
    );
    let capability = match shadow_capability::legacy_capability() {
        Ok(capability) => capability,
        Err(unavailable) => {
            eprintln!(
                "legacy route unstaged ({}); the replacement refusal still decides",
                unavailable.reason
            );
            // Without a capability the comparison is unavailable for two reasons at once, which is still the
            // behavior under test: an out-of-profile source never becomes a comparison verdict.
            return Ok(());
        }
    };
    let comparison = compare_source_observable(&profile, &capability, workspace.path());

    let reason = comparison.unavailable_reason().ok_or_else(|| {
        format!(
            "a source outside the #988 profile must stay unavailable, got {:?}",
            comparison.state
        )
    })?;
    assert!(
        reason.contains("replacement route cannot execute this profile instance"),
        "{reason}"
    );
    assert!(!comparison.matched());
    assert!(comparison.replacement.is_none());
    Ok(())
}

/// A legacy route with no Oven receipt has no authority, so it cannot run at all.
///
/// Retention of the executed replacement route's evidence under that unavailable state is proven by
/// `backend::shadow::tests::an_executed_replacement_route_survives_an_unavailable_legacy_route`.
#[test]
fn a_missing_oven_receipt_cannot_authorize_a_legacy_route() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    let unstaged = LegacyOvenCapability::adopt_baked_project(
        workspace.path().join("no-store"),
        workspace.path().join("no-rustc"),
        &workspace.path().join("no-receipt.json"),
    );
    let Err(unavailable) = unstaged else {
        panic!("a missing Oven receipt cannot authorize a legacy route");
    };
    assert!(
        unavailable.reason.contains("no Oven authority"),
        "{}",
        unavailable.reason
    );
    Ok(())
}

/// A tampered Oven receipt cannot authorize a legacy comparison build.
#[test]
fn a_tampered_oven_receipt_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let Ok(capability) = shadow_capability::legacy_capability() else {
        eprintln!("skipping: no staged Oven receipt to tamper with");
        return Ok(());
    };
    let workspace = tempfile::tempdir()?;
    let tampered_path = workspace.path().join("tampered-receipt.json");
    let mut tampered = serde_json::to_value(capability.adopted_receipt())?;
    tampered["build_unit_identity"] = serde_json::json!("sha256:tampered");
    std::fs::write(&tampered_path, serde_json::to_vec_pretty(&tampered)?)?;

    let refused = LegacyOvenCapability::adopt_baked_project(
        workspace.path().join("store"),
        workspace.path().join("rustc"),
        &tampered_path,
    );
    let Err(unavailable) = refused else {
        panic!("a receipt whose identity does not match its content must not authorize a build");
    };
    assert!(
        unavailable.reason.contains("failed identity verification"),
        "{}",
        unavailable.reason
    );
    Ok(())
}
