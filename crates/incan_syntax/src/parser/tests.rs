#[cfg(test)]
/// Parser unit tests.
///
/// These tests focus on correctness of specific syntactic forms and on the parser’s
/// error recovery behavior (avoiding cascaded errors).
mod tests {
    use super::*;
    use crate::lexer;

    fn parse_str(source: &str) -> Result<Program, Vec<CompileError>> {
        let tokens = lexer::lex(source).map_err(|_| vec![])?;
        parse(&tokens)
    }

    #[test]
    fn test_unexpected_indent_at_toplevel_is_single_clear_error() {
        // We intentionally allow the lexer to emit INDENT/DEDENT tokens at the top-level.
        // The parser should produce a single clear error and avoid cascading failures.
        let source = "  x = 1\n";
        let err = parse_str(source).expect_err("Top-level indentation should be rejected by the parser");
        assert_eq!(err.len(), 1, "Parser should return exactly one error (no cascade)");
        assert!(
            err[0].message.contains("Expected declaration") && err[0].message.contains("Indent"),
            "Error message should clearly indicate the unexpected INDENT token; got: {}",
            err[0].message
        );
    }

    #[test]
    fn test_parse_model() {
        let source = r#"
model User:
  name: str
  age: int = 0
"#;
        let program = parse_str(source).unwrap();
        assert_eq!(program.declarations.len(), 1);
        match &program.declarations[0].node {
            Declaration::Model(m) => {
                assert_eq!(m.name, "User");
                assert_eq!(m.fields.len(), 2);
            }
            _ => panic!("Expected model"),
        }
    }

    #[test]
    fn test_parse_function() {
        let source = r#"
def add(a: int, b: int) -> int:
  return a + b
"#;
        let program = parse_str(source).unwrap();
        assert_eq!(program.declarations.len(), 1);
        match &program.declarations[0].node {
            Declaration::Function(f) => {
                assert_eq!(f.name, "add");
                assert_eq!(f.params.len(), 2);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_parse_import() {
        let source = "import polars::prelude as pl";
        let program = parse_str(source).unwrap();
        assert_eq!(program.declarations.len(), 1);
        match &program.declarations[0].node {
            Declaration::Import(i) => {
                match &i.kind {
                    ImportKind::Module(path) => {
                        assert_eq!(path.segments, vec!["polars".to_string(), "prelude".to_string()]);
                        assert_eq!(path.parent_levels, 0);
                        assert!(!path.is_absolute);
                    }
                    _ => panic!("Expected module import"),
                }
                assert_eq!(i.alias, Some("pl".to_string()));
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_parse_match() {
        let source = r#"
def handle(opt: Option[int]) -> int:
  match opt:
    case Some(x):
      return x
    case None:
      return 0
"#;
        let program = parse_str(source).unwrap();
        assert_eq!(program.declarations.len(), 1);
    }

    #[test]
    fn test_parse_const_decl() {
        let source = r#"
const ANSWER: int = 42
"#;
        let program = parse_str(source).unwrap();
        assert_eq!(program.declarations.len(), 1);
        match &program.declarations[0].node {
            Declaration::Const(c) => {
                assert_eq!(c.name, "ANSWER");
            }
            _ => panic!("Expected const"),
        }
    }
}
