//! Check `match` expressions, patterns, and exhaustiveness.
//!
//! This module validates `match` expressions by type-checking each arm, binding pattern variables,
//! and ensuring exhaustiveness for enums, `Result`, and `Option`.

use std::collections::HashSet;

use crate::frontend::ast::*;
use crate::frontend::diagnostics::errors;
use crate::frontend::symbols::*;
use incan_core::lang::surface::constructors;
use incan_core::lang::surface::constructors::ConstructorId;
use incan_core::lang::types::collections::{self, CollectionTypeId};

use super::TypeChecker;

impl TypeChecker {
    /// Type-check a `match` expression and return its resolved type.
    pub(in crate::frontend::typechecker::check_expr) fn check_match(
        &mut self,
        subject: &Spanned<Expr>,
        arms: &[Spanned<MatchArm>],
        _span: Span,
    ) -> ResolvedType {
        let subject_ty = self.check_expr(subject);

        self.check_match_exhaustiveness(&subject_ty, arms, _span);

        let mut arm_types = Vec::new();

        for arm in arms {
            self.symbols.enter_scope(ScopeKind::Block);
            self.check_pattern(&arm.node.pattern, &subject_ty);

            let arm_ty = match &arm.node.body {
                MatchBody::Expr(e) => self.check_expr(e),
                MatchBody::Block(stmts) => {
                    for stmt in stmts {
                        self.check_statement(stmt);
                    }
                    ResolvedType::Unit
                }
            };
            arm_types.push(arm_ty);

            self.symbols.exit_scope();
        }

        arm_types.first().cloned().unwrap_or(ResolvedType::Unit)
    }

    /// Type-check a pattern against an expected type, defining bindings in the current scope.
    fn check_pattern(&mut self, pattern: &Spanned<Pattern>, expected_ty: &ResolvedType) {
        match &pattern.node {
            Pattern::Wildcard => {}
            Pattern::Binding(name) => {
                self.symbols.define(Symbol {
                    name: name.clone(),
                    kind: SymbolKind::Variable(VariableInfo {
                        ty: expected_ty.clone(),
                        is_mutable: false,
                        is_used: false,
                    }),
                    span: pattern.span,
                    scope: 0,
                });
            }
            Pattern::Literal(_) => {}
            Pattern::Constructor(name, sub_patterns) => {
                if let Some(cid) = constructors::from_str(name.as_str()) {
                    match cid {
                        ConstructorId::Ok => {
                            if let ResolvedType::Generic(type_name, args) = expected_ty {
                                if type_name == collections::as_str(CollectionTypeId::Result) && !args.is_empty() {
                                    if let Some(pat) = sub_patterns.first() {
                                        self.check_pattern(pat, &args[0]);
                                    }
                                    return;
                                }
                            }
                        }
                        ConstructorId::Err => {
                            if let ResolvedType::Generic(type_name, args) = expected_ty {
                                if type_name == collections::as_str(CollectionTypeId::Result) && args.len() >= 2 {
                                    if let Some(pat) = sub_patterns.first() {
                                        self.check_pattern(pat, &args[1]);
                                    }
                                    return;
                                }
                            }
                        }
                        ConstructorId::Some => {
                            if let ResolvedType::Generic(type_name, args) = expected_ty {
                                if type_name == collections::as_str(CollectionTypeId::Option) && !args.is_empty() {
                                    if let Some(pat) = sub_patterns.first() {
                                        self.check_pattern(pat, &args[0]);
                                    }
                                    return;
                                }
                            }
                        }
                        ConstructorId::None => {
                            return;
                        }
                    }
                }

                let variant_name = if name.contains("::") {
                    name.split("::").last().unwrap_or(name)
                } else {
                    name.as_str()
                };

                let field_types: Option<Vec<ResolvedType>> = self
                    .symbols
                    .lookup(variant_name)
                    .and_then(|id| self.symbols.get(id))
                    .and_then(|sym| {
                        if let SymbolKind::Variant(info) = &sym.kind {
                            Some(info.fields.clone())
                        } else {
                            None
                        }
                    });

                if let Some(fields) = field_types {
                    for (pat, field_ty) in sub_patterns.iter().zip(fields.iter()) {
                        self.check_pattern(pat, field_ty);
                    }
                }
            }
            Pattern::Tuple(sub_patterns) => {
                if let ResolvedType::Tuple(elem_types) = expected_ty {
                    for (pat, elem_ty) in sub_patterns.iter().zip(elem_types.iter()) {
                        self.check_pattern(pat, elem_ty);
                    }
                }
            }
        }
    }

    /// Check that a match expression covers all possible cases.
    ///
    /// For enums, `Result`, and `Option`, verifies every variant is handled. Wildcards
    /// (`_`) satisfy all remaining cases. Emits a [`non_exhaustive_match`](errors::non_exhaustive_match)
    /// error if patterns are missing.
    fn check_match_exhaustiveness(&mut self, subject_ty: &ResolvedType, arms: &[Spanned<MatchArm>], span: Span) {
        let variants = if let ResolvedType::Named(name) = subject_ty {
            if let Some(id) = self.symbols.lookup(name) {
                if let Some(sym) = self.symbols.get(id) {
                    if let SymbolKind::Type(TypeInfo::Enum(enum_info)) = &sym.kind {
                        Some(enum_info.variants.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else if subject_ty.is_result() || subject_ty.is_option() {
            if subject_ty.is_result() {
                Some(vec![
                    constructors::as_str(ConstructorId::Ok).to_string(),
                    constructors::as_str(ConstructorId::Err).to_string(),
                ])
            } else {
                Some(vec![
                    constructors::as_str(ConstructorId::Some).to_string(),
                    constructors::as_str(ConstructorId::None).to_string(),
                ])
            }
        } else {
            None
        };

        if let Some(all_variants) = variants {
            let mut covered: HashSet<String> = HashSet::new();
            let mut has_wildcard = false;

            for arm in arms {
                match &arm.node.pattern.node {
                    Pattern::Wildcard | Pattern::Binding(_) => {
                        has_wildcard = true;
                    }
                    Pattern::Literal(Literal::None) if subject_ty.is_option() => {
                        covered.insert(constructors::as_str(ConstructorId::None).to_string());
                    }
                    Pattern::Constructor(name, _) => {
                        let variant_name = if name.contains("::") {
                            name.split("::").last().unwrap_or(name).to_string()
                        } else {
                            name.clone()
                        };
                        covered.insert(variant_name);
                    }
                    _ => {}
                }
            }

            if !has_wildcard {
                let missing: Vec<String> = all_variants.iter().filter(|v| !covered.contains(*v)).cloned().collect();

                if !missing.is_empty() {
                    self.errors.push(errors::non_exhaustive_match(&missing, span));
                }
            }
        }
    }
}
