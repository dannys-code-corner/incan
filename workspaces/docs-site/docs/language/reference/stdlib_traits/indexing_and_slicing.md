# Indexing and slicing (Reference)

This page documents stdlib traits for `obj[key]`, `obj[key] = value`, and slicing.

## Index (read)

- **Syntax**: `obj[key]`
- **Hook**: `__getitem__(self, key: K) -> V`
- **Trait**: `Index[K, V]`

## IndexMut (write)

- **Syntax**: `obj[key] = value`
- **Hook**: `__setitem__(self, key: K, value: V) -> None`
- **Trait**: `IndexMut[K, V]`

## Slicing

- **Syntax**: `obj[start:end:step]`

Incan’s long-term direction is slice-aware `__getitem__` (Python-style).
The current stdlib vocabulary includes `Sliceable[T]` and `__getslice__`, which will be aligned with `__getitem__`
as the feature is finalized.



