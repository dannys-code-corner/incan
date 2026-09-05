//! SPIKE — not for merge. Measures the cost of shipping a package's Body IR as an executable
//! representation and executing across that boundary, versus compiling the dependency from source.
//!
//! Question: does crossing a package boundary by *loading* a representation cost meaningfully more
//! than the ~20ms the interpreted route costs today on local modules?

use std::time::Instant;

use incan::backend::replacement::{
    ReplacementExecutionGraph, ReplacementValue, execute_prevalidated_free_function,
    prepare_free_function_execution_in_graph,
};
use incan::frontend::body_ir::{apply_body_ir_input_contract, build_body_ir_module_v0};
use incan::frontend::typechecker::TypeChecker;
use incan::frontend::{lexer, parser};
use incan_semantics_core::body_ir::BodyIrModule;

const PROVIDER: &str = r#"
pub def collatz_len(start: int) -> int:
  mut n = start
  mut steps = 0
  while n != 1:
    if n % 2 == 0:
      n = n // 2
    else:
      n = 3 * n + 1
    steps = steps + 1
  return steps
"#;

const CONSUMER: &str = r#"
from provider import collatz_len

pub def probe() -> int:
  return collatz_len(27)
"#;

/// Lex, parse, typecheck and lower one module, optionally against one dependency.
fn build(source: &str, path: &[&str], dependency: Option<(&str, &incan::frontend::ast::Program)>) -> BodyIrModule {
    let tokens = lexer::lex(source).unwrap_or_else(|e| panic!("lex: {e:?}"));
    let program = parser::parse(&tokens).unwrap_or_else(|e| panic!("parse: {e:?}"));
    let file = format!("{}.incn", path.join("/"));
    let program = apply_body_ir_input_contract(program, std::path::Path::new(&file))
        .unwrap_or_else(|e| panic!("input contract: {e:?}"));
    let module_path: Vec<String> = path.iter().map(|s| (*s).to_string()).collect();
    let mut checker = TypeChecker::new();
    checker.set_current_module_path(Some(module_path.clone()));
    let deps: Vec<_> = dependency.into_iter().collect();
    if !deps.is_empty() {
        checker.register_dependency_module_path_segments("provider", vec!["provider".to_string()]);
    }
    checker
        .check_with_imports(&program, &deps)
        .unwrap_or_else(|e| panic!("typecheck: {e:?}"));
    let info = checker.type_info().clone();
    build_body_ir_module_v0(&program, &module_path, &info)
}

/// Parse the provider so the consumer can be checked against it.
fn provider_program() -> incan::frontend::ast::Program {
    let tokens = lexer::lex(PROVIDER).unwrap_or_else(|e| panic!("lex: {e:?}"));
    let program = parser::parse(&tokens).unwrap_or_else(|e| panic!("parse: {e:?}"));
    apply_body_ir_input_contract(program, std::path::Path::new("provider.incn"))
        .unwrap_or_else(|e| panic!("input contract: {e:?}"))
}

fn execute(consumer: &BodyIrModule, provider: &BodyIrModule) -> ReplacementValue {
    let graph = ReplacementExecutionGraph::new(consumer, std::iter::once(provider))
        .unwrap_or_else(|e| panic!("graph: {e}"));
    let plan = prepare_free_function_execution_in_graph(graph, "probe", &[], None)
        .unwrap_or_else(|e| panic!("prepare: {e}"));
    execute_prevalidated_free_function(plan)
        .unwrap_or_else(|e| panic!("execute: {e}"))
        .value
}

#[test]
fn spike_measure_package_representation_cost() -> Result<(), Box<dyn std::error::Error>> {
    let provider_ast = provider_program();
    let provider_ir = build(PROVIDER, &["provider"], None);
    let consumer_ir = build(CONSUMER, &["consumer"], Some(("provider", &provider_ast)));

    // Baseline: what the route does today -- the dependency is compiled from source in this process.
    let baseline = execute(&consumer_ir, &provider_ir);
    assert_eq!(baseline, ReplacementValue::Int(111));

    let encoded = serde_json::to_vec(&provider_ir)?;
    let round_tripped: BodyIrModule = serde_json::from_slice(&encoded)?;
    assert_eq!(round_tripped, provider_ir, "the representation must round-trip exactly");

    let from_representation = execute(&consumer_ir, &round_tripped);
    assert_eq!(
        from_representation, baseline,
        "a deserialized representation must execute identically to the compiled-from-source module"
    );

    // ---- Cost of compiling the dependency from source (what a consumer avoids) ----
    let runs = 200;
    let start = Instant::now();
    for _ in 0..runs {
        let _ = build(PROVIDER, &["provider"], None);
    }
    let compile_us = start.elapsed().as_micros() / runs;

    // ---- Cost of loading the representation instead ----
    let start = Instant::now();
    for _ in 0..runs {
        let decoded: BodyIrModule = serde_json::from_slice(&encoded)?;
        std::hint::black_box(&decoded);
    }
    let load_us = start.elapsed().as_micros() / runs;

    let start = Instant::now();
    for _ in 0..runs {
        std::hint::black_box(execute(&consumer_ir, &round_tripped));
    }
    let exec_us = start.elapsed().as_micros() / runs;

    println!("\n=== SPIKE: package representation cost ===");
    println!("provider representation size (JSON):        {} bytes", encoded.len());
    println!("compile dependency from source:             {compile_us} us");
    println!("load representation (deserialize):          {load_us} us");
    println!("execute the cross-boundary call:            {exec_us} us");
    println!("=========================================\n");

    // ---- How does it scale with package size? ----
    println!("=== SPIKE: scaling with package surface ===");
    println!(
        "{:>8}  {:>12}  {:>12}  {:>14}  {:>12}  {:>12}",
        "exports", "json bytes", "json us", "compile us", "bin bytes", "bin us"
    );
    for count in [1usize, 25, 100, 400] {
        let mut source = String::new();
        for i in 0..count {
            source.push_str(&format!(
                "pub def f{i}(start: int) -> int:\n  mut n = start\n  mut steps = 0\n  while n != 1:\n    if n % 2 == 0:\n      n = n // 2\n    else:\n      n = 3 * n + 1\n    steps = steps + 1\n  return steps + {i}\n\n"
            ));
        }
        let ir = build(&source, &["scaled"], None);
        let bytes = serde_json::to_vec(&ir)?;

        let runs = 50;
        let start = Instant::now();
        for _ in 0..runs {
            let decoded: BodyIrModule = serde_json::from_slice(&bytes)?;
            std::hint::black_box(&decoded);
        }
        let load = start.elapsed().as_micros() / runs;

        let start = Instant::now();
        for _ in 0..runs {
            std::hint::black_box(build(&source, &["scaled"], None));
        }
        let compile = start.elapsed().as_micros() / runs;

        let packed = postcard::to_allocvec(&ir).unwrap_or_else(|e| panic!("postcard encode: {e}"));
        let start = Instant::now();
        for _ in 0..runs {
            let decoded: BodyIrModule = postcard::from_bytes(&packed).unwrap_or_else(|e| panic!("postcard decode: {e}"));
            std::hint::black_box(&decoded);
        }
        let packed_load = start.elapsed().as_micros() / runs;
        println!(
            "{count:>8}  {:>12}  {load:>12}  {compile:>14}  {:>12}  {packed_load:>12}",
            bytes.len(),
            packed.len()
        );
    }
    println!("=========================================\n");
    Ok(())
}
