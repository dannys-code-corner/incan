//! Emit Rust code for index, slice, and field access expressions.
//!
//! This module handles:
//! - Index expressions (`list[i]`, `dict[k]`)
//! - Slice expressions (`list[start:end]`)
//! - Field access expressions (`obj.field`)
//!
//! ## Negative index handling
//!
//! Python-style negative indices are converted to `len() - offset` at emit time.
//! This logic is shared across index expressions, lvalue emission, and assignment targets.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::super::super::expr::{IrExprKind, TypedExpr, UnaryOp};
use super::super::super::types::IrType;
use super::super::{EmitError, IrEmitter};

impl<'a> IrEmitter<'a> {
    /// Emit an index expression.
    ///
    /// Handles `list[i]` and `dict[k]` access with:
    /// - Negative index conversion (Python-style)
    /// - Clone insertion for non-Copy types
    /// - Type-aware bracket vs method access
    pub(in super::super) fn emit_index_expr(
        &self,
        object: &TypedExpr,
        index: &TypedExpr,
    ) -> Result<TokenStream, EmitError> {
        let o = self.emit_expr(object)?;

        let index_expr = self.emit_index_with_negative_handling(object, index, &o)?;

        let obj_ty = match &object.ty {
            IrType::Ref(inner) | IrType::RefMut(inner) => inner.as_ref(),
            other => other,
        };

        match obj_ty {
            IrType::Dict(_, v) => {
                let i = self.emit_expr(index)?;
                if v.is_copy() {
                    Ok(quote! { #o[&#i] })
                } else {
                    Ok(quote! { #o[&#i].clone() })
                }
            }
            IrType::List(elem) => {
                if elem.is_copy() {
                    Ok(quote! { #o[#index_expr] })
                } else {
                    Ok(quote! { #o[#index_expr].clone() })
                }
            }
            IrType::Unknown => Ok(quote! { #o[#index_expr].clone() }),
            _ => Ok(quote! { #o[#index_expr] }),
        }
    }

    /// Emit a slice expression.
    ///
    /// Handles `list[start:end]` → `list[start..end].to_vec()`.
    pub(in super::super) fn emit_slice_expr(
        &self,
        target: &TypedExpr,
        start: &Option<Box<TypedExpr>>,
        end: &Option<Box<TypedExpr>>,
    ) -> Result<TokenStream, EmitError> {
        let t = self.emit_expr(target)?;

        let start_expr = if let Some(s) = start {
            let s_tokens = self.emit_expr(s)?;
            quote! { (#s_tokens) as usize }
        } else {
            quote! { 0 }
        };

        let end_expr = if let Some(e) = end {
            let e_tokens = self.emit_expr(e)?;
            quote! { (#e_tokens) as usize }
        } else {
            quote! { #t.len() }
        };

        Ok(quote! { #t[#start_expr..#end_expr].to_vec() })
    }

    /// Emit a field access expression.
    ///
    /// Handles:
    /// - Enum variant access (`Type.Variant` → `Type::Variant`)
    /// - Tuple field access (`tuple.0` → `tuple.0`)
    /// - Regular struct field access (`obj.field` → `obj.field`)
    pub(in super::super) fn emit_field_expr(
        &self,
        object: &TypedExpr,
        field: &str,
    ) -> Result<TokenStream, EmitError> {
        let o = self.emit_expr(object)?;

        // Check if this is an enum variant access using the actual enum registry, not capitalization heuristics
        if let IrExprKind::Var { name, .. } = &object.kind {
            let key = (name.to_string(), field.to_string());
            if self.enum_variant_fields.contains_key(&key) {
                let type_ident = format_ident!("{}", name);
                let f = format_ident!("{}", field);
                return Ok(quote! { #type_ident::#f });
            }
        }

        // Check if field is a numeric index (tuple access)
        if field.chars().all(|c| c.is_ascii_digit()) {
            let idx: syn::Index = field
                .parse::<usize>()
                .map(syn::Index::from)
                .unwrap_or_else(|_| syn::Index::from(0));
            Ok(quote! { #o.#idx })
        } else {
            let f = format_ident!("{}", field);
            Ok(quote! { #o.#f })
        }
    }

    /// Helper: emit an index expression with negative-index handling.
    ///
    /// Converts Python-style negative indices to `len() - offset`.
    /// This helper is used by both `emit_index_expr` and lvalue emission.
    pub(in super::super) fn emit_index_with_negative_handling(
        &self,
        _object: &TypedExpr,
        index: &TypedExpr,
        obj_tokens: &TokenStream,
    ) -> Result<TokenStream, EmitError> {
        match &index.kind {
            IrExprKind::Int(n) if *n < 0 => {
                let offset = n.abs();
                Ok(quote! { #obj_tokens.len() - #offset })
            }
            IrExprKind::UnaryOp {
                op: UnaryOp::Neg,
                operand,
            } => {
                if let IrExprKind::Int(n) = &operand.kind {
                    Ok(quote! { #obj_tokens.len() - #n })
                } else {
                    let i = self.emit_expr(operand)?;
                    Ok(quote! { #obj_tokens.len() - (#i) as usize })
                }
            }
            _ => {
                let i = self.emit_expr(index)?;
                match &index.ty {
                    IrType::Int | IrType::Unknown => Ok(quote! { (#i) as usize }),
                    _ => Ok(i),
                }
            }
        }
    }
}
