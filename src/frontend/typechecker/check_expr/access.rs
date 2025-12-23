//! Check indexing, slicing, field access, and method calls.
//!
//! These helpers validate access patterns like `xs[i]`, `xs[a:b]`, `obj.field`, and
//! `obj.method(...)`, emitting diagnostics for missing fields/methods and incompatible uses.

use crate::frontend::ast::*;
use crate::frontend::diagnostics::errors;
use crate::frontend::symbols::*;

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
                    if !matches!(index_ty, ResolvedType::Int)
                        && !matches!(index_ty, ResolvedType::Unknown)
                    {
                        self.errors.push(errors::index_type_mismatch(
                            "int",
                            &index_ty.to_string(),
                            index.span,
                        ));
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
                        self.errors
                            .push(errors::tuple_index_requires_int_literal(index.span));
                        return ResolvedType::Unknown;
                    };
                    let len = elems.len() as i64;
                    let mut idx = *raw_idx;
                    if idx < 0 {
                        idx += len;
                    }
                    if idx < 0 || idx >= len {
                        self.errors.push(errors::tuple_index_out_of_bounds(
                            *raw_idx,
                            elems.len(),
                            span,
                        ));
                        return ResolvedType::Unknown;
                    }
                    elems
                        .get(idx as usize)
                        .cloned()
                        .unwrap_or(ResolvedType::Unknown)
                }
                _ => ResolvedType::Unknown,
            },
            ResolvedType::Str => ResolvedType::Str,
            ResolvedType::Tuple(elems) => {
                // Guardrail: tuple indexing must be an integer literal so we can bounds-check.
                let Expr::Literal(Literal::Int(raw_idx)) = &index.node else {
                    self.errors
                        .push(errors::tuple_index_requires_int_literal(index.span));
                    return ResolvedType::Unknown;
                };
                let len = elems.len() as i64;
                let mut idx = *raw_idx;
                if idx < 0 {
                    idx += len;
                }
                if idx < 0 || idx >= len {
                    self.errors.push(errors::tuple_index_out_of_bounds(
                        *raw_idx,
                        elems.len(),
                        span,
                    ));
                    return ResolvedType::Unknown;
                }
                elems
                    .get(idx as usize)
                    .cloned()
                    .unwrap_or(ResolvedType::Unknown)
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

        if let Some(start) = &slice.start {
            self.check_expr(start);
        }
        if let Some(end) = &slice.end {
            self.check_expr(end);
        }
        if let Some(step) = &slice.step {
            self.check_expr(step);
        }

        match base_ty {
            ResolvedType::Generic(name, args) => match name.as_str() {
                "List" => ResolvedType::Generic("List".to_string(), args),
                _ => ResolvedType::Unknown,
            },
            ResolvedType::Str => ResolvedType::Str,
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
                if let Some(id) = self.symbols.lookup(type_name) {
                    if let Some(sym) = self.symbols.get(id) {
                        if let SymbolKind::Type(type_info) = &sym.kind {
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
                    }
                }
                self.errors
                    .push(errors::missing_field(type_name, field, span));
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
            if let Some(id) = self.symbols.lookup(enum_name) {
                if let Some(sym) = self.symbols.get(id) {
                    if let SymbolKind::Type(TypeInfo::Enum(enum_info)) = &sym.kind {
                        if enum_info.variants.iter().any(|v| v == method) {
                            // Args were checked above; no strict arity enforcement here.
                            let _ = &arg_types; // keep for potential future validation
                            return ResolvedType::Named(enum_name.clone());
                        }
                    }
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
                "sqrt" | "abs" | "floor" | "ceil" | "round" | "sin" | "cos" | "tan" | "exp"
                | "ln" | "log2" | "log10" => return ResolvedType::Float,
                "is_nan" | "is_infinite" | "is_finite" => return ResolvedType::Bool,
                "powi" => return ResolvedType::Float, // float.powi(int) -> float
                "powf" => return ResolvedType::Float, // float.powf(float) -> float
                _ => {}
            }
        }

        if matches!(base_ty, ResolvedType::Str) {
            match method {
                "upper" | "lower" | "strip" | "replace" | "join" => return ResolvedType::Str,
                // Common string helpers used in examples/std interop
                "split_whitespace" => {
                    return ResolvedType::Generic("List".to_string(), vec![ResolvedType::Str]);
                }
                "to_string" => return ResolvedType::Str,
                "contains" | "startswith" | "endswith" => return ResolvedType::Bool,
                "split" => {
                    return ResolvedType::Generic("List".to_string(), vec![ResolvedType::Str]);
                }
                _ => {}
            }
        }

        if let ResolvedType::Named(name) = &base_ty {
            if name == "FrozenStr" {
                match method {
                    "len" => return ResolvedType::Int,
                    "is_empty" => return ResolvedType::Bool,
                    // Treat FrozenStr like `str` for common string operations.
                    "upper" | "lower" | "strip" | "replace" | "join" => return ResolvedType::Str,
                    "split_whitespace" => {
                        return ResolvedType::Generic("List".to_string(), vec![ResolvedType::Str]);
                    }
                    "to_string" => return ResolvedType::Str,
                    "contains" | "startswith" | "endswith" => return ResolvedType::Bool,
                    "split" => {
                        return ResolvedType::Generic("List".to_string(), vec![ResolvedType::Str]);
                    }
                    _ => {}
                }
            }
            if name == "FrozenBytes" {
                match method {
                    "len" => return ResolvedType::Int,
                    "is_empty" => return ResolvedType::Bool,
                    _ => {}
                }
            }
        }

        if let ResolvedType::Generic(name, type_args) = &base_ty {
            if name == "List" {
                let elem = type_args.first().cloned().unwrap_or(ResolvedType::Unknown);
                match method {
                    "append" => {
                        if let Some(arg0) = arg_types.first() {
                            if !self.types_compatible(arg0, &elem) {
                                self.errors.push(errors::type_mismatch(
                                    &elem.to_string(),
                                    &arg0.to_string(),
                                    span,
                                ));
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
            if name == "Dict" {
                let key = type_args.first().cloned().unwrap_or(ResolvedType::Unknown);
                let val = type_args.get(1).cloned().unwrap_or(ResolvedType::Unknown);
                match method {
                    "keys" => {
                        return ResolvedType::Generic("List".to_string(), vec![key]);
                    }
                    "values" => {
                        return ResolvedType::Generic("List".to_string(), vec![val]);
                    }
                    // Allow get/insert helpers to match examples; keep return types simple.
                    "get" => {
                        return ResolvedType::Generic("Option".to_string(), vec![val.clone()]);
                    }
                    "insert" => {
                        return ResolvedType::Unit;
                    }
                    _ => {}
                }
            }
            if name == "Set" && method == "contains" {
                return ResolvedType::Bool;
            }

            // Frozen read-only APIs
            if name == "FrozenList" {
                match method {
                    "len" => return ResolvedType::Int,
                    "is_empty" => return ResolvedType::Bool,
                    _ => {}
                }
            }
            if name == "FrozenSet" {
                match method {
                    "len" => return ResolvedType::Int,
                    "is_empty" => return ResolvedType::Bool,
                    "contains" => return ResolvedType::Bool,
                    _ => {}
                }
            }
            if name == "FrozenDict" {
                match method {
                    "len" => return ResolvedType::Int,
                    "is_empty" => return ResolvedType::Bool,
                    "contains_key" => return ResolvedType::Bool,
                    _ => {}
                }
            }
        }

        // FIXME: lots of nested ifs here, we should refactor this to be more readable.
        if let ResolvedType::Named(type_name) = &base_ty {
            // If we don't know this type at all (not in symbol table), treat it as an external
            // type (e.g., Rust interop) and be permissive: return Unknown without error.
            if self.symbols.lookup(type_name).is_none() {
                return ResolvedType::Unknown;
            }

            if let Some(id) = self.symbols.lookup(type_name) {
                if let Some(sym) = self.symbols.get(id) {
                    // If the symbol isn't a Type (e.g., a Module/RustModule placeholder),
                    // treat it as external and be permissive.
                    if !matches!(sym.kind, SymbolKind::Type(_)) {
                        return ResolvedType::Unknown;
                    }
                    if let SymbolKind::Type(type_info) = &sym.kind {
                        match type_info {
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
                                                if let Some(method_info) =
                                                    trait_info.methods.get(method)
                                                {
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
                        }
                    }
                }
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
                "Mutex" | "RwLock" | "Semaphore" | "Barrier" | "Result" | "Option" | "HashMap"
                | "Vec" | "List" | "Tuple" => {
                    return ResolvedType::Unknown;
                }
                _ => {}
            }
        }

        // Guardrail: don't silently return Unknown for missing methods on known user types.
        // For unknown/external types we returned Unknown above without error.
        let base_name_str = base_ty.to_string();
        let skip_error_for_known_runtime = matches!(
            base_name_str.as_str(),
            "Mutex" | "RwLock" | "Semaphore" | "Barrier"
        );
        if !(matches!(base_ty, ResolvedType::Named(ref n) if self.symbols.lookup(n).is_none())
            || skip_error_for_known_runtime)
        {
            self.errors
                .push(errors::missing_method(&base_ty.to_string(), method, span));
        }
        ResolvedType::Unknown
    }
}
