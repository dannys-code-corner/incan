//! Frontend bridge from typechecked AST function bodies into Body IR v0.
//!
//! Declaration-level HIR ([`crate::frontend::hir`]) does not model statements or expressions at all (see its module
//! docs), so Body IR v0 lowers directly from `ast::FunctionDecl` bodies plus [`TypeCheckInfo`], rather than from a
//! hypothetical body-shaped HIR that does not exist yet. Every [`Body`](incan_semantics_core::body_ir::Body) this
//! module produces carries a [`CompilerNodeId`] identical to the one [`crate::frontend::hir::build_hir_v0`] would
//! assign the same function's [`crate::frontend::hir`] declaration, so the two can be correlated by id without
//! threading a [`crate::frontend::hir`] value through this API.
//!
//! Body IR v0 lowers a representative, explicitly documented subset of the language surface (see
//! [`incan_semantics_core::body_ir`] module docs for the full rationale). Statements fully lowered: assignment
//! (inferred/let/mutable/reassignment), field/index assignment (including their pre-desugared compound `<op>=`
//! forms), compound assignment (`x <op>= y`), tuple unpacking, multi-target (lvalue) tuple assignment, chained
//! assignment, `return`, `if`/`elif`/`else`, `while`, `for` (both a `start..end` range and a general iterable --
//! builtin collections or a resolved `__iter__`/`__next__` protocol, including the fallible `for item in
//! iterable?:` form), expression statements, statement-position `yield value` (see [`BodyBuilder::lower_stmt_into`]
//! and [`bir::Body::is_generator`]), `assert`, `pass`, `break` (including a value-producing `break` inside a `loop`
//! expression), `continue`. Expressions fully lowered: identifiers, literals (int/float/decimal/bool/string),
//! arithmetic/comparison/boolean binary operators and all three unary operators, calls and method calls (including
//! named, out-of-order, defaulted, and explicitly generic argument spellings -- see [`BodyBuilder::lower_call`]),
//! field access, indexing, slicing, parenthesization, tuples, list/dict/set literals (list and dict spread entries
//! included; set literals have no spread spelling), `model`/`class`
//! construction (named-only at the source level, bound to declared field order -- see
//! [`BodyBuilder::lower_nominal_construction`]), expression-position `if`/`loop`, `try` (`?`), f-strings,
//! list/dict comprehensions, lazy generator expressions, closure literals, partial callables (see
//! [`BodyBuilder::lower_closure`]/[`BodyBuilder::lower_partial`] for how captures
//! are computed and represented explicitly rather than left implicit), and `match` (see [`BodyBuilder::lower_match`]
//! for how patterns are lowered and their bindings scoped).
//!
//! Everything else lowers to an explicit `Statement::Unsupported` / `Operand::Unknown` node rather than panicking,
//! so the model stays total over real programs. That residue is not a short tail, and #1101 tracks it as named
//! remaining work rather than as an implied "almost everything" claim: a spread in a `model`/`class` construction,
//! which refuses as an unresolved field layout because the typechecker records no field binding for it; a spread
//! with no statically proven shape against a callee whose fixed signature *is* resolvable, whose arity no stage can
//! establish; a spread to a locally held callable value; the `**`, bitwise, shift, `in`/`not
//! in`, and `is`/`is not` operators and their compound forms; `if let`/`while let` conditions and destructuring
//! comprehension/generator clauses; statement-position `loop:`; `unsafe:` regions; `await` and `race for`; bytes
//! literals and a `Range` used as a value outside a `for` header; the pattern and `raises` `assert` forms; and
//! vocab/scoped-DSL surface nodes, which reach this module only when a caller skips the desugar pass the legacy
//! pipeline runs first. The sub-issues are #1158 through #1167, plus #1172 for evaluable callable defaults.
//!
//! Two coverage limits are silent rather than marked, and both are deliberate. Expression-position `yield` (the
//! two-way send/receive protocol) is a stub in the existing Rust-emission backend too, so there is no behavior to
//! preserve; the typechecker rejects a bare `yield` with no value before lowering runs. Newtype and enum method
//! bodies produce no [`bir::Body`] at all rather than an `Unsupported` one (#1163) -- see
//! [`lower_owner_method_bodies`].

use std::collections::{HashMap, HashSet};

use incan_core::lang::keywords::KeywordId;
use incan_semantics_core::SurfaceFeatureKey;
use incan_semantics_core::body_ir as bir;
use incan_semantics_core::{
    AbiV0RuntimeRequirement, CanonicalSymbolId, CompilerNodeId, HirSourceSpan, IncanCallableParam,
    IncanCallableParamKind, IncanPrimitiveType, IncanType, SemanticSourceTargetKind, rust_tuple_arity,
};

use incan_core::lang::surface::constructors::{self, ConstructorId};
use incan_core::lang::types::collections::{self, CollectionTypeId};

use crate::frontend::ast;
use crate::frontend::symbols::{CallableParam, ResolvedType};
use crate::frontend::typechecker::{
    FixedUnpackPlan, IdentKind, ResolvedOperatorKind, TypeCheckInfo, semantic_type_from_resolved,
};

/// Build Body IR v0 for every top-level function declaration and every non-abstract class/model/trait method in a
/// typechecked module.
///
/// `ast::Declaration::Function` items each produce one [`bir::Body`], matching the [`CompilerNodeId`]
/// [`crate::frontend::hir::build_hir_v0`] assigns the corresponding declaration (see that function's docs).
/// `ast::Declaration::Model`/`Class`/`Trait` items additionally contribute one [`bir::Body`] per non-abstract method
/// (#1102) — abstract methods (`body: None`, trait requirements with no implementation) contribute nothing, since
/// there is no body to lower. Method [`CompilerNodeId`]s are *not* assigned by [`crate::frontend::hir::build_hir_v0`]
/// today (declaration-level HIR only assigns ids to top-level declarations), so this function constructs its own
/// method ids by scoping the method name under its owning declaration's name — see [`lower_method_body`].
pub fn build_body_ir_module_v0(
    program: &ast::Program,
    module_path: &[String],
    type_info: &TypeCheckInfo,
) -> bir::BodyIrModule {
    let module_identity = body_ir_module_identity(module_path);
    let module_id = CompilerNodeId::module(module_identity.clone());
    let function_default_sources = collect_function_default_sources(program);
    let local_function_declarations = collect_local_function_declarations(program);
    let nominal_declarations = collect_local_nominal_declarations(program, &module_identity);
    let local_nominal_declarations = nominal_declarations
        .iter()
        .map(|declaration| (declaration.name.clone(), declaration.clone()))
        .collect::<LocalNominalDeclarations>();
    let fieldless_enum_declarations = collect_local_fieldless_enum_declarations(program, &module_identity);
    let local_fieldless_enum_declarations = fieldless_enum_declarations
        .iter()
        .map(|declaration| (declaration.name.clone(), declaration.clone()))
        .collect::<LocalFieldlessEnumDeclarations>();
    let value_enum_declarations = collect_local_value_enum_declarations(program, &module_identity);
    let local_value_enum_declarations = value_enum_declarations
        .iter()
        .map(|declaration| (declaration.name.clone(), declaration.clone()))
        .collect::<LocalValueEnumDeclarations>();
    let lowering_facts = BodyIrLoweringFacts {
        type_info,
        function_default_sources: &function_default_sources,
        local_function_declarations: &local_function_declarations,
        local_nominal_declarations: &local_nominal_declarations,
        local_fieldless_enum_declarations: &local_fieldless_enum_declarations,
        local_value_enum_declarations: &local_value_enum_declarations,
        module_identity: &module_identity,
        module_path,
    };
    let bodies = program
        .declarations
        .iter()
        .flat_map(|decl| -> Vec<bir::Body> {
            match &decl.node {
                ast::Declaration::Function(function) => {
                    vec![lower_function_body(function, decl.span, &lowering_facts)]
                }
                ast::Declaration::Model(model) => lower_owner_method_bodies(
                    &model.methods,
                    &model.name,
                    owner_self_type(&model.name, &model.type_params),
                    &lowering_facts,
                ),
                ast::Declaration::Class(class) => lower_owner_method_bodies(
                    &class.methods,
                    &class.name,
                    owner_self_type(&class.name, &class.type_params),
                    &lowering_facts,
                ),
                ast::Declaration::Trait(trait_decl) => lower_owner_method_bodies(
                    &trait_decl.methods,
                    &trait_decl.name,
                    IncanType::SelfType,
                    &lowering_facts,
                ),
                _ => Vec::new(),
            }
        })
        .collect();
    bir::BodyIrModule {
        module_id,
        nominal_declarations,
        fieldless_enum_declarations,
        value_enum_declarations,
        bodies,
    }
}

/// Source-declared ordinary default expressions for each top-level function in this module.
///
/// Body-IR lowering needs this small source map only while it lowers a local `partial target(...)` into a forwarding
/// closure: the checked callable signature retains availability but not the executable default expression. The map
/// never leaves this frontend boundary; the resulting [`bir::CallableParamDefault::Source`] stores only Body IR.
type FunctionDefaultSources = HashMap<String, Vec<FunctionDefaultSource>>;

/// Exact spans of this source module's top-level function declarations, grouped by source spelling.
///
/// The typechecker exposes an intentionally wider overload surface that can include imports and aliases. Direct
/// Body-IR dispatch only admits a declaration physically represented by this module, so lowering retains this
/// small source-local map long enough to attach the chosen declaration identity to each named call.
type LocalFunctionDeclarations = HashMap<String, Vec<ast::Span>>;

/// Plain source-local models whose checked declaration layout is retained for direct nominal execution.
///
/// This frontend map intentionally contains only non-generic, behavior-free models. It is used only while lowering
/// a checked constructor call to attach an exact declaration identity; the resulting [`bir::NominalDeclaration`]
/// records are the direct executor's sole layout authority. Classes, trait-adopting models, and models carrying
/// methods/properties/aliases are absent rather than being approximated as inert field bags.
type LocalNominalDeclarations = HashMap<String, bir::NominalDeclaration>;

/// Source-local fieldless normal enums whose canonical unit variants are retained for direct comparison.
///
/// This map exists only while lowering. The executor receives `BodyIrModule::fieldless_enum_declarations` and
/// revalidates exact enum/member identities there, so a source spelling never selects an imported or aliased enum.
type LocalFieldlessEnumDeclarations = HashMap<String, bir::FieldlessEnumDeclaration>;

/// Source-local RFC 032 value enums whose canonical scalar members are retained for direct execution.
///
/// This map is lowering-only. The executor receives `BodyIrModule::value_enum_declarations` and verifies retained
/// enum/member identities there, so imports, aliases, ordinary enums, and non-retained same-spelling forms never
/// become direct runtime targets.
type LocalValueEnumDeclarations = HashMap<String, bir::ValueEnumDeclaration>;

/// Borrowed module facts shared by every body lowerer.
///
/// These facts are collected once from checked source and remain frontend-only: emitted Body IR carries only the
/// identities and representations a later direct executor needs. Keeping the bundle explicit avoids widening any
/// individual lowering helper's parameter surface as profiles add one bounded source-local fact at a time.
struct BodyIrLoweringFacts<'type_info, 'source> {
    type_info: &'type_info TypeCheckInfo,
    function_default_sources: &'source FunctionDefaultSources,
    local_function_declarations: &'source LocalFunctionDeclarations,
    local_nominal_declarations: &'source LocalNominalDeclarations,
    local_fieldless_enum_declarations: &'source LocalFieldlessEnumDeclarations,
    local_value_enum_declarations: &'source LocalValueEnumDeclarations,
    module_identity: &'source str,
    module_path: &'source [String],
}

/// Source facts a synthesized local partial needs for one target parameter.
#[derive(Clone)]
struct FunctionDefaultSource {
    /// The target parameter's original span.
    param_span: ast::Span,
    /// The target's ordinary source default, if it declared one.
    default: Option<ast::Spanned<ast::Expr>>,
}

/// Collect the source expressions a synthesized local partial needs to retain target defaults in Body IR.
fn collect_function_default_sources(program: &ast::Program) -> FunctionDefaultSources {
    program
        .declarations
        .iter()
        .filter_map(|decl| match &decl.node {
            ast::Declaration::Function(function) => Some((
                function.name.clone(),
                function
                    .params
                    .iter()
                    .map(|param| FunctionDefaultSource {
                        param_span: param.span,
                        default: param.node.default.clone(),
                    })
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

/// Collect the exact source spans eligible for same-module direct named-call dispatch.
fn collect_local_function_declarations(program: &ast::Program) -> LocalFunctionDeclarations {
    let mut declarations = LocalFunctionDeclarations::new();
    for declaration in &program.declarations {
        if let ast::Declaration::Function(function) = &declaration.node {
            declarations
                .entry(function.name.clone())
                .or_default()
                .push(declaration.span);
        }
    }
    declarations
}

/// Determine whether a model can carry the small direct-replacement declaration fact.
///
/// This is deliberately a source-local data-model shape, not a general nominal-semantics predicate. The replacement
/// runtime cannot execute model decorators, trait behavior, methods, field aliases, or generic substitution without
/// facts that Body IR does not retain. Field defaults remain represented by each construction's checked binding, so a
/// fully supplied construction may execute while any omitted default still refuses at that constructor's span.
pub(crate) fn is_direct_replacement_plain_model(model: &ast::ModelDecl) -> bool {
    model.decorators.is_empty()
        && model.type_params.is_empty()
        && model.traits.is_empty()
        && model.method_aliases.is_empty()
        && model.method_partials.is_empty()
        && model.properties.is_empty()
        && model.methods.is_empty()
        && model.fields.iter().all(|field| field.node.metadata.alias.is_none())
}

/// Retain directly executable model declarations in source order.
///
/// Constructor argument binding already comes from the typechecker; this adds only the source-local declaration
/// identity and canonical raw field order the direct runtime otherwise could not establish without reopening AST or
/// typechecker state. This deliberately does not retain a general nominal registry.
fn collect_local_nominal_declarations(program: &ast::Program, module_identity: &str) -> Vec<bir::NominalDeclaration> {
    program
        .declarations
        .iter()
        .filter_map(|declaration| {
            let ast::Declaration::Model(model) = &declaration.node else {
                return None;
            };
            is_direct_replacement_plain_model(model).then(|| bir::NominalDeclaration {
                direct_declaration_id: CompilerNodeId::declaration_span(
                    module_identity,
                    declaration.span.start,
                    declaration.span.end,
                ),
                name: model.name.clone(),
                fields: model.fields.iter().map(|field| field.node.name.clone()).collect(),
                type_parameter_count: model.type_params.len(),
            })
        })
        .collect()
}

/// Determine whether an enum carries the narrow source-local fieldless normal-enum declaration fact.
///
/// This excludes every declaration form whose behavior needs additional semantic representation: scalar value enums,
/// payload construction, aliases, trait dispatch, custom methods, decorators, and generic substitution. The direct
/// runtime can therefore materialize only a canonical unit carrier and compare its retained identity.
pub(crate) fn is_direct_replacement_fieldless_enum(enum_decl: &ast::EnumDecl) -> bool {
    enum_decl.decorators.is_empty()
        && enum_decl.type_params.is_empty()
        && enum_decl.value_type.is_none()
        && enum_decl.traits.is_empty()
        && enum_decl.variant_aliases.is_empty()
        && enum_decl.methods.is_empty()
        && enum_decl
            .variants
            .iter()
            .all(|variant| variant.node.fields.is_empty() && variant.node.value.is_none())
}

/// Retain exact source-local fieldless normal-enum declaration and unit-member facts in source order.
///
/// Only this registry reaches the direct runtime. It deliberately has no payload layouts, aliases, match facts, or
/// source-symbol lookup facility, so its existence cannot widen into general enum execution by spelling alone.
fn collect_local_fieldless_enum_declarations(
    program: &ast::Program,
    module_identity: &str,
) -> Vec<bir::FieldlessEnumDeclaration> {
    program
        .declarations
        .iter()
        .filter_map(|declaration| {
            let ast::Declaration::Enum(enum_decl) = &declaration.node else {
                return None;
            };
            is_direct_replacement_fieldless_enum(enum_decl).then(|| bir::FieldlessEnumDeclaration {
                direct_declaration_id: CompilerNodeId::declaration_span(
                    module_identity,
                    declaration.span.start,
                    declaration.span.end,
                ),
                name: enum_decl.name.clone(),
                variants: enum_decl
                    .variants
                    .iter()
                    .map(|variant| bir::FieldlessEnumVariantDeclaration {
                        direct_declaration_id: CompilerNodeId::declaration_span(
                            module_identity,
                            variant.span.start,
                            variant.span.end,
                        ),
                        name: variant.node.name.clone(),
                    })
                    .collect(),
            })
        })
        .collect()
}

/// Determine whether an enum carries the narrow source-local RFC 032 scalar declaration fact.
///
/// This predicate intentionally excludes aliases and all behavior-bearing forms even when they are source-valid:
/// the direct executor may validate only a canonical literal member and the compiler-provided `.value()` extraction,
/// not trait dispatch, custom methods, alias canonicalization, generic substitution, or payload construction.
pub(crate) fn is_direct_replacement_value_enum(enum_decl: &ast::EnumDecl) -> bool {
    enum_decl.decorators.is_empty()
        && enum_decl.type_params.is_empty()
        && enum_decl.value_type.is_some()
        && enum_decl.traits.is_empty()
        && enum_decl.variant_aliases.is_empty()
        && enum_decl.methods.is_empty()
        && enum_decl.variants.iter().all(|variant| {
            variant.node.fields.is_empty()
                && matches!(
                    variant.node.value.as_ref().map(|value| &value.node),
                    Some(ast::ValueEnumLiteral::Int(_) | ast::ValueEnumLiteral::Str(_))
                )
        })
}

/// Retain exact source-local RFC 032 value-enum declaration and canonical literal-member facts in source order.
///
/// A later direct executor receives only this Body-IR registry. It does not reopen AST/typechecker state to resolve
/// a `Name.Member` spelling, so lowering returns no record for imports, aliases, ordinary enums, or declarations
/// whose shape cannot truthfully support the generated scalar `.value()` surface.
fn collect_local_value_enum_declarations(
    program: &ast::Program,
    module_identity: &str,
) -> Vec<bir::ValueEnumDeclaration> {
    program
        .declarations
        .iter()
        .filter_map(|declaration| {
            let ast::Declaration::Enum(enum_decl) = &declaration.node else {
                return None;
            };
            if !is_direct_replacement_value_enum(enum_decl) {
                return None;
            }
            let backing = match enum_decl.value_type.as_ref().map(|value| value.node) {
                Some(ast::ValueEnumType::Int) => bir::ValueEnumBacking::Int,
                Some(ast::ValueEnumType::Str) => bir::ValueEnumBacking::Str,
                None => return None,
            };
            let variants = enum_decl
                .variants
                .iter()
                .filter_map(|variant| {
                    let raw_value = match variant.node.value.as_ref().map(|value| &value.node) {
                        Some(ast::ValueEnumLiteral::Int(value)) if matches!(backing, bir::ValueEnumBacking::Int) => {
                            bir::Constant::Int(value.value)
                        }
                        Some(ast::ValueEnumLiteral::Str(value)) if matches!(backing, bir::ValueEnumBacking::Str) => {
                            bir::Constant::Str(value.clone())
                        }
                        _ => return None,
                    };
                    Some(bir::ValueEnumVariantDeclaration {
                        direct_declaration_id: CompilerNodeId::declaration_span(
                            module_identity,
                            variant.span.start,
                            variant.span.end,
                        ),
                        name: variant.node.name.clone(),
                        raw_value,
                    })
                })
                .collect::<Vec<_>>();
            (variants.len() == enum_decl.variants.len()).then(|| bir::ValueEnumDeclaration {
                direct_declaration_id: CompilerNodeId::declaration_span(
                    module_identity,
                    declaration.span.start,
                    declaration.span.end,
                ),
                name: enum_decl.name.clone(),
                backing,
                variants,
            })
        })
        .collect()
}

/// Lower every non-abstract method in `methods` (owned by the class/model/trait named `owner_name`) into one
/// [`bir::Body`] each, skipping abstract methods (`body: None`). `receiver_ty` is the typechecker-equivalent type
/// for a declared receiver: a concrete nominal type for models/classes or [`IncanType::SelfType`] for trait defaults.
///
/// Newtype and enum declarations also carry a `methods` field in the AST (see `crates/incan_syntax/src/ast/
/// decls.rs`), but #1102's own scope names only class/model/trait bodies, so this function is deliberately not
/// called for those two declaration kinds. #1163 owns extending it. Until then this is the module's only *silent*
/// coverage gap: every other unsupported construct leaves a `StatementKind::Unsupported` or `Operand::Unknown`
/// marker behind, while a newtype or enum method produces no [`bir::Body`] at all, so a consumer counting bodies
/// reads a program using one as fully represented.
fn lower_owner_method_bodies(
    methods: &[ast::Spanned<ast::MethodDecl>],
    owner_name: &str,
    receiver_ty: IncanType,
    lowering_facts: &BodyIrLoweringFacts<'_, '_>,
) -> Vec<bir::Body> {
    methods
        .iter()
        .filter_map(|method| lower_method_body(&method.node, method.span, owner_name, &receiver_ty, lowering_facts))
        .collect()
}

/// Render a module path into the same module identity spelling [`crate::frontend::hir`] uses, so declaration ids
/// line up between the two representations.
fn body_ir_module_identity(module_path: &[String]) -> String {
    incan_semantics_core::module_identity_for_path(module_path)
}

/// Convert an AST byte-offset span into a Body IR source span.
const fn hir_span(span: ast::Span) -> HirSourceSpan {
    HirSourceSpan::new(span.start, span.end)
}

/// Return the checked payload types of an intrinsic `Result[ok, error]` carrier.
///
/// This is deliberately a narrow query over the typechecker-owned semantic type. The Body-IR lowerer uses it only
/// to retain facts which direct execution cannot reconstruct: which intrinsic constructor is being formed, which
/// pattern payload is being bound, and whether `?` preserves the enclosing error type exactly. It does not infer
/// a conversion or admit a differently shaped generic carrier.
fn result_type_parts(ty: &IncanType) -> Option<(&IncanType, &IncanType)> {
    let IncanType::Generic { base, args } = ty else {
        return None;
    };
    (collections::from_str(base) == Some(CollectionTypeId::Result)).then_some(())?;
    match args.as_slice() {
        [ok_type, error_type] => Some((ok_type, error_type)),
        _ => None,
    }
}

/// Return just the checked error channel for an intrinsic `Result` carrier.
fn result_error_type(ty: &IncanType) -> Option<&IncanType> {
    result_type_parts(ty).map(|(_, error_type)| error_type)
}

/// Map only the compiler-owned intrinsic constructor spellings to Body-IR result variants.
fn result_variant_kind(name: &str) -> Option<bir::ResultVariantKind> {
    match constructors::from_str(name) {
        Some(ConstructorId::Ok) => Some(bir::ResultVariantKind::Ok),
        Some(ConstructorId::Err) => Some(bir::ResultVariantKind::Err),
        _ => None,
    }
}

/// Lower one function declaration's body into Body IR v0.
fn lower_function_body(
    function: &ast::FunctionDecl,
    decl_span: ast::Span,
    lowering_facts: &BodyIrLoweringFacts<'_, '_>,
) -> bir::Body {
    let decl_id = CompilerNodeId::declaration(lowering_facts.module_identity, &function.name);
    let direct_call_id =
        CompilerNodeId::declaration_span(lowering_facts.module_identity, decl_span.start, decl_span.end);
    // The bare-name map is a compatibility projection and collapses top-level overloads. A body is one physical
    // declaration, so its parameter types must come from the same span-keyed fact the direct-call identity uses.
    let binding = lowering_facts
        .type_info
        .declarations
        .function_bindings_by_span
        .get(&(decl_span.start, decl_span.end));
    let owner_return_type = binding
        .map(|binding| semantic_type_from_resolved(&binding.return_type))
        .unwrap_or(IncanType::Unknown);

    let mut builder = BodyBuilder::new(lowering_facts, owner_return_type);
    let root_scope = builder.new_scope(None, hir_span(decl_span));

    let mut param_locals = Vec::with_capacity(function.params.len());
    for (index, param) in function.params.iter().enumerate() {
        let ty = binding
            .and_then(|b| b.params.get(index))
            .map(|p| semantic_type_from_resolved(&p.ty))
            .unwrap_or(IncanType::Unknown);
        let local = builder.declare_new_local(
            param.node.name.clone(),
            ty,
            root_scope,
            hir_span(param.span),
            &function.body,
        );
        builder.locals[local.index()].origin = bir::LocalOrigin::Parameter;
        param_locals.push(local);
    }

    let mut params = Vec::with_capacity(function.params.len());
    for (param, local) in function.params.iter().zip(param_locals.iter().copied()) {
        let ty = builder.locals[local.index()].ty.clone();
        params.push(bir::CallableParam {
            local,
            name: param.node.name.clone(),
            ty,
            span: hir_span(param.span),
            default: builder.lower_callable_default(param.node.default.as_ref(), root_scope),
        });
    }

    let mut stmts = Vec::new();
    builder.lower_block_into(&function.body, root_scope, &mut stmts);
    builder.insert_scope_drops(&mut stmts, root_scope);

    if builder
        .locals
        .iter()
        .any(|local| !local.ty.abi_v0_facts().ownership.is_trivially_copy())
    {
        builder.record_runtime_requirement(AbiV0RuntimeRequirement::Allocator);
    }

    bir::Body {
        decl_id,
        direct_call_id,
        name: function.name.clone(),
        span: hir_span(decl_span),
        locals: builder.locals,
        params,
        param_locals,
        scopes: builder.scopes,
        block: bir::Block {
            scope: root_scope,
            stmts,
        },
        runtime_requirements: builder.runtime_requirements,
        panic_facts: builder.panic_facts,
        is_async: function.is_async(),
    }
}

/// Lower one method declaration's body into Body IR v0, or `None` for an abstract method (`body: None` — a trait
/// requirement with no implementation, which has no body to lower).
///
/// Ordinary (non-receiver) method parameters declare with the resolved type the typechecker recorded in
/// [`DeclarationArtifacts::method_bindings_by_span`](
/// crate::frontend::typechecker::type_info::DeclarationArtifacts::method_bindings_by_span), keyed by this method's
/// own declaration span (#1121) — mirroring exactly how [`lower_function_body`] consumes `function_bindings` for
/// top-level `def` parameters. This lookup can only miss (falling back to [`IncanType::Unknown`], matching
/// `lower_function_body`'s own fallback) when the typechecker genuinely produced no fact for this declaration, such
/// as a method belonging to a declaration kind excluded from `TypeChecker::check_method_with_self_ty`'s call sites;
/// it is not the normal path for an ordinarily checked method. This does not change the accuracy of ownership facts
/// computed for actual *reads* of those parameters inside the body: those go through [`BodyBuilder::resolve_ty`] at
/// each read's own span, which is populated uniformly for every checked expression regardless of whether it sits in
/// a function or a method body.
///
/// The `self`/`mut self` receiver, when present, is declared as the body's first local (before ordinary
/// parameters) via [`BodyBuilder::declare_receiver_local`], typed with the typechecker-equivalent `receiver_ty`.
/// A method with `receiver: None` (a static/associated method) lowers with no receiver local at all, identically
/// in shape to a free function's body; its ordinary parameters still resolve through the same binding lookup.
fn lower_method_body(
    method: &ast::MethodDecl,
    decl_span: ast::Span,
    owner_name: &str,
    receiver_ty: &IncanType,
    lowering_facts: &BodyIrLoweringFacts<'_, '_>,
) -> Option<bir::Body> {
    let body_stmts = method.body.as_ref()?;

    // Method names are not unique across a module the way top-level function names are (two classes can each
    // declare a method named `new`), so the method's CompilerNodeId is scoped under its owning declaration's name
    // rather than reusing `CompilerNodeId::declaration(module_identity, &method.name)` directly.
    let decl_id = CompilerNodeId::declaration(
        lowering_facts.module_identity,
        &format!("{owner_name}::{}", method.name),
    );
    let direct_call_id =
        CompilerNodeId::declaration_span(lowering_facts.module_identity, decl_span.start, decl_span.end);
    let binding = lowering_facts
        .type_info
        .declarations
        .method_bindings_by_span
        .get(&(decl_span.start, decl_span.end));
    let owner_return_type = binding
        .map(|binding| semantic_type_from_resolved(&binding.return_type))
        .unwrap_or(IncanType::Unknown);

    let mut builder = BodyBuilder::new(lowering_facts, owner_return_type);
    let root_scope = builder.new_scope(None, hir_span(decl_span));

    let mut params = Vec::with_capacity(method.params.len() + 1);
    let mut param_locals = Vec::with_capacity(method.params.len() + 1);
    if let Some(receiver) = method.receiver {
        let mutable = matches!(receiver, ast::Receiver::Mutable);
        let self_local = builder.declare_receiver_local(receiver_ty.clone(), mutable, root_scope, hir_span(decl_span));
        param_locals.push(self_local);
        params.push(bir::CallableParam {
            local: self_local,
            name: "self".to_string(),
            ty: receiver_ty.clone(),
            span: hir_span(decl_span),
            default: bir::CallableParamDefault::Required,
        });
    }

    let mut ordinary_param_locals = Vec::with_capacity(method.params.len());
    for (index, param) in method.params.iter().enumerate() {
        let ty = binding
            .and_then(|b| b.params.get(index))
            .map(|p| semantic_type_from_resolved(&p.ty))
            .unwrap_or(IncanType::Unknown);
        let local = builder.declare_new_local(
            param.node.name.clone(),
            ty,
            root_scope,
            hir_span(param.span),
            body_stmts,
        );
        builder.locals[local.index()].origin = bir::LocalOrigin::Parameter;
        param_locals.push(local);
        ordinary_param_locals.push(local);
    }

    for (param, local) in method.params.iter().zip(ordinary_param_locals) {
        let ty = builder.locals[local.index()].ty.clone();
        params.push(bir::CallableParam {
            local,
            name: param.node.name.clone(),
            ty,
            span: hir_span(param.span),
            default: builder.lower_callable_default(param.node.default.as_ref(), root_scope),
        });
    }

    let mut stmts = Vec::new();
    builder.lower_block_into(body_stmts, root_scope, &mut stmts);
    builder.insert_scope_drops(&mut stmts, root_scope);

    if builder
        .locals
        .iter()
        .any(|local| !local.ty.abi_v0_facts().ownership.is_trivially_copy())
    {
        builder.record_runtime_requirement(AbiV0RuntimeRequirement::Allocator);
    }

    Some(bir::Body {
        decl_id,
        direct_call_id,
        name: method.name.clone(),
        span: hir_span(decl_span),
        locals: builder.locals,
        params,
        param_locals,
        scopes: builder.scopes,
        block: bir::Block {
            scope: root_scope,
            stmts,
        },
        runtime_requirements: builder.runtime_requirements,
        panic_facts: builder.panic_facts,
        is_async: method.is_async(),
    })
}

/// Reconstruct the concrete `self` type for a method declared on `owner_name`, mirroring how
/// `check_method_with_self_ty` (`src/frontend/typechecker/check_decl.rs`) derives its own `self` binding's type:
/// a bare [`IncanType::Named`] for a non-generic owner, or an [`IncanType::Generic`] instantiated with the owner's
/// own type parameters (as type variables) for a generic owner. That typechecker-side resolved type is transient
/// checker state, not persisted anywhere in [`TypeCheckInfo`], so lowering rebuilds the equivalent type directly
/// from the AST rather than depending on a lookup table that does not exist.
fn owner_self_type(owner_name: &str, owner_type_params: &[ast::TypeParam]) -> IncanType {
    if owner_type_params.is_empty() {
        IncanType::Named(owner_name.to_string())
    } else {
        IncanType::Generic {
            base: owner_name.to_string(),
            args: owner_type_params
                .iter()
                .map(|type_param| IncanType::TypeVar(type_param.name.clone()))
                .collect(),
        }
    }
}

/// Per-function lowering state: fresh local/scope allocation, current name bindings, and accumulated body-level
/// facts (runtime requirements, panic facts, which locals have been moved out of their declaring scope).
struct BodyBuilder<'type_info, 'source> {
    type_info: &'type_info TypeCheckInfo,
    /// Source defaults for top-level partial targets, retained only until they lower into Body IR.
    function_default_sources: &'source FunctionDefaultSources,
    /// Exact declarations physically present in this module, used only to retain same-module call identities.
    local_function_declarations: &'source LocalFunctionDeclarations,
    /// Source-local plain-model declarations, used only to retain an exact constructor target identity.
    local_nominal_declarations: &'source LocalNominalDeclarations,
    /// Source-local fieldless normal-enum declarations, used only to retain exact unit-member target identities.
    local_fieldless_enum_declarations: &'source LocalFieldlessEnumDeclarations,
    /// Source-local RFC 032 value-enum declarations, used only to retain an exact member target identity.
    local_value_enum_declarations: &'source LocalValueEnumDeclarations,
    /// Owning module identity used to construct a source-span declaration identity without consulting a backend.
    module_identity: &'source str,
    /// Owning module path, used to build the RFC 120 origin of a declaration this module owns.
    module_path: &'source [String],
    /// Checked return type of the function/method currently being lowered, used only to retain `?` error routing.
    owner_return_type: IncanType,
    locals: Vec<bir::LocalDecl>,
    scopes: Vec<bir::ScopeInfo>,
    /// Current source-name -> local binding. Later bindings of the same name (new `let`/`mut` assignments) shadow
    /// earlier ones, matching the source-level scoping `BindingKind::Inferred`/`Let`/`Mutable` produce.
    bindings: HashMap<String, bir::LocalId>,
    /// Names lowering could not resolve to a tracked local (e.g. module-level `const`/`static`), reused across
    /// repeated reads instead of allocating a fresh external local per read.
    external_locals: HashMap<String, bir::LocalId>,
    /// Remaining textual reads for each tracked (non-temporary) local, seeded at declaration time by counting
    /// `Ident` occurrences of its name in the declaring scope's statement suffix (see [`count_reads_in_stmts`]).
    /// Decremented on every read; a decrement that reaches zero selects [`bir::OwnershipFact::Move`].
    remaining_reads: HashMap<bir::LocalId, usize>,
    /// Locals whose value has been moved out via a full-value (non-projected) read, so scope-exit drop insertion
    /// skips them.
    moved_out: HashSet<bir::LocalId>,
    /// Stack of the innermost-to-outermost enclosing loop's `break`-value target, pushed/popped by every loop-
    /// lowering path (`while`, `for`, and value-producing `loop` expressions) around its own body. `Some(local)`
    /// means the innermost loop is a value-producing `loop:` expression (see [`Self::lower_loop_expr`]) whose
    /// `break value` statements should assign into `local` instead of carrying the value on the `Break` statement
    /// itself; `None` means the innermost loop does not produce a value (`while`/`for`, which never legally see a
    /// `break value` today, or a `loop:` expression's own synthetic exit checks). Always non-empty while lowering
    /// any loop body, so [`Self::lower_break`] can look up the innermost target with `.last()`.
    loop_break_targets: Vec<Option<bir::LocalId>>,
    runtime_requirements: Vec<AbiV0RuntimeRequirement>,
    panic_facts: Vec<bir::PanicFact>,
    next_local: u32,
    next_scope: u32,
}

impl<'type_info, 'source> BodyBuilder<'type_info, 'source> {
    /// Start a fresh builder for one function body, with no locals, scopes, or accumulated facts yet.
    fn new(lowering_facts: &BodyIrLoweringFacts<'type_info, 'source>, owner_return_type: IncanType) -> Self {
        Self {
            type_info: lowering_facts.type_info,
            function_default_sources: lowering_facts.function_default_sources,
            local_function_declarations: lowering_facts.local_function_declarations,
            local_nominal_declarations: lowering_facts.local_nominal_declarations,
            local_fieldless_enum_declarations: lowering_facts.local_fieldless_enum_declarations,
            local_value_enum_declarations: lowering_facts.local_value_enum_declarations,
            module_identity: lowering_facts.module_identity,
            module_path: lowering_facts.module_path,
            owner_return_type,
            locals: Vec::new(),
            scopes: Vec::new(),
            bindings: HashMap::new(),
            external_locals: HashMap::new(),
            remaining_reads: HashMap::new(),
            moved_out: HashSet::new(),
            loop_break_targets: Vec::new(),
            runtime_requirements: Vec::new(),
            panic_facts: Vec::new(),
            next_local: 0,
            next_scope: 0,
        }
    }

    // ---- Scopes and locals ----

    /// Allocate a fresh lexical scope with the given `parent`, recording it in `scopes` for later span lookup.
    fn new_scope(&mut self, parent: Option<bir::ScopeId>, span: HirSourceSpan) -> bir::ScopeId {
        let id = bir::ScopeId(self.next_scope);
        self.next_scope += 1;
        self.scopes.push(bir::ScopeInfo { id, parent, span });
        id
    }

    /// Look up the source span recorded for `scope`, or a zero-width span if the id is unknown (defensive default;
    /// every scope this builder hands out is always recorded in `scopes` first).
    fn scope_span(&self, scope: bir::ScopeId) -> HirSourceSpan {
        self.scopes
            .iter()
            .find(|info| info.id == scope)
            .map(|info| info.span)
            .unwrap_or(HirSourceSpan::new(0, 0))
    }

    /// Resolve the expression type recorded by the typechecker for `span`, or [`IncanType::Unknown`] when v0 has no
    /// resolved type available (an explicit unknown rather than a guessed default).
    fn resolve_ty(&self, span: ast::Span) -> IncanType {
        self.type_info
            .expr_type(span)
            .map(semantic_type_from_resolved)
            .unwrap_or(IncanType::Unknown)
    }

    /// Declare a new user-facing local (parameter or source binding), seeding its last-use countdown from the
    /// number of `Ident` reads of `name` found in `remaining` (the declaring block's statement suffix, or a loop
    /// body for per-iteration bindings). Defaults to [`bir::LocalOrigin::UserBinding`]; callers that declare a
    /// parameter overwrite the origin afterward.
    fn declare_new_local(
        &mut self,
        name: String,
        ty: IncanType,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        remaining: &[ast::Spanned<ast::Statement>],
    ) -> bir::LocalId {
        let total_reads = count_reads_in_stmts(&name, remaining);
        self.declare_new_local_with_reads(name, ty, scope, span, total_reads)
    }

    /// Declare a new user-facing local with an already-computed last-use countdown, for declaration sites whose
    /// "remaining reads" context is not a plain statement suffix -- currently only comprehension/generator `for`
    /// clause bindings (see `Self::lower_comprehension_clauses`), whose remaining context is a tail of
    /// [`ast::ComprehensionClause`]s plus a terminal element/key/value expression, not
    /// [`ast::Statement`]s. [`Self::declare_new_local`] is a thin wrapper over this that seeds `total_reads` from a
    /// statement suffix via [`count_reads_in_stmts`].
    fn declare_new_local_with_reads(
        &mut self,
        name: String,
        ty: IncanType,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        total_reads: usize,
    ) -> bir::LocalId {
        let id = bir::LocalId(self.next_local);
        self.next_local += 1;
        self.locals.push(bir::LocalDecl {
            id,
            name: Some(name.clone()),
            ty,
            origin: bir::LocalOrigin::UserBinding,
            scope,
            span,
        });
        self.bindings.insert(name, id);
        self.remaining_reads.insert(id, total_reads);
        id
    }

    /// Declare a method's `self`/`mut self` receiver as a [`bir::LocalOrigin::Receiver`] local, bound under the
    /// name `"self"` in [`Self::bindings`] exactly like an ordinary local so [`Self::local_for_name`] resolves
    /// `self` reads without a separate lookup path.
    ///
    /// Unlike [`Self::declare_new_local`], no last-use countdown is seeded: a receiver is always a Rust-level
    /// reference (`&self`/`&mut self`), so nothing about it can be "used up" the way an owned local's remaining
    /// reads can — see the receiver carve-out in [`Self::ownership_fact_for_place`], which decides the ownership
    /// fact for every `self` read before that countdown would ever be consulted.
    fn declare_receiver_local(
        &mut self,
        ty: IncanType,
        mutable: bool,
        scope: bir::ScopeId,
        span: HirSourceSpan,
    ) -> bir::LocalId {
        let id = bir::LocalId(self.next_local);
        self.next_local += 1;
        self.locals.push(bir::LocalDecl {
            id,
            name: Some("self".to_string()),
            ty,
            origin: bir::LocalOrigin::Receiver { mutable },
            scope,
            span,
        });
        self.bindings.insert("self".to_string(), id);
        id
    }

    /// Allocate a compiler-introduced temporary. Temporaries are always consumed exactly once, immediately after
    /// creation (by construction of the flattening lowering below), so they are excluded from last-use tracking and
    /// scope-exit drop insertion — see [`Self::temp_operand`] and [`Self::insert_scope_drops`].
    fn new_temp(&mut self, ty: IncanType, scope: bir::ScopeId, span: HirSourceSpan) -> bir::LocalId {
        let id = bir::LocalId(self.next_local);
        self.next_local += 1;
        self.locals.push(bir::LocalDecl {
            id,
            name: None,
            ty,
            origin: bir::LocalOrigin::Temporary,
            scope,
            span,
        });
        id
    }

    /// Resolve a source identifier to a local, synthesizing a cached [`bir::LocalOrigin::External`] local for names
    /// v0 cannot bind (module-level `const`/`static`, or anything else lowering does not yet track) instead of
    /// panicking on an unresolved name.
    fn local_for_name(&mut self, name: &str, span: HirSourceSpan) -> bir::LocalId {
        if let Some(&id) = self.bindings.get(name) {
            return id;
        }
        if let Some(&id) = self.external_locals.get(name) {
            return id;
        }
        let id = bir::LocalId(self.next_local);
        self.next_local += 1;
        self.locals.push(bir::LocalDecl {
            id,
            name: Some(name.to_string()),
            ty: IncanType::Unknown,
            origin: bir::LocalOrigin::External,
            scope: bir::ScopeId(0),
            span,
        });
        self.external_locals.insert(name.to_string(), id);
        id
    }

    // ---- Ownership facts ----

    /// Select the Duckborrower fact and last-use marker for reading `place`.
    ///
    /// Projected reads (`.field`, `[index]`) never move: v0 does not track partial-move state, so a non-Copy
    /// projected read always borrows rather than risking an unsound move out of a place the surrounding code still
    /// owns. A bare read of a [`bir::LocalOrigin::Receiver`] local (`self`/`mut self`) never moves either, for a
    /// stronger reason than the projected case: a receiver is always a Rust-level reference at the emission
    /// boundary, so moving a non-Copy value out of it would not even compile — the only sound way to produce an
    /// owned value from it is to clone (mirrors the existing backend ownership planner's treatment of non-Copy
    /// `self` reads in `src/backend/ir/ownership.rs`, which this module's own docs cite as precedent). Every other
    /// bare local read decrements its remaining-reads countdown; reaching zero selects `Move` (and records the
    /// local as moved for [`Self::insert_scope_drops`]), otherwise `Clone`. A local with no tracked countdown (an
    /// [`bir::LocalOrigin::External`] reference) gets the explicit [`bir::OwnershipFact::Unknown`].
    ///
    /// Note that [`count_reads_in_stmts`] counts a `.field`/`[index]` occurrence of a name toward that local's
    /// total the same as a bare occurrence, but only bare reads ever decrement the countdown here. A local read
    /// only through projections therefore never reaches zero and always reads `Clone` on its final bare use rather
    /// than `Move` — an over-seeded, never-decremented countdown biases toward `Clone`, not toward an unsound
    /// `Move`, consistent with this module's documented last-use approximation.
    fn ownership_fact_for_place(&mut self, place: &bir::Place, ty: &IncanType) -> (bir::OwnershipFact, bool) {
        let is_copy = ty.abi_v0_facts().ownership.is_trivially_copy();
        if !place.projection.is_empty() {
            let fact = if is_copy {
                bir::OwnershipFact::Copy
            } else {
                bir::OwnershipFact::Borrow
            };
            return (fact, false);
        }
        if self.is_receiver_local(place.local) {
            let fact = if is_copy {
                bir::OwnershipFact::Copy
            } else {
                bir::OwnershipFact::Clone
            };
            return (fact, false);
        }
        if is_copy {
            if let Some(remaining) = self.remaining_reads.get_mut(&place.local) {
                *remaining = remaining.saturating_sub(1);
            }
            return (bir::OwnershipFact::Copy, false);
        }
        let Some(remaining) = self.remaining_reads.get_mut(&place.local) else {
            return (bir::OwnershipFact::Unknown, false);
        };
        *remaining = remaining.saturating_sub(1);
        if *remaining == 0 {
            self.moved_out.insert(place.local);
            (bir::OwnershipFact::Move, true)
        } else {
            (bir::OwnershipFact::Clone, false)
        }
    }

    /// Whether `local` is a method's `self`/`mut self` receiver, per its recorded [`bir::LocalOrigin`].
    fn is_receiver_local(&self, local: bir::LocalId) -> bool {
        self.locals
            .get(local.index())
            .is_some_and(|decl| matches!(decl.origin, bir::LocalOrigin::Receiver { .. }))
    }

    /// Build the operand for a freshly created temporary's single, immediate use.
    fn temp_operand(&self, local: bir::LocalId, ty: &IncanType) -> bir::Operand {
        let fact = if ty.abi_v0_facts().ownership.is_trivially_copy() {
            bir::OwnershipFact::Copy
        } else {
            bir::OwnershipFact::Move
        };
        bir::Operand::place(bir::Place::from_local(local), fact, true)
    }

    /// Record a runtime/helper requirement for this body, deduplicated and kept in first-seen order (see
    /// [`bir::Body::runtime_requirements`] for why lowering relies on traversal order rather than sorting).
    fn record_runtime_requirement(&mut self, requirement: AbiV0RuntimeRequirement) {
        if !self.runtime_requirements.contains(&requirement) {
            self.runtime_requirements.push(requirement);
        }
    }

    /// Emit explicit `Drop` statements, in reverse declaration order, for every non-Copy `UserBinding`/`Parameter`
    /// local declared directly in `scope` that was never moved out. This is scoped to locals declared *directly* in
    /// this block — it does not attempt cross-branch or early-return/break drop-obligation dataflow, which needs
    /// full control-flow analysis out of scope for v0 (see [`incan_semantics_core::body_ir`] module docs).
    fn insert_scope_drops(&mut self, stmts: &mut Vec<bir::Statement>, scope: bir::ScopeId) {
        let span = self.scope_span(scope);
        let candidates: Vec<bir::LocalId> = self
            .locals
            .iter()
            .rev()
            .filter(|local| local.scope == scope)
            .filter(|local| {
                matches!(
                    local.origin,
                    bir::LocalOrigin::UserBinding | bir::LocalOrigin::Parameter
                )
            })
            .filter(|local| !local.ty.abi_v0_facts().ownership.is_trivially_copy())
            .map(|local| local.id)
            .collect();
        for id in candidates {
            if self.moved_out.contains(&id) {
                continue;
            }
            stmts.push(bir::Statement {
                kind: bir::StatementKind::Drop { local: id },
                span,
            });
        }
    }

    /// Push a [`bir::StatementKind::Unsupported`] statement carrying a short diagnostic `description`, so an
    /// unmodeled source construct still leaves a total, structurally valid statement rather than being dropped.
    fn push_unsupported_stmt(&self, description: String, span: HirSourceSpan, out: &mut Vec<bir::Statement>) {
        out.push(bir::Statement {
            kind: bir::StatementKind::Unsupported { description },
            span,
        });
    }

    /// Emit an `Unsupported` marker statement and return a handle operand for it, so callers evaluating an
    /// unsupported expression in value position still get a structurally valid [`bir::Operand`] to thread onward.
    fn unsupported_operand(
        &mut self,
        description: String,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let temp = self.new_temp(IncanType::Unknown, scope, span);
        self.push_unsupported_stmt(description, span, out);
        bir::Operand::place(bir::Place::from_local(temp), bir::OwnershipFact::Unknown, true)
    }

    // ---- Rvalue / call helpers ----

    /// Allocate a fresh temporary, push an `Assign` statement giving it `rvalue`'s value, and return an operand for
    /// that temporary's single, immediate use (see [`Self::temp_operand`]). The common tail shared by every
    /// expression-lowering path that needs to flatten a computed value into a place before it can be read again.
    fn push_assign_temp(
        &mut self,
        rvalue: bir::Rvalue,
        ty: IncanType,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let temp = self.new_temp(ty.clone(), scope, span);
        out.push(bir::Statement {
            kind: bir::StatementKind::Assign {
                place: bir::Place::from_local(temp),
                rvalue,
            },
            span,
        });
        self.temp_operand(temp, &ty)
    }

    /// Allocate a fresh temporary, push a `Call` statement storing its result there, and return an operand for that
    /// temporary's single, immediate use — the call-lowering counterpart to [`Self::push_assign_temp`].
    #[allow(clippy::too_many_arguments)]
    fn push_call_temp(
        &mut self,
        callee: bir::Callee,
        args: Vec<bir::ArgumentElement>,
        ty: IncanType,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        may_panic: bool,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        let temp = self.new_temp(ty.clone(), scope, span);
        out.push(bir::Statement {
            kind: bir::StatementKind::Call {
                destination: Some(bir::Place::from_local(temp)),
                callee,
                args,
                may_panic,
            },
            span,
        });
        self.temp_operand(temp, &ty)
    }

    /// Build the boolean negation of `operand` as a fresh temporary (`not operand`), used to turn a loop's
    /// continuation condition into its complementary exit condition for the leading conditional `Break`.
    fn negate_operand(
        &mut self,
        operand: bir::Operand,
        scope: bir::ScopeId,
        span: HirSourceSpan,
        out: &mut Vec<bir::Statement>,
    ) -> bir::Operand {
        self.push_assign_temp(
            bir::Rvalue::UnaryOp(bir::UnOp::Not, operand),
            IncanType::Primitive(IncanPrimitiveType::Bool),
            scope,
            span,
            out,
        )
    }

    // ---- Statements ----
}

/// Return the first explicitly unsupported default statement, preserving the source span a direct consumer must
/// show when it refuses an omitted argument.
///
/// [`BodyBuilder::unsupported_operand`] records every unsupported expression as a
/// [`bir::StatementKind::Unsupported`] statement. Defaults can also nest executable statement sequences inside
/// control-flow, race arms, closures, generators, and match arms, so the scan walks each such sequence before the
/// deferred computation becomes callable metadata.
fn first_unsupported_default_statement(stmts: &[bir::Statement]) -> Option<(HirSourceSpan, String)> {
    stmts.iter().find_map(first_unsupported_default_statement_inner)
}

/// Inspect one statement and each rvalue shape that owns a nested executable statement sequence.
fn first_unsupported_default_statement_inner(statement: &bir::Statement) -> Option<(HirSourceSpan, String)> {
    match &statement.kind {
        bir::StatementKind::Unsupported { description } => Some((statement.span, description.clone())),
        bir::StatementKind::Assign { rvalue, .. } => first_unsupported_default_rvalue(rvalue),
        bir::StatementKind::If {
            then_block, else_block, ..
        } => first_unsupported_default_statement(&then_block.stmts).or_else(|| {
            else_block
                .as_ref()
                .and_then(|block| first_unsupported_default_statement(&block.stmts))
        }),
        bir::StatementKind::Loop { body } => first_unsupported_default_statement(&body.stmts),
        bir::StatementKind::Race { arms, .. } => arms
            .iter()
            .find_map(|arm| first_unsupported_default_statement(&arm.body.stmts)),
        _ => None,
    }
}

/// Inspect an rvalue's deferred executable parts without treating its explicit operands as source syntax to rebuild.
fn first_unsupported_default_rvalue(rvalue: &bir::Rvalue) -> Option<(HirSourceSpan, String)> {
    match rvalue {
        bir::Rvalue::Closure { body, .. } => first_unsupported_default_statement(&body.stmts),
        bir::Rvalue::Generator { body, .. } => first_unsupported_default_statement(&body.stmts),
        bir::Rvalue::Match { arms, .. } => arms.iter().find_map(|arm| {
            first_unsupported_default_statement(&arm.guard_stmts)
                .or_else(|| first_unsupported_default_statement(&arm.body_stmts))
        }),
        _ => None,
    }
}

// ============================================================================
// Comprehension desugaring helpers
// ============================================================================

/// The innermost action a list/dict-comprehension clause chain performs once every clause accepts one binding
/// combination -- what [`BodyBuilder::lower_comprehension_terminal`] lowers. It distinguishes a list's
/// single-element push from a dict's key/value insert while sharing the same clause-chain desugar.
enum ComprehensionTerminal<'a> {
    /// Push `element`'s value into the list at `list_local`.
    ListPush {
        list_local: bir::LocalId,
        element: &'a ast::Spanned<ast::Expr>,
    },
    /// Insert `key`/`value` into the dict at `dict_local`.
    DictInsert {
        dict_local: bir::LocalId,
        key: &'a ast::Spanned<ast::Expr>,
        value: &'a ast::Spanned<ast::Expr>,
    },
    /// Suspend the surrounding generator body with `element` for one accepted binding combination.
    GeneratorYield { element: &'a ast::Spanned<ast::Expr> },
}

impl ComprehensionTerminal<'_> {
    /// Count `name` occurrences in this terminal's own expression(s), for seeding a comprehension `for`-clause
    /// binding's last-use countdown (see [`BodyBuilder::declare_new_local_with_reads`]'s doc for why comprehension
    /// bindings cannot reuse the statement-suffix-based [`count_reads_in_stmts`]).
    fn count_reads(&self, name: &str) -> usize {
        match self {
            Self::ListPush { element, .. } => count_reads_in_expr(name, &element.node),
            Self::DictInsert { key, value, .. } => {
                count_reads_in_expr(name, &key.node) + count_reads_in_expr(name, &value.node)
            }
            Self::GeneratorYield { element } => count_reads_in_expr(name, &element.node),
        }
    }
}

/// Build the single mirrored `(pattern, iter, filter)` clause list a list/dict comprehension carries, as an owned
/// `Vec<ast::ComprehensionClause>` so [`BodyBuilder::lower_comprehension_clauses`] can share its
/// `&[ast::ComprehensionClause]`-based recursion with generator expressions' real multi-clause `generator.clauses`
/// without a second clause-walking implementation. See [`BodyBuilder::lower_list_comp`]'s docs for why only this
/// single mirrored clause is used, not the comprehension's own (unread-elsewhere) `clauses` field.
fn single_comprehension_clauses(
    pattern: &ast::Spanned<ast::Pattern>,
    iter: &ast::Spanned<ast::Expr>,
    filter: Option<&ast::Spanned<ast::Expr>>,
) -> Vec<ast::ComprehensionClause> {
    let mut clauses = vec![ast::ComprehensionClause::For {
        pattern: pattern.clone(),
        iter: iter.clone(),
    }];
    if let Some(filter) = filter {
        clauses.push(ast::ComprehensionClause::If(filter.clone()));
    }
    clauses
}

/// Count `name` occurrences across a tail of comprehension/generator clauses, for seeding a `for`-clause binding's
/// last-use countdown alongside [`ComprehensionTerminal::count_reads`] (see
/// [`BodyBuilder::lower_comprehension_clauses`]).
fn count_reads_in_comprehension_clauses(name: &str, clauses: &[ast::ComprehensionClause]) -> usize {
    clauses
        .iter()
        .map(|clause| match clause {
            ast::ComprehensionClause::For { iter, .. } => count_reads_in_expr(name, &iter.node),
            ast::ComprehensionClause::If(cond) => count_reads_in_expr(name, &cond.node),
        })
        .sum()
}

// ============================================================================
// Free helper functions
// ============================================================================

/// One resolved direct-call declaration narrowed to the executor-relevant facts.
///
/// `direct_call_id` is present only for a declaration physically represented by this module. Keeping the target
/// separate from its parameter slots prevents a future consumer from treating a successfully planned argument list
/// as proof that an imported callable is executable here.
///
/// `canonical` answers a deliberately different question: *which declaration* this call selected, in a form that
/// survives an import or a rename. An imported callable therefore has no `direct_call_id` but may still have a
/// canonical identity, and the two must not be read as substitutes for one another.
struct DirectCallDeclaration {
    slots: Option<Vec<DeclaredSlot>>,
    direct_call_id: Option<CompilerNodeId>,
    builtin: Option<bir::NamedCallableBuiltin>,
    canonical: Option<CanonicalSymbolId>,
}

/// One declared callable parameter or nominal field, reduced to the facts call-site binding actually needs.
///
/// Direct functions, methods, local callables, and nominal constructors each carry their declared surface in a
/// different type (`IncanCallableParam`, `symbols::CallableParam`, a field layout). Binding them through one planner
/// is what keeps #1158's "one mechanism" contract honest, so each caller narrows its own declaration surface to this
/// shape first rather than getting its own copy of the binding rules.
struct DeclaredSlot {
    /// Declared name, when the slot can be supplied by name. Positional-only slots carry `None`.
    name: Option<String>,
    /// Whether omitting this slot is legal because the declaration supplies a default.
    has_default: bool,
    /// Whether this slot holds a partial's construction-time preset, which positional binding skips.
    is_partial_preset: bool,
    /// Whether this slot is a `*args`/`**kwargs` rest parameter, which this planner refuses.
    is_rest: bool,
}

impl DeclaredSlot {
    /// Narrow a semantic callable parameter (a local callable value's signature) to its binding-relevant facts.
    fn from_semantic_param(param: &IncanCallableParam) -> Self {
        Self {
            name: param.name.clone(),
            has_default: param.has_default,
            is_partial_preset: param.is_partial_preset,
            is_rest: param.kind != IncanCallableParamKind::Normal,
        }
    }

    /// Narrow a typechecker-resolved source callable parameter to its binding-relevant facts.
    fn from_checked_param(param: &CallableParam) -> Self {
        Self {
            name: param.name.clone(),
            has_default: param.has_default,
            is_partial_preset: param.is_partial_preset,
            is_rest: param.kind != ast::ParamKind::Normal,
        }
    }
}

/// Expand a statically shaped spread argument into the ordinary arguments it stands for.
///
/// The typechecker proves a spread's shape when its operand is written as a literal whose arity is visible before
/// lowering -- `f(*(1, 2))`, `f(**{"a": 1})` -- and records the result as a
/// [`FixedUnpackPlan`](crate::frontend::typechecker::FixedUnpackPlan). Those calls have a perfectly ordinary fixed
/// arity, so they bind through the same declaration-slot planner as any other call rather than being pushed onto
/// the runtime-arity path; a `*(1, 2)` against `def add(a, b)` really is `add(1, 2)`.
///
/// Returns `None` when the spread has no proven shape, which is the ordinary case (`f(*xs)` for a list variable):
/// its arity is a runtime fact and it belongs on the unresolved-arity path. Also returns `None` when a plan exists
/// but the operand is not a destructurable literal -- the plan is recorded for tuple-*typed* operands too, and
/// those have no written elements to expand.
///
/// Parentheses are transparent here exactly as they are for the typechecker's own shape check, so the two stages
/// agree on which spellings count as shaped.
fn expand_shaped_spread(type_info: &TypeCheckInfo, arg: &ast::CallArg) -> Option<Vec<ast::CallArg>> {
    /// Look through any number of parenthesis layers to the expression they wrap.
    ///
    /// The typechecker's own shape check treats parentheses as transparent, so this must too, or the two stages
    /// would disagree about which spellings count as statically shaped.
    fn unparenthesized(expr: &ast::Spanned<ast::Expr>) -> &ast::Spanned<ast::Expr> {
        match &expr.node {
            ast::Expr::Paren(inner) => unparenthesized(inner),
            _ => expr,
        }
    }

    match arg {
        ast::CallArg::PositionalUnpack(source) => {
            if !matches!(
                type_info.fixed_unpack_plan(source.span),
                Some(FixedUnpackPlan::Positional(_))
            ) {
                return None;
            }
            match &unparenthesized(source).node {
                ast::Expr::Tuple(items) => Some(items.iter().cloned().map(ast::CallArg::Positional).collect()),
                ast::Expr::List(entries) => entries
                    .iter()
                    .map(|entry| match entry {
                        ast::ListEntry::Element(value) => Some(ast::CallArg::Positional(value.clone())),
                        ast::ListEntry::Spread(_) => None,
                    })
                    .collect(),
                _ => None,
            }
        }
        ast::CallArg::KeywordUnpack(source) => {
            if !matches!(
                type_info.fixed_unpack_plan(source.span),
                Some(FixedUnpackPlan::Keyword(_))
            ) {
                return None;
            }
            let ast::Expr::Dict(entries) = &unparenthesized(source).node else {
                return None;
            };
            entries
                .iter()
                .map(|entry| match entry {
                    ast::DictEntry::Pair(key, value) => match &unparenthesized(key).node {
                        ast::Expr::Literal(ast::Literal::String(name)) => {
                            Some(ast::CallArg::Named(name.clone(), value.clone()))
                        }
                        _ => None,
                    },
                    ast::DictEntry::Spread(_) => None,
                })
                .collect()
        }
        ast::CallArg::Positional(_) | ast::CallArg::Named(_, _) => None,
    }
}

/// Plan a call's supplied arguments into declaration slots before lowering any expression.
///
/// This validates the whole call before any *argument* ownership read is emitted, then leaves the returned
/// expressions in source evaluation order. A method call is the one exception on the callee side: its receiver is
/// read first, because source evaluation observes the receiver before the arguments, so a refusal here can follow a
/// receiver read that the refused call never consumes. The caller can therefore lower values left-to-right while the
/// final argument vector follows declaration order. Preset-default slots are intentionally omitted from positional
/// binding and may be skipped in the vector because the call's [`bir::ArgumentBinding`] records each supplied operand's
/// declaration slot; an omitted ordinary default is recorded the same way, as a defaulted slot.
///
/// `callee` is the caller's own description of the target (`function \`add\``, `local callable \`g\``,
/// `method \`add\``), so a refusal names the specific spelling that failed rather than a generic label.
fn plan_declared_args<'a>(
    callee: &str,
    params: &[DeclaredSlot],
    args: &'a [ast::CallArg],
) -> Result<Vec<(usize, &'a ast::Spanned<ast::Expr>)>, String> {
    if params.iter().any(|param| param.is_rest) {
        return Err(format!("{callee} has a rest parameter"));
    }
    let positional_slots: Vec<usize> = params
        .iter()
        .enumerate()
        .filter_map(|(index, param)| (!param.is_partial_preset).then_some(index))
        .collect();
    let mut slots: Vec<Option<&ast::Spanned<ast::Expr>>> = vec![None; params.len()];
    let mut positional_index = 0usize;
    let mut planned = Vec::with_capacity(args.len());
    for arg in args {
        let (index, expr) = match arg {
            ast::CallArg::Positional(expr) => {
                if positional_index >= positional_slots.len() {
                    return Err(format!(
                        "{callee} expects at most {} positional arguments, got {}",
                        positional_slots.len(),
                        args.len()
                    ));
                }
                let index = positional_slots[positional_index];
                positional_index += 1;
                (index, expr)
            }
            ast::CallArg::Named(arg_name, expr) => {
                let Some(index) = params
                    .iter()
                    .position(|param| param.name.as_deref() == Some(arg_name.as_str()))
                else {
                    return Err(format!("{callee} has no parameter `{arg_name}`"));
                };
                (index, expr)
            }
            ast::CallArg::PositionalUnpack(_) => {
                return Err(format!("{callee} called with a positional argument spread"));
            }
            ast::CallArg::KeywordUnpack(_) => {
                return Err(format!("{callee} called with a keyword argument spread"));
            }
        };
        if slots[index].is_some() {
            let parameter = params[index].name.as_deref().unwrap_or("<unnamed>");
            return Err(format!("{callee} receives `{parameter}` more than once"));
        }
        slots[index] = Some(expr);
        planned.push((index, expr));
    }

    let required = params.iter().filter(|param| !param.has_default).count();
    if let Some((_index, parameter)) = params
        .iter()
        .enumerate()
        .find(|(index, parameter)| slots[*index].is_none() && !parameter.has_default)
    {
        return Err(format!(
            "{callee} expects at least {required} required arguments, got {}; missing required parameter `{}`",
            args.len(),
            parameter.name.as_deref().unwrap_or("<unnamed>")
        ));
    }
    // An omitted interior default needs no refusal any more. #1124 had to reject one because a flat operand vector
    // could not say which slot a later operand filled; `bir::ArgumentBinding` now records exactly that, so a sparse
    // call is representable rather than ambiguous.
    Ok(planned)
}

/// Wrap fixed operands as single-value element list entries.
///
/// Used by every lowering path that produces a known number of values -- the overwhelming majority. Only a source
/// spread produces a [`bir::ArgumentElement::Spread`], so this keeps those call sites reading as they did before
/// element lists became variable-arity.
fn fixed_elements(operands: Vec<bir::Operand>) -> Vec<bir::ArgumentElement> {
    operands.into_iter().map(bir::ArgumentElement::One).collect()
}

/// Whether a type is string-like enough to route binary operators through the compiler-owned string helpers
/// (mirrors `is_string_like_type` in `src/backend/ir/conversions.rs`, restated here so Body IR does not depend on
/// that Rust-emission-specific module — see this file's module docs).
fn is_string_like(ty: &IncanType) -> bool {
    matches!(
        ty,
        IncanType::Primitive(IncanPrimitiveType::Str | IncanPrimitiveType::FrozenStr)
    )
}

/// Map a string-typed binary operator to its compiler-owned helper operation, or `None` for operators that have no
/// string-specific helper (arithmetic-only operators never reach here because `lower_binary` only checks this for
/// string-like operand types).
fn string_helper_for_binop(op: ast::BinaryOp) -> Option<bir::HelperOp> {
    match op {
        ast::BinaryOp::Add => Some(bir::HelperOp::StrConcat),
        ast::BinaryOp::Eq => Some(bir::HelperOp::StrEq),
        ast::BinaryOp::NotEq => Some(bir::HelperOp::StrNe),
        ast::BinaryOp::Lt => Some(bir::HelperOp::StrLt),
        ast::BinaryOp::LtEq => Some(bir::HelperOp::StrLe),
        ast::BinaryOp::Gt => Some(bir::HelperOp::StrGt),
        ast::BinaryOp::GtEq => Some(bir::HelperOp::StrGe),
        _ => None,
    }
}

/// Map a surface binary operator to Body IR's canonical arithmetic/comparison/boolean operator set, or `None` for
/// operators v0 does not model.
///
/// The unmapped set is `Pow`, `MatMul`, both pipes, the bitwise `BitAnd`/`BitOr`/`BitXor`, the `Shl`/`Shr` shifts,
/// `In`/`NotIn`, and `Is`/`IsNot`. Membership is the notable one: `parity-987-0003` records string `in` as a
/// `Preserved` behavior, so refusing it here is a tracked #1101 gap rather than a settled boundary. Adding any of
/// these needs a matching [`bir::BinOp`] variant, or a compiler-owned [`bir::HelperOp`] where the operation is a
/// runtime call rather than a primitive -- the same split [`string_helper_for_binop`] already makes.
fn lower_binary_op(op: ast::BinaryOp) -> Option<bir::BinOp> {
    match op {
        ast::BinaryOp::Add => Some(bir::BinOp::Add),
        ast::BinaryOp::Sub => Some(bir::BinOp::Sub),
        ast::BinaryOp::Mul => Some(bir::BinOp::Mul),
        ast::BinaryOp::Div => Some(bir::BinOp::Div),
        ast::BinaryOp::FloorDiv => Some(bir::BinOp::FloorDiv),
        ast::BinaryOp::Mod => Some(bir::BinOp::Mod),
        ast::BinaryOp::Eq => Some(bir::BinOp::Eq),
        ast::BinaryOp::NotEq => Some(bir::BinOp::Ne),
        ast::BinaryOp::Lt => Some(bir::BinOp::Lt),
        ast::BinaryOp::LtEq => Some(bir::BinOp::Le),
        ast::BinaryOp::Gt => Some(bir::BinOp::Gt),
        ast::BinaryOp::GtEq => Some(bir::BinOp::Ge),
        ast::BinaryOp::And => Some(bir::BinOp::And),
        ast::BinaryOp::Or => Some(bir::BinOp::Or),
        _ => None,
    }
}

/// Map a surface unary operator to Body IR's unary operator set. Exhaustive: all three surface unary operators have
/// a direct Body IR equivalent.
const fn lower_unary_op(op: ast::UnaryOp) -> bir::UnOp {
    match op {
        ast::UnaryOp::Neg => bir::UnOp::Neg,
        ast::UnaryOp::Not => bir::UnOp::Not,
        ast::UnaryOp::Invert => bir::UnOp::Invert,
    }
}

/// Lower a literal to a Body IR constant, or `None` for literal kinds v0 does not model distinctly (`bytes`).
fn lower_literal(lit: &ast::Literal) -> Option<bir::Constant> {
    match lit {
        ast::Literal::Int(int_lit) => Some(bir::Constant::Int(int_lit.value)),
        ast::Literal::Float(float_lit) => Some(bir::Constant::Float(float_lit.repr.clone())),
        ast::Literal::Decimal(decimal_lit) => Some(bir::Constant::Float(decimal_lit.repr.clone())),
        ast::Literal::String(s) => Some(bir::Constant::Str(s.clone())),
        ast::Literal::Bool(b) => Some(bir::Constant::Bool(*b)),
        ast::Literal::None => Some(bir::Constant::None),
        ast::Literal::Bytes(_) => None,
    }
}

/// Short diagnostic label for a statement kind v0 does not lower.
///
/// Statement-position `loop:` is named explicitly because it is the one entry here whose Body IR vocabulary
/// already exists: [`BodyBuilder::lower_loop_expr`] emits [`bir::StatementKind::Loop`] for the expression
/// spelling, and only [`BodyBuilder::lower_stmt_into`]'s dispatch is missing (#1101). Leaving it under the
/// generic "statement" label made a five-line dispatch gap read like an unmodeled construct.
fn unsupported_stmt_label(stmt: &ast::Statement) -> String {
    match stmt {
        ast::Statement::Loop(_) => "statement-position `loop:`".to_string(),
        ast::Statement::Unsafe(_) => "unsafe block".to_string(),
        ast::Statement::VocabExpressionItem(_) => "vocab expression item".to_string(),
        ast::Statement::Surface(_) => "surface statement".to_string(),
        ast::Statement::VocabBlock(_) => "vocab block".to_string(),
        _ => "statement".to_string(),
    }
}

/// Short diagnostic label for an expression kind v0 does not lower.
///
/// Only reached from [`BodyBuilder::lower_expr_to_operand`]'s fallback arm, so every expression kind that arm
/// dispatches by name -- closures and partial callables included, since #1124 gave both a real lowering -- is
/// deliberately absent here. Async surface (`await`, `race for`) and vocab/scoped-DSL surface are named rather
/// than left to the generic label, because both are tracked remaining work under #1101 (#1164 and #1166
/// respectively) and a diagnostic reading only "expression" hides which one a program actually hit.
fn unsupported_expr_label(expr: &ast::Expr) -> String {
    match expr {
        ast::Expr::Yield(_) => "yield expression".to_string(),
        ast::Expr::Range { .. } => "range expression outside a for-loop".to_string(),
        ast::Expr::Surface(surface) => surface_expr_label(&surface.payload),
        ast::Expr::VocabBlock(_) => "vocab block expression".to_string(),
        _ => "expression".to_string(),
    }
}

/// Name the specific surface-expression payload behind an [`ast::Expr::Surface`] refusal.
///
/// The payloads split into two very different buckets: `await`/`race for` are the async surface #1164 represents,
/// which #1155 needs before it can execute task state, while the remaining payloads are vocab/DSL nodes that the
/// legacy pipeline desugars away before lowering and that only reach here when a caller skips that pass (#1166).
fn surface_expr_label(payload: &ast::SurfaceExprPayload) -> String {
    match payload {
        ast::SurfaceExprPayload::PrefixUnary(_) => {
            "prefix-keyword surface expression (for example `await`)".to_string()
        }
        ast::SurfaceExprPayload::RaceFor(_) => "`race for` expression".to_string(),
        ast::SurfaceExprPayload::LeadingDotPath { .. } => "scoped DSL leading-dot path".to_string(),
        ast::SurfaceExprPayload::ScopedGlyph { .. } => "scoped DSL glyph operator".to_string(),
        ast::SurfaceExprPayload::ScopedSymbolCall { .. } => "scoped DSL symbol call".to_string(),
    }
}

/// Resolve the per-element types for a tuple-typed value being destructured into `count` targets, falling back to
/// [`IncanType::Unknown`] per element when the resolved type is not (or not yet) known to be a tuple of the right
/// arity -- mirrors how the existing Rust-emission backend falls back to `IrType::Unknown` per slot in the same
/// situation (`src/backend/ir/lower/stmt.rs`'s `TupleUnpack` lowering). Used by
/// [`BodyBuilder::lower_tuple_unpack`], [`BodyBuilder::lower_tuple_assign`], and
/// [`BodyBuilder::bind_for_pattern_fields`].
///
/// A tuple type reaches lowering in two spellings and both must be understood here. A tuple *literal* resolves to
/// [`IncanType::Tuple`], while a written `tuple[A, B]` *annotation* resolves through the collection-type registry
/// and therefore arrives as an [`IncanType::Generic`] whose base is that registry's canonical name. Matching only
/// the first spelling silently degraded every element of an annotated tuple to `Unknown`, which in turn made each
/// element read `Borrow` rather than its real Copy/non-Copy fact. The generic base is classified through
/// [`collections::from_str`] rather than compared against a literal name, so the registry stays the single source
/// of truth for that vocabulary.
/// Why a statement-level destructure of `value_ty` into `arity` names cannot be lowered, or `None` when it can.
///
/// The statement sibling of [`unsupported_for_pattern`], and it exempts the same two types for the same reason:
/// `Unknown` and `Never` mean the typechecker either already reported a failure or is looking at unreachable code,
/// so lowering has nothing to refuse. Everything else — including Rust interop, which is checked against the same
/// [`rust_tuple_arity`] rule the typechecker uses rather than waved through — must be a tuple of exactly matching
/// arity before lowering may emit a `.0`/`.1` field projection. Without this, a non-tuple value produced
/// `__incan_tuple_unpack_*.0` against a fieldless value and surfaced as a raw `rustc` E0610 (#1132).
fn unsupported_tuple_destructure(value_ty: &IncanType, arity: usize) -> Option<String> {
    if matches!(value_ty, IncanType::Unknown | IncanType::Never) {
        return None;
    }
    // Interop values go through the same accepted-shape rule the typechecker uses, not an exemption: a readable
    // tuple spelling lowers, and anything opaque refuses. Waving every `RustInteropPath` through would have let a
    // genuine non-tuple Rust value reach a `.0`/`.1` projection, which is the leakage #1132 closes.
    if let IncanType::RustInteropPath(path) = value_ty {
        return match rust_tuple_arity(path) {
            Some(rust_arity) if rust_arity == arity => None,
            Some(rust_arity) => Some(format!(
                "tuple destructure binds {arity} names but Rust value type `{path}` has {rust_arity} elements"
            )),
            None => Some(format!(
                "tuple destructure of Rust value type `{path}` whose tuple shape cannot be verified"
            )),
        };
    }
    let Some(element_types) = tuple_type_elements(value_ty) else {
        return Some(format!("tuple destructure of non-tuple value type `{value_ty}`"));
    };
    if element_types.len() != arity {
        return Some(format!(
            "tuple destructure binds {arity} names but value type `{value_ty}` has {} elements",
            element_types.len()
        ));
    }
    None
}

fn tuple_element_types(ty: &IncanType, count: usize) -> Vec<IncanType> {
    match tuple_type_elements(ty) {
        Some(items) if items.len() == count => items.to_vec(),
        _ => vec![IncanType::Unknown; count],
    }
}

/// The element types of a tuple-shaped [`IncanType`], in either spelling, or `None` when `ty` is not a tuple at
/// all. Backs both [`tuple_element_types`] and [`unsupported_for_pattern`], so the "is this a tuple, and of what
/// arity" question is answered in exactly one place rather than once per caller.
fn tuple_type_elements(ty: &IncanType) -> Option<&[IncanType]> {
    match ty {
        IncanType::Tuple(items) => Some(items),
        IncanType::Generic { base, args } if collections::from_str(base) == Some(CollectionTypeId::Tuple) => Some(args),
        _ => None,
    }
}

/// Count textual `Ident(name)` occurrences reachable from `stmts`, restricted to the same statement/expression
/// subset [`BodyBuilder`] actually lowers. This seeds a local's last-use countdown (see
/// [`BodyBuilder::declare_new_local`]).
///
/// This is a **textual, source-order over-approximation**, not dynamic dataflow: it does not special-case shadowing
/// (a later redeclaration of the same name still contributes to this count) and it counts occurrences across all
/// branches of a conditional rather than only the branch that will execute. Both simplifications only ever make the
/// count too high, which biases the resulting ownership fact toward `Clone`/`Borrow` instead of `Move` — never the
/// reverse — so it cannot produce an unsound move.
fn count_reads_in_stmts(name: &str, stmts: &[ast::Spanned<ast::Statement>]) -> usize {
    stmts.iter().map(|stmt| count_reads_in_stmt(name, &stmt.node)).sum()
}

/// Count `name` occurrences reachable from one statement, recursing into every branch of a conditional/loop rather
/// than only the branch that will execute — part of [`count_reads_in_stmts`]'s documented over-approximation.
/// Statement kinds outside v0's lowered subset are not walked and contribute zero (they cannot themselves bind or
/// read `name` in a way v0's lowering will ever observe).
fn count_reads_in_stmt(name: &str, stmt: &ast::Statement) -> usize {
    match stmt {
        ast::Statement::Assignment(a) => count_reads_in_expr(name, &a.value.node),
        ast::Statement::FieldAssignment(fa) => {
            count_reads_in_expr(name, &fa.object.node) + count_reads_in_expr(name, &fa.value.node)
        }
        ast::Statement::IndexAssignment(ia) => {
            count_reads_in_expr(name, &ia.object.node)
                + count_reads_in_expr(name, &ia.index.node)
                + count_reads_in_expr(name, &ia.value.node)
        }
        ast::Statement::CompoundAssignment(ca) => {
            usize::from(ca.name == name) + count_reads_in_expr(name, &ca.value.node)
        }
        ast::Statement::TupleUnpack(tu) => count_reads_in_expr(name, &tu.value.node),
        ast::Statement::TupleAssign(ta) => {
            ta.targets
                .iter()
                .map(|t| count_reads_in_expr(name, &t.node))
                .sum::<usize>()
                + count_reads_in_expr(name, &ta.value.node)
        }
        ast::Statement::ChainedAssignment(ca) => count_reads_in_expr(name, &ca.value.node),
        ast::Statement::Return(Some(e)) => count_reads_in_expr(name, &e.node),
        ast::Statement::Return(None) => 0,
        ast::Statement::If(if_stmt) => {
            let mut total = count_reads_in_condition(name, &if_stmt.condition);
            total += count_reads_in_stmts(name, &if_stmt.then_body);
            for (cond, body) in &if_stmt.elif_branches {
                total += count_reads_in_expr(name, &cond.node);
                total += count_reads_in_stmts(name, body);
            }
            if let Some(else_body) = &if_stmt.else_body {
                total += count_reads_in_stmts(name, else_body);
            }
            total
        }
        ast::Statement::While(w) => count_reads_in_condition(name, &w.condition) + count_reads_in_stmts(name, &w.body),
        ast::Statement::For(f) => count_reads_in_expr(name, &f.iter.node) + count_reads_in_stmts(name, &f.body),
        ast::Statement::Expr(e) => count_reads_in_expr(name, &e.node),
        ast::Statement::Assert(a) => {
            let mut total = match &a.kind {
                ast::AssertKind::Condition(e) => count_reads_in_expr(name, &e.node),
                _ => 0,
            };
            total += a
                .message
                .as_ref()
                .map(|m| count_reads_in_expr(name, &m.node))
                .unwrap_or(0);
            total
        }
        ast::Statement::Break(Some(e)) => count_reads_in_expr(name, &e.node),
        _ => 0,
    }
}

/// Count `name` occurrences in an `if`/`while` condition, including the value expression of a `Condition::Let`
/// pattern condition (even though v0 lowering does not model `if let`/`while let` themselves — see
/// [`BodyBuilder::lower_if`]/[`BodyBuilder::lower_while`] — so the read-count approximation stays an
/// over-approximation rather than silently under-counting).
fn count_reads_in_condition(name: &str, cond: &ast::Condition) -> usize {
    match cond {
        ast::Condition::Expr(e) => count_reads_in_expr(name, &e.node),
        ast::Condition::Let { value, .. } => count_reads_in_expr(name, &value.node),
    }
}

/// Count `name` occurrences reachable from one expression, recursing into every expression kind v0's lowering
/// itself walks (see this module's module-level docs for the covered subset). Expression kinds outside that subset
/// contribute zero, consistent with [`count_reads_in_stmts`]'s "restricted to the same subset `BodyBuilder` actually
/// lowers" scope.
fn count_reads_in_expr(name: &str, expr: &ast::Expr) -> usize {
    match expr {
        ast::Expr::Ident(id) => usize::from(id == name),
        ast::Expr::Binary(l, _, r) => count_reads_in_expr(name, &l.node) + count_reads_in_expr(name, &r.node),
        ast::Expr::Unary(_, e) => count_reads_in_expr(name, &e.node),
        ast::Expr::Call(callee, _, args) => {
            count_reads_in_expr(name, &callee.node)
                + args.iter().map(|a| count_reads_in_call_arg(name, a)).sum::<usize>()
        }
        ast::Expr::MethodCall(recv, _, _, args) => {
            count_reads_in_expr(name, &recv.node) + args.iter().map(|a| count_reads_in_call_arg(name, a)).sum::<usize>()
        }
        ast::Expr::Field(e, _) => count_reads_in_expr(name, &e.node),
        ast::Expr::Index(e, idx) => count_reads_in_expr(name, &e.node) + count_reads_in_expr(name, &idx.node),
        ast::Expr::Slice(base, slice) => {
            count_reads_in_expr(name, &base.node)
                + slice
                    .start
                    .as_ref()
                    .map(|e| count_reads_in_expr(name, &e.node))
                    .unwrap_or(0)
                + slice
                    .end
                    .as_ref()
                    .map(|e| count_reads_in_expr(name, &e.node))
                    .unwrap_or(0)
                + slice
                    .step
                    .as_ref()
                    .map(|e| count_reads_in_expr(name, &e.node))
                    .unwrap_or(0)
        }
        ast::Expr::Paren(e) | ast::Expr::Try(e) => count_reads_in_expr(name, &e.node),
        ast::Expr::Tuple(items) | ast::Expr::Set(items) => {
            items.iter().map(|i| count_reads_in_expr(name, &i.node)).sum()
        }
        ast::Expr::List(entries) => entries
            .iter()
            .map(|entry| match entry {
                ast::ListEntry::Element(e) | ast::ListEntry::Spread(e) => count_reads_in_expr(name, &e.node),
            })
            .sum(),
        ast::Expr::Dict(entries) => entries
            .iter()
            .map(|entry| match entry {
                ast::DictEntry::Pair(k, v) => count_reads_in_expr(name, &k.node) + count_reads_in_expr(name, &v.node),
                ast::DictEntry::Spread(e) => count_reads_in_expr(name, &e.node),
            })
            .sum(),
        ast::Expr::Constructor(_, args) => args.iter().map(|a| count_reads_in_call_arg(name, a)).sum(),
        ast::Expr::Range { start, end, .. } => {
            count_reads_in_expr(name, &start.node) + count_reads_in_expr(name, &end.node)
        }
        ast::Expr::If(if_expr) => {
            count_reads_in_expr(name, &if_expr.condition.node)
                + count_reads_in_stmts(name, &if_expr.then_body)
                + if_expr
                    .else_body
                    .as_ref()
                    .map(|body| count_reads_in_stmts(name, body))
                    .unwrap_or(0)
        }
        ast::Expr::Loop(loop_expr) => count_reads_in_stmts(name, &loop_expr.body),
        ast::Expr::FString(parts) => parts
            .iter()
            .map(|part| match part {
                ast::FStringPart::Literal(_) => 0,
                ast::FStringPart::Expr { expr, .. } => count_reads_in_expr(name, &expr.node),
            })
            .sum(),
        ast::Expr::ListComp(comp) => {
            count_reads_in_expr(name, &comp.iter.node)
                + comp
                    .filter
                    .as_ref()
                    .map(|f| count_reads_in_expr(name, &f.node))
                    .unwrap_or(0)
                + count_reads_in_expr(name, &comp.expr.node)
        }
        ast::Expr::DictComp(comp) => {
            count_reads_in_expr(name, &comp.iter.node)
                + comp
                    .filter
                    .as_ref()
                    .map(|f| count_reads_in_expr(name, &f.node))
                    .unwrap_or(0)
                + count_reads_in_expr(name, &comp.key.node)
                + count_reads_in_expr(name, &comp.value.node)
        }
        ast::Expr::Generator(generator) => {
            count_reads_in_comprehension_clauses(name, &generator.clauses)
                + count_reads_in_expr(name, &generator.expr.node)
        }
        ast::Expr::Closure(params, body) => {
            // `BodyBuilder::lower_closure` reads a captured free variable exactly once at the closure-creation
            // site, however many times the closure body itself uses it afterward (subsequent uses read the
            // closure's own captured-binding local, not the outer one this count seeds) -- so this contributes at
            // most 1, not the raw in-body occurrence count. A name shadowed by the closure's own parameter is never
            // captured at all and so contributes 0, regardless of how many times the body uses its own parameter.
            if params.iter().any(|p| p.node.name == name) {
                0
            } else {
                usize::from(count_reads_in_expr(name, &body.node) > 0)
            }
        }
        ast::Expr::Partial(partial) => {
            // Unlike a closure's captures, a partial callable's preset values are lowered as ordinary sub-expression
            // reads (see `BodyBuilder::lower_partial`), not deduplicated per free-variable name, so this counts them
            // plainly like any other nested expression.
            count_reads_in_expr(name, &partial.target.node)
                + partial
                    .args
                    .iter()
                    .map(|a| count_reads_in_expr(name, &a.value.node))
                    .sum::<usize>()
        }
        // `BodyBuilder::lower_yield` lowers a yielded value through the same `lower_expr_to_operand` path as any
        // other statement's operand, so a name read inside `yield value` must be counted here too -- otherwise it
        // would be undercounted for last-use purposes, the same soundness gap #1101's f-string bucket found and
        // fixed for `count_reads_in_expr`'s `FString` arm.
        ast::Expr::Yield(value) => value.as_ref().map_or(0, |v| count_reads_in_expr(name, &v.node)),
        // Same soundness class as the `Yield`/`FString` arms above: a `match` scrutinee, guard, or arm body is
        // lowered through the ordinary expression/statement paths (`BodyBuilder::lower_match`), so a read of `name`
        // reachable inside any of them must be counted here too. Unlike `collect_free_vars_in_expr`'s `Match` arm,
        // this does not need to exclude an arm's own pattern-bound names from the count: this function is a coarse,
        // source-order over-approximation by design (see its own docs), and over-counting only ever biases the
        // resulting ownership fact toward `Clone`/`Borrow` rather than `Move` -- never unsound.
        ast::Expr::Match(subject, arms) => {
            count_reads_in_expr(name, &subject.node)
                + arms
                    .iter()
                    .map(|arm| count_reads_in_match_arm(name, &arm.node))
                    .sum::<usize>()
        }
        _ => 0,
    }
}

/// Count `name` occurrences reachable from one `match` arm's guard and body, for seeding a pattern-bound local's
/// last-use countdown the same way [`count_reads_in_stmts`] seeds an ordinary binding's -- see
/// [`BodyBuilder::lower_match_pattern`]. Also reused by [`count_reads_in_expr`]'s own `Match` arm so both counting
/// paths agree on what "a read inside this arm" means.
fn count_reads_in_match_arm(name: &str, arm: &ast::MatchArm) -> usize {
    let guard_reads = arm.guard.as_ref().map_or(0, |g| count_reads_in_expr(name, &g.node));
    let body_reads = match &arm.body {
        ast::MatchBody::Expr(e) => count_reads_in_expr(name, &e.node),
        ast::MatchBody::Block(stmts) => count_reads_in_stmts(name, stmts),
    };
    guard_reads + body_reads
}

/// Whether `pattern` is representable by [`bir::Pattern`]'s closed vocabulary. The only unrepresentable shape is a
/// byte-string literal pattern ([`bir::Constant`] has no byte-string variant -- see [`lower_literal`]'s own `None`
/// case for the identical gap in plain literal *expressions*); every other pattern shape lowers structurally, with
/// [`IncanType::Unknown`] field-type fallbacks where needed rather than an outright failure (see
/// [`BodyBuilder::lower_match_pattern`]'s own docs). Checked for every arm before [`BodyBuilder::lower_match`]
/// lowers any of them, mirroring [`BodyBuilder::binary_op_is_supported`]'s "check before partially lowering"
/// precedent.
fn match_pattern_is_supported(pattern: &ast::Pattern) -> bool {
    match pattern {
        ast::Pattern::Literal(ast::Literal::Bytes(_)) => false,
        ast::Pattern::Literal(_) | ast::Pattern::Wildcard | ast::Pattern::Binding(_) => true,
        ast::Pattern::Tuple(items) => items.iter().all(|item| match_pattern_is_supported(&item.node)),
        ast::Pattern::Constructor(_, args) => args.iter().all(|arg| match arg {
            ast::PatternArg::Positional(pat) | ast::PatternArg::Named(_, pat) => match_pattern_is_supported(&pat.node),
        }),
        ast::Pattern::Group(inner) => match_pattern_is_supported(&inner.node),
        ast::Pattern::Or(items) => items.iter().all(|item| match_pattern_is_supported(&item.node)),
    }
}

/// Name the reason Body IR cannot bind `pattern` against a produced item of type `item_ty`, or `None` when it
/// can. Consulted once, up front, so a refusal never leaves half-emitted bindings behind -- the same precedent as
/// [`match_pattern_is_supported`].
///
/// Two independent things can make a loop pattern unbindable, and both are checked here.
///
/// **Shape.** The accepted subset is deliberately the same one `TypeChecker::define_for_pattern_bindings`
/// (`src/frontend/typechecker/check_stmt.rs`) accepts -- a plain binding, `_`, and recursively a tuple of those
/// (#1125). Naming the offending shape keeps a hand-built AST that bypassed the typechecker diagnosable.
///
/// **Type agreement.** A tuple pattern can only take elements from a tuple. Without this check, `for a, b in
/// items` over a `list[int]` would lower `.0`/`.1` projections out of an `int` -- structurally valid Body IR
/// describing something that does not exist. The typechecker rejects that program first, so this is defence in
/// depth for hand-built ASTs and for lowering that runs despite type errors, not the primary diagnostic.
///
/// Two item types are exempt from the tuple requirement, mirroring `TypeChecker::define_for_pattern_bindings`
/// exactly so the two stages cannot disagree about which programs are bindable.
/// [`IncanType::Unknown`] is recovery-only: it means the type is unresolved, not proven non-tuple, so each element
/// binds as `Unknown` just as [`tuple_element_types`] already falls back to. [`IncanType::Never`] is the bottom
/// type, which the typechecker's own `types_compatible` treats as compatible with every type including a tuple.
///
/// A bare [`IncanType::TypeVar`] is deliberately **not** exempt. An unconstrained `T` is known to be
/// underdetermined rather than merely unknown, and can be instantiated as `int`; Incan has no tuple-shaped bound
/// that could promise otherwise. This does not affect the common `list[Tuple[K, V]]` shape, whose item type is a
/// tuple whose *elements* are type variables.
fn unsupported_for_pattern(pattern: &ast::Pattern, item_ty: &IncanType) -> Option<String> {
    match pattern {
        ast::Pattern::Binding(_) | ast::Pattern::Wildcard => None,
        ast::Pattern::Tuple(items) => {
            if matches!(item_ty, IncanType::Unknown | IncanType::Never) {
                return items
                    .iter()
                    .find_map(|item| unsupported_for_pattern(&item.node, &IncanType::Unknown));
            }
            let Some(element_types) = tuple_type_elements(item_ty) else {
                return Some(format!("for-loop tuple pattern over non-tuple item type `{item_ty}`"));
            };
            if element_types.len() != items.len() {
                return Some(format!(
                    "for-loop tuple pattern binds {} names but item type `{item_ty}` has {} elements",
                    items.len(),
                    element_types.len()
                ));
            }
            items
                .iter()
                .zip(element_types)
                .find_map(|(item, element_ty)| unsupported_for_pattern(&item.node, element_ty))
        }
        ast::Pattern::Literal(_) => Some("for-loop pattern shape: literal".to_string()),
        ast::Pattern::Constructor(..) => Some("for-loop pattern shape: constructor".to_string()),
        ast::Pattern::Group(_) => Some("for-loop pattern shape: parenthesized group".to_string()),
        ast::Pattern::Or(_) => Some("for-loop pattern shape: alternation".to_string()),
    }
}

/// Determine every lexical free variable used after a generator expression's first source, in first-occurrence
/// order. The initial source is evaluated before constructing [`bir::Rvalue::Generator`] and therefore is not a
/// deferred capture. Each `for` pattern becomes bound only after its own source expression has been visited, so a
/// later source/filter/element sees every preceding clause binding but not a name it introduces itself.
fn free_vars_in_generator_deferred_body(generator: &ast::GeneratorExpr) -> Vec<String> {
    let Some((first, remaining)) = generator.clauses.split_first() else {
        return Vec::new();
    };
    let mut bound = HashSet::new();
    if let ast::ComprehensionClause::For { pattern, .. } = first {
        bind_pattern_names(&pattern.node, &mut bound);
    }
    let mut free = Vec::new();
    for clause in remaining {
        match clause {
            ast::ComprehensionClause::For { pattern, iter } => {
                collect_free_vars_in_expr(&iter.node, &mut bound, &mut free);
                bind_pattern_names(&pattern.node, &mut bound);
            }
            ast::ComprehensionClause::If(condition) => {
                collect_free_vars_in_expr(&condition.node, &mut bound, &mut free);
            }
        }
    }
    collect_free_vars_in_expr(&generator.expr.node, &mut bound, &mut free);
    free
}

/// Count a generator capture's deferred reads after the first source. This intentionally remains a conservative
/// source-order over-approximation, like [`count_reads_in_expr`]: a later pattern can shadow the same spelling and
/// leave this count high, which selects a clone rather than an unsound move in the generator-local body.
fn count_reads_in_generator_deferred_body(name: &str, generator: &ast::GeneratorExpr) -> usize {
    let Some((_, remaining)) = generator.clauses.split_first() else {
        return 0;
    };
    count_reads_in_comprehension_clauses(name, remaining) + count_reads_in_expr(name, &generator.expr.node)
}

/// Determine every free variable a closure literal's body reads from its enclosing scope, in first-occurrence
/// source order, given the closure's own declared parameters as the initial bound set. A "free variable" is any
/// `Ident` read the closure body itself does not bind -- exactly the set [`BodyBuilder::lower_closure`] must
/// capture before lowering the body, so each one gets its own explicit Duckborrower read at the point the closure
/// is constructed (see this module's docs on why Body IR cannot rely on a target backend's own closure syntax to
/// auto-capture the way the existing Rust-emission backend does).
fn free_vars_in_closure_body(params: &[ast::Spanned<ast::Param>], body: &ast::Spanned<ast::Expr>) -> Vec<String> {
    let mut bound: HashSet<String> = params.iter().map(|p| p.node.name.clone()).collect();
    let mut free = Vec::new();
    collect_free_vars_in_expr(&body.node, &mut bound, &mut free);
    free
}

/// Record `name` in `free` (in first-occurrence order, deduplicated) unless it is already in `bound`.
fn push_free(name: &str, bound: &HashSet<String>, free: &mut Vec<String>) {
    if !bound.contains(name) && !free.iter().any(|existing| existing == name) {
        free.push(name.to_string());
    }
}

/// Collect every name `pattern` binds into `bound`, recursing into every sub-pattern shape.
///
/// Used by [`collect_free_vars_in_expr`] to exclude a pattern's own bound names from the free variables an
/// enclosing closure must capture, for every construct that binds through a pattern: `match` arms, `for` loops,
/// comprehension/generator `for` clauses, and `if let`/`while let` conditions. A single recursive walk serves all
/// of them because a `for` pattern can now bind more than one name too (#1125) -- a flat "only a plain
/// [`ast::Pattern::Binding`] binds here" walk would leave a destructured loop binding looking free, and an
/// enclosing closure would wrongly capture it.
///
/// This mirrors [`BodyBuilder::lower_match_pattern`]'s and [`BodyBuilder::bind_for_pattern_fields`]' binding walks
/// in spirit, though it only needs the names, not the locals/ownership facts those walks build.
fn bind_pattern_names(pattern: &ast::Pattern, bound: &mut HashSet<String>) {
    match pattern {
        ast::Pattern::Wildcard | ast::Pattern::Literal(_) => {}
        ast::Pattern::Binding(name) => {
            bound.insert(name.clone());
        }
        ast::Pattern::Tuple(items) => {
            for item in items {
                bind_pattern_names(&item.node, bound);
            }
        }
        ast::Pattern::Constructor(_, args) => {
            for arg in args {
                match arg {
                    ast::PatternArg::Positional(pat) | ast::PatternArg::Named(_, pat) => {
                        bind_pattern_names(&pat.node, bound);
                    }
                }
            }
        }
        ast::Pattern::Group(inner) => bind_pattern_names(&inner.node, bound),
        ast::Pattern::Or(items) => {
            for item in items {
                bind_pattern_names(&item.node, bound);
            }
        }
    }
}

/// Recursively collect free variables from an expression, given the names already bound at this point in `bound`.
/// Constructs that introduce their own bindings for a sub-expression (comprehension/`for`-clause patterns, nested
/// closures' own parameters, or a nested expression-position `if`/`loop`'s own statement-block bindings) extend a
/// *cloned* copy of `bound` before recursing into that sub-expression, so a binding introduced in one branch never
/// leaks into a sibling branch or back out to the caller -- unlike [`BodyBuilder`]'s own flat `self.bindings` map,
/// which this analysis runs entirely independently of (see [`free_vars_in_closure_body`]'s docs).
fn collect_free_vars_in_expr(expr: &ast::Expr, bound: &mut HashSet<String>, free: &mut Vec<String>) {
    match expr {
        ast::Expr::Ident(name) => push_free(name, bound, free),
        ast::Expr::Binary(l, _, r) => {
            collect_free_vars_in_expr(&l.node, bound, free);
            collect_free_vars_in_expr(&r.node, bound, free);
        }
        ast::Expr::Unary(_, e) | ast::Expr::Paren(e) | ast::Expr::Try(e) => {
            collect_free_vars_in_expr(&e.node, bound, free)
        }
        ast::Expr::Call(callee, _, args) => {
            collect_free_vars_in_expr(&callee.node, bound, free);
            for arg in args {
                collect_free_vars_in_call_arg(arg, bound, free);
            }
        }
        ast::Expr::MethodCall(recv, _, _, args) => {
            collect_free_vars_in_expr(&recv.node, bound, free);
            for arg in args {
                collect_free_vars_in_call_arg(arg, bound, free);
            }
        }
        ast::Expr::Field(e, _) => collect_free_vars_in_expr(&e.node, bound, free),
        ast::Expr::Index(e, idx) => {
            collect_free_vars_in_expr(&e.node, bound, free);
            collect_free_vars_in_expr(&idx.node, bound, free);
        }
        ast::Expr::Slice(base, slice) => {
            collect_free_vars_in_expr(&base.node, bound, free);
            for component in [&slice.start, &slice.end, &slice.step].into_iter().flatten() {
                collect_free_vars_in_expr(&component.node, bound, free);
            }
        }
        ast::Expr::Tuple(items) | ast::Expr::Set(items) => {
            for item in items {
                collect_free_vars_in_expr(&item.node, bound, free);
            }
        }
        ast::Expr::List(entries) => {
            for entry in entries {
                match entry {
                    ast::ListEntry::Element(e) | ast::ListEntry::Spread(e) => {
                        collect_free_vars_in_expr(&e.node, bound, free)
                    }
                }
            }
        }
        ast::Expr::Dict(entries) => {
            for entry in entries {
                match entry {
                    ast::DictEntry::Pair(k, v) => {
                        collect_free_vars_in_expr(&k.node, bound, free);
                        collect_free_vars_in_expr(&v.node, bound, free);
                    }
                    ast::DictEntry::Spread(e) => collect_free_vars_in_expr(&e.node, bound, free),
                }
            }
        }
        ast::Expr::Constructor(_, args) => {
            for arg in args {
                collect_free_vars_in_call_arg(arg, bound, free);
            }
        }
        ast::Expr::Range { start, end, .. } => {
            collect_free_vars_in_expr(&start.node, bound, free);
            collect_free_vars_in_expr(&end.node, bound, free);
        }
        ast::Expr::FString(parts) => {
            for part in parts {
                if let ast::FStringPart::Expr { expr, .. } = part {
                    collect_free_vars_in_expr(&expr.node, bound, free);
                }
            }
        }
        ast::Expr::If(if_expr) => {
            collect_free_vars_in_expr(&if_expr.condition.node, bound, free);
            let mut then_bound = bound.clone();
            collect_free_vars_in_stmts(&if_expr.then_body, &mut then_bound, free);
            if let Some(else_body) = &if_expr.else_body {
                let mut else_bound = bound.clone();
                collect_free_vars_in_stmts(else_body, &mut else_bound, free);
            }
        }
        ast::Expr::Loop(loop_expr) => {
            let mut loop_bound = bound.clone();
            collect_free_vars_in_stmts(&loop_expr.body, &mut loop_bound, free);
        }
        ast::Expr::ListComp(comp) => {
            collect_free_vars_in_expr(&comp.iter.node, bound, free);
            let mut inner_bound = bound.clone();
            bind_pattern_names(&comp.pattern.node, &mut inner_bound);
            if let Some(filter) = &comp.filter {
                collect_free_vars_in_expr(&filter.node, &mut inner_bound, free);
            }
            collect_free_vars_in_expr(&comp.expr.node, &mut inner_bound, free);
        }
        ast::Expr::DictComp(comp) => {
            collect_free_vars_in_expr(&comp.iter.node, bound, free);
            let mut inner_bound = bound.clone();
            bind_pattern_names(&comp.pattern.node, &mut inner_bound);
            if let Some(filter) = &comp.filter {
                collect_free_vars_in_expr(&filter.node, &mut inner_bound, free);
            }
            collect_free_vars_in_expr(&comp.key.node, &mut inner_bound, free);
            collect_free_vars_in_expr(&comp.value.node, &mut inner_bound, free);
        }
        ast::Expr::Generator(generator) => {
            let mut inner_bound = bound.clone();
            for clause in &generator.clauses {
                match clause {
                    ast::ComprehensionClause::For { pattern, iter } => {
                        collect_free_vars_in_expr(&iter.node, &mut inner_bound, free);
                        bind_pattern_names(&pattern.node, &mut inner_bound);
                    }
                    ast::ComprehensionClause::If(cond) => collect_free_vars_in_expr(&cond.node, &mut inner_bound, free),
                }
            }
            collect_free_vars_in_expr(&generator.expr.node, &mut inner_bound, free);
        }
        ast::Expr::Closure(params, body) => {
            let mut inner_bound = bound.clone();
            for param in params {
                inner_bound.insert(param.node.name.clone());
            }
            collect_free_vars_in_expr(&body.node, &mut inner_bound, free);
        }
        ast::Expr::Partial(partial) => {
            collect_free_vars_in_expr(&partial.target.node, bound, free);
            for arg in &partial.args {
                collect_free_vars_in_expr(&arg.value.node, bound, free);
            }
        }
        // Mirrors `count_reads_in_expr`'s `Yield` arm: a yielded value is an ordinary nested expression for
        // free-variable purposes, so a name it reads from an enclosing closure scope must still be captured.
        ast::Expr::Yield(Some(value)) => collect_free_vars_in_expr(&value.node, bound, free),
        // The scrutinee is read in the enclosing scope like any other sub-expression. Each arm gets its own
        // *cloned* `bound` set (matching the `If`/`Loop` arms above) extended with that arm's own pattern-bound
        // names before walking its guard and body, so one arm's bindings never leak into a sibling arm or shadow an
        // outer free variable of the same name.
        ast::Expr::Match(subject, arms) => {
            collect_free_vars_in_expr(&subject.node, bound, free);
            for arm in arms {
                let mut arm_bound = bound.clone();
                bind_pattern_names(&arm.node.pattern.node, &mut arm_bound);
                if let Some(guard) = &arm.node.guard {
                    collect_free_vars_in_expr(&guard.node, &mut arm_bound, free);
                }
                match &arm.node.body {
                    ast::MatchBody::Expr(e) => collect_free_vars_in_expr(&e.node, &mut arm_bound, free),
                    ast::MatchBody::Block(stmts) => collect_free_vars_in_stmts(stmts, &mut arm_bound, free),
                }
            }
        }
        _ => {}
    }
}

/// Collect free variables from one call argument's expression, regardless of whether it is positional, named, or an
/// unpack -- matching [`count_reads_in_call_arg`]'s own "count the expression either way" stance, even though
/// [`BodyBuilder::lower_positional_args`] itself rejects named/unpack arguments during real lowering.
fn collect_free_vars_in_call_arg(arg: &ast::CallArg, bound: &mut HashSet<String>, free: &mut Vec<String>) {
    match arg {
        ast::CallArg::Positional(e)
        | ast::CallArg::Named(_, e)
        | ast::CallArg::PositionalUnpack(e)
        | ast::CallArg::KeywordUnpack(e) => collect_free_vars_in_expr(&e.node, bound, free),
    }
}

/// Collect free variables from an `if`/`while` condition, including the value expression of a `Condition::Let`
/// pattern condition (even though v0 lowering does not model `if let`/`while let` themselves -- see
/// [`BodyBuilder::lower_if`]/[`BodyBuilder::lower_while`]) -- a pattern-bound name still shadows an outer name of
/// the same spelling for anything nested inside the branch this condition gates, so it is bound defensively here
/// even though the branch itself lowers to `Unsupported`.
fn collect_free_vars_in_condition(cond: &ast::Condition, bound: &mut HashSet<String>, free: &mut Vec<String>) {
    match cond {
        ast::Condition::Expr(e) => collect_free_vars_in_expr(&e.node, bound, free),
        ast::Condition::Let { pattern, value } => {
            collect_free_vars_in_expr(&value.node, bound, free);
            bind_pattern_names(&pattern.node, bound);
        }
    }
}

/// Collect free variables from a statement block in source order, threading a progressively-extended `bound` set
/// through each statement so a binding one statement introduces (`let`, `for`, tuple unpack, ...) is visible to
/// every later statement in the *same* block, matching ordinary lexical scoping -- and, symmetrically, does not
/// leak into a sibling block (an `if`'s `else` body, for instance), since callers always pass a freshly cloned
/// `bound` per block (see [`collect_free_vars_in_expr`]'s `If`/`Loop` arms).
fn collect_free_vars_in_stmts(
    stmts: &[ast::Spanned<ast::Statement>],
    bound: &mut HashSet<String>,
    free: &mut Vec<String>,
) {
    for stmt in stmts {
        collect_free_vars_in_stmt(&stmt.node, bound, free);
    }
}

/// Collect free variables from one statement, recursing into every statement kind [`BodyBuilder`]'s own lowering
/// walks (see this module's module-level docs for the covered subset) and extending `bound` wherever that statement
/// introduces a new binding for the remainder of its enclosing block. Statement kinds outside v0's lowered subset
/// are not walked and neither read nor bind anything this analysis needs to know about.
fn collect_free_vars_in_stmt(stmt: &ast::Statement, bound: &mut HashSet<String>, free: &mut Vec<String>) {
    match stmt {
        ast::Statement::Assignment(a) => {
            collect_free_vars_in_expr(&a.value.node, bound, free);
            bound.insert(a.name.clone());
        }
        ast::Statement::FieldAssignment(fa) => {
            collect_free_vars_in_expr(&fa.object.node, bound, free);
            collect_free_vars_in_expr(&fa.value.node, bound, free);
        }
        ast::Statement::IndexAssignment(ia) => {
            collect_free_vars_in_expr(&ia.object.node, bound, free);
            collect_free_vars_in_expr(&ia.index.node, bound, free);
            collect_free_vars_in_expr(&ia.value.node, bound, free);
        }
        ast::Statement::CompoundAssignment(ca) => {
            // A compound assignment target must already exist, so it is a read of whatever bound it (an outer
            // capture, if this statement lives inside a closure body and `ca.name` was never rebound locally), not
            // a fresh binding -- see `Self::lower_partial`'s docs for the known limitation this implies for
            // mutating a captured variable from inside a closure.
            push_free(&ca.name, bound, free);
            collect_free_vars_in_expr(&ca.value.node, bound, free);
        }
        ast::Statement::TupleUnpack(tu) => {
            collect_free_vars_in_expr(&tu.value.node, bound, free);
            for name in &tu.names {
                bound.insert(name.clone());
            }
        }
        ast::Statement::TupleAssign(ta) => {
            for target in &ta.targets {
                collect_free_vars_in_expr(&target.node, bound, free);
            }
            collect_free_vars_in_expr(&ta.value.node, bound, free);
        }
        ast::Statement::ChainedAssignment(ca) => {
            collect_free_vars_in_expr(&ca.value.node, bound, free);
            for name in &ca.targets {
                bound.insert(name.clone());
            }
        }
        ast::Statement::Return(Some(e)) => collect_free_vars_in_expr(&e.node, bound, free),
        ast::Statement::Return(None) => {}
        ast::Statement::If(if_stmt) => {
            collect_free_vars_in_condition(&if_stmt.condition, bound, free);
            let mut then_bound = bound.clone();
            collect_free_vars_in_stmts(&if_stmt.then_body, &mut then_bound, free);
            for (cond, body) in &if_stmt.elif_branches {
                collect_free_vars_in_expr(&cond.node, bound, free);
                let mut elif_bound = bound.clone();
                collect_free_vars_in_stmts(body, &mut elif_bound, free);
            }
            if let Some(else_body) = &if_stmt.else_body {
                let mut else_bound = bound.clone();
                collect_free_vars_in_stmts(else_body, &mut else_bound, free);
            }
        }
        ast::Statement::While(w) => {
            collect_free_vars_in_condition(&w.condition, bound, free);
            let mut loop_bound = bound.clone();
            collect_free_vars_in_stmts(&w.body, &mut loop_bound, free);
        }
        ast::Statement::For(f) => {
            collect_free_vars_in_expr(&f.iter.node, bound, free);
            let mut loop_bound = bound.clone();
            bind_pattern_names(&f.pattern.node, &mut loop_bound);
            collect_free_vars_in_stmts(&f.body, &mut loop_bound, free);
        }
        ast::Statement::Expr(e) => collect_free_vars_in_expr(&e.node, bound, free),
        ast::Statement::Assert(a) => {
            if let ast::AssertKind::Condition(e) = &a.kind {
                collect_free_vars_in_expr(&e.node, bound, free);
            }
            if let Some(message) = &a.message {
                collect_free_vars_in_expr(&message.node, bound, free);
            }
        }
        ast::Statement::Break(Some(e)) => collect_free_vars_in_expr(&e.node, bound, free),
        _ => {}
    }
}

/// Count `name` occurrences in one call argument's expression, regardless of whether the argument is positional,
/// named, or an unpack — the read-count approximation counts the expression either way even though
/// [`BodyBuilder::lower_positional_args`] itself rejects named/unpack arguments during real lowering.
fn count_reads_in_call_arg(name: &str, arg: &ast::CallArg) -> usize {
    match arg {
        ast::CallArg::Positional(e)
        | ast::CallArg::Named(_, e)
        | ast::CallArg::PositionalUnpack(e)
        | ast::CallArg::KeywordUnpack(e) => count_reads_in_expr(name, &e.node),
    }
}

/// Register the callable-value contracts and private mechanisms owned by Body IR lowering.
///
/// This is deliberately adjacent to [`BodyBuilder::lower_closure`] and [`BodyBuilder::lower_partial`], rather than
/// a row in the compatibility collector. The replacement executor still refuses local callable targets; that fact
/// stays explicit in the collected evidence and does not make either feature execution-complete.
pub(crate) fn replacement_compatibility_body_ir_contribution()
-> crate::replacement_compatibility::ReplacementCompatibilityContribution {
    use crate::replacement_compatibility::{
        feature_requirement_link, implementation_requirement, local_implementation_contribution,
        planned_feature_at_boundary,
    };

    local_implementation_contribution(
        "frontend.body-ir.callable-values",
        "src/frontend/body_ir.rs",
        "fn replacement_compatibility_body_ir_contribution",
        vec![
            planned_feature_at_boundary(
                "call.partial-binding",
                "Partial presets capture at construction, remain overrideable defaults, and preserve named/positional binding rules.",
                1152,
                "Body IR carries the source contract; direct local callable targets remain visibly refused until the callable runtime slice executes them.",
                "src/frontend/typechecker/check_expr/calls.rs",
                "fn check_call",
                "fn lower_call",
                "fn execute_call",
            ),
            planned_feature_at_boundary(
                "call.stored-callables",
                "Stored closures and partials retain lexical capture timing, ownership, and isolated local call frames.",
                1152,
                "Direct execution deliberately refuses local callable targets; this is the coherent callable-frame profile.",
                "src/frontend/typechecker/check_expr/calls.rs",
                "fn check_call",
                "fn lower_call",
                "fn execute_call",
            ),
        ],
        vec![
            implementation_requirement(
                "call.argument-binder",
                "Parameter binding preserves positional, named, default, preset, variadic, and diagnostic rules.",
                "typechecker partial projection and replacement call runtime",
                "partial/default typechecker and Body-IR tests",
                "Binding slots are shared call machinery, not a user feature.",
            ),
            implementation_requirement(
                "captures.lexical-environments",
                "Closure and partial capture reads occur at construction time with explicit ownership.",
                "Body IR closure lowering and replacement runtime",
                "closure/partial capture timing regressions",
                "Lexical environments are private runtime state.",
            ),
        ],
        Vec::new(),
        vec![
            feature_requirement_link("call.partial-binding", "call.argument-binder"),
            feature_requirement_link("call.partial-binding", "captures.lexical-environments"),
            feature_requirement_link("call.stored-callables", "call.frames"),
            feature_requirement_link("call.stored-callables", "captures.lexical-environments"),
        ],
    )
}

mod match_;

mod closures;

mod async_;

mod literals;

mod calls;

mod operators;

mod expr;

mod defaults;

mod assertions;

mod comprehensions;

mod control_flow;

mod stmt;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tuple_destructure_interop_tests {
    use super::{IncanType, unsupported_tuple_destructure};

    /// Lowering must apply the same accepted-shape rule as the typechecker to interop values (#1132).
    ///
    /// A blanket `RustInteropPath` exemption here would leave the original defect reachable through interop: an
    /// opaque Rust value would lower to a `.0`/`.1` projection and fail as raw `rustc` output.
    #[test]
    fn opaque_rust_interop_values_refuse_to_lower_a_tuple_destructure() {
        assert!(
            unsupported_tuple_destructure(&IncanType::RustInteropPath("String".to_string()), 2).is_some(),
            "an opaque Rust value must not lower to a tuple field projection"
        );
        assert!(
            unsupported_tuple_destructure(&IncanType::RustInteropPath("std::vec::Vec<u8>".to_string()), 2).is_some(),
            "a Rust generic that is not a tuple must not lower to a tuple field projection"
        );
        // `(String)` is a parenthesised `String`, not a one-element tuple, so a single-name destructure must not
        // lower to `.0` against it.
        assert!(
            unsupported_tuple_destructure(&IncanType::RustInteropPath("(String)".to_string()), 1).is_some(),
            "a parenthesised Rust type has no `.0` field and must refuse to lower"
        );
        // The genuine one-element spelling still lowers.
        assert!(
            unsupported_tuple_destructure(&IncanType::RustInteropPath("(String,)".to_string()), 1).is_none(),
            "`(String,)` is a real one-element tuple and must keep lowering"
        );
    }

    /// The readable tuple spelling the stdlib relies on must still lower, so the refusal stays narrow.
    #[test]
    fn readable_rust_tuple_values_still_lower_a_tuple_destructure() {
        assert!(
            unsupported_tuple_destructure(
                &IncanType::RustInteropPath("(String,incan_stdlib::json::JsonValue)".to_string()),
                2
            )
            .is_none(),
            "`std.json` destructures a `rust::HashMap` item and must keep lowering"
        );
        assert!(
            unsupported_tuple_destructure(&IncanType::RustInteropPath("(String,JsonValue)".to_string()), 3).is_some(),
            "a Rust tuple of the wrong arity must still be refused"
        );
    }
}
