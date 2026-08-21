//! Frontend bridge from typechecked AST function bodies into Body IR v0.
//!
//! Declaration-level HIR ([`crate::frontend::hir`]) does not model statements or expressions at all (see its module
//! docs), so Body IR v0 lowers directly from `ast::FunctionDecl` bodies plus [`TypeCheckInfo`], rather than from a
//! hypothetical body-shaped HIR that does not exist yet. Every [`Body`](incan_semantics_core::body_ir::Body) this
//! module produces carries a [`CompilerNodeId`] identical to the one [`crate::frontend::hir::build_hir_v0`] would
//! assign the same function's [`crate::frontend::hir`] declaration, so the two can be correlated by id without
//! threading a [`crate::frontend::hir`] value through this API.
//!
//! Body IR v0 lowers a representative, explicitly documented subset of the language surface (see
//! [`incan_semantics_core::body_ir`] module docs for the full rationale). Statements fully lowered: assignment
//! (inferred/let/mutable/reassignment), `return`, `if`/`elif`/`else`, `while`, `for` over a `start..end` range,
//! expression statements, `assert`, `pass`, `break`, `continue`. Expressions fully lowered: identifiers, literals
//! (int/float/decimal/bool/string), binary/unary operators, calls, method calls, field access, indexing,
//! parenthesization, tuples, list literals (no spreads), and constructors. Everything else lowers to an explicit
//! `Statement::Unsupported` / `Operand::Unknown` node rather than panicking, so the model stays total over real
//! programs.

use std::collections::{HashMap, HashSet};

use incan_semantics_core::body_ir as bir;
use incan_semantics_core::{AbiV0RuntimeRequirement, CompilerNodeId, HirSourceSpan, IncanPrimitiveType, IncanType};

use crate::frontend::ast;
use crate::frontend::typechecker::{TypeCheckInfo, semantic_type_from_resolved};

/// Build Body IR v0 for every top-level function declaration in a typechecked module.
///
/// Methods on models/classes/traits are not lowered by v0 (see module docs); only `ast::Declaration::Function` items
/// produce a [`bir::Body`]. `module_path` uses the same convention as [`crate::frontend::hir::build_hir_v0`], so the
/// [`CompilerNodeId`] this function assigns each body matches the id [`crate::frontend::hir::build_hir_v0`] assigns
/// the corresponding declaration.
pub fn build_body_ir_module_v0(
    program: &ast::Program,
    module_path: &[String],
    type_info: &TypeCheckInfo,
) -> bir::BodyIrModule {
    let module_identity = body_ir_module_identity(module_path);
    let module_id = CompilerNodeId::module(module_identity.clone());
    let bodies = program
        .declarations
        .iter()
        .filter_map(|decl| match &decl.node {
            ast::Declaration::Function(function) => {
                Some(lower_function_body(function, decl.span, &module_identity, type_info))
            }
            _ => None,
        })
        .collect();
    bir::BodyIrModule { module_id, bodies }
}

/// Render a module path into the same module identity spelling [`crate::frontend::hir`] uses, so declaration ids
/// line up between the two representations.
fn body_ir_module_identity(module_path: &[String]) -> String {
    if module_path.is_empty() {
        "<module>".to_string()
    } else {
        module_path.join("::")
    }
}

/// Convert an AST byte-offset span into a Body IR source span.
const fn hir_span(span: ast::Span) -> HirSourceSpan {
    HirSourceSpan::new(span.start, span.end)
}

/// Lower one function declaration's body into Body IR v0.
fn lower_function_body(
    function: &ast::FunctionDecl,
    decl_span: ast::Span,
    module_identity: &str,
    type_info: &TypeCheckInfo,
) -> bir::Body {
    let decl_id = CompilerNodeId::declaration(module_identity, &function.name);
    let binding = type_info.declarations.function_bindings.get(&function.name);

    let mut builder = BodyBuilder::new(type_info);
    let root_scope = builder.new_scope(None, hir_span(decl_span));

    let mut param_locals = Vec::with_capacity(function.params.len());
    for (index, param) in function.params.iter().enumerate() {
        let ty = binding
            .and_then(|b| b.params.get(index))
            .map(|p| semantic_type_from_resolved(&p.ty))
            .unwrap_or(IncanType::Unknown);
        let local = builder.declare_new_local(
            param.node.name.clone(),
            ty,
            root_scope,
            hir_span(param.span),
            &function.body,
        );
        builder.locals[local.index()].origin = bir::LocalOrigin::Parameter;
        param_locals.push(local);
    }

    let mut stmts = Vec::new();
    builder.lower_block_into(&function.body, root_scope, &mut stmts);
    builder.insert_scope_drops(&mut stmts, root_scope);

    if builder
        .locals
        .iter()
        .any(|local| !local.ty.abi_v0_facts().ownership.is_trivially_copy())
    {
        builder.record_runtime_requirement(AbiV0RuntimeRequirement::Allocator);
    }

    bir::Body {
        decl_id,
        name: function.name.clone(),
        span: hir_span(decl_span),
        locals: builder.locals,
        param_locals,
        scopes: builder.scopes,
        block: bir::Block {
            scope: root_scope,
            stmts,
        },
        runtime_requirements: builder.runtime_requirements,
        panic_facts: builder.panic_facts,
    }
}

/// Per-function lowering state: fresh local/scope allocation, current name bindings, and accumulated body-level
/// facts (runtime requirements, panic facts, which locals have been moved out of their declaring scope).
struct BodyBuilder<'a> {
    type_info: &'a TypeCheckInfo,
    locals: Vec<bir::LocalDecl>,
    scopes: Vec<bir::ScopeInfo>,
    /// Current source-name -> local binding. Later bindings of the same name (new `let`/`mut` assignments) shadow
    /// earlier ones, matching the source-level scoping `BindingKind::Inferred`/`Let`/`Mutable` produce.
    bindings: HashMap<String, bir::LocalId>,
    /// Names lowering could not resolve to a tracked local (e.g. module-level `const`/`static`), reused across
    /// repeated reads instead of allocating a fresh external local per read.
    external_locals: HashMap<String, bir::LocalId>,
    /// Remaining textual reads for each tracked (non-temporary) local, seeded at declaration time by counting
    /// `Ident` occurrences of its name in the declaring scope's statement suffix (see [`count_reads_in_stmts`]).
    /// Decremented on every read; a decrement that reaches zero selects [`bir::OwnershipFact::Move`].
    remaining_reads: HashMap<bir::LocalId, usize>,
    /// Locals whose value has been moved out via a full-value (non-projected) read, so scope-exit drop insertion
    /// skips them.
    moved_out: HashSet<bir::LocalId>,
    runtime_requirements: Vec<AbiV0RuntimeRequirement>,
    panic_facts: Vec<bir::PanicFact>,
    next_local: u32,
    next_scope: u32,
}

impl<'a> BodyBuilder<'a> {
    /// Start a fresh builder for one function body, with no locals, scopes, or accumulated facts yet.
    fn new(type_info: &'a TypeCheckInfo) -> Self {
        Self {
            type_info,
            locals: Vec::new(),
            scopes: Vec::new(),
            bindings: HashMap::new(),
            external_locals: HashMap::new(),
            remaining_reads: HashMap::new(),
            moved_out: HashSet::new(),
            runtime_requirements: Vec::new(),
            panic_facts: Vec::new(),
            next_local: 0,
            next_scope: 0,
        }
    }

    // ---- Scopes and locals ----

    /// Allocate a fresh lexical scope with the given `parent`, recording it in `scopes` for later span lookup.
    fn new_scope(&mut self, parent: Option<bir::ScopeId>, span: HirSourceSpan) -> bir::ScopeId {
        let id = bir::ScopeId(self.next_scope);
        self.next_scope += 1;
        self.scopes.push(bir::ScopeInfo { id, parent, span });
        id
    }

    /// Look up the source span recorded for `scope`, or a zero-width span if the id is unknown (defensive default;
    /// every scope this builder hands out is always recorded in `scopes` first).
    fn scope_span(&self, scope: bir::ScopeId) -> HirSourceSpan {
        self.scopes
            .iter()
            .find(|info| info.id == scope)
            .map(|info| info.span)
            .unwrap_or(HirSourceSpan::new(0, 0))
    }

    /// Resolve the expression type recorded by the typechecker for `span`, or [`IncanType::Unknown`] when v0 has no
    /// resolved type available (an explicit unknown rather than a guessed default).
    fn resolve_ty(&self, span: ast::Span) -> IncanType {
        self.type_info
            .expr_type(span)
            .map(semantic_type_from_resolved)
            .unwrap_or(IncanType::Unknown)
    }

    /// Declare a new user-facing local (parameter or source binding), seeding its last-use countdown from the
    /// number of `Ident` reads of `name` found in `remaining` (the declaring block's statement suffix, or a loop
    /// body for per-iteration bindings). Defaults to [`bir::LocalOrigin::UserBinding`]; callers that declare a
    /// parameter overwrite the origin afterward.
    fn declare_new_local(
        &mut self,
        name: String,
        ty: IncanType,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        remaining: &[ast::Spanned<ast::Statement>],
    ) -> bir::LocalId {
        let id = bir::LocalId(self.next_local);
        self.next_local += 1;
        self.locals.push(bir::LocalDecl {
            id,
            name: Some(name.clone()),
            ty,
            origin: bir::LocalOrigin::UserBinding,
            scope,
            span,
        });
        let total_reads = count_reads_in_stmts(&name, remaining);
        self.bindings.insert(name, id);
        self.remaining_reads.insert(id, total_reads);
        id
    }

    /// Allocate a compiler-introduced temporary. Temporaries are always consumed exactly once, immediately after
    /// creation (by construction of the flattening lowering below), so they are excluded from last-use tracking and
    /// scope-exit drop insertion — see [`Self::temp_operand`] and [`Self::insert_scope_drops`].
    fn new_temp(&mut self, ty: IncanType, scope: bir::ScopeId, span: HirSourceSpan) -> bir::LocalId {
        let id = bir::LocalId(self.next_local);
        self.next_local += 1;
        self.locals.push(bir::LocalDecl {
            id,
            name: None,
            ty,
            origin: bir::LocalOrigin::Temporary,
            scope,
            span,
        });
        id
    }

    /// Resolve a source identifier to a local, synthesizing a cached [`bir::LocalOrigin::External`] local for names
    /// v0 cannot bind (module-level `const`/`static`, or anything else lowering does not yet track) instead of
    /// panicking on an unresolved name.
    fn local_for_name(&mut self, name: &str, span: HirSourceSpan) -> bir::LocalId {
        if let Some(&id) = self.bindings.get(name) {
            return id;
        }
        if let Some(&id) = self.external_locals.get(name) {
            return id;
        }
        let id = bir::LocalId(self.next_local);
        self.next_local += 1;
        self.locals.push(bir::LocalDecl {
            id,
            name: Some(name.to_string()),
            ty: IncanType::Unknown,
            origin: bir::LocalOrigin::External,
            scope: bir::ScopeId(0),
            span,
        });
        self.external_locals.insert(name.to_string(), id);
        id
    }

    // ---- Ownership facts ----

    /// Select the Duckborrower fact and last-use marker for reading `place`.
    ///
    /// Projected reads (`.field`, `[index]`) never move: v0 does not track partial-move state, so a non-Copy
    /// projected read always borrows rather than risking an unsound move out of a place the surrounding code still
    /// owns. A bare local read decrements its remaining-reads countdown; reaching zero selects `Move` (and records
    /// the local as moved for [`Self::insert_scope_drops`]), otherwise `Clone`. A local with no tracked countdown
    /// (an [`bir::LocalOrigin::External`] reference) gets the explicit [`bir::OwnershipFact::Unknown`].
    ///
    /// Note that [`count_reads_in_stmts`] counts a `.field`/`[index]` occurrence of a name toward that local's
    /// total the same as a bare occurrence, but only bare reads ever decrement the countdown here. A local read
    /// only through projections therefore never reaches zero and always reads `Clone` on its final bare use rather
    /// than `Move` — an over-seeded, never-decremented countdown biases toward `Clone`, not toward an unsound
    /// `Move`, consistent with this module's documented last-use approximation.
    fn ownership_fact_for_place(&mut self, place: &bir::Place, ty: &IncanType) -> (bir::OwnershipFact, bool) {
        let is_copy = ty.abi_v0_facts().ownership.is_trivially_copy();
        if !place.projection.is_empty() {
            let fact = if is_copy {
                bir::OwnershipFact::Copy
            } else {
                bir::OwnershipFact::Borrow
            };
            return (fact, false);
        }
        if is_copy {
            if let Some(remaining) = self.remaining_reads.get_mut(&place.local) {
                *remaining = remaining.saturating_sub(1);
            }
            return (bir::OwnershipFact::Copy, false);
        }
        let Some(remaining) = self.remaining_reads.get_mut(&place.local) else {
            return (bir::OwnershipFact::Unknown, false);
        };
        *remaining = remaining.saturating_sub(1);
        if *remaining == 0 {
            self.moved_out.insert(place.local);
            (bir::OwnershipFact::Move, true)
        } else {
            (bir::OwnershipFact::Clone, false)
        }
    }

    /// Build the operand for a freshly created temporary's single, immediate use.
    fn temp_operand(&self, local: bir::LocalId, ty: &IncanType) -> bir::Operand {
        let fact = if ty.abi_v0_facts().ownership.is_trivially_copy() {
            bir::OwnershipFact::Copy
        } else {
            bir::OwnershipFact::Move
        };
        bir::Operand::place(bir::Place::from_local(local), fact, true)
    }

    /// Record a runtime/helper requirement for this body, deduplicated and kept in first-seen order (see
    /// [`bir::Body::runtime_requirements`] for why lowering relies on traversal order rather than sorting).
    fn record_runtime_requirement(&mut self, requirement: AbiV0RuntimeRequirement) {
        if !self.runtime_requirements.contains(&requirement) {
            self.runtime_requirements.push(requirement);
        }
    }

    /// Emit explicit `Drop` statements, in reverse declaration order, for every non-Copy `UserBinding`/`Parameter`
    /// local declared directly in `scope` that was never moved out. This is scoped to locals declared *directly* in
    /// this block — it does not attempt cross-branch or early-return/break drop-obligation dataflow, which needs
    /// full control-flow analysis out of scope for v0 (see [`incan_semantics_core::body_ir`] module docs).
    fn insert_scope_drops(&mut self, stmts: &mut Vec<bir::Statement>, scope: bir::ScopeId) {
        let span = self.scope_span(scope);
        let candidates: Vec<bir::LocalId> = self
            .locals
            .iter()
            .rev()
            .filter(|local| local.scope == scope)
            .filter(|local| {
                matches!(
                    local.origin,
                    bir::LocalOrigin::UserBinding | bir::LocalOrigin::Parameter
                )
            })
            .filter(|local| !local.ty.abi_v0_facts().ownership.is_trivially_copy())
            .map(|local| local.id)
            .collect();
        for id in candidates {
            if self.moved_out.contains(&id) {
                continue;
            }
            stmts.push(bir::Statement {
                kind: bir::StatementKind::Drop { local: id },
                span,
            });
        }
    }

    /// Push a [`bir::StatementKind::Unsupported`] statement carrying a short diagnostic `description`, so an
    /// unmodeled source construct still leaves a total, structurally valid statement rather than being dropped.
    fn push_unsupported_stmt(&self, description: String, span: HirSourceSpan, out: &mut Vec<bir::Statement>) {
        out.push(bir::Statement {
            kind: bir::StatementKind::Unsupported { description },
            span,
        });
    }

    /// Emit an `Unsupported` marker statement and return a handle operand for it, so callers evaluating an
    /// unsupported expression in value position still get a structurally valid [`bir::Operand`] to thread onward.
    fn unsupported_operand(
        &mut self,
        description: String,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let temp = self.new_temp(IncanType::Unknown, scope, span);
        self.push_unsupported_stmt(description, span, out);
        bir::Operand::place(bir::Place::from_local(temp), bir::OwnershipFact::Unknown, true)
    }

    // ---- Rvalue / call helpers ----

    /// Allocate a fresh temporary, push an `Assign` statement giving it `rvalue`'s value, and return an operand for
    /// that temporary's single, immediate use (see [`Self::temp_operand`]). The common tail shared by every
    /// expression-lowering path that needs to flatten a computed value into a place before it can be read again.
    fn push_assign_temp(
        &mut self,
        rvalue: bir::Rvalue,
        ty: IncanType,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let temp = self.new_temp(ty.clone(), scope, span);
        out.push(bir::Statement {
            kind: bir::StatementKind::Assign {
                place: bir::Place::from_local(temp),
                rvalue,
            },
            span,
        });
        self.temp_operand(temp, &ty)
    }

    /// Allocate a fresh temporary, push a `Call` statement storing its result there, and return an operand for that
    /// temporary's single, immediate use — the call-lowering counterpart to [`Self::push_assign_temp`].
    #[allow(clippy::too_many_arguments)]
    fn push_call_temp(
        &mut self,
        callee: bir::Callee,
        args: Vec<bir::Operand>,
        ty: IncanType,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        may_panic: bool,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let temp = self.new_temp(ty.clone(), scope, span);
        out.push(bir::Statement {
            kind: bir::StatementKind::Call {
                destination: Some(bir::Place::from_local(temp)),
                callee,
                args,
                may_panic,
            },
            span,
        });
        self.temp_operand(temp, &ty)
    }

    /// Build the boolean negation of `operand` as a fresh temporary (`not operand`), used to turn a loop's
    /// continuation condition into its complementary exit condition for the leading conditional `Break`.
    fn negate_operand(
        &mut self,
        operand: bir::Operand,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        self.push_assign_temp(
            bir::Rvalue::UnaryOp(bir::UnOp::Not, operand),
            IncanType::Primitive(IncanPrimitiveType::Bool),
            scope,
            span,
            out,
        )
    }

    // ---- Statements ----

    /// Lower every statement in `stmts` into `out`, within `scope`. Statements are lowered in source order and each
    /// one is given the statement suffix that follows it (`&stmts[index + 1..]`), so last-use countdowns seeded by
    /// [`Self::declare_new_local`] only count reads that can still occur after the declaration.
    fn lower_block_into(
        &mut self,
        stmts: &[ast::Spanned<ast::Statement>],
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) {
        for (index, stmt) in stmts.iter().enumerate() {
            self.lower_stmt_into(stmt, &stmts[index + 1..], scope, out);
        }
    }

    /// Lower one statement into `out`, dispatching on its AST kind. `remaining` is the statement suffix following
    /// `stmt` in its enclosing block, threaded through to [`Self::lower_assignment`] for last-use seeding. Statement
    /// kinds outside v0's covered subset fall through to an explicit [`Self::push_unsupported_stmt`] rather than
    /// panicking (see this module's module-level docs for the exact covered/uncovered split).
    fn lower_stmt_into(
        &mut self,
        stmt: &ast::Spanned<ast::Statement>,
        remaining: &[ast::Spanned<ast::Statement>],
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) {
        let span = hir_span(stmt.span);
        match &stmt.node {
            ast::Statement::Assignment(assignment) => self.lower_assignment(assignment, remaining, scope, span, out),
            ast::Statement::Return(value) => {
                let value = value.as_ref().map(|v| self.lower_expr_to_operand(v, scope, out));
                out.push(bir::Statement {
                    kind: bir::StatementKind::Return { value },
                    span,
                });
            }
            ast::Statement::If(if_stmt) => self.lower_if(if_stmt, scope, span, out),
            ast::Statement::While(while_stmt) => self.lower_while(while_stmt, scope, span, out),
            ast::Statement::For(for_stmt) => self.lower_for(for_stmt, scope, span, out),
            ast::Statement::Expr(expr) => {
                let value = self.lower_expr_to_operand(expr, scope, out);
                out.push(bir::Statement {
                    kind: bir::StatementKind::Expr { value },
                    span,
                });
            }
            ast::Statement::Assert(assert_stmt) => self.lower_assert(assert_stmt, scope, span, out),
            ast::Statement::Pass => {}
            ast::Statement::Break(value) => {
                let value = value.as_ref().map(|v| self.lower_expr_to_operand(v, scope, out));
                out.push(bir::Statement {
                    kind: bir::StatementKind::Break { value },
                    span,
                });
            }
            ast::Statement::Continue => out.push(bir::Statement {
                kind: bir::StatementKind::Continue,
                span,
            }),
            other => self.push_unsupported_stmt(unsupported_stmt_label(other), span, out),
        }
    }

    /// Lower an inferred/`let`/`mutable`/reassignment statement. A `Reassign` binding reuses the existing local for
    /// `assignment.name` when one is already bound (falling back to declaring a new one if reassignment targets an
    /// unbound name), while every other binding kind always declares a fresh local — matching source-level shadowing
    /// semantics, where a repeated `let x = ...` introduces a new binding rather than mutating the old one.
    fn lower_assignment(
        &mut self,
        assignment: &ast::AssignmentStmt,
        remaining: &[ast::Spanned<ast::Statement>],
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let ty = self.resolve_ty(assignment.value.span);
        let value = self.lower_expr_to_operand(&assignment.value, scope, out);
        let local = match assignment.binding {
            ast::BindingKind::Reassign => self
                .bindings
                .get(&assignment.name)
                .copied()
                .unwrap_or_else(|| self.declare_new_local(assignment.name.clone(), ty, scope, span, remaining)),
            ast::BindingKind::Inferred | ast::BindingKind::Let | ast::BindingKind::Mutable => {
                self.declare_new_local(assignment.name.clone(), ty, scope, span, remaining)
            }
        };
        out.push(bir::Statement {
            kind: bir::StatementKind::Assign {
                place: bir::Place::from_local(local),
                rvalue: bir::Rvalue::Use(value),
            },
            span,
        });
    }

    /// Lower `if`/`elif`/`else` into a [`bir::StatementKind::If`] chain. `elif` branches are folded into nested
    /// `else { if ... }` wrappers from the last branch inward (see the inline comment above the fold loop), and an
    /// `if let` pattern condition — not yet modeled by v0 — lowers to an explicit unsupported placeholder instead of
    /// the real branch.
    fn lower_if(
        &mut self,
        if_stmt: &ast::IfStmt,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let ast::Condition::Expr(cond_expr) = &if_stmt.condition else {
            self.push_unsupported_stmt("if-let pattern condition".to_string(), span, out);
            return;
        };
        let cond = self.lower_expr_to_operand(cond_expr, scope, out);

        let then_scope = self.new_scope(Some(scope), span);
        let mut then_stmts = Vec::new();
        self.lower_block_into(&if_stmt.then_body, then_scope, &mut then_stmts);
        self.insert_scope_drops(&mut then_stmts, then_scope);
        let then_block = bir::Block {
            scope: then_scope,
            stmts: then_stmts,
        };

        let mut else_block = if let Some(else_body) = &if_stmt.else_body {
            let else_scope = self.new_scope(Some(scope), span);
            let mut else_stmts = Vec::new();
            self.lower_block_into(else_body, else_scope, &mut else_stmts);
            self.insert_scope_drops(&mut else_stmts, else_scope);
            Some(bir::Block {
                scope: else_scope,
                stmts: else_stmts,
            })
        } else {
            None
        };

        // Fold `elif` branches into nested `else { if ... }` wrappers, innermost (last elif) first, so the earlier
        // conditions end up evaluated first at the top of the chain once wrapped by the outer `if` pushed below.
        for (elif_cond, elif_body) in if_stmt.elif_branches.iter().rev() {
            let body_scope = self.new_scope(Some(scope), span);
            let mut wrapper = Vec::new();
            let cond_operand = self.lower_expr_to_operand(elif_cond, scope, &mut wrapper);
            let mut body_stmts = Vec::new();
            self.lower_block_into(elif_body, body_scope, &mut body_stmts);
            self.insert_scope_drops(&mut body_stmts, body_scope);
            wrapper.push(bir::Statement {
                kind: bir::StatementKind::If {
                    cond: cond_operand,
                    then_block: bir::Block {
                        scope: body_scope,
                        stmts: body_stmts,
                    },
                    else_block,
                },
                span,
            });
            else_block = Some(bir::Block { scope, stmts: wrapper });
        }

        out.push(bir::Statement {
            kind: bir::StatementKind::If {
                cond,
                then_block,
                else_block,
            },
            span,
        });
    }

    /// Lower `while cond: body` into Body IR's single normalized loop shape: a [`bir::StatementKind::Loop`] whose
    /// body opens with `if not cond: break`, followed by the lowered loop body. A `while let` pattern condition —
    /// not yet modeled by v0 — lowers to an explicit unsupported placeholder instead of the real loop.
    fn lower_while(
        &mut self,
        while_stmt: &ast::WhileStmt,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let ast::Condition::Expr(cond_expr) = &while_stmt.condition else {
            self.push_unsupported_stmt("while-let pattern condition".to_string(), span, out);
            return;
        };

        let loop_scope = self.new_scope(Some(scope), span);
        let mut body_stmts = Vec::new();
        let cond_operand = self.lower_expr_to_operand(cond_expr, loop_scope, &mut body_stmts);
        let negated = self.negate_operand(cond_operand, loop_scope, span, &mut body_stmts);
        let break_scope = self.new_scope(Some(loop_scope), span);
        let break_block = bir::Block {
            scope: break_scope,
            stmts: vec![bir::Statement {
                kind: bir::StatementKind::Break { value: None },
                span,
            }],
        };
        body_stmts.push(bir::Statement {
            kind: bir::StatementKind::If {
                cond: negated,
                then_block: break_block,
                else_block: None,
            },
            span,
        });

        self.lower_block_into(&while_stmt.body, loop_scope, &mut body_stmts);
        self.insert_scope_drops(&mut body_stmts, loop_scope);

        out.push(bir::Statement {
            kind: bir::StatementKind::Loop {
                body: bir::Block {
                    scope: loop_scope,
                    stmts: body_stmts,
                },
            },
            span,
        });
    }

    /// Lower `for x in start..end: body` into a normalized counting `Loop`. Only range-shaped iterables are
    /// supported in v0 (see module docs); any other iterable or a non-binding loop pattern lowers to
    /// `Unsupported`, since general iterator-protocol lowering needs call-target resolution machinery out of scope
    /// here.
    fn lower_for(
        &mut self,
        for_stmt: &ast::ForStmt,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let ast::Pattern::Binding(var_name) = &for_stmt.pattern.node else {
            self.push_unsupported_stmt("for-loop pattern is not a simple binding".to_string(), span, out);
            return;
        };
        let ast::Expr::Range { start, end, inclusive } = &for_stmt.iter.node else {
            self.push_unsupported_stmt("for-loop over a non-range iterable".to_string(), span, out);
            return;
        };

        let int_ty = IncanType::Primitive(IncanPrimitiveType::Int);
        let start_operand = self.lower_expr_to_operand(start, scope, out);
        let idx_local = self.new_temp(int_ty.clone(), scope, span);
        out.push(bir::Statement {
            kind: bir::StatementKind::Assign {
                place: bir::Place::from_local(idx_local),
                rvalue: bir::Rvalue::Use(start_operand),
            },
            span,
        });

        let loop_scope = self.new_scope(Some(scope), span);
        let mut body_stmts = Vec::new();

        let end_operand = self.lower_expr_to_operand(end, loop_scope, &mut body_stmts);
        let idx_read = bir::Operand::place(bir::Place::from_local(idx_local), bir::OwnershipFact::Copy, false);
        let cmp_op = if *inclusive { bir::BinOp::Gt } else { bir::BinOp::Ge };
        let cond = self.push_assign_temp(
            bir::Rvalue::BinaryOp(cmp_op, idx_read, end_operand),
            IncanType::Primitive(IncanPrimitiveType::Bool),
            loop_scope,
            span,
            &mut body_stmts,
        );
        let break_scope = self.new_scope(Some(loop_scope), span);
        body_stmts.push(bir::Statement {
            kind: bir::StatementKind::If {
                cond,
                then_block: bir::Block {
                    scope: break_scope,
                    stmts: vec![bir::Statement {
                        kind: bir::StatementKind::Break { value: None },
                        span,
                    }],
                },
                else_block: None,
            },
            span,
        });

        let var_ty = self.resolve_ty(for_stmt.pattern.span);
        let iter_local = self.declare_new_local(var_name.clone(), var_ty, loop_scope, span, &for_stmt.body);
        body_stmts.push(bir::Statement {
            kind: bir::StatementKind::Assign {
                place: bir::Place::from_local(iter_local),
                rvalue: bir::Rvalue::Use(bir::Operand::place(
                    bir::Place::from_local(idx_local),
                    bir::OwnershipFact::Copy,
                    false,
                )),
            },
            span,
        });

        self.lower_block_into(&for_stmt.body, loop_scope, &mut body_stmts);
        self.insert_scope_drops(&mut body_stmts, loop_scope);

        let one = bir::Operand::Constant(bir::Constant::Int(1));
        let idx_read_for_incr = bir::Operand::place(bir::Place::from_local(idx_local), bir::OwnershipFact::Copy, false);
        let incremented = self.push_assign_temp(
            bir::Rvalue::BinaryOp(bir::BinOp::Add, idx_read_for_incr, one),
            int_ty,
            loop_scope,
            span,
            &mut body_stmts,
        );
        body_stmts.push(bir::Statement {
            kind: bir::StatementKind::Assign {
                place: bir::Place::from_local(idx_local),
                rvalue: bir::Rvalue::Use(incremented),
            },
            span,
        });

        out.push(bir::Statement {
            kind: bir::StatementKind::Loop {
                body: bir::Block {
                    scope: loop_scope,
                    stmts: body_stmts,
                },
            },
            span,
        });
    }

    /// Lower `assert cond[, message]`, recording an [`bir::PanicReason::AssertFailure`] panic fact and a
    /// [`AbiV0RuntimeRequirement::PanicStrategy`] runtime requirement since every assert can panic. The `raises`
    /// form of `assert` — not yet modeled by v0 — lowers to an explicit unsupported placeholder instead.
    fn lower_assert(
        &mut self,
        assert_stmt: &ast::AssertStmt,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let ast::AssertKind::Condition(cond_expr) = &assert_stmt.kind else {
            self.push_unsupported_stmt("assert pattern/raises form".to_string(), span, out);
            return;
        };
        let cond = self.lower_expr_to_operand(cond_expr, scope, out);
        let message = assert_stmt
            .message
            .as_ref()
            .map(|m| self.lower_expr_to_operand(m, scope, out));
        self.panic_facts.push(bir::PanicFact {
            span,
            reason: bir::PanicReason::AssertFailure,
        });
        self.record_runtime_requirement(AbiV0RuntimeRequirement::PanicStrategy);
        out.push(bir::Statement {
            kind: bir::StatementKind::Assert {
                cond,
                message,
                may_panic: true,
            },
            span,
        });
    }

    // ---- Expressions ----

    /// Lower one expression into an [`bir::Operand`], dispatching on its AST kind and, where evaluation has side
    /// effects or must be flattened (calls, binary/unary ops, aggregates), pushing supporting statements into `out`
    /// first. Expression kinds outside v0's covered subset fall through to [`Self::unsupported_operand`] rather than
    /// panicking (see this module's module-level docs for the exact covered/uncovered split).
    fn lower_expr_to_operand(
        &mut self,
        expr: &ast::Spanned<ast::Expr>,
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let span = hir_span(expr.span);
        match &expr.node {
            ast::Expr::Ident(name) => {
                let place = bir::Place::from_local(self.local_for_name(name, span));
                let ty = self.resolve_ty(expr.span);
                let (fact, last_use) = self.ownership_fact_for_place(&place, &ty);
                bir::Operand::place(place, fact, last_use)
            }
            ast::Expr::SelfExpr => self.unsupported_operand(
                "self (methods are not lowered by Body IR v0)".to_string(),
                scope,
                span,
                out,
            ),
            ast::Expr::Literal(lit) => match lower_literal(lit) {
                Some(constant) => bir::Operand::Constant(constant),
                None => self.unsupported_operand("bytes literal".to_string(), scope, span, out),
            },
            ast::Expr::Paren(inner) => self.lower_expr_to_operand(inner, scope, out),
            ast::Expr::Field(base, name) => {
                let mut place = self.lower_expr_to_place(base, scope, out);
                place.projection.push(bir::PlaceElem::Field(name.clone()));
                let ty = self.resolve_ty(expr.span);
                let (fact, last_use) = self.ownership_fact_for_place(&place, &ty);
                bir::Operand::place(place, fact, last_use)
            }
            ast::Expr::Index(base, index) => {
                let index_operand = self.lower_expr_to_operand(index, scope, out);
                let mut place = self.lower_expr_to_place(base, scope, out);
                place.projection.push(bir::PlaceElem::Index(Box::new(index_operand)));
                let ty = self.resolve_ty(expr.span);
                let (fact, last_use) = self.ownership_fact_for_place(&place, &ty);
                bir::Operand::place(place, fact, last_use)
            }
            ast::Expr::Unary(op, inner) => {
                let un_op = lower_unary_op(*op);
                let operand = self.lower_expr_to_operand(inner, scope, out);
                let ty = self.resolve_ty(expr.span);
                self.push_assign_temp(bir::Rvalue::UnaryOp(un_op, operand), ty, scope, span, out)
            }
            ast::Expr::Binary(lhs, op, rhs) => self.lower_binary(lhs, *op, rhs, expr.span, scope, out),
            ast::Expr::Call(callee, type_args, args) => self.lower_call(callee, type_args, args, expr.span, scope, out),
            ast::Expr::MethodCall(recv, name, type_args, args) => {
                self.lower_method_call(recv, name, type_args, args, expr.span, scope, out)
            }
            ast::Expr::Tuple(items) => self.lower_aggregate(bir::AggregateKind::Tuple, items, expr.span, scope, out),
            ast::Expr::List(entries) => {
                if entries.iter().any(|entry| matches!(entry, ast::ListEntry::Spread(_))) {
                    return self.unsupported_operand("list spread entries".to_string(), scope, span, out);
                }
                let items: Vec<ast::Spanned<ast::Expr>> = entries
                    .iter()
                    .map(|entry| match entry {
                        ast::ListEntry::Element(e) => e.clone(),
                        ast::ListEntry::Spread(_) => unreachable!("spread entries filtered out above"),
                    })
                    .collect();
                self.lower_aggregate(bir::AggregateKind::List, &items, expr.span, scope, out)
            }
            ast::Expr::Constructor(name, args) => self.lower_constructor(name, args, expr.span, scope, out),
            other => self.unsupported_operand(unsupported_expr_label(other), scope, span, out),
        }
    }

    /// Lower an expression that is being used as a place base (the target of `.field`/`[index]` projection or a
    /// bare name), synthesizing a temporary to hold the value when the expression is not itself place-shaped.
    fn lower_expr_to_place(
        &mut self,
        expr: &ast::Spanned<ast::Expr>,
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Place {
        match &expr.node {
            ast::Expr::Ident(name) => bir::Place::from_local(self.local_for_name(name, hir_span(expr.span))),
            ast::Expr::Field(base, name) => {
                let mut place = self.lower_expr_to_place(base, scope, out);
                place.projection.push(bir::PlaceElem::Field(name.clone()));
                place
            }
            ast::Expr::Index(base, index) => {
                let index_operand = self.lower_expr_to_operand(index, scope, out);
                let mut place = self.lower_expr_to_place(base, scope, out);
                place.projection.push(bir::PlaceElem::Index(Box::new(index_operand)));
                place
            }
            ast::Expr::Paren(inner) => self.lower_expr_to_place(inner, scope, out),
            _ => match self.lower_expr_to_operand(expr, scope, out) {
                bir::Operand::Place(place_operand) => place_operand.place,
                constant @ bir::Operand::Constant(_) => {
                    let ty = self.resolve_ty(expr.span);
                    let temp = self.new_temp(ty, scope, hir_span(expr.span));
                    out.push(bir::Statement {
                        kind: bir::StatementKind::Assign {
                            place: bir::Place::from_local(temp),
                            rvalue: bir::Rvalue::Use(constant),
                        },
                        span: hir_span(expr.span),
                    });
                    bir::Place::from_local(temp)
                }
            },
        }
    }

    /// Lower a binary-operator expression. When both operands are string-like and the operator has a compiler-owned
    /// string helper (see [`string_helper_for_binop`]), the operation is emitted as an explicit
    /// [`bir::Callee::Helper`] call with the matching runtime requirements recorded, rather than as a
    /// [`bir::Rvalue::BinaryOp`] — this is Body IR's compiler-owned-runtime-operation requirement (#653 criterion
    /// 3) applied to string operators specifically. Otherwise the operator lowers to a plain `BinaryOp` rvalue, with
    /// a division/modulo panic fact recorded when [`bir::BinOp::may_panic`] holds. An operator with neither a string
    /// helper nor a direct Body IR equivalent (see [`lower_binary_op`]) lowers to an explicit unsupported
    /// placeholder.
    fn lower_binary(
        &mut self,
        lhs: &ast::Spanned<ast::Expr>,
        op: ast::BinaryOp,
        rhs: &ast::Spanned<ast::Expr>,
        span: ast::Span,
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let hir_span_value = hir_span(span);
        let lhs_ty = self.resolve_ty(lhs.span);
        let rhs_ty = self.resolve_ty(rhs.span);
        let result_ty = self.resolve_ty(span);

        if is_string_like(&lhs_ty)
            && is_string_like(&rhs_ty)
            && let Some(helper) = string_helper_for_binop(op)
        {
            let lhs_operand = self.lower_expr_to_operand(lhs, scope, out);
            let rhs_operand = self.lower_expr_to_operand(rhs, scope, out);
            self.record_runtime_requirement(AbiV0RuntimeRequirement::RuntimeHelper(helper.as_str().to_string()));
            self.record_runtime_requirement(AbiV0RuntimeRequirement::Allocator);
            return self.push_call_temp(
                bir::Callee::Helper(helper),
                vec![lhs_operand, rhs_operand],
                result_ty,
                scope,
                hir_span_value,
                false,
                out,
            );
        }

        let Some(bin_op) = lower_binary_op(op) else {
            return self.unsupported_operand(format!("binary operator {op:?}"), scope, hir_span_value, out);
        };
        let lhs_operand = self.lower_expr_to_operand(lhs, scope, out);
        let rhs_operand = self.lower_expr_to_operand(rhs, scope, out);
        if bin_op.may_panic() {
            self.panic_facts.push(bir::PanicFact {
                span: hir_span_value,
                reason: bir::PanicReason::DivisionOrModulo,
            });
            self.record_runtime_requirement(AbiV0RuntimeRequirement::PanicStrategy);
        }
        self.push_assign_temp(
            bir::Rvalue::BinaryOp(bin_op, lhs_operand, rhs_operand),
            result_ty,
            scope,
            hir_span_value,
            out,
        )
    }

    /// Lower every argument in `args` to an operand, or return `None` if any argument is named or an unpack
    /// (`*args`/`**kwargs`) — v0's [`bir::Callee`] call shape only models positional arguments, so callers use the
    /// `None` case to fall back to an explicit unsupported placeholder instead of dropping the extra arguments.
    fn lower_positional_args(
        &mut self,
        args: &[ast::CallArg],
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) -> Option<Vec<bir::Operand>> {
        let mut operands = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                ast::CallArg::Positional(expr) => operands.push(self.lower_expr_to_operand(expr, scope, out)),
                ast::CallArg::Named(_, _) | ast::CallArg::PositionalUnpack(_) | ast::CallArg::KeywordUnpack(_) => {
                    return None;
                }
            }
        }
        Some(operands)
    }

    /// Lower a direct call `name(args)` to a [`bir::Callee::Function`] call. An indirect callee (anything other than
    /// a bare identifier), explicit type arguments, or non-positional arguments each lower to an explicit
    /// unsupported placeholder instead — v0 defers full call-target resolution (see [`bir::Callee::Function`]'s own
    /// docs) and does not model generic call-site type arguments.
    fn lower_call(
        &mut self,
        callee: &ast::Spanned<ast::Expr>,
        type_args: &[ast::Spanned<ast::Type>],
        args: &[ast::CallArg],
        span: ast::Span,
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let hir_span_value = hir_span(span);
        let ast::Expr::Ident(name) = &callee.node else {
            return self.unsupported_operand("indirect call target".to_string(), scope, hir_span_value, out);
        };
        if !type_args.is_empty() {
            return self.unsupported_operand(
                "call with explicit type arguments".to_string(),
                scope,
                hir_span_value,
                out,
            );
        }
        let name = name.clone();
        let Some(operands) = self.lower_positional_args(args, scope, out) else {
            return self.unsupported_operand(
                "call with named or unpack arguments".to_string(),
                scope,
                hir_span_value,
                out,
            );
        };
        let ty = self.resolve_ty(span);
        self.push_call_temp(
            bir::Callee::Function(name),
            operands,
            ty,
            scope,
            hir_span_value,
            false,
            out,
        )
    }

    /// Lower a method call `recv.name(args)` to a [`bir::Callee::Method`] call, with the receiver prepended to
    /// `args[0]` as a [`bir::OwnershipFact::Borrow`] operand (see the inline comment on the receiver-borrow decision
    /// below). Explicit type arguments or non-positional arguments each lower to an explicit unsupported placeholder
    /// instead, matching [`Self::lower_call`]'s treatment of the same cases.
    #[allow(clippy::too_many_arguments)]
    fn lower_method_call(
        &mut self,
        recv: &ast::Spanned<ast::Expr>,
        name: &str,
        type_args: &[ast::Spanned<ast::Type>],
        args: &[ast::CallArg],
        span: ast::Span,
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let hir_span_value = hir_span(span);
        if !type_args.is_empty() {
            return self.unsupported_operand(
                "method call with explicit type arguments".to_string(),
                scope,
                hir_span_value,
                out,
            );
        }
        let Some(mut arg_operands) = self.lower_positional_args(args, scope, out) else {
            return self.unsupported_operand(
                "method call with named or unpack arguments".to_string(),
                scope,
                hir_span_value,
                out,
            );
        };
        // Method receivers are treated as borrowed rather than moved/cloned, mirroring how the existing Rust-emission
        // backend's ownership planner treats most method receivers (`src/backend/ir/ownership.rs`) — see this
        // module's rustdoc for the full precedent discussion.
        let recv_place = self.lower_expr_to_place(recv, scope, out);
        let mut call_args = Vec::with_capacity(arg_operands.len() + 1);
        call_args.push(bir::Operand::place(recv_place, bir::OwnershipFact::Borrow, false));
        call_args.append(&mut arg_operands);
        let ty = self.resolve_ty(span);
        self.push_call_temp(
            bir::Callee::Method(name.to_string()),
            call_args,
            ty,
            scope,
            hir_span_value,
            false,
            out,
        )
    }

    /// Lower a tuple or (non-spread) list literal to a [`bir::Rvalue::Aggregate`], recording an
    /// [`AbiV0RuntimeRequirement::Allocator`] requirement for lists specifically (list construction always
    /// allocates; tuples do not).
    fn lower_aggregate(
        &mut self,
        kind: bir::AggregateKind,
        items: &[ast::Spanned<ast::Expr>],
        span: ast::Span,
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let hir_span_value = hir_span(span);
        let operands: Vec<bir::Operand> = items
            .iter()
            .map(|item| self.lower_expr_to_operand(item, scope, out))
            .collect();
        let ty = self.resolve_ty(span);
        if matches!(kind, bir::AggregateKind::List) {
            self.record_runtime_requirement(AbiV0RuntimeRequirement::Allocator);
        }
        self.push_assign_temp(bir::Rvalue::Aggregate(kind, operands), ty, scope, hir_span_value, out)
    }

    /// Lower a nominal constructor call `Name(args)` to a [`bir::Rvalue::Aggregate`] with
    /// [`bir::AggregateKind::Constructor`]. Both positional and named arguments lower by value in call-site order
    /// (v0 does not yet reorder named arguments to match field declaration order); an unpack argument
    /// (`*args`/`**kwargs`) lowers to an explicit unsupported placeholder instead.
    fn lower_constructor(
        &mut self,
        name: &str,
        args: &[ast::CallArg],
        span: ast::Span,
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let hir_span_value = hir_span(span);
        let mut operands = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                ast::CallArg::Positional(expr) | ast::CallArg::Named(_, expr) => {
                    operands.push(self.lower_expr_to_operand(expr, scope, out))
                }
                ast::CallArg::PositionalUnpack(_) | ast::CallArg::KeywordUnpack(_) => {
                    return self.unsupported_operand(
                        "constructor with unpack arguments".to_string(),
                        scope,
                        hir_span_value,
                        out,
                    );
                }
            }
        }
        let ty = self.resolve_ty(span);
        self.push_assign_temp(
            bir::Rvalue::Aggregate(bir::AggregateKind::Constructor(name.to_string()), operands),
            ty,
            scope,
            hir_span_value,
            out,
        )
    }
}

// ============================================================================
// Free helper functions
// ============================================================================

/// Whether a type is string-like enough to route binary operators through the compiler-owned string helpers
/// (mirrors `is_string_like_type` in `src/backend/ir/conversions.rs`, restated here so Body IR does not depend on
/// that Rust-emission-specific module — see this file's module docs).
fn is_string_like(ty: &IncanType) -> bool {
    matches!(
        ty,
        IncanType::Primitive(IncanPrimitiveType::Str | IncanPrimitiveType::FrozenStr)
    )
}

/// Map a string-typed binary operator to its compiler-owned helper operation, or `None` for operators that have no
/// string-specific helper (arithmetic-only operators never reach here because `lower_binary` only checks this for
/// string-like operand types).
fn string_helper_for_binop(op: ast::BinaryOp) -> Option<bir::HelperOp> {
    match op {
        ast::BinaryOp::Add => Some(bir::HelperOp::StrConcat),
        ast::BinaryOp::Eq => Some(bir::HelperOp::StrEq),
        ast::BinaryOp::NotEq => Some(bir::HelperOp::StrNe),
        ast::BinaryOp::Lt => Some(bir::HelperOp::StrLt),
        ast::BinaryOp::LtEq => Some(bir::HelperOp::StrLe),
        ast::BinaryOp::Gt => Some(bir::HelperOp::StrGt),
        ast::BinaryOp::GtEq => Some(bir::HelperOp::StrGe),
        _ => None,
    }
}

/// Map a surface binary operator to Body IR's canonical arithmetic/comparison/boolean operator set, or `None` for
/// operators v0 does not model (`MatMul`, pipes, `in`/`not in`, `is`/`is not`).
fn lower_binary_op(op: ast::BinaryOp) -> Option<bir::BinOp> {
    match op {
        ast::BinaryOp::Add => Some(bir::BinOp::Add),
        ast::BinaryOp::Sub => Some(bir::BinOp::Sub),
        ast::BinaryOp::Mul => Some(bir::BinOp::Mul),
        ast::BinaryOp::Div => Some(bir::BinOp::Div),
        ast::BinaryOp::FloorDiv => Some(bir::BinOp::FloorDiv),
        ast::BinaryOp::Mod => Some(bir::BinOp::Mod),
        ast::BinaryOp::Eq => Some(bir::BinOp::Eq),
        ast::BinaryOp::NotEq => Some(bir::BinOp::Ne),
        ast::BinaryOp::Lt => Some(bir::BinOp::Lt),
        ast::BinaryOp::LtEq => Some(bir::BinOp::Le),
        ast::BinaryOp::Gt => Some(bir::BinOp::Gt),
        ast::BinaryOp::GtEq => Some(bir::BinOp::Ge),
        ast::BinaryOp::And => Some(bir::BinOp::And),
        ast::BinaryOp::Or => Some(bir::BinOp::Or),
        _ => None,
    }
}

/// Map a surface unary operator to Body IR's unary operator set. Exhaustive: all three surface unary operators have
/// a direct Body IR equivalent.
const fn lower_unary_op(op: ast::UnaryOp) -> bir::UnOp {
    match op {
        ast::UnaryOp::Neg => bir::UnOp::Neg,
        ast::UnaryOp::Not => bir::UnOp::Not,
        ast::UnaryOp::Invert => bir::UnOp::Invert,
    }
}

/// Lower a literal to a Body IR constant, or `None` for literal kinds v0 does not model distinctly (`bytes`).
fn lower_literal(lit: &ast::Literal) -> Option<bir::Constant> {
    match lit {
        ast::Literal::Int(int_lit) => Some(bir::Constant::Int(int_lit.value)),
        ast::Literal::Float(float_lit) => Some(bir::Constant::Float(float_lit.repr.clone())),
        ast::Literal::Decimal(decimal_lit) => Some(bir::Constant::Float(decimal_lit.repr.clone())),
        ast::Literal::String(s) => Some(bir::Constant::Str(s.clone())),
        ast::Literal::Bool(b) => Some(bir::Constant::Bool(*b)),
        ast::Literal::None => Some(bir::Constant::None),
        ast::Literal::Bytes(_) => None,
    }
}

/// Short diagnostic label for a statement kind v0 does not lower.
fn unsupported_stmt_label(stmt: &ast::Statement) -> String {
    match stmt {
        ast::Statement::FieldAssignment(_) => "field assignment".to_string(),
        ast::Statement::IndexAssignment(_) => "index assignment".to_string(),
        ast::Statement::Unsafe(_) => "unsafe block".to_string(),
        ast::Statement::VocabExpressionItem(_) => "vocab expression item".to_string(),
        ast::Statement::CompoundAssignment(_) => "compound assignment".to_string(),
        ast::Statement::TupleUnpack(_) => "tuple unpack".to_string(),
        ast::Statement::TupleAssign(_) => "tuple assignment".to_string(),
        ast::Statement::ChainedAssignment(_) => "chained assignment".to_string(),
        ast::Statement::Surface(_) => "surface statement".to_string(),
        ast::Statement::VocabBlock(_) => "vocab block".to_string(),
        _ => "statement".to_string(),
    }
}

/// Short diagnostic label for an expression kind v0 does not lower.
fn unsupported_expr_label(expr: &ast::Expr) -> String {
    match expr {
        ast::Expr::Slice(..) => "slice expression".to_string(),
        ast::Expr::Partial(_) => "partial callable preset".to_string(),
        ast::Expr::Try(_) => "try (`?`) expression".to_string(),
        ast::Expr::Match(..) => "match expression".to_string(),
        ast::Expr::If(_) => "if expression".to_string(),
        ast::Expr::Loop(_) => "loop expression".to_string(),
        ast::Expr::ListComp(_) => "list comprehension".to_string(),
        ast::Expr::DictComp(_) => "dict comprehension".to_string(),
        ast::Expr::Generator(_) => "generator expression".to_string(),
        ast::Expr::Closure(..) => "closure".to_string(),
        ast::Expr::Dict(_) => "dict literal".to_string(),
        ast::Expr::Set(_) => "set literal".to_string(),
        ast::Expr::FString(_) => "f-string".to_string(),
        ast::Expr::Yield(_) => "yield expression".to_string(),
        ast::Expr::Range { .. } => "range expression outside a for-loop".to_string(),
        _ => "expression".to_string(),
    }
}

/// Count textual `Ident(name)` occurrences reachable from `stmts`, restricted to the same statement/expression
/// subset [`BodyBuilder`] actually lowers. This seeds a local's last-use countdown (see
/// [`BodyBuilder::declare_new_local`]).
///
/// This is a **textual, source-order over-approximation**, not dynamic dataflow: it does not special-case shadowing
/// (a later redeclaration of the same name still contributes to this count) and it counts occurrences across all
/// branches of a conditional rather than only the branch that will execute. Both simplifications only ever make the
/// count too high, which biases the resulting ownership fact toward `Clone`/`Borrow` instead of `Move` — never the
/// reverse — so it cannot produce an unsound move.
fn count_reads_in_stmts(name: &str, stmts: &[ast::Spanned<ast::Statement>]) -> usize {
    stmts.iter().map(|stmt| count_reads_in_stmt(name, &stmt.node)).sum()
}

/// Count `name` occurrences reachable from one statement, recursing into every branch of a conditional/loop rather
/// than only the branch that will execute — part of [`count_reads_in_stmts`]'s documented over-approximation.
/// Statement kinds outside v0's lowered subset are not walked and contribute zero (they cannot themselves bind or
/// read `name` in a way v0's lowering will ever observe).
fn count_reads_in_stmt(name: &str, stmt: &ast::Statement) -> usize {
    match stmt {
        ast::Statement::Assignment(a) => count_reads_in_expr(name, &a.value.node),
        ast::Statement::Return(Some(e)) => count_reads_in_expr(name, &e.node),
        ast::Statement::Return(None) => 0,
        ast::Statement::If(if_stmt) => {
            let mut total = count_reads_in_condition(name, &if_stmt.condition);
            total += count_reads_in_stmts(name, &if_stmt.then_body);
            for (cond, body) in &if_stmt.elif_branches {
                total += count_reads_in_expr(name, &cond.node);
                total += count_reads_in_stmts(name, body);
            }
            if let Some(else_body) = &if_stmt.else_body {
                total += count_reads_in_stmts(name, else_body);
            }
            total
        }
        ast::Statement::While(w) => count_reads_in_condition(name, &w.condition) + count_reads_in_stmts(name, &w.body),
        ast::Statement::For(f) => count_reads_in_expr(name, &f.iter.node) + count_reads_in_stmts(name, &f.body),
        ast::Statement::Expr(e) => count_reads_in_expr(name, &e.node),
        ast::Statement::Assert(a) => {
            let mut total = match &a.kind {
                ast::AssertKind::Condition(e) => count_reads_in_expr(name, &e.node),
                _ => 0,
            };
            total += a
                .message
                .as_ref()
                .map(|m| count_reads_in_expr(name, &m.node))
                .unwrap_or(0);
            total
        }
        ast::Statement::Break(Some(e)) => count_reads_in_expr(name, &e.node),
        _ => 0,
    }
}

/// Count `name` occurrences in an `if`/`while` condition, including the value expression of a `Condition::Let`
/// pattern condition (even though v0 lowering does not model `if let`/`while let` themselves — see
/// [`BodyBuilder::lower_if`]/[`BodyBuilder::lower_while`] — so the read-count approximation stays an
/// over-approximation rather than silently under-counting).
fn count_reads_in_condition(name: &str, cond: &ast::Condition) -> usize {
    match cond {
        ast::Condition::Expr(e) => count_reads_in_expr(name, &e.node),
        ast::Condition::Let { value, .. } => count_reads_in_expr(name, &value.node),
    }
}

/// Count `name` occurrences reachable from one expression, recursing into every expression kind v0's lowering
/// itself walks (see this module's module-level docs for the covered subset). Expression kinds outside that subset
/// contribute zero, consistent with [`count_reads_in_stmts`]'s "restricted to the same subset `BodyBuilder` actually
/// lowers" scope.
fn count_reads_in_expr(name: &str, expr: &ast::Expr) -> usize {
    match expr {
        ast::Expr::Ident(id) => usize::from(id == name),
        ast::Expr::Binary(l, _, r) => count_reads_in_expr(name, &l.node) + count_reads_in_expr(name, &r.node),
        ast::Expr::Unary(_, e) => count_reads_in_expr(name, &e.node),
        ast::Expr::Call(callee, _, args) => {
            count_reads_in_expr(name, &callee.node)
                + args.iter().map(|a| count_reads_in_call_arg(name, a)).sum::<usize>()
        }
        ast::Expr::MethodCall(recv, _, _, args) => {
            count_reads_in_expr(name, &recv.node) + args.iter().map(|a| count_reads_in_call_arg(name, a)).sum::<usize>()
        }
        ast::Expr::Field(e, _) => count_reads_in_expr(name, &e.node),
        ast::Expr::Index(e, idx) => count_reads_in_expr(name, &e.node) + count_reads_in_expr(name, &idx.node),
        ast::Expr::Paren(e) => count_reads_in_expr(name, &e.node),
        ast::Expr::Tuple(items) | ast::Expr::Set(items) => {
            items.iter().map(|i| count_reads_in_expr(name, &i.node)).sum()
        }
        ast::Expr::List(entries) => entries
            .iter()
            .map(|entry| match entry {
                ast::ListEntry::Element(e) | ast::ListEntry::Spread(e) => count_reads_in_expr(name, &e.node),
            })
            .sum(),
        ast::Expr::Constructor(_, args) => args.iter().map(|a| count_reads_in_call_arg(name, a)).sum(),
        ast::Expr::Range { start, end, .. } => {
            count_reads_in_expr(name, &start.node) + count_reads_in_expr(name, &end.node)
        }
        _ => 0,
    }
}

/// Count `name` occurrences in one call argument's expression, regardless of whether the argument is positional,
/// named, or an unpack — the read-count approximation counts the expression either way even though
/// [`BodyBuilder::lower_positional_args`] itself rejects named/unpack arguments during real lowering.
fn count_reads_in_call_arg(name: &str, arg: &ast::CallArg) -> usize {
    match arg {
        ast::CallArg::Positional(e)
        | ast::CallArg::Named(_, e)
        | ast::CallArg::PositionalUnpack(e)
        | ast::CallArg::KeywordUnpack(e) => count_reads_in_expr(name, &e.node),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::typechecker::TypeChecker;
    use crate::frontend::{lexer, parser};

    fn build(source: &str, module_path: &[&str]) -> Result<bir::BodyIrModule, Box<dyn std::error::Error>> {
        let tokens = lexer::lex(source).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        let program = parser::parse(&tokens).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        let module_path: Vec<String> = module_path.iter().map(|s| s.to_string()).collect();
        let mut checker = TypeChecker::new();
        checker.set_current_module_path(Some(module_path.clone()));
        checker
            .check_program(&program)
            .map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        Ok(build_body_ir_module_v0(&program, &module_path, checker.type_info()))
    }

    #[test]
    fn lowers_arithmetic_with_a_copy_last_use_and_a_move_return() -> Result<(), Box<dyn std::error::Error>> {
        let source = "def add(x: int, y: int) -> int:\n  return x + y\n";
        let module = build(source, &["m", "arith"])?;
        let snapshot_first = module.render_snapshot();
        let snapshot_second = build(source, &["m", "arith"])?.render_snapshot();
        assert_eq!(snapshot_first, snapshot_second, "lowering must be deterministic");

        assert!(snapshot_first.contains("body add decl:m::arith::add"));
        assert!(snapshot_first.contains("local 0 x : int [param]"));
        assert!(snapshot_first.contains("local 1 y : int [param]"));
        // x is not the last read (y is), so x is Copy either way (int is a Copy type); both reads should be `copy`.
        assert!(snapshot_first.contains("copy(_0)"));
        assert!(snapshot_first.contains("copy(_1"));
        // `int` is a Copy-shaped type, so even a freshly created temporary reads as `copy`, not `move`.
        assert!(snapshot_first.contains("return copy(_2, last_use)"));
        Ok(())
    }

    #[test]
    fn lowers_string_concat_as_an_explicit_helper_call_with_runtime_requirements()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = "def greet(name: str) -> str:\n  return \"hi \" + name\n";
        let module = build(source, &["m", "strs"])?;
        let snapshot = module.render_snapshot();

        assert!(snapshot.contains("call helper:str_concat"));
        assert!(snapshot.contains("runtime_requirements:"));
        assert!(snapshot.contains("runtime_helper(str_concat)"));
        assert!(snapshot.contains("allocator"));
        Ok(())
    }

    #[test]
    fn lowers_a_non_copy_binding_and_drops_it_when_never_moved() -> Result<(), Box<dyn std::error::Error>> {
        let source = "def make() -> None:\n  s = \"hello\"\n  return\n";
        let module = build(source, &["m", "drop"])?;
        let snapshot = module.render_snapshot();

        assert!(snapshot.contains("local 0 s : str [binding]"));
        assert!(snapshot.contains("drop _0"));
        Ok(())
    }

    #[test]
    fn lowers_a_non_copy_binding_and_skips_the_drop_when_moved_via_return() -> Result<(), Box<dyn std::error::Error>> {
        let source = "def make() -> str:\n  s = \"hello\"\n  return s\n";
        let module = build(source, &["m", "moved"])?;
        let snapshot = module.render_snapshot();

        assert!(snapshot.contains("return move(_0, last_use)"));
        assert!(
            !snapshot.contains("drop _0"),
            "a moved-out local must not also be dropped: {snapshot}"
        );
        Ok(())
    }

    #[test]
    fn lowers_a_clone_when_a_non_copy_binding_is_read_more_than_once() -> Result<(), Box<dyn std::error::Error>> {
        let source = "def dup(s: str) -> str:\n  first = s\n  return s\n";
        let module = build(source, &["m", "clone"])?;
        let snapshot = module.render_snapshot();

        assert!(
            snapshot.contains("clone(_0)"),
            "the first, non-last read of `s` should clone: {snapshot}"
        );
        assert!(snapshot.contains("return move(_0, last_use)"));
        Ok(())
    }

    #[test]
    fn lowers_if_while_and_for_into_normalized_control_flow() -> Result<(), Box<dyn std::error::Error>> {
        let source = "def run(n: int) -> int:\n  total = 0\n  for i in 0..n:\n    if i > 2:\n      total = total + i\n  while total > 100:\n    total = total - 1\n  return total\n";
        let module = build(source, &["m", "control"])?;
        let snapshot = module.render_snapshot();

        assert!(
            snapshot.contains("loop:"),
            "for/while should desugar to a normalized loop: {snapshot}"
        );
        assert!(snapshot.contains("if "));
        assert!(snapshot.contains("break"));
        Ok(())
    }

    #[test]
    fn lowers_division_and_assert_as_explicit_panic_facts() -> Result<(), Box<dyn std::error::Error>> {
        // Floor division keeps an `int` result (true division promotes to `float`), so this stays a same-type return.
        let source = "def div(a: int, b: int) -> int:\n  assert b != 0\n  return a // b\n";
        let module = build(source, &["m", "panics"])?;
        let snapshot = module.render_snapshot();

        assert!(snapshot.contains("panic_facts:"));
        assert!(snapshot.contains("assert_failure"));
        assert!(snapshot.contains("division_or_modulo"));
        assert!(snapshot.contains("panic_strategy"));
        Ok(())
    }

    #[test]
    fn unsupported_constructs_lower_to_an_explicit_placeholder_instead_of_panicking()
    -> Result<(), Box<dyn std::error::Error>> {
        // v0 only lowers `for` over a `start..end` range; iterating a list literal is valid Incan but hits the
        // explicit `Unsupported` placeholder rather than being silently dropped or panicking.
        let source = "def pick(x: int) -> int:\n  for i in [1, 2, 3]:\n    return i\n  return x\n";
        let module = build(source, &["m", "unsupported"])?;
        let snapshot = module.render_snapshot();

        assert!(
            snapshot.contains("unsupported("),
            "should record an explicit placeholder rather than panicking: {snapshot}"
        );
        Ok(())
    }
}
