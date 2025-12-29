# RFC 005: Rust Interop

**Status:** Draft  
**Created:** 2025-12-10  

## Summary

Define seamless Rust interop for Incan: imports like `rust::serde_json` should work without manual Cargo edits, with clear type mapping, codegen, and build integration.

## Goals

- Allow importing Rust crates/modules from Incan (`rust::serde_json`, `rust::time::Instant`).
- Auto-manage Cargo dependencies/features based on imports.
- Generate correct Rust `use` statements with stable namespacing.
- Provide a well-defined type mapping and safety model.
- Avoid surprises around async runtimes and unwind behavior.

## Non-Goals (initial)

- Procedural macros or derive expansion.
- Arbitrary trait/lifetime-heavy APIs.
- Cross-crate macro resolution.

## Syntax

- Whole-crate import:

  ```incan
  import rust::serde_json as json
  ```

- Selective import:

  ```incan
  from rust::time import Instant, Duration
  ```

- Nested paths:

  ```incan
  from rust::chrono::naive::date import NaiveDate
  ```

### Related: `crate::...` absolute module paths

This RFC also introduces the **Rust-style** `crate` import prefix for **Incan module paths**:

```incan
# Import from the project root (absolute module path)
import crate::config as cfg
from crate::utils import format_date
```

Notes:
- `crate::...` is for **Incan modules** (project root), not for selecting a Rust crate in a `rust::...` import.
- Parent navigation for Incan modules uses `super::...` / `..` and is specified in RFC 000.

## Type Mapping (initial)

- Scalars: `int` ↔ `i64`/`i32`, `float` ↔ `f64`, `bool` ↔ `bool`.
- Strings: `str` ↔ `String`.
- Collections: `List[T]` ↔ `Vec<T>`, `Dict[K,V]` ↔ `HashMap<K,V>`.
- Option/Result: `Option[T]` ↔ `Option<T>`, `Result[T,E]` ↔ `Result<T,E>`.
- Structs/enums: direct mapping when imported; no automatic derive of complex traits.
- Borrowed data: restricted initially—prefer owned types; no implicit lifetimes.

## Codegen

- Emit Rust `use` statements matching imported paths.
- Namespace collision avoidance: preserve module paths; mangle only if needed.
- Error surfacing: map Rust errors to Incan `Result`; no hidden panics.
- Async: if imported functions are async and use Tokio, ensure runtime compatibility.

## Cargo Integration

- Auto-add `[dependencies]` entries when `rust::` imports are used.
- Allow specifying versions/features in an optional manifest override (future).
- Default: latest compatible semver or pinned minimal versions (TBD).
- Feature flags: if `rust::tokio::net` is imported, enable `tokio` with required features.

## Safety & Unwind

- Assume Rust panics should not cross the FFI boundary; prefer Result-returning APIs.
- For now, avoid APIs that require borrowing lifetimes across the boundary.
- Document any `Send`/`Sync` expectations for async interop.

## Async Interop

- Prefer Tokio-backed crates for async.
- Ensure a single Tokio runtime in the test runner/CLI; avoid nested runtimes.
- If a crate requires a current-thread runtime, note incompatibility or provide a flag (future).

## Build Modes

- Generated Cargo: Incan emits Cargo.toml and src with the needed deps/uses.
- User-managed Cargo (future): allow opting out and integrating into existing workspaces.

## Examples

```incan
import rust::serde_json as json

def encode_user(user: User) -> str:
    return json::to_string(&user)?
```

```incan
from rust::time import Instant, Duration

def elapsed_ms(start: Instant) -> int:
    return Duration::from_secs_f64(start.elapsed().as_secs_f64()).as_millis()
```

```incan
from rust::reqwest import Client

async def fetch_text(url: str) -> Result[str, HttpError]:
    client = Client::new()
    resp = await client.get(url).send()?
    return await resp.text()
```

## Limitations (initial)

- No proc-macros/derive.
- No implicit lifetime bridging; borrowed references not supported across the boundary.
- Trait-heavy or GAT/lifetime-heavy APIs may not map cleanly.

## Open Questions

1. Version resolution: pin minimal versions or allow user overrides?
2. Feature auto-detection granularity: per-module vs per-crate?
3. Error mapping: when to wrap vs. propagate Rust error types directly?
4. WASM target: which crates are allowed/blocked when targeting wasm32?

## Checklist

- [ ] Syntax parsing for `rust::` imports
- [ ] Codegen: emit `use` statements
- [ ] Cargo: auto-add dependencies/features
- [ ] Type mapping rules enforced in codegen/typechecker
- [ ] Async interop: runtime compatibility
- [ ] Error handling: map to Result, avoid cross-boundary panics
- [ ] WASM constraints documented
- [ ] Examples and docs added
