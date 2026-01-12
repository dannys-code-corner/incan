# Conversion traits (Reference)

This page documents stdlib traits for explicit conversions.

## From / Into

- **`From[T]`**
    - Hook: `@classmethod def from(cls, value: T) -> Self`
- **`Into[T]`**
    - Hook: `def into(self) -> T`

## TryFrom / TryInto

- **`TryFrom[T]`**
    - Hook: `@classmethod def try_from(cls, value: T) -> Result[Self, str]`
- **`TryInto[T]`**
    - Hook: `def try_into(self) -> Result[T, str]`
