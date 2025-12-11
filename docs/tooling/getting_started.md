# Getting Started with Incan

## Installation

### From Source (Current)

Incan is currently built from source using Cargo (Rust's package manager).

```bash
# Clone the repository
git clone https://github.com/dannymeijer/incan-programming-language.git
cd incan-programming-language

# Build the compiler
cargo build --release

# The binary is at target/release/incan
# Optionally add to PATH:
export PATH="$PATH:$(pwd)/target/release"
```

### Prerequisites

- **Rust** (1.85+): Install via [rustup](https://rustup.rs/) — Incan uses Rust 2024 edition
- **Cargo**: Included with Rust

## Your First Program

Create a file `hello.incn`:

```incan
def main() -> None:
    println("Hello, Incan!")
```

Run it:

```bash
incan run hello.incn
```

## Project Structure

A typical Incan project:

```
my_project/
├── src/
│   ├── main.incn      # Entry point
│   └── utils.incn     # Additional modules
├── tests/
│   └── test_utils.incn
└── incan.toml         # Project config (planned)
```

## CLI Reference

### Running Programs

```bash
# Compile and run
incan run myprogram.incn

# Build executable without running
incan build myprogram.incn

# Build to specific directory
incan build myprogram.incn output/
```

### Type Checking

```bash
# Type check without compiling
incan --check myprogram.incn
```

### Formatting

```bash
# Format files in place
incan fmt .

# Check formatting (CI mode)
incan fmt --check .

# Show diff without modifying
incan fmt --diff myfile.incn
```

### Debugging

```bash
# Show lexer output (tokens)
incan --lex myprogram.incn

# Show parser output (AST)
incan --parse myprogram.incn

# Show generated Rust code
incan --emit-rust myprogram.incn
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `INCAN_STDLIB` | Path to stdlib directory (auto-detected in most cases) |

## Next Steps

- [Formatting Guide](formatting.md) - Code style and `incan fmt`
- [Language Guide](../guide/) - Learn Incan syntax and features
- [Examples](../../examples/) - Sample programs
