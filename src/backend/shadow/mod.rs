//! One bounded, source-observable shadow comparison between the legacy and replacement backends (#1146).
//!
//! #986 gave the compiler a place to *record* a shadow comparison ([`ShadowComparisonState`]) but no way to run
//! one, so every requested comparison was `Unavailable`. This module supplies the first comparison that actually
//! runs, for one deliberately narrow profile, and keeps every other profile explicitly unavailable.
//!
//! ## What is compared
//!
//! The profile is [`SHADOW_COMPARISON_PROFILE_ID`]: one source-only Incan module holding exactly one free
//! function, whose name is not `main`, called with concrete scalar arguments. Both routes observe the *same*
//! source, independently:
//!
//! - **Replacement route.** The module is typechecked, lowered to Body IR, and executed directly by
//!   [`crate::backend::replacement`]. Nothing is generated, compiled, or spawned.
//! - **Legacy route.** The module plus a generated Incan entrypoint that calls the observed function with the profile's
//!   arguments and prints the result is run through Oven, the adopted build and execution authority: [`IrCodegen`]
//!   emits Rust, an Oven receipt authorizes those exact bytes, an immutable store-selected direct-`rustc` plan compiles
//!   them, and the produced program is executed as a process. See [`legacy_oven`] for that boundary.
//!
//! The observed function must not be named `main`, because the legacy route can only observe a scalar by calling
//! the function from an entrypoint and printing the result; a function that *is* the entrypoint has no
//! source-observable return value at all. This is why the CLI's `--backend replacement` build path, which
//! executes the module's `main`, is outside this profile — see [`PROGRAM_ENTRYPOINT_UNAVAILABLE_REASON`].
//!
//! ## The observable is transported losslessly
//!
//! The legacy route reads its result out of a real process's standard output, so the transport has to be exact.
//! Trimming whitespace would silently equate `"x"` with `"x\n"` and `""` with `"\n"`, turning a genuine
//! divergence into a match. The generated entrypoint therefore frames the result between two marker lines, and
//! [`decode_framed_result`] recovers the payload by anchoring on the exact leading and trailing byte sequences —
//! never by searching, trimming, or lossy UTF-8 conversion. Output that does not carry the frame exactly is
//! [`ShadowUnavailable`], because a result that cannot be read back byte-for-byte has not been observed.
//!
//! ## What "matched" is allowed to mean
//!
//! [`SourceObservable`] is the whole comparison surface, and it is deliberately small: either the observed
//! function returned normally (and the routes must agree on the canonical spelling of that scalar), or it stopped
//! on a runtime failure this profile can classify identically on both routes ([`RuntimeFailureClass`]). A failure
//! neither route can classify is [`ShadowUnavailable`], not agreement: "both stopped somehow" is not proof that
//! they stopped the same way, and an arithmetic overflow is not a division by zero.
//!
//! Three things are deliberately *not* comparators, because none of them observes program meaning:
//! generated Rust text, whether that Rust compiled, and the produced artifact's shape. A legacy program that
//! fails to build makes the comparison [`ShadowComparisonState::Unavailable`] with that build failure as the
//! reason — it never becomes a divergence claim, because a comparison that could not run has proven nothing.
//!
//! ## Receipts, and what survives an unavailable comparison
//!
//! Each route that executed is selected and finalized through the canonical #986 boundary ([`select_backend`],
//! [`resolve_execution`], [`finalize_receipt`]); this module never hashes a receipt itself. Both receipts carry
//! the same `source_identity` and the same `shadow_comparison` value, and both are declared
//! [`FallbackPolicy::Refuse`] so neither route can quietly become the other. They *differ*, correctly, in
//! `selected_backend`, `executed_backend`, and `output_identity`: each route's output identity covers what that
//! route actually produced, and pretending the two receipts were interchangeable would erase the very
//! independence the comparison depends on. The legacy route's output identity additionally covers its
//! [`LegacyExecutionAuthority`] — the Oven receipt, build unit, and direct-`rustc` plan that authorized the run —
//! so "which authority produced this legacy answer" is part of the record rather than an assumption.
//!
//! An unavailable comparison does **not** discard work that really happened. If the replacement route executed
//! and only the legacy route could not run, the replacement route keeps its own receipt and Body-IR evidence in
//! [`ShadowComparison::replacement`], carrying the unavailable state. Throwing that evidence away would lose a
//! real execution and make a staging gap look identical to a source the replacement backend cannot run at all.

pub mod legacy_oven;

use std::fmt::Write as _;
use std::path::Path;

use crate::backend::IrCodegen;
use crate::backend::replacement::{
    ReplacementExecution, ReplacementExecutionError, ReplacementValue, execute_prevalidated_free_function,
    prepare_free_function_execution,
};
use crate::backend::selection::{
    BackendExecutionReceipt, BackendKind, BackendSelection, FallbackPolicy, ShadowComparisonState, digest_output,
    finalize_receipt, resolve_execution, select_backend,
};
use crate::frontend::body_ir::{apply_body_ir_input_contract, build_body_ir_module_v0};
use crate::frontend::diagnostics::DIAGNOSTIC_SCHEMA_VERSION;
use crate::frontend::typechecker::TypeChecker;
use crate::frontend::{lexer, parser};

/// Content-stable identity of the one comparison profile this module implements.
///
/// Recorded inside [`ShadowComparisonState::Matched`] and [`ShadowComparisonState::Diverged`] so a later reader
/// can tell *which* comparison a receipt is claiming, and so widening the profile forces a new identity rather
/// than silently reinterpreting old evidence.
pub const SHADOW_COMPARISON_PROFILE_ID: &str = "incan.shadow_comparison.direct_scalar_free_function.v0";

/// Reason a shadow request over a program entrypoint falls outside this profile.
///
/// The CLI's replacement build path executes the module's `main`. A `main` body's return value is not observable
/// from the produced legacy process, so there is nothing for the two routes to agree or disagree about; that is a
/// property of the profile, not a missing implementation, and it must not read as "no comparator exists".
pub const PROGRAM_ENTRYPOINT_UNAVAILABLE_REASON: &str = "the bounded source-observable comparison profile observes a named free function's returned scalar, and a \
     program entrypoint's return value is not observable from the produced legacy process; generated Rust is not \
     semantic proof";

/// Marker line printed immediately before the observed result.
const RESULT_BEGIN_MARKER: &str = "<<<incan-shadow-result-begin>>>";

/// Marker line printed immediately after the observed result.
const RESULT_END_MARKER: &str = "<<<incan-shadow-result-end>>>";

/// Domain tag folded into the legacy route's output identity.
///
/// Keeps a legacy process observation from ever colliding with a replacement execution's output identity, even
/// when both routes observed the same scalar.
const LEGACY_OUTPUT_IDENTITY_TAG: &str = "incan.shadow_comparison.legacy_process_observation.v0";

/// Domain tag folded into a failed direct execution's output identity.
const REPLACEMENT_FAILURE_IDENTITY_TAG: &str = "incan.shadow_comparison.replacement_failure.v0";

// ============================================================================
// Profile
// ============================================================================

/// One instance of the bounded comparison profile: a source module, an observed function, and its arguments.
///
/// The module must hold exactly one free function (the #988 replacement executor refuses anything else), so the
/// legacy entrypoint that calls it is generated for the legacy route's derived program only and never enters the
/// source the replacement route consumes.
///
/// Deliberately `PartialEq` but not `Eq`: `ReplacementValue` now carries generator values, which have no total
/// equality. Two profiles are compared by content identity anyway ([`ShadowComparisonProfile::profile_identity`]),
/// and this profile only ever admits scalar arguments, so nothing here needs a stronger bound than the values it
/// holds can honestly provide.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowComparisonProfile {
    source: String,
    function: String,
    arguments: Vec<ReplacementValue>,
}

impl ShadowComparisonProfile {
    /// Declare a comparison profile over one source module, observed free function, and argument list.
    ///
    /// Nothing is validated here: whether the source, function name, and arguments are inside the profile is
    /// decided when a route is actually observed, so an out-of-profile request produces a recorded
    /// [`ShadowComparisonState::Unavailable`] with a concrete reason instead of a constructor error a caller
    /// could discard.
    #[must_use]
    pub fn new(source: impl Into<String>, function: impl Into<String>, arguments: Vec<ReplacementValue>) -> Self {
        Self {
            source: source.into(),
            function: function.into(),
            arguments,
        }
    }

    /// The module source both routes observe.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The observed free function's source-level name.
    #[must_use]
    pub fn function(&self) -> &str {
        &self.function
    }

    /// Stable kind of comparison this profile performs.
    ///
    /// Recorded alongside the instance identity in every receipt so a consumer keyed on comparison capability —
    /// #1153's compatibility-feature registry, for one — can link to "this kind of comparison" without decoding
    /// a content hash, while evidence still names the exact instance.
    #[must_use]
    pub fn profile_kind(&self) -> &'static str {
        SHADOW_COMPARISON_PROFILE_ID
    }

    /// Content identity of the observed source, used as both routes' selection `source_identity`.
    ///
    /// Deliberately covers the module source alone, so the two routes are provably selected for the same source
    /// even though the legacy route additionally generates an entrypoint to call into it.
    #[must_use]
    pub fn source_identity(&self) -> String {
        digest_output(&[self.source.as_str()])
    }

    /// Content identity of this profile instance: the profile kind, the source, the observed function, and the
    /// exact arguments.
    ///
    /// Two comparisons over the same module but different arguments are different claims, so they must not share
    /// an identity, and [`classify_observations`] refuses to pair observations whose identities disagree. An
    /// argument that has no source spelling makes the profile unrepresentable, and the identity falls back to a
    /// stable marker rather than silently colliding with a representable argument list.
    #[must_use]
    pub fn profile_identity(&self) -> String {
        let arguments = match self.argument_literals() {
            Ok(literals) => literals.join(", "),
            Err(_) => "<unrepresentable arguments>".to_string(),
        };
        digest_output(&[
            SHADOW_COMPARISON_PROFILE_ID,
            self.source.as_str(),
            self.function.as_str(),
            arguments.as_str(),
        ])
    }

    /// Build the Incan program the legacy route runs: this profile's module plus a generated entrypoint that
    /// calls the observed function with the profile's arguments and prints the result between frame markers.
    ///
    /// The generated entrypoint is Incan source, not hand-written Rust, so the legacy route still observes the
    /// observed function through the compiler's own pipeline rather than through a Rust-level harness. The
    /// markers exist so [`decode_framed_result`] can recover the printed value byte-for-byte; see the module
    /// docs on lossless transport.
    pub fn legacy_program_source(&self) -> Result<String, ShadowUnavailable> {
        if self.function == "main" {
            return Err(ShadowUnavailable::new(PROGRAM_ENTRYPOINT_UNAVAILABLE_REASON));
        }
        let program_after_input_contract = self.program_after_input_contract()?;
        let function_is_active = program_after_input_contract.declarations.iter().any(|declaration| {
            matches!(&declaration.node, crate::frontend::ast::Declaration::Function(function) if function.name == self.function)
        });
        if !function_is_active {
            return Err(ShadowUnavailable::new(format!(
                "the comparison profile's function `{}` is absent from the manifest-free feature projection, so neither route may observe it",
                self.function
            )));
        }
        let arguments = self.argument_literals()?.join(", ");
        let mut program = self.source.clone();
        if !program.ends_with('\n') {
            program.push('\n');
        }
        let _ = write!(
            program,
            "\ndef main() -> None:\n    println(\"{RESULT_BEGIN_MARKER}\")\n    println({}({arguments}))\n    \
             println(\"{RESULT_END_MARKER}\")\n",
            self.function
        );
        Ok(program)
    }

    /// Parse and prepare the profile source through the same manifest-free Body-IR input contract as both routes.
    ///
    /// The profile intentionally owns no package manifest or feature selection, so its only valid feature
    /// projection is empty. Preparing before either route observes the selected function prevents a declaration
    /// behind `when feature(...)` from becoming comparison evidence for a compilation that does not contain it.
    fn program_after_input_contract(&self) -> Result<crate::frontend::ast::Program, ShadowUnavailable> {
        let tokens = lexer::lex(self.source())
            .map_err(|errors| ShadowUnavailable::new(format!("comparison source did not lex: {errors:?}")))?;
        let program = parser::parse(&tokens)
            .map_err(|errors| ShadowUnavailable::new(format!("comparison source did not parse: {errors:?}")))?;
        apply_body_ir_input_contract(program, Path::new("incan_shadow_comparison.incn")).map_err(|errors| {
            ShadowUnavailable::new(format!(
                "comparison source did not satisfy the Body IR input contract: {errors:?}"
            ))
        })
    }

    /// Render every argument as the Incan source literal the legacy entrypoint passes.
    fn argument_literals(&self) -> Result<Vec<String>, ShadowUnavailable> {
        self.arguments.iter().map(argument_literal).collect()
    }
}

/// Render one scalar argument as an Incan source literal.
///
/// Only the scalars the profile can spell in source are accepted. A collection, tuple, range, float, or unit
/// argument has no literal form this profile is willing to synthesize, so it becomes an explicit unavailable
/// reason rather than a guessed spelling that would silently change what was executed.
fn argument_literal(value: &ReplacementValue) -> Result<String, ShadowUnavailable> {
    match value {
        ReplacementValue::Int(value) => Ok(value.to_string()),
        ReplacementValue::Bool(value) => Ok(value.to_string()),
        ReplacementValue::Str(value) => incan_string_literal(value),
        other => Err(ShadowUnavailable::new(format!(
            "the bounded source-observable comparison profile can only pass `int`, `bool`, and `str` arguments as \
             source literals; got {other:?}"
        ))),
    }
}

/// Escape one string argument into an Incan double-quoted literal.
///
/// Only the escapes Incan itself spells are emitted. Any other control character is refused instead of being
/// approximated, because a legacy entrypoint that passes a *different* string than the replacement route received
/// would produce a comparison over two different inputs.
fn incan_string_literal(value: &str) -> Result<String, ShadowUnavailable> {
    let mut literal = String::with_capacity(value.len() + 2);
    literal.push('"');
    for character in value.chars() {
        match character {
            '"' => literal.push_str("\\\""),
            '\\' => literal.push_str("\\\\"),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\t' => literal.push_str("\\t"),
            other if other.is_control() => {
                return Err(ShadowUnavailable::new(format!(
                    "the bounded source-observable comparison profile refuses a `str` argument containing the \
                     control character U+{:04X}, which it cannot spell as an Incan literal",
                    u32::from(other)
                )));
            }
            other => literal.push(other),
        }
    }
    literal.push('"');
    Ok(literal)
}

/// Recover the exact printed result from a legacy program's standard output.
///
/// The frame is positional, not searched: the output must begin with the begin-marker line and end with the
/// end-marker line, and everything between them is the payload verbatim — including any leading or trailing
/// newlines that belong to the value itself. This is what makes `"x"` and `"x\n"` distinguishable, which trimming
/// would destroy. Output that does not carry the frame exactly, or whose payload is not valid UTF-8, is
/// unavailable rather than approximated.
pub fn decode_framed_result(stdout: &[u8]) -> Result<String, ShadowUnavailable> {
    let prefix = format!("{RESULT_BEGIN_MARKER}\n").into_bytes();
    let suffix = format!("\n{RESULT_END_MARKER}\n").into_bytes();
    let framed =
        stdout.len() >= prefix.len() + suffix.len() && stdout.starts_with(&prefix) && stdout.ends_with(&suffix);
    if !framed {
        return Err(ShadowUnavailable::new(format!(
            "the legacy program's output did not carry the exact result frame, so its result could not be read \
             back losslessly: {:?}",
            String::from_utf8_lossy(stdout)
        )));
    }
    let payload = &stdout[prefix.len()..stdout.len() - suffix.len()];
    String::from_utf8(payload.to_vec()).map_err(|error| {
        ShadowUnavailable::new(format!(
            "the legacy program printed a result that is not valid UTF-8, so it could not be compared: {error}"
        ))
    })
}

// ============================================================================
// The compared observable
// ============================================================================

/// A runtime failure this profile can recognize identically on both routes.
///
/// Narrow on purpose: a failure class exists here only when the legacy process and the direct Body-IR executor
/// both report it unambiguously. Everything else is [`ShadowUnavailable`], because agreeing that "something went
/// wrong" is not evidence that the same thing went wrong. The classes stay distinct for the same reason — an
/// arithmetic overflow and a division by zero are different program behaviors, and collapsing them would let a
/// real divergence report as agreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeFailureClass {
    /// A source-level `assert` failed.
    Assertion,
    /// An integer division or modulo by zero was attempted.
    DivisionByZero,
    /// An integer operation produced a result the type cannot represent.
    ArithmeticOverflow,
}

impl RuntimeFailureClass {
    /// Stable label used in comparison detail text and receipts.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Assertion => "assertion-failed",
            Self::DivisionByZero => "division-by-zero",
            Self::ArithmeticOverflow => "arithmetic-overflow",
        }
    }
}

/// The entire source-level observable this profile compares.
///
/// Nothing about timing, output shape, artifact layout, or generated code is part of it. Two routes agree exactly
/// when their [`SourceObservable`] values are equal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SourceObservable {
    /// The observed function returned normally, spelled canonically and transported byte-for-byte.
    ///
    /// For the replacement route this is `ReplacementValue::observable_text`; for the legacy route it is exactly
    /// what the produced process printed for the same value, recovered by [`decode_framed_result`].
    Completed { result: String },
    /// The observed function stopped on a classified runtime failure.
    Failed { failure: RuntimeFailureClass },
}

impl SourceObservable {
    /// Render the observable as one comparable line for receipts and divergence detail.
    ///
    /// The completed payload is debug-quoted so whitespace differences stay visible in a divergence report; a
    /// bare rendering would make `"x"` and `"x\n"` look identical in the very message meant to explain them.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Completed { result } => format!("completed({result:?})"),
            Self::Failed { failure } => format!("failed({})", failure.label()),
        }
    }
}

/// One route's observation plus the raw evidence behind it.
///
/// `detail` is never compared; it exists so a divergence or an unavailable result can name what actually
/// happened without widening the comparison surface itself. `profile_identity` binds the observation to the exact
/// profile instance that produced it, so [`classify_observations`] can refuse to compare observations of two
/// different profiles instead of reporting a meaningless verdict.
#[derive(Debug, Clone)]
pub struct RouteObservation {
    /// Stable kind of comparison profile this observation was produced under.
    pub profile_kind: String,
    /// Content identity of the exact profile instance this observation was produced under.
    pub profile_identity: String,
    /// The compared observable.
    pub observable: SourceObservable,
    /// Route-specific evidence retained for reporting only.
    pub detail: String,
    /// Content identity of what this route produced, bound into its receipt.
    pub output_identity: String,
}

/// A comparison that could not run, with the concrete boundary that stopped it.
///
/// Folded into [`ShadowComparisonState::Unavailable`], which is always non-green.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{reason}")]
pub struct ShadowUnavailable {
    /// Concrete, actionable explanation of why no comparison ran.
    pub reason: String,
}

impl ShadowUnavailable {
    /// Record one concrete reason a comparison could not run.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self { reason: reason.into() }
    }
}

// ============================================================================
// Route evidence and the comparison result
// ============================================================================

/// The Oven authority that permitted one legacy-route execution.
///
/// Bound into the legacy receipt's output identity so a reader can tell which project receipt, reusable build
/// unit, and immutable direct-`rustc` plan produced the observed answer. Without it a legacy observation would be
/// an unattributed process result rather than an Oven-owned execution.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LegacyExecutionAuthority {
    /// Identity of the verified Oven receipt that authorized the emitted source.
    pub oven_receipt_identity: String,
    /// Identity of the reusable Oven build unit the plan was selected against.
    pub oven_build_unit_identity: String,
    /// Store identity of the immutable direct-`rustc` plan that compiled the program.
    pub direct_rustc_plan_identity: String,
    /// Digest of the caller-owned native output Oven produced.
    pub output_digest: String,
    /// Whether any Cargo process participated; Oven-owned execution requires `false`.
    pub cargo_process_started: bool,
}

/// Everything one route contributed to a comparison.
///
/// Present whenever the route actually executed. `receipt` is optional only because receipt finalization can
/// itself fail; when it does, the observation is still reported and the comparison state names the failure, so an
/// execution that really happened is never silently discarded.
#[derive(Debug, Clone)]
pub struct RouteEvidence {
    /// The #986 receipt binding this route's execution to its declared selection, when one could be finalized.
    pub receipt: Option<BackendExecutionReceipt>,
    /// The observation the comparison was made over.
    pub observation: RouteObservation,
}

impl RouteEvidence {
    /// The finalized receipt, or the reason this executed route has none.
    ///
    /// Callers that need a receipt should use this rather than unwrapping the option, so the missing-receipt case
    /// reads as the explicit failure it is.
    pub fn receipt(&self) -> Result<&BackendExecutionReceipt, String> {
        self.receipt.as_ref().ok_or_else(|| {
            format!(
                "the {} route executed but its receipt could not be finalized",
                self.observation.profile_kind
            )
        })
    }
}

/// The result of one bounded shadow comparison.
///
/// A route is present exactly when it actually executed. That is deliberate: an unavailable comparison whose
/// replacement route ran still carries that route's receipt and Body-IR evidence, so a staging gap is
/// distinguishable from a source the replacement backend cannot execute at all.
#[derive(Debug, Clone)]
pub struct ShadowComparison {
    /// Stable kind of comparison profile this comparison was declared for.
    pub profile_kind: String,
    /// Content identity of the exact profile instance this comparison was declared for.
    pub profile_identity: String,
    /// Source identity both routes were selected for.
    pub source_identity: String,
    /// The recorded outcome, also carried inside every produced receipt.
    pub state: ShadowComparisonState,
    /// Legacy-route evidence, present only when the legacy program actually ran.
    pub legacy: Option<RouteEvidence>,
    /// Replacement-route evidence, present only when direct execution actually ran.
    pub replacement: Option<RouteEvidence>,
    /// The direct Body-IR execution behind [`ShadowComparison::replacement`], when it completed normally.
    pub replacement_execution: Option<ReplacementExecution>,
    /// The Oven authority behind [`ShadowComparison::legacy`], when the legacy route ran.
    pub legacy_authority: Option<LegacyExecutionAuthority>,
}

impl ShadowComparison {
    /// Whether both routes ran independently and agreed on the compared observable.
    ///
    /// This is the only state that may promote anything to green; `Unavailable` and `Diverged` must not.
    #[must_use]
    pub fn matched(&self) -> bool {
        matches!(self.state, ShadowComparisonState::Matched { .. })
    }

    /// The recorded reason when no comparison was made, if this comparison is unavailable.
    #[must_use]
    pub fn unavailable_reason(&self) -> Option<&str> {
        match &self.state {
            ShadowComparisonState::Unavailable { reason } => Some(reason.as_str()),
            _ => None,
        }
    }
}

/// Run one bounded shadow comparison and bind every route that executed to its own #986 receipt.
///
/// Both routes run independently: neither reads the other's artifacts, and neither is derived from the other's
/// result. `capability` names the Oven store, receipt intent, and compiler that authorize the legacy route;
/// `workspace` is a caller-owned directory the legacy route may write its emitted Rust and produced program into.
///
/// This is total by design. Every failure — an out-of-profile source, an unstaged Oven capability, a legacy build
/// failure, an unclassifiable runtime failure, even a backend-selection error — becomes a recorded
/// [`ShadowComparisonState::Unavailable`] with a concrete reason, because a comparison that silently disappears
/// is indistinguishable from one that passed. Evidence from whichever route *did* run is retained alongside it.
#[must_use]
pub fn compare_source_observable(
    profile: &ShadowComparisonProfile,
    capability: &legacy_oven::LegacyOvenCapability,
    workspace: &Path,
) -> ShadowComparison {
    // ---- Route 1: direct Body IR, no generation and no subprocess ----
    let replacement = observe_replacement_route(profile);

    // ---- Route 2: emitted Rust, Oven-authorized native build, executed as a process ----
    let legacy = legacy_oven::observe_legacy_route(profile, capability, workspace);

    assemble_comparison(profile, replacement, legacy)
}

/// Fold both routes' results into one comparison, retaining every route that actually executed.
///
/// Separated from [`compare_source_observable`] so the assembly rules — classification, receipt binding, and
/// partial-evidence retention — stay testable without staging an Oven capability.
fn assemble_comparison(
    profile: &ShadowComparisonProfile,
    replacement: Result<ReplacementRouteResult, ShadowUnavailable>,
    legacy: Result<LegacyRouteResult, ShadowUnavailable>,
) -> ShadowComparison {
    let profile_kind = profile.profile_kind().to_string();
    let profile_identity = profile.profile_identity();
    let source_identity = profile.source_identity();

    let (replacement_observation, replacement_execution, replacement_unavailable) = match replacement {
        Ok(observed) => (Some(observed.observation), observed.execution, None),
        Err(unavailable) => (None, None, Some(unavailable.reason)),
    };
    let (legacy_observation, legacy_authority, legacy_unavailable) = match legacy {
        Ok(observed) => (Some(observed.observation), Some(observed.authority), None),
        Err(unavailable) => (None, None, Some(unavailable.reason)),
    };

    let state = match (&legacy_observation, &replacement_observation) {
        (Some(legacy), Some(replacement)) => classify_observations(legacy, replacement),
        _ => ShadowComparisonState::Unavailable {
            reason: unavailable_reason(legacy_unavailable, replacement_unavailable),
        },
    };

    // Receipts embed the comparison state, so the state must be settled before any receipt is finalized. A
    // receipt that cannot be finalized makes the comparison unverifiable and withdraws the verdict, so
    // finalization is probed first and only then performed against the settled state. Probing is pure hashing;
    // doing it twice is cheaper than publishing a receipt that claims a verdict the comparison no longer holds.
    let mut receipt_failures = Vec::new();
    for (backend, label, observation) in [
        (BackendKind::Legacy, "legacy", legacy_observation.as_ref()),
        (
            BackendKind::Replacement,
            "replacement",
            replacement_observation.as_ref(),
        ),
    ] {
        let Some(observation) = observation else {
            continue;
        };
        if let Err(error) = route_receipt(&source_identity, backend, observation, &state) {
            receipt_failures.push(format!(
                "the {label} route executed and observed {} but its receipt could not be finalized: {error}",
                observation.observable.describe()
            ));
        }
    }

    let state = if receipt_failures.is_empty() {
        state
    } else {
        ShadowComparisonState::Unavailable {
            reason: format!(
                "both routes' observations were produced, but a receipt could not be finalized, so the comparison \
                 is not verifiable: {}",
                receipt_failures.join("; ")
            ),
        }
    };

    // Every route that executed keeps its evidence, including under an unavailable state: discarding a real
    // execution would make a staging gap indistinguishable from a backend that could not run the source at all.
    let legacy_evidence = route_evidence(&source_identity, BackendKind::Legacy, legacy_observation, &state);
    let replacement_evidence = route_evidence(
        &source_identity,
        BackendKind::Replacement,
        replacement_observation,
        &state,
    );

    ShadowComparison {
        profile_kind,
        profile_identity,
        source_identity,
        state,
        legacy: legacy_evidence,
        replacement: replacement_evidence,
        replacement_execution,
        legacy_authority,
    }
}

/// Bind one executed route's observation to its own receipt, keeping the observation either way.
///
/// A route that did not run contributes nothing. A route that ran always contributes its observation, even when
/// its receipt cannot be finalized — the caller has already withdrawn the verdict and recorded why, and dropping
/// the evidence here would hide that the route executed at all.
fn route_evidence(
    source_identity: &str,
    backend: BackendKind,
    observation: Option<RouteObservation>,
    state: &ShadowComparisonState,
) -> Option<RouteEvidence> {
    let observation = observation?;
    let receipt = route_receipt(source_identity, backend, &observation, state).ok();
    Some(RouteEvidence { receipt, observation })
}

/// Compose the recorded reason for a comparison that could not run, naming every route that failed to execute.
fn unavailable_reason(legacy: Option<String>, replacement: Option<String>) -> String {
    match (legacy, replacement) {
        (Some(legacy), Some(replacement)) => {
            format!("neither route executed; legacy route: {legacy}; replacement route: {replacement}")
        }
        (Some(legacy), None) => format!("the legacy route did not execute: {legacy}"),
        (None, Some(replacement)) => format!("the replacement route did not execute: {replacement}"),
        (None, None) => "no comparison was made and neither route reported a reason".to_string(),
    }
}

/// Decide whether two independent observations of the same profile agree.
///
/// Equality of [`SourceObservable`] is the whole rule, but only after both observations are confirmed to describe
/// the *same* profile instance. Comparing observations produced under different profiles would manufacture a
/// verdict about a comparison nobody ran, so that is refused as unavailable rather than reported as divergence.
#[must_use]
pub fn classify_observations(legacy: &RouteObservation, replacement: &RouteObservation) -> ShadowComparisonState {
    if legacy.profile_kind != replacement.profile_kind || legacy.profile_identity != replacement.profile_identity {
        return ShadowComparisonState::Unavailable {
            reason: format!(
                "the two observations describe different comparison profiles ({}/{} and {}/{}), so no comparison \
                 over one source was made",
                legacy.profile_kind, legacy.profile_identity, replacement.profile_kind, replacement.profile_identity
            ),
        };
    }
    if legacy.observable == replacement.observable {
        return ShadowComparisonState::Matched {
            profile_kind: legacy.profile_kind.clone(),
            profile_identity: legacy.profile_identity.clone(),
            observable: legacy.observable.describe(),
        };
    }
    ShadowComparisonState::Diverged {
        profile_kind: legacy.profile_kind.clone(),
        profile_identity: legacy.profile_identity.clone(),
        detail: format!(
            "legacy route observed {} and replacement route observed {}; legacy detail: {}; replacement detail: {}",
            legacy.observable.describe(),
            replacement.observable.describe(),
            legacy.detail,
            replacement.detail
        ),
    }
}

/// A completed direct execution plus the observation derived from it.
pub(crate) struct ReplacementRouteResult {
    pub(crate) observation: RouteObservation,
    pub(crate) execution: Option<ReplacementExecution>,
}

/// A completed Oven-owned legacy execution plus the authority that permitted it.
pub(crate) struct LegacyRouteResult {
    pub(crate) observation: RouteObservation,
    pub(crate) authority: LegacyExecutionAuthority,
}

/// Observe the replacement route: typecheck, lower to Body IR, and execute the observed function directly.
///
/// Refusals from the bounded #988 profile (unsupported construct, wrong arity, missing function) mean this
/// profile instance is outside the comparison, not that the routes disagreed, so they surface as
/// [`ShadowUnavailable`].
fn observe_replacement_route(profile: &ShadowComparisonProfile) -> Result<ReplacementRouteResult, ShadowUnavailable> {
    let body_ir = {
        let program = profile.program_after_input_contract()?;
        let module_path = vec!["incan_shadow_comparison".to_string()];
        let mut checker = TypeChecker::new();
        checker.set_current_module_path(Some(module_path.clone()));
        checker
            .check_program(&program)
            .map_err(|errors| ShadowUnavailable::new(format!("comparison source did not typecheck: {errors:?}")))?;
        build_body_ir_module_v0(&program, &module_path, checker.type_info())
    };

    let profile_kind = profile.profile_kind().to_string();
    let profile_identity = profile.profile_identity();
    let plan = prepare_free_function_execution(&body_ir, profile.function(), &profile.arguments)
        .map_err(replacement_profile_unavailable)?;
    match execute_prevalidated_free_function(plan) {
        Ok(execution) if !execution.emitted_output().is_empty() => {
            // A program that printed cannot be compared by this profile, and saying so is the point. The compared
            // observable is the function's *returned value*; the legacy route recovers it by requiring the process's
            // stdout to begin and end with the result frame, which the program's own output breaks. Reporting a
            // match on the return value alone would claim agreement over a run whose printed output nothing looked
            // at — the exact silent divergence this comparison exists to prevent.
            Err(ShadowUnavailable::new(format!(
                "the replacement route emitted {} line(s) of program output, which this profile cannot compare: \
                 the legacy route recovers its result from a stdout frame that the program's own output breaks",
                execution.emitted_output().len()
            )))
        }
        Ok(execution) => {
            let observation = RouteObservation {
                profile_kind,
                profile_identity,
                observable: SourceObservable::Completed {
                    result: execution.value.observable_text(),
                },
                detail: format!("direct Body-IR execution of `{}`", profile.function()),
                output_identity: execution.output_identity.clone(),
            };
            Ok(ReplacementRouteResult {
                observation,
                execution: Some(execution),
            })
        }
        Err(ReplacementExecutionError::RuntimeFailure { detail, .. }) => {
            let failure = classify_replacement_failure(&detail)?;
            Ok(ReplacementRouteResult {
                observation: RouteObservation {
                    profile_kind,
                    profile_identity,
                    observable: SourceObservable::Failed { failure },
                    detail: format!("direct Body-IR execution failed: {detail}"),
                    output_identity: digest_output(&[REPLACEMENT_FAILURE_IDENTITY_TAG, &detail]),
                },
                execution: None,
            })
        }
        Err(error) => Err(replacement_profile_unavailable(error)),
    }
}

/// Fold a replacement-profile refusal into an unavailable reason that keeps the original source span.
fn replacement_profile_unavailable(error: ReplacementExecutionError) -> ShadowUnavailable {
    ShadowUnavailable::new(format!(
        "the replacement route cannot execute this profile instance: {error}"
    ))
}

/// Emit Rust for the legacy route's derived program through the real legacy backend.
pub(crate) fn emit_legacy_rust(program: &str) -> Result<String, ShadowUnavailable> {
    let tokens = lexer::lex(program)
        .map_err(|errors| ShadowUnavailable::new(format!("the legacy program did not lex: {errors:?}")))?;
    let ast = parser::parse(&tokens)
        .map_err(|errors| ShadowUnavailable::new(format!("the legacy program did not parse: {errors:?}")))?;
    crate::frontend::typechecker::check(&ast)
        .map_err(|errors| ShadowUnavailable::new(format!("the legacy program did not typecheck: {errors:?}")))?;
    IrCodegen::new()
        .try_generate(&ast)
        .map_err(|error| ShadowUnavailable::new(format!("the legacy backend could not emit Rust: {error}")))
}

/// Turn one completed legacy process result into a comparable observation.
///
/// A successful exit must carry the exact result frame; a failing exit must carry a classifiable failure. Neither
/// the exit status alone nor the presence of output is treated as an answer.
pub(crate) fn observe_legacy_process(
    profile_kind: &str,
    profile_identity: &str,
    authority: &LegacyExecutionAuthority,
    exit_code: Option<i32>,
    stdout: &[u8],
    stderr: &str,
) -> Result<RouteObservation, ShadowUnavailable> {
    let observable = if exit_code == Some(0) {
        SourceObservable::Completed {
            result: decode_framed_result(stdout)?,
        }
    } else {
        SourceObservable::Failed {
            failure: classify_legacy_failure(stderr)?,
        }
    };
    let detail = format!(
        "Oven-owned legacy process exited with {exit_code:?} under direct-rustc plan {}; stderr: {stderr:?}",
        authority.direct_rustc_plan_identity
    );
    let output_identity = digest_output(&[
        LEGACY_OUTPUT_IDENTITY_TAG,
        authority.oven_receipt_identity.as_str(),
        authority.oven_build_unit_identity.as_str(),
        authority.direct_rustc_plan_identity.as_str(),
        authority.output_digest.as_str(),
        observable.describe().as_str(),
    ]);
    Ok(RouteObservation {
        profile_kind: profile_kind.to_string(),
        profile_identity: profile_identity.to_string(),
        observable,
        detail,
        output_identity,
    })
}

/// Classify a legacy process failure into a class the replacement route can also report.
///
/// Generated programs install a panic hook that prints the panic payload, so the panic message is the
/// source-level evidence here. Overflow is tested before division, because Rust's overflow panic for a division
/// also names division and would otherwise be misfiled as a division by zero. An unrecognized failure is
/// unavailable, not a shared failure observation.
fn classify_legacy_failure(stderr: &str) -> Result<RuntimeFailureClass, ShadowUnavailable> {
    let lowered = stderr.to_lowercase();
    if lowered.contains("assertionerror") || lowered.contains("assertion failed") {
        return Ok(RuntimeFailureClass::Assertion);
    }
    if lowered.contains("overflow") {
        return Ok(RuntimeFailureClass::ArithmeticOverflow);
    }
    if lowered.contains("zerodivisionerror")
        || lowered.contains("divide by zero")
        || lowered.contains("division by zero")
        || lowered.contains("remainder with a divisor of zero")
    {
        return Ok(RuntimeFailureClass::DivisionByZero);
    }
    Err(ShadowUnavailable::new(format!(
        "the legacy program failed in a way this profile cannot classify, so agreement cannot be claimed: {stderr}"
    )))
}

/// Classify a direct-execution runtime failure into the same class vocabulary as the legacy route.
///
/// Overflow is tested first for the same reason as on the legacy route: the executor spells an unrepresentable
/// quotient as an "integer division overflow", which is an overflow, not a division by zero.
fn classify_replacement_failure(detail: &str) -> Result<RuntimeFailureClass, ShadowUnavailable> {
    let lowered = detail.to_lowercase();
    if lowered.contains("assertion") {
        return Ok(RuntimeFailureClass::Assertion);
    }
    if lowered.contains("overflow") {
        return Ok(RuntimeFailureClass::ArithmeticOverflow);
    }
    if lowered.contains("division or modulo by zero") {
        return Ok(RuntimeFailureClass::DivisionByZero);
    }
    Err(ShadowUnavailable::new(format!(
        "the replacement route failed in a way this profile cannot classify, so agreement cannot be claimed: \
         {detail}"
    )))
}

// ============================================================================
// Receipt binding
// ============================================================================

/// Declare, resolve, and finalize one route's #986 receipt for a comparison.
///
/// Every route is declared explicitly with [`FallbackPolicy::Refuse`], so a receipt from this module can never
/// record one backend standing in for the other — which is exactly the substitution a shadow comparison exists to
/// rule out.
fn route_receipt(
    source_identity: &str,
    backend: BackendKind,
    observation: &RouteObservation,
    state: &ShadowComparisonState,
) -> Result<BackendExecutionReceipt, crate::backend::selection::BackendSelectionError> {
    let selection: BackendSelection = select_backend(backend, true, true, source_identity, FallbackPolicy::Refuse);
    let executed = resolve_execution(&selection, backend.is_implemented())?;
    finalize_receipt(
        &selection,
        executed,
        observation.output_identity.clone(),
        state.clone(),
        DIAGNOSTIC_SCHEMA_VERSION,
    )
}

#[cfg(test)]
mod tests;
