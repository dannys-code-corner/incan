# String processing

This page collects practical recipes for working with `str` and `bytes`.

## Split and index safely

```incan
line = "alice,30,engineer"
parts = line.split(",")

name = parts[0]
age = parts[1]
role = parts[2]
```

> Note: Indexing panics if out of range. If you need fallible parsing, validate the length before indexing (or use a
> `Result`-returning helper function).

## Build strings

```incan
words = ["hello", "world"]
sentence = " ".join(words)
println(sentence.upper())  # HELLO WORLD
```

## Clean input

```incan
raw_input = "  user@example.com  "
email = raw_input.strip().lower()
println(email)  # user@example.com
```

## See also

- [Strings and bytes (Reference)](../reference/strings.md)
- [Strings and formatting (Tutorial)](../tutorials/book/07_strings_and_formatting.md)
- [File I/O (How-to)](file_io.md)
