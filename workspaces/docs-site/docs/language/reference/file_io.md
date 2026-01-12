# File I/O reference (Path, File, and IoError)

This page is a reference for the `Path` and `File` APIs and the common `IoError` variants.

For task-oriented examples, see [File I/O (how-to)](../how-to/file_io.md).

## Path methods and properties

```incan
# Create from string
p = Path("data/config.toml")

# Join paths with /
cfg = Path("config") / "app.toml"

# Queries
p.exists()
p.is_file()
p.is_dir()
p.is_absolute()

# Components
p.name
p.stem
p.suffix
p.parent
p.parts

# Derive new paths
p.with_suffix(".tmp")
p.with_name("summary.txt")
```

### Path constructors, joins, queries, and components

| API                 | Returns     | Description                                 |
| ------------------- | ----------- | ------------------------------------------- |
| `Path(s)`           | `Path`      | Create a path from a string                 |
| `p / "child"`       | `Path`      | Join paths                                  |
| `p.exists()`        | `bool`      | Does this path exist?                       |
| `p.is_file()`       | `bool`      | Is this path a file?                        |
| `p.is_dir()`        | `bool`      | Is this path a directory?                   |
| `p.is_absolute()`   | `bool`      | Is this an absolute path?                   |
| `p.name`            | `str`       | Filename with extension                     |
| `p.stem`            | `str`       | Filename without extension                  |
| `p.suffix`          | `str`       | File extension (including the dot)          |
| `p.parent`          | `Path`      | Parent directory                            |
| `p.parts`           | `list[str]` | Path components                             |
| `p.with_suffix(s)`  | `Path`      | Return a new path with a different suffix   |
| `p.with_name(name)` | `Path`      | Return a new path with a different filename |

### Path I/O methods

| Method             | Returns                       | Description                    |
| ------------------ | ----------------------------- | ------------------------------ |
| `p.read_text()`    | `Result[str, IoError]`        | Read file as string            |
| `p.read_bytes()`   | `Result[bytes, IoError]`      | Read file as bytes             |
| `p.read_lines()`   | `Result[list[str], IoError]`  | Read file as lines             |
| `p.write_text(s)`  | `Result[None, IoError]`       | Write string to file           |
| `p.write_bytes(b)` | `Result[None, IoError]`       | Write bytes to file            |
| `p.read_dir()`     | `Result[list[Path], IoError]` | List directory                 |
| `p.glob(pattern)`  | `Result[list[Path], IoError]` | Find matching paths            |
| `p.mkdir()`        | `Result[None, IoError]`       | Create directory               |
| `p.mkdir_all()`    | `Result[None, IoError]`       | Create directories recursively |
| `p.remove()`       | `Result[None, IoError]`       | Delete file                    |
| `p.rmdir()`        | `Result[None, IoError]`       | Delete empty directory         |
| `p.remove_all()`   | `Result[None, IoError]`       | Delete directory tree          |
| `p.rename(new)`    | `Result[None, IoError]`       | Rename/move                    |

## File methods

| Method                | Returns                 | Description                 |
| --------------------- | ----------------------- | --------------------------- |
| `File.open(p)`        | `Result[File, IoError]` | Open for reading            |
| `File.create(p)`      | `Result[File, IoError]` | Create/truncate for writing |
| `File.open_append(p)` | `Result[File, IoError]` | Open for appending          |
| `f.read_all()`        | `Result[str, IoError]`  | Read entire file            |
| `f.lines()`           | `Iterator[str]`         | Iterate lines               |
| `f.write(s)`          | `Result[None, IoError]` | Write string                |
| `f.write_line(s)`     | `Result[None, IoError]` | Write line with newline     |

## IoError

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

## RAII: Automatic resource cleanup

Incan uses RAII (Resource Acquisition Is Initialization) for file handles. When a `File` goes out of scope, it’s
automatically closed and flushed.
