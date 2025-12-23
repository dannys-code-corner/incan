//! Const-evaluation / const-validation for RFC 008.
//!
//! This module does not compute runtime values; it validates that an initializer is const-evaluable,
//! determines its type, classifies it (Rust-native vs frozen), and detects const dependency cycles.

use crate::frontend::ast::*;
use crate::frontend::diagnostics::{CompileError, errors};
use crate::frontend::symbols::{ResolvedType, resolve_type};

use super::TypeChecker;

/// Const category used by RFC 008.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstKind {
    /// Can be emitted as a Rust `const` directly.
    RustNative,
    /// Needs frozen stdlib wrappers (deep immutability / baked static data).
    Frozen,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConstEvalResult {
    pub ty: ResolvedType,
    pub kind: ConstKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstEvalState {
    NotStarted,
    InProgress,
    Done,
}

impl TypeChecker {
    /// Convert a user-written type annotation in a `const` declaration to its frozen form.
    ///
    /// This makes `const X: List[T] = [...]` behave as `const X: FrozenList[T] = [...]`, ensuring
    /// the resulting constant has a deeply immutable type (no mutating APIs).
    fn freeze_const_annotation(&self, ty: ResolvedType) -> ResolvedType {
        match ty {
            ResolvedType::Str => ResolvedType::Named("FrozenStr".to_string()),
            ResolvedType::Generic(name, args) => match name.as_str() {
                "List" => ResolvedType::Generic("FrozenList".to_string(), args),
                "Dict" => ResolvedType::Generic("FrozenDict".to_string(), args),
                "Set" => ResolvedType::Generic("FrozenSet".to_string(), args),
                // Keep tuples as tuples; their elements may still be frozen.
                "Tuple" => ResolvedType::Generic("Tuple".to_string(), args),
                _ => ResolvedType::Generic(name, args),
            },
            other => other,
        }
    }

    pub(crate) fn check_and_resolve_const(&mut self, konst: &ConstDecl, decl_span: Span) {
        // Evaluate (with cycle detection) and update the symbol table entry.
        let mut stack = Vec::new();
        let Some(result) = self.eval_const_by_name(&konst.name, &mut stack) else {
            return;
        };

        // Publish classification for downstream stages.
        self.type_info.const_kinds.insert(konst.name.clone(), result.kind);
        // Record the root initializer type so lowering/codegen can use it.
        self.record_expr_type(konst.value.span, result.ty.clone());

        // If an annotation exists, require compatibility.
        if let Some(ann) = &konst.ty {
            let expected = self.freeze_const_annotation(resolve_type(&ann.node, &self.symbols));
            if !self.types_compatible(&result.ty, &expected) {
                self.errors.push(errors::type_mismatch(
                    &expected.to_string(),
                    &result.ty.to_string(),
                    konst.value.span,
                ));
            }
        } else if matches!(result.ty, ResolvedType::Unknown) {
            self.errors.push(CompileError::type_error(
                format!(
                    "Cannot infer type for const '{}'; add an explicit type annotation",
                    konst.name
                ),
                decl_span,
            ));
        }

        // Update the symbol table type (so later expressions see the refined type).
        if let Some(id) = self.symbols.lookup_local(&konst.name) {
            if let Some(sym) = self.symbols.get_mut(id) {
                if let crate::frontend::symbols::SymbolKind::Variable(var_info) = &mut sym.kind {
                    var_info.ty = result.ty.clone();
                }
            }
        }
    }

    fn eval_const_by_name(&mut self, name: &str, stack: &mut Vec<String>) -> Option<ConstEvalResult> {
        if let Some(res) = self.const_eval_cache.get(name).cloned() {
            return Some(res);
        }

        let state = self
            .const_eval_state
            .get(name)
            .copied()
            .unwrap_or(ConstEvalState::NotStarted);
        match state {
            ConstEvalState::Done => return self.const_eval_cache.get(name).cloned(),
            ConstEvalState::InProgress => {
                // Cycle: stack + name
                let mut cycle = stack.clone();
                cycle.push(name.to_string());
                let cycle_str = cycle.join(" -> ");
                let span = self.const_decls.get(name).map(|(_, s)| *s).unwrap_or_default();
                self.errors.push(CompileError::type_error(
                    format!("Const dependency cycle detected: {}", cycle_str),
                    span,
                ));
                return None;
            }
            ConstEvalState::NotStarted => {}
        }

        let Some((decl, decl_span)) = self.const_decls.get(name).cloned() else {
            self.errors.push(errors::unknown_symbol(name, Span::default()));
            return None;
        };

        self.const_eval_state
            .insert(name.to_string(), ConstEvalState::InProgress);
        stack.push(name.to_string());

        let expected = decl.ty.as_ref().map(|t| resolve_type(&t.node, &self.symbols));
        let expected = expected.map(|t| self.freeze_const_annotation(t));
        let result = self.eval_const_expr(&decl.value, expected.as_ref(), stack, decl_span);

        stack.pop();
        self.const_eval_state.insert(name.to_string(), ConstEvalState::Done);

        if let Some(res) = &result {
            self.const_eval_cache.insert(name.to_string(), res.clone());
        }

        result
    }

    fn eval_const_expr(
        &mut self,
        expr: &Spanned<Expr>,
        expected: Option<&ResolvedType>,
        stack: &mut Vec<String>,
        decl_span: Span,
    ) -> Option<ConstEvalResult> {
        match &expr.node {
            Expr::Literal(lit) => Some(self.eval_const_literal(lit, expected, expr.span, decl_span)),
            Expr::Ident(name) => {
                // Only other consts are allowed in const initializers.
                if !self.const_decls.contains_key(name) {
                    self.errors.push(CompileError::type_error(
                        format!("Non-const name '{}' is not allowed in a const initializer", name),
                        expr.span,
                    ));
                    return None;
                }
                self.eval_const_by_name(name, stack)
            }
            Expr::Tuple(items) => {
                let mut tys = Vec::with_capacity(items.len());
                let mut kind = ConstKind::RustNative;
                for item in items {
                    let r = self.eval_const_expr(item, None, stack, decl_span)?;
                    tys.push(r.ty);
                    if r.kind == ConstKind::Frozen {
                        kind = ConstKind::Frozen;
                    }
                }
                Some(ConstEvalResult {
                    ty: ResolvedType::Tuple(tys),
                    kind,
                })
            }
            Expr::Unary(op, inner) => {
                let r = self.eval_const_expr(inner, None, stack, decl_span)?;
                match op {
                    UnaryOp::Neg => {
                        if matches!(r.ty, ResolvedType::Int | ResolvedType::Float) {
                            Some(ConstEvalResult { ty: r.ty, kind: r.kind })
                        } else {
                            self.errors.push(CompileError::type_error(
                                format!("Unary '-' is not supported for type '{}'", r.ty),
                                expr.span,
                            ));
                            None
                        }
                    }
                    UnaryOp::Not => {
                        if matches!(r.ty, ResolvedType::Bool) {
                            Some(ConstEvalResult {
                                ty: ResolvedType::Bool,
                                kind: r.kind,
                            })
                        } else {
                            self.errors.push(CompileError::type_error(
                                format!("Unary 'not' is not supported for type '{}'", r.ty),
                                expr.span,
                            ));
                            None
                        }
                    }
                }
            }
            Expr::Binary(left, op, right) => {
                let l = self.eval_const_expr(left, None, stack, decl_span)?;
                let r = self.eval_const_expr(right, None, stack, decl_span)?;

                let (result_ty, result_kind) = match op {
                    // Numeric ops
                    BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Pow => {
                        // Special-case string concatenation for frozen strings
                        if matches!(op, BinaryOp::Add)
                            && matches!(l.ty, ResolvedType::Named(ref n) if n == "FrozenStr")
                            && matches!(r.ty, ResolvedType::Named(ref n) if n == "FrozenStr")
                        {
                            (ResolvedType::Named("FrozenStr".to_string()), ConstKind::Frozen)
                        } else if matches!(l.ty, ResolvedType::Int | ResolvedType::Float)
                            && self.types_compatible(&r.ty, &l.ty)
                        {
                            (l.ty.clone(), ConstKind::RustNative)
                        } else {
                            self.errors.push(CompileError::type_error(
                                format!(
                                    "Binary operator '{}' is not supported for types '{}' and '{}'",
                                    op, l.ty, r.ty
                                ),
                                expr.span,
                            ));
                            return None;
                        }
                    }
                    // Comparisons always yield bool
                    BinaryOp::Eq | BinaryOp::NotEq | BinaryOp::Lt | BinaryOp::Gt | BinaryOp::LtEq | BinaryOp::GtEq => {
                        (ResolvedType::Bool, ConstKind::RustNative)
                    }
                    BinaryOp::And | BinaryOp::Or => {
                        if matches!(l.ty, ResolvedType::Bool) && matches!(r.ty, ResolvedType::Bool) {
                            (ResolvedType::Bool, ConstKind::RustNative)
                        } else {
                            self.errors.push(CompileError::type_error(
                                format!(
                                    "Logical operator '{}' requires bool operands (got '{}' and '{}')",
                                    op, l.ty, r.ty
                                ),
                                expr.span,
                            ));
                            return None;
                        }
                    }
                    BinaryOp::In | BinaryOp::NotIn | BinaryOp::Is => {
                        self.errors.push(CompileError::type_error(
                            format!("Operator '{}' is not allowed inside const initializers (phase 1)", op),
                            expr.span,
                        ));
                        return None;
                    }
                };

                Some(ConstEvalResult {
                    ty: result_ty,
                    kind: result_kind,
                })
            }
            Expr::List(items) => {
                let elem_expected = expected.and_then(|t| match t {
                    ResolvedType::Generic(name, args) if name == "FrozenList" && !args.is_empty() => Some(&args[0]),
                    _ => None,
                });

                let elem_ty = if items.is_empty() {
                    elem_expected.cloned().unwrap_or(ResolvedType::Unknown)
                } else {
                    let first = self.eval_const_expr(&items[0], elem_expected, stack, decl_span)?;
                    // Evaluate the rest just for validation.
                    for it in items.iter().skip(1) {
                        self.eval_const_expr(it, elem_expected, stack, decl_span)?;
                    }
                    first.ty
                };

                if items.is_empty() && matches!(elem_ty, ResolvedType::Unknown) {
                    self.errors.push(CompileError::type_error(
                        "Cannot infer type for empty const list; annotate as FrozenList[T]".to_string(),
                        expr.span,
                    ));
                }

                Some(ConstEvalResult {
                    ty: ResolvedType::Generic("FrozenList".to_string(), vec![elem_ty]),
                    kind: ConstKind::Frozen,
                })
            }
            Expr::Set(items) => {
                let elem_expected = expected.and_then(|t| match t {
                    ResolvedType::Generic(name, args) if name == "FrozenSet" && !args.is_empty() => Some(&args[0]),
                    _ => None,
                });

                let elem_ty = if items.is_empty() {
                    elem_expected.cloned().unwrap_or(ResolvedType::Unknown)
                } else {
                    let first = self.eval_const_expr(&items[0], elem_expected, stack, decl_span)?;
                    for it in items.iter().skip(1) {
                        self.eval_const_expr(it, elem_expected, stack, decl_span)?;
                    }
                    first.ty
                };

                if items.is_empty() && matches!(elem_ty, ResolvedType::Unknown) {
                    self.errors.push(CompileError::type_error(
                        "Cannot infer type for empty const set; annotate as FrozenSet[T]".to_string(),
                        expr.span,
                    ));
                }

                Some(ConstEvalResult {
                    ty: ResolvedType::Generic("FrozenSet".to_string(), vec![elem_ty]),
                    kind: ConstKind::Frozen,
                })
            }
            Expr::Dict(pairs) => {
                let (k_expected, v_expected) = match expected {
                    Some(ResolvedType::Generic(name, args)) if name == "FrozenDict" && args.len() >= 2 => {
                        (Some(&args[0]), Some(&args[1]))
                    }
                    _ => (None, None),
                };

                let (key_ty, val_ty) = if pairs.is_empty() {
                    (
                        k_expected.cloned().unwrap_or(ResolvedType::Unknown),
                        v_expected.cloned().unwrap_or(ResolvedType::Unknown),
                    )
                } else {
                    let (k0, v0) = &pairs[0];
                    let kk = self.eval_const_expr(k0, k_expected, stack, decl_span)?;
                    let vv = self.eval_const_expr(v0, v_expected, stack, decl_span)?;
                    for (k, v) in pairs.iter().skip(1) {
                        self.eval_const_expr(k, k_expected, stack, decl_span)?;
                        self.eval_const_expr(v, v_expected, stack, decl_span)?;
                    }
                    (kk.ty, vv.ty)
                };

                if pairs.is_empty()
                    && (matches!(key_ty, ResolvedType::Unknown) || matches!(val_ty, ResolvedType::Unknown))
                {
                    self.errors.push(CompileError::type_error(
                        "Cannot infer type for empty const dict; annotate as FrozenDict[K, V]".to_string(),
                        expr.span,
                    ));
                }

                Some(ConstEvalResult {
                    ty: ResolvedType::Generic("FrozenDict".to_string(), vec![key_ty, val_ty]),
                    kind: ConstKind::Frozen,
                })
            }

            // Disallowed constructs for RFC 008 phase 1.
            Expr::Call(_, _)
            | Expr::MethodCall(_, _, _)
            | Expr::ListComp(_)
            | Expr::DictComp(_)
            | Expr::Await(_)
            | Expr::Match(_, _)
            | Expr::If(_)
            | Expr::Closure(_, _)
            | Expr::Yield(_)
            | Expr::Range { .. }
            | Expr::Index(_, _)
            | Expr::Slice(_, _)
            | Expr::Field(_, _)
            | Expr::Try(_)
            | Expr::Paren(_)
            | Expr::Constructor(_, _)
            | Expr::FString(_) => {
                self.errors.push(CompileError::type_error(
                    "Expression is not allowed inside const initializers (phase 1)".to_string(),
                    expr.span,
                ));
                None
            }
            Expr::SelfExpr => {
                self.errors.push(CompileError::type_error(
                    "self is not allowed inside const initializers".to_string(),
                    expr.span,
                ));
                None
            }
        }
    }

    fn eval_const_literal(
        &mut self,
        lit: &Literal,
        expected: Option<&ResolvedType>,
        span: Span,
        _decl_span: Span,
    ) -> ConstEvalResult {
        match lit {
            Literal::Int(_) => ConstEvalResult {
                ty: ResolvedType::Int,
                kind: ConstKind::RustNative,
            },
            Literal::Float(_) => ConstEvalResult {
                ty: ResolvedType::Float,
                kind: ConstKind::RustNative,
            },
            Literal::Bool(_) => ConstEvalResult {
                ty: ResolvedType::Bool,
                kind: ConstKind::RustNative,
            },
            Literal::String(_) => ConstEvalResult {
                ty: ResolvedType::Named("FrozenStr".to_string()),
                kind: ConstKind::Frozen,
            },
            Literal::Bytes(_) => ConstEvalResult {
                ty: ResolvedType::Named("FrozenBytes".to_string()),
                kind: ConstKind::Frozen,
            },
            Literal::None => {
                // None is ambiguous without annotation.
                let ty = expected.cloned().unwrap_or(ResolvedType::Unknown);
                if matches!(ty, ResolvedType::Unknown) {
                    self.errors.push(CompileError::type_error(
                        "Cannot infer type for None in const initializer; add an explicit type annotation".to_string(),
                        span,
                    ));
                }
                ConstEvalResult {
                    ty,
                    kind: ConstKind::RustNative,
                }
            }
        }
    }
}
