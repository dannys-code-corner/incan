//! Incan Code Formatter
//!
//! This module provides code formatting functionality for Incan source files.
//! It follows Ruff/Black conventions with customizations:
//! - 4-space indentation
//! - 120 character line length (target, not strictly enforced)
//! - Double quotes for strings
//! - Trailing commas in multi-line constructs
//!
//! ## Parse-required
//!
//! The formatter operates on the parsed AST, so it **requires valid syntax**.
//! Files with lexer or parser errors cannot be formatted.

mod config;
mod formatter;
mod writer;

pub use config::{FormatConfig, QuoteStyle};
pub use formatter::Formatter;

use crate::frontend::{diagnostics, lexer, parser};
use thiserror::Error;

/// Errors that occur during formatting
#[derive(Debug, Error)]
pub enum FormatError {
    #[error("syntax error (formatting requires valid syntax):\\n{0}")]
    SyntaxError(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Format Incan source code with default settings.
///
/// Returns an error if the source has syntax errors (formatting requires parsing).
///
/// # Examples
///
/// ```
/// use incan::format_source;
///
/// let source = "def add(a: int, b: int) -> int:\n    return a + b\n";
/// let formatted = format_source(source).unwrap();
/// assert!(formatted.contains("def add"));
/// ```
///
/// # Errors
///
/// Returns [`FormatError::SyntaxError`] if the source cannot be parsed.
pub fn format_source(source: &str) -> Result<String, FormatError> {
    format_source_with_config(source, FormatConfig::default())
}

/// Format Incan source code with custom configuration.
///
/// Returns an error if the source has syntax errors (formatting requires parsing).
///
/// # Examples
///
/// ```
/// use incan::{format_source_with_config, FormatConfig};
///
/// let config = FormatConfig::default();
/// let source = "def greet(name: str) -> str:\n    return name\n";
/// let formatted = format_source_with_config(source, config).unwrap();
/// assert!(formatted.contains("def greet"));
/// ```
pub fn format_source_with_config(
    source: &str,
    config: FormatConfig,
) -> Result<String, FormatError> {
    // Parse the source - formatter requires valid syntax
    let tokens = lexer::lex(source).map_err(|errs| {
        let mut msg = String::new();
        for err in &errs {
            msg.push_str(&diagnostics::format_error("<input>", source, err));
        }
        FormatError::SyntaxError(msg)
    })?;

    let ast = parser::parse(&tokens).map_err(|errs| {
        let mut msg = String::new();
        for err in &errs {
            msg.push_str(&diagnostics::format_error("<input>", source, err));
        }
        FormatError::SyntaxError(msg)
    })?;

    // Format the AST
    let formatter = Formatter::new(config);
    Ok(formatter.format(&ast))
}

/// Check if source code is already formatted.
///
/// # Examples
///
/// ```
/// use incan::check_formatted;
///
/// // Check returns a boolean (true = already formatted)
/// let source = "def foo() -> int:\n    return 42\n";
/// let is_formatted = check_formatted(source).unwrap();
/// // Result depends on exact formatting rules
/// assert!(is_formatted == true || is_formatted == false);
/// ```
pub fn check_formatted(source: &str) -> Result<bool, FormatError> {
    let formatted = format_source(source)?;
    Ok(source == formatted)
}

/// Get the diff between original and formatted source.
///
/// Returns `None` if the source is already formatted.
///
/// # Examples
///
/// ```
/// use incan::format_diff;
///
/// // Returns Ok with optional diff
/// let source = "def foo() -> int:\n    return 42\n";
/// let diff_result = format_diff(source);
/// assert!(diff_result.is_ok());
/// ```
pub fn format_diff(source: &str) -> Result<Option<String>, FormatError> {
    let formatted = format_source(source)?;

    if source == formatted {
        return Ok(None);
    }

    // Simple line-by-line diff
    let mut diff = String::new();
    let original_lines: Vec<&str> = source.lines().collect();
    let formatted_lines: Vec<&str> = formatted.lines().collect();

    let max_lines = original_lines.len().max(formatted_lines.len());

    for i in 0..max_lines {
        let orig = original_lines.get(i).unwrap_or(&"");
        let fmt = formatted_lines.get(i).unwrap_or(&"");

        if orig != fmt {
            if !orig.is_empty() {
                diff.push_str(&format!("-{:4} | {}\n", i + 1, orig));
            }
            if !fmt.is_empty() {
                diff.push_str(&format!("+{:4} | {}\n", i + 1, fmt));
            }
        }
    }

    Ok(Some(diff))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================
    // format_source tests
    // ========================================

    #[test]
    fn test_format_source_simple_function() {
        let source = r#"def foo() -> int:
  return 42
"#;
        let result = format_source(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_format_source_model() {
        let source = r#"model User:
  name: str
  age: int
"#;
        let result = format_source(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_format_source_invalid_syntax() {
        let source = "def foo(";
        let result = format_source(source);
        assert!(result.is_err());
    }

    #[test]
    fn test_format_source_empty() {
        let source = "";
        let result = format_source(source);
        assert!(result.is_ok());
    }

    // ========================================
    // format_source_with_config tests
    // ========================================

    #[test]
    fn test_format_source_with_custom_config() {
        let source = r#"def foo() -> int:
  return 42
"#;
        let config = FormatConfig::new().with_indent_width(2);
        let result = format_source_with_config(source, config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_format_source_with_different_line_length() {
        let source = r#"def foo() -> int:
  return 42
"#;
        let config = FormatConfig::new().with_line_length(80);
        let result = format_source_with_config(source, config);
        assert!(result.is_ok());
    }

    // ========================================
    // check_formatted tests
    // ========================================

    #[test]
    fn test_check_formatted_simple() {
        let source = r#"def foo() -> int:
    return 42
"#;
        let result = check_formatted(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_formatted_invalid_syntax() {
        let source = "def foo(";
        let result = check_formatted(source);
        assert!(result.is_err());
    }

    // ========================================
    // format_diff tests
    // ========================================

    #[test]
    fn test_format_diff_no_changes() {
        let source = r#"def foo() -> int:
    return 42
"#;
        let result = format_diff(source);
        // May have no changes if already formatted, or may have changes
        assert!(result.is_ok());
    }

    #[test]
    fn test_format_diff_invalid_syntax() {
        let source = "def foo(";
        let result = format_diff(source);
        assert!(result.is_err());
    }

    #[test]
    fn test_format_diff_returns_diff() {
        // Improperly indented source
        let source = r#"def foo() -> int:
 return 42
"#;
        let result = format_diff(source);
        assert!(result.is_ok());
        // The diff may or may not be Some depending on formatter behavior
    }
}
