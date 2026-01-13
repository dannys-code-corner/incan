//! Emit Rust source code from typed IR.
//!
//! This module defines [`IrEmitter`] and wires together the focused submodules that implement
//! IR → Rust emission. The heavy lifting lives in those submodules; `mod.rs` is intentionally thin.
//!
//! ## Notes
//! - Emission produces a Rust syntax tree (`syn`) and formats it via `prettyplease`.
//! - Ownership/borrow/string conversions are centralized in `backend::ir::conversions` and should not be reimplemented
//!   ad-hoc in emission code.
//!
//! ## See also
//! - [`crate::backend::ir::conversions`]: conversion policy for emitted Rust
//! - [`program`]: program-level emission and formatting
//! - [`decls`]: item/declaration emission
//! - [`statements`]: statement emission
//! - [`expressions`]: expression emission
//! - [`types`]: type/pattern/operator helpers
//! - [`consts`]: RFC-008 const validation and const-friendly helpers

mod consts;
mod decls;
mod errors;
mod expressions;
mod program;
mod statements;
mod types;

pub use errors::EmitError;

use std::cell::RefCell;

use super::FunctionRegistry;
use super::decl::VariantFields;
use super::types::IrType;

/// Emit Rust source code from typed IR.
///
/// This is the main entry point for the IR → Rust backend stage. It is stateful because it:
/// - tracks which imports/features are required,
/// - records auxiliary typing metadata needed for emission (e.g. enum variant fields),
/// - caches resolvable const string values to emit `concat!(...)` in const contexts.
///
/// ## Notes
/// - The public API is `emit_program()` (implemented in `program.rs`).
/// - Most emission helpers are implemented on this type across submodules.
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
    enum_variant_fields: std::collections::HashMap<(String, String), VariantFields>,
    /// Struct field type lookup: (StructName, FieldName) -> IrType
    struct_field_types: std::collections::HashMap<(String, String), IrType>,
    /// Struct field name order (as declared): StructName -> [FieldName...]
    struct_field_names: std::collections::HashMap<String, Vec<String>>,
    /// Struct field default expressions: (StructName, FieldName) -> default expr
    struct_field_defaults: std::collections::HashMap<(String, String), super::IrExpr>,
    /// Whether we're currently emitting a return expression (allows moves instead of clones)
    in_return_context: RefCell<bool>,
    /// Map of const string bindings to their literal values (for const folding of string adds)
    const_string_literals: std::collections::HashMap<String, String>,
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
            struct_field_names: std::collections::HashMap::new(),
            struct_field_defaults: std::collections::HashMap::new(),
            in_return_context: RefCell::new(false),
            const_string_literals: std::collections::HashMap::new(),
        }
    }

    /// Set external rust functions.
    pub fn set_external_rust_functions(&mut self, funcs: std::collections::HashSet<String>) {
        self.external_rust_functions = funcs;
    }

    /// Set whether serde is needed.
    pub fn set_needs_serde(&mut self, needs: bool) {
        self.needs_serde = needs;
    }

    /// Set whether tokio is needed.
    pub fn set_needs_tokio(&mut self, needs: bool) {
        self.needs_tokio = needs;
    }

    /// Set whether axum is needed.
    pub fn set_needs_axum(&mut self, needs: bool) {
        self.needs_axum = needs;
    }

    /// Escape Rust keywords by adding `r#` prefix.
    ///
    /// Note: `self` and `Self` cannot be raw identifiers.
    fn escape_keyword(name: &str) -> String {
        match name {
            "self" | "Self" => name.to_string(),
            // Strict + reserved keywords
            "as" | "break" | "const" | "continue" | "crate" | "else" | "enum" | "extern" | "false" | "fn" | "for"
            | "if" | "impl" | "in" | "let" | "loop" | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return"
            | "static" | "struct" | "super" | "trait" | "true" | "type" | "unsafe" | "use" | "where" | "while"
            | "async" | "await" | "dyn" | "abstract" | "become" | "box" | "do" | "final" | "macro" | "override"
            | "priv" | "typeof" | "unsized" | "virtual" | "yield" | "try" => {
                format!("r#{}", name)
            }
            _ => name.to_string(),
        }
    }

    /// Disable clippy allows (for strict warning-free codegen).
    pub fn without_clippy_allows(mut self) -> Self {
        self.add_clippy_allows = false;
        self
    }

    /// Set whether to emit the Zen of Incan in main.
    pub fn set_emit_zen(&mut self, emit: bool) {
        self.emit_zen_in_main = emit;
    }
}
