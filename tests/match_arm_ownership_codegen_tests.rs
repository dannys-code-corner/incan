//! Generated-Rust regressions for ownership across mutually exclusive `match` arms.

use incan::backend::IrCodegen;
use incan::frontend::{lexer, parser};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Lower one source fixture through the ordinary native pipeline.
fn generate_rust(source: &str) -> Result<String, std::io::Error> {
    let tokens =
        lexer::lex(source).map_err(|errors| std::io::Error::other(format!("fixture did not lex: {errors:?}")))?;
    let program =
        parser::parse(&tokens).map_err(|errors| std::io::Error::other(format!("fixture did not parse: {errors:?}")))?;
    IrCodegen::new()
        .try_generate(&program)
        .map_err(|error| std::io::Error::other(format!("fixture did not codegen: {error:?}")))
}

/// Return whitespace-free generated Rust for stable ownership assertions.
fn compact_rust(source: &str) -> Result<String, std::io::Error> {
    Ok(generate_rust(source)?
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect())
}

#[test]
fn generic_value_moves_in_every_terminal_match_arm_without_clone_bound() -> TestResult {
    let rust = compact_rust(
        r#"
pub def select[T](value: T, choose_first: bool) -> T:
    match choose_first:
        true => return value
        false => return value
"#,
    )?;

    assert!(
        rust.contains("pubfnselect<T>(value:T,choose_first:bool)->T"),
        "terminal match arms must not narrow the generic signature with Clone:\n{rust}"
    );
    assert_eq!(
        rust.matches("returnvalue;").count(),
        2,
        "each mutually exclusive terminal arm must move the same value exactly once:\n{rust}"
    );
    assert!(
        !rust.contains("value.clone()"),
        "terminal match arms must not clone a value that no path uses afterwards:\n{rust}"
    );
    Ok(())
}

#[test]
fn generic_value_clones_in_match_arms_when_used_after_match() -> TestResult {
    let rust = compact_rust(
        r#"
pub def preserve[T](value: T, choose_first: bool) -> T:
    match choose_first:
        true =>
            first = value
        false =>
            second = value
    return value
"#,
    )?;

    assert!(
        rust.contains("pubfnpreserve<T:Clone>(value:T,choose_first:bool)->T"),
        "a value used after the match must retain the required Clone bound:\n{rust}"
    );
    assert_eq!(
        rust.matches("=value.clone();").count(),
        2,
        "each arm must preserve the value for its post-match use:\n{rust}"
    );
    assert!(
        rust.contains("returnvalue;"),
        "the post-match last use must still move the original value:\n{rust}"
    );
    Ok(())
}

#[test]
fn guarded_match_arms_retain_conservative_clone_planning() -> TestResult {
    let rust = compact_rust(
        r#"
pub def guarded[T](value: T, choose_first: bool, admit_first: bool) -> T:
    match choose_first:
        case true if admit_first: return value
        case _: return value
"#,
    )?;

    assert!(
        rust.contains("pubfnguarded<T:Clone>(value:T,choose_first:bool,admit_first:bool)->T"),
        "a guarded arm may fail before a later arm executes, so ownership planning must remain conservative:\n{rust}"
    );
    assert!(
        rust.contains("returnvalue.clone();"),
        "the guarded arm must not consume a value that the fallback arm may still need:\n{rust}"
    );
    assert!(
        rust.contains("returnvalue;"),
        "the final fallback arm may consume the value after the guarded arm is ruled out:\n{rust}"
    );
    Ok(())
}
