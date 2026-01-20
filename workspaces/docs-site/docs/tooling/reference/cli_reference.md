# CLI reference

This is the authoritative CLI reference for `incan` (commands, flags, paths, and environment variables).

--8<-- "_snippets/callouts/no_install_fallback.md"

## Usage

Top-level usage:

```text
incan [OPTIONS] [FILE] [COMMAND]
```

- If you pass a `FILE` without a subcommand, `incan` type-checks it (default action).

Commands:

- `build` - Compile to Rust and build an executable
- `run` - Compile and run a program
- `fmt` - Format Incan source files
- `test` - Run tests (pytest-style)

## Global options

- `--no-banner`: suppress the ASCII logo banner (also via `INCAN_NO_BANNER=1`).
- `--color=auto|always|never`: control ANSI color output (respects `NO_COLOR`).

## Global options (debug)

These flags take a file and run a debug pipeline stage:

```bash
incan --lex path/to/file.incn
incan --parse path/to/file.incn
incan --check path/to/file.incn
incan --emit-rust path/to/file.incn
```

Strict mode:

```bash
incan --strict --emit-rust path/to/file.incn
```

## Commands

### `incan build`

Usage:

```text
incan build <FILE> [OUTPUT_DIR]
```

Behavior:

- Prints the generated Rust project path (example): `target/incan/<name>/`
- Builds the generated Rust project and prints the binary path (example):
  `target/incan/<name>/target/release/<name>`

Example:

```bash
incan build examples/simple/hello.incn
```

### `incan run`

Usage:

```text
incan run [OPTIONS] [FILE]
```

Run a file:

```bash
incan run path/to/file.incn
```

Run inline code:

```bash
incan run -c "import this"
```

### `incan fmt`

Usage:

```text
incan fmt [OPTIONS] [PATH]
```

Examples:

```bash
# Format files in place
incan fmt .

# Check formatting without modifying (CI mode)
incan fmt --check .

# Show what would change without modifying files
incan fmt --diff path/to/file.incn
```

### `incan test`

Usage:

```text
incan test [OPTIONS] [PATH]
```

Examples:

```bash
# Run all tests in a directory
incan test tests/

# Run all tests under a path (default: .)
incan test .

# Filter tests by keyword expression
incan test -k "addition"

# Verbose output (include timing)
incan test -v

# Stop on first failure
incan test -x

# Include slow tests
incan test --slow

# Fail if no tests are collected
incan test --fail-on-empty
```

## Outputs and paths

Build outputs:

- **Generated Rust project**: `target/incan/<name>/`
- **Built binary**: `target/incan/<name>/target/release/<name>`

Cleaning:

```bash
rm -rf target/incan/
```

## Environment variables

- **`INCAN_STDLIB`**: override the stdlib directory (usually auto-detected; set only if detection fails).
- **`INCAN_FANCY_ERRORS`**: enable “fancy” diagnostics rendering (presence-based; output may change).
- **`INCAN_EMIT_SERVICE=1`**: toggle codegen emit mode (internal/debug; not stable).
- **`INCAN_NO_BANNER=1`**: disable the ASCII logo banner.
- **`NO_COLOR`**: disable ANSI color output (standard convention).

## Exit codes

General rule: success is exit code 0; errors are non-zero.

Specific behavior:

- **`incan run`**: returns the program’s exit code.
- **`incan test`**:
    - returns 0 if all tests pass
    - returns 0 if test files exist but no tests are collected
    - returns 1 if `--fail-on-empty` is set and no tests are collected
    - returns 1 if no test files are discovered under the provided path
    - returns 1 if any tests fail or an xfail unexpectedly passes (XPASS)
- **`incan fmt --check`**: returns 1 if any files would be reformatted.
- **`incan build` / `incan --check` / debug flags**: return 1 on compile/build errors.

## Drift prevention (maintainers)

Before a release, verify the docs stay aligned with the real CLI surface:

- Compare `incan --help` and `incan {build,run,fmt,test} --help` against this page.
