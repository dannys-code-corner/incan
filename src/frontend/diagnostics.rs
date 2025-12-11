//! Diagnostics and error reporting for Incan
//!
//! Provides Python-friendly error messages with source highlighting.

use crate::frontend::ast::Span;

/// A compile-time error with location information
#[derive(Debug, Clone, PartialEq)]
pub struct CompileError {
    pub message: String,
    pub span: Span,
    pub kind: ErrorKind,
    pub notes: Vec<String>,
    pub hints: Vec<String>,
}

impl CompileError {
    pub fn new(message: String, span: Span) -> Self {
        Self {
            message,
            span,
            kind: ErrorKind::Error,
            notes: Vec::new(),
            hints: Vec::new(),
        }
    }

    pub fn syntax(message: String, span: Span) -> Self {
        Self {
            message,
            span,
            kind: ErrorKind::Syntax,
            notes: Vec::new(),
            hints: Vec::new(),
        }
    }

    pub fn type_error(message: String, span: Span) -> Self {
        Self {
            message,
            span,
            kind: ErrorKind::Type,
            notes: Vec::new(),
            hints: Vec::new(),
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hints.push(hint.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Error,
    Syntax,
    Type,
    Warning,
    Lint,
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorKind::Error => write!(f, "error"),
            ErrorKind::Syntax => write!(f, "syntax error"),
            ErrorKind::Type => write!(f, "type error"),
            ErrorKind::Warning => write!(f, "warning"),
            ErrorKind::Lint => write!(f, "lint"),
        }
    }
}

/// Print an error with source context (simple implementation)
pub fn print_error(file_name: &str, source: &str, error: &CompileError) {
    let (line_num, col_num, line_text) = get_line_info(source, error.span.start);

    // Color codes
    let red = "\x1b[31m";
    let cyan = "\x1b[36m";
    let yellow = "\x1b[33m";
    let bold = "\x1b[1m";
    let reset = "\x1b[0m";

    let kind_color = match error.kind {
        ErrorKind::Error | ErrorKind::Syntax | ErrorKind::Type => red,
        ErrorKind::Warning | ErrorKind::Lint => yellow,
    };

    // Print header
    eprintln!(
        "{bold}{kind_color}{kind}{reset}{bold}: {message}{reset}",
        kind = error.kind,
        message = error.message,
    );

    // Print location
    eprintln!(
        "  {cyan}-->{reset} {file}:{line}:{col}",
        file = file_name,
        line = line_num,
        col = col_num,
    );

    // Print source line with line number
    let line_num_width = format!("{}", line_num).len();
    eprintln!("  {cyan}{:>width$} |{reset}", "", width = line_num_width);
    eprintln!(
        "  {cyan}{:>width$} |{reset} {}",
        line_num,
        line_text,
        width = line_num_width
    );

    // Print caret pointing to error
    let underline_len = if error.span.end > error.span.start && col_num > 0 {
        let start_offset = error.span.start.saturating_sub(col_num.saturating_sub(1));
        let end_in_line = error.span.end.saturating_sub(start_offset);
        end_in_line.min(line_text.len()).saturating_sub(col_num.saturating_sub(1)).max(1)
    } else {
        1
    };

    eprintln!(
        "  {cyan}{:>width$} |{reset} {}{kind_color}{}{reset}",
        "",
        " ".repeat(col_num - 1),
        "^".repeat(underline_len),
        width = line_num_width
    );

    // Print notes
    for note in &error.notes {
        eprintln!("  {cyan}= note:{reset} {}", note);
    }

    // Print hints
    for hint in &error.hints {
        eprintln!("  {cyan}= hint:{reset} {}", hint);
    }

    eprintln!();
}

/// Get line number, column number, and line text for a byte offset
fn get_line_info(source: &str, offset: usize) -> (usize, usize, &str) {
    let offset = offset.min(source.len());
    let mut line_num = 1;
    let mut line_start = 0;

    for (i, c) in source.char_indices() {
        if i >= offset {
            break;
        }
        if c == '\n' {
            line_num += 1;
            line_start = i + 1;
        }
    }

    let line_end = source[line_start..]
        .find('\n')
        .map(|i| line_start + i)
        .unwrap_or(source.len());

    let line_text = &source[line_start..line_end];
    let col_num = offset - line_start + 1;

    (line_num, col_num, line_text)
}

// ============================================================================
// Error catalog: common errors with Python-friendly explanations
// ============================================================================

/// Create common error types with helpful messages
pub mod errors {
    use super::*;

    pub fn unknown_symbol(name: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Unknown symbol '{}'", name),
            span,
        )
        .with_hint("Did you forget to import it or define it?")
    }

    pub fn type_mismatch(expected: &str, found: &str, span: Span) -> CompileError {
        let mut error = CompileError::type_error(
            format!("Type mismatch: expected '{}', found '{}'", expected, found),
            span,
        );
        
        // Add context-aware hints based on common patterns
        error = add_type_mismatch_hints(error, expected, found);
        error
    }
    
    /// Add smart hints based on the expected and found types
    fn add_type_mismatch_hints(mut error: CompileError, expected: &str, found: &str) -> CompileError {
        // Result/Option unwrapping hints
        if expected.starts_with("Result[") && !found.starts_with("Result[") {
            error = error.with_hint("Wrap the value with Ok(...) to return success");
            error = error.with_hint("Or use Err(...) to return an error");
            error = error.with_note("In Incan, functions that can fail return Result[T, E]");
        }
        
        if found.starts_with("Result[") && !expected.starts_with("Result[") {
            error = error.with_hint("Use the ? operator to unwrap: value?");
            error = error.with_hint("Or handle with match: match result: Ok(v) => ..., Err(e) => ...");
            error = error.with_note("Result must be explicitly unwrapped before use");
        }
        
        if expected.starts_with("Option[") && !found.starts_with("Option[") && found != "None" {
            error = error.with_hint("Wrap the value with Some(...) to make it optional");
        }
        
        if found.starts_with("Option[") && !expected.starts_with("Option[") {
            error = error.with_hint("Use .unwrap() if you're certain the value exists");
            error = error.with_hint("Or handle None with match: match opt: Some(v) => ..., None => ...");
        }
        
        // None vs Option hint
        if found == "None" && !expected.contains("Option") && expected != "None" {
            error = error.with_hint("None can only be used where Option[T] is expected");
        }
        
        // Numeric type hints
        if (expected == "int" && found == "float") || (expected == "float" && found == "int") {
            error = error.with_hint(format!(
                "Use explicit conversion: {}(...)",
                if expected == "int" { "int" } else { "float" }
            ));
        }
        
        // String vs other types
        if expected == "str" && found != "str" {
            error = error.with_hint("Use f-string or str() to convert to string");
        }
        
        // Bool condition hints
        if expected == "bool" {
            if found.starts_with("Option[") {
                error = error.with_hint("Use 'is Some' or 'is None' to check Option values");
                error = error.with_hint("Example: if value is Some(v): ...");
            } else if found.starts_with("Result[") {
                error = error.with_hint("Use 'is Ok' or 'is Err' to check Result values");
                error = error.with_hint("Example: if result is Ok(v): ...");
            } else if found == "int" || found == "float" || found == "str" {
                error = error.with_hint("Use explicit comparison instead of truthiness");
                error = error.with_hint(match found {
                    "int" | "float" => "Example: if value != 0: ...",
                    "str" => "Example: if value != \"\": ...",
                    _ => "Example: if value != default: ...",
                });
                error = error.with_note("Incan prefers explicit checks over implicit truthiness");
            }
        }
        
        // List/collection hints
        if expected.starts_with("List[") && found.starts_with("List[") {
            error = error.with_hint("List element types must match exactly");
        }
        
        error
    }

    pub fn unknown_derive(name: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Unknown derive '{}'", name),
            span,
        )
        .with_hint("Valid derives: Debug, Display, Eq, Ord, Hash, Clone, Copy, Default, Serialize, Deserialize")
        .with_hint("Hint: Use 'with TraitName' syntax for custom trait implementations")
    }

    pub fn derive_wrong_kind(name: &str, kind: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Cannot derive '{}' - it is a {}, not a trait", name, kind),
            span,
        )
        .with_hint(format!("@derive() only works with traits like Debug, Eq, Clone"))
        .with_hint(format!("Did you mean: `with {}` to implement a trait?", name))
    }

    pub fn missing_return_type(span: Span) -> CompileError {
        CompileError::type_error(
            "Function is missing a return type".to_string(),
            span,
        )
        .with_hint("Add a return type annotation: def name(...) -> Type:")
    }

    pub fn incompatible_error_type(expected: &str, found: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!(
                "Cannot use '?' here: function returns Result[_, {}] but expression has error type '{}'",
                expected, found
            ),
            span,
        )
        .with_hint("Use map_err to convert the error type, or add a From implementation")
    }

    pub fn non_exhaustive_match(missing: &[String], span: Span) -> CompileError {
        let missing_str = missing.join(", ");
        CompileError::type_error(
            format!("Non-exhaustive match: missing patterns for {}", missing_str),
            span,
        )
        .with_hint("Add the missing cases or use '_' as a wildcard (use wildcards sparingly)")
    }

    pub fn mutation_without_mut(name: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Cannot mutate '{}' - variable is immutable", name),
            span,
        )
        .with_hint(format!("Declare with 'mut' to allow mutation: mut {} = ...", name))
        .with_note("In Incan, variables are immutable by default for safety")
        .with_note("This prevents accidental modifications and makes code easier to reason about")
    }

    pub fn self_mutation_without_mut(span: Span) -> CompileError {
        CompileError::type_error(
            "Cannot mutate self - method takes immutable self".to_string(),
            span,
        )
        .with_hint("Change the method signature to use 'mut self':")
        .with_hint("  def method(mut self) -> ReturnType:")
        .with_note("Methods that modify self must explicitly declare 'mut self'")
    }
    
    pub fn reassignment_without_mut(name: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Cannot reassign '{}' - variable is immutable", name),
            span,
        )
        .with_hint(format!("Declare with 'mut' to allow reassignment: mut {} = ...", name))
        .with_hint("Or use a new variable name with 'let'")
        .with_note("Reassignment requires the variable to be declared as mutable")
    }

    pub fn try_on_non_result(found: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Cannot use '?' on type '{}' - expected Result[T, E]", found),
            span,
        )
        .with_note("The '?' operator only works on Result types")
        .with_hint("The ? operator unwraps Ok(value) or returns early with Err(error)")
        .with_hint("Example: let user = get_user(id)?  # Returns Err if get_user fails")
        .with_note(if found.starts_with("Option[") {
            "For Option types, use .ok_or(error)? to convert to Result first"
        } else {
            "If this operation can fail, the function should return Result[T, E]"
        })
    }

    pub fn trait_conflict(trait_a: &str, trait_b: &str, method: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!(
                "Conflicting implementations: both {} and {} define method '{}'",
                trait_a, trait_b, method
            ),
            span,
        )
        .with_hint(format!("Resolve the conflict explicitly: {}.{}(self, ...)", trait_a, method))
    }

    pub fn missing_field(type_name: &str, field: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Type '{}' has no field '{}'", type_name, field),
            span,
        )
    }

    pub fn field_type_mismatch(field: &str, expected: &str, found: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Cannot assign '{}' to field '{}' of type '{}'", found, field, expected),
            span,
        )
        .with_hint(format!("Field '{}' expects type '{}', but got '{}'", field, expected, found))
    }

    pub fn not_indexable(type_name: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Type '{}' is not indexable", type_name),
            span,
        )
        .with_hint("Only List, Dict, str, and Tuple types support indexing")
    }

    pub fn index_type_mismatch(expected: &str, found: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Index type mismatch: expected '{}', found '{}'", expected, found),
            span,
        )
        .with_hint(format!("Use '{}' as the index type", expected))
    }

    pub fn index_value_type_mismatch(expected: &str, found: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Cannot assign '{}' to collection element of type '{}'", found, expected),
            span,
        )
        .with_hint(format!("Collection elements are of type '{}', but got '{}'", expected, found))
    }

    pub fn mutable_tuple(span: Span) -> CompileError {
        CompileError::type_error(
            "Tuples are immutable and cannot be declared with 'mut'".to_string(),
            span,
        )
        .with_hint("Remove 'mut' - tuples cannot be modified after creation")
    }

    pub fn tuple_field_assignment(span: Span) -> CompileError {
        CompileError::type_error(
            "Cannot assign to tuple field - tuples are immutable".to_string(),
            span,
        )
        .with_hint("Create a new tuple instead of modifying an existing one")
    }

    pub fn missing_trait_method(trait_name: &str, method: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Trait '{}' requires method '{}' to be implemented", trait_name, method),
            span,
        )
        .with_hint(format!("Add the required method: def {}(self, ...) -> ReturnType:", method))
        .with_note("All required trait methods must be implemented")
    }
    
    pub fn trait_not_implemented(type_name: &str, trait_name: &str, span: Span) -> CompileError {
        let mut error = CompileError::type_error(
            format!("Type '{}' does not implement trait '{}'", type_name, trait_name),
            span,
        );
        
        // Add specific hints based on the trait
        match trait_name {
            "Eq" => {
                error = error.with_hint("Add @derive(Eq) to enable equality comparison (==, !=)");
                error = error.with_hint("Or implement __eq__ manually for custom comparison logic");
            }
            "Ord" => {
                error = error.with_hint("Add @derive(Ord) to enable ordering comparison (<, >, <=, >=)");
                error = error.with_hint("Or implement __lt__ manually for custom ordering");
            }
            "Hash" => {
                error = error.with_hint("Add @derive(Hash) to use this type in Set or as Dict key");
                error = error.with_note("Hash is required for Set membership and Dict keys");
            }
            "Clone" => {
                error = error.with_hint("Add @derive(Clone) to enable .clone() method");
            }
            "Debug" => {
                error = error.with_hint("Add @derive(Debug) to enable {:?} formatting");
            }
            "Display" => {
                error = error.with_hint("Implement __str__ method for string representation");
                error = error.with_hint("Example: def __str__(self) -> str: return f\"{self.name}\"");
            }
            "Default" => {
                error = error.with_hint("Add @derive(Default) to enable Type.default()");
            }
            "Serialize" | "Deserialize" => {
                error = error.with_hint(format!("Add @derive({}) for JSON/serialization support", trait_name));
            }
            "Error" => {
                error = error.with_hint("Implement the Error trait with a message() method");
                error = error.with_hint("Example: def message(self) -> str: return self.msg");
            }
            _ => {
                error = error.with_hint(format!("Implement the {} trait or add 'with {}'", trait_name, trait_name));
            }
        }
        
        error
    }
    
    pub fn cannot_compare(type_name: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Cannot compare values of type '{}' - Eq trait not implemented", type_name),
            span,
        )
        .with_hint("Add @derive(Eq) to the type definition to enable comparison")
        .with_note("Comparison operators (==, !=) require the Eq trait")
    }
    
    pub fn cannot_order(type_name: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Cannot order values of type '{}' - Ord trait not implemented", type_name),
            span,
        )
        .with_hint("Add @derive(Ord) to the type definition to enable ordering")
        .with_note("Ordering operators (<, >, <=, >=) require the Ord trait")
    }
    
    pub fn not_hashable(type_name: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Type '{}' cannot be used in Set or as Dict key - Hash trait not implemented", type_name),
            span,
        )
        .with_hint("Add @derive(Hash, Eq) to make this type hashable")
        .with_note("Both Hash and Eq are required for Set membership and Dict keys")
    }

    pub fn expected_token(expected: &str, found: &str, span: Span) -> CompileError {
        CompileError::syntax(
            format!("Expected {}, found {}", expected, found),
            span,
        )
    }

    pub fn unexpected_token(found: &str, span: Span) -> CompileError {
        CompileError::syntax(
            format!("Unexpected token: {}", found),
            span,
        )
    }

    pub fn invalid_receiver(span: Span) -> CompileError {
        CompileError::syntax(
            "Invalid receiver - expected 'self' or 'mut self'".to_string(),
            span,
        )
    }

    pub fn duplicate_definition(name: &str, span: Span) -> CompileError {
        CompileError::type_error(
            format!("Duplicate definition of '{}'", name),
            span,
        )
    }
}

// ============================================================================
// Lint warnings
// ============================================================================

pub mod lints {
    use super::*;

    pub fn unused_variable(name: &str, span: Span) -> CompileError {
        CompileError {
            message: format!("Unused variable '{}'", name),
            span,
            kind: ErrorKind::Lint,
            notes: vec![],
            hints: vec!["Prefix with underscore to silence: _{}".to_string() + name],
        }
    }

    pub fn unused_import(name: &str, span: Span) -> CompileError {
        CompileError {
            message: format!("Unused import '{}'", name),
            span,
            kind: ErrorKind::Lint,
            notes: vec![],
            hints: vec!["Remove the import or use it".to_string()],
        }
    }

    pub fn wildcard_match(span: Span) -> CompileError {
        CompileError {
            message: "Using wildcard '_' in match - consider handling all cases explicitly".to_string(),
            span,
            kind: ErrorKind::Lint,
            notes: vec![],
            hints: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_line_info() {
        let source = "line 1\nline 2\nline 3";
        
        let (line, col, text) = get_line_info(source, 0);
        assert_eq!(line, 1);
        assert_eq!(col, 1);
        assert_eq!(text, "line 1");

        let (line, col, text) = get_line_info(source, 7);
        assert_eq!(line, 2);
        assert_eq!(col, 1);
        assert_eq!(text, "line 2");

        let (line, col, text) = get_line_info(source, 10);
        assert_eq!(line, 2);
        assert_eq!(col, 4);
        assert_eq!(text, "line 2");
    }
}
