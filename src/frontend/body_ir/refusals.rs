//! Labels and shape checks behind every explicit lowering refusal.

use super::*;

/// Short diagnostic label for a statement kind v0 does not lower.
///
/// Statement-position `loop:` is named explicitly because it is the one entry here whose Body IR vocabulary
/// already exists: [`BodyBuilder::lower_loop_expr`] emits [`bir::StatementKind::Loop`] for the expression
/// spelling, and only [`BodyBuilder::lower_stmt_into`]'s dispatch is missing (#1101). Leaving it under the
/// generic "statement" label made a five-line dispatch gap read like an unmodeled construct.
pub(super) fn unsupported_stmt_label(stmt: &ast::Statement) -> String {
    match stmt {
        ast::Statement::Loop(_) => "statement-position `loop:`".to_string(),
        ast::Statement::Unsafe(_) => "unsafe block".to_string(),
        ast::Statement::VocabExpressionItem(_) => "vocab expression item".to_string(),
        ast::Statement::Surface(_) => "surface statement".to_string(),
        ast::Statement::VocabBlock(_) => "vocab block".to_string(),
        _ => "statement".to_string(),
    }
}
/// Why an admitted provider operation cannot become a checked execution plan, or `None` when it can (#1213).
///
/// Consulted once, before any argument of the call is lowered, so a refusal never leaves the operands of a call that
/// never happens behind -- the same "check before partially lowering" precedent as [`match_pattern_is_supported`].
/// Refusing here rather than emitting a plan is what makes the "no execution receipt for a lowering refusal"
/// guarantee structural: with no [`bir::Callee::ProviderOperation`] statement there is nothing for an executor to
/// run, and nothing for it to report having run.
///
/// Two independent things make an operation unexecutable, and both are checked.
///
/// **Activation.** Only an active provider may be planned against. A disabled or unavailable provider is a real
/// entry in the catalog, so the call did resolve; what it did not do is reach something this compilation can
/// execute, and the two states are named separately because they have different remedies.
///
/// **Capability identity.** The plan promises that [`bir::ProviderOperationPlan::required_capability`] names an RFC
/// 104 `capability` declaration, which is what makes an authority request answerable. An identity of any other kind
/// -- a function, a model, a module -- would produce a request no authority source could decide, so it is refused
/// here rather than carried into a plan that quietly cannot be authorized.
///
/// The message names the *declaration* the identity selected, never the call site's spelling: which operation this
/// is, is a question only the canonical identity answers.
pub(super) fn unsupported_provider_operation(
    operation: &CanonicalSymbolId,
    record: &ProviderOperationRecord,
) -> Option<String> {
    let declared = &operation.declaration_name;
    match record.provider.state {
        bir::ProviderActivationState::Active => {}
        bir::ProviderActivationState::Disabled => {
            return Some(format!(
                "provider operation `{declared}` whose provider is not enabled in this compilation"
            ));
        }
        bir::ProviderActivationState::Unavailable => {
            return Some(format!(
                "provider operation `{declared}` whose provider has no locally available artifact"
            ));
        }
    }
    if record.required_capability.kind != SemanticSourceTargetKind::Capability {
        return Some(format!(
            "provider operation `{declared}` whose required authority does not name a capability declaration"
        ));
    }
    None
}
/// Short diagnostic label for an expression kind v0 does not lower.
///
/// Only reached from [`BodyBuilder::lower_expr_to_operand`]'s fallback arm, so every expression kind that arm
/// dispatches by name -- closures and partial callables included, since #1124 gave both a real lowering -- is
/// deliberately absent here. Async surface (`await`, `race for`) and vocab/scoped-DSL surface are named rather
/// than left to the generic label, because both are tracked remaining work under #1101 (#1164 and #1166
/// respectively) and a diagnostic reading only "expression" hides which one a program actually hit.
pub(super) fn unsupported_expr_label(expr: &ast::Expr) -> String {
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
pub(super) fn surface_expr_label(payload: &ast::SurfaceExprPayload) -> String {
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
pub(super) fn unsupported_tuple_destructure(value_ty: &IncanType, arity: usize) -> Option<String> {
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
pub(super) fn tuple_element_types(ty: &IncanType, count: usize) -> Vec<IncanType> {
    match tuple_type_elements(ty) {
        Some(items) if items.len() == count => items.to_vec(),
        _ => vec![IncanType::Unknown; count],
    }
}
/// The element types of a tuple-shaped [`IncanType`], in either spelling, or `None` when `ty` is not a tuple at
/// all. Backs both [`tuple_element_types`] and [`unsupported_for_pattern`], so the "is this a tuple, and of what
/// arity" question is answered in exactly one place rather than once per caller.
pub(super) fn tuple_type_elements(ty: &IncanType) -> Option<&[IncanType]> {
    match ty {
        IncanType::Tuple(items) => Some(items),
        IncanType::Generic { base, args } if collections::from_str(base) == Some(CollectionTypeId::Tuple) => Some(args),
        _ => None,
    }
}
/// Whether `pattern` is representable by [`bir::Pattern`]'s closed vocabulary. The only unrepresentable shape is a
/// byte-string literal pattern ([`bir::Constant`] has no byte-string variant -- see [`lower_literal`]'s own `None`
/// case for the identical gap in plain literal *expressions*); every other pattern shape lowers structurally, with
/// [`IncanType::Unknown`] field-type fallbacks where needed rather than an outright failure (see
/// [`BodyBuilder::lower_match_pattern`]'s own docs). Checked for every arm before [`BodyBuilder::lower_match`]
/// lowers any of them, mirroring [`BodyBuilder::binary_op_is_supported`]'s "check before partially lowering"
/// precedent.
pub(super) fn match_pattern_is_supported(pattern: &ast::Pattern) -> bool {
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
pub(super) fn unsupported_for_pattern(pattern: &ast::Pattern, item_ty: &IncanType) -> Option<String> {
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
