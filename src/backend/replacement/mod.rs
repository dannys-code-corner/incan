//! Direct execution of the deliberately narrow #988 Body-IR replacement profile.
//!
//! This module consumes [`BodyIrModule`] directly. It never reads generated Rust, never
//! delegates a requested replacement execution to [`crate::backend::ir`], and rejects every operation outside the
//! first free-function profile with the original Body-IR source span. The profile is intentionally limited to
//! scalar local values, scalar tuple collections, arithmetic, compiler-owned string concatenation, branches,
//! normalized loops, returns, and assertions. It admits only the one-level `.0`/`.1` tuple projections Body IR
//! emits for `for a, b in pairs`, where `pairs` is `list[tuple[scalar, scalar]]`; packages, Rust interop, callable
//! values, generator functions, generator adapters, general destructuring, and other projections remain visible
//! refusals for later #988 extensions. A generator expression with the selected `range` source profile may be
//! materialized by its `.collect()` consumer: construction records its source/captures without running the deferred
//! body, and collection executes that body in this interpreter rather than falling back to generated Rust.

use std::collections::{BTreeMap, BTreeSet};

use incan_core::{
    lang::surface::constructors::{ConstructorId, as_str as constructor_name},
    lang::types::collections::{self, CollectionTypeId},
    python_floor_div_i64, python_mod_i64,
};
use incan_semantics_core::body_ir::{
    AggregateKind, BinOp, Body, BodyIrModule, Callee, Constant, GeneratorBody, HelperOp, IterProtocol, LocalId,
    LocalOrigin, Operand, OwnershipFact, Place, PlaceElem, Rvalue, Statement, StatementKind, UnOp,
};
use incan_semantics_core::{AbiV0RuntimeRequirement, HirSourceSpan, IncanPrimitiveType, IncanType};

use crate::backend::selection::digest_output;

/// Bounded instruction count for one replacement execution.
///
/// The first profile deliberately executes normalized loops rather than translating them to native code. Keeping a
/// deterministic bound turns an accidental infinite loop into an explicit unavailable result instead of allowing a
/// test or CLI invocation to hang without a receipt.
const MAX_EXECUTION_STEPS: usize = 100_000;

/// One runtime value supported by the bounded replacement-execution profile.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplacementValue {
    /// An Incan `int` value.
    Int(i64),
    /// An Incan `bool` value.
    Bool(bool),
    /// An owned Incan `str` value.
    Str(String),
    /// An Incan floating-point literal, retained so unsupported float operations refuse honestly.
    Float(String),
    /// An Incan `None`/unit value.
    Unit,
    /// A normalized runtime range iterator for the selected `range` source-spelling control-flow case.
    Range { next: i64, end: i64, step: i64 },
    /// A `list[tuple[scalar, scalar]]` value with the cursor owned by its materialized builtin iterator local.
    List {
        elements: Vec<ReplacementValue>,
        next: usize,
    },
    /// A two-element scalar tuple, retained only as one element of an admitted replacement list value.
    Tuple(Vec<ReplacementValue>),
    /// A generator expression whose source and lexical captures were evaluated at construction time, while its body
    /// remains deferred until an admitted consumer asks for its values.
    Generator(Box<ReplacementGenerator>),
    /// Values materialized by the bounded generator-expression `.collect()` profile.
    ///
    /// This is deliberately distinct from [`Self::List`]: the latter remains the existing scalar-pair collection
    /// profile, while this variant makes the narrow generator consumer explicit and lets its scalar results be
    /// indexed without admitting general list execution.
    CollectedGenerator {
        elements: Vec<ReplacementValue>,
        next: usize,
    },
}

/// Deferred state for a bounded replacement generator expression.
///
/// `source` and `captures` hold the values observed when the surrounding body constructed the generator. `body`
/// contains only compiler-owned Body-IR statements and is interpreted when `.collect()` consumes this value. The
/// fields are private so callers cannot construct a replacement generator without the executor's ownership reads.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplacementGenerator {
    source: ReplacementValue,
    captures: Vec<(LocalId, ReplacementValue)>,
    body: GeneratorBody,
}

impl ReplacementValue {
    /// Render a deterministic source-observable result spelling for replacement receipts and CLI output.
    pub fn observable_text(&self) -> String {
        match self {
            Self::Int(value) => value.to_string(),
            Self::Bool(value) => value.to_string(),
            Self::Str(value) => value.clone(),
            Self::Float(value) => value.clone(),
            Self::Unit => constructor_name(ConstructorId::None).to_string(),
            Self::Range { next, end, step } => format!("range({next}, {end}, {step})"),
            Self::List { elements, .. } => format!(
                "[{}]",
                elements
                    .iter()
                    .map(Self::observable_text)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Tuple(elements) => format!(
                "({})",
                elements
                    .iter()
                    .map(Self::observable_text)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Generator(_) => "<generator>".to_string(),
            Self::CollectedGenerator { elements, .. } => format!(
                "[{}]",
                elements
                    .iter()
                    .map(Self::observable_text)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// One Body-IR place read observed while executing the replacement profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipRead {
    /// The original Incan source span of the statement containing the read.
    pub span: HirSourceSpan,
    /// The compiler-owned ownership fact the executor honored.
    pub fact: OwnershipFact,
    /// Whether lowering identified this read as the local's last use.
    pub last_use: bool,
}

/// Canonical, machine-readable rendering of one ownership read used in a replacement receipt.
///
/// This projection deliberately uses stable source offsets and fact labels rather than Rust `Debug` output, so
/// receipt identities and CLI reports remain stable when implementation-only derives or field formatting change.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OwnershipReadProjection {
    /// Original Incan source span start byte offset.
    pub span_start: usize,
    /// Original Incan source span end byte offset.
    pub span_end: usize,
    /// Stable compiler-owned ownership fact label.
    pub fact: &'static str,
    /// Whether lowering marked this source read as its local's last use.
    pub last_use: bool,
}

/// Canonical, machine-readable rendering of one Body-IR runtime requirement used in a replacement receipt.
///
/// `requirement` is a stable semantic label, not the Rust `Debug` representation of the internal enum.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RuntimeRequirementProjection {
    /// Stable semantic label for the runtime requirement.
    pub requirement: String,
}

/// Successful replacement execution evidence for one free function.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplacementExecution {
    /// The function's source-level return value.
    pub value: ReplacementValue,
    /// Deterministic Body-IR snapshot retained as proof of the consumed input.
    pub body_snapshot: String,
    /// Every ownership decision observed during execution, in execution order.
    pub ownership_reads: Vec<OwnershipRead>,
    /// Runtime/helper requirements carried through from the consumed Body IR.
    pub runtime_requirements: Vec<AbiV0RuntimeRequirement>,
    /// Content identity of the actual Body-IR snapshot, ownership facts, requirements, and observed result.
    pub output_identity: String,
}

/// A free-function execution that has passed the bounded #988 profile validator.
///
/// The capability retains the exact typed Body IR, source-level function name, and concrete arguments that were
/// validated. It lets selection/receipt code decide whether direct execution may proceed without rerunning profile
/// validation or allowing an unvalidated Body IR body to reach the executor.
pub struct ValidatedFreeFunctionExecution<'module, 'args> {
    module: &'module BodyIrModule,
    name: String,
    args: &'args [ReplacementValue],
}

impl ReplacementExecution {
    /// Return the stable ownership evidence bound into this execution's output identity and CLI report.
    #[must_use]
    pub fn ownership_evidence(&self) -> Vec<OwnershipReadProjection> {
        ownership_read_projection(&self.ownership_reads)
    }

    /// Return the stable runtime-requirement evidence bound into this execution's output identity and CLI report.
    #[must_use]
    pub fn runtime_requirement_evidence(&self) -> Vec<RuntimeRequirementProjection> {
        runtime_requirement_projection(&self.runtime_requirements)
    }
}

/// A visible refusal or runtime outcome from the replacement executor.
#[derive(Debug, thiserror::Error)]
pub enum ReplacementExecutionError {
    /// The requested free function was absent from the typed Body-IR module.
    #[error("replacement backend has no free function named `{name}` to execute")]
    MissingFunction {
        /// The requested source-level function name.
        name: String,
    },
    /// The caller supplied a different number of arguments than the Body IR declares.
    #[error("replacement backend cannot execute `{name}`: expected {expected} arguments, got {actual}")]
    ArgumentCount {
        /// Function selected for execution.
        name: String,
        /// Parameter count from Body IR.
        expected: usize,
        /// Argument count supplied by the caller.
        actual: usize,
    },
    /// A Body-IR construct lies outside the declared first replacement profile.
    #[error(
        "replacement backend does not support {description} at original Incan source span {span_start}..{span_end}"
    )]
    Unsupported {
        /// Construct or semantic fact the first profile cannot execute.
        description: String,
        /// Original Incan source span carried by Body IR.
        span: HirSourceSpan,
        /// Start byte offset duplicated for typed error formatting.
        span_start: usize,
        /// End byte offset duplicated for typed error formatting.
        span_end: usize,
    },
    /// A selected operation reached a source-observable runtime failure.
    #[error("replacement backend runtime failure at original Incan source span {span_start}..{span_end}: {detail}")]
    RuntimeFailure {
        /// Source-observable runtime-failure description.
        detail: String,
        /// Original Incan source span carried by Body IR.
        span: HirSourceSpan,
        /// Start byte offset duplicated for typed error formatting.
        span_start: usize,
        /// End byte offset duplicated for typed error formatting.
        span_end: usize,
    },
}

impl ReplacementExecutionError {
    /// Construct a typed, source-span-preserving refusal for an unsupported source-profile boundary.
    #[must_use]
    pub fn unsupported_profile(description: impl Into<String>, span: HirSourceSpan) -> Self {
        unsupported(description, span)
    }

    /// Return the stable diagnostic code for this replacement outcome.
    pub const fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::MissingFunction { .. } | Self::ArgumentCount { .. } => "INCAN-R988-ENTRYPOINT",
            Self::Unsupported { .. } => "INCAN-R988-UNSUPPORTED",
            Self::RuntimeFailure { .. } => "INCAN-R988-RUNTIME",
        }
    }

    /// Return the original Incan source location when this outcome arose from Body IR.
    pub const fn primary_span(&self) -> Option<HirSourceSpan> {
        match self {
            Self::Unsupported { span, .. } | Self::RuntimeFailure { span, .. } => Some(*span),
            Self::MissingFunction { .. } | Self::ArgumentCount { .. } => None,
        }
    }
}

/// Validate and prepare one Body-IR free function for later direct execution.
///
/// This side-effect-free boundary lets callers route availability through #986 selection and receipt logic before
/// committing to execution. Only [`execute_prevalidated_free_function`] can consume the returned capability.
pub fn prepare_free_function_execution<'module, 'args>(
    module: &'module BodyIrModule,
    name: &str,
    args: &'args [ReplacementValue],
) -> Result<ValidatedFreeFunctionExecution<'module, 'args>, ReplacementExecutionError> {
    let body = named_free_function(module, name)?;
    validate_single_function_module(module, name)?;
    if body.is_generator() {
        return Err(unsupported("generator body", body.span));
    }
    if body.param_locals.len() != args.len() {
        return Err(ReplacementExecutionError::ArgumentCount {
            name: name.to_string(),
            expected: body.param_locals.len(),
            actual: args.len(),
        });
    }
    validate_scalar_arguments(args, body.span)?;
    validate_binding_identity(body)?;
    let range_iterator_locals = range_iterator_locals(&body.block);
    validate_collection_local_types(body, &body.block, &range_iterator_locals)?;
    let tuple_iteration_locals = builtin_iteration_destinations(&body.block);
    let scalar_tuple_collection_locals = scalar_tuple_collection_elements(&body.block);
    validate_block_profile(&body.block, &tuple_iteration_locals, &scalar_tuple_collection_locals)?;
    Ok(ValidatedFreeFunctionExecution {
        module,
        name: name.to_string(),
        args,
    })
}

/// Execute one named, free-function Body IR body with concrete scalar arguments.
///
/// The caller must already have parsed and typechecked the source before constructing `module`; this boundary only
/// consumes Body IR and refuses unsupported operations rather than rerunning frontend or generated-Rust semantics.
pub fn execute_free_function(
    module: &BodyIrModule,
    name: &str,
    args: &[ReplacementValue],
) -> Result<ReplacementExecution, ReplacementExecutionError> {
    let execution = prepare_free_function_execution(module, name, args)?;
    execute_prevalidated_free_function(execution)
}

/// Execute one free function that has already passed [`prepare_free_function_execution`].
///
/// This consumes the validated capability, preserving the rule that callers select the source profile before the
/// direct Body-IR executor observes a result.
pub fn execute_prevalidated_free_function(
    execution: ValidatedFreeFunctionExecution<'_, '_>,
) -> Result<ReplacementExecution, ReplacementExecutionError> {
    let body = named_free_function(execution.module, &execution.name)?;

    let mut executor = BodyExecutor::new(body, execution.args);
    let flow = executor.execute_block(&body.block)?;
    let value = match flow {
        Flow::Return(value) => match value {
            Some(value) => value,
            None => ReplacementValue::Unit,
        },
        Flow::Next => ReplacementValue::Unit,
        Flow::Break | Flow::Continue => {
            return Err(unsupported("loop control outside a normalized loop", body.span));
        }
    };
    ensure_scalar_result(&value, body.span)?;
    let body_snapshot = body.render_snapshot();
    let ownership_summary = canonical_ownership_summary(&executor.ownership_reads);
    let requirements_summary = canonical_runtime_requirements_summary(&body.runtime_requirements);
    let output_identity = digest_output(&[
        body_snapshot.as_str(),
        value.observable_text().as_str(),
        ownership_summary.as_str(),
        requirements_summary.as_str(),
    ]);
    Ok(ReplacementExecution {
        value,
        body_snapshot,
        ownership_reads: executor.ownership_reads,
        runtime_requirements: body.runtime_requirements.clone(),
        output_identity,
    })
}

/// Locate the requested free-function body without inventing a fallback entrypoint.
fn named_free_function<'a>(module: &'a BodyIrModule, name: &str) -> Result<&'a Body, ReplacementExecutionError> {
    module
        .bodies
        .iter()
        .find(|body| body.name == name)
        .ok_or_else(|| ReplacementExecutionError::MissingFunction { name: name.to_string() })
}

/// Reject any additional free function because this first CLI profile admits exactly one selected entrypoint body.
fn validate_single_function_module(module: &BodyIrModule, name: &str) -> Result<(), ReplacementExecutionError> {
    if let Some(extra) = module.bodies.iter().find(|body| body.name != name) {
        return Err(unsupported(
            format!(
                "additional free function `{}` outside the selected replacement entrypoint",
                extra.name
            ),
            extra.span,
        ));
    }
    Ok(())
}

/// Reject non-scalar direct API arguments before they can widen the first replacement profile.
fn validate_scalar_arguments(args: &[ReplacementValue], span: HirSourceSpan) -> Result<(), ReplacementExecutionError> {
    for argument in args {
        if !matches!(
            argument,
            ReplacementValue::Int(_) | ReplacementValue::Bool(_) | ReplacementValue::Str(_) | ReplacementValue::Unit
        ) {
            return Err(unsupported(
                format!("{} argument in the scalar replacement profile", value_kind(argument)),
                span,
            ));
        }
    }
    Ok(())
}

/// Reject repeated user-binding spellings because Body IR v0 cannot distinguish shadowing from reassignment safely.
fn validate_binding_identity(body: &Body) -> Result<(), ReplacementExecutionError> {
    let mut declared = BTreeMap::new();
    for local in &body.locals {
        if !matches!(local.origin, LocalOrigin::Parameter | LocalOrigin::UserBinding) {
            continue;
        }
        let Some(name) = local.name.as_deref() else {
            continue;
        };
        if declared.insert(name, local.span).is_some() {
            return Err(unsupported(
                format!(
                    "repeated user binding `{name}` (lexical shadowing or reassignment); Body IR v0 does not yet carry binding-equivalence facts for direct execution"
                ),
                local.span,
            ));
        }
    }
    Ok(())
}

/// Validate every list, tuple, and builtin-iteration local against the typed Body IR profile.
///
/// Runtime operands alone cannot classify an empty list. This pass uses each authoritative local declaration, so an
/// empty `list[int]` refuses before it can be mistaken for the vacuously valid empty form of the selected profile.
fn validate_collection_local_types(
    body: &Body,
    block: &incan_semantics_core::body_ir::Block,
    range_iterator_locals: &BTreeSet<LocalId>,
) -> Result<(), ReplacementExecutionError> {
    for statement in &block.stmts {
        match &statement.kind {
            StatementKind::Assign {
                place,
                rvalue: Rvalue::Aggregate(AggregateKind::Tuple, _),
            } => validate_scalar_pair_tuple_local_type(
                body,
                bare_local(place, statement.span)?,
                statement.span,
                "tuple aggregate destination",
            )?,
            StatementKind::Assign {
                place,
                rvalue: Rvalue::Aggregate(AggregateKind::List, operands),
            } => {
                validate_scalar_pair_list_local_type(
                    body,
                    bare_local(place, statement.span)?,
                    statement.span,
                    "list aggregate destination",
                )?;
                for operand in operands {
                    let Operand::Place(place_operand) = operand else {
                        continue;
                    };
                    validate_scalar_pair_tuple_local_type(
                        body,
                        bare_local(&place_operand.place, statement.span)?,
                        statement.span,
                        "list aggregate element",
                    )?;
                }
            }
            StatementKind::IterNext {
                destination,
                iterator: Operand::Place(iterator),
                protocol: IterProtocol::Builtin,
            } => {
                let iterator_local = bare_local(&iterator.place, statement.span)?;
                if range_iterator_locals.contains(&iterator_local) {
                    validate_range_iteration_local_types(body, destination, iterator_local, statement.span)?;
                } else {
                    validate_scalar_pair_tuple_local_type(
                        body,
                        bare_local(destination, statement.span)?,
                        statement.span,
                        "builtin collection iteration destination",
                    )?;
                    validate_scalar_pair_list_local_type(
                        body,
                        iterator_local,
                        statement.span,
                        "builtin collection iterator",
                    )?;
                }
            }
            StatementKind::If {
                then_block, else_block, ..
            } => {
                validate_collection_local_types(body, then_block, range_iterator_locals)?;
                if let Some(else_block) = else_block {
                    validate_collection_local_types(body, else_block, range_iterator_locals)?;
                }
            }
            StatementKind::Loop { body: loop_body } => {
                validate_collection_local_types(body, loop_body, range_iterator_locals)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Collect builtin iterator locals admitted by the current `range` source-spelling rule.
///
/// Body IR v0 records [`Callee::Function`] by source spelling, not resolved builtin identity. The current replacement
/// CLI profile rejects imports and sibling free functions before it reaches this executor, so its one source-local
/// `main` cannot supply an alternate `range` binding. Within that boundary, `range(...)` enters normalized general
/// iteration as a `Call`, then passes through plain temporary aliases before `IterNext` polls it. The executor
/// materializes that admitted spelling as [`ReplacementValue::Range`] without treating an arbitrary `list[int]` as
/// a collection the new profile admits. Canonical call-target identity is deferred to #1042.
fn range_iterator_locals(block: &incan_semantics_core::body_ir::Block) -> BTreeSet<LocalId> {
    let mut locals = BTreeSet::new();
    while collect_range_iterator_locals(block, &mut locals) {}
    locals
}

/// Extend the admitted `range` source-spelling aliases until the enclosing body reaches a fixed point.
fn collect_range_iterator_locals(
    block: &incan_semantics_core::body_ir::Block,
    range_locals: &mut BTreeSet<LocalId>,
) -> bool {
    let mut changed = false;
    for statement in &block.stmts {
        match &statement.kind {
            StatementKind::Call {
                destination: Some(destination),
                callee: Callee::Function(name),
                ..
            } if name == "range" && destination.projection.is_empty() => {
                changed |= range_locals.insert(destination.local);
            }
            StatementKind::Assign {
                place,
                rvalue: Rvalue::Use(Operand::Place(source)),
            } if place.projection.is_empty()
                && source.place.projection.is_empty()
                && range_locals.contains(&source.place.local) =>
            {
                changed |= range_locals.insert(place.local);
            }
            StatementKind::If {
                then_block, else_block, ..
            } => {
                changed |= collect_range_iterator_locals(then_block, range_locals);
                if let Some(else_block) = else_block {
                    changed |= collect_range_iterator_locals(else_block, range_locals);
                }
            }
            StatementKind::Loop { body } => {
                changed |= collect_range_iterator_locals(body, range_locals);
            }
            _ => {}
        }
    }
    changed
}

/// Validate the compiler-owned local types of a preserved scalar `range` source-spelling iteration.
fn validate_range_iteration_local_types(
    body: &Body,
    destination: &Place,
    iterator: LocalId,
    span: HirSourceSpan,
) -> Result<(), ReplacementExecutionError> {
    let destination = bare_local(destination, span)?;
    let destination_ty = declared_local_type(body, destination, span)?;
    if !is_int_type(destination_ty) {
        return Err(unsupported(
            format!("range iteration destination has Body-IR type `{destination_ty}`, not int"),
            span,
        ));
    }
    let iterator_ty = declared_local_type(body, iterator, span)?;
    if is_range_iterator_type(iterator_ty) {
        Ok(())
    } else {
        Err(unsupported(
            format!("range iteration iterator has Body-IR type `{iterator_ty}`, not list[int]"),
            span,
        ))
    }
}

/// Validate that a Body-IR local has the only tuple type admitted as a replacement collection element.
fn validate_scalar_pair_tuple_local_type(
    body: &Body,
    local: LocalId,
    span: HirSourceSpan,
    role: &str,
) -> Result<(), ReplacementExecutionError> {
    let ty = declared_local_type(body, local, span)?;
    if is_scalar_pair_tuple_type(ty) {
        Ok(())
    } else {
        Err(unsupported(
            format!("{role} has Body-IR type `{ty}`, not tuple[scalar, scalar]"),
            span,
        ))
    }
}

/// Validate that a Body-IR local has the only list type admitted by the replacement collection profile.
fn validate_scalar_pair_list_local_type(
    body: &Body,
    local: LocalId,
    span: HirSourceSpan,
    role: &str,
) -> Result<(), ReplacementExecutionError> {
    let ty = declared_local_type(body, local, span)?;
    if is_scalar_pair_list_type(ty) {
        Ok(())
    } else {
        Err(unsupported(
            format!("{role} has Body-IR type `{ty}`, not list[tuple[scalar, scalar]]"),
            span,
        ))
    }
}

/// Return a local's compiler-owned type or refuse malformed Body IR at the owning source span.
fn declared_local_type(
    body: &Body,
    local: LocalId,
    span: HirSourceSpan,
) -> Result<&IncanType, ReplacementExecutionError> {
    body.locals
        .iter()
        .find(|declaration| declaration.id == local)
        .map(|declaration| &declaration.ty)
        .ok_or_else(|| unsupported("Body-IR local without a declared type", span))
}

/// Return whether a type is a two-element tuple of scalar shapes the replacement runtime already supports.
fn is_scalar_pair_tuple_type(ty: &IncanType) -> bool {
    match ty {
        IncanType::Tuple(elements) => {
            matches!(elements.as_slice(), [left, right] if is_collection_scalar_type(left) && is_collection_scalar_type(right))
        }
        IncanType::Generic { base, args } if collections::from_str(base) == Some(CollectionTypeId::Tuple) => {
            matches!(args.as_slice(), [left, right] if is_collection_scalar_type(left) && is_collection_scalar_type(right))
        }
        _ => false,
    }
}

/// Return whether a type is `list[tuple[scalar, scalar]]` according to the canonical collection registry.
fn is_scalar_pair_list_type(ty: &IncanType) -> bool {
    matches!(
        ty,
        IncanType::Generic { base, args }
            if collections::from_str(base) == Some(CollectionTypeId::List)
                && matches!(args.as_slice(), [element] if is_scalar_pair_tuple_type(element))
    )
}

/// Return whether a type is the compiler's list representation for the profile's `range` iterator.
fn is_range_iterator_type(ty: &IncanType) -> bool {
    matches!(
        ty,
        IncanType::Generic { base, args }
            if collections::from_str(base) == Some(CollectionTypeId::List)
                && matches!(args.as_slice(), [element] if is_int_type(element))
    )
}

/// Return whether a type is the integer scalar used by the selected `range` loop lowering.
const fn is_int_type(ty: &IncanType) -> bool {
    matches!(ty, IncanType::Primitive(IncanPrimitiveType::Int))
}

/// Return whether a type is a scalar shape the selected collection runtime can materialize and project.
fn is_collection_scalar_type(ty: &IncanType) -> bool {
    matches!(
        ty,
        IncanType::Primitive(
            IncanPrimitiveType::Int | IncanPrimitiveType::Bool | IncanPrimitiveType::Str | IncanPrimitiveType::Unit
        )
    )
}

/// Collect the local identities written by builtin collection polling across one normalized body.
///
/// Only these compiler-created item locals may later be projected as a selected scalar tuple element. This keeps a
/// standalone tuple field access outside the profile even though it uses the same `PlaceElem::Field` representation.
fn builtin_iteration_destinations(block: &incan_semantics_core::body_ir::Block) -> BTreeSet<LocalId> {
    let mut destinations = BTreeSet::new();
    collect_builtin_iteration_destinations(block, &mut destinations);
    destinations
}

/// Recurse through normalized control flow to collect every builtin iteration destination local.
fn collect_builtin_iteration_destinations(
    block: &incan_semantics_core::body_ir::Block,
    destinations: &mut BTreeSet<LocalId>,
) {
    for statement in &block.stmts {
        match &statement.kind {
            StatementKind::If {
                then_block, else_block, ..
            } => {
                collect_builtin_iteration_destinations(then_block, destinations);
                if let Some(else_block) = else_block {
                    collect_builtin_iteration_destinations(else_block, destinations);
                }
            }
            StatementKind::Loop { body } => collect_builtin_iteration_destinations(body, destinations),
            StatementKind::IterNext {
                destination,
                protocol: IterProtocol::Builtin,
                ..
            } => {
                destinations.insert(destination.local);
            }
            _ => {}
        }
    }
}

/// Collect the tuple locals that are direct elements of a source-local list aggregate.
///
/// Body IR lowers each tuple literal before the surrounding list aggregate, so a replacement profile cannot infer
/// this relationship from a single rvalue. Intersecting tuple-assignment destinations with direct list operands
/// makes that lowering relationship explicit and prevents standalone tuples or scalar lists from slipping through
/// as unobserved runtime values.
fn scalar_tuple_collection_elements(block: &incan_semantics_core::body_ir::Block) -> BTreeSet<LocalId> {
    let mut tuple_destinations = BTreeSet::new();
    let mut list_operands = BTreeSet::new();
    collect_scalar_tuple_collection_locals(block, &mut tuple_destinations, &mut list_operands);
    tuple_destinations.intersection(&list_operands).copied().collect()
}

/// Recurse through control flow to collect tuple assignments and direct list-aggregate operands.
fn collect_scalar_tuple_collection_locals(
    block: &incan_semantics_core::body_ir::Block,
    tuple_destinations: &mut BTreeSet<LocalId>,
    list_operands: &mut BTreeSet<LocalId>,
) {
    for statement in &block.stmts {
        match &statement.kind {
            StatementKind::Assign {
                place,
                rvalue: Rvalue::Aggregate(AggregateKind::Tuple, _),
            } if place.projection.is_empty() => {
                tuple_destinations.insert(place.local);
            }
            StatementKind::Assign {
                rvalue: Rvalue::Aggregate(AggregateKind::List, operands),
                ..
            } => {
                for operand in operands {
                    if let Operand::Place(place_operand) = operand
                        && place_operand.place.projection.is_empty()
                    {
                        list_operands.insert(place_operand.place.local);
                    }
                }
            }
            StatementKind::If {
                then_block, else_block, ..
            } => {
                collect_scalar_tuple_collection_locals(then_block, tuple_destinations, list_operands);
                if let Some(else_block) = else_block {
                    collect_scalar_tuple_collection_locals(else_block, tuple_destinations, list_operands);
                }
            }
            StatementKind::Loop { body } => {
                collect_scalar_tuple_collection_locals(body, tuple_destinations, list_operands);
            }
            _ => {}
        }
    }
}

/// Validate every statement in one normalized Body-IR block before the direct executor starts.
fn validate_block_profile(
    block: &incan_semantics_core::body_ir::Block,
    tuple_iteration_locals: &BTreeSet<LocalId>,
    scalar_tuple_collection_locals: &BTreeSet<LocalId>,
) -> Result<(), ReplacementExecutionError> {
    for statement in &block.stmts {
        validate_statement_profile(statement, tuple_iteration_locals, scalar_tuple_collection_locals)?;
    }
    Ok(())
}

/// Validate one statement against the deliberately narrow #988 direct-execution profile.
fn validate_statement_profile(
    statement: &Statement,
    tuple_iteration_locals: &BTreeSet<LocalId>,
    scalar_tuple_collection_locals: &BTreeSet<LocalId>,
) -> Result<(), ReplacementExecutionError> {
    match &statement.kind {
        StatementKind::Assign { place, rvalue } => {
            validate_bare_local(place, statement.span)?;
            validate_rvalue_profile(
                rvalue,
                statement.span,
                tuple_iteration_locals,
                scalar_tuple_collection_locals,
                Some(place.local),
            )
        }
        StatementKind::Call {
            destination,
            callee,
            args,
            may_panic: _,
        } => {
            let destination = destination
                .as_ref()
                .ok_or_else(|| unsupported("discarded call result", statement.span))?;
            validate_bare_local(destination, statement.span)?;
            validate_call_profile(callee, args, statement.span, tuple_iteration_locals)
        }
        StatementKind::Drop { .. } => Ok(()),
        StatementKind::If {
            cond,
            then_block,
            else_block,
        } => {
            validate_operand_profile(cond, statement.span, tuple_iteration_locals)?;
            validate_block_profile(then_block, tuple_iteration_locals, scalar_tuple_collection_locals)?;
            if let Some(else_block) = else_block {
                validate_block_profile(else_block, tuple_iteration_locals, scalar_tuple_collection_locals)?;
            }
            Ok(())
        }
        StatementKind::Loop { body } => {
            validate_block_profile(body, tuple_iteration_locals, scalar_tuple_collection_locals)
        }
        StatementKind::Break { value: Some(_) } => Err(unsupported("value-carrying loop break", statement.span)),
        StatementKind::Break { value: None } | StatementKind::Continue => Ok(()),
        StatementKind::Return { value } => value.as_ref().map_or(Ok(()), |value| {
            validate_operand_profile(value, statement.span, tuple_iteration_locals)
        }),
        StatementKind::Assert {
            cond,
            message,
            may_panic: _,
        } => {
            validate_operand_profile(cond, statement.span, tuple_iteration_locals)?;
            message.as_ref().map_or(Ok(()), |message| {
                validate_operand_profile(message, statement.span, tuple_iteration_locals)
            })
        }
        StatementKind::Expr { value } => validate_operand_profile(value, statement.span, tuple_iteration_locals),
        StatementKind::IterNext {
            destination,
            iterator,
            protocol: IterProtocol::Builtin,
        } => {
            validate_bare_local(destination, statement.span)?;
            validate_operand_profile(iterator, statement.span, tuple_iteration_locals)
        }
        StatementKind::Yield { .. } => Err(unsupported("generator yield", statement.span)),
        // #1164 gave Body IR an async vocabulary. Executing it -- task state, suspension, wake/resume, arm
        // selection, cancellation -- is #1155's. Until then these refuse by name at the original source span
        // rather than leaving the executor unable to compile against the representation.
        StatementKind::Await { .. } => Err(unsupported("async await suspension", statement.span)),
        StatementKind::Race { .. } => Err(unsupported("async race selection", statement.span)),
        StatementKind::TryPropagate { .. } => Err(unsupported("try propagation", statement.span)),
        StatementKind::IterNext { .. } => Err(unsupported("non-range iteration", statement.span)),
        StatementKind::Unsupported { description } => Err(unsupported(description, statement.span)),
    }
}

/// Validate one Body-IR call before the executor dispatches it.
fn validate_call_profile(
    callee: &Callee,
    args: &[Operand],
    span: HirSourceSpan,
    tuple_iteration_locals: &BTreeSet<LocalId>,
) -> Result<(), ReplacementExecutionError> {
    let supported = match callee {
        Callee::Helper(HelperOp::StrConcat) => true,
        // Body IR v0 has only this source spelling. The one-function, import-free CLI profile makes it unambiguous
        // for #988; canonical call-target identity belongs to #1042.
        Callee::Function(name) => name == "range",
        Callee::Method(name) => name == "collect",
        Callee::Helper(_) => false,
    };
    if !supported {
        return Err(unsupported(format!("call to {}", callee_label(callee)), span));
    }
    for arg in args {
        validate_operand_profile(arg, span, tuple_iteration_locals)?;
    }
    Ok(())
}

/// Validate one rvalue before it can be evaluated by the bounded executor.
fn validate_rvalue_profile(
    rvalue: &Rvalue,
    span: HirSourceSpan,
    tuple_iteration_locals: &BTreeSet<LocalId>,
    scalar_tuple_collection_locals: &BTreeSet<LocalId>,
    destination: Option<LocalId>,
) -> Result<(), ReplacementExecutionError> {
    match rvalue {
        Rvalue::Use(operand) | Rvalue::UnaryOp(_, operand) => {
            validate_operand_profile(operand, span, tuple_iteration_locals)
        }
        Rvalue::BinaryOp(_, left, right) => {
            validate_operand_profile(left, span, tuple_iteration_locals)?;
            validate_operand_profile(right, span, tuple_iteration_locals)
        }
        Rvalue::Aggregate(kind, operands) => validate_aggregate_profile(
            kind,
            operands,
            span,
            tuple_iteration_locals,
            scalar_tuple_collection_locals,
            destination,
        ),
        Rvalue::Format(_) => Err(unsupported("f-string", span)),
        Rvalue::Closure { .. } => Err(unsupported("callable value", span)),
        Rvalue::Generator {
            source,
            captured_operands,
            body,
        } => {
            validate_operand_profile(source, span, tuple_iteration_locals)?;
            for operand in captured_operands {
                validate_operand_profile(operand, span, tuple_iteration_locals)?;
            }
            validate_generator_body_profile(body, tuple_iteration_locals, scalar_tuple_collection_locals)
        }
        Rvalue::Match { .. } => Err(unsupported("match expression", span)),
    }
}

/// Validate the deferred body carried by an admitted generator-expression rvalue.
///
/// A normal function body still refuses [`StatementKind::Yield`]. Within this explicitly stored generator body,
/// however, `yield` is exactly the deferred result boundary and is interpreted only by `.collect()` below.
fn validate_generator_body_profile(
    body: &GeneratorBody,
    tuple_iteration_locals: &BTreeSet<LocalId>,
    scalar_tuple_collection_locals: &BTreeSet<LocalId>,
) -> Result<(), ReplacementExecutionError> {
    validate_generator_statements_profile(&body.stmts, tuple_iteration_locals, scalar_tuple_collection_locals)
}

/// Recurse through the generator's normalized control flow while admitting only its terminal yields.
fn validate_generator_statements_profile(
    statements: &[Statement],
    tuple_iteration_locals: &BTreeSet<LocalId>,
    scalar_tuple_collection_locals: &BTreeSet<LocalId>,
) -> Result<(), ReplacementExecutionError> {
    for statement in statements {
        match &statement.kind {
            StatementKind::Yield { value } => {
                validate_operand_profile(value, statement.span, tuple_iteration_locals)?;
            }
            StatementKind::If {
                cond,
                then_block,
                else_block,
            } => {
                validate_operand_profile(cond, statement.span, tuple_iteration_locals)?;
                validate_generator_statements_profile(
                    &then_block.stmts,
                    tuple_iteration_locals,
                    scalar_tuple_collection_locals,
                )?;
                if let Some(else_block) = else_block {
                    validate_generator_statements_profile(
                        &else_block.stmts,
                        tuple_iteration_locals,
                        scalar_tuple_collection_locals,
                    )?;
                }
            }
            StatementKind::Loop { body } => validate_generator_statements_profile(
                &body.stmts,
                tuple_iteration_locals,
                scalar_tuple_collection_locals,
            )?,
            StatementKind::Return { .. } => {
                return Err(unsupported("return from a generator expression", statement.span));
            }
            _ => validate_statement_profile(statement, tuple_iteration_locals, scalar_tuple_collection_locals)?,
        }
    }
    Ok(())
}

/// Validate the narrow aggregate vocabulary needed to materialize scalar tuple collections.
fn validate_aggregate_profile(
    kind: &AggregateKind,
    operands: &[Operand],
    span: HirSourceSpan,
    tuple_iteration_locals: &BTreeSet<LocalId>,
    scalar_tuple_collection_locals: &BTreeSet<LocalId>,
    destination: Option<LocalId>,
) -> Result<(), ReplacementExecutionError> {
    match kind {
        AggregateKind::Tuple
            if operands.len() == 2
                && destination.is_some_and(|local| scalar_tuple_collection_locals.contains(&local)) =>
        {
            for operand in operands {
                validate_operand_profile(operand, span, tuple_iteration_locals)?;
            }
            Ok(())
        }
        AggregateKind::Tuple => Err(unsupported(
            "tuple aggregate outside the two-scalar collection-element profile",
            span,
        )),
        AggregateKind::List => {
            for operand in operands {
                let Operand::Place(place_operand) = operand else {
                    return Err(unsupported(
                        "list aggregate outside the list[tuple[scalar, scalar]] profile",
                        span,
                    ));
                };
                if !place_operand.place.projection.is_empty()
                    || !scalar_tuple_collection_locals.contains(&place_operand.place.local)
                {
                    return Err(unsupported(
                        "list aggregate outside the list[tuple[scalar, scalar]] profile",
                        span,
                    ));
                }
                validate_operand_profile(operand, span, tuple_iteration_locals)?;
            }
            Ok(())
        }
        _ => Err(unsupported(format!("{} aggregate", aggregate_label(kind)), span)),
    }
}

/// Validate a single operand's place shape and compiler-owned ownership decision.
fn validate_operand_profile(
    operand: &Operand,
    span: HirSourceSpan,
    tuple_iteration_locals: &BTreeSet<LocalId>,
) -> Result<(), ReplacementExecutionError> {
    let Operand::Place(place_operand) = operand else {
        return Ok(());
    };
    validate_read_place(&place_operand.place, span, tuple_iteration_locals)?;
    if matches!(place_operand.fact, OwnershipFact::Unknown) {
        return Err(unsupported("unknown ownership fact", span));
    }
    Ok(())
}

/// Reject projections at the profile boundary while preserving the statement's original source authority.
fn validate_bare_local(place: &Place, span: HirSourceSpan) -> Result<(), ReplacementExecutionError> {
    bare_local(place, span).map(|_| ())
}

/// Admit only the one-level numeric tuple fields lowering uses for `for a, b in pairs`.
fn validate_read_place(
    place: &Place,
    span: HirSourceSpan,
    tuple_iteration_locals: &BTreeSet<LocalId>,
) -> Result<(), ReplacementExecutionError> {
    match place.projection.as_slice() {
        [] => Ok(()),
        [PlaceElem::Field(field)]
            if matches!(field.as_str(), "0" | "1") && tuple_iteration_locals.contains(&place.local) =>
        {
            Ok(())
        }
        [PlaceElem::Field(field)] if matches!(field.as_str(), "0" | "1") => Err(unsupported(
            "tuple field projection outside a scalar tuple collection iteration",
            span,
        )),
        [PlaceElem::Field(_)] => Err(unsupported(
            "non-numeric tuple field projection outside the scalar tuple collection profile",
            span,
        )),
        // The only admitted index target is a list created by the generator-expression `.collect()` consumer. The
        // runtime check keeps ordinary list indexing outside the original scalar-pair collection profile.
        [PlaceElem::Index(index)] => validate_operand_profile(index, span, tuple_iteration_locals),
        [PlaceElem::Slice { .. }] => Err(unsupported(
            "slice projection outside the scalar tuple collection profile",
            span,
        )),
        _ => Err(unsupported(
            "nested place projection outside the scalar tuple collection profile",
            span,
        )),
    }
}

/// Mutable interpreter state for one Body-IR execution.
struct BodyExecutor {
    locals: BTreeMap<LocalId, ReplacementValue>,
    ownership_reads: Vec<OwnershipRead>,
    steps: usize,
}

impl BodyExecutor {
    /// Bind the already-typechecked call arguments to their Body-IR parameter locals.
    fn new(body: &Body, args: &[ReplacementValue]) -> Self {
        let locals = body.param_locals.iter().copied().zip(args.iter().cloned()).collect();
        Self {
            locals,
            ownership_reads: Vec::new(),
            steps: 0,
        }
    }

    /// Execute one normalized Body-IR block until it falls through or produces control flow.
    fn execute_block(
        &mut self,
        block: &incan_semantics_core::body_ir::Block,
    ) -> Result<Flow, ReplacementExecutionError> {
        for statement in &block.stmts {
            self.record_step(statement.span)?;
            match self.execute_statement(statement)? {
                Flow::Next => {}
                flow => return Ok(flow),
            }
        }
        Ok(Flow::Next)
    }

    /// Execute one Body-IR statement without consulting generated Rust or the legacy backend.
    fn execute_statement(&mut self, statement: &Statement) -> Result<Flow, ReplacementExecutionError> {
        match &statement.kind {
            StatementKind::Assign { place, rvalue } => {
                let local = bare_local(place, statement.span)?;
                let value = self.evaluate_rvalue(rvalue, statement.span)?;
                self.assign_local(local, value);
                Ok(Flow::Next)
            }
            StatementKind::Call {
                destination,
                callee,
                args,
                may_panic: _,
            } => self.execute_call(destination.as_ref(), callee, args, statement.span),
            StatementKind::Drop { local } => {
                let _ = self.locals.remove(local);
                Ok(Flow::Next)
            }
            StatementKind::If {
                cond,
                then_block,
                else_block,
            } => {
                if self.evaluate_operand(cond, statement.span)?.into_bool(statement.span)? {
                    self.execute_block(then_block)
                } else if let Some(else_block) = else_block {
                    self.execute_block(else_block)
                } else {
                    Ok(Flow::Next)
                }
            }
            StatementKind::Loop { body } => self.execute_loop(body, statement.span),
            StatementKind::Break { value: Some(_) } => Err(unsupported("value-carrying loop break", statement.span)),
            StatementKind::Break { value: None } => Ok(Flow::Break),
            StatementKind::Continue => Ok(Flow::Continue),
            StatementKind::Return { value } => Ok(Flow::Return(
                value
                    .as_ref()
                    .map(|value| self.evaluate_operand(value, statement.span))
                    .transpose()?,
            )),
            StatementKind::Assert {
                cond,
                message,
                may_panic: _,
            } => {
                if self.evaluate_operand(cond, statement.span)?.into_bool(statement.span)? {
                    Ok(Flow::Next)
                } else {
                    let detail = match message {
                        Some(message) => format!(
                            "assertion failed: {}",
                            self.evaluate_operand(message, statement.span)?.observable_text()
                        ),
                        None => "assertion failed".to_string(),
                    };
                    Err(runtime_failure(detail, statement.span))
                }
            }
            StatementKind::Expr { value } => {
                let _ = self.evaluate_operand(value, statement.span)?;
                Ok(Flow::Next)
            }
            StatementKind::IterNext {
                destination,
                iterator,
                protocol: IterProtocol::Builtin,
            } => self.execute_builtin_next(destination, iterator, statement.span),
            StatementKind::Yield { .. } => Err(unsupported(
                "generator yield outside a generator expression",
                statement.span,
            )),
            StatementKind::TryPropagate { .. } => Err(unsupported("try propagation", statement.span)),
            // See the matching arms in `validate_statement_profile`: representation is #1164's, execution is #1155's.
            StatementKind::Await { .. } => Err(unsupported("async await suspension", statement.span)),
            StatementKind::Race { .. } => Err(unsupported("async race selection", statement.span)),
            StatementKind::IterNext { .. } => Err(unsupported("non-range iteration", statement.span)),
            StatementKind::Unsupported { description } => Err(unsupported(description, statement.span)),
        }
    }

    /// Evaluate a supported helper, the profile's admitted `range` source-spelling call, or a lazy generator
    /// materialization. Generator construction itself is an rvalue; this call is the first supported consumer.
    fn execute_call(
        &mut self,
        destination: Option<&Place>,
        callee: &Callee,
        args: &[Operand],
        span: HirSourceSpan,
    ) -> Result<Flow, ReplacementExecutionError> {
        let destination = destination.ok_or_else(|| unsupported("discarded string-concatenation result", span))?;
        let local = bare_local(destination, span)?;
        let value = match callee {
            Callee::Helper(HelperOp::StrConcat) => {
                let [left, right] = args else {
                    return Err(unsupported("string-concatenation call arity", span));
                };
                let left = self.evaluate_operand(left, span)?.into_string(span)?;
                let right = self.evaluate_operand(right, span)?.into_string(span)?;
                ReplacementValue::Str(format!("{left}{right}"))
            }
            Callee::Function(name) if name == "range" => self.evaluate_range(args, span)?,
            Callee::Method(name) if name == "collect" => {
                let [receiver] = args else {
                    return Err(unsupported("generator collect call arity", span));
                };
                let generator = self.take_generator_receiver(receiver, span)?;
                self.collect_generator(generator, span)?
            }
            _ => return Err(unsupported(format!("call to {}", callee_label(callee)), span)),
        };
        self.assign_local(local, value);
        Ok(Flow::Next)
    }

    /// Materialize the admitted `range` source-spelling call before its normalized loop.
    fn evaluate_range(
        &mut self,
        args: &[Operand],
        span: HirSourceSpan,
    ) -> Result<ReplacementValue, ReplacementExecutionError> {
        let values = args
            .iter()
            .map(|argument| self.evaluate_operand(argument, span))
            .collect::<Result<Vec<_>, _>>()?;
        let ints = values
            .iter()
            .map(|value| match value {
                ReplacementValue::Int(value) => Ok(*value),
                value => Err(unsupported(format!("range argument using {}", value_kind(value)), span)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (next, end, step) = match ints.as_slice() {
            [end] => (0, *end, 1),
            [start, end] => (*start, *end, 1),
            [start, end, step] if *step != 0 => (*start, *end, *step),
            [_, _, 0] => return Err(runtime_failure("range step cannot be zero".to_string(), span)),
            _ => return Err(unsupported("range call arity", span)),
        };
        Ok(ReplacementValue::Range { next, end, step })
    }

    /// Poll one admitted range or scalar-tuple-list iterator and express exhaustion as the Body-IR loop break it
    /// represents.
    fn execute_builtin_next(
        &mut self,
        destination: &Place,
        iterator: &Operand,
        span: HirSourceSpan,
    ) -> Result<Flow, ReplacementExecutionError> {
        let Operand::Place(iterator) = iterator else {
            return Err(unsupported("non-place builtin iterator", span));
        };
        let iterator_local = bare_local(&iterator.place, span)?;
        self.ownership_reads.push(OwnershipRead {
            span,
            fact: iterator.fact,
            last_use: iterator.last_use,
        });
        let value = match self.locals.get_mut(&iterator_local) {
            Some(ReplacementValue::Range { next, end, step })
                if (*step > 0 && *next < *end) || (*step < 0 && *next > *end) =>
            {
                let value = *next;
                *next += *step;
                ReplacementValue::Int(value)
            }
            Some(ReplacementValue::Range { .. }) => return Ok(Flow::Break),
            Some(ReplacementValue::List { elements, next }) if *next < elements.len() => {
                let value = elements[*next].clone();
                *next += 1;
                value
            }
            Some(ReplacementValue::List { .. }) => return Ok(Flow::Break),
            Some(value) => return Err(unsupported(format!("iteration over {}", value_kind(value)), span)),
            None => {
                return Err(runtime_failure(
                    "read of an unavailable builtin iterator".to_string(),
                    span,
                ));
            }
        };
        self.assign_local(bare_local(destination, span)?, value);
        Ok(Flow::Next)
    }

    /// Assign exactly the authoritative Body-IR `LocalId` selected by lowering.
    ///
    /// The profile preflight rejects repeated user-binding names rather than reconstructing scope equivalence from
    /// spelling. Reassignment is therefore refused until Body IR carries binding-equivalence facts; no name-based
    /// aliasing is permitted here.
    fn assign_local(&mut self, local: LocalId, value: ReplacementValue) {
        self.locals.insert(local, value);
    }

    /// Execute one normalized Body-IR loop and propagate only non-local control flow outward.
    fn execute_loop(
        &mut self,
        body: &incan_semantics_core::body_ir::Block,
        span: HirSourceSpan,
    ) -> Result<Flow, ReplacementExecutionError> {
        loop {
            match self.execute_block(body)? {
                Flow::Next | Flow::Continue => {}
                Flow::Break => return Ok(Flow::Next),
                Flow::Return(value) => return Ok(Flow::Return(value)),
            }
            if self.steps >= MAX_EXECUTION_STEPS {
                return Err(runtime_failure(
                    format!("normalized loop exceeded the {MAX_EXECUTION_STEPS}-step replacement profile limit"),
                    span,
                ));
            }
        }
    }

    /// Evaluate an assignment rvalue supported by the initial profile.
    fn evaluate_rvalue(
        &mut self,
        rvalue: &Rvalue,
        span: HirSourceSpan,
    ) -> Result<ReplacementValue, ReplacementExecutionError> {
        match rvalue {
            Rvalue::Use(operand) => self.evaluate_operand(operand, span),
            Rvalue::UnaryOp(operator, operand) => self.evaluate_unary(*operator, operand, span),
            Rvalue::BinaryOp(operator, left, right) => self.evaluate_binary(*operator, left, right, span),
            Rvalue::Aggregate(kind, operands) => self.evaluate_aggregate(kind, operands, span),
            Rvalue::Format(_) => Err(unsupported("f-string", span)),
            Rvalue::Closure { .. } => Err(unsupported("callable value", span)),
            Rvalue::Generator {
                source,
                captured_operands,
                body,
            } => self.construct_generator(source, captured_operands, body, span),
            Rvalue::Match { .. } => Err(unsupported("match expression", span)),
        }
    }

    /// Capture a generator expression's construction-time source and free values without executing its body.
    fn construct_generator(
        &mut self,
        source: &Operand,
        captured_operands: &[Operand],
        body: &GeneratorBody,
        span: HirSourceSpan,
    ) -> Result<ReplacementValue, ReplacementExecutionError> {
        if captured_operands.len() != body.capture_locals.len() {
            return Err(unsupported("generator capture metadata mismatch", span));
        }
        let source = self.evaluate_operand(source, span)?;
        let captures = captured_operands
            .iter()
            .zip(&body.capture_locals)
            .map(|(operand, local)| self.evaluate_operand(operand, span).map(|value| (*local, value)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ReplacementValue::Generator(Box::new(ReplacementGenerator {
            source,
            captures,
            body: body.clone(),
        })))
    }

    /// Consume an admitted generator expression by running only its deferred Body-IR body.
    ///
    /// The child executor receives the source and captures already observed at construction time. It intentionally
    /// does not inherit any other outer locals, preventing deferred evaluation from reaching mutable construction
    /// scope state by name. Its ownership evidence and bounded step count are merged back into the caller only after
    /// the consumer has actually asked for collection.
    fn collect_generator(
        &mut self,
        generator: ReplacementGenerator,
        span: HirSourceSpan,
    ) -> Result<ReplacementValue, ReplacementExecutionError> {
        let mut locals = BTreeMap::new();
        locals.insert(generator.body.source_local, generator.source);
        for (local, value) in generator.captures {
            if locals.insert(local, value).is_some() {
                return Err(unsupported("generator capture aliases its source local", span));
            }
        }
        let mut deferred = Self {
            locals,
            ownership_reads: Vec::new(),
            steps: self.steps,
        };
        let mut elements = Vec::new();
        match deferred.collect_generator_statements(&generator.body.stmts, &mut elements)? {
            Flow::Next => {}
            Flow::Break | Flow::Continue => {
                return Err(unsupported("generator loop control outside a normalized loop", span));
            }
            Flow::Return(_) => return Err(unsupported("return from a generator expression", span)),
        }
        self.steps = deferred.steps;
        self.ownership_reads.extend(deferred.ownership_reads);
        Ok(ReplacementValue::CollectedGenerator { elements, next: 0 })
    }

    /// Take the generator receiver consumed by `.collect()` while retaining Body IR's recorded receiver read.
    ///
    /// Body-IR's generic method-lowering convention records receivers as borrows, but `Generator.collect` consumes
    /// its iterator in the runtime contract. The bounded executor therefore removes precisely this bare local after
    /// recording that compiler-owned read, so a second collection cannot manufacture a fresh deferred iterator.
    fn take_generator_receiver(
        &mut self,
        receiver: &Operand,
        span: HirSourceSpan,
    ) -> Result<ReplacementGenerator, ReplacementExecutionError> {
        let Operand::Place(place_operand) = receiver else {
            return Err(unsupported("non-place generator collect receiver", span));
        };
        let local = bare_local(&place_operand.place, span)?;
        self.ownership_reads.push(OwnershipRead {
            span,
            fact: place_operand.fact,
            last_use: place_operand.last_use,
        });
        let value = self
            .locals
            .remove(&local)
            .ok_or_else(|| runtime_failure("read of an unavailable generator receiver".to_string(), span))?;
        let ReplacementValue::Generator(generator) = value else {
            return Err(unsupported(
                format!(
                    "collecting {} outside the generator-expression profile",
                    value_kind(&value)
                ),
                span,
            ));
        };
        Ok(*generator)
    }

    /// Execute deferred generator statements, appending each explicit Body-IR `yield` and continuing its loop.
    fn collect_generator_statements(
        &mut self,
        statements: &[Statement],
        elements: &mut Vec<ReplacementValue>,
    ) -> Result<Flow, ReplacementExecutionError> {
        for statement in statements {
            self.record_step(statement.span)?;
            let flow = match &statement.kind {
                StatementKind::Yield { value } => {
                    elements.push(self.evaluate_operand(value, statement.span)?);
                    Flow::Next
                }
                StatementKind::If {
                    cond,
                    then_block,
                    else_block,
                } => {
                    if self.evaluate_operand(cond, statement.span)?.into_bool(statement.span)? {
                        self.collect_generator_statements(&then_block.stmts, elements)?
                    } else if let Some(else_block) = else_block {
                        self.collect_generator_statements(&else_block.stmts, elements)?
                    } else {
                        Flow::Next
                    }
                }
                StatementKind::Loop { body } => self.collect_generator_loop(&body.stmts, elements, statement.span)?,
                StatementKind::Return { .. } => {
                    return Err(unsupported("return from a generator expression", statement.span));
                }
                _ => self.execute_statement(statement)?,
            };
            match flow {
                Flow::Next => {}
                flow => return Ok(flow),
            }
        }
        Ok(Flow::Next)
    }

    /// Execute one normalized loop nested in a deferred generator body.
    fn collect_generator_loop(
        &mut self,
        statements: &[Statement],
        elements: &mut Vec<ReplacementValue>,
        span: HirSourceSpan,
    ) -> Result<Flow, ReplacementExecutionError> {
        loop {
            match self.collect_generator_statements(statements, elements)? {
                Flow::Next | Flow::Continue => {}
                Flow::Break => return Ok(Flow::Next),
                Flow::Return(value) => return Ok(Flow::Return(value)),
            }
            if self.steps >= MAX_EXECUTION_STEPS {
                return Err(runtime_failure(
                    format!(
                        "normalized generator loop exceeded the {MAX_EXECUTION_STEPS}-step replacement profile limit"
                    ),
                    span,
                ));
            }
        }
    }

    /// Materialize only scalar two-tuples and lists composed of those tuples for the selected loop profile.
    fn evaluate_aggregate(
        &mut self,
        kind: &AggregateKind,
        operands: &[Operand],
        span: HirSourceSpan,
    ) -> Result<ReplacementValue, ReplacementExecutionError> {
        let values = operands
            .iter()
            .map(|operand| self.evaluate_operand(operand, span))
            .collect::<Result<Vec<_>, _>>()?;
        match kind {
            AggregateKind::Tuple if values.len() == 2 && values.iter().all(ReplacementValue::is_collection_scalar) => {
                Ok(ReplacementValue::Tuple(values))
            }
            AggregateKind::Tuple => Err(unsupported(
                "tuple aggregate outside the two-scalar collection-element profile",
                span,
            )),
            AggregateKind::List if values.iter().all(ReplacementValue::is_scalar_pair_tuple) => {
                Ok(ReplacementValue::List {
                    elements: values,
                    next: 0,
                })
            }
            AggregateKind::List => Err(unsupported(
                "list aggregate outside the list[tuple[scalar, scalar]] profile",
                span,
            )),
            _ => Err(unsupported(format!("{} aggregate", aggregate_label(kind)), span)),
        }
    }

    /// Evaluate a scalar unary operation.
    fn evaluate_unary(
        &mut self,
        operator: UnOp,
        operand: &Operand,
        span: HirSourceSpan,
    ) -> Result<ReplacementValue, ReplacementExecutionError> {
        let value = self.evaluate_operand(operand, span)?;
        match (operator, value) {
            (UnOp::Neg, ReplacementValue::Int(value)) => Ok(ReplacementValue::Int(-value)),
            (UnOp::Not, ReplacementValue::Bool(value)) => Ok(ReplacementValue::Bool(!value)),
            (UnOp::Invert, ReplacementValue::Int(value)) => Ok(ReplacementValue::Int(!value)),
            (operator, value) => Err(unsupported(
                format!("{} applied to {}", unary_label(operator), value_kind(&value)),
                span,
            )),
        }
    }

    /// Evaluate a scalar arithmetic, comparison, or boolean operation.
    fn evaluate_binary(
        &mut self,
        operator: BinOp,
        left: &Operand,
        right: &Operand,
        span: HirSourceSpan,
    ) -> Result<ReplacementValue, ReplacementExecutionError> {
        let left = self.evaluate_operand(left, span)?;
        let right = self.evaluate_operand(right, span)?;
        match (operator, left, right) {
            (BinOp::Add, ReplacementValue::Int(left), ReplacementValue::Int(right)) => {
                Ok(ReplacementValue::Int(left + right))
            }
            (BinOp::Sub, ReplacementValue::Int(left), ReplacementValue::Int(right)) => {
                Ok(ReplacementValue::Int(left - right))
            }
            (BinOp::Mul, ReplacementValue::Int(left), ReplacementValue::Int(right)) => {
                Ok(ReplacementValue::Int(left * right))
            }
            (BinOp::FloorDiv, ReplacementValue::Int(_), ReplacementValue::Int(0))
            | (BinOp::Mod, ReplacementValue::Int(_), ReplacementValue::Int(0))
            | (BinOp::Div, ReplacementValue::Int(_), ReplacementValue::Int(0)) => {
                Err(runtime_failure("division or modulo by zero".to_string(), span))
            }
            (BinOp::FloorDiv, ReplacementValue::Int(left), ReplacementValue::Int(right)) => {
                checked_python_floor_division(left, right, span)
            }
            (BinOp::Mod, ReplacementValue::Int(left), ReplacementValue::Int(right)) => {
                Ok(ReplacementValue::Int(python_mod_i64(left, right)))
            }
            (BinOp::Eq, left, right) if left.is_collection_scalar() && right.is_collection_scalar() => {
                Ok(ReplacementValue::Bool(left == right))
            }
            (BinOp::Ne, left, right) if left.is_collection_scalar() && right.is_collection_scalar() => {
                Ok(ReplacementValue::Bool(left != right))
            }
            (BinOp::Lt, ReplacementValue::Int(left), ReplacementValue::Int(right)) => {
                Ok(ReplacementValue::Bool(left < right))
            }
            (BinOp::Le, ReplacementValue::Int(left), ReplacementValue::Int(right)) => {
                Ok(ReplacementValue::Bool(left <= right))
            }
            (BinOp::Gt, ReplacementValue::Int(left), ReplacementValue::Int(right)) => {
                Ok(ReplacementValue::Bool(left > right))
            }
            (BinOp::Ge, ReplacementValue::Int(left), ReplacementValue::Int(right)) => {
                Ok(ReplacementValue::Bool(left >= right))
            }
            (BinOp::And, ReplacementValue::Bool(left), ReplacementValue::Bool(right)) => {
                Ok(ReplacementValue::Bool(left && right))
            }
            (BinOp::Or, ReplacementValue::Bool(left), ReplacementValue::Bool(right)) => {
                Ok(ReplacementValue::Bool(left || right))
            }
            (operator, left, right) => Err(unsupported(
                format!(
                    "{} between {} and {}",
                    binary_label(operator),
                    value_kind(&left),
                    value_kind(&right)
                ),
                span,
            )),
        }
    }

    /// Read one constant or local place while applying its recorded ownership decision.
    fn evaluate_operand(
        &mut self,
        operand: &Operand,
        span: HirSourceSpan,
    ) -> Result<ReplacementValue, ReplacementExecutionError> {
        match operand {
            Operand::Constant(constant) => Ok(constant_value(constant)),
            Operand::Place(place_operand) => {
                let local = place_operand.place.local;
                self.ownership_reads.push(OwnershipRead {
                    span,
                    fact: place_operand.fact,
                    last_use: place_operand.last_use,
                });
                let value = match place_operand.fact {
                    OwnershipFact::Copy => self
                        .locals
                        .get(&local)
                        .cloned()
                        .ok_or_else(|| runtime_failure("read of an unavailable local".to_string(), span))?,
                    OwnershipFact::Move => self.read_moved_place(&place_operand.place, span)?,
                    OwnershipFact::Clone | OwnershipFact::Borrow | OwnershipFact::MutBorrow => self
                        .locals
                        .get(&local)
                        .cloned()
                        .ok_or_else(|| runtime_failure("read of an unavailable local".to_string(), span))?,
                    OwnershipFact::Unknown => return Err(unsupported("unknown ownership fact", span)),
                };
                let value = self.project_place(value, &place_operand.place, span)?;
                if matches!(place_operand.fact, OwnershipFact::Copy) && !value.is_copy_shaped() {
                    return Err(unsupported("copy of a non-copy or unavailable local", span));
                }
                Ok(value)
            }
        }
    }

    /// Move a complete local while refusing the partial moves Body IR v0 does not model for collection tuples.
    fn read_moved_place(
        &mut self,
        place: &Place,
        span: HirSourceSpan,
    ) -> Result<ReplacementValue, ReplacementExecutionError> {
        if !place.projection.is_empty() {
            return Err(unsupported(
                "move through a tuple field projection outside the scalar tuple collection profile",
                span,
            ));
        }
        self.locals
            .remove(&place.local)
            .ok_or_else(|| runtime_failure("read of a moved or dropped local".to_string(), span))
    }

    /// Apply the one scalar-tuple field projection or collected-generator list index admitted by this profile.
    fn project_place(
        &mut self,
        value: ReplacementValue,
        place: &Place,
        span: HirSourceSpan,
    ) -> Result<ReplacementValue, ReplacementExecutionError> {
        match place.projection.as_slice() {
            [] | [PlaceElem::Field(_)] => project_tuple_field(value, place, span),
            [PlaceElem::Index(index)] => {
                let index = self.evaluate_operand(index, span)?;
                let ReplacementValue::Int(index) = index else {
                    return Err(unsupported("collected generator index using a non-int value", span));
                };
                let index = usize::try_from(index)
                    .map_err(|_| runtime_failure("collected generator index is negative".to_string(), span))?;
                match value {
                    ReplacementValue::CollectedGenerator { elements, .. } => elements
                        .get(index)
                        .cloned()
                        .ok_or_else(|| runtime_failure("collected generator index is out of range".to_string(), span)),
                    value => Err(unsupported(
                        format!(
                            "indexing {} outside the generator-expression collect profile",
                            value_kind(&value)
                        ),
                        span,
                    )),
                }
            }
            _ => Err(unsupported(
                "place projection outside the scalar tuple collection profile",
                span,
            )),
        }
    }

    /// Record one executed statement and enforce the bounded-profile step limit.
    fn record_step(&mut self, span: HirSourceSpan) -> Result<(), ReplacementExecutionError> {
        self.steps = self.steps.saturating_add(1);
        if self.steps > MAX_EXECUTION_STEPS {
            return Err(runtime_failure(
                format!("replacement profile exceeded the {MAX_EXECUTION_STEPS}-step execution limit"),
                span,
            ));
        }
        Ok(())
    }
}

impl ReplacementValue {
    /// Return whether a value can honor a Body-IR `Copy` read without duplicating owned state.
    const fn is_copy_shaped(&self) -> bool {
        matches!(self, Self::Int(_) | Self::Bool(_) | Self::Unit)
    }

    /// Return whether this value is one of the selected, source-observable scalar tuple element shapes.
    const fn is_collection_scalar(&self) -> bool {
        matches!(self, Self::Int(_) | Self::Bool(_) | Self::Str(_) | Self::Unit)
    }

    /// Return whether this value is an admitted two-scalar tuple stored in a replacement collection.
    fn is_scalar_pair_tuple(&self) -> bool {
        matches!(self, Self::Tuple(elements) if elements.len() == 2 && elements.iter().all(Self::is_collection_scalar))
    }

    /// Return this value as a boolean, refusing a type-shape mismatch at the original source location.
    fn into_bool(self, span: HirSourceSpan) -> Result<bool, ReplacementExecutionError> {
        match self {
            Self::Bool(value) => Ok(value),
            value => Err(unsupported(
                format!("boolean condition using {}", value_kind(&value)),
                span,
            )),
        }
    }

    /// Return this value as an owned string, refusing an incompatible helper call at the original source location.
    fn into_string(self, span: HirSourceSpan) -> Result<String, ReplacementExecutionError> {
        match self {
            Self::Str(value) => Ok(value),
            value => Err(unsupported(
                format!("string operation using {}", value_kind(&value)),
                span,
            )),
        }
    }
}

/// Control flow propagated between normalized blocks.
enum Flow {
    /// Ordinary fallthrough.
    Next,
    /// Break from the innermost normalized loop.
    Break,
    /// Continue the innermost normalized loop.
    Continue,
    /// Return from the selected free function.
    Return(Option<ReplacementValue>),
}

/// Return one bare local id, refusing fields, indexes, and slices in the first profile.
fn bare_local(place: &Place, span: HirSourceSpan) -> Result<LocalId, ReplacementExecutionError> {
    if place.projection.is_empty() {
        Ok(place.local)
    } else {
        Err(unsupported("place projection", span))
    }
}

/// Project one admitted scalar tuple field while retaining the statement's original source authority on refusal.
fn project_tuple_field(
    value: ReplacementValue,
    place: &Place,
    span: HirSourceSpan,
) -> Result<ReplacementValue, ReplacementExecutionError> {
    let [PlaceElem::Field(field)] = place.projection.as_slice() else {
        return if place.projection.is_empty() {
            Ok(value)
        } else {
            Err(unsupported(
                "place projection outside the scalar tuple collection profile",
                span,
            ))
        };
    };
    let index = match field.as_str() {
        "0" => 0,
        "1" => 1,
        _ => {
            return Err(unsupported(
                "non-numeric tuple field projection outside the scalar tuple collection profile",
                span,
            ));
        }
    };
    match value {
        ReplacementValue::Tuple(elements)
            if elements.len() == 2 && elements.iter().all(ReplacementValue::is_collection_scalar) =>
        {
            elements.into_iter().nth(index).ok_or_else(|| {
                unsupported(
                    "tuple field projection outside the scalar tuple collection profile",
                    span,
                )
            })
        }
        value => Err(unsupported(
            format!(
                "tuple field projection `.{}` using {} outside the scalar tuple collection profile",
                field,
                value_kind(&value)
            ),
            span,
        )),
    }
}

/// Construct a source-span-preserving unsupported-profile error.
fn unsupported(description: impl Into<String>, span: HirSourceSpan) -> ReplacementExecutionError {
    ReplacementExecutionError::Unsupported {
        description: description.into(),
        span,
        span_start: span.start,
        span_end: span.end,
    }
}

/// Construct a source-span-preserving runtime-failure error.
fn runtime_failure(detail: String, span: HirSourceSpan) -> ReplacementExecutionError {
    ReplacementExecutionError::RuntimeFailure {
        detail,
        span,
        span_start: span.start,
        span_end: span.end,
    }
}

/// Apply Python integer floor division while keeping an unrepresentable direct-execution quotient visible.
fn checked_python_floor_division(
    left: i64,
    right: i64,
    span: HirSourceSpan,
) -> Result<ReplacementValue, ReplacementExecutionError> {
    if left == i64::MIN && right == -1 {
        return Err(runtime_failure("integer division overflow".to_string(), span));
    }
    Ok(ReplacementValue::Int(python_floor_div_i64(left, right)))
}

/// Reject a return value that would widen the first replacement profile beyond scalar source observables.
fn ensure_scalar_result(value: &ReplacementValue, span: HirSourceSpan) -> Result<(), ReplacementExecutionError> {
    match value {
        ReplacementValue::Int(_) | ReplacementValue::Bool(_) | ReplacementValue::Str(_) | ReplacementValue::Unit => {
            Ok(())
        }
        _ => Err(unsupported(
            format!("returning {} from the scalar replacement profile", value_kind(value)),
            span,
        )),
    }
}

/// Project ownership reads into the stable evidence shape shared by identities and CLI reports.
fn ownership_read_projection(reads: &[OwnershipRead]) -> Vec<OwnershipReadProjection> {
    reads
        .iter()
        .map(|read| OwnershipReadProjection {
            span_start: read.span.start,
            span_end: read.span.end,
            fact: ownership_label(read.fact),
            last_use: read.last_use,
        })
        .collect()
}

/// Project Body-IR runtime requirements into stable semantic labels shared by identities and CLI reports.
fn runtime_requirement_projection(requirements: &[AbiV0RuntimeRequirement]) -> Vec<RuntimeRequirementProjection> {
    requirements
        .iter()
        .map(|requirement| RuntimeRequirementProjection {
            requirement: runtime_requirement_label(requirement),
        })
        .collect()
}

/// Render ownership evidence as one deterministic digest component without relying on `Debug` formatting.
fn canonical_ownership_summary(reads: &[OwnershipRead]) -> String {
    ownership_read_projection(reads)
        .into_iter()
        .map(|read| {
            format!(
                "span={}..{};fact={};last_use={}",
                read.span_start, read.span_end, read.fact, read.last_use
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

/// Render runtime requirements as one deterministic digest component without relying on `Debug` formatting.
fn canonical_runtime_requirements_summary(requirements: &[AbiV0RuntimeRequirement]) -> String {
    runtime_requirement_projection(requirements)
        .into_iter()
        .map(|requirement| requirement.requirement)
        .collect::<Vec<_>>()
        .join("|")
}

/// Render one runtime requirement with the semantic labels used in replacement evidence.
fn runtime_requirement_label(requirement: &AbiV0RuntimeRequirement) -> String {
    match requirement {
        AbiV0RuntimeRequirement::RuntimeHelper(name) => format!("runtime_helper({name})"),
        AbiV0RuntimeRequirement::HostedStd => "hosted_std".to_string(),
        AbiV0RuntimeRequirement::Allocator => "allocator".to_string(),
        AbiV0RuntimeRequirement::PanicStrategy => "panic_strategy".to_string(),
        AbiV0RuntimeRequirement::AsyncRuntime => "async_runtime".to_string(),
    }
}

/// Render one ownership fact without relying on generated-Rust implementation details.
const fn ownership_label(fact: OwnershipFact) -> &'static str {
    match fact {
        OwnershipFact::Copy => "copy",
        OwnershipFact::Move => "move",
        OwnershipFact::Clone => "clone",
        OwnershipFact::Borrow => "borrow",
        OwnershipFact::MutBorrow => "mut_borrow",
        OwnershipFact::Unknown => "unknown",
    }
}

/// Render a narrow unsupported-call label for diagnostics.
fn callee_label(callee: &Callee) -> String {
    match callee {
        Callee::Function(name) => format!("function `{name}`"),
        Callee::Method(name) => format!("method `{name}`"),
        Callee::Helper(helper) => format!("runtime helper `{}`", helper_label(*helper)),
    }
}

/// Render a compiler-owned helper name without depending on generated-Rust spellings.
const fn helper_label(helper: HelperOp) -> &'static str {
    match helper {
        HelperOp::StrConcat => "str_concat",
        HelperOp::StrEq => "str_eq",
        HelperOp::StrNe => "str_ne",
        HelperOp::StrLt => "str_lt",
        HelperOp::StrLe => "str_le",
        HelperOp::StrGt => "str_gt",
        HelperOp::StrGe => "str_ge",
        HelperOp::ListConcat => "list_concat",
    }
}

/// Render an aggregate kind as a compact source-level diagnostic label.
fn aggregate_label(kind: &incan_semantics_core::body_ir::AggregateKind) -> &'static str {
    match kind {
        incan_semantics_core::body_ir::AggregateKind::Tuple => "tuple",
        incan_semantics_core::body_ir::AggregateKind::List => "list",
        incan_semantics_core::body_ir::AggregateKind::Dict => "dict",
        incan_semantics_core::body_ir::AggregateKind::Set => "set",
        incan_semantics_core::body_ir::AggregateKind::Constructor(_) => "constructor",
    }
}

/// Register the bounded scalar/control profile beside the replacement executor that implements it.
///
/// The compatibility collector reports this contribution but does not own its feature definitions. In particular,
/// successful direct execution remains non-green until aggregate source contracts have their own paired comparison
/// evidence; the single `replacement-body-v0-001` match stays case-scoped.
pub(crate) fn replacement_compatibility_direct_execution_contribution()
-> crate::replacement_compatibility::ReplacementCompatibilityContribution {
    use crate::replacement_compatibility::{
        feature_requirement_link, implementation_requirement, local_implementation_contribution,
        preserved_feature_at_boundary,
    };

    local_implementation_contribution(
        "backend.replacement.bounded-scalar-control",
        "src/backend/replacement/mod.rs",
        "fn replacement_compatibility_direct_execution_contribution",
        vec![
            preserved_feature_at_boundary(
                "language.control-flow",
                "Bounded scalar conditionals, loops, returns, assertions, and range iteration execute directly with explicit receipts.",
                "src/frontend/typechecker/check_expr/control_flow.rs",
                "fn check_if_expr",
                "fn lower_if",
                "fn execute_loop",
            ),
            preserved_feature_at_boundary(
                "language.numeric-and-scalar",
                "Bounded scalar arithmetic, comparisons, boolean operators, and strings execute directly from Body IR.",
                "src/frontend/typechecker/check_expr/ops.rs",
                "fn check_binary",
                "fn lower_binary",
                "fn evaluate_binary",
            ),
        ],
        vec![
            implementation_requirement(
                "control.normalized-flow",
                "Branches, loops, returns, assertions, and breaks execute from normalized Body IR.",
                "Body IR lowering and replacement evaluator",
                "replacement-body-v0 corpus",
                "Normalized control nodes are implementation vocabulary.",
            ),
            implementation_requirement(
                "runtime.scalar-values",
                "Scalars, strings, operators, and conversions preserve checked type and failure behavior.",
                "Body IR operands/rvalues and replacement evaluator",
                "replacement-body-v0 scalar corpus",
                "Scalar representation is an internal evaluator mechanism.",
            ),
        ],
        Vec::new(),
        vec![
            feature_requirement_link("language.control-flow", "control.normalized-flow"),
            feature_requirement_link("language.numeric-and-scalar", "runtime.scalar-values"),
        ],
    )
}

/// Render a unary operator as a compact source-level diagnostic label.
const fn unary_label(operator: UnOp) -> &'static str {
    match operator {
        UnOp::Neg => "negation",
        UnOp::Not => "boolean negation",
        UnOp::Invert => "bitwise inversion",
    }
}

/// Render a binary operator as a compact source-level diagnostic label.
const fn binary_label(operator: BinOp) -> &'static str {
    match operator {
        BinOp::Add => "addition",
        BinOp::Sub => "subtraction",
        BinOp::Mul => "multiplication",
        BinOp::Div => "division",
        BinOp::FloorDiv => "floor division",
        BinOp::Mod => "modulo",
        BinOp::Eq => "equality comparison",
        BinOp::Ne => "inequality comparison",
        BinOp::Lt => "less-than comparison",
        BinOp::Le => "less-or-equal comparison",
        BinOp::Gt => "greater-than comparison",
        BinOp::Ge => "greater-or-equal comparison",
        BinOp::And => "boolean conjunction",
        BinOp::Or => "boolean disjunction",
    }
}

/// Render one replacement value's dynamic shape for an unsupported-operation diagnostic.
const fn value_kind(value: &ReplacementValue) -> &'static str {
    match value {
        ReplacementValue::Int(_) => "int",
        ReplacementValue::Bool(_) => "bool",
        ReplacementValue::Str(_) => "str",
        ReplacementValue::Float(_) => "float",
        ReplacementValue::Unit => "unit",
        ReplacementValue::Range { .. } => "range",
        ReplacementValue::List { .. } => "list",
        ReplacementValue::Tuple(_) => "tuple",
        ReplacementValue::Generator(_) => "generator",
        ReplacementValue::CollectedGenerator { .. } => "collected generator list",
    }
}

/// Convert one Body-IR literal into a first-profile replacement value.
fn constant_value(constant: &Constant) -> ReplacementValue {
    match constant {
        Constant::Int(value) => ReplacementValue::Int(*value),
        Constant::Bool(value) => ReplacementValue::Bool(*value),
        Constant::Str(value) => ReplacementValue::Str(value.clone()),
        Constant::Unit | Constant::None => ReplacementValue::Unit,
        Constant::Float(value) => ReplacementValue::Float(value.clone()),
    }
}
