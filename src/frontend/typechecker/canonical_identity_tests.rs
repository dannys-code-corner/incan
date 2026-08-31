//! RFC 120 conformance: canonical symbol identity at declaration sites and on resolved references.
//!
//! These tests pin the identity contract itself rather than any consumer: one compiler-owned identity is minted at
//! each declaration site, an import/alias/re-export binding carries its *target's* identity, same-spelled bindings
//! in different scopes stay distinct, and reference-side recording answers "do these two references mean the same
//! thing" structurally. Body IR's consumption of these facts is pinned separately in
//! `crate::frontend::body_ir::tests`.

use incan_semantics_core::{CanonicalSymbolId, SemanticSourceTargetKind, SymbolNamespace, SymbolOrigin};

use super::TypeChecker;
use crate::frontend::ast::{Program, Span};
use crate::frontend::{lexer, parser};

/// Parse one test program, panicking with context on lex/parse failure.
fn parse(source: &str, context: &str) -> Program {
    let tokens = lexer::lex(source).unwrap_or_else(|errs| panic!("{context} lex failed: {errs:?}"));
    parser::parse(&tokens).unwrap_or_else(|errs| panic!("{context} parse failed: {errs:?}"))
}

/// Check one standalone program and return the checker for identity inspection.
fn check(source: &str, context: &str) -> Result<TypeChecker, String> {
    let program = parse(source, context);
    let mut checker = TypeChecker::new();
    checker.set_current_module_path(Some(vec!["conformance".to_string()]));
    checker
        .check_program(&program)
        .map_err(|errors| format!("{context} should typecheck: {errors:?}"))?;
    Ok(checker)
}

/// Return the span of the `occurrence`-th appearance (0-based) of `needle` in `source`.
fn nth_span(source: &str, needle: &str, occurrence: usize) -> Result<Span, String> {
    source
        .match_indices(needle)
        .nth(occurrence)
        .map(|(start, matched)| Span::new(start, start + matched.len()))
        .ok_or_else(|| format!("occurrence {occurrence} of `{needle}` not found"))
}

/// Return the recorded reference identity at `span`, or an error naming the missing case.
fn identity_at(checker: &TypeChecker, span: Span, context: &str) -> Result<CanonicalSymbolId, String> {
    checker.type_info().resolved_identity(span).cloned().ok_or_else(|| {
        format!(
            "{context}: no resolved identity recorded at {}..{}",
            span.start, span.end
        )
    })
}

/// A module-level declaration's identity is minted once and is independent of how often it is referenced.
#[test]
fn module_declaration_identity_is_reference_independent() -> Result<(), String> {
    let source = r#"
def helper() -> int:
  return 1

def first() -> int:
  value = helper
  return 1

def second() -> int:
  again = helper
  return 2
"#;
    let checker = check(source, "reference independence")?;
    let first_ref = identity_at(&checker, nth_span(source, "helper", 1)?, "first reference")?;
    let second_ref = identity_at(&checker, nth_span(source, "helper", 2)?, "second reference")?;
    assert_eq!(first_ref, second_ref, "two references must record one identity");
    assert_eq!(first_ref.kind, SemanticSourceTargetKind::Function);
    assert_eq!(first_ref.declaration_name, "helper");
    assert_eq!(
        first_ref.origin,
        SymbolOrigin::Module(vec!["conformance".to_string()]),
        "a module declaration is owned by its module"
    );
    assert_eq!(
        first_ref.scope_discriminant, None,
        "module-level declarations are module-unique and carry no discriminant"
    );

    let declaration = checker
        .type_info()
        .declarations
        .declaration_identities
        .values()
        .find(|identity| identity.declaration_name == "helper")
        .ok_or("declaration identity for `helper` must be exported")?;
    assert_eq!(
        declaration, &first_ref,
        "references resolve to the declaration's identity"
    );
    Ok(())
}

/// Two same-spelled bindings in sibling blocks are different declarations with different identities.
#[test]
fn sibling_block_locals_get_distinct_identities() -> Result<(), String> {
    let source = r#"
def run() -> None:
  if true:
    left = 1
    _ = left
  if true:
    left = 2
    _ = left
"#;
    let checker = check(source, "sibling blocks")?;
    // Occurrences: 0 = first binding, 1 = first reference, 2 = second binding, 3 = second reference.
    let first = identity_at(&checker, nth_span(source, "left", 1)?, "first block reference")?;
    let second = identity_at(&checker, nth_span(source, "left", 3)?, "second block reference")?;
    assert_eq!(first.kind, SemanticSourceTargetKind::Local);
    assert_eq!(second.kind, SemanticSourceTargetKind::Local);
    assert_ne!(
        first, second,
        "same-spelled locals in sibling blocks must not collapse to one identity"
    );
    assert_ne!(
        first.scope_discriminant, second.scope_discriminant,
        "sibling blocks are different scopes, so the discriminants must differ"
    );
    Ok(())
}

/// `let` introduces a new binding with a fresh identity over an active outer binding; the outer binding's identity
/// is unchanged and visible again after the block.
#[test]
fn let_shadowing_mints_a_new_identity_and_restores_the_outer_one() -> Result<(), String> {
    let source = r#"
def run() -> None:
  mut shade = 1
  first = shade
  if true:
    let shade = 2
    second = shade
  third = shade
"#;
    let checker = check(source, "let shadowing")?;
    // Occurrences: 0 = outer binding, 1 = outer reference, 2 = `let` binding, 3 = shadowed reference,
    // 4 = post-block reference.
    let outer = identity_at(&checker, nth_span(source, "shade", 1)?, "outer reference")?;
    let shadowed = identity_at(&checker, nth_span(source, "shade", 3)?, "shadowed reference")?;
    let restored = identity_at(&checker, nth_span(source, "shade", 4)?, "post-block reference")?;
    assert_ne!(outer, shadowed, "`let` must mint a fresh identity for the new binding");
    assert_eq!(
        outer, restored,
        "the outer binding's identity is visible again after the block"
    );
    Ok(())
}

/// Plain assignment inside a nested block reassigns the outer binding: later references still carry the outer
/// declaration's identity, not a new one.
#[test]
fn plain_assignment_preserves_the_target_binding_identity() -> Result<(), String> {
    let source = r#"
def run() -> None:
  mut total = 1
  first = total
  if true:
    total = 2
  second = total
"#;
    let checker = check(source, "plain reassignment")?;
    let before = identity_at(&checker, nth_span(source, "total", 1)?, "reference before block")?;
    let after = identity_at(&checker, nth_span(source, "total", 3)?, "reference after block")?;
    assert_eq!(
        before, after,
        "plain assignment reassigns the active binding and must not change its identity"
    );
    Ok(())
}

/// A generic binder has its own identity, scoped to the declaration that introduces it, distinct from any
/// same-spelled concrete type and from another declaration's binder.
#[test]
fn generic_binder_identity_is_declaration_scoped() -> Result<(), String> {
    let source = r#"
model Holder:
  value: int

def wrap[T](value: T) -> T:
  return value

def echo[T](value: T) -> T:
  return value
"#;
    let checker = check(source, "generic binders")?;
    let binder_identities: Vec<CanonicalSymbolId> = checker
        .symbols
        .all_symbols()
        .iter()
        .enumerate()
        .filter(|(_, symbol)| symbol.name == "T")
        .filter_map(|(id, _)| checker.symbols.identity_of(id).cloned())
        .filter(|identity| identity.kind == SemanticSourceTargetKind::GenericBinder)
        .collect();
    assert!(
        binder_identities.len() >= 2,
        "both binder declarations must carry GenericBinder identities, got {binder_identities:?}"
    );
    assert_ne!(
        binder_identities[0], binder_identities[1],
        "two declarations' binders are distinct declarations"
    );
    for binder in &binder_identities {
        assert!(
            binder.scope_discriminant.is_some(),
            "a binder is bounded to its declaration's scope, so it must carry a discriminant"
        );
    }

    let holder = checker
        .type_info()
        .declarations
        .declaration_identities
        .values()
        .find(|identity| identity.declaration_name == "Holder")
        .ok_or("model declaration identity must be exported")?;
    assert_eq!(holder.kind, SemanticSourceTargetKind::Model);
    assert!(
        binder_identities.iter().all(|binder| binder != holder),
        "a binder never compares equal to a concrete type declaration"
    );
    Ok(())
}

/// Parameters and receivers carry their own declaration categories, and a parameter's identity differs from a
/// same-spelled local in another scope.
#[test]
fn parameter_and_receiver_identities_carry_their_categories() -> Result<(), String> {
    let source = r#"
class Greeter:
  name: str

  def greet(self, message: str) -> str:
    return message
"#;
    let checker = check(source, "parameters and receivers")?;
    let message = identity_at(&checker, nth_span(source, "message", 1)?, "parameter reference")?;
    assert_eq!(message.kind, SemanticSourceTargetKind::Parameter);
    assert!(message.scope_discriminant.is_some(), "parameters are scope-bounded");

    // `self` reads resolve through their own dedicated path rather than ordinary identifier checking, so the
    // receiver's identity is observed at its definition in the symbol table.
    let receiver = checker
        .symbols
        .all_symbols()
        .iter()
        .enumerate()
        .filter(|(_, symbol)| symbol.name == "self")
        .filter_map(|(id, _)| checker.symbols.identity_of(id))
        .find(|identity| identity.kind == SemanticSourceTargetKind::Receiver)
        .ok_or("the receiver binding must carry a Receiver-kind identity")?;
    assert!(receiver.scope_discriminant.is_some(), "receivers are scope-bounded");
    Ok(())
}

/// A method declaration's identity lives in the member namespace; two owners' same-named methods stay distinct.
#[test]
fn member_method_identities_are_owner_distinct() -> Result<(), String> {
    let source = r#"
model First:
  value: int

  def describe(self) -> str:
    return "first"

model Second:
  value: int

  def describe(self) -> str:
    return "second"
"#;
    let checker = check(source, "member methods")?;
    let describe_identities: Vec<&CanonicalSymbolId> = checker
        .type_info()
        .declarations
        .method_bindings_by_span
        .values()
        .filter_map(|binding| binding.identity.as_ref())
        .filter(|identity| identity.declaration_name == "describe")
        .collect();
    assert_eq!(
        describe_identities.len(),
        2,
        "both method declarations must carry identities"
    );
    assert_eq!(describe_identities[0].namespace, SymbolNamespace::Member);
    assert_eq!(describe_identities[1].namespace, SymbolNamespace::Member);
    assert_eq!(describe_identities[0].kind, SemanticSourceTargetKind::Method);
    assert_ne!(
        describe_identities[0], describe_identities[1],
        "two owners' same-named methods are different declarations"
    );
    Ok(())
}

/// An import, its alias, and a re-export are bindings to one declaration: every spelling records one identity.
#[test]
fn import_alias_and_reexport_share_the_declaration_identity() -> Result<(), String> {
    let lib = parse(
        r#"
pub def helper() -> int:
  return 1
"#,
        "identity lib",
    );
    let api = parse(
        r#"
from lib import helper as h
"#,
        "identity api facade",
    );
    let consumer_source = r#"
from lib import helper
from lib import helper as h
from api import h as run

def use_all() -> None:
  a = helper
  b = h
  c = run
"#;
    let consumer = parse(consumer_source, "identity consumer");
    let mut checker = TypeChecker::new();
    checker.set_current_module_path(Some(vec!["consumer".to_string()]));
    checker
        .check_with_imports(&consumer, &[("lib", &lib), ("api", &api)])
        .map_err(|errors| format!("identity consumer should typecheck: {errors:?}"))?;

    let direct = checker
        .type_info()
        .resolved_import_identity("helper")
        .ok_or("direct import must prove an identity")?
        .clone();
    let aliased = checker
        .type_info()
        .resolved_import_identity("h")
        .ok_or("aliased import must prove an identity")?
        .clone();
    let reexported = checker
        .type_info()
        .resolved_import_identity("run")
        .ok_or("re-exported import must prove an identity")?
        .clone();

    assert_eq!(direct, aliased, "an alias binds the same declaration");
    assert_eq!(
        direct, reexported,
        "a re-export resolves to the declaring module, never the facade"
    );
    assert_eq!(
        direct.declaration_name, "helper",
        "the declaration-site spelling survives every alias"
    );
    assert_eq!(direct.origin, SymbolOrigin::Module(vec!["lib".to_string()]));

    // Reference-side recording sees the same identity through every spelling.
    let helper_ref = identity_at(&checker, nth_span(consumer_source, "helper", 2)?, "direct reference")?;
    let h_ref = identity_at(
        &checker,
        nth_span(consumer_source, "b = h", 0).map(|span| Span::new(span.end - 1, span.end))?,
        "alias reference",
    )?;
    let run_ref = identity_at(&checker, nth_span(consumer_source, "run", 1)?, "re-export reference")?;
    assert_eq!(helper_ref, direct);
    assert_eq!(h_ref, direct);
    assert_eq!(run_ref, direct);
    Ok(())
}

/// Rebinding a core builtin-function spelling is not a collision, and the rebound declaration's identity differs
/// from the builtin registry identity (#1116's settled contract as an identity fact).
#[test]
fn rebound_builtin_spelling_and_registry_builtin_are_distinct_identities() -> Result<(), String> {
    let source = r#"
def len(value: int) -> int:
  return value + 1

def shadowed() -> int:
  return len(4)
"#;
    let checker = check(source, "builtin rebinding")?;

    let local_len = checker
        .type_info()
        .declarations
        .declaration_identities
        .values()
        .find(|identity| identity.declaration_name == "len")
        .ok_or("the local `len` declaration must carry an identity")?;
    assert_eq!(local_len.kind, SemanticSourceTargetKind::Function);
    assert_eq!(local_len.origin, SymbolOrigin::Module(vec!["conformance".to_string()]));

    let registry_len = checker
        .symbols
        .all_symbols()
        .iter()
        .enumerate()
        .filter(|(_, symbol)| symbol.name == "len")
        .filter_map(|(id, _)| checker.symbols.identity_of(id))
        .find(|identity| identity.origin == SymbolOrigin::Builtin)
        .ok_or("the builtin registry identity for `len` must still exist")?;
    assert_eq!(registry_len.kind, SemanticSourceTargetKind::Builtin);
    assert_ne!(
        registry_len, local_len,
        "the rebound spelling and the registry builtin are two different canonical identities"
    );
    Ok(())
}

/// Builtin alias spellings share one canonical registry identity instead of minting one identity per spelling.
#[test]
fn builtin_alias_spellings_share_one_registry_identity() -> Result<(), String> {
    let checker = check("def noop() -> None:\n  pass\n", "builtin aliases")?;
    let mut int_identities = Vec::new();
    for (id, symbol) in checker.symbols.all_symbols().iter().enumerate() {
        if (symbol.name == "int" || symbol.name == "i64")
            && let Some(identity) = checker.symbols.identity_of(id)
            && identity.origin == SymbolOrigin::Builtin
        {
            int_identities.push(identity.clone());
        }
    }
    assert!(
        int_identities.len() >= 2,
        "expected canonical and alias spellings of the int builtin, got {int_identities:?}"
    );
    assert!(
        int_identities.iter().all(|identity| identity == &int_identities[0]),
        "every alias spelling must carry the one canonical registry identity: {int_identities:?}"
    );
    Ok(())
}

/// Consts and statics carry their declaration categories from the one mint point.
#[test]
fn const_and_static_identities_carry_their_categories() -> Result<(), String> {
    let source = r#"
const LIMIT: int = 10

static counter: int = 0

def read() -> int:
  return LIMIT
"#;
    let checker = check(source, "const and static")?;
    let limit = identity_at(&checker, nth_span(source, "LIMIT", 1)?, "const reference")?;
    assert_eq!(limit.kind, SemanticSourceTargetKind::Const);
    assert_eq!(limit.scope_discriminant, None);

    let counter = checker
        .type_info()
        .declarations
        .declaration_identities
        .values()
        .find(|identity| identity.declaration_name == "counter")
        .ok_or("static declaration identity must be exported")?;
    assert_eq!(counter.kind, SemanticSourceTargetKind::Static);
    Ok(())
}

/// Two same-spelled module declarations are two declarations with two identities.
///
/// RFC 120's decided rule makes a duplicate module-scope declaration a *diagnostic* from the one shared
/// binding-registration mechanism; that mechanism is Slice 3 of the RFC's implementation plan and is not delivered
/// by the identity core. What the identity core must already guarantee — and what this test pins — is that the two
/// declarations never collapse into one identity: whichever binding wins lookup, each declaration site keeps its
/// own canonical identity, so the later collision diagnostic can name both declarations by identity and span.
#[test]
fn duplicate_module_declarations_keep_distinct_identities() -> Result<(), String> {
    let source = r#"
model User:
  name: str

model User:
  age: int
"#;
    let checker = check(source, "duplicate declarations")?;
    let user_identities: Vec<&CanonicalSymbolId> = checker
        .type_info()
        .declarations
        .declaration_identities
        .values()
        .filter(|identity| identity.declaration_name == "User")
        .collect();
    assert_eq!(
        user_identities.len(),
        2,
        "both declaration sites must keep their own exported identity"
    );
    assert_ne!(
        user_identities[0], user_identities[1],
        "two declaration sites are two identities, never one merged winner"
    );
    Ok(())
}

/// A local declaration over an imported binding keeps the local declaration's identity on later references.
///
/// The decided rule makes this collision a diagnostic once RFC 120 Slice 3's shared mechanism lands; the identity
/// core pins the meaning-side half: the reference resolves to exactly one binding, and its identity is the local
/// declaration's — not the import's, and not a blend of the two.
#[test]
fn local_declaration_over_import_resolves_to_the_local_identity() -> Result<(), String> {
    let provider = parse("pub def helper() -> int:\n  return 1\n", "collision provider");
    let consumer_source = r#"
from lib import helper

def helper() -> int:
  return 2

def read() -> None:
  observed = helper
"#;
    let consumer = parse(consumer_source, "collision consumer");
    let mut checker = TypeChecker::new();
    checker.set_current_module_path(Some(vec!["consumer".to_string()]));
    checker
        .check_with_imports(&consumer, &[("lib", &provider)])
        .map_err(|errors| format!("collision consumer should typecheck today: {errors:?}"))?;

    let observed = identity_at(
        &checker,
        nth_span(consumer_source, "helper", 2)?,
        "post-collision reference",
    )?;
    assert_eq!(
        observed.origin,
        SymbolOrigin::Module(vec!["consumer".to_string()]),
        "the reference resolves to the local declaration, so it must carry the local identity"
    );
    assert_eq!(observed.kind, SemanticSourceTargetKind::Function);
    Ok(())
}
