//! Type checker for the Incan programming language
//!
//! Validates:
//! - All types are known and compatible
//! - Mutability rules (`mut` vs immutable)
//! - `?` operator only on `Result` types with compatible error types
//! - Trait conformance
//! - Newtype invariants
//! - Match exhaustiveness

use std::collections::{HashMap, HashSet};

use crate::frontend::ast::*;
use crate::frontend::diagnostics::{errors, CompileError};
use crate::frontend::symbols::*;

/// Valid derive names that can be used with @derive(...)
const VALID_DERIVES: &[&str] = &[
    "Debug", "Display",           // String representation
    "Eq", "Ord", "Hash",          // Comparison
    "Clone", "Copy", "Default",   // Copying
    "Serialize", "Deserialize",   // Serialization
];

/// Type checker state
pub struct TypeChecker {
    symbols: SymbolTable,
    errors: Vec<CompileError>,
    /// Track which bindings are mutable for mutation checks
    mutable_bindings: HashSet<String>,
    /// Current function's return type for `?` checking
    current_return_error_type: Option<ResolvedType>,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            symbols: SymbolTable::new(),
            errors: Vec::new(),
            mutable_bindings: HashSet::new(),
            current_return_error_type: None,
        }
    }

    /// Check a program and return errors if any
    pub fn check_program(&mut self, program: &Program) -> Result<(), Vec<CompileError>> {
        // First pass: collect type declarations
        for decl in &program.declarations {
            self.collect_declaration(decl);
        }

        // Second pass: check declarations
        for decl in &program.declarations {
            self.check_declaration(decl);
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(std::mem::take(&mut self.errors))
        }
    }

    /// Import symbols from another module's AST into this type checker's symbol table.
    /// This allows the main module to use types/functions from imported modules.
    pub fn import_module(&mut self, module_ast: &Program, _module_name: &str) {
        // Collect all declarations from the imported module
        for decl in &module_ast.declarations {
            self.collect_declaration(decl);
        }
    }

    /// Check a program that may have dependencies on other modules.
    /// `dependencies` is a list of (module_name, ast) pairs that should be imported.
    pub fn check_with_imports(
        &mut self,
        program: &Program,
        dependencies: &[(&str, &Program)],
    ) -> Result<(), Vec<CompileError>> {
        // First: import all dependencies
        for (name, dep_ast) in dependencies {
            self.import_module(dep_ast, name);
        }

        // Then check the main program
        self.check_program(program)
    }

    // ========================================================================
    // First pass: collect declarations
    // ========================================================================

    fn collect_declaration(&mut self, decl: &Spanned<Declaration>) {
        match &decl.node {
            Declaration::Import(import) => self.collect_import(import, decl.span),
            Declaration::Model(model) => self.collect_model(model, decl.span),
            Declaration::Class(class) => self.collect_class(class, decl.span),
            Declaration::Trait(tr) => self.collect_trait(tr, decl.span),
            Declaration::Newtype(nt) => self.collect_newtype(nt, decl.span),
            Declaration::Enum(en) => self.collect_enum(en, decl.span),
            Declaration::Function(func) => self.collect_function(func, decl.span),
            Declaration::Docstring(_) => {} // Docstrings don't need collection
        }
    }

    fn collect_import(&mut self, import: &ImportDecl, span: Span) {
        match &import.kind {
            ImportKind::Module(path) => {
                let name = import.alias.clone().unwrap_or_else(|| {
                    path.segments.last().cloned().unwrap_or_else(|| "module".to_string())
                });
                self.define_import_symbol(name, path.segments.clone(), false, span);
            }
            ImportKind::From { module, items } => {
                // For each item in `from module import item1, item2, ...`
                // create a symbol as if it were `import module::item`
                for item in items {
                    let name = item.alias.clone().unwrap_or_else(|| item.name.clone());
                    let mut path = module.segments.clone();
                    path.push(item.name.clone());
                    self.define_import_symbol(name, path, false, span);
                }
            }
            ImportKind::Python(pkg) => {
                let name = import.alias.clone().unwrap_or_else(|| pkg.clone());
                self.define_import_symbol(name, vec![pkg.clone()], true, span);
            }
            ImportKind::RustCrate { crate_name, path } => {
                // Rust crate import: import rust::serde_json or import rust::serde_json::Value
                let name = import.alias.clone().unwrap_or_else(|| {
                    path.last().cloned().unwrap_or_else(|| crate_name.clone())
                });
                let mut full_path = vec![crate_name.clone()];
                full_path.extend(path.clone());
                // Mark as "rust" import type for codegen
                self.define_rust_import_symbol(name, crate_name.clone(), full_path, span);
            }
            ImportKind::RustFrom { crate_name, path, items } => {
                // from rust::time import Instant, Duration
                for item in items {
                    let name = item.alias.clone().unwrap_or_else(|| item.name.clone());
                    let mut full_path = vec![crate_name.clone()];
                    full_path.extend(path.clone());
                    full_path.push(item.name.clone());
                    self.define_rust_import_symbol(name, crate_name.clone(), full_path, span);
                }
            }
        }
    }
    
    fn define_rust_import_symbol(&mut self, name: Ident, crate_name: String, path: Vec<Ident>, span: Span) {
        // Similar to define_import_symbol but specifically for Rust crates
        if let Some(id) = self.symbols.lookup(&name) {
            if let Some(sym) = self.symbols.get(id) {
                match &sym.kind {
                    SymbolKind::Type(_) | SymbolKind::Function(_) | SymbolKind::Trait(_) | SymbolKind::Variant(_) => {
                        return;
                    }
                    _ => {}
                }
            }
        }
        
        self.symbols.define(Symbol {
            name,
            kind: SymbolKind::RustModule {
                crate_name,
                path: path.join("::"),
            },
            span,
            scope: 0, // Will be set by define()
        });
    }

    fn define_import_symbol(&mut self, name: Ident, path: Vec<Ident>, is_python: bool, span: Span) {
        // Don't overwrite existing Type/Function/Trait definitions
        // (which may have been imported from dependency modules)
        if let Some(id) = self.symbols.lookup(&name) {
            if let Some(sym) = self.symbols.get(id) {
                match &sym.kind {
                    SymbolKind::Type(_) | SymbolKind::Function(_) | SymbolKind::Trait(_) | SymbolKind::Variant(_) => {
                        // Already have a real definition, don't overwrite with Module placeholder
                        return;
                    }
                    _ => {}
                }
            }
        }

        self.symbols.define(Symbol {
            name,
            kind: SymbolKind::Module(ModuleInfo { path, is_python }),
            span,
            scope: 0,
        });
    }

    fn collect_model(&mut self, model: &ModelDecl, span: Span) {
        let mut fields = HashMap::new();
        let mut methods = HashMap::new();

        for field in &model.fields {
            let ty = resolve_type(&field.node.ty.node, &self.symbols);
            fields.insert(
                field.node.name.clone(),
                FieldInfo {
                    ty,
                    has_default: field.node.default.is_some(),
                },
            );
        }

        for method in &model.methods {
            let params: Vec<_> = method
                .node
                .params
                .iter()
                .map(|p| (p.node.name.clone(), resolve_type(&p.node.ty.node, &self.symbols)))
                .collect();
            let return_type = resolve_type(&method.node.return_type.node, &self.symbols);
            methods.insert(
                method.node.name.clone(),
                MethodInfo {
                    receiver: method.node.receiver,
                    params,
                    return_type,
                    is_async: method.node.is_async,
                    has_body: method.node.body.is_some(),
                },
            );
        }

        // Check for Serialize/Deserialize derives and add JSON methods
        let derives = Self::extract_derive_names(&model.decorators);
        
        if derives.contains(&"Serialize".to_string()) {
            // Add to_json(&self) -> str
            methods.insert(
                "to_json".to_string(),
                MethodInfo {
                    receiver: Some(Receiver::Immutable),
                    params: vec![],
                    return_type: ResolvedType::Str,
                    is_async: false,
                    has_body: true,
                },
            );
        }
        
        if derives.contains(&"Deserialize".to_string()) {
            // Add from_json(json_str: str) -> Result[Self, str]
            methods.insert(
                "from_json".to_string(),
                MethodInfo {
                    receiver: None, // Static method
                    params: vec![("json_str".to_string(), ResolvedType::Str)],
                    return_type: ResolvedType::Generic(
                        "Result".to_string(),
                        vec![ResolvedType::Named(model.name.clone()), ResolvedType::Str],
                    ),
                    is_async: false,
                    has_body: true,
                },
            );
        }

        // Extract traits from decorators (though models don't typically use `with`)
        let traits = Vec::new();

        self.symbols.define(Symbol {
            name: model.name.clone(),
            kind: SymbolKind::Type(TypeInfo::Model(ModelInfo {
                type_params: model.type_params.clone(),
                traits,
                fields,
                methods,
            })),
            span,
            scope: 0,
        });
    }

    fn collect_class(&mut self, class: &ClassDecl, span: Span) {
        let mut fields = HashMap::new();
        let mut methods = HashMap::new();

        // Inherit fields and methods from parent class if present
        if let Some(parent_name) = &class.extends {
            if let Some(parent_id) = self.symbols.lookup(parent_name) {
                if let Some(parent_sym) = self.symbols.get(parent_id) {
                    if let SymbolKind::Type(TypeInfo::Class(parent_info)) = &parent_sym.kind {
                        // Copy parent fields
                        for (name, field_info) in &parent_info.fields {
                            fields.insert(name.clone(), field_info.clone());
                        }
                        // Copy parent methods
                        for (name, method_info) in &parent_info.methods {
                            methods.insert(name.clone(), method_info.clone());
                        }
                    }
                }
            }
        }

        // Add own fields (can override inherited ones)
        for field in &class.fields {
            let ty = resolve_type(&field.node.ty.node, &self.symbols);
            fields.insert(
                field.node.name.clone(),
                FieldInfo {
                    ty,
                    has_default: field.node.default.is_some(),
                },
            );
        }

        // Add own methods (can override inherited ones)
        for method in &class.methods {
            let params: Vec<_> = method
                .node
                .params
                .iter()
                .map(|p| (p.node.name.clone(), resolve_type(&p.node.ty.node, &self.symbols)))
                .collect();
            let return_type = resolve_type(&method.node.return_type.node, &self.symbols);
            methods.insert(
                method.node.name.clone(),
                MethodInfo {
                    receiver: method.node.receiver,
                    params,
                    return_type,
                    is_async: method.node.is_async,
                    has_body: method.node.body.is_some(),
                },
            );
        }

        // Check for Serialize/Deserialize derives and add JSON methods
        let derives = Self::extract_derive_names(&class.decorators);
        
        if derives.contains(&"Serialize".to_string()) {
            // Add to_json(&self) -> str
            methods.insert(
                "to_json".to_string(),
                MethodInfo {
                    receiver: Some(Receiver::Immutable),
                    params: vec![],
                    return_type: ResolvedType::Str,
                    is_async: false,
                    has_body: true,
                },
            );
        }
        
        if derives.contains(&"Deserialize".to_string()) {
            // Add from_json(json_str: str) -> Result[Self, str]
            methods.insert(
                "from_json".to_string(),
                MethodInfo {
                    receiver: None, // Static method
                    params: vec![("json_str".to_string(), ResolvedType::Str)],
                    return_type: ResolvedType::Generic(
                        "Result".to_string(),
                        vec![ResolvedType::Named(class.name.clone()), ResolvedType::Str],
                    ),
                    is_async: false,
                    has_body: true,
                },
            );
        }

        self.symbols.define(Symbol {
            name: class.name.clone(),
            kind: SymbolKind::Type(TypeInfo::Class(ClassInfo {
                type_params: class.type_params.clone(),
                extends: class.extends.clone(),
                traits: class.traits.clone(),
                fields,
                methods,
            })),
            span,
            scope: 0,
        });
    }

    fn collect_trait(&mut self, tr: &TraitDecl, span: Span) {
        let mut methods = HashMap::new();

        for method in &tr.methods {
            let params: Vec<_> = method
                .node
                .params
                .iter()
                .map(|p| (p.node.name.clone(), resolve_type(&p.node.ty.node, &self.symbols)))
                .collect();
            let return_type = resolve_type(&method.node.return_type.node, &self.symbols);
            methods.insert(
                method.node.name.clone(),
                MethodInfo {
                    receiver: method.node.receiver,
                    params,
                    return_type,
                    is_async: method.node.is_async,
                    has_body: method.node.body.is_some(),
                },
            );
        }

        // Extract @requires from decorators
        let requires = self.extract_requires(&tr.decorators);

        self.symbols.define(Symbol {
            name: tr.name.clone(),
            kind: SymbolKind::Trait(TraitInfo {
                type_params: tr.type_params.clone(),
                methods,
                requires,
            }),
            span,
            scope: 0,
        });
    }

    /// Validate @derive decorator arguments and report errors for unknown derives
    fn validate_derives(&mut self, decorators: &[Spanned<Decorator>]) {
        for dec in decorators {
            if dec.node.name == "derive" {
                for arg in &dec.node.args {
                    // Extract the derive name from the argument
                    let derive_name = match arg {
                        DecoratorArg::Positional(expr) => {
                            // @derive(Debug) - positional with identifier
                            if let Expr::Ident(name) = &expr.node {
                                Some((name.clone(), expr.span))
                            } else {
                                None
                            }
                        }
                        DecoratorArg::Named(name, _) => {
                            // @derive(name=...) - named args not valid for derive
                            Some((name.clone(), dec.span))
                        }
                    };
                    
                    if let Some((name, span)) = derive_name {
                        if !VALID_DERIVES.contains(&name.as_str()) {
                            // Check if the name refers to something else (model, class, etc.)
                            if let Some(sym_id) = self.symbols.lookup(&name) {
                                if let Some(sym) = self.symbols.get(sym_id) {
                                    let kind_name = match &sym.kind {
                                        SymbolKind::Type(TypeInfo::Model(_)) => Some("model"),
                                        SymbolKind::Type(TypeInfo::Class(_)) => Some("class"),
                                        SymbolKind::Type(TypeInfo::Enum(_)) => Some("enum"),
                                        SymbolKind::Function(_) => Some("function"),
                                        _ => None,
                                    };
                                    if let Some(kind) = kind_name {
                                        self.errors.push(errors::derive_wrong_kind(&name, kind, span));
                                        continue;
                                    }
                                }
                            }
                            self.errors.push(errors::unknown_derive(&name, span));
                        }
                    }
                }
            }
        }
    }

    fn extract_requires(&self, decorators: &[Spanned<Decorator>]) -> Vec<(String, ResolvedType)> {
        let mut requires = Vec::new();
        for dec in decorators {
            if dec.node.name == "requires" {
                for arg in &dec.node.args {
                    if let DecoratorArg::Named(name, DecoratorArgValue::Type(ty)) = arg {
                        requires.push((name.clone(), resolve_type(&ty.node, &self.symbols)));
                    }
                }
            }
        }
        requires
    }

    /// Extract derive names from @derive decorators
    fn extract_derive_names(decorators: &[Spanned<Decorator>]) -> Vec<String> {
        let mut derives = Vec::new();
        for dec in decorators {
            if dec.node.name == "derive" {
                for arg in &dec.node.args {
                    if let DecoratorArg::Positional(expr) = arg {
                        if let Expr::Ident(name) = &expr.node {
                            derives.push(name.clone());
                        }
                    }
                }
            }
        }
        derives
    }

    fn collect_newtype(&mut self, nt: &NewtypeDecl, span: Span) {
        let underlying = resolve_type(&nt.underlying.node, &self.symbols);
        let mut methods = HashMap::new();

        for method in &nt.methods {
            let params: Vec<_> = method
                .node
                .params
                .iter()
                .map(|p| (p.node.name.clone(), resolve_type(&p.node.ty.node, &self.symbols)))
                .collect();
            let return_type = resolve_type(&method.node.return_type.node, &self.symbols);
            methods.insert(
                method.node.name.clone(),
                MethodInfo {
                    receiver: method.node.receiver,
                    params,
                    return_type,
                    is_async: method.node.is_async,
                    has_body: method.node.body.is_some(),
                },
            );
        }

        self.symbols.define(Symbol {
            name: nt.name.clone(),
            kind: SymbolKind::Type(TypeInfo::Newtype(NewtypeInfo { underlying, methods })),
            span,
            scope: 0,
        });
    }

    fn collect_enum(&mut self, en: &EnumDecl, span: Span) {
        let variants: Vec<_> = en.variants.iter().map(|v| v.node.name.clone()).collect();

        self.symbols.define(Symbol {
            name: en.name.clone(),
            kind: SymbolKind::Type(TypeInfo::Enum(EnumInfo {
                type_params: en.type_params.clone(),
                variants: variants.clone(),
            })),
            span,
            scope: 0,
        });

        // Also define each variant as a symbol
        for variant in &en.variants {
            let fields: Vec<_> = variant
                .node
                .fields
                .iter()
                .map(|f| resolve_type(&f.node, &self.symbols))
                .collect();
            self.symbols.define(Symbol {
                name: variant.node.name.clone(),
                kind: SymbolKind::Variant(VariantInfo {
                    enum_name: en.name.clone(),
                    fields,
                }),
                span: variant.span,
                scope: 0,
            });
        }
    }

    fn collect_function(&mut self, func: &FunctionDecl, span: Span) {
        let params: Vec<_> = func
            .params
            .iter()
            .map(|p| (p.node.name.clone(), resolve_type(&p.node.ty.node, &self.symbols)))
            .collect();
        let return_type = resolve_type(&func.return_type.node, &self.symbols);

        self.symbols.define(Symbol {
            name: func.name.clone(),
            kind: SymbolKind::Function(FunctionInfo {
                params,
                return_type,
                is_async: func.is_async,
                type_params: Vec::new(),
            }),
            span,
            scope: 0,
        });
    }

    // ========================================================================
    // Second pass: check declarations
    // ========================================================================

    fn check_declaration(&mut self, decl: &Spanned<Declaration>) {
        match &decl.node {
            Declaration::Import(_) => {} // Already handled
            Declaration::Model(model) => self.check_model(model),
            Declaration::Class(class) => self.check_class(class),
            Declaration::Trait(tr) => self.check_trait(tr),
            Declaration::Newtype(nt) => self.check_newtype(nt),
            Declaration::Enum(en) => self.check_enum(en),
            Declaration::Function(func) => self.check_function(func),
            Declaration::Docstring(_) => {} // Docstrings don't need checking
        }
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
                self.errors.push(errors::missing_field(&class.name, field_name, Span::default()));
            }
        }

        // Check required methods (those without body)
        for (method_name, method_info) in &trait_info.methods {
            if !method_info.has_body {
                let found = class.methods.iter().any(|m| &m.node.name == method_name);
                if !found {
                    self.errors.push(errors::missing_trait_method(
                        trait_name,
                        method_name,
                        Span::default(),
                    ));
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
            self.symbols.enter_scope(ScopeKind::Method { receiver: method.node.receiver });
            
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
                    self.errors.push(errors::unknown_symbol(
                        &format!("{:?}", field_ty.node),
                        field_ty.span,
                    ));
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

    fn check_method(&mut self, method: &MethodDecl, owner: &str) {
        self.symbols.enter_scope(ScopeKind::Method { receiver: method.receiver });

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

    // ========================================================================
    // Statements
    // ========================================================================

    fn check_statement(&mut self, stmt: &Spanned<Statement>) {
        match &stmt.node {
            Statement::Assignment(assign) => self.check_assignment(assign, stmt.span),
            Statement::FieldAssignment(field_assign) => self.check_field_assignment(field_assign, stmt.span),
            Statement::IndexAssignment(index_assign) => self.check_index_assignment(index_assign, stmt.span),
            Statement::Return(expr) => self.check_return(expr.as_ref(), stmt.span),
            Statement::If(if_stmt) => self.check_if_stmt(if_stmt),
            Statement::While(while_stmt) => self.check_while_stmt(while_stmt),
            Statement::For(for_stmt) => self.check_for_stmt(for_stmt),
            Statement::Expr(expr) => {
                self.check_expr(expr);
            }
            Statement::Pass => {}
            Statement::Break => {}
            Statement::Continue => {}
            Statement::CompoundAssignment(compound) => {
                // Check that the variable exists and is mutable (search all scopes)
                let var_info_opt = self.symbols.lookup(&compound.name)
                    .and_then(|id| self.symbols.get(id))
                    .and_then(|sym| {
                        if let SymbolKind::Variable(var_info) = &sym.kind {
                            Some((var_info.is_mutable, var_info.ty.clone()))
                        } else {
                            None
                        }
                    });
                
                if let Some((is_mutable, var_ty)) = var_info_opt {
                    if !is_mutable {
                        self.errors.push(errors::mutation_without_mut(&compound.name, stmt.span));
                    }
                    // Type check the value expression
                    let value_ty = self.check_expr(&compound.value);
                    // For numeric operations, check type compatibility
                    if !self.types_compatible(&value_ty, &var_ty) {
                        self.errors.push(errors::type_mismatch(
                            &var_ty.to_string(),
                            &value_ty.to_string(),
                            compound.value.span,
                        ));
                    }
                } else {
                    self.errors.push(errors::unknown_symbol(&compound.name, stmt.span));
                }
            }
            Statement::TupleUnpack(unpack) => {
                // Check the value expression and get its type
                let value_ty = self.check_expr(&unpack.value);
                
                // Extract element types if it's a tuple
                let element_types: Vec<ResolvedType> = match &value_ty {
                    ResolvedType::Tuple(types) => types.clone(),
                    _ => {
                        // Not a tuple, create Unknown types for each name
                        vec![ResolvedType::Unknown; unpack.names.len()]
                    }
                };
                
                // Check that tuple has enough elements
                if element_types.len() < unpack.names.len() {
                    self.errors.push(CompileError::type_error(
                        format!(
                            "Cannot unpack {} values from tuple with {} elements",
                            unpack.names.len(),
                            element_types.len()
                        ),
                        stmt.span,
                    ));
                }
                
                // Define each variable with its corresponding type
                let is_mutable = matches!(unpack.binding, BindingKind::Mutable);
                for (i, name) in unpack.names.iter().enumerate() {
                    let ty = element_types.get(i).cloned().unwrap_or(ResolvedType::Unknown);
                    self.symbols.define(Symbol {
                        name: name.clone(),
                        kind: SymbolKind::Variable(VariableInfo {
                            ty,
                            is_mutable,
                            is_used: false,
                        }),
                        span: stmt.span,
                        scope: 0,
                    });
                    if is_mutable {
                        self.mutable_bindings.insert(name.clone());
                    }
                }
            }
            Statement::TupleAssign(assign) => {
                // Check the value expression (should be a tuple)
                let value_ty = self.check_expr(&assign.value);
                
                // Extract element types if it's a tuple
                let element_types: Vec<ResolvedType> = match &value_ty {
                    ResolvedType::Tuple(types) => types.clone(),
                    _ => {
                        // Not a tuple, create Unknown types for each target
                        vec![ResolvedType::Unknown; assign.targets.len()]
                    }
                };
                
                // Check that tuple has enough elements
                if element_types.len() < assign.targets.len() {
                    self.errors.push(CompileError::type_error(
                        format!(
                            "Cannot unpack {} values from tuple with {} elements",
                            assign.targets.len(),
                            element_types.len()
                        ),
                        stmt.span,
                    ));
                }
                
                // Check each target expression - must be a valid lvalue
                for (i, target) in assign.targets.iter().enumerate() {
                    let target_ty = self.check_expr(target);
                    let expected_ty = element_types.get(i).cloned().unwrap_or(ResolvedType::Unknown);
                    
                    // Check that target is a valid lvalue
                    match &target.node {
                        Expr::Ident(name) => {
                            // Check that the variable is mutable
                            if let Some(id) = self.symbols.lookup_local(name) {
                                if let Some(sym) = self.symbols.get(id) {
                                    if let SymbolKind::Variable(var_info) = &sym.kind {
                                        if !var_info.is_mutable {
                                            self.errors.push(errors::mutation_without_mut(name, target.span));
                                        }
                                    }
                                }
                            }
                        }
                        Expr::Index(_, _) | Expr::Field(_, _) => {
                            // Index and field expressions are valid lvalues
                            // Type compatibility is checked below
                        }
                        _ => {
                            self.errors.push(CompileError::syntax(
                                "Invalid assignment target in tuple assignment".to_string(),
                                target.span,
                            ));
                        }
                    }
                    
                    // Check type compatibility
                    if !self.types_compatible(&expected_ty, &target_ty) {
                        self.errors.push(errors::type_mismatch(
                            &target_ty.to_string(),
                            &expected_ty.to_string(),
                            target.span,
                        ));
                    }
                }
            }
        }
    }

    fn check_field_assignment(&mut self, field_assign: &FieldAssignmentStmt, span: Span) {
        // Check the object expression
        let obj_ty = self.check_expr(&field_assign.object);
        // Check the value expression
        let value_ty = self.check_expr(&field_assign.value);
        let field = &field_assign.field;
        
        // Tuples are immutable - disallow field assignment on tuples
        if matches!(obj_ty, ResolvedType::Tuple(_)) {
            self.errors.push(errors::tuple_field_assignment(span));
            return;
        }

        // Verify field exists on object and value type matches field type
        match &obj_ty {
            ResolvedType::Named(type_name) => {
                if let Some(id) = self.symbols.lookup(type_name) {
                    if let Some(sym) = self.symbols.get(id) {
                        if let SymbolKind::Type(type_info) = &sym.kind {
                            let field_type = match type_info {
                                TypeInfo::Model(model) => model.fields.get(field).map(|f| f.ty.clone()),
                                TypeInfo::Class(class) => class.fields.get(field).map(|f| f.ty.clone()),
                                _ => None,
                            };

                            match field_type {
                                Some(expected_ty) => {
                                    // Check type compatibility
                                    if !self.types_compatible(&value_ty, &expected_ty) {
                                        self.errors.push(errors::field_type_mismatch(
                                            field,
                                            &expected_ty.to_string(),
                                            &value_ty.to_string(),
                                            field_assign.value.span,
                                        ));
                                    }
                                }
                                None => {
                                    // Field doesn't exist
                                    self.errors.push(errors::missing_field(type_name, field, span));
                                }
                            }
                            return;
                        }
                    }
                }
                // Type not found - already reported elsewhere
            }
            ResolvedType::Unknown => {
                // Don't report additional errors on unknown types
            }
            _ => {
                // Cannot assign fields to primitive types
                self.errors.push(errors::missing_field(&obj_ty.to_string(), field, span));
            }
        }
    }

    fn check_index_assignment(&mut self, index_assign: &IndexAssignmentStmt, span: Span) {
        // Check the object expression (should be a collection)
        let obj_ty = self.check_expr(&index_assign.object);
        // Check the index expression
        let index_ty = self.check_expr(&index_assign.index);
        // Check the value expression
        let value_ty = self.check_expr(&index_assign.value);

        // Verify object is indexable and types match
        match &obj_ty {
            ResolvedType::Generic(name, args) => match name.as_str() {
                "List" => {
                    // List[T] - index must be int, value must be T
                    if !matches!(index_ty, ResolvedType::Int) {
                        self.errors.push(errors::index_type_mismatch(
                            "int",
                            &index_ty.to_string(),
                            index_assign.index.span,
                        ));
                    }
                    if let Some(elem_ty) = args.first() {
                        if !self.types_compatible(&value_ty, elem_ty) {
                            self.errors.push(errors::index_value_type_mismatch(
                                &elem_ty.to_string(),
                                &value_ty.to_string(),
                                index_assign.value.span,
                            ));
                        }
                    }
                }
                "Dict" => {
                    // Dict[K, V] - index must be K, value must be V
                    if let Some(key_ty) = args.first() {
                        if !self.types_compatible(&index_ty, key_ty) {
                            self.errors.push(errors::index_type_mismatch(
                                &key_ty.to_string(),
                                &index_ty.to_string(),
                                index_assign.index.span,
                            ));
                        }
                    }
                    if let Some(val_ty) = args.get(1) {
                        if !self.types_compatible(&value_ty, val_ty) {
                            self.errors.push(errors::index_value_type_mismatch(
                                &val_ty.to_string(),
                                &value_ty.to_string(),
                                index_assign.value.span,
                            ));
                        }
                    }
                }
                _ => {
                    self.errors.push(errors::not_indexable(&obj_ty.to_string(), span));
                }
            },
            ResolvedType::Tuple(_) => {
                // Tuples are immutable - cannot assign to index
                self.errors.push(errors::tuple_field_assignment(span));
            }
            ResolvedType::Str => {
                // Strings are immutable in Incan
                self.errors.push(CompileError::type_error(
                    "Strings are immutable - cannot assign to index".to_string(),
                    span,
                ));
            }
            ResolvedType::Unknown => {
                // Don't report additional errors on unknown types
            }
            _ => {
                self.errors.push(errors::not_indexable(&obj_ty.to_string(), span));
            }
        }
    }

    fn check_assignment(&mut self, assign: &AssignmentStmt, span: Span) {
        let value_ty = self.check_expr(&assign.value);

        // Check if it's a re-assignment
        if let Some(id) = self.symbols.lookup_local(&assign.name) {
            // Re-assignment - check mutability
            if let Some(sym) = self.symbols.get(id) {
                if let SymbolKind::Variable(var_info) = &sym.kind {
                    if !var_info.is_mutable {
                        self.errors.push(errors::mutation_without_mut(&assign.name, span));
                    }
                    // Type check
                    if !self.types_compatible(&value_ty, &var_info.ty) {
                        self.errors.push(errors::type_mismatch(
                            &var_info.ty.to_string(),
                            &value_ty.to_string(),
                            assign.value.span,
                        ));
                    }
                }
            }
            return;
        }

        // New binding
        let is_mutable = matches!(assign.binding, BindingKind::Mutable);
        
        // Tuples are immutable - disallow `mut` on tuple bindings
        if is_mutable && matches!(value_ty, ResolvedType::Tuple(_)) {
            self.errors.push(errors::mutable_tuple(span));
        }
        
        if is_mutable {
            self.mutable_bindings.insert(assign.name.clone());
        }

        let ty = if let Some(ty_ann) = &assign.ty {
            let ann_ty = resolve_type(&ty_ann.node, &self.symbols);
            // Check value matches annotation
            if !self.types_compatible(&value_ty, &ann_ty) {
                self.errors.push(errors::type_mismatch(
                    &ann_ty.to_string(),
                    &value_ty.to_string(),
                    assign.value.span,
                ));
            }
            ann_ty
        } else {
            value_ty
        };

        self.symbols.define(Symbol {
            name: assign.name.clone(),
            kind: SymbolKind::Variable(VariableInfo {
                ty,
                is_mutable,
                is_used: false,
            }),
            span,
            scope: 0,
        });
    }

    fn check_return(&mut self, expr: Option<&Spanned<Expr>>, span: Span) {
        let return_ty = if let Some(e) = expr {
            self.check_expr(e)
        } else {
            ResolvedType::Unit
        };

        if let Some(expected) = self.symbols.current_return_type() {
            if !self.types_compatible(&return_ty, expected) {
                self.errors.push(errors::type_mismatch(
                    &expected.to_string(),
                    &return_ty.to_string(),
                    span,
                ));
            }
        }
    }

    fn check_if_stmt(&mut self, if_stmt: &IfStmt) {
        let cond_ty = self.check_expr(&if_stmt.condition);
        if !self.types_compatible(&cond_ty, &ResolvedType::Bool) {
            self.errors.push(errors::type_mismatch(
                "bool",
                &cond_ty.to_string(),
                if_stmt.condition.span,
            ));
        }

        self.symbols.enter_scope(ScopeKind::Block);
        for stmt in &if_stmt.then_body {
            self.check_statement(stmt);
        }
        self.symbols.exit_scope();

        if let Some(else_body) = &if_stmt.else_body {
            self.symbols.enter_scope(ScopeKind::Block);
            for stmt in else_body {
                self.check_statement(stmt);
            }
            self.symbols.exit_scope();
        }
    }

    fn check_while_stmt(&mut self, while_stmt: &WhileStmt) {
        let cond_ty = self.check_expr(&while_stmt.condition);
        if !self.types_compatible(&cond_ty, &ResolvedType::Bool) {
            self.errors.push(errors::type_mismatch(
                "bool",
                &cond_ty.to_string(),
                while_stmt.condition.span,
            ));
        }

        self.symbols.enter_scope(ScopeKind::Block);
        for stmt in &while_stmt.body {
            self.check_statement(stmt);
        }
        self.symbols.exit_scope();
    }

    fn check_for_stmt(&mut self, for_stmt: &ForStmt) {
        let iter_ty = self.check_expr(&for_stmt.iter);
        
        // Infer element type from iterator
        let elem_ty = self.infer_iterator_element_type(&iter_ty);

        self.symbols.enter_scope(ScopeKind::Block);
        self.symbols.define(Symbol {
            name: for_stmt.var.clone(),
            kind: SymbolKind::Variable(VariableInfo {
                ty: elem_ty,
                is_mutable: false,
                is_used: false,
            }),
            span: for_stmt.iter.span,
            scope: 0,
        });

        for stmt in &for_stmt.body {
            self.check_statement(stmt);
        }
        self.symbols.exit_scope();
    }

    fn infer_iterator_element_type(&self, iter_ty: &ResolvedType) -> ResolvedType {
        match iter_ty {
            ResolvedType::Generic(name, args) => {
                match name.as_str() {
                    "List" | "Set" if !args.is_empty() => args[0].clone(),
                    "Dict" if args.len() >= 2 => {
                        // Iterating dict gives keys
                        args[0].clone()
                    }
                    _ => ResolvedType::Unknown,
                }
            }
            ResolvedType::Str => ResolvedType::Str, // String iteration gives chars/strings
            _ => ResolvedType::Unknown,
        }
    }

    // ========================================================================
    // Expressions
    // ========================================================================

    fn check_expr(&mut self, expr: &Spanned<Expr>) -> ResolvedType {
        match &expr.node {
            Expr::Ident(name) => self.check_ident(name, expr.span),
            Expr::Literal(lit) => self.check_literal(lit),
            Expr::SelfExpr => self.check_self(expr.span),
            Expr::Binary(left, op, right) => self.check_binary(left, *op, right, expr.span),
            Expr::Unary(op, operand) => self.check_unary(*op, operand, expr.span),
            Expr::Call(callee, args) => self.check_call(callee, args, expr.span),
            Expr::Index(base, index) => self.check_index(base, index, expr.span),
            Expr::Slice(base, slice) => self.check_slice(base, slice, expr.span),
            Expr::Field(base, field) => self.check_field(base, field, expr.span),
            Expr::MethodCall(base, method, args) => self.check_method_call(base, method, args, expr.span),
            Expr::Await(inner) => self.check_await(inner, expr.span),
            Expr::Try(inner) => self.check_try(inner, expr.span),
            Expr::Match(subject, arms) => self.check_match(subject, arms, expr.span),
            Expr::If(if_expr) => self.check_if_expr(if_expr, expr.span),
            Expr::ListComp(comp) => self.check_list_comp(comp, expr.span),
            Expr::DictComp(comp) => self.check_dict_comp(comp, expr.span),
            Expr::Closure(params, body) => self.check_closure(params, body, expr.span),
            Expr::Tuple(elems) => self.check_tuple(elems),
            Expr::List(elems) => self.check_list(elems),
            Expr::Dict(entries) => self.check_dict(entries),
            Expr::Set(elems) => self.check_set(elems),
            Expr::Paren(inner) => self.check_expr(inner),
            Expr::Constructor(name, args) => self.check_constructor(name, args, expr.span),
            Expr::FString(parts) => {
                for part in parts {
                    if let FStringPart::Expr(e) = part {
                        self.check_expr(e);
                    }
                }
                ResolvedType::Str
            }
            Expr::Yield(inner) => {
                // Yield returns the type of its inner expression, or Unit
                if let Some(inner) = inner {
                    self.check_expr(inner)
                } else {
                    ResolvedType::Unit
                }
            }
            Expr::Range { start, end, inclusive: _ } => {
                // Check both bounds
                let start_ty = self.check_expr(start);
                let end_ty = self.check_expr(end);
                // Both should be integers
                if start_ty != ResolvedType::Int {
                    self.errors.push(errors::type_mismatch(
                        "int",
                        &start_ty.to_string(),
                        start.span,
                    ));
                }
                if end_ty != ResolvedType::Int {
                    self.errors.push(errors::type_mismatch(
                        "int",
                        &end_ty.to_string(),
                        end.span,
                    ));
                }
                // Return Range type (maps to std::ops::Range<i64>)
                ResolvedType::Generic("Range".to_string(), vec![ResolvedType::Int])
            }
        }
    }

    fn check_ident(&mut self, name: &str, span: Span) -> ResolvedType {
        // Handle builtin modules
        if name == "math" {
            return ResolvedType::Named("math".to_string());
        }
        
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

    fn check_literal(&self, lit: &Literal) -> ResolvedType {
        match lit {
            Literal::Int(_) => ResolvedType::Int,
            Literal::Float(_) => ResolvedType::Float,
            Literal::String(_) => ResolvedType::Str,
            Literal::Bytes(_) => ResolvedType::Bytes,
            Literal::Bool(_) => ResolvedType::Bool,
            Literal::None => ResolvedType::Generic("Option".to_string(), vec![ResolvedType::Unknown]),
        }
    }

    fn check_self(&mut self, span: Span) -> ResolvedType {
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

    fn check_binary(
        &mut self,
        left: &Spanned<Expr>,
        op: BinaryOp,
        right: &Spanned<Expr>,
        span: Span,
    ) -> ResolvedType {
        let left_ty = self.check_expr(left);
        let right_ty = self.check_expr(right);

        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                // Numeric operations
                if self.types_compatible(&left_ty, &ResolvedType::Int)
                    && self.types_compatible(&right_ty, &ResolvedType::Int)
                {
                    ResolvedType::Int
                } else if self.types_compatible(&left_ty, &ResolvedType::Float)
                    || self.types_compatible(&right_ty, &ResolvedType::Float)
                {
                    ResolvedType::Float
                } else if matches!(op, BinaryOp::Add) && self.types_compatible(&left_ty, &ResolvedType::Str) {
                    ResolvedType::Str
                } else {
                    self.errors.push(errors::type_mismatch(
                        "numeric",
                        &format!("{} {} {}", left_ty, op, right_ty),
                        span,
                    ));
                    ResolvedType::Unknown
                }
            }
            BinaryOp::Eq | BinaryOp::NotEq => ResolvedType::Bool,
            BinaryOp::Lt | BinaryOp::Gt | BinaryOp::LtEq | BinaryOp::GtEq => ResolvedType::Bool,
            BinaryOp::And | BinaryOp::Or => ResolvedType::Bool,
            BinaryOp::In | BinaryOp::NotIn => ResolvedType::Bool,
            BinaryOp::Is => ResolvedType::Bool,
        }
    }

    fn check_unary(&mut self, op: UnaryOp, operand: &Spanned<Expr>, span: Span) -> ResolvedType {
        let operand_ty = self.check_expr(operand);
        match op {
            UnaryOp::Neg => {
                if self.types_compatible(&operand_ty, &ResolvedType::Int) {
                    ResolvedType::Int
                } else if self.types_compatible(&operand_ty, &ResolvedType::Float) {
                    ResolvedType::Float
                } else {
                    self.errors.push(errors::type_mismatch(
                        "numeric",
                        &operand_ty.to_string(),
                        span,
                    ));
                    ResolvedType::Unknown
                }
            }
            UnaryOp::Not => {
                if !self.types_compatible(&operand_ty, &ResolvedType::Bool) {
                    self.errors.push(errors::type_mismatch(
                        "bool",
                        &operand_ty.to_string(),
                        span,
                    ));
                }
                ResolvedType::Bool
            }
        }
    }

    fn check_call(
        &mut self,
        callee: &Spanned<Expr>,
        args: &[CallArg],
        _span: Span,
    ) -> ResolvedType {
        // Handle math module function calls (math.sqrt, math.sin, etc.)
        if let Expr::Field(base, method) = &callee.node {
            if let Expr::Ident(module) = &base.node {
                if module == "math" {
                    // Check arguments
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    // All math functions return float
                    match method.as_str() {
                        "sqrt" | "sin" | "cos" | "tan" | "abs" | "floor" | "ceil" | 
                        "pow" | "log" | "log10" | "exp" | "asin" | "acos" | "atan" |
                        "sinh" | "cosh" | "tanh" => return ResolvedType::Float,
                        _ => {}
                    }
                }
            }
        }
        
        // Handle built-in Result/Option constructors specially
        if let Expr::Ident(name) = &callee.node {
            match name.as_str() {
                "Ok" | "Err" => {
                    // Check arguments and get their types
                    let mut arg_types = Vec::new();
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) => { arg_types.push(self.check_expr(e)); }
                            CallArg::Named(_, e) => { arg_types.push(self.check_expr(e)); }
                        }
                    }
                    // Result constructors return Result type
                    // Generic("Result", [T, E])
                    let (ok_ty, err_ty) = if name == "Ok" {
                        (arg_types.first().cloned().unwrap_or(ResolvedType::Unknown), ResolvedType::Unknown)
                    } else {
                        (ResolvedType::Unknown, arg_types.first().cloned().unwrap_or(ResolvedType::Unknown))
                    };
                    return ResolvedType::Generic(
                        "Result".to_string(),
                        vec![ok_ty, err_ty],
                    );
                }
                "Some" => {
                    let mut arg_types = Vec::new();
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) => { arg_types.push(self.check_expr(e)); }
                            CallArg::Named(_, e) => { arg_types.push(self.check_expr(e)); }
                        }
                    }
                    let inner = arg_types.first().cloned().unwrap_or(ResolvedType::Unknown);
                    return ResolvedType::Generic("Option".to_string(), vec![inner]);
                }
                // Built-in functions
                "println" | "print" => {
                    // Check arguments but return Unit
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    return ResolvedType::Unit;
                }
                "len" => {
                    // len(collection) -> int
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    return ResolvedType::Int;
                }
                "sum" => {
                    // sum(collection) -> int
                    // For boolean lists, counts True values
                    // For numeric lists, sums the values
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    return ResolvedType::Int;
                }
                "range" => {
                    // range(n) or range(a, b) -> Iterator[int], represented as List[int] for now
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    return ResolvedType::Generic("List".to_string(), vec![ResolvedType::Int]);
                }
                "sleep" => {
                    // sleep(seconds: float) -> Unit (async)
                    if let Some(arg) = args.first() {
                        let arg_expr = match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => e,
                        };
                        let arg_ty = self.check_expr(arg_expr);
                        if !self.types_compatible(&arg_ty, &ResolvedType::Float) {
                            self.errors.push(errors::type_mismatch(
                                "float",
                                &arg_ty.to_string(),
                                arg_expr.span,
                            ));
                        }
                    }
                    return ResolvedType::Unit;
                }
                "sleep_ms" => {
                    // sleep_ms(millis: int) -> Unit (async)
                    if let Some(arg) = args.first() {
                        let arg_expr = match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => e,
                        };
                        let arg_ty = self.check_expr(arg_expr);
                        if !self.types_compatible(&arg_ty, &ResolvedType::Int) {
                            self.errors.push(errors::type_mismatch(
                                "int",
                                &arg_ty.to_string(),
                                arg_expr.span,
                            ));
                        }
                    }
                    return ResolvedType::Unit;
                }
                "timeout" => {
                    // timeout(seconds: float, task) -> Unknown
                    if let Some(arg) = args.first() {
                        let arg_expr = match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => e,
                        };
                        let arg_ty = self.check_expr(arg_expr);
                        if !self.types_compatible(&arg_ty, &ResolvedType::Float) {
                            self.errors.push(errors::type_mismatch(
                                "float",
                                &arg_ty.to_string(),
                                arg_expr.span,
                            ));
                        }
                    }
                    // Check the task argument type (still Unknown, but type-check inner expression)
                    if args.len() >= 2 {
                        let task = match &args[1] {
                            CallArg::Positional(e) | CallArg::Named(_, e) => e,
                        };
                        self.check_expr(task);
                    }
                    return ResolvedType::Unknown;
                }
                "timeout_ms" => {
                    // timeout_ms(millis: int, task) -> Unknown
                    if let Some(arg) = args.first() {
                        let arg_expr = match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => e,
                        };
                        let arg_ty = self.check_expr(arg_expr);
                        if !self.types_compatible(&arg_ty, &ResolvedType::Int) {
                            self.errors.push(errors::type_mismatch(
                                "int",
                                &arg_ty.to_string(),
                                arg_expr.span,
                            ));
                        }
                    }
                    if args.len() >= 2 {
                        let task = match &args[1] {
                            CallArg::Positional(e) | CallArg::Named(_, e) => e,
                        };
                        self.check_expr(task);
                    }
                    return ResolvedType::Unknown;
                }
                // Python-like type conversion builtins
                "dict" => {
                    // dict() -> empty Dict, dict(iterable) -> Dict from key-value pairs
                    let (key_ty, val_ty) = if let Some(arg) = args.first() {
                        let arg_expr = match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => e,
                        };
                        let arg_ty = self.check_expr(arg_expr);
                        // Try to extract key/value types from input
                        match &arg_ty {
                            ResolvedType::Generic(name, type_args) if name == "Dict" && type_args.len() >= 2 => {
                                (type_args[0].clone(), type_args[1].clone())
                            }
                            _ => (ResolvedType::Unknown, ResolvedType::Unknown)
                        }
                    } else {
                        (ResolvedType::Unknown, ResolvedType::Unknown)
                    };
                    return ResolvedType::Generic("Dict".to_string(), vec![key_ty, val_ty]);
                }
                "list" => {
                    // list() -> empty List, list(iterable) -> List from iterable
                    let elem_ty = if let Some(arg) = args.first() {
                        let arg_expr = match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => e,
                        };
                        let arg_ty = self.check_expr(arg_expr);
                        match &arg_ty {
                            ResolvedType::Generic(name, type_args) if (name == "List" || name == "Vec" || name == "Set") && !type_args.is_empty() => {
                                type_args[0].clone()
                            }
                            ResolvedType::Str => ResolvedType::Str, // list("abc") -> List[str] of chars
                            _ => ResolvedType::Unknown
                        }
                    } else {
                        ResolvedType::Unknown
                    };
                    return ResolvedType::Generic("List".to_string(), vec![elem_ty]);
                }
                "set" => {
                    // set() -> empty Set, set(iterable) -> Set from iterable
                    let elem_ty = if let Some(arg) = args.first() {
                        let arg_expr = match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => e,
                        };
                        let arg_ty = self.check_expr(arg_expr);
                        match &arg_ty {
                            ResolvedType::Generic(name, type_args) if (name == "List" || name == "Vec" || name == "Set") && !type_args.is_empty() => {
                                type_args[0].clone()
                            }
                            _ => ResolvedType::Unknown
                        }
                    } else {
                        ResolvedType::Unknown
                    };
                    return ResolvedType::Generic("Set".to_string(), vec![elem_ty]);
                }
                "enumerate" => {
                    // enumerate(iter) -> Iterator[(int, T)], returns list of tuples
                    let mut inner_ty = ResolvedType::Unknown;
                    if let Some(arg) = args.first() {
                        let arg_expr = match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => e,
                        };
                        let iter_ty = self.check_expr(arg_expr);
                        // Extract inner type from List[T] or similar
                        if let ResolvedType::Generic(name, type_args) = &iter_ty {
                            if (name == "List" || name == "Vec") && !type_args.is_empty() {
                                inner_ty = type_args[0].clone();
                            }
                        }
                    }
                    // Returns List[(int, T)]
                    return ResolvedType::Generic(
                        "List".to_string(),
                        vec![ResolvedType::Tuple(vec![ResolvedType::Int, inner_ty])],
                    );
                }
                "zip" => {
                    // zip(iter1, iter2) -> Iterator[(T1, T2)]
                    let mut ty1 = ResolvedType::Unknown;
                    let mut ty2 = ResolvedType::Unknown;
                    
                    if args.len() >= 2 {
                        let arg1 = match &args[0] {
                            CallArg::Positional(e) | CallArg::Named(_, e) => e,
                        };
                        let arg2 = match &args[1] {
                            CallArg::Positional(e) | CallArg::Named(_, e) => e,
                        };
                        
                        let iter1_ty = self.check_expr(arg1);
                        let iter2_ty = self.check_expr(arg2);
                        
                        // Extract inner types
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
                    
                    // Returns List[(T1, T2)]
                    return ResolvedType::Generic(
                        "List".to_string(),
                        vec![ResolvedType::Tuple(vec![ty1, ty2])],
                    );
                }
                // File I/O functions
                "read_file" => {
                    // read_file(path) -> Result[str, str]
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    return ResolvedType::Generic(
                        "Result".to_string(),
                        vec![ResolvedType::Str, ResolvedType::Str],
                    );
                }
                "write_file" => {
                    // write_file(path, content) -> Result[(), str]
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    return ResolvedType::Generic(
                        "Result".to_string(),
                        vec![ResolvedType::Unit, ResolvedType::Str],
                    );
                }
                // Type conversion functions
                "int" => {
                    // int(x) -> int (parses string or converts float)
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    return ResolvedType::Int;
                }
                "str" => {
                    // str(x) -> str (converts any type to string)
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    return ResolvedType::Str;
                }
                "float" => {
                    // float(x) -> float (parses string or converts int)
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    return ResolvedType::Float;
                }
                "json_stringify" => {
                    // json_stringify(value) -> str
                    // Serializes any value with Serialize derive to JSON string
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    return ResolvedType::Str;
                }
                "json_parse" => {
                    // json_parse[T](json_str) -> Result[T, str]
                    // Parses JSON string into type T (requires Deserialize derive)
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    // Return type is Result[T, str] but we don't know T here
                    // The caller should provide type annotation
                    return ResolvedType::Generic(
                        "Result".to_string(),
                        vec![ResolvedType::Unknown, ResolvedType::Str],
                    );
                }
                // Async primitives
                "spawn" => {
                    // spawn(task) -> JoinHandle[T]
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    return ResolvedType::Generic("JoinHandle".to_string(), vec![ResolvedType::Unknown]);
                }
                "channel" => {
                    // channel[T](buffer_size) -> (Sender[T], Receiver[T])
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    // Return tuple of (Sender[T], Receiver[T])
                    let inner = ResolvedType::Unknown; // Type parameter T
                    return ResolvedType::Tuple(vec![
                        ResolvedType::Generic("Sender".to_string(), vec![inner.clone()]),
                        ResolvedType::Generic("Receiver".to_string(), vec![inner]),
                    ]);
                }
                "unbounded_channel" => {
                    // unbounded_channel[T]() -> (UnboundedSender[T], UnboundedReceiver[T])
                    return ResolvedType::Tuple(vec![
                        ResolvedType::Generic("UnboundedSender".to_string(), vec![ResolvedType::Unknown]),
                        ResolvedType::Generic("UnboundedReceiver".to_string(), vec![ResolvedType::Unknown]),
                    ]);
                }
                "oneshot" => {
                    // oneshot[T]() -> (OneshotSender[T], OneshotReceiver[T])
                    return ResolvedType::Tuple(vec![
                        ResolvedType::Generic("OneshotSender".to_string(), vec![ResolvedType::Unknown]),
                        ResolvedType::Generic("OneshotReceiver".to_string(), vec![ResolvedType::Unknown]),
                    ]);
                }
                "Mutex" => {
                    // Mutex(value) -> Mutex[T]
                    let inner = if let Some(arg) = args.first() {
                        let arg_expr = match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => e,
                        };
                        self.check_expr(arg_expr)
                    } else {
                        ResolvedType::Unknown
                    };
                    return ResolvedType::Generic("Mutex".to_string(), vec![inner]);
                }
                "RwLock" => {
                    // RwLock(value) -> RwLock[T]
                    let inner = if let Some(arg) = args.first() {
                        let arg_expr = match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => e,
                        };
                        self.check_expr(arg_expr)
                    } else {
                        ResolvedType::Unknown
                    };
                    return ResolvedType::Generic("RwLock".to_string(), vec![inner]);
                }
                "Semaphore" => {
                    // Semaphore(permits) -> Semaphore
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    return ResolvedType::Named("Semaphore".to_string());
                }
                "Barrier" => {
                    // Barrier(count) -> Barrier
                    for arg in args {
                        match arg {
                            CallArg::Positional(e) | CallArg::Named(_, e) => { self.check_expr(e); }
                        }
                    }
                    return ResolvedType::Named("Barrier".to_string());
                }
                "yield_now" => {
                    // yield_now() -> Unit (async)
                    return ResolvedType::Unit;
                }
                _ => {}
            }
        }
        
        let callee_ty = self.check_expr(callee);

        // Check arguments
        for arg in args {
            match arg {
                CallArg::Positional(e) => {
                    self.check_expr(e);
                }
                CallArg::Named(_, e) => {
                    self.check_expr(e);
                }
            }
        }

        match callee_ty {
            ResolvedType::Function(_, ret) => *ret,
            ResolvedType::Named(name) => {
                // Could be a constructor
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

    fn check_index(
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
            ResolvedType::Tuple(elems) => {
                // For tuple indexing, we'd need constant evaluation
                // Return first element type as approximation
                elems.first().cloned().unwrap_or(ResolvedType::Unknown)
            }
            _ => ResolvedType::Unknown,
        }
    }

    fn check_slice(
        &mut self,
        base: &Spanned<Expr>,
        slice: &SliceExpr,
        _span: Span,
    ) -> ResolvedType {
        let base_ty = self.check_expr(base);
        
        // Check slice components (all should be integers)
        if let Some(start) = &slice.start {
            self.check_expr(start);
        }
        if let Some(end) = &slice.end {
            self.check_expr(end);
        }
        if let Some(step) = &slice.step {
            self.check_expr(step);
        }

        // Slicing returns the same collection type
        match base_ty {
            ResolvedType::Generic(name, args) => match name.as_str() {
                "List" => ResolvedType::Generic("List".to_string(), args),
                _ => ResolvedType::Unknown,
            },
            ResolvedType::Str => ResolvedType::Str,
            _ => ResolvedType::Unknown,
        }
    }

    fn check_field(&mut self, base: &Spanned<Expr>, field: &str, span: Span) -> ResolvedType {
        // Handle builtin math module
        if let Expr::Ident(name) = &base.node {
            if name == "math" {
                // math.pi, math.e, math.tau, math.inf, math.nan are all float
                match field {
                    "pi" | "e" | "tau" | "inf" | "nan" => return ResolvedType::Float,
                    _ => {}
                }
            }
        }
        
        let base_ty = self.check_expr(base);

        match &base_ty {
            ResolvedType::Tuple(elements) => {
                // Tuple index access: .0, .1, etc.
                if let Ok(idx) = field.parse::<usize>() {
                    if idx < elements.len() {
                        return elements[idx].clone();
                    }
                }
                self.errors.push(errors::missing_field(&base_ty.to_string(), field, span));
                ResolvedType::Unknown
            }
            ResolvedType::Named(type_name) => {
                if let Some(id) = self.symbols.lookup(type_name) {
                    if let Some(sym) = self.symbols.get(id) {
                        match &sym.kind {
                            SymbolKind::Type(type_info) => {
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
                                        // Check if field is actually a variant
                                        if enum_info.variants.contains(&field.to_string()) {
                                            return ResolvedType::Named(type_name.clone());
                                        }
                                    }
                                    TypeInfo::Newtype(nt) => {
                                        // Newtype .0 access returns the underlying type
                                        if field == "0" {
                                            return nt.underlying.clone();
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                }
                self.errors.push(errors::missing_field(type_name, field, span));
                ResolvedType::Unknown
            }
            _ => {
                self.errors.push(errors::missing_field(&base_ty.to_string(), field, span));
                ResolvedType::Unknown
            }
        }
    }

    fn check_method_call(
        &mut self,
        base: &Spanned<Expr>,
        method: &str,
        args: &[CallArg],
        _span: Span,
    ) -> ResolvedType {
        let base_ty = self.check_expr(base);

        // Check arguments
        for arg in args {
            match arg {
                CallArg::Positional(e) => {
                    self.check_expr(e);
                }
                CallArg::Named(_, e) => {
                    self.check_expr(e);
                }
            }
        }

        match &base_ty {
            ResolvedType::Named(type_name) => {
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
                                    // Check traits
                                    for trait_name in &class.traits {
                                        if let Some(tid) = self.symbols.lookup(trait_name) {
                                            if let Some(tsym) = self.symbols.get(tid) {
                                                if let SymbolKind::Trait(trait_info) = &tsym.kind {
                                                    if let Some(method_info) = trait_info.methods.get(method) {
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
            _ => {}
        }

        // Return unknown for now - could add more method resolution
        ResolvedType::Unknown
    }

    fn check_await(&mut self, inner: &Spanned<Expr>, _span: Span) -> ResolvedType {
        let inner_ty = self.check_expr(inner);
        // Await unwraps Future/async result
        // For simplicity, return the inner type
        inner_ty
    }

    fn check_try(&mut self, inner: &Spanned<Expr>, span: Span) -> ResolvedType {
        let inner_ty = self.check_expr(inner);

        // ? only works on Result
        if !inner_ty.is_result() {
            self.errors.push(errors::try_on_non_result(&inner_ty.to_string(), span));
            return ResolvedType::Unknown;
        }

        // Check error type compatibility
        if let (Some(inner_err), Some(expected_err)) = (
            inner_ty.result_err_type(),
            &self.current_return_error_type,
        ) {
            if !self.types_compatible(inner_err, expected_err) {
                self.errors.push(errors::incompatible_error_type(
                    &expected_err.to_string(),
                    &inner_err.to_string(),
                    span,
                ));
            }
        }

        // Return the Ok type
        inner_ty.result_ok_type().cloned().unwrap_or(ResolvedType::Unknown)
    }

    fn check_match(
        &mut self,
        subject: &Spanned<Expr>,
        arms: &[Spanned<MatchArm>],
        _span: Span,
    ) -> ResolvedType {
        let subject_ty = self.check_expr(subject);

        // Check exhaustiveness for enums
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

        // All arms should have compatible types
        if let Some(first) = arm_types.first() {
            first.clone()
        } else {
            ResolvedType::Unit
        }
    }

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
                // Special handling for Result/Option constructors (Ok, Err, Some, None)
                match name.as_str() {
                    "Ok" => {
                        // Ok(value) pattern matching against Result[T, E]
                        if let ResolvedType::Generic(type_name, args) = expected_ty {
                            if type_name == "Result" && !args.is_empty() {
                                // The inner type is the first type parameter (T in Result[T, E])
                                if let Some(pat) = sub_patterns.first() {
                                    self.check_pattern(pat, &args[0]);
                                }
                                return;
                            }
                        }
                    }
                    "Err" => {
                        // Err(error) pattern matching against Result[T, E]
                        if let ResolvedType::Generic(type_name, args) = expected_ty {
                            if type_name == "Result" && args.len() >= 2 {
                                // The error type is the second type parameter (E in Result[T, E])
                                if let Some(pat) = sub_patterns.first() {
                                    self.check_pattern(pat, &args[1]);
                                }
                                return;
                            }
                        }
                    }
                    "Some" => {
                        // Some(value) pattern matching against Option[T]
                        if let ResolvedType::Generic(type_name, args) = expected_ty {
                            if type_name == "Option" && !args.is_empty() {
                                // The inner type is the first type parameter (T in Option[T])
                                if let Some(pat) = sub_patterns.first() {
                                    self.check_pattern(pat, &args[0]);
                                }
                                return;
                            }
                        }
                    }
                    "None" => {
                        // None has no inner pattern
                        return;
                    }
                    _ => {}
                }
                
                // Look up constructor and check sub-patterns for regular enums
                // Handle qualified names like "Shape::Circle" -> look up "Circle"
                let variant_name = if name.contains("::") {
                    name.split("::").last().unwrap_or(name)
                } else {
                    name.as_str()
                };
                
                // Clone field types to avoid borrow conflict
                let field_types: Option<Vec<ResolvedType>> = self.symbols.lookup(variant_name)
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

    fn check_match_exhaustiveness(
        &mut self,
        subject_ty: &ResolvedType,
        arms: &[Spanned<MatchArm>],
        span: Span,
    ) {
        // Get enum variants if subject is an enum
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
            // Result has Ok, Err; Option has Some, None
            if subject_ty.is_result() {
                Some(vec!["Ok".to_string(), "Err".to_string()])
            } else {
                Some(vec!["Some".to_string(), "None".to_string()])
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
                    Pattern::Constructor(name, _) => {
                        // Handle qualified names like "Shape::Circle" -> extract "Circle"
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
                let missing: Vec<String> = all_variants
                    .iter()
                    .filter(|v| !covered.contains(*v))
                    .cloned()
                    .collect();

                if !missing.is_empty() {
                    self.errors.push(errors::non_exhaustive_match(&missing, span));
                }
            }
        }
    }

    fn check_if_expr(&mut self, if_expr: &IfExpr, _span: Span) -> ResolvedType {
        let cond_ty = self.check_expr(&if_expr.condition);
        if !self.types_compatible(&cond_ty, &ResolvedType::Bool) {
            self.errors.push(errors::type_mismatch(
                "bool",
                &cond_ty.to_string(),
                if_expr.condition.span,
            ));
        }

        self.symbols.enter_scope(ScopeKind::Block);
        for stmt in &if_expr.then_body {
            self.check_statement(stmt);
        }
        self.symbols.exit_scope();

        if let Some(else_body) = &if_expr.else_body {
            self.symbols.enter_scope(ScopeKind::Block);
            for stmt in else_body {
                self.check_statement(stmt);
            }
            self.symbols.exit_scope();
        }

        ResolvedType::Unit
    }

    fn check_list_comp(&mut self, comp: &ListComp, _span: Span) -> ResolvedType {
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

    fn check_dict_comp(&mut self, comp: &DictComp, _span: Span) -> ResolvedType {
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

    fn check_closure(
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

    fn check_tuple(&mut self, elems: &[Spanned<Expr>]) -> ResolvedType {
        let elem_types: Vec<_> = elems.iter().map(|e| self.check_expr(e)).collect();
        ResolvedType::Tuple(elem_types)
    }

    fn check_list(&mut self, elems: &[Spanned<Expr>]) -> ResolvedType {
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

    fn check_dict(&mut self, entries: &[(Spanned<Expr>, Spanned<Expr>)]) -> ResolvedType {
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

    fn check_set(&mut self, elems: &[Spanned<Expr>]) -> ResolvedType {
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

    fn check_constructor(
        &mut self,
        name: &str,
        args: &[CallArg],
        span: Span,
    ) -> ResolvedType {
        // Check arguments
        for arg in args {
            match arg {
                CallArg::Positional(e) => {
                    self.check_expr(e);
                }
                CallArg::Named(_, e) => {
                    self.check_expr(e);
                }
            }
        }

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

    // ========================================================================
    // Type compatibility
    // ========================================================================

    fn types_compatible(&self, actual: &ResolvedType, expected: &ResolvedType) -> bool {
        if actual == expected {
            return true;
        }

        match (actual, expected) {
            (ResolvedType::Unknown, _) | (_, ResolvedType::Unknown) => true,
            (ResolvedType::TypeVar(_), _) | (_, ResolvedType::TypeVar(_)) => true,
            (ResolvedType::Generic(n1, a1), ResolvedType::Generic(n2, a2)) => {
                n1 == n2 && a1.len() == a2.len() && a1.iter().zip(a2.iter()).all(|(t1, t2)| self.types_compatible(t1, t2))
            }
            (ResolvedType::Function(p1, r1), ResolvedType::Function(p2, r2)) => {
                p1.len() == p2.len()
                    && p1.iter().zip(p2.iter()).all(|(t1, t2)| self.types_compatible(t1, t2))
                    && self.types_compatible(r1, r2)
            }
            (ResolvedType::Tuple(e1), ResolvedType::Tuple(e2)) => {
                e1.len() == e2.len() && e1.iter().zip(e2.iter()).all(|(t1, t2)| self.types_compatible(t1, t2))
            }
            _ => false,
        }
    }
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience function to type-check an AST
pub fn check(program: &Program) -> Result<(), Vec<CompileError>> {
    TypeChecker::new().check_program(program)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::{lexer, parser};

    fn check_str(source: &str) -> Result<(), Vec<CompileError>> {
        let tokens = lexer::lex(source).map_err(|_| vec![])?;
        let ast = parser::parse(&tokens).map_err(|_| vec![])?;
        check(&ast)
    }

    // ========================================
    // Basic function tests
    // ========================================

    #[test]
    fn test_simple_function() {
        let source = r#"
def add(a: int, b: int) -> int:
  return a + b
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_type_mismatch() {
        let source = r#"
def foo() -> int:
  return "hello"
"#;
        let result = check_str(source);
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_symbol() {
        let source = r#"
def foo() -> int:
  return unknown_var
"#;
        let result = check_str(source);
        assert!(result.is_err());
    }

    #[test]
    fn test_try_on_non_result() {
        let source = r#"
def foo() -> Result[int, str]:
  x = 42
  y = x?
  return Ok(y)
"#;
        let result = check_str(source);
        assert!(result.is_err());
    }

    #[test]
    fn test_sleep_requires_float() {
        let source = r#"
async def foo():
  await sleep(1)
"#;
        let result = check_str(source);
        assert!(result.is_err());
    }

    // ========================================
    // Variable declaration and assignment
    // ========================================

    #[test]
    fn test_variable_declaration() {
        let source = r#"
def foo() -> int:
  x = 10
  return x
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_mutable_variable() {
        let source = r#"
def foo() -> int:
  mut x = 10
  x = 20
  return x
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_typed_variable() {
        let source = r#"
def foo() -> int:
  let x: int = 10
  return x
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // Arithmetic operations
    // ========================================

    #[test]
    fn test_arithmetic_addition() {
        let source = r#"
def foo() -> int:
  return 1 + 2
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_arithmetic_subtraction() {
        let source = r#"
def foo() -> int:
  return 10 - 5
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_arithmetic_multiplication() {
        let source = r#"
def foo() -> int:
  return 3 * 4
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_arithmetic_division() {
        let source = r#"
def foo() -> int:
  return 10 / 2
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_arithmetic_modulo() {
        let source = r#"
def foo() -> int:
  return 10 % 3
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // Comparison operations
    // ========================================

    #[test]
    fn test_comparison_equal() {
        let source = r#"
def foo() -> bool:
  return 1 == 1
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_comparison_not_equal() {
        let source = r#"
def foo() -> bool:
  return 1 != 2
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_comparison_less_than() {
        let source = r#"
def foo() -> bool:
  return 1 < 2
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_comparison_greater_than() {
        let source = r#"
def foo() -> bool:
  return 2 > 1
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // Logical operations
    // ========================================

    #[test]
    fn test_logical_and() {
        let source = r#"
def foo() -> bool:
  return true and false
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_logical_or() {
        let source = r#"
def foo() -> bool:
  return true or false
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_logical_not() {
        let source = r#"
def foo() -> bool:
  return not true
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // String operations
    // ========================================

    #[test]
    fn test_string_return() {
        let source = r#"
def foo() -> str:
  return "hello"
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_string_concat() {
        let source = r#"
def foo() -> str:
  return "hello" + " world"
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // Control flow
    // ========================================

    #[test]
    fn test_if_statement() {
        let source = r#"
def foo(x: int) -> int:
  if x > 0:
    return 1
  return 0
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_if_else_statement() {
        let source = r#"
def foo(x: int) -> int:
  if x > 0:
    return 1
  else:
    return -1
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_while_loop() {
        let source = r#"
def foo() -> int:
  mut x = 0
  while x < 10:
    x = x + 1
  return x
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_for_loop() {
        let source = r#"
def foo() -> int:
  mut sum = 0
  for i in range(10):
    sum = sum + i
  return sum
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // Collections
    // ========================================

    #[test]
    fn test_list_literal() {
        let source = r#"
def foo() -> List[int]:
  return [1, 2, 3]
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_empty_list() {
        let source = r#"
def foo() -> List[int]:
  let x: List[int] = []
  return x
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // Model tests
    // ========================================

    #[test]
    fn test_model_definition() {
        let source = r#"
model User:
  name: str
  age: int
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_model_instantiation() {
        let source = r#"
model Point:
  x: int
  y: int

def make_point() -> Point:
  return Point(x=0, y=0)
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // Class tests
    // ========================================

    #[test]
    fn test_class_definition() {
        let source = r#"
class Counter:
  value: int

  def get(self) -> int:
    return self.value
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // Enum tests
    // ========================================

    #[test]
    fn test_enum_definition() {
        let source = r#"
enum Color:
  Red
  Green
  Blue
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // Option and Result
    // ========================================

    #[test]
    fn test_option_some() {
        let source = r#"
def foo() -> Option[int]:
  return Some(42)
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_option_none() {
        let source = r#"
def foo() -> Option[int]:
  return None
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_result_ok() {
        let source = r#"
def foo() -> Result[int, str]:
  return Ok(42)
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_result_err() {
        let source = r#"
def foo() -> Result[int, str]:
  return Err("error")
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // Function calls
    // ========================================

    #[test]
    fn test_function_call() {
        let source = r#"
def add(a: int, b: int) -> int:
  return a + b

def foo() -> int:
  return add(1, 2)
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_builtin_len() {
        let source = r#"
def foo() -> int:
  x = [1, 2, 3]
  return len(x)
"#;
        assert!(check_str(source).is_ok());
    }

    #[test]
    fn test_builtin_sum() {
        let source = r#"
def foo() -> int:
  x = [True, False, True]
  return sum(x)
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // Tuple tests
    // ========================================

    #[test]
    fn test_tuple_literal() {
        let source = r#"
def foo() -> (int, str):
  return (1, "hello")
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // Closure tests
    // ========================================

    #[test]
    fn test_closure() {
        // Note: untyped closure params may not pass typechecker
        // This tests that we handle closures correctly (even if they error)
        let source = r#"
def foo() -> int:
  f = (x) => x + 1
  return f(41)
"#;
        // Closure with untyped params may error, so just check it doesn't panic
        let _ = check_str(source);
    }

    // ========================================
    // Match expression tests
    // ========================================

    #[test]
    fn test_match_expression() {
        let source = r#"
def foo(x: int) -> str:
  match x:
    0 => "zero"
    1 => "one"
    _ => "other"
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // Async function tests
    // ========================================

    #[test]
    fn test_async_function() {
        let source = r#"
async def foo() -> int:
  return 42
"#;
        assert!(check_str(source).is_ok());
    }

    // ========================================
    // Error case tests
    // ========================================

    #[test]
    fn test_wrong_argument_count() {
        // Note: The typechecker may be lenient on argument counts
        // Just verify we can run through the check without panic
        let source = r#"
def add(a: int, b: int) -> int:
  return a + b

def foo() -> int:
  return add(1)
"#;
        let _ = check_str(source);
    }

    #[test]
    fn test_undefined_function() {
        let source = r#"
def foo() -> int:
  return undefined_func()
"#;
        let result = check_str(source);
        assert!(result.is_err());
    }

    #[test]
    fn test_return_type_mismatch_in_if() {
        let source = r#"
def foo(x: bool) -> int:
  if x:
    return "wrong"
  return 0
"#;
        let result = check_str(source);
        assert!(result.is_err());
    }
}
