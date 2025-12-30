/// Parser core types and entrypoint.
///
/// This chunk defines the [`Parser`] type and its top-level `parse()` entrypoint.
/// It also contains a few small internal helper types shared across the other
/// parser chunks.
///
/// ## Notes
/// - This file is `include!`'d into `crate::parser` to keep all parser methods in a
///   single module while avoiding a single “god file”.
type FieldsAndMethods = (Vec<Spanned<FieldDecl>>, Vec<Spanned<MethodDecl>>);

/// Result of parsing `[...]` postfix syntax: either a single index or a slice.
enum IndexOrSlice {
    Index(Spanned<Expr>),
    Slice(SliceExpr),
}

/// Parser state.
///
/// ## Notes
/// - The parser is intentionally single-pass and recovers from errors where possible by
///   synchronizing at statement/declaration boundaries.
/// - Most parsing helpers are implemented on `Parser` but split across multiple files.
pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    errors: Vec<CompileError>,
}

impl<'a> Parser<'a> {
    /// Create a new parser for a token stream.
    ///
    /// ## Parameters
    /// - `tokens`: Token stream produced by `incan_syntax::lexer`.
    pub fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            pos: 0,
            errors: Vec::new(),
        }
    }

    /// Parse the entire token stream into a [`Program`].
    ///
    /// ## Errors
    /// Returns a list of [`CompileError`]s if parsing fails. The parser attempts
    /// to recover and continue after an error to report multiple issues in one pass.
    pub fn parse(mut self) -> Result<Program, Vec<CompileError>> {
        let mut declarations = Vec::new();

        // Skip leading newlines
        self.skip_newlines();
        // Stray top-level DEDENT can appear after error recovery (e.g. unexpected indentation).
        // Ignore it at the module level to avoid cascaded errors.
        self.skip_dedents();

        while !self.is_at_end() {
            match self.declaration() {
                Ok(decl) => declarations.push(decl),
                Err(e) => {
                    self.errors.push(e);
                    self.synchronize();
                }
            }
            self.skip_newlines();
            // Same rationale as above: at the module level we should not see DEDENT tokens,
            // but the lexer may emit them and recovery may leave us positioned on them.
            self.skip_dedents();
        }

        if self.errors.is_empty() {
            Ok(Program { declarations })
        } else {
            Err(self.errors)
        }
    }
}
