# Incan Code Formatting

The `incan fmt` command formats Incan source code following a consistent style inspired by [Ruff](https://docs.astral.sh/ruff/) and [Black](https://black.readthedocs.io/).

## Quick Start

```bash
# Format a single file
incan fmt myfile.incn

# Format all .incn files in a directory
incan fmt src/

# Check if files need formatting (CI mode)
incan fmt --check .

# Show what would change without modifying
incan fmt --diff myfile.incn
```

## Style Guide

### Indentation

- **4 spaces** per indentation level
- No tabs

```incan
def calculate(x: int) -> int:
    if x > 0:
        return x * 2
    return 0
```

### Line Length

- **120 characters** maximum
- Long lines are wrapped after opening parentheses/brackets

### Blank Lines

- **2 blank lines** between top-level declarations (functions, classes, models, traits)
- **1 blank line** between methods within a class/model

```incan
def first_function() -> None:
    pass


def second_function() -> None:
    pass


model User:
    name: str
    age: int

    def greet(self) -> str:
        return f"Hello, {self.name}"

    def is_adult(self) -> bool:
        return self.age >= 18
```

### Spacing

- **Spaces around binary operators**: `a + b`, not `a+b`
- **No space after function name**: `foo(x)`, not `foo (x)`
- **Space after comma**: `foo(a, b)`, not `foo(a,b)`
- **Space after colon in type annotations**: `x: int`, not `x:int`
- **No space around `=` in named arguments**: `User(name="Alice")`, not `User(name = "Alice")`

### Strings

- **Double quotes** preferred for strings
- Single quotes preserved if already used

### Trailing Commas

- Added in multi-line constructs for cleaner diffs

### Docstrings

- Single-line docstrings on one line: `"""Brief description"""`
- Multi-line docstrings with content on separate lines:

```incan
"""
This is a longer docstring.

It can span multiple lines.
"""
```

## CLI Options

| Option | Description |
|--------|-------------|
| `incan fmt <path>` | Format file(s) in place |
| `--check` | Exit non-zero if files would be reformatted (useful for CI) |
| `--diff` | Show what would change without modifying files |

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success (no changes needed, or formatting complete) |
| 1 | Files need formatting (with `--check`) or errors occurred |

## CI Integration

Add to your CI pipeline to enforce consistent formatting:

```yaml
# GitHub Actions example
- name: Check formatting
  run: incan fmt --check .
```

## Configuration

Currently, formatting options use sensible defaults based on Ruff/Black conventions. Configuration file support (e.g., `incan.toml`) is planned for future releases.

Default settings:

- Indent: 4 spaces
- Line length: 120 characters
- Quote style: Double quotes
- Trailing commas: Yes (in multi-line)

## Limitations

The formatter can only process files that parse successfully. Files with syntax errors will be reported but not modified:

```bash
Error formatting myfile.incn: Parser error: [...]
```

Fix syntax errors before formatting.

## Next Steps

- [Language Guide](../guide/README.md) - Learn Incan syntax and features
- [Examples](../../examples/) - Sample programs
- [testing](testing.md) - Test runner and fixtures
