//! Reflection support for Incan models and classes.
//!
//! The `FieldInfo` trait provides introspection capabilities for structured types,
//! allowing generated code to query field names and types at runtime.

/// Provides reflection information about a type's fields.
///
/// This trait is typically derived using `#[derive(FieldInfo)]` on models and classes.
///
/// # Examples
///
/// ```ignore
/// #[derive(FieldInfo)]
/// struct Person {
///     name: String,
///     age: i64,
/// }
///
/// // Generated implementation provides:
/// assert_eq!(Person::field_names(), vec!["name", "age"]);
/// assert_eq!(Person::field_types(), vec!["String", "i64"]);
/// ```
pub trait FieldInfo {
    /// Returns the names of all fields in this type.
    fn field_names() -> Vec<&'static str>;

    /// Returns the type names of all fields in this type.
    fn field_types() -> Vec<&'static str>;
}
