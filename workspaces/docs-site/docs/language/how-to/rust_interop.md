# Rust Interoperability

Incan compiles to Rust, which means you can import from Rust crates and interoperate with Rust types.

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

When you use `import rust::crate_name`, Incan automatically adds the dependency to your generated `Cargo.toml`.

For the bigger picture, see: [Projects today](../../tooling/explanation/projects_today.md).

### Strict Dependency Policy

Incan uses a **strict dependency policy** to reduce dependency drift and keep builds predictable:

- **Known-good crates**: Curated crates have tested version/feature combinations (see table below)
- **Unknown crates**: Currently produce a compiler error (planned: explicit versions and project config)

This policy prevents "works on my machine" issues caused by wildcard (`*`) dependencies
that resolve to different versions at different times.

### Known-Good Crates

The following crates have pre-configured versions with appropriate features:

| Crate      | Version | Features                            |
| ---------- | ------- | ----------------------------------- |
| serde      | 1.0     | derive                              |
| serde_json | 1.0     | -                                   |
| tokio      | 1       | rt-multi-thread, macros, time, sync |
| time       | 0.3     | formatting, macros                  |
| chrono     | 0.4     | serde                               |
| reqwest    | 0.11    | json                                |
| uuid       | 1.0     | v4, serde                           |
| rand       | 0.8     | -                                   |
| regex      | 1.0     | -                                   |
| anyhow     | 1.0     | -                                   |
| thiserror  | 1.0     | -                                   |
| tracing    | 0.1     | -                                   |
| clap       | 4.0     | derive                              |
| log        | 0.4     | -                                   |
| env_logger | 0.10    | -                                   |
| sqlx       | 0.7     | runtime-tokio-native-tls, postgres  |
| futures    | 0.3     | -                                   |
| bytes      | 1.0     | -                                   |
| itertools  | 0.12    | -                                   |

### Using Unknown Crates

**Current behavior**: If you try to import a crate not in the known-good list, you'll see an error:

```bash
Error: unknown Rust crate `my_crate`: no known-good version mapping exists.

To use this crate today, request that `my_crate` be added to the known-good list by opening an issue/PR.
```

> **Why so strict?** Implicit wildcard (`*`) dependencies can cause “works on my machine” failures when crate versions change.
> This policy will evolve—[RFC 013](../../RFCs/013_rust_crate_dependencies.md) proposes inline version annotations, `incan.toml`
> project configuration, and lock files for full flexibility.

**Current workarounds**:

1. Open an issue/PR to add the crate to the known-good list
2. (Temporary) Manually edit the generated `Cargo.toml` to add your dependency

### Adding to the Known-Good List

If you'd like a crate added to the known-good list:

1. Open an issue or PR on the Incan repository
2. Include the crate name, recommended version, and any required features
3. Explain why this crate is commonly useful

The maintainers will test the crate and add it to `src/backend/project.rs`.

### Coming Soon: Version Annotations and `incan.toml`

[RFC 013](../../RFCs/013_rust_crate_dependencies.md) defines a comprehensive dependency system that will allow:

```incan
# Inline version annotations (planned)
import rust::my_crate @ "1.0"
import rust::tokio @ "1.35" with ["full"]
```

```toml
# incan.toml project configuration (planned)
[project]
name = "my_app"
version = "0.1.0"

[rust.dependencies]
my_crate = "1.0"
tokio = { version = "1.35", features = ["full"] }
```

This will enable any Rust crate while maintaining reproducibility.

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

| Incan          | Rust            |
| -------------- | --------------- |
| `int`          | `i64`           |
| `float`        | `f64`           |
| `str`          | `String`        |
| `bool`         | `bool`          |
| `List[T]`      | `Vec<T>`        |
| `Dict[K, V]`   | `HashMap<K, V>` |
| `Set[T]`       | `HashSet<T>`    |
| `Option[T]`    | `Option<T>`     |
| `Result[T, E]` | `Result<T, E>`  |

## Understanding Rust types (optional)

??? tip "Coming from Python?"
    If you’re new to Rust types like `Vec`, `HashMap`, `String`, `Option`, and `Result`, see
    [Understanding Rust types (coming from Python)](rust_types_for_python_devs.md).

## Limitations

1. **Lifetime annotations**: Rust's borrow checker and lifetime annotations are not exposed in Incan.
    Types that require explicit lifetime management may not work directly.

2. **Generic bounds**: Complex trait bounds on generic types are simplified.
    Some advanced generic patterns may need wrapper functions.

3. **Unsafe code**: Incan cannot call unsafe Rust functions directly.
    If you need unsafe operations, create a safe wrapper in Rust first.

4. **Macros**: Rust macros are not directly callable. Use the expanded form or a wrapper function.

5. **Feature flags**: Default features are used for common crates.
    For custom feature combinations, edit the generated `Cargo.toml` manually.

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

- [Error Handling](../explanation/error_handling.md) - Working with `Result` types
- [Derives & Traits](../reference/derives_and_traits.md) - Drop trait for custom cleanup
- [File I/O](file_io.md) - Reading, writing, and path handling
- [Async Programming](async_programming.md) - Async/await with Tokio
- [Imports & Modules](imports_and_modules.md) - Module system, imports, and built-in functions
- [Rust Interop](rust_interop.md) - Using Rust crates directly from Incan
- [Web Framework](../tutorials/web_framework.md) - Building web apps with Axum
