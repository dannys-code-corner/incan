# Incan Compiler Architecture

This document describes the internal architecture of the Incan compiler.

## Compilation Pipeline

This diagram shows the compilation pipeline of the Incan compiler in high level.

```bash
┌─────────────────────────────────────────────────────────────────────────────┐
│                                FRONTEND                                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│     .incn source ──► Lexer ──► Parser ──► TypeChecker ──► AST (typed)       │
│                                                                             │
└────────────────────────────────────┬────────────────────────────────────────┘
                                     │
                                     ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                                 BACKEND                                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│   AST ──► AstLowering ──► IR ──► IrEmitter ──► TokenStream ──► Rust source  │
│                                     ▲                                       │
│                                     └──► prettyplease (formatting)          │
│                                                                             │
└────────────────────────────────────┬────────────────────────────────────────┘
                                     │
                                     ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           PROJECT GENERATION                                │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│       ProjectGenerator ──► Cargo.toml + src/*.rs ──► cargo build/run        │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Glossary

- **Frontend**: Parses `.incn` source and typechecks it, producing a typed AST (or diagnostics).
- **.incn source**: The source code of an Incan program.
- **Lexer**: Tokenizes source text into lexer tokens (used by the parser). I.e. it converts the source code into a stream of tokens that the parser can understand.
- **Parser**: Parses lexer tokens into an AST.
- **AST**: The abstract syntax tree (syntax structure + spans for diagnostics).
- **Typechecker**: Resolves names/imports and checks types, annotating the AST with type information.
- **Typed AST**: The AST after typechecking, with resolved types attached to relevant nodes.
- **Backend**: Generates Rust code from the typed AST.
- **Lowering**: Transforms typed AST → IR (including ownership/mutability/conversion decisions).
- **IR**: A Rust-oriented, ownership-aware intermediate representation used for code generation.
- **IrEmitter**: Emits IR into Rust code structures (TokenStream) before final formatting.
- **TokenStream**: A Rust `TokenStream` (from `proc_macro2`) produced by codegen (`quote`/`syn`) before being formatted into Rust source (not the same as the lexer tokens mentioned above).
- **prettyplease**: Formats Rust syntax/TokenStream output into human-readable Rust source text.
- **Rust source**: The generated Rust code as text.
- **ProjectGenerator**: Writes the generated Rust code into a standalone Cargo project (and can invoke `cargo build/run`).
- **Cargo project**: The generated Rust project directory (`Cargo.toml` + `src/*` + `target/*` after builds).
- **Cargo**: Rust’s build system and package manager.
- **CLI**: Command-line entrypoint that orchestrates compile/build/run/fmt/test workflows.
- **LSP**: IDE-facing server that runs compiler frontend stages and returns diagnostics/hover/definition via the Language Server Protocol.
- **Runtime crates**: `incan_stdlib` / `incan_derive` crates used by generated Rust programs (not the compiler).

## Walkthrough: `incan build`

This section describes what happens internally when you run `incan build path/to/main.incn`.

```bash
incan build file.incn
  │
  ├──▶ 1) Collect modules (imports)
  │       - Parse the entry file and any imported local modules
  │       - Produces: a list of parsed modules (source + AST) in dependency order
  │
  ├──▶ 2) Type check (with imports)
  │       - Name resolution + type checking across the module set
  │       - Produces: a typed AST (or structured diagnostics)
  │
  ├──▶ 3) Backend preparation
  │       - Scan for feature usage (e.g. serde / async / web / helpers)
  │       - Collect `rust::` crate imports for Cargo dependency injection
  │
  ├──▶ 4) Code generation
  │       - Lower typed AST → ownership-aware IR
  │       - Emit IR → Rust TokenStream → formatted Rust source
  │       - If imports are present: generate a nested Rust module tree
  │
  ├──▶ 5) Project generation
  │       - Write a standalone Cargo project (Cargo.toml + src/*.rs)
  │       - Default output dir: target/incan/<project_name>/
  │
  └──▶ 6) Build
          - `cargo build --release`
          - Binary path: target/incan/<project_name>/target/release/<project_name>
```

Notes:

- **Debugging individual stages**: Use CLI stage flags (`--lex`, `--parse`, `--check`, `--emit-rust`) to inspect intermediate outputs (see [Getting Started](tooling/getting_started.md)).
- **Multi-file projects**: Import resolution rules and module layout are described in [Imports & Modules](guide/imports_and_modules.md).
- **Rust interop dependencies**: `rust::` imports trigger Cargo dependency injection with a strict policy (see [Rust Interop](guide/rust_interop.md) and [RFC 013](RFCs/013_rust_crate_dependencies.md)).
- **Runtime boundary**: Generated programs depend on `incan_stdlib` and `incan_derive`, but the compiler does not (see `crates/`).

## Module Layout

```bash
Frontend (turns source text into a typed AST)
  ├──▶ Lexing + parsing
  │     - Converts `.incn` text into an AST
  │     - Attaches spans for precise diagnostics
  ├──▶ Name resolution + type checking
  │     - Builds symbol tables, resolves imports
  │     - Produces a typed AST (or structured errors)
  └──▶ Diagnostics (shared)
        - Pretty, source-context errors used by CLI / Formatter / LSP

Backend (turns typed AST into Rust code)
  ├──▶ Feature scanning
  │     - Detects language features used (serde/async/web/this/etc.)
  │     - Collects required Rust crates / routes / runtime needs
  ├──▶ IR + lowering
  │     - Lowers typed AST to a Rust-oriented, ownership-aware IR
  │     - Central place for ownership/mutability/conversion decisions
  └──▶ Emission + formatting
        - Emits Rust (TokenStream → formatted Rust source)
        - Applies consistent interop rules (borrows/clones/String conversions)

Project generation (turns Rust code into a runnable Cargo project)
  ├──▶ Planning (pure)
  │     - Compute dirs/files + chosen cargo action (build/run)
  ├──▶ Execution (side effects)
  │     - Writes files, creates dirs, shells out to cargo
  └──▶ Dependency policy
        - Controlled mapping for `rust::` imports (no silent wildcard deps)

CLI (user-facing orchestration)
  ├──▶ Compile actions
  │     - build/run: Frontend → Backend → Project generation → cargo
  ├──▶ Developer actions
  │     - lex/parse/check/emit-rust: inspect intermediate stages
  └──▶ Tool actions
        - fmt: format valid syntax
        - test: discover/run tests (pytest-style)

LSP (IDE-facing orchestration)
  ├──▶ Language server
  │     - Runs Frontend (and selected tooling) on edits
  └──▶ Protocol adapters
        - Converts compiler diagnostics into LSP diagnostics (and more over time)

Runtime crates (used by generated Rust programs, not the compiler)
  ├──▶ incan_stdlib
  │      - Traits + helpers (prelude, reflection, JSON helpers, etc.)
  └──▶ incan_derive
        - Proc-macro derives to generate impls for stdlib traits
```

### Frontend (`src/frontend/`)

| Module            | Purpose |
|-------------------|---------|
| `lexer/`          | Tokenization; keyword recognition via `phf` perfect hash |
| `parser.rs`       | Recursive descent; precedence climbing for expressions |
| `ast.rs`          | Untyped AST; `Spanned<T>` for error reporting |
| `module.rs`       | Module metadata and import path modeling |
| `resolver.rs`     | Import/module resolution across multi-file programs |
| `typechecker/`    | Two-pass symbol collection + type checking (see submodules below) |
| `symbols.rs`      | Symbol table; import resolution |
| `diagnostics.rs`  | Error types and pretty printing via `miette` |

#### Typechecker submodules (`typechecker/`)

| File              | Responsibility |
|-------------------|----------------|
| `mod.rs`          | `TypeChecker` struct + public API (`check_program`, `check_with_imports`) |
| `collect.rs`      | First pass: register types, functions, imports into symbol table |
| `check_decl.rs`   | Second pass: validate declarations (models, classes, traits, enums, functions) |
| `check_stmt.rs`   | Statement checking: assignments, returns, control flow |
| `check_expr/`     | Expression checking: dispatcher + themed helpers (calls, indexing, ops, match, etc.) |

### Backend (`src/backend/`)

| Module                | Purpose |
|-----------------------|---------|
| `ir/codegen.rs`       | **Main entry point** (`IrCodegen`); orchestrates lowering & emission |
| `ir/lower/`           | AST → IR lowering; resolves types and ownership semantics |
| `ir/emit/`            | IR → Rust via `syn`/`quote`; applies type conversions |
| `ir/emit_service/`    | Emission helpers split by IR layer (decl/stmt/expr/builtins) |
| `ir/conversions.rs`   | Centralized type conversions (`&str` → `String`, borrows, clones) |
| `ir/types.rs`         | IR type definitions (`IrType`) |
| `ir/expr.rs`          | IR expression definitions (`IrExpr`, `TypedExpr`, `BuiltinFn`, `MethodKind`) |
| `ir/stmt.rs`          | IR statement definitions (`IrStmt`) |
| `ir/decl.rs`          | IR declaration definitions (`IrDecl`) |
| `project.rs`          | Cargo project scaffolding and generation |

#### Expression Emission (`src/backend/ir/emit/expressions/`)

The expression emitter is split into focused submodules for maintainability:

| Submodule            | Purpose |
|----------------------|---------|
| `mod.rs`             | Main `emit_expr` entry point; dispatches to specialized handlers |
| `builtins.rs`        | Emit `BuiltinCall` variants (`print`, `len`, `range`, etc.) |
| `methods.rs`         | Emit `KnownMethodCall` variants and string-based method fallback |
| `calls.rs`           | Regular function calls and binary operations |
| `indexing.rs`        | Index, slice, and field access; centralized negative-index handling |
| `comprehensions.rs`  | List and dict comprehensions |
| `structs_enums.rs`   | Struct constructor expressions |
| `format.rs`          | Format strings (f-strings) and range expressions |
| `lvalue.rs`          | Assignment target expressions (left-hand side of `=`) |

**Enum-based dispatch**: Built-in functions and known methods use enum types (`BuiltinFn`, `MethodKind`) instead of string matching. This provides compile-time exhaustiveness checking and makes it easier to add new builtins/methods (see [Extending Incan](contributing/extending_language.md)).

### CLI (`src/cli/`)

| Module            | Purpose |
|-------------------|---------|
| `commands.rs`     | Command handlers: `build`, `run`, `fmt`, `test` |
| `test_runner.rs`  | pytest-style test discovery & execution |
| `prelude.rs`      | Stdlib loading (embedded `.incn` files) |

### Tooling

| Module    | Purpose |
|-----------|---------|
| `format/` | Source code formatter |
| `lsp/`    | LSP backend logic (diagnostics/hover/goto) |
| `bin/`    | Extra binaries (e.g. `lsp`) |

## Key Data Types

```text
AST (frontend)          IR (backend)              Rust (output)
─────────────────       ──────────────────        ─────────────────
ast::Program       ──►  IrProgram            ──►  TokenStream
ast::Declaration   ──►  IrDecl               ──►  (syn Items)
ast::Statement     ──►  IrStmt               ──►  (syn Stmts)
ast::Expr          ──►  TypedExpr            ──►  (syn Expr)
ast::Type          ──►  IrType               ──►  (syn Type)
```

## Ownership & Data Flow

1. **Frontend** parses and typechecks, producing an owned `ast::Program`
2. **Backend** borrows `&ast::Program`, produces owned `IrProgram`
3. **Emitter** borrows `&IrProgram`, produces owned `TokenStream`
4. **prettyplease** formats `TokenStream` to `String`
5. **ProjectGenerator** writes files to disk

## Entry Points

- **CLI**: `src/main.rs` → `cli::run()`
- **Codegen**: `IrCodegen::new()` → `.generate(&ast)`
- **LSP**: `src/bin/lsp.rs`

## Extending the Language

Incan’s compiler is intentionally staged (Lexer → Parser/AST → Typechecker → IR → Rust). This makes the system easier to reason about, but it also means “language changes” can touch multiple layers.

For contributor guidance on **when to add a builtin vs when to add new syntax**, plus end-to-end checklists, see:

- [Extending Incan: Builtins vs New Syntax](contributing/extending_language.md)
