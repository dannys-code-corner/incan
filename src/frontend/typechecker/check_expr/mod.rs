//! Check expressions and resolve their types.
//!
//! This module owns the expression-checking entrypoint (`check_expr`) and delegates to themed
//! submodules for maintainability. Expression checking is error-accumulating: on invalid input it
//! returns [`ResolvedType::Unknown`] so later checks can continue.
//!
//! ## See also
//! - [`super::TypeChecker`]: the main type checker entrypoint.

use crate::frontend::ast::*;
use crate::frontend::symbols::ResolvedType;

use super::TypeChecker;

mod access;
mod basics;
mod calls;
mod collections;
mod comps;
mod control_flow;
mod match_;
mod ops;

impl TypeChecker {
    // ========================================================================
    // Expressions
    // ========================================================================

    /// Validate an expression and return its resolved type.
    ///
    /// Dispatches to specialized helpers (`check_call`, `check_binary`, `check_match`, etc.)
    /// and accumulates errors. Returns [`ResolvedType::Unknown`] when the expression is
    /// invalid so checking can continue.
    pub(crate) fn check_expr(&mut self, expr: &Spanned<Expr>) -> ResolvedType {
        let ty = match &expr.node {
            Expr::Ident(name) => self.check_ident(name, expr.span),
            Expr::Literal(lit) => self.check_literal(lit),
            Expr::SelfExpr => self.check_self(expr.span),
            Expr::Binary(left, op, right) => self.check_binary(left, *op, right, expr.span),
            Expr::Unary(op, operand) => self.check_unary(*op, operand, expr.span),
            Expr::Call(callee, args) => self.check_call(callee, args, expr.span),
            Expr::Index(base, index) => self.check_index(base, index, expr.span),
            Expr::Slice(base, slice) => self.check_slice(base, slice, expr.span),
            Expr::Field(base, field) => self.check_field(base, field, expr.span),
            Expr::MethodCall(base, method, args) => self.check_method_call(base, method, args, expr.span),
            Expr::Await(inner) => self.check_await(inner, expr.span),
            Expr::Try(inner) => self.check_try(inner, expr.span),
            Expr::Match(subject, arms) => self.check_match(subject, arms, expr.span),
            Expr::If(if_expr) => self.check_if_expr(if_expr, expr.span),
            Expr::ListComp(comp) => self.check_list_comp(comp, expr.span),
            Expr::DictComp(comp) => self.check_dict_comp(comp, expr.span),
            Expr::Closure(params, body) => self.check_closure(params, body, expr.span),
            Expr::Tuple(elems) => self.check_tuple(elems),
            Expr::List(elems) => self.check_list(elems),
            Expr::Dict(entries) => self.check_dict(entries),
            Expr::Set(elems) => self.check_set(elems),
            Expr::Paren(inner) => self.check_expr(inner),
            Expr::Constructor(name, args) => self.check_constructor(name, args, expr.span),
            Expr::FString(parts) => {
                for part in parts {
                    if let FStringPart::Expr(e) = part {
                        self.check_expr(e);
                    }
                }
                ResolvedType::Str
            }
            Expr::Yield(inner) => {
                // Yield returns the type of its inner expression, or Unit
                if let Some(inner) = inner {
                    self.check_expr(inner)
                } else {
                    ResolvedType::Unit
                }
            }
            Expr::Range {
                start,
                end,
                inclusive: _,
            } => self.check_range_expr(start, end),
        };

        // Record for downstream stages (lowering/codegen).
        self.record_expr_type(expr.span, ty.clone());
        ty
    }
}
