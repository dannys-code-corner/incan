//! Second-pass declaration checking: validate models, classes, traits, enums, functions, methods.

use crate::frontend::ast::*;
use crate::frontend::diagnostics::errors;
use crate::frontend::symbols::*;

use super::TypeChecker;

impl TypeChecker {
    // ========================================================================
    // Second pass: check declarations
    // ========================================================================

    /// Validate a declaration's body and semantics (second pass).
    ///
    /// Dispatches to `check_model`, `check_class`, etc. Expects symbols to
    /// already be registered via [`collect_declaration`](Self::collect_declaration).
    pub(crate) fn check_declaration(&mut self, decl: &Spanned<Declaration>) {
        match &decl.node {
            Declaration::Import(_) => {} // Already handled
            Declaration::Const(konst) => self.check_const(konst, decl.span),
            Declaration::Model(model) => self.check_model(model),
            Declaration::Class(class) => self.check_class(class),
            Declaration::Trait(tr) => self.check_trait(tr),
            Declaration::Newtype(nt) => self.check_newtype(nt),
            Declaration::Enum(en) => self.check_enum(en),
            Declaration::Function(func) => self.check_function(func),
            Declaration::Docstring(_) => {} // Docstrings don't need checking
        }
    }

    fn check_const(&mut self, konst: &ConstDecl, span: Span) {
        // RFC 008: const-eval (with cycle detection + category classification).
        self.check_and_resolve_const(konst, span);
    }

    fn check_model(&mut self, model: &ModelDecl) {
        self.symbols.enter_scope(ScopeKind::Model);

        // Validate @derive decorators
        self.validate_derives(&model.decorators);

        // Define type parameters
        for param in &model.type_params {
            self.symbols.define(Symbol {
                name: param.clone(),
                kind: SymbolKind::Type(TypeInfo::Builtin), // Type var placeholder
                span: Span::default(),
                scope: 0,
            });
        }

        // Define fields in scope
        for field in &model.fields {
            let ty = resolve_type(&field.node.ty.node, &self.symbols);
            self.symbols.define(Symbol {
                name: field.node.name.clone(),
                kind: SymbolKind::Field(FieldInfo {
                    ty,
                    has_default: field.node.default.is_some(),
                }),
                span: field.span,
                scope: 0,
            });

            // Check default expression type
            if let Some(default) = &field.node.default {
                let default_ty = self.check_expr(default);
                let field_ty = resolve_type(&field.node.ty.node, &self.symbols);
                if !self.types_compatible(&default_ty, &field_ty) {
                    self.errors.push(errors::type_mismatch(
                        &field_ty.to_string(),
                        &default_ty.to_string(),
                        default.span,
                    ));
                }
            }
        }

        // Check methods
        for method in &model.methods {
            self.check_method(&method.node, &model.name);
        }

        self.symbols.exit_scope();
    }

    fn check_class(&mut self, class: &ClassDecl) {
        self.symbols.enter_scope(ScopeKind::Class);

        // Validate @derive decorators
        self.validate_derives(&class.decorators);

        // Check base class exists
        if let Some(base) = &class.extends {
            if self.symbols.lookup(base).is_none() {
                self.errors.push(errors::unknown_symbol(base, Span::default()));
            }
        }

        // Check traits exist and are satisfied
        for trait_name in &class.traits {
            if let Some(id) = self.symbols.lookup(trait_name) {
                if let Some(sym) = self.symbols.get(id) {
                    if let SymbolKind::Trait(trait_info) = &sym.kind {
                        self.check_trait_conformance(class, trait_info.clone(), trait_name);
                    }
                }
            } else {
                self.errors.push(errors::unknown_symbol(trait_name, Span::default()));
            }
        }

        // Define fields
        for field in &class.fields {
            let ty = resolve_type(&field.node.ty.node, &self.symbols);
            self.symbols.define(Symbol {
                name: field.node.name.clone(),
                kind: SymbolKind::Field(FieldInfo {
                    ty,
                    has_default: field.node.default.is_some(),
                }),
                span: field.span,
                scope: 0,
            });

            if let Some(default) = &field.node.default {
                let default_ty = self.check_expr(default);
                let field_ty = resolve_type(&field.node.ty.node, &self.symbols);
                if !self.types_compatible(&default_ty, &field_ty) {
                    self.errors.push(errors::type_mismatch(
                        &field_ty.to_string(),
                        &default_ty.to_string(),
                        default.span,
                    ));
                }
            }
        }

        // Check methods
        for method in &class.methods {
            self.check_method(&method.node, &class.name);
        }

        self.symbols.exit_scope();
    }

    fn check_trait_conformance(&mut self, class: &ClassDecl, trait_info: TraitInfo, trait_name: &str) {
        // Check required fields
        for (field_name, _field_ty) in &trait_info.requires {
            let found = class.fields.iter().any(|f| &f.node.name == field_name);
            if !found {
                self.errors
                    .push(errors::missing_field(&class.name, field_name, Span::default()));
            }
        }

        // Check required methods (those without body)
        for (method_name, method_info) in &trait_info.methods {
            if !method_info.has_body {
                let found = class.methods.iter().any(|m| &m.node.name == method_name);
                if !found {
                    self.errors
                        .push(errors::missing_trait_method(trait_name, method_name, Span::default()));
                }
            }
        }
    }

    fn check_trait(&mut self, tr: &TraitDecl) {
        self.symbols.enter_scope(ScopeKind::Trait);

        for method in &tr.methods {
            if method.node.body.is_some() {
                self.check_method(&method.node, &tr.name);
            }
        }

        self.symbols.exit_scope();
    }

    fn check_newtype(&mut self, nt: &NewtypeDecl) {
        // Check underlying type exists
        let underlying = resolve_type(&nt.underlying.node, &self.symbols);
        if matches!(underlying, ResolvedType::Unknown) {
            self.errors.push(errors::unknown_symbol(
                &format!("{:?}", nt.underlying.node),
                nt.underlying.span,
            ));
        }

        // Check methods
        for method in &nt.methods {
            self.symbols.enter_scope(ScopeKind::Method {
                receiver: method.node.receiver,
            });

            // Define self
            self.symbols.define(Symbol {
                name: "self".to_string(),
                kind: SymbolKind::Variable(VariableInfo {
                    ty: ResolvedType::Named(nt.name.clone()),
                    is_mutable: matches!(method.node.receiver, Some(Receiver::Mutable)),
                    is_used: true,
                }),
                span: Span::default(),
                scope: 0,
            });

            // Check body
            if let Some(body) = &method.node.body {
                let return_type = resolve_type(&method.node.return_type.node, &self.symbols);
                self.symbols.set_return_type(return_type.clone());

                // Set error type for ? checking
                self.current_return_error_type = return_type.result_err_type().cloned();

                for stmt in body {
                    self.check_statement(stmt);
                }

                self.current_return_error_type = None;
            }

            self.symbols.exit_scope();
        }
    }

    fn check_enum(&mut self, en: &EnumDecl) {
        // Check variant field types exist
        for variant in &en.variants {
            for field_ty in &variant.node.fields {
                let resolved = resolve_type(&field_ty.node, &self.symbols);
                if matches!(resolved, ResolvedType::Unknown) {
                    self.errors
                        .push(errors::unknown_symbol(&format!("{:?}", field_ty.node), field_ty.span));
                }
            }
        }
    }

    fn check_function(&mut self, func: &FunctionDecl) {
        self.symbols.enter_scope(ScopeKind::Function);

        // Define parameters
        for param in &func.params {
            let ty = resolve_type(&param.node.ty.node, &self.symbols);
            self.symbols.define(Symbol {
                name: param.node.name.clone(),
                kind: SymbolKind::Variable(VariableInfo {
                    ty,
                    is_mutable: false,
                    is_used: false,
                }),
                span: param.span,
                scope: 0,
            });
        }

        let return_type = resolve_type(&func.return_type.node, &self.symbols);
        self.symbols.set_return_type(return_type.clone());

        // Set error type for ? checking
        self.current_return_error_type = return_type.result_err_type().cloned();

        // Check body
        for stmt in &func.body {
            self.check_statement(stmt);
        }

        self.current_return_error_type = None;
        self.symbols.exit_scope();
    }

    pub(crate) fn check_method(&mut self, method: &MethodDecl, owner: &str) {
        self.symbols.enter_scope(ScopeKind::Method {
            receiver: method.receiver,
        });

        // Define self if present
        if let Some(receiver) = method.receiver {
            let is_mutable = matches!(receiver, Receiver::Mutable);
            if is_mutable {
                self.mutable_bindings.insert("self".to_string());
            }
            self.symbols.define(Symbol {
                name: "self".to_string(),
                kind: SymbolKind::Variable(VariableInfo {
                    ty: ResolvedType::Named(owner.to_string()),
                    is_mutable,
                    is_used: true,
                }),
                span: Span::default(),
                scope: 0,
            });
        }

        // Define parameters
        for param in &method.params {
            let ty = resolve_type(&param.node.ty.node, &self.symbols);
            self.symbols.define(Symbol {
                name: param.node.name.clone(),
                kind: SymbolKind::Variable(VariableInfo {
                    ty,
                    is_mutable: false,
                    is_used: false,
                }),
                span: param.span,
                scope: 0,
            });
        }

        let return_type = resolve_type(&method.return_type.node, &self.symbols);
        self.symbols.set_return_type(return_type.clone());

        // Set error type for ? checking
        self.current_return_error_type = return_type.result_err_type().cloned();

        // Check body
        if let Some(body) = &method.body {
            for stmt in body {
                self.check_statement(stmt);
            }
        }

        self.current_return_error_type = None;
        self.mutable_bindings.remove("self");
        self.symbols.exit_scope();
    }
}
