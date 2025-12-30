/// Parse a token stream into an AST [`Program`].
///
/// This is the main public entrypoint for parsing.
///
/// ## Parameters
/// - `tokens`: Token stream produced by `incan_syntax::lexer`.
///
/// ## Errors
/// Returns `Err(Vec<CompileError>)` if parsing fails.
#[tracing::instrument(skip_all, fields(token_count = tokens.len()))]
pub fn parse(tokens: &[Token]) -> Result<Program, Vec<CompileError>> {
    Parser::new(tokens).parse()
}
