// Test-only inspection helpers for canonical identities embedded in generated Rust spellings.
//
// Semantic compiler paths may only project checked identities into generated names; they must never recover source
// meaning by decoding those names. Artifact-facing tests still need to inspect the projection, so that deliberately
// reverse-facing work lives outside `src/` at the same boundary as other generated-artifact assertions.

use std::collections::HashSet;

use incan_semantics_core::{
    CanonicalSymbolId, SemanticSourceTargetKind, decode_incan_symbol_identity, encode_incan_symbol_identity,
};

/// Recover every projected identity for one source declaration from generated Rust.
pub(crate) fn projected_identities(
    code: &str,
    source_name: &str,
    kind: SemanticSourceTargetKind,
) -> HashSet<CanonicalSymbolId> {
    code.split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .filter(|token| token.starts_with("__incan_v"))
        .filter_map(|token| decode_incan_symbol_identity(token).ok().flatten())
        .filter(|identity| identity.kind == kind && identity.declaration_name == source_name)
        .collect()
}

/// Recover the one projected identity expected for a source declaration.
pub(crate) fn projected_identity(code: &str, source_name: &str, kind: SemanticSourceTargetKind) -> CanonicalSymbolId {
    let identities = projected_identities(code, source_name, kind.clone());
    assert_eq!(
        identities.len(),
        1,
        "expected exactly one {kind:?} identity for `{source_name}`, got {identities:?} in:\n{code}"
    );
    identities
        .into_iter()
        .next()
        .unwrap_or_else(|| unreachable!("identity count checked above"))
}

/// Recover the exact generated Rust projection for one source declaration.
pub(crate) fn projected_name(code: &str, source_name: &str, kind: SemanticSourceTargetKind) -> String {
    encode_incan_symbol_identity(&projected_identity(code, source_name, kind))
}
