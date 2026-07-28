Warning: truncated output (original token count: 168928)
Total output lines: 18855

//! Integration tests for the Incan compiler frontend

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

mod support;

use incan::frontend::module::{ExportedTypeLikeDoc, ExportedTypeLikeKind, exported_type_like_docs};
use incan::frontend::{lexer, parser, typechecker};

/// Shared with `src/frontend/module.rs` tests (`exported_type_like_docs`) for GitHub #247.
const BLOCK_DOCSTRING_PUBLIC_TYPE_LIKE: &str = include_str!("fixtures/block_docstring_public_type_like.incn");

/// Helper to run full pipeline on a source file
fn compile_file(path: &Path) -> Result<(), Vec<String>> {
    let source = fs::read_to_string(path).map_err(|e| vec![e.to_string()])?;
    compile_source(&source)
}

fn compile_source(source: &str) -> Result<(), Vec<String>> {
    let tokens = lexer::lex(source).map_err(|errs| errs.iter().map(|e| e.message.clone()).collect::<Vec<_>>())?;

    let ast = parser::parse(&tokens).map_err(|errs| errs.iter().map(|e| e.message.clone()).collect::<Vec<_>>())?;

    typechecker::check(&ast).map_err(|errs| errs.iter().map(|e| e.message.clone()).collect::<Vec<_>>())?;

    Ok(())
}

fn strip_ansi_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

/// Parse JSON log records from stdout that may also contain human logging or ordinary print lines.
fn parse_json_log_records(stdout: &str) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
    stdout
        .lines()
        .filter(|line| line.trim_start().starts_with('{'))
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .map_err(Into::into)
}

/// Find a JSON logging record by its string body.
fn json_record_by_body<'a>(records: &'a [serde_json::Value], body: &str) -> Option<&'a serde_json::Value> {
    records
        .iter()
        .find(|record| record["Body"]["StringValue"] == serde_json::json!(body))
}

static TEST_PROJECT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Create a throwaway project name that does not collide under parallel nextest workers.
///
/// Several CLI tests rely on the default `target/incan/<name>` output location. The generated project name includes
/// both the current process id and a local counter so those tests do not trample each other's generated Cargo projects.
fn unique_test_project_name(prefix: &str) -> String {
    let unique = TEST_PROJECT_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{}", std::process::id(), unique)
}

/// Create a minimal throwaway Incan project for end-to-end runtime error assertions.
fn write_runtime_error_project(source: &str) -> Result<(tempfile::TempDir, PathBuf), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project_name = unique_test_project_name("runtime_error_contract");
    let src_dir = tmp.path().join("src");
    fs::create_dir_all(&src_dir)?;
    fs::write(
        tmp.path().join("incan.toml"),
        format!("[project]\nname = \"{project_name}\"\nversion = \"0.1.0\"\n"),
    )?;
    let main_path = src_dir.join("main.incn");
    fs::write(&main_path, source)?;
    Ok((tmp, main_path))
}

/// Assert that a program compiles successfully but fails at runtime with a canonical Incan diagnostic.
///
/// This helper intentionally checks the CLI surface rather than internal helper text so regressions in generated-main
/// panic formatting or subprocess execution still fail the contract.
fn assert_runtime_error_cli(
    source: &str,
    kind: &str,
    detail_markers: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    let (_tmp, main_path) = write_runtime_error_project(source)?;

    let run_output = incan_command()
        .arg("run")
        .arg(&main_path)
        .env("CARGO_NET_OFFLINE", "true")
        .output()?;
    assert!(
        !run_output.status.success(),
        "expected runtime failure, stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run_output.stdout),
        String::from_utf8_lossy(&run_output.stderr)
    );

    let stdout = strip_ansi_escapes(&String::from_utf8_lossy(&run_output.stdout));
    let stderr = strip_ansi_escapes(&String::from_utf8_lossy(&run_output.stderr));
    let combined = format!("{stdout}\n{stderr}");
    assert!(
        combined.contains(kind),
        "expected `{kind}` in runtime diagnostic, got:\n{combined}"
    );
    for marker in detail_markers {
        assert!(
            combined.contains(marker),
            "expected runtime diagnostic to contain `{marker}`, got:\n{combined}"
        );
    }
    for forbidden in ["panicked at", "thread 'main'", ".rs:"] {
        assert!(
            !combined.contains(forbidden),
            "expected runtime diagnostic to avoid raw Rust leakage `{forbidden}`, got:\n{combined}"
        );
    }

    Ok(())
}

#[test]
fn bare_incan_run_uses_project_main_script() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let src_dir = tmp.path().join("src");
    fs::create_dir_all(&src_dir)?;
    fs::write(
        tmp.path().join("incan.toml"),
        r#"[project]
name = "bare_run_project"
version = "0.1.0"

[project.scripts]
main = "src/main.incn"
"#,
    )?;
    fs::write(
        src_dir.join("main.incn"),
        r#"def main() -> None:
  println("bare run works")
"#,
    )?;

    let output = incan_command()
        .arg("run")
        .current_dir(tmp.path())
        .env("CARGO_NET_OFFLINE", "true")
        .output()?;

    assert!(
        output.status.success(),
        "expected bare `incan run` to succeed from project root.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("bare run works"),
        "expected bare `incan run` to execute [project.scripts].main, got:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );

    Ok(())
}

#[test]
fn build_rejects_unbound_type_annotation_before_generated_rust_issue902() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let cases = [("simple", "Count", "Count"), ("qualified", "missing::Count", "missing")];

    for (case_name, annotation, expected_symbol) in cases {
        let project_name = format!("unbound_type_annotation_{case_name}");
        let project_root = tmp.path().join(case_name);
        let src_dir = project_root.join("src");
        fs::create_dir_all(&src_dir)?;
        fs::write(
            project_root.join("incan.toml"),
            format!("[project]\nname = \"{project_name}\"\nversion = \"0.1.0\"\n"),
        )?;
        let main_path = src_dir.join("main.incn");
        fs::write(
            &main_path,
            format!(
                r#"def accepts(value: {annotation}) -> {annotation}:
  return value

def main() -> None:
  print(accepts(7))
"#
            ),
        )?;

        let output = incan_command()
            .args(["build", main_path.to_string_lossy().as_ref(), "--no-locked"])
            .current_dir(&project_root)
            .env("CARGO_NET_OFFLINE", "true")
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = strip_ansi_escapes(&String::from_utf8_lossy(&output.stderr));
        let generated_project = project_root.join("target/incan").join(&project_name);

        assert!(
            !output.status.success(),
            "expected the {case_name} unbound annotation to fail"
        );
        assert!(
            stderr.contains(&format!("Unknown symbol '{expected_symbol}'")),
            "expected an Incan diagnostic for {case_name}, got:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            !stdout.contains("Generated Rust project in:")
                && !stdout.contains("Building...")
                && !stderr.contains("E0425")
                && !stderr.contains("E0433")
                && !generated_project.exists(),
            "the {case_name} annotation must fail before generated Rust is published:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }

    Ok(())
}

#[test]
fn std_logging_runtime_surfaces_share_one_generated_run() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project_name = unique_test_project_name("std_logging_runtime_surfaces");
    let src_dir = tmp.path().join("src");
    fs::create_dir_all(&src_dir)?;
    fs::write(
        tmp.path().join("incan.toml"),
        format!("[project]\nname = \"{project_name}\"\nversion = \"0.1.0\"\n"),
    )?;
    fs::write(
        src_dir.join("worker.incn"),
        r#"from std.logging import get_logger

pub def run_get_logger_worker() -> None:
  log = get_logger()
  log.info("worker ready")

pub def run_ambient_worker() -> None:
  log.info("worker ambient log ready")
"#,
    )?;
    let source = r#"from std.logging import ColorPolicy, Level, LogFormat, LogStyle, LoggerName, OutputTarget, basic_config, get_logger
from std.telemetry.core import TelemetryValue
from worker import run_ambient_worker, run_get_logger_worker

model LocalLog:
  def info(self, message: str) -> None:
    println(f"local:{message}")

def logger_context_case() -> None:
  basic_config(level=Level.WARNING, style=LogStyle.VERBOSE, color=ColorPolicy.NEVER, target="stdout")
  root = get_logger("app").bind({"shared": "root"})
  child = root.child("loader").bind({"component": "loader"})

  root.info("silent info")
  if root.is_enabled(Level.INFO):
    println("unexpected info enabled")
  if not child.is_enabled(Level.ERROR):
    println("unexpected error disabled")

  root.error("root event")
  child.warning("child event", fields={"shared": "event"})

def json_record_shape_case() -> None:
  basic_config(level=Level.DEBUG, format=LogFormat.JSON, target="stdout")
  log = get_logger()
  log.debug("json works", fields={"request_id": "abc", "component": "loader"})

def default_target_case() -> None:
  basic_config(level=Level.INFO)
  get_logger("app").info("stderr event")

def shadow_case() -> None:
  basic_config(level=Level.INFO, format=LogFormat.JSON, target="stdout")
  log = LocalLog()
  log.info("shadowed")

def ambient_root_case() -> None:
  basic_config(level=Level.INFO, format=LogFormat.JSON, target="stdout")
  log.info("snippet ambient")

def structured_fields_case() -> None:
  basic_config(level=Level.INFO, format=LogFormat.JSON, target="stdout")
  log.info("structured", fields={
    "rows": 42,
    "ok": true,
    "ratio": 1.5,
    "missing": None,
    "items": TelemetryValue.array([TelemetryValue.int(1), TelemetryValue.bool(false)]),
    "nested": TelemetryValue.map({"child": TelemetryValue.string("yes")}),
  })

def telemetry_constructor_case() -> None:
  text = TelemetryValue.string("alpha")
  payload = TelemetryValue.map({
    "items": TelemetryValue.array([TelemetryValue.int(42), TelemetryValue.bool(true)]),
    "empty": TelemetryValue.none(),
    "encoded": TelemetryValue.bytes("ff"),
    "ratio": TelemetryValue.float(1.5),
  })
  println(f"telemetry:{text.display_text()}")
  println(f"telemetry:{payload.display_text()}")

def validator_case() -> None:
  match LoggerName.from_underlying(""):
    Ok(_) => println("unexpected accepted empty logger name")
    Err(err) => println(f"validation:empty_logger:{err.to_string()}")
  match LoggerName.from_underlying(".app"):
    Ok(_) => println("unexpected accepted edge logger name")
    Err(err) => println(f"validation:edge_logger:{err.to_string()}")
  match LoggerName.from_underlying("app..db"):
    Ok(_) => println("unexpected accepted segmented logger name")
    Err(err) => println(f"validation:segmented_logger:{err.to_string()}")
  match OutputTarget.from_underlying("bogus"):
    Ok(_) => println("unexpected accepted output target")
    Err(err) => println(f"validation:output_target:{err.to_string()}")

def human_styles_case() -> None:
  basic_config(level=Level.INFO, style=LogStyle.MINIMAL, target="stdout")
  get_logger("app").info("minimal event")
  basic_config(level=Level.INFO, style=LogStyle.SHORT, target="stdout")
  get_logger("app").info("short event")
  basic_config(level=Level.INFO, style=LogStyle.COMPLETE, target="stdout")
  get_logger("app").info("complete event")
  basic_config(level=Level.INFO, style=LogStyle.VERBOSE, target="stdout")
  get_logger("app").info("verbose event")
  run_get_logger_worker()
  run_ambient_worker()

def main() -> None:
  logger_context_case()
  json_record_shape_case()
  default_target_case()
  shadow_case()
  ambient_root_case()
  structured_fields_case()
  telemetry_constructor_case()
  validator_case()
  human_styles_case()
"#;
    let main_path = src_dir.join("main.incn");
    fs::write(&main_path, source)?;

    let output = incan_command()
        .args(["run", main_path.to_string_lossy().as_ref()])
        .env("CARGO_NET_OFFLINE", "true")
        .output()?;

    assert!(
        output.status.success(),
        "expected combined std.logging source surface run to succeed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains("silent info"),
        "expected INFO event to be filtered by source basic_config, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("unexpected"),
        "expected is_enabled filtering checks to pass, got:\n{stdout}"
    );
    assert!(
        stdout.contains("[ERROR] root event") && stdout.contains(r#"shared="root""#) && stdout.contains("logger=app"),
        "expected root logger context to remain unmodified, got:\n{stdout}"
    );
    assert!(
        stdout.contains("[WARNING] child event")
            && stdout.contains(r#"component="loader""#)
            && stdout.contains(r#"shared="event""#),
        "expected child logger bound fields and event override, got:\n{stdout}"
    );
    assert!(
        stdout.contains("logger=app.loader"),
        "expected child logger name, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("stderr event") && stderr.contains("stderr event"),
        "expected default logging target to route the event to stderr.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("local:shadowed") && !stdout.contains(r#""Body":{"Type":"string","StringValue":"shadowed"}"#),
        "expected local log binding to remain ordinary source, got:\n{stdout}"
    );
    for expected in [
        "validation:empty_logger:std.logging logger names must not be empty",
        "validation:edge_logger:std.logging logger names must not start or end with '.'",
        "validation:segmented_logger:std.logging logger names must not contain empty segments",
        "validation:output_target:std.logging target must be 'stdout' or 'stderr'",
    ] {
        assert!(stdout.contains(expected), "expected `{expected}`, got:\n{stdout}");
    }
    assert!(
        !stdout.contains("unexpected accepted"),
        "expected std.logging validators to reject invalid values, got:\n{stdout}"
    );

    let records = parse_json_log_records(&stdout)?;
    let record = json_record_by_body(&records, "json works")
        .ok_or_else(|| std::io::Error::other(format!("missing `json works` record in:\n{stdout}")))?;
    assert_eq!(record["SeverityText"], serde_json::json!("DEBUG"));
    assert_eq!(record["SeverityNumber"], serde_json::json!(5));
    assert_eq!(record["InstrumentationScope"]["Name"], serde_json::json!("main"));
    assert_eq!(record["Body"]["Type"], serde_json::json!("string"));
    assert_eq!(record["Attributes"]["request_id"]["Type"], serde_json::json!("string"));
    assert_eq!(
        record["Attributes"]["request_id"]["StringValue"],
        serde_json::json!("abc")
    );
    assert_eq!(record["Attributes"]["component"]["Type"], serde_json::json!("string"));
    assert_eq!(
        record["Attributes"]["component"]["StringValue"],
        serde_json::json!("loader")
    );
    assert_eq!(record["Resource"]["Attributes"], serde_json::json!({}));
    assert!(
        record.get("request_id").is_none() && record.get("component").is_none(),
        "expected user fields to stay under Attributes, got:\n{record}"
    );

    let ambient = json_record_by_body(&records, "snippet ambient")
        .ok_or_else(|| std::io::Error::other(format!("missing `snippet ambient` record in:\n{stdout}")))?;
    assert_eq!(ambient["InstrumentationScope"]["Name"], serde_json::json!("main"));

    let structured = json_record_by_body(&records, "structured")
        .ok_or_else(|| std::io::Error::other(format!("missing `structured` record in:\n{stdout}")))?;
    let attributes = &structured["Attributes"];
    assert_eq!(attributes["rows"]["Type"], serde_json::json!("int"));
    assert_eq!(attributes["rows"]["IntValue"], serde_json::json!(42));
    assert_eq!(attributes["ok"]["Type"], serde_json::json!("bool"));
    assert_eq!(attributes["ok"]["BoolValue"], serde_json::json!(true));
    assert_eq!(attributes["ratio"]["Type"], serde_json::json!("float"));
    assert_eq!(attributes["ratio"]["FloatValue"], serde_json::json!(1.5));
    assert_eq!(attributes["missing"]["Type"], serde_json::json!("none"));
    assert_eq!(attributes["items"]["Type"], serde_json::json!("array"));
    assert_eq!(attributes["nested"]["Type"], serde_json::json!("map"));
    assert!(
        structured.get("rows").is_none() && structured.get("nested").is_none(),
        "expected structured fields to stay under Attributes, got:\n{structured}"
    );

    let log_lines: Vec<&str> = stdout.lines().filter(|line| line.contains("[INFO]")).collect();
    let short_line = log_lines
        .iter()
        .copied()
        .find(|line| line.contains("short event"))
        .unwrap_or("");
    let complete_line = log_lines
        .iter()
        .copied()
        .find(|line| line.contains("complete event"))
        .unwrap_or("");

    assert!(
        stdout.contains("[INFO] minimal event"),
        "expected minimal line, got:\n{stdout}"
    );
    assert_eq!(
        short_line.find(" [INFO] short event"),
        Some(8),
        "expected short style to use compact time-of-day timestamp, got:\n{stdout}"
    );
    assert!(
        complete_line.contains('T') && complete_line.contains("Z [INFO] complete event"),
        "expected complete style to use full datetime timestamp, got:\n{stdout}"
    );
    assert!(
        stdout.contains("[INFO] verbose event\n  logger=app"),
        "expected verbose style to add logger metadata on a second line, got:\n{stdout}"
    );
    assert!(
        stdout.contains("telemetry:alpha")
            && stdout.contains(r#""Type":"map""#)
            && stdout.contains(r#""items":{"Type":"array""#)
            && stdout.contains(r#""IntValue":42"#)
            && stdout.contains(r#""BoolValue":true"#)
            && stdout.contains(r#""BytesValue":"ff""#)
            && stdout.contains(r#""FloatValue":1.5"#),
        "expected telemetry value constructors to preserve structured values, got:\n{stdout}"
    );
    assert!(
        stdout.contains("worker ready")
            && stdout.contains("worker ambient log ready")
            && stdout.contains("logger=worker")
            && !stdout.contains("logger=std.logging"),
        "expected worker module logging to infer logger=worker, got:\n{stdout}"
    );

    Ok(())
}

#[test]
fn validated_newtype_runtime_scenarios() -> Result<(), Box<dyn std::error::Error>> {
    let output = incan_command()
        .args([
            "run",
            "-c",
            r#"
type Attempts = newtype int:
  def from_underlying(n: int) -> Result[Self, ValidationError]:
    if n <= 0:
      return Err(ValidationError("attempts must be >= 1"))
    return Ok(Attempts(n))

def retry(attempts: Attempts) -> None:
  println(f"retry={attempts.0}")

def main() -> None:
  retry(3)
  attempts: Attempts = 4
  println(f"local={attempts.0}")
"#,
        ])
        .env("CARGO_NET_OFFLINE", "true")
        .output()?;

    assert!(
        output.status.success(),
        "validated-newtype success program failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("retry=3"), "unexpected stdout:\n{stdout}");
    assert!(stdout.contains("local=4"), "unexpected stdout:\n{stdout}");

    assert_runtime_error_cli(
        r#"
type Attempts = newtype int:
  def from_underlying(n: int) -> Result[Self, ValidationError]:
    if n <= 0:
      return Err(ValidationError("attempts must be >= 1"))
    return Ok(Attempts(n))

def retry(attempts: Attempts) -> None:
  return

def read_attempts(attempts: Attempts) -> int:
  return attempts.0

def main() -> None:
  println(f"ok={read_attempts(Attempts(1))}")
  retry(0)
"#,
        "ValidationError",
        &["Attempts::from_underlying", "attempts must be >= 1"],
    )?;

    assert_runtime_error_cli(
        r#"
type PositiveInt = newtype int:
  def from_underlying(n: int) -> Result[Self, ValidationError]:
    if n <= 0:
      return Err(ValidationError("positive int must be greater than zero"))
    return Ok(PositiveInt(n))

model Bounds:
  low: PositiveInt
  high: PositiveInt

def width(bounds: Bounds) -> int:
  return bounds.high.0 - bounds.low.0

def main() -> None:
  println(f"width={width(Bounds(low=1, high=2))}")
  _ = Bounds(low=0, high=-1)
"#,
        "ValidationError",
        &[
            "Bounds validation failed with 2 error(s)",
            "low: positive int must be greater than zero",
            "high: positive int must be greater than zero",
        ],
    )?;

    Ok(())
}

#[test]
fn validated_newtype_json_deserialization_runtime() -> Result<(), Box<dyn std::error::Error>> {
    let output = incan_command()
        .args([
            "run",
            "-c",
            r#"
from std.serde import json
from std.serde.json import Deserialize

@derive(Clone, json)
type ShortId = newtype str:
  def from_underlying(value: str) -> Result[Self, ValidationError]:
    if len(value) > 8:
      return Err(ValidationError("identifier too long"))
    return Ok(ShortId(value))

type PositiveInt = newtype int[gt=0]

@derive(Clone, Deserialize)
type CheckedBox[D] = newtype D:
  def from_underlying(value: D) -> Result[Self, ValidationError]:
    return Ok(CheckedBox[D](value))

@derive(json)
model Envelope:
  id: ShortId

@derive(Deserialize)
model IdList:
  ids: list[ShortId]

@derive(Deserialize)
model OptionalId:
  id: Option[ShortId]

@derive(Deserialize)
model PositiveEnvelope:
  value: PositiveInt

@derive(Deserialize)
model GenericEnvelope:
  value: CheckedBox[str]

def main() -> None:
  match Envelope.from_json('{"id":"short"}'):
    case Ok(value):
      println(f"model_roundtrip:{value.to_json()}")
    case Err(_):
      println("model_valid_rejected")
  match Envelope.from_json('{"id":"identifier_too_long"}'):
    case Ok(_):
      println("model_invalid_accepted")
    case Err(_):
      println("model_invalid_rejected")
  match IdList.from_json('{"ids":["short","identifier_too_long"]}'):
    case Ok(_):
      println("list_invalid_accepted")
    case Err(_):
      println("list_invalid_rejected")
  match OptionalId.from_json('{"id":"identifier_too_long"}'):
    case Ok(_):
      println("optional_invalid_accepted")
    case Err(_):
      println("optional_invalid_rejected")
  match PositiveEnvelope.from_json('{"value":1}'):
    case Ok(value):
      println(f"constraint_valid:{value.value.0}")
    case Err(_):
      println("constraint_valid_rejected")
  match PositiveEnvelope.from_json('{"value":0}'):
    case Ok(_):
      println("constraint_invalid_accepted")
    case Err(_):
      println("constraint_invalid_rejected")
  match GenericEnvelope.from_json('{"value":"generic"}'):
    case Ok(value):
      println(f"generic_valid:{value.value.0}")
    case Err(_):
      println("generic_valid_rejected")
"#,
        ])
        .env("CARGO_NET_OFFLINE", "true")
        .output()?;

    assert!(
        output.status.success(),
        "validated-newtype JSON program failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        vec![
            "model_roundtrip:{\"id\":\"short\"}",
            "model_invalid_rejected",
            "list_invalid_rejected",
            "optional_invalid_rejected",
            "constraint_valid:1",
            "constraint_invalid_rejected",
            "generic_valid:generic",
        ],
        "expected every JSON ingress to preserve newtype validation, got:\n{stdout}"
    );

    Ok(())
}

/// Issue #914: derived traits on a nested newtype must survive frontend bound checking and generated-Rust compilation.
#[test]
fn derived_nested_newtype_generic_bounds_build_and_run_issue914() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let source_path = temp.path().join("nested_newtype_generic_bounds.incn");
    let output_dir = temp.path().join("generated");
    fs::write(
        &source_path,
        r#"@derive(Clone, Eq)
type BaseId = newtype str

@derive(Clone, Eq)
type ChildId = newtype BaseId

def has_duplicates[T with (Clone, Eq)](values: list[T]) -> bool:
    for left in range(len(values)):
        for right in range(len(values)):
            if right > left and values[left] == values[right]:
                return true
    return false

def main() -> None:
    identifiers = [ChildId(BaseId("one")), ChildId(BaseId("one"))]
    println(has_duplicates(identifiers))
"#,
    )?;

    let build = incan_command()
        .args([
            "build",
            source_path.to_string_lossy().as_ref(),
            output_dir.to_string_lossy().as_ref(),
        ])
        .output()?;
    assert!(
        build.status.success(),
        "nested newtype generic-bound build failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    let run = incan_command().arg("run").arg(&source_path).output()?;
    assert!(
        run.status.success(),
        "nested newtype generic-bound run failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8(run.stdout)?, "true\n");
    Ok(())
}

#[test]
fn rfc028_user_defined_operators_run_end_to_end() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let src_dir = tmp.path().join("src");
    fs::create_dir_all(&src_dir)?;
    fs::write(
        tmp.path().join("incan.toml"),
        r#"[project]
name = "rfc028_user_defined_operators"
version = "0.1.0"
"#,
    )?;
    fs::write(
        src_dir.join("main.incn"),
        r#"model Money:
  cents: int

  def __add__(self, other: Money) -> Money:
    return Money(cents=self.cents + other.cents)

  def __lt__(self, other: Money) -> bool:
    return self.cents < other.cents


model Row:
  value: int

  def __getitem__(self, index: int) -> int:
    return self.value + index

  def __setitem__(self, index: int, value: int) -> None:
    pass


model OpBox:
  value: int

  def __matmul__(self, other: OpBox) -> OpBox:
    return OpBox(value=self.value + other.value)

  def __invert__(self) -> OpBox:
    return OpBox(value=0 - self.value)


def main() -> None:
  total = Money(cents=100) + Money(cents=25)
  println(total.cents)
  println(Money(cents=25) < Money(cents=100))
  row = Row(value=4)
  row[3] = 9
  println(row[3])
  mat = OpBox(value=2) @ OpBox(value=3)
  println(mat.value)
  inverted = ~OpBox(value=8)
  println(inverted.value)
"#,
    )?;

    let output = incan_command()
        .arg("run")
        .arg("src/main.incn")
        .current_dir(tmp.path())
        .env("CARGO_NET_OFFLINE", "true")
        .output()?;
    assert!(
        output.status.success(),
        "expected RFC 028 operator program to run.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("125") && stdout.contains("true") && stdout.contains("7") && stdout.contains("5"),
        "unexpected RFC 028 operator output:\n{stdout}"
    );

    Ok(())
}

/// Locate the `incan` binary for subprocess tests.
///
/// Uses `CARGO_BIN_EXE_incan` when present (integration tests under `cargo test`) so we always run the artifact from
/// the current build, including when `CARGO_TARGET_DIR` is not the default `target/`.
fn incan_debug_binary() -> std::path::PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_incan") {
        let path = std::path::PathBuf::from(path);
        if path.exists() {
            return path;
        }
    }
    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        let p = std::path::PathBuf::from(&target_dir).join("debug/incan");
        if p.exists() {
            return p;
        }
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/incan")
}

fn shared_generated_cargo_target_dir() -> std::path::PathBuf {
    support::generated_cargo_target_dir()
}

fn incan_command() -> Command {
    let mut command = Command::new(incan_debug_binary());
    command
        .env("INCAN_GENERATED_CARGO_TARGET_DIR", shared_generated_cargo_target_dir())
        .env("CARGO_NET_OFFLINE", "true")
        .env("INCAN_INTERNAL_SDK_PROVIDER_STORE", support::sdk_provider_store());
    command
}

/// Resolve one exact immutable compiled SDK provider selected for a generated consumer.
fn compiled_sdk_provider_artifact_root(
    generated_project: &Path,
    crate_name: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cargo_toml = fs::read_to_string(generated_project.join("Cargo.toml"))?;
    let manifest: toml::Value = toml::from_str(&cargo_toml)?;
    let path = manifest
        .get("dependencies")
        .and_then(|dependencies| dependencies.get(crate_name))
        .and_then(|dependency| dependency.get("path"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| format!("generated consumer did not select compiled SDK provider `{crate_name}`"))?;
    let path = PathBuf::from(path);
    Ok(if path.is_absolute() {
        path
    } else {
        generated_project.join(path)
    })
}

#[test]
fn binary_read_result_context_crosses_boundaries_issue955() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project_name = unique_test_project_name("typed_binary_read_result");
    let src_dir = tmp.path().join("src");
    let tests_dir = tmp.path().join("tests");
    fs::create_dir_all(&src_dir)?;
    fs::create_dir_all(&tests_dir)?;
    fs::write(
        tmp.path().join("incan.toml"),
        format!("[project]\nname = \"{project_name}\"\nversion = \"0.1.0\"\n"),
    )?;
    fs::write(
        src_dir.join("io_facade.incn"),
        "pub from std.io import BytesIO, Endian, IoError\n",
    )?;
    let main_path = src_dir.join("main.incn");
    fs::write(
        &main_path,
        r#"from io_facade import BytesIO, Endian, IoError

def read_u8() -> Result[u8, IoError]:
  result: Result[u8, IoError] = BytesIO(b"").read(Endian.Big)
  return result

def read_i8() -> Result[i8, IoError]:
  result: Result[i8, IoError] = BytesIO(b"").read(Endian.Big)
  return result

def read_u16() -> Result[u16, IoError]:
  result: Result[u16, IoError] = BytesIO(b"").read(Endian.Big)
  return result

def read_i16() -> Result[i16, IoError]:
  result: Result[i16, IoError] = BytesIO(b"").read(Endian.Big)
  return result

def read_u32() -> Result[u32, IoError]:
  result: Result[u32, IoError] = BytesIO(b"").read(Endian.Big)
  return result

def read_i32() -> Result[i32, IoError]:
  result: Result[i32, IoError] = BytesIO(b"").read(Endian.Big)
  return result

def read_u64() -> Result[u64, IoError]:
  result: Result[u64, IoError] = BytesIO(b"").read(Endian.Big)
  return result

def read_i64() -> Result[i64, IoError]:
  result: Result[i64, IoError] = BytesIO(b"").read(Endian.Big)
  return result

def read_u128() -> Result[u128, IoError]:
  result: Result[u128, IoError] = BytesIO(b"").read(Endian.Big)
  return result

def read_i128() -> Result[i128, IoError]:
  result: Result[i128, IoError] = BytesIO(b"").read(Endian.Big)
  return result

def read_f32() -> Result[f32, IoError]:
  result: Result[f32, IoError] = BytesIO(b"").read(Endian.Big)
  return result

def read_f64() -> Result[f64, IoError]:
  result: Result[f64, IoError] = BytesIO(b"").read(Endian.Big)
  return result

def main() -> None:
  read_u8()
  read_i8()
  read_u16()
  read_i16()
  read_u32()
  read_i32()
  read_u64()
  read_i64()
  read_u128()
  read_i128()
  read_f32()
  read_f64()
"#,
    )?;
    fs::write(
        tests_dir.join("test_binary_read.incn"),
        r#"from io_facade import BytesIO, Endian, IoError
from std.testing import assert_eq, fail

def test_typed_binary_read_results() -> None:
  u16_result: Result[u16, IoError] = BytesIO(b"\x00\x01").read(Endian.Big)
  match u16_result:
    Ok(value) => assert_eq(value, 1)
    Err(error) => fail(error.message())

  u32_result: Result[u32, IoError] = BytesIO(b"\x00\x00\x00\x02").read(Endian.Big)
  match u32_result:
    Ok(value) => assert_eq(value, 2)
    Err(error) => fail(error.message())

  f64_result: Result[f64, IoError] = BytesIO(b"\x00\x00\x00\x00\x00\x00\x00\x00").read(Endian.Big)
  match f64_result:
    Ok(value) => assert_eq(value, 0.0)
    Err(error) => fail(error.message())
"#,
    )?;

    let out_dir = tmp.path().join("out");
    let build_output = incan_command()
        .args([
            "build",
            main_path.to_string_lossy().as_ref(),
            out_dir.to_string_lossy().as_ref(),
        ])
        .current_dir(tmp.path())
        .output()?;
    assert!(
        build_output.status.success(),
        "expected every BinaryRead width to compile from its typed Result context.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build_output.stdout),
        String::from_utf8_lossy(&build_output.stderr)
    );

    let generated_main = fs::read_to_string(out_dir.join("src/main.rs"))?;
    let normalized = generated_main
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    for rust_type in [
        "u8", "i8", "u16", "i16", "u32", "i32", "u64", "i64", "u128", "i128", "f32", "f64",
    ] {
        assert!(
            normalized.contains(&format!("BinaryRead::<{rust_type},>::read")),
            "expected exact BinaryRead::<{rust_type}> dispatch in generated Rust:\n{generated_main}"
        );
    }

    let test_output = incan_command()
        .args(["test", tests_dir.to_string_lossy().as_ref()])
        .current_dir(tmp.path())
        .env(
            "INCAN_TEST_SHARED_TARGET_DIR",
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("incan_e2e_shared_target"),
        )
        .output()?;
    assert!(
        test_output.status.success(),
        "expected package test batch to preserve typed BinaryRead dispatch.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&test_output.stdout),
        String::from_utf8_lossy(&test_output.stderr)
    );
    Ok(())
}

fn run_incan_command_with_timeout(
    mut command: Command,
    timeout: std::time::Duration,
) -> std::io::Result<(std::process::Output, bool)> {
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    let mut child = command.spawn()?;
    let start = std::time::Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output().map(|output| (output, false));
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            return child.wait_with_output().map(|output| (output, true));
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

fn is_incan_fixture(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("incn") | Some("incan"))
}

/// Make a temporary test directory to be able to run the CLI tests.
fn make_temp_test_dir() -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    let uniq = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    dir.push(format!("incan_cli_test_{}", uniq));
    let Ok(()) = std::fs::create_dir_all(&dir) else {
        panic!("failed to create temp test dir");
    };
    dir
}

fn write_cycle_explicit_call_site_generics_project(dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let src_dir = dir.join("src");
    std::fs::create_dir_all(&src_dir)?;
    std::fs::write(
        dir.join("incan.toml"),
        r#"[project]
name = "cycle_explicit_call_site_generics"
version = "0.1.0"
"#,
    )?;
    std::fs::write(
        src_dir.join("dataset.incn"),
        r#"from session import collect_with_active_session

pub model DataSet[T]:
  pub value: T

pub def collect_with_dataset[T](dataset: DataSet[T]) -> T:
  return collect_with_active_session[T](dataset)
"#,
    )?;
    std::fs::write(
        src_dir.join("session.incn"),
        r#"from dataset import DataSet

pub def collect_with_active_session[T](dataset: DataSet[T]) -> T:
  return dataset.value
"#,
    )?;
    let main_path = src_dir.join("main.incn");
    std::fs::write(
        &main_path,
        r#"from dataset import DataSet, collect_with_dataset

def main() -> None:
  let ds = DataSet(value=1)
  println(collect_with_dataset[int](ds))
"#,
    )?;
    Ok(main_path)
}

/// Regression (GitHub #247): `incan fmt` on disk must preserve body docstrings for all public block-like type
/// declarations, and [`exported_type_like_docs`] must still see them after the CLI round-trip.
///
/// `format_files` delegates to [`incan::format::format_source`]; this still covers subprocess + I/O if those paths
/// diverge from in-process formatting.
#[test]
fn test_cli_fmt_preserves_block_decl_docstrings_and_export_doc_surface() -> Result<(), Box<dyn std::error::Error>> {
    let dir = make_temp_test_dir();
    let path = dir.join("block_docstrings_cli.incn");
    fs::write(&path, BLOCK_DOCSTRING_PUBLIC_TYPE_LIKE)?;
    let status = incan_command().arg("fmt").arg(&path).status()?;
    assert!(status.success(), "incan fmt failed");

    let formatted = fs::read_to_string(&path)?;
    let tokens = lexer::lex(&formatted)
        .map_err(|errs| std::io::Error::other(errs.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("\n")))?;
    let ast = parser::parse(&tokens)
        .map_err(|errs| std::io::Error::other(errs.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("\n")))?;

    fn assert_markers(doc: Option<&str>, ctx: &str) -> Result<(), Box<dyn std::error::Error>> {
        let Some(doc) = doc else {
            return Err(std::io::Error::other(format!("{ctx}: missing docstring after CLI fmt")).into());
        };
        let t = doc.trim();
        if !t.contains("Line A documents the class API.") {
            return Err(std::io::Error::other(format!("{ctx}: missing marker A in {t:?}")).into());
        }
        if !t.contains("Line B keeps interior newlines after trim().") {
         …148928 tokens truncated…         generated_main_rs.contains(".to_string()"),
            "expected helper string arguments to use normal owned-string conversion, got:\n{generated_main_rs}"
        );
        Ok(())
    }

    #[test]
    fn consumer_build_plans_source_backed_pub_helper_calls_with_defaults_and_unions_issue729()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let wasm = compile_desugarer_wasm(0, "[]", "")?;
        write_source_pub_library_with_vocab_desugarer_and_query_helpers(tmp.path(), &wasm)?;

        let main_path = write_project_files(
            tmp.path(),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"deps/querykit\" }\n",
            r#"from pub::querykit import aggregate_as, aggregate_default, count, lit

def main() -> None:
  aggregate_as(lit(5), "adjusted")
  aggregate_as(count(), "order_count")
  aggregate_default(lit(7))
"#,
        )?;

        let out_dir = tmp.path().join("out");
        let output = run_build(&main_path, &out_dir)?;
        let generated_main_rs = std::fs::read_to_string(out_dir.join("src/main.rs")).unwrap_or_default();
        assert!(
            output.status.success(),
            "expected ordinary pub helper calls to share exported default, union, and string planning.\ngenerated main.rs:\n{}\nstdout:\n{}\nstderr:\n{}",
            generated_main_rs,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            generated_main_rs.contains("querykit::count(")
                || generated_main_rs.contains("__incan_vocab_helper_querykit_count("),
            "expected omitted count() argument to be filled from the helper's default expression, got:\n{generated_main_rs}"
        );
        assert!(
            !generated_main_rs.contains("querykit::count()")
                && !generated_main_rs.contains("__incan_vocab_helper_querykit_count()"),
            "ordinary pub helper default planning must not emit a zero-argument Rust count call, got:\n{generated_main_rs}"
        );
        assert!(
            generated_main_rs.contains("querykit::helpers::COUNT_SENTINEL"),
            "ordinary public dependency const defaults must keep the defining provider module path, got:\n{generated_main_rs}"
        );
        assert!(
            !generated_main_rs.contains("pub enum __IncanUnion"),
            "ordinary public dependency calls must not re-own dependency anonymous unions, got:\n{generated_main_rs}"
        );
        assert!(
            generated_main_rs.contains(".to_string()"),
            "expected ordinary pub helper string arguments to use normal owned-string conversion, got:\n{generated_main_rs}"
        );
        assert!(
            generated_main_rs.contains("querykit::helpers::DEFAULT_LABEL"),
            "expected public const defaults to emit through their provider module path, got:\n{generated_main_rs}"
        );
        Ok(())
    }

    #[test]
    fn consumer_check_passes_scoped_query_surface_artifacts_to_desugarer() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let response = incan_vocab::DesugarResponse::statements(vec![incan_vocab::IncanStatement::Let {
            name: "query_generated".to_string(),
            mutable: false,
            value: incan_vocab::IncanExpr::Int(1),
        }]);
        let output_payload = serde_json::to_string(&response)?;
        let wasm = compile_desugarer_wasm_requiring_request_substring(
            &output_payload,
            "missing scoped query surface artifact",
            r#""descriptor_key":"query.field""#,
        )?;
        write_pub_library_with_querykit_surface_desugarer(tmp.path(), &wasm)?;

        let main_path = write_project_files(
            tmp.path(),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"deps/querykit\" }\n",
            r#"import pub::querykit

def main() -> None:
  query:
    .amount > 100
    .customer_id
"#,
        )?;

        let output = run_check(&main_path)?;
        assert!(
            output.status.success(),
            "expected check to succeed when querykit-style leading-dot artifacts reach the desugarer.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let negative_main_path = write_project_files(
            tmp.path().join("negative_consumer").as_path(),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"../deps/querykit\" }\n",
            r#"import pub::querykit

def main() -> None:
  query:
    amount > 100
"#,
        )?;
        let negative_output = run_check(&negative_main_path)?;
        assert!(
            !negative_output.status.success(),
            "expected check to fail when no scoped query artifact reaches the desugarer.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&negative_output.stdout),
            String::from_utf8_lossy(&negative_output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&negative_output.stderr).contains("missing scoped query surface artifact"),
            "expected desugarer failure to prove the request substring assertion was active.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&negative_output.stdout),
            String::from_utf8_lossy(&negative_output.stderr)
        );
        Ok(())
    }

    #[test]
    fn consumer_check_passes_expr_list_item_metadata_to_desugarer_issue724() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let response = incan_vocab::DesugarResponse::statements(vec![incan_vocab::IncanStatement::Let {
            name: "query_generated".to_string(),
            mutable: false,
            value: incan_vocab::IncanExpr::Int(1),
        }]);
        let output_payload = serde_json::to_string(&response)?;
        let wasm = compile_desugarer_wasm_requiring_request_substring(
            &output_payload,
            "missing expression-list modifier payload",
            r#""keyword":"with""#,
        )?;
        write_pub_library_with_querykit_select_desugarer(tmp.path(), &wasm)?;

        let main_path = write_project_files(
            tmp.path(),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"deps/querykit\" }\n",
            r#"import pub::querykit

def main() -> None:
  query:
    SELECT:
      sum(amount) as total for customer with context
"#,
        )?;

        let output = run_check(&main_path)?;
        assert!(
            output.status.success(),
            "expected check to pass expression-list item metadata to the desugarer.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    #[test]
    fn consumer_check_desugars_colon_vocab_expression_in_assignment_issue727() -> Result<(), Box<dyn std::error::Error>>
    {
        let tmp = tempfile::tempdir()?;
        let response = incan_vocab::DesugarResponse::expression(incan_vocab::IncanExpr::Int(7));
        let output_payload = serde_json::to_string(&response)?;
        let wasm = compile_desugarer_wasm_requiring_request_substring(
            &output_payload,
            "missing expression-desugaring declaration payload",
            r#""keyword":"query""#,
        )?;
        write_pub_library_with_querykit_select_desugarer(tmp.path(), &wasm)?;

        let main_path = write_project_files(
            tmp.path(),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"deps/querykit\" }\n",
            r#"import pub::querykit

def main() -> None:
  value: int = query:
    SELECT:
      amount as total
"#,
        )?;

        let output = run_check(&main_path)?;
        assert!(
            output.status.success(),
            "expected check to desugar expression-position vocab block in assignment.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    #[test]
    fn consumer_check_desugars_colon_vocab_expression_in_return_issue727() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let response = incan_vocab::DesugarResponse::expression(incan_vocab::IncanExpr::Int(7));
        let output_payload = serde_json::to_string(&response)?;
        let wasm = compile_desugarer_wasm_requiring_request_substring(
            &output_payload,
            "missing expression-desugaring declaration payload",
            r#""keyword":"query""#,
        )?;
        write_pub_library_with_querykit_select_desugarer(tmp.path(), &wasm)?;

        let main_path = write_project_files(
            tmp.path(),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"deps/querykit\" }\n",
            r#"import pub::querykit

def build_value() -> int:
  return query:
    SELECT:
      amount as total
"#,
        )?;

        let output = run_check(&main_path)?;
        assert!(
            output.status.success(),
            "expected check to desugar expression-position vocab block in return.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    #[test]
    fn consumer_check_desugars_colon_vocab_expression_preserves_inline_clauses_issue727()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let response = incan_vocab::DesugarResponse::expression(incan_vocab::IncanExpr::Int(7));
        let output_payload = serde_json::to_string(&response)?;
        let wasm = compile_desugarer_wasm_requiring_request_substring(
            &output_payload,
            "missing inline FROM clause payload",
            r#""keyword":"FROM""#,
        )?;
        write_pub_library_with_querykit_expression_clause_desugarer(tmp.path(), &wasm)?;

        let main_path = write_project_files(
            tmp.path(),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"deps/querykit\" }\n",
            r#"import pub::querykit

def main() -> None:
  selected: int = query:
    FROM orders
    SELECT:
      amount as total
"#,
        )?;

        let output = run_check(&main_path)?;
        assert!(
            output.status.success(),
            "expected check to pass inline colon-expression clauses to the desugarer.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    #[test]
    fn consumer_check_desugars_braced_vocab_expression_with_compound_clauses_issue727()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let response = incan_vocab::DesugarResponse::expression(incan_vocab::IncanExpr::Int(7));
        let output_payload = serde_json::to_string(&response)?;
        let wasm = compile_desugarer_wasm_requiring_request_substring(
            &output_payload,
            "missing compound clause payload",
            r#""compound_tokens":["BY"]"#,
        )?;
        write_pub_library_with_querykit_expression_clause_desugarer(tmp.path(), &wasm)?;

        let main_path = write_project_files(
            tmp.path(),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"deps/querykit\" }\n",
            r#"import pub::querykit

def main() -> None:
  value: int = query { FROM orders GROUP BY amount as grouped SELECT total as total }
"#,
        )?;

        let output = run_check(&main_path)?;
        assert!(
            output.status.success(),
            "expected check to desugar braced expression-position vocab block.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    #[test]
    fn consumer_check_desugared_public_field_callee_call_typechecks_as_method_issue727()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let response = incan_vocab::DesugarResponse::expression(incan_vocab::IncanExpr::Call {
            callee: Box::new(incan_vocab::IncanExpr::Field {
                object: Box::new(incan_vocab::IncanExpr::Name("orders".to_string())),
                field: "select".to_string(),
            }),
            args: Vec::new(),
        });
        let output_payload = serde_json::to_string(&response)?;
        let wasm = compile_desugarer_wasm_requiring_request_substring(
            &output_payload,
            "missing FROM clause payload",
            r#""keyword":"FROM""#,
        )?;
        write_pub_library_with_querykit_expression_clause_desugarer(tmp.path(), &wasm)?;

        let main_path = write_project_files(
            tmp.path(),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"deps/querykit\" }\n",
            r#"import pub::querykit

class LazyFrame:
  def select(self) -> Self:
    return self

def main() -> None:
  orders = LazyFrame()
  selected: LazyFrame = query { FROM orders SELECT amount as amount }
"#,
        )?;

        let output = run_check(&main_path)?;
        assert!(
            output.status.success(),
            "expected public field-callee desugar output to typecheck as a method call.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    #[test]
    fn consumer_check_desugared_generic_method_call_uses_expected_return_type_issue735()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let response = incan_vocab::DesugarResponse::expression(incan_vocab::IncanExpr::Call {
            callee: Box::new(incan_vocab::IncanExpr::Field {
                object: Box::new(incan_vocab::IncanExpr::Name("orders".to_string())),
                field: "select".to_string(),
            }),
            args: vec![incan_vocab::IncanExpr::List(vec![incan_vocab::IncanExpr::Call {
                callee: Box::new(incan_vocab::IncanExpr::Name("with_column_assignment".to_string())),
                args: vec![
                    incan_vocab::IncanExpr::Str("customer".to_string()),
                    incan_vocab::IncanExpr::Call {
                        callee: Box::new(incan_vocab::IncanExpr::Name("current_field".to_string())),
                        args: vec![
                            incan_vocab::IncanExpr::Name("orders".to_string()),
                            incan_vocab::IncanExpr::Str("customer_id".to_string()),
                        ],
                    },
                ],
            }])],
        });
        let output_payload = serde_json::to_string(&response)?;
        let wasm = compile_desugarer_wasm_requiring_request_substring(
            &output_payload,
            "missing SELECT clause payload",
            r#""keyword":"SELECT""#,
        )?;
        write_pub_library_with_querykit_expression_clause_desugarer(tmp.path(), &wasm)?;

        let main_path = write_project_files(
            tmp.path(),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"deps/querykit\" }\n",
            r#"import pub::querykit

@derive(Clone)
model Order:
  customer_id: str

@derive(Clone)
model Selected:
  customer: str

@derive(Clone)
model ColumnExpr:
  source: str

@derive(Clone)
model ColumnAssignment[T with Clone]:
  name: str

def current_field[T with Clone](_frame: LazyFrame[T], source: str) -> ColumnExpr:
  return ColumnExpr(source=source)

def with_column_assignment[T with Clone](name: str, _expr: ColumnExpr) -> ColumnAssignment[T]:
  return ColumnAssignment[T](name=name)

@derive(Clone)
class LazyFrame[T with Clone]:
  _type_witness: list[T]

  def select[U with Clone](self, columns: list[ColumnAssignment[U]]) -> LazyFrame[U]:
    return LazyFrame[U](_type_witness=[])

def direct_method_call(orders: LazyFrame[Order]) -> LazyFrame[Selected]:
  return orders.select([with_column_assignment("customer", current_field(orders, "customer_id"))])

def query_block_call(orders: LazyFrame[Order]) -> LazyFrame[Selected]:
  return query { FROM orders SELECT customer_id as customer }
"#,
        )?;

        let output = run_check(&main_path)?;
        assert!(
            output.status.success(),
            "expected desugared generic method call to use the same contextual return type as direct source.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    #[test]
    fn consumer_test_activates_dependency_vocab_surfaces_issue730_issue756() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let response = incan_vocab::DesugarResponse::expression(incan_vocab::IncanExpr::Int(7));
        let output_payload = serde_json::to_string(&response)?;
        let wasm = compile_desugarer_wasm_requiring_request_substring(
            &output_payload,
            "missing SELECT clause payload",
            r#""keyword":"SELECT""#,
        )?;
        write_pub_library_with_querykit_expression_clause_desugarer(tmp.path(), &wasm)?;

        write_project_files(
            tmp.path(),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"deps/querykit\" }\n",
            "def main() -> None:\n  return\n",
        )?;
        let tests_dir = tmp.path().join("tests");
        std::fs::create_dir_all(&tests_dir)?;
        let test_path = tests_dir.join("test_query_vocab.incn");
        std::fs::write(
            &test_path,
            r#"import pub::querykit

def test_dependency_vocab_query_block() -> None:
    selected: int = query {
        FROM orders
        GROUP BY
            amount as grouped,
            region as region_group
        SELECT
            amount as total
        ORDER BY amount
        WINDOW BY
            rank = amount
    }
    assert selected == 7
"#,
        )?;

        let fmt_output = run_fmt(&test_path)?;
        assert!(
            fmt_output.status.success(),
            "expected incan fmt to parse dependency-activated vocab in a nested package test file.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&fmt_output.stdout),
            String::from_utf8_lossy(&fmt_output.stderr)
        );

        let formatted_source = std::fs::read_to_string(&test_path)?;
        for clause in [
            "selected: int = query {",
            "        GROUP BY\n            amount as grouped,\n            region as region_group",
            "        ORDER BY amount",
            "        WINDOW BY\n            rank = amount",
        ] {
            assert!(
                formatted_source.contains(clause),
                "expected incan fmt to preserve dependency-vocab expression block shape `{clause}`.\nformatted source:\n{}",
                formatted_source
            );
        }
        for rejected_clause in ["query:", "GROUP BY:", "ORDER BY:", "WINDOW BY:"] {
            assert!(
                !formatted_source.contains(rejected_clause),
                "expected incan fmt not to rewrite expression vocab block through colon clause `{rejected_clause}`.\nformatted source:\n{}",
                formatted_source
            );
        }

        let fmt_output = run_fmt_check(&test_path)?;
        assert!(
            fmt_output.status.success(),
            "expected formatted dependency-activated vocab file to pass incan fmt --check.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&fmt_output.stdout),
            String::from_utf8_lossy(&fmt_output.stderr)
        );

        let check_output = run_check(&test_path)?;
        assert!(
            check_output.status.success(),
            "expected ordinary check to parse dependency-activated vocab in a test file.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&check_output.stdout),
            String::from_utf8_lossy(&check_output.stderr)
        );

        let test_output = run_test(&test_path)?;
        assert!(
            test_output.status.success(),
            "expected incan test to parse and run dependency-activated vocab in a test file.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&test_output.stdout),
            String::from_utf8_lossy(&test_output.stderr)
        );
        Ok(())
    }

    #[test]
    fn consumer_build_and_test_desugars_quality_expression_vocab_clauses_issue813()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let response = incan_vocab::DesugarResponse::expression(incan_vocab::IncanExpr::Int(7));
        let output_payload = serde_json::to_string(&response)?;
        let wasm = compile_desugarer_wasm_requiring_request_substring(
            &output_payload,
            "quality expression vocab body did not expose EXPECT as a clause",
            r#""keyword":"EXPECT""#,
        )?;
        write_pub_library_with_mixed_quality_clause_desugarer(tmp.path(), &wasm)?;

        let main_path = write_project_files(
            tmp.path(),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"deps/querykit\" }\n",
            r#"import pub::querykit

def main() -> None:
    checks: int = quality {
        FROM orders
        REQUIRE row_count() >= 1 as non_empty_orders
        GROUP BY .customer_id
        EXPECT count() >= 1 as customer_groups_present
    }
    assert checks == 7
"#,
        )?;
        let tests_dir = tmp.path().join("tests");
        std::fs::create_dir_all(&tests_dir)?;
        let test_path = tests_dir.join("test_quality.incn");
        std::fs::write(
            &test_path,
            r#"import pub::querykit

def test_quality_expression_vocab_result() -> None:
    checks: int = quality {
        FROM orders
        REQUIRE row_count() >= 1 as non_empty_orders
        GROUP BY .customer_id
        EXPECT count() >= 1 as customer_groups_present
    }
    assert checks == 7
"#,
        )?;

        let check_output = run_check(&main_path)?;
        assert!(
            check_output.status.success(),
            "expected quality expression vocab declaration to typecheck.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&check_output.stdout),
            String::from_utf8_lossy(&check_output.stderr)
        );

        let out_dir = tmp.path().join("out");
        let build_output = run_build(&main_path, &out_dir)?;
        let generated_main = std::fs::read_to_string(out_dir.join("src/main.rs"))?;
        assert!(
            build_output.status.success(),
            "expected generated Rust build for the quality expression vocab declaration to succeed.\ngenerated main.rs:\n{}\nstdout:\n{}\nstderr:\n{}",
            generated_main,
            String::from_utf8_lossy(&build_output.stdout),
            String::from_utf8_lossy(&build_output.stderr)
        );
        assert!(
            generated_main.contains("checks"),
            "expected generated Rust to retain the desugared typed assignment.\ngenerated main.rs:\n{generated_main}"
        );

        let test_output = run_test(&test_path)?;
        assert!(
            test_output.status.success(),
            "expected incan test to execute a typed result from the quality expression vocab declaration.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&test_output.stdout),
            String::from_utf8_lossy(&test_output.stderr)
        );
        Ok(())
    }

    #[test]
    fn consumer_check_desugars_each_quality_clause_issue813() -> Result<(), Box<dyn std::error::Error>> {
        for (clause, request_fragment) in [
            ("FROM", r#""keyword":"FROM""#),
            ("REQUIRE", r#""keyword":"REQUIRE""#),
            ("GROUP BY", r#""keyword":"GROUP","compound_tokens":["BY"]"#),
            ("EXPECT", r#""keyword":"EXPECT""#),
        ] {
            let tmp = tempfile::tempdir()?;
            let response = incan_vocab::DesugarResponse::expression(incan_vocab::IncanExpr::Int(7));
            let output_payload = serde_json::to_string(&response)?;
            let wasm = compile_desugarer_wasm_requiring_request_substring(
                &output_payload,
                &format!("quality expression vocab body did not expose {clause} as a clause"),
                request_fragment,
            )?;
            write_pub_library_with_mixed_quality_clause_desugarer(tmp.path(), &wasm)?;

            let main_path = write_project_files(
                tmp.path(),
                "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"deps/querykit\" }\n",
                r#"import pub::querykit

def main() -> None:
    checks: int = quality {
        FROM orders
        REQUIRE row_count() >= 1 as non_empty_orders
        GROUP BY .customer_id
        EXPECT count() >= 1 as customer_groups_present
    }
    assert checks == 7
"#,
            )?;

            let check_output = run_check(&main_path)?;
            assert!(
                check_output.status.success(),
                "expected quality expression vocab declaration to expose {clause} to the desugarer.\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&check_output.stdout),
                String::from_utf8_lossy(&check_output.stderr)
            );
        }
        Ok(())
    }

    #[test]
    fn fmt_activates_clean_source_dependency_vocab_before_parsing_issue756() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let producer_root = tmp.path().join("deps").join("querykit");
        std::fs::create_dir_all(producer_root.join("src"))?;
        std::fs::write(
            producer_root.join("incan.toml"),
            "[project]\nname = \"querykit\"\nversion = \"0.1.0\"\n\n[vocab]\ncrate = \"vocab_companion\"\n",
        )?;
        std::fs::write(
            producer_root.join("src/lib.incn"),
            "pub def ready() -> int:\n  return 1\n",
        )?;
        write_vocab_companion_crate_with_source(
            &producer_root,
            "vocab_companion",
            "querykit_vocab_companion",
            r#"use incan_vocab::{ClauseSurface, DeclarationSurface, DslSurface, VocabRegistration};

pub fn library_vocab() -> VocabRegistration {
    VocabRegistration::new().with_surface(
        DslSurface::on_import("querykit").with_declaration(
            DeclarationSurface::named("query")
                .with_clause_body()
                .desugars_to_expression()
                .with_clauses([
                    ClauseSurface::expr("FROM").required(),
                    ClauseSurface::expr_list("SELECT").required(),
                ]),
        ),
    )
}
"#,
        )?;

        let consumer_root = tmp.path().join("consumer");
        std::fs::create_dir_all(consumer_root.join("src"))?;
        std::fs::write(
            consumer_root.join("incan.toml"),
            "[project]\nname = \"consumer\"\nversion = \"0.1.0\"\n\n[dependencies]\nquerykit = { path = \"../deps/querykit\" }\n",
        )?;
        let main_path = consumer_root.join("src/main.incn");
        std::fs::write(
            &main_path,
            r#"import pub::querykit

def main() -> None:
    value = query {
        FROM orders
        SELECT
            amount as total
    }
"#,
        )?;

        let artifact_root = producer_root.join("target").join("lib");
        assert!(
            !artifact_root.exists(),
            "regression must start from a clean source dependency without prebuilt library artifacts"
        );

        let fmt_output = super::incan_command()
            .args(["fmt", main_path.to_string_lossy().as_ref()])
            .env("CARGO_NET_OFFLINE", "true")
            .env(INTERNAL_MANIFEST_OVERRIDE_ENV, consumer_root.join("incan.toml"))
            .env(INTERNAL_PROJECT_ROOT_OVERRIDE_ENV, &consumer_root)
            .output()?;
        assert!(
            fmt_output.status.success(),
            "expected fmt to prepare source dependency vocab before parsing clean query block, even when the parent command carries an internal manifest override.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&fmt_output.stdout),
            String::from_utf8_lossy(&fmt_output.stderr)
        );
        assert!(
            !artifact_root.join("querykit.incnlib").exists(),
            "fmt should activate parser vocab from source without preparing a persistent dependency artifact"
        );
        let combined_output = format!(
            "{}\n{}",
            String::from_utf8_lossy(&fmt_output.stdout),
            String::from_utf8_lossy(&fmt_output.stderr)
        );
        assert!(
            !combined_output.contains("Preparing missing pub::querykit dependency artifact"),
            "fmt source vocab activation should not invoke persistent artifact preparation, got: {combined_output}"
        );

        Ok(())
    }

    #[test]
    fn equivalent_helper_backed_keywords_typecheck() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let response = incan_vocab::DesugarResponse::expression(incan_vocab::IncanExpr::Call {
            callee: Box::new(incan_vocab::IncanExpr::Helper("filter".to_string())),
            args: vec![incan_vocab::IncanExpr::Int(1)],
        });
        let output_payload = serde_json::to_string(&response)?;
        let wasm = compile_desugarer_wasm(0, &output_payload, "")?;
        write_pub_library_with_vocab_desugarer_and_filter_helper_keywords(
            tmp.path(),
            "querykit",
            "querykit_core",
            &wasm,
            &["where", "screen"],
        )?;

        let where_main = write_project_files(
            tmp.path().join("where_consumer").as_path(),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"../deps/querykit\" }\n",
            "import pub::querykit\n\ndef main() -> None:\n  where true:\n    pass\n",
        )?;
        let screen_main = write_project_files(
            tmp.path().join("screen_consumer").as_path(),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nquerykit = { path = \"../deps/querykit\" }\n",
            "import pub::querykit\n\ndef main() -> None:\n  screen true:\n    pass\n",
        )?;

        let where_check = run_check(&where_main)?;
        assert!(
            where_check.status.success(),
            "expected helper-backed `where` check to succeed.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&where_check.stdout),
            String::from_utf8_lossy(&where_check.stderr)
        );

        let screen_check = run_check(&screen_main)?;
        assert!(
            screen_check.status.success(),
            "expected helper-backed `screen` check to succeed.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&screen_check.stdout),
            String::from_utf8_lossy(&screen_check.stderr)
        );

        let where_out_dir = tmp.path().join("where_out");
        let where_build = run_build(&where_main, &where_out_dir)?;
        assert!(
            where_build.status.success(),
            "expected helper-backed `where` build to succeed.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&where_build.stdout),
            String::from_utf8_lossy(&where_build.stderr)
        );
        let screen_out_dir = tmp.path().join("screen_out");
        let screen_build = run_build(&screen_main, &screen_out_dir)?;
        assert!(
            screen_build.status.success(),
            "expected helper-backed `screen` build to succeed.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&screen_build.stdout),
            String::from_utf8_lossy(&screen_build.stderr)
        );
        let where_generated = std::fs::read_to_string(where_out_dir.join("src/main.rs"))?;
        let screen_generated = std::fs::read_to_string(screen_out_dir.join("src/main.rs"))?;
        assert_eq!(
            where_generated, screen_generated,
            "equivalent helper-backed keywords should emit identical Rust"
        );
        Ok(())
    }

    #[test]
    fn provider_requirements_and_pub_vocab_flow_through_build_test_and_lock() -> Result<(), Box<dyn std::error::Error>>
    {
        let tmp = tempfile::tempdir()?;
        let project_root = tmp.path();
        std::fs::create_dir_all(project_root.join("src"))?;
        std::fs::create_dir_all(project_root.join("tests"))?;

        write_pub_library_with_provider_requirements_and_assert_keyword(
            project_root,
            "widgets",
            "widgets_core",
            vec![incan_vocab::CargoDependency {
                crate_name: "axum".to_string(),
                source: incan_vocab::CargoDependencySource::Version("0.8".to_string()),
            }],
            vec!["web"],
        )?;

        std::fs::write(
            project_root.join("incan.toml"),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nwidgets = { path = \"deps/widgets\" }\n",
        )?;
        let main_path = project_root.join("src/main.incn");
        std::fs::write(&main_path, "def main() -> None:\n  pass\n")?;
        std::fs::write(
            project_root.join("tests/test_provider.incn"),
            "import pub::widgets\n\ndef test_provider_parity() -> None:\n  assert true\n",
        )?;

        let build_out_dir = project_root.join("out");
        let build_output = run_build(&main_path, &build_out_dir)?;
        assert!(
            build_output.status.success(),
            "expected build to succeed.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&build_output.stdout),
            String::from_utf8_lossy(&build_output.stderr)
        );

        let lock_output = run_lock(&main_path)?;
        assert!(
            lock_output.status.success(),
            "expected lock to succeed.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&lock_output.stdout),
            String::from_utf8_lossy(&lock_output.stderr)
        );

        let test_output = run_test(&project_root.join("tests"))?;
        assert!(
            test_output.status.success(),
            "expected test run to succeed.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&test_output.stdout),
            String::from_utf8_lossy(&test_output.stderr)
        );

        let build_toml = std::fs::read_to_string(build_out_dir.join("Cargo.toml"))?;
        let lock_toml = std::fs::read_to_string(project_root.join("target/incan_lock/Cargo.toml"))?;
        let test_manifest_path = test_runner_batch_manifest_path(project_root)?;
        let test_toml = std::fs::read_to_string(&test_manifest_path).map_err(|err| {
            std::io::Error::new(
                err.kind(),
                format!(
                    "failed reading test runner Cargo.toml at {}: {err}",
                    test_manifest_path.display()
                ),
            )
        })?;

        for cargo_toml in [&build_toml, &lock_toml, &test_toml] {
            assert!(
                cargo_toml.contains(r#"axum = "0.8""#),
                "expected provider dependency in generated Cargo.toml, got:\n{cargo_toml}"
            );
            assert!(
                cargo_toml.contains("incan_stdlib"),
                "expected stdlib dependency in generated Cargo.toml, got:\n{cargo_toml}"
            );
            assert!(
                cargo_toml.contains("\"web\""),
                "expected provider stdlib feature in generated Cargo.toml, got:\n{cargo_toml}"
            );
        }

        Ok(())
    }

    #[test]
    fn conflicting_provider_requirements_fail_build_test_and_lock() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let project_root = tmp.path();
        std::fs::create_dir_all(project_root.join("src"))?;
        std::fs::create_dir_all(project_root.join("tests"))?;

        write_pub_library_with_provider_requirements(
            project_root,
            "widgets",
            "widgets_core",
            vec![incan_vocab::CargoDependency {
                crate_name: "serde_json".to_string(),
                source: incan_vocab::CargoDependencySource::Version("1.0".to_string()),
            }],
            vec![],
        )?;
        write_pub_library_with_provider_requirements(
            project_root,
            "analytics",
            "analytics_core",
            vec![incan_vocab::CargoDependency {
                crate_name: "serde_json".to_string(),
                source: incan_vocab::CargoDependencySource::Version("2.0".to_string()),
            }],
            vec![],
        )?;

        std::fs::write(
            project_root.join("incan.toml"),
            "[project]\nname = \"consumer\"\n\n[dependencies]\nwidgets = { path = \"deps/widgets\" }\nanalytics = { path = \"deps/analytics\" }\n",
        )?;
        let main_path = project_root.join("src/main.incn");
        std::fs::write(&main_path, "def main() -> None:\n  pass\n")?;
        std::fs::write(
            project_root.join("tests/test_conflict.incn"),
            "def test_conflict_path() -> None:\n  pass\n",
        )?;

        let build_output = run_build(&main_path, &project_root.join("out"))?;
        assert!(
            !build_output.status.success(),
            "expected build to fail for conflicting provider deps.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&build_output.stdout),
            String::from_utf8_lossy(&build_output.stderr)
        );
        let build_stderr = strip_ansi_escapes(&String::from_utf8_lossy(&build_output.stderr));
        assert!(
            build_stderr.contains("failed to merge provider requirements"),
            "expected provider conflict diagnostic in build stderr, got:\n{build_stderr}"
        );
        assert!(
            build_stderr.contains("serde_json"),
            "expected conflicting crate name in build stderr, got:\n{build_stderr}"
        );

        let lock_output = run_lock(&main_path)?;
        assert!(
            !lock_output.status.success(),
            "expected lock to fail for conflicting provider deps.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&lock_output.stdout),
            String::from_utf8_lossy(&lock_output.stderr)
        );
        let lock_stderr = strip_ansi_escapes(&String::from_utf8_lossy(&lock_output.stderr));
        assert!(
            lock_stderr.contains("failed to merge provider requirements"),
            "expected provider conflict diagnostic in lock stderr, got:\n{lock_stderr}"
        );
        assert!(
            lock_stderr.contains("serde_json"),
            "expected conflicting crate name in lock stderr, got:\n{lock_stderr}"
        );

        let test_output = run_test(&project_root.join("tests"))?;
        assert!(
            !test_output.status.success(),
            "expected test to fail for conflicting provider deps.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&test_output.stdout),
            String::from_utf8_lossy(&test_output.stderr)
        );
        let test_stdout = strip_ansi_escapes(&String::from_utf8_lossy(&test_output.stdout));
        assert!(
            test_stdout.contains("failed to merge provider requirements"),
            "expected provider conflict diagnostic in test output, got:\n{test_stdout}"
        );
        assert!(
            test_stdout.contains("serde_json"),
            "expected conflicting crate name in test output, got:\n{test_stdout}"
        );

        Ok(())
    }
}
