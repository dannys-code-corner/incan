# Incan Programming Language

> *Incan* is short for *Incandescence*: emitting visible light due to high temperature.

> **⚠️ Alpha Software:** Incan is in active development. The language, compiler, and APIs are subject to change. Feedback and contributions are welcome!

Incan combines the safety and performance of Rust with the expressiveness of Python. It's designed for developers who want Python's ergonomics with Rust's guarantees.

## Features

- **Static typing** with Pydantic-like model ergonomics
- **Rust-style error handling** with `Result`, `Option`, and `?` operator  
- **Behavior-only traits** with deterministic composition (no MRO surprises)
- **Newtypes** for zero-cost type safety
- **Async/await** with structured concurrency (no GIL)
- **First-class Rust interop** with optional Python compatibility

## Quick Example

```incan
import polars::prelude as pl

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

## Building

```bash
# Build the compiler
cargo build

# Run tests
cargo test

# Check a file
cargo run -- --check examples/hello.incn
```

## Project Structure

```text
src/
├── lib.rs              # Library entry point
├── main.rs             # CLI entry point
└── frontend/
    ├── mod.rs          # Frontend module
    ├── lexer.rs        # Tokenizer
    ├── parser.rs       # Parser
    ├── ast.rs          # AST definitions
    ├── symbols.rs      # Symbol table
    ├── typechecker.rs  # Type checker
    └── diagnostics.rs  # Error reporting

tests/
├── integration_tests.rs
└── fixtures/
    ├── valid/          # Should compile
    └── invalid/        # Should error
```

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
