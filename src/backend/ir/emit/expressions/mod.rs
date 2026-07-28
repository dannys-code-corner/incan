Warning: truncated output (original token count: 42765)
Total output lines: 4063

//! Emit Rust expressions from Incan IR.
//!
//! This module converts IR expressions ([`TypedExpr`]/[`IrExprKind`]) into Rust expression
//! fragments ([`TokenStream`]).
//!
//! It is used by [`IrEmitter`] to implement the "IR → Rust" portion of the backend at the
//! expression level (literals, operators, calls, method calls, comprehensions, indexing/slicing,
//! and control flow).
//!
//! ## Module organization
//!
//! The expression emitter is split into focused submodules:
//!
//! - [`builtins`]: Built-in function calls (`print`, `len`, `range`, etc.)
//! - [`methods`]: Method calls (both known methods via `MethodKind` and regular Rust method-call emission)
//! - [`calls`]: Regular function calls and binary operations
//! - [`indexing`]: Index, slice, and field access expressions
//! - [`comprehensions`]: List comprehensions, dict comprehensions, and generator expressions
//! - [`structs_enums`]: Struct constructor expressions
//! - [`mod@format`]: Format strings and range expressions
//! - [`lvalue`]: Assignment target expressions
//!
//! ## Notes
//!
//! - **Not lexer tokens**: [`TokenStream`] here is `proc_macro2::TokenStream` used for Rust codegen. Lexer output is a
//!   separate token type in the frontend.
//! - **Ownership planning is centralized**: Ownership/borrow/copy/string adjustments should go through
//!   `backend::ir::ownership` instead of being hand-coded inline.
//! - **Side-effect free**: Emission is pure codegen; it does not touch the filesystem.
//!
//! ## Examples
//!
//! ```rust,ignore
//! // Pseudocode: IrEmitter is constructed by the backend codegen pipeline.
//! let tokens: proc_macro2::TokenStream = emitter.emit_expr(&typed_expr)?;
//! ```
//!
//! ## See also
//!
//! - `src/backend/ir/ownership.rs`: ownership/coercion planner for emitted Rust boundaries
//! - `src/backend/ir/emit/mod.rs`: higher-level emission (items/statements) that calls into this module

mod builtins;
mod calls;
mod comprehensions;
mod format;
mod indexing;
mod interop_coercions;
mod lvalue;
mod methods;
mod structs_enums;

use proc_macro2::{Literal, TokenStream};
use quote::{ToTokens, format_ident, quote};
use std::sync::LazyLock;

use super::super::decl::IrInteropAdapterKind;
use super::super::expr::{
    CollectionMethodKind, IrDictEntry, IrExprKind, IrInteropCoercionKind, IrListEntry, IrMethodDispatch,
    Literal as IrLiteral, MethodKind, NumericResizePolicy, TypedExpr, UnaryOp, VarRefKind,
};
use super::super::types::IrType;
use super::{EmitError, IrEmitter};
use crate::backend::ir::ownership::{ValueUseSite, plan_value_use, value_use_site_target_ty};
use incan_core::lang::surface::methods::{dict_methods, list_methods};
use incan_core::lang::types::collections::{self, CollectionTypeId};

#[derive(Debug, Clone)]
pub(super) enum StorageRoot {
    /// A module-level static storage slot.
    Static(String),
    /// A local alias that wraps static storage in the current emitted statement slice.
    Binding(String),
}

/// Whether a lowered known method mutates its receiver.
///
/// This is the canonical receiver-mutability policy for `MethodKind` in IR emission. Keep method mutability decisions
/// in one place to avoid drift between statement analysis, parameter mutation scan, and emitted storage-lock behavior.
pub(in crate::backend::ir::emit) fn method_kind_uses_mutable_receiver(kind: &MethodKind) -> bool {
    matches!(
        kind,
        MethodKind::Collection(
            CollectionMethodKind::Add
                | CollectionMethodKind::Insert
                | CollectionMethodKind::Remove
                | CollectionMethodKind::Append
                | CollectionMethodKind::Extend
                | CollectionMethodKind::Pop
                | CollectionMethodKind::Swap
                | CollectionMethodKind::Reserve
                | CollectionMethodKind::ReserveExact
        )
    )
}

/// Return whether frontend semantic method resolution selected a trait method with `mut self`.
pub(in crate::backend::ir::emit) fn method_dispatch_uses_mutable_receiver(dispatch: Option<&IrMethodDispatch>) -> bool {
    matches!(
        dispatch,
        Some(IrMethodDispatch::Trait {
            receiver_is_mutable: true,
            ..
        })
    )
}

/// String-named methods that mutate their receiver.
///
/// Lowering normally classifies collection methods as [`MethodKind`], but Rust interop and a few compiler-internal
/// rewrites can still reach emission as string-named method calls. Keep the name policy next to the `MethodKind` policy
/// so parameter scanning, local-binding emission, and storage-lock analysis do not drift.
static MUTATING_METHOD_NAMES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        list_methods::as_str(list_methods::ListMethodId::Append),
        list_methods::as_str(list_methods::ListMethodId::Extend),
        list_methods::as_str(list_methods::ListMethodId::Pop),
        list_methods::as_str(list_methods::ListMethodId::Swap),
        list_methods::as_str(list_methods::ListMethodId::Reserve),
        list_methods::as_str(list_methods::ListMethodId::ReserveExact),
        list_methods::as_str(list_methods::ListMethodId::Remove),
        dict_methods::as_str(dict_methods::DictMethodId::Insert),
        "push",
        "clear",
    ]
});

/// Return whether a string-named method should be emitted with a mutable receiver borrow.
pub(in crate::backend::ir::emit) fn method_name_uses_mutable_receiver(name: &str) -> bool {
    MUTATING_METHOD_NAMES.contains(&name)
}

impl<'a> IrEmitter<'a> {
    /// Bridge a canonical Incan `Iterator[T]` value into the Rust iterator consumed by loop and comprehension syntax.
    ///
    /// Adapter models expose the source-owned `Iterator.__next__() -> Option[T]` contract rather than implementing
    /// Rust's `std::iter::Iterator`. This bridge keeps polling semantics in one place and avoids source-expression
    /// heuristics for individual adapters such as `take`, `zip`, or `enumerate`.
    pub(in crate::backend::ir::emit) fn emit_incan_iterator_source(
        &self,
        iterable: &TypedExpr,
    ) -> Result<Option<TokenStream>, EmitError> {
        if !iterable.ty.is_iterator_protocol() {
            return Ok(None);
        }
        let source = self.emit_expr(iterable)?;
        let next = methods::iterator_methods::next_call(&quote! { __incan_iter });
        Ok(Some(quote! {{
            let mut __incan_iter = #source;
            std::iter::from_fn(move || #next)
        }}))
    }

    /// Build a typed tuple-field read for compiler-expanded tuple unpacking.
    pub(super) fn tuple_field_expr(expr: &TypedExpr, idx: usize, ty: IrType) -> TypedExpr {
        TypedExpr::new(
            IrExprKind::Field {
                object: Box::new(expr.clone()),
                field: idx.to_string(),
            },
            ty,
        )
        .with_span(expr.span)
    }

    /// Emit explicit callable-name metadata for a concrete function pointer.
    fn emit_register_callable_name(&self, callable: &TypedExpr, source_name: &str) -> Result<TokenStream, EmitError> {
        let IrType::Function { params, ret } = &callable.ty else {
            return Ok(quote! { () });
        };
        let Some(signature_key) = Self::callable_name_signature_key(params, ret) else {
            return Ok(quote! { () });
        };
        let register = Self::callable_name_register_ident(&signature_key);
        let fn_ty = self.emit_callable_fn_type(params, ret);
        let callable = self.emit_expr(callable)?;
        let source_name = Literal::string(source_name);
        Ok(quote! {{
            let __incan_callable: #fn_ty = #callable;
            #register(__incan_callable, #source_name);
        }})
    }

    /// Emit a cached wrapper for a generic decorated function.
    fn emit_cache_generic_decorated_function(
        &self,
        cache_name: &str,
        type_param_names: &[String],
        value: &TypedExpr,
    ) -> Result<TokenStream, EmitError> {
        if !matches!(value.ty, IrType::Function { .. }) {
            return Err(EmitError::Unsupported(
                "generic decorated function cache requires a function pointer type".to_string(),
            ));
        }
        let cache_ident = Self::rust_static_ident(&format!("__incan_generic_decorated_{cache_name}"));
        let fn_ty = self.emit_type(&value.ty);
        let value_tokens = self.emit_expr(value)?;
        let type_key_parts = type_param_names
            .iter()
            .map(|name| {
                let ident = Self::rust_ident(name);
                quote! { std::any::type_name::<#ident>() }
            })
            .collect::<Vec<_>>();
        let type_key = if type_key_parts.is_empty() {
            quote! { String::new() }
        } else {
            quote! { [#(#type_key_parts),*].join("\u{1f}") }
        };

        Ok(quote! {{
            static #cache_ident: std::sync::OnceLock<std::sync::Mutex<Vec<(String, #fn_ty)>>> =
                std::sync::OnceLock::new();
            let __incan_type_key = #type_key;
            let mut __incan_entries = #cache_ident
                .get_or_init(|| std::sync::Mutex::new(Vec::new()))
                .lock()
                .unwrap_or_else(|__incan_poisoned| __incan_poisoned.into_inner());
            if let Some((_, __incan_cached)) = __incan_entries
                .iter()
                .find(|(__incan_key, _)| __incan_key == &__incan_type_key)
            {
                *__incan_cached
            } else {
                let __incan_decorated = #value_tokens;
                __incan_entries.push((__incan_type_key, __incan_decorated));
                __incan_decorated
            }
        }})
    }

    /// Emit one list-literal element, materializing owned sink semantics at the literal boundary.
    ///
    /// Incan `list[str]` literals should store owned Rust `String` elements up front, but ordinary Incan-to-Incan
    /// helper calls should not re-lower already-owned `list[str]` variables through consuming iterator conversions.
    /// Keeping this as a dedicated helper makes that ownership rule explicit instead of leaking a more incidental
    /// conversion context into the call site.
    fn emit_list_literal_item(
        &self,
        item: &TypedExpr,
        item_target_ty: Option<&IrType>,
        target_union_qualifier: Option<&[String]>,
    ) -> Result<TokenStream, EmitError> {
        self.emit_expr_for_use_with_union_qualifier(
            item,
            ValueUseSite::CollectionElement {
                target_ty: item_target_ty,
            },
            target_union_qualifier,
        )
    }

    /// Emit a list literal while preserving direct and spread entry order.
    fn emit_list_literal_entries(
        &self,
        items: &[IrListEntry],
        item_target_ty: Option<&IrType>,
        target_union_qualifier: Option<&[String]>,
    ) -> Result<TokenStream, EmitError> {
        if items.iter().all(|entry| matches!(entry, IrListEntry::Element(_))) {
            let item_tokens: Vec<TokenStream> = items
                .iter()
                .map(|entry| match entry {
                    IrListEntry::Element(item) => {
                        self.emit_list_literal_item(item, item_target_ty, target_union_qualifier)
                    }
                    IrListEntry::Spread(_) => Err(EmitError::InternalInvariant(
                        "unexpected list spread in direct-only literal emission".to_string(),
                    )),
                })
                .collect::<Result<_, _>>()?;
            return Ok(quote! { vec![#(#item_tokens),*] });
        }

        let steps: Vec<TokenStream> = items
            .iter()
            .map(|entry| match entry {
                IrListEntry::Element(item) => {
                    let item_tokens = self.emit_list_literal_item(item, item_target_ty, target_union_qualifier)?;
                    Ok(quote! { __incan_list.push(#item_tokens); })
                }
                IrListEntry::Spread(value) => {
                    if let IrType::Tuple(items) = &value.ty {
                        let mut pushes = Vec::with_capacity(items.len());
                        for (idx, item_ty) in items.iter().enumerate() {
                            let item = Self::tuple_field_expr(value, idx, item_ty.clone());
                            let item_tokens =
                                self.emit_list_literal_item(&item, item_target_ty, target_union_qualifier)?;
                            pushes.push(quote! { __incan_list.push(#item_tokens); });
                        }
                        Ok(quote! { #(#pushes)* })
                    } else {
                        let value_tokens = self.emit_expr(value)?;
                        Ok(quote! { __incan_list.extend((#value_tokens).into_iter()); })
                    }
                }
            })
            .collect::<Result<_, EmitError>>()?;

        Ok(quote! {{
            let mut __incan_list = Vec::new();
            #(#steps)*
            __incan_list
        }})
    }

    /// Emit a dictionary literal while preserving direct and spread entry order.
    fn emit_dict_literal_entries(
        &self,
        pairs: &[IrDictEntry],
        key_target_ty: Option<&IrType>,
        value_target_ty: Option<&IrType>,
        target_union_qualifier: Option<&[String]>,
    ) -> Result<TokenStream, EmitError> {
        if pairs.is_empty() {
            return Ok(quote! { std::collections::HashMap::new() });
        }

        if pairs.iter().all(|entry| matches!(entry, IrDictEntry::Pair(_, _))) {
            let pair_tokens: Vec<TokenStream> = pairs
                .iter()
                .map(|entry| match entry {
                    IrDictEntry::Pair(key, value) => {
                        let key_tokens = self.emit_expr_for_use_with_union_qualifier(
                            key,
                            ValueUseSite::CollectionElement {
                                target_ty: key_target_ty,
                            },
                            target_union_qualifier,
                        )?;
                        let value_tokens = self.emit_expr_for_use_with_union_qualifier(
                            value,
                            ValueUseSite::CollectionElement {
                                target_ty: value_target_ty,
                            },
                            target_union_qualifier,
                        )?;
                        Ok(quote! { (#key_tokens, #value_tokens) })
                    }
                    IrDictEntry::Spread(_) => Err(EmitError::InternalInvariant(
                        "unexpected dict spread in direct-only literal emission".to_string(),
                    )),
                })
                .collect::<Result<_, EmitError>>()?;
            return Ok(quote! { [#(#pair_tokens),*].into_iter().collect::<std::collections::HashMap<_, _>>() });
        }

        let steps: Vec<TokenStream> = pairs
            .iter()
            .map(|entry| match entry {
                IrDictEntry::Pair(key, value) => {
                    let key_tokens = self.emit_expr_for_use_with_union_qualifier(
                        key,
                        ValueUseSite::CollectionElement {
                            target_ty: key_target_ty,
                        },
                        target_union_qualifier,
                    )?;
                    let value_tokens = self.emit_expr_for_use_with_union_qualifier(
                        value,
                        ValueUseSite::CollectionElement {
                            target_ty: value_target_ty,
                        },
                        target_union_qualifier,
                    )?;
                    Ok(quote! { __incan_dict.insert(#key_tokens, #value_tokens); })
                }
                IrDictEntry::Spread(value) => {
                    let value_tokens = self.emit_expr(value)?;
                    Ok(quote! {
                        for (__incan_key, __incan_value) in (#value_tokens).into_iter() {
                            __incan_dict.insert(__incan_key, __incan_value);
                        }
                    })
                }
            })
            .collect::<Result<_, EmitError>>()?;

        Ok(quote! {{
            let mut __incan_dict = std::collections::HashMap::new();
            #(#steps)*
            __incan_dict
        }})
    }

    /// Return the target type carried by a value-use site, if the site has one.
    fn use_site_target_ty<'b>(site: ValueUseSite<'b>) -> Option<&'b IrType> {
        value_use_site_target_ty(site)
    }

    /// Prefer the call-site target type for aggregate literal elements.
    ///
    /// Generic targets still matter for ownership conversion: a string literal passed into `list[K]` should materialize
    /// as an owned `String` for Incan calls, not leak Rust's `&str` literal type into the generic container.
    fn concrete_literal_target<'b>(
        target_ty: Option<&'b IrType>,
        inferred_ty: Option<&'b IrType>,
    ) -> Option<&'b IrType> {
        match target_ty {
            Some(ty) => Some(ty),
            None => inferred_ty,
        }
    }

    /// Rebuild a parent value-use site for one tuple item while preserving the parent ownership context.
    fn tuple_item_use_site<'b>(site: ValueUseSite<'b>, target_ty: Option<&'b IrType>) -> ValueUseSite<'b> {
        Self::retarget_value_use_site(site, target_ty)
    }

    /// Rebuild a value-use site with a more specific target type while preserving the context kind.
    fn retarget_value_use_site<'b>(site: ValueUseSite<'b>, target_ty: Option<&'b IrType>) -> ValueUseSite<'b> {
        match site {
            ValueUseSite::IncanCallArg { in_return, .. } => ValueUseSite::IncanCallArg {
                target_ty,
                callee_param: None,
                in_return,
            },
            ValueUseSite::ExternalCallArg { .. } => ValueUseSite::ExternalCallArg { target_ty },
            ValueUseSite::ExternalInferredGenericArg { .. } => ValueUseSite::ExternalInferredGenericArg { target_ty },
            ValueUseSite::StructField { .. } => ValueUseSite::StructField { target_ty },
            ValueUseSite::CollectionElement { .. } => ValueUseSite::CollectionElement { target_ty },
            ValueUseSite::Assignment { .. } => ValueUseSite::Assignment { target_ty },
            ValueUseSite::ReturnValue { .. } => ValueUseSite::ReturnValue { target_ty },
            ValueUseSite::MatchScrutinee { .. } => ValueUseSite::MatchScrutinee { target_ty },
            ValueUseSite::MethodArg => ValueUseSite::MethodArg,
        }
    }

    /// Return the semantic list type and owner qualifier that should drive element-level union widening.
    ///
    /// Imported public calls can carry dependency-owned generated union wrappers inside a list. This mirrors scalar
    /// union widening source discovery so non-literal list arguments do not fall back to a caller-owned wrapper shape.
    pub(in super::super) fn list_element_widening_source_for_expr(
        &self,
        expr: &TypedExpr,
    ) -> (IrType, Option<Vec<String>>) {
        match &expr.kind {
            IrExprKind::Call {
                callable_signature: Some(signature),
                canonical_path,
                ..
            } if self
                .list_element_union_type(&signature.return_type)
                .is_some_and(|elem| elem.union_members().is_some()) =>
            {
                (
                    signature.return_type.clone(),
                    Self::pub_library_union_qualifier(canonical_path.as_deref()),
                )
            }
            IrExprKind::MethodCall {
                callable_signature: Some(signature),
                ..
            } if self
                .list_element_union_type(&signature.return_type)
                .is_some_and(|elem| elem.union_members().is_some()) =>
            {
                (signature.return_type.clone(), None)
            }
            _ => (expr.ty.clone(), None),
        }
    }

    /// Return the resolved element type for a list shape.
    fn list_element_union_type(&self, ty: &IrType) -> Option<IrType> {
        match self.resolve_type_aliases_for_emit(ty) {
            IrType::List(elem) => Some(*elem),
            _ => None,
        }
    }

    /// Return whether `List[S]` needs an element-wise union conversion to satisfy `List[T]`.
    pub(in super::super) fn list_element_widening_needed(&self, source_ty: &IrType, target_ty: &IrType) -> bool {
        let source_ty = self.resolve_type_aliases_for_emit(source_ty);
        let target_ty = self.resolve_type_aliases_for_emit(target_ty);
        let (IrType::List(source_elem), IrType::List(target_elem)) = (&source_ty, &target_ty) else {
            return false;
        };
        if source_elem == target_elem {
            return false;
        }
        target_elem.union_variant_index_for_member(source_elem).is_some()
            || self.union_widening_needed(source_elem, target_elem)
    }

    /// Emit an element-wise conversion from `List[S]` to `List[Union[...S...]]` or a wider union list.
    fn emit_list_element_widening_value(
        &self,
        source_ty: &IrType,
        target_ty: &IrType,
        source_tokens: TokenStream,
        source_qualifier: Option<&[String]>,
        target_qualifier: Option<&[String]>,
    ) -> Result<Option<TokenStream>, EmitError> {
        let source_ty = self.resolve_type_aliases_for_emit(source_ty);
        let target_ty = self.resolve_type_aliases_for_emit(target_ty);
        let (IrType::List(source_elem), IrType::List(target_elem)) = (&source_ty, &target_ty) else {
            return Ok(None);
        };
        if source_elem == target_elem {
            return Ok(None);
        }

        if let Some(converted_item) = self.emit_union_widening_value(
            source_elem,
            target_elem,
            quote! { __incan_item },
            source_qualifier,
            target_qualifier,
        )? {
            return Ok(Some(quote! {
                (#source_tokens).into_iter().map(|__incan_item| #converted_item).collect::<Vec<_>>()
            }));
        }

        let Some(variant_index) = target_elem.union_variant_index_for_member(source_elem) else {
            return Ok(None);
        };
        let Some(members) = target_elem.union_members() else {
            return Ok(None);
        };
        let Some(member_ty) = members.get(variant_index) else {
            return Ok(None);
        };
        let item_tokens = if matches!(member_ty, IrType::String)
            && matches!(
                source_elem.as_ref(),
                IrType::StaticStr | IrType::StrRef | IrType::FrozenStr
            ) {
            quote! { __incan_item.to_string() }
        } else {
            quote! { __incan_item }
        };
        let variant_ident = quote::format_ident!("{}", IrType::union_variant_name(variant_index));
        let union_path = self.emit_union_type_path_with_qualifier(target_elem, target_qualifier);
        Ok(Some(quote! {
            (#source_tokens)
                .into_iter()
                .map(|__incan_item| #union_path :: #variant_ident(#item_tokens))
                .collect::<Vec<_>>()
        }))
    }

    /// Return the `Result[output, error]` target type for the inner expression of `output?`.
    fn try_inner_target_type(&self, output_ty: &IrType, inner: &TypedExpr) -> Option<IrType> {
        if matches!(output_ty, IrType::Unknown) {
            return None;
        }
        let err_ty = match &inner.ty {
            IrType::Result(_, err_ty) => Some(err_ty.as_ref().clone()),
            _ => self
                .current_function_return_type
                .borrow()
                .as_ref()
                .and_then(|return_ty| {
                    if let IrType::Result(_, err_ty) = return_ty {
                        Some(err_ty.as_ref().clone())
                    } else {
                        None
                    }
                }),
        }?;
        Some(IrType::Result(Box::new(output_ty.clone()), Box::new(err_ty)))
    }

    /// Emit an expression directly against an ownership-planned sink/source boundary.
    ///
    /// Aggregate literals are handled recursively so element-level ownership policy is applied before the outer
    /// expression is emitted. Non-aggregate expressions are emitted normally, then the planned conversion is applied to
    /// the resulting token stream.
    pub(super) fn emit_expr_for_use(&self, expr: &TypedExpr, site: ValueUseSite<'_>) -> Result<TokenStream, EmitError> {
        self.emit_expr_for_use_with_union_qualifier(expr, site, None)
    }

    /// Emit an expression for a value-use site while preserving the owner of generated anonymous union wrappers.
    ///
    /// Public dependency calls use provider-owned wrapper types. Passing the qualifier through target-aware aggregate
    /// and union-widening emission keeps nested generated wrapper paths rooted in the dependency instead of
    /// accidentally re-owning them in the consuming crate.
    pub(super) fn emit_expr_for_use_with_union_qualifier(
        &self,
        expr: &TypedExpr,
        site: ValueUseSite<'_>,
        target_union_qualifier: Option<&[String]>,
    ) -> Result<TokenStream, EmitError> {
        let resolved_target_ty = Self::use_site_target_ty(site).map(|ty| self.resolve_type_aliases_for_emit(ty));
        let target_type_union_qualifier = resolved_target_ty
            .as_ref()
            .and_then(Self::external_union_qualifier_for_type);
        let target_union_qualifier = target_type_union_qualifier.as_deref().or(target_union_qualifier);
        if let Some(target_ty) = resolved_target_ty.as_ref() {
            match (&expr.kind, target_ty) {
                (IrExprKind::String(value), IrType::FrozenStr) => {
                    return Ok(quote! { incan_stdlib::frozen::FrozenStr::new(#value) });
                }
                (IrExprKind::Bytes(bytes), IrType::FrozenBytes) => {
                    let lit = Literal::byte_string(bytes);
                    return Ok(quote! { incan_stdlib::frozen::FrozenBytes::new(#lit) });
                }
                _ => {}
            }
            if let Some(wrapped) =
                self.emit_union_payload_arg_for_site(expr, target_ty, target_union_qualifier, site)?
            {
                return Ok(wrapped);
            }
            if matches!(site, ValueUseSite::CollectionElement { .. })
                && let Some(wrapped) = self.emit_inference_seeded_literal_arg_with_union_qualifier(
                    expr,
                    target_ty,
                    target_union_qualifier,
                )?
            {
                return Ok(wrapped);
            }
        }

        match &expr.kind {
            IrExprKind::InteropCoerce { expr: inner, .. }
                if Self::use_site_target_ty(site).is_some()
                    && matches!(
                        inner.kind,
                        IrExprKind::List(_) | IrExprKind::Dict(_) | IrExprKind::Set(_) | IrExprKind::Tuple(_)
                    ) =>
            {
                return self.emit_expr_for_use_with_union_qualifier(inner, site, target_union_qualifier);
            }
            IrExprKind::InteropCoerce { expr: inner, .. }
                if Self::use_site_target_ty(site).is_some()
                    && matches!(inner.kind, IrExprKind::Call { .. } | IrExprKind::MethodCall { .. }) =>
            {
                return self.emit_expr_for_use_with_union_qualifier(inner, site, target_union_qualifier);
            }
            IrExprKind::List(items) => {
                let site_item_ty = match resolved_target_ty.as_ref() {
                    Some(IrType::List(elem)) => Some(elem.as_ref()),
                    _ => None,
                };
                let inferred_item_ty = match &expr.ty {
                    IrType::List(elem) => Some(elem.as_ref()),
                    _ => None,
                };
                let item_target_ty = Self::concrete_literal_target(site_item_ty, inferred_item_ty);
                return self.emit_list_literal_entries(items, item_target_ty, target_union_qualifier);
            }
            IrExprKind::Dict(pairs) => {
                let (site_key_ty, site_value_ty) = match resolved_target_ty.as_ref() {
                    Some(IrType::Dict(key, value)) => (Some(key.as_ref()), Some(value.as_ref())),
                    _ => (None, None),
                };
                let (inferred_key_ty, inferred_value_ty) = match &expr.ty {
                    IrType::Dict(key, value) => (Some(key.as_ref()), Some(value.as_ref())),
                    _ => (None, None),
                };
                let key_target_ty = Self::concrete_literal_target(site_key_ty, inferred_key_ty);
                let value_target_ty = Self::concrete_literal_target(site_value_ty, inferred_value_ty);
                return self.emit_dict_literal_entries(pairs, key_target_ty, value_target_ty, target_union_qualifier);
            }
            IrExprKind::Set(items) => {
                if items.is_empty() {
                    return Ok(quote! { std::collections::HashSet::new() });
                }
                let site_item_ty = match resolved_target_ty.as_ref() {
                    Some(IrType::Set(elem)) => Some(elem.as_ref()),
                    _ => None,
                };
                let inferred_item_ty = match &expr.ty {
                    IrType::Set(elem) => Some(elem.as_ref()),
                    _ => None,
                };
                let item_target_ty = Self::concrete_literal_target(site_item_ty, inferred_item_ty);
                let item_tokens: Vec<TokenStream> = items
                    .iter()
                    .map(|item| {
                        self.emit_expr_for_use_with_union_qualifier(
                            item,
                            ValueUseSite::CollectionElement {
                                target_ty: item_target_ty,
                            },
                            target_union_qualifier,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                return Ok(quote! { [#(#item_tokens),*].into_iter().collect::<std::collections::HashSet<_>>() });
            }
            IrExprKind::Tuple(items) => {
                let site_tuple_items = match resolved_target_ty.as_ref() {
                    Some(IrType::Tuple(items)) => Some(items.as_slice()),
                    _ => None,
                };
                let inferred_tuple_items = match &expr.ty {
                    IrType::Tuple(items) => Some(items.as_slice()),
                    _ => None,
                };
                let item_tokens: Vec<TokenStream> = items
                    .iter()
                    .enumerate()
                    .map(|(idx, item)| {
                        let site_item_ty = site_tuple_items.and_then(|items| items.get(idx));
                        let inferred_item_ty = inferred_tuple_items.and_then(|items| items.get(idx));
                        let item_target_ty = Self::concrete_literal_target(site_item_ty, inferred_item_ty);
                        self.emit_expr_for_use_with_union_qualifier(
                            item,
                            Self::tuple_item_use_site(site, item_target_ty),
                            target_union_qualifier,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                return Ok(quote! { (#(#item_tokens),*) });
            }
            IrExprKind::Try(inner) => {
                let site_target_ty = Self::use_site_target_ty(site);
                let inner_tokens = if let Some(inner_target_ty) =
                    site_target_ty.and_then(|target_ty| self.try_inner_target_type(target_ty, inner))
                {
                    self.emit_expr_for_use_with_union_qualifier(
                        inner,
                        Self::retarget_value_use_site(site, Some(&inner_target_ty)),
                        target_union_qualifier,
                    )?
                } else {
                    self.emit_expr(inner)?
                };
                return Ok(quote! { #inner_tokens? });
            }
            IrExprKind::MethodCall {
                receiver,
                method,
                dispatch,
                type_args,
                args,
                callable_signature,
                arg_policy,
            } => {
                let emitted = self.emit_method_call_expr_for_use(
                    receiver,
                    method,
                    dispatch.as_ref(),
                    type_args,
                    args,
                    callable_signature.as_ref(),
                    *arg_policy,
                    site,
                )?;
                if let Some(target_ty) = resolved_target_ty.as_ref() {
                    let (source_ty, source_qualifier) = self.list_element_widening_source_for_expr(expr);
                    if let Some(converted) = self.emit_list_element_widening_value(
                        &source_ty,
                        target_ty,
                        emitted.clone(),
                        source_qualifier.as_deref(),
                        target_union_qualifier,
                    )? {
                        return Ok(converted);
                    }
                    let (source_ty, source_qualifier) = self.union_widening_source_for_expr(expr);
                    if let Some(converted) = self.emit_union_widening_value(
                        &source_ty,
                        target_ty,
                        emitted.clone(),
                        source_qualifier.as_deref(),
                        target_union_qualifier,
                    )? {
                        return Ok(converted);
                    }
                }
                return Ok(emitted);
            }
            IrExprKind::Call {
                func,
                type_args,
                args,
                callable_signature,
                canonical_path,
            } => {
                let target_site = if let Some(target_ty) = resolved_target_ty.as_ref() {
                    Self::retarget_value_use_site(site, Some(target_ty))
                } else {
                    site
                };
                let emitted = self.emit_call_expr_for_use(
                    func,
                    type_args,
                    args,
                    callable_signature.as_ref(),
                    canonical_path.as_deref(),
                    target_site,
                )?;
                if let Some(target_ty) = resolved_target_ty.as_ref() {
                    let (source_ty, source_qualifier) = self.list_element_widening_source_for_expr(expr);
                    if let Some(converted) = self.emit_list_element_widening_value(
                        &source_ty,
                        target_ty,
                        emitted.clone(),
                        source_qualifier.as_deref(),
                        target_union_qualifier,
                    )? {
                        return Ok(converted);
                    }
                    let (source_ty, source_qualifier) = self.union_widening_source_for_expr(expr);
                    if let Some(converted) = self.emit_union_widening_value(
                        &source_ty,
                        target_ty,
                        emitted.clone(),
                        source_qualifier.as_deref(),
                        target_union_qualifier,
                    )? {
                        return Ok(converted);
                    }
                }
                return Ok(emitted);
            }
            _ => {}
        }

        let emitted = self.emit_expr(expr)?;
        let plan = plan_value_use(expr, site);
        let emitted = plan.apply(emitted);
        if let Some(target_ty) = resolved_target_ty.as_ref() {
            let (source_ty, source_qualifier) = self.list_element_widening_source_for_expr(expr);
            if let Some(converted) = self.emit_list_element_widening_value(
                &source_ty,
                target_ty,
                emitted.clone(),
                source_qualifier.as_deref(),
                target_union_qualifier,
            )? {
                return Ok(converted);
            }
            let (source_ty, source_qualifier) = self.union_widening_source_for_expr(expr);
            if let Some(converted) = self.emit_union_widening_value(
                &source_ty,
                target_ty,
                emitted.clone(),
                source_qualifier.as_deref(),
                target_union_qualifier,
            )? {
                return Ok(converted);
            }
        }
        Ok(emitted)
    }

    /// Return the dependency qualifier carried by an external anonymous union target type.
    fn external_union_qualifier_for_type(ty: &IrType) -> Option<Vec<String>> {
        match ty {
            IrType::ExternalUnion { library, .. } => Some(vec![library.clone()]),
            IrType::List(inner)
            | IrType::Set(inner)
            | IrType::Option(inner)
            | IrType::Ref(inner)
            | IrType::RefMut(inner)
            | IrType::TypeToken(inner) => Self::external_union_qualifier_for_type(inner),
            IrType::Dict(key, value) | IrType::Result(key, value) => {
                Self::external_union_qualifier_for_type(key).or_else(|| Self::external_union_qualifier_for_type(value))
            }
            IrType::Tuple(items) | IrType::NamedGeneric(_, items) => {
                items.iter().find_map(Self::external_union_qualifier_for_type)
            }
            IrType::Function { params, ret } => params
                .iter()
                .find_map(Self::external_union_qualifier_for_type)
                .or_else(|| Self::external_union_qualifier_for_type(ret)),
            _ => None,
        }
    }

    /// Return whether match scrutinee emission should preserve a `Result` value without extra ownership shaping.
    fn type_is_result_like(ty: &IrType) -> bool {
        match ty {
            IrType::Result(_, _) => true,
            IrType::NamedGeneric(name, args) if args.len() == 2 => {
                collections::from_str(name.rsplit("::").next().unwrap_or(name)) == Some(CollectionTypeId::Result)
            }
            _ => false,
        }
    }

    /// Emit the scrutinee expression for a match statement.
    pub(super) fn emit_match_scrutinee(&self, scrutinee: &TypedExpr) -> Result<TokenStream, EmitError> {
        if matches!(scrutinee.ty, IrType::Unknown) || Self::type_is_result_like(&scrutinee.ty) {
            return self.emit_expr(scrutinee);
        }
        self.emit_expr_for_use(
            scrutinee,
            ValueUseSite::MatchScrutinee {
                target_ty: Some(&scrutinee.ty),
            },
        )
    }

    /// Check whether an expression is a type-like identifier that should use Rust path syntax.
    ///
    /// This covers Incan type names, enum variants, module placeholders, and external Rust imports.
    pub(super) fn expr_is_type_like(expr: &TypedExpr) -> bool {
        match &expr.kind {
            IrExprKind::Var { ref_kind, .. } => {
                matches!(
                    ref_kind,
                    VarRefKind::TypeName | VarRefKind::ExternalName | VarRefKind::ExternalRustName
                )
            }
            _ => false,
        }
    }

    pub(super) fn expr_storage_root(expr: &TypedExpr) -> Option<StorageRoot> {
        match &expr.kind {
            IrExprKind::StaticRead { name } => Some(StorageRoot::Static(name.clone())),
            IrExprKind::Var {…22765 tokens truncated…              kind: MethodKind::Collection(CollectionMethodKind::Get),
                args: vec![IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(IrExprKind::String("the".to_string()), IrType::String),
                }],
            },
            IrType::Option(Box::new(IrType::Int)),
        );

        let emitted = emitter
            .emit_expr(&expr)
            .map_err(|err| format!("expected successful expression emission, got {err:?}"))?;
        let rendered = emitted.to_string();
        assert!(
            rendered.contains("counts . get (< _ as AsRef < str >> :: as_ref (& \"the\"))"),
            "expected string-key dict lookup to normalize via fully-qualified `AsRef<str>`, got `{rendered}`"
        );
        Ok(())
    }

    #[test]
    fn dict_index_with_string_literal_uses_str_lookup_shape() -> Result<(), String> {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);
        let expr = TypedExpr::new(
            IrExprKind::Index {
                object: Box::new(TypedExpr::new(
                    IrExprKind::Var {
                        name: "counts".to_string(),
                        access: VarAccess::Read,
                        ref_kind: VarRefKind::Value,
                    },
                    IrType::Dict(Box::new(IrType::String), Box::new(IrType::Int)),
                )),
                index: Box::new(TypedExpr::new(IrExprKind::String("the".to_string()), IrType::String)),
            },
            IrType::Int,
        );

        let emitted = emitter
            .emit_expr(&expr)
            .map_err(|err| format!("expected successful expression emission, got {err:?}"))?;
        let rendered = emitted.to_string();
        assert!(
            rendered.contains(
                "incan_stdlib :: collections :: dict_get (& counts , < _ as AsRef < str >> :: as_ref (& \"the\"))"
            ),
            "expected dict index to normalize string probes via fully-qualified `AsRef<str>`, got `{rendered}`"
        );
        Ok(())
    }

    #[test]
    fn known_list_methods_emit_checked_runtime_helpers() -> Result<(), String> {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);

        let receiver = || {
            Box::new(TypedExpr::new(
                IrExprKind::Var {
                    name: "items".to_string(),
                    access: VarAccess::Read,
                    ref_kind: VarRefKind::Value,
                },
                IrType::List(Box::new(IrType::Int)),
            ))
        };

        let render = |expr: TypedExpr| -> Result<String, String> {
            emitter
                .emit_expr(&expr)
                .map(|tokens| tokens.to_string())
                .map_err(|err| format!("expected successful expression emission, got {err:?}"))
        };

        let index_rendered = render(TypedExpr::new(
            IrExprKind::KnownMethodCall {
                receiver: receiver(),
                kind: MethodKind::Collection(CollectionMethodKind::Index),
                args: vec![IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(IrExprKind::Int(9), IrType::Int),
                }],
            },
            IrType::Int,
        ))?;
        assert!(
            index_rendered.contains("incan_stdlib :: collections :: list_index (& items , & 9)"),
            "expected list.index to route through checked runtime helper, got `{index_rendered}`"
        );

        let count_rendered = render(TypedExpr::new(
            IrExprKind::KnownMethodCall {
                receiver: receiver(),
                kind: MethodKind::Collection(CollectionMethodKind::Count),
                args: vec![IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(IrExprKind::Int(9), IrType::Int),
                }],
            },
            IrType::Int,
        ))?;
        assert!(
            count_rendered.contains("incan_stdlib :: collections :: list_count (& items , & 9)"),
            "expected list.count to route through checked runtime helper, got `{count_rendered}`"
        );

        let remove_rendered = render(TypedExpr::new(
            IrExprKind::KnownMethodCall {
                receiver: receiver(),
                kind: MethodKind::Collection(CollectionMethodKind::Remove),
                args: vec![IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(IrExprKind::Int(9), IrType::Int),
                }],
            },
            IrType::Unit,
        ))?;
        assert!(
            remove_rendered.contains("incan_stdlib :: collections :: list_remove"),
            "expected list.remove to route through checked runtime helper, got `{remove_rendered}`"
        );

        let swap_rendered = render(TypedExpr::new(
            IrExprKind::KnownMethodCall {
                receiver: receiver(),
                kind: MethodKind::Collection(CollectionMethodKind::Swap),
                args: vec![
                    IrCallArg {
                        name: None,
                        kind: IrCallArgKind::Positional,
                        expr: TypedExpr::new(IrExprKind::Int(0), IrType::Int),
                    },
                    IrCallArg {
                        name: None,
                        kind: IrCallArgKind::Positional,
                        expr: TypedExpr::new(IrExprKind::Int(9), IrType::Int),
                    },
                ],
            },
            IrType::Unit,
        ))?;
        assert!(
            swap_rendered.contains("incan_stdlib :: collections :: list_swap"),
            "expected list.swap to route through checked runtime helper, got `{swap_rendered}`"
        );

        Ok(())
    }

    #[test]
    fn external_nominal_method_call_keeps_external_string_conversion() -> Result<(), String> {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);
        let expr = TypedExpr::new(
            IrExprKind::MethodCall {
                receiver: Box::new(TypedExpr::new(
                    IrExprKind::Var {
                        name: "builder".to_string(),
                        access: VarAccess::Read,
                        ref_kind: VarRefKind::Value,
                    },
                    IrType::Struct("ExternalBuilder".to_string()),
                )),
                method: "rename".to_string(),
                dispatch: None,
                type_args: Vec::new(),
                args: vec![IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(IrExprKind::String("logs".to_string()), IrType::String),
                }],
                callable_signature: None,
                arg_policy: MethodCallArgPolicy::Default,
            },
            IrType::Unknown,
        );

        let emitted = emitter
            .emit_expr(&expr)
            .map_err(|err| format!("expected successful expression emission, got {err:?}"))?;
        let rendered = emitted.to_string();
        assert!(
            rendered.contains("builder . rename (\"logs\" . into ())"),
            "expected external nominal method call to preserve `.into()` coercion, got `{rendered}`"
        );
        assert!(
            !rendered.contains("\"logs\" . to_string ()"),
            "external nominal method call must not use Incan-owned string coercion, got `{rendered}`"
        );
        Ok(())
    }

    #[test]
    fn internal_dependency_nominal_method_call_does_not_borrow_string_arguments() -> Result<(), String> {
        let registry = FunctionRegistry::new();
        let mut emitter = IrEmitter::new(&registry);
        emitter.set_type_module_paths(
            std::collections::HashMap::from([("Session".to_string(), vec!["session".to_string()])]),
            std::collections::HashSet::new(),
        );
        let expr = TypedExpr::new(
            IrExprKind::MethodCall {
                receiver: Box::new(TypedExpr::new(
                    IrExprKind::Var {
                        name: "session".to_string(),
                        access: VarAccess::Read,
                        ref_kind: VarRefKind::Value,
                    },
                    IrType::Struct("Session".to_string()),
                )),
                method: "read_csv".to_string(),
                dispatch: None,
                type_args: vec![IrType::Struct("OrderLine".to_string())],
                args: vec![
                    IrCallArg {
                        name: None,
                        kind: IrCallArgKind::Positional,
                        expr: TypedExpr::new(IrExprKind::String("order_lines".to_string()), IrType::String),
                    },
                    IrCallArg {
                        name: None,
                        kind: IrCallArgKind::Positional,
                        expr: TypedExpr::new(
                            IrExprKind::Var {
                                name: "input_uri".to_string(),
                                access: VarAccess::Read,
                                ref_kind: VarRefKind::Value,
                            },
                            IrType::String,
                        ),
                    },
                ],
                callable_signature: None,
                arg_policy: MethodCallArgPolicy::Default,
            },
            IrType::Unknown,
        );

        let emitted = emitter
            .emit_expr(&expr)
            .map_err(|err| format!("expected successful expression emission, got {err:?}"))?;
        let rendered = emitted.to_string();
        assert!(
            rendered.contains("session . read_csv :: < OrderLine >"),
            "expected regular method-call emission on internal dependency type, got `{rendered}`"
        );
        assert!(
            !rendered.contains("& input_uri") && !rendered.contains("&input_uri"),
            "internal dependency method call must not borrow owned string args like an external Rust receiver, got `{rendered}`"
        );
        Ok(())
    }

    #[test]
    fn external_name_namespace_call_uses_incan_function_arg_conversion() -> Result<(), String> {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);
        let expr = TypedExpr::new(
            IrExprKind::MethodCall {
                receiver: Box::new(TypedExpr::new(
                    IrExprKind::Var {
                        name: "widgets".to_string(),
                        access: VarAccess::Read,
                        ref_kind: VarRefKind::ExternalName,
                    },
                    IrType::Struct("widgets".to_string()),
                )),
                method: "make_widget".to_string(),
                dispatch: None,
                type_args: Vec::new(),
                args: vec![IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(
                        IrExprKind::Var {
                            name: "DEFAULT_NAME".to_string(),
                            access: VarAccess::Read,
                            ref_kind: VarRefKind::Value,
                        },
                        IrType::String,
                    ),
                }],
                callable_signature: None,
                arg_policy: MethodCallArgPolicy::Default,
            },
            IrType::Unknown,
        );

        let emitted = emitter
            .emit_expr(&expr)
            .map_err(|err| format!("expected successful expression emission, got {err:?}"))?;
        let rendered = emitted.to_string();
        assert!(
            rendered.contains("widgets :: make_widget (DEFAULT_NAME"),
            "expected namespace call to stay on the ordinary function-conversion path, got `{rendered}`"
        );
        assert!(
            !rendered.contains("& DEFAULT_NAME"),
            "namespace call must not borrow owned string args like an external Rust receiver, got `{rendered}`"
        );
        assert!(
            !rendered.contains(". into ()"),
            "namespace call must not apply external-Rust `.into()` coercions, got `{rendered}`"
        );
        Ok(())
    }

    #[test]
    fn external_rust_call_coerces_list_elements_to_target_vec_element() -> Result<(), String> {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);
        let expr = TypedExpr::new(
            IrExprKind::Call {
                func: Box::new(TypedExpr::new(
                    IrExprKind::Var {
                        name: "build_frame".to_string(),
                        access: VarAccess::Copy,
                        ref_kind: VarRefKind::ExternalRustName,
                    },
                    IrType::Function {
                        params: vec![IrType::List(Box::new(IrType::Struct(
                            "polars::prelude::Column".to_string(),
                        )))],
                        ret: Box::new(IrType::Unit),
                    },
                )),
                type_args: Vec::new(),
                args: vec![IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(
                        IrExprKind::Var {
                            name: "columns".to_string(),
                            access: VarAccess::Move,
                            ref_kind: VarRefKind::Value,
                        },
                        IrType::List(Box::new(IrType::Struct("polars::series::Series".to_string()))),
                    ),
                }],
                callable_signature: None,
                canonical_path: None,
            },
            IrType::Unit,
        );

        let emitted = emitter
            .emit_expr(&expr)
            .map_err(|err| format!("expected successful expression emission, got {err:?}"))?;
        let rendered = emitted.to_string();
        assert!(
            rendered.contains("(columns) . into_iter () . map"),
            "expected external Rust list arg to map elements through Into, got `{rendered}`"
        );
        assert!(
            rendered.contains(":: std :: convert :: Into :: into"),
            "expected external Rust list arg to use fully qualified Into::into, got `{rendered}`"
        );
        assert!(
            rendered.contains("collect :: < Vec < _ >> ()"),
            "expected external Rust list arg to collect into Vec<_>, got `{rendered}`"
        );

        let literal_expr = TypedExpr::new(
            IrExprKind::Call {
                func: Box::new(TypedExpr::new(
                    IrExprKind::Var {
                        name: "build_frame".to_string(),
                        access: VarAccess::Copy,
                        ref_kind: VarRefKind::ExternalRustName,
                    },
                    IrType::Function {
                        params: vec![IrType::List(Box::new(IrType::Struct(
                            "polars::prelude::Column".to_string(),
                        )))],
                        ret: Box::new(IrType::Unit),
                    },
                )),
                type_args: Vec::new(),
                args: vec![IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(
                        IrExprKind::List(vec![
                            IrListEntry::Element(TypedExpr::new(
                                IrExprKind::Var {
                                    name: "id_series".to_string(),
                                    access: VarAccess::Move,
                                    ref_kind: VarRefKind::Value,
                                },
                                IrType::Struct("polars::series::Series".to_string()),
                            )),
                            IrListEntry::Element(TypedExpr::new(
                                IrExprKind::Var {
                                    name: "value_series".to_string(),
                                    access: VarAccess::Move,
                                    ref_kind: VarRefKind::Value,
                                },
                                IrType::Struct("polars::series::Series".to_string()),
                            )),
                        ]),
                        IrType::List(Box::new(IrType::Struct("polars::series::Series".to_string()))),
                    ),
                }],
                callable_signature: None,
                canonical_path: None,
            },
            IrType::Unit,
        );
        let literal_rendered = emitter
            .emit_expr(&literal_expr)
            .map_err(|err| format!("expected successful expression emission, got {err:?}"))?
            .to_string();
        assert!(
            literal_rendered.contains("vec ! [id_series , value_series]") && literal_rendered.contains("into_iter"),
            "expected list literal external arg to get element coercion wrapper, got `{literal_rendered}`"
        );
        Ok(())
    }

    #[test]
    fn external_rust_call_leaves_matching_list_elements_unmapped() -> Result<(), String> {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);
        let series_ty = IrType::Struct("polars::series::Series".to_string());
        let expr = TypedExpr::new(
            IrExprKind::Call {
                func: Box::new(TypedExpr::new(
                    IrExprKind::Var {
                        name: "use_series".to_string(),
                        access: VarAccess::Copy,
                        ref_kind: VarRefKind::ExternalRustName,
                    },
                    IrType::Function {
                        params: vec![IrType::List(Box::new(series_ty.clone()))],
                        ret: Box::new(IrType::Unit),
                    },
                )),
                type_args: Vec::new(),
                args: vec![IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(
                        IrExprKind::Var {
                            name: "series".to_string(),
                            access: VarAccess::Move,
                            ref_kind: VarRefKind::Value,
                        },
                        IrType::List(Box::new(series_ty)),
                    ),
                }],
                callable_signature: None,
                canonical_path: None,
            },
            IrType::Unit,
        );

        let emitted = emitter
            .emit_expr(&expr)
            .map_err(|err| format!("expected successful expression emission, got {err:?}"))?;
        let rendered = emitted.to_string();
        assert!(
            rendered.contains("use_series (series)"),
            "expected matching Vec element types to pass through directly, got `{rendered}`"
        );
        assert!(
            !rendered.contains("into_iter"),
            "matching Vec element types must not add element coercion, got `{rendered}`"
        );
        Ok(())
    }

    #[test]
    fn external_rust_associated_method_coerces_list_elements_to_target_vec_element() -> Result<(), String> {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);
        let expr = TypedExpr::new(
            IrExprKind::MethodCall {
                receiver: Box::new(TypedExpr::new(
                    IrExprKind::Var {
                        name: "DataFrame".to_string(),
                        access: VarAccess::Copy,
                        ref_kind: VarRefKind::ExternalRustName,
                    },
                    IrType::Struct("polars::prelude::DataFrame".to_string()),
                )),
                method: "new".to_string(),
                dispatch: None,
                type_args: Vec::new(),
                args: vec![IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(
                        IrExprKind::Var {
                            name: "columns".to_string(),
                            access: VarAccess::Move,
                            ref_kind: VarRefKind::Value,
                        },
                        IrType::List(Box::new(IrType::Struct("polars::series::Series".to_string()))),
                    ),
                }],
                callable_signature: Some(FunctionSignature {
                    params: vec![FunctionParam {
                        name: "columns".to_string(),
                        ty: IrType::List(Box::new(IrType::Struct("polars::prelude::Column".to_string()))),
                        mutability: Mutability::Immutable,
                        is_self: false,
                        kind: crate::frontend::ast::ParamKind::Normal,
                        default: None,
                    }],
                    return_type: IrType::Struct("polars::prelude::DataFrame".to_string()),
                }),
                arg_policy: MethodCallArgPolicy::Default,
            },
            IrType::Struct("polars::prelude::DataFrame".to_string()),
        );

        let emitted = emitter
            .emit_expr(&expr)
            .map_err(|err| format!("expected successful expression emission, got {err:?}"))?;
        let rendered = emitted.to_string();
        assert!(
            rendered.contains("DataFrame :: new ((columns) . into_iter () . map"),
            "expected external Rust associated method list arg to map elements through Into, got `{rendered}`"
        );
        assert!(
            rendered.contains(":: std :: convert :: Into :: into"),
            "expected external Rust associated method list arg to use fully qualified Into::into, got `{rendered}`"
        );
        assert!(
            rendered.contains("collect :: < Vec < _ >> ()"),
            "expected external Rust associated method list arg to collect into Vec<_>, got `{rendered}`"
        );
        Ok(())
    }

    #[test]
    fn rusttype_surface_associated_function_uses_incan_string_conversion() -> Result<(), String> {
        let registry = FunctionRegistry::new();
        let mut emitter = IrEmitter::new(&registry);
        emitter.rusttype_alias_names.insert("Name".to_string());
        let expr = TypedExpr::new(
            IrExprKind::MethodCall {
                receiver: Box::new(TypedExpr::new(
                    IrExprKind::Var {
                        name: "Name".to_string(),
                        access: VarAccess::Read,
                        ref_kind: VarRefKind::TypeName,
                    },
                    IrType::Struct("Name".to_string()),
                )),
                method: "parse".to_string(),
                dispatch: None,
                type_args: Vec::new(),
                args: vec![IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(IrExprKind::String("alice@example.com".to_string()), IrType::String),
                }],
                callable_signature: None,
                arg_policy: MethodCallArgPolicy::Default,
            },
            IrType::Struct("Name".to_string()),
        );

        let emitted = emitter
            .emit_expr(&expr)
            .map_err(|err| format!("expected successful expression emission, got {err:?}"))?;
        let rendered = emitted.to_string();
        assert!(
            rendered.contains("Name :: parse (\"alice@example.com\" . to_string ())"),
            "expected rusttype surface associated function to use Incan string conversion, got `{rendered}`"
        );
        assert!(
            !rendered.contains(". into ()"),
            "rusttype surface associated function must not use external-Rust `.into()` conversion, got `{rendered}`"
        );
        Ok(())
    }

    #[test]
    fn qualified_rusttype_receiver_method_uses_rust_signature_borrowing() -> Result<(), String> {
        let registry = FunctionRegistry::new();
        let mut emitter = IrEmitter::new(&registry);
        emitter.rusttype_alias_names.insert("_RawRegex".to_string());
        emitter.struct_field_types.insert(
            ("Regex".to_string(), "raw".to_string()),
            IrType::Struct("crate::__incan_std::regex::_RawRegex".to_string()),
        );
        let expr = TypedExpr::new(
            IrExprKind::MethodCall {
                receiver: Box::new(TypedExpr::new(
                    IrExprKind::Field {
                        object: Box::new(TypedExpr::new(
                            IrExprKind::Var {
                                name: "self".to_string(),
                                access: VarAccess::Read,
                                ref_kind: VarRefKind::Value,
                            },
                            IrType::Struct("Regex".to_string()),
                        )),
                        field: "raw".to_string(),
                    },
                    IrType::Unknown,
                )),
                method: "find_iter".to_string(),
                dispatch: None,
                type_args: Vec::new(),
                args: vec![IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(
                        IrExprKind::Var {
                            name: "text".to_string(),
                            access: VarAccess::Read,
                            ref_kind: VarRefKind::Value,
                        },
                        IrType::String,
                    ),
                }],
                callable_signature: Some(FunctionSignature {
                    params: vec![FunctionParam {
                        name: "text".to_string(),
                        ty: IrType::Ref(Box::new(IrType::String)),
                        mutability: Mutability::Immutable,
                        is_self: false,
                        kind: crate::frontend::ast::ParamKind::Normal,
                        default: None,
                    }],
                    return_type: IrType::Struct("_RawMatchIterator".to_string()),
                }),
                arg_policy: MethodCallArgPolicy::Default,
            },
            IrType::Struct("_RawMatchIterator".to_string()),
        );

        let emitted = emitter
            .emit_expr(&expr)
            .map_err(|err| format!("expected successful expression emission, got {err:?}"))?;
        let rendered = emitted.to_string();
        assert!(
            rendered.contains("self . raw . find_iter"),
            "expected regular method-call emission on qualified rusttype receiver, got `{rendered}`"
        );
        assert!(
            rendered.contains("find_iter (& text)") || rendered.contains("find_iter (&text)"),
            "metadata-resolved rusttype receiver methods should borrow owned strings for Rust &str params, got `{rendered}`"
        );
        assert!(
            !rendered.contains("to_string"),
            "metadata-resolved rusttype receiver methods should not clone strings before borrowing, got `{rendered}`"
        );
        Ok(())
    }

    #[test]
    fn known_iterator_adapter_methods_emit_incan_stdlib_models() -> Result<(), String> {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);

        let render = |kind: IteratorMethodKind, args: Vec<IrCallArg>| -> Result<String, String> {
            let expr = TypedExpr::new(
                IrExprKind::KnownMethodCall {
                    receiver: Box::new(iterator_receiver()),
                    kind: MethodKind::Iterator(kind),
                    args,
                },
                IrType::Unknown,
            );
            emitter
                .emit_expr(&expr)
                .map(|tokens| tokens.to_string())
                .map_err(|err| format!("expected successful expression emission, got {err:?}"))
        };

        let callback = || {
            vec![IrCallArg {
                name: None,
                kind: IrCallArgKind::Positional,
                expr: function_var("transform"),
            }]
        };
        let count = |value| {
            vec![IrCallArg {
                name: None,
                kind: IrCallArgKind::Positional,
                expr: TypedExpr::new(IrExprKind::Int(value), IrType::Int),
            }]
        };
        let other = || {
            vec![IrCallArg {
                name: None,
                kind: IrCallArgKind::Positional,
                expr: TypedExpr::new(
                    IrExprKind::Var {
                        name: "others".to_string(),
                        access: VarAccess::Read,
                        ref_kind: VarRefKind::Value,
                    },
                    IrType::NamedGeneric(core_traits::as_str(TraitId::Iterator).to_string(), vec![IrType::Int]),
                ),
            }]
        };

        let map_rendered = render(IteratorMethodKind::Map, callback())?;
        assert!(
            map_rendered.contains("collection :: MapIterator") && map_rendered.contains("f : transform"),
            "unexpected map emission: {map_rendered}"
        );

        let filter_rendered = render(IteratorMethodKind::Filter, callback())?;
        assert!(
            filter_rendered.contains("collection :: FilterIterator") && filter_rendered.contains("f : transform"),
            "unexpected filter emission: {filter_rendered}"
        );

        let enumerate_rendered = render(IteratorMethodKind::Enumerate, Vec::new())?;
        assert!(
            enumerate_rendered.contains("collection :: EnumerateIterator")
                && enumerate_rendered.contains("index : 0i64"),
            "unexpected enumerate emission: {enumerate_rendered}"
        );

        let zip_rendered = render(IteratorMethodKind::Zip, other())?;
        assert!(
            zip_rendered.contains("collection :: ZipIterator") && zip_rendered.contains("right : (others)"),
            "unexpected zip emission: {zip_rendered}"
        );

        let take_rendered = render(IteratorMethodKind::Take, count(3))?;
        assert!(
            take_rendered.contains("collection :: TakeIterator") && take_rendered.contains("remaining : 3"),
            "unexpected take emission: {take_rendered}"
        );

        let skip_rendered = render(IteratorMethodKind::Skip, count(-2))?;
        assert!(
            skip_rendered.contains("collection :: SkipIterator") && skip_rendered.contains("remaining : - 2"),
            "unexpected skip emission: {skip_rendered}"
        );

        let take_while_rendered = render(IteratorMethodKind::TakeWhile, callback())?;
        assert!(
            take_while_rendered.contains("collection :: TakeWhileIterator")
                && take_while_rendered.contains("f : transform"),
            "unexpected take_while emission: {take_while_rendered}"
        );

        let skip_while_rendered = render(IteratorMethodKind::SkipWhile, callback())?;
        assert!(
            skip_while_rendered.contains("collection :: SkipWhileIterator")
                && skip_while_rendered.contains("f : transform"),
            "unexpected skip_while emission: {skip_while_rendered}"
        );

        let chain_rendered = render(IteratorMethodKind::Chain, other())?;
        assert!(
            chain_rendered.contains("collection :: ChainIterator") && chain_rendered.contains("second : (others)"),
            "unexpected chain emission: {chain_rendered}"
        );

        let flat_map_rendered = render(IteratorMethodKind::FlatMap, callback())?;
        assert!(
            flat_map_rendered.contains("collection :: FlatMapIterator")
                && flat_map_rendered.contains("current : Vec :: new ()"),
            "unexpected flat_map emission: {flat_map_rendered}"
        );

        let batch_rendered = render(IteratorMethodKind::Batch, count(2))?;
        assert!(
            batch_rendered.contains("collection :: BatchIterator") && batch_rendered.contains("size : 2"),
            "unexpected batch emission: {batch_rendered}"
        );

        Ok(())
    }

    #[test]
    fn source_trait_take_dispatch_preserves_incan_int_argument() -> Result<(), String> {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);
        let expr = TypedExpr::new(
            IrExprKind::MethodCall {
                receiver: Box::new(TypedExpr::new(
                    IrExprKind::Var {
                        name: "stream".to_string(),
                        access: VarAccess::Read,
                        ref_kind: VarRefKind::Value,
                    },
                    IrType::Struct("Stream".to_string()),
                )),
                method: "take".to_string(),
                dispatch: Some(IrMethodDispatch::Trait {
                    trait_path: "crate::__incan_std::derives::collection::FallibleIterator".to_string(),
                    type_args: vec![IrType::Int, IrType::String],
                    receiver_is_mutable: false,
                }),
                type_args: Vec::new(),
                args: vec![IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(IrExprKind::Int(3), IrType::Int),
                }],
                callable_signature: None,
                arg_policy: MethodCallArgPolicy::Default,
            },
            IrType::Unknown,
        );

        let rendered = emitter
            .emit_expr(&expr)
            .map_err(|error| format!("expected successful source trait emission, got {error:?}"))?
            .to_string();
        assert!(
            rendered.contains("take (& stream , 3)"),
            "unexpected source trait take emission: {rendered}"
        );
        assert!(
            !rendered.contains("u64 :: try_from"),
            "source trait method arguments must follow their declared Incan signature: {rendered}"
        );
        Ok(())
    }

    #[test]
    fn known_iterator_terminal_methods_emit_incan_next_loops_except_source_owned_sum() -> Result<(), String> {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);

        let render = |kind: IteratorMethodKind, args: Vec<IrCallArg>| -> Result<String, String> {
            let expr = TypedExpr::new(
                IrExprKind::KnownMethodCall {
                    receiver: Box::new(iterator_receiver()),
                    kind: MethodKind::Iterator(kind),
                    args,
                },
                IrType::Unknown,
            );
            emitter
                .emit_expr(&expr)
                .map(|tokens| tokens.to_string())
                .map_err(|err| format!("expected successful expression emission, got {err:?}"))
        };

        let callback = || {
            vec![IrCallArg {
                name: None,
                kind: IrCallArgKind::Positional,
                expr: function_var("predicate"),
            }]
        };

        let collect_rendered = render(IteratorMethodKind::Collect, Vec::new())?;
        assert!(
            collect_rendered.contains("collection :: Iterator :: __next__")
                && collect_rendered.contains("__incan_items . push"),
            "unexpected collect emission: {collect_rendered}"
        );

        let count_rendered = render(IteratorMethodKind::Count, Vec::new())?;
        assert!(
            count_rendered.contains("collection :: Iterator :: __next__")
                && count_rendered.contains("__incan_total += 1"),
            "unexpected count emission: {count_rendered}"
        );

        let reduce_rendered = render(
            IteratorMethodKind::Reduce,
            vec![
                IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(IrExprKind::Int(0), IrType::Int),
                },
                IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: function_var("predicate"),
                },
            ],
        )?;
        assert!(
            reduce_rendered.contains("collection :: Iterator :: __next__")
                && reduce_rendered.contains("__incan_acc = (predicate) (__incan_acc , __incan_item)"),
            "unexpected reduce emission: {reduce_rendered}"
        );

        let fold_rendered = render(
            IteratorMethodKind::Fold,
            vec![
                IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: TypedExpr::new(IrExprKind::Int(0), IrType::Int),
                },
                IrCallArg {
                    name: None,
                    kind: IrCallArgKind::Positional,
                    expr: function_var("predicate"),
                },
            ],
        )?;
        assert!(
            fold_rendered.contains("collection :: Iterator :: __next__")
                && fold_rendered.contains("__incan_acc = (predicate) (__incan_acc , __incan_item)"),
            "unexpected fold emission: {fold_rendered}"
        );

        let any_rendered = render(IteratorMethodKind::Any, callback())?;
        assert!(
            any_rendered.contains("collection :: Iterator :: __next__") && any_rendered.contains("(predicate)"),
            "unexpected any emission: {any_rendered}"
        );

        let all_rendered = render(IteratorMethodKind::All, callback())?;
        assert!(
            all_rendered.contains("collection :: Iterator :: __next__") && all_rendered.contains("(predicate)"),
            "unexpected all emission: {all_rendered}"
        );

        let find_rendered = render(IteratorMethodKind::Find, callback())?;
        assert!(
            find_rendered.contains("collection :: Iterator :: __next__") && find_rendered.contains("(predicate)"),
            "unexpected find emission: {find_rendered}"
        );

        let for_each_rendered = render(IteratorMethodKind::ForEach, callback())?;
        assert!(
            for_each_rendered.contains("collection :: Iterator :: __next__")
                && for_each_rendered.contains("(predicate) (__incan_item)"),
            "unexpected for_each emission: {for_each_rendered}"
        );

        let sum_rendered = render(IteratorMethodKind::Sum, Vec::new())?;
        assert!(
            sum_rendered.contains("collection :: Iterator :: sum (& mut __incan_iterator_sum)")
                && !sum_rendered.contains("__incan_sum += __incan_item"),
            "unexpected sum emission: {sum_rendered}"
        );

        Ok(())
    }

    fn iterator_receiver() -> TypedExpr {
        TypedExpr::new(
            IrExprKind::Var {
                name: "items".to_string(),
                access: VarAccess::Read,
                ref_kind: VarRefKind::Value,
            },
            IrType::NamedGeneric(core_traits::as_str(TraitId::Iterator).to_string(), vec![IrType::Int]),
        )
    }

    fn function_var(name: &str) -> TypedExpr {
        TypedExpr::new(
            IrExprKind::Var {
                name: name.to_string(),
                access: VarAccess::Read,
                ref_kind: VarRefKind::Value,
            },
            IrType::Unknown,
        )
    }
}
