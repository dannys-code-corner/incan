# Incan Roadmap (Status-Focused)

Purpose: Track implementation status and near-term planning (no timelines).

Incan development is driven by RFCs (Request for Comments).

- An RFC captures a design proposal for a feature, including syntax, semantics, and implementation details.
- RFCs are not necessarily implemented in the order they are written.

## RFC status table

<!-- include all RFCs here -->
[RFC 000]: RFCs/000_core_rfc.md
[RFC 001]: RFCs/001_test_fixtures.md
[RFC 002]: RFCs/002_test_parametrize.md
[RFC 003]: RFCs/003_frontend_wasm.md
[RFC 004]: RFCs/004_async_fixtures.md
[RFC 005]: RFCs/005_rust_interop.md
[RFC 006]: RFCs/006_generators.md
[RFC 007]: RFCs/007_inline_tests.md
[RFC 008]: RFCs/008_const_bindings.md
[RFC 009]: RFCs/009_sized_integers.md
[RFC 010]: RFCs/010_tempfile.md
[RFC 011]: RFCs/011_fstring_error_spans.md
[RFC 012]: RFCs/012_json_value.md
[RFC 013]: RFCs/013_rust_crate_dependencies.md
[RFC 014]: RFCs/014_generated_code_error_handling.md
[RFC 015]: RFCs/015_hatch_like_tooling.md
[RFC 016]: RFCs/016_loop_and_break_value.md

| RFC       | RFC status     | Title                                                                       |
|----------:|----------------|-----------------------------------------------------------------------------|
| [RFC 000] | ✅ Done        | Incan Core Language RFC (Phase 1)                                           |
| [RFC 001] | 🔄 In Progress | Test Fixtures (yield expr, fixture discovery, scopes, autouse, parametrize) |
| [RFC 002] | ⏸️ Draft       | Parametrized Tests                                                          |
| [RFC 003] | ⏸️ Blocked     | Frontend & WebAssembly Support                                              |
| [RFC 004] | ⏸️ Draft       | Async Fixtures                                                              |
| [RFC 005] | ⏸️ Draft       | Rust Interop                                                                |
| [RFC 006] | 🟦 Planned     | Python-Style Generators                                                     |
| [RFC 007] | 🟦 Planned     | Inline Tests                                                                |
| [RFC 008] | ✅ Done        | Const Bindings (compile-time constants)                                     |
| [RFC 009] | 🟦 Planned     | Sized Integer Types & Builtin Type Registry                                 |
| [RFC 010] | ⏸️ Draft       | Temporary Files and Directories                                             |
| [RFC 011] | 🟦 Planned     | Precise Error Spans in F-Strings                                            |
| [RFC 012] | ⏸️ Draft       | JsonValue Type for Dynamic JSON                                             |
| [RFC 013] | ⏸️ Draft       | Rust Crate Dependencies                                                     |
| [RFC 014] | ⏸️ Draft       | Error Handling in Generated Rust Code                                       |
| [RFC 015] | 🟦 Planned     | Hatch-like Tooling (Project Lifecycle CLI)                                  |
| [RFC 016] | 🟦 Planned     | `loop` and `break <value>` (Loop Expressions)                               |

### Status Legend

- ✅ Done
- 🔄 In Progress
- 🟦 Planned
- ⏸️ Draft
- ⏸️ Blocked/Deferred

## Core Phases (overview)

- Core language + runtime
- Stdlib + tooling (fmt, test, LSP, VS Code extensions)
- Web backend (Axum)
- Frontend/WASM (UI, JSX, 3D)
- Rust interop

## Current Focus

- 🔄 LLanguage stability/feature freeze (core semantics + test surface):
  - [RFC 000] (core semantics  ✅ Done
  - [RFC 008] (const bindings) ✅ Done
  - “tests surface”:
    - ([RFC 001] (test fixtures) 🔄 In Progress,
    - [RFC 002] (parametrized tests) ⏸️ Draft,
    - [RFC 004] (async fixtures) ⏸️ Draft)
- 🔄 Frontend/WASM ([RFC 003]): JSX wrapper, signals/runtime, wasm codegen, dev/prod tooling

## Ecosystem keystones (planned)

These are the cross-cutting capabilities that make Incan feel “capable” for real engineering work. This list is
intentionally kept high-level and status-oriented (RFCs will be added over time).

- 🟦 Standard library contracts for real programs (HTTP, filesystem/paths, process, env, time, logging, config)
- 🟦 Capability-based access model for IO/process/env/network (secure-by-default for tools)
- 🟦 Interactive execution engine: `incan run -i` (expression-first) → notebook kernel interop → richer workspace UX
- 🟦 Packaging/distribution story for tools and projects (reproducible builds, artifact creation)

## Completed

- ✅ Incan initial setup ([RFC 000]) — core semantics, runtime, stdlib, tooling
- ✅ Rust 2024 edition — enables `gen` blocks, async closures, improved RPIT lifetimes
- ✅ Testing: fixtures/parametrize (RFCs 001, 002, 004) — parser (`yield`), runner discovery, codegen infrastructure
- ✅ Rust interop (RFC 005): `rust::` imports, `use` codegen, auto Cargo.toml dependency injection
- ✅ Const bindings (RFC 008) — `const NAME [: Type] = <const-expr>` with compile-time checks

## Status by Area

- ✅ Core semantics/runtime (initial)
- 🔄 Stdlib (async/time/channels)
- 🟦 Stdlib contracts (planned): HTTP, filesystem/paths, process, env, logging, config
- ✅ Formatter (`incan fmt`)
- ✅ Test framework — yield expr, fixture discovery, scopes, autouse, parametrize
- 🔄 Async fixtures (RFC 004) — Tokio integration design
- ✅ Web backend (Axum) + codegen
- ✅ Web stdlib (App/route/Json/Response)
- ✅ VS Code extensions (full + lite)
- ✅ LSP server (initial)
- 🟦 Interactive (planned): `incan run -i` REPL → notebook kernel interop
- 🟦 Packaging/distribution (planned): reproducible builds + artifact creation
- 🔄 Frontend/WASM UI + JSX — RFC done; parser/codegen pending
- 🟦 3D (wgpu) + assets
- 🟦 Dev server / HMR / bundler
- ✅ Rust interop — `import rust::`, `from rust::`, auto deps with version mapping
- 🟦 Generators (RFC 006) — Python-style `yield`, lazy iteration via Rust `gen` blocks
- 🟦 Inline tests (RFC 007) — `@test` functions in source files, stripped from production
- ✅ Const bindings (RFC 008) — `const NAME [: Type] = <const-expr>` with compile-time checks

## Upcoming (next)

- WASM/JSX parser & codegen
- UI runtime (signals/effects/components) + examples
- Test runner fixture execution (setup/teardown lifecycle)
- Dev server + prod build pipeline for WASM target
- Python-style generators (RFC 006) — `yield` + `Iterator[T]` → Rust `gen` blocks
- Inline tests (RFC 007) — `@test` in source files, Rust-style proximity

## Deferred / Later

The following items are intentionally deferred to later, and might be revisited in the future:

- SSR/SSG for frontend: Server-Side Rendering / Static Site Generation for the WASM/UI stack (render pages ahead of time
    or on the server, then hydrate).
- Desktop/mobile via wgpu: using the wgpu graphics stack to run Incan apps as native desktop/mobile apps (instead of
    browser-only).
- CRDT/collab features: real-time collaboration primitives (Conflict-free Replicated Data Types) for things like
    collaborative editing, shared state, etc.

### Guides

- Web framework guide: `docs/guide/web_framework.md`
- Rust interop guide: `docs/guide/rust_interop.md`
- Testing guide: `docs/tooling/testing.md`
