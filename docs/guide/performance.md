# Performance Guide

This guide covers benchmarking and profiling Incan programs.

## Benchmark Suite

Incan includes a comprehensive benchmark suite comparing performance against Rust and Python.

### Available Benchmarks

| Category  | Benchmark    | Description |
|-----------|--------------|-------------|
| Compute   | `fib`        | Iterative Fibonacci (N=1,000,000) |
| Compute   | `collatz`    | Collatz sequence (1,000,000 numbers) |
| Compute   | `gcd`        | GCD of 10,000,000 pairs |
| Compute   | `mandelbrot` | 2000×2000 escape iterations |
| Compute   | `nbody`      | N-body simulation (500,000 steps) |
| Compute   | `primes`     | Sieve up to 50,000,000 |
| Sorting   | `quicksort`  | In-place sort (1M integers) |
| Sorting   | `mergesort`  | Merge sort (1M integers) |

### Running Benchmarks

```bash
# Prerequisites
brew install hyperfine jq bc  # macOS
cargo build --release

# Run all benchmarks
make benchmarks

# Or directly
./benchmarks/run_all.sh
```

### Running Individual Benchmarks

```bash
cd benchmarks/compute/fib

# Build Incan version
../../../target/release/incan build fib.incn
cp ../../../target/incan/fib/target/release/fib ./fib_incan

# Build Rust baseline
rustc -O fib.rs -o fib_rust

# Compare
hyperfine --warmup 2 --min-runs 5 \
  './fib_incan' \
  './fib_rust' \
  'python3 fib.py'
```

## Performance Characteristics

Incan compiles to native Rust, so runtime performance matches hand-written Rust within ~1-3%. The overhead comes from:

1. **Compilation time**: Incan → Rust → binary (two compilation steps compared to 1 give a small performance penalty when it comes to compilation speed)
2. **Generated code patterns**: Some patterns may not optimize as well as hand-written Rust

### Expected Results

| Benchmark  | Incan vs Rust | Incan vs Python |
|------------|---------------|-----------------|
| fib        | ~1.0x         | ~30x faster     |
| mandelbrot | ~1.0x         | ~50x faster     |
| quicksort  | ~1.0x         | ~25x faster     |
| mergesort  | ~1.0x         | ~20x faster     |

> Note: in this table, we are not measuring the performance of the compilers or the time it takes to compile the Rust and Incan code.
> We are only measuring the performance of the (generated) Rust code.

## Profiling

### Profiling Generated Code

```bash
# Build with release optimizations
incan build --release myprogram.incn

# Profile with Instruments (macOS)
xcrun xctrace record --template "Time Profiler" \
  --launch ./target/incan/myprogram/target/release/myprogram

# Profile with perf (Linux)
perf record ./target/incan/myprogram/target/release/myprogram
perf report
```

### Profiling the Compiler

```bash
# Profile compilation itself
cargo flamegraph --bin incan -- build examples/advanced/async_await.incn

# Or with samply (macOS)
samply record ./target/release/incan build large_program.incn
```

## Optimization Tips

1. **Use release builds**: `incan build --release` or `cargo build --release`
2. **Avoid unnecessary clones**: Incan's codegen minimizes clones, but explicit `.clone()` in source will be preserved
3. **Prefer iterators**: List comprehensions compile to iterator chains
4. **Check generated Rust**: Use `incan build --emit-rust` to inspect output

## Adding New Benchmarks

1. Create directory under `benchmarks/compute/` or `benchmarks/sorting/`
2. Add three implementations: `name.incn`, `name.rs`, `name.py`
3. Each should print a single result line for verification
4. Run `./benchmarks/run_all.sh` to include in suite
