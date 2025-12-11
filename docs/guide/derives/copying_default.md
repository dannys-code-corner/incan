# Copying and Default Derives

This page covers `Clone`, `Copy`, and `Default` for copying instances and creating defaults.

## Clone

**What it does**: Enables creating deep copies with `.clone()`.

**Python equivalent**: `__copy__`, `__deepcopy__`

**Rust generated**: `#[derive(Clone)]`

### Basic Clone Usage

```incan
@derive(Debug, Clone)
model Config:
    host: str
    port: int
    debug: bool

def main() -> None:
    original = Config(host="localhost", port=8080, debug=true)
    backup = original.clone()

    # backup is an independent copy
    println(f"{backup.host}")  # localhost
```

### Why Clone Matters

Without `Clone`, you can't duplicate values. This is especially important when you need to:

- Keep a backup before modifying
- Pass a copy to a function while retaining the original
- Store copies in multiple places

```incan
@derive(Debug, Clone)
model Settings:
    theme: str
    font_size: int

def save_and_modify(settings: Settings) -> Settings:
    backup = settings.clone()  # Keep original
    # ... modify settings ...
    return backup

def main() -> None:
    settings = Settings(theme="dark", font_size=14)
    original = save_and_modify(settings.clone())
    println(f"Original theme: {original.theme}")
```

### Automatic Default

Every `model` and `class` automatically gets `Clone`, so you rarely need to add it explicitly:

```incan
# These are equivalent:
model User:
    name: str

@derive(Clone)
model User:
    name: str
```

---

## Copy

**What it does**: Enables implicit copying (no `.clone()` needed).

**Python equivalent**: No direct equivalent (Python always references)

**Rust generated**: `#[derive(Copy, Clone)]`

### Basic Copy Usage

```incan
@derive(Debug, Copy)
model Point:
    x: int
    y: int

def main() -> None:
    p1 = Point(x=10, y=20)
    p2 = p1  # Implicit copy - p2 is independent

    # Both exist independently
    println(f"{p1.x}, {p2.x}")  # 10, 10
```

### Copy vs Clone

| Aspect | Clone | Copy |
|--------|-------|------|
| **Syntax** | `value.clone()` | Just `value` |
| **Explicit** | Yes - you see the copy | No - implicit |
| **Use for** | Any type | Simple value types only |

### Copy is a Marker Trait

**What's a marker trait?** A trait with no methods that just "marks" a type as having a property.

Copy doesn't add any methods to your type. Instead, it tells the compiler: "This type is safe to copy implicitly." The compiler then changes how assignment works for that type.

Other marker traits in programming:

- `Send` - "Safe to send to another thread"
- `Sync` - "Safe to share between threads"

### When to Use Copy

Only use `Copy` for **small, simple value types**:

- ✅ Wrapper types around primitives
- ✅ Small structs with only Copy fields
- ❌ Types with `String`, `Vec`, or heap data
- ❌ Large structs (copying is expensive)

```incan
# Good: small value type
@derive(Debug, Copy)
model Celsius:
    value: float

# Bad: contains String (not Copy-able in Rust)
# This would fail to compile:
# @derive(Debug, Copy)
# model User:
#     name: str  # str maps to String, which isn't Copy
```

### Copy Requires Clone

`Copy` always includes `Clone` automatically:

```incan
@derive(Debug, Copy)  # Clone is auto-included
model Coordinate:
    x: int
    y: int
```

---

## Default

**What it does**: Enables creating instances with default field values.

**Python equivalent**: `__init__` with default arguments

**Rust generated**: `#[derive(Default)]`

### Basic Default Usage

```incan
@derive(Debug, Default)
model Settings:
    theme: str = "dark"
    font_size: int = 14
    auto_save: bool = true

def main() -> None:
    # Create with all defaults
    settings = Settings()
    println(f"Theme: {settings.theme}")      # dark
    println(f"Font: {settings.font_size}")   # 14
    println(f"Auto-save: {settings.auto_save}")  # true
```

### Partial Defaults

You can override some defaults while keeping others:

```incan
@derive(Debug, Default)
model Config:
    host: str = "localhost"
    port: int = 8080
    timeout: int = 30

def main() -> None:
    # Override just the port
    config = Config(port=3000)
    println(f"{config.host}:{config.port}")  # localhost:3000
```

### Defaults for Different Types

| Type | Default Value |
|------|---------------|
| `int` | `0` |
| `float` | `0.0` |
| `bool` | `false` |
| `str` | `""` (empty string) |
| `List[T]` | `[]` (empty list) |
| `Dict[K, V]` | `{}` (empty dict) |
| `Option[T]` | `None` |

### When to Use Default

- **Configuration types** with sensible defaults
- **Builder patterns** where you set only what you need
- **Optional fields** that have reasonable fallbacks

```incan
@derive(Debug, Default)
model HttpRequest:
    method: str = "GET"
    path: str = "/"
    headers: Dict[str, str] = {}
    body: str = ""
    timeout_ms: int = 5000

def main() -> None:
    # Simple GET request with defaults
    request = HttpRequest(path="/api/users")
    println(f"{request.method} {request.path}")  # GET /api/users
```

---

## Quick Reference

| Derive | Method/Syntax | Use Case |
|--------|---------------|----------|
| `Clone` | `.clone()` | Create explicit deep copies |
| `Copy` | (implicit) | Small value types that copy automatically |
| `Default` | `Type()` | Create instances with default values |

## See Also

- [Derives Overview](./derives_and_traits.md) - Complete reference
