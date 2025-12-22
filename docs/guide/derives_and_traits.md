# Derives and Traits in Incan

Incan uses a **derive system** that automatically generates common behaviors for your types. If you're familiar with
Python, think of *derives* as automatically implementing the "dunder methods" (`__eq__`, `__hash__`, `__str__`, etc.)
without writing any code. In Rust, you would write a full `impl` block to implement these traits.

## Incan vs Rust: Key Differences

While Incan compiles to Rust and uses Rust's derive system under the hood, it provides a **Python-friendly interface**:

| Aspect | Rust | Incan |
|--------|------|-------|
| **Syntax** | `#[derive(Debug, Clone)]` | `@derive(Debug, Clone)` |
| **Mental model** | "Implement traits" | "Add behaviors" (like Python dunders) |
| **Override mechanism** | Write a full `impl` block | Define `__eq__`, `__str__`, etc. methods |
| **Reflection** | Requires macros/crates | Built-in `__fields__()`, `__class_name__()` |

**The key insight**: In Rust, you think "I need to implement the `PartialEq` trait." In Incan, you think "I want my type
to support `==` like Python's `__eq__`." Same result, different mental model.

Incan gives you **Rust's performance** with **Python's ergonomics**.

---

## Complete Derive Reference

### String Representation

| Trait | What It Does | Python Equivalent |
|-------|--------------|-------------------|
| [`Debug`](./derives/string_representation.md#debug-automatic) | Debug-friendly string representation (auto) | `__repr__` |
| [`Display`](./derives/string_representation.md#display-custom-with-__str__) | User-friendly string representation | `__str__` |

### Comparison & Hashing

| Trait | What It Does | Python Equivalent |
|-------|--------------|-------------------|
| [`Eq`](./derives/comparison.md#eq) | Equality comparison (`==`, `!=`) | `__eq__`, `__ne__` |
| [`Ord`](./derives/comparison.md#ord) | Ordering (`<`, `<=`, `>`, `>=`) | `__lt__`, `__le__`, `__gt__`, `__ge__` |
| [`Hash`](./derives/comparison.md#hash) | Enable use as `Dict` key or in `Set` | `__hash__` |

### Copying & Defaults

| Trait | What It Does | Python Equivalent |
|-------|--------------|-------------------|
| [`Clone`](./derives/copying_default.md#clone) | Create deep copies with `.clone()` | `__copy__` |
| [`Copy`](./derives/copying_default.md#copy) | Implicit copying (marker trait) | — |
| [`Default`](./derives/copying_default.md#default) | Create with default field values | `__init__` defaults |

### Truthiness & Length

| Trait | What It Does | Python Equivalent |
|-------|--------------|-------------------|
| `Bool` | Truthiness testing (`if obj:`) — ⚠️ discouraged, prefer explicit checks | `__bool__` |
| `Len` | Length via `len(obj)` | `__len__` |

> **Note**: `Bool` cannot be auto-derived. If you need it, implement `__bool__` manually. Incan encourages explicit checks like `if x is Some(v):` or `if len(items) > 0:` over implicit truthiness.

### Iteration

| Trait | What It Does | Python Equivalent |
|-------|--------------|-------------------|
| `Iterable[T]` | Make type usable in `for` loops | `__iter__` |
| `Iterator[T]` | Produce next item in sequence | `__next__` |

### Type Conversions

| Trait | What It Does | Rust Equivalent |
|-------|--------------|-----------------|
| `From[T]` | Convert from another type | `std::convert::From` |
| `Into[T]` | Convert into another type | `std::convert::Into` |
| `TryFrom[T]` | Fallible conversion from | `std::convert::TryFrom` |
| `TryInto[T]` | Fallible conversion into | `std::convert::TryInto` |

### Arithmetic Operators

| Trait | What It Does | Python Equivalent |
|-------|--------------|-------------------|
| `Add[Rhs, Out]` | Addition (`a + b`) | `__add__` |
| `Sub[Rhs, Out]` | Subtraction (`a - b`) | `__sub__` |
| `Mul[Rhs, Out]` | Multiplication (`a * b`) | `__mul__` |
| `Div[Rhs, Out]` | Division (`a / b`) | `__truediv__` |
| `Neg[Out]` | Negation (`-a`) | `__neg__` |
| `Mod[Rhs, Out]` | Modulo (`a % b`) | `__mod__` |

### Serialization

| Trait | What It Does | Python Equivalent |
|-------|--------------|-------------------|
| [`Serialize`](./derives/serialization.md#serialize) | Convert to JSON | `json.dumps()` |
| [`Deserialize`](./derives/serialization.md#deserialize) | Parse from JSON | `json.loads()` |

### Indexing & Slicing

| Trait | What It Does | Python Equivalent |
|-------|--------------|-------------------|
| `Index[K, V]` | Read access: `obj[key]` | `__getitem__` |
| `IndexMut[K, V]` | Write access: `obj[key] = val` | `__setitem__` |
| `Sliceable[T]` | Slice access: `obj[1:3]` | `__getitem__` with slice |

### Membership

| Trait | What It Does | Python Equivalent |
|-------|--------------|-------------------|
| `Contains[T]` | Membership: `item in collection` | `__contains__` |

### Callable Objects

| Trait | What It Does | Python Equivalent |
|-------|--------------|-------------------|
| `Callable0[R]` | Call with no args: `obj()` | `__call__(self)` |
| `Callable1[A, R]` | Call with one arg: `obj(x)` | `__call__(self, x)` |
| `Callable2[A, B, R]` | Call with two args: `obj(x, y)` | `__call__(self, x, y)` |

### Error Handling

| Trait | What It Does | Python Equivalent |
|-------|--------------|-------------------|
| `Error` | Custom error types for `Result[T, E]` | `Exception` base class |

> **Note**: Incan uses Rust-style `Result[T, E]` error handling, not Python exceptions. See the [Error Handling Guide](./error_handling.md) for comprehensive documentation on `Result`, `Option`, the `?` operator, and the `Error` trait.

---

## Quick Start

```incan
@derive(Debug, Clone, Eq, Hash)
model User:
    name: str
    email: str
```

This single `@derive(...)` line gives your `User` type:

- **Debug** → printable for debugging (`{user:?}`)
- **Clone** → `.clone()` creates copies
- **Eq** → `==` and `!=` work
- **Hash** → can be used in `Set[User]` or as `Dict` key

---

## Automatic Defaults

Every `model` and `class` automatically gets `Debug` and `Clone`:

```incan
# These are equivalent:
model User:
    name: str

@derive(Debug, Clone)
model User:
    name: str
```

---

## Derive Dependencies

Some derives require others (Incan handles this automatically):

| If You Use | Incan Also Includes |
|------------|---------------------|
| `Hash` | `Eq` (required for correctness) |
| `Ord` | `Eq` (ordering requires equality) |
| `Copy` | `Clone` (Copy is a subset of Clone) |

> **Note**: `Copy` is a **marker trait** — it has no methods. It just tells the compiler "this type can be copied implicitly." See [Copying & Defaults → Marker Traits](./derives/copying_default.md#copy-is-a-marker-trait) for details.

---

## Compiler Errors

The compiler validates `@derive()` arguments and provides helpful errors:

### Unknown Derive

```incan
@derive(Debg)  # Typo!
model User:
    name: str
```

```bash
type error: Unknown derive 'Debg'
  --> example.incn:1:9
   |
 1 | @derive(Debg)
   |         ^^^^
= hint: Valid derives: Debug, Display, Eq, Ord, Hash, Clone, Copy, Default, Serialize, Deserialize
= hint: Use 'with TraitName' syntax for custom trait implementations
```

### Deriving a Model/Class (Not a Trait)

```incan
model User:
    name: str

@derive(User)  # Wrong! User is a model, not a trait
model Admin:
    email: str
```

```bash
type error: Cannot derive 'User' - it is a model, not a trait
  --> example.incn:4:9
   |
 4 | @derive(User)
   |         ^^^^
= hint: @derive() only works with traits like Debug, Eq, Clone
= hint: Did you mean: `with User` to implement a trait?
```

### Valid Derives Reference

| Derive | Category |
|--------|----------|
| `Debug`, `Display` | String representation |
| `Eq`, `Ord`, `Hash` | Comparison |
| `Clone`, `Copy`, `Default` | Copying |
| `Serialize`, `Deserialize` | JSON serialization |

---

## How Derives Work (Under the Hood)

When you Cmd+click on a trait like `Debug` or `Clone`, you'll see trait definitions in `stdlib/derives/`. These files serve two purposes:

1. **Documentation** — You can inspect what each derive provides
2. **Expansion templates** — Methods marked `@compiler_expand` are expanded by the compiler

### The `@compiler_expand` Decorator

```incan
trait Debug:
    @compiler_expand
    def __repr__(self) -> str: ...
```

The `@compiler_expand` decorator tells the compiler: "When a type derives this trait, generate a concrete implementation of this method."

**What happens at compile time:**

```incan
@derive(Debug)
model Point:
    x: int
    y: int
```

The compiler sees `@derive(Debug)`, finds the `Debug` trait definition, and expands the `@compiler_expand` methods into type-specific implementations. The generated code knows about `Point`'s fields (`x`, `y`) and produces appropriate Rust code.

**Why this matters:**

- **No runtime reflection** — field access is compiled directly, not looked up at runtime
- **Inspectable** — you can Cmd+click to see exactly what a derive provides
- **Type-safe** — the compiler validates everything at compile time

### Marker Traits vs Expanded Traits

| Type | Has `@compiler_expand` methods | Example |
|------|-------------------------------|---------|
| **Expanded traits** | ✅ Yes — compiler generates code | `Debug`, `Clone`, `Eq` |
| **Marker traits** | ❌ No — just marks the type | `Copy` |

---

## Custom Behavior

When auto-generated behavior doesn't fit your needs:

### Dunder Overrides

Override with Python-style `__method__` names:

```incan
@derive(Debug, Clone)
model User:
    id: int
    name: str
    cache_key: str  # Shouldn't affect equality

    def __eq__(self, other: User) -> bool:
        return self.id == other.id  # Only compare IDs
```

| Dunder | Overrides | Format | Purpose |
|--------|-----------|--------|---------|
| `__eq__` | `Eq` | — | Custom equality |
| `__hash__` | `Hash` | — | Custom hashing |
| `__lt__` | `Ord` | — | Custom ordering |
| `__str__` | `Display` | `{value}` | Custom string output |

> **Note**: `Debug` (`{value:?}`) cannot be overridden — it's always auto-generated.

[→ Full Dunder Documentation](./derives/custom_behavior.md#dunder-method-overrides)

### Traits with Defaults

Define reusable behaviors:

```incan
trait Describable:
    def describe(self) -> str:
        return "An object"

class Product with Describable:
    name: str
    price: float

# product.describe() returns "An object"
```

[→ Full Traits Documentation](./derives/custom_behavior.md#traits-with-default-implementations)

### Reflection

All types get automatic introspection:

```incan
user.__fields__()       # ["name", "email"]
user.__class_name__()   # "User"
```

[→ Full Reflection Documentation](./derives/custom_behavior.md#reflection-methods)

---

## Recommended Patterns

```incan
# Data transfer object (DTO)
@derive(Debug, Clone, Serialize, Deserialize)
model UserDTO:
    id: int
    name: str

# Value object (immutable, comparable)
@derive(Debug, Clone, Eq, Hash)
model Email:
    value: str

# Configuration with defaults
@derive(Debug, Clone, Default)
model Config:
    host: str = "localhost"
    port: int = 8080

# Full-featured entity
@derive(Debug, Clone, Eq, Ord, Hash, Serialize, Deserialize)
model User:
    id: int
    name: str
```

---

## Detailed Documentation

- **[String Representation](./derives/string_representation.md)** — `Debug`, `Display`
- **[Comparison & Hashing](./derives/comparison.md)** — `Eq`, `Ord`, `Hash`
- **[Copying & Defaults](./derives/copying_default.md)** — `Clone`, `Copy`, `Default`
- **[JSON Serialization](./derives/serialization.md)** — `Serialize`, `Deserialize`
- **[Custom Behavior](./derives/custom_behavior.md)** — Dunders, Traits, Reflection

---

## What We Intentionally Skip

Some Python/Rust features are **not** included in Incan's trait system:

| Feature | Why It's Skipped |
|---------|-----------------|
| `__enter__`/`__exit__` (Context Managers) | Rust's RAII model handles resource cleanup automatically. When a value goes out of scope, it's cleaned up. No need for explicit `with` statements. |
| `__del__` / `Drop` (Destructors) | Rust manages destruction automatically. Exposing this adds complexity without benefit for most users. |
| `Deref` / `DerefMut` | Smart pointer semantics are Rust-specific. Python developers don't need this concept. |
| `AsRef` / `AsMut` | Borrowing semantics - too Rust-specific for our Python-focused audience. |
| `__reversed__` | Use `.reverse()` method or `reversed()` builtin instead. |
| `__format__` | `Display` (`__str__`) covers 95% of use cases. |

### Resource Management Without Context Managers

Instead of Python's `with` statement, Incan uses Rust's automatic cleanup:

```incan
def process_file() -> Result[str, str]:
    # File is automatically closed when `file` goes out of scope
    file = File.open("data.txt")?
    content = file.read_all()?
    return Ok(content)
    # <- file is cleaned up here automatically
```

This is safer than Python's approach because you can't forget to close resources. For more on file operations, see the [File I/O Guide](./file_io.md).

---

## See Also

- [File I/O Guide](./file_io.md) — Reading, writing, and path handling
- [Error Handling Guide](./error_handling.md) — Working with Result types
- [Derives Reference](./derives/) — Detailed docs for each derive
- [Types Overview](./incan_types_overview.md) — All Incan types
- [Models vs Classes](./models_and_classes.md) — When to use each
- Examples:
  - `examples/advanced/derives_and_json.incn` — JSON serialization
  - `examples/advanced/dunder_and_traits.incn` — Dunder overrides
