//! Standard library for Incan-generated Rust code.
//!
//! This crate provides traits and utilities that generated Incan code depends on, including reflection capabilities,
//! JSON serialization helpers, and numeric operations.

#![deny(clippy::unwrap_used)]

pub mod collections;
pub mod conversions;
pub mod errors;
pub mod frozen;
pub mod iter;
pub mod num;
pub mod prelude;
pub mod reflection;
pub mod strings;

#[cfg(feature = "json")]
pub mod json;

// Re-export commonly used items
pub use reflection::FieldInfo;

#[cfg(feature = "json")]
pub use json::{FromJson, ToJson};
