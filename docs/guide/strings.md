# Strings and Bytes in Incan

Incan has two text-related types:

- `str` — Text strings (compiles to Rust's `String`)
- `bytes` — Binary data (compiles to Rust's `Vec<u8>`)

String methods use Python-style names for familiarity.

## Quick Reference

| Incan | Python | Rust Generated |
|-------|--------|----------------|
| `s.upper()` | `s.upper()` | `s.to_uppercase()` |
| `s.lower()` | `s.lower()` | `s.to_lowercase()` |
| `s.strip()` | `s.strip()` | `s.trim().to_string()` |
| `s.split(",")` | `s.split(",")` | `s.split(",").map(\|s\| s.to_string()).collect()` |
| `", ".join(list)` | `", ".join(list)` | `list.join(", ")` |
| `s.contains("x")` | `"x" in s` | `s.contains("x")` |
| `s.replace("a", "b")` | `s.replace("a", "b")` | `s.replace("a", "b")` |

## String Methods

### Case Conversion

```incan
text = "Hello World"
println(text.upper())  # HELLO WORLD
println(text.lower())  # hello world
```

**Python equivalent:** Identical — `str.upper()` and `str.lower()`.

### Whitespace Trimming

```incan
padded = "  hello  "
println(padded.strip())  # "hello"
```

**Python equivalent:** Identical — `str.strip()`.

**Note:** Unlike Python, Incan currently only has `strip()` (both sides). Python's `lstrip()` and `rstrip()` are not yet implemented.

### Splitting Strings

```incan
csv = "alice,bob,carol"
names = csv.split(",")  # ["alice", "bob", "carol"]

# Access elements
first = names[0]  # "alice"
```

**Python equivalent:** Identical — `str.split(delimiter)`.

**Returns:** `List[str]`

### Joining Strings

```incan
names = ["alice", "bob", "carol"]
result = ", ".join(names)  # "alice, bob, carol"
```

**Python equivalent:** Identical — `separator.join(list)`.

**Note:** Like Python, the separator is the object and the list is the argument. This differs from some languages where it's `list.join(separator)`.

### Substring Check

```incan
sentence = "the quick brown fox"

if sentence.contains("quick"):
    println("Found it!")
```

**Python difference:** Python uses the `in` operator (`"quick" in sentence`), while Incan uses the `contains()` method.

**Returns:** `bool`

### String Replacement

```incan
text = "hello world"
result = text.replace("world", "incan")  # "hello incan"
```

**Python equivalent:** Identical — `str.replace(old, new)`.

**Note:** Replaces all occurrences, like Python's default behavior.

## F-Strings (Formatted Strings)

Incan supports Python-style f-strings:

```incan
name = "Alice"
age = 30
println(f"Name: {name}, Age: {age}")
```

### Debug Formatting

Use `:?` for debug output (shows type structure):

```incan
model Point:
    x: int
    y: int

p = Point(x=10, y=20)
println(f"Debug: {p:?}")  # Point { x: 10, y: 20 }
```

See [String Representation](./derives/string_representation.md) for details on Debug vs Display formatting.

## String Literals

```incan
# Single or double quotes
s1 = "hello"
s2 = 'hello'

# Multiline strings (triple quotes)
multi = """
This is a
multiline string
"""

# F-strings
formatted = f"Value: {x}"
```

## Bytes (Binary Data)

The `bytes` type represents binary data as a sequence of bytes. It compiles to Rust's `Vec<u8>`.

### Byte String Literals

Use the `b"..."` prefix for byte strings:

```incan
# ASCII byte string
data = b"Hello"

# Hex escapes for arbitrary bytes
binary = b"\x00\x01\x02\xff"

# Common escapes
newline = b"\n"
tab = b"\t"
null = b"\0"
```

**Supported escape sequences:**

| Escape | Meaning |
|--------|---------|
| `\n` | Newline |
| `\t` | Tab |
| `\r` | Carriage return |
| `\\` | Backslash |
| `\0` | Null byte |
| `\xNN` | Hex byte (e.g., `\xff` = 255) |

**Note:** Byte strings only accept ASCII characters. Non-ASCII characters produce an error.

### Type Annotation

```incan
def process_binary(data: bytes) -> bytes:
    # Work with binary data
    return data
```

### Python Comparison

| Incan | Python | Notes |
|-------|--------|-------|
| `bytes` | `bytes` | Same concept |
| `b"hello"` | `b"hello"` | Same syntax |
| `b"\xff"` | `b"\xff"` | Same hex escapes |
| `Vec<u8>` (Rust) | `bytes` | Underlying type |

**Key difference:** Python's `bytes` is immutable, while Incan's `bytes` (Rust's `Vec<u8>`) is mutable.

### When to Use Bytes vs Strings

| Use Case | Type |
|----------|------|
| Text, user-facing content | `str` |
| File contents (text) | `str` |
| Binary files (images, etc.) | `bytes` |
| Network protocols | `bytes` |
| Cryptographic operations | `bytes` |
| Raw file I/O | `bytes` |

## Comparison to Python

### What's the Same

| Feature | Incan | Python |
|---------|-------|--------|
| F-strings | `f"Hello {name}"` | `f"Hello {name}"` |
| `upper()`/`lower()` | ✅ | ✅ |
| `strip()` | ✅ | ✅ |
| `split(delimiter)` | ✅ | ✅ |
| `separator.join(list)` | ✅ | ✅ |
| `replace(old, new)` | ✅ | ✅ |
| Triple-quoted strings | ✅ | ✅ |

### What's Different

| Feature | Incan | Python |
|---------|-------|--------|
| Substring check | `s.contains("x")` | `"x" in s` |
| `lstrip()`/`rstrip()` | ❌ Not yet | ✅ |
| `startswith()`/`endswith()` | ❌ Not yet | ✅ |
| `find()`/`index()` | ❌ Not yet | ✅ |
| `isdigit()`/`isalpha()` | ❌ Not yet | ✅ |
| String multiplication | ❌ Not yet | `"x" * 3` → `"xxx"` |
| `format()` method | ❌ Use f-strings | `"{} {}".format(a, b)` |

## Comparison to Rust

Incan's string methods are thin wrappers over Rust's `String` methods:

| Incan | Rust | Notes |
|-------|------|-------|
| `str` type | `String` | Owned, heap-allocated |
| `s.upper()` | `s.to_uppercase()` | Python naming |
| `s.lower()` | `s.to_lowercase()` | Python naming |
| `s.strip()` | `s.trim().to_string()` | Returns owned String |
| `s.split(",")` | `s.split(",").collect()` | Returns `Vec<String>` |
| `s.contains("x")` | `s.contains("x")` | Same |
| `s.replace("a", "b")` | `s.replace("a", "b")` | Same |

**Key difference:** Rust distinguishes between `&str` (borrowed slice) and `String` (owned). Incan abstracts this — you always work with owned strings, and borrowing is handled automatically during compilation.

## Common Patterns

### Parsing CSV

```incan
line = "alice,30,engineer"
parts = line.split(",")
name = parts[0]
age = parts[1]
role = parts[2]
println(f"{name} is a {age}-year-old {role}")
```

### Building Strings

```incan
words = ["hello", "world"]
sentence = " ".join(words)
println(sentence.upper())  # HELLO WORLD
```

### Cleaning Input

```incan
raw_input = "  user@example.com  "
email = raw_input.strip().lower()
println(email)  # user@example.com
```

## See Also

- [String Representation](./derives/string_representation.md) — Debug and Display formatting
- [Examples: Strings](../../examples/simple/strings.incn) — String method examples
- [Examples: Bytes I/O](../../examples/advanced/bytes_io.incn) — Binary data examples
