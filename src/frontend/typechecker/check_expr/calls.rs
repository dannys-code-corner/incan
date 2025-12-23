//! Check calls, constructors, and builtins.
//!
//! This module handles the main call-expression logic (`foo(...)`), including special-cased
//! builtins like `Ok(...)`/`Err(...)` and runtime helpers like `sleep(...)`. It also provides
//! small utilities to type-check call argument lists consistently.

use crate::frontend::ast::*;
use crate::frontend::diagnostics::errors;
use crate::frontend::symbols::*;

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
        match name {
            "Ok" | "Err" => {
                let arg_types = self.check_call_arg_types(args);
                let (ok_ty, err_ty) = if name == "Ok" {
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
                Some(ResolvedType::Generic("Result".to_string(), vec![ok_ty, err_ty]))
            }
            "Some" => {
                let arg_types = self.check_call_arg_types(args);
                let inner = arg_types.first().cloned().unwrap_or(ResolvedType::Unknown);
                Some(ResolvedType::Generic("Option".to_string(), vec![inner]))
            }

            // I/O / utility
            "println" | "print" => {
                self.check_call_args(args);
                Some(ResolvedType::Unit)
            }
            "len" => {
                self.check_call_args(args);
                Some(ResolvedType::Int)
            }
            "sum" => {
                self.check_call_args(args);
                Some(ResolvedType::Int)
            }
            "range" => {
                self.check_call_args(args);
                Some(ResolvedType::Generic("List".to_string(), vec![ResolvedType::Int]))
            }

            // Time / async-ish helpers
            "sleep" => {
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
            "sleep_ms" => {
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
            "timeout" => {
                if let Some(arg) = args.first() {
                    let arg_expr = Self::call_arg_expr(arg);
                    let arg_ty = self.check_expr(arg_expr);
                    if !self.types_compatible(&arg_ty, &ResolvedType::Float) {
                        self.errors
                            .push(errors::type_mismatch("float", &arg_ty.to_string(), arg_expr.span));
                    }
                }
                if args.len() >= 2 {
                    let task_expr = Self::call_arg_expr(&args[1]);
                    self.check_expr(task_expr);
                }
                Some(ResolvedType::Unknown)
            }
            "timeout_ms" => {
                if let Some(arg) = args.first() {
                    let arg_expr = Self::call_arg_expr(arg);
                    let arg_ty = self.check_expr(arg_expr);
                    if !self.types_compatible(&arg_ty, &ResolvedType::Int) {
                        self.errors
                            .push(errors::type_mismatch("int", &arg_ty.to_string(), arg_expr.span));
                    }
                }
                if args.len() >= 2 {
                    let task_expr = Self::call_arg_expr(&args[1]);
                    self.check_expr(task_expr);
                }
                Some(ResolvedType::Unknown)
            }

            // Python-like type conversion builtins
            "dict" => {
                let (key_ty, val_ty) = if let Some(arg) = args.first() {
                    let arg_expr = Self::call_arg_expr(arg);
                    let arg_ty = self.check_expr(arg_expr);
                    match &arg_ty {
                        ResolvedType::Generic(name, type_args) if name == "Dict" && type_args.len() >= 2 => {
                            (type_args[0].clone(), type_args[1].clone())
                        }
                        _ => (ResolvedType::Unknown, ResolvedType::Unknown),
                    }
                } else {
                    (ResolvedType::Unknown, ResolvedType::Unknown)
                };
                Some(ResolvedType::Generic("Dict".to_string(), vec![key_ty, val_ty]))
            }
            "list" => {
                let elem_ty = if let Some(arg) = args.first() {
                    let arg_expr = Self::call_arg_expr(arg);
                    let arg_ty = self.check_expr(arg_expr);
                    match &arg_ty {
                        ResolvedType::Generic(name, type_args)
                            if (name == "List" || name == "Vec" || name == "Set") && !type_args.is_empty() =>
                        {
                            type_args[0].clone()
                        }
                        ResolvedType::Str => ResolvedType::Str,
                        _ => ResolvedType::Unknown,
                    }
                } else {
                    ResolvedType::Unknown
                };
                Some(ResolvedType::Generic("List".to_string(), vec![elem_ty]))
            }
            "set" => {
                let elem_ty = if let Some(arg) = args.first() {
                    let arg_expr = Self::call_arg_expr(arg);
                    let arg_ty = self.check_expr(arg_expr);
                    match &arg_ty {
                        ResolvedType::Generic(name, type_args)
                            if (name == "List" || name == "Vec" || name == "Set") && !type_args.is_empty() =>
                        {
                            type_args[0].clone()
                        }
                        _ => ResolvedType::Unknown,
                    }
                } else {
                    ResolvedType::Unknown
                };
                Some(ResolvedType::Generic("Set".to_string(), vec![elem_ty]))
            }
            "enumerate" => {
                let mut inner_ty = ResolvedType::Unknown;
                if let Some(arg) = args.first() {
                    let arg_expr = Self::call_arg_expr(arg);
                    let iter_ty = self.check_expr(arg_expr);
                    if let ResolvedType::Generic(name, type_args) = &iter_ty {
                        if (name == "List" || name == "Vec") && !type_args.is_empty() {
                            inner_ty = type_args[0].clone();
                        }
                    }
                }
                Some(ResolvedType::Generic(
                    "List".to_string(),
                    vec![ResolvedType::Tuple(vec![ResolvedType::Int, inner_ty])],
                ))
            }
            "zip" => {
                let mut ty1 = ResolvedType::Unknown;
                let mut ty2 = ResolvedType::Unknown;

                if args.len() >= 2 {
                    let arg1 = Self::call_arg_expr(&args[0]);
                    let arg2 = Self::call_arg_expr(&args[1]);

                    let iter1_ty = self.check_expr(arg1);
                    let iter2_ty = self.check_expr(arg2);

                    if let ResolvedType::Generic(name, type_args) = &iter1_ty {
                        if (name == "List" || name == "Vec") && !type_args.is_empty() {
                            ty1 = type_args[0].clone();
                        }
                    }
                    if let ResolvedType::Generic(name, type_args) = &iter2_ty {
                        if (name == "List" || name == "Vec") && !type_args.is_empty() {
                            ty2 = type_args[0].clone();
                        }
                    }
                }

                Some(ResolvedType::Generic(
                    "List".to_string(),
                    vec![ResolvedType::Tuple(vec![ty1, ty2])],
                ))
            }

            // File I/O functions
            "read_file" => Some(ResolvedType::Generic(
                "Result".to_string(),
                vec![ResolvedType::Str, ResolvedType::Str],
            )),
            "write_file" => Some(ResolvedType::Generic(
                "Result".to_string(),
                vec![ResolvedType::Unit, ResolvedType::Str],
            )),

            // Type conversion functions
            "int" => {
                self.check_call_args(args);
                Some(ResolvedType::Int)
            }
            "str" => {
                self.check_call_args(args);
                Some(ResolvedType::Str)
            }
            "float" => {
                self.check_call_args(args);
                Some(ResolvedType::Float)
            }

            // JSON helpers
            "json_stringify" => {
                self.check_call_args(args);
                Some(ResolvedType::Str)
            }
            "json_parse" => {
                self.check_call_args(args);
                Some(ResolvedType::Generic(
                    "Result".to_string(),
                    vec![ResolvedType::Unknown, ResolvedType::Str],
                ))
            }

            // Async primitives
            "spawn" => {
                self.check_call_args(args);
                Some(ResolvedType::Generic(
                    "JoinHandle".to_string(),
                    vec![ResolvedType::Unknown],
                ))
            }
            "channel" => {
                self.check_call_args(args);
                let inner = ResolvedType::Unknown;
                Some(ResolvedType::Tuple(vec![
                    ResolvedType::Generic("Sender".to_string(), vec![inner.clone()]),
                    ResolvedType::Generic("Receiver".to_string(), vec![inner]),
                ]))
            }
            "unbounded_channel" => Some(ResolvedType::Tuple(vec![
                ResolvedType::Generic("UnboundedSender".to_string(), vec![ResolvedType::Unknown]),
                ResolvedType::Generic("UnboundedReceiver".to_string(), vec![ResolvedType::Unknown]),
            ])),
            "oneshot" => Some(ResolvedType::Tuple(vec![
                ResolvedType::Generic("OneshotSender".to_string(), vec![ResolvedType::Unknown]),
                ResolvedType::Generic("OneshotReceiver".to_string(), vec![ResolvedType::Unknown]),
            ])),
            "Mutex" => {
                let inner = if let Some(arg) = args.first() {
                    self.check_expr(Self::call_arg_expr(arg))
                } else {
                    ResolvedType::Unknown
                };
                Some(ResolvedType::Generic("Mutex".to_string(), vec![inner]))
            }
            "RwLock" => {
                let inner = if let Some(arg) = args.first() {
                    self.check_expr(Self::call_arg_expr(arg))
                } else {
                    ResolvedType::Unknown
                };
                Some(ResolvedType::Generic("RwLock".to_string(), vec![inner]))
            }
            "Semaphore" => {
                self.check_call_args(args);
                Some(ResolvedType::Named("Semaphore".to_string()))
            }
            "Barrier" => {
                self.check_call_args(args);
                Some(ResolvedType::Named("Barrier".to_string()))
            }
            "yield_now" => Some(ResolvedType::Unit),

            // Builtins not handled here
            _ => None,
        }
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
                if module == "math" {
                    self.check_call_args(args);
                    match method.as_str() {
                        "sqrt" | "sin" | "cos" | "tan" | "abs" | "floor" | "ceil" | "pow" | "log" | "log10" | "exp"
                        | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" => return ResolvedType::Float,
                        _ => {}
                    }
                }
            }
        }

        if let Expr::Ident(name) = &callee.node {
            if let Some(result) = self.check_builtin_call(name, args) {
                // Preserve prior behavior: some builtins did *not* check args.
                // For those that should check args, `check_builtin_call` does it itself.
                if matches!(name.as_str(), "read_file" | "write_file") {
                    self.check_call_args(args);
                }
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
