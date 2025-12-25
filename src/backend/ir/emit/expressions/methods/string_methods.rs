use proc_macro2::TokenStream;
use quote::quote;

use crate::backend::ir::emit::expressions::methods::ReceiverInfo;
use crate::backend::ir::emit::{EmitError, IrEmitter};
use crate::backend::ir::expr::{MethodKind, TypedExpr};
use crate::backend::ir::types::IrType;

/// Emit known string methods (shared by enum-dispatch and string-fallback paths).
pub(super) fn emit_string_method(
    emitter: &IrEmitter,
    receiver_ty: &IrType,
    info: &ReceiverInfo,
    kind: &MethodKind,
    args: &[TypedExpr],
) -> Option<Result<TokenStream, EmitError>> {
    let r = &info.r;
    let r_borrow = &info.r_borrow;
    let is_stringish = info.is_stringish;

    match kind {
        MethodKind::Upper => Some(if is_stringish {
            Ok(quote! { incan_stdlib::strings::str_upper(#r_borrow) })
        } else {
            Ok(quote! { #r.to_uppercase() })
        }),
        MethodKind::Lower => Some(if is_stringish {
            Ok(quote! { incan_stdlib::strings::str_lower(#r_borrow) })
        } else {
            Ok(quote! { #r.to_lowercase() })
        }),
        MethodKind::Strip => Some(if is_stringish {
            Ok(quote! { incan_stdlib::strings::str_strip(#r_borrow) })
        } else {
            Ok(quote! { #r.trim().to_string() })
        }),
        MethodKind::Split => {
            let sep = if let Some(arg) = args.first() {
                match emitter.emit_expr(arg) {
                    Ok(a) => quote! { Some(&#a) },
                    Err(e) => return Some(Err(e)),
                }
            } else {
                quote! { None }
            };
            Some(Ok(quote! { incan_stdlib::strings::str_split(#r_borrow, #sep) }))
        }
        MethodKind::Replace => Some(if args.len() >= 2 {
            let pattern = match emitter.emit_expr(&args[0]) {
                Ok(p) => p,
                Err(e) => return Some(Err(e)),
            };
            let replacement = match emitter.emit_expr(&args[1]) {
                Ok(r) => r,
                Err(e) => return Some(Err(e)),
            };
            Ok(quote! { incan_stdlib::strings::str_replace(#r_borrow, &#pattern, &#replacement) })
        } else {
            Ok(quote! { #r.to_string() })
        }),
        MethodKind::Join => Some(if let Some(arg) = args.first() {
            let items = match emitter.emit_expr(arg) {
                Ok(it) => it,
                Err(e) => return Some(Err(e)),
            };
            Ok(quote! { incan_stdlib::strings::str_join(#r_borrow, &#items) })
        } else {
            Ok(quote! { String::new() })
        }),
        MethodKind::StartsWith => Some(if let Some(arg) = args.first() {
            let a = match emitter.emit_expr(arg) {
                Ok(v) => v,
                Err(e) => return Some(Err(e)),
            };
            if is_stringish {
                Ok(quote! { incan_stdlib::strings::str_starts_with(#r_borrow, &#a) })
            } else {
                Ok(quote! { #r.starts_with(#a) })
            }
        } else {
            Ok(quote! { true })
        }),
        MethodKind::EndsWith => Some(if let Some(arg) = args.first() {
            let a = match emitter.emit_expr(arg) {
                Ok(v) => v,
                Err(e) => return Some(Err(e)),
            };
            if is_stringish {
                Ok(quote! { incan_stdlib::strings::str_ends_with(#r_borrow, &#a) })
            } else {
                Ok(quote! { #r.ends_with(#a) })
            }
        } else {
            Ok(quote! { true })
        }),
        MethodKind::Contains => Some(if let Some(arg) = args.first() {
            let a = match emitter.emit_expr(arg) {
                Ok(v) => v,
                Err(e) => return Some(Err(e)),
            };
            match receiver_ty {
                IrType::FrozenStr => Ok(quote! { incan_stdlib::strings::str_contains(#r_borrow, &#a) }),
                IrType::String => Ok(quote! { incan_stdlib::strings::str_contains(#r_borrow, &#a) }),
                IrType::List(_) | IrType::Set(_) => Ok(quote! { #r.contains(&#a) }),
                IrType::Dict(_, _) => Ok(quote! { #r.contains_key(&#a) }),
                _ => Ok(quote! { #r.contains(&#a) }),
            }
        } else {
            Ok(quote! { false })
        }),
        _ => None,
    }
}
