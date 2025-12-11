//! Incan Compiler Frontend
//!
//! This module contains all frontend components:
//! - `lexer`: tokenization of source code
//! - `parser`: parsing tokens into AST
//! - `ast`: abstract syntax tree definitions
//! - `symbols`: symbol table and scope management
//! - `typechecker`: type checking and validation
//! - `diagnostics`: error reporting and lints

pub mod lexer;
pub mod parser;
pub mod ast;
pub mod symbols;
pub mod typechecker;
pub mod diagnostics;
pub mod module;
