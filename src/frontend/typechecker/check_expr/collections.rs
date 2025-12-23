//! Check collection literals (tuple, list, dict, and set).
//!
//! These helpers validate collection literal expressions and compute container element types using
//! the current checker’s compatibility rules.

use crate::frontend::ast::*;
use crate::frontend::symbols::ResolvedType;

use super::TypeChecker;

impl TypeChecker {
    /// Type-check a tuple literal.
    pub(in crate::frontend::typechecker::check_expr) fn check_tuple(
        &mut self,
        elems: &[Spanned<Expr>],
    ) -> ResolvedType {
        let elem_types: Vec<_> = elems.iter().map(|e| self.check_expr(e)).collect();
        ResolvedType::Tuple(elem_types)
    }

    /// Type-check a list literal.
    pub(in crate::frontend::typechecker::check_expr) fn check_list(&mut self, elems: &[Spanned<Expr>]) -> ResolvedType {
        let elem_ty = if let Some(first) = elems.first() {
            self.check_expr(first)
        } else {
            ResolvedType::Unknown
        };

        for elem in elems.iter().skip(1) {
            self.check_expr(elem);
        }

        ResolvedType::Generic("List".to_string(), vec![elem_ty])
    }

    /// Type-check a dict literal.
    pub(in crate::frontend::typechecker::check_expr) fn check_dict(
        &mut self,
        entries: &[(Spanned<Expr>, Spanned<Expr>)],
    ) -> ResolvedType {
        let (key_ty, val_ty) = if let Some((k, v)) = entries.first() {
            (self.check_expr(k), self.check_expr(v))
        } else {
            (ResolvedType::Unknown, ResolvedType::Unknown)
        };

        for (k, v) in entries.iter().skip(1) {
            self.check_expr(k);
            self.check_expr(v);
        }

        ResolvedType::Generic("Dict".to_string(), vec![key_ty, val_ty])
    }

    /// Type-check a set literal.
    pub(in crate::frontend::typechecker::check_expr) fn check_set(&mut self, elems: &[Spanned<Expr>]) -> ResolvedType {
        let elem_ty = if let Some(first) = elems.first() {
            self.check_expr(first)
        } else {
            ResolvedType::Unknown
        };

        for elem in elems.iter().skip(1) {
            self.check_expr(elem);
        }

        ResolvedType::Generic("Set".to_string(), vec![elem_ty])
    }
}
