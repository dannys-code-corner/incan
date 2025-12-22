//! Standard library for Incan-generated Rust code.
//!
//! This crate provides traits and utilities that generated Incan code depends on,
//! including reflection capabilities and JSON serialization helpers.

#![deny(clippy::unwrap_used)]

pub mod prelude;
pub mod reflection;

#[cfg(feature = "json")]
pub mod json;

// Re-export commonly used items
pub use reflection::FieldInfo;

#[cfg(feature = "json")]
pub use json::{FromJson, ToJson};
