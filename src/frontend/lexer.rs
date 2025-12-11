//! Lexer for the Incan programming language
//!
//! Handles tokenization including:
//! - Keywords (def, async, await, class, model, trait, etc.)
//! - Identifiers and literals (int, float, string, f-string)
//! - Operators and punctuation (::, =>, ?, etc.)
//! - Indentation-based blocks (INDENT/DEDENT tokens)

use crate::frontend::ast::Span;
use crate::frontend::diagnostics::CompileError;

/// Token types for Incan
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // ========== Keywords ==========
    Def,
    Async,
    Await,
    Class,
    Model,
    Trait,
    Enum,
    Type,
    Newtype,
    Import,
    As,
    Py,
    From,
    With,
    Extends,
    Return,
    If,
    Else,
    While,
    For,
    Break,
    Continue,
    In,
    Match,
    Case,
    And,
    Or,
    Not,
    Is,
    True,
    False,
    None,
    Let,
    Mut,
    SelfKw,
    Pass,
    Pub,
    Super,   // super (parent module)
    Crate,   // crate (project root)
    Yield,   // yield (for fixtures/generators)
    RustKw,  // rust (for rust:: imports)

    // ========== Identifiers and Literals ==========
    Ident(String),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    FString(Vec<FStringPart>),

    // ========== Operators ==========
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    Percent,    // %
    Eq,         // =
    EqEq,       // ==
    NotEq,      // !=
    PlusEq,     // +=
    MinusEq,    // -=
    StarEq,     // *=
    SlashEq,    // /=
    PercentEq,  // %=
    Lt,         // <
    Gt,         // >
    LtEq,       // <=
    GtEq,       // >=
    Arrow,      // ->
    FatArrow,   // =>
    Question,   // ?
    Colon,      // :
    ColonColon, // ::
    Dot,        // .
    DotDot,     // ..
    DotDotEq,   // ..= (inclusive range)
    Comma,      // ,
    At,         // @

    // ========== Brackets ==========
    LParen,   // (
    RParen,   // )
    LBracket, // [
    RBracket, // ]
    LBrace,   // {
    RBrace,   // }

    // ========== Indentation ==========
    Newline,
    Indent,
    Dedent,

    // ========== Special ==========
    Ellipsis, // ...
    Eof,
}

/// Part of an f-string
#[derive(Debug, Clone, PartialEq)]
pub enum FStringPart {
    Literal(String),
    Expr(String), // We store the raw expression string; parser will parse it
}

/// A token with its kind and span
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// Lexer state
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

    /// Tokenize the entire source
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
            '+' => {
                if self.match_char('=') {
                    self.add_token(TokenKind::PlusEq, start);
                } else {
                    self.add_token(TokenKind::Plus, start);
                }
            }
            '-' => {
                if self.match_char('>') {
                    self.add_token(TokenKind::Arrow, start);
                } else if self.match_char('=') {
                    self.add_token(TokenKind::MinusEq, start);
                } else {
                    self.add_token(TokenKind::Minus, start);
                }
            }
            '*' => {
                if self.match_char('=') {
                    self.add_token(TokenKind::StarEq, start);
                } else {
                    self.add_token(TokenKind::Star, start);
                }
            }
            '/' => {
                if self.match_char('=') {
                    self.add_token(TokenKind::SlashEq, start);
                } else {
                    self.add_token(TokenKind::Slash, start);
                }
            }
            '%' => {
                if self.match_char('=') {
                    self.add_token(TokenKind::PercentEq, start);
                } else {
                    self.add_token(TokenKind::Percent, start);
                }
            }
            '?' => self.add_token(TokenKind::Question, start),
            '@' => self.add_token(TokenKind::At, start),
            ',' => self.add_token(TokenKind::Comma, start),
            '(' => {
                self.bracket_depth += 1;
                self.add_token(TokenKind::LParen, start);
            }
            ')' => {
                if self.bracket_depth > 0 {
                    self.bracket_depth -= 1;
                }
                self.add_token(TokenKind::RParen, start);
            }
            '[' => {
                self.bracket_depth += 1;
                self.add_token(TokenKind::LBracket, start);
            }
            ']' => {
                if self.bracket_depth > 0 {
                    self.bracket_depth -= 1;
                }
                self.add_token(TokenKind::RBracket, start);
            }
            '{' => {
                self.bracket_depth += 1;
                self.add_token(TokenKind::LBrace, start);
            }
            '}' => {
                if self.bracket_depth > 0 {
                    self.bracket_depth -= 1;
                }
                self.add_token(TokenKind::RBrace, start);
            }

            ':' => {
                if self.match_char(':') {
                    self.add_token(TokenKind::ColonColon, start);
                } else {
                    self.add_token(TokenKind::Colon, start);
                }
            }

            '=' => {
                if self.match_char('=') {
                    self.add_token(TokenKind::EqEq, start);
                } else if self.match_char('>') {
                    self.add_token(TokenKind::FatArrow, start);
                } else {
                    self.add_token(TokenKind::Eq, start);
                }
            }

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

            '<' => {
                if self.match_char('=') {
                    self.add_token(TokenKind::LtEq, start);
                } else {
                    self.add_token(TokenKind::Lt, start);
                }
            }

            '>' => {
                if self.match_char('=') {
                    self.add_token(TokenKind::GtEq, start);
                } else {
                    self.add_token(TokenKind::Gt, start);
                }
            }

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
                let quote = self.advance().unwrap();
                self.scan_fstring(start, quote);
            }
            
            // b-strings (byte strings)
            'b' if self.peek() == Some('"') || self.peek() == Some('\'') => {
                let quote = self.advance().unwrap();
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

    fn match_char(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn add_token(&mut self, kind: TokenKind, start: usize) {
        self.tokens
            .push(Token::new(kind, Span::new(start, self.current_pos)));
    }

    fn handle_indentation(&mut self) {
        let start = self.current_pos;
        let mut indent = 0;

        // Count leading spaces/tabs
        while let Some(c) = self.peek() {
            match c {
                ' ' => {
                    indent += 1;
                    self.advance();
                }
                '\t' => {
                    // Treat tab as 2 spaces (Incan uses 2-space indentation)
                    indent += 2;
                    self.advance();
                }
                '#' => {
                    // Comment line - skip to end
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.advance();
                    }
                    if self.peek() == Some('\n') {
                        self.advance();
                    }
                    return; // Stay at line start
                }
                '\n' => {
                    // Blank line - skip
                    self.advance();
                    return; // Stay at line start
                }
                '\r' => {
                    self.advance();
                }
                _ => break,
            }
        }

        // At end of file?
        if self.is_at_end() {
            self.at_line_start = false;
            return;
        }

        let current_indent = *self.indent_stack.last().unwrap_or(&0);

        if indent > current_indent {
            self.indent_stack.push(indent);
            self.tokens
                .push(Token::new(TokenKind::Indent, Span::new(start, self.current_pos)));
        } else if indent < current_indent {
            // Count how many dedents we need BEFORE modifying the stack
            let mut count = 0;
            for &level in self.indent_stack.iter().rev() {
                if indent >= level {
                    break;
                }
                count += 1;
            }

            // Pop indent levels
            while let Some(&top) = self.indent_stack.last() {
                if indent >= top {
                    break;
                }
                self.indent_stack.pop();
                if self.indent_stack.is_empty() {
                    self.indent_stack.push(0);
                    break;
                }
            }

            // Verify we landed on a valid indent level
            let final_indent = *self.indent_stack.last().unwrap_or(&0);
            if indent != final_indent {
                self.errors.push(CompileError::new(
                    format!("Inconsistent indentation: expected {} spaces, got {}", final_indent, indent),
                    Span::new(start, self.current_pos),
                ));
            }

            // Emit dedent tokens
            if count > 0 {
                self.tokens
                    .push(Token::new(TokenKind::Dedent, Span::new(start, self.current_pos)));
                if count > 1 {
                    self.pending_dedents = count - 1;
                }
            }
        }

        self.at_line_start = false;
    }

    fn scan_string(&mut self, start: usize, quote: char) {
        // Check for triple-quoted string
        let triple = if self.peek() == Some(quote) {
            if self.peek_next() == Some(quote) {
                self.advance(); // consume second quote
                self.advance(); // consume third quote
                true
            } else {
                false
            }
        } else {
            false
        };

        let mut value = String::new();

        loop {
            match self.peek() {
                None => {
                    self.errors.push(CompileError::new(
                        "Unterminated string".to_string(),
                        Span::new(start, self.current_pos),
                    ));
                    break;
                }
                Some(c) if c == quote => {
                    if triple {
                        // Need three quotes to close
                        self.advance();
                        if self.peek() == Some(quote) {
                            self.advance();
                            if self.peek() == Some(quote) {
                                self.advance();
                                break;
                            } else {
                                value.push(quote);
                                value.push(quote);
                            }
                        } else {
                            value.push(quote);
                        }
                    } else {
                        self.advance();
                        break;
                    }
                }
                Some('\n') if !triple => {
                    self.errors.push(CompileError::new(
                        "Unterminated string (newline in single-quoted string)".to_string(),
                        Span::new(start, self.current_pos),
                    ));
                    break;
                }
                Some('\\') => {
                    self.advance();
                    match self.advance() {
                        Some('n') => value.push('\n'),
                        Some('t') => value.push('\t'),
                        Some('r') => value.push('\r'),
                        Some('\\') => value.push('\\'),
                        Some(q) if q == quote => value.push(q),
                        Some(c) => {
                            value.push('\\');
                            value.push(c);
                        }
                        None => {
                            self.errors.push(CompileError::new(
                                "Unterminated escape sequence".to_string(),
                                Span::new(start, self.current_pos),
                            ));
                            break;
                        }
                    }
                }
                Some(c) => {
                    value.push(c);
                    self.advance();
                }
            }
        }

        self.tokens.push(Token::new(
            TokenKind::String(value),
            Span::new(start, self.current_pos),
        ));
    }
    
    fn scan_byte_string(&mut self, start: usize, quote: char) {
        let mut value = Vec::new();

        loop {
            match self.peek() {
                None => {
                    self.errors.push(CompileError::new(
                        "Unterminated byte string".to_string(),
                        Span::new(start, self.current_pos),
                    ));
                    break;
                }
                Some(c) if c == quote => {
                    self.advance();
                    break;
                }
                Some('\n') => {
                    self.errors.push(CompileError::new(
                        "Unterminated byte string (newline in string)".to_string(),
                        Span::new(start, self.current_pos),
                    ));
                    break;
                }
                Some('\\') => {
                    self.advance();
                    match self.advance() {
                        Some('n') => value.push(b'\n'),
                        Some('t') => value.push(b'\t'),
                        Some('r') => value.push(b'\r'),
                        Some('\\') => value.push(b'\\'),
                        Some('0') => value.push(0),
                        Some('x') => {
                            // Hex escape \xNN
                            let mut hex = String::new();
                            if let Some(c) = self.advance() { hex.push(c); }
                            if let Some(c) = self.advance() { hex.push(c); }
                            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                                value.push(byte);
                            } else {
                                self.errors.push(CompileError::new(
                                    format!("Invalid hex escape: \\x{}", hex),
                                    Span::new(start, self.current_pos),
                                ));
                            }
                        }
                        Some(q) if q == quote => value.push(q as u8),
                        Some(c) => {
                            value.push(b'\\');
                            value.push(c as u8);
                        }
                        None => {
                            self.errors.push(CompileError::new(
                                "Unterminated escape sequence".to_string(),
                                Span::new(start, self.current_pos),
                            ));
                            break;
                        }
                    }
                }
                Some(c) => {
                    // Byte strings should only contain ASCII
                    if c.is_ascii() {
                        value.push(c as u8);
                    } else {
                        self.errors.push(CompileError::new(
                            format!("Non-ASCII character in byte string: '{}'", c),
                            Span::new(start, self.current_pos),
                        ));
                    }
                    self.advance();
                }
            }
        }

        self.tokens.push(Token::new(
            TokenKind::Bytes(value),
            Span::new(start, self.current_pos),
        ));
    }

    fn scan_fstring(&mut self, start: usize, quote: char) {
        let mut parts = Vec::new();
        let mut literal = String::new();

        loop {
            match self.peek() {
                None => {
                    self.errors.push(CompileError::new(
                        "Unterminated f-string".to_string(),
                        Span::new(start, self.current_pos),
                    ));
                    break;
                }
                Some(c) if c == quote => {
                    self.advance();
                    break;
                }
                Some('{') => {
                    self.advance();
                    if self.peek() == Some('{') {
                        // Escaped brace
                        self.advance();
                        literal.push('{');
                    } else {
                        // Push current literal
                        if !literal.is_empty() {
                            parts.push(FStringPart::Literal(std::mem::take(&mut literal)));
                        }
                        // Scan expression
                        let expr = self.scan_fstring_expr();
                        parts.push(FStringPart::Expr(expr));
                    }
                }
                Some('}') => {
                    self.advance();
                    if self.peek() == Some('}') {
                        self.advance();
                        literal.push('}');
                    } else {
                        self.errors.push(CompileError::new(
                            "Unmatched '}' in f-string".to_string(),
                            Span::new(start, self.current_pos),
                        ));
                    }
                }
                Some('\\') => {
                    self.advance();
                    match self.advance() {
                        Some('n') => literal.push('\n'),
                        Some('t') => literal.push('\t'),
                        Some('r') => literal.push('\r'),
                        Some('\\') => literal.push('\\'),
                        Some(q) if q == quote => literal.push(q),
                        Some(c) => {
                            literal.push('\\');
                            literal.push(c);
                        }
                        None => {
                            self.errors.push(CompileError::new(
                                "Unterminated escape in f-string".to_string(),
                                Span::new(start, self.current_pos),
                            ));
                            break;
                        }
                    }
                }
                Some('\n') => {
                    self.errors.push(CompileError::new(
                        "Unterminated f-string".to_string(),
                        Span::new(start, self.current_pos),
                    ));
                    break;
                }
                Some(c) => {
                    literal.push(c);
                    self.advance();
                }
            }
        }

        if !literal.is_empty() {
            parts.push(FStringPart::Literal(literal));
        }

        self.tokens.push(Token::new(
            TokenKind::FString(parts),
            Span::new(start, self.current_pos),
        ));
    }

    fn scan_fstring_expr(&mut self) -> String {
        let mut expr = String::new();
        let mut depth = 1; // We're already past the opening {

        while depth > 0 {
            match self.peek() {
                None => break,
                Some('{') => {
                    expr.push('{');
                    self.advance();
                    depth += 1;
                }
                Some('}') => {
                    depth -= 1;
                    if depth > 0 {
                        expr.push('}');
                    }
                    self.advance();
                }
                Some(c) => {
                    expr.push(c);
                    self.advance();
                }
            }
        }

        expr
    }

    fn scan_number(&mut self, start: usize, first: char) {
        let mut value = String::from(first);
        let mut is_float = false;

        // Integer part
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' {
                if c != '_' {
                    value.push(c);
                }
                self.advance();
            } else {
                break;
            }
        }

        // Decimal part
        if self.peek() == Some('.') {
            // Look ahead to ensure it's not `..` (range) or method call
            if self.peek_next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                is_float = true;
                value.push('.');
                self.advance(); // consume .
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() || c == '_' {
                        if c != '_' {
                            value.push(c);
                        }
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
        }

        // Exponent part
        if self.peek() == Some('e') || self.peek() == Some('E') {
            is_float = true;
            value.push('e');
            self.advance();
            if self.peek() == Some('+') || self.peek() == Some('-') {
                value.push(self.advance().unwrap());
            }
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    value.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
        }

        if is_float {
            match value.parse::<f64>() {
                Ok(f) => self.add_token(TokenKind::Float(f), start),
                Err(_) => {
                    self.errors.push(CompileError::new(
                        format!("Invalid float literal: {}", value),
                        Span::new(start, self.current_pos),
                    ));
                }
            }
        } else {
            match value.parse::<i64>() {
                Ok(i) => self.add_token(TokenKind::Int(i), start),
                Err(_) => {
                    self.errors.push(CompileError::new(
                        format!("Invalid integer literal: {}", value),
                        Span::new(start, self.current_pos),
                    ));
                }
            }
        }
    }

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

        let kind = match name.as_str() {
            "def" => TokenKind::Def,
            "async" => TokenKind::Async,
            "await" => TokenKind::Await,
            "class" => TokenKind::Class,
            "model" => TokenKind::Model,
            "trait" => TokenKind::Trait,
            "enum" => TokenKind::Enum,
            "type" => TokenKind::Type,
            "newtype" => TokenKind::Newtype,
            "import" => TokenKind::Import,
            "as" => TokenKind::As,
            "py" => TokenKind::Py,
            "from" => TokenKind::From,
            "with" => TokenKind::With,
            "extends" => TokenKind::Extends,
            "return" => TokenKind::Return,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "in" => TokenKind::In,
            "match" => TokenKind::Match,
            "case" => TokenKind::Case,
            "and" => TokenKind::And,
            "or" => TokenKind::Or,
            "not" => TokenKind::Not,
            "is" => TokenKind::Is,
            "true" | "True" => TokenKind::True,
            "false" | "False" => TokenKind::False,
            "None" => TokenKind::None,
            "let" => TokenKind::Let,
            "mut" => TokenKind::Mut,
            "self" => TokenKind::SelfKw,
            "pass" => TokenKind::Pass,
            "pub" => TokenKind::Pub,
            "super" => TokenKind::Super,
            "crate" => TokenKind::Crate,
            "yield" => TokenKind::Yield,
            "rust" => TokenKind::RustKw,
            _ => TokenKind::Ident(name),
        };

        self.add_token(kind, start);
    }
}

/// Check if a character can start an identifier
fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

/// Check if a character can continue an identifier
fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Convenience function to lex a source string
pub fn lex(source: &str) -> Result<Vec<Token>, Vec<CompileError>> {
    Lexer::new(source).tokenize()
}

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
}
