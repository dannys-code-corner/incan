//! Lowering for binary operators, including dispatch to a user-defined operator method.

use super::*;

impl<'type_info, 'source> BodyBuilder<'type_info, 'source> {
    /// Lower a binary-operator expression. Bails out to an explicit unsupported placeholder *before* evaluating
    /// either operand when `op` has no Body IR v0 handling at all (see [`Self::binary_op_is_supported`]), so an
    /// unsupported operator's sub-expressions are never partially lowered. Otherwise defers to
    /// [`Self::lower_binary_from_operands`] for the actual string-helper-or-plain-binop emission, which is also
    /// shared with [`Self::lower_compound_assignment`].
    pub(super) fn lower_binary(
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

        // A user-defined operator is a method call, not a primitive operation. The typechecker already resolved
        // which dunder this spelling dispatches to, so lowering follows that decision rather than falling through
        // to the primitive operator set -- which would represent `a + b` on two `Vec2` values as machine addition.
        if let Some(dispatch) = self.type_info.resolved_operator_call(span)
            && dispatch.kind == ResolvedOperatorKind::Binary
        {
            let method = dispatch.method.clone();
            return self.lower_operator_dispatch(&method, lhs, rhs, result_ty, scope, hir_span_value, out);
        }

        if !Self::binary_op_is_supported(op, &lhs_ty, &rhs_ty) {
            return self.unsupported_operand(format!("binary operator {op:?}"), scope, hir_span_value, out);
        }
        let lhs_operand = self.lower_expr_to_operand(lhs, scope, out);
        let rhs_operand = self.lower_expr_to_operand(rhs, scope, out);
        self.lower_binary_from_operands(
            op,
            &lhs_ty,
            lhs_operand,
            &rhs_ty,
            rhs_operand,
            result_ty,
            scope,
            hir_span_value,
            out,
        )
    }

    /// Whether `op` between operands of `lhs_ty`/`rhs_ty` has any Body IR v0 handling (either the string-helper
    /// path or a direct [`bir::BinOp`] mapping). Checked *before* evaluating operand sub-expressions in both
    /// [`Self::lower_binary`] and [`Self::lower_compound_assignment`], so an operator v0 does not model never
    /// causes its operands' side effects (calls, reads) to be lowered on the way to an unsupported placeholder.
    /// Lower a user-defined operator to the dunder method call the typechecker resolved for it.
    ///
    /// RFC 028 lets a type define `__add__`, `__and__`, `__contains__` and friends, and the typechecker records
    /// which method one operator spelling dispatches to. Body IR must follow that decision: representing `a + b` on
    /// two `Vec2` values as [`bir::BinOp::Add`] would claim a primitive machine operation where the source calls a
    /// method, which is a wrong representation rather than an honest refusal — no `Unsupported` marker, nothing for
    /// a consumer to notice.
    ///
    /// The left operand becomes the receiver and the right becomes the single argument, matching how
    /// [`Self::lower_method_call`] arranges an ordinary method call: `args[0]` is the receiver, borrowed. The
    /// binding is [`bir::ArgumentBinding::UnresolvedPositional`] because an operator spelling names no parameter
    /// and this stage resolves no declared slot for it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_operator_dispatch(
        &mut self,
        method: &str,
        lhs: &ast::Spanned<ast::Expr>,
        rhs: &ast::Spanned<ast::Expr>,
        result_ty: IncanType,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        // Source evaluation observes the receiver before the argument, exactly as for a written method call.
        let receiver_place = self.lower_expr_to_place(lhs, scope, out);
        let receiver = bir::Operand::place(receiver_place, bir::OwnershipFact::Borrow, false);
        let argument = self.lower_expr_to_operand(rhs, scope, out);
        self.push_call_temp(
            bir::Callee::Method(bir::MethodTarget::synthesized(method)),
            vec![bir::ArgumentElement::One(receiver), bir::ArgumentElement::One(argument)],
            result_ty,
            scope,
            span,
            false,
            out,
        )
    }

    pub(super) fn binary_op_is_supported(op: ast::BinaryOp, lhs_ty: &IncanType, rhs_ty: &IncanType) -> bool {
        (is_string_like(lhs_ty) && is_string_like(rhs_ty) && string_helper_for_binop(op).is_some())
            || lower_binary_op(op).is_some()
    }

    /// Emit the result of a binary operator given already-lowered operands: an explicit [`bir::Callee::Helper`]
    /// call (with runtime requirements recorded) when both operand types are string-like and `op` has a
    /// compiler-owned string helper (see [`string_helper_for_binop`]) -- Body IR's compiler-owned-runtime-operation
    /// requirement (#653 criterion 3) applied to string operators specifically -- otherwise a plain
    /// [`bir::Rvalue::BinaryOp`], with a division/modulo panic fact recorded when [`bir::BinOp::may_panic`] holds.
    /// Callers are expected to have already checked [`Self::binary_op_is_supported`]; an operator with neither
    /// handling still falls back to an explicit unsupported placeholder defensively rather than panicking.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_binary_from_operands(
        &mut self,
        op: ast::BinaryOp,
        lhs_ty: &IncanType,
        lhs_operand: bir::Operand,
        rhs_ty: &IncanType,
        rhs_operand: bir::Operand,
        result_ty: IncanType,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        if is_string_like(lhs_ty)
            && is_string_like(rhs_ty)
            && let Some(helper) = string_helper_for_binop(op)
        {
            self.record_runtime_requirement(AbiV0RuntimeRequirement::RuntimeHelper(helper.as_str().to_string()));
            self.record_runtime_requirement(AbiV0RuntimeRequirement::Allocator);
            return self.push_call_temp(
                bir::Callee::Helper(helper),
                fixed_elements(vec![lhs_operand, rhs_operand]),
                result_ty,
                scope,
                span,
                false,
                out,
            );
        }

        let Some(bin_op) = lower_binary_op(op) else {
            return self.unsupported_operand(format!("binary operator {op:?}"), scope, span, out);
        };
        if bin_op.may_panic() {
            self.panic_facts.push(bir::PanicFact {
                span,
                reason: bir::PanicReason::DivisionOrModulo,
            });
            self.record_runtime_requirement(AbiV0RuntimeRequirement::PanicStrategy);
        }
        self.push_assign_temp(
            bir::Rvalue::BinaryOp(bin_op, lhs_operand, rhs_operand),
            result_ty,
            scope,
            span,
            out,
        )
    }
}
