//! End-to-end proof for the bounded #988 Body-IR replacement executor.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use incan::backend::replacement::{ReplacementValue, execute_free_function};
use incan::backend::selection::{
    BackendKind, FallbackOutcome, FallbackPolicy, ShadowComparisonState, digest_output, finalize_receipt,
    select_backend,
};
use incan::frontend::body_ir::build_body_ir_module_v0;
use incan::frontend::typechecker::TypeChecker;
use incan::frontend::{lexer, parser};
use incan_semantics_core::body_ir::{BodyIrModule, OwnershipFact};

/// Lower one self-contained, typechecked source module into the Body IR the replacement backend consumes.
fn lower_typed_body_ir(source: &str) -> Result<BodyIrModule, Box<dyn std::error::Error>> {
    let tokens = lexer::lex(source).map_err(|errors| std::io::Error::other(format!("{errors:?}")))?;
    let program = parser::parse(&tokens).map_err(|errors| std::io::Error::other(format!("{errors:?}")))?;
    let module_path = vec!["replacement_execution".to_string()];
    let mut checker = TypeChecker::new();
    checker.set_current_module_path(Some(module_path.clone()));
    checker
        .check_program(&program)
        .map_err(|errors| std::io::Error::other(format!("{errors:?}")))?;
    Ok(build_body_ir_module_v0(&program, &module_path, checker.type_info()))
}

/// Locate the compiler binary built for this integration-test invocation.
fn incan_binary() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_incan")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/incan"))
}

/// Execute typed source through Body IR and bind the observed result to an explicit replacement receipt.
#[test]
fn replacement_executes_core_body_and_binds_a_real_receipt() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
def main() -> int:
  left = 40
  right = 2
  if left > right:
    return left + right
  return right
"#;
    let module = lower_typed_body_ir(source)?;
    let execution = execute_free_function(&module, "main", &[])?;

    assert_eq!(execution.value, ReplacementValue::Int(42));
    assert!(execution.body_snapshot.contains("body main"));
    assert!(execution.body_snapshot.contains("span="));
    assert!(
        execution
            .ownership_reads
            .iter()
            .all(|read| !matches!(read.fact, OwnershipFact::Unknown)),
        "the selected core corpus must not erase ownership facts: {:?}",
        execution.ownership_reads
    );

    let selection = select_backend(
        BackendKind::Replacement,
        true,
        false,
        digest_output(&[source]),
        FallbackPolicy::Refuse,
    );
    let receipt = finalize_receipt(
        &selection,
        BackendKind::Replacement,
        execution.output_identity.clone(),
        ShadowComparisonState::NotRequested,
        incan::frontend::diagnostics::DIAGNOSTIC_SCHEMA_VERSION,
    )?;
    receipt.verify_identity()?;
    assert_eq!(receipt.executed_backend, BackendKind::Replacement);
    assert_eq!(receipt.fallback_outcome, FallbackOutcome::NotNeeded);
    assert_eq!(receipt.selection.source_identity, digest_output(&[source]));
    let ownership_evidence = execution.ownership_evidence();
    let runtime_evidence = execution.runtime_requirement_evidence();
    assert!(
        ownership_evidence
            .iter()
            .all(|read| !read.fact.is_empty() && read.span_end >= read.span_start),
        "canonical ownership evidence must retain stable facts and spans: {ownership_evidence:?}"
    );
    assert_eq!(
        serde_json::to_string(&runtime_evidence)?,
        serde_json::to_string(&execution.runtime_requirement_evidence())?,
        "runtime-requirement evidence must be deterministic across repeated projections"
    );
    let repeated = execute_free_function(&module, "main", &[])?;
    assert_eq!(execution.output_identity, repeated.output_identity);
    assert_eq!(execution.ownership_evidence(), repeated.ownership_evidence());
    assert_eq!(
        execution.runtime_requirement_evidence(),
        repeated.runtime_requirement_evidence()
    );
    Ok(())
}

/// Reject a list aggregate before the first replacement profile can execute a compound local value.
#[test]
fn replacement_refuses_unsupported_body_ir_with_the_original_source_span() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
def main() -> int:
  values = [1, 2]
  return 0
"#;
    let module = lower_typed_body_ir(source)?;
    let error = match execute_free_function(&module, "main", &[]) {
        Ok(execution) => {
            return Err(format!(
                "list aggregates are outside #988's core profile but executed as {:?}",
                execution.value
            )
            .into());
        }
        Err(error) => error,
    };

    assert!(
        error.primary_span().is_some(),
        "unsupported execution must retain an Incan source span: {error}"
    );
    let expected_start = source
        .find("[1, 2]")
        .ok_or("aggregate fixture must contain its list assignment")?;
    let expected_end = expected_start + "[1, 2]".len();
    let span = error
        .primary_span()
        .ok_or("aggregate refusal must retain its original source span")?;
    assert_eq!(span.start, expected_start);
    assert_eq!(span.end, expected_end);
    assert!(
        error.to_string().contains("list aggregate"),
        "unsupported operation must be named visibly: {error}"
    );
    Ok(())
}

/// Refuse a list aggregate even when it appears directly in the return expression.
#[test]
fn replacement_refuses_list_aggregate_return_values() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
def main() -> list[int]:
  return [1]
"#;
    let module = lower_typed_body_ir(source)?;
    let error = match execute_free_function(&module, "main", &[]) {
        Ok(execution) => {
            return Err(format!(
                "a list aggregate is outside #988's scalar profile but executed as {:?}",
                execution.value
            )
            .into());
        }
        Err(error) => error,
    };

    assert!(
        error.primary_span().is_some(),
        "list aggregate refusal must retain an Incan source span: {error}"
    );
    assert!(
        error.to_string().contains("list aggregate"),
        "list aggregate must be named visibly: {error}"
    );
    Ok(())
}

/// Materialize a lazy generator expression in the replacement executor without evaluating its filter or element
/// while constructing the Body-IR value. The selected profile admits only the explicit `.collect()` consumer and
/// indexes its scalar result; it must not treat the generator rvalue itself as an eager list or use generated Rust.
#[test]
fn replacement_executes_a_lazy_generator_expression_only_when_collect_consumes_it()
-> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
def main() -> int:
  values = (value * 10 for value in range(1, 5) if value > 2).collect()
  return values[0] + values[1]
"#;
    let module = lower_typed_body_ir(source)?;
    let execution = execute_free_function(&module, "main", &[])?;
    assert_eq!(execution.value, ReplacementValue::Int(70));
    assert!(
        execution.body_snapshot.contains("generator(source="),
        "replacement execution must consume the explicit deferred generator rvalue"
    );
    assert!(
        execution.body_snapshot.contains("yield"),
        "the Body-IR proof must retain a yield rather than materializing an eager list"
    );
    Ok(())
}

/// Constructing a generator must not evaluate its deferred element body. The division would fail if the replacement
/// executor accidentally treated the rvalue as an eager collection; dropping the unconsumed value must still return
/// the surrounding scalar result.
#[test]
fn replacement_keeps_an_unconsumed_generator_expression_deferred() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
def main() -> int:
  unused = (value // 0 for value in range(1, 2))
  return 7
"#;
    let module = lower_typed_body_ir(source)?;
    let execution = execute_free_function(&module, "main", &[])?;
    assert_eq!(execution.value, ReplacementValue::Int(7));
    assert!(
        execution.body_snapshot.contains("yield ") && execution.body_snapshot.contains("const(0)"),
        "the deferred body must remain represented even though it was never consumed"
    );
    Ok(())
}

/// Execute the selected plain builtin-collection loop over scalar tuple values.
#[test]
fn replacement_executes_plain_scalar_tuple_collection_loop() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
def main() -> int:
  pairs = [(1, 2), (3, 4)]
  for pair in pairs:
    if false:
      return 0
  return 7
"#;
    let module = lower_typed_body_ir(source)?;
    let execution = execute_free_function(&module, "main", &[])?;
    assert_eq!(execution.value, ReplacementValue::Int(7));
    assert!(execution.body_snapshot.contains("iter_next("));
    Ok(())
}

/// Execute `for a, b in pairs` directly and bind the actual result to a replacement receipt.
#[test]
fn replacement_executes_scalar_tuple_collection_destructuring_with_a_replacement_receipt()
-> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
def main() -> int:
  pairs = [(1, 2), (4, 5)]
  for a, b in pairs:
    if a == 4:
      return a * 10 + b
  return 0
"#;
    let module = lower_typed_body_ir(source)?;
    let execution = execute_free_function(&module, "main", &[])?;
    assert_eq!(execution.value, ReplacementValue::Int(45));
    assert!(execution.body_snapshot.contains(".0"));
    assert!(execution.body_snapshot.contains(".1"));

    let selection = select_backend(
        BackendKind::Replacement,
        true,
        false,
        digest_output(&[source]),
        FallbackPolicy::Refuse,
    );
    let receipt = finalize_receipt(
        &selection,
        BackendKind::Replacement,
        execution.output_identity,
        ShadowComparisonState::NotRequested,
        incan::frontend::diagnostics::DIAGNOSTIC_SCHEMA_VERSION,
    )?;
    receipt.verify_identity()?;
    assert_eq!(receipt.executed_backend, BackendKind::Replacement);
    assert_eq!(receipt.fallback_outcome, FallbackOutcome::NotNeeded);
    Ok(())
}

/// Refuse an index projection outside the selected tuple-field loop profile at its original source span.
#[test]
fn replacement_refuses_collection_index_projection_with_the_original_source_span()
-> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
def main() -> int:
  pairs = [(1, 2)]
  return pairs[0][0]
"#;
    let module = lower_typed_body_ir(source)?;
    let error = match execute_free_function(&module, "main", &[]) {
        Ok(execution) => return Err(format!("index projection executed as {:?}", execution.value).into()),
        Err(error) => error,
    };
    let expected_start = source
        .find("return pairs[0][0]")
        .ok_or("index-projection fixture must contain its source statement")?;
    let span = error
        .primary_span()
        .ok_or("collection index-projection refusal must retain its source span")?;
    assert_eq!(span.start, expected_start);
    assert!(
        error.to_string().contains("nested place projection"),
        "collection index projection must remain visible: {error}"
    );
    Ok(())
}

/// Refuse a standalone tuple before it can be used outside the selected collection profile.
#[test]
fn replacement_refuses_standalone_tuple_with_the_original_source_span() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
def main() -> int:
  pair = (1, 2)
  return pair.0
"#;
    let module = lower_typed_body_ir(source)?;
    let error = match execute_free_function(&module, "main", &[]) {
        Ok(execution) => return Err(format!("standalone tuple executed as {:?}", execution.value).into()),
        Err(error) => error,
    };
    let expected_start = source
        .find("(1, 2)")
        .ok_or("standalone-tuple fixture must contain its source tuple")?;
    let span = error
        .primary_span()
        .ok_or("standalone tuple refusal must retain its source span")?;
    assert_eq!(span.start, expected_start);
    assert!(
        error
            .to_string()
            .contains("tuple aggregate outside the two-scalar collection-element profile"),
        "standalone tuple must remain visibly outside the selected collection profile: {error}"
    );
    Ok(())
}

/// Refuse a lexical shadowing shape until Body IR carries a binding-equivalence fact the direct executor can honor.
#[test]
fn replacement_refuses_lexically_shadowed_bindings() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
def main() -> int:
  x = 1
  if true:
    let x = 2
  return x
"#;
    let module = lower_typed_body_ir(source)?;
    let error = match execute_free_function(&module, "main", &[]) {
        Ok(execution) => {
            return Err(format!(
                "lexically shadowed source must refuse instead of executing as {:?}",
                execution.value
            )
            .into());
        }
        Err(error) => error,
    };
    let expected_start = source
        .find("let x = 2")
        .ok_or("shadowing fixture must contain the inner binding")?;
    let span = error
        .primary_span()
        .ok_or("shadowing refusal must retain the inner binding span")?;
    assert_eq!(span.start, expected_start);
    assert!(error.to_string().contains("lexical shadowing"));
    Ok(())
}

/// Refuse same-scope reassignment until Body IR carries a binding-equivalence fact the executor can honor.
#[test]
fn replacement_refuses_reassignment_with_a_repeated_user_binding_name() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
def main() -> int:
  mut x = 1
  x = 2
  return x
"#;
    let module = lower_typed_body_ir(source)?;
    let error = match execute_free_function(&module, "main", &[]) {
        Ok(execution) => {
            return Err(format!(
                "reassignment must refuse until binding equivalence is available, but executed as {:?}",
                execution.value
            )
            .into());
        }
        Err(error) => error,
    };
    let expected_start = source
        .rfind("x = 2")
        .ok_or("reassignment fixture must contain the second binding")?;
    let span = error
        .primary_span()
        .ok_or("reassignment refusal must retain the second binding span")?;
    assert_eq!(span.start, expected_start);
    assert!(error.to_string().contains("repeated user binding"));
    assert!(error.to_string().contains("reassignment"));
    Ok(())
}

/// Match Incan's Python-style signed integer division and modulo contract through the replacement executor.
#[test]
fn replacement_uses_python_signed_floor_division_and_modulo() -> Result<(), Box<dyn std::error::Error>> {
    let division = lower_typed_body_ir(
        r#"def main() -> int:
  return 7 // -3
"#,
    )?;
    let modulo = lower_typed_body_ir(
        r#"def main() -> int:
  return 7 % -3
"#,
    )?;

    assert_eq!(
        execute_free_function(&division, "main", &[])?.value,
        ReplacementValue::Int(-3)
    );
    assert_eq!(
        execute_free_function(&modulo, "main", &[])?.value,
        ReplacementValue::Int(-2)
    );
    Ok(())
}

/// Execute the remaining source-only cases selected for the first #988 proof corpus.
#[test]
fn replacement_executes_the_selected_string_ownership_control_flow_and_assertion_cases()
-> Result<(), Box<dyn std::error::Error>> {
    let cases = [
        (
            "string concatenation",
            r#"def main() -> str:
  name = "Ada"
  return "hi " + name
"#,
            ReplacementValue::Str("hi Ada".to_string()),
            "helper:str_concat",
        ),
        (
            "non-copy return",
            r#"def main() -> str:
  value = "owned"
  return value
"#,
            ReplacementValue::Str("owned".to_string()),
            "move(_",
        ),
        (
            "normalized range and while control flow",
            r#"def main() -> int:
  for value in range(1, 5):
    if value % 2 == 0:
      continue
  while false:
    return 0
  return 10
"#,
            ReplacementValue::Int(10),
            "loop:",
        ),
        (
            "assertion and floor division",
            r#"def main() -> int:
  a = 84
  b = 2
  assert b != 0
  return a // b
"#,
            ReplacementValue::Int(42),
            "assert",
        ),
    ];
    for (name, source, expected, snapshot_evidence) in cases {
        let module = lower_typed_body_ir(source)?;
        let execution = execute_free_function(&module, "main", &[]).map_err(|error| {
            format!(
                "replacement execution failed for {name}: {error}\n{}",
                module.render_snapshot()
            )
        })?;
        assert_eq!(execution.value, expected, "replacement result diverged for {name}");
        assert!(
            execution.body_snapshot.contains(snapshot_evidence),
            "Body IR proof for {name} omitted `{snapshot_evidence}`:\n{}",
            execution.body_snapshot
        );
    }

    let failing_assertion = lower_typed_body_ir(
        r#"def main() -> int:
  a = 84
  b = 0
  assert b != 0
  return a // b
"#,
    )?;
    let error = match execute_free_function(&failing_assertion, "main", &[]) {
        Ok(execution) => return Err(format!("failing assertion executed as {:?}", execution.value).into()),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("assertion failed"),
        "unexpected assertion outcome: {error}"
    );
    assert!(
        error.primary_span().is_some(),
        "assertion failure lost its source span: {error}"
    );
    Ok(())
}

/// Run the explicit replacement CLI path without allowing a legacy fallback.
#[test]
fn replacement_cli_executes_typed_body_ir_and_persists_a_replacement_receipt() -> Result<(), Box<dyn std::error::Error>>
{
    let temporary = tempfile::tempdir()?;
    let entrypoint = temporary.path().join("main.incn");
    fs::write(
        &entrypoint,
        r#"def main() -> int:
  left = 40
  right = 2
  if left > right:
    return left + right
  return right
"#,
    )?;

    let output = Command::new(incan_binary())
        .args([
            "build",
            entrypoint.to_string_lossy().as_ref(),
            "--backend",
            "replacement",
            "--backend-fallback",
            "refuse",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "replacement build must execute directly. stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("replacement backend executed `main`: 42"),
        "unexpected replacement output: {stdout}"
    );
    assert!(
        !stdout.contains("Generated Rust project"),
        "replacement path must not enter Rust generation: {stdout}"
    );
    assert!(
        !temporary.path().join("target/incan").exists(),
        "direct replacement execution must not create a legacy generated-project directory"
    );

    let receipt_path = temporary.path().join(".incan/backend/receipt.json");
    let receipt: serde_json::Value = serde_json::from_str(&fs::read_to_string(receipt_path)?)?;
    assert_eq!(receipt["executed_backend"], "replacement");
    assert_eq!(receipt["selection"]["selected_backend"], "replacement");
    assert_eq!(receipt["fallback_outcome"], serde_json::json!("not_needed"));
    assert!(
        receipt["replacement_execution"].is_null(),
        "persisted #986 receipts must remain receipt-only; execution evidence belongs to the report payload"
    );
    assert!(
        receipt["identity"]
            .as_str()
            .is_some_and(|identity| identity.starts_with("sha256:"))
    );
    Ok(())
}

/// Execute a typed empty scalar-pair list directly and publish only replacement receipt evidence.
#[test]
fn replacement_cli_executes_typed_empty_scalar_tuple_list_with_a_replacement_receipt()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let entrypoint = temporary.path().join("main.incn");
    fs::write(
        &entrypoint,
        r#"def main() -> int:
  values: list[tuple[int, int]] = []
  for a, b in values:
    return a + b
  return 42
"#,
    )?;

    let output = Command::new(incan_binary())
        .args([
            "build",
            entrypoint.to_string_lossy().as_ref(),
            "--backend",
            "replacement",
            "--backend-fallback",
            "refuse",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "typed empty scalar-pair list must execute directly. stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("replacement backend executed `main`: 42"),
        "unexpected typed empty pair-list output: {stdout}"
    );
    assert!(
        !temporary.path().join("target/incan").exists(),
        "direct replacement execution must not create a legacy generated-project directory"
    );

    let receipt_path = temporary.path().join(".incan/backend/receipt.json");
    let receipt: serde_json::Value = serde_json::from_str(&fs::read_to_string(receipt_path)?)?;
    assert_eq!(receipt["executed_backend"], "replacement");
    assert_eq!(receipt["fallback_outcome"], serde_json::json!("not_needed"));
    Ok(())
}

/// Emit stable direct-execution evidence in the distinct, non-Oven replacement JSON report.
#[test]
fn replacement_cli_json_report_projects_canonical_execution_evidence() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let entrypoint = temporary.path().join("main.incn");
    fs::write(&entrypoint, "def main() -> int:\n  return 42\n")?;

    let output = Command::new(incan_binary())
        .args([
            "build",
            entrypoint.to_string_lossy().as_ref(),
            "--backend",
            "replacement",
            "--backend-fallback",
            "refuse",
            "--report",
            "json",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "replacement JSON report must succeed. stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["schema_version"], "incan.replacement_execution.v0");
    assert_eq!(report["status"], "success");
    assert_eq!(report["mode"], "executable");
    assert_eq!(report["backend"]["executed_backend"], "replacement");
    assert_eq!(report["replacement_execution"]["result"], "42");
    assert!(
        report["replacement_execution"]["output_identity"]
            .as_str()
            .is_some_and(|identity| identity.starts_with("sha256:")),
        "direct report must project the receipt-bound output identity: {report}"
    );
    assert!(
        report["replacement_execution"]["ownership_reads"].is_array()
            && report["replacement_execution"]["runtime_requirements"].is_array(),
        "direct report must project canonical ownership and runtime evidence: {report}"
    );
    assert!(
        report.get("generated").is_none() && report.get("oven").is_none(),
        "direct replacement report must not invent generated-Rust or Oven evidence: {report}"
    );
    Ok(())
}

/// Reject the unimplemented legacy fallback spelling before it can create an artifact or receipt.
#[test]
fn replacement_cli_rejects_legacy_fallback_without_artifacts_or_receipts() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let entrypoint = temporary.path().join("main.incn");
    fs::write(&entrypoint, "def main() -> int:\n  return 42\n")?;

    let output = Command::new(incan_binary())
        .args([
            "build",
            entrypoint.to_string_lossy().as_ref(),
            "--backend",
            "replacement",
            "--backend-fallback",
            "legacy",
        ])
        .output()?;
    assert!(
        !output.status.success(),
        "the unavailable legacy fallback spelling must be rejected"
    );
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("possible values: refuse"),
        "CLI must make the only supported fallback policy visible: {combined}"
    );
    assert!(
        !temporary.path().join("target/incan").exists()
            && !temporary.path().join(".incan/backend/receipt.json").exists(),
        "an invalid fallback policy must not produce legacy output or a replacement receipt"
    );
    Ok(())
}

/// Ensure an unsupported source shape fails before any legacy output can be generated.
#[test]
fn replacement_cli_refuses_unsupported_source_without_legacy_generation() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let entrypoint = temporary.path().join("main.incn");
    fs::write(
        &entrypoint,
        r#"def main() -> int:
  values = [1, 2]
  return 0
"#,
    )?;

    let output = Command::new(incan_binary())
        .args([
            "build",
            entrypoint.to_string_lossy().as_ref(),
            "--backend",
            "replacement",
            "--backend-fallback",
            "refuse",
        ])
        .output()?;
    assert!(
        !output.status.success(),
        "unsupported source must refuse instead of falling back"
    );
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("list aggregate"),
        "unsupported construct must be visible: {combined}"
    );
    assert!(
        combined.contains("original Incan source span"),
        "refusal must retain source authority: {combined}"
    );
    let expected_start = r#"def main() -> int:
  values = [1, 2]
  return 0
"#
    .find("[1, 2]")
    .ok_or("aggregate fixture must contain its list literal")?;
    let expected_end = expected_start + "[1, 2]".len();
    assert!(
        combined.contains(&format!(
            "primary Incan source location: {}:{expected_start}..{expected_end}",
            entrypoint.display()
        )),
        "CLI refusal must identify the aggregate-bearing source expression exactly: {combined}"
    );
    assert!(
        !combined.contains("Generated Rust project"),
        "refusal must not enter legacy generation: {combined}"
    );
    assert!(
        !temporary.path().join("target/incan").exists(),
        "unsupported replacement input must not create a legacy generated-project directory"
    );
    Ok(())
}

/// Refuse a typed empty scalar list before replacement can publish a success receipt.
#[test]
fn replacement_cli_refuses_typed_empty_scalar_list_without_a_receipt() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let entrypoint = temporary.path().join("main.incn");
    fs::write(
        &entrypoint,
        r#"def main() -> int:
  values: list[int] = []
  for value in values:
    return value
  return 42
"#,
    )?;

    let output = Command::new(incan_binary())
        .args([
            "build",
            entrypoint.to_string_lossy().as_ref(),
            "--backend",
            "replacement",
            "--backend-fallback",
            "refuse",
        ])
        .output()?;
    assert!(
        !output.status.success(),
        "typed empty scalar list must refuse instead of executing directly"
    );
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("list[tuple[scalar, scalar]]"),
        "refusal must name the selected list profile: {combined}"
    );
    assert!(
        combined.contains("original Incan source span"),
        "refusal must retain source authority: {combined}"
    );
    assert!(
        !temporary.path().join("target/incan").exists()
            && !temporary.path().join(".incan/backend/receipt.json").exists(),
        "typed empty scalar-list refusal must not create legacy output or a replacement receipt"
    );
    Ok(())
}

/// Persist a requested shadow comparison as explicitly unavailable when no legacy runtime comparator exists.
#[test]
fn replacement_cli_shadow_receipt_is_explicitly_non_green() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let entrypoint = temporary.path().join("main.incn");
    fs::write(&entrypoint, "def main() -> int:\n  return 42\n")?;

    let output = Command::new(incan_binary())
        .args([
            "build",
            entrypoint.to_string_lossy().as_ref(),
            "--backend",
            "replacement",
            "--backend-fallback",
            "refuse",
            "--shadow",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "shadowed replacement build must execute directly. stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt_path = temporary.path().join(".incan/backend/receipt.json");
    let receipt: serde_json::Value = serde_json::from_str(&fs::read_to_string(receipt_path)?)?;
    assert_eq!(receipt["selection"]["shadow_requested"], true);
    let reason = receipt["shadow_comparison"]["unavailable"]["reason"]
        .as_str()
        .ok_or("requested shadow comparison must persist an unavailable reason")?;
    assert_eq!(reason, incan::backend::selection::SHADOW_COMPARISON_UNAVAILABLE_REASON);
    assert!(
        !temporary.path().join("target/incan").exists(),
        "shadow comparison must not generate a legacy project as semantic evidence"
    );
    Ok(())
}

/// Refuse imports and Rust-module directives with typed, file-addressable source diagnostics.
#[test]
fn replacement_cli_refuses_module_boundaries_with_primary_spans() -> Result<(), Box<dyn std::error::Error>> {
    let cases = [
        (
            "import",
            "import std.io\n\ndef main() -> int:\n  return 42\n",
            "import declaration",
        ),
        (
            "rust-module",
            "rust.module(\"incan_stdlib::testing\")\n\ndef main() -> int:\n  return 42\n",
            "Rust interop `rust.module` directive",
        ),
    ];
    for (name, source, expected_boundary) in cases {
        let temporary = tempfile::tempdir()?;
        let entrypoint = temporary.path().join(format!("{name}.incn"));
        fs::write(&entrypoint, source)?;
        let output = Command::new(incan_binary())
            .args([
                "build",
                entrypoint.to_string_lossy().as_ref(),
                "--backend",
                "replacement",
                "--backend-fallback",
                "refuse",
            ])
            .output()?;
        assert!(
            !output.status.success(),
            "{name} must be refused by the source-only profile"
        );
        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.contains("INCAN-R988-UNSUPPORTED"),
            "missing typed diagnostic: {combined}"
        );
        assert!(
            combined.contains(expected_boundary),
            "missing refusal boundary: {combined}"
        );
        assert!(
            combined.contains(&format!("primary Incan source location: {}:0..", entrypoint.display())),
            "refusal must retain the first unsupported source span: {combined}"
        );
        assert!(
            !temporary.path().join("target/incan").exists(),
            "{name} refusal must not create a legacy generated-project directory"
        );
    }
    Ok(())
}

/// Refuse an unsupported sibling function before a source-wide replacement receipt can be published.
#[test]
fn replacement_cli_refuses_additional_free_functions_before_writing_a_receipt() -> Result<(), Box<dyn std::error::Error>>
{
    let temporary = tempfile::tempdir()?;
    let entrypoint = temporary.path().join("main.incn");
    fs::write(
        &entrypoint,
        r#"def main() -> int:
  return 42

def unused() -> list[int]:
  return [1]
"#,
    )?;

    let output = Command::new(incan_binary())
        .args([
            "build",
            entrypoint.to_string_lossy().as_ref(),
            "--backend",
            "replacement",
            "--backend-fallback",
            "refuse",
        ])
        .output()?;
    assert!(
        !output.status.success(),
        "a source-wide replacement receipt must not ignore an unsupported sibling function"
    );
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("additional free function `unused`"),
        "the rejected sibling function must be visible: {combined}"
    );
    assert!(
        !temporary.path().join("target/incan").exists()
            && !temporary.path().join(".incan/backend/receipt.json").exists(),
        "a source-wide profile refusal must not produce legacy output or a replacement receipt"
    );
    Ok(())
}
