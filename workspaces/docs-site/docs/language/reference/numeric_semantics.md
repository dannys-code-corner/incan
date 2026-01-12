# Numeric semantics

This page documents the numeric rules Incan implements today (typing + runtime behavior).

Incan is **statically typed**, so numeric operations affect **expression types and codegen**, not “the type of a variable
over time”.

!!! note "Notes if you’re coming from Python"
    Incan’s numeric intent is often Python-like, but some behavior differs because:

    - Incan is statically typed (no implicit “variable becomes float later”).
    - Runtime behavior is produced by generated Rust + `incan_stdlib` (so IEEE float edge cases apply).

!!! note "Notes if you’re coming from Rust"
    Some operators intentionally follow Python-style semantics (not Rust’s), notably floor division (`//`) and modulo (`%`)
    for negative numbers.

## Table of contents

[Supported numeric types]: #supported-numeric-types
[Numeric promotion rules]: #numeric-promotion-rules
[Operator semantics]: #operator-semantics
[Compound assignments]: #compound-assignments
[NaN / Infinity]: #nan-infinity

| Section                   | What it covers                                       |
| ------------------------- | ---------------------------------------------------- |
| [Supported numeric types] | `int`/`float` and their current Rust representations |
| [Numeric promotion rules] | When expressions promote to `float`                  |
| [Operator semantics]      | `/`, `//`, `%`, `**`, comparisons                    |
| [Compound assignments]    | `+=`, `-=`, `*=`, `/=`, `//=`, `%=` typing rules     |
| [NaN / Infinity]          | IEEE behavior and current limitations                |

## Supported numeric types

Today, the compiler uses these Rust representations:

- `int`: currently compiled as Rust `i64`.
- `float`: currently compiled as Rust `f64`.

## Numeric promotion rules

In expressions, operands may be promoted to `float`:

- For arithmetic ops, if either operand is `float`, the operation is performed in `float`
  (with some operator-specific rules below).
- For mixed comparisons (`int` vs `float`), operands are promoted to `float` for
  comparison.

Promotion affects expression typing and generated code, not variable types.

## Operator semantics

### `/` (true division)

Division is **always `float`**, even for `int / int`.

#### Returns (`/`)

- `float`

#### Examples (`/`)

```incan
1 / 2      # 0.5
4 / 2      # 2.0
7.0 / 2    # 3.5
7 / 2.0    # 3.5
```

!!! note "Python comparison"
    Python raises `ZeroDivisionError` for division by zero. Incan currently **panics at runtime** with a
    `ZeroDivisionError: float division by zero`-style message.

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

!!! note "Rust comparison"
    Rust integer division (`/`) truncates toward zero. Incan `//` is floor division (toward \( -\infty \)).

#### Panics (`//`)

- Panics on zero divisor with `ZeroDivisionError: float division by zero`.

### `%` (modulo / remainder)

Modulo uses Python remainder semantics.  
The remainder has the **sign of the divisor** and satisfies the following equation:

\[
a = \left\lfloor \frac{a}{b} \right\rfloor b + (a \bmod b)
\]

> In Incan operator notation: `a == (a // b) * b + (a % b)`

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

!!! note "Rust comparison"
    Rust’s `%` is a remainder operator with truncating division, so negative cases differ from Incan.

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

## Compound assignments

This section covers: `+=`, `-=`, `*=`, `/=`, `//=`, `%=`

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

## NaN / Infinity

This section documents how `float` behaves today.

For `float`, NaN/Inf behavior follows IEEE/Rust behavior from the generated code and `incan_stdlib`.

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

!!! note "Python comparison"
    Python’s float behavior differs in some edge cases and error behavior (exceptions vs panics).
