//! Predicates for IR expressions that already emit Rust reference-shaped values.
//!
//! Ownership and coercion planning may still see these expressions as ordinary Incan surface types. Keep the
//! reference-shape predicate here so conversions, method emission, and argument planning do not drift.

use super::expr::{IrExpr, IrExprKind};
use super::types::IrType;

/// Return whether an IR type is already represented as a Rust reference-like value.
#[must_use]
pub fn type_has_rust_reference_shape(ty: &IrType) -> bool {
    match ty {
        IrType::Ref(_) | IrType::RefMut(_) | IrType::StrRef | IrType::StaticStr => true,
        // Callback parameters from inspected Rust APIs retain their exact emitted spelling in `RustDisplay` because
        // the Incan surface type model cannot otherwise represent every borrowed Rust shape. They already evaluate
        // to references, so ownership planning must not clone or add a second borrow.
        IrType::RustDisplay(display) => display.trim_start().starts_with('&'),
        _ => false,
    }
}

/// Return whether an expression already emits a Rust reference-shaped value despite carrying an owned Incan surface
/// type in IR.
///
/// Method receivers named `self` are represented by the enclosing Rust method's `&self` or `&mut self` parameter even
/// though their source-level nominal type remains owned in IR.
#[must_use]
pub fn expr_has_rust_reference_shape(expr: &IrExpr) -> bool {
    if type_has_rust_reference_shape(&expr.ty) {
        return true;
    }
    matches!(
        &expr.kind,
        IrExprKind::Var { name, .. } if name == "self"
    ) || matches!(
        &expr.kind,
        IrExprKind::MethodCall { method, args, .. }
            if args.is_empty() && matches!(method.as_str(), "as_slice" | "as_str")
    )
}

#[cfg(test)]
mod tests {
    use super::type_has_rust_reference_shape;
    use crate::backend::ir::IrType;

    #[test]
    fn borrowed_callback_display_preserves_reference_shape() {
        assert!(type_has_rust_reference_shape(&IrType::RustDisplay(
            "&mut egui::Ui".to_string()
        )));
        assert!(type_has_rust_reference_shape(&IrType::RustDisplay(
            "&eframe::CreationContext".to_string()
        )));
        assert!(!type_has_rust_reference_shape(&IrType::RustDisplay(
            "egui::Ui".to_string()
        )));
    }
}
