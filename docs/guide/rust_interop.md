# Rust Interoperability

Incan compiles to Rust, which means you can directly use any Rust crate from your Incan code.

## Importing Rust Crates

Use the `rust::` prefix to import from Rust crates:

```incan
# Import entire crate
import rust::serde_json as json

# Import specific items
from rust::time import Instant, Duration
from rust::std::collections import HashMap, HashSet

# Import nested items
import rust::serde_json::Value
```

## Automatic Dependency Management

When you use `import rust::crate_name`, Incan automatically adds the dependency to your generated `Cargo.toml`. Common crates have pre-configured versions with appropriate features:

| Crate | Version | Features |
|-------|---------|----------|
| serde | 1.0 | derive |
| serde_json | 1.0 | - |
| tokio | 1 | rt-multi-thread, macros, time, sync |
| time | 0.3 | formatting, macros |
| chrono | 0.4 | serde |
| reqwest | 0.11 | json |
| uuid | 1.0 | v4, serde |
| rand | 0.8 | - |
| regex | 1.0 | - |
| anyhow | 1.0 | - |
| thiserror | 1.0 | - |
| clap | 4.0 | derive |

For unknown crates, Incan uses the latest compatible version.

## Examples

### Working with JSON (serde_json)

```incan
import rust::serde_json as json
from rust::serde_json import Value

def parse_json(data: str) -> Value:
    return json.from_str(data).unwrap()

def main() -> None:
    data = '{"name": "Alice", "age": 30}'
    parsed = parse_json(data)
    println(f"Name: {parsed['name']}")
```

### Working with Time

```incan
from rust::time import Instant, Duration

def measure_operation() -> None:
    start = Instant.now()
    
    # Do some work
    for i in range(1000000):
        pass
    
    elapsed = start.elapsed()
    println(f"Operation took: {elapsed}")
```

### HTTP Requests (reqwest)

```incan
import rust::reqwest

async def fetch_data(url: str) -> str:
    response = await reqwest.get(url)
    return await response.text()

async def main() -> None:
    data = await fetch_data("https://api.example.com/data")
    println(data)
```

### Using Collections

```incan
from rust::std::collections import HashMap, HashSet

def count_words(text: str) -> HashMap[str, int]:
    counts = HashMap.new()
    for word in text.split():
        count = counts.get(word).unwrap_or(0)
        counts.insert(word, count + 1)
    return counts
```

### Random Numbers

```incan
from rust::rand import Rng, thread_rng

def random_int(min: int, max: int) -> int:
    rng = thread_rng()
    return rng.gen_range(min..max)

def main() -> None:
    for _ in range(5):
        println(f"Random: {random_int(1, 100)}")
```

### UUIDs

```incan
from rust::uuid import Uuid

def generate_id() -> str:
    return Uuid.new_v4().to_string()

def main() -> None:
    id = generate_id()
    println(f"Generated ID: {id}")
```

## Type Mapping

Incan types map to Rust types:

| Incan | Rust |
|-------|------|
| `int` | `i64` |
| `float` | `f64` |
| `str` | `String` |
| `bool` | `bool` |
| `List[T]` | `Vec<T>` |
| `Dict[K, V]` | `HashMap<K, V>` |
| `Set[T]` | `HashSet<T>` |
| `Option[T]` | `Option<T>` |
| `Result[T, E]` | `Result<T, E>` |

## For Python Developers: Understanding Rust Types

If you're coming from Python, here's what you need to know about common Rust types:

### Collections: Dict vs HashMap

**They're the same thing!** Incan's `Dict` compiles to Rust's `HashMap`. Use whichever you prefer:

```incan
# These are equivalent:
counts: Dict[str, int] = {}           # Pythonic
counts: HashMap[str, int] = HashMap.new()  # Rust-style
```

**When to use `Dict`** (recommended):

- Writing normal Incan code
- Pythonic syntax with `{}` literals

**When to use `HashMap` directly**:

- Interfacing with Rust crate APIs that return `HashMap`
- Need specific Rust methods like `.entry()` or `.retain()`

### Common Rust Types Explained

| Rust Type | Python Equivalent | Notes |
|-----------|------------------|-------|
| `HashMap<K, V>` | `dict` | Same as Incan's `Dict` |
| `HashSet<T>` | `set` | Same as Incan's `Set` |
| `Vec<T>` | `list` | Same as Incan's `List` |
| `String` | `str` | Owned string (same as Incan's `str`) |
| `&str` | `str` | Borrowed string slice — avoid in Incan |
| `Option<T>` | `Optional[T]` or `None` | `Some(value)` or `None` |
| `Result<T, E>` | No direct equivalent | Success (`Ok`) or Error (`Err`) |
| `Instant` | `datetime.now()` | Point in time for measuring |
| `Duration` | `timedelta` | Length of time |

### Method Naming Conventions

Rust uses different conventions than Python:

| Python | Rust | Example |
|--------|------|---------|
| `dict.get(key)` | `map.get(&key)` | Returns `Option` |
| `dict[key]` | `map[&key]` | Panics if missing |
| `dict.get(key, default)` | `map.get(&key).unwrap_or(default)` | With default |
| `str(x)` | `x.to_string()` | Convert to string |
| `len(x)` | `x.len()` | Get length |

### The `.unwrap()` Pattern

Rust functions often return `Option` or `Result` instead of raising exceptions:

```incan
# Python style (would raise KeyError):
value = my_dict["key"]

# Rust/Incan style:
value = my_dict.get("key").unwrap()     # Panics if None
value = my_dict.get("key").unwrap_or(0) # Default if None
value = my_dict.get("key")?             # Propagate None/Err
```

## Limitations

1. **Lifetime annotations**: Rust's borrow checker and lifetime annotations are not exposed in Incan. Types that require explicit lifetime management may not work directly.

2. **Generic bounds**: Complex trait bounds on generic types are simplified. Some advanced generic patterns may need wrapper functions.

3. **Unsafe code**: Incan cannot call unsafe Rust functions directly. If you need unsafe operations, create a safe wrapper in Rust first.

4. **Macros**: Rust macros are not directly callable. Use the expanded form or a wrapper function.

5. **Feature flags**: Default features are used for common crates. For custom feature combinations, edit the generated `Cargo.toml` manually.

## Best Practices

1. **Prefer Incan types**: Use Incan's built-in types when possible. Use Rust types only when you need specific functionality.

2. **Handle Results**: Rust crate functions often return `Result`. Use `?` or explicit matching:

    ```incan
    def safe_parse(s: str) -> Result[int, str]:
        return s.parse()  # Returns Result

    def main() -> None:
        match safe_parse("42"):
            case Ok(n):
                println(f"Parsed: {n}")
            case Err(e):
                println(f"Error: {e}")
    ```

3. **Async compatibility**: If using async Rust crates, make sure your Incan functions are also async.

4. **Error types**: Rust's error types can be complex. Consider using `anyhow` for simple error handling:

    ```incan
    from rust::anyhow import Result, Context

    def read_config(path: str) -> Result[Config]:
        content = fs.read_to_string(path).context("Failed to read config")?
        return parse_config(content)
    ```

## See Also

- [Error Handling](error_handling.md) - Working with `Result` types
- [Derives & Traits](derives_and_traits.md) - Drop trait for custom cleanup
- [File I/O](file_io.md) - Reading, writing, and path handling
- [Async Programming](async_programming.md) - Async/await with Tokio
- [Imports & Modules](imports_and_modules.md) - Module system, imports, and built-in functions
- [Rust Interop](rust_interop.md) - Using Rust crates directly from Incan
- [Web Framework](web_framework.md) - Building web apps with Axum
