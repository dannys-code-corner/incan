//! Check basic expressions (identifiers, literals, and `self`).
//!
//! These helpers implement the low-level building blocks used throughout expression checking:
//! name resolution against the [`SymbolTable`], literal typing, and resolving `self` inside methods.

use crate::frontend::ast::*;
use crate::frontend::diagnostics::errors;
use crate::frontend::symbols::*;

use super::TypeChecker;

impl TypeChecker {
    /// Resolve an identifier to its type.
    pub(in crate::frontend::typechecker::check_expr) fn check_ident(
        &mut self,
        name: &str,
        span: Span,
    ) -> ResolvedType {
        // Note: `math` module requires `import math` (like Python).
        // When imported, it's registered as a Module symbol and found via normal lookup.

        if let Some(id) = self.symbols.lookup(name) {
            if let Some(sym) = self.symbols.get(id) {
                match &sym.kind {
                    SymbolKind::Variable(info) => info.ty.clone(),
                    SymbolKind::Function(info) => ResolvedType::Function(
                        info.params.iter().map(|(_, ty)| ty.clone()).collect(),
                        Box::new(info.return_type.clone()),
                    ),
                    SymbolKind::Type(_) => ResolvedType::Named(name.to_string()),
                    SymbolKind::Variant(info) => {
                        // Return the enum type
                        ResolvedType::Named(info.enum_name.clone())
                    }
                    SymbolKind::Field(info) => info.ty.clone(),
                    SymbolKind::Module(_) => ResolvedType::Named(name.to_string()),
                    SymbolKind::Trait(_) => ResolvedType::Named(name.to_string()),
                    SymbolKind::RustModule { .. } => ResolvedType::Named(name.to_string()),
                }
            } else {
                ResolvedType::Unknown
            }
        } else {
            self.errors.push(errors::unknown_symbol(name, span));
            ResolvedType::Unknown
        }
    }

    /// Resolve a literal value to its type.
    pub(in crate::frontend::typechecker::check_expr) fn check_literal(
        &self,
        lit: &Literal,
    ) -> ResolvedType {
        match lit {
            Literal::Int(_) => ResolvedType::Int,
            Literal::Float(_) => ResolvedType::Float,
            Literal::String(_) => ResolvedType::Str,
            Literal::Bytes(_) => ResolvedType::Bytes,
            Literal::Bool(_) => ResolvedType::Bool,
            Literal::None => {
                ResolvedType::Generic("Option".to_string(), vec![ResolvedType::Unknown])
            }
        }
    }

    /// Resolve the `self` expression inside a method body.
    pub(in crate::frontend::typechecker::check_expr) fn check_self(
        &mut self,
        span: Span,
    ) -> ResolvedType {
        if let Some(id) = self.symbols.lookup("self") {
            if let Some(sym) = self.symbols.get(id) {
                if let SymbolKind::Variable(info) = &sym.kind {
                    return info.ty.clone();
                }
            }
        }
        self.errors.push(errors::unknown_symbol("self", span));
        ResolvedType::Unknown
    }
}
