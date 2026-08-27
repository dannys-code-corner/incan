//! Lowering for `assert` forms and their explicit panic facts.

use super::*;

impl<'type_info, 'source> BodyBuilder<'type_info, 'source> {
    /// Lower `assert cond[, message]`, recording an [`bir::PanicReason::AssertFailure`] panic fact and a
    /// [`AbiV0RuntimeRequirement::PanicStrategy`] runtime requirement since every assert can panic. The pattern
    /// (`assert value is Some(name)`) and `raises` (`assert call() raises E`) forms are not modeled by v0 and lower
    /// to an explicit unsupported placeholder instead (#1167). The pattern form's placeholder is lossy rather than
    /// merely incomplete: it discards the names the pattern would bind, so a later read of one lowers against a
    /// local this body never declared.
    pub(super) fn lower_assert(
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

    // ---- Callable defaults ----
}
