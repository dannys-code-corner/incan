//! Incan Compiler Backend
//!
//! This module handles code generation from the typed AST to Rust source code.
//!
//! The pipeline is:
//! 1. Typed AST from frontend → RustCodegen → Rust source files
//! 2. Generate Cargo.toml for the output project
//! 3. Invoke `cargo build` to produce the final binary
//!
//! ## Module Organization
//!
//! - `codegen/` - Code generation from AST to Rust
//!   - `mod.rs` - Main RustCodegen struct and entry point
//!   - `types.rs` - Helper type definitions
//!   - `type_conv.rs` - Type conversion utilities
//!   - `imports.rs` - Import statement handling
//!   - `declarations.rs` - Model, class, trait, enum emission
//!   - `functions.rs` - Function and method emission
//!   - `statements.rs` - Statement emission
//!   - `expressions.rs` - Expression emission
//!   - `patterns.rs` - Pattern matching emission
//! - `rust_emitter.rs` - Low-level Rust code string builder
//! - `project.rs` - Cargo project generation

pub mod codegen;
pub mod rust_emitter;
pub mod project;

pub use codegen::RustCodegen;
pub use project::ProjectGenerator;
