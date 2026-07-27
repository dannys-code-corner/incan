use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod support;

fn incan_binary() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_incan") {
        return PathBuf::from(path);
    }
    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        let path = PathBuf::from(target_dir).join("debug").join("incan");
        if path.exists() {
            return path;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/incan")
}

fn run_incan(current_dir: &Path, args: &[&str]) -> Result<Output, Box<dyn std::error::Error>> {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let stdlib_root = source_root.join("crates/incan_stdlib/stdlib");
    Ok(Command::new(incan_binary())
        .args(args)
        .current_dir(current_dir)
        .env("CARGO_NET_OFFLINE", "true")
        .env("INCAN_NO_BANNER", "1")
        .env(
            "INCAN_GENERATED_CARGO_TARGET_DIR",
            support::generated_cargo_target_dir(),
        )
        .env("INCAN_INTERNAL_SDK_PROVIDER_STORE", support::sdk_provider_store())
        .env("INCAN_SOURCE_ROOT", source_root)
        .env("INCAN_STDLIB", &stdlib_root)
        .env("INCAN_STDLIB_DIR", &stdlib_root)
        .env("INCAN_TOOLCHAIN_CRATES_DIR", source_root.join("crates"))
        .output()?)
}

fn run_cargo(current_dir: &Path, args: &[&str], target_dir: &Path) -> Result<Output, Box<dyn std::error::Error>> {
    Ok(Command::new("cargo")
        .args(args)
        .current_dir(current_dir)
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", target_dir)
        .output()?)
}

fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_fixture_file(root: &Path, relative_path: &str, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = root.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

/// Materialize the generated-library producer fixture, including sealed-model coverage.
fn write_producer(root: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let producer = root.join("native_items");
    write_fixture_file(
        &producer,
        "incan.toml",
        include_str!("fixtures/generated_rust_native_consumer/producer/incan.toml"),
    )?;
    write_fixture_file(
        &producer,
        "src/lib.incn",
        include_str!("fixtures/generated_rust_native_consumer/producer/src/lib.incn"),
    )?;
    write_fixture_file(
        &producer,
        "src/counters.incn",
        include_str!("fixtures/generated_rust_native_consumer/producer/src/counters.incn"),
    )?;
    write_fixture_file(
        &producer,
        "src/admission.incn",
        include_str!("fixtures/generated_rust_native_consumer/producer/src/admission.incn"),
    )?;
    Ok(producer)
}

fn write_consumer(root: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let consumer = root.join("native_consumer");
    write_fixture_file(
        &consumer,
        "Cargo.toml",
        include_str!("fixtures/generated_rust_native_consumer/consumer/Cargo.toml"),
    )?;
    write_fixture_file(
        &consumer,
        "src/lib.rs",
        include_str!("fixtures/generated_rust_native_consumer/consumer/src/lib.rs"),
    )?;
    Ok(consumer)
}

/// Materialize the native Rust crate that must not forge private model construction.
fn write_forge(root: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let forge = root.join("forge");
    write_fixture_file(
        &forge,
        "Cargo.toml",
        include_str!("fixtures/generated_rust_native_consumer/forge/Cargo.toml"),
    )?;
    write_fixture_file(
        &forge,
        "src/lib.rs",
        include_str!("fixtures/generated_rust_native_consumer/forge/src/lib.rs"),
    )?;
    Ok(forge)
}

#[test]
/// Verify generated-library Rust retains public capabilities without exposing private constructors.
fn native_rust_consumer_can_call_generated_public_items() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let producer = write_producer(tmp.path())?;

    let build_output = run_incan(&producer, &["build", "--lib"])?;
    assert_success(&build_output, "incan build --lib native consumer producer");
    let build_diagnostics = format!(
        "{}\n{}",
        String::from_utf8_lossy(&build_output.stdout),
        String::from_utf8_lossy(&build_output.stderr)
    );
    assert!(
        !build_diagnostics.contains("private_interfaces"),
        "generated producer leaked a private type through a public Rust interface:\n{build_diagnostics}"
    );

    let artifact_root = producer.join("target/lib");
    assert!(
        artifact_root.join("Cargo.toml").is_file(),
        "expected generated Rust library Cargo.toml at {}",
        artifact_root.display()
    );
    assert!(
        artifact_root.join("src/lib.rs").is_file(),
        "expected generated Rust library root at {}",
        artifact_root.join("src/lib.rs").display()
    );

    let consumer = write_consumer(tmp.path())?;
    let cargo_test = run_cargo(
        &consumer,
        &["test", "--offline"],
        &tmp.path().join("native-cargo-target"),
    )?;
    assert_success(&cargo_test, "native Rust cargo test against generated library");

    let forge = write_forge(tmp.path())?;
    let forge_check = run_cargo(
        &forge,
        &["check", "--offline", "--all-features"],
        &tmp.path().join("native-cargo-target"),
    )?;
    assert!(
        !forge_check.status.success(),
        "native Rust forge unexpectedly compiled private model constructor inputs.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&forge_check.stdout),
        String::from_utf8_lossy(&forge_check.stderr)
    );
    let forge_diagnostics = String::from_utf8_lossy(&forge_check.stderr);
    for nominal in ["Admission", "Defaulted", "Mixed"] {
        assert!(
            forge_diagnostics.contains(nominal),
            "native Rust forge failed for an unrelated reason; expected a constructor diagnostic for {nominal}:\n\
             {forge_diagnostics}"
        );
    }
    assert!(
        forge_diagnostics.contains("expected function, tuple struct or tuple variant")
            || forge_diagnostics.contains("takes 1 argument but 2 arguments were supplied"),
        "native Rust forge did not fail at the sealed constructor boundary:\n{forge_diagnostics}"
    );

    Ok(())
}
