# 9. Enums and better `match`

Enums let you represent “one of several shapes” as a real type, and `match` lets you handle it exhaustively.

## A small command enum

```incan
enum Command:
    Add(int)
    Remove(int)
    List
```

!!! tip "Coming from Python?"
    Python’s `Enum` is mostly “named constants”. If you want variants that carry different data, Python often uses:
    strings, `dataclass` hierarchies, or `Union` types.

    In Incan, `enum` variants can carry data (like `Add(int)`), and `match` gives you an exhaustive, compiler-checked way
    to handle every case.

## Matching on enums

```incan
def run(cmd: Command) -> None:
    match cmd:
        Add(x) => println(f"add {x}")
        Remove(x) => println(f"remove {x}")
        List => println("list")

def main() -> None:
    run(Command.Add(1))     # outputs: add 1
    run(Command.Remove(1))  # outputs: remove 1
    run(Command.List)       # outputs: list
```

## Why this matters

Unlike a stringly-typed “command name”, enums:

- prevent typos
- make invalid states unrepresentable
- give you exhaustive `match` checking

## Try it

1. Add a `Clear` variant and update the `match` accordingly.
2. Add a `Rename(str)` variant and print the new name.

??? example "One possible solution"

    ```incan
    enum Command:
        Add(int)
        Remove(int)
        Rename(str)
        Clear
        List

    def run(cmd: Command) -> None:
        match cmd:
            Add(x) => println(f"add {x}")
            Remove(x) => println(f"remove {x}")
            Rename(name) => println(f"rename {name}")
            Clear => println("clear")
            List => println("list")
    ```

## Where to learn more

- Enums deep dive: [Enums](../../explanation/enums.md)

## Next

Back: [8. Collections and iteration](08_collections_and_iteration.md)

Next chapter: [10. Models vs classes](10_models_vs_classes.md)



