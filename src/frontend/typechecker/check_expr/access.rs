Warning: truncated output (original token count: 52710)
Total output lines: 4787

//! Check indexing, slicing, field access, and method calls.
//!
//! These helpers validate access patterns like `xs[i]`, `xs[a:b]`, `obj.field`, and `obj.method(...)`, emitting
//! diagnostics for missing fields/methods and incompatible uses.

use std::collections::HashMap;

use crate::frontend::ast::*;
use crate::frontend::diagnostics::{CompileError, errors};
use crate::frontend::resolved_type_subst::{substitute_resolved_type, type_param_subst_map};
use crate::frontend::symbols::*;
use crate::frontend::typechecker::helpers::{
    collection_name, collection_type_id, generator_ty, is_frozen_bytes, is_frozen_str, is_intlike_for_index, list_ty,
    option_ty, render_resolved_type_as_rust_arg, string_method_return,
};
use crate::frontend::typechecker::type_info::{RustMethodTraitImportUse, RustTraitImportInfo};
use crate::frontend::typechecker::{IdentKind, canonical_public_library_type_name};
use incan_core::interop::{
    RustCollectionFamily, RustFieldInfo, RustFunctionSig, RustItemKind, RustItemMetadata,
    metadata_free_method_signature,
};
use incan_core::lang::magic_methods;
use incan_core::lang::surface::collection_helpers::{self, BuiltinCollectionHelperId};
use incan_core::lang::surface::result_methods::ResultMethodId;
use incan_core::lang::surface::types as surface_types;
use incan_core::lang::surface::types::{SEMAPHORE_ACQUIRE_ERROR_TYPE_NAME, SEMAPHORE_PERMIT_TYPE_NAME, SurfaceTypeId};
use incan_core::lang::surface::{
    dict_methods, float_methods, frozen_bytes_methods, frozen_dict_methods, frozen_list_methods, frozen_set_methods,
    iterator_methods, list_methods, result_methods, set_methods,
};
use incan_core::lang::traits::{self as core_traits, TraitId};
use incan_core::lang::types::collections::CollectionTypeId;
use incan_core::lang::types::numerics::NumericFamily;
use incan_core::lang::{conventions, stdlib};
use incan_core::lang::{enum_helpers, surface::option_methods};
use quote::ToTokens;
use syn::{GenericArgument, PathArguments, ReturnType, Type as SynType, TypeParamBound};

use super::calls::PublicModuleConstructorContext;

use super::TypeChecker;

#[derive(Debug, Clone)]
struct MethodCandidate {
    info: MethodInfo,
    dispatch: Option<crate::frontend::typechecker::ResolvedMethodDispatch>,
}

struct SourceMethodPrepass<'a> {
    method: &'a str,
    type_args: &'a [Spanned<Type>],
    args: &'a [CallArg],
    span: Span,
    receiver_ty: &'a ResolvedType,
    expected_return_ty: Option<&'a ResolvedType>,
}

struct ValueEnumGeneratedCall<'a> {
    enum_name: &'a str,
    value_enum: &'a ValueEnumInfo,
    method: &'a str,
    base_is_type_name: bool,
    type_args: &'a [Spanned<Type>],
    args: &'a [CallArg],
    arg_types: &'a [ResolvedType],
    span: Span,
}

struct RustTraitMethodCall<'a> {
    rust_path: &'a str,
    method: &'a str,
    sig: &'a RustFunctionSig,
    type_args: &'a [Spanned<Type>],
    args: &'a [CallArg],
    arg_types: &'a [ResolvedType],
    preserves_lookup_arg_shape: bool,
    span: Span,
}

struct RustPathMethodCall<'a> {
    rust_path: &'a str,
    method: &'a str,
    receiver_metadata: Option<&'a RustItemMetadata>,
    type_args: &'a [Spanned<Type>],
    args: &'a [CallArg],
    arg_types: &'a [ResolvedType],
    receiver_span: Span,
    expected_return_ty: Option<&'a ResolvedType>,
    span: Span,
}

#[derive(Debug, Clone)]
struct RustCallableAliasParam {
    rust_display: String,
    resolved_ty: ResolvedType,
}

#[derive(Debug, Clone)]
struct RustCallableAliasSignature {
    params: Vec<RustCallableAliasParam>,
    return_ty: ResolvedType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumericResizeMethodPolicy {
    Lossless,
    Try,
    Wrapping,
    Saturating,
}

/// Diagnostic label for a Rust path receiver in type errors (`rust::{path}`).
fn rust_receiver_display(path: &str) -> String {
    format!("rust::{path}")
}

impl TypeChecker {
    /// Resolve a source-facing Rust field spelling to the metadata field it names.
    ///
    /// Rust raw identifier fields should be written with the Rust source name at Incan field-use sites. For example, a
    /// Rust field declared as `r#type` is accessed as `obj.type` and constructed with `TypeName(type=...)`; emission
    /// rawifies the keyword identifier back to `r#type`. An ordinary Rust field declared as `type_` remains available
    /// only as `obj.type_`.
    pub(in crate::frontend::typechecker::check_expr) fn rust_field_for_source_name<'a>(
        fields: &'a [RustFieldInfo],
        source_name: &str,
    ) -> Option<&'a RustFieldInfo> {
        fields.iter().find(|field| field.name == source_name)
    }

    /// Resolve a Rust field type from its display string against the owning Rust type.
    ///
    /// Rust field metadata carries both a structural shape and the source display. Field access should use the display
    /// because it preserves exact numeric widths and owner-relative paths that matter when a Rust field is copied back
    /// into another Rust boundary. The structural shape remains useful for broad semantic classification and older
    /// metadata entries whose display cannot be resolved.
    fn resolved_rust_field_type(&self, owner_path: &str, field: &RustFieldInfo) -> ResolvedType {
        let display = self.rust_display_for_owner_path(field.type_display.as_str(), owner_path);
        let resolved = self.resolved_type_from_rust_display(display.as_str());
        if matches!(resolved, ResolvedType::Unknown) {
            self.resolved_type_from_rust_shape(&field.type_shape)
        } else {
            resolved
        }
    }

    /// Return the target display for a Rust type alias when the expected destination type names one.
    fn rust_callable_alias_target_display(&self, expected_ty: &ResolvedType) -> Option<String> {
        let ResolvedType::RustPath(path) = expected_ty else {
            return None;
        };
        self.rust_callable_alias_target_display_for_path(path, &mut std::collections::HashSet::new())
    }

    /// Follow Rust type-alias chains until they expose a callable trait object target.
    ///
    /// This is intentionally metadata-driven rather than crate-specific. DataFusion's
    /// `ScalarFunctionImplementation -> Arc<dyn Fn(...)>` chain is one motivating surface, but the compiler must not
    /// special-case DataFusion or require regression tests to compile that heavyweight crate.
    ///
    /// Use blocking metadata reads here so contextual closure typing does not depend on whether a transitive alias was
    /// already imported elsewhere or happened to be warmed by an earlier arm in the same expression.
    fn rust_callable_alias_target_display_for_path(
        &self,
        path: &str,
        seen: &mut std::collections::HashSet<String>,
    ) -> Option<String> {
        let canonical_path = Self::normalize_rust_namespace_path(path).to_string();
        if !seen.insert(canonical_path.clone()) {
            return None;
        }
        if let Some(metadata) = self.rust_item_metadata_for_path_blocking(path)
            && let RustItemKind::Type(type_info) = &metadata.kind
            && let Some(target) = type_info.alias_target.as_ref()
        {
            let display = self.rust_display_for_owner_path(target, canonical_path.as_str());
            if Self::rust_display_has_callable_fn_bound(display.as_str()) {
                return Some(display);
            }
            let (target_base, _) = self.rust_path_base_and_args(display.as_str());
            if target_base != canonical_path
                && let Some(expanded) = self.rust_callable_alias_target_display_for_path(target_base.as_str(), seen)
            {
                return Some(expanded);
            }
            return None;
        }
        Some(self.rust_display_for_owner_path(path, path))
            .filter(|display| Self::rust_display_has_callable_fn_bound(display.as_str()))
    }

    /// Parse a Rust callable alias target such as `Arc<dyn Fn(&[T]) -> Result<U, E> + Send + Sync>`.
    fn rust_callable_alias_signature(&self, expected_ty: &ResolvedType) -> Option<RustCallableAliasSignature> {
        let target_display = self.rust_callable_alias_target_display(expected_ty)?;
        let ty = syn::parse_str::<SynType>(&target_display).ok()?;
        let fn_bound = Self::rust_callable_fn_bound(&ty)?;

        let params = fn_bound
            .inputs
            .iter()
            .map(|input| {
                let rust_display = Self::compact_rust_display(&input.to_token_stream().to_string());
                RustCallableAliasParam {
                    resolved_ty: self.resolved_param_type_from_rust_display(&rust_display),
                    rust_display,
                }
            })
            .collect::<Vec<_>>();
        let return_ty = match &fn_bound.output {
            ReturnType::Default => ResolvedType::Unit,
            ReturnType::Type(_, ty) => {
                let rust_display = Self::compact_rust_display(&ty.to_token_stream().to_string());
                self.resolved_type_from_rust_display(&rust_display)
            }
        };
        Some(RustCallableAliasSignature { params, return_ty })
    }

    /// Return whether a Rust display type contains a callable trait-object target.
    fn rust_display_has_callable_fn_bound(display: &str) -> bool {
        let Ok(ty) = syn::parse_str::<SynType>(display) else {
            return false;
        };
        Self::rust_callable_fn_bound(&ty).is_some()
    }

    /// Return the `Fn(...) -> ...` bound carried by a Rust callable trait-object target.
    fn rust_callable_fn_bound(ty: &SynType) -> Option<&syn::ParenthesizedGenericArguments> {
        let trait_object = Self::rust_callable_trait_object(ty)?;
        trait_object.bounds.iter().find_map(|bound| {
            let TypeParamBound::Trait(trait_bound) = bound else {
                return None;
            };
            let segment = trait_bound.path.segments.last()?;
            if !matches!(segment.ident.to_string().as_str(), "Fn" | "FnMut" | "FnOnce") {
                return None;
            }
            let PathArguments::Parenthesized(args) = &segment.arguments else {
                return None;
            };
            Some(args)
        })
    }

    /// Find the Rust trait-object type wrapped by a callable alias target.
    fn rust_callable_trait_object(ty: &SynType) -> Option<&syn::TypeTraitObject> {
        match ty {
            SynType::TraitObject(trait_object) => Some(trait_object),
            SynType::Group(group) => Self::rust_callable_trait_object(&group.elem),
            SynType::Paren(paren) => Self::rust_callable_trait_object(&paren.elem),
            SynType::Path(path) => {
                let segment = path.path.segments.last()?;
                let PathArguments::AngleBracketed(args) = &segment.arguments else {
                    return None;
                };
                args.args.iter().find_map(|arg| match arg {
                    GenericArgument::Type(inner) => Self::rust_callable_trait_object(inner),
                    _ => None,
                })
            }
            _ => None,
        }
    }

    /// Check a closure expression against a Rust callable alias.
    fn check_closure_with_rust_callable_alias(
        &mut self,
        expr: &Spanned<Expr>,
        signature: &RustCallableAliasSignature,
    ) -> ResolvedType {
        let Expr::Closure(params, body) = &expr.node else {
            return self.check_expr(expr);
        };
        if params.len() != signature.params.len() {
            self.errors.push(errors::builtin_arity(
                "closure",
                signature.params.len(),
                params.len(),
                expr.span,
            ));
            return ResolvedType::Unknown;
        }

        self.symbols.enter_scope(ScopeKind::Function);

        let prev_in_async_body = self.in_async_body;
        self.in_async_body = false;
        let prev_return_error_type = self.current_return_error_type.take();

        let param_types = params
            .iter()
            .zip(signature.params.iter())
            .map(|(param, expected)| {
                let ty = expected.resolved_ty.clone();
                self.symbols.define(Symbol {
                    name: param.node.name.clone(),
                    kind: SymbolKind::Variable(VariableInfo {
                        ty: ty.clone(),
                        is_mutable: false,
                        is_used: false,
                    }),
                    span: param.span,
                    scope: 0,
                });
                CallableParam::named(param.node.name.clone(), ty, param.node.kind)
            })
            .collect::<Vec<_>>();

        let return_ty = self.check_expr_with_expected(body, Some(&signature.return_ty));
        if !matches!(return_ty, ResolvedType::Unknown) && !self.types_compatible(&return_ty, &signature.return_ty) {
            self.errors.push(errors::type_mismatch(
                &signature.return_ty.to_string(),
                &return_ty.to_string(),
                body.span,
            ));
        }

        self.current_return_error_type = prev_return_error_type;
        self.in_async_body = prev_in_async_body;
        self.symbols.exit_scope();

        self.type_info.rust.closure_param_type_displays.insert(
            (expr.span.start, expr.span.end),
            signature
                .params
                .iter()
                .map(|param| param.rust_display.clone())
                .collect(),
        );

        let closure_ty = ResolvedType::Function(param_types, Box::new(signature.return_ty.clone()));
        self.record_expr_type(expr.span, closure_ty.clone());
        closure_ty
    }

    /// Check a method argument against a Rust callable alias.
    fn check_method_arg_with_rust_callable_alias(
        &mut self,
        arg: &CallArg,
        signature: Option<&RustCallableAliasSignature>,
    ) -> ResolvedType {
        match arg {
            CallArg::Positional(expr)
            | CallArg::Named(_, expr)
            | CallArg::PositionalUnpack(expr)
            | CallArg::KeywordUnpack(expr) => {
                if let Some(signature) = signature
                    && matches!(expr.node, Expr::Closure(_, _))
                {
                    return self.check_closure_with_rust_callable_alias(expr, signature);
                }
                self.check_expr(expr)
            }
        }
    }

    /// Return whether `method` names an RFC 070 `Result[T, E]` combinator.
    fn result_combinator_name(method: &str) -> bool {
        matches!(
            result_methods::from_str(method),
            Some(
                result_methods::ResultMethodId::Map
                    | result_methods::ResultMethodId::MapErr
                    | result_methods::ResultMethodId::AndThen
                    | result_methods::ResultMethodId::OrElse
                    | result_methods::ResultMethodId::Inspect
                    | result_methods::ResultMethodId::InspectErr
            )
        )
    }

    /// Resolve a callable function or callable object to its parameter and return types.
    fn callable_signature_for_value_type(
        &mut self,
        ty: &ResolvedType,
        span: Span,
    ) -> Option<(Vec<CallableParam>, ResolvedType)> {
        match ty {
            ResolvedType::Function(params, ret) => Some((params.clone(), ret.as_ref().clone())),
            ResolvedType::Generic(name, _) | ResolvedType::Named(name) => {
                let type_info = self.lookup_semantic_type_info(name).cloned()?;
                let methods = match type_info {
                    TypeInfo::Model(model) => model.methods,
                    TypeInfo::Class(class) => class.methods,
                    TypeInfo::Enum(en) => en.methods,
                    TypeInfo::Newtype(newtype) => newtype.methods,
                    _ => return None,
                };
                let Some(call) = methods.get("__call__") else {
                    self.errors
                        .push(errors::missing_method(&ty.to_string(), "__call__", span));
                    return None;
                };
                Some(self.method_types_substituting_call_site_self(call, ty))
            }
            ResolvedType::Unknown => Some((Vec::new(), ResolvedType::Unknown)),
            _ => {
                self.errors
                    .push(errors::missing_method(&ty.to_string(), "__call__", span));
                None
            }
        }
    }

    /// Validate the callback passed to one `Result[T, E]` combinator and return its output type.
    fn validate_result_combinator_callback(
        &mut self,
        _method: &str,
        callback_ty: &ResolvedType,
        input_ty: &ResolvedType,
        expected_ret: Option<&ResolvedType>,
        span: Span,
    ) -> ResolvedType {
        let Some((params, ret)) = self.callable_signature_for_value_type(callback_ty, span) else {
            return ResolvedType::Unknown;
        };
        if params.len() != 1 {
            self.errors.push(errors::type_mismatch(
                "one-parameter callable",
                &format!("{}-parameter callable", params.len()),
                span,
            ));
            return ResolvedType::Unknown;
        }
        if let Some(param) = params.first()
            && !self.types_compatible(input_ty, &param.ty)
        {
            self.errors.push(errors::type_mismatch(
                &param.ty.to_string(),
                &input_ty.to_string(),
                span,
            ));
        }
        if let Some(expected) = expected_ret
            && !self.types_compatible(&ret, expected)
        {
            self.errors
                .push(errors::type_mismatch(&expected.to_string(), &ret.to_string(), span));
        }
        ret
    }

    /// Typecheck one RFC 070 `Result[T, E]` combinator method call.
    fn check_result_combinator_method(
        &mut self,
        ok_ty: ResolvedType,
        err_ty: ResolvedType,
        method: &str,
        args: &[CallArg],
        arg_types: &[ResolvedType],
        span: Span,
    ) -> ResolvedType {
        if args.len() != 1 {
            self.errors.push(errors::type_mismatch(
                "one callable argument",
                &format!("{} argument(s)", args.len()),
                span,
            ));
            return ResolvedType::Unknown;
        }
        let Some(callback_ty) = arg_types.first() else {
            return ResolvedType::Unknown;
        };
        let Some(method_id) = result_methods::from_str(method) else {
            return ResolvedType::Unknown;
        };
        match method_id {
            result_methods::ResultMethodId::Map => {
                let ret = self.validate_result_combinator_callback(method, callback_ty, &ok_ty, None, span);
                ResolvedType::Generic("Result".to_string(), vec![ret, err_ty])
            }
            result_methods::ResultMethodId::MapErr => {
                let ret = self.validate_result_combinator_callback(method, callback_ty, &err_ty, None, span);
                ResolvedType::Generic("Result".to_string(), vec![ok_ty, ret])
            }
            result_methods::ResultMethodId::AndThen => {
                let expected = ResolvedType::Generic("Result".to_string(), vec![ResolvedType::Unknown, err_ty.clone()]);
                let ret = self.validate_result_combinator_callback(method, callback_ty, &ok_ty, Some(&expected), span);
                let ResolvedType::Generic(name, args) = ret else {
                    return ResolvedType::Generic("Result".to_string(), vec![ResolvedType::Unknown, err_ty]);
                };
                if collection_type_id(name.as_str()) == Some(CollectionTypeId::Result) && args.len() == 2 {
                    return ResolvedType::Generic(name, args);
                }
                ResolvedType::Generic("Result".to_string(), vec![ResolvedType::Unknown, err_ty])
            }
            result_methods::ResultMethodId::OrElse => {
                let expected = ResolvedType::Generic("Result".to_string(), vec![ok_ty.clone(), ResolvedType::Unknown]);
                let ret = self.validate_result_combinator_callback(method, callback_ty, &err_ty, Some(&expected), span);
                let ResolvedType::Generic(name, args) = ret else {
                    return ResolvedType::Generic("Result".to_string(), vec![ok_ty, ResolvedType::Unknown]);
                };
                if collection_type_id(name.as_str()) == Some(CollectionTypeId::Result) && args.len() == 2 {
                    return ResolvedType::Generic(name, args);
                }
                ResolvedType::Generic("Result".to_string(), vec![ok_ty, ResolvedType::Unknown])
            }
            result_methods::ResultMethodId::Inspect => {
                self.validate_result_combinator_callback(method, callback_ty, &ok_ty, Some(&ResolvedType::Unit), span);
                ResolvedType::Generic("Result".to_string(), vec![ok_ty, err_ty])
            }
            result_methods::ResultMethodId::InspectErr => {
                self.validate_result_combinator_callback(method, callback_ty, &err_ty, Some(&ResolvedType::Unit), span);
                ResolvedType::Generic("Result".to_string(), vec![ok_ty, err_ty])
            }
            result_methods::ResultMethodId::Unwrap | result_methods::ResultMethodId::UnwrapOr => ResolvedType::Unknown,
        }
    }

    /// Bind `list.repeat(...)` arguments to the helper's fixed `value` and `count` parameters.
    ///
    /// This mirrors ordinary fixed-parameter call binding closely enough for named arguments while keeping unpacking
    /// rejected: `list.repeat` has no rest parameters and lowering expects the two canonical arguments explicitly.
    fn bind_builtin_list_repeat_args<'a>(
        &mut self,
        args: &'a [CallArg],
        span: Span,
    ) -> (Option<&'a Spanned<Expr>>, Option<&'a Spanned<Expr>>, bool) {
        let helper = BuiltinCollectionHelperId::ListRepeat;
        let callee = collection_helpers::full_name(helper);
        let mut value: Option<&Spanned<Expr>> = None;
        let mut count: Option<&Spanned<Expr>> = None;
        let mut valid = true;
        let mut positional_index = 0usize;

        for arg in args {
            match arg {
                CallArg::Positional(expr) => {
                    let target = match positional_index {
                        0 => Some((&mut value, "value")),
                        1 => Some((&mut count, "count")),
                        _ => None,
                    };
                    positional_index += 1;
                    if let Some((slot, name)) = target {
                        if let Some(first_expr) = *slot {
                            self.errors.push(errors::duplicate_call_argument(
                                callee,
                                name,
                                first_expr.span,
                                expr.span,
                            ));
                            self.check_expr(expr);
                            valid = false;
                        } else {
                            *slot = Some(expr);
                        }
                    } else {
                        self.check_expr(expr);
                        valid = false;
                    }
                }
                CallArg::Named(name, expr) => {
                    let slot = match name.as_str() {
                        "value" => Some(&mut value),
                        "count" => Some(&mut count),
                        _ => None,
                    };
                    if let Some(slot) = slot {
                        if let Some(first_expr) = *slot {
                            self.errors.push(errors::duplicate_call_argument(
                                callee,
                                name,
                                first_expr.span,
                                expr.span,
                            ));
                            self.check_expr(expr);
                            valid = false;
                        } else {
                            *slot = Some(expr);
                        }
                    } else {
                        self.errors
                            .push(errors::unknown_keyword_argument(callee, name, expr.span));
                        self.check_expr(expr);
                        valid = false;
                    }
                }
                CallArg::PositionalUnpack(expr) => {
                    self.errors
                        .push(errors::call_unpack_without_rest(callee, "*", expr.span));
                    self.check_expr(expr);
                    valid = false;
                }
                CallArg::KeywordUnpack(expr) => {
                    self.errors
                        .push(errors::call_unpack_without_rest(callee, "**", expr.span));
                    self.check_expr(expr);
                    valid = false;
                }
            }
        }

        if args.len() != 2 {
            self.errors.push(errors::builtin_arity(callee, 2, args.len(), span));
            valid = false;
        }
        if value.is_none() {
            self.errors
                .push(errors::missing_required_argument(callee, "value", span));
            valid = false;
        }
        if count.is_none() {
            self.errors
                .push(errors::missing_required_argument(callee, "count", span));
            valid = false;
        }

        (value, count, valid)
    }

    /// Type-check the built-in `list.repeat(value, count)` helper.
    ///
    /// The receiver is the built-in `list` collection surface, not a runtime list value, so this has to run before
    /// ordinary receiver expression checking would report `list` as an unknown value symbol.
    fn check_builtin_list_repeat_call(&mut self, args: &[CallArg], span: Span) -> ResolvedType {
        let (value_arg, count_arg, valid_args) = self.bind_builtin_list_repeat_args(args, span);

        let value_ty = value_arg
            .map(|expr| self.check_expr(expr))
            .unwrap_or(ResolvedType::Unknown);

        if let Some(count_arg) = count_arg {
            let count_ty = self.check_expr(count_arg);
            if !self.types_compatible(&count_ty, &ResolvedType::Int) {
                self.errors
                    .push(errors::type_mismatch("int", &count_ty.to_string(), span));
            }
        }

        if !matches!(value_ty, ResolvedType::Unknown) && !self.is_copy_type(&value_ty) && !self.is_clone_type(&value_ty)
        {
            self.errors
                .push(errors::list_repeat_requires_clone(&value_ty.to_string(), span));
        }

        if !valid_args {
            return ResolvedType::Unknown;
        }

        list_ty(value_ty)
    }

    /// Return whether `base` resolves to the built-in `list` collection type surface.
    fn is_builtin_list_surface_receiver(&self, base: &Spanned<Expr>) -> bool {
        let Expr::Ident(name) = &base.node else {
            return false;
        };
        name == collection_helpers::receiver(BuiltinCollectionHelperId::ListRepeat)
            && collection_type_id(name.as_str()) == Some(CollectionTypeId::List)
            && self
                .lookup_symbol(name)
                .is_some_and(|sym| matches!(sym.kind, SymbolKind::Type(TypeInfo::Builtin)))
    }

    /// Return the canonical stdlib iterator trait name from the shared language registry.
    fn iterator_protocol_name() -> &'static str {
        core_traits::as_str(TraitId::Iterator)
    }

    /// Return the canonical RFC 088 iterable protocol trait spelling.
    fn iterable_protocol_name() -> &'static str {
        core_traits::as_str(TraitId::Iterable)
    }

    /// Construct the protocol-facing `Iterator[T]` type used by RFC 088 adapter method typing.
    fn iterator_protocol_ty(elem: ResolvedType) -> ResolvedType {
        ResolvedType::Generic(Self::iterator_protocol_name().to_string(), vec![elem])
    }

    /// Return whether `name` is the canonical RFC 088 iterable protocol trait spelling.
    fn is_iterable_protocol_name(name: &str) -> bool {
        core_traits::from_qualified_str(name) == Some(TraitId::Iterable)
    }

    /// Return whether `name` is the canonical RFC 088 iterator protocol trait spelling.
    fn is_iterator_protocol_name(name: &str) -> bool {
        core_traits::from_qualified_str(name) == Some(TraitId::Iterator)
    }

    /// Return the element type for values that can participate in the RFC 088 iterator protocol surface.
    ///
    /// This intentionally recognizes both explicit trait-typed values (`Iterator[T]` / `Iterable[T]`) and builtin
    /// collection values that have an obvious frontend iterator element type. It is a typechecker-only surface helper;
    /// lowering and emission use the same protocol shape to route known iterator methods through dedicated backend
    /// handling.
    fn iterable_protocol_element_type(&self, ty: &ResolvedType) -> Option<ResolvedType> {
        match ty {
            ResolvedType::Generic(name, args)
                if (Self::is_iterator_protocol_name(name) || Self::is_iterable_protocol_name(name))
                    && args.len() == 1 =>
            {
                args.first().cloned()
            }
            ResolvedType::Generic(name, args)
                if matches!(
                    collection_type_id(name.as_str()),
                    Some(
                        CollectionTypeId::List
                            | CollectionTypeId::Set
                            | CollectionTypeId::FrozenList
                            | CollectionTypeId::FrozenSet
                    )
                ) && args.len() == 1 =>
            {
                args.first().cloned()
            }
            ResolvedType::FrozenList(inner) | ResolvedType::FrozenSet(inner) => Some((**inner).clone()),
            _ => None,
        }
    }

    /// Return the element type for values that are already typed as `Iterator[T]`.
    fn iterator_protocol_element_type(&self, ty: &ResolvedType) -> Option<ResolvedType> {
        match ty {
            ResolvedType::Generic(name, args) if Self::is_iterator_protocol_name(name) && args.len() == 1 => {
                args.first().cloned()
            }
            _ => None,
        }
    }

    /// Validate fixed-arity RFC 088 method calls and report the same arity diagnostic style as other builtin calls.
    fn validate_iterator_method_arity(&mut self, method: &str, expected: usize, found: usize, span: Span) -> bool {
        if found == expected {
            return true;
        }
        self.errors.push(errors::builtin_arity(
            &format!("{}.{method}", Self::iterator_protocol_name()),
            expected,
            found,
            span,
        ));
        false
    }

    /// Build a resolved callable type from parameter and return types for adapter diagnostics.
    fn iterator_callback_ty(params: Vec<ResolvedType>, ret: ResolvedType) -> ResolvedType {
        ResolvedType::Function(
            params.into_iter().map(CallableParam::positional).collect(),
            Box::new(ret),
        )
    }

    /// Reject `.batch(size)` calls when a non-positive literal size is visible to the frontend.
    fn validate_iterator_batch_size_literal(&mut self, args: &[CallArg], span: Span) {
        let Some(CallArg::Positional(expr)) = args.first() else {
            return;
        };
        let Expr::Literal(Literal::Int(value)) = &expr.node else {
            return;
        };
        if value.value > 0 {
            return;
        }
        self.errors.push(CompileError::type_error(
            "Iterator.batch() size must be greater than zero".to_string(),
            span,
        ));
    }

    /// Return whether `method` consumes the receiver under RFC 088 terminal semantics.
    fn is_iterator_terminal_method(method: &str) -> bool {
        iterator_methods::from_str(method).is_some_and(iterator_methods::is_terminal)
    }

    /// Validate `.sum()` item types against the source-owned `Sum[T]` capability surface.
    fn iterator_sum_output_type(&mut self, elem: &ResolvedType, span: Span) -> ResolvedType {
        if self
            .temporary_trait_capability_supports_type(incan_core::lang::trait_capabilities::iterator_sum(), elem)
            .is_some_and(|supported| supported)
        {
            return elem.clone();
        }
        self.errors.push(CompileError::type_error(
            format!(
                "Iterator.sum() requires int, float, or a newtype over a summable type; found {}",
                elem
            ),
            span,
        ));
        ResolvedType::Unknown
    }

    /// Track the narrow same-binding case after a terminal iterator method consumes a direct local binding.
    fn mark_direct_iterator_binding_consumed(&mut self, base: &Spanned<Expr>, method: &str, span: Span) {
        if !Self::is_iterator_terminal_method(method) {
            return;
        }
        let Expr::Ident(name) = &base.node else {
            return;
        };
        self.consumed_iterator_bindings.insert(name.clone(), span);
    }

    /// Validate an adapter callback whose return type is fully specified by the method contract.
    fn validate_iterator_callback_return(
        &mut self,
        method: &str,
        actual: &ResolvedType,
        params: Vec<ResolvedType>,
        ret: ResolvedType,
        span: Span,
    ) {
        if matches!(actual, ResolvedType::Unknown) {
            return;
        }
        let expected = Self::iterator_callback_ty(params, ret);
        if !self.types_compatible(actual, &expected) {
            self.errors
                .push(errors::type_mismatch(&expected.to_string(), &actual.to_string(), span));
        }
        if !matches!(actual, ResolvedType::Function(_, _)) {
            self.errors.push(errors::missing_method(
                &actual.to_string(),
                &format!("__call__ for {method} callback"),
                span,
            ));
        }
    }

    /// Validate a mapping-style callback and return its concrete output type when it is known.
    fn iterator_mapping_callback_return_type(
        &mut self,
        actual: &ResolvedType,
        param_ty: ResolvedType,
        span: Span,
    ) -> ResolvedType {
        let ResolvedType::Function(params, ret) = actual else {
            if !matches!(actual, ResolvedType::Unknown) {
                let expected = Self::iterator_callback_ty(vec![param_ty], ResolvedType::Unknown);
                self.errors
                    .push(errors::type_mismatch(&expected.to_string(), &actual.to_string(), span));
            }
            return ResolvedType::Unknown;
        };
        if params.len() != 1 || !self.types_compatible(&params[0].ty, &param_ty) {
            let expected = Self::iterator_callback_ty(vec![param_ty], (**ret).clone());
            self.errors
                .push(errors::type_mismatch(&expected.to_string(), &actual.to_string(), span));
        }
        (**ret).clone()
    }

    /// Typecheck one RFC 088 iterator/iterable adapter or terminal method.
    ///
    /// The frontend treats these protocol methods as a typed surface even when the receiver is a builtin collection
    /// whose methods are not represented as ordinary user-declared methods. Backend lowering and emission classify the
    /// same method family as known iterator calls.
    fn resolve_iterator_protocol_method_call(
        &mut self,
        base_ty: &ResolvedType,
        method: &str,
        args: &[CallArg],
        arg_types: &[ResolvedType],
        span: Span,
    ) -> Option<ResolvedType> {
        let elem = self.iterable_protocol_element_type(base_ty)?;
        let iterator_elem = self
            .iterator_protocol_element_type(base_ty)
            .unwrap_or_else(|| elem.clone());
        let method_id = iterator_methods::from_str(method)?;
        use iterator_methods::IteratorMethodId as M;

        match method_id {
            M::Iter => {
                self.validate_iterator_method_arity(method, 0, args.len(), span);
                Some(Self::iterator_protocol_ty(elem))
            }
            M::Map => {
                if !self.validate_iterator_method_arity(method, 1, args.len(), span) {
                    return Some(Self::iterator_protocol_ty(ResolvedType::Unknown));
                }
                let mapped = self.iterator_mapping_callback_return_type(
                    arg_types.first().unwrap_or(&ResolvedType::Unknown),
                    iterator_elem,
                    span,
                );
                Some(Self::iterator_protocol_ty(mapped))
            }
            M::Filter | M::TakeWhile | M::SkipWhile => {
                if self.validate_iterator_method_arity(method, 1, args.len(), span) {
                    self.validate_iterator_callback_return(
                        method,
                        arg_types.first().unwrap_or(&ResolvedType::Unknown),
                        vec![iterator_elem.clone()],
                        ResolvedType::Bool,
                        span,
                    );
                }
                Some(Self::iterator_protocol_ty(iterator_elem))
            }
            M::FlatMap => {
                if !self.validate_iterator_method_arity(method, 1, args.len(), span) {
                    return Some(Self::iterator_protocol_ty(ResolvedType::Unknown));
                }
                let returned = self.iterator_mapping_callback_return_type(
                    arg_types.first().unwrap_or(&ResolvedType::Unknown),
                    iterator_elem,
                    span,
                );
                let Some(flat_elem) = self.iterable_protocol_element_type(&returned) else {
                    if !matches!(returned, ResolvedType::Unknown) {
                        let expected = ResolvedType::Generic(
                            Self::iterable_protocol_name().to_string(),
                            vec![ResolvedType::Unknown],
                        );
                        self.errors.push(errors::type_mismatch(
                            &expected.to_string(),
                            &returned.to_string(),
                            span,
                        ));
                    }
                    return Some(Self::iterator_protocol_ty(ResolvedType::Unknown));
                };
                Some(Self::iterator_protocol_ty(flat_elem))
            }
            M::Take | M::Skip => {
                if self.validate_iterator_method_arity(method, 1, args.len(), span)
                    && let Some(arg_ty) = arg_types.first()
                    && !self.types_compatible(arg_ty, &ResolvedType::Int)
                {
                    self.errors
                        .push(errors::type_mismatch("int", &arg_ty.to_string(), span));
                }
                Some(Self::iterator_protocol_ty(iterator_elem))
            }
            M::Chain => {
                if self.validate_iterator_method_arity(method, 1, args.len(), span)
                    && let Some(arg_ty) = arg_types.first()
                {
                    let expected = Self::iterator_protocol_ty(iterator_elem.clone());
                    if !self.types_compatible(arg_ty, &expected) {
                        self.errors
                            .push(errors::type_mismatch(&expected.to_string(), &arg_ty.to_string(), span));
                    }
                }
                Some(Self::iterator_protocol_ty(iterator_elem))
            }
            M::Enumerate => {
                self.validate_iterator_method_arity(method, 0, args.len(), span);
                Some(Self::iterator_protocol_ty(ResolvedType::Tuple(vec![
                    ResolvedType::Int,
                    iterat…32710 tokens truncated…all(
                            callable.as_str(),
                            &info,
                            type_args,
                            args,
                            span,
                            expected_return_ty,
                        )
                    }
                    (SymbolKind::FunctionOverloads(overloads), _) => {
                        self.record_source_target(span, source_module_path, source_name, "function");
                        self.validate_function_overload_call(
                            callable.as_str(),
                            &overloads,
                            type_args,
                            args,
                            span,
                            expected_return_ty,
                        )
                    }
                    (
                        SymbolKind::Type(type_info @ (TypeInfo::Model(_) | TypeInfo::Class(_) | TypeInfo::Newtype(_))),
                        Some(library),
                    ) => {
                        let mut source_type_path = source_module_path.iter().skip(2).cloned().collect::<Vec<_>>();
                        source_type_path.push(source_name.clone());
                        let canonical_name = canonical_public_library_type_name(&library, &source_type_path.join("::"));
                        let source_kind = Self::source_target_kind_for_type_info(&type_info).unwrap_or("type");
                        self.record_source_target(span, source_module_path, source_name, source_kind);
                        self.check_public_module_type_constructor_call(
                            PublicModuleConstructorContext {
                                display_name: &callable,
                                canonical_name: &canonical_name,
                                type_info: &type_info,
                            },
                            type_args,
                            args,
                            span,
                            span,
                        )
                    }
                    _ => ResolvedType::Unknown,
                };
            }
            self.errors
                .push(errors::missing_method(module_name.as_str(), method, span));
            return ResolvedType::Unknown;
        }

        if let Some(ret) = self.resolve_unambiguous_source_method_without_arg_prepass(
            &base_ty,
            method,
            type_args,
            args,
            span,
            expected_return_ty,
        ) {
            self.type_info.inherit_same_trait_method_module(base.span, span);
            return ret;
        }

        let contextual_rust_callable = expected_return_ty.and_then(|expected| {
            if args.len() == 1 {
                self.rust_callable_alias_signature(expected)
            } else {
                None
            }
        });

        // Rust callable bounds are not available until receiver metadata selects the method signature below. Defer
        // direct closures on Rust receivers so the selected `Fn`/`FnMut` bound can contextually type them once, rather
        // than first checking their parameters as unknowns and retaining spurious errors from that incomplete pass.
        let rust_receiver_path = self.rust_canonical_path_for_receiver_type(&base_ty);
        let defer_rust_closures = rust_receiver_path.is_some();

        // Collect arg types for method-specific validation.
        let arg_types: Vec<ResolvedType> = args
            .iter()
            .map(|arg| {
                let is_closure = match arg {
                    CallArg::Positional(expr)
                    | CallArg::Named(_, expr)
                    | CallArg::PositionalUnpack(expr)
                    | CallArg::KeywordUnpack(expr) => matches!(expr.node, Expr::Closure(_, _)),
                };
                if defer_rust_closures && contextual_rust_callable.is_none() && is_closure {
                    ResolvedType::Unknown
                } else {
                    self.check_method_arg_with_rust_callable_alias(arg, contextual_rust_callable.as_ref())
                }
            })
            .collect();

        if self.receiver_has_computed_property(&base_ty, method, span) {
            self.errors.push(errors::property_called_as_method(method, span));
            return ResolvedType::Unknown;
        }

        if let Some(ret) = self.resolve_iterator_protocol_method_call(&base_ty, method, args, &arg_types, span) {
            self.mark_direct_iterator_binding_consumed(base, method, span);
            return ret;
        }

        if let Some(path) = rust_receiver_path {
            if let Some(params) = self.rust_variant_callable_params(&path, method) {
                if !type_args.is_empty() {
                    self.errors
                        .push(errors::explicit_call_site_type_args_not_supported(span));
                }
                let arg_types = self.check_call_arg_types_for_params(args, &params);
                let mut type_bindings = std::collections::HashMap::new();
                self.validate_callable_arg_bindings(
                    format!("rust::{path}.{method}").as_str(),
                    &params,
                    args,
                    &arg_types,
                    &mut type_bindings,
                    span,
                );
                self.type_info.record_call_site_callable_params_exact(span, &params);
                return ResolvedType::RustPath(path);
            }
            if let Some(ret) = Self::known_rust_path_method_return(path.as_str(), method) {
                return ret;
            }
            let Some(ret) = self.resolve_rust_path_method_call(RustPathMethodCall {
                rust_path: &path,
                method,
                receiver_metadata: self.rust_metadata_for_receiver_expr(base).as_ref(),
                type_args,
                args,
                arg_types: &arg_types,
                receiver_span: base.span,
                expected_return_ty,
                span,
            }) else {
                // Metadata backend disabled/unavailable: preserve permissive RFC 005 behavior.
                return ResolvedType::Unknown;
            };
            return ret;
        }

        if let Some(ret) = self.check_numeric_resize_method(&base_ty, method, type_args, args, span, expected_return_ty)
        {
            return ret;
        }
        // Trait default methods typecheck against `Self`, so be permissive here too.
        if matches!(base_ty, ResolvedType::SelfType) {
            return ResolvedType::Unknown;
        }

        if self.nominal_type_supports_reflection_magic(&base_ty, method)
            && let Some(ret) = self.reflection_magic_method_return_type(&base_ty, method)
        {
            self.validate_reflection_magic_call(method, type_args, args, span);
            return ret;
        }

        let base_is_type_name = self.is_enum_type_name_expr(base);
        if let ResolvedType::Named(enum_name) = &base_ty
            && let Some(TypeInfo::Enum(enum_info)) = self.lookup_semantic_type_info(enum_name)
            && let Some(value_enum) = enum_info.value_enum.clone()
            && let Some(ret) = self.check_value_enum_generated_method_call(ValueEnumGeneratedCall {
                enum_name,
                value_enum: &value_enum,
                method,
                base_is_type_name,
                type_args,
                args,
                arg_types: &arg_types,
                span,
            })
        {
            return ret;
        }

        // Treat Enum.Variant(...) method-style calls as variant constructors
        if let ResolvedType::Named(enum_name) = &base_ty
            && let Some(TypeInfo::Enum(enum_info)) = self.lookup_semantic_type_info(enum_name)
            && (enum_info.variants.iter().any(|v| v == method) || enum_info.variant_aliases.contains_key(method))
        {
            // Args were checked above; no strict arity enforcement here.
            let _ = &arg_types; // keep for potential future validation
            return ResolvedType::Named(enum_name.clone());
        }

        // External/runtime-provided concurrency primitives: be permissive for surface types that have no local Incan
        // definition. Types defined in `.incn` source are resolved below through their extracted method signatures.
        if let ResolvedType::Named(name) = &base_ty
            && surface_types::from_str(name.as_str()).is_some()
            && self.lookup_semantic_type_info(name).is_none()
        {
            return ResolvedType::Unknown;
        }

        if matches!(
            &base_ty,
            ResolvedType::Named(name) if name == surface_types::as_str(SurfaceTypeId::Semaphore)
        ) {
            return match method {
                "acquire" => ResolvedType::Generic(
                    "Result".to_string(),
                    vec![
                        ResolvedType::Named(SEMAPHORE_PERMIT_TYPE_NAME.to_string()),
                        ResolvedType::Named(SEMAPHORE_ACQUIRE_ERROR_TYPE_NAME.to_string()),
                    ],
                ),
                "try_acquire" => option_ty(ResolvedType::Named(SEMAPHORE_PERMIT_TYPE_NAME.to_string())),
                "available_permits" => ResolvedType::Int,
                _ => ResolvedType::Unknown,
            };
        }

        if let ResolvedType::Generic(name, type_args) = &base_ty
            && surface_types::from_str(name.as_str()) == Some(SurfaceTypeId::Mutex)
        {
            let inner = type_args.first().cloned().unwrap_or(ResolvedType::Unknown);
            return match method {
                "lock" => ResolvedType::Generic("MutexGuard".to_string(), vec![inner.clone()]),
                "try_lock" => option_ty(ResolvedType::Generic("MutexGuard".to_string(), vec![inner])),
                _ => ResolvedType::Unknown,
            };
        }

        if let ResolvedType::Generic(name, type_args) = &base_ty
            && surface_types::from_str(name.as_str()) == Some(SurfaceTypeId::RwLock)
        {
            let inner = type_args.first().cloned().unwrap_or(ResolvedType::Unknown);
            return match method {
                "read" => ResolvedType::Generic("RwLockReadGuard".to_string(), vec![inner.clone()]),
                "write" => ResolvedType::Generic("RwLockWriteGuard".to_string(), vec![inner.clone()]),
                "try_read" => option_ty(ResolvedType::Generic(
                    "RwLockReadGuard".to_string(),
                    vec![inner.clone()],
                )),
                "try_write" => option_ty(ResolvedType::Generic("RwLockWriteGuard".to_string(), vec![inner])),
                _ => ResolvedType::Unknown,
            };
        }

        // Builtin methods for builtin types (so we don't report missing methods).
        if (matches!(base_ty, ResolvedType::Float)
            || matches!(
                base_ty,
                ResolvedType::Numeric(
                    incan_core::lang::types::numerics::NumericTypeId::F32
                        | incan_core::lang::types::numerics::NumericTypeId::F64
                )
            ))
            && let Some(id) = float_methods::from_str(method)
        {
            use float_methods::FloatMethodId as M;
            match id {
                M::IsNan | M::IsInfinite | M::IsFinite => return ResolvedType::Bool,
                _ => return ResolvedType::Float,
            }
        }
        if matches!(base_ty, ResolvedType::Bytes) && method == "as_slice" && args.is_empty() {
            return ResolvedType::Bytes;
        }

        if matches!(base_ty, ResolvedType::Str)
            && let Some(ret) = string_method_return(method, false)
        {
            return ret;
        }

        if is_frozen_str(&base_ty)
            && let Some(ret) = string_method_return(method, true)
        {
            return ret;
        }
        if is_frozen_bytes(&base_ty)
            && let Some(id) = frozen_bytes_methods::from_str(method)
        {
            use frozen_bytes_methods::FrozenBytesMethodId as M;
            match id {
                M::Len => return ResolvedType::Int,
                M::IsEmpty => return ResolvedType::Bool,
            }
        }

        match &base_ty {
            ResolvedType::FrozenList(_) => {
                if let Some(id) = frozen_list_methods::from_str(method) {
                    use frozen_list_methods::FrozenListMethodId as M;
                    match id {
                        M::Len => return ResolvedType::Int,
                        M::IsEmpty => return ResolvedType::Bool,
                    }
                }
            }
            ResolvedType::FrozenSet(_) => {
                if let Some(id) = frozen_set_methods::from_str(method) {
                    use frozen_set_methods::FrozenSetMethodId as M;
                    match id {
                        M::Len => return ResolvedType::Int,
                        M::IsEmpty | M::Contains => return ResolvedType::Bool,
                    }
                }
            }
            ResolvedType::FrozenDict(_, _) => {
                if let Some(id) = frozen_dict_methods::from_str(method) {
                    use frozen_dict_methods::FrozenDictMethodId as M;
                    match id {
                        M::Len => return ResolvedType::Int,
                        M::IsEmpty | M::ContainsKey => return ResolvedType::Bool,
                    }
                }
            }
            _ => {}
        }

        // Option[T] helpers.
        //
        // NOTE: `Dict.get(k)` is backed by Rust `HashMap::get`, which returns `Option<&V>`.
        // We model that as `Option[&V]` internally, so helpers like `.copied()` can typecheck in the same way they do
        // in Rust.
        if base_ty.is_option() {
            let inner = base_ty.option_inner_type().cloned().unwrap_or(ResolvedType::Unknown);
            match option_methods::from_str(method) {
                Some(option_methods::OptionMethodId::Copied) => {
                    // Rust: `Option<&T>::copied() -> Option<T>` (for `T: Copy`).
                    if let ResolvedType::Ref(t) | ResolvedType::RefMut(t) = inner {
                        let t = (*t).clone();
                        let is_unresolved_rust_generic = matches!(&t, ResolvedType::RustPath(path) if TypeChecker::rust_display_type_var_name(path).is_some());
                        if self.is_copy_type(&t) || self.is_generic_placeholder_type(&t) || is_unresolved_rust_generic {
                            return option_ty(t);
                        }
                    }
                }
                Some(option_methods::OptionMethodId::UnwrapOr) => {
                    // Rust: `Option<T>::unwrap_or(default: T) -> T`
                    //
                    // For `Option<&T>`, this is `unwrap_or(default: &T) -> &T`.
                    if let Some(default_ty) = arg_types.first()
                        && !self.types_compatible(default_ty, &inner)
                    {
                        self.errors
                            .push(errors::type_mismatch(&inner.to_string(), &default_ty.to_string(), span));
                    }
                    return inner;
                }
                Some(option_methods::OptionMethodId::Unwrap) => {
                    return inner;
                }
                None => {}
            }
        }

        if let ResolvedType::Generic(name, type_args) = &base_ty
            && collection_type_id(name.as_str()) == Some(CollectionTypeId::Result)
            && type_args.len() == 2
        {
            let ok_ty = type_args[0].clone();
            match result_methods::from_str(method) {
                Some(result_methods::ResultMethodId::Unwrap) => {
                    if !args.is_empty() {
                        self.errors.push(errors::type_mismatch(
                            "no arguments",
                            &format!("{} argument(s)", args.len()),
                            span,
                        ));
                    }
                    return ok_ty;
                }
                Some(result_methods::ResultMethodId::UnwrapOr) => {
                    if let Some(default_ty) = arg_types.first()
                        && !self.types_compatible(default_ty, &ok_ty)
                    {
                        self.errors
                            .push(errors::type_mismatch(&ok_ty.to_string(), &default_ty.to_string(), span));
                    }
                    if args.len() != 1 {
                        self.errors.push(errors::type_mismatch(
                            "one default argument",
                            &format!("{} argument(s)", args.len()),
                            span,
                        ));
                    }
                    return ok_ty;
                }
                _ => {}
            }
        }

        if let ResolvedType::Generic(name, type_args) = &base_ty
            && collection_type_id(name.as_str()) == Some(CollectionTypeId::Result)
            && type_args.len() == 2
            && Self::result_combinator_name(method)
        {
            return self.check_result_combinator_method(
                type_args[0].clone(),
                type_args[1].clone(),
                method,
                args,
                &arg_types,
                span,
            );
        }

        // FIXME: Too many levels of nesting here.
        if let ResolvedType::Generic(name, type_args) = &base_ty {
            if collection_type_id(name.as_str()) == Some(CollectionTypeId::Generator) {
                let elem = type_args.first().cloned().unwrap_or(ResolvedType::Unknown);
                use iterator_methods::IteratorMethodId as M;
                match iterator_methods::from_str(method) {
                    Some(M::Map) => {
                        let mapped = self.generator_map_return_type(&elem, args, &arg_types, span);
                        return generator_ty(mapped);
                    }
                    Some(M::Filter) => {
                        self.validate_generator_filter_arg(&elem, args, &arg_types, span);
                        return generator_ty(elem);
                    }
                    Some(M::Take) => {
                        self.validate_generator_take_arg(args, &arg_types, span);
                        return generator_ty(elem);
                    }
                    Some(M::Collect) => {
                        if !args.is_empty() {
                            self.errors.push(errors::type_mismatch(
                                "no arguments",
                                &format!("{} argument(s)", args.len()),
                                span,
                            ));
                        }
                        return list_ty(elem);
                    }
                    _ => {}
                }
            }

            if collection_type_id(name.as_str()) == Some(CollectionTypeId::List) {
                let elem = type_args.first().cloned().unwrap_or(ResolvedType::Unknown);
                if let Some(id) = list_methods::from_str(method) {
                    use list_methods::ListMethodId as M;
                    match id {
                        M::Append => {
                            let clone_ty = arg_types.first().unwrap_or(&elem);
                            if let Some(arg0) = arg_types.first()
                                && !self.types_compatible(arg0, &elem)
                            {
                                self.errors
                                    .push(errors::type_mismatch(&elem.to_string(), &arg0.to_string(), span));
                            }
                            if !self.is_copy_type(clone_ty) && !self.is_clone_type(clone_ty) {
                                self.errors
                                    .push(errors::list_append_requires_clone(&clone_ty.to_string(), span));
                            }
                            return ResolvedType::Unit;
                        }
                        M::Extend => {
                            let other_list_ty = list_ty(elem.clone());
                            if let Some(arg0) = arg_types.first()
                                && !self.types_compatible(arg0, &other_list_ty)
                            {
                                self.errors.push(errors::type_mismatch(
                                    &other_list_ty.to_string(),
                                    &arg0.to_string(),
                                    span,
                                ));
                            }
                            if !self.is_copy_type(&elem) && !self.is_clone_type(&elem) {
                                self.errors
                                    .push(errors::list_extend_requires_clone(&elem.to_string(), span));
                            }
                            return ResolvedType::Unit;
                        }
                        M::Clone => {
                            if !args.is_empty() {
                                self.errors.push(errors::type_mismatch(
                                    "no arguments",
                                    &format!("{} argument(s)", args.len()),
                                    span,
                                ));
                            }
                            if !self.is_copy_type(&elem) && !self.is_clone_type(&elem) {
                                self.errors
                                    .push(errors::list_clone_requires_clone(&elem.to_string(), span));
                            }
                            return list_ty(elem.clone());
                        }
                        M::Pop => return elem,
                        M::Contains => return ResolvedType::Bool,
                        M::Swap | M::Reserve | M::ReserveExact | M::Remove => return ResolvedType::Unit,
                        M::Count | M::Index => return ResolvedType::Int,
                    }
                }
            }
            if collection_type_id(name.as_str()) == Some(CollectionTypeId::Dict) {
                let key = type_args.first().cloned().unwrap_or(ResolvedType::Unknown);
                let val = type_args.get(1).cloned().unwrap_or(ResolvedType::Unknown);
                if let Some(id) = dict_methods::from_str(method) {
                    use dict_methods::DictMethodId as M;
                    match id {
                        M::Keys => return list_ty(key),
                        M::Values => return list_ty(val),
                        // `Dict.get(k)` is backed by Rust `HashMap::get`, which returns `Option<&V>`.
                        // Model this as an internal reference so chained Rust-idiom helpers (like `.copied()`)
                        // typecheck consistently with codegen.
                        M::Get => return option_ty(ResolvedType::Ref(Box::new(val.clone()))),
                        M::Insert => return ResolvedType::Unit,
                    }
                }
            }
            if collection_type_id(name.as_str()) == Some(CollectionTypeId::Set)
                && let Some(id) = set_methods::from_str(method)
            {
                use set_methods::SetMethodId as M;
                return match id {
                    M::Add => ResolvedType::Unit,
                    M::Contains => ResolvedType::Bool,
                };
            }
            if collection_type_id(name.as_str()) == Some(CollectionTypeId::Option) && method == "clone" {
                if !args.is_empty() {
                    self.errors.push(errors::type_mismatch(
                        "no arguments",
                        &format!("{} argument(s)", args.len()),
                        span,
                    ));
                }
                let inner = type_args.first().cloned().unwrap_or(ResolvedType::Unknown);
                if !self.is_copy_type(&inner)
                    && !self.is_clone_type(&inner)
                    && !matches!(inner, ResolvedType::RustPath(_))
                {
                    self.errors
                        .push(errors::list_clone_requires_clone(&inner.to_string(), span));
                }
                return option_ty(inner);
            }
        }

        if let Some(ret) =
            self.resolve_union_clone_trait_method_call(&base_ty, method, type_args, args, &arg_types, span)
        {
            return ret;
        }

        let trait_receiver_name = match &base_ty {
            ResolvedType::Named(name) | ResolvedType::Generic(name, _) => Some(name.as_str()),
            _ => None,
        };
        if trait_receiver_name.is_some_and(|name| self.lookup_semantic_trait_info(name).is_some()) {
            if let Some(ret) = self.resolve_trait_receiver_method(
                &base_ty,
                method,
                type_args,
                args,
                &arg_types,
                span,
                expected_return_ty,
            ) {
                self.type_info.inherit_same_trait_method_module(base.span, span);
                return ret;
            }
            self.errors
                .push(errors::missing_method(&base_ty.to_string(), method, span));
            return ResolvedType::Unknown;
        }

        if let ResolvedType::Generic(type_name, _type_args) = &base_ty
            && let Some(type_info) = self.lookup_semantic_type_info(type_name).cloned()
        {
            match type_info {
                TypeInfo::Model(model) => {
                    let trait_adoptions = self.trait_adoptions_for_type_methods(&model.trait_adoptions, &model.derives);
                    if let Some(ret) = self.resolve_named_method(
                        &model.methods,
                        Some(&model.method_overloads),
                        Some(&trait_adoptions),
                        method,
                        type_args,
                        args,
                        &arg_types,
                        span,
                        &base_ty,
                        expected_return_ty,
                    ) {
                        return ret;
                    }
                }
                TypeInfo::Class(class) => {
                    let trait_adoptions = self.trait_adoptions_for_type_methods(&class.trait_adoptions, &class.derives);
                    if let Some(ret) = self.resolve_named_method(
                        &class.methods,
                        Some(&class.method_overloads),
                        Some(&trait_adoptions),
                        method,
                        type_args,
                        args,
                        &arg_types,
                        span,
                        &base_ty,
                        expected_return_ty,
                    ) {
                        return ret;
                    }
                }
                TypeInfo::Enum(en) => {
                    let trait_adoptions = self.trait_adoptions_for_type_methods(&en.trait_adoptions, &en.derives);
                    if let Some(ret) = self.resolve_named_method(
                        &en.methods,
                        Some(&en.method_overloads),
                        Some(&trait_adoptions),
                        method,
                        type_args,
                        args,
                        &arg_types,
                        span,
                        &base_ty,
                        expected_return_ty,
                    ) {
                        return ret;
                    }
                }
                TypeInfo::Newtype(newtype) => {
                    let resolved_method = self.resolve_newtype_method_name(&newtype, method);
                    if let Some(ret) = self.resolve_named_method(
                        &newtype.methods,
                        Some(&newtype.method_overloads),
                        Some(&newtype.trait_adoptions),
                        resolved_method,
                        type_args,
                        args,
                        &arg_types,
                        span,
                        &base_ty,
                        expected_return_ty,
                    ) {
                        if newtype.is_rusttype {
                            self.maybe_record_rusttype_return_coercion(&newtype, resolved_method, &ret, span);
                        }
                        return ret;
                    }
                    if newtype.is_rusttype
                        && let ResolvedType::RustPath(path) = &newtype.underlying
                        && let Some(ret) = self.resolve_rust_path_method_call(RustPathMethodCall {
                            rust_path: path,
                            method: resolved_method,
                            receiver_metadata: None,
                            type_args,
                            args,
                            arg_types: &arg_types,
                            receiver_span: base.span,
                            expected_return_ty,
                            span,
                        })
                    {
                        return ret;
                    }
                }
                _ => {}
            }
        }

        // Named types: look up methods from the type definition.
        // If the symbol doesn't exist or isn't a type (e.g., Module/RustItem placeholder), treat it as external and
        // be permissive.
        if let ResolvedType::Named(type_name) = &base_ty {
            match self.lookup_semantic_type_info(type_name).cloned() {
                None => {
                    // Symbol not found or not a Type - treat as external, be permissive.
                    return ResolvedType::Unknown;
                }
                Some(type_info) => match type_info {
                    TypeInfo::Model(model) => {
                        let trait_adoptions =
                            self.trait_adoptions_for_type_methods(&model.trait_adoptions, &model.derives);
                        if let Some(ret) = self.resolve_named_method(
                            &model.methods,
                            Some(&model.method_overloads),
                            Some(&trait_adoptions),
                            method,
                            type_args,
                            args,
                            &arg_types,
                            span,
                            &base_ty,
                            expected_return_ty,
                        ) {
                            return ret;
                        }
                    }
                    TypeInfo::Class(class) => {
                        let trait_adoptions =
                            self.trait_adoptions_for_type_methods(&class.trait_adoptions, &class.derives);
                        if let Some(ret) = self.resolve_named_method(
                            &class.methods,
                            Some(&class.method_overloads),
                            Some(&trait_adoptions),
                            method,
                            type_args,
                            args,
                            &arg_types,
                            span,
                            &base_ty,
                            expected_return_ty,
                        ) {
                            return ret;
                        }
                    }
                    TypeInfo::Enum(en) => {
                        if enum_helpers::from_str(method) == Some(enum_helpers::EnumHelperId::Message) {
                            return ResolvedType::Str;
                        }
                        let trait_adoptions = self.trait_adoptions_for_type_methods(&en.trait_adoptions, &en.derives);
                        if let Some(ret) = self.resolve_named_method(
                            &en.methods,
                            Some(&en.method_overloads),
                            Some(&trait_adoptions),
                            method,
                            type_args,
                            args,
                            &arg_types,
                            span,
                            &base_ty,
                            expected_return_ty,
                        ) {
                            return ret;
                        }
                    }
                    TypeInfo::Newtype(nt) => {
                        let resolved_method = self.resolve_newtype_method_name(&nt, method);
                        if let Some(ret) = self.resolve_named_method(
                            &nt.methods,
                            Some(&nt.method_overloads),
                            Some(&nt.trait_adoptions),
                            resolved_method,
                            type_args,
                            args,
                            &arg_types,
                            span,
                            &base_ty,
                            expected_return_ty,
                        ) {
                            // When the method body is abstract and the underlying Rust type is known,
                            // check whether the actual Rust return type needs a coercion (e.g. &str → String).
                            if nt.is_rusttype {
                                self.maybe_record_rusttype_return_coercion(&nt, resolved_method, &ret, span);
                            }
                            return ret;
                        }
                        if nt.is_rusttype
                            && let ResolvedType::RustPath(path) = &nt.underlying
                            && let Some(ret) = self.resolve_rust_path_method_call(RustPathMethodCall {
                                rust_path: path,
                                method: resolved_method,
                                receiver_metadata: None,
                                type_args,
                                args,
                                arg_types: &arg_types,
                                receiver_span: base.span,
                                expected_return_ty,
                                span,
                            })
                        {
                            return ret;
                        }
                    }
                    _ => {}
                },
            }
        }

        // Reflection magic helpers are modeled explicitly above and should error on unsupported receivers rather than
        // silently degrading to Unknown. Keep the older permissive fallback only for the remaining backend-only magic.
        if let Some(id) = magic_methods::from_str(method)
            && !matches!(
                id,
                magic_methods::MagicMethodId::ClassName
                    | magic_methods::MagicMethodId::Fields
                    | magic_methods::MagicMethodId::FieldValue
                    | magic_methods::MagicMethodId::FieldItems
            )
        {
            return ResolvedType::Unknown;
        }

        // For common external generic types (interop/runtime-provided) that we don't model in the checker, be
        // permissive and do not error on unknown methods.
        if let ResolvedType::Generic(name, _args) = &base_ty
            && self.lookup_semantic_type_info(name).is_none()
        {
            return ResolvedType::Unknown;
        }

        // RFC 023: Method calls on generic type variables are permissive.
        //
        // The Rust backend infers the required trait bounds (e.g., `x.clone()` → `T: Clone`).
        // At the Incan typechecker level we allow the call and return the same type variable.
        if self.is_generic_placeholder_type(&base_ty) {
            if let Some(placeholder_name) = self.generic_placeholder_name(&base_ty).map(str::to_string)
                && let Some(ret) = self.resolve_generic_placeholder_method(
                    &placeholder_name,
                    method,
                    type_args,
                    args,
                    &arg_types,
                    span,
                    &base_ty,
                    expected_return_ty,
                )
            {
                return ret;
            }
            if let Some(ret) = self.generic_reflection_magic_method_return_type(method) {
                self.validate_reflection_magic_call(method, type_args, args, span);
                return ret;
            }
            return base_ty.clone();
        }

        // Guardrail: don't silently return Unknown for missing methods on known user types.
        // For unknown/external types we returned Unknown above without error.
        let base_name_str = base_ty.to_string();
        let skip_error_for_known_runtime = surface_types::from_str(base_name_str.as_str()).is_some();
        if !(matches!(base_ty, ResolvedType::Named(ref n) if self.symbols.lookup(n).is_none())
            || skip_error_for_known_runtime)
        {
            self.errors
                .push(errors::missing_method(&base_ty.to_string(), method, span));
        }
        ResolvedType::Unknown
    }

    /// Resolve methods supplied by Clone for anonymous union wrappers.
    fn resolve_union_clone_trait_method_call(
        &mut self,
        receiver_ty: &ResolvedType,
        method: &str,
        type_args: &[Spanned<Type>],
        args: &[CallArg],
        arg_types: &[ResolvedType],
        span: Span,
    ) -> Option<ResolvedType> {
        if !receiver_ty.is_union() {
            return None;
        }

        let adoption = TypeBoundInfo {
            name: core_traits::as_str(TraitId::Clone).to_string(),
            source_name: None,
            type_args: Vec::new(),
            module_path: None,
        };
        let method_info = self.trait_method_info_resolved_for_adoption(&adoption, method, span)?;
        if !self.is_clone_type(receiver_ty) {
            self.errors.push(CompileError::type_error(
                format!("Union type '{receiver_ty}' cannot use '{method}(...)' because not all variants are cloneable"),
                span,
            ));
            return Some(ResolvedType::Unknown);
        }
        Some(self.check_generic_method_call(method, method_info, type_args, args, arg_types, span, receiver_ty, None))
    }

    /// Return known method result types for Rust imports when rust-inspect metadata is not specific enough.
    fn known_rust_path_method_return(path: &str, method: &str) -> Option<ResolvedType> {
        use incan_core::lang::types::numerics::NumericTypeId as N;

        match (path, method) {
            ("xxhash_rust::xxh32::Xxh32", "digest") => Some(ResolvedType::Numeric(N::U32)),
            ("xxhash_rust::xxh64::Xxh64", "digest") => Some(ResolvedType::Numeric(N::U64)),
            ("xxhash_rust::xxh3::Xxh3Default", "digest") => Some(ResolvedType::Numeric(N::U64)),
            ("xxhash_rust::xxh3::Xxh3Default", "digest128") => Some(ResolvedType::Numeric(N::U128)),
            _ => None,
        }
    }

    /// Return whether a borrowed Rust value's `to_vec` method produces Incan `bytes`.
    fn rust_ref_to_vec_returns_bytes(path: &str) -> bool {
        path.starts_with("[u8") || (path.contains("GenericArray") && (path.contains("<u8") || path.contains("u8,")))
    }

    /// Return whether a `to_vec` receiver shape is known to produce digest bytes even when rust-inspect erases it.
    fn rust_to_vec_receiver_is_known_byte_output(base: &Spanned<Expr>) -> bool {
        match &base.node {
            Expr::MethodCall(_, method, _, _) => matches!(method.as_str(), "as_bytes" | "digest" | "finalize_reset"),
            _ => false,
        }
    }
}
