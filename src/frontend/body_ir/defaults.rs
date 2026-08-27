//! Lowering a declared parameter default into the form a direct consumer can supply for an omitted argument.

use super::*;

impl<'type_info, 'source> BodyBuilder<'type_info, 'source> {
    /// Lower one source-declared default into a deferred Body-IR computation.
    ///
    /// The ordinary function body may not contain this computation: source defaults run only when the matching
    /// parameter is omitted. While lowering it, callable-local bindings are hidden because the legacy path
    /// materializes source defaults while assembling call arguments, before the callee frame is bound. A default
    /// therefore becomes a closed Body-IR computation or a tagged refusal: a callable-local or other external
    /// source read, every explicitly unsupported Body-IR form, and a default without a usable canonical type fact
    /// refuse at the default expression's own span. The final condition is deliberately fail-closed: Body IR may
    /// not make an unchecked source default executable by reconstructing source semantics. This leaves a direct
    /// consumer no reason to consult AST/HIR/typechecker state or legacy execution.
    pub(super) fn lower_callable_default(
        &mut self,
        default_expr: Option<&ast::Spanned<ast::Expr>>,
        scope: bir::ScopeId,
    ) -> bir::CallableParamDefault {
        let Some(default_expr) = default_expr else {
            return bir::CallableParamDefault::Required;
        };

        let locals_len = self.locals.len();
        let scopes_len = self.scopes.len();
        let runtime_requirements_len = self.runtime_requirements.len();
        let panic_facts_len = self.panic_facts.len();
        let next_local = self.next_local;
        let next_scope = self.next_scope;
        let saved_remaining_reads = self.remaining_reads.clone();
        let saved_moved_out = self.moved_out.clone();
        let saved_bindings = std::mem::take(&mut self.bindings);
        let saved_external_locals = std::mem::take(&mut self.external_locals);
        let mut stmts = Vec::new();
        let result = self.lower_expr_to_operand(default_expr, scope, &mut stmts);
        let mut unresolved_names: Vec<String> = self.external_locals.keys().cloned().collect();
        unresolved_names.sort();
        self.bindings = saved_bindings;
        self.external_locals = saved_external_locals;

        let refusal = first_unsupported_default_statement(&stmts)
            .or_else(|| {
                (!unresolved_names.is_empty()).then(|| {
                    (
                        hir_span(default_expr.span),
                        format!(
                            "default reads Body-IR-external name(s): {}",
                            unresolved_names.join(", ")
                        ),
                    )
                })
            })
            .or_else(|| {
                self.type_info
                    .validated_newtype_coercion(default_expr.span)
                    .is_some()
                    .then(|| {
                        (
                            hir_span(default_expr.span),
                            "default requires a validated-newtype coercion Body IR does not yet represent".to_string(),
                        )
                    })
            })
            .or_else(|| {
                matches!(
                    self.resolve_ty(default_expr.span),
                    IncanType::Unknown | IncanType::Never
                )
                .then(|| {
                    (
                        hir_span(default_expr.span),
                        "default expression lacks a usable typecheck fact".to_string(),
                    )
                })
            });
        if let Some((span, description)) = refusal {
            self.locals.truncate(locals_len);
            self.scopes.truncate(scopes_len);
            self.runtime_requirements.truncate(runtime_requirements_len);
            self.panic_facts.truncate(panic_facts_len);
            self.next_local = next_local;
            self.next_scope = next_scope;
            self.remaining_reads = saved_remaining_reads;
            self.moved_out = saved_moved_out;
            return bir::CallableParamDefault::Unsupported { span, description };
        }

        bir::CallableParamDefault::Source(bir::DefaultComputation {
            span: hir_span(default_expr.span),
            stmts,
            result,
        })
    }

    // ---- Expressions ----
}
