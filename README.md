# Incan Programming Language

*Incan* is short for **Incandescence** - *the glow of something heated until it shines*.  

Incan feels much like Python, but compiles to native Rust. It’s a smooth path from Python to performance: you keep the readability and expressiveness you’re used to, while gaining static typing, predictable speed, and Rust-level safety guarantees. The result is clear, high-level code that runs fast—without giving up ergonomics.

The goal isn’t to be Python, but to be a smooth path for developers familiar with Python to reach the performance and safety that Rust provides.

Incan is also for Rust developers who want a simpler, more expressive way to write everyday application code without giving up Rust’s strengths. It keeps the “good parts” of Rust—static typing, explicit error handling, and native performance—but offers a more concise surface syntax for modeling data, composing behavior, and writing glue code. You can still drop down to Rust crates when needed, while using Incan for the parts where you’d rather focus on intent than boilerplate.

> **⚠️ Alpha Software ⚠️**  
> Incan is in active development. The language, compiler, and APIs are subject to change.  
> Feedback and contributions are of course welcome!

## Features

- **Static typing** with Pydantic-like model ergonomics
- **Rust-style error handling** with `Result`, `Option`, and `?` operator  
- **Behavior-only traits** with deterministic composition (no MRO surprises)
- **Newtypes** for zero-cost type safety
- **Async/await** with structured concurrency
- **First-class Rust interop** with optional Python compatibility

## Performance

Incan compiles to native Rust, often delivering performance close to hand-written Rust while being dramatically faster than Python:

| Benchmark                 | Incan | Rust  | Python   | Incan vs Python   |
|---------------------------|------:|------:|---------:|------------------:|
| Fibonacci (1M iterations) | 15ms  | 17ms  | 490ms    | **32.6×** faster  |
| Collatz (1M numbers)      | 152ms | 155ms | 9,043ms  | **59.4×** faster  |
| GCD (10M pairs)           | 277ms | 298ms | 2,037ms  | **7.3×** faster   |
| Mandelbrot (2K×2K)        | 250ms | 248ms | 12,268ms | **49.0×** faster  |
| N-Body (500K steps)       | 39ms  | 39ms  | 4,934ms  | **126.5×** faster |
| Prime Sieve (10M)         | 117ms | 120ms | 9,520ms  | **81.3×** faster  |
| Quicksort (1M elements)   | 79ms  | 78ms  | 2,435ms  | **30.8×** faster  |
| Mergesort (1M elements)   | 195ms | 196ms | 3,629ms  | **18.9×** faster  |

**Benchmark details:**

- **Machine:** Apple Silicon (results may vary)
- **Incan/Rust:** Release builds with optimizations
- **Python:** CPython 3.12
- **Methodology:** [hyperfine](https://github.com/sharkdp/hyperfine) with warmup runs

Benchmarks are in [`benchmarks/`](benchmarks/) - run with `./benchmarks/run_all.sh`.

## Quick Example

This example attempts to show Incan's overall design and ergonomics.

```incan
enum AppError:
  NotFound
  InvalidInput(str)

type Email = newtype str:
  def from_str(v: str) -> Result[Email, AppError]:
    if "@" not in v:
      return Err(AppError.InvalidInput("missing @"))
    return Ok(Email(v.lower()))

@derive(Debug, Eq, Clone)
model User:
  id: int
  email: Email
  is_active: bool = true

trait Loggable:
  def log(self, msg: str):
    print(f"[{self.name}] {msg}")

class UserService with Loggable:
  name: str
  users: Dict[int, User]

  def create(mut self, email_str: str) -> Result[User, AppError]:
    email = Email.from_str(email_str)?
    user = User(id=len(self.users) + 1, email=email)
    self.users[user.id] = user
    self.log(f"created user {user.id}")
    return Ok(user)
```

### Python vs Incan (at a glance)

The code above should feel familiar if you write Python: indentation-based syntax and straightforward control flow. The difference is what you get **without changing how you like to write**:

- **Static typing by default**: better editor tooling and earlier feedback (fewer late surprises).
- **Rust-grade performance**: Incan compiles to native Rust, so hot loops and data-heavy workloads run fast.
- **Safety with explicit error handling**: `Result`, `Option`, and `?` encourage correct-by-construction programs.

Incan also adds language features that Python doesn’t have (or typically gets via libraries):

- **Data modeling**: `model` and `newtype` for structured data with defaults and explicit conversions.
- **Traits**: `trait` for defining behavior contracts and composing functionality deterministically.
- **Derives**: `@derive` for common traits like `Debug`, `Clone`, `Eq`, `Ord`, `Hash`, `Serialize`, and `Deserialize`.
- **Visibility (`pub`/private)**: designed to be private by default, with explicit `pub` for public APIs.
- **No GIL**: parallelism is real. Async tasks and threads can run across cores without a global interpreter lock.

Incan also intentionally removes (or avoids relying on) a few things that can make large codebases harder to reason about:

- **Multiple inheritance / MRO**: composition is explicit and deterministic (no method-resolution surprises).
- **Duck typing as the default**: behavior is expressed via explicit types and trait contracts, so refactors are safer.
- **Implicit “anything goes” mutation**: mutation is opt-in (`mut`), which keeps intent clear.

Here’s a small example that reads like Python, but is statically typed and compiled:

```incan
def sum_even(nums: List[int]) -> int:
  mut total = 0
  for n in nums:
    if n % 2 == 0:
      total += n
  return total

def main() -> None:
  nums = [1, 2, 3, 4, 5, 6]
  println(f"sum_even = {sum_even(nums)}")
```

See the [Documentation](#documentation) section for more examples and guides.

## Building

```bash
# Build the compiler (release)
make release

# Run a file (using the locally-built compiler binary)
./target/release/incan run examples/simple/hello.incn

# Typecheck only
./target/release/incan --check examples/simple/hello.incn

# Smoke-test examples (checks all files; runs only quick entrypoints)
make examples

# Run benchmark suite (requires hyperfine)
make benchmarks
```

## Project Structure

```text
src/                        # Rust compiler library
├── lib.rs                  # Compiler library entry point
├── main.rs                 # CLI entry point (`incan`)
├── frontend/               # Frontend module: Lexer → parser → AST → typechecking
│   ├── lexer/              # Tokenizer (split by concern)
│   │   ├── mod.rs
│   │   ├── indent.rs       # Indentation + newline handling
│   │   ├── numbers.rs      # Numeric literal lexing
│   │   ├── strings.rs      # String + f-string lexing
│   │   └── tokens.rs       # Token kinds + keyword table
│   ├── parser.rs           # Parser (tokens → AST)
│   ├── ast.rs              # AST definitions
│   ├── symbols.rs          # Symbol table + resolved types
│   ├── typechecker.rs      # Type checker / inference rules
│   └── diagnostics.rs      # Error reporting + spans
├── backend/                # Backend module: Rust code generation and project emission
│   ├── codegen/            # Expression/statement/function emission
│   ├── rust_emitter.rs     # Pretty-printer for Rust output
│   └── project.rs          # Cargo project scaffolding for generated code
├── cli/                    # CLI subcommands (run/build/check/fmt/test)
├── format/                 # Incan formatter (`incan fmt`)
└── lsp/                    # LSP backend logic (diagnostics/hover/goto)

stdlib/                      # Incan stdlib (prelude, traits, derives, web, async)
examples/                    # Runnable language examples (simple/intermediate/advanced/web)
benchmarks/                  # Benchmark suite + runner scripts
scripts/                     # Helper scripts (examples runner, benchmarks wrapper)
docs/                        # Guides, tooling docs, roadmap, RFCs

tests/
├── integration_tests.rs     # End-to-end compiler tests
└── fixtures/
    ├── valid/               # Should compile
    └── invalid/             # Should error
```

**How the pieces fit together:**

- **`src/frontend/`**: turns source text into a checked AST (lex → parse → typecheck).
- **`src/backend/`**: emits Rust code + a Cargo project, then builds/runs it.
- **`src/cli/`**: user-facing commands (`incan run`, `incan build`, `incan --check`, etc.).
- **`src/format/`**: the Incan formatter (`incan fmt`).
- **`src/lsp/` + `src/bin/lsp.rs`**: Language Server support for editor diagnostics and navigation.
- **`stdlib/`**: the language’s standard library and prelude definitions.
- **`examples/` / `benchmarks/`**: real programs used as smoke tests and performance validation.

## Design Principles

1. **Readability counts** – clarity over cleverness
2. **Safety over silence** – errors surface as `Result`, not hide
3. **Explicit over implicit** – magic is opt-in and marked
4. **Fast is better than slow** – performance costs must be visible
5. **One obvious way** – conventions beat novelty

## Documentation

**Tooling & Setup:**

- [Getting Started](docs/tooling/getting_started.md) - Installation and first steps
- [Editor Setup](docs/tooling/editor_setup.md) - IDE and syntax highlighting
- [Formatting](docs/tooling/formatting.md) - Code formatter (`incan fmt`)
- [Testing](docs/tooling/testing.md) - Test runner and fixtures

**Language Guide:**

- [Error Handling](docs/guide/error_handling.md) - Result, Option, and `?`
- [Async Programming](docs/guide/async_programming.md) - Async/await with Tokio
- [Derives & Traits](docs/guide/derives_and_traits.md) - Derive macros and traits
- [Rust Interop](docs/guide/rust_interop.md) - Using Rust crates from Incan

**Reference:**

- [Full Documentation](docs/README.md) - Overview and navigation
- [Roadmap](docs/ROADMAP.md) - Status and planned work
- [RFCs](docs/RFCs/) - Design proposals and decisions

## CLI Commands

```bash
# Build and run an Incan program
incan run myprogram.incn

# Build without running
incan build myprogram.incn

# Type check only
incan --check myprogram.incn

# Format code
incan fmt .
incan fmt --check .   # CI mode

# Run tests (pytest-style)
incan test tests/
incan test -k "addition"  # Filter by keyword
```

## Status

**Implemented:**

- Compiler frontend (lexer, parser, type checker)
- Rust code generation backend
- Async/await with Tokio runtime
- Derive system (Debug, Clone, Eq, Ord, Hash, Serialize, Deserialize)
- Code formatter (`incan fmt`)
- Test runner (`incan test`) with pytest-style output

**Next:** LSP support, fixtures, parametrized tests

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

Apache 2.0
