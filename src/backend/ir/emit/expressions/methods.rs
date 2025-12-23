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
    /// - When we have stronger information (e.g. enum variant registries), prefer that over
    ///   heuristics.
    fn is_title_case_type_name(name: &str) -> bool {
        let has_upper = name
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false);
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
        let receiver_is_frozen_str = matches!(&receiver.ty, IrType::Struct(n) if n == "FrozenStr");
        let r = if receiver_is_frozen_str {
            quote! { #r0.as_str() }
        } else {
            r0
        };

        match kind {
            // ---- String methods ----
            MethodKind::Upper => Ok(quote! { #r.to_uppercase() }),
            MethodKind::Lower => Ok(quote! { #r.to_lowercase() }),
            MethodKind::Strip => Ok(quote! { #r.trim().to_string() }),
            MethodKind::Split => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(quote! { #r.split(#a).map(|s| s.to_string()).collect::<Vec<_>>() })
                } else {
                    Ok(quote! { vec![#r.to_string()] })
                }
            }
            MethodKind::Replace => {
                if args.len() >= 2 {
                    let pattern = self.emit_expr(&args[0])?;
                    let replacement = self.emit_expr(&args[1])?;
                    Ok(quote! { #r.replace(#pattern, #replacement) })
                } else {
                    Ok(quote! { #r.to_string() })
                }
            }
            MethodKind::Join => {
                if let Some(arg) = args.first() {
                    let items = self.emit_expr(arg)?;
                    Ok(quote! { #items.join(#r) })
                } else {
                    Ok(quote! { String::new() })
                }
            }
            MethodKind::StartsWith => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(quote! { #r.starts_with(#a) })
                } else {
                    Ok(quote! { true })
                }
            }
            MethodKind::EndsWith => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(quote! { #r.ends_with(#a) })
                } else {
                    Ok(quote! { true })
                }
            }

            // ---- Collection methods ----
            MethodKind::Contains => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    match &receiver.ty {
                        IrType::Struct(n) if n == "FrozenStr" => Ok(quote! { #r.contains(#a) }),
                        IrType::String => Ok(quote! { #r.contains(#a) }),
                        IrType::List(_) | IrType::Set(_) => Ok(quote! { #r.contains(&#a) }),
                        IrType::Dict(_, _) => Ok(quote! { #r.contains_key(&#a) }),
                        _ => Ok(quote! { #r.contains(&#a) }),
                    }
                } else {
                    Ok(quote! { false })
                }
            }
            MethodKind::Get => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(quote! { #r.get(#a) })
                } else {
                    Ok(quote! { None })
                }
            }
            MethodKind::Insert => {
                if args.len() >= 2 {
                    let k = self.emit_expr(&args[0])?;
                    let v = self.emit_expr(&args[1])?;
                    Ok(quote! { #r.insert(#k, #v) })
                } else {
                    Ok(quote! { () })
                }
            }
            MethodKind::Remove => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(quote! { #r.remove(#a) })
                } else {
                    Ok(quote! { None })
                }
            }

            // ---- List methods ----
            MethodKind::Append => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(quote! { #r.push(#a) })
                } else {
                    Ok(quote! { () })
                }
            }
            MethodKind::Pop => {
                // Snapshot-stable behavior: always default the popped value.
                // (This is used in both Vec::pop and user-defined `pop()` methods in generated code.)
                Ok(quote! { #r.pop().unwrap_or_default() })
            }
            MethodKind::Swap => {
                if args.len() >= 2 {
                    let a1 = self.emit_expr(&args[0])?;
                    let a2 = self.emit_expr(&args[1])?;
                    Ok(quote! { #r.swap((#a1) as usize, (#a2) as usize) })
                } else {
                    Ok(quote! { () })
                }
            }
            MethodKind::Reserve => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(quote! { #r.reserve((#a) as usize) })
                } else {
                    Ok(quote! { () })
                }
            }
            MethodKind::ReserveExact => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    Ok(quote! { #r.reserve_exact((#a) as usize) })
                } else {
                    Ok(quote! { () })
                }
            }

            // ---- Internal/special methods ----
            MethodKind::Slice => {
                if args.len() >= 2 {
                    let start = self.emit_expr(&args[0])?;
                    let end = self.emit_expr(&args[1])?;
                    if let IrExprKind::Int(-1) = &args[1].kind {
                        Ok(quote! { #r[(#start as usize)..] })
                    } else {
                        Ok(quote! { #r[(#start as usize)..(#end as usize)] })
                    }
                } else {
                    Ok(quote! { #r[..] })
                }
            }
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
        let receiver_is_frozen_str = matches!(&receiver.ty, IrType::Struct(n) if n == "FrozenStr");
        let r = if receiver_is_frozen_str {
            quote! { #r0.as_str() }
        } else {
            r0
        };

        // Handle special methods (legacy string-based dispatch)
        match method {
            "upper" => return Ok(quote! { #r.to_uppercase() }),
            "lower" => return Ok(quote! { #r.to_lowercase() }),
            "strip" => return Ok(quote! { #r.trim().to_string() }),
            "split" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    return Ok(quote! { #r.split(#a).map(|s| s.to_string()).collect::<Vec<_>>() });
                }
            }
            "contains" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    match &receiver.ty {
                        IrType::String => return Ok(quote! { #r.contains(#a) }),
                        IrType::List(_) | IrType::Set(_) => return Ok(quote! { #r.contains(&#a) }),
                        IrType::Dict(_, _) => return Ok(quote! { #r.contains_key(&#a) }),
                        _ => return Ok(quote! { #r.contains(&#a) }),
                    }
                }
            }
            "replace" => {
                if args.len() == 2 {
                    let pattern = self.emit_expr(&args[0])?;
                    let replacement = self.emit_expr(&args[1])?;
                    return Ok(quote! { #r.replace(#pattern, #replacement) });
                }
            }
            "join" => {
                if let Some(arg) = args.first() {
                    let items = self.emit_expr(arg)?;
                    return Ok(quote! { #items.join(#r) });
                }
            }
            "startswith" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    return Ok(quote! { #r.starts_with(#a) });
                }
            }
            "endswith" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    return Ok(quote! { #r.ends_with(#a) });
                }
            }
            "__slice__" => {
                if args.len() == 2 {
                    let start = self.emit_expr(&args[0])?;
                    let end = self.emit_expr(&args[1])?;
                    if let IrExprKind::Int(-1) = &args[1].kind {
                        return Ok(quote! { #r[(#start as usize)..] });
                    }
                    return Ok(quote! { #r[(#start as usize)..(#end as usize)] });
                }
            }
            "swap" => {
                if args.len() == 2 {
                    let a1 = self.emit_expr(&args[0])?;
                    let a2 = self.emit_expr(&args[1])?;
                    return Ok(quote! { #r.swap((#a1) as usize, (#a2) as usize) });
                }
            }
            "append" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    return Ok(quote! { #r.push(#a) });
                }
            }
            "pop" => {
                // Snapshot-stable behavior: always default the popped value.
                return Ok(quote! { #r.pop().unwrap_or_default() });
            }
            "get" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    return Ok(quote! { #r.get(#a) });
                }
            }
            "insert" => {
                if args.len() == 2 {
                    let k = self.emit_expr(&args[0])?;
                    let v = self.emit_expr(&args[1])?;
                    return Ok(quote! { #r.insert(#k, #v) });
                }
            }
            "remove" => {
                if let Some(arg) = args.first() {
                    let a = self.emit_expr(arg)?;
                    return Ok(quote! { #r.remove(#a) });
                }
            }
            "reserve" | "reserve_exact" => {
                if let Some(arg) = args.first() {
                    let is_list = match &receiver.ty {
                        IrType::List(_) => true,
                        IrType::RefMut(inner) | IrType::Ref(inner) => {
                            matches!(inner.as_ref(), IrType::List(_))
                        }
                        _ => false,
                    };
                    if is_list {
                        let a = self.emit_expr(arg)?;
                        let m = format_ident!("{}", method);
                        return Ok(quote! { #r.#m((#a) as usize) });
                    }
                }
            }
            _ => {}
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
            if Self::is_title_case_type_name(name) {
                let type_ident = format_ident!("{}", name);
                let m = format_ident!("{}", method);
                let arg_tokens: Vec<TokenStream> = args
                    .iter()
                    .map(|a| self.emit_expr(a))
                    .collect::<Result<_, _>>()?;
                return Ok(quote! { #type_ident::#m(#(#arg_tokens),*) });
            }
        }

        // Regular method call
        let m = format_ident!("{}", method);
        let arg_tokens: Vec<TokenStream> = args
            .iter()
            .map(|a| self.emit_expr(a))
            .collect::<Result<_, _>>()?;
        Ok(quote! { #r.#m(#(#arg_tokens),*) })
    }

    /// Emit an enum variant construction call (Type.Variant(...) -> Type::Variant(...)).
    pub(in super::super) fn emit_enum_variant_call(
        &self,
        type_name: &str,
        variant: &str,
        args: &[TypedExpr],
    ) -> Result<TokenStream, EmitError> {
        let variant_key = (type_name.to_string(), variant.to_string());
        let arg_tokens: Vec<TokenStream> = if let Some(fields) =
            self.enum_variant_fields.get(&variant_key)
        {
            match fields {
                super::super::super::decl::VariantFields::Unit => Vec::new(),
                super::super::super::decl::VariantFields::Tuple(field_tys) => args
                    .iter()
                    .zip(field_tys.iter())
                    .map(|(a, ty)| {
                        let emitted = self.emit_expr(a)?;
                        let conv =
                            determine_conversion(a, Some(ty), ConversionContext::IncanFunctionArg);
                        Ok(conv.apply(emitted))
                    })
                    .collect::<Result<_, _>>()?,
                super::super::super::decl::VariantFields::Struct(_) => args
                    .iter()
                    .map(|a| {
                        let emitted = self.emit_expr(a)?;
                        let conv =
                            determine_conversion(a, None, ConversionContext::IncanFunctionArg);
                        Ok(conv.apply(emitted))
                    })
                    .collect::<Result<_, _>>()?,
            }
        } else {
            args.iter()
                .map(|a| {
                    let emitted = self.emit_expr(a)?;
                    let conv = determine_conversion(
                        a,
                        Some(&IrType::String),
                        ConversionContext::IncanFunctionArg,
                    );
                    Ok(conv.apply(emitted))
                })
                .collect::<Result<_, _>>()?
        };

        let type_ident = format_ident!("{}", type_name);
        let m = format_ident!("{}", variant);
        Ok(quote! { #type_ident::#m(#(#arg_tokens),*) })
    }
}
