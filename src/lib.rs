//! Incan Programming Language Compiler
//!
//! Incan combines Rust's safety and performance with Python's expressiveness.
//! This crate provides the compiler: frontend (lexer, parser, type checker),
//! backend (Rust code generation), and tooling (formatter, LSP).

pub mod frontend;
pub mod backend;
pub mod format;
pub mod lsp;
pub mod cli;

pub use frontend::lexer;
pub use frontend::parser;
pub use frontend::ast;
pub use frontend::symbols;
pub use frontend::typechecker;
pub use frontend::diagnostics;

pub use backend::codegen::RustCodegen;
pub use backend::project::ProjectGenerator;

pub use format::{format_source, format_source_with_config, check_formatted, format_diff, FormatConfig};
