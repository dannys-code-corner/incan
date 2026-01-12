# Control flow

This page explains branching and looping constructs in Incan.

## Branching with `if`

```incan
def classify(n: int) -> str:
    if n < 0:
        return "negative"
    elif n == 0:
        return "zero"
    else:
        return "positive"
```

## Pattern matching with `match`

Use `match` to branch on enum values like `Result` and `Option`.

```incan
def main() -> None:
    result = parse_port("8080")

    match result:
        case Ok(port): println(f"port={port}")
        case Err(e): println(f"error: {e}")
```

## Looping with `for`

Incan supports Python-like `for` loops:

```incan
def main() -> None:
    items = ["Alice", "Bob", "Cara"]

    for name in items:
        println(name)
```

Break early when needed:

```incan
for name in items:
    if name == "Bob":
        break
```

## See also

- Book chapter: [4. Control flow](../tutorials/book/04_control_flow.md)
- Enums and `match`: [Enums](enums.md)
- Error-driven control flow (`Result`/`Option`): [Error Handling](error_handling.md)
