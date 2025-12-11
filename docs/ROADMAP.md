# Incan Roadmap (Status-Focused)

Purpose: Track implementation status and near-term planning (no timelines).

Incan development is driven by RFCs (Request for Comments).

- An RFC captures a design proposal for a feature, including syntax, semantics, and implementation details.
- RFCs are not necessarily implemented in the order they are written.

## Status Legend

- ✅ Done
- 🔄 In Progress
- 🟦 Planned
- ⏸️ Blocked/Deferred

## Core Phases (overview)

- Core language + runtime
- Stdlib + tooling (fmt, test, LSP, VS Code extensions)
- Web backend (Axum)
- Frontend/WASM (UI, JSX, 3D)
- Rust interop

## Current Focus

- 🔄 Frontend/WASM (RFC 003): JSX wrapper, signals/runtime, wasm codegen, dev/prod tooling

## Recently Completed

- ✅ Rust 2024 edition — enables `gen` blocks, async closures, improved RPIT lifetimes
- ✅ Testing: fixtures/parametrize (RFCs 001, 002, 004) — parser (`yield`), runner discovery, codegen infrastructure
- ✅ Rust interop (RFC 005): `rust::` imports, `use` codegen, auto Cargo.toml dependency injection

## Status by Area

- ✅ Core semantics/runtime (initial)
- 🔄 Stdlib (async/time/channels)
- ✅ Formatter (`incan fmt`)
- ✅ Test framework — yield expr, fixture discovery, scopes, autouse, parametrize
- 🔄 Async fixtures (RFC 004) — Tokio integration design
- ✅ Web backend (Axum) + codegen
- ✅ Web stdlib (App/route/Json/Response)
- ✅ VS Code extensions (full + lite)
- ✅ LSP server (initial)
- 🔄 Frontend/WASM UI + JSX — RFC done; parser/codegen pending
- 🟦 3D (wgpu) + assets
- 🟦 Dev server / HMR / bundler
- ✅ Rust interop — `import rust::`, `from rust::`, auto deps with version mapping
- 🟦 Generators (RFC 006) — Python-style `yield`, lazy iteration via Rust `gen` blocks
- 🟦 Inline tests (RFC 007) — `@test` functions in source files, stripped from production
- 🟦 Const bindings (RFC 008) — `const NAME [: Type] = <const-expr>` with compile-time checks

## Upcoming (next)

- WASM/JSX parser & codegen
- UI runtime (signals/effects/components) + examples
- Test runner fixture execution (setup/teardown lifecycle)
- Dev server + prod build pipeline for WASM target
- Python-style generators (RFC 006) — `yield` + `Iterator[T]` → Rust `gen` blocks
- Inline tests (RFC 007) — `@test` in source files, Rust-style proximity

## Deferred / Later

- SSR/SSG for frontend
- Desktop/mobile via wgpu
- CRDT/collab features

## Key RFCs / Docs

- RFC 000: Core language semantics
- RFC 001: Test fixtures
- RFC 002: Parametrize
- RFC 003: Frontend & WASM (UI/JSX/3D)
- RFC 004: Async fixtures (Tokio)
- RFC 005: Rust interop (`rust::crate` imports, auto deps)
- RFC 006: Python-style generators (`yield` → Rust `gen` blocks)
- RFC 007: Inline tests (`@test` in source files)
- RFC 008: Const bindings (compile-time constants)
- Web framework guide: `docs/guide/web_framework.md`
- Rust interop guide: `docs/guide/rust_interop.md`
- Testing guide: `docs/tooling/testing.md`
