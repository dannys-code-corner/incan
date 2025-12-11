//! Declaration emission for code generation
//!
//! Handles emitting models, classes, traits, newtypes, and enums.

use std::collections::HashSet;

use crate::frontend::ast::*;
use crate::backend::rust_emitter::{RustEmitter, to_rust_ident};

use super::types::DunderMethods;
use super::RustCodegen;

impl RustCodegen<'_> {
    /// Emit a declaration
    pub(crate) fn emit_declaration(&mut self, decl: &Spanned<Declaration>) {
        match &decl.node {
            Declaration::Import(import) => self.emit_import(import),
            Declaration::Model(model) => self.emit_model(model),
            Declaration::Class(class) => self.emit_class(class),
            Declaration::Trait(tr) => self.emit_trait(tr),
            Declaration::Newtype(nt) => self.emit_newtype(nt),
            Declaration::Enum(en) => self.emit_enum(en),
            Declaration::Function(func) => self.emit_function(func),
            Declaration::Docstring(doc) => {
                for line in doc.lines() {
                    self.emitter.line(&format!("// {}", line));
                }
            }
        }
    }

    /// Emit a model (struct with derives)
    fn emit_model(&mut self, model: &ModelDecl) {
        let dunder_methods = Self::find_dunder_methods(&model.methods);

        // Collect derives from decorators and track Serialize/Deserialize
        let mut derives_set: HashSet<&str> = ["Debug", "Clone"].into_iter().collect();
        let mut has_serialize = false;
        let mut has_deserialize = false;
        
        for dec in &model.decorators {
            if dec.node.name == "derive" {
                for arg in &dec.node.args {
                    if let DecoratorArg::Positional(expr) = arg {
                        if let Expr::Ident(name) = &expr.node {
                            if name == "Serialize" {
                                has_serialize = true;
                            }
                            if name == "Deserialize" {
                                has_deserialize = true;
                            }
                            for rust_derive in self.derive_to_rust_vec(name) {
                                derives_set.insert(rust_derive);
                            }
                        }
                    }
                }
            }
        }

        // Remove derives overridden by dunder methods
        if dunder_methods.has_eq {
            derives_set.remove("PartialEq");
            derives_set.remove("Eq");
        }
        if dunder_methods.has_hash {
            derives_set.remove("Hash");
        }
        if dunder_methods.has_ord {
            derives_set.remove("PartialOrd");
            derives_set.remove("Ord");
        }

        let derives: Vec<&str> = derives_set.into_iter().collect();

        let fields: Vec<(String, String)> = model
            .fields
            .iter()
            .map(|f| {
                let rust_type = self.type_to_rust(&f.node.ty.node);
                (to_rust_ident(&f.node.name), rust_type)
            })
            .collect();

        self.emitter.struct_def(&derives, "pub", &model.name, &fields);

        let field_names: Vec<String> = model.fields.iter()
            .map(|f| f.node.name.clone())
            .collect();

        // Generate impl block for non-dunder methods plus reflection and JSON methods
        let regular_methods: Vec<_> = model.methods.iter()
            .filter(|m| !m.node.name.starts_with("__") || m.node.name == "__init__")
            .collect();

        self.emitter.blank_line();
        self.emitter.impl_block(None, &model.name, |e| {
            for method in &regular_methods {
                Self::emit_method_in_impl(e, &method.node);
            }
            Self::emit_reflection_methods(e, &model.name, &field_names);
            
            // Emit to_json() if model has Serialize derive
            if has_serialize {
                Self::emit_to_json_method(e);
            }
            
            // Emit from_json() if model has Deserialize derive
            if has_deserialize {
                Self::emit_from_json_method(e);
            }
        });

        // Generate trait implementations for dunder methods
        self.emit_dunder_trait_impls(&model.name, &model.methods, &dunder_methods);
    }

    /// Emit a class (struct + impl + trait impls)
    fn emit_class(&mut self, class: &ClassDecl) {
        let dunder_methods = Self::find_dunder_methods(&class.methods);

        let mut derives_set: HashSet<&str> = ["Debug", "Clone"].into_iter().collect();
        let mut has_serialize = false;
        let mut has_deserialize = false;
        
        for dec in &class.decorators {
            if dec.node.name == "derive" {
                for arg in &dec.node.args {
                    if let DecoratorArg::Positional(expr) = arg {
                        if let Expr::Ident(name) = &expr.node {
                            if name == "Serialize" {
                                has_serialize = true;
                            }
                            if name == "Deserialize" {
                                has_deserialize = true;
                            }
                            for rust_derive in self.derive_to_rust_vec(name) {
                                derives_set.insert(rust_derive);
                            }
                        }
                    }
                }
            }
        }

        if dunder_methods.has_eq {
            derives_set.remove("PartialEq");
            derives_set.remove("Eq");
        }
        if dunder_methods.has_hash {
            derives_set.remove("Hash");
        }
        if dunder_methods.has_ord {
            derives_set.remove("PartialOrd");
            derives_set.remove("Ord");
        }

        let derives: Vec<&str> = derives_set.into_iter().collect();

        // Collect fields including inherited
        let mut fields: Vec<(String, String)> = Vec::new();

        if let Some(parent_name) = &class.extends {
            if let Some(parent_class) = self.find_class(parent_name) {
                fields.extend(self.get_all_class_fields(parent_class));
            }
        }

        for f in &class.fields {
            let rust_type = self.type_to_rust(&f.node.ty.node);
            fields.push((to_rust_ident(&f.node.name), rust_type));
        }

        self.emitter.struct_def(&derives, "pub", &class.name, &fields);

        let field_names: Vec<String> = fields.iter()
            .map(|(name, _)| name.clone())
            .collect();

        let all_methods = self.get_all_class_methods(class);

        let trait_method_names: HashSet<String> = class.traits.iter()
            .flat_map(|t| self.get_trait_method_names(t))
            .collect();

        let struct_methods: Vec<_> = all_methods.iter()
            .filter(|m| !trait_method_names.contains(&m.name))
            .filter(|m| !Self::is_dunder_method(&m.name) || m.name == "__init__")
            .cloned()
            .collect::<Vec<_>>();

        let trait_methods_by_trait: Vec<(String, Vec<MethodDecl>)> = class.traits.iter()
            .map(|trait_name| {
                let methods: Vec<_> = all_methods.iter()
                    .filter(|m| self.get_trait_method_names(trait_name).contains(&m.name))
                    .cloned()
                    .collect();
                (trait_name.clone(), methods)
            })
            .collect();

        // Emit struct impl
        self.emitter.blank_line();
        self.emitter.impl_block(None, &class.name, |e| {
            for method in &struct_methods {
                Self::emit_method_in_impl(e, method);
            }
            Self::emit_reflection_methods(e, &class.name, &field_names);
            
            // Emit to_json() if class has Serialize derive
            if has_serialize {
                Self::emit_to_json_method(e);
            }
            
            // Emit from_json() if class has Deserialize derive
            if has_deserialize {
                Self::emit_from_json_method(e);
            }
        });

        // Emit trait impls
        for (trait_name, trait_methods) in &trait_methods_by_trait {
            self.emitter.blank_line();
            if trait_methods.is_empty() {
                self.emitter.line(&format!("impl {} for {} {{}}", trait_name, class.name));
            } else {
                self.emitter.impl_block(Some(trait_name), &class.name, |e| {
                    for method in trait_methods {
                        Self::emit_method_in_trait_impl(e, method);
                    }
                });
            }
        }

        for trait_name in &class.traits {
            let has_impl = trait_methods_by_trait.iter().any(|(name, _)| name == trait_name);
            if !has_impl {
                self.emitter.blank_line();
                self.emitter.line(&format!("impl {} for {} {{}}", trait_name, class.name));
            }
        }

        self.emit_dunder_trait_impls(&class.name, &class.methods, &dunder_methods);
    }

    /// Emit a trait
    fn emit_trait(&mut self, tr: &TraitDecl) {
        self.emitter.trait_def("pub", &tr.name, |e| {
            for method in &tr.methods {
                Self::emit_trait_method(e, &method.node);
            }
        });
    }

    /// Emit a newtype
    fn emit_newtype(&mut self, nt: &NewtypeDecl) {
        let inner_type = self.type_to_rust(&nt.underlying.node);
        self.emitter.newtype_def(
            &["Debug", "Clone", "PartialEq", "Eq"],
            "pub",
            &nt.name,
            &inner_type,
        );

        if !nt.methods.is_empty() {
            self.emitter.blank_line();
            self.emitter.impl_block(None, &nt.name, |e| {
                for method in &nt.methods {
                    Self::emit_method_in_impl(e, &method.node);
                }
            });
        }
    }

    /// Emit an enum
    fn emit_enum(&mut self, en: &EnumDecl) {
        let variants: Vec<(String, Vec<String>)> = en
            .variants
            .iter()
            .map(|v| {
                let field_types: Vec<String> = v
                    .node
                    .fields
                    .iter()
                    .map(|f| Self::type_to_rust_static(&f.node))
                    .collect();
                (v.node.name.clone(), field_types)
            })
            .collect();

        self.emitter.enum_def(
            &["Debug", "Clone", "PartialEq", "Eq"],
            "pub",
            &en.name,
            &variants,
        );
    }

    /// Emit reflection methods
    pub(crate) fn emit_reflection_methods(emitter: &mut RustEmitter, type_name: &str, field_names: &[String]) {
        emitter.blank_line();
        emitter.line("/// Returns the names of all fields in this type");
        emitter.line("pub fn __fields__(&self) -> Vec<&'static str> {");
        emitter.indent();
        let fields_str = field_names.iter()
            .map(|f| format!("\"{}\"", f))
            .collect::<Vec<_>>()
            .join(", ");
        emitter.line(&format!("vec![{}]", fields_str));
        emitter.dedent();
        emitter.line("}");

        emitter.blank_line();
        emitter.line("/// Returns the name of this type");
        emitter.line("pub fn __class_name__(&self) -> &'static str {");
        emitter.indent();
        emitter.line(&format!("\"{}\"", type_name));
        emitter.dedent();
        emitter.line("}");
    }

    /// Emit to_json() instance method for models with Serialize derive
    pub(crate) fn emit_to_json_method(emitter: &mut RustEmitter) {
        emitter.blank_line();
        emitter.line("/// Serialize this model to a JSON string");
        emitter.line("pub fn to_json(&self) -> String {");
        emitter.indent();
        emitter.line("serde_json::to_string(self).unwrap()");
        emitter.dedent();
        emitter.line("}");
    }

    /// Emit from_json() static method for models with Deserialize derive
    pub(crate) fn emit_from_json_method(emitter: &mut RustEmitter) {
        emitter.blank_line();
        emitter.line("/// Deserialize a JSON string into this model");
        emitter.line("pub fn from_json(json_str: &str) -> Result<Self, String> {");
        emitter.indent();
        emitter.line("serde_json::from_str(json_str).map_err(|e| e.to_string())");
        emitter.dedent();
        emitter.line("}");
    }

    /// Check if a method name is a dunder method
    pub(crate) fn is_dunder_method(name: &str) -> bool {
        name.starts_with("__") && name.ends_with("__") && name.len() > 4
    }

    /// Find which dunder methods are defined
    pub(crate) fn find_dunder_methods(methods: &[Spanned<MethodDecl>]) -> DunderMethods {
        let mut result = DunderMethods::new();

        for method in methods {
            match method.node.name.as_str() {
                "__eq__" | "__ne__" => result.has_eq = true,
                "__hash__" => result.has_hash = true,
                "__lt__" | "__le__" | "__gt__" | "__ge__" | "__cmp__" => result.has_ord = true,
                "__str__" => result.has_str = true,
                _ => {}
            }
        }

        result
    }

    /// Emit trait implementations for dunder methods
    fn emit_dunder_trait_impls(&mut self, type_name: &str, methods: &[Spanned<MethodDecl>], dunders: &DunderMethods) {
        let eq_method = methods.iter().find(|m| m.node.name == "__eq__");
        let hash_method = methods.iter().find(|m| m.node.name == "__hash__");
        let lt_method = methods.iter().find(|m| m.node.name == "__lt__");
        let str_method = methods.iter().find(|m| m.node.name == "__str__");

        // PartialEq impl
        if dunders.has_eq {
            if let Some(eq_m) = eq_method {
                self.emitter.blank_line();
                self.emitter.line(&format!("impl PartialEq for {} {{", type_name));
                self.emitter.indent();
                self.emitter.line("fn eq(&self, other: &Self) -> bool {");
                self.emitter.indent();
                if let Some(body) = &eq_m.node.body {
                    for stmt in body {
                        Self::emit_statement(&mut self.emitter, &stmt.node);
                    }
                }
                self.emitter.dedent();
                self.emitter.line("}");
                self.emitter.dedent();
                self.emitter.line("}");

                self.emitter.blank_line();
                self.emitter.line(&format!("impl Eq for {} {{}}", type_name));
            }
        }

        // Hash impl
        if dunders.has_hash {
            if let Some(hash_m) = hash_method {
                self.emitter.blank_line();
                self.emitter.impl_block(None, type_name, |e| {
                    Self::emit_method_in_impl(e, &hash_m.node);
                });

                self.emitter.blank_line();
                self.emitter.line(&format!("impl std::hash::Hash for {} {{", type_name));
                self.emitter.indent();
                self.emitter.line("fn hash<H: std::hash::Hasher>(&self, state: &mut H) {");
                self.emitter.indent();
                self.emitter.line("self.__hash__().hash(state);");
                self.emitter.dedent();
                self.emitter.line("}");
                self.emitter.dedent();
                self.emitter.line("}");
            }
        }

        // PartialOrd/Ord impl
        if dunders.has_ord {
            if let Some(lt_m) = lt_method {
                self.emitter.blank_line();
                self.emitter.line(&format!("impl PartialOrd for {} {{", type_name));
                self.emitter.indent();
                self.emitter.line("fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {");
                self.emitter.indent();
                self.emitter.line("Some(self.cmp(other))");
                self.emitter.dedent();
                self.emitter.line("}");
                self.emitter.dedent();
                self.emitter.line("}");

                self.emitter.blank_line();
                self.emitter.line(&format!("impl Ord for {} {{", type_name));
                self.emitter.indent();
                self.emitter.line("fn cmp(&self, other: &Self) -> std::cmp::Ordering {");
                self.emitter.indent();
                self.emitter.line("// Using __lt__ for comparison");
                self.emitter.line("if self.__lt__(other) { std::cmp::Ordering::Less }");
                self.emitter.line("else if other.__lt__(self) { std::cmp::Ordering::Greater }");
                self.emitter.line("else { std::cmp::Ordering::Equal }");
                self.emitter.dedent();
                self.emitter.line("}");
                self.emitter.dedent();
                self.emitter.line("}");

                self.emitter.blank_line();
                self.emitter.impl_block(None, type_name, |e| {
                    Self::emit_method_in_impl(e, &lt_m.node);
                });
            }
        }

        // Display impl for __str__
        if dunders.has_str {
            if let Some(str_m) = str_method {
                self.emitter.blank_line();
                self.emitter.line(&format!("impl std::fmt::Display for {} {{", type_name));
                self.emitter.indent();
                self.emitter.line("fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {");
                self.emitter.indent();
                self.emitter.line("write!(f, \"{}\", self.__str__())");
                self.emitter.dedent();
                self.emitter.line("}");
                self.emitter.dedent();
                self.emitter.line("}");

                self.emitter.blank_line();
                self.emitter.impl_block(None, type_name, |e| {
                    Self::emit_method_in_impl(e, &str_m.node);
                });
            }
        }
    }

    /// Find a class declaration by name
    pub(crate) fn find_class(&self, class_name: &str) -> Option<&ClassDecl> {
        if let Some(program) = &self.current_program {
            for decl in &program.declarations {
                if let Declaration::Class(class) = &decl.node {
                    if class.name == class_name {
                        return Some(class);
                    }
                }
            }
        }
        None
    }

    /// Get all fields for a class including inherited
    pub(crate) fn get_all_class_fields(&self, class: &ClassDecl) -> Vec<(String, String)> {
        let mut fields = Vec::new();

        if let Some(parent_name) = &class.extends {
            if let Some(parent_class) = self.find_class(parent_name) {
                fields.extend(self.get_all_class_fields(parent_class));
            }
        }

        for f in &class.fields {
            let rust_type = self.type_to_rust(&f.node.ty.node);
            fields.push((to_rust_ident(&f.node.name), rust_type));
        }

        fields
    }

    /// Get all methods for a class including inherited
    pub(crate) fn get_all_class_methods(&self, class: &ClassDecl) -> Vec<MethodDecl> {
        let mut methods: std::collections::HashMap<String, MethodDecl> = std::collections::HashMap::new();

        if let Some(parent_name) = &class.extends {
            if let Some(parent_class) = self.find_class(parent_name) {
                for method in self.get_all_class_methods(parent_class) {
                    methods.insert(method.name.clone(), method);
                }
            }
        }

        for m in &class.methods {
            methods.insert(m.node.name.clone(), m.node.clone());
        }

        methods.into_values().collect()
    }

    /// Get method names defined in a trait
    pub(crate) fn get_trait_method_names(&self, trait_name: &str) -> HashSet<String> {
        if let Some(program) = &self.current_program {
            for decl in &program.declarations {
                if let Declaration::Trait(tr) = &decl.node {
                    if tr.name == trait_name {
                        return tr.methods.iter().map(|m| m.node.name.clone()).collect();
                    }
                }
            }
        }
        HashSet::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::codegen::RustCodegen;

    fn make_spanned<T>(node: T) -> Spanned<T> {
        Spanned { node, span: Span::default() }
    }

    // ========================================
    // DunderMethods tests
    // ========================================

    #[test]
    fn test_find_dunder_methods_empty() {
        let methods: Vec<Spanned<MethodDecl>> = vec![];
        let dunder = RustCodegen::find_dunder_methods(&methods);
        assert!(!dunder.has_eq);
        assert!(!dunder.has_hash);
        assert!(!dunder.has_ord);
        assert!(!dunder.has_str);
    }

    #[test]
    fn test_find_dunder_methods_eq() {
        let methods = vec![
            make_spanned(MethodDecl {
                decorators: vec![],
                is_async: false,
                name: "__eq__".to_string(),
                receiver: Some(Receiver::Immutable),
                params: vec![],
                return_type: make_spanned(Type::Simple("bool".to_string())),
                body: None,
            }),
        ];
        let dunder = RustCodegen::find_dunder_methods(&methods);
        assert!(dunder.has_eq);
        assert!(!dunder.has_hash);
    }

    #[test]
    fn test_find_dunder_methods_hash() {
        let methods = vec![
            make_spanned(MethodDecl {
                decorators: vec![],
                is_async: false,
                name: "__hash__".to_string(),
                receiver: Some(Receiver::Immutable),
                params: vec![],
                return_type: make_spanned(Type::Simple("int".to_string())),
                body: None,
            }),
        ];
        let dunder = RustCodegen::find_dunder_methods(&methods);
        assert!(dunder.has_hash);
    }

    #[test]
    fn test_find_dunder_methods_ord() {
        let methods = vec![
            make_spanned(MethodDecl {
                decorators: vec![],
                is_async: false,
                name: "__lt__".to_string(),
                receiver: Some(Receiver::Immutable),
                params: vec![],
                return_type: make_spanned(Type::Simple("bool".to_string())),
                body: None,
            }),
        ];
        let dunder = RustCodegen::find_dunder_methods(&methods);
        assert!(dunder.has_ord);
    }

    #[test]
    fn test_find_dunder_methods_str() {
        let methods = vec![
            make_spanned(MethodDecl {
                decorators: vec![],
                is_async: false,
                name: "__str__".to_string(),
                receiver: Some(Receiver::Immutable),
                params: vec![],
                return_type: make_spanned(Type::Simple("str".to_string())),
                body: None,
            }),
        ];
        let dunder = RustCodegen::find_dunder_methods(&methods);
        assert!(dunder.has_str);
    }

    #[test]
    fn test_find_dunder_methods_repr() {
        // __repr__ is NOT the same as __str__ in the current implementation
        // Only __str__ triggers has_str
        let methods = vec![
            make_spanned(MethodDecl {
                decorators: vec![],
                is_async: false,
                name: "__repr__".to_string(),
                receiver: Some(Receiver::Immutable),
                params: vec![],
                return_type: make_spanned(Type::Simple("str".to_string())),
                body: None,
            }),
        ];
        let dunder = RustCodegen::find_dunder_methods(&methods);
        // __repr__ doesn't set has_str
        assert!(!dunder.has_str);
    }

    #[test]
    fn test_find_dunder_methods_multiple() {
        let methods = vec![
            make_spanned(MethodDecl {
                decorators: vec![],
                is_async: false,
                name: "__eq__".to_string(),
                receiver: Some(Receiver::Immutable),
                params: vec![],
                return_type: make_spanned(Type::Simple("bool".to_string())),
                body: None,
            }),
            make_spanned(MethodDecl {
                decorators: vec![],
                is_async: false,
                name: "__hash__".to_string(),
                receiver: Some(Receiver::Immutable),
                params: vec![],
                return_type: make_spanned(Type::Simple("int".to_string())),
                body: None,
            }),
            make_spanned(MethodDecl {
                decorators: vec![],
                is_async: false,
                name: "__str__".to_string(),
                receiver: Some(Receiver::Immutable),
                params: vec![],
                return_type: make_spanned(Type::Simple("str".to_string())),
                body: None,
            }),
        ];
        let dunder = RustCodegen::find_dunder_methods(&methods);
        assert!(dunder.has_eq);
        assert!(dunder.has_hash);
        assert!(dunder.has_str);
        assert!(!dunder.has_ord);
    }

    // ========================================
    // RustCodegen declaration tests
    // ========================================

    #[test]
    fn test_emit_model_simple() {
        let mut codegen = RustCodegen::new();
        let model = ModelDecl {
            decorators: vec![],
            name: "User".to_string(),
            type_params: vec![],
            fields: vec![
                make_spanned(FieldDecl {
                    name: "name".to_string(),
                    ty: make_spanned(Type::Simple("str".to_string())),
                    default: None,
                }),
                make_spanned(FieldDecl {
                    name: "age".to_string(),
                    ty: make_spanned(Type::Simple("int".to_string())),
                    default: None,
                }),
            ],
            methods: vec![],
        };
        codegen.emit_model(&model);
        let output = codegen.emitter.finish();
        assert!(output.contains("struct User"));
        assert!(output.contains("pub name: String"));
        assert!(output.contains("pub age: i64"));
        assert!(output.contains("#[derive("));
    }

    #[test]
    fn test_emit_enum_simple() {
        let mut codegen = RustCodegen::new();
        let enum_decl = EnumDecl {
            name: "Color".to_string(),
            type_params: vec![],
            variants: vec![
                make_spanned(VariantDecl {
                    name: "Red".to_string(),
                    fields: vec![],
                }),
                make_spanned(VariantDecl {
                    name: "Green".to_string(),
                    fields: vec![],
                }),
                make_spanned(VariantDecl {
                    name: "Blue".to_string(),
                    fields: vec![],
                }),
            ],
        };
        codegen.emit_enum(&enum_decl);
        let output = codegen.emitter.finish();
        assert!(output.contains("enum Color"));
        assert!(output.contains("Red"));
        assert!(output.contains("Green"));
        assert!(output.contains("Blue"));
    }

    #[test]
    fn test_emit_enum_with_fields() {
        let mut codegen = RustCodegen::new();
        let enum_decl = EnumDecl {
            name: "Message".to_string(),
            type_params: vec![],
            variants: vec![
                make_spanned(VariantDecl {
                    name: "Text".to_string(),
                    fields: vec![make_spanned(Type::Simple("str".to_string()))],
                }),
                make_spanned(VariantDecl {
                    name: "Number".to_string(),
                    fields: vec![make_spanned(Type::Simple("int".to_string()))],
                }),
            ],
        };
        codegen.emit_enum(&enum_decl);
        let output = codegen.emitter.finish();
        assert!(output.contains("enum Message"));
        assert!(output.contains("Text(String)"));
        assert!(output.contains("Number(i64)"));
    }

    #[test]
    fn test_emit_newtype_simple() {
        let mut codegen = RustCodegen::new();
        let newtype = NewtypeDecl {
            name: "UserId".to_string(),
            underlying: make_spanned(Type::Simple("int".to_string())),
            methods: vec![],
        };
        codegen.emit_newtype(&newtype);
        let output = codegen.emitter.finish();
        assert!(output.contains("struct UserId"));
        assert!(output.contains("i64"));
    }

    #[test]
    fn test_emit_trait_empty() {
        let mut codegen = RustCodegen::new();
        let trait_decl = TraitDecl {
            decorators: vec![],
            name: "Printable".to_string(),
            type_params: vec![],
            methods: vec![],
        };
        codegen.emit_trait(&trait_decl);
        let output = codegen.emitter.finish();
        assert!(output.contains("trait Printable"));
    }

    #[test]
    fn test_emit_trait_with_method() {
        let mut codegen = RustCodegen::new();
        let trait_decl = TraitDecl {
            decorators: vec![],
            name: "Describable".to_string(),
            type_params: vec![],
            methods: vec![
                make_spanned(MethodDecl {
                    decorators: vec![],
                    is_async: false,
                    name: "describe".to_string(),
                    receiver: Some(Receiver::Immutable),
                    params: vec![],
                    return_type: make_spanned(Type::Simple("str".to_string())),
                    body: None,
                }),
            ],
        };
        codegen.emit_trait(&trait_decl);
        let output = codegen.emitter.finish();
        assert!(output.contains("trait Describable"));
        assert!(output.contains("fn describe"));
    }

    #[test]
    fn test_emit_class_simple() {
        let mut codegen = RustCodegen::new();
        let class = ClassDecl {
            decorators: vec![],
            name: "Counter".to_string(),
            type_params: vec![],
            extends: None,
            traits: vec![],
            fields: vec![
                make_spanned(FieldDecl {
                    name: "value".to_string(),
                    ty: make_spanned(Type::Simple("int".to_string())),
                    default: None,
                }),
            ],
            methods: vec![],
        };
        codegen.emit_class(&class);
        let output = codegen.emitter.finish();
        assert!(output.contains("struct Counter"));
        assert!(output.contains("pub value: i64"));
    }

    #[test]
    fn test_emit_docstring() {
        let mut codegen = RustCodegen::new();
        let decl = make_spanned(Declaration::Docstring("This is a docstring.\nSecond line.".to_string()));
        codegen.emit_declaration(&decl);
        let output = codegen.emitter.finish();
        assert!(output.contains("// This is a docstring."));
        assert!(output.contains("// Second line."));
    }

    // ========================================
    // get_trait_method_names tests
    // ========================================

    #[test]
    fn test_get_trait_method_names_no_program() {
        let codegen = RustCodegen::new();
        let names = codegen.get_trait_method_names("SomeTrait");
        assert!(names.is_empty());
    }

    // ========================================
    // derive_to_rust_vec tests
    // ========================================

    #[test]
    fn test_derive_to_rust_vec_serialize() {
        let codegen = RustCodegen::new();
        let derives = codegen.derive_to_rust_vec("Serialize");
        assert!(derives.contains(&"serde::Serialize"));
    }

    #[test]
    fn test_derive_to_rust_vec_deserialize() {
        let codegen = RustCodegen::new();
        let derives = codegen.derive_to_rust_vec("Deserialize");
        assert!(derives.contains(&"serde::Deserialize"));
    }

    #[test]
    fn test_derive_to_rust_vec_default() {
        let codegen = RustCodegen::new();
        let derives = codegen.derive_to_rust_vec("Default");
        assert!(derives.contains(&"Default"));
    }

    #[test]
    fn test_derive_to_rust_vec_hash() {
        let codegen = RustCodegen::new();
        let derives = codegen.derive_to_rust_vec("Hash");
        assert!(derives.contains(&"Hash"));
    }

    #[test]
    fn test_derive_to_rust_vec_eq() {
        let codegen = RustCodegen::new();
        let derives = codegen.derive_to_rust_vec("Eq");
        assert!(derives.contains(&"Eq"));
        assert!(derives.contains(&"PartialEq"));
    }

    #[test]
    fn test_derive_to_rust_vec_ord() {
        let codegen = RustCodegen::new();
        let derives = codegen.derive_to_rust_vec("Ord");
        assert!(derives.contains(&"Ord"));
        assert!(derives.contains(&"PartialOrd"));
        assert!(derives.contains(&"Eq"));
        assert!(derives.contains(&"PartialEq"));
    }
}
