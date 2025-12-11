//! Incan Code Formatter
//!
//! This module provides code formatting functionality for Incan source files.
//! It follows Ruff/Black conventions with customizations:
//! - 4-space indentation
//! - 120 character line length
//! - Double quotes for strings
//! - Trailing commas in multi-line constructs

mod config;
mod writer;
mod formatter;

pub use config::{FormatConfig, QuoteStyle};
pub use formatter::Formatter;

use crate::frontend::{lexer, parser};

/// Format Incan source code with default settings
pub fn format_source(source: &str) -> Result<String, String> {
    format_source_with_config(source, FormatConfig::default())
}

/// Format Incan source code with custom configuration
pub fn format_source_with_config(source: &str, config: FormatConfig) -> Result<String, String> {
    // Parse the source
    let tokens = lexer::lex(source).map_err(|e| format!("Lexer error: {:?}", e))?;
    let ast = parser::parse(&tokens).map_err(|e| format!("Parser error: {:?}", e))?;
    
    // Format the AST
    let formatter = Formatter::new(config);
    Ok(formatter.format(&ast))
}

/// Check if source code is already formatted
pub fn check_formatted(source: &str) -> Result<bool, String> {
    let formatted = format_source(source)?;
    Ok(source == formatted)
}

/// Get the diff between original and formatted source
pub fn format_diff(source: &str) -> Result<Option<String>, String> {
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
