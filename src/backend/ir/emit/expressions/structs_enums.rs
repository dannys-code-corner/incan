//! Emit Rust code for struct constructor expressions.
//!
//! This module handles struct instantiation with both named and positional fields.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::super::super::conversions::{ConversionContext, determine_conversion};
use super::super::super::expr::TypedExpr;
use super::super::{EmitError, IrEmitter};

impl<'a> IrEmitter<'a> {
    /// Emit a struct constructor expression.
    ///
    /// Handles:
    /// - Named field construction: `Point { x: 1, y: 2 }`
    /// - Positional (tuple-style) construction: `Point(1, 2)`
    /// - Empty struct construction: `Unit {}`
    pub(in super::super) fn emit_struct_expr(
        &self,
        name: &str,
        fields: &[(String, TypedExpr)],
    ) -> Result<TokenStream, EmitError> {
        let n = format_ident!("{}", Self::escape_keyword(name));
        let all_named = fields.iter().all(|(fname, _)| !fname.is_empty());

        if all_named && !fields.is_empty() {
            // Named field construction
            let field_tokens: Vec<TokenStream> = fields
                .iter()
                .map(|(fname, fval)| {
                    let fn_ident = format_ident!("{}", fname);
                    let emitted = self.emit_expr(fval)?;
                    let target_type = self
                        .struct_field_types
                        .get(&(name.to_string(), fname.clone()));
                    let conversion =
                        determine_conversion(fval, target_type, ConversionContext::StructField);
                    let fv = conversion.apply(emitted);
                    Ok(quote! { #fn_ident: #fv })
                })
                .collect::<Result<_, EmitError>>()?;
            Ok(quote! { #n { #(#field_tokens),* } })
        } else if fields.is_empty() {
            // Empty struct construction
            Ok(quote! { #n {} })
        } else {
            // Positional (tuple-style) construction
            let value_tokens: Vec<TokenStream> = fields
                .iter()
                .map(|(_, fval)| {
                    let emitted = self.emit_expr(fval)?;
                    let conversion =
                        determine_conversion(fval, None, ConversionContext::IncanFunctionArg);
                    Ok(conversion.apply(emitted))
                })
                .collect::<Result<_, EmitError>>()?;
            Ok(quote! { #n(#(#value_tokens),*) })
        }
    }
}
