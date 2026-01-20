//! Shared compiler conventions (well-known identifiers).

/// Entry point function name.
pub const ENTRYPOINT_NAME: &str = "main";

/// Tuple newtype field index used by codegen (`struct Newtype(T)` field name).
pub const NEWTYPE_TUPLE_FIELD: &str = "0";

/// Preferred validated-constructor method for newtypes.
pub const NEWTYPE_FROM_UNDERLYING_METHOD: &str = "from_underlying";

/// Convention: validation method name for `@derive(Validate)`.
pub const VALIDATE_METHOD: &str = "validate";

/// Convention: constructor method name for derived validation helpers.
pub const NEW_METHOD: &str = "new";

/// Type name alias for Unit.
pub const UNIT_TYPE_NAME: &str = "Unit";

/// Type name alias for None (treated as Unit in type position).
pub const NONE_TYPE_NAME: &str = "None";
