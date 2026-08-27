//! Lowering for list and dict comprehensions and generator expressions, and the clause/terminal machinery they share.

use super::*;

impl<'type_info, 'source> BodyBuilder<'type_info, 'source> {
    /// Lower a list comprehension `[expr for pattern in iter if filter]` into: an empty
    /// `AggregateKind::List` temporary, the desugared clause-chain loop (see
    /// [`Self::lower_comprehension_clauses`]), pushing each accepted element into it via a compiler-synthesized
    /// `push` [`bir::Callee::Method`] call, then a read of the completed list. Only v0's single mirrored
    /// `(pattern, iter, filter)` clause is lowered -- `comp.clauses` is intentionally not consulted, since neither
    /// the typechecker (`check_list_comp` in `src/frontend/typechecker/check_expr/comps.rs`) nor the existing
    /// Rust-emission backend (`src/backend/ir/lower/expr/comprehensions.rs`) reads it either; a list comprehension
    /// with more than one `for` clause is not actually type-checked or emitted as multi-clause today; treating
    /// `comp.clauses` as authoritative here would silently lower a shape nothing else in the pipeline validates.
    pub(super) fn lower_list_comp(
        &mut self,
        comp: &ast::ListComp,
        span: ast::Span,
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let hir_span_value = hir_span(span);
        let ty = self.resolve_ty(span);
        let list_local = self.new_temp(ty.clone(), scope, hir_span_value);
        out.push(bir::Statement {
            kind: bir::StatementKind::Assign {
                place: bir::Place::from_local(list_local),
                rvalue: bir::Rvalue::Aggregate(bir::AggregateKind::List, Vec::new()),
            },
            span: hir_span_value,
        });
        self.record_runtime_requirement(AbiV0RuntimeRequirement::Allocator);

        let clauses = single_comprehension_clauses(&comp.pattern, &comp.iter, comp.filter.as_ref());
        let terminal = ComprehensionTerminal::ListPush {
            list_local,
            element: &comp.expr,
        };
        self.lower_scoped_comprehension_clauses(&clauses, &terminal, scope, hir_span_value, out);
        self.temp_operand(list_local, &ty)
    }

    /// Lower a dict comprehension `{key: value for pattern in iter if filter}` the same way
    /// [`Self::lower_list_comp`] lowers a list comprehension, but growing an `AggregateKind::Dict` temporary via a
    /// compiler-synthesized `insert` call. See [`Self::lower_list_comp`]'s docs for why only the single mirrored
    /// clause is lowered, not `comp.clauses`.
    pub(super) fn lower_dict_comp(
        &mut self,
        comp: &ast::DictComp,
        span: ast::Span,
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let hir_span_value = hir_span(span);
        let ty = self.resolve_ty(span);
        let dict_local = self.new_temp(ty.clone(), scope, hir_span_value);
        out.push(bir::Statement {
            kind: bir::StatementKind::Assign {
                place: bir::Place::from_local(dict_local),
                rvalue: bir::Rvalue::Dict(Vec::new()),
            },
            span: hir_span_value,
        });
        self.record_runtime_requirement(AbiV0RuntimeRequirement::Allocator);

        let clauses = single_comprehension_clauses(&comp.pattern, &comp.iter, comp.filter.as_ref());
        let terminal = ComprehensionTerminal::DictInsert {
            dict_local,
            key: &comp.key,
            value: &comp.value,
        };
        self.lower_scoped_comprehension_clauses(&clauses, &terminal, scope, hir_span_value, out);
        self.temp_operand(dict_local, &ty)
    }

    /// Lower a generator expression into a distinct, deferred [`bir::Rvalue::Generator`].
    ///
    /// The first `for` source is evaluated exactly once at construction, matching the established legacy
    /// iterator-adapter emitter. Its value and every other needed outer lexical value are then captured into fresh
    /// generator-local bindings. Clause polling, later `for` sources, filters, and element evaluation lower only
    /// into the generator body, so the enclosing body neither materializes the sequence nor runs a deferred effect.
    ///
    /// Body IR currently accepts only plain binding patterns for generator clauses. It rejects a whole generator
    /// expression before evaluating its source when another pattern shape would require a partially represented
    /// deferred binding protocol; that keeps unsupported forms visible rather than approximating them as a list.
    pub(super) fn lower_generator_expr(
        &mut self,
        generator: &ast::GeneratorExpr,
        span: ast::Span,
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let hir_span_value = hir_span(span);
        let Some((first_clause, remaining_clauses)) = generator.clauses.split_first() else {
            return self.unsupported_operand(
                "generator expression without a for clause".to_string(),
                scope,
                hir_span_value,
                out,
            );
        };
        let ast::ComprehensionClause::For {
            pattern: first_pattern,
            iter: first_iter,
        } = first_clause
        else {
            return self.unsupported_operand(
                "generator expression whose first clause is not a for clause".to_string(),
                scope,
                hir_span_value,
                out,
            );
        };
        let ast::Pattern::Binding(first_name) = &first_pattern.node else {
            return self.unsupported_operand(
                "generator for-clause pattern is not a simple binding".to_string(),
                scope,
                hir_span_value,
                out,
            );
        };
        if generator.clauses.iter().any(|clause| {
            matches!(clause, ast::ComprehensionClause::For { pattern, .. }
                if !matches!(pattern.node, ast::Pattern::Binding(_)))
        }) {
            return self.unsupported_operand(
                "generator for-clause pattern is not a simple binding".to_string(),
                scope,
                hir_span_value,
                out,
            );
        }

        // The first source is the legacy adapter chain's eager boundary. Lowering it before creating the rvalue
        // preserves source-visible construction timing; all remaining expression lowering below writes only into
        // `generator_stmts` and therefore happens at poll time.
        let first_protocol = self.type_info.protocol_iteration(first_iter.span).cloned();
        let first_is_fallible = first_protocol
            .as_ref()
            .is_some_and(|protocol| protocol.fallible_error_type.is_some());
        let effective_first_iter: &ast::Spanned<ast::Expr> = match (&first_iter.node, first_is_fallible) {
            (ast::Expr::Try(inner), true) => inner,
            _ => first_iter,
        };
        let source = self.lower_expr_to_operand(effective_first_iter, scope, out);

        let generator_scope = self.new_scope(Some(scope), hir_span_value);
        let source_local = self.new_temp(
            self.resolve_ty(effective_first_iter.span),
            generator_scope,
            hir_span_value,
        );
        self.locals[source_local.index()].origin = bir::LocalOrigin::Captured;

        // Capture every lexical value used after the first source once, at construction. The body cannot read the
        // enclosing place directly after this point, and restoring the full binding map below prevents generator
        // clause/capture names from leaking into the following enclosing statement.
        let enclosing_bindings = self.bindings.clone();
        let free_names = free_vars_in_generator_deferred_body(generator);
        let mut captured_operands = Vec::with_capacity(free_names.len());
        let mut capture_locals = Vec::with_capacity(free_names.len());
        for name in &free_names {
            let Some(&outer_local) = self.bindings.get(name) else {
                // Module/external names remain explicit `External` references when the deferred body is lowered;
                // there is no local value available to capture and rebind here.
                continue;
            };
            let outer_ty = self.locals[outer_local.index()].ty.clone();
            let outer_place = bir::Place::from_local(outer_local);
            let (fact, last_use) = self.ownership_fact_for_place(&outer_place, &outer_ty);
            captured_operands.push(bir::Operand::place(outer_place, fact, last_use));

            let total_reads = count_reads_in_generator_deferred_body(name, generator);
            let capture_local =
                self.declare_new_local_with_reads(name.clone(), outer_ty, generator_scope, hir_span_value, total_reads);
            self.locals[capture_local.index()].origin = bir::LocalOrigin::Captured;
            capture_locals.push(capture_local);
        }

        let first_loop_scope = self.new_scope(Some(generator_scope), hir_span_value);
        let first_total_reads = count_reads_in_expr(first_name, &generator.expr.node)
            + count_reads_in_comprehension_clauses(first_name, remaining_clauses);
        let first_local = self.declare_new_local_with_reads(
            first_name.clone(),
            self.resolve_ty(first_pattern.span),
            first_loop_scope,
            hir_span(first_pattern.span),
            first_total_reads,
        );

        let mut generator_stmts = Vec::new();
        let iterator_ty = match &first_protocol {
            Some(protocol) => semantic_type_from_resolved(&protocol.iterator_type),
            None => self.resolve_ty(effective_first_iter.span),
        };
        let iterator_local = self.new_temp(iterator_ty, generator_scope, hir_span_value);
        match &first_protocol {
            Some(protocol) => generator_stmts.push(bir::Statement {
                kind: bir::StatementKind::Call {
                    destination: Some(bir::Place::from_local(iterator_local)),
                    callee: bir::Callee::Method(bir::MethodTarget::synthesized(protocol.iter_method.clone())),
                    args: fixed_elements(vec![bir::Operand::place(
                        bir::Place::from_local(source_local),
                        bir::OwnershipFact::Borrow,
                        false,
                    )]),
                    may_panic: false,
                },
                span: hir_span_value,
            }),
            None => generator_stmts.push(bir::Statement {
                kind: bir::StatementKind::Assign {
                    place: bir::Place::from_local(iterator_local),
                    rvalue: bir::Rvalue::Use(bir::Operand::place(
                        bir::Place::from_local(source_local),
                        bir::OwnershipFact::Borrow,
                        false,
                    )),
                },
                span: hir_span_value,
            }),
        }

        self.loop_break_targets.push(None);
        let mut first_loop_stmts = vec![bir::Statement {
            kind: bir::StatementKind::IterNext {
                destination: bir::Place::from_local(first_local),
                iterator: bir::Operand::place(
                    bir::Place::from_local(iterator_local),
                    bir::OwnershipFact::MutBorrow,
                    false,
                ),
                protocol: match &first_protocol {
                    Some(protocol) => bir::IterProtocol::UserDefined {
                        next_method: protocol.next_method.clone(),
                        fallible: first_is_fallible,
                    },
                    None => bir::IterProtocol::Builtin,
                },
            },
            span: hir_span_value,
        }];
        let terminal = ComprehensionTerminal::GeneratorYield {
            element: &generator.expr,
        };
        self.lower_comprehension_clauses(
            remaining_clauses,
            &terminal,
            first_loop_scope,
            hir_span_value,
            &mut first_loop_stmts,
        );
        self.insert_scope_drops(&mut first_loop_stmts, first_loop_scope);
        self.loop_break_targets.pop();
        generator_stmts.push(bir::Statement {
            kind: bir::StatementKind::Loop {
                body: bir::Block {
                    scope: first_loop_scope,
                    stmts: first_loop_stmts,
                },
            },
            span: hir_span_value,
        });
        self.bindings = enclosing_bindings;

        // `Generator::new` owns a boxed iterator in the legacy runtime, even when every captured source value is
        // Copy-shaped. Record that allocation fact directly rather than relying on incidental temporary locals.
        self.record_runtime_requirement(AbiV0RuntimeRequirement::Allocator);
        let ty = self.resolve_ty(span);
        self.push_assign_temp(
            bir::Rvalue::Generator {
                source,
                captured_operands,
                body: Box::new(bir::GeneratorBody {
                    source_local,
                    capture_locals,
                    stmts: generator_stmts,
                }),
            },
            ty,
            scope,
            hir_span_value,
            out,
        )
    }

    /// Lower a comprehension/generator clause chain with bindings that are lexical to that expression. The clause
    /// lowering itself declares each `for` pattern binding through [`Self::declare_new_local_with_reads`] so normal
    /// operand lowering can resolve it. Those bindings must disappear when the expression ends, however: unlike a
    /// statement `for`, a comprehension's `x` in `[x for x in values]` cannot shadow an enclosing `x` in the next
    /// enclosing statement. Preserve the outer lookup map while retaining the locals and ownership facts the nested
    /// lowering legitimately recorded in the Body IR.
    pub(super) fn lower_scoped_comprehension_clauses(
        &mut self,
        clauses: &[ast::ComprehensionClause],
        terminal: &ComprehensionTerminal<'_>,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let enclosing_bindings = self.bindings.clone();
        self.lower_comprehension_clauses(clauses, terminal, scope, span, out);
        self.bindings = enclosing_bindings;
    }

    /// Recursively desugar a comprehension/generator clause chain into nested `Loop`/`If` statements, terminating
    /// in `terminal`'s compiler-synthesized collection-growth call once every clause has been satisfied for one
    /// binding combination. `For` clauses reuse [`Self::lower_general_iteration`] (the same builtin-vs-protocol
    /// iteration primitive [`Self::lower_for`] uses), so comprehensions never duplicate that split. A non-binding
    /// `For` clause pattern lowers to `Unsupported`, matching [`Self::lower_for`]'s own restriction (destructuring
    /// patterns need `match`-shaped compilation, out of scope here).
    pub(super) fn lower_comprehension_clauses(
        &mut self,
        clauses: &[ast::ComprehensionClause],
        terminal: &ComprehensionTerminal<'_>,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) {
        let Some((head, tail)) = clauses.split_first() else {
            self.lower_comprehension_terminal(terminal, scope, out);
            return;
        };
        match head {
            ast::ComprehensionClause::If(cond) => {
                let cond_operand = self.lower_expr_to_operand(cond, scope, out);
                let then_scope = self.new_scope(Some(scope), span);
                let mut then_stmts = Vec::new();
                self.lower_comprehension_clauses(tail, terminal, then_scope, span, &mut then_stmts);
                out.push(bir::Statement {
                    kind: bir::StatementKind::If {
                        cond: cond_operand,
                        then_block: bir::Block {
                            scope: then_scope,
                            stmts: then_stmts,
                        },
                        else_block: None,
                    },
                    span,
                });
            }
            ast::ComprehensionClause::For { pattern, iter } => {
                let ast::Pattern::Binding(var_name) = &pattern.node else {
                    self.push_unsupported_stmt(
                        "comprehension for-clause pattern is not a simple binding".to_string(),
                        span,
                        out,
                    );
                    return;
                };
                let var_ty = self.resolve_ty(pattern.span);
                let loop_scope = self.new_scope(Some(scope), span);
                let total_reads = terminal.count_reads(var_name) + count_reads_in_comprehension_clauses(var_name, tail);
                let pattern_local =
                    self.declare_new_local_with_reads(var_name.clone(), var_ty, loop_scope, span, total_reads);
                self.lower_general_iteration(
                    iter,
                    pattern_local,
                    scope,
                    loop_scope,
                    span,
                    out,
                    move |builder, loop_scope, body_stmts| {
                        builder.lower_comprehension_clauses(tail, terminal, loop_scope, span, body_stmts);
                        builder.insert_scope_drops(body_stmts, loop_scope);
                    },
                );
            }
        }
    }

    /// Lower the innermost action of one accepted comprehension/generator binding combination: evaluate the
    /// element (or key/value) expression(s) and push a compiler-synthesized `push`/`insert`
    /// [`bir::Callee::Method`] call growing the target collection. The receiver is read as
    /// [`bir::OwnershipFact::MutBorrow`] since the call mutates the collection in place -- the first real producer
    /// of that fact in this module (every other place read so far has been `Copy`/`Move`/`Clone`/`Borrow`).
    pub(super) fn lower_comprehension_terminal(
        &mut self,
        terminal: &ComprehensionTerminal<'_>,
        scope: bir::ScopeId,
        out: &mut Vec<bir::Statement>,
    ) {
        match terminal {
            ComprehensionTerminal::ListPush { list_local, element } => {
                let element_operand = self.lower_expr_to_operand(element, scope, out);
                let span = hir_span(element.span);
                out.push(bir::Statement {
                    kind: bir::StatementKind::Call {
                        destination: None,
                        callee: bir::Callee::Method(bir::MethodTarget::synthesized("push")),
                        args: fixed_elements(vec![
                            bir::Operand::place(
                                bir::Place::from_local(*list_local),
                                bir::OwnershipFact::MutBorrow,
                                false,
                            ),
                            element_operand,
                        ]),
                        may_panic: false,
                    },
                    span,
                });
            }
            ComprehensionTerminal::DictInsert { dict_local, key, value } => {
                let key_operand = self.lower_expr_to_operand(key, scope, out);
                let value_operand = self.lower_expr_to_operand(value, scope, out);
                let span = hir_span(value.span);
                out.push(bir::Statement {
                    kind: bir::StatementKind::Call {
                        destination: None,
                        callee: bir::Callee::Method(bir::MethodTarget::synthesized("insert")),
                        args: fixed_elements(vec![
                            bir::Operand::place(
                                bir::Place::from_local(*dict_local),
                                bir::OwnershipFact::MutBorrow,
                                false,
                            ),
                            key_operand,
                            value_operand,
                        ]),
                        may_panic: false,
                    },
                    span,
                });
            }
            ComprehensionTerminal::GeneratorYield { element } => {
                let value = self.lower_expr_to_operand(element, scope, out);
                out.push(bir::Statement {
                    kind: bir::StatementKind::Yield { value },
                    span: hir_span(element.span),
                });
            }
        }
    }
}
