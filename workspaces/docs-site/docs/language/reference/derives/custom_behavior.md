# Derives: Custom behavior (Reference)

This page lists the dunder hooks used to customize derived behavior.

See also:

- [Derives & traits](../derives_and_traits.md)
- [Customize derived behavior (How-to)](../../how-to/customize_derived_behavior.md)
- [Stdlib traits](../stdlib_traits/index.md)

---

## Dunder hooks

| Hook | Purpose |
| --- | --- |
| `__str__` | Display formatting (`{value}`) |
| `__eq__` | Equality (`==`, `!=`) |
| `__lt__` | Ordering (`<`, sorting) |
| `__hash__` | Hashing (`Set` / `Dict` keys) |

Rule:

- You must not combine a hook with the corresponding `@derive(...)` (conflict).

---

## Reflection helpers

Models and classes provide:

- `__fields__() -> List[str]`
- `__class_name__() -> str`

Example:

```incan
model User:
    name: str

def main() -> None:
    u = User(name="Alice")
    println(u.__class_name__())
    println(u.__fields__())
```


