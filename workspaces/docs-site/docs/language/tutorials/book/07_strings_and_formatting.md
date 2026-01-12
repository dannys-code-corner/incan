# 7. Strings and formatting

Strings are `str`, and you’ll often build output using f-strings.

## String methods

```incan
def main() -> None:
    raw = "  Alice  "
    cleaned = raw.strip().lower()
    println(cleaned)
```

## F-strings (interpolation)

```incan
def main() -> None:
    name = "Alice"
    age = 30
    println(f"{name} age={age}")
```

## Try it

1. Normalize an input string with `strip().lower()`.
2. Build an output line using an f-string.
3. Use one string method you didn’t use yet (for example `upper()`).

??? example "One possible solution"

    ```incan
    def main() -> None:
        raw = "  Alice  "
        cleaned = raw.strip().lower()
        println(f"cleaned={cleaned}")
        println(cleaned.upper())
    ```

## Where to learn more

- Full strings guide: [Strings](../../reference/strings.md)

## Next

Back: [6. Errors (Result/Option and `?`)](06_errors.md)

Next chapter: [8. Collections and iteration](08_collections_and_iteration.md)



