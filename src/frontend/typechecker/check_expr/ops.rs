//! Check unary and binary operators.
//!
//! These helpers validate operator semantics (e.g., numeric ops, boolean ops) and compute the resulting type, emitting
//! diagnostics on mismatches.
//!
//! Numeric semantics follow Python-like rules:
//!
//! - `/` always yields `Float` (even `int / int`)
//! - `%` supports floats with Python remainder semantics
//! - `**` yields `Int` only for non-negative int literal exponents; otherwise `Float`
//! - Mixed numeric comparisons are allowed (promote to float for comparison)

use crate::frontend::ast::*;
use crate::frontend::diagnostics::errors;
use crate::frontend::symbols::ResolvedType;
use crate::numeric::{NumericTy, result_numeric_type};
use crate::numeric_adapters::{numeric_op_from_ast, numeric_ty_from_resolved, pow_exponent_kind_from_ast};

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
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::FloorDiv
            | BinaryOp::Mod
            | BinaryOp::Pow => {
                // String concatenation special case
                if matches!(op, BinaryOp::Add) && self.types_compatible(&left_ty, &ResolvedType::Str) {
                    return ResolvedType::Str;
                }

                // Check both operands are numeric
                let lhs_num = numeric_ty_from_resolved(&left_ty);
                let rhs_num = numeric_ty_from_resolved(&right_ty);

                match (lhs_num, rhs_num) {
                    (Some(lhs), Some(rhs)) => {
                        let num_op = numeric_op_from_ast(&op).expect("INVARIANT: arithmetic op");
                        let pow_exp = if matches!(op, BinaryOp::Pow) {
                            Some(pow_exponent_kind_from_ast(right, &right_ty))
                        } else {
                            None
                        };
                        let result = result_numeric_type(num_op, lhs, rhs, pow_exp);
                        match result {
                            NumericTy::Int => ResolvedType::Int,
                            NumericTy::Float => ResolvedType::Float,
                        }
                    }
                    _ => {
                        self.errors.push(errors::type_mismatch(
                            "numeric",
                            &format!("{} {} {}", left_ty, op, right_ty),
                            span,
                        ));
                        ResolvedType::Unknown
                    }
                }
            }
            // Comparisons: allow mixed numeric types (promote for comparison), result is Bool
            BinaryOp::Eq | BinaryOp::NotEq | BinaryOp::Lt | BinaryOp::Gt | BinaryOp::LtEq | BinaryOp::GtEq => {
                // If both are numeric, allow mixed comparisons
                let lhs_num = numeric_ty_from_resolved(&left_ty);
                let rhs_num = numeric_ty_from_resolved(&right_ty);
                if lhs_num.is_some() && rhs_num.is_some() {
                    // Mixed numeric comparison is valid (promotion handled at codegen)
                    ResolvedType::Bool
                } else if left_ty == right_ty || self.types_compatible(&left_ty, &right_ty) {
                    // Same-type or compatible comparison
                    ResolvedType::Bool
                } else {
                    // Different non-numeric types
                    self.errors.push(errors::type_mismatch(
                        &format!("comparable to {}", left_ty),
                        &right_ty.to_string(),
                        span,
                    ));
                    ResolvedType::Bool
                }
            }
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
