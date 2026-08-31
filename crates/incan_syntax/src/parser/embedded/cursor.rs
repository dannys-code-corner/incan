// Shared raw-text scanning cursor for embedded-fragment submode tokenizers (RFC 081, `#1023`).

/// Byte-position cursor over one embedded fragment's raw source slice.
///
/// Every submode tokenizer (`markup.rs`, `style.rs`, `raw_text.rs`, `regex_template.rs`,
/// `selector_declaration_value.rs`, `type_position.rs`) scans its fragment with one of these rather than going
/// through the ordinary lexer's `TokenKind` alphabet, because submode token forms (`#1166ff`, `16px`, `<section>`,
/// `/pattern/flags`) have no faithful decomposition into ordinary Incan tokens. `base_offset` is the fragment's
/// starting absolute byte offset in the original source file, so every span this cursor reports anchors back to
/// real source coordinates for diagnostics and tooling.
struct EmbeddedCursor<'src> {
    text: &'src str,
    pos: usize,
    base_offset: usize,
}

impl<'src> EmbeddedCursor<'src> {
    /// Create a cursor over `text`, whose first byte sits at `base_offset` in the original source file.
    fn new(text: &'src str, base_offset: usize) -> Self {
        Self {
            text,
            pos: 0,
            base_offset,
        }
    }

    /// Return whether the cursor has consumed the whole fragment.
    fn is_eof(&self) -> bool {
        self.pos >= self.text.len()
    }

    /// Return the remaining unconsumed text.
    fn remaining(&self) -> &'src str {
        &self.text[self.pos..]
    }

    /// Return the next character without consuming it.
    fn peek(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    /// Return the character one position past the next character, without consuming either.
    fn peek2(&self) -> Option<char> {
        let mut chars = self.remaining().chars();
        chars.next()?;
        chars.next()
    }

    /// Consume and return the next character.
    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// Return whether the remaining text starts with the exact literal `pattern`.
    fn starts_with(&self, pattern: &str) -> bool {
        self.remaining().starts_with(pattern)
    }

    /// Consume the exact literal `pattern` if the remaining text starts with it.
    fn eat_str(&mut self, pattern: &str) -> bool {
        if self.starts_with(pattern) {
            self.pos += pattern.len();
            true
        } else {
            false
        }
    }

    /// Consume the next character if it satisfies `predicate`, returning whether it did.
    fn eat_if(&mut self, predicate: impl FnOnce(char) -> bool) -> bool {
        match self.peek() {
            Some(c) if predicate(c) => {
                self.advance();
                true
            }
            _ => false,
        }
    }

    /// Consume a run of consecutive characters satisfying `predicate`, returning the consumed text.
    fn eat_while(&mut self, mut predicate: impl FnMut(char) -> bool) -> &'src str {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if !predicate(c) {
                break;
            }
            self.advance();
        }
        &self.text[start..self.pos]
    }

    /// Consume ASCII space/tab/newline whitespace.
    fn skip_ws(&mut self) {
        self.eat_while(|c| c.is_whitespace());
    }

    /// Return the absolute source offset of the cursor's current position.
    fn abs_pos(&self) -> usize {
        self.base_offset + self.pos
    }

    /// Return the absolute-coordinate span from local byte offset `local_start` to the cursor's current position.
    fn span_from(&self, local_start: usize) -> Span {
        Span::new(self.base_offset + local_start, self.abs_pos())
    }

    /// Return the absolute-coordinate span covering the whole fragment this cursor scans.
    fn full_span(&self) -> Span {
        Span::new(self.base_offset, self.base_offset + self.text.len())
    }
}

/// Return whether `c` may start or continue an embedded-submode bare identifier (ASCII letters, digits, `_`, `-`).
///
/// Hyphens are included because CSS-shaped identifiers (`accent-color`, custom properties' `--name` suffix,
/// selector class/pseudo names) and markup attribute/tag names commonly contain them; ordinary Incan identifiers do
/// not use `-`, so this is intentionally more permissive than `incan_syntax`'s own identifier rules.
fn is_embedded_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Return whether `c` may start an embedded-submode bare identifier.
fn is_embedded_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

/// Parse a brace-delimited expression hole `{expr}`, re-entering ordinary Incan parsing for its contents.
///
/// Shared across submodes that use bare `{`/`}` as the hole delimiter (`Markup` content/attribute holes, style and
/// selector/declaration-value holes, and `RawText` holes). The closing `}` is found by brace-depth counting, the
/// same simplification the existing f-string interpolation lexer already uses (`Lexer::scan_fstring_expr` in
/// `lexer/strings.rs`) — it is not string-literal-aware, so a `}` inside a nested string literal within the hole
/// would close the hole early. This is an accepted, pre-existing simplification, not a new regression.
///
/// Assumes `cursor` is currently positioned at the opening `{`.
fn parse_embedded_brace_hole(cursor: &mut EmbeddedCursor<'_>) -> Result<Spanned<EmbeddedNode>, CompileError> {
    let start = cursor.pos;
    cursor.eat_str("{");
    let inner_start = cursor.pos;
    let mut depth = 1u32;
    loop {
        match cursor.peek() {
            None => {
                return Err(CompileError::syntax(
                    "Unterminated expression hole: expected `}`".to_string(),
                    cursor.span_from(start),
                ));
            }
            Some('{') => {
                cursor.advance();
                depth += 1;
            }
            Some('}') => {
                cursor.advance();
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Some(_) => {
                cursor.advance();
            }
        }
    }
    let inner_end = cursor.pos - 1;
    let inner_text = &cursor.text[inner_start..inner_end];
    let expr = parse_embedded_hole_expr(inner_text, cursor.base_offset + inner_start)?;
    Ok(Spanned::new(
        EmbeddedNode::Hole(Box::new(expr)),
        cursor.span_from(start),
    ))
}
