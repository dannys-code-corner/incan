//! Tracked coverage of the documented capability surface by the real example corpus.
//!
//! [`replacement_example_coverage`] measures what the replacement backend can *run* out of the corpus. It says
//! nothing about what the corpus *contains*, and that is the other half of the same problem (#1252): a backend could
//! reach every committed example and still never execute `if let`, generators, iterator adapters, value enums, or
//! most of the standard library, because no example uses them. A cutover decision resting on a green execution
//! number would be resting on a denominator nobody had checked.
//!
//! The v0.5 capability catalogue is the authority for what Incan claims to ship, so the target list is derived from
//! it rather than invented here. A capability counts as demonstrated when a distinctive fragment of one of its
//! `canonical_forms` appears somewhere in the corpus.
//!
//! Two decisions worth stating, because both are judgement calls a reader should be able to disagree with:
//!
//! - **Matching is on a derived marker, not the whole canonical form.** Requiring the entire form verbatim finds only 8
//!   of 63 capabilities, because forms are illustrative lines (`from std.hash import Sha256Hasher, sha256,
//!   file_digest`) rather than text anyone copies literally. The marker is the form's distinctive head, cut before the
//!   first `(`, `[`, or `=`. That is deliberately generous: this suite answers "is this feature demonstrated anywhere",
//!   and a false *positive* costs one missing example while a false negative would make the number untrustworthy and
//!   get the suite ignored.
//! - **Exact baselines, not floors**, following the sibling suite's reasoning: a floor lets a number sit at its
//!   starting value forever without anything saying so, which is how the original gap survived. An exact count makes
//!   movement in either direction a reviewed event.
//!
//! When this fails, it prints the capabilities that regressed or newly landed. Record the new number in the same
//! change.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use regex::Regex;

/// Stable capabilities whose canonical forms are source, not a CLI invocation.
///
/// Seven of the 63 stable capabilities document themselves with `incan ...` command lines. #1252 puts those out of
/// scope explicitly: an example file cannot demonstrate a shell invocation, and pretending otherwise would inflate
/// the denominator with entries no example could ever satisfy.
const SOURCE_SHAPED_BASELINE: usize = 56;

/// Source-shaped stable capabilities demonstrated by at least one committed example.
///
/// The starting measurement was 18 of 56, not a target: 38 documented stable capabilities had no example at all,
/// which is the gap #1252 exists to close. This number should climb toward [`SOURCE_SHAPED_BASELINE`] as examples
/// land.
///
/// Moved 18 -> 21 by `examples/intermediate/log_levels.incn`, which demonstrates value enums, `if let`, pattern
/// alternation, and `loop:` expressions in one program rather than four. That grouping is deliberate: #1252 asks the
/// corpus to stay readable as teaching material rather than become a conformance dump, so features that co-occur in
/// real code are shown co-occurring.
///
/// Moved 21 -> 22 by `examples/intermediate/signed_payload.incn`. `std.hash` had no example at all despite being
/// stable since 0.3, so the module was one of the fourteen `std.*` gaps this issue names.
const COVERED_BASELINE: usize = 22;

/// One capability entry read from the v0.5 catalogue.
struct Capability {
    /// Stable typed identity, for example `IfWhileLet`.
    id: String,
    /// Illustrative source lines the catalogue publishes for this capability.
    forms: Vec<String>,
}

impl Capability {
    /// Return whether every published form is a command line rather than Incan source.
    fn is_cli_only(&self) -> bool {
        !self.forms.is_empty() && self.forms.iter().all(|form| form.trim_start().starts_with("incan "))
    }
}

/// Return the catalogue path that owns the documented capability surface.
fn catalogue_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/replacement_compatibility/migration_baselines/v0.5.0/capabilities.incn")
}

/// Collect every committed example source into one searchable buffer.
///
/// The corpus is searched as a single text rather than per file because this suite asks whether a capability is
/// demonstrated anywhere, not which example demonstrates it.
fn example_corpus_text() -> Result<String, Box<dyn std::error::Error>> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "target") {
                    continue;
                }
                walk(&path, found)?;
            } else if path.extension().is_some_and(|ext| ext == "incn") {
                found.push(path);
            }
        }
        Ok(())
    }

    let mut found = Vec::new();
    walk(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples").as_path(),
        &mut found,
    )?;
    found.sort();

    let mut text = String::new();
    for path in found {
        text.push_str(&std::fs::read_to_string(&path)?);
        text.push('\n');
    }
    Ok(text)
}

/// Parse the catalogue's stable capabilities and the canonical forms each publishes.
fn stable_capabilities() -> Result<Vec<Capability>, Box<dyn std::error::Error>> {
    let catalogue = std::fs::read_to_string(catalogue_path())?;

    // `const <name>_forms: FrozenList[str] = ["...", "..."]`
    let forms_decl = Regex::new(r#"(?s)const\s+(\w+_forms):\s*FrozenList\[str\]\s*=\s*\[(.*?)\]\n"#)?;
    let quoted = Regex::new(r#""((?:[^"\\]|\\.)*)""#)?;
    let mut forms_by_name = std::collections::BTreeMap::new();
    for capture in forms_decl.captures_iter(&catalogue) {
        let (Some(name), Some(body)) = (capture.get(1), capture.get(2)) else {
            continue;
        };
        let values = quoted
            .captures_iter(body.as_str())
            .filter_map(|form| form.get(1).map(|value| value.as_str().replace("\\\"", "\"")))
            .collect::<Vec<_>>();
        forms_by_name.insert(name.as_str().to_string(), values);
    }

    let entry = Regex::new(r#"(?s)capabilities\.entry\((.*?)\n\)\n"#)?;
    let id_field = Regex::new(r#"id=CapabilityId\("([^"]+)"\)"#)?;
    let stability_field = Regex::new(r#"stability=CapabilityStability\.(\w+)"#)?;
    let forms_field = Regex::new(r#"canonical_forms=(\w+)"#)?;

    let mut capabilities = Vec::new();
    for block in entry.captures_iter(&catalogue) {
        let Some(body) = block.get(1).map(|matched| matched.as_str()) else {
            continue;
        };
        let stable = stability_field
            .captures(body)
            .and_then(|found| found.get(1))
            .is_some_and(|kind| kind.as_str() == "Stable");
        if !stable {
            continue;
        }
        let Some(id) = id_field.captures(body).and_then(|found| found.get(1)) else {
            continue;
        };
        let forms = forms_field
            .captures(body)
            .and_then(|found| found.get(1))
            .and_then(|name| forms_by_name.get(name.as_str()))
            .cloned()
            .unwrap_or_default();
        capabilities.push(Capability {
            id: id.as_str().to_string(),
            forms,
        });
    }
    Ok(capabilities)
}

/// Reduce a canonical form to the distinctive head a real example would contain.
///
/// Forms carry illustrative arguments and receivers that vary between programs (`decode_rows[Order, _](path)`), so
/// everything from the first opening delimiter or assignment onward is dropped. Heads shorter than four characters
/// are rejected rather than matched: `T`, `if`, or `=` would match nearly any source and make coverage meaningless.
fn marker(form: &str) -> Option<String> {
    let trimmed = form.trim();
    let head = trimmed
        .find(['(', '[', '='])
        .map_or(trimmed, |cut| &trimmed[..cut])
        .trim();
    (head.len() >= 4).then(|| normalize_whitespace(head))
}

/// Collapse runs of whitespace so a marker matches source formatted across different line widths.
fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn stable_capability_example_coverage_does_not_regress() -> Result<(), Box<dyn std::error::Error>> {
    let capabilities = stable_capabilities()?;
    let corpus = normalize_whitespace(&example_corpus_text()?);

    let source_shaped = capabilities
        .iter()
        .filter(|capability| !capability.is_cli_only())
        .collect::<Vec<_>>();

    let mut covered = BTreeSet::new();
    let mut uncovered = BTreeSet::new();
    for capability in &source_shaped {
        let demonstrated = capability
            .forms
            .iter()
            .filter_map(|form| marker(form))
            .any(|needle| corpus.contains(&needle));
        if demonstrated {
            covered.insert(capability.id.clone());
        } else {
            uncovered.insert(capability.id.clone());
        }
    }

    assert_eq!(
        source_shaped.len(),
        SOURCE_SHAPED_BASELINE,
        "the catalogue's source-shaped stable capability count moved; record the new denominator with the change \
         that moved it"
    );
    assert_eq!(
        covered.len(),
        COVERED_BASELINE,
        "example coverage of documented stable capabilities moved from {COVERED_BASELINE} to {}.\n\
         Record the new number in this change. Still undemonstrated ({}):\n  {}",
        covered.len(),
        uncovered.len(),
        uncovered.iter().cloned().collect::<Vec<_>>().join("\n  ")
    );
    Ok(())
}
