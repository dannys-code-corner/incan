//! Type definitions for code generation
//!
//! Contains helper structs used throughout the codegen module.

/// Represents which dunder methods are defined on a type
pub(crate) struct DunderMethods {
    pub has_eq: bool,
    pub has_hash: bool,
    pub has_ord: bool,
    pub has_str: bool,
}

impl DunderMethods {
    pub fn new() -> Self {
        Self {
            has_eq: false,
            has_hash: false,
            has_ord: false,
            has_str: false,
        }
    }
}

impl Default for DunderMethods {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about a route collected from @route decorators
#[derive(Debug, Clone)]
pub(crate) struct RouteInfo {
    /// Handler function name
    pub handler_name: String,
    /// URL path pattern (e.g., "/api/users/{id}")
    pub path: String,
    /// HTTP methods (e.g., ["GET", "POST"])
    pub methods: Vec<String>,
    /// Whether the handler is async
    /// Note: Currently stored for future async route handler support
    #[allow(dead_code)]
    pub is_async: bool,
}
