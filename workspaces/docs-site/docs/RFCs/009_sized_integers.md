# RFC 009: Sized Integer Types & Builtin Type Registry

**Status:** Planned  
**Created:** 2024-12-11  
**Updated:** 2024-12-23

## Summary

Add explicit sized integer and float types to Incan, enabling precise control over numeric representation for FFI,
binary protocols, and memory-sensitive applications.

This RFC also introduces a **Builtin Type Registry** infrastructure (Phase 0) that allows `incan_core::lang`
registries to be the source of truth for builtin type methods, eliminating hardcoded method lookups scattered across
the compiler.

## Motivation

Currently, Incan has only two numeric types:

- `int` → `i64` (64-bit signed)
- `float` → `f64` (64-bit float)

This simplicity is great for general-purpose code, but insufficient for:

1. **FFI / Rust Interop** — Calling Rust functions that expect specific sizes
2. **Binary Protocols** — Parsing network packets, file formats (PNG, MP3, etc.)
3. **Memory Efficiency** — Large arrays where `i8` vs `i64` matters (8x difference)
4. **Hardware Interfaces** — GPIO pins, registers, embedded systems
5. **Cryptography** — Exact bit-width operations

### Example Problem

```incan
# Current: Can't express this cleanly
from rust::std::net import TcpStream

# TcpStream::connect expects &str, but port is u16 internally
# How do we work with u16 port numbers?
```

## Design

### New Types

| Incan Type | Rust Type | Description                    |
| ---------- | --------- | ------------------------------ |
| `i8`       | `i8`      | 8-bit signed (-128 to 127)     |
| `i16`      | `i16`     | 16-bit signed                  |
| `i32`      | `i32`     | 32-bit signed                  |
| `i64`      | `i64`     | 64-bit signed (same as `int`)  |
| `i128`     | `i128`    | 128-bit signed                 |
| `u8`       | `u8`      | 8-bit unsigned (0 to 255)      |
| `u16`      | `u16`     | 16-bit unsigned                |
| `u32`      | `u32`     | 32-bit unsigned                |
| `u64`      | `u64`     | 64-bit unsigned                |
| `u128`     | `u128`    | 128-bit unsigned               |
| `f32`      | `f32`     | 32-bit float                   |
| `f64`      | `f64`     | 64-bit float (same as `float`) |
| `isize`    | `isize`   | Pointer-sized signed           |
| `usize`    | `usize`   | Pointer-sized unsigned         |

### Type Aliases (Preserved)

The existing `int` and `float` remain as aliases:

```incan
# These are equivalent
x: int    # Preferred for general code
x: i64    # Explicit when size matters

y: float  # Preferred for general code  
y: f64    # Explicit when size matters
```

### Literals

#### Without Suffix (Default)

Unsuffixed literals infer type from context, defaulting to `int`/`float`:

```incan
let x = 42        # int (i64)
let y = 3.14      # float (f64)
let z: i32 = 42   # i32 (from annotation)
```

#### With Suffix (Explicit)

```incan
let a = 42i8      # i8
let b = 42u16     # u16
let c = 42i32     # i32
let d = 42u64     # u64
let e = 3.14f32   # f32
let f = 255u8     # u8
```

### Conversions

#### Explicit Casting Required

No implicit narrowing — all conversions are explicit:

```incan
let big: i64 = 1000
let small: i16 = big as i16        # Explicit cast (may truncate)
let safe: i16 = i16.try_from(big)? # Safe conversion (returns Result)
```

#### Conversion Functions

```incan
# Fallible conversions (return Result)
let x: Result[i16, ConversionError] = i16.try_from(big_value)

# Infallible widening (always safe)
let wide: i64 = i64.from(small_value)  # i16 -> i64 always works

# Truncating cast (use with caution)
let truncated: i8 = big_value as i8    # May lose data
```

### Arithmetic

Same-type operations return the same type:

```incan
let a: i32 = 10
let b: i32 = 20
let c: i32 = a + b  # i32

# Mixed types require explicit conversion
let x: i32 = 10
let y: i64 = 20
let z = x as i64 + y  # Must convert first
```

### Overflow Behavior

Follow Rust's behavior:

- **Debug builds**: Panic on overflow
- **Release builds**: Wrap (two's complement)

Explicit wrapping/saturating operations available:

```incan
let x: u8 = 255
let y = x.wrapping_add(1)    # 0 (wrapped)
let z = x.saturating_add(1)  # 255 (saturated)
let w = x.checked_add(1)     # None (overflow detected)
```

## Examples

### Binary Protocol Parsing

```incan
def parse_header(data: bytes) -> Result[Header, ParseError]:
    if len(data) < 8:
        return Err(ParseError.TooShort)
    
    let magic: u32 = u32.from_le_bytes(data[0..4])
    let version: u16 = u16.from_le_bytes(data[4..6])
    let flags: u8 = data[6]
    let reserved: u8 = data[7]
    
    return Ok(Header(magic, version, flags))
```

### Network Port Handling

```incan
def connect(host: str, port: u16) -> Result[Connection, IoError]:
    let addr = f"{host}:{port}"
    return Connection.open(addr)

# Usage
let conn = connect("localhost", 8080u16)?
```

### Memory-Efficient Arrays

```incan
# Image pixel data - 3 bytes per pixel instead of 24
model Pixel:
    r: u8
    g: u8
    b: u8

let image: List[Pixel] = load_image("photo.png")
```

### Bit Manipulation

```incan
def set_bit(value: u32, bit: u8) -> u32:
    return value | (1u32 << bit)

def clear_bit(value: u32, bit: u8) -> u32:
    return value & ~(1u32 << bit)

def test_bit(value: u32, bit: u8) -> bool:
    return (value & (1u32 << bit)) != 0
```

## Implementation Plan

### Phase 0: Builtin Type Registry

Before adding new types, establish **registry-first** infrastructure so the compiler has a single source of truth for:

- builtin type spellings (e.g. `i32`, `u8`)
- builtin/surface method vocabulary (e.g. `str.upper()`, `List.append()`)
- metadata like “introduced in RFC”, stability, and docs

**Current crate layout note:** in the current workspace, canonical language vocabulary lives in:

- `crates/incan_core/src/lang/types/*` for builtin type names
- `crates/incan_core/src/lang/surface/*` for method/function/type “surface” vocabulary
- `crates/incan_syntax` for lexer/token/parsing (syntax), consuming registries where appropriate

So Phase 0 should be implemented by extending these registries and migrating compiler call sites to consult them,
rather than parsing `incan_stdlib` source code.

#### Problem

Currently, builtin type methods (e.g., `str.upper()`, `List.append()`, `FrozenStr.len()`) are hardcoded in multiple places:

- `src/frontend/typechecker/check_expr/access.rs` — method return types
- `src/backend/ir/emit/mod.rs` — code generation
- `src/backend/ir/emit/expressions/methods.rs` — method emission

This creates triple maintenance burden and increases drift risk between docs/spec/compiler behavior.

#### Solution

1. **Define method vocab in `incan_core`** (source of truth):

    - `crates/incan_core/src/lang/surface/methods.rs` (surface method vocab registries; re-exported as
      `incan_core::lang::surface::{string_methods, list_methods, dict_methods, ...}` for stable import paths)
    - For sized integer methods, add a new registry section within `methods.rs` (or a sibling module re-exported from
      `crates/incan_core/src/lang/surface/mod.rs` if it grows too large)

2. **Compiler consumes registries** instead of string matches:

    - Typechecker consults the registries to determine allowed methods and return types.
    - Backend/codegen consults the same registries (or enum/ID mapping derived from them) to emit method calls.

3. **Guardrails**

    - Add tests that ensure registry uniqueness and prevent drift with lexer/parser expectations (parity checks).

#### Benefits

- **Single source of truth**: `incan_core` defines the language surface vocabulary.
- **No hardcoding**: typechecker and emitter read from registries.
- **Extensibility**: adding a method is a single edit in `incan_core::lang::surface`.
- **Documentation**: reference docs can be generated from registries.

#### Scope

Phase 0 covers:

- Registry infrastructure (IDs + metadata tables + docs generation + guardrails)
- Migration of existing hardcoded types:
    - `str`, `bytes`
    - `List`, `Dict`, `Set`
    - `FrozenStr`, `FrozenBytes`
    - `FrozenList`, `FrozenSet`, `FrozenDict`
- Foundation for sized integer methods in Phase 5

### Phase 1: Lexer

Add token recognition for:

- Type names: `i8`, `i16`, `i32`, `i64`, `i128`, `u8`, `u16`, `u32`, `u64`, `u128`, `f32`, `isize`, `usize`
- Literal suffixes: `42i32`, `255u8`, `3.14f32`

### Phase 2: Parser

- Parse sized type annotations
- Parse suffixed numeric literals

### Phase 3: Type Checker

- Add sized integer types to type system
- Enforce explicit conversions (no implicit narrowing)
- Type inference for unsuffixed literals
- Use builtin registry (from Phase 0) for method lookups

### Phase 4: Codegen

- Map types directly to Rust equivalents
- Emit proper literal suffixes
- Generate conversion code
- Use builtin registry for method emission

### Phase 5: Standard Library

- Add conversion methods (`try_from`, `from`, `as`) to registry
- Add byte conversion methods (`from_le_bytes`, `to_be_bytes`, etc.) to registry
- Add overflow-handling methods (`wrapping_add`, `saturating_sub`, etc.) to registry
- Implement corresponding Rust methods in `incan_stdlib`

## Alternatives Considered

### 1. Only Add When Needed via Rust Interop

```incan
from rust::std::primitive import i16, u8
```

**Rejected**: Awkward syntax, doesn't integrate well with literals.

### 2. Python-Style Arbitrary Precision

Python's `int` is arbitrary precision. We could do the same.

**Rejected**: Doesn't help with FFI, binary protocols, or memory efficiency. Also has performance overhead.

### 3. Wrapper Types Only

```incan
newtype Port = u16  # Define in stdlib
```

**Rejected**: Still need the underlying sized types.

### 4. C-Style Type Names

```incan
let x: short = 10      # i16
let y: unsigned int = 20  # u32
```

**Rejected**: Verbose, platform-dependent sizes in C, Rust-style is clearer.

## Open Questions

1. **Literal inference**: Should `let x: u8 = 256` be a compile error or runtime panic?
   - Proposal: Compile error for out-of-range literals

2. **Default integer type**: Keep `int` as `i64` or make it platform-dependent like `isize`?
   - Proposal: Keep as `i64` for predictability

3. **Char type**: Add `char` as a separate type (Rust's 4-byte Unicode scalar)?
   - Proposal: Defer to separate RFC

4. **SIMD types**: Should we include `i8x16`, `f32x4` etc?
   - Proposal: Defer to separate RFC for SIMD

5. **List/array indexing**: How should `int` work with indexing?
   - Problem: `arr[i]` where `i: int` fails in Rust (needs `usize`)
   - Option A: Auto-coerce `int` → `usize` in indexing context (ergonomic, implicit)
   - Option B: Require explicit cast `arr[i as usize]` (explicit, verbose)
   - Option C: Make `range()` return `usize` (natural for loops, but breaks if used elsewhere)
   - Proposal: Option A — compiler inserts `as usize` for indexing; matches Python's ergonomics

## Checklist

### Phase 0: Builtin Type Registry (checklist)

- [ ] Extend `incan_core::lang::types` with sized type vocabulary (`i8`, `u16`, `f32`, `usize`, ...)
- [ ] Extend `incan_core::lang::surface` with sized integer method vocabulary (new registry module)
- [ ] Typechecker: consult `incan_core::lang::surface` registries for method typing
- [ ] Backend: consult `incan_core::lang::surface` registries (or enum mapping) for method emission
- [ ] Migrate `str` methods from hardcoded to registry
- [ ] Migrate `bytes` methods from hardcoded to registry
- [ ] Migrate `List` methods from hardcoded to registry
- [ ] Migrate `Dict` methods from hardcoded to registry
- [ ] Migrate `Set` methods from hardcoded to registry
- [ ] Migrate `FrozenStr` methods from hardcoded to registry
- [ ] Migrate `FrozenBytes` methods from hardcoded to registry
- [ ] Migrate `FrozenList`/`FrozenSet`/`FrozenDict` methods from hardcoded to registry
- [ ] Update emitter to use registry for method emission
- [ ] Remove hardcoded method matches from `check_expr/access.rs`
- [ ] Remove hardcoded method matches from `emit/mod.rs`

### Phase 1-4: Sized Types

- [ ] Lexer: recognize sized type names
- [ ] Lexer: parse literal suffixes (`42i32`, `3.14f32`)
- [ ] Parser: sized type annotations
- [ ] Type checker: sized integer types in type system
- [ ] Type checker: enforce explicit conversions
- [ ] Type checker: literal range checking
- [ ] Codegen: emit proper Rust types
- [ ] Codegen: emit literal suffixes
- [ ] Codegen: auto-coerce `int` → `usize` for list indexing

### Phase 5: Standard Library Methods

- [ ] Stdlib: add sized type interface traits (`I8Methods`, `U16Methods`, etc.)
- [ ] Stdlib: conversion methods (`try_from`, `from`)
- [ ] Stdlib: byte conversion methods (`from_le_bytes`, `to_be_bytes`, etc.)
- [ ] Stdlib: overflow-handling methods (`wrapping_add`, `saturating_sub`, etc.)

### Documentation

- [ ] Documentation update
- [ ] Examples

## References

- [Rust Primitive Types](https://doc.rust-lang.org/std/primitive/index.html)
- [Rust Integer Overflow](https://doc.rust-lang.org/book/ch03-02-data-types.html#integer-overflow)
- [Python struct module](https://docs.python.org/3/library/struct.html) (for comparison)
