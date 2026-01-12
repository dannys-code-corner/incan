# 2. Values, variables, and types

This chapter introduces basic values and types.

If you get stuck on terminology, see: [Glossary](../../reference/glossary.md).

## Basic values

Common built-in types:

- `int` (integers)
- `float` (floating point)
- `bool` (`true` / `false`)
- `str` (strings)

## Variables

Assign a value to a name:

!!! tip "Coming from Python?"
    Incan supports Python-style f-strings for string interpolation:

    ```incan
    name = "Incan"
    println(f"hello, {name}")
    ```

    For more examples and formatting details, see: [Strings](../../reference/strings.md).

```incan
def main() -> None:
    name = "Incan"
    answer = 42
    println(f"{name}: {answer}")
```

## Function arguments and return types

Incan uses type annotations on function arguments and return types:

```incan
def add(a: int, b: int) -> int:
    return a + b
```

## Try it

1. Create a `str`, `int`, `float`, and `bool` variable and print them.
2. Use an f-string with multiple `{...}` interpolations.
3. Call `add(10, 32)` and print the result.

??? example "One possible solution"

    ```incan
    def add(a: int, b: int) -> int:
        return a + b
    
    def main() -> None:
        name = "Incan"
        count = 3
        ratio = 0.5
        enabled = true

        println(f"name={name} count={count} ratio={ratio} enabled={enabled}")
        println(f"add={add(10, 32)}")
    ```

## Next

Back: [1. Hello world](01_hello_world.md)

Next: [3. Functions](03_functions.md)
