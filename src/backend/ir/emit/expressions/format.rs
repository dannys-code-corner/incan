//! Emit Rust code for format strings and range expressions.
//!
//! This module handles:
//! - Format string expressions (f-strings): `f"Hello {name}"`
//! - Range expressions: `start..end`, `start..=end`, `..end`, `start..`

use proc_macro2::TokenStream;
use quote::quote;

use super::super::super::expr::{FormatPart, TypedExpr};
use super::super::{EmitError, IrEmitter};

impl<'a> IrEmitter<'a> {
    /// Emit a format string expression.
    ///
    /// Converts f-strings to Rust's `format!()` macro calls.
    ///
    /// ## Examples
    /// - `f"Hello {name}"` → `format!("Hello {}", name)`
    /// - `f"Value: {x + 1}"` → `format!("Value: {}", x + 1)`
    pub(in super::super) fn emit_format_expr(&self, parts: &[FormatPart]) -> Result<TokenStream, EmitError> {
        let mut fmt_string = String::new();
        let mut fmt_args: Vec<TokenStream> = Vec::new();

        for part in parts {
            match part {
                FormatPart::Literal(s) => {
                    // Escape braces in literal parts
                    fmt_string.push_str(&s.replace('{', "{{").replace('}', "}}"));
                }
                FormatPart::Expr(e) => {
                    fmt_string.push_str("{}");
                    if let Ok(arg) = self.emit_expr(e) {
                        fmt_args.push(arg);
                    }
                }
            }
        }

        if fmt_args.is_empty() {
            Ok(quote! { #fmt_string.to_string() })
        } else {
            Ok(quote! { format!(#fmt_string, #(#fmt_args),*) })
        }
    }

    /// Emit a range expression.
    ///
    /// Converts Incan range syntax to Rust range expressions:
    /// - `start..end` (exclusive)
    /// - `start..=end` (inclusive)
    /// - `start..` (open-ended)
    /// - `..end` (from zero)
    /// - `..=end` (from zero, inclusive)
    pub(in super::super) fn emit_range_expr(
        &self,
        start: Option<&TypedExpr>,
        end: Option<&TypedExpr>,
        inclusive: bool,
    ) -> Result<TokenStream, EmitError> {
        match (start, end, inclusive) {
            (Some(s), Some(e), false) => {
                let ss = self.emit_expr(s)?;
                let ee = self.emit_expr(e)?;
                Ok(quote! { #ss..#ee })
            }
            (Some(s), Some(e), true) => {
                let ss = self.emit_expr(s)?;
                let ee = self.emit_expr(e)?;
                Ok(quote! { #ss..=#ee })
            }
            (Some(s), None, _) => {
                let ss = self.emit_expr(s)?;
                Ok(quote! { #ss.. })
            }
            (None, Some(e), false) => {
                let ee = self.emit_expr(e)?;
                Ok(quote! { ..#ee })
            }
            (None, Some(e), true) => {
                let ee = self.emit_expr(e)?;
                Ok(quote! { ..=#ee })
            }
            (None, None, _) => Ok(quote! { .. }),
        }
    }
}
