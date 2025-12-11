# Incan Documentation

## Tooling & Setup

How to install, configure, and use Incan tools.

| Document | Description |
|----------|-------------|
| [Getting Started](tooling/getting_started.md) | Installation and first steps |
| [Editor Setup](tooling/editor_setup.md) | IDE configuration and syntax highlighting |
| [Language Server](tooling/lsp.md) | LSP for diagnostics, hover, and go-to-definition |
| [Formatting](tooling/formatting.md) | Code formatter (`incan fmt`) |
| [Testing](tooling/testing.md) | Test runner (`incan test`) |

## Language Guide

How to write Incan code.

| Document | Description |
|----------|-------------|
| [Error Messages](guide/error_messages.md) | Understanding and fixing compiler errors |
| [Error Handling](guide/error_handling.md) | Result, Option, and the `?` operator |
| [File I/O](guide/file_io.md) | Reading, writing files and path handling |
| [Async Programming](guide/async_programming.md) | Async/await with Tokio |
| [Derives & Traits](guide/derives_and_traits.md) | Derive macros and trait system |
| [Imports & Modules](guide/imports_and_modules.md) | Module system, imports, and built-in functions |
| [Rust Interop](guide/rust_interop.md) | Using Rust crates directly from Incan |
| [Web Framework](guide/web_framework.md) | Building web apps with Axum |

### Derives Reference

| Document | Description |
|----------|-------------|
| [String Representation](guide/derives/string_representation.md) | Debug and Display |
| [Comparison](guide/derives/comparison.md) | Eq, Ord, Hash |
| [Copying & Default](guide/derives/copying_default.md) | Clone, Copy, Default |
| [Serialization](guide/derives/serialization.md) | Serialize, Deserialize |
| [Custom Behavior](guide/derives/custom_behavior.md) | Overriding derived behavior |

## RFCs (Request for Comments)

Design proposals for upcoming features:

| RFC | Status | Description |
|-----|--------|-------------|
| [000 Core RFC](RFCs/000_core_rfc.md) | Implemented | Core language design |
| [001 Test Fixtures](RFCs/001_test_fixtures.md) | Draft | pytest-style fixtures |
| [002 Test Parametrize](RFCs/002_test_parametrize.md) | Draft | Parametrized tests |
| [010 Tempfile](RFCs/010_tempfile.md) | Draft | Temporary files and directories |
