# Scopes & Name Resolution

Incan is **lexically scoped** (like Python and Rust): a name refers to the *nearest* binding in the surrounding source
structure. The practical differences from Python are:

- Incan has **block scope** (most indented blocks introduce a new scope).
- Incan’s “plain assignment” (`x = ...` without `let`/`mut`) is **inferred**: if `x` already exists in an enclosing
  scope, it is treated as a **reassignment** (so it requires `x` to be mutable). If `x` does not exist yet, it creates a
  new binding in the current scope.

This page explains what that means in day-to-day code.

## Mental model

Think of the compiler keeping a **stack of scopes**. When you enter a new syntactic region (module, function, block, …)
it pushes a scope; when you leave it pops the scope. Name lookup walks from the top of the stack outward until it finds
a match.

Internally this is implemented by the compiler’s symbol table (`lookup()` searches outward; `lookup_local()` checks only
the current scope).

## What counts as a scope in Incan?

### Module scope (file scope)

Everything at top level in a file lives in the **module scope**:

- `import` / `from ... import ...` bindings
- `const` bindings
- top-level function definitions, models/classes/enums, etc.

### Function / method scope

Each `def ...:` body is a **function scope**.

Methods also have a function-like scope and define `self` (and `mut self`) for the method body.

### Block scope (important difference from Python)

Bodies of these constructs are checked in a **block scope**:

- `if` / `else`
- `while`
- `for`
- `if` expressions (treated as statement-like blocks in the current checker)
- list/dict comprehensions (the loop variable is local to the comprehension)

So variables defined in such blocks do **not** “leak out” to the surrounding scope (unlike Python’s `if`/`for`/`while`,
which don’t create a new scope).

## Bindings vs reassignment

Incan uses the same syntax `x = value` for both:

- creating a **new binding**, and
- **reassigning** an existing one.

The current rule is:

- `let x = ...` always creates a **new immutable binding** in the current scope (this is how you intentionally shadow).
- `mut x = ...` always creates a **new mutable binding** in the current scope (a fresh binding, but mutable).
- Plain `x = ...` is **inferred**:
  - if `x` already exists in any enclosing scope, it is a **reassignment** (so `x` must be mutable)
  - otherwise it creates a **new immutable binding** in the current scope

### `mut` controls reassignment

- A binding is **immutable by default**.
- You can only reassign a binding if it was declared `mut`.
- Shadowing an outer name is done with `let` (a new binding in the inner scope).

Example:

```incan
def reassigns_outer() -> int:
    mut x = 1

    if true:
        x = 2

    return x  # 2
```

Shadowing example (block-local):

```incan
def shadows_in_block() -> int:
    let x = 1

    if true:
        let x = 2  # new binding in the block scope (shadows outer x)

    return x  # still 1
```

Notes on `let`:

- `let x = ...` is the explicit form for “**introduce a new binding** in the current scope”.
- It’s **optional** when introducing a name for the first time (plain `x = ...` will do that if `x` doesn’t exist yet),
  but `let` is how you make **shadowing** unambiguous and readable in nested scopes.
- `mut x = ...` introduces a **new mutable** binding. Reassigning later uses plain `x = ...` (and requires that `x` was
  introduced with `mut`).

You get an error if you write this:

```incan
def errors_without_mut() -> Unit:
    let x = 1
    if true:
        x = 2  # error: cannot reassign immutable variable 'x'
```

That’s because plain `x = 2` is treated as a reassignment (since `x` already exists), and the outer `x` is immutable.

### Prefer explicit `let` / `mut`

Use `let`/`mut` when you care about whether you’re shadowing or reassigning; it avoids surprises and matches what you
intended.

## Closures and capturing

Incan closures use arrow syntax:

```incan
add1 = (x) => x + 1
```

Closures introduce their own function scope (parameters are local to the closure body). Names from outer scopes can be
**read** by normal lexical lookup.

Plain assignment inside a closure behaves the same as elsewhere: if the name exists already, it’s treated as a
reassignment (so it requires the outer binding to be `mut`). Shadow with `let` if you want a closure-local name.

Incan does not currently expose Python-style `global` / `nonlocal` declarations.

## Comparison to Python (LEGB)

Python’s classic rule-of-thumb is “LEGB” (Local → Enclosing → Global → Builtins), but Python has a
surprising extra rule:

- **Any assignment** to a name anywhere inside a function makes that name **local to the whole function**
  unless you declare it `global`/`nonlocal`.

In Incan:

- name lookup is lexical (nearest scope wins), **and**
- assignments decide “new vs reassign” based on the **current scope**, **and**
- `if/for/while` bodies are **block-scoped**, so block-local names don’t leak.

## Common gotchas (and how to think about them)

- **“Why did `x = ...` inside an `if` error?”**
  - If `x` already exists, plain `x = ...` is treated as a reassignment, so `x` must be `mut`.
  - If you intended a new block-local `x`, use `let x = ...` in the block.

- **“Why does `x = ...` sometimes require `mut`?”**
  - Only *reassignment* requires `mut`. Plain assignment is inferred: it reassigns if `x` already exists; otherwise it
    creates a new immutable binding.

- **“Where do imported names live?”**
  - Imports add names to the **module scope** (file scope). Inside functions, you can refer to them like any
    other outer binding.

## See also

- [Imports & Modules](imports_and_modules.md) — module boundaries, exports (`pub`), and importing names
- [Closures](closures.md) — arrow syntax and when to use closures
