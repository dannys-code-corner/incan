# Imports and Modules in Incan

Incan uses a module system that combines Rust's safety with Python's ergonomics.

## Import Syntax

Incan supports two import styles that can be mixed freely:

### Python-style: `from module import ...`

Best for importing multiple items from the same module:

```incan
# Import multiple items at once
from models import User, Product, Order

# Import with aliases
from utils import format_currency as fmt, validate_email as check_email
```

### Rust-style: `import module::item`

Best for single imports or when you want explicit paths:

```incan
# Import a specific item
import models::User

# Import with an alias
import utils::format_currency as fmt
```

### Mixing Both Styles

You can use both styles in the same file:

```incan
# Python-style for multiple items from same module
from models import User, Product

# Rust-style for individual items
import models::calculate_total
import utils::format_currency
```

## Advanced Import Paths

Incan supports rich path syntax for navigating complex project structures.

### Child Directory Imports

For nested project structures, use dots (Python-style) or `::` (Rust-style):

```incan
# Python-style: dots for nested paths
from db.models import User, Product
from api.handlers.auth import login, logout

# Rust-style: :: for nested paths
import db::models::User
import api::handlers::auth::login
```

Both compile to Rust's nested module syntax:

```rust
use db::models::{User, Product};
use api::handlers::auth::{login, logout};
```

### Parent Directory Imports

Navigate to parent directories using `..` (Python-style) or `super` (Rust-style):

```incan
# Python-style: .. for parent
from ..common import Logger
from ...shared.utils import format_date

# Rust-style: super keyword
import super::common::Logger
import super::super::shared::utils::format_date
```

| Prefix | Meaning |
|--------|---------|
| `..` or `super::` | Parent directory (one level up) |
| `...` or `super::super::` | Grandparent directory (two levels up) |

### Absolute Imports (Project Root)

Import from the project root using `crate`:

```incan
# From anywhere in the project, import from root
from crate.config import Settings
import crate::lib::database::Connection
```

The compiler finds the project root by looking for `Cargo.toml` or `src/` directory.

### Path Summary

| Incan Path | Meaning | Rust Equivalent |
|------------|---------|-----------------|
| `models` | Same directory | `models` |
| `db.models` | Child `db/models.incn` | `db::models` |
| `..common` | Parent's `common.incn` | `super::common` |
| `super::utils` | Parent's `utils.incn` | `super::utils` |
| `crate.config` | Root's `config.incn` | `crate::config` |

## The Prelude

The **prelude** is a set of types and traits automatically available in every Incan file without explicit imports. This mirrors Rust's prelude concept.

### What's in the Incan Prelude

These types are always available:

| Incan Type | Rust Type | Description |
|------------|-----------|-------------|
| `int` | `i64` | 64-bit signed integer |
| `float` | `f64` | 64-bit floating point |
| `bool` | `bool` | Boolean (true/false) |
| `str` | `String` | UTF-8 string |
| `bytes` | `Vec<u8>` | Byte array |
| `List[T]` | `Vec<T>` | Dynamic array |
| `Dict[K, V]` | `HashMap<K, V>` | Hash map |
| `Set[T]` | `HashSet<T>` | Hash set |
| `Option[T]` | `Option<T>` | Optional value (Some/None) |
| `Result[T, E]` | `Result<T, E>` | Success or error (Ok/Err) |

### Built-in Functions (Always Available)

```incan
# Output
println(value)      # Print with newline
print(value)        # Print without newline

# Collections
len(collection)     # Get length

# Iteration
range(n)            # Iterator 0..n
range(start, end)   # Iterator start..end
range(start..end)   # Iterator start..end (Rust-style range literal)
range(start..=end)  # Iterator start..=end (inclusive end)
enumerate(iter)     # Iterator with indices
zip(iter1, iter2)   # Pair up two iterators

# Type conversion (Python-like)
dict()              # Empty Dict
dict(mapping)       # Convert to Dict
list()              # Empty List
list(iterable)      # Convert to List
set()               # Empty Set
set(iterable)       # Convert to Set
```

Incan syntax options for `range`:

- Rust-style: `range(start..end)` or `range(start..=end)`
- Python-style: `range(start, end)`
- Single-arg: `range(n)` yields `0..n`

Choose whichever style you prefer; both are equivalent in Incan and compile to Rust ranges.

## Special import: `import this`

`import this` is always available and prints the Incan “Zen” design principles when imported. It works in regular modules and inline snippets, e.g.:

```bash
incan run -c "import this"
```

### Why a Prelude?

Without a prelude, every file would need:

```incan
import std::collections::HashMap
import std::collections::HashSet
import std::vec::Vec
import std::string::String
import std::option::Option
import std::result::Result
# ... tedious!
```

The prelude eliminates this boilerplate for common types.

## Multi-File Projects

### Simple Project Structure

```bash
myproject/
├── main.incn      # Entry point
├── models.incn    # Data types
└── utils.incn     # Helper functions
```

### Nested Project Structure

```bash
myproject/
├── src/
│   ├── main.incn      # Entry point
│   ├── config.incn    # Configuration
│   ├── db/
│   │   └── models.incn    # Database models
│   ├── api/
│   │   └── handlers.incn  # API handlers
│   └── shared/
│       └── utils.incn     # Shared utilities
```

### How It Works

When you import from a local module, the compiler:

1. Resolves the path (handling `.`, `..`, `super`, `crate`)
2. Looks for the `.incn` file (or `mod.incn` for directories)
3. Parses and type-checks that file
4. Makes its types and functions available in your main file
5. Generates combined Rust code with all definitions

**Coming from Rust?**  
Incan doesn't require explicit `mod` declarations. In Rust, you must declare modules before importing:

```rust
// Rust: must declare first
mod models;
use models::User;
```

In Incan, just import — the compiler discovers modules automatically:

```incan
# Incan: no mod declaration needed
from models import User
```

**Coming from Python?**  
Incan doesn't need `__init__.py` files. In Python, every package directory requires an `__init__.py` to be importable. In Incan, directories are automatically recognized as modules — just create your `.incn` files and import. For directories that need a "main" file (like Python's `__init__.py`), use `mod.incn`.

### Module Visibility

By default, items are private to their module. Use `pub` to export:

```incan
# In models.incn

# Public - can be imported by other modules
pub model User:
    name: str
    email: str

# Private - only usable within this file
model InternalCache:
    data: Dict[str, str]

# Public function
pub def create_user(name: str) -> User:
    return User(name=name, email="")

# Private helper
def validate_internal(data: str) -> bool:
    return len(data) > 0
```

## Rust Standard Library Access

Incan can import from Rust's standard library:

```incan
import std::fs           # File system operations
import std::env          # Environment variables
import std::path::Path   # Path manipulation
import std::time         # Time operations
```

However, using these requires understanding the underlying Rust types. The built-in functions (`read_file`, `write_file`, etc.) provide a more ergonomic interface for common operations.

## Current Status

Multi-file compilation is fully supported:

- ✅ Python-style imports: `from module import item1, item2`
- ✅ Rust-style imports: `import module::item`
- ✅ Nested paths: `from db.models import User`
- ✅ Parent navigation: `..common` or `super::common`
- ✅ Absolute paths: `crate::config`
- ✅ Aliases: `import module::item as alias`
- ✅ Cross-file type resolution
- ✅ Combined Rust code generation

### Limitations

1. **No wildcard imports**: `from module import *` is not supported
2. **No re-exports**: Cannot re-export imported items
3. **No circular imports**: Module A cannot import B if B imports A

## Examples

### Example 1: Simple Multi-File Project

See `examples/advanced/multifile/` for a basic example:

```bash
examples/advanced/multifile/
├── main.incn      # Entry point with imports
├── models.incn    # User and Product models  
└── utils.incn     # Helper functions
```

Run the example:

```bash
incan run examples/advanced/multifile/main.incn
```

**main.incn** - Both import styles:

```incan
# Python-style - multiple items from same module
from models import User, Product

# Rust-style - individual items
import models::product_total
import utils::format_currency

def main() -> None:
    user = User(name="Alice", email="alice@example.com", age=30)
    laptop = Product(name="Laptop", price=999.99, quantity=1)
    total = product_total(laptop)
    println(f"Total: {format_currency(total)}")
```

### Example 2: Nested Project Structure

See `examples/nested_project/` for advanced imports:

```bash
examples/advanced/nested_project/
└── src/
    ├── main.incn
    ├── db/
    │   └── models.incn
    ├── api/
    │   └── handlers.incn
    └── shared/
        └── utils.incn
```

**src/main.incn**:

```incan
# Child directory imports (using dots)
from db.models import User, Product

# Child directory imports (using ::)
import api.handlers::handle_request
import shared.utils::format_date

def main() -> None:
    user = User(name="Alice", email="alice@example.com")
    response = handle_request("/api/users")
    date = format_date(2024, 12, 8)
    println(f"User: {user.name}, Response: {response}, Date: {date}")
```

Run the example:

```bash
incan run examples/advanced/nested_project/src/main.incn
```

## See Also

- [Error Handling](error_handling.md) - Working with `Result` types
- [Derives & Traits](derives_and_traits.md) - Drop trait for custom cleanup
- [File I/O](file_io.md) - Reading, writing, and path handling
- [Async Programming](async_programming.md) - Async/await with Tokio
- [Imports & Modules](imports_and_modules.md) - Module system, imports, and built-in functions
- [Rust Interop](rust_interop.md) - Using Rust crates directly from Incan
- [Web Framework](web_framework.md) - Building web apps with Axum
