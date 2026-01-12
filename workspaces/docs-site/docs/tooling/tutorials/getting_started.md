# Getting Started with Incan

## Prerequisites

- Rust (1.85+): install via [rustup](https://rustup.rs/)
- `git`: to clone the repository
- `make`: for the canonical make-first workflow

These instructions assume a Unix-like shell environment (macOS/Linux). If you’re on Windows, use WSL:

- WSL install guide: `https://learn.microsoft.com/windows/wsl/install`

## Install/build/run (canonical)

Follow: [Install, build, and run](../how-to/install_and_run.md).

## Your First Program

Create a file `hello.incn`:

```incan
def main() -> None:
    println("Hello, Incan!")
```

Run it:

If you used `make install`:

```bash
incan run hello.incn
```

If you used the no-install fallback:

```bash
./target/release/incan run hello.incn
```

## Project Structure

A typical Incan project is just a folder of `.incn` files (and optionally `tests/`):

```text
my_project/
├── src/
│   ├── main.incn      # Entry point
│   └── utils.incn     # Additional modules
├── tests/
│   └── test_utils.incn
└── incan.toml         # Project config (planned)
```

Note: `incan.toml` is **planned** and not required to get started today.

## Next Steps

- [Formatting Guide](../how-to/formatting.md) - Code style and `incan fmt`
- [CLI Reference](../reference/cli_reference.md) - Commands, flags, and environment variables
- [Projects today](../explanation/projects_today.md) - Where builds go, what is regenerated, and what’s planned
- [Troubleshooting](../how-to/troubleshooting.md) - Common setup and “it didn’t work” fixes
- [Language: Start here](../../language/index.md) - Learn Incan syntax and patterns
- [Stability policy](../../stability.md) - Versioning expectations and “Since” semantics
- [Examples](https://github.com/dannys-code-corner/incan/tree/main/examples) - Sample programs
