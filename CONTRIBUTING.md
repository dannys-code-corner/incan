# Contributing to Incan

Thank you for your interest in contributing to Incan! This document provides guidelines for contributing to the project.

## Getting Started

### Prerequisites

- **Rust 1.85+** (Incan uses Rust 2024 edition)
- **Cargo** (included with Rust)

```bash
# Install/update Rust
rustup update stable

# Verify version
rustc --version  # Should be 1.85.0 or higher
```

### Building

```bash
# Clone the repository
git clone https://github.com/dannys-code-corner/incan-programming-language.git
cd incan-programming-language

# Build the compiler
cargo build

# Run tests
cargo test

# Build in release mode
cargo build --release
```

### Running the Compiler

```bash
# Type check a file
cargo run -- --check examples/simple/hello.incn

# Build and run
cargo run -- run examples/simple/hello.incn

# See generated Rust code
cargo run -- --emit-rust examples/simple/hello.incn
```

## Development Workflow

### Code Style

- **Rust code**: Follow standard Rust conventions. Run `cargo fmt` before committing.
- **Incan code**: Use `incan fmt` for examples and stdlib.
- **Commits**: Use clear, descriptive commit messages.

```bash
# Format Rust code
cargo fmt

# Check for warnings/lints
cargo clippy
```

### Testing

All changes should include tests where applicable:

```bash
# Run all tests
cargo test

# Run a specific test
cargo test test_name

# Run tests with output
cargo test -- --nocapture
```

Tests live in two places:

- **Unit tests**: Inline in source files (`#[cfg(test)] mod tests`)
- **Integration tests**: In `tests/` directory

### Adding Examples

Examples go in `examples/` organized by complexity:

- `examples/simple/` - Basic syntax demos
- `examples/intermediate/` - Language features
- `examples/advanced/` - Complex patterns
- `examples/web/` - Web framework examples

## Making Changes

### For Bug Fixes and Small Changes

1. Fork the repository
2. Create a branch: `git checkout -b fix/description`
3. Make your changes
4. Run tests: `cargo test`
5. Format code: `cargo fmt`
6. Submit a pull request

### For New Features

Large/sprawling new features and enhancements require an RFC (Request for Comments). This give the community a chance to review and provide feedback before implementation begins.

To open an RFC:

1. Check existing [RFCs](docs/RFCs/) for related proposals
2. Open an issue to discuss the feature
3. If approved, create an RFC in `docs/RFCs/`:
   - Use the next available number (e.g., `009_feature_name.md`)
   - Follow the format of existing RFCs
4. Implement after RFC is accepted

### RFC Structure

```markdown
# RFC NNN: Feature Name

**Status:** Proposed | In Progress | Implemented
**Created:** YYYY-MM-DD

## Summary
One paragraph explanation.

## Motivation
Why is this needed?

## Design
How does it work? Include syntax examples.

## Implementation
How will it be built?

## Alternatives Considered
What else was considered and why was it rejected?
```

> Note: Github discussions are also a good place to discuss new features and enhancements. An RFC can follow a discussion if it is deemed necessary.

## Project Structure

```
src/
├── frontend/       # Lexer, parser, type checker
├── backend/        # Rust code generation
├── cli/            # Command-line interface
├── lsp/            # Language server
└── format/         # Code formatter

docs/
├── guide/          # User documentation
├── tooling/        # Tool documentation
└── RFCs/           # Design proposals

examples/           # Example Incan programs
stdlib/             # Standard library
tests/              # Integration tests
editors/            # Editor extensions (VS Code)
```

## Areas for Contribution

### Good First Issues

- Documentation improvements
- Additional examples
- Error message improvements
- Test coverage

### Larger Projects

- LSP features (completions, hover info)
- Formatter improvements
- New derives
- Standard library additions

Check the [Roadmap](docs/ROADMAP.md) for planned work.

## Code of Conduct

Be respectful and constructive. We're building something together.

## Questions?

- Open an issue for bugs or feature discussions
- Check existing issues and RFCs before creating new ones

## Attribution

Contributors are welcome to add themselves to the `authors` list in `Cargo.toml`:

```toml
authors = [
    ...,
    "Your Name <your.email@example.com>",  # Append your name and email at the end of the list
]
```

This is optional but encouraged for those who want recognition for their work.

## License

By contributing, you agree that your contributions will be licensed under the Apache 2.0 license.
