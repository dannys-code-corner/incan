# File I/O in Incan

This page is a practical guide to common file and path tasks.

File operations return `Result`, so failures are explicit and easy to handle.

??? tip "Coming from Python?"
    Incan’s `Path` is similar in spirit to `pathlib.Path`, but file operations return `Result` instead of raising exceptions.

??? tip "Coming from Rust?"
    Incan’s `Path` is closest to a `std::path::PathBuf`-style API, and file I/O mirrors `std::fs`.
    Errors are explicit via `Result`, so you’ll typically use `?` to propagate failures.

??? tip "Coming from JavaScript / TypeScript?"
    Think of `Path` as a typed, composable alternative to Node’s `path` utilities, and file I/O as `fs/promises`-style
    operations—except errors come back as `Result` instead of throwing (so you use `match` / `?` instead of `try/catch`).

## Recipe: Read a file (text or bytes)

```incan
def read_config_text() -> Result[str, IoError]:
    return Path("config.toml").read_text()

def read_image_bytes() -> Result[bytes, IoError]:
    return Path("image.png").read_bytes()
```

## Recipe: Handle vs propagate I/O errors

Use `?` to propagate errors:

```incan
def read_config_text() -> Result[str, IoError]:
    content = Path("config.toml").read_text()?
    return Ok(content)
```

Use `match` when you want to recover:

```incan
def safe_read_text(path: Path) -> str:
    match path.read_text():
        case Ok(content): return content
        case Err(IoError.NotFound(_)): return ""
        case Err(e):
            println(f"Failed to read {path}: {e.message()}")
            return ""
```

## Recipe: Print a file line-by-line

```incan
def print_file(path: Path) -> Result[None, IoError]:
    # `read_lines()` returns Result[list[str], IoError]
    # `?` propagates Err(...) to the caller, otherwise unwraps Ok(lines).
    for line in path.read_lines()?:
        println(line)
    return Ok(None)
```

## Recipe: Stream a large file (use `File`)

For large files or when you need more control, use `File.open()` and iterate:

```incan
def count_lines(path: Path) -> Result[int, IoError]:
    file = File.open(path)?

    mut count = 0
    for _ in file.lines():
        count += 1

    return Ok(count)
    # file is closed automatically when it goes out of scope (RAII)
```

## Recipe: Write a file (text or bytes)

```incan
def save_config(content: str) -> Result[None, IoError]:
    Path("config.toml").write_text(content)?
    return Ok(None)

def save_image(data: bytes) -> Result[None, IoError]:
    Path("output.png").write_bytes(data)?
    return Ok(None)
```

## Recipe: Append to a file

```incan
def append_log(message: str) -> Result[None, IoError]:
    mut file = File.open_append("app.log")?
    file.write_line(f"[{timestamp()}] {message}")?
    return Ok(None)
```

## Recipe: Safe update (write temp, then rename)

```incan
def safe_update(path: Path, content: str) -> Result[None, IoError]:
    temp_path = path.with_suffix(".tmp")

    temp_path.write_text(content)?
    temp_path.rename(path)?

    return Ok(None)
```

## Recipe: Walk a directory and filter files

```incan
def list_toml_configs() -> Result[list[Path], IoError]:
    mut configs = []
    for entry in Path("config").read_dir()?:
        if entry.suffix == ".toml":
            configs.append(entry)
    return Ok(configs)
```

## Recipe: Find files with glob patterns

```incan
def find_python_files() -> Result[list[Path], IoError]:
    return Ok(Path(".").glob("**/*.py")?)

def find_configs() -> Result[list[Path], IoError]:
    return Ok(Path("config").glob("*.toml")?)
```

## Recipe: Ensure a config file exists (create default if missing)

```incan
const DEFAULT_CONFIG: str = "debug = true\n"

def ensure_config() -> Result[str, IoError]:
    path = Path("config.toml")

    if not path.exists():
        path.write_text(DEFAULT_CONFIG)?

    return path.read_text()
```

## See also

- [Error handling (concepts)](../explanation/error_handling.md)
- [File I/O reference (Path/File APIs)](../reference/file_io.md)
