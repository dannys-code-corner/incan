//! Emit RFC-008 consts safely and predictably.
//!
//! This module contains:
//!
//! - RFC-008 const representability checks (rejecting non-emittable consts with actionable errors)
//! - const-friendly helpers for `&'static str` concatenation chains (emitting `concat!(...)`)
//!
//! ## Notes
//!
//! - Const evaluation happens in the frontend; this module focuses on *emission constraints* and
//!   ergonomics in Rust `const` contexts.
//! - Const string folding is intentionally conservative; if a value cannot be proven to be a
//!   `'static` literal chain, emission falls back to regular expression emission.
//!
//! ## See also
//!
//! - `docs/RFCs/008_const_bindings.md`
//! - [`crate::backend::ir::emit::program`]: where const string folding is initialized

use proc_macro2::TokenStream;
use quote::quote;

use super::super::expr::{BinOp, IrExprKind, Literal as IrLiteral, TypedExpr};
use super::super::types::IrType;
use super::{EmitError, IrEmitter};

impl<'a> IrEmitter<'a> {
    /// RFC 008 const representability check.
    ///
    /// Allowed (phase 1):
    /// - Int, Float, Bool
    /// - StaticStr (e.g., "literal")
    /// - StaticBytes (e.g., b"...")
    /// - Tuples of allowed consts
    /// - FrozenList/Set/Dict with allowed element types (emitted via `FrozenX::new(&[..])`)
    ///
    /// Everything else is rejected with an actionable error.
    pub(super) fn validate_const_emittable(
        &self,
        name: &str,
        ty: &IrType,
        _value: &TypedExpr,
    ) -> Result<(), EmitError> {
        fn ok_ty(ty: &IrType) -> bool {
            match ty {
                IrType::Int
                | IrType::Float
                | IrType::Bool
                | IrType::StaticStr
                | IrType::StaticBytes => true,
                IrType::Tuple(items) => items.iter().all(ok_ty),
                IrType::NamedGeneric(name, args) if name == "FrozenList" => {
                    args.first().map(ok_ty).unwrap_or(false)
                }
                IrType::NamedGeneric(name, args) if name == "FrozenSet" => {
                    args.first().map(ok_ty).unwrap_or(false)
                }
                IrType::NamedGeneric(name, args) if name == "FrozenDict" => {
                    args.first().map(ok_ty).unwrap_or(false)
                        && args.get(1).map(ok_ty).unwrap_or(false)
                }
                _ => false,
            }
        }

        if ok_ty(ty) {
            return Ok(());
        }

        let ty_name = ty.rust_name();
        Err(EmitError::Unsupported(format!(
            "const '{}' of type '{}' is not representable as a Rust const.\n\
             Allowed: int/float/bool/&'static str/&'static [u8]/tuples, FrozenList/Set/Dict with allowed element types.\n\
             Consider computing at runtime or simplifying the const.",
            name, ty_name
        )))
    }

    /// Evaluate a TypedExpr as a compile-time `'static` string value, if possible.
    fn eval_static_str_expr(
        expr: &TypedExpr,
        const_exprs: &std::collections::HashMap<String, TypedExpr>,
        visiting: &mut std::collections::HashSet<String>,
        cache: &mut std::collections::HashMap<String, String>,
    ) -> Option<String> {
        match &expr.kind {
            IrExprKind::String(s) => Some(s.clone()),
            IrExprKind::Literal(IrLiteral::StaticStr(s)) => Some(s.clone()),
            IrExprKind::Var { name, .. } => {
                Self::resolve_static_str_const(name, const_exprs, visiting, cache)
            }
            IrExprKind::BinOp {
                op: BinOp::Add,
                left,
                right,
            } => {
                let l = Self::eval_static_str_expr(left, const_exprs, visiting, cache)?;
                let r = Self::eval_static_str_expr(right, const_exprs, visiting, cache)?;
                Some(format!("{l}{r}"))
            }
            _ => None,
        }
    }

    /// Resolve a const name to its full `'static` string literal value (if representable).
    ///
    /// Uses a small DFS with cycle protection and memoization.
    pub(super) fn resolve_static_str_const(
        name: &str,
        const_exprs: &std::collections::HashMap<String, TypedExpr>,
        visiting: &mut std::collections::HashSet<String>,
        cache: &mut std::collections::HashMap<String, String>,
    ) -> Option<String> {
        if let Some(v) = cache.get(name) {
            return Some(v.clone());
        }
        if visiting.contains(name) {
            // Defensive: frontend should reject const cycles, but avoid infinite recursion here.
            return None;
        }

        let expr = const_exprs.get(name)?;
        visiting.insert(name.to_string());
        let out = Self::eval_static_str_expr(expr, const_exprs, visiting, cache);
        visiting.remove(name);

        if let Some(v) = out.clone() {
            cache.insert(name.to_string(), v);
        }
        out
    }

    /// Try to emit a const-friendly concatenation for `&'static str` additions.
    ///
    /// Supports cases where both sides are string literals or const `&'static str`
    /// bindings (recorded during program scan). Emits `concat!(.., ..)` which
    /// is valid in const contexts.
    pub(super) fn try_emit_static_str_add(
        &self,
        left: &TypedExpr,
        right: &TypedExpr,
    ) -> Result<Option<TokenStream>, EmitError> {
        // Helper to convert an expr into a string-literal token if possible
        let to_lit_tokens = |e: &TypedExpr| -> Option<TokenStream> {
            match &e.kind {
                IrExprKind::String(s) => Some(quote! { #s }),
                IrExprKind::Literal(IrLiteral::StaticStr(s)) => Some(quote! { #s }),
                IrExprKind::Var { name, .. } => self.const_string_literals.get(name).map(|lit| {
                    let l = lit.clone();
                    quote! { #l }
                }),
                _ => None,
            }
        };

        let l_tok = to_lit_tokens(left);
        let r_tok = to_lit_tokens(right);
        if let (Some(lt), Some(rt)) = (l_tok, r_tok) {
            return Ok(Some(quote! { concat!(#lt, #rt) }));
        }
        Ok(None)
    }
}
