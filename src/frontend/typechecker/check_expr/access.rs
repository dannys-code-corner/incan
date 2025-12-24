//! Check indexing, slicing, field access, and method calls.
//!
//! These helpers validate access patterns like `xs[i]`, `xs[a:b]`, `obj.field`, and
//! `obj.method(...)`, emitting diagnostics for missing fields/methods and incompatible uses.

use crate::frontend::ast::*;
use crate::frontend::diagnostics::errors;
use crate::frontend::symbols::*;
use crate::frontend::typechecker::helpers::{
    DICT_TY_NAME, LIST_TY_NAME, SET_TY_NAME, is_frozen_bytes, is_frozen_str, is_intlike_for_index, list_ty, option_ty,
    string_method_return,
};

use super::TypeChecker;

impl TypeChecker {
    /// Type-check an indexing expression (`base[index]`) and return the element type.
    pub(in crate::frontend::typechecker::check_expr) fn check_index(
        &mut self,
        base: &Spanned<Expr>,
        index: &Spanned<Expr>,
        span: Span,
    ) -> ResolvedType {
        let base_ty = self.check_expr(base);
        let index_ty = self.check_expr(index);

        match base_ty {
            ResolvedType::Generic(name, args) => match name.as_str() {
                "List" if !args.is_empty() => {
                    if !is_intlike_for_index(&index_ty) {
                        self.errors
                            .push(errors::index_type_mismatch("int", &index_ty.to_string(), index.span));
                    }
                    args[0].clone()
                }
                "Dict" if args.len() >= 2 => {
                    let key_ty = &args[0];
                    if !self.types_compatible(&index_ty, key_ty) {
                        self.errors.push(errors::index_type_mismatch(
                            &key_ty.to_string(),
                            &index_ty.to_string(),
                            index.span,
                        ));
                    }
                    args[1].clone()
                }
                "Tuple" => {
                    // `Tuple[T1, ...]` (and `tuple[...]` normalized) behaves like a tuple.
                    let elems = args;
                    let Expr::Literal(Literal::Int(raw_idx)) = &index.node else {
                        self.errors.push(errors::tuple_index_requires_int_literal(index.span));
                        return ResolvedType::Unknown;
                    };
                    let len = elems.len() as i64;
                    let mut idx = *raw_idx;
                    if idx < 0 {
                        idx += len;
                    }
                    if idx < 0 || idx >= len {
                        self.errors
                            .push(errors::tuple_index_out_of_bounds(*raw_idx, elems.len(), span));
                        return ResolvedType::Unknown;
                    }
                    elems.get(idx as usize).cloned().unwrap_or(ResolvedType::Unknown)
                }
                _ => ResolvedType::Unknown,
            },
            ty if matches!(ty, ResolvedType::Str) || is_frozen_str(&ty) => {
                if !is_intlike_for_index(&index_ty) {
                    self.errors
                        .push(errors::index_type_mismatch("int", &index_ty.to_string(), index.span));
                }
                ResolvedType::Str
            }
            ResolvedType::Tuple(elems) => {
                // Guardrail: tuple indexing must be an integer literal so we can bounds-check.
                let Expr::Literal(Literal::Int(raw_idx)) = &index.node else {
                    self.errors.push(errors::tuple_index_requires_int_literal(index.span));
                    return ResolvedType::Unknown;
                };
                let len = elems.len() as i64;
                let mut idx = *raw_idx;
                if idx < 0 {
                    idx += len;
                }
                if idx < 0 || idx >= len {
                    self.errors
                        .push(errors::tuple_index_out_of_bounds(*raw_idx, elems.len(), span));
                    return ResolvedType::Unknown;
                }
                elems.get(idx as usize).cloned().unwrap_or(ResolvedType::Unknown)
            }
            _ => ResolvedType::Unknown,
        }
    }

    /// Type-check a slicing expression (`base[start:end:step]`) and return the sliced type.
    pub(in crate::frontend::typechecker::check_expr) fn check_slice(
        &mut self,
        base: &Spanned<Expr>,
        slice: &SliceExpr,
        _span: Span,
    ) -> ResolvedType {
        let base_ty = self.check_expr(base);

        let start_ty = slice.start.as_ref().map(|s| self.check_expr(s));
        let end_ty = slice.end.as_ref().map(|e| self.check_expr(e));
        let step_ty = slice.step.as_ref().map(|st| self.check_expr(st));

        // Helper: validate that an already-computed type is int-like (or Unknown during inference).
        let check_intlike_ty = |ty: &ResolvedType, span: Span, errors: &mut Vec<_>| {
            if !is_intlike_for_index(ty) {
                errors.push(errors::index_type_mismatch("int", &ty.to_string(), span));
            }
        };
        // Helper: if a slice component exists, validate its already-computed type using the component span.
        let check_component = |ty_opt: Option<&ResolvedType>, expr_opt: Option<&Spanned<Expr>>, errors: &mut Vec<_>| {
            if let (Some(ty), Some(expr)) = (ty_opt, expr_opt) {
                check_intlike_ty(ty, expr.span, errors);
            }
        };

        match base_ty {
            ResolvedType::Generic(name, args) => match name.as_str() {
                LIST_TY_NAME => ResolvedType::Generic(LIST_TY_NAME.to_string(), args),
                _ => ResolvedType::Unknown,
            },
            ResolvedType::Str => {
                // We typecheck each slice component once (above) and reuse the computed types here.
                // This avoids re-walking the same expression multiple times and keeps error reporting
                // anchored to the original component spans.
                check_component(start_ty.as_ref(), slice.start.as_deref(), &mut self.errors);
                check_component(end_ty.as_ref(), slice.end.as_deref(), &mut self.errors);
                check_component(step_ty.as_ref(), slice.step.as_deref(), &mut self.errors);
                ResolvedType::Str
            }
            ty if is_frozen_str(&ty) => {
                // `FrozenStr` is the const-eval / deeply-immutable string type, but for indexing/slicing
                // it behaves like `str`: indices must be int-like (or Unknown during inference).
                // Reuse the exact same helper as `str` (the only difference is the receiver type).
                check_component(start_ty.as_ref(), slice.start.as_deref(), &mut self.errors);
                check_component(end_ty.as_ref(), slice.end.as_deref(), &mut self.errors);
                check_component(step_ty.as_ref(), slice.step.as_deref(), &mut self.errors);
                ResolvedType::Str
            }
            _ => ResolvedType::Unknown,
        }
    }

    /// Type-check a field access (`base.field`) and return the field type.
    pub(in crate::frontend::typechecker::check_expr) fn check_field(
        &mut self,
        base: &Spanned<Expr>,
        field: &str,
        span: Span,
    ) -> ResolvedType {
        // Handle builtin math module
        if let Expr::Ident(name) = &base.node {
            if name == "math" {
                match field {
                    "pi" | "e" | "tau" | "inf" | "nan" => return ResolvedType::Float,
                    _ => {}
                }
            }
        }

        let base_ty = self.check_expr(base);

        // Be permissive for unknown receivers: allow field access and continue typechecking.
        if matches!(base_ty, ResolvedType::Unknown) {
            return ResolvedType::Unknown;
        }

        match &base_ty {
            ResolvedType::Tuple(elements) => {
                if let Ok(idx) = field.parse::<usize>() {
                    if idx < elements.len() {
                        return elements[idx].clone();
                    }
                }
                self.errors
                    .push(errors::missing_field(&base_ty.to_string(), field, span));
                ResolvedType::Unknown
            }
            ResolvedType::Named(type_name) => {
                if let Some(type_info) = self.lookup_type_info(type_name) {
                    match type_info {
                        TypeInfo::Model(model) => {
                            if let Some(field_info) = model.fields.get(field) {
                                return field_info.ty.clone();
                            }
                        }
                        TypeInfo::Class(class) => {
                            if let Some(field_info) = class.fields.get(field) {
                                return field_info.ty.clone();
                            }
                        }
                        TypeInfo::Enum(enum_info) => {
                            if enum_info.variants.contains(&field.to_string()) {
                                return ResolvedType::Named(type_name.clone());
                            }
                        }
                        TypeInfo::Newtype(nt) => {
                            if field == "0" {
                                return nt.underlying.clone();
                            }
                        }
                        _ => {}
                    }
                }
                self.errors.push(errors::missing_field(type_name, field, span));
                ResolvedType::Unknown
            }
            _ => {
                self.errors
                    .push(errors::missing_field(&base_ty.to_string(), field, span));
                ResolvedType::Unknown
            }
        }
    }

    /// Type-check a method call (`base.method(args...)`) and return the method's return type.
    pub(in crate::frontend::typechecker::check_expr) fn check_method_call(
        &mut self,
        base: &Spanned<Expr>,
        method: &str,
        args: &[CallArg],
        span: Span,
    ) -> ResolvedType {
        let base_ty = self.check_expr(base);
        // Collect arg types for method-specific validation.
        let arg_types: Vec<ResolvedType> = args
            .iter()
            .map(|arg| match arg {
                CallArg::Positional(e) | CallArg::Named(_, e) => self.check_expr(e),
            })
            .collect();

        // If the receiver type is Unknown, be permissive and do not error on methods.
        if matches!(base_ty, ResolvedType::Unknown) {
            return ResolvedType::Unknown;
        }

        // Treat Enum.Variant(...) method-style calls as variant constructors
        if let ResolvedType::Named(enum_name) = &base_ty {
            if let Some(TypeInfo::Enum(enum_info)) = self.lookup_type_info(enum_name) {
                if enum_info.variants.iter().any(|v| v == method) {
                    // Args were checked above; no strict arity enforcement here.
                    let _ = &arg_types; // keep for potential future validation
                    return ResolvedType::Named(enum_name.clone());
                }
            }
        }

        // External/runtime-provided concurrency primitives: be permissive
        if let ResolvedType::Named(name) = &base_ty {
            match name.as_str() {
                "Mutex" | "RwLock" | "Semaphore" | "Barrier" => {
                    return ResolvedType::Unknown;
                }
                _ => {}
            }
        }

        // Builtin methods for builtin types (so we don't report missing methods).
        if matches!(base_ty, ResolvedType::Float) {
            match method {
                // Math functions available on f64 in Rust
                "sqrt" | "abs" | "floor" | "ceil" | "round" | "sin" | "cos" | "tan" | "exp" | "ln" | "log2"
                | "log10" => return ResolvedType::Float,
                "is_nan" | "is_infinite" | "is_finite" => return ResolvedType::Bool,
                "powi" => return ResolvedType::Float, // float.powi(int) -> float
                "powf" => return ResolvedType::Float, // float.powf(float) -> float
                _ => {}
            }
        }

        if matches!(base_ty, ResolvedType::Str) {
            if let Some(ret) = string_method_return(method, false) {
                return ret;
            }
        }

        if is_frozen_str(&base_ty) {
            if let Some(ret) = string_method_return(method, true) {
                return ret;
            }
        }
        if is_frozen_bytes(&base_ty) {
            match method {
                "len" => return ResolvedType::Int,
                "is_empty" => return ResolvedType::Bool,
                _ => {}
            }
        }

        match &base_ty {
            ResolvedType::FrozenList(_) => match method {
                "len" => return ResolvedType::Int,
                "is_empty" => return ResolvedType::Bool,
                _ => {}
            },
            ResolvedType::FrozenSet(_) => match method {
                "len" => return ResolvedType::Int,
                "is_empty" => return ResolvedType::Bool,
                "contains" => return ResolvedType::Bool,
                _ => {}
            },
            ResolvedType::FrozenDict(_, _) => match method {
                "len" => return ResolvedType::Int,
                "is_empty" => return ResolvedType::Bool,
                "contains_key" => return ResolvedType::Bool,
                _ => {}
            },
            _ => {}
        }

        if let ResolvedType::Generic(name, type_args) = &base_ty {
            if name == LIST_TY_NAME {
                let elem = type_args.first().cloned().unwrap_or(ResolvedType::Unknown);
                match method {
                    "append" => {
                        if let Some(arg0) = arg_types.first() {
                            if !self.types_compatible(arg0, &elem) {
                                self.errors
                                    .push(errors::type_mismatch(&elem.to_string(), &arg0.to_string(), span));
                            }
                        }
                        return ResolvedType::Unit;
                    }
                    "pop" => return elem,
                    "contains" => return ResolvedType::Bool,
                    "swap" => return ResolvedType::Unit,
                    "reserve" => return ResolvedType::Unit,
                    "reserve_exact" => return ResolvedType::Unit,
                    "remove" => return ResolvedType::Unit,
                    "count" => return ResolvedType::Int,
                    "index" => return ResolvedType::Int,
                    _ => {}
                }
            }
            if name == DICT_TY_NAME {
                let key = type_args.first().cloned().unwrap_or(ResolvedType::Unknown);
                let val = type_args.get(1).cloned().unwrap_or(ResolvedType::Unknown);
                match method {
                    "keys" => return list_ty(key),
                    "values" => return list_ty(val),
                    // Allow get/insert helpers to match examples; keep return types simple.
                    "get" => return option_ty(val.clone()),
                    "insert" => return ResolvedType::Unit,
                    _ => {}
                }
            }
            if name == SET_TY_NAME && method == "contains" {
                return ResolvedType::Bool;
            }
        }

        // Named types: look up methods from the type definition.
        // If the symbol doesn't exist or isn't a type (e.g., Module/RustModule placeholder),
        // treat it as external and be permissive.
        if let ResolvedType::Named(type_name) = &base_ty {
            match self.lookup_type_info(type_name) {
                None => {
                    // Symbol not found or not a Type - treat as external, be permissive.
                    return ResolvedType::Unknown;
                }
                Some(type_info) => match type_info {
                    TypeInfo::Model(model) => {
                        if let Some(method_info) = model.methods.get(method) {
                            return method_info.return_type.clone();
                        }
                    }
                    TypeInfo::Class(class) => {
                        if let Some(method_info) = class.methods.get(method) {
                            return method_info.return_type.clone();
                        }
                        for trait_name in &class.traits {
                            if let Some(tid) = self.symbols.lookup(trait_name) {
                                if let Some(tsym) = self.symbols.get(tid) {
                                    if let SymbolKind::Trait(trait_info) = &tsym.kind {
                                        if let Some(method_info) = trait_info.methods.get(method) {
                                            return method_info.return_type.clone();
                                        }
                                    }
                                }
                            }
                        }
                    }
                    TypeInfo::Enum(_enum_info) => {
                        // Be permissive for common error/display helpers on enums
                        if method == "message" {
                            return ResolvedType::Str;
                        }
                    }
                    TypeInfo::Newtype(nt) => {
                        if let Some(method_info) = nt.methods.get(method) {
                            return method_info.return_type.clone();
                        }
                    }
                    _ => {}
                },
            }
        }

        // For dunder-like helpers that codegen injects (e.g., __class_name__, __fields__),
        // be permissive at typecheck time since they are backend-provided.
        if method.starts_with("__") {
            return ResolvedType::Unknown;
        }

        // For common external generic types (interop/runtime-provided) that we don't model in
        // the checker, be permissive and do not error on unknown methods.
        if let ResolvedType::Generic(name, _args) = &base_ty {
            match name.as_str() {
                "Mutex" | "RwLock" | "Semaphore" | "Barrier" | "Result" | "Option" | "HashMap" | "Vec" | "List"
                | "Tuple" => {
                    return ResolvedType::Unknown;
                }
                _ => {}
            }
        }

        // Guardrail: don't silently return Unknown for missing methods on known user types.
        // For unknown/external types we returned Unknown above without error.
        let base_name_str = base_ty.to_string();
        let skip_error_for_known_runtime =
            matches!(base_name_str.as_str(), "Mutex" | "RwLock" | "Semaphore" | "Barrier");
        if !(matches!(base_ty, ResolvedType::Named(ref n) if self.symbols.lookup(n).is_none())
            || skip_error_for_known_runtime)
        {
            self.errors
                .push(errors::missing_method(&base_ty.to_string(), method, span));
        }
        ResolvedType::Unknown
    }
}
