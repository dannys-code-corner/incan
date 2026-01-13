//! Emit Rust code for method calls.
//!
//! This module handles emission of both known methods (enum-based dispatch via `MethodKind`)
//! and unknown methods (string-based fallback).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::super::super::conversions::{ConversionContext, determine_conversion};
use super::super::super::expr::{IrExprKind, MethodKind, TypedExpr};
use super::super::super::types::IrType;
use super::super::{EmitError, IrEmitter};

mod collection_methods;
mod string_methods;

use collection_methods::emit_collection_method;
use string_methods::emit_string_method;

/// Compute common receiver setup for method emission.
///
/// This deduplicates the pattern of:
/// - Detecting `FrozenStr` receivers
/// - Unwrapping them via `.as_str()`
/// - Computing whether the receiver is string-like for stdlib routing
pub(super) struct ReceiverInfo {
    /// The receiver token stream (possibly wrapped in `.as_str()` for FrozenStr).
    pub(super) r: TokenStream,
    /// A borrow of the receiver: `&#r`.
    pub(super) r_borrow: TokenStream,
    /// Whether the receiver is a string-like type (String or FrozenStr).
    pub(super) is_stringish: bool,
}

impl ReceiverInfo {
    /// Build receiver info from the receiver type and emitted receiver tokens.
    fn new(receiver_ty: &IrType, emitted: TokenStream) -> Self {
        let is_frozen_str = matches!(receiver_ty, IrType::FrozenStr);
        let r = if is_frozen_str {
            quote! { #emitted.as_str() }
        } else {
            emitted
        };
        let r_borrow = quote! { &#r };
        let is_stringish = matches!(receiver_ty, IrType::String | IrType::FrozenStr);
        Self {
            r,
            r_borrow,
            is_stringish,
        }
    }
}

impl<'a> IrEmitter<'a> {
    /// Check whether a variable name looks like a Rust type name (TitleCase).
    ///
    /// This heuristic is used to decide whether `Type.method(...)` should be emitted as an associated function call
    /// `Type::method(...)` in Rust.
    ///
    /// ## Parameters
    ///
    /// - `name`: Identifier text from IR (not yet escaped).
    ///
    /// ## Returns
    ///
    /// - `true`: if `name` looks like `TitleCase` (first char uppercase and contains at least one lowercase char).
    /// - `false`: otherwise.
    ///
    /// ## Notes
    ///
    /// - This is intentionally conservative: it avoids rewriting ALLCAPS constants as types.
    /// - When we have stronger information (e.g. enum variant registries), prefer that over heuristics.
    fn is_title_case_type_name(name: &str) -> bool {
        let has_upper = name.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false);
        let has_lower = name.chars().any(|c| c.is_ascii_lowercase());
        has_upper && has_lower
    }

    /// Emit a known method call using enum-based dispatch.
    ///
    /// This handles calls that have been lowered to `IrExprKind::KnownMethodCall`.
    ///
    /// ## Parameters
    ///
    /// - `receiver`: The receiver expression
    /// - `kind`: The method kind enum variant
    /// - `args`: The method call arguments
    ///
    /// ## Returns
    ///
    /// - A Rust `TokenStream` for the method call
    pub(in super::super) fn emit_known_method_call(
        &self,
        receiver: &TypedExpr,
        kind: &MethodKind,
        args: &[TypedExpr],
    ) -> Result<TokenStream, EmitError> {
        let r0 = self.emit_expr(receiver)?;
        let info = ReceiverInfo::new(&receiver.ty, r0);
        if let Some(res) = emit_string_method(self, &receiver.ty, &info, kind, args) {
            return res;
        }
        if let Some(res) = emit_collection_method(self, receiver, &info, kind, args) {
            return res;
        }

        match kind {
            // ---- Internal/special methods ----
            MethodKind::Slice => self.emit_runtime_str_slice(&info, args),
            _ => unreachable!("string methods are handled via emit_string_method"),
        }
    }

    /// Emit a method call expression (string-based fallback).
    ///
    /// This handles `IrExprKind::MethodCall` where the method name is a string.
    /// Known methods are handled inline; unknown methods pass through as-is.
    pub(in super::super) fn emit_method_call_expr(
        &self,
        receiver: &TypedExpr,
        method: &str,
        args: &[TypedExpr],
    ) -> Result<TokenStream, EmitError> {
        let r0 = self.emit_expr(receiver)?;
        let info = ReceiverInfo::new(&receiver.ty, r0);
        let r = &info.r;

        if let Some(kind) = MethodKind::from_name(method) {
            if let Some(res) = emit_string_method(self, &receiver.ty, &info, &kind, args) {
                return res;
            }
            if let Some(res) = emit_collection_method(self, receiver, &info, &kind, args) {
                return res;
            }
        }

        // Handle special methods (legacy string-based dispatch)
        if method == "__slice__" {
            return self.emit_runtime_str_slice(&info, args);
        }

        // Check if this is an enum variant construction.
        //
        // Important: do NOT treat any uppercase variable as a type name. Only rewrite when we actually know this
        // (Type, Variant) pair exists in the enum variant registry.
        if let IrExprKind::Var { name, .. } = &receiver.kind {
            let key = (name.to_string(), method.to_string());
            if self.enum_variant_fields.contains_key(&key) {
                return self.emit_enum_variant_call(name, method, args);
            }
        }

        // Associated function call on a type: `Type.method(...)` → `Type::method(...)`
        //
        // This is needed for external Rust types like `Uuid`, `Instant`, `HashMap`, and also for
        // Incan-generated impl methods called in a "static" style (e.g. `User.from_json(...)`).
        if let IrExprKind::Var { name, .. } = &receiver.kind {
            // Rewrite `Type.method(...)` to `Type::method(...)` only when we know the identifier is a type,
            // using backend metadata (struct/enum registries). Avoid rewriting runtime variables.
            let is_known_struct = self.struct_field_names.contains_key(name);
            let is_known_enum = self.enum_variant_fields.keys().any(|(etype, _)| etype == name);
            // External Rust interop types (e.g. Uuid/Instant/HashMap) don't populate our registries.
            // In those cases the receiver typically has `Unknown` IR type (not a bound value), so fall back
            // to a conservative TitleCase heuristic.
            // TODO: #52 Remove this TitleCase heuristic by carrying explicit “type-name vs value-binding” metadata from
            // the frontend/typechecker into IR. Today we only use this when `receiver.ty` is `Unknown` to support Rust
            // interop associated fns (Uuid::new_v4, HashMap::new, Instant::now), but a TitleCase runtime variable that
            // remains `Unknown` could be mis-emitted as `Type::method`.
            let is_external_rust_type_like =
                matches!(receiver.ty, IrType::Unknown) && Self::is_title_case_type_name(name);

            if is_known_struct || is_known_enum || is_external_rust_type_like {
                let type_ident = format_ident!("{}", name);
                let m = format_ident!("{}", method);
                // Apply Incan-style argument conversions when calling associated functions on Incan-owned types
                // (structs/enums/traits). This is important for `str` literals which are emitted as `&'static str`,
                // but many Incan-level signatures expect owned `String` in Rust (e.g., newtype `from_underlying(v:
                // str)`).
                //
                // For unknown/external types, keep the previous behavior to avoid accidental conversions for Rust APIs
                // that truly want `&str`.
                let apply_incan_arg_conversions =
                    matches!(receiver.ty, IrType::Struct(_) | IrType::Enum(_) | IrType::Trait(_));

                let arg_tokens: Vec<TokenStream> = if apply_incan_arg_conversions {
                    args.iter()
                        .map(|a| {
                            let emitted = self.emit_expr(a)?;
                            let conv = determine_conversion(a, None, ConversionContext::IncanFunctionArg);
                            Ok(conv.apply(emitted))
                        })
                        .collect::<Result<_, _>>()?
                } else {
                    args.iter().map(|a| self.emit_expr(a)).collect::<Result<_, _>>()?
                };
                return Ok(quote! { #type_ident::#m(#(#arg_tokens),*) });
            }
        }

        // Regular method call
        let m = format_ident!("{}", method);
        // Apply Incan-style argument conversions for method calls on Incan-owned types (structs/enums/traits).
        // This is important for `str` literals: we often emit `"x"` as `&'static str`, but many Incan-level method
        // signatures expect owned `String` in Rust.
        //
        // For unknown/external types, keep the previous behavior to avoid accidental conversions for Rust APIs that
        // truly want `&str`.
        let apply_incan_arg_conversions = matches!(receiver.ty, IrType::Struct(_) | IrType::Enum(_) | IrType::Trait(_));
        let arg_tokens: Vec<TokenStream> = if apply_incan_arg_conversions {
            args.iter()
                .map(|a| {
                    let emitted = self.emit_expr(a)?;
                    let conv = determine_conversion(a, None, ConversionContext::IncanFunctionArg);
                    Ok(conv.apply(emitted))
                })
                .collect::<Result<_, _>>()?
        } else {
            args.iter().map(|a| self.emit_expr(a)).collect::<Result<_, _>>()?
        };
        Ok(quote! { #r.#m(#(#arg_tokens),*) })
    }

    /// Emit a runtime string slice call using shared stdlib/semantics helpers.
    ///
    /// This ensures emitted Rust uses the same Unicode/panic behavior as runtime and avoids drift
    /// from direct range slicing on Rust strings.
    fn emit_runtime_str_slice(&self, info: &ReceiverInfo, args: &[TypedExpr]) -> Result<TokenStream, EmitError> {
        let r_borrow = &info.r_borrow;

        let start_tokens = if let Some(arg0) = args.first() {
            let start = self.emit_expr(arg0)?;
            quote! { Some((#start) as i64) }
        } else {
            quote! { None }
        };

        let end_tokens = if let Some(arg1) = args.get(1) {
            if matches!(arg1.kind, IrExprKind::Int(-1)) {
                quote! { None }
            } else {
                let end = self.emit_expr(arg1)?;
                quote! { Some((#end) as i64) }
            }
        } else {
            quote! { None }
        };

        Ok(quote! { incan_stdlib::strings::str_slice(#r_borrow, #start_tokens, #end_tokens, None) })
    }

    /// Emit an enum variant construction call (Type.Variant(...) -> Type::Variant(...)).
    pub(in super::super) fn emit_enum_variant_call(
        &self,
        type_name: &str,
        variant: &str,
        args: &[TypedExpr],
    ) -> Result<TokenStream, EmitError> {
        let variant_key = (type_name.to_string(), variant.to_string());
        let arg_tokens: Vec<TokenStream> = if let Some(fields) = self.enum_variant_fields.get(&variant_key) {
            match fields {
                super::super::super::decl::VariantFields::Unit => Vec::new(),
                super::super::super::decl::VariantFields::Tuple(field_tys) => args
                    .iter()
                    .zip(field_tys.iter())
                    .map(|(a, ty)| {
                        let emitted = self.emit_expr(a)?;
                        let conv = determine_conversion(a, Some(ty), ConversionContext::IncanFunctionArg);
                        Ok(conv.apply(emitted))
                    })
                    .collect::<Result<_, _>>()?,
                super::super::super::decl::VariantFields::Struct(_) => args
                    .iter()
                    .map(|a| {
                        let emitted = self.emit_expr(a)?;
                        let conv = determine_conversion(a, None, ConversionContext::IncanFunctionArg);
                        Ok(conv.apply(emitted))
                    })
                    .collect::<Result<_, _>>()?,
            }
        } else {
            args.iter()
                .map(|a| {
                    let emitted = self.emit_expr(a)?;
                    let conv = determine_conversion(a, Some(&IrType::String), ConversionContext::IncanFunctionArg);
                    Ok(conv.apply(emitted))
                })
                .collect::<Result<_, _>>()?
        };

        let type_ident = format_ident!("{}", type_name);
        let m = format_ident!("{}", variant);
        Ok(quote! { #type_ident::#m(#(#arg_tokens),*) })
    }
}
