//! Convert Incan compiler diagnostics to LSP diagnostics

use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, Position, Range, Url,
};

use crate::frontend::diagnostics::{CompileError, ErrorKind};

/// Convert a byte offset to LSP Position (0-based line and character)
pub fn offset_to_position(source: &str, offset: usize) -> Position {
    let offset = offset.min(source.len());
    let mut line = 0u32;
    let mut col = 0u32;

    for (i, c) in source.char_indices() {
        if i >= offset {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }

    Position::new(line, col)
}

/// Convert a span to LSP Range
pub fn span_to_range(source: &str, start: usize, end: usize) -> Range {
    let start_pos = offset_to_position(source, start);
    let end_pos = offset_to_position(source, end.max(start + 1));
    Range::new(start_pos, end_pos)
}

/// Convert ErrorKind to LSP DiagnosticSeverity
fn error_kind_to_severity(kind: ErrorKind) -> DiagnosticSeverity {
    match kind {
        ErrorKind::Error | ErrorKind::Syntax | ErrorKind::Type => DiagnosticSeverity::ERROR,
        ErrorKind::Warning => DiagnosticSeverity::WARNING,
        ErrorKind::Lint => DiagnosticSeverity::HINT,
    }
}

/// Convert a CompileError to LSP Diagnostic
pub fn compile_error_to_diagnostic(
    error: &CompileError,
    source: &str,
    uri: &Url,
) -> Diagnostic {
    let range = span_to_range(source, error.span.start, error.span.end);
    let severity = error_kind_to_severity(error.kind);

    // Build the message with notes and hints
    let mut message = error.message.clone();

    // Add notes
    for note in &error.notes {
        message.push_str("\n\nnote: ");
        message.push_str(note);
    }

    // Add hints
    for hint in &error.hints {
        message.push_str("\n\nhint: ");
        message.push_str(hint);
    }

    // Create related information for notes/hints (shows in Problems panel)
    let mut related_information = Vec::new();

    for note in &error.notes {
        related_information.push(DiagnosticRelatedInformation {
            location: Location {
                uri: uri.clone(),
                range,
            },
            message: format!("note: {}", note),
        });
    }

    for hint in &error.hints {
        related_information.push(DiagnosticRelatedInformation {
            location: Location {
                uri: uri.clone(),
                range,
            },
            message: format!("hint: {}", hint),
        });
    }

    Diagnostic {
        range,
        severity: Some(severity),
        code: None,
        code_description: None,
        source: Some("incan".to_string()),
        message,
        related_information: if related_information.is_empty() {
            None
        } else {
            Some(related_information)
        },
        tags: None,
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offset_to_position() {
        let source = "line 1\nline 2\nline 3";

        let pos = offset_to_position(source, 0);
        assert_eq!(pos.line, 0);
        assert_eq!(pos.character, 0);

        let pos = offset_to_position(source, 7); // Start of "line 2"
        assert_eq!(pos.line, 1);
        assert_eq!(pos.character, 0);

        let pos = offset_to_position(source, 10); // "e 2"
        assert_eq!(pos.line, 1);
        assert_eq!(pos.character, 3);
    }
}
