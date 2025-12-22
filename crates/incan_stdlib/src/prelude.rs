//! Prelude module for common runtime imports.
//!
//! Import this in generated code to get access to all runtime functionality:
//! ```
//! use incan_stdlib::prelude::*;
//! ```

// Re-export runtime traits
pub use crate::reflection::FieldInfo;

#[cfg(feature = "json")]
pub use crate::json::{FromJson, ToJson};

// Re-export derive macros from incan_derive
// Note: These are proc macros and must be re-exported with `pub use`
pub use incan_derive::{FieldInfo as DeriveFieldInfo, IncanClass, IncanJson, IncanReflect};
