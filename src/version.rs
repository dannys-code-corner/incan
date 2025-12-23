//! Incan compiler version information.
//!
//! This module exposes the compiler version as a single constant so all subsystems
//! (CLI, codegen headers, project generator) agree on the same value.
//!
//! ## Notes
//!
//! - The value is taken from Cargo metadata (`CARGO_PKG_VERSION`) at compile time.
//! - Prefer this constant over repeating `env!("CARGO_PKG_VERSION")` in multiple places.

/// The Incan compiler version string (for example, `0.1.0-alpha.2`).
pub const INCAN_VERSION: &str = env!("CARGO_PKG_VERSION");
