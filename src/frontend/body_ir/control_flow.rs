//! Lowering for conditional and looping control flow, and the iteration protocol behind `for`.

use super::args::*;
use super::primitives::*;
use super::reads::*;
use super::refusals::*;
use super::*;

/// Where one range-shaped `for` loop takes its bounds, step, and inclusivity from.
///
/// The two variants are the two ways the surface can spell a range in iterable position, and they differ only in
/// where those facts live: written into the header, or carried by an already-built range value. Both drive the
/// same normalized counting loop -- see [`BodyBuilder::lower_range_counting_loop`].
enum RangeLoopSource<'ast> {
    /// An inline `start..end` / `start..=end` loop header, still holding its un-lowered bound expressions.
    Header {
        start: &'ast ast::Spanned<ast::Expr>,
        end: &'ast ast::Spanned<ast::Expr>,
        inclusive: bool,
    },
    /// A materialized [`bir::AggregateKind::Range`] value, read back through its own declared fields.
    Value(bir::Place),
}

/// Read a counting loop's index local.
///
/// The index is a compiler-owned `int` temporary that the loop itself both writes and re-reads several times per
/// iteration, so its reads are always plain [`bir::OwnershipFact::Copy`] and never a last use. Going through
/// [`BodyBuilder::ownership_fact_for_place`] would consult a read countdown that was never seeded for a temporary.
fn index_read(idx_local: bir::LocalId) -> bir::Operand {
    bir::Operand::place(bir::Place::from_local(idx_local), bir::OwnershipFact::Copy, false)
}

impl<'type_info, 'source> BodyBuilder<'type_info, 'source> {
    /// Lower `if`/`elif`/`else` into a [`bir::StatementKind::If`] chain. `elif` branches are folded into nested
    /// `else { if ... }` wrappers from the last branch inward (see the inline comment above the fold loop), and an
    /// `if let` pattern condition — not yet modeled by v0 — lowers to an explicit unsupported placeholder instead of
    /// the real branch.
    pub(super) fn lower_if(
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

        let then_block = self.lower_branch_block(&if_stmt.then_body, scope, span);
        let mut else_block = if_stmt
            .else_body
            .as_ref()
            .map(|else_body| self.lower_branch_block(else_body, scope, span));

        // Fold `elif` branches into nested `else { if ... }` wrappers, innermost (last elif) first, so the earlier
        // conditions end up evaluated first at the top of the chain once wrapped by the outer `if` pushed below.
        for (elif_cond, elif_body) in if_stmt.elif_branches.iter().rev() {
            let mut wrapper = Vec::new();
            let cond_operand = self.lower_expr_to_operand(elif_cond, scope, &mut wrapper);
            let then_block = self.lower_branch_block(elif_body, scope, span);
            wrapper.push(bir::Statement {
                kind: bir::StatementKind::If {
                    cond: cond_operand,
                    then_block,
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

    /// Lower one `if`/`elif`/`else` branch body into its own scoped [`bir::Block`]: allocate a child scope, lower
    /// the statements into it, then insert scope-exit drops. Shared by [`Self::lower_if`]'s then/else/elif bodies
    /// and [`Self::lower_if_expr`]'s then/else bodies, since both need exactly this shape.
    pub(super) fn lower_branch_block(
        &mut self,
        body: &[ast::Spanned<ast::Statement>],
        parent_scope: bir::ScopeId,
        span: HirSourceSpan,
    ) -> bir::Block {
        let branch_scope = self.new_scope(Some(parent_scope), span);
        let mut stmts = Vec::new();
        self.lower_block_into(body, branch_scope, &mut stmts);
        self.insert_scope_drops(&mut stmts, branch_scope);
        bir::Block {
            scope: branch_scope,
            stmts,
        }
    }

    /// Lower an expression-position `if` (`ast::Expr::If`) into the same [`bir::StatementKind::If`] shape
    /// statement-position `if` uses (see [`Self::lower_if`]), reusing [`Self::lower_branch_block`] for both
    /// branches. The typechecker gives an expression-position `if` type `Unit` unconditionally (`check_if_expr` in
    /// `src/frontend/typechecker/check_expr/control_flow.rs` discards any branch value and always returns
    /// `ResolvedType::Unit`) -- unlike a `loop` expression, an `if` expression cannot yet produce a value from its
    /// branches, so its Body IR operand is always the `Unit` constant rather than a place read.
    pub(super) fn lower_if_expr(
        &mut self,
        if_expr: &ast::IfExpr,
        scope: bir::ScopeId,
        span: ast::Span,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let hir_span_value = hir_span(span);
        let cond = self.lower_expr_to_operand(&if_expr.condition, scope, out);
        let then_block = self.lower_branch_block(&if_expr.then_body, scope, hir_span_value);
        let else_block = if_expr
            .else_body
            .as_ref()
            .map(|body| self.lower_branch_block(body, scope, hir_span_value));
        out.push(bir::Statement {
            kind: bir::StatementKind::If {
                cond,
                then_block,
                else_block,
            },
            span: hir_span_value,
        });
        bir::Operand::Constant(bir::Constant::Unit)
    }

    /// Lower a value-producing `loop:` expression (`ast::Expr::Loop`) into a [`bir::StatementKind::Loop`] plus a
    /// dedicated result local that every `break value` inside the loop's *own* body (not a nested loop's --
    /// enforced by [`Self::loop_break_targets`]) assigns into before exiting. The typechecker resolves this
    /// expression's type from the union of its `break value` operand types (`check_loop_expr` in
    /// `src/frontend/typechecker/check_expr/control_flow.rs`), so -- unlike an `if` expression, which is always
    /// `Unit` -- a `loop` expression's produced value genuinely comes from its branches and needs this
    /// merge-into-one-place treatment; see [`Self::lower_break`] for the other half of the mechanism.
    pub(super) fn lower_loop_expr(
        &mut self,
        loop_expr: &ast::LoopExpr,
        scope: bir::ScopeId,
        span: ast::Span,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let hir_span_value = hir_span(span);
        let ty = self.resolve_ty(span);
        let loop_scope = self.new_scope(Some(scope), hir_span_value);
        let result_local = self.new_temp(ty.clone(), loop_scope, hir_span_value);

        self.loop_break_targets.push(Some(result_local));
        let mut body_stmts = Vec::new();
        self.lower_block_into(&loop_expr.body, loop_scope, &mut body_stmts);
        self.insert_scope_drops(&mut body_stmts, loop_scope);
        self.loop_break_targets.pop();

        out.push(bir::Statement {
            kind: bir::StatementKind::Loop {
                body: bir::Block {
                    scope: loop_scope,
                    stmts: body_stmts,
                },
            },
            span: hir_span_value,
        });
        self.temp_operand(result_local, &ty)
    }

    /// Lower `while cond: body` into Body IR's single normalized loop shape: a [`bir::StatementKind::Loop`] whose
    /// body opens with `if not cond: break`, followed by the lowered loop body. A `while let` pattern condition —
    /// not yet modeled by v0 — lowers to an explicit unsupported placeholder instead of the real loop.
    pub(super) fn lower_while(
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
        // `while` never produces a value from `break`, so push `None`: a `break` inside this loop's body must
        // resolve to a plain valueless exit even if this `while` is lexically nested inside a value-producing
        // `loop:` expression (see `Self::loop_break_targets`'s docs for why the stack exists).
        self.loop_break_targets.push(None);
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
        self.loop_break_targets.pop();

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

    /// Lower a `for` statement. Range-shaped iterables lower into a normalized counting `Loop`, preserving
    /// #1103's original range-loop shape for the inline `for x in start..end:` header unchanged. Every other
    /// iterable -- builtin collections (`List`/`Dict`/`String`) and user-defined iterables implementing the RFC
    /// 068 `__iter__`/`__next__` protocol, including the fallible `for item in iterable?:` form (RFC 115) --
    /// lowers through [`Self::lower_general_iteration`], sharing its per-clause iteration primitive with
    /// comprehensions and generator expressions (see [`Self::lower_comprehension_clauses`]).
    ///
    /// "Range-shaped" covers two spellings, and both reach the same counting loop (#1165). One is the inline
    /// header, whose bounds are still lowered straight out of the AST. The other is a range *value* -- `r = 0..10`
    /// then `for i in r:` -- which reaches this point as an ordinary expression of the checked range type; it is
    /// materialized as a place and the loop reads its declared [`bir::AggregateKind::RANGE_FIELDS`] back, so a
    /// bound range iterates with the same facts as the range it was bound from instead of degrading to an opaque
    /// [`bir::StatementKind::IterNext`] poll. See [`Self::range_loop_source`] for why reading those fields back is
    /// sound, and [`Self::range_value_stop_condition`] for the one place the two spellings genuinely differ.
    ///
    /// Both paths accept the same loop-pattern subset the typechecker accepts -- a plain binding, `_`, and
    /// (recursively) a tuple of those, per `TypeChecker::define_for_pattern_bindings` in
    /// `src/frontend/typechecker/check_stmt.rs` (#1125). A plain `for x in ...` binds the produced item directly;
    /// every other shape writes it into a per-iteration temporary that [`Self::bind_for_pattern`] then projects one
    /// real named binding out of per bound name. Any shape outside that subset -- which the typechecker already
    /// rejects with its own diagnostic before lowering ever runs -- lowers to `Unsupported` naming the offending
    /// shape, checked up front so a refusal never leaves half-emitted bindings behind (the same
    /// "check before partially lowering" precedent as [`Self::lower_binary`] and [`Self::lower_match`]). The same
    /// up-front check also refuses a tuple pattern whose produced item is not a tuple of matching arity, so
    /// lowering can never invent `.0`/`.1` projections into a value that has no such fields -- see
    /// [`unsupported_for_pattern`].
    pub(super) fn lower_for(
        &mut self,
        for_stmt: &ast::ForStmt,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let iter_ty = self.resolve_ty(for_stmt.iter.span);
        let item_ty = self.for_item_type(&for_stmt.pattern, &iter_ty);
        if let Some(reason) = unsupported_for_pattern(&for_stmt.pattern.node, &item_ty) {
            self.push_unsupported_stmt(reason, span, out);
            return;
        }
        // The typechecker enters a lexical block scope for the loop header/body, so every binding introduced by the
        // pattern must disappear after the statement. Keep the active lookup map for restoration while leaving the
        // loop locals themselves in Body IR for the loop's statements to reference.
        let enclosing_bindings = self.bindings.clone();
        let Some(range) = self.range_loop_source(&for_stmt.iter, &iter_ty, scope, out) else {
            let loop_scope = self.new_scope(Some(scope), span);
            let item_local = self.declare_for_item_local(&for_stmt.pattern, &item_ty, loop_scope, span, &for_stmt.body);
            self.lower_general_iteration(
                &for_stmt.iter,
                item_local,
                scope,
                loop_scope,
                span,
                out,
                |builder, loop_scope, body_stmts| {
                    builder.bind_for_pattern(
                        &for_stmt.pattern,
                        &item_ty,
                        item_local,
                        loop_scope,
                        &for_stmt.body,
                        body_stmts,
                    );
                    builder.lower_block_into(&for_stmt.body, loop_scope, body_stmts);
                    builder.insert_scope_drops(body_stmts, loop_scope);
                },
            );
            self.bindings = enclosing_bindings;
            return;
        };
        self.lower_range_counting_loop(for_stmt, &item_ty, &range, scope, span, out);
        self.bindings = enclosing_bindings;
    }

    /// The item type a `for` loop's pattern binds, falling back to a range value's own element type.
    ///
    /// Normally this is just the typechecker's recorded type for the pattern span. A range value is the one case
    /// that needs help: `TypeChecker::infer_iterator_element_type` has no `Range` arm, so `for i in some_range`
    /// records `i` as [`IncanType::Unknown`] even though the range's own type argument says exactly what it
    /// yields. Taking the element type from there is not lowering inventing a type -- it is reading the one the
    /// typechecker already resolved for the range -- and without it the item local would declare as `?` and every
    /// read of it would carry [`bir::OwnershipFact::Unknown`] rather than the `copy` an `int` gets, which would
    /// leave a bound range iterating with visibly different facts from the inline header form.
    fn for_item_type(&self, pattern: &ast::Spanned<ast::Pattern>, iter_ty: &IncanType) -> IncanType {
        let checked = self.resolve_ty(pattern.span);
        if !matches!(checked, IncanType::Unknown) {
            return checked;
        }
        range_value_element_type(iter_ty).cloned().unwrap_or(checked)
    }

    /// Classify a `for` header's iterable as a range the counting loop can drive, lowering it to a place when it
    /// is a range *value*, or `None` when the loop belongs on the general-iteration path.
    ///
    /// An inline `start..end` header keeps its AST sub-expressions, because its bounds are still lowered directly
    /// (and its `end` deliberately re-lowered per iteration -- see [`Self::lower_range_counting_loop`]). Anything
    /// else is a range only if the typechecker says so, and reading its fields back afterwards is sound because
    /// the checked range type has exactly one producer: `TypeChecker::check_range_expr` (see [`RANGE_TYPE_BASE`]).
    /// Every value of that type therefore came from a range expression, which [`Self::lower_range_value`] is the
    /// only lowering for -- so the [`bir::AggregateKind::Range`] field layout this returns a place into is
    /// guaranteed to be the layout that value actually has. The `range()` builtin resolves to a different type and
    /// keeps its existing iterator path.
    fn range_loop_source<'ast>(
        &mut self,
        iter_expr: &'ast ast::Spanned<ast::Expr>,
        iter_ty: &IncanType,
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) -> Option<RangeLoopSource<'ast>> {
        if let ast::Expr::Range { start, end, inclusive } = &iter_expr.node {
            return Some(RangeLoopSource::Header {
                start,
                end,
                inclusive: *inclusive,
            });
        }
        range_value_element_type(iter_ty)?;
        Some(RangeLoopSource::Value(self.lower_expr_to_place(iter_expr, scope, out)))
    }

    /// Read one declared field off a materialized [`bir::AggregateKind::Range`] value.
    ///
    /// Every range field is a scalar read through a non-empty projection, so this always resolves to
    /// [`bir::OwnershipFact::Copy`] and never consumes the range local's last use -- a loop may read the same
    /// field on every iteration without the range appearing to be moved out from under itself.
    fn read_range_field(&mut self, range: &bir::Place, field: &str, field_ty: &IncanType) -> bir::Operand {
        let mut place = range.clone();
        place.projection.push(bir::PlaceElem::Field(field.to_string()));
        let (fact, last_use) = self.ownership_fact_for_place(&place, field_ty);
        bir::Operand::place(place, fact, last_use)
    }

    /// Build the break condition for a loop over a range *value*: stop once the index has passed the range's end,
    /// or has reached an end the range does not include.
    ///
    /// This is the one place the two range spellings differ, and the reason is that inclusivity is a property of
    /// the value rather than of the loop. An inline header knows which of `>` and `>=` it wants at lowering time,
    /// because `..` versus `..=` is written right there. A bound range does not: the statement that built the
    /// value and the loop that consumes it are different statements, and nothing stops a program from binding one
    /// range on one branch and another on the other. Choosing the comparison here from whichever construction
    /// lowering happened to see first would be a guess that a reassignment silently invalidates. So this computes
    /// *both* of the comparisons the inline form picks between, and lets the range's own
    /// [`bir::AggregateKind::RANGE_FIELD_INCLUSIVE`] operand select between them -- the same decision, made from
    /// the value instead of from the syntax.
    ///
    /// Every field is re-read per iteration, matching the inline form, which also re-lowers its `end` expression
    /// each time around rather than snapshotting it before the loop.
    fn range_value_stop_condition(
        &mut self,
        range: &bir::Place,
        idx_local: bir::LocalId,
        loop_scope: bir::ScopeId,
        span: HirSourceSpan,
        body_stmts: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let int_ty = IncanType::Primitive(IncanPrimitiveType::Int);
        let bool_ty = IncanType::Primitive(IncanPrimitiveType::Bool);
        let past_end = {
            let end = self.read_range_field(range, bir::AggregateKind::RANGE_FIELD_END, &int_ty);
            let idx = index_read(idx_local);
            self.push_assign_temp(
                bir::Rvalue::BinaryOp(bir::BinOp::Gt, idx, end),
                bool_ty.clone(),
                loop_scope,
                span,
                body_stmts,
            )
        };
        let at_or_past_end = {
            let end = self.read_range_field(range, bir::AggregateKind::RANGE_FIELD_END, &int_ty);
            let idx = index_read(idx_local);
            self.push_assign_temp(
                bir::Rvalue::BinaryOp(bir::BinOp::Ge, idx, end),
                bool_ty.clone(),
                loop_scope,
                span,
                body_stmts,
            )
        };
        let inclusive = self.read_range_field(range, bir::AggregateKind::RANGE_FIELD_INCLUSIVE, &bool_ty);
        let exclusive = self.push_assign_temp(
            bir::Rvalue::UnaryOp(bir::UnOp::Not, inclusive),
            bool_ty.clone(),
            loop_scope,
            span,
            body_stmts,
        );
        let stops_at_end = self.push_assign_temp(
            bir::Rvalue::BinaryOp(bir::BinOp::And, at_or_past_end, exclusive),
            bool_ty.clone(),
            loop_scope,
            span,
            body_stmts,
        );
        self.push_assign_temp(
            bir::Rvalue::BinaryOp(bir::BinOp::Or, past_end, stops_at_end),
            bool_ty,
            loop_scope,
            span,
            body_stmts,
        )
    }

    /// Lower one range-shaped `for` loop into the normalized counting `Loop` both range spellings share: seed an
    /// index from the range's start, break once it reaches the end, bind the loop pattern from the index, run the
    /// body, then advance the index by the range's step.
    ///
    /// Where the two spellings get those three pieces from is the only difference, and each is taken from
    /// `source`: an inline header lowers its own AST sub-expressions and knows its step and inclusivity
    /// statically, while a range value reads them back off the place it was materialized into. The `end` bound is
    /// evaluated *inside* the loop body in both cases, preserving the inline form's established re-evaluation
    /// timing rather than hoisting it.
    fn lower_range_counting_loop(
        &mut self,
        for_stmt: &ast::ForStmt,
        item_ty: &IncanType,
        source: &RangeLoopSource<'_>,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let int_ty = IncanType::Primitive(IncanPrimitiveType::Int);
        let start_operand = match source {
            RangeLoopSource::Header { start, .. } => self.lower_expr_to_operand(start, scope, out),
            RangeLoopSource::Value(range) => {
                self.read_range_field(range, bir::AggregateKind::RANGE_FIELD_START, &int_ty)
            }
        };
        let idx_local = self.new_temp(int_ty.clone(), scope, span);
        out.push(bir::Statement {
            kind: bir::StatementKind::Assign {
                place: bir::Place::from_local(idx_local),
                rvalue: bir::Rvalue::Use(start_operand),
            },
            span,
        });

        let loop_scope = self.new_scope(Some(scope), span);
        // `for` never produces a value from `break` (same reasoning as `while` -- see `Self::lower_while`).
        self.loop_break_targets.push(None);
        let mut body_stmts = Vec::new();

        let cond = match source {
            RangeLoopSource::Header { end, inclusive, .. } => {
                let end_operand = self.lower_expr_to_operand(end, loop_scope, &mut body_stmts);
                let cmp_op = if *inclusive { bir::BinOp::Gt } else { bir::BinOp::Ge };
                self.push_assign_temp(
                    bir::Rvalue::BinaryOp(cmp_op, index_read(idx_local), end_operand),
                    IncanType::Primitive(IncanPrimitiveType::Bool),
                    loop_scope,
                    span,
                    &mut body_stmts,
                )
            }
            RangeLoopSource::Value(range) => {
                self.range_value_stop_condition(range, idx_local, loop_scope, span, &mut body_stmts)
            }
        };
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

        // `for _ in start..end` binds nothing and the range's own index already drives the loop, so it needs no
        // per-iteration item local at all -- unlike the general path, where `IterNext` must still write the polled
        // item somewhere for the poll itself to happen.
        if !matches!(for_stmt.pattern.node, ast::Pattern::Wildcard) {
            let item_local = self.declare_for_item_local(&for_stmt.pattern, item_ty, loop_scope, span, &for_stmt.body);
            body_stmts.push(bir::Statement {
                kind: bir::StatementKind::Assign {
                    place: bir::Place::from_local(item_local),
                    rvalue: bir::Rvalue::Use(index_read(idx_local)),
                },
                span,
            });
            self.bind_for_pattern(
                &for_stmt.pattern,
                item_ty,
                item_local,
                loop_scope,
                &for_stmt.body,
                &mut body_stmts,
            );
        }

        self.lower_block_into(&for_stmt.body, loop_scope, &mut body_stmts);
        self.insert_scope_drops(&mut body_stmts, loop_scope);

        let step = match source {
            RangeLoopSource::Header { .. } => bir::Operand::Constant(bir::Constant::Int(RANGE_UNIT_STEP)),
            RangeLoopSource::Value(range) => {
                self.read_range_field(range, bir::AggregateKind::RANGE_FIELD_STEP, &int_ty)
            }
        };
        let incremented = self.push_assign_temp(
            bir::Rvalue::BinaryOp(bir::BinOp::Add, index_read(idx_local), step),
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
        self.loop_break_targets.pop();

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

    /// Declare the local each produced item of a `for` loop is written into.
    ///
    /// A plain `for x in ...` binds the item directly: the item local *is* `x`'s local, so the produced value is
    /// never copied and the loop shape #1103/#1101 established is preserved byte-for-byte. Every other supported
    /// pattern shape has no single name to write into, so the item goes into a temporary that
    /// [`Self::bind_for_pattern`] projects the real bindings out of -- the same "materialize once, then bind each
    /// element off a projection" shape [`Self::lower_tuple_unpack`] already uses for `a, b = value`.
    pub(super) fn declare_for_item_local(
        &mut self,
        pattern: &ast::Spanned<ast::Pattern>,
        item_ty: &IncanType,
        loop_scope: bir::ScopeId,
        span: HirSourceSpan,
        body: &[ast::Spanned<ast::Statement>],
    ) -> bir::LocalId {
        match &pattern.node {
            ast::Pattern::Binding(name) => {
                let total_reads = count_reads_in_stmts(name, body);
                self.declare_new_local_with_reads(name.clone(), item_ty.clone(), loop_scope, span, total_reads)
            }
            _ => self.new_temp(item_ty.clone(), loop_scope, span),
        }
    }

    /// Emit the binding statements a `for` loop's pattern needs against the item local, immediately after the
    /// per-iteration `IterNext` (or, on the range path, after the index copy) has written it.
    ///
    /// A bare [`ast::Pattern::Binding`] emits nothing: [`Self::declare_for_item_local`] already declared the item
    /// local *as* that binding, so there is nothing left to project. Every other shape delegates to
    /// [`Self::bind_for_pattern_fields`], which means every binding that walk reaches is nested under at least one
    /// tuple field and therefore always reads through a projection.
    pub(super) fn bind_for_pattern(
        &mut self,
        pattern: &ast::Spanned<ast::Pattern>,
        item_ty: &IncanType,
        item_local: bir::LocalId,
        loop_scope: bir::ScopeId,
        body: &[ast::Spanned<ast::Statement>],
        out: &mut Vec<bir::Statement>,
    ) {
        if matches!(pattern.node, ast::Pattern::Binding(_)) {
            return;
        }
        let item_place = bir::Place::from_local(item_local);
        self.bind_for_pattern_fields(pattern, item_ty, &item_place, loop_scope, body, out);
    }

    /// Recursively bind one `for`-pattern node against `place`, the (already projected) part of the produced item
    /// it corresponds to, emitting one `Assign` per bound name in source order.
    ///
    /// Iteration binding is *irrefutable*: unlike [`Self::lower_match_pattern`], which builds a [`bir::Pattern`]
    /// for match-arm dispatch, there is nothing here to test or branch on, so this walk emits plain assignments and
    /// deliberately does not reuse that machinery (#1125 names conflating the two as a non-goal). What it does
    /// share is that walk's projection convention -- the zero-based tuple-element index spelled as a
    /// [`bir::PlaceElem::Field`], matching [`Self::lower_tuple_unpack`]'s `.0`/`.1` spelling -- and its
    /// [`tuple_element_types`] source for per-element types, so a nested tuple keeps resolved element types all the
    /// way down and falls back to [`IncanType::Unknown`] per slot only where the resolved type is not a tuple of
    /// the right arity.
    ///
    /// Each element is read through [`Self::ownership_fact_for_place`], exactly as
    /// [`Self::lower_tuple_unpack`] reads its own elements, so a non-Copy element borrows rather than moving out of
    /// a place v0 does not track partial-move state for. Each bound name becomes a real
    /// [`bir::LocalOrigin::UserBinding`] local in `loop_scope`, seeded with its own last-use countdown over the
    /// loop body, so [`Self::insert_scope_drops`] gives every non-Copy binding an explicit per-iteration drop.
    ///
    /// [`unsupported_for_pattern`] has already rejected every shape outside the accepted subset -- and every item
    /// type that is not a tuple of matching arity -- before [`Self::lower_for`] reaches this walk, so the remaining
    /// arms are unreachable in practice; they emit nothing rather than panicking if that invariant is ever violated
    /// by a hand-built AST.
    pub(super) fn bind_for_pattern_fields(
        &mut self,
        pattern: &ast::Spanned<ast::Pattern>,
        expected_ty: &IncanType,
        place: &bir::Place,
        loop_scope: bir::ScopeId,
        body: &[ast::Spanned<ast::Statement>],
        out: &mut Vec<bir::Statement>,
    ) {
        let span = hir_span(pattern.span);
        match &pattern.node {
            ast::Pattern::Wildcard => {}
            ast::Pattern::Binding(name) => {
                let (fact, last_use) = self.ownership_fact_for_place(place, expected_ty);
                let element = bir::Operand::place(place.clone(), fact, last_use);
                let total_reads = count_reads_in_stmts(name, body);
                let local =
                    self.declare_new_local_with_reads(name.clone(), expected_ty.clone(), loop_scope, span, total_reads);
                out.push(bir::Statement {
                    kind: bir::StatementKind::Assign {
                        place: bir::Place::from_local(local),
                        rvalue: bir::Rvalue::Use(element),
                    },
                    span,
                });
            }
            ast::Pattern::Tuple(items) => {
                let element_types = tuple_element_types(expected_ty, items.len());
                for (index, (item, element_ty)) in items.iter().zip(&element_types).enumerate() {
                    let mut field_place = place.clone();
                    field_place.projection.push(bir::PlaceElem::Field(index.to_string()));
                    self.bind_for_pattern_fields(item, element_ty, &field_place, loop_scope, body, out);
                }
            }
            ast::Pattern::Literal(_) | ast::Pattern::Constructor(..) | ast::Pattern::Group(_) | ast::Pattern::Or(_) => {
            }
        }
    }

    /// Lower one general (non-range) iteration: materialize an iterator from `iter_expr` before the loop, then push
    /// a single [`bir::StatementKind::Loop`] whose body opens with a [`bir::StatementKind::IterNext`] writing each
    /// produced item into `pattern_local`, followed by `body_fn`. Shared by [`Self::lower_for`]'s general-iterable
    /// path and [`Self::lower_comprehension_clauses`]'s `for`-clause handling, so builtin-vs-protocol iteration is
    /// resolved in exactly one place rather than twice.
    ///
    /// Looks up [`TypeCheckInfo::protocol_iteration`] at `iter_expr`'s span to decide the [`bir::IterProtocol`]:
    /// `None` means a builtin collection or range, where "the iterator" is modeled as the iterable's own value (no
    /// method dispatch) -- a plain `Assign`; `Some` means a resolved `__iter__`/`__next__` protocol, where the
    /// iterator is obtained via an explicit `iter_method` [`bir::Callee::Method`] call. When the resolved protocol
    /// is fallible (`for item in iterable?:`, RFC 115), `iter_expr` is itself `ast::Expr::Try(inner)` with the `?`
    /// acting as the fallible-poll marker rather than an ordinary `Result` unwrap -- `inner` is lowered directly as
    /// the iterable in that case (matching the existing Rust-emission backend's own `(Expr::Try(inner), Some(_)) =>
    /// lower inner` special case in `src/backend/ir/lower/stmt.rs`), so the marker `?` is not double-lowered through
    /// [`Self::lower_try`]. Any other `Expr::Try` (an ordinary `for item in result_of_iterable?:` unwrap) falls
    /// through to the normal expression-lowering path, which already turns it into a
    /// [`bir::StatementKind::TryPropagate`] ahead of the loop via [`Self::lower_expr_to_place`]'s existing
    /// `Expr::Try` handling -- no special-casing needed for that form.
    ///
    /// The iterable is always read as a [`bir::OwnershipFact::Borrow`], matching
    /// [`Self::lower_method_call`]'s established receiver-borrow precedent (never an unsound move, and consistent
    /// with obtaining an iterator conceptually borrowing its source rather than consuming it at this normalized
    /// level); the materialized iterator local is polled with [`bir::OwnershipFact::MutBorrow`] each iteration,
    /// since polling advances its internal state.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_general_iteration(
        &mut self,
        iter_expr: &ast::Spanned<ast::Expr>,
        pattern_local: bir::LocalId,
        outer_scope: bir::ScopeId,
        loop_scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
        body_fn: impl FnOnce(&mut Self, bir::ScopeId, &mut Vec<bir::Statement>),
    ) {
        let protocol = self.type_info.protocol_iteration(iter_expr.span).cloned();
        let fallible = protocol.as_ref().is_some_and(|p| p.fallible_error_type.is_some());
        let effective_iter_expr: &ast::Spanned<ast::Expr> = match (&iter_expr.node, fallible) {
            (ast::Expr::Try(inner), true) => inner,
            _ => iter_expr,
        };

        let iterable_place = self.lower_expr_to_place(effective_iter_expr, outer_scope, out);
        let iterator_ty = match &protocol {
            Some(p) => semantic_type_from_resolved(&p.iterator_type),
            None => self.resolve_ty(effective_iter_expr.span),
        };
        let iterator_local = self.new_temp(iterator_ty, outer_scope, span);
        match &protocol {
            Some(p) => out.push(bir::Statement {
                kind: bir::StatementKind::Call {
                    destination: Some(bir::Place::from_local(iterator_local)),
                    callee: bir::Callee::Method(bir::MethodTarget::synthesized(p.iter_method.clone())),
                    args: fixed_elements(vec![bir::Operand::place(
                        iterable_place,
                        bir::OwnershipFact::Borrow,
                        false,
                    )]),
                    may_panic: false,
                },
                span,
            }),
            None => out.push(bir::Statement {
                kind: bir::StatementKind::Assign {
                    place: bir::Place::from_local(iterator_local),
                    rvalue: bir::Rvalue::Use(bir::Operand::place(iterable_place, bir::OwnershipFact::Borrow, false)),
                },
                span,
            }),
        }

        self.loop_break_targets.push(None);
        let mut body_stmts = Vec::new();

        let iter_protocol = match &protocol {
            Some(p) => bir::IterProtocol::UserDefined {
                next_method: p.next_method.clone(),
                fallible,
            },
            None => bir::IterProtocol::Builtin,
        };
        body_stmts.push(bir::Statement {
            kind: bir::StatementKind::IterNext {
                destination: bir::Place::from_local(pattern_local),
                iterator: bir::Operand::place(
                    bir::Place::from_local(iterator_local),
                    bir::OwnershipFact::MutBorrow,
                    false,
                ),
                protocol: iter_protocol,
            },
            span,
        });

        body_fn(self, loop_scope, &mut body_stmts);
        self.loop_break_targets.pop();

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
}
