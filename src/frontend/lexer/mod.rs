//! Lexer for the Incan programming language
//!
//! Handles tokenization including:
//! - Keywords (def, async, await, class, model, trait, etc.)
//! - Identifiers and literals (int, float, string, f-string)
//! - Operators and punctuation (::, =>, ?, etc.)
//! - Indentation-based blocks (INDENT/DEDENT tokens)
//!
//! ## Module Structure
//!
//! - `tokens` - Token types (TokenKind, Token, FStringPart)
//! - `strings` - String/f-string/byte-string scanning
//! - `numbers` - Numeric literal scanning
//! - `indent` - INDENT/DEDENT handling

mod indent;
mod numbers;
mod strings;
pub mod tokens;

pub use tokens::{FStringPart, Token, TokenKind};

use crate::frontend::ast::Span;
use crate::frontend::diagnostics::CompileError;
use tokens::KEYWORDS;

// ============================================================================
// LEXER STATE
// ----------------------------------------------------------------------------
// Lexer state diagram (simplfied):
//
// [Start of line] → count spaces → [Inside code]
//                                       ↓
//                                      see '(' → [bracket_depth++]
//                                       ↓
//                                      see '\n' → skip (inside brackets)
//                                       ↓
//                                      see ')' → [bracket_depth--]
// ============================================================================

/// Lexer for Incan source code.
///
/// Converts source text into a stream of tokens, handling:
/// - Keywords and identifiers
/// - Numeric and string literals (including f-strings and byte strings)
/// - Operators and punctuation
/// - Python-style indentation (INDENT/DEDENT tokens)
/// - Implicit line continuation inside brackets
pub struct Lexer<'a> {
    source: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    current_pos: usize,
    indent_stack: Vec<usize>,
    pending_dedents: usize,
    at_line_start: bool,
    /// Bracket depth for implicit line continuation (parens, brackets, braces)
    bracket_depth: usize,
    tokens: Vec<Token>,
    errors: Vec<CompileError>,
}

impl<'a> Lexer<'a> {
    /// Create a new lexer for the given source code.
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            chars: source.char_indices().peekable(),
            current_pos: 0,
            indent_stack: vec![0],
            pending_dedents: 0,
            at_line_start: true,
            bracket_depth: 0,
            tokens: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Tokenize the entire source code.
    ///
    /// Returns a vector of tokens on success, or a vector of errors on failure.
    /// The token stream always ends with an `Eof` token.
    pub fn tokenize(mut self) -> Result<Vec<Token>, Vec<CompileError>> {
        while !self.is_at_end() {
            self.scan_token();
        }

        // Emit remaining dedents at EOF
        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            self.tokens.push(Token::new(
                TokenKind::Dedent,
                Span::new(self.current_pos, self.current_pos),
            ));
        }

        self.tokens.push(Token::new(
            TokenKind::Eof,
            Span::new(self.current_pos, self.current_pos),
        ));

        if self.errors.is_empty() {
            Ok(self.tokens)
        } else {
            Err(self.errors)
        }
    }

    // ========================================================================
    // Core character handling
    // ========================================================================

    fn is_at_end(&mut self) -> bool {
        self.chars.peek().is_none()
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().map(|(_, c)| *c)
    }

    fn peek_next(&self) -> Option<char> {
        let mut iter = self.source[self.current_pos..].char_indices();
        iter.next(); // skip current
        iter.next().map(|(_, c)| c)
    }

    fn advance(&mut self) -> Option<char> {
        if let Some((pos, c)) = self.chars.next() {
            self.current_pos = pos + c.len_utf8();
            Some(c)
        } else {
            None
        }
    }

    // ========================================================================
    // Main scanning dispatch
    // ========================================================================

    fn scan_token(&mut self) {
        // Handle pending dedents first
        if self.pending_dedents > 0 {
            self.pending_dedents -= 1;
            self.tokens.push(Token::new(
                TokenKind::Dedent,
                Span::new(self.current_pos, self.current_pos),
            ));
            return;
        }

        // Handle indentation at line start
        if self.at_line_start {
            self.handle_indentation();
            return;
        }

        // Skip whitespace (but not newlines)
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' {
                self.advance();
            } else {
                break;
            }
        }

        let start = self.current_pos;

        let Some(c) = self.advance() else {
            return;
        };

        match c {
            // Comments
            '#' => {
                while let Some(c) = self.peek() {
                    if c == '\n' {
                        break;
                    }
                    self.advance();
                }
            }

            // Newlines
            '\n' => {
                // Implicit line continuation: skip newlines inside brackets
                if self.bracket_depth > 0 {
                    // Inside brackets - don't emit newline, don't trigger indentation
                    return;
                }
                // Skip blank lines (don't emit newline if we're already at line start)
                if !self.at_line_start {
                    self.tokens
                        .push(Token::new(TokenKind::Newline, Span::new(start, self.current_pos)));
                }
                self.at_line_start = true;
            }

            // Skip carriage return
            '\r' => {}

            // Operators and punctuation
            '+' => self.operator(start, TokenKind::Plus, &[('=', TokenKind::PlusEq)]),
            '-' => self.operator(
                start,
                TokenKind::Minus,
                &[('>', TokenKind::Arrow), ('=', TokenKind::MinusEq)],
            ),
            '*' => self.operator(
                start,
                TokenKind::Star,
                &[('*', TokenKind::StarStar), ('=', TokenKind::StarEq)],
            ),
            '/' => self.scan_slash(start),
            '%' => self.operator(start, TokenKind::Percent, &[('=', TokenKind::PercentEq)]),
            '?' => self.add_token(TokenKind::Question, start),
            '@' => self.add_token(TokenKind::At, start),
            ',' => self.add_token(TokenKind::Comma, start),
            '(' => self.open_bracket(TokenKind::LParen, start),
            ')' => self.close_bracket(TokenKind::RParen, start),
            '[' => self.open_bracket(TokenKind::LBracket, start),
            ']' => self.close_bracket(TokenKind::RBracket, start),
            '{' => self.open_bracket(TokenKind::LBrace, start),
            '}' => self.close_bracket(TokenKind::RBrace, start),
            ':' => self.operator(start, TokenKind::Colon, &[(':', TokenKind::ColonColon)]),
            '=' => self.operator(
                start,
                TokenKind::Eq,
                &[('=', TokenKind::EqEq), ('>', TokenKind::FatArrow)],
            ),
            '!' => {
                if self.match_char('=') {
                    self.add_token(TokenKind::NotEq, start);
                } else {
                    self.errors.push(CompileError::new(
                        "Unexpected character '!'".to_string(),
                        Span::new(start, self.current_pos),
                    ));
                }
            }
            '<' => self.operator(start, TokenKind::Lt, &[('=', TokenKind::LtEq)]),
            '>' => self.operator(start, TokenKind::Gt, &[('=', TokenKind::GtEq)]),
            '.' => {
                if self.match_char('.') {
                    if self.match_char('.') {
                        self.add_token(TokenKind::Ellipsis, start);
                    } else if self.match_char('=') {
                        self.add_token(TokenKind::DotDotEq, start);
                    } else {
                        self.add_token(TokenKind::DotDot, start);
                    }
                } else {
                    self.add_token(TokenKind::Dot, start);
                }
            }

            // Strings
            '"' => self.scan_string(start, '"'),
            '\'' => self.scan_string(start, '\''),

            // f-strings
            'f' if self.peek() == Some('"') || self.peek() == Some('\'') => {
                // Safe: we just checked peek() is Some quote char
                let quote = self.advance().expect("f-string quote after peek check");
                self.scan_fstring(start, quote);
            }

            // b-strings (byte strings)
            'b' if self.peek() == Some('"') || self.peek() == Some('\'') => {
                // Safe: we just checked peek() is Some quote char
                let quote = self.advance().expect("b-string quote after peek check");
                self.scan_byte_string(start, quote);
            }

            // Numbers
            '0'..='9' => self.scan_number(start, c),

            // Identifiers and keywords
            _ if is_ident_start(c) => self.scan_identifier(start, c),

            _ => {
                self.errors.push(CompileError::new(
                    format!("Unexpected character '{}'", c),
                    Span::new(start, self.current_pos),
                ));
            }
        }
    }

    // ========================================================================
    // Operator helpers
    // ========================================================================

    fn match_char(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn add_token(&mut self, kind: TokenKind, start: usize) {
        self.tokens.push(Token::new(kind, Span::new(start, self.current_pos)));
    }

    /// Try to match compound operator, fallback to simple.
    fn operator(&mut self, start: usize, simple: TokenKind, compounds: &[(char, TokenKind)]) {
        for (c, kind) in compounds {
            if self.match_char(*c) {
                self.add_token(kind.clone(), start);
                return;
            }
        }
        self.add_token(simple, start);
    }

    /// Scan slash operators: `/`, `/=`, `//`, `//=`.
    fn scan_slash(&mut self, start: usize) {
        if self.match_char('/') {
            // `//` or `//=`
            if self.match_char('=') {
                self.add_token(TokenKind::SlashSlashEq, start);
            } else {
                self.add_token(TokenKind::SlashSlash, start);
            }
        } else if self.match_char('=') {
            // `/=`
            self.add_token(TokenKind::SlashEq, start);
        } else {
            // `/`
            self.add_token(TokenKind::Slash, start);
        }
    }

    /// Emit a bracket token and track bracket depth.
    fn open_bracket(&mut self, kind: TokenKind, start: usize) {
        self.bracket_depth += 1;
        self.add_token(kind, start);
    }

    /// Emit a closing bracket token and decrement bracket depth.
    /// Produces an error if there's no matching opening bracket.
    fn close_bracket(&mut self, kind: TokenKind, start: usize) {
        if self.bracket_depth == 0 {
            self.errors.push(CompileError::new(
                "Unmatched closing bracket".to_string(),
                Span::new(start, self.current_pos),
            ));
        } else {
            self.bracket_depth -= 1;
        }
        self.add_token(kind, start);
    }

    // ========================================================================
    // Identifier scanning
    // ========================================================================

    fn scan_identifier(&mut self, start: usize, first: char) {
        let mut name = String::from(first);

        while let Some(c) = self.peek() {
            if is_ident_continue(c) {
                name.push(c);
                self.advance();
            } else {
                break;
            }
        }

        // Look up keyword in O(1) using perfect hash map
        let kind = KEYWORDS.get(name.as_str()).cloned().unwrap_or(TokenKind::Ident(name));

        self.add_token(kind, start);
    }
}

// ============================================================================
// Helper functions
// ============================================================================

/// Check if a character can start an identifier (ASCII-only).
fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

/// Check if a character can continue an identifier (ASCII-only).
fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Convenience function to lex a source string.
///
/// This is a shorthand for `Lexer::new(source).tokenize()`.
#[tracing::instrument(skip_all, fields(source_len = source.len()))]
pub fn lex(source: &str) -> Result<Vec<Token>, Vec<CompileError>> {
    Lexer::new(source).tokenize()
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keywords() {
        let tokens = lex("def async await class model trait").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Def));
        assert!(matches!(tokens[1].kind, TokenKind::Async));
        assert!(matches!(tokens[2].kind, TokenKind::Await));
        assert!(matches!(tokens[3].kind, TokenKind::Class));
        assert!(matches!(tokens[4].kind, TokenKind::Model));
        assert!(matches!(tokens[5].kind, TokenKind::Trait));
    }

    #[test]
    fn test_operators() {
        let tokens = lex("+ - * / :: => -> ? @ == !=").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Plus));
        assert!(matches!(tokens[1].kind, TokenKind::Minus));
        assert!(matches!(tokens[2].kind, TokenKind::Star));
        assert!(matches!(tokens[3].kind, TokenKind::Slash));
        assert!(matches!(tokens[4].kind, TokenKind::ColonColon));
        assert!(matches!(tokens[5].kind, TokenKind::FatArrow));
        assert!(matches!(tokens[6].kind, TokenKind::Arrow));
        assert!(matches!(tokens[7].kind, TokenKind::Question));
        assert!(matches!(tokens[8].kind, TokenKind::At));
        assert!(matches!(tokens[9].kind, TokenKind::EqEq));
        assert!(matches!(tokens[10].kind, TokenKind::NotEq));
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_numbers() {
        let tokens = lex("42 3.14 1_000_000 1e10").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Int(42)));
        assert!(matches!(tokens[1].kind, TokenKind::Float(f) if (f - 3.14).abs() < 0.001));
        assert!(matches!(tokens[2].kind, TokenKind::Int(1000000)));
        assert!(matches!(tokens[3].kind, TokenKind::Float(_)));
    }

    #[test]
    fn test_strings() {
        let tokens = lex(r#""hello" 'world'"#).unwrap();
        assert!(matches!(&tokens[0].kind, TokenKind::String(s) if s == "hello"));
        assert!(matches!(&tokens[1].kind, TokenKind::String(s) if s == "world"));
    }

    #[test]
    fn test_indentation() {
        let source = "def foo():\n  x = 1\n  y = 2\nx = 3";
        let tokens = lex(source).unwrap();

        // Find indent and dedent tokens
        let indent_count = tokens.iter().filter(|t| matches!(t.kind, TokenKind::Indent)).count();
        let dedent_count = tokens.iter().filter(|t| matches!(t.kind, TokenKind::Dedent)).count();

        assert_eq!(indent_count, 1, "Should have 1 INDENT token");
        assert_eq!(dedent_count, 1, "Should have 1 DEDENT token");
    }

    #[test]
    fn test_import_path() {
        let tokens = lex("import polars::prelude as pl").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Import));
        assert!(matches!(&tokens[1].kind, TokenKind::Ident(s) if s == "polars"));
        assert!(matches!(tokens[2].kind, TokenKind::ColonColon));
        assert!(matches!(&tokens[3].kind, TokenKind::Ident(s) if s == "prelude"));
        assert!(matches!(tokens[4].kind, TokenKind::As));
        assert!(matches!(&tokens[5].kind, TokenKind::Ident(s) if s == "pl"));
    }

    #[test]
    fn test_fstring() {
        let tokens = lex(r#"f"Hello {name}!""#).unwrap();
        match &tokens[0].kind {
            TokenKind::FString(parts) => {
                assert_eq!(parts.len(), 3);
                assert!(matches!(&parts[0], FStringPart::Literal(s) if s == "Hello "));
                assert!(matches!(&parts[1], FStringPart::Expr(s) if s == "name"));
                assert!(matches!(&parts[2], FStringPart::Literal(s) if s == "!"));
            }
            _ => panic!("Expected FString token"),
        }
    }

    #[test]
    fn test_unicode_identifier_rejected() {
        // Unicode characters should not be valid identifiers (ASCII-only)
        let result = lex("π = 1");
        assert!(result.is_err(), "Unicode identifier should produce an error");
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("Unexpected character"));
    }

    #[test]
    fn test_unmatched_closing_bracket() {
        // Closing bracket without matching open should produce an error
        let result = lex(")");
        assert!(result.is_err(), "Unmatched ) should produce an error");
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("Unmatched closing bracket"));

        // Same for ] and }
        let result = lex("]");
        assert!(result.is_err());
        assert!(result.unwrap_err()[0].message.contains("Unmatched closing bracket"));

        let result = lex("}");
        assert!(result.is_err());
        assert!(result.unwrap_err()[0].message.contains("Unmatched closing bracket"));
    }

    #[test]
    fn test_matched_brackets_ok() {
        // Properly matched brackets should work fine
        let tokens = lex("(x)").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::LParen));
        assert!(matches!(&tokens[1].kind, TokenKind::Ident(s) if s == "x"));
        assert!(matches!(tokens[2].kind, TokenKind::RParen));
    }

    #[test]
    fn test_multiple_dedents() {
        // Multiple dedent levels in one line
        let source = "def foo():\n  if True:\n    x = 1\ny = 2";
        let tokens = lex(source).unwrap();

        let indent_count = tokens.iter().filter(|t| matches!(t.kind, TokenKind::Indent)).count();
        let dedent_count = tokens.iter().filter(|t| matches!(t.kind, TokenKind::Dedent)).count();

        assert_eq!(indent_count, 2, "Should have 2 INDENT tokens");
        assert_eq!(dedent_count, 2, "Should have 2 DEDENT tokens");
    }

    #[test]
    fn test_tabs_as_spaces() {
        // Tabs count as 2 spaces
        let source = "def foo():\n\tx = 1"; // Tab indentation
        let tokens = lex(source).unwrap();

        let indent_count = tokens.iter().filter(|t| matches!(t.kind, TokenKind::Indent)).count();
        assert_eq!(indent_count, 1, "Tab should produce INDENT");
    }

    #[test]
    fn test_newlines_inside_brackets() {
        // Newlines inside brackets should NOT emit Newline tokens (implicit continuation)
        let source = "foo(\n  x,\n  y\n)";
        let tokens = lex(source).unwrap();

        let newline_count = tokens.iter().filter(|t| matches!(t.kind, TokenKind::Newline)).count();
        assert_eq!(newline_count, 0, "No Newline tokens inside brackets");
    }

    #[test]
    fn test_range_not_float() {
        // 1..2 should be Int, DotDot, Int - not a float
        let tokens = lex("1..2").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Int(1)));
        assert!(matches!(tokens[1].kind, TokenKind::DotDot));
        assert!(matches!(tokens[2].kind, TokenKind::Int(2)));
    }

    #[test]
    fn test_inclusive_range() {
        // 1..=5 should work
        let tokens = lex("1..=5").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Int(1)));
        assert!(matches!(tokens[1].kind, TokenKind::DotDotEq));
        assert!(matches!(tokens[2].kind, TokenKind::Int(5)));
    }
}
