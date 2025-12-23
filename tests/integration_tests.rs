//! Integration tests for the Incan compiler frontend

use std::fs;
use std::path::Path;

use incan::frontend::{lexer, parser, typechecker};

/// Helper to run full pipeline on a source file
fn compile_file(path: &Path) -> Result<(), Vec<String>> {
    let source = fs::read_to_string(path).map_err(|e| vec![e.to_string()])?;

    let tokens = lexer::lex(&source).map_err(|errs| errs.iter().map(|e| e.message.clone()).collect::<Vec<_>>())?;

    let ast = parser::parse(&tokens).map_err(|errs| errs.iter().map(|e| e.message.clone()).collect::<Vec<_>>())?;

    typechecker::check(&ast).map_err(|errs| errs.iter().map(|e| e.message.clone()).collect::<Vec<_>>())?;

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
    use incan::frontend::lexer::{TokenKind, lex};

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
    use incan::backend::IrCodegen;
    use incan::frontend::{lexer, parser, typechecker};
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn rustc_compile_ok(source: &str) -> Result<(), String> {
        let mut dir = std::env::temp_dir();
        let uniq = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        dir.push(format!("incan_bench_smoke_{}", uniq));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

        let rs_path = dir.join("main.rs");
        let bin_path = dir.join("bin");
        std::fs::write(&rs_path, source).map_err(|e| e.to_string())?;

        let out = Command::new("rustc")
            .arg("--edition=2021")
            .arg(&rs_path)
            .arg("-o")
            .arg(&bin_path)
            .output()
            .map_err(|e| e.to_string())?;

        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).to_string())
        }
    }

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
        let rust_code = IrCodegen::new().generate(&ast);

        // Verify the generated code contains expected elements
        assert!(rust_code.contains("fn main()"), "Should have main function");
        assert!(rust_code.contains("println!"), "Should have println macro");
        assert!(rust_code.contains("Hello from Incan!"), "Should have the message");
    }

    #[test]
    fn test_run_c_import_this() {
        let output = Command::new("target/debug/incan")
            .args(["run", "-c", "import this"])
            // This test should not require network access. We expect the workspace dependencies to already be available
            // (the test suite built them)
            .env("CARGO_NET_OFFLINE", "true")
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

    #[test]
    fn test_benchmark_quicksort_codegen_compiles() {
        let path = Path::new("benchmarks/sorting/quicksort/quicksort.incn");
        if !path.exists() {
            return;
        }

        let source = fs::read_to_string(path).unwrap();
        let tokens = lexer::lex(&source).unwrap();
        let ast = parser::parse(&tokens).unwrap();
        typechecker::check(&ast).unwrap();

        let rust_code = IrCodegen::new().generate(&ast);

        // Regression: Vec::swap indices must be cast to usize.
        let mut ok = true;
        let mut search_from = 0usize;
        while let Some(pos) = rust_code[search_from..].find(".swap(") {
            let abs = search_from + pos;
            let window_end = (abs + 120).min(rust_code.len());
            let window = &rust_code[abs..window_end];
            if !window.contains("as usize") {
                ok = false;
                break;
            }
            search_from = abs + 5;
        }
        assert!(
            ok,
            "expected quicksort to cast swap indices to usize; generated:\n{}",
            rust_code
        );

        // Note: This test uses standalone rustc compilation, which can't access incan_stdlib/incan_derive.
        // Skip the compilation check if stdlib imports are present (models/classes with derives).
        if rust_code.contains("use incan_stdlib::prelude") || rust_code.contains("use incan_derive") {
            // Skip rustc compilation test for code that requires stdlib crates
            return;
        }

        rustc_compile_ok(&rust_code).expect("generated quicksort Rust failed to compile");
    }

    #[test]
    fn test_const_declarations_compile_and_run() {
        let output = Command::new("target/debug/incan")
            .args([
                "run",
                "-c",
                r#"
const PI: float = 3.14159
const APP_NAME: str = "Incan"
const MAGIC: int = 42
const ENABLED: bool = true
const RAW_DATA: bytes = b"\x00\x01\x02\x03"
const FROZEN_TEXT: FrozenStr = "frozen"
const NUMBERS: FrozenList[int] = [1, 2, 3, 4, 5]
const GREETING: str = "Hello World"

def main() -> None:
    print(PI)
    print(APP_NAME)
    print(MAGIC)
    print(ENABLED)
    print(RAW_DATA.len())
    print(FROZEN_TEXT.len())
    print(NUMBERS.len())
    print(GREETING)
"#,
            ])
            .env("CARGO_NET_OFFLINE", "true")
            .output()
            .expect("failed to run incan");

        assert!(
            output.status.success(),
            "const declarations test failed: status={:?} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("3.14159"), "PI const not emitted correctly");
        assert!(stdout.contains("Incan"), "APP_NAME const not emitted correctly");
        assert!(stdout.contains("42"), "MAGIC const not emitted correctly");
        assert!(stdout.contains("true"), "ENABLED const not emitted correctly");
        assert!(stdout.contains("4"), "RAW_DATA length incorrect");
        assert!(stdout.contains("6"), "FROZEN_TEXT length incorrect");
        assert!(stdout.contains("5"), "NUMBERS length incorrect");
        assert!(stdout.contains("Hello World"), "GREETING concat not working");
    }

    #[test]
    fn test_mixed_numeric_codegen_runs() {
        let output = Command::new("target/debug/incan")
            .args([
                "run",
                "-c",
                r#"
def main() -> None:
    size: int = 2
    x: float = 3.0
    result = 2.0 * x / size
    println(result)
"#,
            ])
            .env("CARGO_NET_OFFLINE", "true")
            .output()
            .expect("failed to run incan");

        assert!(
            output.status.success(),
            "mixed numeric run failed: status={:?} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains('3'),
            "mixed numeric output missing expected result; stdout={}",
            stdout
        );
    }
}

/// Test specific parser behavior
mod parser_tests {
    use incan::frontend::ast::*;
    use incan::frontend::{lexer, parser};

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
            Declaration::Function(f) => match &f.return_type.node {
                Type::Generic(name, args) => {
                    assert_eq!(name, "Result");
                    assert_eq!(args.len(), 2);
                }
                _ => panic!("Expected generic type"),
            },
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
            Declaration::Import(i) => match &i.kind {
                ImportKind::RustFrom {
                    crate_name,
                    path,
                    items,
                } => {
                    assert_eq!(crate_name, "time");
                    assert!(path.is_empty());
                    assert_eq!(items.len(), 2);
                    assert_eq!(items[0].name, "Instant");
                    assert_eq!(items[1].name, "Duration");
                }
                _ => panic!("Expected RustFrom import kind"),
            },
            _ => panic!("Expected import"),
        }
    }
}
