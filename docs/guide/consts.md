# Const bindings

If you’re coming from Python, Incan’s `const` is a way to say: **this name is a compile-time constant, not a regular variable**.
That gives the compiler more freedom to validate, optimize, and generate efficient Rust output.

## Why is `const` a thing?

In Python, “constants” are a convention (e.g. `MAX_RETRIES = 5`) and can still be reassigned at runtime.
In Incan, `const` is a language feature:

- **Intent**: communicates “this never changes” to readers and tools.
- **Safety**: prevents accidental mutation/reassignment (and enables deep immutability for collections via frozen types).
- **Performance**: allows the compiler to bake data into the output (Rust `const` / `'static` data).
- **Portability to Rust**: Rust distinguishes between runtime variables and compile-time constants; Incan maps cleanly to that.

## Notes

## How do I use it?

- **Syntax**: `const NAME [: Type] = <expr>`
- **Scope**: `const` is currently **module-level only**
- **Type annotations**: optional; if omitted, the compiler must be able to infer the type
- **Initializer rules**: the initializer must be **const-evaluable** (no runtime calls)

## How is `const` different from a regular variable?

- **`const`**:
  - evaluated at compile time
  - cannot depend on runtime values or non-const variables
  - intended to be immutable (and for collections: deeply immutable via frozen types)
- **regular variables (`let` / bindings)**:
  - computed at runtime
  - can call functions, use loops, read inputs, etc.
  - may be mutable (`mut`) depending on the binding

## Why are there “Rust-native” vs “Frozen” consts?

In Incan we support two constant “families” because Rust’s `const` rules are strict and because we want **deep immutability**:

- **Rust-native consts**:
  - map directly to a Rust `const`
  - best for numbers, booleans, and tuples of those
- **Frozen consts**:
  - for data that should be baked into the program but exposed through a **read-only API**
  - represented using frozen stdlib wrappers like `FrozenStr`, `FrozenList[T]`, `FrozenDict[K, V]`, `FrozenSet[T]`
  - the compiler emits baked `'static` backing data and constructs frozen wrappers from it

As a Python mental model: frozen types are closer to “an immutable view over a baked literal” than to `list`/`dict` objects you mutate at runtime.

## Examples

### Rust-native consts

```incan
const MAX_RETRIES: int = 5
const TIMEOUT_SECS: float = 2.5
const IS_DEBUG: bool = False
const PORT = 8080  # type may be inferred
const GREETING = "hello" + " world"  # string concatenation is allowed
```

### Frozen consts (deeply immutable)

```incan
const GREETING: FrozenStr = "hello"
const NUMS: FrozenList[int] = [1, 2, 3]
const HEADERS: FrozenDict[FrozenStr, FrozenStr] = {"User-Agent": "incan"}
const DATA: FrozenBytes = b"\x00\x01"
```

### Consts can reference other consts

```incan
const BASE: int = 10
const LIMIT: int = BASE * 2
```

## Errors

- If the initializer is not const-evaluable: when it uses runtime constructs (calls, comprehensions, f-strings, ranges, or non-const variables).
- If a const dependency cycle exists: when consts reference each other in a loop.
- If types do not match: when an explicit type annotation is incompatible with the initializer.
