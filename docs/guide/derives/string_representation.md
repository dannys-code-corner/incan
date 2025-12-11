# String Representation Derives

This page covers how your types are converted to strings in Incan.

## Quick Summary

| Format | Derive | Dunder Override | Purpose |
|--------|--------|-----------------|---------|
| `{value:?}` | `Debug` (auto) | ❌ Not supported | Debugging |
| `{value}` | `Display` | ✅ `__str__` | User-facing |

---

## Debug (Automatic)

**What it does**: Generates a debug-friendly string representation.

**Format syntax**: `{value:?}` (note the `:?`)

**Rust generated**: `#[derive(Debug)]`

### Key Point: Debug is Always Automatic

Every `model` and `class` automatically gets `Debug`. You never need to add it:

```incan
# Debug is auto-included - no @derive needed!
model Point:
    x: int
    y: int

def main() -> None:
    p = Point(x=10, y=20)
    println(f"Debug: {p:?}")  # Point { x: 10, y: 20 }
```

### Debug Output Format

Debug always shows the type name and all fields in a structured format:

```
TypeName { field1: value1, field2: value2 }
```

For nested types:

```incan
model Address:
    city: str
    zip: str

model Person:
    name: str
    address: Address

def main() -> None:
    addr = Address(city="NYC", zip="10001")
    person = Person(name="Alice", address=addr)
    println(f"{person:?}")
    # Person { name: "Alice", address: Address { city: "NYC", zip: "10001" } }
```

### Can You Override Debug?

**No.** Currently Incan does not support overriding the Debug output. The `{:?}` format always uses the auto-generated representation.

If you need custom formatting, use Display with `__str__` (see below).

---

## Display (Custom with `__str__`)

**What it does**: User-friendly string representation that YOU define.

**Format syntax**: `{value}` (no `:?`)

**How to customize**: Define a `__str__` method

### Usage

```incan
model User:
    name: str
    email: str

    def __str__(self) -> str:
        return f"{self.name} <{self.email}>"

def main() -> None:
    user = User(name="Alice", email="alice@example.com")
    println(f"{user}")      # Alice <alice@example.com>  ← uses __str__
    println(f"{user:?}")    # User { name: "Alice", email: "alice@example.com" }  ← auto Debug
```

### Without `__str__`

If you don't define `__str__`, using `{value}` may not compile or will fall back to Debug:

```incan
model Point:
    x: int
    y: int
    # No __str__ defined

def main() -> None:
    p = Point(x=10, y=20)
    # println(f"{p}")   # May not work - no Display impl
    println(f"{p:?}")   # Works - Debug is auto-derived
```

---

## Debug vs Display

| Aspect | Debug (`{:?}`) | Display (`{}`) |
|--------|----------------|----------------|
| **Purpose** | Debugging/development | User-facing output |
| **Format** | Shows all internal details | Clean, readable output |
| **Automatic** | ✅ Always auto-derived | ❌ Needs `__str__` |
| **Customizable** | ❌ No | ✅ Yes, via `__str__` |
| **Example output** | `User { name: "Alice", age: 30 }` | `Alice (30 years old)` |

### When to Use Each

```incan
model Temperature:
    celsius: float

    def __str__(self) -> str:
        return f"{self.celsius}°C"

def main() -> None:
    temp = Temperature(celsius=23.5)
    
    # For users - clean output
    println(f"Current temperature: {temp}")  # Current temperature: 23.5°C
    
    # For debugging - shows structure
    println(f"Debug: {temp:?}")  # Debug: Temperature { celsius: 23.5 }
```

### Recommendation

- **Always use `{:?}`** for logging and error messages (shows full structure)
- **Define `__str__`** when you need user-friendly output
- **Don't rely on `{}`** without defining `__str__`

---

## Common Patterns

### Money/Currency

```incan
model Money:
    cents: int
    currency: str

    def __str__(self) -> str:
        dollars = self.cents / 100
        return f"{self.currency}{dollars:.2f}"

def main() -> None:
    price = Money(cents=1999, currency="$")
    println(f"Price: {price}")    # Price: $19.99
    println(f"Debug: {price:?}")  # Debug: Money { cents: 1999, currency: "$" }
```

### Identifiers

```incan
model UserId:
    value: int

    def __str__(self) -> str:
        return f"user_{self.value}"

def main() -> None:
    id = UserId(value=42)
    println(f"ID: {id}")     # ID: user_42
    println(f"Raw: {id:?}")  # Raw: UserId { value: 42 }
```

---

## See Also

- [Derives Overview](../derives_and_traits.md) - Complete reference
- [Custom Behavior](./custom_behavior.md) - All dunder overrides
