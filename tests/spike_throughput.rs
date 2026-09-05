//! SPIKE — not for merge. Interpreted Body IR versus native Rust on the same compute-bound function.
//!
//! The step cap is raised locally for this measurement; it is an infinite-loop guard, not a
//! performance ceiling.

use std::time::Instant;

use incan::backend::replacement::{
    ReplacementValue, execute_prevalidated_free_function, prepare_free_function_execution,
};
use incan::frontend::body_ir::{apply_body_ir_input_contract, build_body_ir_module_v0};
use incan::frontend::typechecker::TypeChecker;
use incan::frontend::{lexer, parser};
use incan_semantics_core::body_ir::BodyIrModule;

const WORKLOAD: &str = r#"
pub def collatz_total(limit: int) -> int:
  mut total = 0
  mut i = 1
  while i < limit:
    mut n = i
    mut steps = 0
    while n != 1:
      if n % 2 == 0:
        n = n // 2
      else:
        n = 3 * n + 1
      steps = steps + 1
    total = total + steps
    i = i + 1
  return total
"#;

/// The same algorithm, written directly in Rust, as the native baseline.
fn collatz_total_native(limit: i64) -> i64 {
    let mut total = 0i64;
    let mut i = 1i64;
    while i < limit {
        let mut n = i;
        let mut steps = 0i64;
        while n != 1 {
            if n % 2 == 0 {
                n /= 2;
            } else {
                n = 3 * n + 1;
            }
            steps += 1;
        }
        total += steps;
        i += 1;
    }
    total
}

fn build_workload() -> BodyIrModule {
    let tokens = lexer::lex(WORKLOAD).unwrap_or_else(|e| panic!("lex: {e:?}"));
    let program = parser::parse(&tokens).unwrap_or_else(|e| panic!("parse: {e:?}"));
    let program = apply_body_ir_input_contract(program, std::path::Path::new("workload.incn"))
        .unwrap_or_else(|e| panic!("input contract: {e:?}"));
    let module_path = vec!["workload".to_string()];
    let mut checker = TypeChecker::new();
    checker.set_current_module_path(Some(module_path.clone()));
    checker.check_program(&program).unwrap_or_else(|e| panic!("typecheck: {e:?}"));
    let info = checker.type_info().clone();
    build_body_ir_module_v0(&program, &module_path, &info)
}

#[test]
fn spike_measure_interpreted_versus_native_throughput() -> Result<(), Box<dyn std::error::Error>> {
    let module = build_workload();

    println!("\n=== SPIKE: interpreted vs native throughput ===");
    println!("{:>8}  {:>16}  {:>14}  {:>10}", "limit", "interpreted ms", "native ms", "ratio");

    for limit in [1_000i64, 5_000, 20_000] {
        let args = [ReplacementValue::Int(limit)];

        // Correctness first: both routes must agree before either is timed.
        let plan = prepare_free_function_execution(&module, "collatz_total", &args)
            .unwrap_or_else(|e| panic!("prepare: {e}"));
        let interpreted = execute_prevalidated_free_function(plan)
            .unwrap_or_else(|e| panic!("execute: {e}"))
            .value;
        let native = collatz_total_native(limit);
        assert_eq!(
            interpreted,
            ReplacementValue::Int(native),
            "interpreted and native must agree before timing means anything"
        );

        let start = Instant::now();
        let plan = prepare_free_function_execution(&module, "collatz_total", &args)
            .unwrap_or_else(|e| panic!("prepare: {e}"));
        std::hint::black_box(execute_prevalidated_free_function(plan).unwrap_or_else(|e| panic!("execute: {e}")));
        let interpreted_ms = start.elapsed().as_secs_f64() * 1000.0;

        let runs = 20;
        let start = Instant::now();
        for _ in 0..runs {
            std::hint::black_box(collatz_total_native(limit));
        }
        let native_ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);

        println!(
            "{limit:>8}  {interpreted_ms:>16.1}  {native_ms:>14.3}  {:>9.0}x",
            interpreted_ms / native_ms
        );
    }
    println!("==============================================\n");
    Ok(())
}
