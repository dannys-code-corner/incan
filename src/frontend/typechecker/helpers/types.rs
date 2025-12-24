//! Type name constants and generic constructors used across the typechecker.
use crate::frontend::symbols::ResolvedType;

/// Name of the `List` generic type.
pub const LIST_TY_NAME: &str = "List";
/// Name of the `Dict` generic type.
pub const DICT_TY_NAME: &str = "Dict";
/// Name of the `Set` generic type.
pub const SET_TY_NAME: &str = "Set";
/// Name of the `Tuple` generic type.
pub const TUPLE_TY_NAME: &str = "Tuple";
/// Name of the `Option` generic type.
pub const OPTION_TY_NAME: &str = "Option";
/// Name of the `Result` generic type.
pub const RESULT_TY_NAME: &str = "Result";

/// Name of the frozen string wrapper type.
pub const FROZEN_STR_TY_NAME: &str = "FrozenStr";
/// Name of the frozen bytes wrapper type.
pub const FROZEN_BYTES_TY_NAME: &str = "FrozenBytes";
/// Name of the frozen list wrapper type.
pub const FROZEN_LIST_TY_NAME: &str = "FrozenList";
/// Name of the frozen dict wrapper type.
pub const FROZEN_DICT_TY_NAME: &str = "FrozenDict";
/// Name of the frozen set wrapper type.
pub const FROZEN_SET_TY_NAME: &str = "FrozenSet";

/// Construct a `List[T]` type.
///
/// ## Parameters
/// - `elem`: The element type `T`.
///
/// ## Returns
/// - The resolved type `List[T]`.
pub fn list_ty(elem: ResolvedType) -> ResolvedType {
    ResolvedType::Generic(LIST_TY_NAME.to_string(), vec![elem])
}

/// Construct a `Dict[K, V]` type.
///
/// ## Parameters
/// - `key`: The key type `K`.
/// - `val`: The value type `V`.
///
/// ## Returns
/// - The resolved type `Dict[K, V]`.
pub fn dict_ty(key: ResolvedType, val: ResolvedType) -> ResolvedType {
    ResolvedType::Generic(DICT_TY_NAME.to_string(), vec![key, val])
}

/// Construct an `Option[T]` type.
///
/// ## Parameters
/// - `inner`: The inner type `T`.
///
/// ## Returns
/// - The resolved type `Option[T]`.
pub fn option_ty(inner: ResolvedType) -> ResolvedType {
    ResolvedType::Generic(OPTION_TY_NAME.to_string(), vec![inner])
}

/// Construct a `Result[Ok, Err]` type.
///
/// ## Parameters
/// - `ok`: The ok type.
/// - `err`: The error type.
///
/// ## Returns
/// - The resolved type `Result[Ok, Err]`.
pub fn result_ty(ok: ResolvedType, err: ResolvedType) -> ResolvedType {
    ResolvedType::Generic(RESULT_TY_NAME.to_string(), vec![ok, err])
}
