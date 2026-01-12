# Derives: Serialization (Reference)

This page documents `Serialize` and `Deserialize` for JSON.

See also:

- [Derives & traits](../derives_and_traits.md)
- [Error handling](../../explanation/error_handling.md)

---

## Serialize

- **Derive**: `@derive(Serialize)`
- **API**: `json_stringify(value) -> str`

```incan
@derive(Serialize)
model User:
    name: str
    age: int

def main() -> None:
    u = User(name="Alice", age=30)
    println(json_stringify(u))
```

---

## Deserialize

- **Derive**: `@derive(Deserialize)`
- **API**: `T.from_json(input: str) -> Result[T, str]`

```incan
@derive(Deserialize)
model User:
    name: str
    age: int

def main() -> None:
    result: Result[User, str] = User.from_json("{\"name\":\"Alice\",\"age\":30}")
```

---

## Type mappings (Incan → JSON)

| Incan | JSON |
| --- | --- |
| `str` | string |
| `int` | number |
| `float` | number |
| `bool` | boolean |
| `List[T]` | array |
| `Dict[str, T]` | object |
| `Option[T]` | value or `null` |
| `model` / `class` | object |


