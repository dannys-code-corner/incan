# Callable objects (Reference)

This page documents stdlib traits for callable objects.

## Callable0 / Callable1 / Callable2

These model “objects that can be called” like `obj()`, `obj(x)`, `obj(x, y)`.

- **Callable0[R]**
  - Hook: `__call__(self) -> R`
- **Callable1[A, R]**
  - Hook: `__call__(self, arg: A) -> R`
- **Callable2[A, B, R]**
  - Hook: `__call__(self, a: A, b: B) -> R`



