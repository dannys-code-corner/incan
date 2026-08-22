//! Direct execution of the deliberately narrow #988 Body-IR replacement profile.
//!
//! This module consumes [`incan_semantics_core::BodyIrModule`] directly. It never reads generated Rust, never
//! delegates a requested replacement execution to [`crate::backend::ir`], and rejects every operation outside the
//! first free-function profile with the original Body-IR source span. The profile is intentionally limited to
//! scalar local values, arithmetic, compiler-owned string concatenation, branches, normalized loops, returns, and
//! assertions; packages, Rust interop, callable values, generators, destructuring, and projections remain visible
//! refusals for later #988 extensions.

use std::collections::BTreeMap;

use incan_semantics_core::body_ir::{
    BinOp, Body, BodyIrModule, Callee, Constant, HelperOp, IterProtocol, LocalId, LocalOrigin, Operand, OwnershipFact,
    Place, Rvalue, Statement, StatementKind, UnOp,
};
use incan_semantics_core::{AbiV0RuntimeRequirement, HirSourceSpan};

use crate::backend::selection::digest_output;

/// Bounded instruction count for one replacement execution.
///
/// The first profile deliberately executes normalized loops rather than translating them to native code. Keeping a
/// deterministic bound turns an accidental infinite loop into an explicit unavailable result instead of allowing a
/// test or CLI invocation to hang without a receipt.
const MAX_EXECUTION_STEPS: usize = 100_000;

/// One scalar value supported by the first replacement-execution profile.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// A list retained only long enough to make a subsequent projection refusal attributable to that projection.
    List(Vec<ReplacementValue>),
    /// A normalized builtin range iterator for the selected range-`for` control-flow case.
    Range { next: i64, end: i64, step: i64 },
}

impl ReplacementValue {
    /// Render a deterministic source-observable result spelling for receipts and legacy runtime comparison.
    pub fn observable_text(&self) -> String {
        match self {
            Self::Int(value) => value.to_string(),
            Self::Bool(value) => value.to_string(),
            Self::Str(value) => value.clone(),
            Self::Float(value) => value.clone(),
            Self::Unit => "None".to_string(),
            Self::List(values) => {
                let items = values.iter().map(Self::observable_text).collect::<Vec<_>>();
                format!("[{}]", items.join(", "))
            }
            Self::Range { next, end, step } => format!("range({next}, {end}, {step})"),
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

/// Successful replacement execution evidence for one free function.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Return the original Incan source location when this outcome arose from Body IR.
    pub const fn primary_span(&self) -> Option<HirSourceSpan> {
        match self {
            Self::Unsupported { span, .. } | Self::RuntimeFailure { span, .. } => Some(*span),
            Self::MissingFunction { .. } | Self::ArgumentCount { .. } => None,
        }
    }
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
    let body = module
        .bodies
        .iter()
        .find(|body| body.name == name)
        .ok_or_else(|| ReplacementExecutionError::MissingFunction { name: name.to_string() })?;
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

    let mut executor = BodyExecutor::new(body, args);
    let flow = executor.execute_block(&body.block)?;
    let value = match flow {
        Flow::Return(value) => value.unwrap_or(ReplacementValue::Unit),
        Flow::Next => ReplacementValue::Unit,
        Flow::Break | Flow::Continue => {
            return Err(unsupported("loop control outside a normalized loop", body.span));
        }
    };
    let body_snapshot = body.render_snapshot();
    let ownership_summary = executor
        .ownership_reads
        .iter()
        .map(|read| format!("{}:{}:{}", read.span.start, ownership_label(read.fact), read.last_use))
        .collect::<Vec<_>>()
        .join("|");
    let requirements_summary = format!("{:?}", body.runtime_requirements);
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

/// Mutable interpreter state for one Body-IR execution.
struct BodyExecutor {
    locals: BTreeMap<LocalId, ReplacementValue>,
    local_bindings: BTreeMap<LocalId, (Option<String>, LocalOrigin)>,
    ownership_reads: Vec<OwnershipRead>,
    steps: usize,
}

impl BodyExecutor {
    /// Bind the already-typechecked call arguments to their Body-IR parameter locals.
    fn new(body: &Body, args: &[ReplacementValue]) -> Self {
        let locals = body.param_locals.iter().copied().zip(args.iter().cloned()).collect();
        let local_bindings = body
            .locals
            .iter()
            .map(|local| (local.id, (local.name.clone(), local.origin)))
            .collect();
        Self {
            locals,
            local_bindings,
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
            } => self.execute_range_next(destination, iterator, statement.span),
            StatementKind::Yield { .. } => Err(unsupported("generator yield", statement.span)),
            StatementKind::TryPropagate { .. } => Err(unsupported("try propagation", statement.span)),
            StatementKind::IterNext { .. } => Err(unsupported("non-range iteration", statement.span)),
            StatementKind::Unsupported { description } => Err(unsupported(description, statement.span)),
        }
    }

    /// Evaluate a compiler-owned helper call in the first execution profile.
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
            _ => return Err(unsupported(format!("call to {}", callee_label(callee)), span)),
        };
        self.assign_local(local, value);
        Ok(Flow::Next)
    }

    /// Materialize the builtin `range` call that Body IR retains before its normalized loop.
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

    /// Poll one builtin range iterator and express exhaustion as the Body-IR loop break it represents.
    fn execute_range_next(
        &mut self,
        destination: &Place,
        iterator: &Operand,
        span: HirSourceSpan,
    ) -> Result<Flow, ReplacementExecutionError> {
        let Operand::Place(iterator) = iterator else {
            return Err(unsupported("non-place range iterator", span));
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
                value
            }
            Some(ReplacementValue::Range { .. }) => return Ok(Flow::Break),
            Some(value) => return Err(unsupported(format!("iteration over {}", value_kind(value)), span)),
            None => {
                return Err(runtime_failure(
                    "read of an unavailable range iterator".to_string(),
                    span,
                ));
            }
        };
        self.assign_local(bare_local(destination, span)?, ReplacementValue::Int(value));
        Ok(Flow::Next)
    }

    /// Assign a Body-IR local while retaining the first profile's source-binding equivalence.
    ///
    /// Body IR v0 records source assignment sites as fresh locals even though later condition reads can still point
    /// at the original local. For the selected free-function profile, equal user-binding names denote the same
    /// source binding; update those aliases together rather than allowing a generated local id to create stale
    /// source reads. Shadowing remains unsupported until Body IR carries an explicit binding-equivalence fact.
    fn assign_local(&mut self, local: LocalId, value: ReplacementValue) {
        let Some((Some(name), LocalOrigin::UserBinding)) = self.local_bindings.get(&local) else {
            self.locals.insert(local, value);
            return;
        };
        let aliases = self
            .local_bindings
            .iter()
            .filter_map(|(candidate, (candidate_name, origin))| {
                (matches!(origin, LocalOrigin::UserBinding) && candidate_name.as_deref() == Some(name.as_str()))
                    .then_some(*candidate)
            })
            .collect::<Vec<_>>();
        for alias in aliases {
            self.locals.insert(alias, value.clone());
        }
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
            Rvalue::Aggregate(incan_semantics_core::body_ir::AggregateKind::List, values) => values
                .iter()
                .map(|value| self.evaluate_operand(value, span))
                .collect::<Result<Vec<_>, _>>()
                .map(ReplacementValue::List),
            Rvalue::Aggregate(kind, _) => Err(unsupported(format!("{} aggregate", aggregate_label(kind)), span)),
            Rvalue::Format(_) => Err(unsupported("f-string", span)),
            Rvalue::Closure { .. } => Err(unsupported("callable value", span)),
            Rvalue::Match { .. } => Err(unsupported("match expression", span)),
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
                Ok(ReplacementValue::Int(left.div_euclid(right)))
            }
            (BinOp::Mod, ReplacementValue::Int(left), ReplacementValue::Int(right)) => {
                Ok(ReplacementValue::Int(left.rem_euclid(right)))
            }
            (BinOp::Eq, left, right) => Ok(ReplacementValue::Bool(left == right)),
            (BinOp::Ne, left, right) => Ok(ReplacementValue::Bool(left != right)),
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
                let local = bare_local(&place_operand.place, span)?;
                self.ownership_reads.push(OwnershipRead {
                    span,
                    fact: place_operand.fact,
                    last_use: place_operand.last_use,
                });
                match place_operand.fact {
                    OwnershipFact::Copy => self
                        .locals
                        .get(&local)
                        .cloned()
                        .filter(ReplacementValue::is_copy_shaped)
                        .ok_or_else(|| unsupported("copy of a non-copy or unavailable local", span)),
                    OwnershipFact::Move => self
                        .locals
                        .remove(&local)
                        .ok_or_else(|| runtime_failure("read of a moved or dropped local".to_string(), span)),
                    OwnershipFact::Clone | OwnershipFact::Borrow | OwnershipFact::MutBorrow => self
                        .locals
                        .get(&local)
                        .cloned()
                        .ok_or_else(|| runtime_failure("read of an unavailable local".to_string(), span)),
                    OwnershipFact::Unknown => Err(unsupported("unknown ownership fact", span)),
                }
            }
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
        ReplacementValue::List(_) => "list",
        ReplacementValue::Range { .. } => "range",
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
