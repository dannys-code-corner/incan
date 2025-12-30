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
| [Scopes & Name Resolution](guide/scopes_and_name_resolution.md) | Block scoping, shadowing, and how names are resolved |
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
| [001 Test Fixtures](RFCs/001_test_fixtures.md) | In Progress | pytest-style fixtures |
| [002 Test Parametrize](RFCs/002_test_parametrize.md) | Draft | Parametrized tests |
| [003 Frontend WASM](RFCs/003_frontend_wasm.md) | Future | Frontend & WebAssembly support |
| [004 Async Fixtures](RFCs/004_async_fixtures.md) | Draft | Async test fixtures |
| [005 Rust Interop](RFCs/005_rust_interop.md) | Draft | Rust crate imports and type mapping |
| [006 Generators](RFCs/006_generators.md) | Proposed | Python-style generators |
| [007 Inline Tests](RFCs/007_inline_tests.md) | Proposed | Tests in source files |
| [008 Const Bindings](RFCs/008_const_bindings.md) | Draft | Compile-time constants |
| [009 Sized Integers](RFCs/009_sized_integers.md) | Proposed | i8, i16, i32, u8, u16, etc. |
| [010 Tempfile](RFCs/010_tempfile.md) | Draft | Temporary files and directories |
| [011 F-String Spans](RFCs/011_fstring_error_spans.md) | Draft | Precise error spans in f-strings |
| [012 JsonValue](RFCs/012_json_value.md) | Draft | Dynamic JSON type |
| [013 Rust Crate Dependencies](RFCs/013_rust_crate_dependencies.md) | Draft | Version annotations, incan.toml, lock files |
| [014 Generated Code Errors](RFCs/014_generated_code_error_handling.md) | Draft | Better error messages in generated Rust |

## Compiler & Contributing

Docs for contributors working on the compiler and language evolution:

| Document | Description |
|----------|-------------|
| [Compiler Architecture](architecture.md) | Compilation pipeline, module layout, and internal stages |
| [Extending the Language](contributing/extending_language.md) | When to add builtins vs new syntax; end-to-end checklists |
| [Contributing Index](contributing/README.md) | Contributor documentation landing page |
