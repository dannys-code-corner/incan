//! Layering guardrails to prevent the compiler crate from depending on the runtime stdlib.
//!
//! The compiler (`incan` crate) may only use `incan_stdlib` as a **dev-dependency** (for parity tests).
//! This test scans the root `Cargo.toml` and fails if `incan_stdlib` appears in `[dependencies]`.

#[test]
fn compiler_does_not_depend_on_stdlib_in_main_dependencies() {
    let manifest = include_str!("../Cargo.toml");
    let mut in_dependencies = false;

    for raw_line in manifest.lines() {
        let line = raw_line.trim();
        // Track when we enter/exit the `[dependencies]` table.
        if line.starts_with('[') {
            if line == "[dependencies]" {
                in_dependencies = true;
                continue;
            }
            // Any new section after `[dependencies]` ends the scan window.
            if in_dependencies {
                break;
            }
        }

        if !in_dependencies || line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Strip inline comments for robustness.
        let line_no_comment = line.split('#').next().unwrap_or("").trim();
        if line_no_comment.starts_with("incan_stdlib") {
            panic!("`incan_stdlib` must not appear in [dependencies]; use [dev-dependencies] instead");
        }
    }
}
