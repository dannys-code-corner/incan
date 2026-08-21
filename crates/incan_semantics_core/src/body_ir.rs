//! Incan Body IR v0 data model.
//!
//! Body IR v0 is the backend-facing, target-agnostic representation of one function/method body. It sits between
//! typechecked source (AST + [`crate::SemanticFactStore`] / declaration-level [`crate::HirModule`]) and the
//! target-specific backend lowering under `src/backend/ir/` — it must be consumable by a replacement backend without
//! that backend needing to read generated-Rust semantics or private compiler internals.
//!
//! Body IR v0 deliberately models a **normalized** statement/control-flow vocabulary rather than a flattened
//! basic-block CFG: `while`/`for` desugar into a single canonical `Loop` + conditional `Break` shape during
//! lowering, and every place-read carries its own [`OwnershipFact`] plus [`bool`] last-use marker inline, instead of
//! relying on a separate borrow-checker-shaped analysis pass. This keeps the model close to what a v0 slice can
//! compute and verify deterministically, while leaving full CFG flattening, precise per-path drop dataflow, and a
//! committed panic strategy to later, more optimizer-shaped work (explicitly out of scope for #653).
//!
//! Unsupported source constructs (comprehensions, match, closures, generators, f-strings, tuple unpacking, and so
//! on) lower to an explicit [`StatementKind::Unsupported`] node rather than panicking or being silently dropped, so
//! the model stays total over real programs while being honest about v0's coverage.

use std::fmt::Write as _;

use crate::{AbiV0RuntimeRequirement, CompilerNodeId, HirSourceSpan, IncanType};

// ============================================================================
// Module / body containers
// ============================================================================

/// One module's worth of lowered function/method bodies.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyIrModule {
    /// Identity of the owning module, matching [`crate::HirModule::id`].
    pub module_id: CompilerNodeId,
    /// One [`Body`] per lowered function/method declaration in the module.
    pub bodies: Vec<Body>,
}

impl BodyIrModule {
    /// Render a deterministic maintainer-facing snapshot of every body in the module.
    pub fn render_snapshot(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(&mut out, "body_ir_module {}", self.module_id);
        for body in &self.bodies {
            out.push_str(&body.render_snapshot());
        }
        out
    }
}

/// Body IR v0 for a single function or method.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    /// Identity of the owning declaration, matching the [`crate::HirDeclaration::id`] this body was lowered from.
    pub decl_id: CompilerNodeId,
    /// Source-level function/method name.
    pub name: String,
    /// Full source span of the declaration this body was lowered from.
    pub span: HirSourceSpan,
    /// Every local (parameter, user binding, or compiler-introduced temporary) declared in this body, in
    /// declaration order. Referenced elsewhere by [`LocalId`] index.
    pub locals: Vec<LocalDecl>,
    /// Locals bound to this body's parameters, in parameter order.
    pub param_locals: Vec<LocalId>,
    /// Source scope tree for this body, used to map statements/locals back to lexical scopes for diagnostics.
    pub scopes: Vec<ScopeInfo>,
    /// Normalized statement sequence for the body, rooted at the function's top-level block.
    pub block: Block,
    /// Runtime/helper requirements this body imposes, deduplicated and kept in first-seen (source-order) sequence
    /// for deterministic snapshots — [`crate::types::AbiV0RuntimeRequirement`] does not derive `Ord`, so lowering
    /// relies on deterministic traversal order rather than sorting. Lets a later target profile decide
    /// hosted-only / alloc-requiring / panic-requiring / potentially freestanding-compatible without inferring
    /// facts from generated Rust helper calls.
    pub runtime_requirements: Vec<AbiV0RuntimeRequirement>,
    /// Panic-interaction facts recorded for this body, without committing to a stable public panic strategy.
    pub panic_facts: Vec<PanicFact>,
}

impl Body {
    /// Return the locals in this body whose type is not [`crate::types::AbiV0Ownership::CopyOrTrivial`] and are
    /// therefore drop-relevant if a panic unwinds through this body.
    ///
    /// This is a **conservative over-approximation**: it returns every non-Copy local in the body rather than the
    /// precise set still live at a specific panic site, because computing the precise per-site set needs full
    /// control-flow dataflow that is out of scope for v0 (see the module-level docs). Callers that need a stable
    /// panic strategy must not treat this as a final drop plan; it only exposes that such locals exist.
    pub fn locals_requiring_unwind_drop(&self) -> Vec<LocalId> {
        self.locals
            .iter()
            .filter(|local| !local.ty.abi_v0_facts().ownership.is_trivially_copy())
            .map(|local| local.id)
            .collect()
    }

    /// Render a deterministic maintainer-facing snapshot of this body.
    pub fn render_snapshot(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            &mut out,
            "body {} {} span={}..{}",
            self.name, self.decl_id, self.span.start, self.span.end
        );
        for local in &self.locals {
            let _ = writeln!(&mut out, "  {}", local.render_snapshot());
        }
        render_block(&mut out, &self.block, 1);
        if !self.runtime_requirements.is_empty() {
            let _ = writeln!(&mut out, "  runtime_requirements:");
            for req in &self.runtime_requirements {
                let _ = writeln!(&mut out, "    {}", render_runtime_requirement(req));
            }
        }
        if !self.panic_facts.is_empty() {
            let _ = writeln!(&mut out, "  panic_facts:");
            for fact in &self.panic_facts {
                let _ = writeln!(&mut out, "    {}", fact.render_snapshot());
            }
        }
        out
    }
}

/// Render one [`AbiV0RuntimeRequirement`] using a stable, deterministic spelling.
///
/// [`AbiV0RuntimeRequirement`] does not derive `Display`, so Body IR renders it locally rather than depending on
/// `{:?}` output remaining stable across unrelated changes to that enum's derive list.
fn render_runtime_requirement(req: &AbiV0RuntimeRequirement) -> String {
    match req {
        AbiV0RuntimeRequirement::RuntimeHelper(name) => format!("runtime_helper({name})"),
        AbiV0RuntimeRequirement::HostedStd => "hosted_std".to_string(),
        AbiV0RuntimeRequirement::Allocator => "allocator".to_string(),
        AbiV0RuntimeRequirement::PanicStrategy => "panic_strategy".to_string(),
    }
}

// ============================================================================
// Locals, scopes, places
// ============================================================================

/// Stable index of one local within a [`Body`]'s `locals` vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalId(pub u32);

impl LocalId {
    /// Look up this local's index for direct `locals[..]` access.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// One explicit local or temporary value declared in a body.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalDecl {
    pub id: LocalId,
    /// Source-level binding name, or `None` for a compiler-introduced temporary.
    pub name: Option<String>,
    pub ty: IncanType,
    pub origin: LocalOrigin,
    /// Lexical scope this local is declared in.
    pub scope: ScopeId,
    pub span: HirSourceSpan,
}

impl LocalDecl {
    /// Render a deterministic maintainer-facing snapshot line for this local.
    fn render_snapshot(&self) -> String {
        let name = self.name.as_deref().unwrap_or("<tmp>");
        format!(
            "local {} {} : {} [{}] scope={} span={}..{}",
            self.id.0,
            name,
            self.ty,
            self.origin.as_str(),
            self.scope.0,
            self.span.start,
            self.span.end
        )
    }
}

/// Where a local came from, for diagnostics and drop-planning purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalOrigin {
    /// Bound to a function/method parameter.
    Parameter,
    /// Bound by a source-level assignment (`x = ...`, `let x = ...`, `mut x = ...`).
    UserBinding,
    /// Introduced by lowering to hold an intermediate value (e.g. flattening a nested call or binary expression).
    Temporary,
    /// A name lowering could not resolve to a parameter or local binding within this body (for example a
    /// module-level `const`/`static`, or a reference lowering does not yet track). Modeled as an opaque local with
    /// [`OwnershipFact::Unknown`](crate::body_ir::OwnershipFact::Unknown) reads rather than silently treated as a
    /// resolved local, per #653's "explicit unknowns" requirement.
    External,
}

impl LocalOrigin {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Parameter => "param",
            Self::UserBinding => "binding",
            Self::Temporary => "temp",
            Self::External => "external",
        }
    }
}

/// Stable index of one lexical scope within a [`Body`]'s `scopes` vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopeId(pub u32);

/// One lexical source scope, used to map statements and locals back to diagnostics-relevant source structure.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeInfo {
    pub id: ScopeId,
    /// Enclosing scope, or `None` for the body's root scope.
    pub parent: Option<ScopeId>,
    pub span: HirSourceSpan,
}

/// A place in memory: a local plus zero or more projections (field/index) into it.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub local: LocalId,
    pub projection: Vec<PlaceElem>,
}

impl Place {
    /// Build a bare place referring to a local with no projection.
    pub const fn from_local(local: LocalId) -> Self {
        Self {
            local,
            projection: Vec::new(),
        }
    }

    /// Render a deterministic maintainer-facing spelling for this place.
    fn render_snapshot(&self) -> String {
        let mut out = format!("_{}", self.local.0);
        for elem in &self.projection {
            match elem {
                PlaceElem::Field(name) => {
                    let _ = write!(&mut out, ".{name}");
                }
                PlaceElem::Index(operand) => {
                    let _ = write!(&mut out, "[{}]", operand.render_snapshot());
                }
            }
        }
        out
    }
}

/// One projection step applied to a place.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaceElem {
    /// `.field` access.
    Field(String),
    /// `[index]` access. Boxed because the index itself is an arbitrary operand.
    Index(Box<Operand>),
}

// ============================================================================
// Operands and ownership facts
// ============================================================================

/// A value used as input to an [`Rvalue`], call argument, branch condition, or return value.
///
/// Every place-read carries its own [`OwnershipFact`] and last-use marker directly (see [`PlaceOperand`]) — this is
/// the Duckborrower fact for that read, exposed as compiler-owned data rather than left for a backend to re-derive
/// from generated Rust.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    /// A read of a place, annotated with its ownership decision.
    Place(PlaceOperand),
    /// A literal constant value.
    Constant(Constant),
}

impl Operand {
    /// Build a place-read operand with an explicit ownership fact and last-use marker.
    pub const fn place(place: Place, fact: OwnershipFact, last_use: bool) -> Self {
        Self::Place(PlaceOperand { place, fact, last_use })
    }

    /// Render a deterministic maintainer-facing spelling for this operand.
    fn render_snapshot(&self) -> String {
        match self {
            Self::Place(op) => format!(
                "{}({}{})",
                op.fact.as_str(),
                op.place.render_snapshot(),
                if op.last_use { ", last_use" } else { "" }
            ),
            Self::Constant(c) => c.render_snapshot(),
        }
    }
}

/// A place-read operand paired with its Duckborrower ownership fact.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceOperand {
    pub place: Place,
    /// The ownership decision selected for this read.
    pub fact: OwnershipFact,
    /// Whether this is statically the last read of `place`'s local within its declaring scope. `fact` is only ever
    /// `Move` when this is `true` for a non-Copy type; the flag is retained separately (rather than folded
    /// implicitly into `fact`) because #653 names last-use as its own explicit fact, independent of which decision
    /// it produced.
    pub last_use: bool,
}

/// A literal constant operand value.
#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Int(i64),
    Float(String),
    Bool(bool),
    Str(String),
    Unit,
    None,
}

impl Constant {
    /// Render a deterministic maintainer-facing spelling for this constant.
    fn render_snapshot(&self) -> String {
        match self {
            Self::Int(v) => format!("const({v})"),
            Self::Float(v) => format!("const({v})"),
            Self::Bool(v) => format!("const({v})"),
            Self::Str(v) => format!("const({v:?})"),
            Self::Unit => "const(())".to_string(),
            Self::None => "const(none)".to_string(),
        }
    }
}

/// Duckborrower ownership decision for one place-read.
///
/// This refines [`crate::types::AbiV0Ownership`] (which only distinguishes "trivially copy" from "owned") with the
/// move/clone split that a real Rust emission boundary needs, plus an explicit [`OwnershipFact::Unknown`] escape
/// hatch for reads Body IR v0 cannot yet classify — per #653, ownership decisions must be represented "as
/// Duckborrower facts or explicit unknowns," not silently defaulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipFact {
    /// Trivial bitwise copy of a `Copy`-shaped type.
    Copy,
    /// Ownership of the place is transferred out (its last use, non-Copy type).
    Move,
    /// The place is cloned because it is read again later and its type is not trivially copyable.
    Clone,
    /// A shared borrow of the place is taken without transferring or duplicating ownership.
    Borrow,
    /// A mutable borrow of the place is taken.
    MutBorrow,
    /// Ownership could not yet be classified by v0's lowering (explicit unknown, not a silent default).
    Unknown,
}

impl OwnershipFact {
    /// Compact snapshot spelling for this fact.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "move",
            Self::Clone => "clone",
            Self::Borrow => "borrow",
            Self::MutBorrow => "mut_borrow",
            Self::Unknown => "unknown",
        }
    }
}

// ============================================================================
// Rvalues
// ============================================================================

/// The right-hand side of an [`StatementKind::Assign`].
#[derive(Debug, Clone, PartialEq)]
pub enum Rvalue {
    /// Use an operand's value directly.
    Use(Operand),
    UnaryOp(UnOp, Operand),
    BinaryOp(BinOp, Operand, Operand),
    /// Build a tuple, list, or nominal-constructor value from its element/field operands.
    Aggregate(AggregateKind, Vec<Operand>),
}

impl Rvalue {
    /// Render a deterministic maintainer-facing spelling for this rvalue.
    fn render_snapshot(&self) -> String {
        match self {
            Self::Use(op) => op.render_snapshot(),
            Self::UnaryOp(op, operand) => format!("{}{}", op.as_str(), operand.render_snapshot()),
            Self::BinaryOp(op, lhs, rhs) => {
                format!("{} {} {}", lhs.render_snapshot(), op.as_str(), rhs.render_snapshot())
            }
            Self::Aggregate(kind, operands) => {
                let items: Vec<String> = operands.iter().map(Operand::render_snapshot).collect();
                format!("{}[{}]", kind.as_str(), items.join(", "))
            }
        }
    }
}

/// Unary operator kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Invert,
}

impl UnOp {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Neg => "-",
            Self::Not => "not ",
            Self::Invert => "~",
        }
    }
}

/// Binary operator kind supported by v0 lowering (arithmetic, comparison, boolean).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinOp {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::FloorDiv => "//",
            Self::Mod => "%",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::And => "and",
            Self::Or => "or",
        }
    }

    /// Whether this operator can panic at runtime (division/modulo by a possibly-zero divisor).
    pub const fn may_panic(self) -> bool {
        matches!(self, Self::Div | Self::FloorDiv | Self::Mod)
    }
}

/// Aggregate value shape built by [`Rvalue::Aggregate`].
#[derive(Debug, Clone, PartialEq)]
pub enum AggregateKind {
    Tuple,
    List,
    Constructor(String),
}

impl AggregateKind {
    fn as_str(&self) -> String {
        match self {
            Self::Tuple => "tuple".to_string(),
            Self::List => "list".to_string(),
            Self::Constructor(name) => format!("constructor({name})"),
        }
    }
}

// ============================================================================
// Calls and runtime helpers
// ============================================================================

/// Call target for a [`StatementKind::Call`].
#[derive(Debug, Clone, PartialEq)]
pub enum Callee {
    /// A direct call to a named Incan function, by its source-level spelling.
    ///
    /// Full call-target resolution (which physical declaration binds through imports/traits/overloads) mirrors the
    /// typechecker/backend resolution passes and is deferred past v0; Body IR records the source-level callee
    /// spelling plus argument ownership facts, which is enough to prove the model end-to-end.
    Function(String),
    /// A method call `receiver.method(args)`. `args[0]` in the surrounding [`StatementKind::Call`] is the receiver.
    Method(String),
    /// A compiler-owned runtime/helper operation, represented explicitly instead of as a generated-Rust helper-call
    /// idiom (#653 criterion 3).
    Helper(HelperOp),
}

impl Callee {
    fn render_snapshot(&self) -> String {
        match self {
            Self::Function(name) => format!("fn:{name}"),
            Self::Method(name) => format!("method:{name}"),
            Self::Helper(op) => format!("helper:{}", op.as_str()),
        }
    }
}

/// Compiler-owned runtime/helper operation. These correspond to the stdlib helper calls the existing Rust-emission
/// backend generates for string/list operators (see `src/backend/ir/conversions.rs::determine_binop_plan`), but are
/// represented here as an explicit Body IR operation rather than inferred later from generated Rust call shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperOp {
    StrConcat,
    StrEq,
    StrNe,
    StrLt,
    StrLe,
    StrGt,
    StrGe,
    ListConcat,
}

impl HelperOp {
    /// Compact snapshot spelling for this helper operation, also used as the [`AbiV0RuntimeRequirement::RuntimeHelper`]
    /// name so callers building runtime-requirement facts stay on the same helper naming as the snapshot renderer.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StrConcat => "str_concat",
            Self::StrEq => "str_eq",
            Self::StrNe => "str_ne",
            Self::StrLt => "str_lt",
            Self::StrLe => "str_le",
            Self::StrGt => "str_gt",
            Self::StrGe => "str_ge",
            Self::ListConcat => "list_concat",
        }
    }
}

// ============================================================================
// Statements and blocks
// ============================================================================

/// A normalized block of statements within a body, scoped to one [`ScopeId`].
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub scope: ScopeId,
    pub stmts: Vec<Statement>,
}

/// Render a block's statements at the given indentation depth (in two-space units).
fn render_block(out: &mut String, block: &Block, depth: usize) {
    let indent = "  ".repeat(depth);
    for stmt in &block.stmts {
        render_statement(out, stmt, &indent, depth);
    }
}

/// Render one statement, recursing into nested blocks for control-flow statements.
fn render_statement(out: &mut String, stmt: &Statement, indent: &str, depth: usize) {
    match &stmt.kind {
        StatementKind::Assign { place, rvalue } => {
            let _ = writeln!(
                out,
                "{indent}{} = {}",
                place.render_snapshot(),
                rvalue.render_snapshot()
            );
        }
        StatementKind::Call {
            destination,
            callee,
            args,
            may_panic,
        } => {
            let dest = destination
                .as_ref()
                .map(|p| format!("{} = ", p.render_snapshot()))
                .unwrap_or_default();
            let args_str: Vec<String> = args.iter().map(Operand::render_snapshot).collect();
            let panic_marker = if *may_panic { " may_panic" } else { "" };
            let _ = writeln!(
                out,
                "{indent}{dest}call {}({}){panic_marker}",
                callee.render_snapshot(),
                args_str.join(", ")
            );
        }
        StatementKind::Drop { local } => {
            let _ = writeln!(out, "{indent}drop _{}", local.0);
        }
        StatementKind::If {
            cond,
            then_block,
            else_block,
        } => {
            let _ = writeln!(out, "{indent}if {}:", cond.render_snapshot());
            render_block(out, then_block, depth + 1);
            if let Some(else_block) = else_block {
                let _ = writeln!(out, "{indent}else:");
                render_block(out, else_block, depth + 1);
            }
        }
        StatementKind::Loop { body } => {
            let _ = writeln!(out, "{indent}loop:");
            render_block(out, body, depth + 1);
        }
        StatementKind::Break { value } => {
            let value_str = value.as_ref().map(Operand::render_snapshot).unwrap_or_default();
            let _ = writeln!(out, "{indent}break {value_str}");
        }
        StatementKind::Continue => {
            let _ = writeln!(out, "{indent}continue");
        }
        StatementKind::Return { value } => {
            let value_str = value.as_ref().map(Operand::render_snapshot).unwrap_or_default();
            let _ = writeln!(out, "{indent}return {value_str}");
        }
        StatementKind::Assert {
            cond,
            message,
            may_panic,
        } => {
            let msg = message
                .as_ref()
                .map(|m| format!(", {}", m.render_snapshot()))
                .unwrap_or_default();
            let panic_marker = if *may_panic { " may_panic" } else { "" };
            let _ = writeln!(out, "{indent}assert {}{msg}{panic_marker}", cond.render_snapshot());
        }
        StatementKind::Expr { value } => {
            let _ = writeln!(out, "{indent}expr {}", value.render_snapshot());
        }
        StatementKind::Unsupported { description } => {
            let _ = writeln!(out, "{indent}unsupported({description})");
        }
    }
}

/// One statement in a normalized Body IR block, carrying its own source span for diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub kind: StatementKind,
    pub span: HirSourceSpan,
}

/// Canonical statement vocabulary for Body IR v0.
///
/// `while`/`for` source statements desugar into `Loop` + a conditional `Break` during lowering, rather than being
/// represented as distinct statement kinds, so the canonical vocabulary has a single loop shape.
#[derive(Debug, Clone, PartialEq)]
pub enum StatementKind {
    /// Assign an rvalue into a place.
    Assign { place: Place, rvalue: Rvalue },
    /// Call a function, method, or runtime helper, optionally storing its result.
    Call {
        destination: Option<Place>,
        callee: Callee,
        args: Vec<Operand>,
        /// Whether this call is known to be able to panic (helper operations that check preconditions).
        may_panic: bool,
    },
    /// Explicit drop of a local whose value was never moved out of its declaring scope.
    Drop { local: LocalId },
    /// `if cond: then_block [else: else_block]`.
    If {
        cond: Operand,
        then_block: Block,
        else_block: Option<Block>,
    },
    /// Normalized loop body. `while`/`for` desugar into this plus a leading conditional `Break`.
    Loop { body: Block },
    /// Exit the innermost enclosing `Loop`, optionally producing a value (loop-expression support is deferred; v0
    /// only lowers `break` as a statement, so this is always `None` from lowering today but is modeled for forward
    /// compatibility with loop-expression support).
    Break { value: Option<Operand> },
    /// Skip to the next iteration of the innermost enclosing `Loop`.
    Continue,
    /// Return from the body, optionally with a value.
    Return { value: Option<Operand> },
    /// `assert cond[, message]`.
    Assert {
        cond: Operand,
        message: Option<Operand>,
        may_panic: bool,
    },
    /// An expression evaluated for its side effects only; its value is discarded.
    Expr { value: Operand },
    /// A source construct v0 lowering does not yet model. Keeps the model total over real programs instead of
    /// panicking or silently dropping the construct.
    Unsupported { description: String },
}

// ============================================================================
// Panic facts
// ============================================================================

/// One panic-interaction fact recorded for a body, without committing to a stable public panic strategy. This only
/// exposes *that* a statement may panic and *why* — strategy decisions (unwind vs. abort, drop-on-unwind ordering)
/// are left to later, target-specific work.
#[derive(Debug, Clone, PartialEq)]
pub struct PanicFact {
    pub span: HirSourceSpan,
    pub reason: PanicReason,
}

impl PanicFact {
    fn render_snapshot(&self) -> String {
        format!("{} span={}..{}", self.reason.as_str(), self.span.start, self.span.end)
    }
}

/// Why a statement may panic at runtime.
#[derive(Debug, Clone, PartialEq)]
pub enum PanicReason {
    AssertFailure,
    DivisionOrModulo,
    HelperMayPanic(HelperOp),
}

impl PanicReason {
    fn as_str(&self) -> String {
        match self {
            Self::AssertFailure => "assert_failure".to_string(),
            Self::DivisionOrModulo => "division_or_modulo".to_string(),
            Self::HelperMayPanic(op) => format!("helper_may_panic({})", op.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompilerNodeKind, IncanPrimitiveType};

    fn sample_body() -> Body {
        let decl_id = CompilerNodeId::declaration("m", "add");
        let local_x = LocalId(0);
        let local_y = LocalId(1);
        let local_tmp = LocalId(2);
        Body {
            decl_id: decl_id.clone(),
            name: "add".to_string(),
            span: HirSourceSpan::new(0, 30),
            locals: vec![
                LocalDecl {
                    id: local_x,
                    name: Some("x".to_string()),
                    ty: IncanType::Primitive(IncanPrimitiveType::Int),
                    origin: LocalOrigin::Parameter,
                    scope: ScopeId(0),
                    span: HirSourceSpan::new(4, 5),
                },
                LocalDecl {
                    id: local_y,
                    name: Some("y".to_string()),
                    ty: IncanType::Primitive(IncanPrimitiveType::Int),
                    origin: LocalOrigin::Parameter,
                    scope: ScopeId(0),
                    span: HirSourceSpan::new(7, 8),
                },
                LocalDecl {
                    id: local_tmp,
                    name: None,
                    ty: IncanType::Primitive(IncanPrimitiveType::Int),
                    origin: LocalOrigin::Temporary,
                    scope: ScopeId(0),
                    span: HirSourceSpan::new(20, 25),
                },
            ],
            param_locals: vec![local_x, local_y],
            scopes: vec![ScopeInfo {
                id: ScopeId(0),
                parent: None,
                span: HirSourceSpan::new(0, 30),
            }],
            block: Block {
                scope: ScopeId(0),
                stmts: vec![
                    Statement {
                        kind: StatementKind::Assign {
                            place: Place::from_local(local_tmp),
                            rvalue: Rvalue::BinaryOp(
                                BinOp::Add,
                                Operand::place(Place::from_local(local_x), OwnershipFact::Copy, false),
                                Operand::place(Place::from_local(local_y), OwnershipFact::Copy, true),
                            ),
                        },
                        span: HirSourceSpan::new(20, 25),
                    },
                    Statement {
                        kind: StatementKind::Return {
                            value: Some(Operand::place(Place::from_local(local_tmp), OwnershipFact::Move, true)),
                        },
                        span: HirSourceSpan::new(20, 30),
                    },
                ],
            },
            runtime_requirements: Vec::new(),
            panic_facts: Vec::new(),
        }
    }

    #[test]
    fn body_snapshot_is_deterministic() {
        let body = sample_body();
        assert_eq!(body.render_snapshot(), body.render_snapshot());
    }

    #[test]
    fn body_snapshot_renders_locals_and_control_flow() {
        let snapshot = sample_body().render_snapshot();
        assert!(snapshot.contains("body add decl:m::add span=0..30"));
        assert!(snapshot.contains("local 0 x : int [param]"));
        assert!(snapshot.contains("local 2 <tmp> : int [temp]"));
        assert!(snapshot.contains("_2 = copy(_0) + copy(_1, last_use)"));
        assert!(snapshot.contains("return move(_2, last_use)"));
    }

    #[test]
    fn helper_call_and_runtime_requirements_render() {
        let mut body = sample_body();
        body.block.stmts.insert(
            0,
            Statement {
                kind: StatementKind::Call {
                    destination: Some(Place::from_local(LocalId(2))),
                    callee: Callee::Helper(HelperOp::StrConcat),
                    args: vec![Operand::Constant(Constant::Str("a".to_string()))],
                    may_panic: false,
                },
                span: HirSourceSpan::new(0, 1),
            },
        );
        body.runtime_requirements = vec![
            AbiV0RuntimeRequirement::Allocator,
            AbiV0RuntimeRequirement::RuntimeHelper("str_concat".to_string()),
        ];
        body.panic_facts = vec![PanicFact {
            span: HirSourceSpan::new(20, 25),
            reason: PanicReason::DivisionOrModulo,
        }];

        let snapshot = body.render_snapshot();
        assert!(snapshot.contains("call helper:str_concat(const(\"a\"))"));
        assert!(snapshot.contains("runtime_requirements:"));
        assert!(snapshot.contains("allocator"));
        assert!(snapshot.contains("runtime_helper(str_concat)"));
        assert!(snapshot.contains("panic_facts:"));
        assert!(snapshot.contains("division_or_modulo span=20..25"));
    }

    #[test]
    fn locals_requiring_unwind_drop_is_conservative_over_non_copy_locals() {
        let mut body = sample_body();
        body.locals.push(LocalDecl {
            id: LocalId(3),
            name: Some("s".to_string()),
            ty: IncanType::Primitive(IncanPrimitiveType::Str),
            origin: LocalOrigin::UserBinding,
            scope: ScopeId(0),
            span: HirSourceSpan::new(10, 15),
        });

        let drop_relevant = body.locals_requiring_unwind_drop();
        assert_eq!(drop_relevant, vec![LocalId(3)]);
    }

    #[test]
    fn body_ir_module_snapshot_wraps_bodies() {
        let module = BodyIrModule {
            module_id: CompilerNodeId::new(CompilerNodeKind::Module, "m"),
            bodies: vec![sample_body()],
        };
        let snapshot = module.render_snapshot();
        assert!(snapshot.starts_with("body_ir_module module:m\n"));
        assert!(snapshot.contains("body add decl:m::add"));
    }
}
