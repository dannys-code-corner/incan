//! Check unary and binary operators.
//!
//! These helpers validate operator semantics (e.g., numeric ops, boolean ops) and compute the
//! resulting type, emitting diagnostics on mismatches.

use crate::frontend::ast::*;
use crate::frontend::diagnostics::errors;
use crate::frontend::symbols::ResolvedType;

use super::TypeChecker;

impl TypeChecker {
    /// Type-check a binary operation and return its result type.
    pub(in crate::frontend::typechecker::check_expr) fn check_binary(
        &mut self,
        left: &Spanned<Expr>,
        op: BinaryOp,
        right: &Spanned<Expr>,
        span: Span,
    ) -> ResolvedType {
        let left_ty = self.check_expr(left);
        let right_ty = self.check_expr(right);

        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Pow => {
                // Numeric operations
                if self.types_compatible(&left_ty, &ResolvedType::Int)
                    && self.types_compatible(&right_ty, &ResolvedType::Int)
                {
                    ResolvedType::Int
                } else if self.types_compatible(&left_ty, &ResolvedType::Float)
                    || self.types_compatible(&right_ty, &ResolvedType::Float)
                {
                    ResolvedType::Float
                } else if matches!(op, BinaryOp::Add) && self.types_compatible(&left_ty, &ResolvedType::Str) {
                    ResolvedType::Str
                } else {
                    self.errors.push(errors::type_mismatch(
                        "numeric",
                        &format!("{} {} {}", left_ty, op, right_ty),
                        span,
                    ));
                    ResolvedType::Unknown
                }
            }
            BinaryOp::Eq | BinaryOp::NotEq => ResolvedType::Bool,
            BinaryOp::Lt | BinaryOp::Gt | BinaryOp::LtEq | BinaryOp::GtEq => ResolvedType::Bool,
            BinaryOp::And | BinaryOp::Or => ResolvedType::Bool,
            BinaryOp::In | BinaryOp::NotIn => ResolvedType::Bool,
            BinaryOp::Is => ResolvedType::Bool,
        }
    }

    /// Type-check a unary operation and return its result type.
    pub(in crate::frontend::typechecker::check_expr) fn check_unary(
        &mut self,
        op: UnaryOp,
        operand: &Spanned<Expr>,
        span: Span,
    ) -> ResolvedType {
        let operand_ty = self.check_expr(operand);
        match op {
            UnaryOp::Neg => {
                if self.types_compatible(&operand_ty, &ResolvedType::Int) {
                    ResolvedType::Int
                } else if self.types_compatible(&operand_ty, &ResolvedType::Float) {
                    ResolvedType::Float
                } else {
                    self.errors
                        .push(errors::type_mismatch("numeric", &operand_ty.to_string(), span));
                    ResolvedType::Unknown
                }
            }
            UnaryOp::Not => {
                if !self.types_compatible(&operand_ty, &ResolvedType::Bool) {
                    self.errors
                        .push(errors::type_mismatch("bool", &operand_ty.to_string(), span));
                }
                ResolvedType::Bool
            }
        }
    }
}
