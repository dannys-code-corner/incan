//! Emit Rust code for lvalue (assignment target) expressions.
//!
//! This module handles emission of expressions when they appear as assignment targets
//! (left-hand side of assignments). The key differences from regular expression emission:
//!
//! - No `.clone()` insertion for index operations
//! - Negative index handling for Python-style indexing
//!
//! ## See also
//! - `indexing.rs`: shared negative-index handling logic

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::super::super::expr::{IrExprKind, TypedExpr};
use super::super::super::stmt::AssignTarget;
use super::super::super::types::IrType;
use super::super::{EmitError, IrEmitter};

impl<'a> IrEmitter<'a> {
    /// Emit an IR expression in lvalue (assignment-target) context.
    ///
    /// ## Parameters
    /// - `expr`: Expression used as an assignment target (var / field / index).
    ///
    /// ## Returns
    /// - A Rust `TokenStream` suitable for the left-hand side of an assignment.
    ///
    /// ## Errors
    /// - `EmitError`: if emitting any sub-expression fails.
    ///
    /// ## Notes
    /// - Unlike [`Self::emit_expr`], this path avoids inserting `.clone()` for index operations when the expression
    ///   appears on the assignment LHS.
    pub(in super::super) fn emit_lvalue_expr(
        &self,
        expr: &TypedExpr,
    ) -> Result<TokenStream, EmitError> {
        match &expr.kind {
            IrExprKind::Var { name, access: _ } => {
                let n = format_ident!("{}", Self::escape_keyword(name));
                Ok(quote! { #n })
            }
            IrExprKind::Index { object, index } => {
                let o = self.emit_lvalue_expr(object)?;

                // Use the shared negative-index handling from indexing module
                let index_expr = self.emit_index_with_negative_handling(object, index, &o)?;

                Ok(quote! { #o[#index_expr] })
            }
            IrExprKind::Field { object, field } => {
                let o = self.emit_lvalue_expr(object)?;
                let f = format_ident!("{}", field);
                Ok(quote! { #o.#f })
            }
            _ => self.emit_expr(expr),
        }
    }

    /// Emit an assignment target into a Rust lvalue `TokenStream`.
    ///
    /// ## Parameters
    /// - `target`: The assignment target (variable, field access, or index).
    ///
    /// ## Returns
    /// - A Rust `TokenStream` suitable for assignment (e.g. `x`, `obj.field`, `vec[i]`).
    ///
    /// ## Errors
    /// - `EmitError`: if emitting any sub-expression fails.
    ///
    /// ## Notes
    /// - Negative indices are translated into `len() - offset` (Python-style indexing).
    pub(in super::super) fn emit_assign_target(
        &self,
        target: &AssignTarget,
    ) -> Result<TokenStream, EmitError> {
        match target {
            AssignTarget::Var(name) => {
                let n = format_ident!("{}", Self::escape_keyword(name));
                Ok(quote! { #n })
            }
            AssignTarget::Field { object, field } => {
                let o = self.emit_lvalue_expr(object)?;
                let f = format_ident!("{}", field);
                Ok(quote! { #o.#f })
            }
            AssignTarget::Index { object, index } => {
                let o = self.emit_lvalue_expr(object)?;

                let index_expr = self.emit_assign_target_index(object, index, &o)?;

                Ok(quote! { #o[#index_expr] })
            }
        }
    }

    /// Helper: emit index expression for assignment target context.
    ///
    /// Handles the dict vs list distinction for assignment targets.
    /// Uses the shared negative-index handling from `emit_index_with_negative_handling`,
    /// but for dicts with int keys, keeps the key as-is (no usize conversion).
    fn emit_assign_target_index(
        &self,
        object: &TypedExpr,
        index: &TypedExpr,
        obj_tokens: &TokenStream,
    ) -> Result<TokenStream, EmitError> {
        // For dict assignment with int keys, don't do usize conversion
        if matches!(&index.ty, IrType::Int) {
            let obj_ty = match &object.ty {
                IrType::Ref(inner) | IrType::RefMut(inner) => inner.as_ref(),
                other => other,
            };
            if matches!(obj_ty, IrType::Dict(_, _)) {
                return self.emit_expr(index);
            }
        }

        // Otherwise use the shared negative-index handling
        self.emit_index_with_negative_handling(object, index, obj_tokens)
    }
}
