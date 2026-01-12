# Derives: String representation (Reference)

This page documents `Debug` and `Display` derives and their behavior.

See also:

- [Derives & traits](../derives_and_traits.md)

---

## Debug (Automatic)

- **Format**: `{value:?}`
- **User override**: not supported
- **Intended behavior**: structured output (type name + fields)

```incan
model Point:
    x: int
    y: int

def main() -> None:
    p = Point(x=10, y=20)
    println(f"{p:?}")  # Point { x: 10, y: 20 }
```

---

## Display (Automatic, customizable with `__str__`)

- **Format**: `{value}`
- **Default behavior**: types have a default Display representation (similar to Python’s default `__str__`)
- **Custom behavior**: define `__str__(self) -> str`
- **Conflict rule**: if you define `__str__`, do not also `@derive(Display)`

```incan
model User:
    name: str
    email: str

    def __str__(self) -> str:
        return f"{self.name} <{self.email}>"

def main() -> None:
    u = User(name="Alice", email="alice@example.com")
    println(f"{u}")    # Alice <alice@example.com>
    println(f"{u:?}")  # User { name: "Alice", email: "alice@example.com" }
```

---

## Debug vs Display (quick guide)

| Aspect | Debug (`{:?}`) | Display (`{}`) |
| --- | --- | --- |
| Purpose | developers/logs | users/output |
| Customizable | no | yes (`__str__`) |



