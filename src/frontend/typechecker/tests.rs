Warning: truncated output (original token count: 155702)
Total output lines: 20753

//! Typechecker unit tests.

use super::*;
use crate::frontend::api_metadata::{
    ApiDeclaration, ApiFunction, CHECKED_API_METADATA_SCHEMA_VERSION, CheckedApiMetadata, CheckedApiMetadataPackage,
    SourceAnchor, SourceSpan, collect_checked_api_alias_metadata, collect_checked_api_metadata,
    materialize_api_alias_projections, materialize_checked_api_public_namespaces,
};
use crate::frontend::ast::TypeConstraintKey;
use crate::frontend::library_exports::{
    CheckedAliasExport, CheckedExportIdentity, CheckedExportKind, CheckedNamedExport, CheckedPartialTargetKind,
    CheckedPresetValue, collect_checked_public_exports,
};
use crate::frontend::library_manifest_index::{
    LibraryArtifactMetadata, LibraryManifestFailureKind, LibraryManifestIndex, LibraryManifestIndexEntry,
    LibraryManifestLoadFailure,
};
use crate::frontend::testing_markers::TestingFixtureScope;
use crate::frontend::{lexer, parser};
use crate::library_manifest::{
    AliasExport, ClassExport, ConstExport, EnumExport, EnumValueExport, EnumValueTypeExport, EnumVariantExport,
    ExportIdentity, ExportIdentityKind, ExportIdentityProjection, FieldExport, FieldVisibilityExport, FunctionExport,
    LIBRARY_IDENTITY_GRAPH_SCHEMA_VERSION, LibraryContractMetadata, LibraryExports, LibraryIdentityGraph,
    LibraryManifest, LibraryRustAbi, MethodExport, ModelExport, ParamDefaultCallArgExport,
    ParamDefaultCallSignatureExport, ParamDefaultExport, ParamExport, ParamKindExport, PartialExport,
    PartialPresetExport, PartialTargetKindExport, PresetValueExport, ReceiverExport, StaticExport, TraitExport,
    TypeAliasExport, TypeBoundExport, TypeParamExport, TypeRef,
};
use crate::provider::{
    NamespaceAuthority, ProviderIdentity, ProviderPlan, ProviderPlanError, ProviderProvenance, ProviderRecord,
};
#[cfg(feature = "rust_inspect")]
use crate::rust_inspect::{Inspector, InspectorConfig, write_borrowed_param_probe_crate, write_substrait_probe_crate};
use incan_core::interop::{
    CoercionPolicy, RustFieldInfo, RustFunctionSig, RustImplementedTrait, RustItemKind, RustItemMetadata,
    RustMethodSig, RustParam, RustTraitAssoc, RustTraitInfo, RustTypeInfo, RustTypeShape, RustVariantInfo,
    RustVisibility,
};
use incan_core::lang::surface::constructors::{self as surface_constructors, ConstructorId};
use incan_core::lang::traits::{self as builtin_traits, TraitId};
use incan_core::lang::types::collections::{self as collection_types, CollectionTypeId};
use std::collections::{BTreeSet, HashMap};
#[cfg(feature = "rust_inspect")]
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

fn check_str(source: &str) -> Result<(), Vec<CompileError>> {
    let tokens = lexer::lex(source)?;
    let ast = parser::parse(&tokens)?;
    check(&ast)
}

#[test]
fn metadata_free_path_new_records_a_borrowed_argument_boundary() -> Result<(), String> {
    let source = r#"
from rust::std::path import Path

def main() -> None:
  path = "example.txt"
  _ = Path.new(path)
"#;
    let ast = parse_program(source, "metadata-free Path.new borrow boundary");
    let mut checker = TypeChecker::new();
    checker
        .check_program(&ast)
        .map_err(|errors| format!("Path.new should typecheck: {errors:?}"))?;

    let params = checker
        .type_info()
        .calls
        .call_site_callable_params
        .values()
        .find(|params| params.len() == 1)
        .ok_or("Path.new call should record one callable parameter")?;
    assert!(
        matches!(params[0].ty, ResolvedType::Ref(_)),
        "Path.new should retain a borrowed Rust parameter, got {:?}",
        params[0].ty
    );
    Ok(())
}

fn parse_program(source: &str, context: &str) -> crate::frontend::ast::Program {
    let tokens = lexer::lex(source).unwrap_or_else(|errs| panic!("{context} lex failed: {errs:?}"));
    parser::parse(&tokens).unwrap_or_else(|errs| panic!("{context} parse failed: {errs:?}"))
}

#[test]
fn set_constructor_calls_record_canonical_collection_identity_issue951() -> Result<(), String> {
    let ast = parse_program(
        r#"
def main(values: List[str]) -> None:
  lower = set(values)
  canonical = Set(values)
"#,
        "issue951 set constructor identity",
    );
    let mut checker = TypeChecker::new();
    checker
        .check_program(&ast)
        .map_err(|errors| format!("set constructors should typecheck: {errors:?}"))?;

    let constructors = checker
        .type_info()
        .calls
        .resolved_collection_constructors
        .values()
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(
        constructors,
        vec![CollectionTypeId::Set, CollectionTypeId::Set],
        "both accepted spellings should resolve through the canonical Set identity"
    );
    Ok(())
}

#[test]
fn user_defined_set_call_does_not_record_collection_constructor_issue951() -> Result<(), String> {
    let ast = parse_program(
        r#"
def set(values: List[str]) -> int:
  return len(values)

def main(values: List[str]) -> None:
  count = set(values)
"#,
        "issue951 shadowed set function",
    );
    let mut checker = TypeChecker::new();
    checker
        .check_program(&ast)
        .map_err(|errors| format!("shadowing source function should typecheck: {errors:?}"))?;

    assert!(
        checker.type_info().calls.resolved_collection_constructors.is_empty(),
        "a user-defined set function must not be lowered as the Set collection constructor"
    );
    Ok(())
}

#[test]
fn set_constructor_rejects_more_than_one_source_collection_issue951() {
    let errors = check_str_err(
        r#"
def main(left: List[str], right: List[str]) -> None:
  invalid = set(left, right)
"#,
        "set() should reject multiple source collections",
    );

    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("set() expects at most 1 argument(s), got 2")),
        "expected a source diagnostic before lowering, got {errors:?}"
    );
}

#[test]
fn stdlib_module_function_calls_accept_default_arguments() -> Result<(), String> {
    let source = r#"
from std.encoding import hex
from std.io import BytesIO

def main(payload: bytes) -> None:
  target = BytesIO()
  encoded = hex.encode(payload, target)
"#;
    check_str(source).map_err(|errs| format!("{errs:?}"))
}

fn check_str_err(source: &str, context: &str) -> Vec<CompileError> {
    match check_str(source) {
        Err(errs) => errs,
        Ok(()) => panic!("{context}"),
    }
}

fn check_str_with_library_index_err(
    source: &str,
    library_index: LibraryManifestIndex,
    context: &str,
) -> Result<Vec<CompileError>, String> {
    match check_str_with_library_index(source, library_index) {
        Err(errs) => Ok(errs),
        Ok(()) => Err(context.to_string()),
    }
}

fn check_str_warnings(source: &str, context: &str) -> Vec<CompileError> {
    let tokens = match lexer::lex(source) {
        Ok(tokens) => tokens,
        Err(errs) => panic!("{context} lex failed: {errs:?}"),
    };
    let ast = match parser::parse(&tokens) {
        Ok(ast) => ast,
        Err(errs) => panic!("{context} parse failed: {errs:?}"),
    };
    let mut checker = TypeChecker::new();
    if let Err(errs) = checker.check_program(&ast) {
        panic!("{context} typecheck failed: {errs:?}");
    }
    checker.warnings
}

fn has_unknown_symbol_error(errors: &[CompileError], symbol: &str) -> bool {
    let needle = format!("Unknown symbol '{symbol}'");
    errors.iter().any(|err| err.message.contains(&needle))
}

#[test]
fn test_ellipsis_abstract_method_outside_trait_is_type_error() {
    let source = r#"
model User:
  def name(self) -> str: ...
"#;
    let errs = check_str_err(source, "abstract concrete method should fail typechecking");
    assert!(
        errs.iter().any(|err| err
            .message
            .contains("Method 'name' must have a body outside trait declarations")),
        "expected concrete method body diagnostic, got: {errs:?}"
    );
}

fn check_str_with_library_index(source: &str, library_index: LibraryManifestIndex) -> Result<(), Vec<CompileError>> {
    let tokens = lexer::lex(source)?;
    let ast = parser::parse(&tokens)?;
    let mut checker = TypeChecker::new();
    checker.set_library_manifest_index(library_index);
    checker.check_program(&ast)
}

fn check_type_info_with_library_index(
    source: &str,
    library_index: LibraryManifestIndex,
) -> Result<TypeCheckInfo, String> {
    let tokens = lexer::lex(source).map_err(|errors| format!("lex failed: {errors:?}"))?;
    let ast = parser::parse(&tokens).map_err(|errors| format!("parse failed: {errors:?}"))?;
    let mut checker = TypeChecker::new();
    checker.set_library_manifest_index(library_index);
    checker
        .check_program(&ast)
        .map_err(|errors| format!("typecheck failed: {errors:?}"))?;
    Ok(checker.type_info().clone())
}

fn synthetic_artifact_root(name: &str) -> PathBuf {
    let mut root = std::env::temp_dir();
    root.push(format!("incan_test_{name}_artifacts"));
    root.push("target");
    root.push("lib");
    root
}

fn library_index_with_rust_abi_item(name: &str, metadata: RustItemMetadata) -> LibraryManifestIndex {
    let mut manifest = LibraryManifest::new("runtime_facade", "0.1.0");
    manifest.rust_abi = LibraryRustAbi::from_items(vec![metadata]);
    LibraryManifestIndex::from_entries(HashMap::from([(
        "runtime_facade".to_string(),
        LibraryManifestIndexEntry::Loaded {
            manifest: Box::new(manifest),
            metadata: LibraryArtifactMetadata::from_crate_root(
                "runtime_facade",
                "runtime_facade",
                synthetic_artifact_root(name),
            ),
        },
    )]))
}

#[test]
fn rust_item_metadata_prefers_shipped_library_abi() {
    let manifest_metadata = RustItemMetadata {
        canonical_path: "demo_runtime::parse".to_string(),
        definition_path: Some("demo_runtime::parse".to_string()),
        visibility: RustVisibility::Public,
        kind: RustItemKind::Function(RustFunctionSig {
            type_params: Vec::new(),
            params: vec![RustParam {
                name: Some("source".to_string()),
                type_display: "&str".to_string(),
            }],
            return_type: "demo_runtime::Plan".to_string(),
            is_async: false,
            is_unsafe: false,
        }),
    };

    let mut checker = TypeChecker::new();
    checker.set_library_manifest_index(library_index_with_rust_abi_item(
        "demo_runtime_parse",
        manifest_metadata.clone(),
    ));

    let Some(actual) = checker.rust_item_metadata_for_path("rust::demo_runtime::parse") else {
        panic!("expected shipped Rust ABI metadata");
    };
    assert_eq!(actual, manifest_metadata);
}

fn clone_trait_name() -> String {
    builtin_traits::as_str(TraitId::Clone).to_string()
}

fn none_constructor_name() -> String {
    surface_constructors::as_str(ConstructorId::None).to_string()
}

#[test]
fn test_partial_function_presets_project_as_defaults() {
    let source = r#"
def route(method: str, path: str, content_type: str = "text") -> str:
  return method

get = partial route(method="GET")

def use() -> str:
  a = get(path="/health")
  b = get(method="POST", path="/submit")
  return b
"#;
    let ast = parse_program(source, "partial function defaults");
    let mut checker = TypeChecker::new();
    checker
        .check_program(&ast)
        .unwrap_or_else(|errs| panic!("typecheck failed: {errs:?}"));
    let sym = checker
        .lookup_symbol("get")
        .unwrap_or_else(|| panic!("missing projected partial symbol"));
    let SymbolKind::Function(info) = &sym.kind else {
        panic!("expected function symbol for partial, got {:?}", sym.kind);
    };
    let method = info.params.iter().find(|param| param.name() == Some("method")).unwrap();
    let path = info.params.iter().find(|param| param.name() == Some("path")).unwrap();
    let content_type = info
        .params
        .iter()
        .find(|param| param.name() == Some("content_type"))
        .unwrap();
    assert!(method.has_default, "{info:?}");
    assert!(!path.has_default, "{info:?}");
    assert!(content_type.has_default, "{info:?}");
}

#[test]
fn test_public_partial_exports_projected_defaults() {
    let source = r#"
pub def route(method: str, path: str, content_type: str = "text") -> str:
  return method

pub get = partial route(method="GET")
"#;
    let ast = parse_program(source, "partial public export");
    let mut checker = TypeChecker::new();
    checker
        .check_program(&ast)
        .unwrap_or_else(|errs| panic!("typecheck failed: {errs:?}"));

    let exports = collect_checked_public_exports(&ast, &checker);
    let get = exports
        .iter()
        .find_map(|export| match &export.kind {
            CheckedExportKind::Partial(partial) if partial.name == "get" => Some(partial),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing public partial export: {exports:?}"));
    assert_eq!(get.target_path, vec!["route"]);
    assert_eq!(get.target_kind, CheckedPartialTargetKind::Function);
    assert_eq!(get.presets[0].name, "method");
    assert_eq!(get.presets[0].value, CheckedPresetValue::String("GET".to_string()));
    let method = get.params.iter().find(|param| param.name() == Some("method")).unwrap();
    let path = get.params.iter().find(|param| param.name() == Some("path")).unwrap();
    let content_type = get
        .params
        .iter()
        .find(|param| param.name() == Some("content_type"))
        .unwrap();
    assert!(method.has_default, "{get:?}");
    assert!(!path.has_default, "{get:?}");
    assert!(content_type.has_default, "{get:?}");

    let manifest = LibraryManifest::from_checked_exports("routes".to_string(), "0.1.0".to_string(), &exports);
    assert_eq!(manifest.exports.partials.len(), 1);
    assert_eq!(
        manifest.exports.partials[0].target_kind,
        PartialTargetKindExport::Function
    );
    assert_eq!(
        manifest.exports.partials[0].presets[0].value,
        PresetValueExport::String("GET".to_string())
    );
}

#[test]
fn test_public_partial_exports_declaration_safe_preset_values() {
    let source = r#"
pub model Profile:
  pub name: str

pub def configure(headers: dict[str, str], codes: list[int], profile: Profile) -> str:
  return profile.name

pub default_config = partial configure(headers={"accept": "json"}, codes=[200], profile=Profile(name="ops"))
"#;
    let ast = parse_program(source, "partial public preset metadata");
    let mut checker = TypeChecker::new();
    checker
        .check_program(&ast)
        .unwrap_or_else(|errs| panic!("typecheck failed: {errs:?}"));

    let exports = collect_checked_public_exports(&ast, &checker);
    let default_config = exports
        .iter()
        .find_map(|export| match &export.kind {
            CheckedExportKind::Partial(partial) if partial.name == "default_config" => Some(partial),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing partial export: {exports:?}"));

    assert!(
        default_config
            .presets
            .iter()
            .any(|preset| matches!(preset.value, CheckedPresetValue::Dict(_))),
        "{default_config:?}"
    );
    assert!(
        default_config
            .presets
            .iter()
            .any(|preset| matches!(preset.value, CheckedPresetValue::List(_))),
        "{default_config:?}"
    );
    assert!(
        default_config
            .presets
            .iter()
            .any(|preset| matches!(preset.value, CheckedPresetValue::ModelLiteral { .. })),
        "{default_config:?}"
    );
}

#[test]
fn test_source_imported_partial_records_projection_metadata() {
    let provider = parse_program(
        r#"
pub model Policy:
  pub family: FrozenStr
  pub role: FrozenStr
  pub enabled: bool

pub policy = partial Policy(family="core", enabled=true)
"#,
        "imported partial projection provider",
    );
    let consumer = parse_program(
        r#"
from provider import Policy, policy

const DEFAULT_POLICY: Policy = policy(role="admin")

def runtime_policy_enabled() -> bool:
  return policy(role="runtime").enabled
"#,
        "imported partial projection consumer",
    );
    let mut checker = TypeChecker::new();
    checker
        .check_with_imports(&consumer, &[("provider", &provider)])
        .unwrap_or_else(|errs| panic!("imported partial projection should typecheck: {errs:?}"));
    assert!(
        checker.type_info.partial_projection("policy").is_some(),
        "imported partial should preserve projection metadata"
    );
    let policy_symbol = checker
        .lookup_symbol("policy")
        .unwrap_or_else(|| panic!("imported partial symbol should exist"));
    let SymbolKind::Function(policy_info) = &policy_symbol.kind else {
        panic!("imported partial should be a function, got {:?}", policy_symbol.kind);
    };
    let param_names = policy_info
        .params
        .iter()
        .map(|param| param.name.as_deref().unwrap_or("<anonymous>"))
        .collect::<Vec<_>>();
    assert_eq!(param_names, ["family", "role", "enabled"], "{:?}", policy_info.params);
    assert!(
        matches!(
            policy_info.params.as_slice(),
            [
                CallableParam {
                    ty: ResolvedType::FrozenStr,
                    ..
                },
                CallableParam {
                    ty: ResolvedType::FrozenStr,
                    ..
                },
                CallableParam {
                    ty: ResolvedType::Bool,
                    ..
                }
            ]
        ),
        "expected imported partial params to preserve FrozenStr, got {:?}",
        policy_info.params
    );
    let call_params = checker
        .type_info
        .calls
        .call_site_callable_params
        .values()
        .find(|params| {
            params
                .iter()
                .filter_map(|param| param.name.as_deref())
                .collect::<Vec<_>>()
                == ["family", "role", "enabled"]
        })
        .unwrap_or_else(|| {
            panic!(
                "runtime partial call should record source-ordered call-site params, got {:?}",
                checker.type_info.calls.call_site_callable_params
            )
        });
    assert!(
        matches!(
            call_params.as_slice(),
            [
                CallableParam {
                    ty: ResolvedType::FrozenStr,
                    ..
                },
                CallableParam {
                    ty: ResolvedType::FrozenStr,
                    ..
                },
                CallableParam {
                    ty: ResolvedType::Bool,
                    ..
                }
            ]
        ),
        "expected runtime partial call params to preserve FrozenStr, got {call_params:?}"
    );
}

#[test]
fn test_top_level_partial_rejects_runtime_preset_values() {
    let source = r#"
def default_method() -> str:
  return "GET"

def route(method: str, path: str) -> str:
  return method

get = partial route(method=default_method())
"#;
    let errors = check_str_err(source, "top-level partial runtime preset should fail");
    assert!(
        errors.iter().any(|error| error
            .message
            .contains("Top-level partial preset 'method' must be declaration-safe")),
        "expected declaration-safe preset diagnostic, got {errors:?}"
    );
}

#[test]
fn test_top_level_partial_invalid_diagnostics_are_complete() {
    for (source, expected, context) in [
        (
            r#"
def route(method: str) -> str:
  return method

noop = partial route()
"#,
            "must preset at least one keyword",
            "empty partial should be rejected",
        ),
        (
            r#"
def route(method: str) -> str:
  return method

get = partial route(method="GET", method="POST")
"#,
            "repeats preset keyword 'method'",
            "duplicate partial preset should be rejected",
        ),
        (
            r#"
def route(method: str) -> str:
  return method

get = partial route(verb="GET")
"#,
            "presets unknown parameter 'verb'",
            "unknown partial preset should be rejected",
        ),
        (
            r#"
const method = "GET"
get = partial method(value="GET")
"#,
            "targets unsupported symbol 'method'",
            "unsupported partial target should be rejected",
        ),
        (
            r#"
static method: str = "GET"
get = partial method(value="GET")
"#,
            "targets unsupported symbol 'method'",
            "unsupported static partial target should be rejected",
        ),
        (
            r#"
trait Labelled:
  def label(self) -> str: ...

get = partial Labelled(value="GET")
"#,
            "targets unsupported symbol 'Labelled'",
            "unsupported trait partial target should be rejected",
        ),
        (
            r#"
enum Method:
  Get

get = partial Get(value="GET")
"#,
            "targets unsupported symbol 'Get'",
            "unsupported enum variant partial target should be rejected",
        ),
        (
            r#"
get = partial route(method="GET")
"#,
            "targets unknown callable 'route'",
            "unknown partial target should be rejected",
        ),
        (
            r#"
def route(method: str, **labels: str) -> str:
  return method

get = partial route(labels={"accept": "json"})
"#,
            "cannot target callable 'route' because parameter 'labels' is a rest parameter",
            "rest keyword partial target should be rejected",
        ),
        (
            r#"
def route(method: str, *segments: str) -> str:
  return method

get = partial route(method="GET")
"#,
            "cannot target callable 'route' because parameter 'segments' is a rest parameter",
            "rest positional partial target should be rejected even when preset fills a normal parameter",
        ),
    ] {
        let errors = check_str_err(source, context);
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "expected diagnostic containing `{expected}` for {context}, got {errors:?}"
        );
    }
}

#[test]
fn test_top_level_partial_cycles_are_rejected() {
    for (source, expected) in [
        (
            r#"
get = partial get(method="GET")
"#,
            "Partial cycle detected: get -> get",
        ),
        (
            r#"
get = partial alias_get(method="GET")
alias_get = get
"#,
            "Partial cycle detected: get -> alias_get -> get",
        ),
        (
            r#"
left = partial right(method="GET")
right = partial left(method="POST")
"#,
            "Partial cycle detected",
        ),
    ] {
        let errors = check_str_err(source, "partial cycle should fail");
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "expected partial cycle diagnostic containing `{expected}`, got {errors:?}"
        );
    }
}

#[test]
fn test_public_partial_rejects_private_target_and_private_preset_values() {
    for (source, expected) in [
        (
            r#"
def route(method: str) -> str:
  return method

pub get = partial route(method="GET")
"#,
            "Public partial 'get' targets private symbol 'route'",
        ),
        (
            r#"
const DEFAULT_METHOD = "GET"

pub def route(method: str) -> str:
  return method

pub get = partial route(method=DEFAULT_METHOD)
"#,
            "Public partial 'get' preset 'method' references private symbol 'DEFAULT_METHOD'",
        ),
        (
            r#"
model Profile:
  name: str

pub def configure(profile: Profile) -> str:
  return profile.name

pub default_config = partial configure(profile=Profile(name="ops"))
"#,
            "Public partial 'default_config' preset 'profile' references private symbol 'Profile'",
        ),
    ] {
        let errors = check_str_err(source, "public partial visibility leak should fail");
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "expected visibility diagnostic containing `{expected}`, got {errors:?}"
        );
    }
}

#[test]
fn test_import_module_collects_public_partial_as_callable() {
    let library = parse_program(
        r#"
pub def route(method: str, path: str) -> str:
  return path

pub get = partial route(method="GET")
"#,
        "partial import library",
    );
    let consumer = parse_program(
        r#"
def use() -> str:
  return get(path="/health")
"#,
        "partial import consumer",
    );

    let mut checker = TypeChecker::new();
    checker.import_module(&library, "routes");
    checker
        .check_program(&consumer)
        .unwrap_or_else(|errs| panic!("consumer should import public partial callable: {errs:?}"));
}

#[test]
fn test_from_import_accepts_public_partial_export() {
    let library = parse_program(
        r#"
pub model Spec:
  pub namespace: str
  pub policy: str
  pub klass: str
  pub lifecycle: str

pub core_spec = partial Spec(namespace="core", policy="portable")
"#,
        "partial import library",
    );
    let consumer = parse_program(
        r#"
from presets import core_spec

def use() -> str:
  spec = core_spec(klass="scalar", lifecycle="v1")
  return spec.namespace
"#,
        "partial from-import consumer",
    );

    let mut checker = TypeChecker::new();
    checker
        .check_with_imports(&consumer, &[("presets", &library)])
        .unwrap_or_else(|errs| panic!("consumer should import public partial callable by name: {errs:?}"));
}

#[test]
fn test_type_name_value_requires_type_token_expected_context() {
    let source = r#"
def accepts_any[T](value: T) -> None:
  return

def use() -> None:
  accepts_any(int)
"#;
    let errs = check_str_err(
        source,
        "bare primitive type value should require Type[T] expected context",
    );
    assert!(
        errs.iter()
            .any(|err| err.message.contains("Cannot use type 'int' as a value")),
        "expected type-name-as-value diagnostic, got {errs:?}"
    );
}

#[test]
fn test_generic_type_token_parameter_accepts_type_name_value() {
    let source = r#"
def accepts_type[T](value: Type[T]) -> str:
  return "ok"

def use() -> str:
  return accepts_type(int)
"#;
    let result = check_str(source);
    assert!(
        result.is_ok(),
        "expected generic Type[T] parameter to accept primitive type token, got {result:?}"
    );
}

#[test]
fn test_top_level_alias_preserves_overloaded_type_token_function_set() -> Result<(), String> {
    let source = r#"
model ColumnExpr:
  name: str

model IntColumnExpr:
  source: str

model FloatColumnExpr:
  source: str

def col(name: str) -> ColumnExpr:
  return ColumnExpr(name=name)

def cast(expr: ColumnExpr, target: Type[int]) -> IntColumnExpr:
  return IntColumnExpr(source=expr.name)

def cast(expr: ColumnExpr, target: Type[float]) -> FloatColumnExpr:
  return FloatColumnExpr(source=expr.name)

def cast(expr: ColumnExpr, target: str) -> ColumnExpr:
  return ColumnExpr(name=target)

safe_cast = alias cast

def use() -> None:
  typed: FloatColumnExpr = safe_cast(col("amount"), float)
  fallback: ColumnExpr = safe_cast(col("amount"), "float64")
  return
"#;
    let tokens = lexer::lex(source).map_err(|errs| format!("{errs:?}"))?;
    let ast = parser::parse(&tokens).map_err(|errs| format!("{errs:?}"))?;
    let mut checker = TypeChecker::new();
    checker
        .check_program(&ast)
        .map_err(|errs| format!("overloaded alias should typecheck: {errs:?}"))?;

    let alias = checker
        .lookup_symbol("safe_cast")
        .ok_or_else(|| "expected overloaded alias symbol".to_string())?;
    let SymbolKind::FunctionOverloads(overloads) = &alias.kind else {
        return Err(format!("expected safe_cast overload set, got {:?}", alias.kind));
    };
    assert_eq!(overloads.len(), 3);
    assert_eq!(
        checker
            .type_info()
            .function_overloads("safe_cast")
            .map(|overloads| overloads.len()),
        Some(3)
    );
    Ok(())
}

#[test]
fn test_from_import_accepts_public_source_enum_variant_export() -> Result<(), Box<dyn std::error::Error>> {
    let library = parse_program(
        r#"
pub enum Status(str):
  Active = "active"
  Disabled = "disabled"
"#,
        "enum variant import library",
    );
    let consumer = parse_program(
        r#"
from statuses import Active, Status

def current() -> Status:
  return Active
"#,
        "enum variant import consumer",
    );

    let mut checker = TypeChecker::new();
    checker
        .check_with_imports(&consumer, &[("statuses", &library)])
        .map_err(|errs| format!("consumer should import public source enum variants by name: {errs:?}"))?;
    Ok(())
}

#[test]
fn test_dependency_overload_cache_keeps_module_local_symbols_when_spans_collide()
-> Result<(), Box<dyn std::error::Error>> {
    let left = parse_program(
        r#"
pub model Alpha:
  pub value: str

pub model Bravo:
  pub value: str

pub def choose(value: Type[Alpha]) -> Alpha:
  return Alpha(value="a")

pub def choose(value: Type[Bravo]) -> Bravo:
  return Bravo(value="b")
"#,
        "left overload dependency",
    );
    let right = parse_program(
        r#"
pub model Gamma:
  pub value: str

pub model Delta:
  pub value: str

pub def choose(value: Type[Gamma]) -> Gamma:
  return Gamma(value="g")

pub def choose(value: Type[Delta]) -> Delta:
  return Delta(value="d")
"#,
        "right overload dependency",
    );
    let consumer = parse_program(
        r#"
from left import Alpha, choose as choose_left
from right import Gamma, choose as choose_right

def use() -> None:
  choose_left(Alpha)
  choose_right(Gamma)
"#,
        "overload span collision consumer",
    );

    let mut checker = TypeChecker::new();
    checker
        .check_with_imports(&consumer, &[("left", &left), ("right", &right)])
        .map_err(|errs| format!("consumer should import both overload groups independently: {errs:?}"))?;

    let left_path = ImportPath {
        is_absolute: false,
        parent_levels: 0,
        segments: vec!["left".to_string()],
    };
    let right_path = ImportPath {
        is_absolute: false,
        parent_levels: 0,
        segments: vec!["right".to_string()],
    };
    let left_symbol = checker
        .dependency_member_symbol_for_path(&left_path, "choose")
        .ok_or_else(|| "expected left.choose to be present in dependency member cache".to_string())?;
    let SymbolKind::FunctionOverloads(left_overloads) = left_symbol else {
        return Err(format!("expected left.choose to be cached as function overloads, got {left_symbol:?}").into());
    };
    let right_symbol = checker
        .dependency_member_symbol_for_path(&right_path, "choose")
        .ok_or_else(|| "expected right.choose to be present in dependency member cache".to_string())?;
    let SymbolKind::FunctionOverloads(right_overloads) = right_symbol else {
        return Err(format!("expected right.choose to be cached as function overloads, got {right_symbol:?}").into());
    };

    let left_return_types = left_overloads
        .iter()
        .map(|overload| overload.info.return_type.to_string())
        .collect::<Vec<_>>();
    let right_return_types = right_overloads
        .iter()
        .map(|overload| overload.info.return_type.to_string())
        .collect::<Vec<_>>();
    assert_eq!(left_return_types, vec!["Alpha", "Bravo"]);
    assert_eq!(right_return_types, vec!["Gamma", "Delta"]);

    let left_emitted_names = left_overloads
        .iter()
        .filter_map(|overload| overload.info.emitted_name.as_deref())
        .collect::<Vec<_>>();
    let right_emitted_names = right_overloads
        .iter()
        .filter_map(|overload| overload.info.emitted_name.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(left_emitted_names.len(), 2);
    assert_eq!(right_emitted_names.len(), 2);
    assert!(
        left_emitted_names
            .iter()
            .all(|name| name.starts_with("choose_overload_")),
        "left overloads should keep deterministic emitted names, got {left_emitted_names:?}"
    );
    assert!(
        right_emitted_names
            .iter()
            .all(|name| name.starts_with("choose_overload_")),
        "right overloads should keep deterministic emitted names, got {right_emitted_names:?}"
    );
    assert_ne!(
        left_emitted_names, right_emitted_names,
        "same-span overload declarations from different modules must not collapse to one emitted-name set"
    );
    Ok(())
}

#[test]
fn test_method_partial_presets_project_as_defaults_for_trait_and_model() {
    let source = r#"
trait Named:
  def label(self, prefix: str, suffix: str = "!") -> str:
    return prefix
  short = partial label(prefix="name")

model User with Named:
  name: str
  def label(self, prefix: str, suffix: str = "!") -> str:
    return prefix
  loud = partial label(prefix="user")

def use(user: User) -> str:
  a = user.loud()
  b = user.loud(prefix="admin")
  c = user.short()
  return b
"#;
    check_str(source).unwrap_or_else(|errs| panic!("typecheck failed: {errs:?}"));
}

#[test]
fn test_method_partial_preset_values_are_typechecked() {
    let source = r#"
trait Named:
  def label(self, prefix: str) -> str:
    return prefix
  short = partial label(prefix=1)
"#;
    let errors = check_str_err(source, "method partial preset should be typechecked");
    let messages: Vec<_> = errors.iter().map(|err| err.message.as_str()).collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Type mismatch") || message.contains("expected str")),
        "expected type mismatch, got {messages:?}"
    );
}

#[test]
fn test_method_partial_name_collisions_are_rejected() {
    for (source, expected) in [
        (
            r#"
trait Named:
  def label(self, prefix: str) -> str:
    return prefix
  label = partial label(prefix="name")
"#,
            "Duplicate method partial 'Named.label'",
        ),
        (
            r#"
trait Named:
  def label(self, prefix: str) -> str:
    return prefix
  short = partial label(prefix="name")
  short = partial label(prefix="user")
"#,
            "Duplicate method partial 'Named.short'",
        ),
        (
            r#"
trait Named:
  def label(self, prefix: str) -> str:
    return prefix
  short = label
  short = partial label(prefix="name")
"#,
            "Duplicate method partial 'Named.short'",
        ),
    ] {
        let errors = check_str_err(source, "method partial collision should fail");
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "expected method partial collision diagnostic containing `{expected}`, got {errors:?}"
        );
    }
}

#[test]
fn test_method_partial_can_target_same_type_method_alias() {
    let source = r#"
trait Named:
  def label(self, prefix: str) -> str:
    return prefix
  labelled = label
  short = partial labelled(prefix="name")

model User with Named:
  def label(self, prefix: str) -> str:
    return prefix

def use(user: User) -> str:
  return user.short()
"#;
    check_str(source).unwrap_or_else(|errs| panic!("method partial targeting alias should typecheck: {errs:?}"));
}

#[test]
fn test_trait_partial_override_conflict_is_rejected() {
    let source = r#"
trait Named:
  def label(self, prefix: str) -> str:
    return prefix
  short = partial label(prefix="name")

model User with Named:
  def label(self, prefix: str) -> str:
    return prefix
  def short(self, prefix: int) -> str:
    return "bad"
"#;
    let errors = check_str_err(source, "trait partial override conflict should fail");
    assert!(
        errors.iter().any(|error| error
            .message
            .contains("Trait 'Named' requires 'User'::short to match its signature")),
        "expected trait partial override conflict, got {errors:?}"
    );
}

#[test]
fn test_inherited_trait_partial_ambiguity_is_rejected() {
    let source = r#"
trait Left:
  def label(self, prefix: str) -> str:
    return prefix
  short = partial label(prefix="left")

trait Right:
  def label(self, prefix: str) -> str:
    return prefix
  short = partial label(prefix="right")

trait Both with Left, Right:
  def both(self) -> str: ...

model User with Both:
  def label(self, prefix: str) -> str:
    return prefix
  def both(self) -> str:
    return "both"
"#;
    let errors = check_str_err(source, "inherited trait partial ambiguity should fail");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("Ambiguous trait method 'short'")),
        "expected inherited trait partial ambiguity diagnostic, got {errors:?}"
    );
}

#[test]
fn test_subtrait_partial_override_must_match_inherited_partial_signature() {
    let source = r#"
trait Base:
  def label(self, prefix: str) -> str:
    return prefix
  short = partial label(prefix="base")

trait Child with Base:
  def labelled(self, prefix: str, count: int) -> str:
    return prefix
  short = partial labelled(prefix="child")
"#;
    let errors = check_str_err(source, "incompatible subtrait partial override should fail");
    assert!(
        errors.iter().any(|error| error
            .message
            .contains("Trait 'Base' requires 'Child'::short to match its signature")),
        "expected inherited partial override conflict, got {errors:?}"
    );
}

#[test]
fn test_subtrait_partial_can_override_inherited_partial_with_compatible_signature() {
    let source = r#"
trait Base:
  def label(self, prefix: str) -> str:
    return prefix
  short = partial label(prefix="base")

trait Child with Base:
  def label_child(self, prefix: str) -> str:
    return prefix
  short = partial label_child(prefix="child")

model User with Child:
  def label(self, prefix: str) -> str:
    return prefix
  def label_child(self, prefix: str) -> str:
    return prefix
"#;
    check_str(source).unwrap_or_else(|errs| panic!("compatible inherited partial override should typecheck: {errs:?}"));
}

#[test]
fn test_generic_trait_bound_partial_ambiguity_is_rejected() {
    let source = r#"
trait Left:
  def label(self, prefix: str) -> str:
    return prefix
  short = partial label(prefix="left")

trait Right:
  def label(self, prefix: str) -> str:
    return prefix
  short = partial label(prefix="right")

trait Both with Left, Right:
  def both(self) -> str: ...

def use[T with Both](value: T) -> str:
  return value.short()
"#;
    let errors = check_str_err(source, "generic trait-bound partial ambiguity should fail");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("Ambiguous trait method 'short'")),
        "expected generic trait-bound partial ambiguity diagnostic, got {errors:?}"
    );
}

#[test]
fn rfc009_exact_width_numeric_widening_typechecks() -> Result<(), String> {
    let source = r#"
def main() -> None:
  small: i16 = 120
  wide: i64 = small
"#;
    check_str(source).map_err(|errs| format!("{errs:?}"))
}

#[test]
fn rfc009_exact_width_numeric_narrowing_requires_explicit_policy() {
    let source = r#"
def main() -> None:
  wide: i16 = 120
  narrow: i8 = wide
"#;
    let errors = check_str_err(source, "expected narrowing assignment to fail");
    assert!(
        errors
            .iter()
            .any(|err| err.message.contains("expected 'i8', found 'i16'")),
        "expected i16 -> i8 mismatch, got: {errors:?}"
    );
}

#[test]
fn rfc009_integer_literals_are_range_checked_for_exact_width_targets() {
    let source = r#"
def main() -> None:
  small: i8 = 300
"#;
    let errors = check_str_err(source, "expected out-of-range i8 literal to fail");
    assert!(
        errors
            .iter()
            .any(|err| err.message.contains("Integer literal 300 does not fit in i8")),
        "expected i8 range diagnostic, got: {errors:?}"
    );
}

#[test]
fn rfc009_const_integer_literals_use_exact_width_annotation() -> Result<(), String> {
    let source = r#"
const NA…135702 tokens truncated…c associated calls must enforce owner arity".into()),
        Err(errors) => errors,
    };
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("new expects 1 explicit type argument(s), got 2")),
        "expected owner-generic arity diagnostic, got {errors:?}"
    );
    Ok(())
}

#[cfg(feature = "rust_inspect")]
#[test]
fn rust_associated_calls_apply_owner_generics_to_parameters_and_returns() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
from rust::demo import Factory

def explicit() -> Factory[i64, str]:
  return Factory.new[i64, str](1, "marker")

def contextual() -> Factory[i64, str]:
  return Factory.new(7, "marker")

def owner_value() -> str:
  return Factory.first[str, i64]("value")

def accept_factory(value: Factory[i64, str]) -> None:
  pass

def parameter_context() -> None:
  accept_factory(Factory.new(7, "marker"))
"#;
    let ast = parse_program(source, "Rust associated owner substitution");
    let tmp = seeded_rust_inspect_workspace()?;
    let manifest_dir = tmp.path().to_path_buf();
    let mut checker = TypeChecker::new();
    checker.set_rust_inspect_manifest_dir(manifest_dir.clone());
    checker.rust_inspect_cache.insert_test_item(
        &manifest_dir,
        RustItemMetadata {
            canonical_path: "demo::Factory".to_string(),
            definition_path: Some("demo::Factory".to_string()),
            visibility: RustVisibility::Public,
            kind: RustItemKind::Type(RustTypeInfo {
                type_params: vec!["T".to_string(), "U".to_string()],
                has_const_params: false,
                alias_target: None,
                metadata_completeness: Default::default(),
                methods: vec![
                    RustMethodSig {
                        name: "new".to_string(),
                        signature: RustFunctionSig {
                            type_params: Vec::new(),
                            params: vec![
                                RustParam {
                                    name: Some("value".to_string()),
                                    type_display: "T".to_string(),
                                },
                                RustParam {
                                    name: Some("marker".to_string()),
                                    type_display: "U".to_string(),
                                },
                            ],
                            return_type: "Self".to_string(),
                            is_async: false,
                            is_unsafe: false,
                        },
                    },
                    RustMethodSig {
                        name: "first".to_string(),
                        signature: RustFunctionSig {
                            type_params: Vec::new(),
                            params: vec![RustParam {
                                name: Some("value".to_string()),
                                type_display: "T".to_string(),
                            }],
                            return_type: "T".to_string(),
                            is_async: false,
                            is_unsafe: false,
                        },
                    },
                ],
                implemented_traits: Vec::new(),
                fields: Vec::new(),
                variants: Vec::new(),
            }),
        },
    )?;

    checker
        .check_program(&ast)
        .map_err(|errors| format!("expected owner-generic parameters and returns to specialize, got {errors:?}"))?;
    let specialized_calls = checker
        .type_info()
        .calls
        .call_site_callable_params
        .values()
        .filter(|params| {
            matches!(
                params.as_slice(),
                [
                    CallableParam {
                        ty: ResolvedType::Int,
                        ..
                    },
                    CallableParam {
                        ty: ResolvedType::Str,
                        ..
                    }
                ]
            )
        })
        .count();
    assert!(
        specialized_calls >= 3,
        "expected explicit, return-context, and parameter-context calls to preserve exact owner-specialized parameters; \
         got {:?}",
        checker.type_info().calls.call_site_callable_params
    );
    Ok(())
}

#[cfg(feature = "rust_inspect")]
fn receiver_factory_manifest(library_name: &str, value_type: &str) -> LibraryManifest {
    let target_path = vec!["rust".to_string(), "demo".to_string(), "PairFactory".to_string()];
    let checked_export = CheckedNamedExport {
        name: "PairFactory".to_string(),
        identity: CheckedExportIdentity::reexport(target_path.clone(), target_path.clone()),
        kind: CheckedExportKind::Alias(CheckedAliasExport {
            name: "PairFactory".to_string(),
            target_path,
            projected_function: None,
        }),
    };
    let mut manifest = LibraryManifest::from_checked_exports(library_name, "0.1.0", &[checked_export]);
    manifest.rust_abi = LibraryRustAbi::from_items(vec![RustItemMetadata {
        canonical_path: "demo::PairFactory".to_string(),
        definition_path: Some("demo::PairFactory".to_string()),
        visibility: RustVisibility::Public,
        kind: RustItemKind::Type(RustTypeInfo {
            type_params: vec!["T".to_string(), "U".to_string()],
            has_const_params: false,
            alias_target: None,
            metadata_completeness: Default::default(),
            methods: vec![RustMethodSig {
                name: "new".to_string(),
                signature: RustFunctionSig {
                    type_params: Vec::new(),
                    params: vec![
                        RustParam {
                            name: Some("value".to_string()),
                            type_display: value_type.to_string(),
                        },
                        RustParam {
                            name: Some("marker".to_string()),
                            type_display: "U".to_string(),
                        },
                    ],
                    return_type: "demo::PairFactory<T, U>".to_string(),
                    is_async: false,
                    is_unsafe: false,
                },
            }],
            implemented_traits: Vec::new(),
            fields: Vec::new(),
            variants: Vec::new(),
        }),
    }]);
    manifest
}

#[cfg(feature = "rust_inspect")]
#[test]
fn compiled_library_rust_reexport_restores_receiver_generic_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = receiver_factory_manifest("receiver_factory_api", "T");
    let index = LibraryManifestIndex::from_entries(HashMap::from([(
        "receiver_factory_api".to_string(),
        LibraryManifestIndexEntry::Loaded {
            manifest: Box::new(manifest),
            metadata: LibraryArtifactMetadata::from_crate_root(
                "receiver_factory_api",
                "receiver_factory_api",
                synthetic_artifact_root("receiver_factory_api"),
            ),
        },
    )]));
    let source = r#"
from pub::receiver_factory_api import PairFactory

def accept_pair(value: PairFactory[i64, str]) -> None:
  pass

def run() -> None:
  accept_pair(PairFactory.new(7, "marker"))
"#;

    check_str_with_library_index(source, index)
        .map_err(|errors| format!("expected compiled Rust reexport metadata to typecheck, got {errors:?}"))?;
    Ok(())
}

#[cfg(feature = "rust_inspect")]
#[test]
fn compiled_rust_reexport_uses_selected_provider_abi_for_method_resolution() -> Result<(), Box<dyn std::error::Error>> {
    let conflicting = receiver_factory_manifest("a_conflicting_factory", "str");
    let selected = receiver_factory_manifest("z_selected_factory", "T");
    let index = LibraryManifestIndex::from_entries(HashMap::from([
        (
            "a_conflicting_factory".to_string(),
            LibraryManifestIndexEntry::Loaded {
                manifest: Box::new(conflicting),
                metadata: LibraryArtifactMetadata::from_crate_root(
                    "a_conflicting_factory",
                    "a_conflicting_factory",
                    synthetic_artifact_root("a_conflicting_factory"),
                ),
            },
        ),
        (
            "z_selected_factory".to_string(),
            LibraryManifestIndexEntry::Loaded {
                manifest: Box::new(selected),
                metadata: LibraryArtifactMetadata::from_crate_root(
                    "z_selected_factory",
                    "z_selected_factory",
                    synthetic_artifact_root("z_selected_factory"),
                ),
            },
        ),
    ]));
    let source = r#"
from pub::z_selected_factory import PairFactory

def run() -> PairFactory[i64, str]:
  return PairFactory.new(7, "marker")
"#;

    check_str_with_library_index(source, index)
        .map_err(|errors| format!("expected the selected provider ABI to own method resolution, got {errors:?}"))?;
    Ok(())
}

#[cfg(feature = "rust_inspect")]
#[test]
fn rust_associated_context_rejects_arguments_that_conflict_with_owner_generics()
-> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
from rust::demo import Factory

def invalid() -> Factory[i64]:
  return Factory.new("text")
"#;
    let ast = parse_program(source, "Rust associated owner mismatch");
    let tmp = seeded_rust_inspect_workspace()?;
    let manifest_dir = tmp.path().to_path_buf();
    let mut checker = TypeChecker::new();
    checker.set_rust_inspect_manifest_dir(manifest_dir.clone());
    checker.rust_inspect_cache.insert_test_item(
        &manifest_dir,
        RustItemMetadata {
            canonical_path: "demo::Factory".to_string(),
            definition_path: Some("demo::Factory".to_string()),
            visibility: RustVisibility::Public,
            kind: RustItemKind::Type(RustTypeInfo {
                type_params: vec!["T".to_string()],
                has_const_params: false,
                alias_target: None,
                metadata_completeness: Default::default(),
                methods: vec![RustMethodSig {
                    name: "new".to_string(),
                    signature: RustFunctionSig {
                        type_params: Vec::new(),
                        params: vec![RustParam {
                            name: Some("value".to_string()),
                            type_display: "T".to_string(),
                        }],
                        return_type: "Self".to_string(),
                        is_async: false,
                        is_unsafe: false,
                    },
                }],
                implemented_traits: Vec::new(),
                fields: Vec::new(),
                variants: Vec::new(),
            }),
        },
    )?;

    let errors = match checker.check_program(&ast) {
        Ok(_) => return Err("contextual owner specialization must validate its parameter types".into()),
        Err(errors) => errors,
    };
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("expected 'i64'") && error.message.contains("found 'str'")),
        "expected contextual owner mismatch diagnostic, got {errors:?}"
    );
    Ok(())
}

#[cfg(feature = "rust_inspect")]
#[test]
fn rust_associated_receiver_specialization_rejects_const_generic_owners() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
from rust::demo import Factory

def invalid() -> Factory[f32]:
  return Factory.new[f32]()

def contextual() -> Factory[f32]:
  return Factory.new()
"#;
    let ast = parse_program(source, "Rust associated const-generic owner");
    let tmp = seeded_rust_inspect_workspace()?;
    let manifest_dir = tmp.path().to_path_buf();
    let mut checker = TypeChecker::new();
    checker.set_rust_inspect_manifest_dir(manifest_dir.clone());
    checker.rust_inspect_cache.insert_test_item(
        &manifest_dir,
        RustItemMetadata {
            canonical_path: "demo::Factory".to_string(),
            definition_path: Some("demo::Factory".to_string()),
            visibility: RustVisibility::Public,
            kind: RustItemKind::Type(RustTypeInfo {
                type_params: vec!["T".to_string()],
                has_const_params: true,
                alias_target: None,
                metadata_completeness: Default::default(),
                methods: vec![RustMethodSig {
                    name: "new".to_string(),
                    signature: RustFunctionSig {
                        type_params: Vec::new(),
                        params: Vec::new(),
                        return_type: "Self".to_string(),
                        is_async: false,
                        is_unsafe: false,
                    },
                }],
                implemented_traits: Vec::new(),
                fields: Vec::new(),
                variants: Vec::new(),
            }),
        },
    )?;

    let errors = match checker.check_program(&ast) {
        Ok(_) => return Err("const-generic receiver specialization must fail closed".into()),
        Err(errors) => errors,
    };
    let const_generic_errors = errors
        .iter()
        .filter(|error| error.message.contains("has const generic parameters"))
        .count();
    assert_eq!(
        const_generic_errors, 2,
        "expected explicit and contextual const-generic receiver diagnostics, got {errors:?}"
    );
    Ok(())
}

#[test]
fn explicit_call_type_args_rejected_on_indirect_function_value_call() {
    let source = r#"
def id[T](x: T) -> T:
  return x

def run() -> int:
  let f = id
  return f[int](1)
"#;
    let errs = check_str_err(source, "expected unsupported explicit type args on indirect call");
    assert!(
        errs.iter()
            .any(|e| e.message.contains("not supported for this call form")),
        "expected unsupported call-site type args diagnostic, got {errs:?}"
    );
}

fn typecheck_info_for_module(
    source: &str,
    module_path: Vec<String>,
    context: &str,
) -> Result<TypeCheckInfo, Box<dyn std::error::Error>> {
    let tokens = lexer::lex(source).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
    let ast = parser::parse(&tokens).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
    let mut checker = TypeChecker::new();
    checker.set_current_module_path(Some(module_path));
    checker
        .check_program(&ast)
        .map_err(|errs| std::io::Error::other(format!("{context}: {errs:?}")))?;
    Ok(checker.type_info().clone())
}

#[test]
fn type_info_semantic_fact_store_exports_expression_types_deterministically() -> Result<(), Box<dyn std::error::Error>>
{
    let module_path = vec!["facts".to_string(), "sample".to_string()];
    let info = typecheck_info_for_module(
        r#"
def run() -> int:
  value = 41
  return value + 1
"#,
        module_path.clone(),
        "semantic fact type export",
    )?;

    let facts = info.semantic_fact_store(&module_path);
    let rendered = facts.iter().map(|fact| fact.render_snapshot()).collect::<Vec<_>>();
    let mut sorted = rendered.clone();
    sorted.sort();

    assert_eq!(rendered, sorted, "semantic facts should iterate deterministically");
    assert!(
        rendered
            .iter()
            .any(|fact| fact.starts_with("expr:facts::sample#") && fact.ends_with(" type=int")),
        "expected at least one expression type fact, got {rendered:?}"
    );
    assert!(
        facts.iter().any(|fact| {
            fact.kind == incan_semantics_core::SemanticFactKind::Type
                && matches!(
                    &fact.value,
                    incan_semantics_core::SemanticFactValue::Type(incan_semantics_core::IncanType::Primitive(
                        incan_semantics_core::IncanPrimitiveType::Int
                    ))
                )
        }),
        "expected a structured int semantic type fact"
    );
    Ok(())
}

#[test]
fn type_info_semantic_fact_store_exports_source_targets() -> Result<(), Box<dyn std::error::Error>> {
    let module_path = vec!["facts".to_string(), "sample".to_string()];
    let info = typecheck_info_for_module(
        r#"
def helper() -> int:
  return 1

def run() -> int:
  return helper()
"#,
        module_path.clone(),
        "semantic fact source target export",
    )?;

    let facts = info.semantic_fact_store(&module_path);
    let rendered = facts.iter().map(|fact| fact.render_snapshot()).collect::<Vec<_>>();

    assert!(
        rendered
            .iter()
            .any(|fact| fact.contains(" symbol_target=function:facts::sample::helper")),
        "expected helper source-target fact, got {rendered:?}"
    );
    assert!(
        facts.iter().any(|fact| matches!(
            &fact.value,
            incan_semantics_core::SemanticFactValue::SourceTarget(target)
                if target.kind == incan_semantics_core::SemanticSourceTargetKind::Function
                    && target.module_path == vec!["facts".to_string(), "sample".to_string()]
                    && target.name == "helper"
        )),
        "expected structured helper source-target fact"
    );
    Ok(())
}

#[test]
fn type_info_semantic_fact_store_preserves_imported_source_targets() -> Result<(), Box<dyn std::error::Error>> {
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
    let helper_ast = parser::parse(&helper_tokens).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
    let main_tokens = lexer::lex(main_source).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
    let main_ast = parser::parse(&main_tokens).map_err(|errs| std::io::Error::other(format!("{errs:?}")))?;
    let module_path = vec!["app".to_string()];
    let mut checker = TypeChecker::new();
    checker.set_current_module_path(Some(module_path.clone()));
    checker
        .check_with_imports(&main_ast, &[("helpers", &helper_ast)])
        .map_err(|errs| std::io::Error::other(format!("semantic fact imported target export: {errs:?}")))?;

    let facts = checker.type_info().semantic_fact_store(&module_path);
    assert!(
        facts.iter().any(|fact| matches!(
            &fact.value,
            incan_semantics_core::SemanticFactValue::SourceTarget(target)
                if target.kind == incan_semantics_core::SemanticSourceTargetKind::Function
                    && target.module_path == vec!["helpers".to_string()]
                    && target.name == "helper"
        )),
        "expected imported helper source-target fact"
    );
    Ok(())
}

#[test]
fn type_info_semantic_fact_store_exports_nominal_and_collection_types() -> Result<(), Box<dyn std::error::Error>> {
    let module_path = vec!["facts".to_string(), "nominal".to_string()];
    let info = typecheck_info_for_module(
        r#"
model User:
  name: str

enum Status:
  Active

def first_user(users: list[User]) -> User:
  return users[0]

def current_status() -> Status:
  return Status.Active
"#,
        module_path.clone(),
        "semantic fact nominal type export",
    )?;

    let facts = info.semantic_fact_store(&module_path);
    assert!(
        facts.iter().any(|fact| matches!(
            &fact.value,
            incan_semantics_core::SemanticFactValue::Type(incan_semantics_core::IncanType::Named(name))
                if name == "User"
        )),
        "expected a model nominal semantic type fact"
    );
    assert!(
        facts.iter().any(|fact| matches!(
            &fact.value,
            incan_semantics_core::SemanticFactValue::Type(incan_semantics_core::IncanType::Named(name))
                if name == "Status"
        )),
        "expected an enum nominal semantic type fact"
    );
    assert!(
        facts.iter().any(|fact| matches!(
            &fact.value,
            incan_semantics_core::SemanticFactValue::Type(incan_semantics_core::IncanType::Generic { base, args })
                if base == collection_types::as_str(CollectionTypeId::List)
                    && matches!(
                        args.as_slice(),
                        [incan_semantics_core::IncanType::Named(name)] if name == "User"
                    )
        )),
        "expected a collection semantic type fact for list[User]"
    );
    Ok(())
}

#[test]
fn type_info_semantic_fact_store_exports_function_declaration_type() -> Result<(), Box<dyn std::error::Error>> {
    let module_path = vec!["facts".to_string(), "decls".to_string()];
    let info = typecheck_info_for_module(
        r#"
def add(x: int, y: int = 1) -> int:
  return x + y
"#,
        module_path.clone(),
        "semantic fact declaration type export",
    )?;

    let facts = info.semantic_fact_store(&module_path);
    let add_fact = facts
        .iter()
        .find(|fact| {
            fact.subject.to_string() == "decl:facts::decls::add"
                && fact.kind == incan_semantics_core::SemanticFactKind::Type
        })
        .ok_or("missing add declaration type fact")?;
    let incan_semantics_core::SemanticFactValue::Type(incan_semantics_core::IncanType::Function {
        params,
        return_type,
    }) = &add_fact.value
    else {
        return Err(format!("expected function type fact, got {add_fact:?}").into());
    };

    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name.as_deref(), Some("x"));
    assert!(!params[0].has_default);
    assert_eq!(params[1].name.as_deref(), Some("y"));
    assert!(params[1].has_default);
    assert!(matches!(
        return_type.as_ref(),
        incan_semantics_core::IncanType::Primitive(incan_semantics_core::IncanPrimitiveType::Int)
    ));
    Ok(())
}

#[test]
fn type_info_semantic_fact_store_exports_checked_registry_descriptions() -> Result<(), Box<dyn std::error::Error>> {
    let module_path = vec!["facts".to_string(), "registry".to_string()];
    let info = typecheck_info_for_module(
        r#"
from std.registry import Registry, SubjectKind, describe

type FunctionId = newtype str

@derive(Descriptor)
model FunctionSpec:
  summary: str

pub static functions: Registry[FunctionId, FunctionSpec] = Registry.define(
  subjects=[SubjectKind.Function],
)

@describe(functions, FunctionId("normalize"), FunctionSpec(summary="Normalize text"))
def normalize(value: str) -> str:
  return value
"#,
        module_path.clone(),
        "registry fact export",
    )?;

    let facts = info.semantic_fact_store(&module_path);
    let fact = facts
        .iter()
        .find(|fact| {
            fact.subject.to_string() == "decl:facts::registry::normalize"
                && fact.kind == incan_semantics_core::SemanticFactKind::Registry
        })
        .ok_or("missing checked registry description fact")?;
    let incan_semantics_core::SemanticFactValue::RegistryEntry(entry) = &fact.value else {
        return Err(format!("expected registry fact, got {fact:?}").into());
    };
    assert_eq!(entry.registry.to_string(), "decl:facts::registry::functions");
    assert_eq!(
        entry.subject_kind,
        incan_semantics_core::SemanticRegistrySubjectKind::Function
    );
    assert_eq!(entry.subject_identity, "facts::registry.normalize");
    assert!(matches!(
        &entry.key,
        incan_semantics_core::SemanticRegistryValue::Newtype { name, .. } if name == "FunctionId"
    ));
    assert!(matches!(
        &entry.descriptor,
        incan_semantics_core::SemanticRegistryValue::Model { name, fields }
            if name == "FunctionSpec" && fields == &vec![("summary".to_string(), incan_semantics_core::SemanticRegistryValue::String("Normalize text".to_string()))]
    ));
    Ok(())
}

#[test]
fn type_info_semantic_fact_store_exports_checked_registry_method_descriptions() -> Result<(), Box<dyn std::error::Error>>
{
    let module_path = vec!["facts".to_string(), "methods".to_string()];
    let info = typecheck_info_for_module(
        r#"
from std.registry import Registry, SubjectKind, describe

type MethodId = newtype str

@derive(Descriptor)
model MethodSpec:
  summary: str

pub static methods: Registry[MethodId, MethodSpec] = Registry.define(
  subjects=[SubjectKind.Method],
)

model Normalizer:
  @describe(methods, MethodId("normalize"), MethodSpec(summary="Normalize text"))
  def normalize(self, value: str) -> str:
    return value
"#,
        module_path.clone(),
        "registry method fact export",
    )?;

    let facts = info.semantic_fact_store(&module_path);
    let fact = facts
        .iter()
        .find(|fact| {
            fact.subject.to_string() == "decl:facts::methods::Normalizer.normalize"
                && fact.kind == incan_semantics_core::SemanticFactKind::Registry
        })
        .ok_or("missing checked registry method description fact")?;
    let incan_semantics_core::SemanticFactValue::RegistryEntry(entry) = &fact.value else {
        return Err(format!("expected registry fact, got {fact:?}").into());
    };
    assert_eq!(entry.registry.to_string(), "decl:facts::methods::methods");
    assert_eq!(
        entry.subject_kind,
        incan_semantics_core::SemanticRegistrySubjectKind::Method
    );
    assert_eq!(entry.subject_identity, "facts::methods.Normalizer.normalize");
    Ok(())
}

#[test]
fn type_info_semantic_fact_store_exports_explicit_registry_subject_entries() -> Result<(), Box<dyn std::error::Error>> {
    let module_path = vec!["facts".to_string(), "capabilities".to_string()];
    let info = typecheck_info_for_module(
        r#"
from std.registry import Registry, RegistryEntry, RegistrySubject, SubjectKind

type CapabilityId = newtype str

@derive(Descriptor)
model CapabilitySpec:
  title: str

pub static capabilities: Registry[CapabilityId, CapabilitySpec] = Registry.define(
  subjects=[SubjectKind.CompilationUnit, SubjectKind.Package],
)

pub static unit_capability: RegistryEntry[CapabilityId, CapabilitySpec] = capabilities.entry(
  key=CapabilityId("unit"),
  subject=RegistrySubject.current_unit(),
  descriptor=CapabilitySpec(title="Current unit"),
)

pub static package_capability: RegistryEntry[CapabilityId, CapabilitySpec] = capabilities.entry(
  key=CapabilityId("package"),
  subject=RegistrySubject.package(),
  descriptor=CapabilitySpec(title="Current package"),
)
"#,
        module_path.clone(),
        "explicit registry entry fact export",
    )?;

    let facts = info.semantic_fact_store(&module_path);
    let unit_entry = facts
        .iter()
        .find(|fact| {
            fact.subject.to_string() == "module:facts::capabilities"
                && fact.kind == incan_semantics_core::SemanticFactKind::Registry
        })
        .ok_or("missing checked compilation-unit registry entry")?;
    let incan_semantics_core::SemanticFactValue::RegistryEntry(unit_entry) = &unit_entry.value else {
        return Err(format!("expected registry fact, got {unit_entry:?}").into());
    };
    assert_eq!(
        unit_entry.subject_kind,
        incan_semantics_core::SemanticRegistrySubjectKind::CompilationUnit
    );
    assert_eq!(unit_entry.subject_identity, "facts::capabilities");

    let package_entry = facts
        .iter()
        .find(|fact| {
            fact.subject.to_string() == "package:facts::capabilities::package"
                && fact.kind == incan_semantics_core::SemanticFactKind::Registry
        })
        .ok_or("missing checked package registry entry")?;
    let incan_semantics_core::SemanticFactValue::RegistryEntry(package_entry) = &package_entry.value else {
        return Err(format!("expected registry fact, got {package_entry:?}").into());
    };
    assert_eq!(
        package_entry.subject_kind,
        incan_semantics_core::SemanticRegistrySubjectKind::Package
    );
    assert_eq!(package_entry.subject_identity, "facts::capabilities::package");

    let package_facts = info.semantic_fact_store_with_package(&module_path, Some("capability_catalog"));
    let package_entry = package_facts
        .iter()
        .find(|fact| {
            fact.subject.to_string() == "package:capability_catalog"
                && fact.kind == incan_semantics_core::SemanticFactKind::Registry
        })
        .ok_or("missing package-identified checked registry entry")?;
    let incan_semantics_core::SemanticFactValue::RegistryEntry(package_entry) = &package_entry.value else {
        return Err(format!("expected registry fact, got {package_entry:?}").into());
    };
    assert_eq!(package_entry.subject_identity, "capability_catalog");
    Ok(())
}

#[test]
fn registry_entry_rejects_runtime_and_compiler_reserved_mutation_surfaces() {
    let runtime_entry = check_str_err(
        r#"
from std.registry import Registry, RegistryEntry, RegistrySubject, SubjectKind

@derive(Clone, Eq)
type CapabilityId = newtype str

@derive(Descriptor)
model CapabilitySpec:
    title: str

static capabilities: Registry[CapabilityId, CapabilitySpec] = Registry.define(
    subjects=[SubjectKind.CompilationUnit],
)

static parenthesized_entry: RegistryEntry[CapabilityId, CapabilitySpec] = (
    capabilities.entry(
        key=CapabilityId("parenthesized"),
        subject=RegistrySubject.current_unit(),
        descriptor=CapabilitySpec(title="parenthesized"),
    )
)

def register_at_runtime() -> None:
    capabilities.entries.append(
        RegistryEntry(
            key=CapabilityId("forged-field"),
            descriptor=CapabilitySpec(title="forged field"),
            subject=RegistrySubject.current_unit(),
        ),
    )
    capabilities.entry(
        key=CapabilityId("runtime"),
        subject=RegistrySubject.current_unit(),
        descriptor=CapabilitySpec(title="runtime"),
    )
    capabilities._describe(
        CapabilityId("forged"),
        CapabilitySpec(title="forged"),
        RegistrySubject.current_unit(),
    )
"#,
        "Registry.entry must not become a dynamic runtime registration API",
    );
    assert!(
        runtime_entry
            .iter()
            .any(|error| error.message.contains("Registry.entry(...) is declaration-only")),
        "expected declaration-only Registry.entry diagnostic, got: {runtime_entry:?}"
    );
    assert!(
        runtime_entry
            .iter()
            .any(|error| error.message.contains("Field 'entries' on 'Registry' is private")),
        "expected private runtime-entry storage diagnostic, got: {runtime_entry:?}"
    );
    assert!(
        runtime_entry
            .iter()
            .any(|error| error.message.contains("Registry._describe(...) is compiler-reserved")),
        "expected compiler-reserved Registry._describe diagnostic, got: {runtime_entry:?}"
    );

    let compiler_helper = check_str_err(
        r#"
from std.registry import RegistrySubject

def forge_package_subject() -> RegistrySubject:
    return RegistrySubject._checked_package("forged")
"#,
        "compiler-only RegistrySubject constructor must not be callable from source",
    );
    assert!(
        compiler_helper.iter().any(|error| error
            .message
            .contains("RegistrySubject._checked_package(...) is compiler-reserved")),
        "expected compiler-reserved RegistrySubject diagnostic, got: {compiler_helper:?}"
    );
}

#[test]
fn registry_descriptions_reject_dynamic_metadata_expressions() {
    let errors = check_str_err(
        r#"
from std.registry import Registry, SubjectKind, describe

type FunctionId = newtype str

@derive(Descriptor)
model FunctionSpec:
  summary: str

pub static functions: Registry[FunctionId, FunctionSpec] = Registry.define(
  subjects=[SubjectKind.Function],
)

def dynamic_key() -> FunctionId:
  return FunctionId("dynamic")

@describe(functions, dynamic_key(), FunctionSpec(summary="Normalize text"))
def normalize(value: str) -> str:
  return value
"#,
        "expected registry structural-value rejection",
    );
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("structural calls must construct")),
        "expected structural registry metadata diagnostic, got {errors:?}"
    );
}

#[test]
fn registry_descriptions_reject_trait_method_targets_until_traits_have_canonical_subjects() {
    let errors = check_str_err(
        r#"
from std.registry import Registry, SubjectKind, describe

type FunctionId = newtype str

@derive(Descriptor)
model FunctionSpec:
  summary: str

static functions: Registry[FunctionId, FunctionSpec] = Registry.define(
  subjects=[SubjectKind.Method],
)

trait Normalizer:
  @describe(functions, FunctionId("normalize"), FunctionSpec(summary="Normalize text"))
  def normalize(self, value: str) -> str:
    return value
"#,
        "expected unsupported trait registry description to fail",
    );
    assert!(
        errors.iter().any(|error| error
            .message
            .contains("@describe is currently supported only on concrete functions and methods, not a trait method")),
        "expected a source-target diagnostic for trait @describe, got {errors:?}"
    );
}

#[test]
fn registry_descriptions_expand_structural_const_values() -> Result<(), Box<dyn std::error::Error>> {
    let module_path = vec!["facts".to_string(), "const_registry".to_string()];
    let info = typecheck_info_for_module(
        r#"
from std.registry import Registry, SubjectKind, describe

type FunctionId = newtype str

@derive(Descriptor)
model FunctionSpec:
  forms: FrozenList[str]

const FORMS: FrozenList[str] = ["normalize(value)", "normalize_all(values)"]

pub static functions: Registry[FunctionId, FunctionSpec] = Registry.define(
  subjects=[SubjectKind.Function],
)

@describe(functions, FunctionId("normalize"), FunctionSpec(forms=FORMS))
def normalize(value: str) -> str:
  return value
"#,
        module_path,
        "registry const descriptor expansion",
    )?;

    let description = info
        .registry
        .descriptions
        .first()
        .ok_or("missing checked registry description")?;
    assert!(matches!(
        &description.descriptor,
        incan_semantics_core::SemanticRegistryValue::Model { fields, .. }
            if fields == &vec![(
                "forms".to_string(),
                incan_semantics_core::SemanticRegistryValue::List(vec![
                    incan_semantics_core::SemanticRegistryValue::String("normalize(value)".to_string()),
                    incan_semantics_core::SemanticRegistryValue::String("normalize_all(values)".to_string()),
                ]),
            )]
    ));
    Ok(())
}

#[test]
fn registry_descriptions_encode_some_as_a_structural_option() -> Result<(), Box<dyn std::error::Error>> {
    let info = typecheck_info_for_module(
        r#"
from std.registry import Registry, SubjectKind, describe

type FunctionId = newtype str

@derive(Descriptor)
model FunctionSpec:
  replacement: Option[str]

pub static functions: Registry[FunctionId, FunctionSpec] = Registry.define(
  subjects=[SubjectKind.Function],
)

@describe(functions, FunctionId("normalize"), FunctionSpec(replacement=Some("normalized")))
def normalize(value: str) -> str:
  return value
"#,
        vec!["facts".to_string(), "option_registry".to_string()],
        "registry option descriptor snapshot",
    )?;

    let description = info
        .registry
        .descriptions
        .first()
        .ok_or("missing checked registry description")?;
    assert!(matches!(
        &description.descriptor,
        incan_semantics_core::SemanticRegistryValue::Model { fields, .. }
            if fields == &vec![(
                "replacement".to_string(),
                incan_semantics_core::SemanticRegistryValue::Option(Box::new(
                    incan_semantics_core::SemanticRegistryValue::String("normalized".to_string())
                )),
            )]
    ));
    Ok(())
}

#[test]
fn registry_descriptions_encode_concrete_type_tokens() -> Result<(), Box<dyn std::error::Error>> {
    let info = typecheck_info_for_module(
        r#"
from std.registry import Registry, SubjectKind, describe

type FunctionId = newtype str

@derive(Descriptor)
model FunctionSpec:
  target: Type[int]

pub static functions: Registry[FunctionId, FunctionSpec] = Registry.define(
  subjects=[SubjectKind.Function],
)

@describe(functions, FunctionId("normalize"), FunctionSpec(target=int))
def normalize(value: str) -> str:
  return value
"#,
        vec!["facts".to_string(), "type_token_registry".to_string()],
        "registry type-token descriptor snapshot",
    )?;

    let description = info
        .registry
        .descriptions
        .first()
        .ok_or("missing checked registry description")?;
    assert!(matches!(
        &description.descriptor,
        incan_semantics_core::SemanticRegistryValue::Model { fields, .. }
            if fields == &vec![(
                "target".to_string(),
                incan_semantics_core::SemanticRegistryValue::Type("int".to_string()),
            )]
    ));
    Ok(())
}

#[test]
fn descriptor_derive_rejects_mutable_and_non_descriptor_field_shapes() {
    let errors = check_str_err(
        r#"
@derive(Descriptor)
model BadDescriptor:
  labels: list[str]
  callback: (str) -> str
"#,
        "expected descriptor shape diagnostics",
    );
    assert!(
        errors.iter().any(|error| error
            .message
            .contains("mutable collections are not descriptor snapshots")),
        "expected mutable collection diagnostic, got {errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("functions require runtime execution")),
        "expected function field diagnostic, got {errors:?}"
    );
}

#[test]
fn descriptor_derive_accepts_frozen_and_nested_structural_fields() {
    assert_check_ok(
        r#"
type DescriptorId = newtype str

enum Level:
  Info
  Error

@derive(Descriptor)
model Nested:
  label: str

@derive(Descriptor)
model Descriptor:
  id: DescriptorId
  level: Level
  nested: Nested
  tags: FrozenList[str]
  labels: FrozenDict[str, str]
  optional_label: Option[str]
"#,
    );
}

#[test]
fn descriptor_derive_rejects_nested_models_without_descriptor_contract() {
    let errors = check_str_err(
        r#"
model RuntimeState:
  label: str

@derive(Descriptor)
model BadDescriptor:
  state: RuntimeState
"#,
        "expected nested descriptor contract diagnostic",
    );
    assert!(
        errors.iter().any(|error| error
            .message
            .contains("nested models must also use @derive(Descriptor)")),
        "expected nested descriptor contract diagnostic, got {errors:?}"
    );
}

#[test]
fn loop_expression_infers_break_value_type() {
    assert_check_ok(
        r#"
def run() -> int:
  return loop:
    break 42
"#,
    );
}

#[test]
fn break_value_requires_loop_expression() {
    let errs = check_str_err(
        r#"
def run(xs: list[int]) -> None:
  for x in xs:
    break x
"#,
        "expected break-value diagnostic in for loop",
    );
    assert!(
        errs.iter()
            .any(|e| e.message.contains("only valid inside `loop:` expressions")),
        "expected loop-expression-only diagnostic, got {errs:?}"
    );
}

#[test]
fn loop_expression_without_break_is_rejected() {
    let errs = check_str_err(
        r#"
def run() -> int:
  return loop:
    pass
"#,
        "expected missing-break diagnostic for loop expression",
    );
    assert!(
        errs.iter()
            .any(|e| e.message.contains("loop expression must contain at least one `break`")),
        "expected missing-break diagnostic, got {errs:?}"
    );
}

#[test]
fn break_outside_loop_uses_typed_diagnostic() {
    let errs = check_str_err(
        r#"
def run() -> None:
  break
"#,
        "expected break-outside-loop diagnostic",
    );
    assert!(
        errs.iter()
            .any(|e| e.message.contains("`break` is only valid inside loops")),
        "expected break-outside-loop diagnostic, got {errs:?}"
    );
}
