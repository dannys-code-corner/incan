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
        _span: Span,
    ) -> ResolvedType {
        let base_ty = self.check_expr(base);
        self.check_expr(index);

        match base_ty {
            ResolvedType::Generic(name, args) => match name.as_str() {
                "List" if !args.is_empty() => args[0].clone(),
                "Dict" if args.len() >= 2 => args[1].clone(),
                _ => ResolvedType::Unknown,
            },
            ResolvedType::Str => ResolvedType::Str,
            ResolvedType::Tuple(elems) => elems.first().cloned().unwrap_or(ResolvedType::Unknown),
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
        _span: Span,
    ) -> ResolvedType {
        let base_ty = self.check_expr(base);
        self.check_call_args(args);

        // FIXME: lots of nested ifs here, we should refactor this to be more readable.
        if let ResolvedType::Named(type_name) = &base_ty {
            if let Some(id) = self.symbols.lookup(type_name) {
                if let Some(sym) = self.symbols.get(id) {
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

        ResolvedType::Unknown
    }
}
