# Numeric Semantics (Python-like)

Incan numeric operations are **mostly Python-like**, but with two important differences:

- Incan is **statically typed**: variables do **not** change type after initialization
  unless you explicitly annotate them differently (or use a different variable).
- Numeric runtime behavior is implemented by the generated Rust + `incan_stdlib`, so some
  corner cases (notably NaN/Inf) follow **IEEE/Rust** behavior.

This document describes the exact rules we implement today.

## Supported numeric types (today)

- `int`: currently compiled as Rust `i64`.
- `float`: currently compiled as Rust `f64`.

## Numeric promotion rules

In expressions, operands may be promoted to float:

- For arithmetic ops, if either operand is `float`, the operation is performed in `float`
  (with some operator-specific rules below).
- For mixed comparisons (`int` vs `float`), operands are promoted to `float` for
  comparison.

Promotion affects expression typing and generated code, not variable types.

## Operator semantics

### `/` (true division)

Division is **always float**, even for `int / int`.

#### Returns (`/`)

- `float`

#### Examples (`/`)

```incan
1 / 2      # 0.5
4 / 2      # 2.0
7.0 / 2    # 3.5
7 / 2.0    # 3.5
```

#### Differences from Python

- For division by zero, Python raises `ZeroDivisionError`. Incan currently
  **panics at runtime** with:
  - `ZeroDivisionError: float division by zero`

### `//` (floor division)

Floor division rounds toward **negative infinity**.

#### Returns (`//`)

- `int` if both operands are `int`
- otherwise `float`

#### Examples (`//`)

```incan
7 // 3        # 2
-7 // 3       # -3
7 // -3       # -3
-7 // -3      # 2

7.0 // 3      # 2.0
-7.0 // 3     # -3.0
7 // 3.0      # 2.0
```

#### Differences from Rust

Rust integer division truncates toward zero. Incan `//` is Python floor division
(toward -∞).

#### Panics (`//`)

- Panics on zero divisor with `ZeroDivisionError: float division by zero`.

### `%` (modulo / remainder)

Modulo uses Python remainder semantics: the remainder has the **sign of the divisor**
and satisfies:

\[
a == (a // b) * b + (a \% b)
\]

#### Returns (`%`)

- `int` if both operands are `int`
- otherwise `float`

#### Examples (`%`)

```incan
7 % 3         # 1
-7 % 3        # 2      (Rust would give -1)
7 % -3        # -2     (Rust would give 1)
-7 % -3       # -1

7.0 % 3.0     # 1.0
-7.0 % 3.0    # 2.0
7.0 % -3.0    # -2.0
```

#### Panics (`%`)

- Panics on zero divisor with `ZeroDivisionError: float division by zero`.

### `**` (power)

Power is mostly Python-like, with one deliberate typing rule to keep codegen efficient:

- `int ** int` returns `int` **only** when the exponent is a **non-negative `int`
  literal**.
- In all other cases, the result is `float`.

#### Returns (`**`)

- `int` only for `int ** <non-negative int literal>`
- otherwise `float`

#### Examples (`**`)

```incan
2 ** 3        # 8        (int)
2 ** 0        # 1        (int)
2 ** -1       # 0.5      (float)

exp = 3
2 ** exp      # float (even if exp is int at runtime)

2.0 ** 3      # float
2 ** 3.0      # float
```

#### Notes

- This is a **compile-time** rule: we only choose integer exponentiation when we can prove
  the exponent is a non-negative integer literal.

### Comparisons (`==`, `!=`, `<`, `<=`, `>`, `>=`)

Mixed numeric comparisons are allowed:

- `int` and `float` can be compared.
- Operands are promoted to `float` for comparison.
- The result type is `bool`.

#### Examples (comparisons)

```incan
1 == 1.0      # true
1 < 1.5       # true
2 >= 2.0      # true
```

## Compound assignments (`+=`, `-=`, `*=`, `/=`, `//=`, `%=`)

Compound assignment is typechecked as if it were:

```text
x <op>= y   ≈   x = (x <op> y)
```

Because variables are statically typed, the **result type** of `(x <op> y)` must be
assignable back to `x`.

### Examples (compound assignments)

```incan
mut x: int = 10
x += 2        # ok (int)
x *= 3        # ok (int)
# x /= 2      # error: (int / int) is float, cannot assign to int

mut y: float = 10.0
y /= 2        # ok (float)
y %= 7        # ok (float)
```

## NaN / Infinity (documented divergence)

For `float`, NaN/Inf behavior follows IEEE/Rust behavior from the generated code and
`incan_stdlib`.
This may differ from Python in some corner cases.

### Examples (NaN/Inf)

```incan
# Conceptual examples; exact behavior depends on IEEE rules.
#
# Note: Incan currently panics on division by zero, so these *do not* produce
# NaN/Inf today:
# nan = 0.0 / 0.0
# inf = 1.0 / 0.0
#
# NaN/Inf can still appear via Rust interop or other IEEE-producing operations.
pass
```
