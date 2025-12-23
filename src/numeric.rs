//! Numeric policy: single source of truth for Incan's Python-like numeric semantics.
//!
//! This module defines the rules for:
//! - Division (`/`): always yields `Float` (even `int / int`)
//! - Modulo (`%`): supports floats; uses Python remainder semantics
//! - Power (`**`): `int ** int` yields `Int` only for non-negative int literal exponents; otherwise `Float`
//! - Mixed numeric comparisons: promote to `Float` when comparing `Int` and `Float`
//!
//! Both frontend (typechecker, const-eval) and backend (lowering, conversions, emitter) use these
//! helpers to ensure consistent behavior across the entire pipeline.

/// Simplified numeric type for policy decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericTy {
    Int,
    Float,
}

/// Numeric operators subject to promotion/coercion rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericOp {
    Add,
    Sub,
    Mul,
    Div,
    /// `//` (Python-style floor division): returns `Int` for `Int // Int`, otherwise `Float`.
    FloorDiv,
    Mod,
    Pow,
    // Comparisons (for coercion, not result type)
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

/// Context for power operator literal detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowExponentKind {
    /// A non-negative integer literal (e.g., `2`, `0`)
    NonNegativeIntLiteral,
    /// A negative integer literal (e.g., `-1`)
    NegativeIntLiteral,
    /// A variable or non-literal expression
    Variable,
    /// A float literal or expression
    Float,
}

impl PowExponentKind {
    /// Determine the exponent kind from an optional integer literal value.
    ///
    /// ## Parameters
    /// - `rhs_is_float`: whether the right operand is a float type
    /// - `rhs_int_literal`: if the right operand is an int literal, its value
    pub fn from_literal_info(rhs_is_float: bool, rhs_int_literal: Option<i64>) -> Self {
        if rhs_is_float {
            PowExponentKind::Float
        } else if let Some(val) = rhs_int_literal {
            if val >= 0 {
                PowExponentKind::NonNegativeIntLiteral
            } else {
                PowExponentKind::NegativeIntLiteral
            }
        } else {
            PowExponentKind::Variable
        }
    }
}

/// Determine the result type for a numeric binary operation.
///
/// ## Parameters
///
/// - `op`: the numeric operator
/// - `lhs`: left operand type
/// - `rhs`: right operand type
/// - `pow_exp_kind`: for `Pow`, describes whether exponent is a non-negative int literal, etc.
///
/// ## Returns
///
/// The result type (`Int` or `Float`).
///
/// ## Rules (Python-like)
///
/// - `/`: always `Float`
/// - `%`: `Float` if either operand is `Float`, else `Int`
/// - `**`: `Int` only if both operands are `Int` AND exponent is a non-negative int literal; else `Float`
/// - `+`, `-`, `*`: `Float` if either operand is `Float`, else `Int`
/// - Comparisons: result is always `Bool` (not handled here), but operands may need promotion
pub fn result_numeric_type(
    op: NumericOp,
    lhs: NumericTy,
    rhs: NumericTy,
    pow_exp_kind: Option<PowExponentKind>,
) -> NumericTy {
    match op {
        NumericOp::Div => NumericTy::Float,

        // FloorDiv: returns int when both are int, float when either is float
        NumericOp::FloorDiv | NumericOp::Mod | NumericOp::Add | NumericOp::Sub | NumericOp::Mul => {
            if lhs == NumericTy::Float || rhs == NumericTy::Float {
                NumericTy::Float
            } else {
                NumericTy::Int
            }
        }

        NumericOp::Pow => {
            // Int result only when: both operands Int AND exponent is non-negative int literal
            if lhs == NumericTy::Int && rhs == NumericTy::Int {
                match pow_exp_kind {
                    Some(PowExponentKind::NonNegativeIntLiteral) => NumericTy::Int,
                    _ => NumericTy::Float,
                }
            } else {
                NumericTy::Float
            }
        }

        // Comparisons don't produce numeric results, but this function is about operand types
        // so we return Float if either side is Float (for coercion purposes).
        NumericOp::Eq | NumericOp::NotEq | NumericOp::Lt | NumericOp::LtEq | NumericOp::Gt | NumericOp::GtEq => {
            if lhs == NumericTy::Float || rhs == NumericTy::Float {
                NumericTy::Float
            } else {
                NumericTy::Int
            }
        }
    }
}

/// Determine what promotions are needed for a numeric binary operation.
///
/// ## Returns
///
/// `(lhs_to_float, rhs_to_float)` - whether each operand needs to be promoted to `f64`.
pub fn needs_float_promotion(
    op: NumericOp,
    lhs: NumericTy,
    rhs: NumericTy,
    pow_exp_kind: Option<PowExponentKind>,
) -> (bool, bool) {
    let result_ty = result_numeric_type(op, lhs, rhs, pow_exp_kind);

    if result_ty == NumericTy::Float {
        (lhs == NumericTy::Int, rhs == NumericTy::Int)
    } else {
        (false, false)
    }
}

/// Check if a binary operator is a numeric arithmetic operator.
pub fn is_numeric_arithmetic_op(op: NumericOp) -> bool {
    matches!(
        op,
        NumericOp::Add
            | NumericOp::Sub
            | NumericOp::Mul
            | NumericOp::Div
            | NumericOp::FloorDiv
            | NumericOp::Mod
            | NumericOp::Pow
    )
}

/// Check if a binary operator is a numeric comparison.
pub fn is_numeric_comparison_op(op: NumericOp) -> bool {
    matches!(
        op,
        NumericOp::Eq | NumericOp::NotEq | NumericOp::Lt | NumericOp::LtEq | NumericOp::Gt | NumericOp::GtEq
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_div_always_float() {
        assert_eq!(
            result_numeric_type(NumericOp::Div, NumericTy::Int, NumericTy::Int, None),
            NumericTy::Float
        );
        assert_eq!(
            result_numeric_type(NumericOp::Div, NumericTy::Int, NumericTy::Float, None),
            NumericTy::Float
        );
        assert_eq!(
            result_numeric_type(NumericOp::Div, NumericTy::Float, NumericTy::Int, None),
            NumericTy::Float
        );
        assert_eq!(
            result_numeric_type(NumericOp::Div, NumericTy::Float, NumericTy::Float, None),
            NumericTy::Float
        );
    }

    #[test]
    fn test_mod_promotion() {
        assert_eq!(
            result_numeric_type(NumericOp::Mod, NumericTy::Int, NumericTy::Int, None),
            NumericTy::Int
        );
        assert_eq!(
            result_numeric_type(NumericOp::Mod, NumericTy::Int, NumericTy::Float, None),
            NumericTy::Float
        );
        assert_eq!(
            result_numeric_type(NumericOp::Mod, NumericTy::Float, NumericTy::Int, None),
            NumericTy::Float
        );
    }

    #[test]
    fn test_pow_literal_exponent() {
        // Non-negative int literal exponent → Int result
        assert_eq!(
            result_numeric_type(
                NumericOp::Pow,
                NumericTy::Int,
                NumericTy::Int,
                Some(PowExponentKind::NonNegativeIntLiteral)
            ),
            NumericTy::Int
        );
        // Negative int literal → Float result
        assert_eq!(
            result_numeric_type(
                NumericOp::Pow,
                NumericTy::Int,
                NumericTy::Int,
                Some(PowExponentKind::NegativeIntLiteral)
            ),
            NumericTy::Float
        );
        // Variable exponent → Float result
        assert_eq!(
            result_numeric_type(
                NumericOp::Pow,
                NumericTy::Int,
                NumericTy::Int,
                Some(PowExponentKind::Variable)
            ),
            NumericTy::Float
        );
        // Float exponent → Float result
        assert_eq!(
            result_numeric_type(
                NumericOp::Pow,
                NumericTy::Int,
                NumericTy::Float,
                Some(PowExponentKind::Float)
            ),
            NumericTy::Float
        );
    }

    #[test]
    fn test_add_sub_mul_promotion() {
        assert_eq!(
            result_numeric_type(NumericOp::Add, NumericTy::Int, NumericTy::Int, None),
            NumericTy::Int
        );
        assert_eq!(
            result_numeric_type(NumericOp::Add, NumericTy::Int, NumericTy::Float, None),
            NumericTy::Float
        );
        assert_eq!(
            result_numeric_type(NumericOp::Sub, NumericTy::Float, NumericTy::Int, None),
            NumericTy::Float
        );
        assert_eq!(
            result_numeric_type(NumericOp::Mul, NumericTy::Float, NumericTy::Float, None),
            NumericTy::Float
        );
    }

    #[test]
    fn test_needs_float_promotion() {
        // Div always promotes ints
        assert_eq!(
            needs_float_promotion(NumericOp::Div, NumericTy::Int, NumericTy::Int, None),
            (true, true)
        );
        assert_eq!(
            needs_float_promotion(NumericOp::Div, NumericTy::Float, NumericTy::Int, None),
            (false, true)
        );

        // Add with mixed types
        assert_eq!(
            needs_float_promotion(NumericOp::Add, NumericTy::Int, NumericTy::Float, None),
            (true, false)
        );
        assert_eq!(
            needs_float_promotion(NumericOp::Add, NumericTy::Int, NumericTy::Int, None),
            (false, false)
        );

        // Pow with non-negative literal
        assert_eq!(
            needs_float_promotion(
                NumericOp::Pow,
                NumericTy::Int,
                NumericTy::Int,
                Some(PowExponentKind::NonNegativeIntLiteral)
            ),
            (false, false)
        );
        // Pow with variable exponent
        assert_eq!(
            needs_float_promotion(
                NumericOp::Pow,
                NumericTy::Int,
                NumericTy::Int,
                Some(PowExponentKind::Variable)
            ),
            (true, true)
        );
    }
}
