# File I/O in Incan

Incan provides file and path handling inspired by Python's `pathlib` and Rust's `std::fs`. If you're coming from Python, the API will feel familiar — but with explicit error handling via `Result`.

> **Coming from Python?** Incan's `Path` type works like `pathlib.Path`, and file operations mirror what you'd do with `open()`. The key difference: errors are returned as `Result`, not raised as exceptions.

## Paths

### Creating Paths

```incan
# From string
path = Path("data/config.toml")

# Join paths with /
config_dir = Path("config")
config_file = config_dir / "app.toml"  # config/app.toml

# Home directory
home = Path.home()
downloads = home / "Downloads"

# Current directory
cwd = Path.cwd()
```

**Python equivalent:**

```python
from pathlib import Path

path = Path("data/config.toml")
config_file = Path("config") / "app.toml"
home = Path.home()
cwd = Path.cwd()
```

### Path Components

```incan
path = Path("/home/user/documents/report.pdf")

path.name        # "report.pdf"
path.stem        # "report"
path.suffix      # ".pdf"
path.parent      # Path("/home/user/documents")
path.parts       # ["/", "home", "user", "documents", "report.pdf"]

# Change extension
new_path = path.with_suffix(".txt")  # /home/user/documents/report.txt

# Change filename
renamed = path.with_name("summary.pdf")  # /home/user/documents/summary.pdf
```

### Path Queries

```incan
path = Path("config.toml")

path.exists()       # bool - does it exist?
path.is_file()      # bool - is it a file?
path.is_dir()       # bool - is it a directory?
path.is_absolute()  # bool - absolute path?

# Make absolute
abs_path = path.absolute()
```

---

## Reading Files

### Read Entire File

```incan
# Read as string - returns Result[str, IoError]
content = Path("config.toml").read_text()

# Read as bytes - returns Result[bytes, IoError]
data = Path("image.png").read_bytes()
```

Since these return `Result`, you need to handle the potential error:

```incan
# Option 1: Use ? to propagate (in a function returning Result)
content = Path("config.toml").read_text()?

# Option 2: Match on the result
match Path("config.toml").read_text():
    case Ok(content): println(content)
    case Err(e): println(f"Failed: {e}")

# Option 3: Provide a default
content = Path("config.toml").read_text().unwrap_or("")
```

**Python equivalent:**

```python
# Python - exceptions instead of Result
content = Path("config.toml").read_text()  # raises if file missing
data = Path("image.png").read_bytes()
```

### Read Lines

```incan
def process_log() -> Result[None, IoError]:
    for line in Path("app.log").read_lines()?:
        if "ERROR" in line:
            println(line)
    return Ok(None)
```

### File Handle (for large files)

For large files or when you need more control, use `File.open()`:

```incan
def process_large_file(path: Path) -> Result[int, IoError]:
    file = File.open(path)?
    
    mut count = 0
    for line in file.lines():
        count += 1
    
    return Ok(count)
    # <- file is automatically closed here (RAII)
```

> **Coming from Python?** Unlike Python's `with open(...) as f:` context manager, Incan uses RAII (Resource Acquisition Is Initialization). The file is automatically closed when the `file` variable goes out of scope — no `with` block needed.

---

## Writing Files

### Write Entire File

```incan
# Write string
def save_config(content: str) -> Result[None, IoError]:
    Path("config.toml").write_text(content)?
    return Ok(None)

# Write bytes
def save_image(data: bytes) -> Result[None, IoError]:
    Path("output.png").write_bytes(data)?
    return Ok(None)
```

**Python equivalent:**

```python
# Python
Path("config.toml").write_text(content)
Path("output.png").write_bytes(data)
```

### File Handle (for streaming writes)

```incan
def write_report(data: list[str]) -> Result[None, IoError]:
    mut file = File.create("report.txt")?
    
    for line in data:
        file.write_line(line)?
    
    return Ok(None)
    # <- file is flushed and closed automatically
```

### Append to File

```incan
def append_log(message: str) -> Result[None, IoError]:
    mut file = File.open_append("app.log")?
    file.write_line(f"[{timestamp()}] {message}")?
    return Ok(None)
```

---

## Directory Operations

### List Directory Contents

```incan
def list_configs() -> Result[list[Path], IoError]:
    mut configs = []
    for entry in Path("config").read_dir()?:
        if entry.suffix == ".toml":
            configs.append(entry)
    return Ok(configs)
```

**Python equivalent:**

```python
# Python
configs = [p for p in Path("config").iterdir() if p.suffix == ".toml"]
```

### Glob Patterns

```incan
# Find all .py files recursively
def find_python_files() -> Result[list[Path], IoError]:
    files = Path(".").glob("**/*.py")?
    return Ok(files)

# Non-recursive glob
def find_configs() -> Result[list[Path], IoError]:
    files = Path("config").glob("*.toml")?
    return Ok(files)
```

**Python equivalent:**

```python
# Python
files = list(Path(".").glob("**/*.py"))
configs = list(Path("config").glob("*.toml"))
```

### Create Directories

```incan
# Create single directory
Path("output").mkdir()?

# Create nested directories (like mkdir -p)
Path("output/reports/2024").mkdir_all()?
```

### Remove Files and Directories

```incan
# Remove file
Path("temp.txt").remove()?

# Remove empty directory
Path("empty_dir").rmdir()?

# Remove directory and contents (careful!)
Path("build").remove_all()?
```

---

## Error Handling

File operations return `Result[T, IoError]`. Common error variants:

```incan
enum IoError:
    NotFound(path: Path)
    PermissionDenied(path: Path)
    AlreadyExists(path: Path)
    IsDirectory(path: Path)
    NotDirectory(path: Path)
    Other(message: str)
```

### Handling Errors

```incan
def safe_read(path: Path) -> str:
    match path.read_text():
        case Ok(content): return content
        case Err(IoError.NotFound(_)): return ""
        case Err(e):
            println(f"Error reading {path}: {e.message()}")
            return ""
```

### Check Before Operating

```incan
def ensure_config() -> Result[str, IoError]:
    path = Path("config.toml")
    
    if not path.exists():
        # Create default config
        path.write_text(DEFAULT_CONFIG)?
    
    return path.read_text()
```

---

## RAII: Automatic Resource Cleanup

Incan uses RAII (Resource Acquisition Is Initialization) for file handles. When a `File` goes out of scope, it's automatically closed and flushed.

```incan
def process_file() -> Result[str, IoError]:
    file = File.open("data.txt")?
    content = file.read_all()?
    return Ok(content)
    # <- file is automatically closed here
```

This is equivalent to Python's context manager, but implicit:

```python
# Python equivalent
with open("data.txt") as file:
    content = file.read()
# <- file closed here
```

**Why RAII?**

- No forgotten `file.close()` calls
- No need for `with` blocks everywhere
- Resources are always cleaned up, even if errors occur
- The compiler ensures correctness

---

## Common Patterns

### Config File Loading

```incan
model Config:
    host: str
    port: int
    debug: bool

def load_config(path: Path) -> Result[Config, AppError]:
    content = path.read_text().map_err(AppError.Io)?
    config = parse_toml[Config](content).map_err(AppError.Parse)?
    return Ok(config)
```

### Safe File Update (write to temp, then rename)

```incan
def safe_update(path: Path, content: str) -> Result[None, IoError]:
    temp_path = path.with_suffix(".tmp")
    
    # Write to temporary file
    temp_path.write_text(content)?
    
    # Atomic rename
    temp_path.rename(path)?
    
    return Ok(None)
```

### Process All Files in Directory

```incan
def process_all_logs() -> Result[int, IoError]:
    mut total_lines = 0
    
    for path in Path("logs").glob("*.log")?:
        lines = path.read_lines()?
        total_lines += len(lines)
    
    return Ok(total_lines)
```

---

## API Reference

### Path Methods

| Method | Returns | Description |
|--------|---------|-------------|
| `Path(s)` | `Path` | Create from string |
| `Path.home()` | `Path` | User's home directory |
| `Path.cwd()` | `Path` | Current working directory |
| `p / "child"` | `Path` | Join paths |
| `p.name` | `str` | Filename with extension |
| `p.stem` | `str` | Filename without extension |
| `p.suffix` | `str` | File extension |
| `p.parent` | `Path` | Parent directory |
| `p.exists()` | `bool` | Check existence |
| `p.is_file()` | `bool` | Is a file? |
| `p.is_dir()` | `bool` | Is a directory? |
| `p.read_text()` | `Result[str, IoError]` | Read file as string |
| `p.read_bytes()` | `Result[bytes, IoError]` | Read file as bytes |
| `p.read_lines()` | `Result[list[str], IoError]` | Read lines |
| `p.write_text(s)` | `Result[None, IoError]` | Write string to file |
| `p.write_bytes(b)` | `Result[None, IoError]` | Write bytes to file |
| `p.read_dir()` | `Result[list[Path], IoError]` | List directory |
| `p.glob(pattern)` | `Result[list[Path], IoError]` | Find matching paths |
| `p.mkdir()` | `Result[None, IoError]` | Create directory |
| `p.mkdir_all()` | `Result[None, IoError]` | Create directories recursively |
| `p.remove()` | `Result[None, IoError]` | Delete file |
| `p.rmdir()` | `Result[None, IoError]` | Delete empty directory |
| `p.remove_all()` | `Result[None, IoError]` | Delete directory tree |
| `p.rename(new)` | `Result[None, IoError]` | Rename/move |

### File Methods

| Method | Returns | Description |
|--------|---------|-------------|
| `File.open(p)` | `Result[File, IoError]` | Open for reading |
| `File.create(p)` | `Result[File, IoError]` | Create/truncate for writing |
| `File.open_append(p)` | `Result[File, IoError]` | Open for appending |
| `f.read_all()` | `Result[str, IoError]` | Read entire file |
| `f.lines()` | `Iterator[str]` | Iterate lines |
| `f.write(s)` | `Result[None, IoError]` | Write string |
| `f.write_line(s)` | `Result[None, IoError]` | Write line with newline |

---

## See Also

- [Error Handling Guide](./error_handling.md) - Working with `Result` types
- [Derives & Traits](./derives_and_traits.md) - Drop trait for custom cleanup
