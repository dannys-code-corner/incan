//! Check calls, constructors, and builtins.
//!
//! This module handles the main call-expression logic (`foo(...)`), including special-cased
//! builtins like `Ok(...)`/`Err(...)` and runtime helpers like `sleep(...)`. It also provides
//! small utilities to type-check call argument lists consistently.

use crate::frontend::ast::*;
use crate::frontend::diagnostics::errors;
use crate::frontend::symbols::*;
use crate::frontend::typechecker::helpers::{collection_type_id, dict_ty, list_ty, option_ty, result_ty, set_ty};
use incan_core::lang::builtins::{self, BuiltinFnId};
use incan_core::lang::surface::constructors::{self, ConstructorId};
use incan_core::lang::surface::functions::{self as surface_functions, SurfaceFnId};
use incan_core::lang::surface::math;
use incan_core::lang::surface::types::{self as surface_types, SurfaceTypeId};
use incan_core::lang::types::collections::CollectionTypeId;

use super::TypeChecker;

impl TypeChecker {
    /// Extract the expression from a call argument (positional or named).
    fn call_arg_expr(arg: &CallArg) -> &Spanned<Expr> {
        match arg {
            CallArg::Positional(e) | CallArg::Named(_, e) => e,
        }
    }

    /// Type-check all call arguments (positional and named).
    pub(in crate::frontend::typechecker::check_expr) fn check_call_args(&mut self, args: &[CallArg]) {
        for arg in args {
            self.check_expr(Self::call_arg_expr(arg));
        }
    }

    /// Type-check all call arguments and collect their resolved types.
    fn check_call_arg_types(&mut self, args: &[CallArg]) -> Vec<ResolvedType> {
        args.iter()
            .map(|arg| self.check_expr(Self::call_arg_expr(arg)))
            .collect()
    }

    /// Handle a known builtin call (if the callee is a builtin name).
    fn check_builtin_call(&mut self, name: &str, args: &[CallArg]) -> Option<ResolvedType> {
        // Constructors (variant-like)
        if let Some(cid) = constructors::from_str(name) {
            return match cid {
                ConstructorId::Ok | ConstructorId::Err => {
                    let arg_types = self.check_call_arg_types(args);
                    let (ok_ty, err_ty) = if cid == ConstructorId::Ok {
                        (
                            arg_types.first().cloned().unwrap_or(ResolvedType::Unknown),
                            ResolvedType::Unknown,
                        )
                    } else {
                        (
                            ResolvedType::Unknown,
                            arg_types.first().cloned().unwrap_or(ResolvedType::Unknown),
                        )
                    };
                    Some(result_ty(ok_ty, err_ty))
                }
                ConstructorId::Some => {
                    let arg_types = self.check_call_arg_types(args);
                    let inner = arg_types.first().cloned().unwrap_or(ResolvedType::Unknown);
                    Some(option_ty(inner))
                }
                ConstructorId::None => Some(option_ty(ResolvedType::Unknown)),
            };
        }

        // Core builtin functions (registry-driven)
        if let Some(bid) = builtins::from_str(name) {
            return match bid {
                BuiltinFnId::Print => {
                    self.check_call_args(args);
                    Some(ResolvedType::Unit)
                }
                BuiltinFnId::Len => {
                    self.check_call_args(args);
                    Some(ResolvedType::Int)
                }
                BuiltinFnId::Sum => {
                    self.check_call_args(args);
                    Some(ResolvedType::Int)
                }
                BuiltinFnId::Str => {
                    self.check_call_args(args);
                    Some(ResolvedType::Str)
                }
                BuiltinFnId::Int => {
                    self.check_call_args(args);
                    Some(ResolvedType::Int)
                }
                BuiltinFnId::Float => {
                    self.check_call_args(args);
                    Some(ResolvedType::Float)
                }
                BuiltinFnId::Abs => {
                    self.check_call_args(args);
                    Some(ResolvedType::Int)
                }
                BuiltinFnId::Range => {
                    self.check_call_args(args);
                    Some(list_ty(ResolvedType::Int))
                }
                BuiltinFnId::Enumerate => {
                    // enumerate(xs) -> List[(int, T)] (simple)
                    let mut inner_ty = ResolvedType::Unknown;
                    if let Some(arg) = args.first() {
                        let iter_ty = self.check_expr(Self::call_arg_expr(arg));
                        if let ResolvedType::Generic(name, type_args) = &iter_ty {
                            if (name == surface_types::as_str(SurfaceTypeId::Vec)
                                || matches!(
                                    collection_type_id(name.as_str()),
                                    Some(CollectionTypeId::List | CollectionTypeId::FrozenList)
                                ))
                                && !type_args.is_empty()
                            {
                                inner_ty = type_args[0].clone();
                            }
                        }
                    }
                    self.check_call_args(args);
                    Some(list_ty(ResolvedType::Tuple(vec![ResolvedType::Int, inner_ty])))
                }
                BuiltinFnId::Zip => {
                    // zip(a, b) -> List[(T1, T2)] (simple)
                    let mut ty1 = ResolvedType::Unknown;
                    let mut ty2 = ResolvedType::Unknown;
                    if args.len() >= 2 {
                        let iter1_ty = self.check_expr(Self::call_arg_expr(&args[0]));
                        let iter2_ty = self.check_expr(Self::call_arg_expr(&args[1]));
                        if let ResolvedType::Generic(name, type_args) = &iter1_ty {
                            if (name == surface_types::as_str(SurfaceTypeId::Vec)
                                || matches!(
                                    collection_type_id(name.as_str()),
                                    Some(CollectionTypeId::List | CollectionTypeId::FrozenList)
                                ))
                                && !type_args.is_empty()
                            {
                                ty1 = type_args[0].clone();
                            }
                        }
                        if let ResolvedType::Generic(name, type_args) = &iter2_ty {
                            if (name == surface_types::as_str(SurfaceTypeId::Vec)
                                || matches!(
                                    collection_type_id(name.as_str()),
                                    Some(CollectionTypeId::List | CollectionTypeId::FrozenList)
                                ))
                                && !type_args.is_empty()
                            {
                                ty2 = type_args[0].clone();
                            }
                        }
                    }
                    self.check_call_args(args);
                    Some(list_ty(ResolvedType::Tuple(vec![ty1, ty2])))
                }
                BuiltinFnId::ReadFile => {
                    self.check_call_args(args);
                    Some(result_ty(ResolvedType::Str, ResolvedType::Str))
                }
                BuiltinFnId::WriteFile => {
                    self.check_call_args(args);
                    Some(result_ty(ResolvedType::Unit, ResolvedType::Str))
                }
                BuiltinFnId::JsonStringify => {
                    self.check_call_args(args);
                    Some(ResolvedType::Str)
                }
                BuiltinFnId::Sleep => {
                    if let Some(arg) = args.first() {
                        let arg_expr = Self::call_arg_expr(arg);
                        let arg_ty = self.check_expr(arg_expr);
                        if !self.types_compatible(&arg_ty, &ResolvedType::Float) {
                            self.errors
                                .push(errors::type_mismatch("float", &arg_ty.to_string(), arg_expr.span));
                        }
                    }
                    Some(ResolvedType::Unit)
                }
            };
        }

        // Surface/runtime functions (registry-driven)
        if let Some(fid) = surface_functions::from_str(name) {
            return match fid {
                SurfaceFnId::SleepMs => {
                    if let Some(arg) = args.first() {
                        let arg_expr = Self::call_arg_expr(arg);
                        let arg_ty = self.check_expr(arg_expr);
                        if !self.types_compatible(&arg_ty, &ResolvedType::Int) {
                            self.errors
                                .push(errors::type_mismatch("int", &arg_ty.to_string(), arg_expr.span));
                        }
                    }
                    Some(ResolvedType::Unit)
                }
                SurfaceFnId::Timeout | SurfaceFnId::TimeoutMs | SurfaceFnId::SelectTimeout => {
                    if let Some(arg) = args.first() {
                        let arg_expr = Self::call_arg_expr(arg);
                        let arg_ty = self.check_expr(arg_expr);
                        let (expected_name, expected_ty) = if fid == SurfaceFnId::Timeout {
                            ("float", ResolvedType::Float)
                        } else {
                            ("int", ResolvedType::Int)
                        };
                        if !self.types_compatible(&arg_ty, &expected_ty) {
                            self.errors
                                .push(errors::type_mismatch(expected_name, &arg_ty.to_string(), arg_expr.span));
                        }
                    }
                    self.check_call_args(args);
                    Some(ResolvedType::Unknown)
                }
                SurfaceFnId::YieldNow => Some(ResolvedType::Unit),
                SurfaceFnId::Spawn | SurfaceFnId::SpawnBlocking => {
                    self.check_call_args(args);
                    Some(ResolvedType::Generic(
                        surface_types::as_str(SurfaceTypeId::JoinHandle).to_string(),
                        vec![ResolvedType::Unknown],
                    ))
                }
                SurfaceFnId::Channel => {
                    self.check_call_args(args);
                    let inner = ResolvedType::Unknown;
                    Some(ResolvedType::Tuple(vec![
                        ResolvedType::Generic(
                            surface_types::as_str(SurfaceTypeId::Sender).to_string(),
                            vec![inner.clone()],
                        ),
                        ResolvedType::Generic(surface_types::as_str(SurfaceTypeId::Receiver).to_string(), vec![inner]),
                    ]))
                }
                SurfaceFnId::UnboundedChannel => {
                    self.check_call_args(args);
                    Some(ResolvedType::Tuple(vec![
                        ResolvedType::Generic(
                            surface_types::as_str(SurfaceTypeId::UnboundedSender).to_string(),
                            vec![ResolvedType::Unknown],
                        ),
                        ResolvedType::Generic(
                            surface_types::as_str(SurfaceTypeId::UnboundedReceiver).to_string(),
                            vec![ResolvedType::Unknown],
                        ),
                    ]))
                }
                SurfaceFnId::Oneshot => {
                    self.check_call_args(args);
                    Some(ResolvedType::Tuple(vec![
                        ResolvedType::Generic(
                            surface_types::as_str(SurfaceTypeId::OneshotSender).to_string(),
                            vec![ResolvedType::Unknown],
                        ),
                        ResolvedType::Generic(
                            surface_types::as_str(SurfaceTypeId::OneshotReceiver).to_string(),
                            vec![ResolvedType::Unknown],
                        ),
                    ]))
                }
            };
        }

        // Surface types that behave like constructors and whose result type depends on args.
        if let Some(tid) = surface_types::from_str(name) {
            return match tid {
                SurfaceTypeId::Mutex => {
                    let inner = if let Some(arg) = args.first() {
                        self.check_expr(Self::call_arg_expr(arg))
                    } else {
                        ResolvedType::Unknown
                    };
                    Some(ResolvedType::Generic(
                        surface_types::as_str(SurfaceTypeId::Mutex).to_string(),
                        vec![inner],
                    ))
                }
                SurfaceTypeId::RwLock => {
                    let inner = if let Some(arg) = args.first() {
                        self.check_expr(Self::call_arg_expr(arg))
                    } else {
                        ResolvedType::Unknown
                    };
                    Some(ResolvedType::Generic(
                        surface_types::as_str(SurfaceTypeId::RwLock).to_string(),
                        vec![inner],
                    ))
                }
                SurfaceTypeId::Semaphore => {
                    self.check_call_args(args);
                    Some(ResolvedType::Named(
                        surface_types::as_str(SurfaceTypeId::Semaphore).to_string(),
                    ))
                }
                SurfaceTypeId::Barrier => {
                    self.check_call_args(args);
                    Some(ResolvedType::Named(
                        surface_types::as_str(SurfaceTypeId::Barrier).to_string(),
                    ))
                }
                _ => None,
            };
        }

        // Python-like type conversion helpers (surface). These are not part of `lang::builtins`.
        if let Some(cid) = collection_type_id(name) {
            return match cid {
                CollectionTypeId::Dict => {
                    let (key_ty, val_ty) = if let Some(arg) = args.first() {
                        let arg_expr = Self::call_arg_expr(arg);
                        let arg_ty = self.check_expr(arg_expr);
                        match &arg_ty {
                            ResolvedType::Generic(name, type_args)
                                if collection_type_id(name.as_str()) == Some(CollectionTypeId::Dict)
                                    && type_args.len() >= 2 =>
                            {
                                (type_args[0].clone(), type_args[1].clone())
                            }
                            _ => (ResolvedType::Unknown, ResolvedType::Unknown),
                        }
                    } else {
                        (ResolvedType::Unknown, ResolvedType::Unknown)
                    };
                    Some(dict_ty(key_ty, val_ty))
                }
                CollectionTypeId::List => {
                    let elem_ty = if let Some(arg) = args.first() {
                        let arg_expr = Self::call_arg_expr(arg);
                        let arg_ty = self.check_expr(arg_expr);
                        match &arg_ty {
                            ResolvedType::Generic(name, type_args)
                                if (name == surface_types::as_str(SurfaceTypeId::Vec)
                                    || matches!(
                                        collection_type_id(name.as_str()),
                                        Some(
                                            CollectionTypeId::List
                                                | CollectionTypeId::Set
                                                | CollectionTypeId::FrozenList
                                                | CollectionTypeId::FrozenSet
                                        )
                                    ))
                                    && !type_args.is_empty() =>
                            {
                                type_args[0].clone()
                            }
                            ResolvedType::Str => ResolvedType::Str,
                            _ => ResolvedType::Unknown,
                        }
                    } else {
                        ResolvedType::Unknown
                    };
                    Some(list_ty(elem_ty))
                }
                CollectionTypeId::Set => {
                    let elem_ty = if let Some(arg) = args.first() {
                        let arg_expr = Self::call_arg_expr(arg);
                        let arg_ty = self.check_expr(arg_expr);
                        match &arg_ty {
                            ResolvedType::Generic(name, type_args)
                                if (name == surface_types::as_str(SurfaceTypeId::Vec)
                                    || matches!(
                                        collection_type_id(name.as_str()),
                                        Some(
                                            CollectionTypeId::List
                                                | CollectionTypeId::Set
                                                | CollectionTypeId::FrozenList
                                                | CollectionTypeId::FrozenSet
                                        )
                                    ))
                                    && !type_args.is_empty() =>
                            {
                                type_args[0].clone()
                            }
                            _ => ResolvedType::Unknown,
                        }
                    } else {
                        ResolvedType::Unknown
                    };
                    Some(set_ty(elem_ty))
                }
                _ => None,
            };
        }

        None
    }

    /// Type-check a call expression and return its result type.
    pub(in crate::frontend::typechecker::check_expr) fn check_call(
        &mut self,
        callee: &Spanned<Expr>,
        args: &[CallArg],
        _span: Span,
    ) -> ResolvedType {
        // Special-case: Enum variant constructor syntax `Enum.Variant(...)`.
        // If callee is a field access where the base resolves to a known enum type
        // and the field name matches a variant, treat this as a constructor and
        // return the enum type.
        if let Expr::Field(base, variant_name) = &callee.node {
            let base_ty = self.check_expr(base);
            if let ResolvedType::Named(enum_name) = &base_ty {
                if let Some(id) = self.symbols.lookup(enum_name) {
                    if let Some(sym) = self.symbols.get(id) {
                        if let SymbolKind::Type(TypeInfo::Enum(enum_info)) = &sym.kind {
                            if enum_info.variants.iter().any(|v| v == variant_name) {
                                // Validate arguments but do not attempt strict arity/type checking here.
                                self.check_call_args(args);
                                return ResolvedType::Named(enum_name.clone());
                            }
                        }
                    }
                }
            }
        }

        // Handle math module function calls (math.sqrt, math.sin, etc.)
        if let Expr::Field(base, method) = &callee.node {
            if let Expr::Ident(module) = &base.node {
                if module == math::MATH_MODULE_NAME {
                    self.check_call_args(args);
                    if math::fn_from_str(method.as_str()).is_some() {
                        return ResolvedType::Float;
                    }
                }
            }
        }

        if let Expr::Ident(name) = &callee.node {
            if let Some(result) = self.check_builtin_call(name, args) {
                return result;
            }
        }

        let callee_ty = self.check_expr(callee);
        self.check_call_args(args);

        match callee_ty {
            ResolvedType::Function(_, ret) => *ret,
            ResolvedType::Named(name) => {
                if let Some(id) = self.symbols.lookup(&name) {
                    if let Some(sym) = self.symbols.get(id) {
                        match &sym.kind {
                            SymbolKind::Type(_) => ResolvedType::Named(name),
                            SymbolKind::Variant(info) => ResolvedType::Named(info.enum_name.clone()),
                            _ => ResolvedType::Unknown,
                        }
                    } else {
                        ResolvedType::Unknown
                    }
                } else {
                    ResolvedType::Unknown
                }
            }
            _ => ResolvedType::Unknown,
        }
    }

    /// Type-check a constructor-like call (`TypeName(...)` / `VariantName(...)`).
    pub(in crate::frontend::typechecker::check_expr) fn check_constructor(
        &mut self,
        name: &str,
        args: &[CallArg],
        span: Span,
    ) -> ResolvedType {
        self.check_call_args(args);

        if let Some(id) = self.symbols.lookup(name) {
            if let Some(sym) = self.symbols.get(id) {
                match &sym.kind {
                    SymbolKind::Type(_) => ResolvedType::Named(name.to_string()),
                    SymbolKind::Variant(info) => ResolvedType::Named(info.enum_name.clone()),
                    _ => ResolvedType::Unknown,
                }
            } else {
                ResolvedType::Unknown
            }
        } else {
            self.errors.push(errors::unknown_symbol(name, span));
            ResolvedType::Unknown
        }
    }
}
