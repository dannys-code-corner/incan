# Operator traits (Reference)

This page documents stdlib traits that describe operator behavior for custom types.

These traits define dunder-like hooks such as `__add__`, `__sub__`, etc.

## Add / Sub / Mul / Div / Neg / Mod

- **`Add[Rhs, Output]`**: `__add__(self, other: Rhs) -> Output`
- **`Sub[Rhs, Output]`**: `__sub__(self, other: Rhs) -> Output`
- **`Mul[Rhs, Output]`**: `__mul__(self, other: Rhs) -> Output`
- **`Div[Rhs, Output]`**: `__div__(self, other: Rhs) -> Output`
- **`Neg[Output]`**: `__neg__(self) -> Output`
- **`Mod[Rhs, Output]`**: `__mod__(self, other: Rhs) -> Output`



