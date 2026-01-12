# Collection protocols (Reference)

This page documents stdlib traits that model Python-like collection behavior.

## Contains (membership)

- **Syntax**: `item in collection` / `item not in collection`
- **Hook**: `__contains__(self, item: T) -> bool`
- **Trait**: `Contains[T]`

## Len (length)

- **Syntax**: `len(x)`
- **Hook**: `__len__(self) -> int`
- **Trait**: `Len`

## Iterable / Iterator (iteration)

- **Syntax**: `for x in y:`
- **Hooks**:
    - `__iter__(self) -> Iterator[T]`
    - `__next__(self) -> Option[T]`
- **Traits**:
    - `Iterable[T]`
    - `Iterator[T]`

## Bool (truthiness)

- **Syntax**: `if x:` / `while x:`
- **Hook**: `__bool__(self) -> bool`
- **Trait**: `Bool`
