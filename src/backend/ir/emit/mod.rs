//! IR (Intermediate Representation) to Rust code emission using `syn`/`prettyplease`/`quote!`.
//!
//! This module converts the typed IR to Rust source code using syn's TokenStream representation.
//! This approach provides:
//!
//! - **Correct syntax**: `syn` validates the output is valid Rust
//! - **Pretty formatting**: `prettyplease` formats the output
//! - **No string escaping bugs**: `quote!` handles escaping automatically
//!
//! ## Module Structure
//!
//! - `mod.rs` - IrEmitter struct, public API, import tracking
//! - `types.rs` - Type, visibility, operator, and pattern emission
//! - `statements.rs` - Statement emission
//! - `expressions/` - Expression emission (split into focused submodules)
//!   - `mod.rs` - Main `emit_expr` entry point
//!   - `builtins.rs` - Built-in function calls
//!   - `methods.rs` - Method calls
//!   - `calls.rs` - Regular function calls and binary operations
//!   - `indexing.rs` - Index, slice, and field access
//!   - `comprehensions.rs` - List and dict comprehensions
//!   - `structs_enums.rs` - Struct constructors
//!   - `format.rs` - Format strings and range expressions
//!   - `lvalue.rs` - Assignment targets
//!
//! ## Type Conversions
//!
//! All type conversions (String borrowing, `.to_string()`, etc.) are handled by the centralized
//! [`conversions`](super::conversions) module.
//! See the conversions module documentation for details on conversion rules.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use incan::backend::ir::emit::IrEmitter;
//!
//! let emitter = IrEmitter::new();
//! let rust_code = emitter.emit_program(&ir_program)?;
//! ```

mod expressions;
mod statements;
mod types;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::cell::RefCell;
use syn;

use super::decl::IrDeclKind;
use super::stmt::IrStmtKind;
use super::types::{IrType, Mutability};
use super::*;

/// IR to Rust code emitter
pub struct IrEmitter<'a> {
    /// Whether to add clippy allows (should be false for warning-free codegen)
    add_clippy_allows: bool,
    /// Whether to emit the Zen of Incan in main
    emit_zen_in_main: bool,
    /// Whether serde is needed (for Serialize/Deserialize derives)
    needs_serde: bool,
    /// Whether tokio is needed (for async runtime)
    needs_tokio: bool,
    /// Whether axum web framework is needed
    needs_axum: bool,
    /// Function registry for call-site type checking
    function_registry: &'a FunctionRegistry,
    /// Track struct derives for generating serde methods in impl blocks
    struct_derives: std::collections::HashMap<String, Vec<String>>,
    /// Current function's return type (for applying conversions in return statements)
    current_function_return_type: RefCell<Option<IrType>>,
    /// Functions imported from external Rust crates
    external_rust_functions: std::collections::HashSet<String>,
    /// Enum variant field typing lookup: (EnumName, VariantName) -> VariantFields
    enum_variant_fields: std::collections::HashMap<(String, String), super::decl::VariantFields>,
    /// Struct field type lookup: (StructName, FieldName) -> IrType
    struct_field_types: std::collections::HashMap<(String, String), IrType>,
    /// Whether we're currently emitting a return expression (allows moves instead of clones)
    in_return_context: RefCell<bool>,
}

/// Import tracking for warning-free codegen
#[derive(Default)]
struct ImportTracker {
    needs_hashmap: bool,
    needs_hashset: bool,
}

impl ImportTracker {
    // scan the program for imports
    fn scan_program(&mut self, program: &IrProgram) {
        for decl in &program.declarations {
            self.scan_decl(decl);
        }
    }

    // scan a declaration for imports
    fn scan_decl(&mut self, decl: &IrDecl) {
        match &decl.kind {
            IrDeclKind::Function(f) => self.scan_function(f),
            IrDeclKind::Impl(impl_block) => {
                for method in &impl_block.methods {
                    self.scan_function(method);
                }
            }
            _ => {}
        }
    }

    // scan a function for imports
    fn scan_function(&mut self, f: &IrFunction) {
        for stmt in &f.body {
            self.scan_stmt(stmt);
        }
    }

    // scan a statement for imports
    fn scan_stmt(&mut self, stmt: &IrStmt) {
        match &stmt.kind {
            IrStmtKind::Let { value, .. } => self.scan_expr(value),
            IrStmtKind::Expr(e) => self.scan_expr(e),
            IrStmtKind::Return(Some(e)) => self.scan_expr(e),
            IrStmtKind::Assign { value, .. } => self.scan_expr(value),
            IrStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.scan_expr(condition);
                for s in then_branch {
                    self.scan_stmt(s);
                }
                if let Some(else_stmts) = else_branch {
                    for s in else_stmts {
                        self.scan_stmt(s);
                    }
                }
            }
            IrStmtKind::While {
                condition, body, ..
            } => {
                self.scan_expr(condition);
                for s in body {
                    self.scan_stmt(s);
                }
            }
            IrStmtKind::For { iterable, body, .. } => {
                self.scan_expr(iterable);
                for s in body {
                    self.scan_stmt(s);
                }
            }
            IrStmtKind::Match { scrutinee, arms } => {
                self.scan_expr(scrutinee);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.scan_expr(guard);
                    }
                    self.scan_expr(&arm.body);
                }
            }
            _ => {}
        }
    }

    // scan an expression for imports
    fn scan_expr(&mut self, expr: &TypedExpr) {
        match &expr.kind {
            IrExprKind::Dict(pairs) => {
                self.needs_hashmap = true;
                for (k, v) in pairs {
                    self.scan_expr(k);
                    self.scan_expr(v);
                }
            }
            IrExprKind::Set(items) => {
                self.needs_hashset = true;
                for item in items {
                    self.scan_expr(item);
                }
            }
            IrExprKind::List(items) => {
                for item in items {
                    self.scan_expr(item);
                }
            }
            IrExprKind::Call { func, args } => {
                self.scan_expr(func);
                for arg in args {
                    self.scan_expr(arg);
                }
            }
            IrExprKind::BuiltinCall { args, .. } => {
                for arg in args {
                    self.scan_expr(arg);
                }
            }
            IrExprKind::MethodCall { receiver, args, .. } => {
                self.scan_expr(receiver);
                for arg in args {
                    self.scan_expr(arg);
                }
            }
            IrExprKind::KnownMethodCall { receiver, args, .. } => {
                self.scan_expr(receiver);
                for arg in args {
                    self.scan_expr(arg);
                }
            }
            IrExprKind::BinOp { left, right, .. } => {
                self.scan_expr(left);
                self.scan_expr(right);
            }
            IrExprKind::UnaryOp { operand, .. } => self.scan_expr(operand),
            IrExprKind::Index { object, index } => {
                self.scan_expr(object);
                self.scan_expr(index);
            }
            IrExprKind::Field { object, .. } => self.scan_expr(object),
            IrExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.scan_expr(condition);
                self.scan_expr(then_branch);
                if let Some(e) = else_branch {
                    self.scan_expr(e);
                }
            }
            IrExprKind::Block { stmts, value } => {
                for s in stmts {
                    self.scan_stmt(s);
                }
                if let Some(v) = value {
                    self.scan_expr(v);
                }
            }
            IrExprKind::Struct { fields, .. } => {
                for (_, e) in fields {
                    self.scan_expr(e);
                }
            }
            _ => {}
        }
    }
}

impl<'a> IrEmitter<'a> {
    pub fn new(function_registry: &'a FunctionRegistry) -> Self {
        Self {
            // Enable minimal allows for patterns that can't easily be made warning-free:
            // - dead_code: library modules export functions that may not be used by main
            // - unused_imports: user imports may not all be used
            // - unused_variables: pattern bindings like `_x` in destructuring
            add_clippy_allows: true,
            emit_zen_in_main: false,
            needs_serde: false,
            needs_tokio: false,
            needs_axum: false,
            function_registry,
            struct_derives: std::collections::HashMap::new(),
            current_function_return_type: RefCell::new(None),
            external_rust_functions: std::collections::HashSet::new(),
            enum_variant_fields: std::collections::HashMap::new(),
            struct_field_types: std::collections::HashMap::new(),
            in_return_context: RefCell::new(false),
        }
    }

    /// Set external rust functions
    pub fn set_external_rust_functions(&mut self, funcs: std::collections::HashSet<String>) {
        self.external_rust_functions = funcs;
    }

    /// Set whether serde is needed
    pub fn set_needs_serde(&mut self, needs: bool) {
        self.needs_serde = needs;
    }

    /// Set whether tokio is needed
    pub fn set_needs_tokio(&mut self, needs: bool) {
        self.needs_tokio = needs;
    }

    /// Set whether axum is needed
    pub fn set_needs_axum(&mut self, needs: bool) {
        self.needs_axum = needs;
    }

    /// Escape Rust keywords by adding r# prefix
    /// Note: self and Self cannot be raw identifiers
    fn escape_keyword(name: &str) -> String {
        match name {
            // Special cases that cannot be raw identifiers
            "self" | "Self" => name.to_string(),
            // Strict keywords
            "as" | "break" | "const" | "continue" | "crate" | "else" | "enum" | "extern" |
            "false" | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop" | "match" |
            "mod" | "move" | "mut" | "pub" | "ref" | "return" |
            "static" | "struct" | "super" | "trait" | "true" | "type" | "unsafe" |
            "use" | "where" | "while" | "async" | "await" | "dyn" |
            // Reserved keywords
            "abstract" | "become" | "box" | "do" | "final" | "macro" | "override" |
            "priv" | "typeof" | "unsized" | "virtual" | "yield" | "try" => {
                format!("r#{}", name)
            }
            _ => name.to_string()
        }
    }

    /// Disable clippy allows (for strict warning-free codegen)
    pub fn without_clippy_allows(mut self) -> Self {
        self.add_clippy_allows = false;
        self
    }

    /// Set whether to emit the Zen of Incan in main
    pub fn set_emit_zen(&mut self, emit: bool) {
        self.emit_zen_in_main = emit;
    }

    /// Emit a complete IR program to formatted Rust code
    #[tracing::instrument(skip_all, fields(decl_count = program.declarations.len()))]
    pub fn emit_program(&mut self, program: &IrProgram) -> Result<String, EmitError> {
        // First pass: collect struct derives, struct field types, and enum variant typing
        for decl in &program.declarations {
            if let super::decl::IrDeclKind::Struct(s) = &decl.kind {
                // Collect derives for serde method generation
                if !s.derives.is_empty() {
                    self.struct_derives
                        .insert(s.name.clone(), s.derives.clone());
                }
                // Collect field types for conversion targeting
                for field in &s.fields {
                    self.struct_field_types
                        .insert((s.name.clone(), field.name.clone()), field.ty.clone());
                }
            }
            if let super::decl::IrDeclKind::Enum(e) = &decl.kind {
                for v in &e.variants {
                    self.enum_variant_fields
                        .insert((e.name.clone(), v.name.clone()), v.fields.clone());
                }
            }
        }

        let tokens = self.emit_program_tokens(program)?;
        let syntax_tree = syn::parse2(tokens).map_err(|e| EmitError::SynParse(e.to_string()))?;
        let formatted = prettyplease::unparse(&syntax_tree);

        // Prepend version header, inner attributes, then mod insertion marker
        // Inner attributes (#![...]) must come before any items, including mod declarations
        // So we structure it as: header, #![allow()], marker, imports...
        let header = "// Generated by the Incan compiler v0.1.0-alpha.1\n\n";

        // Find the end of the inner attribute block and insert marker after it
        // Look for "]\nuse " which marks the end of inner attrs and start of imports
        let with_marker = if formatted.contains("]\nuse ") {
            formatted.replacen("]\nuse ", "]\n\n// __INCAN_INSERT_MODS__\n\nuse ", 1)
        } else if formatted.contains("]\n\nuse ") {
            formatted.replacen("]\n\nuse ", "]\n\n// __INCAN_INSERT_MODS__\n\nuse ", 1)
        } else {
            // No use statement, add marker after the allow block
            formatted.replacen("]\n", "]\n\n// __INCAN_INSERT_MODS__\n\n", 1)
        };

        Ok(format!("{}{}", header, with_marker))
    }

    /// Emit a program to TokenStream (without formatting)
    pub fn emit_program_tokens(&self, program: &IrProgram) -> Result<TokenStream, EmitError> {
        let mut items = Vec::new();

        // Add crate-level attributes (version header and mod marker added in emit_program after formatting)
        // These are minimal allows for patterns that can't easily be made warning-free:
        // - dead_code: library modules export functions that may not be used by main
        // - unused_imports: user imports may not all be used (we track HashMap/HashSet but not user imports)
        // - unused_variables: pattern bindings in destructuring
        // Note: unused_parens and unused_mut have been fixed in codegen
        if self.add_clippy_allows {
            items.push(quote! {
                #![allow(unused_imports, dead_code, unused_variables)]
            });
        }

        // Track which imports are needed
        let mut tracker = ImportTracker::default();
        tracker.scan_program(program);

        // Always add stdlib prelude for reflection and helpers
        items.push(quote! { use incan_stdlib::prelude::*; });

        // Add derive macros from incan_derive (proc macros can't be re-exported through use *)
        items.push(quote! { use incan_derive::{FieldInfo, IncanClass}; });

        // Only emit imports that are actually used
        match (tracker.needs_hashmap, tracker.needs_hashset) {
            (true, true) => items.push(quote! { use std::collections::{HashMap, HashSet}; }),
            (true, false) => items.push(quote! { use std::collections::HashMap; }),
            (false, true) => items.push(quote! { use std::collections::HashSet; }),
            (false, false) => {} // No imports needed
        }

        // Add serde imports if needed
        if self.needs_serde {
            items.push(quote! { use serde::{Serialize, Deserialize}; });
        }

        // Add tokio imports if needed
        if self.needs_tokio {
            items.push(quote! { use tokio::time::{sleep, timeout, Duration}; });
            items.push(quote! { use tokio::sync::{mpsc, Mutex, RwLock}; });
            items.push(quote! { use tokio::task::JoinHandle; });
        }

        // Add axum imports if needed
        if self.needs_axum {
            items.push(quote! {
                use axum::{
                    Router,
                    routing::{get, post, put, delete, patch},
                    Json,
                    response::{Html, IntoResponse, Response},
                    extract::{Path, Query, State}
                };
            });
            items.push(quote! { use std::net::SocketAddr; });
        }

        // Emit each declaration
        for decl in &program.declarations {
            let item = self.emit_decl(decl)?;
            items.push(item);
        }

        Ok(quote! {
            #(#items)*
        })
    }

    /// Emit a declaration
    fn emit_decl(&self, decl: &IrDecl) -> Result<TokenStream, EmitError> {
        match &decl.kind {
            IrDeclKind::Function(func) => self.emit_function(func),
            IrDeclKind::Struct(s) => self.emit_struct(s),
            IrDeclKind::Enum(e) => self.emit_enum(e),
            IrDeclKind::TypeAlias { name, ty } => {
                let name_ident = format_ident!("{}", name);
                let ty_tokens = self.emit_type(ty);
                Ok(quote! {
                    type #name_ident = #ty_tokens;
                })
            }
            IrDeclKind::Const { name, ty, value } => {
                let name_ident = format_ident!("{}", name);
                let ty_tokens = self.emit_type(ty);
                let value_tokens = self.emit_expr(value)?;
                Ok(quote! {
                    const #name_ident: #ty_tokens = #value_tokens;
                })
            }
            IrDeclKind::Import { path, alias, items } => {
                // Skip serde imports if we're already importing them automatically
                if self.needs_serde && path.len() == 1 && path[0] == "serde" {
                    // Check if importing Serialize or Deserialize
                    let is_serde_trait = items
                        .iter()
                        .any(|item| item.name == "Serialize" || item.name == "Deserialize");
                    if is_serde_trait {
                        return Ok(quote! {}); // Skip duplicate import
                    }
                }

                let path_tokens: Vec<_> = path.iter().map(|s| format_ident!("{}", s)).collect();

                if let Some(alias_name) = alias {
                    // `import x as y` or `import a::b as y`
                    let alias_ident = format_ident!("{}", alias_name);
                    Ok(quote! {
                        use #(#path_tokens)::* as #alias_ident;
                    })
                } else if !items.is_empty() {
                    // `from x import a, b` or `from x import a as b`
                    // Handle items with potential aliases
                    let item_stmts: Vec<TokenStream> = items.iter().map(|item| {
                        let name_ident = format_ident!("{}", &item.name);
                        let path_tokens_clone = path_tokens.clone();
                        if let Some(alias) = &item.alias {
                            let alias_ident = format_ident!("{}", alias);
                            quote! { use #(#path_tokens_clone)::*::#name_ident as #alias_ident; }
                        } else {
                            quote! { use #(#path_tokens_clone)::*::#name_ident; }
                        }
                    }).collect();
                    Ok(quote! { #(#item_stmts)* })
                } else {
                    // Simple `import x` - emit as `use x::*;`
                    // But for local modules (single segment), skip entirely since mod is enough
                    if path.len() == 1 {
                        // Local module - skip use statement (mod is sufficient)
                        Ok(quote! {})
                    } else {
                        Ok(quote! {
                            use #(#path_tokens)::*;
                        })
                    }
                }
            }
            IrDeclKind::Impl(impl_block) => self.emit_impl(impl_block),
            IrDeclKind::Trait(trait_decl) => self.emit_trait(trait_decl),
        }
    }

    /// Emit a trait definition
    fn emit_trait(&self, trait_decl: &super::decl::IrTrait) -> Result<TokenStream, EmitError> {
        let name = format_ident!("{}", &trait_decl.name);

        // Emit default method implementations (without pub visibility)
        let methods: Vec<TokenStream> = trait_decl
            .methods
            .iter()
            .map(|m| self.emit_trait_method(m))
            .collect::<Result<_, _>>()?;

        Ok(quote! {
            pub trait #name {
                #(#methods)*
            }
        })
    }

    /// Emit a trait method (no visibility, may have default body or just signature)
    fn emit_trait_method(&self, func: &super::decl::IrFunction) -> Result<TokenStream, EmitError> {
        let name = format_ident!("{}", &func.name);

        let params: Vec<TokenStream> = func
            .params
            .iter()
            .map(|p| {
                if p.is_self {
                    match p.mutability {
                        Mutability::Mutable => quote! { &mut self },
                        Mutability::Immutable => quote! { &self },
                    }
                } else {
                    let pname = format_ident!("{}", &p.name);
                    let pty = self.emit_type(&p.ty);
                    quote! { #pname: #pty }
                }
            })
            .collect();

        let ret = match &func.return_type {
            IrType::Unit => quote! {},
            ty => {
                let t = self.emit_type(ty);
                quote! { -> #t }
            }
        };

        // If body is empty, emit signature only; otherwise emit default implementation
        if func.body.is_empty() {
            Ok(quote! {
                fn #name(#(#params),*) #ret;
            })
        } else {
            // Set return type context for conversions (like emit_method and emit_function)
            *self.current_function_return_type.borrow_mut() = Some(func.return_type.clone());

            let body_stmts: Vec<TokenStream> = func
                .body
                .iter()
                .map(|s| self.emit_stmt(s))
                .collect::<Result<_, _>>()?;

            // Clear context
            *self.current_function_return_type.borrow_mut() = None;

            Ok(quote! {
                fn #name(#(#params),*) #ret {
                    #(#body_stmts)*
                }
            })
        }
    }

    /// Emit an impl block
    fn emit_impl(&self, impl_block: &super::decl::IrImpl) -> Result<TokenStream, EmitError> {
        let target_type = format_ident!("{}", &impl_block.target_type);

        // Separate dunder methods that need special trait impls
        let mut regular_methods = Vec::new();
        let mut trait_impls = Vec::new();

        for method in &impl_block.methods {
            match method.name.as_str() {
                "__eq__" => {
                    // Generate impl PartialEq
                    let body_stmts: Vec<TokenStream> = method
                        .body
                        .iter()
                        .map(|s| self.emit_stmt(s))
                        .collect::<Result<_, _>>()?;
                    trait_impls.push(quote! {
                        impl PartialEq for #target_type {
                            fn eq(&self, other: &Self) -> bool {
                                #(#body_stmts)*
                            }
                        }
                    });
                }
                "__str__" => {
                    // Generate impl Display
                    // Keep __str__ as a regular method, then impl Display calls it
                    regular_methods.push(self.emit_method(method)?);
                    trait_impls.push(quote! {
                        impl std::fmt::Display for #target_type {
                            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                                write!(f, "{}", self.__str__())
                            }
                        }
                    });
                }
                "__class_name__" | "__fields__" => {
                    // Keep these as regular methods for now (reflection)
                    regular_methods.push(self.emit_method(method)?);
                }
                _ => {
                    regular_methods.push(self.emit_method(method)?);
                }
            }
        }

        // Add to_json/from_json for models with Serialize/Deserialize
        // OLD CODEGEN BEHAVIOR: Generate these methods when serde derives are present
        if impl_block.trait_name.is_none() {
            if let Some(derives) = self.struct_derives.get(&impl_block.target_type) {
                let has_serialize = derives.iter().any(|d| d == "Serialize");
                let has_deserialize = derives.iter().any(|d| d == "Deserialize");

                if has_serialize {
                    regular_methods.push(quote! {
                        /// Serialize this model to a JSON string
                        pub fn to_json(&self) -> String {
                            serde_json::to_string(self).expect("JSONError: failed to serialize to JSON")
                        }
                    });
                }

                if has_deserialize {
                    regular_methods.push(quote! {
                        /// Deserialize a JSON string into this model
                        pub fn from_json(json_str: String) -> Result<Self, String> {
                            serde_json::from_str(&json_str).map_err(|e| e.to_string())
                        }
                    });
                }
            }
        }

        // Build the main impl block
        let main_impl = if !regular_methods.is_empty() || impl_block.trait_name.is_none() {
            if let Some(trait_name) = &impl_block.trait_name {
                // Trait impl - re-emit methods without pub visibility
                let trait_methods: Vec<TokenStream> = impl_block
                    .methods
                    .iter()
                    .filter(|m| {
                        !matches!(
                            m.name.as_str(),
                            "__eq__" | "__str__" | "__class_name__" | "__fields__"
                        )
                    })
                    .map(|m| self.emit_trait_method(m))
                    .collect::<Result<_, _>>()?;
                let trait_ident = format_ident!("{}", trait_name);
                quote! {
                    impl #trait_ident for #target_type {
                        #(#trait_methods)*
                    }
                }
            } else if !regular_methods.is_empty() {
                quote! {
                    impl #target_type {
                        #(#regular_methods)*
                    }
                }
            } else {
                quote! {}
            }
        } else if let Some(trait_name) = &impl_block.trait_name {
            // Empty impl for a trait (uses defaults)
            let trait_ident = format_ident!("{}", trait_name);
            quote! {
                impl #trait_ident for #target_type {}
            }
        } else {
            quote! {}
        };

        // Combine main impl with trait impls
        Ok(quote! {
            #main_impl
            #(#trait_impls)*
        })
    }

    /// Emit a method (like a function but inside an impl block)
    fn emit_method(&self, func: &super::decl::IrFunction) -> Result<TokenStream, EmitError> {
        let name = format_ident!("{}", &func.name);
        let vis = self.emit_visibility(&func.visibility);

        let params: Vec<TokenStream> = func
            .params
            .iter()
            .map(|p| {
                if p.is_self {
                    // self parameter
                    match p.mutability {
                        Mutability::Mutable => quote! { &mut self },
                        Mutability::Immutable => quote! { &self },
                    }
                } else {
                    let pname = format_ident!("{}", &p.name);
                    let pty = self.emit_type(&p.ty);
                    match p.mutability {
                        Mutability::Mutable => quote! { mut #pname: #pty },
                        Mutability::Immutable => quote! { #pname: #pty },
                    }
                }
            })
            .collect();

        let ret = match &func.return_type {
            IrType::Unit => quote! {},
            ty => {
                let t = self.emit_type(ty);
                quote! { -> #t }
            }
        };

        // Set current function return type for return statement conversions
        *self.current_function_return_type.borrow_mut() = Some(func.return_type.clone());

        let body_stmts: Vec<TokenStream> = func
            .body
            .iter()
            .map(|s| self.emit_stmt(s))
            .collect::<Result<_, _>>()?;

        // Clear return type after method emission
        *self.current_function_return_type.borrow_mut() = None;

        Ok(quote! {
            #vis fn #name(#(#params),*) #ret {
                #(#body_stmts)*
            }
        })
    }

    /// Emit a function
    fn emit_function(&self, func: &super::decl::IrFunction) -> Result<TokenStream, EmitError> {
        let name = format_ident!("{}", &func.name);

        // main() is special: no pub, no return type
        let is_main = func.name == "main";
        let vis = if is_main {
            quote! {} // main is not pub
        } else {
            self.emit_visibility(&func.visibility)
        };

        let params: Vec<TokenStream> = func
            .params
            .iter()
            .map(|p| {
                let pname = format_ident!("{}", Self::escape_keyword(&p.name));
                let pty = self.emit_type(&p.ty);
                if p.is_self {
                    if matches!(p.mutability, Mutability::Mutable) {
                        quote! { &mut self }
                    } else {
                        quote! { &self }
                    }
                } else if matches!(p.mutability, Mutability::Mutable) {
                    // For `mut` parameters:
                    // - Copy types (int, float, bool): mutable binding, pass by value
                    // - Non-copy types: pass by mutable reference for in-place mutation
                    match &p.ty {
                        IrType::Int | IrType::Float | IrType::Bool => {
                            quote! { mut #pname: #pty }
                        }
                        _ => {
                            quote! { #pname: &mut #pty }
                        }
                    }
                } else {
                    // For non-mut parameters: pass by value (owned)
                    // OLD CODEGEN BEHAVIOR: Accept owned values, let caller use .clone()
                    // This matches format_function_params from old code
                    quote! { #pname: #pty }
                }
            })
            .collect();

        // Set current function return type for return statement conversions
        *self.current_function_return_type.borrow_mut() = Some(func.return_type.clone());

        let body_stmts: Vec<TokenStream> = func
            .body
            .iter()
            .map(|s| self.emit_stmt(s))
            .collect::<Result<_, _>>()?;

        // Clear return type after function emission
        *self.current_function_return_type.borrow_mut() = None;

        let async_kw = if func.is_async {
            quote! { async }
        } else {
            quote! {}
        };

        // For main, optionally emit the Zen of Incan at the start
        let zen_stmt = if is_main && self.emit_zen_in_main {
            let zen_text = r#"
┌──────────────────────────────────────────────────────────────────────┐
│  The Zen of Incan                                                    │
│  by Danny Meijer (inspired by Tim Peters' "The Zen of Python")       │
└──────────────────────────────────────────────────────────────────────┘

  › Readability counts          ─  clarity over cleverness
  › Safety over silence         ─  errors surface as Result, not hide
  › Explicit over implicit      ─  magic is opt-in and marked
  › Fast is better than slow    ─  performance costs must be visible
  › Namespaces are great        ─  keep modules and traits explicit
  › One obvious way             ─  conventions beat novelty,
                                   with escape hatches documented
"#; // FIXME: we should take this from the file rather than hardcoding - unless there is a reason to keep it here??!?
            quote! { println!(#zen_text); }
        } else {
            quote! {}
        };

        // Add #[tokio::main] attribute for async main
        let tokio_main_attr = if is_main && func.is_async && self.needs_tokio {
            quote! { #[tokio::main] }
        } else {
            quote! {}
        };

        // Omit return type for main and functions returning unit
        let ret_ty_is_unit = matches!(func.return_type, IrType::Unit);
        if is_main || ret_ty_is_unit {
            Ok(quote! {
                #tokio_main_attr
                #vis #async_kw fn #name(#(#params),*) {
                    #zen_stmt
                    #(#body_stmts)*
                }
            })
        } else {
            let ret_ty = self.emit_type(&func.return_type);
            Ok(quote! {
                #tokio_main_attr
                #vis #async_kw fn #name(#(#params),*) -> #ret_ty {
                    #(#body_stmts)*
                }
            })
        }
    }

    /// Emit a struct
    fn emit_struct(&self, s: &super::decl::IrStruct) -> Result<TokenStream, EmitError> {
        let name = format_ident!("{}", Self::escape_keyword(&s.name));
        let vis = self.emit_visibility(&s.visibility);

        let derives: Vec<TokenStream> = s
            .derives
            .iter()
            .map(|d| {
                // Use fully qualified paths for serde derives
                match d.as_str() {
                    "Serialize" => quote! { serde::Serialize },
                    "Deserialize" => quote! { serde::Deserialize },
                    _ => {
                        let d_ident = format_ident!("{}", d);
                        quote! { #d_ident }
                    }
                }
            })
            .collect();

        let derive_attr = if derives.is_empty() {
            quote! {}
        } else {
            quote! { #[derive(#(#derives),*)] }
        };

        // Check if this is a tuple struct (newtype) - field names are numeric
        let is_tuple_struct = !s.fields.is_empty()
            && s.fields
                .iter()
                .all(|f| f.name.chars().all(|c| c.is_ascii_digit()));

        if is_tuple_struct {
            // Emit as tuple struct: struct Name(pub Type1, pub Type2);
            let tuple_fields: Vec<TokenStream> = s
                .fields
                .iter()
                .map(|f| {
                    let fty = self.emit_type(&f.ty);
                    let fvis = self.emit_visibility(&f.visibility);
                    quote! { #fvis #fty }
                })
                .collect();

            Ok(quote! {
                #derive_attr
                #vis struct #name(#(#tuple_fields),*);
            })
        } else {
            // Emit as named struct
            let fields: Vec<TokenStream> = s
                .fields
                .iter()
                .map(|f| {
                    let fname = format_ident!("{}", &f.name);
                    let fty = self.emit_type(&f.ty);
                    let fvis = self.emit_visibility(&f.visibility);
                    quote! { #fvis #fname: #fty }
                })
                .collect();

            // Generate constructor function for models/classes
            // This allows both User(name, email, age) and User { name, email, age } syntax
            let constructor = if !s.fields.is_empty() {
                let param_tokens: Vec<TokenStream> = s
                    .fields
                    .iter()
                    .map(|f| {
                        let fname = format_ident!("{}", &f.name);
                        let fty = self.emit_type(&f.ty);
                        quote! { #fname: #fty }
                    })
                    .collect();

                let field_assigns: Vec<TokenStream> = s
                    .fields
                    .iter()
                    .map(|f| {
                        let fname = format_ident!("{}", &f.name);
                        quote! { #fname }
                    })
                    .collect();

                quote! {
                    #[allow(non_snake_case, clippy::too_many_arguments)]
                    #vis fn #name(#(#param_tokens),*) -> #name {
                        #name {
                            #(#field_assigns),*
                        }
                    }
                }
            } else {
                quote! {}
            };

            Ok(quote! {
                #derive_attr
                #vis struct #name {
                    #(#fields),*
                }

                #constructor
            })
        }
    }

    /// Emit an enum
    fn emit_enum(&self, e: &super::decl::IrEnum) -> Result<TokenStream, EmitError> {
        let name = format_ident!("{}", &e.name);
        let vis = self.emit_visibility(&e.visibility);

        let variants: Vec<TokenStream> = e
            .variants
            .iter()
            .map(|v| {
                let vname = format_ident!("{}", &v.name);
                match &v.fields {
                    super::decl::VariantFields::Unit => quote! { #vname },
                    super::decl::VariantFields::Tuple(types) => {
                        let type_tokens: Vec<_> = types.iter().map(|t| self.emit_type(t)).collect();
                        quote! { #vname(#(#type_tokens),*) }
                    }
                    super::decl::VariantFields::Struct(fields) => {
                        let field_tokens: Vec<_> = fields
                            .iter()
                            .map(|f| {
                                let fname = format_ident!("{}", &f.name);
                                let fty = self.emit_type(&f.ty);
                                quote! { #fname: #fty }
                            })
                            .collect();
                        quote! { #vname { #(#field_tokens),* } }
                    }
                }
            })
            .collect();

        let derives: Vec<TokenStream> = e
            .derives
            .iter()
            .map(|d| {
                // Use fully qualified paths for serde derives
                match d.as_str() {
                    "Serialize" => quote! { serde::Serialize },
                    "Deserialize" => quote! { serde::Deserialize },
                    _ => {
                        let d_ident = format_ident!("{}", d);
                        quote! { #d_ident }
                    }
                }
            })
            .collect();

        let derive_attr = if derives.is_empty() {
            quote! {}
        } else {
            quote! { #[derive(#(#derives),*)] }
        };

        // Generate message() impl for error-like enums
        let variant_match_arms: Vec<TokenStream> = e
            .variants
            .iter()
            .map(|v| {
                let vname = format_ident!("{}", &v.name);
                let vname_str = &v.name;
                match &v.fields {
                    super::decl::VariantFields::Unit => {
                        quote! { Self::#vname => #vname_str.to_string() }
                    }
                    super::decl::VariantFields::Tuple(types) => {
                        let wildcards: Vec<_> = (0..types.len()).map(|_| quote! { _ }).collect();
                        quote! { Self::#vname(#(#wildcards),*) => #vname_str.to_string() }
                    }
                    super::decl::VariantFields::Struct(_) => {
                        quote! { Self::#vname { .. } => #vname_str.to_string() }
                    }
                }
            })
            .collect();

        Ok(quote! {
            #derive_attr
            #vis enum #name {
                #(#variants),*
            }

            impl #name {
                pub fn message(&self) -> String {
                    match self {
                        #(#variant_match_arms),*
                    }
                }
            }
        })
    }
}

// Methods emit_stmt, emit_expr, emit_type, etc. moved to submodules:
// - statements.rs
// - expressions.rs
// - types.rs

/// Error during IR emission
#[derive(Debug)]
pub enum EmitError {
    SynParse(String),
    Unsupported(String),
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmitError::SynParse(msg) => write!(f, "syn parse error: {}", msg),
            EmitError::Unsupported(msg) => write!(f, "unsupported: {}", msg),
        }
    }
}

impl std::error::Error for EmitError {}

#[cfg(test)]
mod tests {
    use super::super::decl::Visibility;
    use super::super::expr::{BinOp, IrExprKind, VarAccess};
    use super::*;

    #[test]
    fn test_emit_simple_function() {
        let func = super::super::decl::IrFunction {
            name: "add".to_string(),
            params: vec![
                super::super::decl::FunctionParam {
                    name: "a".to_string(),
                    ty: IrType::Int,
                    mutability: Mutability::Immutable,
                    is_self: false,
                },
                super::super::decl::FunctionParam {
                    name: "b".to_string(),
                    ty: IrType::Int,
                    mutability: Mutability::Immutable,
                    is_self: false,
                },
            ],
            return_type: IrType::Int,
            body: vec![IrStmt::new(IrStmtKind::Return(Some(TypedExpr::new(
                IrExprKind::BinOp {
                    op: BinOp::Add,
                    left: Box::new(TypedExpr::new(
                        IrExprKind::Var {
                            name: "a".to_string(),
                            access: VarAccess::Copy,
                        },
                        IrType::Int,
                    )),
                    right: Box::new(TypedExpr::new(
                        IrExprKind::Var {
                            name: "b".to_string(),
                            access: VarAccess::Copy,
                        },
                        IrType::Int,
                    )),
                },
                IrType::Int,
            ))))],
            is_async: false,
            visibility: Visibility::Public,
            type_params: vec![],
        };

        let program = IrProgram {
            declarations: vec![IrDecl::new(IrDeclKind::Function(func))],
            entry_point: None,
            function_registry: FunctionRegistry::new(),
        };

        let mut emitter = IrEmitter::new(&program.function_registry);
        let result = emitter.emit_program(&program);
        assert!(result.is_ok());
        let code = match result {
            Ok(code) => code,
            Err(e) => panic!("emit_program failed: {e:?}"),
        };
        assert!(code.contains("fn add"));
        assert!(code.contains("a + b"));
    }

    #[test]
    fn test_emit_struct() {
        let s = super::super::decl::IrStruct {
            name: "User".to_string(),
            fields: vec![
                super::super::decl::StructField {
                    name: "name".to_string(),
                    ty: IrType::String,
                    visibility: Visibility::Public,
                },
                super::super::decl::StructField {
                    name: "age".to_string(),
                    ty: IrType::Int,
                    visibility: Visibility::Public,
                },
            ],
            derives: vec!["Debug".to_string(), "Clone".to_string()],
            visibility: Visibility::Public,
            type_params: vec![],
        };

        let program = IrProgram {
            declarations: vec![IrDecl::new(IrDeclKind::Struct(s))],
            entry_point: None,
            function_registry: FunctionRegistry::new(),
        };

        let mut emitter = IrEmitter::new(&program.function_registry);
        let result = emitter.emit_program(&program);
        assert!(result.is_ok());
        let code = match result {
            Ok(code) => code,
            Err(e) => panic!("emit_program failed: {e:?}"),
        };
        assert!(code.contains("struct User"));
        assert!(code.contains("derive"));
    }

    // ============================================================
    // Type Emission Tests
    // ============================================================

    #[test]
    fn test_emit_type_int() {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);
        let ty = IrType::Int;
        let result = emitter.emit_type(&ty);
        assert_eq!(result.to_string(), "i64");
    }

    #[test]
    fn test_emit_type_list_int() {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);
        let ty = IrType::List(Box::new(IrType::Int));
        let result = emitter.emit_type(&ty);
        assert_eq!(result.to_string(), "Vec < i64 >");
    }

    #[test]
    fn test_emit_type_option_string() {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);
        let ty = IrType::Option(Box::new(IrType::String));
        let result = emitter.emit_type(&ty);
        assert_eq!(result.to_string(), "Option < String >");
    }

    #[test]
    fn test_emit_type_dict_string_int() {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);
        let ty = IrType::Dict(Box::new(IrType::String), Box::new(IrType::Int));
        let result = emitter.emit_type(&ty);
        assert_eq!(
            result.to_string(),
            "std :: collections :: HashMap < String , i64 >"
        );
    }

    // ============================================================
    // Operator Emission Tests
    // ============================================================

    #[test]
    fn test_emit_binop_add() {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);
        let result = emitter.emit_binop(&BinOp::Add);
        assert_eq!(result.to_string(), "+");
    }

    #[test]
    fn test_emit_compound_op_mul() {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);
        let result = emitter.emit_compound_op(&BinOp::Mul);
        assert_eq!(result.to_string(), "*=");
    }

    #[test]
    fn test_all_binary_operators_map() {
        let registry = FunctionRegistry::new();
        let emitter = IrEmitter::new(&registry);

        // Test a representative set of binary operators
        let tests = vec![
            (BinOp::Add, "+"),
            (BinOp::Sub, "-"),
            (BinOp::Mul, "*"),
            (BinOp::Div, "/"),
            (BinOp::Mod, "%"),
            (BinOp::Eq, "=="),
            (BinOp::Ne, "!="),
            (BinOp::Lt, "<"),
            (BinOp::Le, "<="),
            (BinOp::Gt, ">"),
            (BinOp::Ge, ">="),
            (BinOp::And, "&&"),
            (BinOp::Or, "||"),
            (BinOp::BitAnd, "&"),
            (BinOp::BitOr, "|"),
            (BinOp::BitXor, "^"),
            (BinOp::Shl, "<<"),
            (BinOp::Shr, ">>"),
        ];

        for (op, expected) in tests {
            let result = emitter.emit_binop(&op);
            assert_eq!(result.to_string(), expected, "Failed for {:?}", op);
        }
    }

    // ============================================================
    // Keyword Escaping Tests
    // ============================================================

    #[test]
    fn test_escape_keyword_box() {
        let result = IrEmitter::escape_keyword("box");
        assert_eq!(result, "r#box");
    }

    #[test]
    fn test_escape_keyword_type() {
        let result = IrEmitter::escape_keyword("type");
        assert_eq!(result, "r#type");
    }

    #[test]
    fn test_escape_keyword_self_no_escape() {
        let result = IrEmitter::escape_keyword("self");
        assert_eq!(result, "self");
    }

    #[test]
    fn test_escape_keyword_normal() {
        let result = IrEmitter::escape_keyword("normal");
        assert_eq!(result, "normal");
    }
}
