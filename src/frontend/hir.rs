//! Frontend bridge into Incan HIR v0.
//!
//! This module builds the first declaration-level HIR snapshot from parsed AST plus `TypeCheckInfo`. It does not lower
//! bodies or replace the Rust-source backend; it gives the v0.5 middle-end a deterministic shape to grow from.

use crate::frontend::ast::{self, Declaration};
use crate::frontend::typechecker::TypeCheckInfo;
use incan_semantics_core::{
    CompilerNodeId, HirDeclaration, HirDeclarationKind, HirModule, HirSourceSpan, SemanticFactStore,
    SemanticModuleSnapshot,
};

/// Build declaration-level HIR v0 for a typechecked module.
pub fn build_hir_v0(program: &ast::Program, module_path: &[String], type_info: &TypeCheckInfo) -> HirModule {
    let module_identity = hir_module_identity(module_path);
    let facts = type_info.semantic_fact_store(module_path);
    build_hir_v0_with_facts(program, module_identity, type_info, &facts)
}

/// Build the bundled semantic module snapshot v0 for a typechecked module.
pub fn build_semantic_module_snapshot_v0(
    program: &ast::Program,
    module_path: &[String],
    type_info: &TypeCheckInfo,
) -> SemanticModuleSnapshot {
    let module_identity = hir_module_identity(module_path);
    let facts = type_info.semantic_fact_store(module_path);
    let hir = build_hir_v0_with_facts(program, module_identity, type_info, &facts);
    SemanticModuleSnapshot { hir, facts }
}

/// Build declaration-level HIR after semantic facts have already been collected.
///
/// Each declaration's RFC 120 identity is *consumed* from the typechecker's exported facts — the span-keyed
/// declaration identities for local declarations, and the local-name-keyed resolved import identities for
/// single-binding imports — never re-derived here from module path plus spelling. A declaration with no exported
/// identity carries none.
fn build_hir_v0_with_facts(
    program: &ast::Program,
    module_identity: String,
    type_info: &TypeCheckInfo,
    facts: &SemanticFactStore,
) -> HirModule {
    let declarations = program
        .declarations
        .iter()
        .map(|decl| {
            let (kind, name) = hir_decl_kind_and_name(&decl.node);
            // An import keeps its span-derived id even when named: a name-derived id would collide with a local
            // declaration of the same spelling (which the import may legally shadow or be shadowed by), and
            // spelling-derived ids are the v0 surface RFC 120 retires rather than extends.
            let id = match &decl.node {
                Declaration::Import(_) => hir_span_decl_id(&module_identity, decl.span),
                _ => name
                    .as_deref()
                    .map(|name| hir_named_decl_id(&module_identity, name))
                    .unwrap_or_else(|| hir_span_decl_id(&module_identity, decl.span)),
            };
            let type_fact_subject = facts.type_facts_for(&id).next().is_some().then_some(id.clone());
            let canonical = match &decl.node {
                Declaration::Import(_) => name
                    .as_deref()
                    .and_then(|local| type_info.declarations.resolved_import_identities.get(local))
                    .cloned(),
                _ => type_info
                    .declarations
                    .declaration_identities
                    .get(&(decl.span.start, decl.span.end))
                    .cloned(),
            };
            HirDeclaration {
                id,
                kind,
                name,
                span: HirSourceSpan::new(decl.span.start, decl.span.end),
                type_fact_subject,
                canonical,
            }
        })
        .collect();

    HirModule {
        id: CompilerNodeId::module(module_identity.clone()),
        path: module_identity,
        declarations,
    }
}

/// Map a frontend declaration to the HIR v0 declaration category and optional name.
///
/// An import declaration is named by its single local binding when it introduces exactly one — the alias when
/// written, the item name otherwise — so that binding's resolved identity can ride on the declaration. Multi-item
/// and namespace imports stay anonymous in HIR v0; per-binding import declarations are RFC 120 Slice 4's remaining
/// scope.
fn hir_decl_kind_and_name(decl: &Declaration) -> (HirDeclarationKind, Option<String>) {
    match decl {
        Declaration::Import(import) => (HirDeclarationKind::Import, single_import_binding_name(import)),
        Declaration::Const(decl) => (HirDeclarationKind::Const, Some(decl.name.clone())),
        Declaration::Static(decl) => (HirDeclarationKind::Static, Some(decl.name.clone())),
        Declaration::Model(decl) => (HirDeclarationKind::Model, Some(decl.name.clone())),
        Declaration::Capability(decl) => (HirDeclarationKind::Capability, Some(decl.name.clone())),
        Declaration::Class(decl) => (HirDeclarationKind::Class, Some(decl.name.clone())),
        Declaration::Trait(decl) => (HirDeclarationKind::Trait, Some(decl.name.clone())),
        Declaration::Alias(decl) => (HirDeclarationKind::Alias, Some(decl.name.clone())),
        Declaration::Partial(decl) => (HirDeclarationKind::Partial, Some(decl.name.clone())),
        Declaration::TypeAlias(decl) => (HirDeclarationKind::TypeAlias, Some(decl.name.clone())),
        Declaration::Newtype(decl) => (
            if decl.is_rusttype {
                HirDeclarationKind::Rusttype
            } else {
                HirDeclarationKind::Newtype
            },
            Some(decl.name.clone()),
        ),
        Declaration::Enum(decl) => (HirDeclarationKind::Enum, Some(decl.name.clone())),
        Declaration::Function(decl) => (HirDeclarationKind::Function, Some(decl.name.clone())),
        Declaration::TestModule(decl) => (HirDeclarationKind::TestModule, Some(decl.name.clone())),
        Declaration::VocabBlock(_) => (HirDeclarationKind::Docstring, None),
        Declaration::Docstring(_) => (HirDeclarationKind::Docstring, None),
    }
}

/// Return the one local binding name a `from ... import` declaration introduces, when it introduces exactly one.
///
/// The alias wins over the item name because the alias is the binding the module actually gains. `None` covers
/// module/namespace imports and multi-item imports, whose bindings HIR v0 does not yet model individually.
fn single_import_binding_name(import: &ast::ImportDecl) -> Option<String> {
    let items = match &import.kind {
        ast::ImportKind::From { items, .. }
        | ast::ImportKind::PubFrom { items, .. }
        | ast::ImportKind::RustFrom { items, .. } => items,
        _ => return None,
    };
    let [item] = items.as_slice() else {
        return None;
    };
    Some(item.alias.clone().unwrap_or_else(|| item.name.clone()))
}

/// Render a module path into the semantic module identity used by HIR v0.
fn hir_module_identity(module_path: &[String]) -> String {
    incan_semantics_core::module_identity_for_path(module_path)
}

/// Build the HIR declaration identity for a named declaration.
fn hir_named_decl_id(module_identity: &str, name: &str) -> CompilerNodeId {
    CompilerNodeId::declaration(module_identity, name)
}

/// Build the HIR declaration identity for an anonymous declaration.
fn hir_span_decl_id(module_identity: &str, span: ast::Span) -> CompilerNodeId {
    CompilerNodeId::declaration_span(module_identity, span.start, span.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::typechecker::TypeChecker;
    use crate::frontend::{lexer, parser};

    #[test]
    fn build_hir_v0_renders_deterministic_declaration_snapshot() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
model User:
  name: str

enum Status:
  Active

def add(x: int, y: int = 1) -> int:
  return x + y
"#;
        let tokens = lexer::lex(source).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        let program = parser::parse(&tokens).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        let module_path = vec!["facts".to_string(), "hir".to_string()];
        let mut checker = TypeChecker::new();
        checker.set_current_module_path(Some(module_path.clone()));
        checker
            .check_program(&program)
            .map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;

        let first = build_hir_v0(&program, &module_path, checker.type_info()).render_snapshot();
        let second = build_hir_v0(&program, &module_path, checker.type_info()).render_snapshot();

        assert_eq!(first, second);
        assert!(first.contains("module facts::hir module:facts::hir\n"));
        assert!(first.contains("decl model User decl:facts::hir::User"));
        assert!(first.contains("decl enum Status decl:facts::hir::Status"));
        assert!(first.contains("decl function add decl:facts::hir::add"));
        assert!(first.contains("type_fact=decl:facts::hir::add"));
        assert!(!first.contains("type_fact=decl:facts::hir::User"));
        assert!(!first.contains("type_fact=decl:facts::hir::Status"));
        Ok(())
    }

    #[test]
    fn build_semantic_module_snapshot_v0_renders_hir_and_fact_sections() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
def add(x: int, y: int = 1) -> int:
  return x + y
"#;
        let tokens = lexer::lex(source).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        let program = parser::parse(&tokens).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        let module_path = vec!["facts".to_string(), "snapshot".to_string()];
        let mut checker = TypeChecker::new();
        checker.set_current_module_path(Some(module_path.clone()));
        checker
            .check_program(&program)
            .map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;

        let snapshot = build_semantic_module_snapshot_v0(&program, &module_path, checker.type_info()).render_snapshot();

        assert!(snapshot.contains("module facts::snapshot module:facts::snapshot\n"));
        assert!(snapshot.contains("decl function add decl:facts::snapshot::add"));
        assert!(snapshot.contains("\nfacts\n"));
        assert!(snapshot.contains("decl:facts::snapshot::add type=(int, int) -> int"));
        Ok(())
    }

    /// RFC 120: a declaration's HIR record carries its minted identity, and a single-binding aliased import carries
    /// the *declaring* module's identity — so the import and its target declaration are visibly one symbol in the
    /// HIR handoff without consulting spellings.
    #[test]
    fn build_hir_v0_attaches_canonical_identities_to_declarations_and_single_imports()
    -> Result<(), Box<dyn std::error::Error>> {
        let helper_source = r#"
pub def helper() -> int:
  return 1
"#;
        let main_source = r#"
from helpers import helper as h

def run() -> int:
  return h()
"#;
        let helper_tokens = lexer::lex(helper_source).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        let helper_program =
            parser::parse(&helper_tokens).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        let main_tokens = lexer::lex(main_source).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        let main_program = parser::parse(&main_tokens).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        let module_path = vec!["app".to_string()];
        let mut checker = TypeChecker::new();
        checker.set_current_module_path(Some(module_path.clone()));
        checker
            .check_with_imports(&main_program, &[("helpers", &helper_program)])
            .map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;

        let hir = build_hir_v0(&main_program, &module_path, checker.type_info());

        let import_decl = hir
            .declarations
            .iter()
            .find(|decl| decl.kind == incan_semantics_core::HirDeclarationKind::Import)
            .ok_or("import declaration missing from HIR")?;
        assert_eq!(
            import_decl.name.as_deref(),
            Some("h"),
            "a single-binding import is named by its local binding"
        );
        let import_identity = import_decl
            .canonical
            .as_ref()
            .ok_or("single-binding import must carry its target's identity")?;
        assert_eq!(import_identity.declaration_name, "helper");
        assert_eq!(
            import_identity.module_path(),
            Some(["helpers".to_string()].as_slice()),
            "the import carries the declaring module's identity, not the consumer's"
        );

        let run_decl = hir
            .declarations
            .iter()
            .find(|decl| decl.name.as_deref() == Some("run"))
            .ok_or("run declaration missing from HIR")?;
        let run_identity = run_decl
            .canonical
            .as_ref()
            .ok_or("local declaration must carry its identity")?;
        assert_eq!(run_identity.module_path(), Some(["app".to_string()].as_slice()));

        let snapshot = hir.render_snapshot();
        assert!(
            snapshot.contains("identity=function:helpers::helper"),
            "the snapshot renders the import's declaring identity: {snapshot}"
        );
        Ok(())
    }

    #[test]
    fn build_semantic_module_snapshot_v0_preserves_imported_source_targets() -> Result<(), Box<dyn std::error::Error>> {
        let helper_source = r#"
pub def helper() -> int:
  return 1
"#;
        let main_source = r#"
from helpers import helper

def run() -> int:
  return helper()
"#;
        let helper_tokens = lexer::lex(helper_source).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        let helper_program =
            parser::parse(&helper_tokens).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        let main_tokens = lexer::lex(main_source).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        let main_program = parser::parse(&main_tokens).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
        let module_path = vec!["app".to_string()];
        let mut checker = TypeChecker::new();
        checker.set_current_module_path(Some(module_path.clone()));
        checker
            .check_with_imports(&main_program, &[("helpers", &helper_program)])
            .map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;

        let snapshot =
            build_semantic_module_snapshot_v0(&main_program, &module_path, checker.type_info()).render_snapshot();

        assert!(snapshot.contains("\nfacts\n"));
        assert!(snapshot.contains("symbol_target=function:helpers::helper"));
        assert!(!snapshot.contains("symbol_target=function:app::helper"));
        Ok(())
    }
}
