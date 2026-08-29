//! Lowering for statements and every assignment form: field, index, compound, tuple, and chained.

use super::refusals::*;
use super::*;

impl<'type_info, 'source> BodyBuilder<'type_info, 'source> {
    /// Lower every statement in `stmts` into `out`, within `scope`. Statements are lowered in source order and each
    /// one is given the statement suffix that follows it (`&stmts[index + 1..]`), so last-use countdowns seeded by
    /// [`Self::declare_new_local`] only count reads that can still occur after the declaration.
    pub(super) fn lower_block_into(
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
    pub(super) fn lower_stmt_into(
        &mut self,
        stmt: &ast::Spanned<ast::Statement>,
        remaining: &[ast::Spanned<ast::Statement>],
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) {
        let span = hir_span(stmt.span);
        match &stmt.node {
            ast::Statement::Assignment(assignment) => self.lower_assignment(assignment, remaining, scope, span, out),
            ast::Statement::FieldAssignment(field_assignment) => {
                self.lower_field_assignment(field_assignment, scope, span, out)
            }
            ast::Statement::IndexAssignment(index_assignment) => {
                self.lower_index_assignment(index_assignment, scope, span, out)
            }
            ast::Statement::CompoundAssignment(compound_assignment) => {
                // Passed the raw AST span, not `span`: the typechecker keys a resolved operator hook by source
                // span, and only the untranslated span can look one up.
                self.lower_compound_assignment(compound_assignment, scope, stmt.span, out)
            }
            ast::Statement::TupleUnpack(tuple_unpack) => {
                self.lower_tuple_unpack(tuple_unpack, remaining, scope, span, out)
            }
            ast::Statement::TupleAssign(tuple_assign) => self.lower_tuple_assign(tuple_assign, scope, span, out),
            ast::Statement::ChainedAssignment(chained_assignment) => {
                self.lower_chained_assignment(chained_assignment, remaining, scope, span, out)
            }
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
                // `yield value` parses as an ordinary expression statement wrapping `ast::Expr::Yield(Some(_))`
                // (there is no separate `ast::Statement::Yield` AST node) -- mirror the existing Rust-emission
                // backend's own `lower_statement` (`src/backend/ir/lower/stmt.rs`), which special-cases this exact
                // shape before falling back to generic expression-statement lowering. A bare `yield` (no value)
                // falls through to the generic `Expr` arm below, same as that backend, and lowers via the
                // expression-position `yield` stub (see the module docs).
                if let ast::Expr::Yield(Some(value)) = &expr.node {
                    self.lower_yield(value, scope, span, out);
                } else {
                    let value = self.lower_expr_to_operand(expr, scope, out);
                    out.push(bir::Statement {
                        kind: bir::StatementKind::Expr { value },
                        span,
                    });
                }
            }
            ast::Statement::Assert(assert_stmt) => self.lower_assert(assert_stmt, remaining, scope, span, out),
            ast::Statement::Pass => {}
            ast::Statement::Break(value) => self.lower_break(value.as_ref(), scope, span, out),
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
    pub(super) fn lower_assignment(
        &mut self,
        assignment: &ast::AssignmentStmt,
        remaining: &[ast::Spanned<ast::Statement>],
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        // A closure value already carries the typechecker's callable shape. A partial retains that full callable
        // type, with captured presets represented as named-overrideable defaults. Positional calls skip those preset
        // slots, and `LocalCallableTarget::binding` records the resulting declaration mapping. Keeping this type on
        // the binding makes the local call contract agree with the `Rvalue::Closure` that creates the value.
        let ty = self
            .callable_value_ty(&assignment.value)
            .unwrap_or_else(|| self.resolve_ty(assignment.value.span));
        let materializes_range = self.expr_has_materialized_range_layout(&assignment.value);
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
        if materializes_range {
            self.materialized_range_locals.insert(local);
        } else {
            self.materialized_range_locals.remove(&local);
        }
    }

    /// Lower `obj.field = value` (including the compound `obj.field <op>= value` form). The parser already
    /// desugars a compound `FieldAssignmentStmt` so `value` is the full `obj.field <op> rhs` expression
    /// (`crates/incan_syntax/src/parser/stmts.rs`'s `assignment_or_expr_stmt`) -- `fa.compound_op` is purely a
    /// formatter hint for round-tripping `+=` spelling and carries no separate lowering semantics here, so this
    /// only needs to build the write-side place and lower `value` normally.
    pub(super) fn lower_field_assignment(
        &mut self,
        field_assignment: &ast::FieldAssignmentStmt,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let mut place = self.lower_expr_to_place(&field_assignment.object, scope, out);
        place
            .projection
            .push(bir::PlaceElem::Field(field_assignment.field.clone()));
        let value = self.lower_expr_to_operand(&field_assignment.value, scope, out);
        out.push(bir::Statement {
            kind: bir::StatementKind::Assign {
                place,
                rvalue: bir::Rvalue::Use(value),
            },
            span,
        });
    }

    /// Lower `obj[index] = value` (including the compound `obj[index] <op>= value` form, pre-desugared into
    /// `value` by the parser -- see [`Self::lower_field_assignment`]'s docs for the same note on
    /// `IndexAssignmentStmt::compound_op`). The object place is lowered before the index operand, preserving the
    /// established assignment evaluation order in the Rust-emission backend: object, index, then assigned value.
    pub(super) fn lower_index_assignment(
        &mut self,
        index_assignment: &ast::IndexAssignmentStmt,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let mut place = self.lower_expr_to_place(&index_assignment.object, scope, out);
        let index_operand = self.lower_expr_to_operand(&index_assignment.index, scope, out);
        place.projection.push(bir::PlaceElem::Index(Box::new(index_operand)));
        let value = self.lower_expr_to_operand(&index_assignment.value, scope, out);
        out.push(bir::Statement {
            kind: bir::StatementKind::Assign {
                place,
                rvalue: bir::Rvalue::Use(value),
            },
            span,
        });
    }

    /// Lower `name <op>= value` (`x += y`, `x &= y`, `x <<= y`, ...). Unlike field/index compound assignment, the
    /// parser leaves `ca.value` as the plain right-hand operand rather than pre-desugaring it, so this explicitly
    /// reads `name`'s current value, combines it with `value` via [`Self::lower_binary_from_operands`] (shared with
    /// [`Self::lower_binary`], so every compound form gets exactly the operator representation its binary spelling
    /// gets — a primitive [`bir::BinOp`] for the arithmetic, bitwise, and shift operators, a `Callee::Helper` call
    /// for string concatenation), and writes the result back.
    ///
    /// Three cases refuse instead, each with an explicit unsupported placeholder rather than a panic or a guess:
    /// an operator with no Body IR equivalent (see [`lower_binary_op`]), a name that is not currently bound (should
    /// not happen after a successful typecheck), and a compound assignment the typechecker resolved to a
    /// user-defined operator hook.
    ///
    /// That last case is the one worth stating. `v &= w` on a type defining `__iand__` is a *method call*, and the
    /// typechecker records the resolved dispatch against this statement's span. Combining the operands with
    /// [`bir::BinOp::BitAnd`] here would claim a machine operation the source never asked for — the same wrong
    /// representation [`Self::lower_binary`] avoids by consulting the recorded dispatch first. Body IR has no
    /// place-targeted operator-dispatch form yet (`lower_operator_dispatch` needs two expression operands, and a
    /// compound assignment's left side is a bound name, not an expression), so the honest answer is a named
    /// refusal. Owner: #1101's operator-dispatch lowering. `@=` is permanently in this group — `__imatmul__` is a
    /// protocol hook with no primitive form at all.
    pub(super) fn lower_compound_assignment(
        &mut self,
        compound_assignment: &ast::CompoundAssignmentStmt,
        scope: bir::ScopeId,
        source_span: ast::Span,
        out: &mut Vec<bir::Statement>,
    ) {
        let span = hir_span(source_span);
        let Some(&local) = self.bindings.get(&compound_assignment.name) else {
            self.push_unsupported_stmt(
                format!("compound assignment to unbound name `{}`", compound_assignment.name),
                span,
                out,
            );
            return;
        };

        // A resolved operator hook makes this a method call, not a primitive combination. Checked before the
        // operand is lowered, matching the "never partially lower an operator we will refuse" rule the binary
        // path holds to.
        if let Some(dispatch) = self.type_info.resolved_operator_call(source_span)
            && dispatch.kind == ResolvedOperatorKind::Binary
        {
            self.push_unsupported_stmt(
                format!("compound assignment through operator hook `{}`", dispatch.method),
                span,
                out,
            );
            return;
        }

        let lhs_ty = self.locals[local.index()].ty.clone();
        let op = compound_assignment.op.binary_op();
        let rhs_ty = self.resolve_ty(compound_assignment.value.span);
        if !Self::binary_op_is_supported(op, &lhs_ty, &rhs_ty) {
            self.push_unsupported_stmt(
                format!("compound assignment operator {:?}", compound_assignment.op),
                span,
                out,
            );
            return;
        }
        let lhs_place = bir::Place::from_local(local);
        let (fact, last_use) = self.ownership_fact_for_place(&lhs_place, &lhs_ty);
        let lhs_operand = bir::Operand::place(lhs_place, fact, last_use);
        let rhs_operand = self.lower_expr_to_operand(&compound_assignment.value, scope, out);
        let result = self.lower_binary_from_operands(
            op,
            &lhs_ty,
            lhs_operand,
            &rhs_ty,
            rhs_operand,
            lhs_ty.clone(),
            scope,
            span,
            out,
        );
        out.push(bir::Statement {
            kind: bir::StatementKind::Assign {
                place: bir::Place::from_local(local),
                rvalue: bir::Rvalue::Use(result),
            },
            span,
        });
    }

    /// Resolve or declare the local for one name bound by a multi-target assignment (tuple unpack or chained
    /// assignment). A `Reassign` binding reuses an existing local exactly like [`Self::lower_assignment`] does for
    /// a plain single-target reassignment; every other binding kind always declares a fresh local, matching
    /// source-level shadowing semantics.
    pub(super) fn bind_multi_target_name(
        &mut self,
        name: &str,
        ty: IncanType,
        binding: ast::BindingKind,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        remaining: &[ast::Spanned<ast::Statement>],
    ) -> bir::LocalId {
        match binding {
            ast::BindingKind::Reassign => self
                .bindings
                .get(name)
                .copied()
                .unwrap_or_else(|| self.declare_new_local(name.to_string(), ty, scope, span, remaining)),
            ast::BindingKind::Inferred | ast::BindingKind::Let | ast::BindingKind::Mutable => {
                self.declare_new_local(name.to_string(), ty, scope, span, remaining)
            }
        }
    }

    /// Lower `a, b = value` / `let a, b = value` into a sequence of single-target `Assign` statements: materialize
    /// `value` once, then bind each name to the corresponding `.{index}` tuple-field projection off it, in
    /// left-to-right order. Element reads go through the same [`Self::ownership_fact_for_place`] a plain
    /// `.field`/`[index]` read anywhere else in v0 uses, so a non-Copy element borrows rather than moves (v0 does
    /// not track partial-move state out of a place, per [`Self::ownership_fact_for_place`]'s own docs) --
    /// consistent with, not a special case of, that existing policy.
    pub(super) fn lower_tuple_unpack(
        &mut self,
        tuple_unpack: &ast::TupleUnpackStmt,
        remaining: &[ast::Spanned<ast::Statement>],
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let value_ty = self.resolve_ty(tuple_unpack.value.span);
        if let Some(reason) = unsupported_tuple_destructure(&value_ty, tuple_unpack.names.len()) {
            self.push_unsupported_stmt(reason, span, out);
            return;
        }
        let value_operand = self.lower_expr_to_operand(&tuple_unpack.value, scope, out);
        let value_place = self.materialize_operand_to_place(value_operand, value_ty.clone(), scope, span, out);
        let element_types = tuple_element_types(&value_ty, tuple_unpack.names.len());

        for (index, (name, element_ty)) in tuple_unpack.names.iter().zip(&element_types).enumerate() {
            let mut element_place = value_place.clone();
            element_place.projection.push(bir::PlaceElem::Field(index.to_string()));
            let (fact, last_use) = self.ownership_fact_for_place(&element_place, element_ty);
            let element_operand = bir::Operand::place(element_place, fact, last_use);
            let local =
                self.bind_multi_target_name(name, element_ty.clone(), tuple_unpack.binding, scope, span, remaining);
            out.push(bir::Statement {
                kind: bir::StatementKind::Assign {
                    place: bir::Place::from_local(local),
                    rvalue: bir::Rvalue::Use(element_operand),
                },
                span,
            });
        }
    }

    /// Lower `t1, t2 = value` where the targets are lvalue expressions (`arr[i], arr[j] = ...`), not new bindings
    /// -- used for swaps and other multi-target reassignments. Materializes `value` once, then reads and
    /// materializes each element into its own fresh temporary *before* writing to any target, so aliased targets
    /// and sources (for example `arr[i], arr[j] = arr[j], arr[i]`) read the pre-assignment values rather than one
    /// another's already-written results. This is genuinely new coverage: the existing Rust-emission backend does
    /// not implement `TupleAssign` at all (`src/backend/ir/lower/stmt.rs` returns a `LoweringError`), so there is
    /// no existing behavior to mirror here -- the evaluation order above is v0's own design, chosen specifically
    /// to make `a, b = b, a` swap correctly.
    pub(super) fn lower_tuple_assign(
        &mut self,
        tuple_assign: &ast::TupleAssignStmt,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let value_ty = self.resolve_ty(tuple_assign.value.span);
        if let Some(reason) = unsupported_tuple_destructure(&value_ty, tuple_assign.targets.len()) {
            self.push_unsupported_stmt(reason, span, out);
            return;
        }
        let value_operand = self.lower_expr_to_operand(&tuple_assign.value, scope, out);
        let value_place = self.materialize_operand_to_place(value_operand, value_ty.clone(), scope, span, out);
        let element_types = tuple_element_types(&value_ty, tuple_assign.targets.len());

        let mut element_operands = Vec::with_capacity(tuple_assign.targets.len());
        for (index, element_ty) in element_types.iter().enumerate() {
            let mut element_place = value_place.clone();
            element_place.projection.push(bir::PlaceElem::Field(index.to_string()));
            let (fact, last_use) = self.ownership_fact_for_place(&element_place, element_ty);
            let element_operand = bir::Operand::place(element_place, fact, last_use);
            element_operands.push(self.push_assign_temp(
                bir::Rvalue::Use(element_operand),
                element_ty.clone(),
                scope,
                span,
                out,
            ));
        }

        for (target, value) in tuple_assign.targets.iter().zip(element_operands) {
            let place = self.lower_expr_to_place(target, scope, out);
            out.push(bir::Statement {
                kind: bir::StatementKind::Assign {
                    place,
                    rvalue: bir::Rvalue::Use(value),
                },
                span,
            });
        }
    }

    /// Lower `x = y = z = value` into `z = value; y = <read z>; x = <read y>` (rightmost target first), matching
    /// the direction the existing Rust-emission backend already chose for this same desugar
    /// (`src/backend/ir/lower/stmt.rs`'s `ChainedAssignment` arm).
    pub(super) fn lower_chained_assignment(
        &mut self,
        chained_assignment: &ast::ChainedAssignmentStmt,
        remaining: &[ast::Spanned<ast::Statement>],
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let Some(last_name) = chained_assignment.targets.last() else {
            self.push_unsupported_stmt("empty chained assignment".to_string(), span, out);
            return;
        };
        let value_ty = self.resolve_ty(chained_assignment.value.span);
        let value_operand = self.lower_expr_to_operand(&chained_assignment.value, scope, out);
        let mut prev_local = self.bind_multi_target_name(
            last_name,
            value_ty.clone(),
            chained_assignment.binding,
            scope,
            span,
            remaining,
        );
        out.push(bir::Statement {
            kind: bir::StatementKind::Assign {
                place: bir::Place::from_local(prev_local),
                rvalue: bir::Rvalue::Use(value_operand),
            },
            span,
        });

        // Walk the remaining targets right-to-left, each one reading the local immediately to its right.
        for name in chained_assignment.targets[..chained_assignment.targets.len() - 1]
            .iter()
            .rev()
        {
            // `remaining_reads[prev_local]` was seeded only from statements *after* this whole chained-assignment
            // statement (see `Self::declare_new_local`'s `remaining` parameter) -- it does not know about the
            // synthetic read performed right here, within the very statement that (re)bound `prev_local`. Bump it
            // by one first so the shared `Self::ownership_fact_for_place` decrement below still lands on the
            // correct move/clone decision instead of under-counting by one.
            if let Some(remaining_count) = self.remaining_reads.get_mut(&prev_local) {
                *remaining_count += 1;
            }
            let place = bir::Place::from_local(prev_local);
            let (fact, last_use) = self.ownership_fact_for_place(&place, &value_ty);
            let operand = bir::Operand::place(place, fact, last_use);
            let local = self.bind_multi_target_name(
                name,
                value_ty.clone(),
                chained_assignment.binding,
                scope,
                span,
                remaining,
            );
            out.push(bir::Statement {
                kind: bir::StatementKind::Assign {
                    place: bir::Place::from_local(local),
                    rvalue: bir::Rvalue::Use(operand),
                },
                span,
            });
            prev_local = local;
        }
    }

    /// Lower a `break` / `break value` statement. A value routes into the innermost enclosing loop's result place
    /// when that loop is a value-producing `loop:` expression (see [`Self::lower_loop_expr`]) -- otherwise it stays
    /// on the `Break` statement itself, matching [`bir::StatementKind::Break`]'s documented default. The innermost
    /// context comes from [`Self::loop_break_targets`], which every loop-lowering path pushes/pops around its own
    /// body so a `break` always targets the loop it is lexically inside, never an outer one.
    pub(super) fn lower_break(
        &mut self,
        value: Option<&ast::Spanned<ast::Expr>>,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let target = self.loop_break_targets.last().copied().flatten();
        match (value, target) {
            (Some(expr), Some(result_local)) => {
                let operand = self.lower_expr_to_operand(expr, scope, out);
                out.push(bir::Statement {
                    kind: bir::StatementKind::Assign {
                        place: bir::Place::from_local(result_local),
                        rvalue: bir::Rvalue::Use(operand),
                    },
                    span,
                });
                out.push(bir::Statement {
                    kind: bir::StatementKind::Break { value: None },
                    span,
                });
            }
            _ => {
                let operand = value.map(|v| self.lower_expr_to_operand(v, scope, out));
                out.push(bir::Statement {
                    kind: bir::StatementKind::Break { value: operand },
                    span,
                });
            }
        }
    }

    /// Lower a statement-position `yield value` (`ast::Expr::Yield(Some(value))` reached through
    /// [`Self::lower_stmt_into`]'s `ast::Statement::Expr` arm) into a [`bir::StatementKind::Yield`].
    ///
    /// `value` is lowered through the same [`Self::lower_expr_to_operand`] path every other statement's operand
    /// goes through, so ownership facts/last-use tracking apply to a yielded value exactly like any other read.
    /// Records the runtime dependencies the existing Rust-emission backend's own `yield` lowering actually needs
    /// (`__incan_yield.yield_value(..)` on a `GeneratorYield` handle backed by `std::thread::spawn` and
    /// `std::sync::mpsc::sync_channel` -- see `crates/incan_stdlib/src/iter.rs`'s `Generator`/`SpawnedGenerator`):
    /// a named runtime helper (mirroring how [`Self::lower_fstring`] records `"fstring"` without a new
    /// [`bir::HelperOp`] variant, since `Yield` is its own statement kind, not a [`bir::Callee::Helper`] call),
    /// [`AbiV0RuntimeRequirement::HostedStd`] (the spawned-thread/channel machinery is not freestanding-compatible),
    /// and [`AbiV0RuntimeRequirement::Allocator`] (the channel and boxed iterator both allocate).
    pub(super) fn lower_yield(
        &mut self,
        value: &ast::Spanned<ast::Expr>,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let operand = self.lower_expr_to_operand(value, scope, out);
        out.push(bir::Statement {
            kind: bir::StatementKind::Yield { value: operand },
            span,
        });
        self.record_runtime_requirement(AbiV0RuntimeRequirement::RuntimeHelper("generator".to_string()));
        self.record_runtime_requirement(AbiV0RuntimeRequirement::HostedStd);
        self.record_runtime_requirement(AbiV0RuntimeRequirement::Allocator);
    }
}
