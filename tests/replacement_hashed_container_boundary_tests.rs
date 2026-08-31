//! Legacy-boundary evidence for #1247's hashed-container representation slice.
//!
//! These pins record where set membership refuses *today*: at the aggregate, for want of a set value in the
//! replacement executor — not for want of a helper, which Body IR already names. They are the set-side siblings of
//! `replacement_cannot_reach_dict_membership_because_it_has_no_dict_value` in
//! `replacement_backend_execution_tests.rs`, which stays untouched as the dict-side pin. The
//! `backend::replacement::hashed` module now holds the representation that moves this boundary; once the #1247
//! integration work wires it into `ReplacementValue` and the membership arms, these pins and the dict-side one are
//! meant to be replaced by execution coverage, per the issue's acceptance criteria.

use incan::backend::replacement::execute_free_function;
use incan::frontend::body_ir::build_body_ir_module_v0;
use incan::frontend::typechecker::TypeChecker;
use incan::frontend::{lexer, parser};
use incan_semantics_core::body_ir::BodyIrModule;

/// Lower one self-contained, typechecked source module into the Body IR the replacement backend consumes.
///
/// Same harness shape as `replacement_backend_execution_tests.rs`: the pins must go through the real pipeline so
/// the refusal they record is the one a real program hits.
fn lower_typed_body_ir(source: &str) -> Result<BodyIrModule, Box<dyn std::error::Error>> {
    let tokens = lexer::lex(source).map_err(|errors| std::io::Error::other(format!("{errors:?}")))?;
    let program = parser::parse(&tokens).map_err(|errors| std::io::Error::other(format!("{errors:?}")))?;
    let module_path = vec!["replacement_hashed_boundary".to_string()];
    let mut checker = TypeChecker::new();
    checker.set_current_module_path(Some(module_path.clone()));
    checker
        .check_program(&program)
        .map_err(|errors| std::io::Error::other(format!("{errors:?}")))?;
    Ok(build_body_ir_module_v0(&program, &module_path, checker.type_info()))
}

#[test]
fn replacement_cannot_reach_set_membership_because_it_has_no_set_value() -> Result<(), Box<dyn std::error::Error>> {
    // The honest boundary, set edition: `set_contains`/`set_not_contains` exist in `incan_stdlib`, and Body IR
    // emits calls to them, but the executor has no set value, so the refusal lands on the aggregate before either
    // membership form is reached. Both spellings sit in one program so the pin also records that lowering
    // represents each source operator as its own helper call.
    let source = r#"
def main() -> bool:
  xs = {1, 2}
  if 3 not in xs:
    return 1 in xs
  return false
"#;
    let module = lower_typed_body_ir(source)?;
    let Err(error) = execute_free_function(&module, "main", &[]) else {
        return Err("set membership must refuse until the executor has a set value".into());
    };

    let reported = format!("{error:?}");
    assert!(
        reported.contains("set aggregate"),
        "the refusal must name the value kind it lacks: {reported}"
    );
    let snapshot = module.render_snapshot();
    for helper in ["set_contains", "set_not_contains"] {
        assert!(
            snapshot.contains(&format!("call helper:{helper}(")),
            "Body IR must still represent the {helper} membership the executor cannot run: {snapshot}"
        );
    }
    Ok(())
}
