//! Integration tests for the Incan compiler frontend

use std::fs;
use std::path::Path;

use incan::frontend::{lexer, parser, typechecker};

/// Helper to run full pipeline on a source file
fn compile_file(path: &Path) -> Result<(), Vec<String>> {
    let source = fs::read_to_string(path).map_err(|e| vec![e.to_string()])?;
    
    let tokens = lexer::lex(&source).map_err(|errs| {
        errs.iter().map(|e| e.message.clone()).collect::<Vec<_>>()
    })?;
    
    let ast = parser::parse(&tokens).map_err(|errs| {
        errs.iter().map(|e| e.message.clone()).collect::<Vec<_>>()
    })?;
    
    typechecker::check(&ast).map_err(|errs| {
        errs.iter().map(|e| e.message.clone()).collect::<Vec<_>>()
    })?;
    
    Ok(())
}

/// Test that all valid fixtures compile successfully
#[test]
fn test_valid_fixtures() {
    let fixtures_dir = Path::new("tests/fixtures/valid");
    if !fixtures_dir.exists() {
        return; // Skip if fixtures not present
    }
    
    for entry in fs::read_dir(fixtures_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().map(|e| e == "incan").unwrap_or(false) {
            let result = compile_file(&path);
            assert!(
                result.is_ok(),
                "Expected {} to compile successfully, got errors: {:?}",
                path.display(),
                result.unwrap_err()
            );
        }
    }
}

/// Test that invalid fixtures produce errors
#[test]
fn test_invalid_fixtures() {
    let fixtures_dir = Path::new("tests/fixtures/invalid");
    if !fixtures_dir.exists() {
        return; // Skip if fixtures not present
    }
    
    for entry in fs::read_dir(fixtures_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().map(|e| e == "incan").unwrap_or(false) {
            let result = compile_file(&path);
            assert!(
                result.is_err(),
                "Expected {} to fail compilation, but it succeeded",
                path.display()
            );
        }
    }
}

/// Test specific lexer behavior
mod lexer_tests {
    use incan::frontend::lexer::{lex, TokenKind};

    #[test]
    fn test_rust_style_imports() {
        let tokens = lex("import foo::bar::baz as fb").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Import));
        assert!(matches!(&tokens[1].kind, TokenKind::Ident(s) if s == "foo"));
        assert!(matches!(tokens[2].kind, TokenKind::ColonColon));
        assert!(matches!(&tokens[3].kind, TokenKind::Ident(s) if s == "bar"));
        assert!(matches!(tokens[4].kind, TokenKind::ColonColon));
        assert!(matches!(&tokens[5].kind, TokenKind::Ident(s) if s == "baz"));
        assert!(matches!(tokens[6].kind, TokenKind::As));
        assert!(matches!(&tokens[7].kind, TokenKind::Ident(s) if s == "fb"));
    }

    #[test]
    fn test_try_operator() {
        let tokens = lex("result?").unwrap();
        assert!(matches!(&tokens[0].kind, TokenKind::Ident(s) if s == "result"));
        assert!(matches!(tokens[1].kind, TokenKind::Question));
    }

    #[test]
    fn test_fat_arrow() {
        let tokens = lex("x => y").unwrap();
        assert!(matches!(tokens[1].kind, TokenKind::FatArrow));
    }

    #[test]
    fn test_case_keyword() {
        let tokens = lex("case Some(x):").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Case));
    }

    #[test]
    fn test_pass_keyword() {
        let tokens = lex("pass").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Pass));
    }

    #[test]
    fn test_mut_self() {
        let tokens = lex("mut self").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Mut));
        assert!(matches!(tokens[1].kind, TokenKind::SelfKw));
    }

    #[test]
    fn test_fstring() {
        let tokens = lex(r#"f"Hello {name}""#).unwrap();
        assert!(matches!(&tokens[0].kind, TokenKind::FString(_)));
    }

    #[test]
    fn test_yield_keyword() {
        let tokens = lex("yield value").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Yield));
        assert!(matches!(&tokens[1].kind, TokenKind::Ident(s) if s == "value"));
    }

    #[test]
    fn test_rust_keyword() {
        let tokens = lex("import rust::serde_json").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Import));
        assert!(matches!(tokens[1].kind, TokenKind::RustKw));
        assert!(matches!(tokens[2].kind, TokenKind::ColonColon));
        assert!(matches!(&tokens[3].kind, TokenKind::Ident(s) if s == "serde_json"));
    }
}

/// End-to-end codegen tests
mod codegen_tests {
    use std::path::Path;
    use std::fs;
    use std::process::Command;
    use incan::frontend::{lexer, parser, typechecker};
    use incan::backend::codegen::RustCodegen;

    #[test]
    fn test_hello_world_codegen() {
        let path = Path::new("examples/hello.incn");
        if !path.exists() {
            return; // Skip if example not present
        }
        
        let source = fs::read_to_string(path).unwrap();
        let tokens = lexer::lex(&source).unwrap();
        let ast = parser::parse(&tokens).unwrap();
        typechecker::check(&ast).unwrap(); // Verify it type-checks
        let rust_code = RustCodegen::new().generate(&ast);
        
        // Verify the generated code contains expected elements
        assert!(rust_code.contains("fn main()"), "Should have main function");
        assert!(rust_code.contains("println!"), "Should have println macro");
        assert!(rust_code.contains("Hello from Incan!"), "Should have the message");
    }

    #[test]
    fn test_run_c_import_this() {
        let output = Command::new("target/debug/incan")
            .args(["run", "-c", "import this"])
            .output()
            .expect("failed to run incan");
        assert!(
            output.status.success(),
            "incan run -c import this failed: status={:?} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("The Zen of Incan") && stdout.contains("Readability counts"),
            "stdout missing zen line; got:\n{}",
            stdout
        );
    }
}

/// Test specific parser behavior
mod parser_tests {
    use incan::frontend::{lexer, parser};
    use incan::frontend::ast::*;

    fn parse_str(source: &str) -> Result<Program, ()> {
        let tokens = lexer::lex(source).map_err(|_| ())?;
        parser::parse(&tokens).map_err(|_| ())
    }

    #[test]
    fn test_model_with_decorator() {
        let source = r#"
@derive(Debug, Eq)
model User:
  name: str
"#;
        let program = parse_str(source).unwrap();
        match &program.declarations[0].node {
            Declaration::Model(m) => {
                assert_eq!(m.decorators.len(), 1);
                assert_eq!(m.decorators[0].node.name, "derive");
            }
            _ => panic!("Expected model"),
        }
    }

    #[test]
    fn test_class_with_traits() {
        let source = r#"
class Service with Loggable, Serializable:
  name: str
"#;
        let program = parse_str(source).unwrap();
        match &program.declarations[0].node {
            Declaration::Class(c) => {
                assert_eq!(c.traits.len(), 2);
                assert_eq!(c.traits[0], "Loggable");
                assert_eq!(c.traits[1], "Serializable");
            }
            _ => panic!("Expected class"),
        }
    }

    #[test]
    fn test_method_with_mut_self() {
        let source = r#"
class Counter:
  value: int = 0
  
  def inc(mut self) -> Unit:
    pass
"#;
        let program = parse_str(source).unwrap();
        match &program.declarations[0].node {
            Declaration::Class(c) => {
                assert_eq!(c.methods[0].node.receiver, Some(Receiver::Mutable));
            }
            _ => panic!("Expected class"),
        }
    }

    #[test]
    fn test_match_with_case() {
        let source = r#"
def foo(x: Option[int]) -> int:
  match x:
    case Some(n):
      return n
    case None:
      return 0
"#;
        let program = parse_str(source).unwrap();
        match &program.declarations[0].node {
            Declaration::Function(f) => {
                assert_eq!(f.body.len(), 1);
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_list_comprehension() {
        let source = r#"
def squares(nums: List[int]) -> List[int]:
  return [x * x for x in nums if x > 0]
"#;
        let program = parse_str(source).unwrap();
        assert_eq!(program.declarations.len(), 1);
    }

    #[test]
    fn test_generic_type() {
        let source = r#"
def foo() -> Result[int, str]:
  return Ok(42)
"#;
        let program = parse_str(source).unwrap();
        match &program.declarations[0].node {
            Declaration::Function(f) => {
                match &f.return_type.node {
                    Type::Generic(name, args) => {
                        assert_eq!(name, "Result");
                        assert_eq!(args.len(), 2);
                    }
                    _ => panic!("Expected generic type"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_yield_expression() {
        let source = r#"
def fixture() -> str:
  value = "test"
  yield value
"#;
        let program = parse_str(source).unwrap();
        match &program.declarations[0].node {
            Declaration::Function(f) => {
                assert_eq!(f.body.len(), 2);
                // Second statement should be the yield
                match &f.body[1].node {
                    Statement::Expr(expr) => {
                        match &expr.node {
                            Expr::Yield(Some(_)) => {} // Success
                            _ => panic!("Expected yield expression with value"),
                        }
                    }
                    _ => panic!("Expected expression statement"),
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_fixture_decorator() {
        let source = r#"
@fixture(scope="module")
def database() -> Database:
  db = connect()
  yield db
"#;
        let program = parse_str(source).unwrap();
        match &program.declarations[0].node {
            Declaration::Function(f) => {
                assert_eq!(f.decorators.len(), 1);
                assert_eq!(f.decorators[0].node.name, "fixture");
                assert!(!f.decorators[0].node.args.is_empty());
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_rust_crate_import() {
        let source = r#"import rust::serde_json as json"#;
        let program = parse_str(source).unwrap();
        match &program.declarations[0].node {
            Declaration::Import(i) => {
                match &i.kind {
                    ImportKind::RustCrate { crate_name, path } => {
                        assert_eq!(crate_name, "serde_json");
                        assert!(path.is_empty());
                    }
                    _ => panic!("Expected RustCrate import kind"),
                }
                assert_eq!(i.alias.as_deref(), Some("json"));
            }
            _ => panic!("Expected import"),
        }
    }

    #[test]
    fn test_rust_from_import() {
        let source = r#"from rust::time import Instant, Duration"#;
        let program = parse_str(source).unwrap();
        match &program.declarations[0].node {
            Declaration::Import(i) => {
                match &i.kind {
                    ImportKind::RustFrom { crate_name, path, items } => {
                        assert_eq!(crate_name, "time");
                        assert!(path.is_empty());
                        assert_eq!(items.len(), 2);
                        assert_eq!(items[0].name, "Instant");
                        assert_eq!(items[1].name, "Duration");
                    }
                    _ => panic!("Expected RustFrom import kind"),
                }
            }
            _ => panic!("Expected import"),
        }
    }
}

