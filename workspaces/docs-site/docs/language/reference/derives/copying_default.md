# Derives: Copying and Default (Reference)

This page documents `Clone`, `Copy`, and `Default`, plus field defaults in constructors.

See also:

- [Derives & traits](../derives_and_traits.md)

---

## Clone

- **Enables**: `.clone()`
- **Meaning**: explicit duplication

---

## Copy

- **Meaning**: marker trait for implicit copying
- **Intended use**: small value types (no heap ownership)

Copy vs Clone (rule of thumb):

| Trait | Copy happens when | Use for |
| --- | --- | --- |
| `Clone` | you call `.clone()` | any type |
| `Copy` | assignment/pass-by-value | small, simple value types |

---

## Field defaults (construction)

Field defaults are written on fields:

```incan
model Settings:
    theme: str = "dark"
    font_size: int = 14
```

Rules:

- If a field has a default, you may omit it in construction and the default is used.
- If a field has no default, it must be provided when constructing the type.

---

## Default

- **Enables**: `Type.default()`
- **Semantics**:
  - fields with explicit defaults use those defaults
  - otherwise, the default of the field type is used

Common type defaults:

| Type | Default |
| --- | --- |
| `int` | `0` |
| `float` | `0.0` |
| `bool` | `false` |
| `str` | `""` |
| `List[T]` | `[]` |
| `Dict[K, V]` | `{}` |
| `Set[T]` | `set()` |
| `Option[T]` | `None` |


