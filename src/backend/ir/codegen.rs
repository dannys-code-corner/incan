Warning: truncated output (original token count: 52446)
Total output lines: 5086

//! IR-based code generation facade
//!
//! This module provides `IrCodegen`, a unified API for generating Rust code from Incan AST using the IR pipeline:
//!
//! ```text
//! AST → AstLowering → IR → IrEmitter (quote!) → prettyplease → RustSource
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use incan::backend::IrCodegen;
//!
//! // Fallible API (recommended):
//! let codegen = IrCodegen::new();
//! let rust_code = codegen.try_generate(&ast)?;
//!
//! // Convenience API (returns error comments on failure):
//! let mut codegen = IrCodegen::new();
//! let rust_code = codegen.generate(&ast);
//! ```
//!
//! ## Error Handling
//!
//! The `try_generate*` family of methods return `Result<_, GenerationError>`,
//! allowing callers to handle lowering and emission errors explicitly.
//! The `generate*` methods are convenience wrappers that return error comments
//! on failure (useful for debugging but not recommended for production).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::env;
#[cfg(feature = "rust_inspect")]
use std::path::PathBuf;
use std::sync::Arc;

use crate::frontend::ast::{Declaration, ImportKind, Program};
use crate::frontend::diagnostics::CompileError;
use crate::frontend::library_manifest_index::LibraryManifestIndex;
use crate::frontend::module::canonicalize_source_module_segments;
use crate::frontend::typechecker::TypeCheckInfo;
use crate::frontend::typechecker::stdlib_loader::StdlibAstCache;
use crate::library_manifest::LibraryManifest;
use crate::provider::{ProviderPlan, SDK_PROVIDER_BUILD_ENV};
use incan_core::lang::{rust_keywords, stdlib};

use super::emit::CallableNameResolution;
use super::scanners::{
    check_for_this_import as scan_check_for_this_import, collect_rust_crates as scan_collect_rust_crates,
    detect_serde_usage,
};
use super::{AstLowering, EmitError, EmitService, FunctionRegistry, IrEmitter, IrProgram, LoweringErrors};

mod capability_bridge;
mod dependency_metadata;
mod ordinal_bridge;
mod serde_activation;
mod string_try_from_bridge;

use dependency_metadata::{
    DependencySymbolMetadata, collect_dependency_symbol_metadata, collect_externally_reachable_items_by_module,
    collect_model_field_aliases, record_direct_generated_path_support_items_from_ir,
    should_preserve_dependency_public_items,
};
use ordinal_bridge::{OrdinalBridgeConfig, compilation_imports_std_ordinal_contract, imports_std_ordinal_contract};
use serde_activation::{add_serde_to_newtypes, collect_serde_derives};
use string_try_from_bridge::{
    StringTryFromBridgeConfig, compilation_imports_std_string_try_from_contract, imports_std_string_try_from_contract,
};

/// Error during Rust code generation.
///
/// This error type wraps all possible errors that can occur during code generation,
/// including AST lowering errors and IR emission errors.
///
/// ## Examples
///
/// ```rust,ignore
/// use incan::backend::{IrCodegen, GenerationError};
///
/// let codegen = IrCodegen::new();
/// match codegen.try_generate(&ast) {
///     Ok(code) => println!("{}", code),
///     Err(GenerationError::Lowering(errors)) => {
///         for err in errors.iter() {
///             eprintln!("Lowering error: {}", err);
///         }
///     }
///     Err(GenerationError::Emission(e)) => eprintln!("Emission failed: {}", e),
/// }
/// ```
#[derive(Debug)]
pub enum GenerationError {
    /// Errors during frontend typechecking.
    TypeCheck(Vec<CompileError>),
    /// Errors during AST to IR lowering (may contain multiple errors)
    Lowering(LoweringErrors),
    /// Error during IR to Rust emission
    Emission(EmitError),
}

impl std::fmt::Display for GenerationError {
    /// Format generation errors for CLI and integration-test diagnostics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GenerationError::TypeCheck(errs) => {
                if errs.is_empty() {
                    write!(f, "typecheck failed")
                } else {
                    // We intentionally avoid rich source formatting here (no file/source context at this layer), but
                    // include every message so generated-project stdlib failures are actionable.
                    let messages = errs
                        .iter()
                        .map(|err| err.message.as_str())
                        .collect::<Vec<_>>()
                        .join("; ");
                    write!(f, "typecheck failed ({} errors): {}", errs.len(), messages)
                }
            }
            GenerationError::Lowering(e) => write!(f, "{}", e),
            GenerationError::Emission(e) => write!(f, "emission error: {}", e),
        }
    }
}

impl std::error::Error for GenerationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GenerationError::TypeCheck(_) => None,
            GenerationError::Lowering(e) => Some(e),
            GenerationError::Emission(e) => Some(e),
        }
    }
}

impl From<LoweringErrors> for GenerationError {
    fn from(e: LoweringErrors) -> Self {
        GenerationError::Lowering(e)
    }
}

impl From<EmitError> for GenerationError {
    fn from(e: EmitError) -> Self {
        GenerationError::Emission(e)
    }
}

/// Options for one IR-to-Rust generation pass that needs cross-module identity side channels.
struct IrGenerationOptions<'a> {
    /// Shared anonymous union definitions keyed by stable union shape.
    generated_union_types: HashMap<String, super::types::IrType>,
    /// Whether anonymous union references should be emitted from the crate root.
    qualify_union_types_from_crate: bool,
    /// Shared callable-name resolutions collected while emitting multi-module generated code.
    callable_name_resolutions: Option<&'a mut HashMap<String, CallableNameResolution>>,
    /// Callable signature keys that require `__IncanCallableName` support.
    callable_name_used_signature_keys: Option<&'a mut HashSet<String>>,
    /// Collect callable signatures from this program when an imported module uses the generic callable-name trait.
    ///
    /// An imported generic helper can receive a function declared by the root program.  The helper's module owns the
    /// trait declaration, so it must receive the root program's concrete function-pointer signature even when the
    /// root program does not itself read `F.__name__`.
    collect_function_arg_signatures_for_imported_generic_callable_name_trait: bool,
    /// Dependency support items required by generated paths observed in lowered IR.
    direct_generated_path_support_items: Option<&'a mut HashMap<Vec<String>, HashSet<String>>>,
}

/// Lowered metadata-only modules whose generated Rust identity belongs to compiled SDK providers.
type CompiledSdkMetadataPrograms = Vec<(Vec<String>, IrProgram)>;

impl IrGenerationOptions<'_> {
    /// Build options for an ordinary single-program generation pass.
    fn ordinary() -> Self {
        Self {
            generated_union_types: HashMap::new(),
            qualify_union_types_from_crate: false,
            callable_name_resolutions: None,
            callable_name_used_signature_keys: None,
            collect_function_arg_signatures_for_imported_generic_callable_name_trait: false,
            direct_generated_path_support_items: None,
        }
    }
}

/// IR-based Rust code generator
///
/// This is the unified entrypoint for code generation. It uses the typed IR and syn/quote for code emission.
pub struct IrCodegen<'a> {
    /// The current program being generated
    current_program: Option<&'a Program>,
    /// Dependency modules to include before main.
    ///
    /// Stores both the flat module name (used for build graph identity) and the nested module path
    /// segments (used for correct Rust qualification in codegen).
    dependency_modules: Vec<(&'a str, &'a Program, Option<Vec<String>>)>,
    /// Source-derived dependency symbols used for Rust qualification but linked from an external artifact.
    ///
    /// The compiler typechecks provider imports against checked contracts. Once a module is supplied by a compiled
    /// provider, codegen must retain those contracts' canonical symbol paths without treating the module as a
    /// consumer-local Rust source module.
    dependency_symbol_modules: Vec<(&'a str, &'a Program, Option<Vec<String>>)>,
    /// Canonical nested paths learned while lowering emitted source dependencies for root metadata emission.
    source_dependency_module_paths: Vec<(&'a Program, Vec<String>)>,
    /// Whether serde is needed for emitted Rust derives or helpers.
    // Serde still affects emitted Rust imports and derive augmentation in IR emission, so this remains an
    // emission-internal signal even after project-level requirement collection moved to provider manifests.
    needs_serde: bool,
    /// Fixtures available for test functions (name -> (has_teardown, dependencies))
    fixtures: HashMap<String, (bool, Vec<String>)>,
    /// Rust crates imported via `import rust::` or `from rust::`
    rust_crates: HashSet<String>,
    /// Crate roots required to keep public class-field Rust identities nameable through a compiled provider.
    provider_rust_bridge_roots: BTreeSet<String>,
    /// Whether to emit the Zen of Incan at the start of main (set by `import this`)
    emit_zen_in_main: bool,
    /// Functions imported from external Rust crates (name -> true for external)
    external_rust_functions: HashSet<String>,
    /// Declared Rust crate names from `incan.toml [rust-dependencies]` (RFC 013 / RFC 023).
    ///
    /// When set, internal typechecking (used to obtain `TypeCheckInfo` for lowering) will validate `rust.module()`
    /// crate segments against this set.
    declared_crate_names: Option<HashSet<String>>,
    /// Shared provider and feature projection used by checking, lowering, and emission.
    provider_plan: Option<Arc<ProviderPlan>>,
    /// Whether generated Rust should deny warnings so tests can prove normal emission stays warning-clean.
    strict_generated_lints: bool,
    /// Private IR items called by generated code that is appended outside normal IR emission.
    externally_reachable_items: HashSet<String>,
    /// Private dependency-module IR items called by generated code appended inside that module.
    externally_reachable_items_by_module: HashMap<Vec<String>, HashSet<String>>,
    /// Public serialized value-enum identities for library builds, keyed by source identity (`module.Type`).
    public_ordinal_type_identities: HashMap<String, String>,
    /// Whether non-stdlib dependency modules keep public items that are not otherwise reachable.
    preserve_dependency_public_items: bool,
    /// Dependency module paths that should typecheck with source-visible public import rules.
    public_typecheck_module_paths: HashSet<Vec<String>>,
    /// Canonical defining package identity supplied by the command that owns the generated artifact.
    registry_package_identity: Option<String>,
    /// Canonical source-module path for the root program when its parsed AST lacks a source path.
    root_source_module_name: Option<String>,
    /// Shared stdlib source metadata cache reused across the repeated internal typecheck/lowering passes that codegen
    /// performs for multi-module builds.
    stdlib_cache: StdlibAstCache,
    /// Main-module facts supplied by the owning compilation session.
    ///
    /// Direct backend API callers may omit this temporarily; that fallback is removed when every caller constructs its
    /// lowering request from a compilation-session analysis (#225).
    prechecked_main_type_info: Option<TypeCheckInfo>,
    /// Dependency facts from the same session analysis, keyed by module identity.
    prechecked_dependency_type_info: HashMap<Vec<String>, TypeCheckInfo>,
    /// Manifest/workspace root for rust-inspect-backed typechecking during IR generation.
    #[cfg(feature = "rust_inspect")]
    rust_inspect_manifest_dir: Option<PathBuf>,
}

impl<'a> IrCodegen<'a> {
    /// Create a new IR-based code generator
    pub fn new() -> Self {
        Self {
            current_program: None,
            dependency_modules: Vec::new(),
            dependency_symbol_modules: Vec::new(),
            source_dependency_module_paths: Vec::new(),
            needs_serde: false,
            external_rust_functions: HashSet::new(),
            fixtures: HashMap::new(),
            rust_crates: HashSet::new(),
            provider_rust_bridge_roots: BTreeSet::new(),
            emit_zen_in_main: false,
            declared_crate_names: None,
            provider_plan: None,
            strict_generated_lints: false,
            externally_reachable_items: HashSet::new(),
            externally_reachable_items_by_module: HashMap::new(),
            public_ordinal_type_identities: HashMap::new(),
            preserve_dependency_public_items: true,
            public_typecheck_module_paths: HashSet::new(),
            registry_package_identity: None,
            root_source_module_name: None,
            stdlib_cache: StdlibAstCache::new(),
            prechecked_main_type_info: None,
            prechecked_dependency_type_info: HashMap::new(),
            #[cfg(feature = "rust_inspect")]
            rust_inspect_manifest_dir: None,
        }
    }

    /// Return the stable module key used by source imports and CLI collection for one dependency module.
    fn dependency_module_key(name: &str, path_segments: &Option<Vec<String>>) -> String {
        path_segments
            .as_deref()
            .map(canonicalize_source_module_segments)
            .map(|segments| segments.join("_"))
            .unwrap_or_else(|| name.to_string())
    }

    /// Return the transitive local source dependency subset needed to typecheck one program.
    ///
    /// Codegen typechecking must mirror the CLI checker: a module should see its declared local imports and their
    /// transitive signature dependencies, not every module collected for the output project. Importing the whole
    /// dependency universe lets same-name public helpers from unrelated modules collide before `from ... import ... as
    /// ...` collection, which changes behavior between `--check` and `--emit-rust`.
    fn imported_dependency_modules_for_program(
        program: &Program,
        dependencies: &[(&'a str, &'a Program, Option<Vec<String>>)],
        self_key: Option<&str>,
    ) -> Vec<(&'a str, &'a Program)> {
        let mut module_idx_by_key = HashMap::new();
        for (idx, (name, _, path_segments)) in dependencies.iter().enumerate() {
            module_idx_by_key.insert(Self::dependency_module_key(name, path_segments), idx);
        }

        let mut selected = BTreeSet::new();
        let mut pending = Self::direct_imported_dependency_indexes(program, &module_idx_by_key, self_key);
        while let Some(idx) = pending.pop() {
            let (name, ast, path_segments) = &dependencies[idx];
            let dep_key = Self::dependency_module_key(name, path_segments);
            if self_key == Some(dep_key.as_str()) || !selected.insert(idx) {
                continue;
            }
            pending.extend(Self::direct_imported_dependency_indexes(
                ast,
                &module_idx_by_key,
                Some(dep_key.as_str()),
            ));
        }

        selected
            .into_iter()
            .map(|idx| {
                let (name, ast, _) = dependencies[idx];
                (name, ast)
            })
            .collect()
    }

    /// Return direct dependency-module indexes named by source imports in one program.
    fn direct_imported_dependency_indexes(
        program: &Program,
        module_idx_by_key: &HashMap<String, usize>,
        self_key: Option<&str>,
    ) -> Vec<usize> {
        let mut dep_indexes = BTreeSet::new();
        for decl in &program.declarations {
            let Declaration::Import(import) = &decl.node else {
                continue;
            };
            match &import.kind {
                ImportKind::From { module, .. } => {
                    if module.parent_levels > 0 || module.segments.is_empty() {
                        continue;
                    }
                    let key = canonicalize_source_module_segments(&module.segments).join("_");
                    if self_key != Some(key.as_str())
                        && let Some(dep_idx) = module_idx_by_key.get(&key).copied()
                    {
                        dep_indexes.insert(dep_idx);
                    }
                }
                ImportKind::Module(path) => {
                    if path.parent_levels > 0 || path.segments.is_empty() {
                        continue;
                    }
                    let full_key = canonicalize_source_module_segments(&path.segments).join("_");
                    if self_key != Some(full_key.as_str())
                        && let Some(dep_idx) = module_idx_by_key.get(&full_key).copied()
                    {
                        dep_indexes.insert(dep_idx);
                    }
                    if path.segments.len() > 1 {
                        let parent_key =
                            canonicalize_source_module_segments(&path.segments[..path.segments.len() - 1]).join("_");
                        if self_key != Some(parent_key.as_str())
                            && let Some(dep_idx) = module_idx_by_key.get(&parent_key).copied()
                        {
                            dep_indexes.insert(dep_idx);
                        }
                    }
                }
                _ => {}
            }
        }
        dep_indexes.into_iter().collect()
    }

    /// Build a registry for explicit canonical cross-module calls.
    fn canonical_registry_for_programs<'program>(
        programs: impl IntoIterator<Item = (&'program [String], &'program IrProgram)>,
    ) -> FunctionRegistry {
        let programs: Vec<_> = programs.into_iter().collect();
        let mut registry = FunctionRegistry::new();
        for (module_path, program) in &programs {
            for (name, signature) in program.function_registry.iter() {
                let mut canonical_path = (*module_path).to_vec();
                canonical_path.push(name.clone());
                registry.register_canonical_path(
                    &canonical_path,
                    signature.params.clone(),
                    signature.return_type.clone(),
                );
            }
        }

        let mut pending_reexports = Vec::new();
        for (module_path, program) in &programs {
            for reexport in &program.function_reexports {
                let mut alias_path = (*module_path).to_vec();
                alias_path.push(reexport.name.clone());
                pending_reexports.push((alias_path, reexport.target_path.clone()));
            }
        }
        while !pending_reexports.is_empty() {
            let mut unresolved = Vec::new();
            let mut made_progress = false;
            for (alias_path, target_path) in pending_reexports {
                if registry.get_canonical_path(&alias_path).is_some() {
                    made_progress = true;
                    continue;
                }
                if let Some(signature) = registry.get_canonical_path(&target_path).cloned() {
                    registry.register_canonical_path(
                        &alias_path,
                        signature.params.clone(),
                        signature.return_type.clone(),
                    );
                    made_progress = true;
                } else {
                    unresolved.push((alias_path, target_path));
                }
            }
            if !made_progress {
                break;
            }
            pending_reexports = unresolved;
        }
        registry
    }

    /// Apply dependency symbol metadata to generated Rust codegen state.
    fn apply_dependency_symbol_metadata(
        emitter: &mut IrEmitter<'_>,
        metadata: &DependencySymbolMetadata,
        provider_plan: Option<&ProviderPlan>,
    ) {
        let stdlib_module_paths = provider_plan
            .map(ProviderPlan::active_std_module_paths)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|path| {
                path.strip_prefix(&[stdlib::STDLIB_ROOT.to_string()])
                    .map(<[String]>::to_vec)
            })
            .collect();
        emitter.set_compiled_sdk_module_paths(stdlib_module_paths);
        emitter.set_type_module_paths(metadata.module_paths.clone(), metadata.ambiguous_type_names.clone());
        emitter.set_value_module_paths(
            metadata.value_module_paths.clone(),
            metadata.ambiguous_value_names.clone(),
        );
        let mut enum_type_names = metadata.enum_type_names.clone();
        if let Some(plan) = provider_plan {
            for provider in plan.active_sdk_records() {
                let Some(manifest) = provider.manifest.as_deref() else {
                    continue;
                };
                enum_type_names.extend(manifest.exports.enums.iter().map(|enum_| enum_.name.clone()));
                enum_type_names.extend(
                    manifest
                        .contract_metadata
                        .api
                        .iter()
                        .flat_map(|api| api.modules.iter())
                        .flat_map(|module| module.declarations.iter())
                        .filter_map(|declaration| match declaration {
                            crate::frontend::api_metadata::ApiDeclaration::Enum(enum_) => Some(enum_.name.clone()),
                            _ => None,
                        }),
                );
            }
        }
        emitter.set_dependency_enum_types(enum_type_names);
        if let Some(plan) = provider_plan {
            emitter.seed_public_dependency_nominal_metadata(plan.library_manifest_index());
            for provider in plan.active_sdk_records() {
                if let Some(manifest) = provider.manifest.as_deref() {
                    emitter.seed_sdk_provider_manifest_metadata(manifest);
                }
            }
        }
    }

    /// Configure source-import emission with the checked module graph for this generated crate.
    fn configure_source_import_paths(
        emitter: &mut IrEmitter<'_>,
        current_module: Option<&str>,
        source_module_paths: &HashSet<Vec<String>>,
    ) {
        emitter.set_source_module_paths(source_module_paths.clone());
        emitter.set_current_source_module_path(
            current_module.map(|module| module.split('.').map(str::to_string).collect()),
        );
    }

    /// Enable strict generated Rust lint validation for `--emit-rust --strict`.
    pub fn set_strict_generated_lints(&mut self, enabled: bool) {
        self.strict_generated_lints = enabled;
    }

    /// Set private generated Rust entrypoints called by code injected after IR emission.
    pub fn set_externally_reachable_items(&mut self, names: HashSet<String>) {
        self.externally_reachable_items = names;
    }

    /// Set private generated Rust entrypoints called by code injected into dependency modules.
    pub fn set_externally_reachable_items_by_module(&mut self, names: HashMap<Vec<String>, HashSet<String>>) {
        self.externally_reachable_items_by_module = names;
    }

    /// Set public serialized value-enum identities for library emission.
    pub fn set_public_ordinal_type_identities(&mut self, identities: HashMap<String, String>) {
        self.public_ordinal_type_identities = identities;
    }

    /// Collect the OrdinalKey bridge facts needed by the emitter for this program.
    fn ordinal_bridge_config(&self, uses_std_ordinal_contract: bool) -> OrdinalBridgeConfig {
        OrdinalBridgeConfig::for_crate_root(
            uses_std_ordinal_contract,
            self.provider_plan.as_deref().map(ProviderPlan::library_manifest_index),
        )
    }

    /// Collect `TryFrom[str]` bridge facts needed at the generated crate root.
    fn string_try_from_bridge_config(&self, uses_contract: bool) -> StringTryFromBridgeConfig {
        StringTryFromBridgeConfig::for_crate_root(uses_contract)
    }

    /// Apply collected OrdinalKey bridge metadata to a freshly created emitter.
    fn apply_ordinal_bridge_config(&self, emitter: &mut IrEmitter, config: &OrdinalBridgeConfig) {
        emitter.set_emit_std_ordinal_value_enum_impls(config.emit_std_ordinal_value_enum_impls);
        emitter.set_external_ordinal_value_enums(config.external_value_enums.clone());
        emitter.set_external_ordinal_custom_keys(config.external_custom_keys.clone());
        emitter.set_public_ordinal_type_identities(self.public_ordinal_type_identities.clone());
    }

    /// Apply compiler-provided `TryFrom[str]` bridge metadata to a freshly created emitter.
    fn apply_string_try_from_bridge_config(&self, emitter: &mut IrEmitter, config: &StringTryFromBridgeConfig) {
        emitter.set_emit_std_string_try_from_newtype_impls(config.emit_local_newtype_impls);
    }

    /// Apply every temporary source-owned capability bridge to a freshly created emitter.
    fn apply_capability_bridge_configs(
        &self,
        emitter: &mut IrEmitter,
        ordinal: &OrdinalBridgeConfig,
        string_conversion: &StringTryFromBridgeConfig,
    ) {
        self.apply_ordinal_bridge_config(emitter, ordinal);
        self.apply_string_try_from_bridge_config(emitter, string_conversion);
    }

    /// Set whether non-stdlib dependency modules preserve their public API surface during emission.
    ///
    /// Library builds keep this enabled so public dependency declarations remain available at the Rust crate boundary.
    /// Binary and test harness builds can disable it so unused dependency declarations are pruned instead of warning.
    pub fn set_preserve_dependency_public_items(&mut self, enabled: bool) {
        self.preserve_dependency_public_items = enabled;
    }

    /// Set the package identity used when materializing explicit package-level registry subjects.
    pub fn set_registry_package_identity(&mut self, identity: Option<String>) {
        self.registry_package_identity = identity;
    }

    /// Set the root compilation-unit identity when parsing did not retain a source path.
    pub fn set_root_source_module_name(&mut self, name: Option<String>) {
        self.root_source_module_name = name;
    }

    /// Set dependency module paths that should typecheck with public source import rules.
    ///
    /// CLI test batches can emit individual test files as generated dependency modules so each file keeps its own Rust
    /// module scope. Those test files are still user source and must typecheck like focused `incan test file.incn`
    /// runs, not like compiler-internal source dependencies that may inspect private module items.
    pub fn set_public_typecheck_module_paths(&mut self, paths: HashSet<Vec<String>>) {
        self.public_typecheck_module_paths = paths;
    }

    /// Seed codegen with stdlib metadata already collected by an earlier typecheck phase.
    pub(crate) fn set_stdlib_cache(&mut self, cache: StdlibAstCache) {
        self.stdlib_cache = cache;
    }

    /// Supply the checked lowering inputs owned by one compilation session.
    ///
    /// Production command paths use this to prevent lowering from rechecking source after diagnostics and semantic
    /// facts have already been produced.
    pub(crate) fn set_prechecked_type_info(
        &mut self,
        main: TypeCheckInfo,
        dependencies: HashMap<Vec<String>, TypeCheckInfo>,
    ) {
        self.prechecked_main_type_info = Some(main);
        self.prechecked_dependency_type_info = dependencies;
    }

    /// Return session-owned facts for one dependency module when supplied.
    fn prechecked_dependency_type_info(&self, path: &[String]) -> Option<TypeCheckInfo> {
        self.prechecked_dependency_type_info.get(path).cloned()
    }

    /// Set declared Rust crate names from `incan.toml [rust-dependencies]`. (RFC 031)
    ///
    /// This is used for validating `rust.module()` paths during the internal typechecking that precedes IR lowering.
    pub fn set_declared_crate_names(&mut self, names: HashSet<String>) {
        self.declared_crate_names = Some(names);
    }

    /// Set the consumer-side library manifest index for focused `pub::` tests and embedding adapters.
    pub fn set_library_manifest_index(&mut self, index: LibraryManifestIndex) {
        self.provider_plan = Some(Arc::new(ProviderPlan::for_library_index(index)));
    }

    /// Set one in-memory SDK provider manifest for focused compiler tests.
    #[doc(hidden)]
    pub fn set_sdk_provider_manifest(&mut self, manifest: LibraryManifest) {
        let library_index = self
            .provider_plan
            .as_deref()
            .map(ProviderPlan::library_manifest_index)
            .cloned()
            .unwrap_or_default();
        self.provider_plan = Some(Arc::new(ProviderPlan::for_in_memory_sdk_manifest(
            library_index,
            manifest,
        )));
    }

    /// Set SDK-provider module paths already derived from a producer entrypoint or checked manifest.
    ///
    /// Compiler frontends should normally call [`Self::set_sdk_provider_manifest`]. This lower-level hook supports
    /// source-backed codegen fixtures and embedders that already own equivalent checked module discovery.
    #[doc(hidden)]
    pub fn set_sdk_provider_module_paths(&mut self, module_paths: Vec<Vec<String>>) {
        let library_index = self
            .provider_plan
            .as_deref()
            .map(ProviderPlan::library_manifest_index)
            .cloned()
            .unwrap_or_default();
        self.provider_plan = Some(Arc::new(ProviderPlan::for_in_memory_sdk_modules(
            library_index,
            module_paths,
        )));
    }

    /// Set the immutable provider plan shared across every compiler stage.
    pub fn set_provider_plan(&mut self, plan: Arc<ProviderPlan>) {
        self.provider_plan = Some(plan);
    }

    /// Set the manifest/workspace root used for rust-inspect-backed typechecking during IR generation.
    #[cfg(feature = "rust_inspect")]
    pub fn set_rust_inspect_manifest_dir(&mut self, dir: PathBuf) {
        self.rust_inspect_manifest_dir = Some(dir);
    }

    /// Get the Rust crates imported via `import rust::` or `from rust::`
    pub fn rust_crates(&self) -> &HashSet<String> {
        &self.rust_crates
    }

    /// Register a fixture for test code generation
    pub fn add_fixture(&mut self, name: &str, has_teardown: bool, dependencies: Vec<String>) {
        self.fixtures.insert(name.to_string(), (has_teardown, dependencies));
    }

    /// Check if serde is needed.
    #[cfg(test)]
    fn needs_serde(&self) -> bool {
        self.needs_serde
    }

    /// Apply codegen's shared project context to an internal typechecker pass.
    fn configure_typechecker(&self, tc: &mut crate::frontend::typechecker::TypeChecker) {
        tc.stdlib_cache = self.stdlib_cache.clone();
        if let Some(names) = self.declared_crate_names.clone() {
            tc.set_declared_crate_names(names);
        }
        if let Some(plan) = self.provider_plan.clone() {
            tc.set_provider_plan(plan);
        }
        #[cfg(feature = "rust_inspect")]
        if let Some(dir) = self.rust_inspect_manifest_dir.clone() {
            tc.set_rust_inspect_manifest_dir(dir);
        }
    }

    /// Prefix internal codegen typecheck diagnostics with the module being lowered.
    fn typecheck_errors_for_module(module: &str, mut errors: Vec<CompileError>) -> GenerationError {
        for error in &mut errors {
            error.message = format!("in module `{module}`: {}", error.message);
        }
        GenerationError::TypeCheck(errors)
    }

    /// Preserve stdlib metadata warmed by an internal typechecker pass for later codegen passes.
    fn capture_typechecker_stdlib_cache(&mut self, tc: &crate::frontend::typechecker::TypeChecker) {
        self.stdlib_cache = tc.stdlib_cache.clone();
    }

    /// Apply codegen's shared metadata context to one AST lowering pass.
    fn configure_lowering(&self, lowering: &mut AstLowering) {
        lowering.set_stdlib_cache(self.stdlib_cache.clone());
        lowering.set_provider_plan(self.provider_plan.clone());
        lowering.set_sdk_provider_build(env::var_os(SDK_PROVIDER_BUILD_ENV).is_some());
        lowering.set_registry_package_identity(self.registry_package_identity.clone());
    }

    /// Add a dependency module (for multi-file compilation)
    pub fn add_module(&mut self, module_name: &'a str, module_ast: &'a Program) {
        self.dependency_modules.push((module_name, module_ast, None));
    }

    /// Add a dependency module with its nested module path segments.
    ///
    /// This is used by the CLI multi-file nested mode where a module like `api.routes` is emitted as
    /// `crate::api::routes` in Rust (even though we may use a flattened name like `api_routes` for internal identity).
    pub fn add_module_with_path_segments(
        &mut self,
        module_name: &'a str,
        module_ast: &'a Program,
        path_segments: Vec<String>,
    ) {
        self.dependency_modules
            .push((module_name, module_ast, Some(path_segments)));
    }

    /// Add dependency source metadata without scheduling that module for local Rust emission.
    ///
    /// This remains available for non-emitted source dependencies. Compiled SDK-provider imports instead derive
    /// their semantics from the compiled artifact manifest and resolve Rust symbols through the linked artifact crate.
    pub fn add_dependency_symbol_module_with_path_segments(
        &mut self,
        module_name: &'a str,
        module_ast: &'a Program,
        path_segments: Vec<String>,
    ) {
        self.dependency_symbol_modules
            .push((module_name, module_ast, Some(path_segments)));
    }

    /// Return emitted and metadata-only dependencies, deduplicated by canonical source module identity.
    fn dependency_modules_for_symbol_metadata(&self) -> Vec<(&'a str, &'a Program, Option<Vec<String>>)> {
        let mut modules = self.dependency_modules.clone();
        for module in &self.dependency_symbol_modules {
            let key = Self::dependency_module_key(module.0, &module.2);
            if !modules
                .iter()
                .any(|candidate| Self::dependency_module_key(candidate.0, &candidate.2) == key)
            {
                modules.push(module.clone());
            }
        }
        modules
    }

    /// Lower metadata-only stdlib modules enough to discover anonymous union wrappers owned by the artifact crate.
    ///
    /// Anonymous unions have stable structural names but no source-level name to place in the `.incnlib` contract yet.
    /// Until that manifest capability exists, this source-derived registry preserves one Rust nominal identity without
    /// re-emitting the provider modules in every consumer.
    fn compiled_sdk_metadata_programs(&mut self) -> Result<CompiledSdkMetadataPrograms, GenerationError> {
        if let Some(plan) = self.provider_plan.as_deref() {
            let mut has_compiled_provider = false;
            for provider in plan.active_sdk_records() {
                let Some(_manifest) = provider.manifest.as_deref() else {
                    continue;
                };
                has_compiled_provider = true;
            }
            if has_compiled_provider {
                return Ok(Vec::new());
            }
        }
        if self.dependency_symbol_modules.is_empty() {
            return Ok(Vec::new());
        }

        let dependencies = self.dependency_modules_for_symbol_metadata();
        let symbol_modules = self.dependency_symbol_modules.clone();
        let mut programs = Vec::new();
        for (module_name, module_ast, path_segments) in symbol_modules {
            let Some(path_segments) = path_segments.as_ref() else {
                continue;
            };
            if path_segments.first().map(String::as_str) != Some(stdlib::INCAN_STD_NAMESPACE) {
                continue;
            }
            let module_key = Self::dependency_module_key(module_name, &Some(path_segments.clone()));
            let module_type_info = {
                use crate::frontend::typechecker::TypeChecker;
                let mut tc = TypeChecker::new();
                self.configure_typechecker(&mut tc);
                let typecheck_deps =
                    Self::imported_dependency_modules_for_program(module_ast, &dependencies, Some(&module_key));
                let result = match tc.check_with_imports_allow_private(module_ast, &typecheck_deps) {
                    Ok(()) => tc.type_info().clone(),
                    Err(errs) => return Err(Self::typecheck_errors_for_module(&module_key, errs)),
                };
                self.capture_typechecker_stdlib_cache(&tc);
                result
            };
            self.collect_provider_rust_bridge_roots(&module_type_info)?;
            let mut lowering = AstLowering::new_with_type_info(module_type_info);
            self.configure_lowering(&mut lowering);
            lowering.set_current_source_module_name(Some(path_segments.join(".")));
            lowering.seed_dependency_trait_decls(&dependencies);
            let ir = lowering.lower_program(module_ast)?;
            programs.push((path_segments.clone(), ir));
        }
        Ok(programs)
    }

    /// Backfill nested module path segments for a dependency module by name.
    ///
    /// This is primarily used by tests or older call sites that only registered a flat
    /// module name via `add_module()`. If a matching module entry exists and has no
    /// path segments yet, this sets them.
    pub fn set_module_path_segments(&mut self, module_name: &str, path_segments: Vec<String>) {
        if let Some((_name, _ast, segs)) = self
            .dependency_modules
            .iter_mut()
            .find(|(name, _, _)| *name == module_name)
            && segs.is_none()
        {
            *segs = Some(path_segments);
        }
    }

    // =========================================================================
    // Feature Detection
    // =========================================================================

    /// Scan a program for external Rust function imports
    fn collect_external_rust_functions(&mut self, program: &Program) {
        use crate::frontend::ast::{Declaration, ImportKind};

        for decl in &program.declarations {
            if let Declaration::Import(import) = &decl.node {
                match &import.kind {
                    // from rust::crate import items
                    ImportKind::RustFrom { items, .. } => {
                        for item in items {
                            let func_name = item.alias.as_ref().unwrap_or(&item.name);
                            self.external_rust_functions.insert(func_name.clone());
                        }
                    }
                    // Legacy: from rust::crate import items (parsed as From with rust:: module)
                    ImportKind::From { module, items }
                        if !module.segments.is_empty() && module.segments.first() == Some(&"rust".to_string()) =>
                    {
                        for item in items {
                            let func_name = item.alias.as_ref().unwrap_or(&item.name);
                            self.external_rust_functions.insert(func_name.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Scan a program for serde-backed derives.
    ///
    /// This remains an internal compatibility hook because serde-backed derives a…32446 tokens truncated…                      ],
                        variants: Vec::new(),
                    }),
                },
            )
            .map_err(|e| std::io::Error::other(format!("seed rust-inspect type: {e}")))?;
        tc.check_program(&ast)
            .map_err(|errs| std::io::Error::other(format!("typecheck failed: {errs:?}")))?;

        let mut lowering = AstLowering::new_with_type_info(tc.type_info().clone());
        let ir_program = lowering
            .lower_program(&ast)
            .map_err(|err| std::io::Error::other(format!("lowering failed: {err:?}")))?;
        let mut emitter = IrEmitter::new(&ir_program.function_registry);
        let code = emitter
            .emit_program(&ir_program)
            .map_err(|err| std::io::Error::other(format!("emit failed: {err:?}")))?;

        assert!(
            code.contains("Pair {") && code.contains("zeta: 1") && code.contains("alpha: 2"),
            "expected named-field Rust struct literal in generated code; got:\n{code}"
        );
        assert!(
            !code.contains("Pair(1, 2)"),
            "imported named-field Rust structs must not emit tuple-style constructors; got:\n{code}"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn test_codegen_emits_raw_rust_field_names_for_keyword_fields_issue725() -> Result<(), Box<dyn std::error::Error>> {
        use crate::frontend::typechecker::TypeChecker;
        use incan_core::interop::{
            RustFieldInfo, RustItemKind, RustItemMetadata, RustTypeInfo, RustTypeShape, RustVisibility,
        };

        let source = r#"
from rust::demo import JoinRel

pub def get_type(join: JoinRel) -> int:
  return join.type + join.match + join.type_

pub def rebuild(join: JoinRel) -> JoinRel:
  return JoinRel(type=join.type, match=join.match, type_=join.type_)
"#;
        let tokens = must_ok(lexer::lex(source));
        let ast = must_ok(parser::parse(&tokens));

        let tmp = seeded_rust_inspect_workspace()?;
        let manifest_dir = tmp.path().to_path_buf();
        let mut tc = TypeChecker::new();
        tc.set_rust_inspect_manifest_dir(manifest_dir.clone());
        tc.rust_inspect_cache
            .insert_test_item(
                &manifest_dir,
                RustItemMetadata {
                    canonical_path: "demo::JoinRel".to_string(),
                    definition_path: Some("demo::JoinRel".to_string()),
                    visibility: RustVisibility::Public,
                    kind: RustItemKind::Type(RustTypeInfo {
                        type_params: Vec::new(),
                        has_const_params: false,
                        alias_target: None,
                        metadata_completeness: Default::default(),
                        methods: Vec::new(),
                        implemented_traits: Vec::new(),
                        fields: vec![
                            RustFieldInfo {
                                name: "type".to_string(),
                                type_display: "i64".to_string(),
                                type_shape: RustTypeShape::Int,
                            },
                            RustFieldInfo {
                                name: "match".to_string(),
                                type_display: "i64".to_string(),
                                type_shape: RustTypeShape::Int,
                            },
                            RustFieldInfo {
                                name: "type_".to_string(),
                                type_display: "i64".to_string(),
                                type_shape: RustTypeShape::Int,
                            },
                        ],
                        variants: Vec::new(),
                    }),
                },
            )
            .map_err(|e| std::io::Error::other(format!("seed rust-inspect type: {e}")))?;
        tc.check_program(&ast)
            .map_err(|errs| std::io::Error::other(format!("typecheck failed: {errs:?}")))?;

        let mut lowering = AstLowering::new_with_type_info(tc.type_info().clone());
        let ir_program = lowering
            .lower_program(&ast)
            .map_err(|err| std::io::Error::other(format!("lowering failed: {err:?}")))?;
        let mut emitter = IrEmitter::new(&ir_program.function_registry);
        let code = emitter
            .emit_program(&ir_program)
            .map_err(|err| std::io::Error::other(format!("emit failed: {err:?}")))?;

        assert!(
            code.contains("join.r#type")
                && code.contains("join.r#match")
                && code.contains("join.type_")
                && code.contains("r#type: join.r#type")
                && code.contains("r#match: join.r#match")
                && code.contains("type_: join.type_"),
            "expected keyword fields to emit raw Rust identifiers while ordinary trailing-underscore fields stay unchanged; got:\n{code}"
        );
        assert!(
            !code.contains("r#type: join.type_") && !code.contains("type_: join.r#type"),
            "Rust keyword fields and ordinary trailing-underscore fields must not be cross-wired; got:\n{code}"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn test_codegen_uses_source_field_names_for_metadata_free_rust_type_constructor()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::frontend::typechecker::TypeChecker;

        let source = r#"
from rust::demo import Pair

pub def make_pair() -> Pair:
  return Pair(zeta=1, alpha=2)
"#;
        let tokens = must_ok(lexer::lex(source));
        let ast = must_ok(parser::parse(&tokens));

        let mut tc = TypeChecker::new();
        tc.check_program(&ast)
            .map_err(|errs| std::io::Error::other(format!("typecheck failed: {errs:?}")))?;

        let mut lowering = AstLowering::new_with_type_info(tc.type_info().clone());
        let ir_program = lowering
            .lower_program(&ast)
            .map_err(|err| std::io::Error::other(format!("lowering failed: {err:?}")))?;
        let mut emitter = IrEmitter::new(&ir_program.function_registry);
        let code = emitter
            .emit_program(&ir_program)
            .map_err(|err| std::io::Error::other(format!("emit failed: {err:?}")))?;

        assert!(
            code.contains("Pair {") && code.contains("zeta: 1") && code.contains("alpha: 2"),
            "expected source-named Rust struct literal in generated code; got:\n{code}"
        );
        assert!(
            !code.contains("Pair(zeta = 1, alpha = 2)") && !code.contains("Pair(1, 2)"),
            "metadata-free named Rust constructors must not emit call syntax; got:\n{code}"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn test_codegen_borrows_rust_backed_method_args_from_metadata() -> Result<(), Box<dyn std::error::Error>> {
        use crate::frontend::typechecker::TypeChecker;
        use incan_core::interop::{
            RustFunctionSig, RustItemKind, RustItemMetadata, RustMethodSig, RustParam, RustTypeInfo, RustVisibility,
        };

        let source = r#"
from rust::demo import Builder

model Payload:
  name: str

pub def forward(payload: Payload) -> int:
  builder = Builder.new()
  return builder.json(payload)
"#;
        let tokens = must_ok(lexer::lex(source));
        let ast = must_ok(parser::parse(&tokens));

        let tmp = seeded_rust_inspect_workspace()?;
        let manifest_dir = tmp.path().to_path_buf();
        let mut tc = TypeChecker::new();
        tc.set_rust_inspect_manifest_dir(manifest_dir.clone());
        tc.rust_inspect_cache
            .insert_test_item(
                &manifest_dir,
                RustItemMetadata {
                    canonical_path: "demo::Builder".to_string(),
                    definition_path: Some("demo::Builder".to_string()),
                    visibility: RustVisibility::Public,
                    kind: RustItemKind::Type(RustTypeInfo {
                        type_params: Vec::new(),
                        has_const_params: false,
                        alias_target: None,
                        metadata_completeness: Default::default(),
                        methods: vec![
                            RustMethodSig {
                                name: "new".to_string(),
                                signature: RustFunctionSig {
                                    type_params: Vec::new(),
                                    params: Vec::new(),
                                    return_type: "demo::Builder".to_string(),
                                    is_async: false,
                                    is_unsafe: false,
                                },
                            },
                            RustMethodSig {
                                name: "json".to_string(),
                                signature: RustFunctionSig {
                                    type_params: Vec::new(),
                                    params: vec![RustParam {
                                        name: Some("value".to_string()),
                                        type_display: "&T".to_string(),
                                    }],
                                    return_type: "i64".to_string(),
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
            )
            .map_err(|e| std::io::Error::other(format!("seed rust-inspect type: {e}")))?;
        tc.check_program(&ast)
            .map_err(|errs| std::io::Error::other(format!("typecheck failed: {errs:?}")))?;

        let mut lowering = AstLowering::new_with_type_info(tc.type_info().clone());
        let ir_program = lowering
            .lower_program(&ast)
            .map_err(|err| std::io::Error::other(format!("lowering failed: {err:?}")))?;

        let mut codegen = IrCodegen::new();
        codegen.collect_external_rust_functions(&ast);

        let mut emitter = IrEmitter::new(&ir_program.function_registry);
        emitter.set_external_rust_functions(codegen.external_rust_functions.clone());
        let code = emitter
            .emit_program(&ir_program)
            .map_err(|err| std::io::Error::other(format!("emit failed: {err:?}")))?;

        assert!(
            code.contains("builder.json(&payload);"),
            "expected borrowed rust method arg in generated code; got:\n{code}"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn test_codegen_borrows_reqwest_json_payload_returned_from_registry_client()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::frontend::typechecker::TypeChecker;

        let source = r#"
from rust::reqwest import Client

model Payload:
  name: str

pub def forward(payload: Payload) -> None:
  builder = Client.new().post("https://example.invalid")
  _ = builder.json(payload)
"#;
        let tokens = must_ok(lexer::lex(source));
        let ast = must_ok(parser::parse(&tokens));

        let tmp = reqwest_shaped_rust_inspect_workspace()?;
        let manifest_dir = tmp.path().to_path_buf();
        prewarm_metadata(&manifest_dir, &["reqwest::Client"])?;

        let mut tc = TypeChecker::new();
        tc.set_rust_inspect_manifest_dir(manifest_dir);
        tc.check_program(&ast)
            .map_err(|errs| std::io::Error::other(format!("typecheck failed: {errs:?}")))?;

        let mut lowering = AstLowering::new_with_type_info(tc.type_info().clone());
        let ir_program = lowering
            .lower_program(&ast)
            .map_err(|err| std::io::Error::other(format!("lowering failed: {err:?}")))?;

        let mut codegen = IrCodegen::new();
        codegen.collect_external_rust_functions(&ast);

        let mut emitter = IrEmitter::new(&ir_program.function_registry);
        emitter.set_external_rust_functions(codegen.external_rust_functions.clone());
        let code = emitter
            .emit_program(&ir_program)
            .map_err(|err| std::io::Error::other(format!("emit failed: {err:?}")))?;

        assert!(
            code.contains("builder.json(&payload);"),
            "expected registry-returned reqwest RequestBuilder::json payload to be borrowed; got:\n{code}"
        );
        assert!(
            code.contains(r#"Client::new().post("https://example.invalid")"#),
            "expected generic reqwest Client::post string literal to keep inferable &str shape; got:\n{code}"
        );
        assert!(
            !code.contains(r#".post("https://example.invalid".into())"#),
            "generic reqwest Client::post must not force ambiguous `.into()` on string literals; got:\n{code}"
        );
        Ok(())
    }

    #[test]
    fn test_codegen_keeps_nested_rust_associated_calls_type_like_when_outer_receiver_is_unknown()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::frontend::typechecker::TypeChecker;

        let source = r#"
from rust::datafusion::execution::context import SessionContext
from rust::datafusion::dataframe import DataFrameWriteOptions

pub def f(uri: str) -> None:
  ctx = SessionContext.new()
  _ = ctx.write_csv(uri, DataFrameWriteOptions.new(), None)
"#;
        let tokens = must_ok(lexer::lex(source));
        let ast = must_ok(parser::parse(&tokens));

        let mut tc = TypeChecker::new();
        tc.check_program(&ast)
            .map_err(|errs| std::io::Error::other(format!("typecheck failed: {errs:?}")))?;

        let mut lowering = AstLowering::new_with_type_info(tc.type_info().clone());
        let ir_program = lowering
            .lower_program(&ast)
            .map_err(|err| std::io::Error::other(format!("lowering failed: {err:?}")))?;

        let mut codegen = IrCodegen::new();
        codegen.collect_external_rust_functions(&ast);

        let mut emitter = IrEmitter::new(&ir_program.function_registry);
        emitter.set_external_rust_functions(codegen.external_rust_functions.clone());
        let code = emitter
            .emit_program(&ir_program)
            .map_err(|err| std::io::Error::other(format!("emit failed: {err:?}")))?;

        assert!(
            code.contains("ctx.write_csv(&uri, DataFrameWriteOptions::new(), None::<_>);"),
            "expected nested rust associated call to keep :: syntax; got:\n{code}"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn test_codegen_borrows_async_rust_backed_free_function_args_from_metadata()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::frontend::typechecker::TypeChecker;
        use incan_core::interop::{RustFunctionSig, RustItemKind, RustItemMetadata, RustParam, RustVisibility};

        let source = r#"
from std.async import sleep
from rust::demo import State
from rust::demo import Plan
from rust::demo import consume

pub async def run(state: State, plan: Plan) -> None:
  await sleep(0.01)
  await consume(state, plan)
"#;
        let tokens = must_ok(lexer::lex(source));
        let ast = must_ok(parser::parse(&tokens));

        let tmp = seeded_rust_inspect_workspace()?;
        let manifest_dir = tmp.path().to_path_buf();
        let mut tc = TypeChecker::new();
        tc.set_rust_inspect_manifest_dir(manifest_dir.clone());
        tc.rust_inspect_cache
            .insert_test_item(
                &manifest_dir,
                RustItemMetadata {
                    canonical_path: "demo::consume".to_string(),
                    definition_path: Some("demo::consume".to_string()),
                    visibility: RustVisibility::Public,
                    kind: RustItemKind::Function(RustFunctionSig {
                        type_params: Vec::new(),
                        params: vec![
                            RustParam {
                                name: Some("state".to_string()),
                                type_display: "&demo::State".to_string(),
                            },
                            RustParam {
                                name: Some("plan".to_string()),
                                type_display: "&demo::Plan".to_string(),
                            },
                        ],
                        return_type: "()".to_string(),
                        is_async: true,
                        is_unsafe: false,
                    }),
                },
            )
            .map_err(|e| std::io::Error::other(format!("seed rust-inspect function: {e}")))?;
        tc.check_program(&ast)
            .map_err(|errs| std::io::Error::other(format!("typecheck failed: {errs:?}")))?;

        let mut lowering = AstLowering::new_with_type_info(tc.type_info().clone());
        let ir_program = lowering
            .lower_program(&ast)
            .map_err(|err| std::io::Error::other(format!("lowering failed: {err:?}")))?;

        let mut codegen = IrCodegen::new();
        codegen.collect_external_rust_functions(&ast);

        let mut emitter = IrEmitter::new(&ir_program.function_registry);
        emitter.set_external_rust_functions(codegen.external_rust_functions.clone());
        let code = emitter
            .emit_program(&ir_program)
            .map_err(|err| std::io::Error::other(format!("emit failed: {err:?}")))?;

        assert!(
            code.contains("consume(&state, &plan).await"),
            "expected borrowed async rust free-function args in generated code; got:\n{code}"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn test_codegen_awaits_async_rust_backed_method_from_metadata() -> Result<(), Box<dyn std::error::Error>> {
        use crate::frontend::typechecker::TypeChecker;
        use incan_core::interop::{
            RustFunctionSig, RustItemKind, RustItemMetadata, RustMethodSig, RustParam, RustTypeInfo, RustVisibility,
        };

        let source = r#"
import std.async
from rust::demo import SessionContext
from rust::demo import CsvReadOptions
from rust::demo import make_context
from rust::demo import make_options

pub async def register_csv() -> None:
  ctx = make_context()
  opts = make_options()
  match await ctx.register_csv("orders", "orders.csv", opts):
    Ok(_) => pass
    Err(_) => pass
"#;
        let tokens = must_ok(lexer::lex(source));
        let ast = must_ok(parser::parse(&tokens));

        let tmp = seeded_rust_inspect_workspace()?;
        let manifest_dir = tmp.path().to_path_buf();
        let mut tc = TypeChecker::new();
        tc.set_rust_inspect_manifest_dir(manifest_dir.clone());
        tc.rust_inspect_cache
            .insert_test_item(
                &manifest_dir,
                RustItemMetadata {
                    canonical_path: "demo::SessionContext".to_string(),
                    definition_path: Some("demo::SessionContext".to_string()),
                    visibility: RustVisibility::Public,
                    kind: RustItemKind::Type(RustTypeInfo {
                        type_params: Vec::new(),
                        has_const_params: false,
                        alias_target: None,
                        metadata_completeness: Default::default(),
                        methods: vec![
                            RustMethodSig {
                                name: "new".to_string(),
                                signature: RustFunctionSig {
                                    type_params: Vec::new(),
                                    params: Vec::new(),
                                    return_type: "demo::SessionContext".to_string(),
                                    is_async: false,
                                    is_unsafe: false,
                                },
                            },
                            RustMethodSig {
                                name: "register_csv".to_string(),
                                signature: RustFunctionSig {
                                    type_params: Vec::new(),
                                    params: vec![
                                        RustParam {
                                            name: Some("self".to_string()),
                                            type_display: "&self".to_string(),
                                        },
                                        RustParam {
                                            name: Some("name".to_string()),
                                            type_display: "&str".to_string(),
                                        },
                                        RustParam {
                                            name: Some("path".to_string()),
                                            type_display: "&str".to_string(),
                                        },
                                        RustParam {
                                            name: Some("options".to_string()),
                                            type_display: "demo::CsvReadOptions".to_string(),
                                        },
                                    ],
                                    return_type: "Result<(), demo::DataFusionError>".to_string(),
                                    is_async: true,
                                    is_unsafe: false,
                                },
                            },
                        ],
                        implemented_traits: Vec::new(),
                        fields: vec![],
                        variants: vec![],
                    }),
                },
            )
            .map_err(|e| std::io::Error::other(format!("seed rust-inspect context: {e}")))?;
        tc.rust_inspect_cache
            .insert_test_item(
                &manifest_dir,
                RustItemMetadata {
                    canonical_path: "demo::CsvReadOptions".to_string(),
                    definition_path: Some("demo::CsvReadOptions".to_string()),
                    visibility: RustVisibility::Public,
                    kind: RustItemKind::Type(RustTypeInfo {
                        type_params: Vec::new(),
                        has_const_params: false,
                        alias_target: None,
                        metadata_completeness: Default::default(),
                        methods: vec![RustMethodSig {
                            name: "new".to_string(),
                            signature: RustFunctionSig {
                                type_params: Vec::new(),
                                params: Vec::new(),
                                return_type: "demo::CsvReadOptions".to_string(),
                                is_async: false,
                                is_unsafe: false,
                            },
                        }],
                        implemented_traits: Vec::new(),
                        fields: vec![],
                        variants: vec![],
                    }),
                },
            )
            .map_err(|e| std::io::Error::other(format!("seed rust-inspect options: {e}")))?;
        tc.rust_inspect_cache
            .insert_test_item(
                &manifest_dir,
                RustItemMetadata {
                    canonical_path: "demo::make_context".to_string(),
                    definition_path: Some("demo::make_context".to_string()),
                    visibility: RustVisibility::Public,
                    kind: RustItemKind::Function(RustFunctionSig {
                        type_params: Vec::new(),
                        params: Vec::new(),
                        return_type: "demo::SessionContext".to_string(),
                        is_async: false,
                        is_unsafe: false,
                    }),
                },
            )
            .map_err(|e| std::io::Error::other(format!("seed rust-inspect context factory: {e}")))?;
        tc.rust_inspect_cache
            .insert_test_item(
                &manifest_dir,
                RustItemMetadata {
                    canonical_path: "demo::make_options".to_string(),
                    definition_path: Some("demo::make_options".to_string()),
                    visibility: RustVisibility::Public,
                    kind: RustItemKind::Function(RustFunctionSig {
                        type_params: Vec::new(),
                        params: Vec::new(),
                        return_type: "demo::CsvReadOptions".to_string(),
                        is_async: false,
                        is_unsafe: false,
                    }),
                },
            )
            .map_err(|e| std::io::Error::other(format!("seed rust-inspect options factory: {e}")))?;
        tc.check_program(&ast)
            .map_err(|errs| std::io::Error::other(format!("typecheck failed: {errs:?}")))?;

        let mut lowering = AstLowering::new_with_type_info(tc.type_info().clone());
        let ir_program = lowering
            .lower_program(&ast)
            .map_err(|err| std::io::Error::other(format!("lowering failed: {err:?}")))?;

        let mut codegen = IrCodegen::new();
        codegen.collect_external_rust_functions(&ast);

        let mut emitter = IrEmitter::new(&ir_program.function_registry);
        emitter.set_external_rust_functions(codegen.external_rust_functions.clone());
        let code = emitter
            .emit_program(&ir_program)
            .map_err(|err| std::io::Error::other(format!("emit failed: {err:?}")))?;

        assert!(
            code.contains("ctx.register_csv(") && code.contains(").await"),
            "expected async Rust method call to be awaited in generated code; got:\n{code}"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn test_codegen_borrows_async_rust_backed_free_function_args_from_real_rust_inspect()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::frontend::typechecker::TypeChecker;
        use crate::rust_inspect::write_async_result_probe_crate;

        let source = r#"
from std.async import sleep
from rust::ra_async_result_probe import State
from rust::ra_async_result_probe import Plan
from rust::ra_async_result_probe import consume

pub async def run(state: State, plan: Plan) -> None:
  await sleep(0.01)
  await consume(state, plan)
"#;
        let tokens = must_ok(lexer::lex(source));
        let ast = must_ok(parser::parse(&tokens));

        let tmp = tempfile::tempdir()?;
        write_async_result_probe_crate(tmp.path())?;

        let mut tc = TypeChecker::new();
        tc.set_rust_inspect_manifest_dir(tmp.path().to_path_buf());
        prewarm_metadata(
            tmp.path(),
            &[
                "ra_async_result_probe::State",
                "ra_async_result_probe::Plan",
                "ra_async_result_probe::consume",
            ],
        )?;
        tc.check_program(&ast)
            .map_err(|errs| std::io::Error::other(format!("typecheck failed: {errs:?}")))?;

        let mut lowering = AstLowering::new_with_type_info(tc.type_info().clone());
        let ir_program = lowering
            .lower_program(&ast)
            .map_err(|err| std::io::Error::other(format!("lowering failed: {err:?}")))?;

        let mut codegen = IrCodegen::new();
        codegen.collect_external_rust_functions(&ast);

        let mut emitter = IrEmitter::new(&ir_program.function_registry);
        emitter.set_external_rust_functions(codegen.external_rust_functions.clone());
        let code = emitter
            .emit_program(&ir_program)
            .map_err(|err| std::io::Error::other(format!("emit failed: {err:?}")))?;

        assert!(
            code.contains("consume(&state, &plan).await"),
            "expected borrowed async rust free-function args from real metadata; got:\n{code}"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn test_codegen_borrows_async_rust_backed_free_function_args_from_generated_lock_workspace()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::backend::project::ProjectGenerator;
        use crate::frontend::typechecker::TypeChecker;
        use crate::manifest::{DependencySource, DependencySpec};
        use crate::rust_inspect::write_hyphenated_function_probe_crate;

        let source = r#"
from std.async import sleep
from rust::foo_bar import State
from rust::foo_bar import Plan
from rust::foo_bar::consumer import consume

pub async def run(state: State, plan: Plan) -> None:
  await sleep(0.01)
  await consume(state, plan)
"#;
        let tokens = must_ok(lexer::lex(source));
        let ast = must_ok(parser::parse(&tokens));

        let tmp = tempfile::tempdir()?;
        let dep_root = tmp.path().join("foo-bar-dep");
        write_hyphenated_function_probe_crate(&dep_root)?;

        let lock_root = tmp.path().join("generated_lock");
        let mut generator = ProjectGenerator::new(&lock_root, "lock_probe", true);
        generator.set_dependencies(vec![DependencySpec {
            crate_name: "foo-bar".to_string(),
            version: None,
            features: vec![],
            default_features: true,
            source: DependencySource::Path { path: dep_root.clone() },
            optional: false,
            package: None,
        }]);
        generator.generate("fn main() {}\n")?;

        let mut tc = TypeChecker::new();
        tc.set_rust_inspect_manifest_dir(lock_root.clone());
        prewarm_metadata(
            &lock_root,
            &["foo_bar::State", "foo_bar::Plan", "foo_bar::consumer::consume"],
        )?;
        tc.check_program(&ast)
            .map_err(|errs| std::io::Error::other(format!("typecheck failed: {errs:?}")))?;

        let mut lowering = AstLowering::new_with_type_info(tc.type_info().clone());
        let ir_program = lowering
            .lower_program(&ast)
            .map_err(|err| std::io::Error::other(format!("lowering failed: {err:?}")))?;

        let mut codegen = IrCodegen::new();
        codegen.collect_external_rust_functions(&ast);

        let mut emitter = IrEmitter::new(&ir_program.function_registry);
        emitter.set_external_rust_functions(codegen.external_rust_functions.clone());
        let code = emitter
            .emit_program(&ir_program)
            .map_err(|err| std::io::Error::other(format!("emit failed: {err:?}")))?;

        assert!(
            code.contains("consume(&state, &plan).await"),
            "expected borrowed async rust free-function args from generated lock workspace; got:\n{code}"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn test_nested_module_codegen_borrows_async_rust_args_from_generated_lock_workspace()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::backend::project::ProjectGenerator;
        use crate::manifest::{DependencySource, DependencySpec};
        use crate::rust_inspect::write_hyphenated_function_probe_crate;

        let main_module = parse_program(
            r#"
def main() -> None:
  return
"#,
        );
        let dep_module = parse_program(
            r#"
from std.async import sleep
from rust::foo_bar import State
from rust::foo_bar import Plan
from rust::foo_bar::consumer import consume

pub async def run(state: State, plan: Plan) -> None:
  await sleep(0.01)
  await consume(state, plan)
"#,
        );

        let tmp = tempfile::tempdir()?;
        let dep_root = tmp.path().join("foo-bar-dep");
        write_hyphenated_function_probe_crate(&dep_root)?;

        let lock_root = tmp.path().join("generated_lock");
        let mut generator = ProjectGenerator::new(&lock_root, "lock_probe", true);
        generator.set_dependencies(vec![DependencySpec {
            crate_name: "foo-bar".to_string(),
            version: None,
            features: vec![],
            default_features: true,
            source: DependencySource::Path { path: dep_root.clone() },
            optional: false,
            package: None,
        }]);
        generator.generate("fn main() {}\n")?;

        let worker_path = vec!["worker".to_string()];
        let mut codegen = IrCodegen::new();
        codegen.set_rust_inspect_manifest_dir(lock_root);
        codegen.add_module_with_path_segments("worker", &dep_module, worker_path.clone());

        let (_main_code, rust_modules) =
            must_ok(codegen.try_generate_multi_file_nested(&main_module, std::slice::from_ref(&worker_path)));
        let worker_code = must_some(rust_modules.get(&worker_path), "missing generated worker module");

        assert!(
            worker_code.contains("consume(&state, &plan).await"),
            "expected borrowed async rust free-function args in generated nested module; got:\n{worker_code}"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn test_try_generate_module_keeps_root_rust_trait_import_issue827() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        write_message_trait_probe_crate(tmp.path())?;

        let worker_module = parse_program(
            r#"
from rust::message_probe import Message, Packet

pub def encode_packet(packet: Packet) -> None:
  _ = packet.encode_to_vec()
"#,
        );
        let mut codegen = IrCodegen::new();
        codegen.set_rust_inspect_manifest_dir(tmp.path().to_path_buf());
        codegen.add_module("worker", &worker_module);

        let code = must_ok(codegen.try_generate_module("worker", &worker_module));

        assert!(
            code.contains("use ::message_probe::{Message, Packet};")
                || (code.contains("use ::message_probe::Message;") && code.contains("use ::message_probe::Packet;")),
            "expected module generation to preserve root Rust trait import needed by encode_to_vec(); got:\n{code}"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn test_codegen_borrows_async_rust_args_after_rust_method_return() -> Result<(), Box<dyn std::error::Error>> {
        use crate::frontend::typechecker::TypeChecker;
        use crate::rust_inspect::write_async_result_probe_crate;

        let source = r#"
from std.async import sleep
from rust::ra_async_result_probe import SessionContext
from rust::ra_async_result_probe import Plan
from rust::ra_async_result_probe import consume

pub async def run(plan: Plan) -> None:
  ctx = SessionContext.new()
  state = ctx.state()
  await sleep(0.01)
  await consume(state, plan)
"#;
        let tokens = must_ok(lexer::lex(source));
        let ast = must_ok(parser::parse(&tokens));

        let tmp = tempfile::tempdir()?;
        write_async_result_probe_crate(tmp.path())?;

        let mut tc = TypeChecker::new();
        tc.set_rust_inspect_manifest_dir(tmp.path().to_path_buf());
        prewarm_metadata(
            tmp.path(),
            &[
                "ra_async_result_probe::SessionContext",
                "ra_async_result_probe::Plan",
                "ra_async_result_probe::consume",
            ],
        )?;
        tc.check_program(&ast)
            .map_err(|errs| std::io::Error::other(format!("typecheck failed: {errs:?}")))?;

        let mut lowering = AstLowering::new_with_type_info(tc.type_info().clone());
        let ir_program = lowering
            .lower_program(&ast)
            .map_err(|err| std::io::Error::other(format!("lowering failed: {err:?}")))?;

        let mut codegen = IrCodegen::new();
        codegen.collect_external_rust_functions(&ast);

        let mut emitter = IrEmitter::new(&ir_program.function_registry);
        emitter.set_external_rust_functions(codegen.external_rust_functions.clone());
        let code = emitter
            .emit_program(&ir_program)
            .map_err(|err| std::io::Error::other(format!("emit failed: {err:?}")))?;

        assert!(
            code.contains("consume(&state, &plan).await"),
            "expected borrowed async rust free-function args after rust method return; got:\n{code}"
        );
        Ok(())
    }

    #[cfg(feature = "rust_inspect")]
    #[test]
    fn test_ir_codegen_uses_configured_rust_inspect_workspace_for_async_borrows()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::rust_inspect::write_hyphenated_function_probe_crate;

        let tmp = tempfile::tempdir()?;
        let dep_root = tmp.path().join("foo-bar-dep");
        write_hyphenated_function_probe_crate(&dep_root)?;

        let host_root = tmp.path().join("host");
        std::fs::create_dir_all(host_root.join("src"))?;
        std::fs::write(
            host_root.join("Cargo.toml"),
            format!(
                "[package]\nname = \"host\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies.foo_bar]\npackage = \"foo-bar\"\npath = \"{}\"\n",
                dep_root.display()
            ),
        )?;
        std::fs::write(host_root.join("src/lib.rs"), "pub fn touch() {}\n")?;

        let source = r#"
from std.async import sleep
from rust::foo_bar import State
from rust::foo_bar import Plan
from rust::foo_bar::consumer import consume

pub async def run(state: State, plan: Plan) -> None:
  await sleep(0.01)
  await consume(state, plan)
"#;
        let ast = parse_program(source);
        let mut codegen = IrCodegen::new();
        codegen.set_rust_inspect_manifest_dir(host_root);
        let code = must_ok(codegen.try_generate(&ast));

        assert!(
            code.contains("consume(&state, &plan).await"),
            "expected IrCodegen to preserve borrowed async args via the configured metadata workspace; got:\n{code}"
        );
        Ok(())
    }

    #[test]
    fn test_codegen_emits_explicit_function_call_type_args() {
        let source = r#"
def id[T](x: T) -> T:
  return x

pub def run() -> int:
  return id[int](1)
"#;
        let ast = parse_program(source);
        let code = must_ok(IrCodegen::new().try_generate(&ast));
        assert!(
            code.contains("id::<i64>(1)") || code.contains("id :: < i64 > (1)"),
            "expected explicit function type args to emit Rust turbofish, got:\n{code}"
        );
    }

    #[test]
    fn test_codegen_emits_explicit_method_call_type_args() {
        let source = r#"
class Box:
  def pick[T](self, value: T) -> T:
    return value

pub def run() -> int:
  let b = Box()
  return b.pick[int](1)
"#;
        let ast = parse_program(source);
        let code = must_ok(IrCodegen::new().try_generate(&ast));
        assert!(
            code.contains("pick::<i64>") || code.contains("pick :: < i64 >"),
            "expected explicit method type args to emit Rust turbofish, got:\n{code}"
        );
    }

    #[test]
    fn test_codegen_emits_full_turbofish_for_mixed_explicit_and_inferred_type_args() {
        let source = r#"
def pair_map[T, U](x: T, y: U) -> int:
  return 0

pub def run() -> int:
  return pair_map[int, _](1, 2)
"#;
        let ast = parse_program(source);
        let code = must_ok(IrCodegen::new().try_generate(&ast));
        assert!(
            code.contains("pair_map::<i64, i64>") || code.contains("pair_map :: < i64 , i64 >"),
            "expected full turbofish for mixed explicit/`_` call-site generics, got:\n{code}"
        );
    }

    #[test]
    fn try_generate_module_uses_checked_composed_newtype_conversion_plan() {
        let ast = parse_program(
            r#"
from std.environ import get_as
from std.traits.convert import TryFrom

type Port = newtype int
type WrappedPort = newtype Port

def read() -> None:
  get_as[WrappedPort]("PORT")
"#,
        );
        let mut codegen = IrCodegen::new();
        let code = must_ok(codegen.try_generate_module("env_types", &ast));
        assert!(
            code.contains("for WrappedPort"),
            "expected checked composed-newtype bridge in generated module:\n{code}"
        );
    }
}
