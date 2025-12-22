//! Check comprehensions and closures.
//!
//! This module implements list/dict comprehensions and closure expressions, introducing local
//! bindings and type-checking the generated element/value expressions in a nested scope.

use crate::frontend::ast::*;
use crate::frontend::symbols::*;

use super::TypeChecker;

impl TypeChecker {
    /// Type-check a list comprehension and return `List[T]`.
    pub(in crate::frontend::typechecker::check_expr) fn check_list_comp(
        &mut self,
        comp: &ListComp,
        _span: Span,
    ) -> ResolvedType {
        let iter_ty = self.check_expr(&comp.iter);
        let elem_ty = self.infer_iterator_element_type(&iter_ty);

        self.symbols.enter_scope(ScopeKind::Block);
        self.symbols.define(Symbol {
            name: comp.var.clone(),
            kind: SymbolKind::Variable(VariableInfo {
                ty: elem_ty,
                is_mutable: false,
                is_used: false,
            }),
            span: comp.iter.span,
            scope: 0,
        });

        if let Some(filter) = &comp.filter {
            self.check_expr(filter);
        }

        let result_elem_ty = self.check_expr(&comp.expr);
        self.symbols.exit_scope();

        ResolvedType::Generic("List".to_string(), vec![result_elem_ty])
    }

    /// Type-check a dict comprehension and return `Dict[K, V]`.
    pub(in crate::frontend::typechecker::check_expr) fn check_dict_comp(
        &mut self,
        comp: &DictComp,
        _span: Span,
    ) -> ResolvedType {
        let iter_ty = self.check_expr(&comp.iter);
        let elem_ty = self.infer_iterator_element_type(&iter_ty);

        self.symbols.enter_scope(ScopeKind::Block);
        self.symbols.define(Symbol {
            name: comp.var.clone(),
            kind: SymbolKind::Variable(VariableInfo {
                ty: elem_ty,
                is_mutable: false,
                is_used: false,
            }),
            span: comp.iter.span,
            scope: 0,
        });

        if let Some(filter) = &comp.filter {
            self.check_expr(filter);
        }

        let key_ty = self.check_expr(&comp.key);
        let val_ty = self.check_expr(&comp.value);
        self.symbols.exit_scope();

        ResolvedType::Generic("Dict".to_string(), vec![key_ty, val_ty])
    }

    /// Type-check a closure expression and return a function type.
    pub(in crate::frontend::typechecker::check_expr) fn check_closure(
        &mut self,
        params: &[Spanned<Param>],
        body: &Spanned<Expr>,
        _: Span,
    ) -> ResolvedType {
        self.symbols.enter_scope(ScopeKind::Function);

        let param_types: Vec<_> = params
            .iter()
            .map(|p| {
                let ty = resolve_type(&p.node.ty.node, &self.symbols);
                self.symbols.define(Symbol {
                    name: p.node.name.clone(),
                    kind: SymbolKind::Variable(VariableInfo {
                        ty: ty.clone(),
                        is_mutable: false,
                        is_used: false,
                    }),
                    span: p.span,
                    scope: 0,
                });
                ty
            })
            .collect();

        let return_ty = self.check_expr(body);
        self.symbols.exit_scope();

        ResolvedType::Function(param_types, Box::new(return_ty))
    }
}
