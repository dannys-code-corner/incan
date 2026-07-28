Warning: truncated output (original token count: 47904)
Total output lines: 4460

//! Call expression lowering: struct constructors, builtin dispatch, newtype checked construction, and regular function
//! calls.

use std::collections::HashMap;

use super::super::super::decl::FunctionParam;
use super::super::super::expr::{
    BuiltinFn, IrCallArg, IrCallArgKind, IrDictEntry, IrExprKind, IrInteropCoercionKind, IrListEntry,
    Literal as IrLiteral, MatchArm, MethodCallArgPolicy, Pattern, VarAccess, VarRefKind,
};
use super::super::super::stmt::IrStmtKind;
use super::super::super::types::IrType;
use super::super::super::{FunctionSignature, IrStmt, Mutability, TypedExpr};
use super::super::AstLowering;
use super::super::errors::LoweringError;
use crate::frontend::api_metadata::{
    ApiDeclaration, checked_api_public_namespace, function_export_from_api, function_export_from_api_projected,
    method_export_from_api,
};
use crate::frontend::ast::{self, TypeConstraintKey};
use crate::frontend::library_exports::CheckedPresetValue;
use crate::frontend::library_manifest_index::LibraryManifestIndexEntry;
use crate::frontend::partial_projection::{PartialPresetRef, merge_named_partial_args};
use crate::frontend::symbols::{CallableParam, NewtypePrimitiveConstraint, ResolvedType};
use crate::frontend::typechecker::{FixedUnpackPlan, RustArgCoercionKind, ValidatedNewtypeCoercionMode};
use crate::frontend::typechecker::{IdentKind, ResolvedOperatorKind};
use crate::library_manifest::{
    FunctionExport, LibraryManifest, MethodExport, ParamDefaultCallArgExport, ParamDefaultCallSignatureExport,
    ParamDefaultExport, ParamExport, ParamKindExport,
};
use crate::provider::{ProviderModuleResolution, ProviderRecord};
use incan_core::lang::keywords::{self, KeywordId};
use incan_core::lang::stdlib;
use incan_core::lang::stdlib::{STDLIB_BUILTINS, STDLIB_ROOT};
use incan_core::lang::surface::constructors::{self, ConstructorId};
use incan_core::lang::surface::types as surface_types;
use incan_core::lang::testing::{self, TestingAssertHelperId};
use incan_core::lang::types::collections::{self, CollectionTypeId};

const TYPE_CONSTRUCTOR_HOOK: &str = "__incan_new";
const API_CRATE_ROOT_SEGMENT: &str = "crate";

impl AstLowering {
    /// Return the builtin member name for an explicit `std.builtins.<name>` callee.
    pub(in crate::backend::ir::lower::expr) fn explicit_builtin_member_name(
        callee: &ast::Spanned<ast::Expr>,
    ) -> Option<&str> {
        let ast::Expr::Field(namespace, member) = &callee.node else {
            return None;
        };
        if Self::is_explicit_builtin_namespace_expr(namespace) {
            Some(member.as_str())
        } else {
            None
        }
    }

    /// Return whether an expression is the explicit builtin namespace `std.builtins`.
    pub(in crate::backend::ir::lower::expr) fn is_explicit_builtin_namespace_expr(
        expr: &ast::Spanned<ast::Expr>,
    ) -> bool {
        let ast::Expr::Field(root, namespace) = &expr.node else {
            return false;
        };
        namespace == STDLIB_BUILTINS && matches!(&root.node, ast::Expr::Ident(name) if name == STDLIB_ROOT)
    }

    /// Rebuild a callable signature from frontend metadata for rest-aware IR emission.
    fn callable_signature_from_params(&self, params: &[CallableParam], ret: &ResolvedType) -> FunctionSignature {
        FunctionSignature {
            params: params
                .iter()
                .enumerate()
                .map(|(idx, param)| {
                    let base_ty = self.lower_resolved_type(&param.ty);
                    let ty = Self::lower_param_container_type(param.kind, base_ty);
                    FunctionParam {
                        name: param.name.clone().unwrap_or_else(|| format!("__incan_arg_{idx}")),
                        ty,
                        mutability: super::super::super::types::Mutability::Immutable,
                        is_self: false,
                        kind: param.kind,
                        default: None,
                    }
                })
                .collect(),
            return_type: self.lower_resolved_type(ret),
        }
    }

    /// Rebuild a callable signature directly from a stdlib method declaration so default expressions survive import
    /// metadata boundaries.
    fn callable_signature_from_stdlib_method_decl(
        &mut self,
        method: &ast::MethodDecl,
    ) -> Result<FunctionSignature, LoweringError> {
        Ok(FunctionSignature {
            params: method
                .params
                .iter()
                .map(|param| {
                    let base_ty = self.lower_type(&param.node.ty.node);
                    let ty = Self::lower_param_container_type(param.node.kind, base_ty);
                    Ok(FunctionParam {
                        name: param.node.name.clone(),
                        ty,
                        mutability: if param.node.is_mut {
                            super::super::super::types::Mutability::Mutable
                        } else {
                            super::super::super::types::Mutability::Immutable
                        },
                        is_self: false,
                        kind: param.node.kind,
                        default: self.lower_param_default_expr(param.node.default.as_ref())?,
                    })
                })
                .collect::<Result<_, LoweringError>>()?,
            return_type: self.lower_type(&method.return_type.node),
        })
    }

    /// Rebuild a callable signature directly from a stdlib function declaration so default expressions survive import
    /// metadata boundaries.
    fn callable_signature_from_stdlib_function_decl(
        &mut self,
        func: &ast::FunctionDecl,
    ) -> Result<FunctionSignature, LoweringError> {
        Ok(FunctionSignature {
            params: func
                .params
                .iter()
                .map(|param| {
                    let base_ty = self.lower_type(&param.node.ty.node);
                    let ty = Self::lower_param_container_type(param.node.kind, base_ty);
                    Ok(FunctionParam {
                        name: param.node.name.clone(),
                        ty,
                        mutability: if param.node.is_mut {
                            super::super::super::types::Mutability::Mutable
                        } else {
                            super::super::super::types::Mutability::Immutable
                        },
                        is_self: false,
                        kind: param.node.kind,
                        default: self.lower_param_default_expr(param.node.default.as_ref())?,
                    })
                })
                .collect::<Result<_, LoweringError>>()?,
            return_type: self.lower_type(&func.return_type.node),
        })
    }

    /// Resolve a callable signature from a public dependency manifest, including materialized default expressions.
    fn callable_signature_for_imported_pub_path(&mut self, path: &[String]) -> Option<FunctionSignature> {
        if path.len() < 3 || path.first().map(String::as_str) != Some("pub") {
            return None;
        }
        let library = path.get(1)?;
        let public_path = path.get(2..)?;
        let function = self.pub_function_export_for_path(library, public_path)?;
        Some(self.callable_signature_from_pub_function_export(library, &function))
    }

    /// Resolve the canonical imported callee path for identifier and module-qualified calls.
    fn imported_callee_path_for_expr(&self, expr: &ast::Spanned<ast::Expr>) -> Option<Vec<String>> {
        if let Some(target) = self.type_info.as_ref().and_then(|info| info.source_target(expr.span))
            && target.module_path.first().map(String::as_str) == Some("pub")
        {
            let mut path = target.module_path.clone();
            path.push(target.name.clone());
            return Some(path);
        }
        match &expr.node {
            ast::Expr::Ident(name) => self
                .active_trait_default_function_path(name)
                .or_else(|| self.import_aliases.get(name).cloned()),
            ast::Expr::Field(object, field) => {
                let mut path = self.imported_field_base_path(&object.node)?;
                path.push(field.clone());
                Some(path)
            }
            _ => None,
        }
    }

    /// Resolve the imported module path that roots a field-chain callee such as `widgets.make_widget`.
    fn imported_field_base_path(&self, expr: &ast::Expr) -> Option<Vec<String>> {
        match expr {
            ast::Expr::Ident(name) => self.import_aliases.get(name).cloned(),
            ast::Expr::Field(object, field) => {
                let mut path = self.imported_field_base_path(&object.node)?;
                path.push(field.clone());
                Some(path)
            }
            _ => None,
        }
    }

    /// Resolve `module.function(...)` syntax when the receiver is an imported stdlib or public dependency module.
    pub(in crate::backend::ir::lower) fn imported_module_function_callee_path(
        &self,
        receiver: &ast::Expr,
        method_name: &str,
    ) -> Option<Vec<String>> {
        let mut path = self.imported_field_base_path(receiver)?;
        match path.first().map(String::as_str) {
            Some(stdlib::STDLIB_ROOT) if self.is_provider_or_legacy_stdlib_module(&path) => {}
            Some("pub") => {
                let library = path.get(1)?;
                let mut public_path = path.get(2..)?.to_vec();
                public_path.push(method_name.to_string());
                self.pub_function_export_for_path(library, &public_path)?;
            }
            _ => return None,
        }
        path.push(method_name.to_string());
        Some(path)
    }

    /// Resolve stdlib module ownership through the active provider catalog, retaining the compiler registry only for
    /// legacy source-only sessions and the provider bootstrap that produces the checked SDK artifacts.
    fn is_provider_or_legacy_stdlib_module(&self, module_path: &[String]) -> bool {
        let Some(provider_plan) = self.provider_plan.as_deref() else {
            return stdlib::is_known_stdlib_module(module_path);
        };
        match provider_plan.resolve_module(module_path) {
            ProviderModuleResolution::Active(_) => true,
            ProviderModuleResolution::Unknown if provider_plan.bootstrap_owns_sdk_module(module_path) => {
                // The bootstrap grant authorizes one top-level namespace; it does not turn imported types and values
                // beneath that root into modules. Until the publisher emits its checked exact claims, source discovery
                // remains the narrow authority for deciding whether this exact path denotes a module.
                stdlib::is_known_stdlib_module(module_path)
            }
            ProviderModuleResolution::Unknown if !provider_plan.has_sdk_catalog() => {
                stdlib::is_known_stdlib_module(module_path)
            }
            ProviderModuleResolution::Disabled(_)
            | ProviderModuleResolution::Unavailable(_)
            | ProviderModuleResolution::Unknown => false,
        }
    }

    /// Fetch the public function export or projected alias export that backs an imported public callable.
    fn pub_function_export(&self, library: &str, function_name: &str) -> Option<FunctionExport> {
        let index = self.provider_plan.as_deref()?.library_manifest_index();
        let LibraryManifestIndexEntry::Loaded { manifest, .. } = index.get(library)? else {
            return None;
        };
        if let Some(function) = manifest
            .exports
            .functions
            .iter()
            .find(|function| function.name == function_name)
        {
            return Some(function.clone());
        }
        if let Some(function) = Self::api_function_export_for_public_name(manifest, function_name) {
            return Some(function);
        }
        manifest
            .exports
            .aliases
            .iter()
            .find(|alias| alias.name == function_name)
            .and_then(|alias| alias.projected_function.clone())
    }

    /// Resolve a public dependency callable by exact checked source path when a module namespace selected it.
    fn pub_function_export_for_path(&self, library: &str, public_path: &[String]) -> Option<FunctionExport> {
        let function_name = public_path.last()?;
        if public_path.len() == 1 {
            return self.pub_function_export(library, function_name);
        }
        let index = self.provider_plan.as_deref()?.library_manifest_index();
        let LibraryManifestIndexEntry::Loaded { manifest, .. } = index.get(library)? else {
            return None;
        };
        let api = manifest.contract_metadata.api.as_ref()?;
        if let Some(function) = Self::api_function_export_for_target_path(api, public_path) {
            return Some(function);
        }
        let (member, namespace_path) = public_path.split_last()?;
        let namespace = checked_api_public_namespace(api, namespace_path)?;
        let mut matches = namespace.members.iter().filter(|candidate| candidate.name == *member);
        let source_path = matches.next()?.source_path.clone();
        if matches.next().is_some() {
            return None;
        }
        Self::api_function_export_for_target_path(api, &source_path)
    }

    /// Resolve public callable aliases through the manifest identity graph before falling back to public-name scans.
    fn api_function_export_for_public_name(
        manifest: &crate::library_manifest::LibraryManifest,
        function_name: &str,
    ) -> Option<FunctionExport> {
        let target_path = manifest
            .contract_metadata
            .identity_graph
            .entry_for_public_name(function_name)
            .and_then(|entry| entry.target_path())?;
        let api = manifest.contract_metadata.api.as_ref()?;
        Self::api_function_export_for_target_path(api, target_path)
    }

    /// Resolve one checked API function from a module-qualified public callable target path.
    fn api_function_export_for_target_path(
        api: &crate::frontend::api_metadata::CheckedApiMetadataPackage,
        target_path: &[String],
    ) -> Option<FunctionExport> {
        let function_name = target_path.last()?;
        let path = if target_path
            .first()
            .is_some_and(|segment| segment == API_CRATE_ROOT_SEGMENT)
        {
            &target_path[1..]
        } else {
            target_path
        };
        let module_path = path.get(..path.len().saturating_sub(1))?;
        let module = api.modules.iter().find(|module| module.module_path == module_path)?;
        let declaration = module.declarations.iter().find(|declaration| match declaration {
            ApiDeclaration::Function(function) => function.name == *function_name,
            ApiDeclaration::Alias(alias) => alias.name == *function_name,
            ApiDeclaration::Partial(partial) => partial.name == *function_name,
            _ => false,
        })?;
        if let ApiDeclaration::Alias(alias) = declaration
            && alias.projected_function.is_none()
        {
            return Self::api_function_export_for_target_path(api, &alias.target_path);
        }
        Self::api_function_export_for_declaration(declaration, function_name)
    }

    /// Convert one checked API declaration into the function export requested by backend call planning.
    fn api_function_export_for_declaration(
        declaration: &ApiDeclaration,
        function_name: &str,
    ) -> Option<FunctionExport> {
        match declaration {
            ApiDeclaration::Function(function) if function.name == function_name => {
                Some(function_export_from_api(function))
            }
            ApiDeclaration::Alias(alias) if alias.name == function_name => alias
                .projected_function
                .as_ref()
                .map(function_export_from_api_projected),
            ApiDeclaration::Partial(partial) if partial.name == function_name => {
                let partial = crate::frontend::api_metadata::partial_export_from_api(partial);
                Some(FunctionExport {
                    name: partial.name,
                    emitted_name: None,
                    type_params: partial.type_params,
                    params: partial.params,
                    return_type: partial.return_type,
                    is_async: partial.is_async,
                })
            }
            _ => None,
        }
    }

    /// Rebuild a public dependency callable signature from manifest metadata, including materialized parameter
    /// defaults.
    fn callable_signature_from_pub_function_export(
        &mut self,
        library: &str,
        function: &FunctionExport,
    ) -> FunctionSignature {
        FunctionSignature {
            params: function
                .params
                .iter()
                .map(|param| {
                    let base_ty = self.lower_pub_manifest_type_ref(library, &param.ty);
                    let kind = param_kind_from_manifest(param.kind);
                    FunctionParam {
                        name: param.name.clone(),
                        ty: Self::lower_param_container_type(kind, base_ty),
                        mutability: Mutability::Immutable,
                        is_self: false,
                        kind,
                        default: self.lower_pub_param_default(library, param),
                    }
                })
                .collect(),
            return_type: self.lower_pub_manifest_type_ref(library, &function.return_type),
        }
    }

    /// Lower one exported parameter default into IR so omitted public dependency arguments can be emitted at call
    /// sites.
    fn lower_pub_param_default(&mut self, library: &str, param: &ParamExport) -> Option<TypedExpr> {
        match param.default.as_ref() {
            Some(ParamDefaultExport::Unsupported) | None => None,
            Some(default) if default.is_materializable() => self.lower_pub_default_expr(library, default),
            Some(_) => None,
        }
    }

    /// Lower a metadata-safe exported default expression into the subset of IR that can be materialized by consumers.
    pub(in crate::backend::ir::lower) fn lower_pub_default_expr(
        &mut self,
        library: &str,
        default: &ParamDefaultExport,
    ) -> Option<TypedExpr> {
        match default {
            ParamDefaultExport::Int(value) => Some(TypedExpr::new(IrExprKind::Int(*value), IrType::Int)),
            ParamDefaultExport::Float(value) => value
                .parse::<f64>()
                .ok()
                .map(|value| TypedExpr::new(IrExprKind::Float(value), IrType::Float)),
            ParamDefaultExport::Bool(value) => Some(TypedExpr::new(IrExprKind::Bool(*value), IrType::Bool)),
            ParamDefaultExport::String(value) => Some(TypedExpr::new(
                IrExprKind::Literal(IrLiteral::StaticStr(value.clone())),
                IrType::StaticStr,
            )),
            ParamDefaultExport::Bytes(value) => Some(TypedExpr::new(IrExprKind::Bytes(value.clone()), IrType::Bytes)),
            ParamDefaultExport::None => Some(TypedExpr::new(IrExprKind::None, IrType::Unit)),
            ParamDefaultExport::List(values) => {
                let entries = values
                    .iter()
                    .map(|value| self.lower_pub_default_expr(library, value).map(IrListEntry::Element))
                    .collect::<Option<Vec<_>>>()?;
                Some(TypedExpr::new(
                    IrExprKind::List(entries),
                    IrType::List(Box::new(IrType::Unknown)),
                ))
            }
            ParamDefaultExport::Dict(entries) => {
                let entries = entries
                    .iter()
                    .map(|entry| {
                        Some(IrDictEntry::Pair(
                            self.lower_pub_default_expr(library, &entry.key)?,
                            Box::new(self.lower_pub_default_expr(library, &entry.value)?),
                        ))
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(TypedExpr::new(
                    IrExprKind::Dict(entries),
                    IrType::Dict(Box::new(IrType::Unknown), Box::new(IrType::Unknown)),
                ))
            }
            ParamDefaultExport::ConstRef(path) => self.lower_pub_default_const_ref(library, path),
            ParamDefaultExport::Call { path, args, signature } => {
                self.lower_pub_default_call(library, path, args, signature.as_ref())
            }
            ParamDefaultExport::Unsupported => None,
        }
    }

    /// Lower a default constant reference as a dependency-qualified value expression.
    fn lower_pub_default_const_ref(&mut self, library: &str, path: &[String]) -> Option<TypedExpr> {
        if path.is_empty() {
            return None;
        }
        let mut expr = TypedExpr::new(
            IrExprKind::Var {
                name: library.to_string(),
                access: VarAccess::Read,
                ref_kind: VarRefKind::ExternalName,
            },
            IrType::Unknown,
        );
        for segment in path {
            expr = TypedExpr::new(
                IrExprKind::Field {
                    object: Box::new(expr),
                    field: segment.clone(),
                },
                IrType::Unknown,
            );
        }
        Some(expr)
    }

    /// Lower an exported default call while preserving the public dependency canonical path for nested call planning.
    fn lower_pub_default_call(
        &mut self,
        library: &str,
        path: &[String],
        args: &[ParamDefaultCallArgExport],
        signature: Option<&ParamDefaultCallSignatureExport>,
    ) -> Option<TypedExpr> {
        let function_name = path.last()?.clone();
        let canonical_path = self.pub_default_canonical_path(library, path);
        let function = self.pub_function_export(library, &function_name);
        let callable_signature = signature
            .map(|signature| self.callable_signature_from_pub_default_call_signature(library, signature))
            .or_else(|| {
                function
                    .as_ref()
                    .map(|function| self.callable_signature_from_pub_function_export(library, function))
            });
        let return_type = signature
            .map(|signature| self.lower_pub_manifest_type_ref(library, &signature.return_type))
            .or_else(|| {
                function
                    .as_ref()
                    .map(|function| self.lower_pub_manifest_type_ref(library, &function.return_type))
            })
            .unwrap_or(IrType::Unknown);
        let args = args
            .iter()
            .map(|arg| {
                Some(IrCallArg {
                    name: arg.name.clone(),
                    kind: if arg.name.is_some() {
                        IrCallArgKind::Named
                    } else {
                        IrCallArgKind::Positional
                    },
                    expr: self.lower_pub_default_expr(library, &arg.value)?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(TypedExpr::new(
            IrExprKind::Call {
                func: Box::new(TypedExpr::new(
                    IrExprKind::Var {
                        name: function_name,
                        access: VarAccess::Read,
                        ref_kind: VarRefKind::Value,
                    },
                    IrType::Unknown,
                )),
                type_args: Vec::new(),
                args,
                callable_signature,
                canonical_path: Some(canonical_path),
            },
            self.pub_external_type(library, return_type),
        ))
    }

    /// Rebuild the source callable surface captured for a provider-owned default helper call.
    fn callable_signature_from_pub_default_call_signature(
        &mut self,
        library: &str,
        signature: &ParamDefaultCallSignatureExport,
    ) -> FunctionSignature {
        FunctionSignature {
            params: signature
                .params
                .iter()
                .map(|param| {
                    let base_ty = self.lower_pub_manifest_type_ref(library, &param.ty);
                    let kind = param_kind_from_manifest(param.kind);
                    FunctionParam {
                        name: param.name.clone(),
                        ty: Self::lower_param_container_type(kind, base_ty),
                        mutability: Mutability::Immutable,
                        is_self: false,
                        kind,
                        default: self.lower_pub_param_default(library, param),
                    }
                })
                .collect(),
            return_type: self.lower_pub_manifest_type_ref(library, &signature.return_type),
        }
    }

    /// Convert a default-expression path from manifest-local spelling into a public dependency canonical path.
    fn pub_default_canonical_path(&self, library: &str, path: &[String]) -> Vec<String> {
        let mut canonical = vec!["pub".to_string(), library.to_string()];
        canonical.extend(path.iter().cloned());
        canonical
    }

    /// Build the emitted function type for a public dependency callable without losing semantic call-planning metadata.
    fn pub_external_function_type(&self, library: &str, signature: &FunctionSignature) -> IrType {
        IrType::Function {
            params: signature
                .params
                .iter()
                .map(|param| self.pub_external_type(library, param.ty.clone()))
                .collect(),
            ret: Box::new(self.pub_external_type(library, signature.return_type.clone())),
        }
    }

    /// Resolve an imported stdlib type method signature by loading the owning stdlib stub AST.
    ///
    /// Function metadata already has a direct stdlib lookup path, but type-member calls such as `App.run()` arrive as
    /// method calls. The lightweight frontend import metadata only records `has_default`, so this path rehydrates the
    /// actual default expressions from the stdlib source declaration before IR emission fills omitted arguments.
    pub(in crate::backend::ir::lower) fn callable_signature_for_imported_stdlib_type_method_path(
        &mut self,
        path: &[String],
        method_name: &str,
    ) -> Result<Option<FunctionSignature>, LoweringError> {
        if path.len() < 3 || path.first().map(String::as_str) != Some(incan_core::lang::stdlib::STDLIB_ROOT) {
            return Ok(None);
        }
        let Some(type_name) = path.last() else {
            return Ok(None);
        };
        let module_path = &path[..path.len() - 1];
        if let Some(provider_crate) = self.sdk_provider_crate_for_module(module_path)
            && let Some(manifest) = self.sdk_provider_manifest_for_module(module_path)
            && let Some(method) = Self::api_method_export_for_pub_type(manifest, type_name, method_name)
        {
            let signature = self.callable_signature_from_compiled_provider_method_export(&provider_crate, &method);
            return Ok(Some(
                self.compiled_provider_external_signature(&provider_crate, signature),
            ));
        }
        if let Some(method) = self
            .stdlib_cache
            .lookup_type_method_decl(module_path, type_name, method_name)
        {
            return self.callable_signature_from_stdlib_method_decl(&method).map(Some);
        }
        Ok(None)
    }

    /// Return the checked SDK provider manifest that owns one exact canonical `std.*` module.
    fn sdk_provider_manifest_for_module(&self, module_path: &[String]) -> Option<&LibraryManifest> {
        self.provider_plan
            .as_deref()?
            .active_sdk_provider_for_module(module_path)?
            .manifest
            .as_deref()
    }

    /// Return the generated Rust crate that owns one compiled SDK module's nominal artifact types.
    fn sdk_provider_crate_for_module(&self, module_path: &[String]) -> Option<String> {
        let provider = self
            .provider_plan
            .as_deref()?
            .active_sdk_provider_for_module(module_path)?;
        Some(Self::sdk_provider_rust_dependency_key(provider))
    }

    /// Return the Rust-import-safe dependency key for one compiled or in-memory SDK provider.
    ///
    /// Installed providers always carry the exact generated crate key in artifact metadata. Source-backed compiler
    /// tests have no physical artifact, so their provider package spelling follows Cargo's hyphen normalization.
    fn sdk_provider_rust_dependency_key(provider: &ProviderRecord) -> String {
        provider
            .artifact
            .as_ref()
            .map(|artifact| artifact.dependency_key.clone())
            .unwrap_or_else(|| provider.identity.name.replace('-', "_"))
    }

    /// Return the unique active SDK provider manifest that declares one nominal type.
    fn sdk_provider_manifest_for_type(&self, type_name: &str) -> Option<&LibraryManifest> {
        let mut matches = self
            .provider_plan
            .as_deref()?
            .active_sdk_records()
            .filter_map(|provider| provider.manifest.as_deref())
            .filter(|manifest| {
                manifest.exports.models.iter().any(|model| model.name == type_name)
                    || manifest.exports.classes.iter().any(|class| class.name == type_name)
                    || manifest.exports.enums.iter().any(|enum_| enum_.name == type_name)
                    || manifest
                        .exports
                        .newtypes
                        .iter()
                        .any(|newtype| newtype.name == type_name)
                    || manifest.contract_metadata.api.iter().any(|api| {
                        api.modules.iter().flat_map(|module| &module.declarations).any(|declaration| {
                            matches!(declaration, ApiDeclaration::Model(model) if model.name == type_name)
                                || matches!(declaration, ApiDeclaration::Class(class) if class.name == type_name)
                                || matches!(declaration, ApiDeclaration::Enum(enum_) if enum_.name == type_name)
                                || matches!(declaration, ApiDeclaration::Newtype(newtype) if newtype.name == type_name)
                        })
                    })
            });
        let manifest = matches.next()?;
        matches.next().is_none().then_some(manifest)
    }

    /// Return the unique compiled SDK provider crate that owns one nominal type.
    pub(in crate::backend::ir::lower) fn sdk_provider_crate_for_type(&self, type_name: &str) -> Option<String> {
        if self.struct_names.contains_key(type_name)
            || self.enum_names.contains_key(type_name)
            || self.class_decls.contains_key(type_name)
            || self.trait_decls.contains_key(type_name)
            || self.newtype_construction.contains_key(type_name)
            || self.source_type_alias_targets.contains_key(type_name)
        {
            return None;
        }
        let mut matches = self
            .provider_plan
            .as_deref()?
            .active_sdk_records()
            .filter(|provider| {
                provider.manifest.as_deref().is_some_and(|manifest| {
                    manifest.exports.models.iter().any(|model| model.name == type_name)
                        || manifest.exports.classes.iter().any(|class| class.name == type_name)
                        || manifest.exports.enums.iter().any(|enum_| enum_.name == type_name)
                        || manifest.exports.newtypes.iter().any(|newtype| newtype.name == type_name)
                        || manifest.contract_metadata.api.iter().any(|api| {
                            api.modules.iter().flat_map(|module| &module.declarations).any(|declaration| {
                                matches!(declaration, ApiDeclaration::Model(model) if model.name == type_name)
                                    || matches!(declaration, ApiDeclaration::Class(class) if class.name == type_name)
                                    || matches!(declaration, ApiDeclaration::Enum(enum_) if enum_.name == type_name)
                                    || matches!(declaration, ApiDeclaration::Newtype(newtype) if newtype.name == type_name)
                            })
                        })
                })
            });
        let provider = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(Self::sdk_provider_rust_dependency_key(provider))
    }

    /// Return the canonical Rust path for a uniquely-owned SDK-provider nominal type.
    ///
    /// Checked provider API metadata owns both the nominal declaration and its source module. Method-dispatch type
    /// arguments can outlive the import expression that introduced them, so lowering must recover that identity from
    /// the provider graph instead of emitting an unqualified short name. Local declarations always win, and an
    /// ambiguous provider graph deliberately produces no path.
    pub(in crate::backend::ir::lower) fn sdk_provider_path_for_type(&self, type_name: &str) -> Option<String> {
        if self.struct_names.contains_key(type_name)
            || self.enum_names.contains_key(type_name)
            || self.class_decls.contains_key(type_name)
            || self.trait_decls.contains_key(type_name)
            || self.newtype_construction.contains_key(type_name)
            || self.source_type_alias_targets.contains_key(type_name)
        {
            return None;
        }

        let mut matches = self
            .provider_plan
            .as_deref()?
            .active_sdk_records()
            .flat_map(|provider| {
                provider
                    .manifest
                    .as_deref()
                    .and_then(|manifest| manifest.contract_metadata.api.as_ref())
                    .into_iter()
                    .flat_map(move |api| {
                        api.modules.iter().filter_map(move |module| {
                            let declares_type = module.declarations.iter().any(|declaration| {
                                matches!(declaration, ApiDeclaration::Model(model) if model.name == type_name)
                                    || matches!(declaration, ApiDeclaration::Class(class) if class.name == type_name)
                                    || matches!(declaration, ApiDeclaration::Enum(enum_) if enum_.name == type_name)
                                    || matches!(declaration, ApiDeclaration::Newtype(newtype) if newtype.name == type_name)
                            });
                            declares_type.then(|| {
                                (
                                    Self::sdk_provider_rust_dependency_key(provider),
                                    module.module_path.clone(),
                                )
                            })
                        })
                    })
            })
            .collect::<Vec<_>>();
        matches.sort();
        matches.dedup();
        let [(provider_crate, module_path)] = matches.as_slice() else {
            return None;
        };

        let source_module = module_path
            .iter()
            .skip_while(|segment| segment.as_str() == stdlib::STDLIB_ROOT)
            .cloned()
            .collect::<Vec<_>>()
            .join("::");
        let owner = if self.sdk_provider_build {
            format!("crate::{}", stdlib::INCAN_STD_NAMESPACE)
        } else {
            format!("{provider_crate}::{}", stdlib::INCAN_STD_NAMESPACE)
        };
        Some(if source_module.is_empty() {
            format!("{owner}::{type_name}")
        } else {
            format!("{owner}::{source_module}::{type_name}")
        })
    }

    /// Resolve a compiled-stdlib method signature from a typed receiver, including artifact-owned defaults.
    ///
    /// Calls on local values such as `path.open("rb")` no longer retain the import expression that introduced
    /// `Path`, so import-path-only lookup cannot recover its omitted arguments. The checked artifact metadata is the
    /// canonical source for these consumer calls.
    pub(in crate::backend::ir::lower) fn callable_signature_for_compiled_provider_type_method(
        &mut self,
        receiver_ty: &IrType,
        method_name: &str,
    ) -> Option<FunctionSignature> {
        let type_name = Self::nominal_receiver_type_name(receiver_ty)?;
        if self.struct_names.contains_key(type_name) || self.enum_names.contains_key(type_name) {
            return None;
        }
        let provider_crate = self.sdk_provider_crate_for_type(type_name)?;
        let manifest = self.sdk_provider_manifest_for_type(type_name)?;
        let method = Self::api_method_export_for_pub_type(manifest, type_name, method_name)?;
        let signature = self.callable_signature_from_compiled_provider_method_export(&provider_crate, &method);
        Some(self.compiled_provider_external_signature(&provider_crate, signature))
    }

    /// Resolve an imported public dependency model/class method signature from the provider manifest.
    pub(in crate::backend::ir::lower) fn callable_signature_for_imported_pub_type_method(
        &mut self,
        library: &str,
        receiver_ty: &IrType,
        method_name: &str,
    ) -> Option<FunctionSignature> {
        let type_name = Self::nominal_receiver_type_name(receiver_ty)?;
        let manifest_index = self.provider_plan.as_deref()?.library_manifest_index();
        let LibraryManifestIndexEntry::Loaded { manifest, .. } = manifest_index.get(library)? else {
            return None;
        };
        let exact_method = Self::public_dependency_type_path(receiver_ty, library).and_then(|target_path| {
            let api = manifest.contract_metadata.api.as_ref()?;
            Self::api_method_export_for_target_path(api, &target_path, method_name)
        });
        let method = exact_method.or_else(|| {
            manifest
                .exports
                .models
                .iter()
                .find(|model| model.name == type_name)
                .and_then(|model| model.methods.iter().find(|method| method.name == method_name))
                .cloned()
                .or_else(|| {
                    manifest
                        .exports
                        .classes
                        .iter()
                        .find(|class| class.name == type_name)
                        .and_then(|class| class.methods.iter().find(|method| method.name == method_name))
                        .cloned()
                })
                .or_else(|| {
                    manifest
                        .exports
                        .newtypes
                        .iter()
                        .find(|newtype| newtype.name == type_name)
                        .and_then(|newtype| newtype.methods.iter().find(|method| method.name == method_name))
                        .cloned()
                })
                .or_else(|| {
                    manifest
                        .exports
                        .enums
                        .iter()
                        .find(|enum_| enum_.name == type_name)
                        .and_then(|enum_| enum_.methods.iter().find(|method| method.name == method_name))
                        .cloned()
                })
                .or_else(|| Self::api_method_export_for_pub_type(manifest, type_name, method_name))
        })?;
        Some(sel…27904 tokens truncated…iDeclaration, ApiFunction, ApiModel, CHECKED_API_METADATA_SCHEMA_VERSION, CheckedApiMetadata,
        CheckedApiMetadataPackage, SourceAnchor, SourceSpan, materialize_checked_api_public_namespaces,
    };
    use crate::frontend::ast::{
        CallArg, Expr, InteropAdapterKind, InteropDirection, InteropEdgeDecl, Literal, Span, Spanned, Type,
    };
    use crate::frontend::library_exports::CheckedPresetValue;
    use crate::frontend::library_manifest_index::{
        LibraryArtifactMetadata, LibraryManifestIndex, LibraryManifestIndexEntry,
    };
    use crate::frontend::symbols::ResolvedType;
    use crate::frontend::typechecker::{
        PartialProjectionInfo, PartialProjectionPreset, PartialProjectionTargetKind, RustArgCoercionInfo,
        RustArgCoercionKind, TypeCheckInfo,
    };
    use crate::library_manifest::{
        AliasExport, CompiledProviderMetadata, ExportIdentity, ExportIdentityKind, ExportIdentityProjection,
        FunctionExport, LIBRARY_IDENTITY_GRAPH_SCHEMA_VERSION, LibraryExports, LibraryIdentityGraph, LibraryManifest,
        ParamDefaultExport, ParamExport, ParamKindExport, ProviderModuleClaim, TypeRef,
    };
    use crate::provider::ProviderPlan;
    use incan_core::interop::CoercionPolicy;
    use incan_core::lang::surface::constructors::{self, ConstructorId};

    fn mk_edge(
        direction: InteropDirection,
        ty: Type,
        adapter_kind: InteropAdapterKind,
        adapter_name: &str,
    ) -> InteropEdgeDecl {
        InteropEdgeDecl {
            direction,
            ty: Spanned::new(ty, Span::new(0, 0)),
            adapter_kind,
            adapter: Spanned::new(Expr::Ident(adapter_name.to_string()), Span::new(0, 0)),
        }
    }

    fn exported_fn(name: &str, param: &str, ret: &str) -> FunctionExport {
        FunctionExport {
            name: name.to_string(),
            emitted_name: None,
            type_params: Vec::new(),
            params: vec![ParamExport {
                name: "value".to_string(),
                ty: TypeRef::Named {
                    name: param.to_string(),
                },
                kind: ParamKindExport::Normal,
                has_default: false,
                default: None,
            }],
            return_type: TypeRef::Named { name: ret.to_string() },
            is_async: false,
        }
    }

    #[test]
    fn qualified_partial_expands_presets_without_a_callable_snapshot_issue948() -> Result<(), String> {
        let span = Span::new(1, 24);
        let preset = Spanned::new(Expr::Literal(Literal::String("preset".to_string())), Span::new(25, 33));
        let mut type_info = TypeCheckInfo::default();
        type_info.record_partial_projection(PartialProjectionInfo {
            name: "hyperquant.default_index".to_string(),
            target_path: vec!["hyperquant".to_string(), "index".to_string(), "build_index".to_string()],
            target_kind: PartialProjectionTargetKind::Function,
            presets: vec![PartialProjectionPreset {
                name: "size".to_string(),
                value: preset,
                external_value: Some(CheckedPresetValue::ConstRef(vec![
                    "hyperquant".to_string(),
                    "index".to_string(),
                    "DEFAULT_SIZE".to_string(),
                ])),
            }],
            external_library: Some("modulelib".to_string()),
        });
        let mut lowering = AstLowering::new_with_type_info(type_info);
        let callee = Spanned::new(
            Expr::Field(
                Box::new(Spanned::new(Expr::Ident("hyperquant".to_string()), Span::new(1, 11))),
                "default_index".to_string(),
            ),
            span,
        );

        let args = lowering
            .partial_projection_call_args(&callee, &[], span)
            .ok_or_else(|| "expected a qualified partial to expand its preset without call metadata".to_string())?;
        let [CallArg::Named(name, value)] = args.as_slice() else {
            return Err(format!("expected one named preset, got {args:?}"));
        };
        assert_eq!(name, "size");
        assert!(matches!(value.node, Expr::Literal(Literal::String(ref value)) if value == "preset"));

        let mut lowered = lowering
            .lower_call_args(&args)
            .map_err(|error| format!("failed to lower expanded preset: {error:?}"))?;
        lowering.materialize_external_partial_presets(&callee, &[], &mut lowered);
        let Some(argument) = lowered.first() else {
            return Err("expected one lowered preset".to_string());
        };
        let IrExprKind::Field {
            object: index,
            field: constant,
        } = &argument.expr.kind
        else {
            return Err(format!(
                "expected external constant field, got {:?}",
                argument.expr.kind
            ));
        };
        let IrExprKind::Field {
            object: namespace,
            field: index_module,
        } = &index.kind
        else {
            return Err(format!("expected external index module, got {:?}", index.kind));
        };
        let IrExprKind::Field {
            object: library,
            field: namespace_module,
        } = &namespace.kind
        else {
            return Err(format!("expected external namespace module, got {:?}", namespace.kind));
        };
        assert_eq!(constant, "DEFAULT_SIZE");
        assert_eq!(index_module, "index");
        assert_eq!(namespace_module, "hyperquant");
        assert!(matches!(
            &library.kind,
            IrExprKind::Var {
                name,
                ref_kind: VarRefKind::ExternalName,
                ..
            } if name == "modulelib"
        ));
        Ok(())
    }

    #[test]
    fn source_partial_without_callable_snapshot_defers_to_canonical_defaults_issue701() {
        let span = Span::new(1, 24);
        let mut type_info = TypeCheckInfo::default();
        type_info.record_partial_projection(PartialProjectionInfo {
            name: "spec".to_string(),
            target_path: vec!["registry".to_string(), "Spec".to_string()],
            target_kind: PartialProjectionTargetKind::ModelConstructor,
            presets: vec![PartialProjectionPreset {
                name: "namespace".to_string(),
                value: Spanned::new(Expr::Ident("DEFAULT_NAMESPACE".to_string()), Span::new(25, 42)),
                external_value: None,
            }],
            external_library: None,
        });
        let lowering = AstLowering::new_with_type_info(type_info);
        let callee = Spanned::new(Expr::Ident("spec".to_string()), span);

        assert!(
            lowering.partial_projection_call_args(&callee, &[], span).is_none(),
            "source partials must leave declaration defaults on their canonical callable signature"
        );
    }

    /// Method-dispatch arguments retain the defining SDK module even after their import expression has disappeared.
    #[test]
    fn method_type_arg_uses_unique_sdk_provider_nominal_path() {
        let mut manifest = LibraryManifest::new("incan-stdlib-system", "0.5.0");
        manifest.contract_metadata.provider = CompiledProviderMetadata {
            namespace_claims: vec![ProviderModuleClaim {
                module_path: vec!["std".to_string(), "io".to_string()],
                required_features: BTreeSet::new(),
            }],
            ..CompiledProviderMetadata::default()
        };
        manifest.contract_metadata.api = Some(CheckedApiMetadataPackage {
            schema_version: CHECKED_API_METADATA_SCHEMA_VERSION,
            package: None,
            modules: vec![CheckedApiMetadata {
                schema_version: CHECKED_API_METADATA_SCHEMA_VERSION,
                module_path: vec!["std".to_string(), "io".to_string()],
                declarations: vec![ApiDeclaration::Model(ApiModel {
                    name: "IoError".to_string(),
                    anchor: SourceAnchor {
                        id: "std.io.IoError".to_string(),
                        span: SourceSpan { start: 0, end: 0 },
                    },
                    docstring: None,
                    docstring_sections: None,
                    decorators: Vec::new(),
                    type_params: Vec::new(),
                    traits: Vec::new(),
                    trait_adoptions: Vec::new(),
                    derives: Vec::new(),
                    fields: Vec::new(),
                    properties: Vec::new(),
                    methods: Vec::new(),
                })],
            }],
            public_namespaces: Vec::new(),
        });
        let mut lowering = AstLowering::new();
        lowering.set_provider_plan(Some(Arc::new(ProviderPlan::for_in_memory_sdk_manifest(
            LibraryManifestIndex::default(),
            manifest,
        ))));
        lowering.set_sdk_provider_build(true);

        assert_eq!(
            lowering.lower_resolved_method_type_arg(&ResolvedType::Named("IoError".to_string())),
            IrType::Struct("crate::__incan_std::io::IoError".to_string())
        );
    }

    #[test]
    fn imported_pub_callable_signature_uses_identity_graph_before_short_name_lookup() -> Result<(), String> {
        let mut manifest = LibraryManifest::new("mylib", "0.1.0");
        manifest.exports = LibraryExports {
            functions: vec![exported_fn("cast", "int", "int")],
            aliases: vec![AliasExport {
                name: "safe_cast".to_string(),
                target_path: vec!["helpers".to_string(), "cast".to_string()],
                projected_function: None,
            }],
            ..LibraryExports::default()
        };
        manifest.contract_metadata.api = Some(CheckedApiMetadataPackage {
            schema_version: CHECKED_API_METADATA_SCHEMA_VERSION,
            package: None,
            modules: vec![CheckedApiMetadata {
                schema_version: CHECKED_API_METADATA_SCHEMA_VERSION,
                module_path: vec!["helpers".to_string()],
                declarations: vec![ApiDeclaration::Function(ApiFunction {
                    name: "cast".to_string(),
                    anchor: SourceAnchor {
                        id: "helpers.cast".to_string(),
                        span: SourceSpan { start: 0, end: 0 },
                    },
                    docstring: None,
                    docstring_sections: None,
                    decorators: Vec::new(),
                    type_params: Vec::new(),
                    params: exported_fn("cast", "str", "str").params,
                    return_type: TypeRef::Named {
                        name: "str".to_string(),
                    },
                    is_async: false,
                })],
            }],
            public_namespaces: Vec::new(),
        });
        manifest.contract_metadata.identity_graph = LibraryIdentityGraph {
            schema_version: LIBRARY_IDENTITY_GRAPH_SCHEMA_VERSION,
            exports: vec![ExportIdentity {
                public_name: "safe_cast".to_string(),
                public_path: vec!["mylib".to_string(), "safe_cast".to_string()],
                source_path: vec!["facade".to_string(), "safe_cast".to_string()],
                kind: ExportIdentityKind::Alias,
                projection: ExportIdentityProjection::Alias {
                    target_path: vec!["helpers".to_string(), "cast".to_string()],
                },
            }],
        };

        let index = LibraryManifestIndex::from_entries(HashMap::from([(
            "mylib".to_string(),
            LibraryManifestIndexEntry::Loaded {
                manifest: Box::new(manifest),
                metadata: LibraryArtifactMetadata::from_crate_root(
                    "mylib",
                    "mylib",
                    std::env::temp_dir().join("incan_identity_graph_backend_test"),
                ),
            },
        )]));
        let mut lowering = AstLowering::new();
        lowering.set_provider_plan(Some(Arc::new(ProviderPlan::for_library_index(index))));

        let signature = lowering
            .callable_signature_for_imported_pub_path(&[
                "pub".to_string(),
                "mylib".to_string(),
                "safe_cast".to_string(),
            ])
            .ok_or_else(|| "expected identity graph to resolve safe_cast through helpers.cast".to_string())?;

        assert_eq!(signature.params[0].ty, IrType::String);
        assert_eq!(signature.return_type, IrType::String);
        Ok(())
    }

    #[test]
    fn imported_pub_parent_namespace_routes_to_nested_checked_callable_issue948() -> Result<(), String> {
        let mut api = CheckedApiMetadataPackage {
            schema_version: CHECKED_API_METADATA_SCHEMA_VERSION,
            package: None,
            modules: vec![CheckedApiMetadata {
                schema_version: CHECKED_API_METADATA_SCHEMA_VERSION,
                module_path: vec!["hyperquant".to_string(), "index".to_string()],
                declarations: vec![ApiDeclaration::Function(ApiFunction {
                    name: "default_index".to_string(),
                    anchor: SourceAnchor {
                        id: "hyperquant.index.default_index".to_string(),
                        span: SourceSpan { start: 0, end: 0 },
                    },
                    docstring: None,
                    docstring_sections: None,
                    decorators: Vec::new(),
                    type_params: Vec::new(),
                    params: Vec::new(),
                    return_type: TypeRef::Named {
                        name: "int".to_string(),
                    },
                    is_async: false,
                })],
            }],
            public_namespaces: Vec::new(),
        };
        materialize_checked_api_public_namespaces(&mut api).map_err(|error| error.to_string())?;
        let mut manifest = LibraryManifest::new("modulelib", "0.1.0");
        manifest.contract_metadata.api = Some(api);
        let index = LibraryManifestIndex::from_entries(HashMap::from([(
            "modulelib".to_string(),
            LibraryManifestIndexEntry::Loaded {
                manifest: Box::new(manifest),
                metadata: LibraryArtifactMetadata::from_crate_root(
                    "modulelib",
                    "modulelib",
                    std::env::temp_dir().join("incan_issue948_nested_callable"),
                ),
            },
        )]));
        let mut lowering = AstLowering::new();
        lowering.set_provider_plan(Some(Arc::new(ProviderPlan::for_library_index(index))));
        lowering.import_aliases.insert(
            "hyperquant".to_string(),
            vec!["pub".to_string(), "modulelib".to_string(), "hyperquant".to_string()],
        );

        assert_eq!(
            lowering.imported_module_function_callee_path(&Expr::Ident("hyperquant".to_string()), "default_index"),
            Some(vec![
                "pub".to_string(),
                "modulelib".to_string(),
                "hyperquant".to_string(),
                "default_index".to_string(),
            ])
        );
        Ok(())
    }

    #[test]
    fn compiled_provider_signature_uses_artifact_api_and_preserves_union_owner() -> Result<(), String> {
        let provider_name = "artifact_only_provider";
        let mut manifest = LibraryManifest::new(provider_name, "0.5.0");
        manifest.contract_metadata.provider = CompiledProviderMetadata {
            namespace_claims: vec![ProviderModuleClaim {
                module_path: vec!["artifact_only".to_string()],
                required_features: BTreeSet::new(),
            }],
            ..CompiledProviderMetadata::default()
        };
        manifest.contract_metadata.api = Some(CheckedApiMetadataPackage {
            schema_version: CHECKED_API_METADATA_SCHEMA_VERSION,
            package: None,
            modules: vec![CheckedApiMetadata {
                schema_version: CHECKED_API_METADATA_SCHEMA_VERSION,
                module_path: vec!["artifact_only".to_string()],
                declarations: vec![ApiDeclaration::Function(ApiFunction {
                    name: "consume".to_string(),
                    anchor: SourceAnchor {
                        id: "artifact_only.consume".to_string(),
                        span: SourceSpan { start: 0, end: 0 },
                    },
                    docstring: None,
                    docstring_sections: None,
                    decorators: Vec::new(),
                    type_params: Vec::new(),
                    params: vec![ParamExport {
                        name: "value".to_string(),
                        ty: TypeRef::Applied {
                            name: "Union".to_string(),
                            args: vec![
                                TypeRef::Named {
                                    name: "int".to_string(),
                                },
                                TypeRef::Named {
                                    name: "str".to_string(),
                                },
                            ],
                        },
                        kind: ParamKindExport::Normal,
                        has_default: true,
                        default: Some(ParamDefaultExport::ConstRef(vec![
                            "artifact_only".to_string(),
                            "Defaults".to_string(),
                            "VALUE".to_string(),
                        ])),
                    }],
                    return_type: TypeRef::Named {
                        name: constructors::as_str(ConstructorId::None).to_string(),
                    },
                    is_async: false,
                })],
            }],
            public_namespaces: Vec::new(),
        });
        let mut lowering = AstLowering::new();
        lowering.set_provider_plan(Some(Arc::new(ProviderPlan::for_in_memory_sdk_manifest(
            LibraryManifestIndex::default(),
            manifest,
        ))));

        let signature = lowering
            .callable_signature_for_imported_stdlib_path(&[
                "std".to_string(),
                "artifact_only".to_string(),
                "consume".to_string(),
            ])
            .map_err(|error| error.message)?
            .ok_or_else(|| "expected compiled provider API signature".to_string())?;

        assert!(matches!(
            &signature.params[0].ty,
            IrType::ExternalUnion { library, .. } if library == provider_name
        ));
        assert!(
            signature.params[0].default.is_some(),
            "artifact-owned const defaults must survive without provider source"
        );

        lowering.import_aliases.insert(
            "artifact".to_string(),
            vec!["std".to_string(), "artifact_only".to_string()],
        );
        assert_eq!(
            lowering.imported_module_function_callee_path(&Expr::Ident("artifact".to_string()), "consume"),
            Some(vec![
                "std".to_string(),
                "artifact_only".to_string(),
                "consume".to_string()
            ]),
            "module-qualified calls must use provider claims rather than the compiler's legacy stdlib registry"
        );
        Ok(())
    }

    #[test]
    fn lower_rusttype_interop_adapter_uses_into_edge_for_rusttype_argument() -> Result<(), String> {
        let mut lowering = AstLowering::new();
        lowering.rusttype_interop_edges.insert(
            "Email".to_string(),
            vec![mk_edge(
                InteropDirection::Into,
                Type::Simple("str".to_string()),
                InteropAdapterKind::Via,
                "email_into_str",
            )],
        );

        let adapter = lowering
            .lower_rusttype_interop_adapter(&IrType::Struct("Email".to_string()), &IrType::String)
            .map_err(|err| format!("expected successful adapter lowering, got {err:?}"))?;

        assert!(adapter.is_some(), "expected into edge adapter to resolve");
        Ok(())
    }

    #[test]
    fn lower_rusttype_interop_adapter_uses_from_edge_for_rusttype_target() -> Result<(), String> {
        let mut lowering = AstLowering::new();
        lowering.rusttype_interop_edges.insert(
            "Email".to_string(),
            vec![mk_edge(
                InteropDirection::From,
                Type::Simple("str".to_string()),
                InteropAdapterKind::Try,
                "email_parse",
            )],
        );

        let adapter = lowering
            .lower_rusttype_interop_adapter(&IrType::String, &IrType::Struct("Email".to_string()))
            .map_err(|err| format!("expected successful adapter lowering, got {err:?}"))?;

        assert!(adapter.is_some(), "expected from edge adapter to resolve");
        Ok(())
    }

    #[test]
    fn lower_method_call_wraps_args_with_rust_arg_coercion() -> Result<(), String> {
        let arg_span = Span::new(10, 20);
        let mut type_info = TypeCheckInfo::default();
        type_info.rust.arg_coercions.insert(
            (arg_span.start, arg_span.end),
            RustArgCoercionInfo {
                rust_target_type: "&str".to_string(),
                target_type: ResolvedType::Ref(Box::new(ResolvedType::Str)),
                kind: RustArgCoercionKind::Builtin(CoercionPolicy::Borrow),
            },
        );

        let mut lowering = AstLowering::new_with_type_info(type_info);
        let expr = Expr::MethodCall(
            Box::new(Spanned::new(Expr::Ident("value".to_string()), Span::new(0, 5))),
            "coerce_me".to_string(),
            Vec::new(),
            vec![CallArg::Positional(Spanned::new(
                Expr::Literal(Literal::String("hello".to_string())),
                arg_span,
            ))],
        );

        let lowered = lowering
            .lower_expr(&expr, Span::new(0, 100))
            .map_err(|err| format!("expected successful lowering, got {err:?}"))?;

        match lowered.kind {
            IrExprKind::MethodCall { args, .. } => {
                let Some(first_arg) = args.first() else {
                    return Err("expected lowered method arg".to_string());
                };
                match &first_arg.expr.kind {
                    IrExprKind::InteropCoerce { to_ty, .. } => {
                        assert_eq!(
                            *to_ty,
                            IrType::StrRef,
                            "expected borrowed str target to lower to StrRef"
                        );
                    }
                    other => {
                        return Err(format!(
                            "expected first method arg to be wrapped in InteropCoerce, got {other:?}"
                        ));
                    }
                }
            }
            other => return Err(format!("expected MethodCall lowering, got {other:?}")),
        }
        Ok(())
    }

    #[test]
    fn lower_rust_named_field_constructor_wraps_fields_with_rust_arg_coercion() -> Result<(), String> {
        let call_span = Span::new(0, 40);
        let callee_span = Span::new(0, 14);
        let arg_span = Span::new(20, 31);
        let mut type_info = TypeCheckInfo::default();
        type_info.expressions.ident_kinds.insert(
            (callee_span.start, callee_span.end),
            crate::frontend::typechecker::IdentKind::TypeName,
        );
        type_info.expressions.expr_types.insert(
            (call_span.start, call_span.end),
            ResolvedType::RustPath("demo::FunctionOption".to_string()),
        );
        type_info.record_rust_named_field_constructor_fields(call_span, vec!["name".to_string()]);
        type_info.rust.arg_coercions.insert(
            (arg_span.start, arg_span.end),
            RustArgCoercionInfo {
                rust_target_type: "String".to_string(),
                target_type: ResolvedType::Str,
                kind: RustArgCoercionKind::Builtin(CoercionPolicy::Exact),
            },
        );

        let mut lowering = AstLowering::new_with_type_info(type_info);
        let expr = Expr::Call(
            Box::new(Spanned::new(Expr::Ident("FunctionOption".to_string()), callee_span)),
            Vec::new(),
            vec![CallArg::Named(
                "name".to_string(),
                Spanned::new(Expr::Ident("OPTION_NAME".to_string()), arg_span),
            )],
        );

        let lowered = lowering
            .lower_expr(&expr, call_span)
            .map_err(|err| format!("expected successful lowering, got {err:?}"))?;

        match lowered.kind {
            IrExprKind::Struct { fields, .. } => {
                let Some((field_name, field_expr)) = fields.first() else {
                    return Err("expected one lowered Rust constructor field".to_string());
                };
                assert_eq!(field_name, "name");
                if !matches!(field_expr.kind, IrExprKind::InteropCoerce { .. }) {
                    return Err(format!(
                        "expected Rust constructor field to be wrapped in InteropCoerce, got {:?}",
                        field_expr.kind
                    ));
                }
            }
            other => return Err(format!("expected Rust Struct lowering, got {other:?}")),
        }
        Ok(())
    }

    #[test]
    fn lower_rust_boundary_target_preserves_nested_borrowed_str_refs() {
        let lowering = AstLowering::new();
        let target = ResolvedType::Generic("List".to_string(), vec![ResolvedType::Ref(Box::new(ResolvedType::Str))]);

        assert_eq!(
            lowering.lower_rust_boundary_target_type(&target),
            IrType::List(Box::new(IrType::StrRef)),
        );
    }

    #[test]
    fn lower_method_call_threads_arg_shape_hint_from_typechecker() -> Result<(), String> {
        let receiver_span = Span::new(0, 5);
        let arg_span = Span::new(10, 17);
        let mut type_info = TypeCheckInfo::default();
        type_info.record_regular_method_arg_shape(receiver_span, "get");
        type_info.rust.arg_coercions.insert(
            (arg_span.start, arg_span.end),
            RustArgCoercionInfo {
                rust_target_type: "&Q".to_string(),
                target_type: ResolvedType::Ref(Box::new(ResolvedType::RustPath("Q".to_string()))),
                kind: RustArgCoercionKind::Builtin(CoercionPolicy::Borrow),
            },
        );

        let mut lowering = AstLowering::new_with_type_info(type_info);
        let expr = Expr::MethodCall(
            Box::new(Spanned::new(Expr::Ident("value".to_string()), receiver_span)),
            "get".to_string(),
            Vec::new(),
            vec![CallArg::Positional(Spanned::new(
                Expr::Literal(Literal::String("hello".to_string())),
                arg_span,
            ))],
        );

        let lowered = lowering
            .lower_expr(&expr, Span::new(0, 100))
            .map_err(|err| format!("expected successful lowering, got {err:?}"))?;

        match lowered.kind {
            IrExprKind::MethodCall { arg_policy, args, .. } => {
                assert_eq!(arg_policy, MethodCallArgPolicy::PreserveShape);
                assert!(
                    !matches!(
                        args.first().map(|arg| &arg.expr.kind),
                        Some(IrExprKind::InteropCoerce { .. })
                    ),
                    "expected preserved lookup method args to skip rust arg coercion wrapping, got {args:?}"
                );
            }
            other => return Err(format!("expected MethodCall lowering, got {other:?}")),
        }
        Ok(())
    }

    #[test]
    fn lower_method_call_applies_required_concrete_borrow_despite_arg_shape_hint() -> Result<(), String> {
        let receiver_span = Span::new(0, 5);
        let arg_span = Span::new(10, 16);
        let mut type_info = TypeCheckInfo::default();
        type_info.record_regular_method_arg_shape(receiver_span, "append_data");
        type_info.rust.arg_coercions.insert(
            (arg_span.start, arg_span.end),
            RustArgCoercionInfo {
                rust_target_type: "&mut demo::Header".to_string(),
                target_type: ResolvedType::RefMut(Box::new(ResolvedType::RustPath("demo::Header".to_string()))),
                kind: RustArgCoercionKind::Borrow { mutable: true },
            },
        );

        let mut lowering = AstLowering::new_with_type_info(type_info);
        let expr = Expr::MethodCall(
            Box::new(Spanned::new(Expr::Ident("builder".to_string()), receiver_span)),
            "append_data".to_string(),
            Vec::new(),
            vec![CallArg::Positional(Spanned::new(
                Expr::Ident("header".to_string()),
                arg_span,
            ))],
        );

        let lowered = lowering
            .lower_expr(&expr, Span::new(0, 100))
            .map_err(|err| format!("expected successful lowering, got {err:?}"))?;

        match lowered.kind {
            IrExprKind::MethodCall { arg_policy, args, .. } => {
                assert_eq!(arg_policy, MethodCallArgPolicy::PreserveShape);
                assert!(matches!(
                    args.first().map(|arg| &arg.expr.kind),
                    Some(IrExprKind::InteropCoerce {
                        kind: IrInteropCoercionKind::RustBorrow { mutable: true },
                        ..
                    })
                ));
            }
            other => return Err(format!("expected MethodCall lowering, got {other:?}")),
        }
        Ok(())
    }

    #[test]
    fn lower_rust_import_associated_method_keeps_type_like_receiver() -> Result<(), String> {
        use crate::frontend::{lexer, parser, typechecker::TypeChecker};

        let source = r#"
from rust::datafusion::dataframe import DataFrameWriteOptions

def f() -> None:
  _ = DataFrameWriteOptions.new()
"#;
        let tokens = lexer::lex(source).map_err(|errs| format!("lex failed: {errs:?}"))?;
        let ast = parser::parse(&tokens).map_err(|errs| format!("parse failed: {errs:?}"))?;

        let mut checker = TypeChecker::new();
        checker
            .check_program(&ast)
            .map_err(|errs| format!("typecheck failed: {errs:?}"))?;

        let mut lowering = AstLowering::new_with_type_info(checker.type_info().clone());
        let program = lowering
            .lower_program(&ast)
            .map_err(|err| format!("lowering failed: {err:?}"))?;

        let function = program
            .declarations
            .iter()
            .find_map(|decl| match &decl.kind {
                IrDeclKind::Function(function) if function.name == "f" => Some(function),
                _ => None,
            })
            .ok_or_else(|| "expected lowered function `f`".to_string())?;
        let Some(stmt) = function.body.first() else {
            return Err("expected expression statement body".to_string());
        };
        let IrStmtKind::Let { value: expr, .. } = &stmt.kind else {
            return Err(format!("expected expression statement body, got {:?}", function.body));
        };

        match &expr.kind {
            IrExprKind::MethodCall { receiver, method, .. } => {
                assert_eq!(method, "new");
                match &receiver.kind {
                    IrExprKind::Var { name, ref_kind, .. } => {
                        assert_eq!(name, "DataFrameWriteOptions");
                        assert_eq!(*ref_kind, VarRefKind::ExternalRustName);
                    }
                    other => return Err(format!("expected variable receiver, got {other:?}")),
                }
            }
            other => return Err(format!("expected MethodCall lowering, got {other:?}")),
        }

        Ok(())
    }

    #[test]
    fn lower_nested_rust_associated_method_arg_keeps_type_like_receiver() -> Result<(), String> {
        use crate::frontend::{lexer, parser, typechecker::TypeChecker};

        let source = r#"
from rust::datafusion::execution::context import SessionContext
from rust::datafusion::dataframe import DataFrameWriteOptions

def f(uri: str) -> None:
  ctx = SessionContext.new()
  _ = ctx.write_csv(uri, DataFrameWriteOptions.new(), None)
"#;
        let tokens = lexer::lex(source).map_err(|errs| format!("lex failed: {errs:?}"))?;
        let ast = parser::parse(&tokens).map_err(|errs| format!("parse failed: {errs:?}"))?;

        let mut checker = TypeChecker::new();
        checker
            .check_program(&ast)
            .map_err(|errs| format!("typecheck failed: {errs:?}"))?;

        let mut lowering = AstLowering::new_with_type_info(checker.type_info().clone());
        let program = lowering
            .lower_program(&ast)
            .map_err(|err| format!("lowering failed: {err:?}"))?;

        let function = program
            .declarations
            .iter()
            .find_map(|decl| match &decl.kind {
                IrDeclKind::Function(function) if function.name == "f" => Some(function),
                _ => None,
            })
            .ok_or_else(|| "expected lowered function `f`".to_string())?;
        let Some(stmt) = function.body.get(1) else {
            return Err(format!("expected nested write_csv statement, got {:?}", function.body));
        };
        let IrStmtKind::Let { value: expr, .. } = &stmt.kind else {
            return Err(format!("expected let statement, got {:?}", function.body));
        };

        let IrExprKind::MethodCall { args, .. } = &expr.kind else {
            return Err(format!("expected outer MethodCall, got {:?}", expr.kind));
        };
        let nested = args
            .get(1)
            .ok_or_else(|| format!("expected second method arg, got {:?}", args))?;

        match &nested.expr.kind {
            IrExprKind::MethodCall { receiver, method, .. } => {
                assert_eq!(method, "new");
                match &receiver.kind {
                    IrExprKind::Var { name, ref_kind, .. } => {
                        assert_eq!(name, "DataFrameWriteOptions");
                        assert_eq!(*ref_kind, VarRefKind::ExternalRustName);
                    }
                    other => return Err(format!("expected variable receiver, got {other:?}")),
                }
            }
            IrExprKind::InteropCoerce { expr, .. } => match &expr.kind {
                IrExprKind::MethodCall { receiver, method, .. } => {
                    assert_eq!(method, "new");
                    match &receiver.kind {
                        IrExprKind::Var { name, ref_kind, .. } => {
                            assert_eq!(name, "DataFrameWriteOptions");
                            assert_eq!(*ref_kind, VarRefKind::ExternalRustName);
                        }
                        other => return Err(format!("expected variable receiver, got {other:?}")),
                    }
                }
                other => return Err(format!("expected nested MethodCall in InteropCoerce, got {other:?}")),
            },
            other => return Err(format!("expected nested MethodCall arg, got {other:?}")),
        }

        Ok(())
    }

    #[test]
    fn lower_rust_constant_method_receiver_as_value_not_type_like() -> Result<(), String> {
        use crate::frontend::{lexer, parser, typechecker::TypeChecker};

        let source = r#"
from rust::std::time import Duration, UNIX_EPOCH

def f() -> None:
  duration = Duration.from_secs(1)
  _ = UNIX_EPOCH.saturating_add(duration)
"#;
        let tokens = lexer::lex(source).map_err(|errs| format!("lex failed: {errs:?}"))?;
        let ast = parser::parse(&tokens).map_err(|errs| format!("parse failed: {errs:?}"))?;

        let mut checker = TypeChecker::new();
        checker
            .check_program(&ast)
            .map_err(|errs| format!("typecheck failed: {errs:?}"))?;

        let mut lowering = AstLowering::new_with_type_info(checker.type_info().clone());
        let program = lowering
            .lower_program(&ast)
            .map_err(|err| format!("lowering failed: {err:?}"))?;

        let function = program
            .declarations
            .iter()
            .find_map(|decl| match &decl.kind {
                IrDeclKind::Function(function) if function.name == "f" => Some(function),
                _ => None,
            })
            .ok_or_else(|| "expected lowered function `f`".to_string())?;
        let Some(stmt) = function.body.get(1) else {
            return Err(format!("expected UNIX_EPOCH method statement, got {:?}", function.body));
        };
        let IrStmtKind::Let { value: expr, .. } = &stmt.kind else {
            return Err(format!("expected let statement, got {:?}", function.body));
        };

        match &expr.kind {
            IrExprKind::MethodCall { receiver, method, .. } => {
                assert_eq!(method, "saturating_add");
                match &receiver.kind {
                    IrExprKind::Var { name, ref_kind, .. } => {
                        assert_eq!(name, "UNIX_EPOCH");
                        assert_eq!(*ref_kind, VarRefKind::Value);
                    }
                    other => return Err(format!("expected variable receiver, got {other:?}")),
                }
            }
            other => return Err(format!("expected MethodCall lowering, got {other:?}")),
        }

        Ok(())
    }

    #[test]
    fn lower_generic_box_as_ref_preserves_nominal_generic_receiver_args() -> Result<(), String> {
        use crate::backend::ir::decl::IrDeclKind;
        use crate::backend::ir::stmt::IrStmtKind;
        use crate::frontend::{lexer, parser, typechecker::TypeChecker};

        let source = r#"
from rust::std::boxed import Box

@derive(Clone)
class Node[T]:
  pub value: T

def take[T](node: Node[T]) -> T:
  return node.value

def from_box[T](child: Box[Node[T]]) -> T:
  return take(child.as_ref())
"#;
        let tokens = lexer::lex(source).map_err(|errs| format!("lex failed: {errs:?}"))?;
        let ast = parser::parse(&tokens).map_err(|errs| format!("parse failed: {errs:?}"))?;

        let mut checker = TypeChecker::new();
        checker
            .check_program(&ast)
            .map_err(|errs| format!("typecheck failed: {errs:?}"))?;

        let mut lowering = AstLowering::new_with_type_info(checker.type_info().clone());
        let program = lowering
            .lower_program(&ast)
            .map_err(|err| format!("lowering failed: {err:?}"))?;

        let function = program
            .declarations
            .iter()
            .find_map(|decl| match &decl.kind {
                IrDeclKind::Function(function) if function.name == "from_box" => Some(function),
                _ => None,
            })
            .ok_or_else(|| "expected lowered function `from_box`".to_string())?;
        let Some(stmt) = function.body.first() else {
            return Err("expected return statement body".to_string());
        };
        let IrStmtKind::Return(Some(expr)) = &stmt.kind else {
            return Err(format!("expected return statement body, got {:?}", function.body));
        };
        let IrExprKind::Call { args, .. } = &expr.kind else {
            return Err(format!("expected call expression, got {:?}", expr.kind));
        };
        let arg = args.first().ok_or_else(|| "expected call arg".to_string())?;

        match &arg.expr.kind {
            IrExprKind::MethodCall { receiver, method, .. } => {
                assert_eq!(method, "as_ref");
                assert_eq!(
                    receiver.ty,
                    IrType::NamedGeneric(
                        "Box".to_string(),
                        vec![IrType::NamedGeneric(
                            "Node".to_string(),
                            vec![IrType::Generic("T".to_string())]
                        )]
                    )
                );
            }
            other => return Err(format!("expected nested MethodCall arg, got {other:?}")),
        }

        Ok(())
    }
}
