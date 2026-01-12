# 8. Collections and iteration

Incan has Python-like collections and a familiar `for` loop.

!!! tip "Coming from Rust?"
    Incan’s built-in collections compile to the standard Rust collections:

    - `list[T]` / `List[T]` → `Vec<T>`
    - `dict[K, V]` / `Dict[K, V]` → `HashMap<K, V>`
    - `set[T]` / `Set[T]` → `HashSet<T>`

    (See the prelude type mapping in the [imports/modules reference](../../reference/imports_and_modules.md) for details.)

## Lists

```incan
def main() -> None:
    xs = [1, 2, 3]
    println(f"first={xs[0]}")
```

## Dicts

```incan
def main() -> None:
    scores = dict([("alice", 10), ("bob", 7)])
    println(f"alice={scores['alice']}")
```

## Sets

Use a set to deduplicate values:

```incan
def main() -> None:
    names = ["Alice", "Bob", "Alice"]
    unique = set(names)
    println(f"unique_count={len(unique)}")
```

## Iteration with `for`

```incan
def main() -> None:
    names = ["Alice", "Bob", "Cara"]
    for name in names:
        println(name)
```

## Comprehensions (quick transforms)

Use comprehensions to build a new list/dict from an existing collection:

```incan
def main() -> None:
    names = [" Alice ", "Bob", " Cara "]

    normalized = [name.strip().lower() for name in names]
    counts = {name: 1 for name in normalized}

    println(f"normalized={normalized:?}")
    println(f"counts={counts:?}")
```

## Try it

1. Read a list of names, normalize (`strip().lower()`), and print them.
2. Create a `dict[str, int]` of counts for how often each name appears.
3. Use a `set` to print only unique names.

??? example "One possible solution"

    ```incan
    def main() -> None:
        names = ["Alice", "Alice", "Bob", "Bob", "Cara"]

        # 1) Normalize + print
        normalized = [name.strip().lower() for name in names]
        for name in normalized:
            println(name)

        # 2) Count occurrences
        name_counts: Dict[str, int] = {}
        for name in normalized:
            current_count = name_counts.get(name).unwrap_or(0)
            name_counts[name] = current_count + 1

        # 3) Deduplicate + print counts (set iteration order is not guaranteed)
        unique = set(normalized)
        for name in unique:
            println(f"{name}: {name_counts.get(name).unwrap()}")
    ```

## Where to learn more

- Strings and slicing: [Strings](../../reference/strings.md)
- Control flow overview: [Control flow](../../explanation/control_flow.md)

## Next

Back: [7. Strings and formatting](07_strings_and_formatting.md)

Next chapter: [9. Enums and better `match`](09_enums.md)



