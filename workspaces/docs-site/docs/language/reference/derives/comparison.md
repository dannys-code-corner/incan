# Derives: Comparison (Reference)

This page documents `Eq`, `Ord`, and `Hash`.

See also:

- [Derives & traits](../derives_and_traits.md)
- [Customize derived behavior (How-to)](../../how-to/customize_derived_behavior.md)

---

## Eq (equality)

- **Enables**: `==`, `!=`
- **Default behavior**: structural, field-based equality
- **Custom behavior**: define `__eq__(self, other: Self) -> bool`
- **Conflict rule**: if you define `__eq__`, do not also `@derive(Eq)`

---

## Ord (ordering)

- **Enables**: `<`, `<=`, `>`, `>=`, `sorted(...)`
- **Default behavior**: structural ordering by field order
- **Custom behavior**: define `__lt__(self, other: Self) -> bool`
- **Conflict rule**: if you define `__lt__`, do not also `@derive(Ord)`

---

## Hash

- **Enables**: use as `Set` members and `Dict` keys
- **Default behavior**: structural, field-based hashing
- **Custom behavior**: define `__hash__(self) -> int`
- **Conflict rule**: if you define `__hash__`, do not also `@derive(Hash)`

Consistency rule:

- If `a == b`, then `a.__hash__() == b.__hash__()`.

---

## Tie-breakers (ordering)

If you implement `__lt__`, avoid “accidental ties” by adding a deterministic tie-breaker:

```incan
model Task:
    priority: int
    name: str

    def __lt__(self, other: Task) -> bool:
        if self.priority != other.priority:
            return self.priority < other.priority
        return self.name < other.name
```


