//! AST to IR lowering pass.
//!
//! This module converts the Incan frontend AST to the typed IR representation.
//! The lowering pass:
//!
//! 1. Resolves types from AST type annotations
//! 2. Determines ownership/borrowing semantics
//! 3. Converts AST nodes to their IR equivalents
//!
//! # Architecture
//!
//! The lowering module is split into submodules for maintainability:
//!
//! - `errors` - Error types (`LoweringError`, `LoweringErrors`)
//! - `types` - Type lowering utilities
//! - `decl` - Declaration lowering (functions, models, classes, enums, etc.)
//! - `stmt` - Statement lowering
//! - `expr` - Expression lowering
//!
//! # Usage
//!
//! ```rust,ignore
//! use incan::backend::ir::lower::AstLowering;
//!
//! let mut lowering = AstLowering::new();
//! let ir_program = lowering.lower_program(&ast_program)?;
//! ```

mod decl;
mod errors;
mod expr;
mod stmt;
mod types;

use std::collections::HashMap;

use super::decl::{FunctionParam, IrDecl, IrDeclKind};
use super::types::IrType;
use super::{IrProgram, Mutability};
use crate::frontend::ast;
use crate::frontend::typechecker::TypeCheckInfo;

// Re-export error types
pub use errors::{LoweringError, LoweringErrors};

/// AST to IR lowering context.
///
/// Maintains state needed during the lowering pass:
/// - Scope chain for variable type lookups
/// - Registered struct/enum names for constructor detection
/// - Mutable variable tracking for borrow insertion
/// - Class declarations for inheritance resolution
/// - Trait method names for impl filtering
///
/// # Examples
///
/// ```rust,ignore
/// use incan::backend::ir::lower::AstLowering;
///
/// let mut lowering = AstLowering::new();
/// let ir_program = lowering.lower_program(&ast_program)?;
/// ```
pub struct AstLowering {
    /// Scope chain for variable type lookups (innermost last)
    pub(super) scopes: Vec<HashMap<String, IrType>>,
    /// Track declared structs/models/classes for constructor detection
    pub(super) struct_names: HashMap<String, IrType>,
    /// Track declared enums for type resolution
    pub(super) enum_names: HashMap<String, IrType>,
    /// Track mutable variables for auto-borrow at call sites
    pub(super) mutable_vars: HashMap<String, bool>,
    /// Track class declarations for inheritance resolution
    pub(super) class_decls: HashMap<String, ast::ClassDecl>,
    /// Track trait method names for filtering trait impls
    pub(super) trait_methods: HashMap<String, Vec<String>>,
    /// Optional typechecker output used to drive lowering (avoid heuristics).
    pub(super) type_info: Option<TypeCheckInfo>,
}

impl AstLowering {
    /// Create a new lowering context.
    ///
    /// Initializes an empty scope chain and type registries.
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            struct_names: HashMap::new(),
            enum_names: HashMap::new(),
            mutable_vars: HashMap::new(),
            class_decls: HashMap::new(),
            trait_methods: HashMap::new(),
            type_info: None,
        }
    }

    /// Create a lowering context that uses typechecker output for more accurate lowering.
    pub fn new_with_type_info(type_info: TypeCheckInfo) -> Self {
        let mut s = Self::new();
        s.type_info = Some(type_info);
        s
    }

    /// Lower a complete AST program to IR.
    ///
    /// This is the main entry point for the lowering pass. It performs:
    ///
    /// 1. First pass: Collect class declarations and trait method names
    /// 2. Second pass: Collect function signatures for the registry
    /// 3. Third pass: Lower all declarations to IR
    ///
    /// # Parameters
    ///
    /// * `program` - The AST program to lower
    ///
    /// # Returns
    ///
    /// An `IrProgram` containing all lowered declarations.
    ///
    /// # Errors
    ///
    /// Returns `LoweringErrors` containing all errors encountered during lowering.
    /// This allows callers to display multiple errors to the user at once.
    #[tracing::instrument(skip_all, fields(decl_count = program.declarations.len()))]
    pub fn lower_program(&mut self, program: &ast::Program) -> Result<IrProgram, LoweringErrors> {
        let mut ir_program = IrProgram::new();
        let mut errors: Vec<LoweringError> = Vec::new();

        // First pass: collect class declarations and trait method names
        for decl in &program.declarations {
            if let ast::Declaration::Class(ref c) = decl.node {
                self.class_decls.insert(c.name.clone(), c.clone());
            }
            if let ast::Declaration::Trait(ref t) = decl.node {
                let method_names: Vec<String> =
                    t.methods.iter().map(|m| m.node.name.clone()).collect();
                self.trait_methods.insert(t.name.clone(), method_names);
            }
        }

        // Pass 1.5: register module-level const names into the root scope for lookups.
        // (Type inference/refinement happens later; Unknown is fine for non-const contexts.)
        for decl in &program.declarations {
            if let ast::Declaration::Const(ref c) = decl.node {
                let ty = if let Some(ann) = &c.ty {
                    self.lower_type(&ann.node)
                } else {
                    IrType::Unknown
                };
                if let Some(scope) = self.scopes.first_mut() {
                    scope.insert(c.name.clone(), ty);
                }
            }
        }

        // Second pass: collect all function signatures
        for decl in &program.declarations {
            if let ast::Declaration::Function(ref f) = decl.node {
                let params: Vec<FunctionParam> = f
                    .params
                    .iter()
                    .map(|p| {
                        let base_ty = self.lower_type(&p.node.ty.node);
                        FunctionParam {
                            name: p.node.name.clone(),
                            ty: base_ty,
                            mutability: if p.node.is_mut {
                                Mutability::Mutable
                            } else {
                                Mutability::Immutable
                            },
                            is_self: false,
                        }
                    })
                    .collect();
                let return_type = self.lower_type(&f.return_type.node);
                ir_program
                    .function_registry
                    .register(f.name.clone(), params, return_type);
            }
        }

        // Third pass: lower declarations
        for decl in &program.declarations {
            // Handle models - generate both struct and impl
            // Models always get impl blocks (for serde methods even if no user methods)
            match &decl.node {
                ast::Declaration::Model(m) => {
                    // Generate struct
                    match self.lower_model(m) {
                        Ok(struct_ir) => {
                            self.struct_names.insert(
                                struct_ir.name.clone(),
                                IrType::Struct(struct_ir.name.clone()),
                            );
                            ir_program
                                .declarations
                                .push(IrDecl::new(IrDeclKind::Struct(struct_ir.clone())));

                            // Generate impl block (may be empty if no methods, serde methods added during emission)
                            match self.lower_model_methods(&struct_ir.name, &m.methods) {
                                Ok(impl_ir) => {
                                    ir_program
                                        .declarations
                                        .push(IrDecl::new(IrDeclKind::Impl(impl_ir)));
                                }
                                Err(e) => errors.push(e),
                            }
                        }
                        Err(e) => errors.push(e),
                    }
                }
                ast::Declaration::Docstring(_) => {
                    // Module-level docstrings are not part of IR; ignore silently
                    continue;
                }
                ast::Declaration::Class(c) => {
                    // Generate struct
                    match self.lower_class(c) {
                        Ok(struct_ir) => {
                            self.struct_names.insert(
                                struct_ir.name.clone(),
                                IrType::Struct(struct_ir.name.clone()),
                            );
                            ir_program
                                .declarations
                                .push(IrDecl::new(IrDeclKind::Struct(struct_ir.clone())));

                            // Collect methods from this class and all parent classes
                            let mut all_methods = Vec::new();
                            if let Err(e) =
                                self.collect_inherited_methods(&c.name, &mut all_methods)
                            {
                                errors.push(e);
                            }

                            // Generate impl block for all methods (inherited + own)
                            if !all_methods.is_empty() {
                                match self.lower_class_methods(&struct_ir.name, &all_methods) {
                                    Ok(impl_ir) => {
                                        ir_program
                                            .declarations
                                            .push(IrDecl::new(IrDeclKind::Impl(impl_ir)));
                                    }
                                    Err(e) => errors.push(e),
                                }
                            }

                            // Generate trait impls for each trait this class implements
                            for trait_name in &c.traits {
                                match self.lower_trait_impl(&struct_ir.name, trait_name, &c.methods)
                                {
                                    Ok(trait_impl) => {
                                        ir_program
                                            .declarations
                                            .push(IrDecl::new(IrDeclKind::Impl(trait_impl)));
                                    }
                                    Err(e) => errors.push(e),
                                }
                            }
                        }
                        Err(e) => errors.push(e),
                    }
                }
                _ => {
                    // Regular declaration lowering
                    match self.lower_declaration(&decl.node) {
                        Ok(ir_decl) => {
                            if let IrDeclKind::Function(ref func) = ir_decl.kind {
                                if func.name == "main" {
                                    ir_program.entry_point = Some("main".to_string());
                                }
                            }
                            ir_program.declarations.push(ir_decl);
                        }
                        Err(e) => errors.push(e),
                    }
                }
            }
        }
        if errors.is_empty() {
            Ok(ir_program)
        } else {
            // Return all collected errors
            Err(LoweringErrors(errors))
        }
    }
}

impl Default for AstLowering {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::frontend::{lexer, parser};

    fn lower_source(source: &str) -> Result<IrProgram, LoweringErrors> {
        let tokens = lexer::lex(source).unwrap_or_else(|errs| {
            panic!("lexer failed: {errs:?}");
        });
        let ast = parser::parse(&tokens).unwrap_or_else(|errs| {
            panic!("parser failed: {errs:?}");
        });
        let mut lowering = AstLowering::new();
        lowering.lower_program(&ast)
    }

    #[test]
    fn test_lower_simple_function() {
        let ir = lower_source(
            r#"
def add(a: int, b: int) -> int:
    return a + b
"#,
        )
        .unwrap();
        assert_eq!(ir.declarations.len(), 1);
        if let IrDeclKind::Function(f) = &ir.declarations[0].kind {
            assert_eq!(f.name, "add");
            assert_eq!(f.params.len(), 2);
        } else {
            panic!("Expected function declaration");
        }
    }

    #[test]
    fn test_lower_model() {
        let ir = lower_source(
            r#"
model User:
    name: str
    age: int
"#,
        )
        .unwrap();
        // Model generates both struct and impl
        assert_eq!(ir.declarations.len(), 2);
        if let IrDeclKind::Struct(s) = &ir.declarations[0].kind {
            assert_eq!(s.name, "User");
            assert_eq!(s.fields.len(), 2);
        } else {
            panic!("Expected struct declaration");
        }
    }

    #[test]
    fn test_lower_main_entry() {
        let ir = lower_source(
            r#"
def main() -> None:
    pass
"#,
        )
        .unwrap();
        assert_eq!(ir.entry_point, Some("main".to_string()));
    }

    #[test]
    fn test_lower_if_statement() {
        let ir = lower_source(
            r#"
def check(x: int) -> str:
    if x > 0:
        return "positive"
    elif x < 0:
        return "negative"
    else:
        return "zero"
"#,
        )
        .unwrap();
        assert_eq!(ir.declarations.len(), 1);
        if let IrDeclKind::Function(f) = &ir.declarations[0].kind {
            assert!(!f.body.is_empty());
        } else {
            panic!("Expected function declaration");
        }
    }

    #[test]
    fn test_lower_for_loop() {
        let ir = lower_source(
            r#"
def count() -> None:
    for i in range(10):
        print(i)
"#,
        )
        .unwrap();
        assert_eq!(ir.declarations.len(), 1);
    }

    #[test]
    fn test_lower_binary_expressions() {
        let ir = lower_source(
            r#"
def math(a: int, b: int) -> int:
    x = a + b
    y = a * b
    z = a - b
    return x + y + z
"#,
        )
        .unwrap();
        assert_eq!(ir.declarations.len(), 1);
    }

    #[test]
    fn test_lower_list_literal() {
        let ir = lower_source(
            r#"
def get_list() -> List[int]:
    return [1, 2, 3]
"#,
        )
        .unwrap();
        assert_eq!(ir.declarations.len(), 1);
    }

    #[test]
    fn test_lower_enum() {
        let ir = lower_source(
            r#"
enum Color:
    Red
    Green
    Blue
"#,
        )
        .unwrap();
        assert_eq!(ir.declarations.len(), 1);
        if let IrDeclKind::Enum(e) = &ir.declarations[0].kind {
            assert_eq!(e.name, "Color");
            assert_eq!(e.variants.len(), 3);
        } else {
            panic!("Expected enum declaration");
        }
    }

    #[test]
    fn test_inferred_reassign_mutable() {
        // `mut x = 1; x = 2` should succeed because x is mutable.
        let source = r#"
def test() -> int:
    mut x = 1
    x = 2
    return x
"#;
        let ir = lower_source(source).unwrap();
        assert_eq!(ir.declarations.len(), 1);
        if let IrDeclKind::Function(f) = &ir.declarations[0].kind {
            // Expected: Let, Assign, Return (3 statements)
            assert_eq!(f.body.len(), 3, "Expected 3 statements");
        } else {
            panic!("Expected function declaration");
        }
    }

    #[test]
    fn test_inferred_reassign_immutable_error() {
        // `x = 1; x = 2` should fail because x is immutable.
        let source = r#"
def test() -> int:
    x = 1
    x = 2
    return x
"#;
        let result = lower_source(source);
        assert!(result.is_err(), "Expected error for immutable reassignment");
        let errors = result.unwrap_err();
        assert!(
            errors.0[0].message.contains("immutable"),
            "Error should mention immutable"
        );
    }
}
