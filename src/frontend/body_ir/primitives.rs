//! Narrow queries over checked types and literals that lowering maps to Body IR primitives.

use super::*;

/// Return the checked payload types of an intrinsic `Result[ok, error]` carrier.
///
/// This is deliberately a narrow query over the typechecker-owned semantic type. The Body-IR lowerer uses it only
/// to retain facts which direct execution cannot reconstruct: which intrinsic constructor is being formed, which
/// pattern payload is being bound, and whether `?` preserves the enclosing error type exactly. It does not infer
/// a conversion or admit a differently shaped generic carrier.
pub(super) fn result_type_parts(ty: &IncanType) -> Option<(&IncanType, &IncanType)> {
    let IncanType::Generic { base, args } = ty else {
        return None;
    };
    (collections::from_str(base) == Some(CollectionTypeId::Result)).then_some(())?;
    match args.as_slice() {
        [ok_type, error_type] => Some((ok_type, error_type)),
        _ => None,
    }
}
/// Return just the checked error channel for an intrinsic `Result` carrier.
pub(super) fn result_error_type(ty: &IncanType) -> Option<&IncanType> {
    result_type_parts(ty).map(|(_, error_type)| error_type)
}
/// Map only the compiler-owned intrinsic constructor spellings to Body-IR result variants.
pub(super) fn result_variant_kind(name: &str) -> Option<bir::ResultVariantKind> {
    match constructors::from_str(name) {
        Some(ConstructorId::Ok) => Some(bir::ResultVariantKind::Ok),
        Some(ConstructorId::Err) => Some(bir::ResultVariantKind::Err),
        _ => None,
    }
}
/// Whether a type is string-like enough to route binary operators through the compiler-owned string helpers
/// (mirrors `is_string_like_type` in `src/backend/ir/conversions.rs`, restated here so Body IR does not depend on
/// that Rust-emission-specific module — see this file's module docs).
pub(super) fn is_string_like(ty: &IncanType) -> bool {
    matches!(
        ty,
        IncanType::Primitive(IncanPrimitiveType::Str | IncanPrimitiveType::FrozenStr)
    )
}
/// Map a string-typed binary operator to its compiler-owned helper operation, or `None` for operators that have no
/// string-specific helper (arithmetic-only operators never reach here because `lower_binary` only checks this for
/// string-like operand types).
pub(super) fn string_helper_for_binop(op: ast::BinaryOp) -> Option<bir::HelperOp> {
    match op {
        ast::BinaryOp::Add => Some(bir::HelperOp::StrConcat),
        ast::BinaryOp::Eq => Some(bir::HelperOp::StrEq),
        ast::BinaryOp::NotEq => Some(bir::HelperOp::StrNe),
        ast::BinaryOp::Lt => Some(bir::HelperOp::StrLt),
        ast::BinaryOp::LtEq => Some(bir::HelperOp::StrLe),
        ast::BinaryOp::Gt => Some(bir::HelperOp::StrGt),
        ast::BinaryOp::GtEq => Some(bir::HelperOp::StrGe),
        _ => None,
    }
}
/// Map a surface binary operator to Body IR's canonical arithmetic/comparison/boolean operator set, or `None` for
/// operators v0 does not model.
///
/// The unmapped set is `Pow`, `MatMul`, both pipes, the bitwise `BitAnd`/`BitOr`/`BitXor`, the `Shl`/`Shr` shifts,
/// `In`/`NotIn`, and `Is`/`IsNot`. Membership is the notable one: `parity-987-0003` records string `in` as a
/// `Preserved` behavior, so refusing it here is a tracked #1101 gap rather than a settled boundary. Adding any of
/// these needs a matching [`bir::BinOp`] variant, or a compiler-owned [`bir::HelperOp`] where the operation is a
/// runtime call rather than a primitive -- the same split [`string_helper_for_binop`] already makes.
pub(super) fn lower_binary_op(op: ast::BinaryOp) -> Option<bir::BinOp> {
    match op {
        ast::BinaryOp::Add => Some(bir::BinOp::Add),
        ast::BinaryOp::Sub => Some(bir::BinOp::Sub),
        ast::BinaryOp::Mul => Some(bir::BinOp::Mul),
        ast::BinaryOp::Div => Some(bir::BinOp::Div),
        ast::BinaryOp::FloorDiv => Some(bir::BinOp::FloorDiv),
        ast::BinaryOp::Mod => Some(bir::BinOp::Mod),
        ast::BinaryOp::Eq => Some(bir::BinOp::Eq),
        ast::BinaryOp::NotEq => Some(bir::BinOp::Ne),
        ast::BinaryOp::Lt => Some(bir::BinOp::Lt),
        ast::BinaryOp::LtEq => Some(bir::BinOp::Le),
        ast::BinaryOp::Gt => Some(bir::BinOp::Gt),
        ast::BinaryOp::GtEq => Some(bir::BinOp::Ge),
        ast::BinaryOp::And => Some(bir::BinOp::And),
        ast::BinaryOp::Or => Some(bir::BinOp::Or),
        _ => None,
    }
}
/// Lower a literal to a Body IR constant, or `None` for literal kinds v0 does not model distinctly (`bytes`).
pub(super) fn lower_literal(lit: &ast::Literal) -> Option<bir::Constant> {
    match lit {
        ast::Literal::Int(int_lit) => Some(bir::Constant::Int(int_lit.value)),
        ast::Literal::Float(float_lit) => Some(bir::Constant::Float(float_lit.repr.clone())),
        ast::Literal::Decimal(decimal_lit) => Some(bir::Constant::Float(decimal_lit.repr.clone())),
        ast::Literal::String(s) => Some(bir::Constant::Str(s.clone())),
        ast::Literal::Bool(b) => Some(bir::Constant::Bool(*b)),
        ast::Literal::None => Some(bir::Constant::None),
        ast::Literal::Bytes(_) => None,
    }
}
