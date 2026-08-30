//! Tracked coverage of the real example corpus under the replacement backend.
//!
//! Every focused suite for the replacement backend passes against fixtures written to exercise one construct each.
//! That says nothing about whether the backend can run a program somebody would actually write, and the gap between
//! those two questions went unmeasured long enough that "no example executes at all" was discovered by accident
//! rather than reported. This suite exists to make that a number.
//!
//! Two numbers, because they fail for different reasons and have different owners:
//!
//! - **Represented**: the example lowers to Body IR. A shortfall here is source-representation work (#1101).
//! - **Executed**: the example's `main` runs to a value. A shortfall here is executor work (#988 and its slice).
//!
//! Both are recorded as exact baselines rather than floors. A floor would have let the executed count sit at zero
//! forever without anything saying so — which is precisely how this went unnoticed — and a floor of zero is not an
//! assertion at all. An exact baseline makes movement in *either* direction a deliberate, reviewed event: improve
//! the backend and the suite tells you to record the new number in the same change.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use incan::backend::replacement::execute_free_function;
use incan::frontend::body_ir::build_body_ir_module_v0;
use incan::frontend::typechecker::TypeChecker;
use incan::frontend::{lexer, parser};

/// Examples that lower to Body IR today. Update this in the same change that moves it.
const REPRESENTED_BASELINE: usize = 61;

/// Examples whose `main` executes today. Update this in the same change that moves it.
///
/// It moved from zero to four when `print` gained a represented builtin identity and an executed implementation:
/// 25 of the 68 examples had been stopping at their first call.
///
/// The remaining blockers have owners, and two of them are sequenced rather than merely unstarted. #1250 owns model
/// construction and method dispatch; #989 owns imports and multi-module execution. Reassignment belongs to #1072
/// (plain assignment must walk enclosing scopes) followed by RFC 120's Slice 5, which replaces Body IR's flat
/// name-to-local map with identity-keyed resolution — the map is interim by the RFC's own account, so a fix built
/// on it inside Body IR would be resolving bindings in the one place the RFC says must not decide them.
///
/// #1252 owns the other half of the problem, and it is the one that decides what this number is worth: the corpus
/// covers roughly a third of the capability surface the v0.5 catalogue documents, so reaching 68 here would still
/// leave `if let`, generators, iterator adapters, value enums and most of the standard library unexecuted. Both
/// sit under Slice 1 (#1137), because execution evidence has to be trustworthy before anything is cut over to it.
const EXECUTED_BASELINE: usize = 4;

/// How far one example got through the replacement pipeline.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Outcome {
    /// Ran to a value.
    Executed,
    /// Lowered to Body IR but refused during execution, with the refusal's own wording.
    RepresentedNotExecuted(String),
    /// Did not reach Body IR. Covers imports and other multi-module shapes a single file cannot resolve.
    NotRepresented(String),
}

/// Collect every committed example source, skipping build output.
fn example_sources() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
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
    Ok(found)
}

/// Run one example as far through the replacement pipeline as it will go.
fn classify(source_path: &Path) -> Result<Outcome, Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(source_path)?;
    let Ok(tokens) = lexer::lex(&source) else {
        return Ok(Outcome::NotRepresented("lex".to_string()));
    };
    let Ok(program) = parser::parse(&tokens) else {
        return Ok(Outcome::NotRepresented("parse".to_string()));
    };

    let module_path = vec!["example_coverage".to_string()];
    let mut checker = TypeChecker::new();
    checker.set_current_module_path(Some(module_path.clone()));
    if checker.check_program(&program).is_err() {
        return Ok(Outcome::NotRepresented("typecheck".to_string()));
    }

    let module = build_body_ir_module_v0(&program, &module_path, checker.type_info());
    match execute_free_function(&module, "main", &[]) {
        Ok(_) => Ok(Outcome::Executed),
        Err(error) => Ok(Outcome::RepresentedNotExecuted(refusal_bucket(&format!("{error:?}")))),
    }
}

/// Reduce a refusal to a stable bucket so the report groups by cause rather than by span.
fn refusal_bucket(reported: &str) -> String {
    // `MissingFunction` is not a defect: a library module has no entrypoint to run, and this path requires one.
    // It is bucketed rather than dropped so the denominator stays honest about what was actually attempted.
    if reported.contains("MissingFunction") {
        return "no `main` (library module, not a defect)".to_string();
    }
    for marker in [
        "import declaration",
        "non-function top-level declaration",
        "repeated user binding",
        "constructor",
        "call to function",
        "call to method",
        "f-string",
        "dict aggregate",
        "collection iteration",
        "unsupported Body-IR type",
    ] {
        if reported.contains(marker) {
            return marker.to_string();
        }
    }
    "other".to_string()
}

#[test]
fn replacement_example_corpus_coverage_does_not_regress() -> Result<(), Box<dyn std::error::Error>> {
    let sources = example_sources()?;
    assert!(!sources.is_empty(), "the example corpus must not be empty");

    let mut executed = 0usize;
    let mut represented = 0usize;
    let mut buckets: BTreeMap<String, usize> = BTreeMap::new();

    for source_path in &sources {
        match classify(source_path)? {
            Outcome::Executed => {
                executed += 1;
                represented += 1;
            }
            Outcome::RepresentedNotExecuted(bucket) => {
                represented += 1;
                *buckets.entry(bucket).or_default() += 1;
            }
            Outcome::NotRepresented(stage) => {
                *buckets.entry(format!("not represented ({stage})")).or_default() += 1;
            }
        }
    }

    println!("replacement backend coverage over {} committed examples", sources.len());
    println!("  represented (lowers to Body IR): {represented}");
    println!("  executed (main runs to a value): {executed}");
    for (bucket, count) in &buckets {
        println!("  blocked by {bucket}: {count}");
    }

    assert_eq!(
        represented, REPRESENTED_BASELINE,
        "representation coverage moved to {represented} from the recorded {REPRESENTED_BASELINE}. If it went up, \
         record the new number here in the same change. If it went down, say why before doing so."
    );
    assert_eq!(
        executed, EXECUTED_BASELINE,
        "execution coverage moved to {executed} from the recorded {EXECUTED_BASELINE}. If it went up, record the \
         new number here in the same change — that is the number this suite exists to move."
    );
    Ok(())
}
