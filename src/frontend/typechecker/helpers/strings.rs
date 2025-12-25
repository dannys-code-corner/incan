//! String-related helpers for the typechecker (predicates, method returns).
use crate::frontend::symbols::ResolvedType;

use super::{FROZEN_BYTES_TY_NAME, FROZEN_STR_TY_NAME, LIST_TY_NAME};

/// Check whether a resolved type should be treated as string-like.
///
/// This returns `true` for:
/// - `str` (runtime string)
/// - `FrozenStr` (const-eval / frozen string)
pub fn is_str_like(ty: &ResolvedType) -> bool {
    matches!(ty, ResolvedType::Str | ResolvedType::FrozenStr)
        || matches!(ty, ResolvedType::Named(name) if name == FROZEN_STR_TY_NAME)
}

/// Check whether a resolved type is `FrozenStr`.
pub fn is_frozen_str(ty: &ResolvedType) -> bool {
    matches!(ty, ResolvedType::FrozenStr) || matches!(ty, ResolvedType::Named(name) if name == FROZEN_STR_TY_NAME)
}

/// Construct the resolved type `FrozenStr`.
pub fn frozen_str_ty() -> ResolvedType {
    ResolvedType::FrozenStr
}

/// Check whether a resolved type is `FrozenBytes`.
pub fn is_frozen_bytes(ty: &ResolvedType) -> bool {
    matches!(ty, ResolvedType::FrozenBytes) || matches!(ty, ResolvedType::Named(name) if name == FROZEN_BYTES_TY_NAME)
}

/// Construct the resolved type `FrozenBytes`.
pub fn frozen_bytes_ty() -> ResolvedType {
    ResolvedType::FrozenBytes
}

/// Return the resolved type for a supported string method, if known.
pub fn string_method_return(method: &str, include_len: bool) -> Option<ResolvedType> {
    match method {
        "upper" | "lower" | "strip" | "replace" | "join" | "to_string" => Some(ResolvedType::Str),
        "split_whitespace" | "split" => Some(ResolvedType::Generic(LIST_TY_NAME.to_string(), vec![ResolvedType::Str])),
        "contains" | "startswith" | "endswith" => Some(ResolvedType::Bool),
        "len" if include_len => Some(ResolvedType::Int),
        "is_empty" if include_len => Some(ResolvedType::Bool),
        _ => None,
    }
}
