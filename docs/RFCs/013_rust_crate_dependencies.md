# RFC 013: Rust Crate Dependencies

**Status:** Draft  
**Created:** 2025-12-16  
**Supersedes:** Parts of RFC 005 (Cargo integration section)

## Summary

Define a comprehensive system for specifying Rust crate dependencies in Incan, including inline version annotations, project configuration (`incan.toml`), and lock files for reproducibility.

## Motivation

Incan compiles to Rust, meaning access to the Rust ecosystem is a core value proposition. The current implementation has limitations:

1. **Known-good crates work** - common crates like `serde`, `tokio` have curated defaults
2. **Unknown crates error** - no user-facing way to specify version/features
3. **Workaround is manual** - edit generated `Cargo.toml` (clunky, not reproducible)

This RFC introduces a proper dependency management system that is:

- **Easy by default** - common crates "just work"
- **Flexible when needed** - any crate can be used with explicit config
- **Reproducible** - builds are deterministic via lock files
- **Pythonic** - `incan.toml` feels like `pyproject.toml`

## Goals

- Allow any Rust crate with explicit version specification
- Maintain known-good defaults as convenient fallbacks
- Support features, git sources, and path dependencies
- Provide a familiar project configuration format
- Enable reproducible builds via lock file

## Non-Goals (this RFC)

- Automatic version resolution/upgrade (future: `incan update`)
- Private registry support
- Workspace/multi-project configuration
- Native FFI bindings beyond Rust

---

## 1. Inline Version Annotations

### 1.1 Basic Version

```incan
# Pin a specific version
import rust::my_crate @ "1.0"
from rust::obscure_lib @ "0.5" import Widget

# Semver operators (like Cargo)
import rust::some_crate @ "^1.2"   # >=1.2.0, <2.0.0
import rust::other_crate @ "~1.2"  # >=1.2.0, <1.3.0
import rust::exact_crate @ "=1.2.3" # exactly 1.2.3
```

### 1.2 Version with Features

```incan
# Enable features
import rust::tokio @ "1.0" with ["full"]
import rust::serde @ "1.0" with ["derive", "rc"]

# Import from crate with features
from rust::sqlx @ "0.7" with ["runtime-tokio", "postgres"] import Pool
```

### 1.3 Grammar Extension

```ebnf
(* Extended import syntax for rust:: crates *)
rust_import     = "import" "rust" "::" crate_path [ version_spec ] [ "as" IDENT ]
                | "from" "rust" "::" crate_path [ version_spec ] "import" import_list ;

crate_path      = IDENT { "::" IDENT } ;
version_spec    = "@" version_string [ "with" feature_list ] ;
version_string  = STRING ;  (* "1.0", "^1.2", "~0.5", "=1.2.3" *)
feature_list    = "[" STRING { "," STRING } "]" ;
import_list     = IDENT { "," IDENT } ;
```

---

## 2. Project Configuration (`incan.toml`)

The `incan.toml` format is inspired by Python's `pyproject.toml` - familiar, readable, and declarative.

### 2.1 Minimal Example

```toml
[project]
name = "my_app"
version = "0.1.0"
```

### 2.2 Full Example

```toml
[project]
name = "my_app"
version = "0.1.0"
description = "An example Incan application"
authors = ["Alice <alice@example.com>"]
license = "MIT"
readme = "README.md"

# Minimum Incan version required
requires-incan = ">=0.2.0"

# Entry point for `incan run`
[project.scripts]
main = "src/main.incn"

# Rust crate dependencies
[rust.dependencies]
# Simple version string
reqwest = "0.12"
rand = "0.8"

# Version with features
sqlx = { version = "0.7", features = ["runtime-tokio", "postgres", "mysql"] }
tokio = { version = "1.35", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }

# Git dependency
my_internal_crate = { git = "https://github.com/company/crate", tag = "v1.0.0" }

# Path dependency (for local development)
local_lib = { path = "../my-local-lib" }

# Optional dependencies (enabled via features)
[rust.dependencies.optional]
fancy_logging = { version = "0.3", optional = true }

# Development dependencies (not included in release builds)
[rust.dev-dependencies]
criterion = "0.5"
proptest = "1.0"

# Build configuration
[build]
# Rust edition for generated code
rust-edition = "2021"

# Optimization level: "debug", "release", "release-lto"
profile = "release"

# Target triple (optional, defaults to host)
# target = "x86_64-unknown-linux-gnu"

# Feature flags for the Incan project itself
[project.features]
default = ["json"]
json = []  # Enables JSON serialization support
full = ["json", "fancy_logging"]
```

### 2.3 Section Reference

| Section                   | Description                                     |
|---------------------------|-------------------------------------------------|
| `[project]`               | Project metadata (name, version, authors, etc.) |
| `[project.scripts]`       | Entry points for CLI commands                   |
| `[project.features]`      | Optional feature flags                          |
| `[rust.dependencies]`     | Rust crate dependencies                         |
| `[rust.dev-dependencies]` | Development-only dependencies                   |
| `[build]`                 | Build configuration options                     |

### 2.4 Dependency Specification Formats

```toml
[rust.dependencies]
# String shorthand - just version
crate_a = "1.0"

# Table form - version + features
crate_b = { version = "1.0", features = ["foo", "bar"] }

# Table form - git source
crate_c = { git = "https://github.com/...", branch = "main" }
crate_d = { git = "https://github.com/...", tag = "v1.0.0" }
crate_e = { git = "https://github.com/...", rev = "abc1234" }

# Table form - path source
crate_f = { path = "../local-crate" }

# Optional dependency
crate_g = { version = "1.0", optional = true }

# Default features disabled
crate_h = { version = "1.0", default-features = false, features = ["only-this"] }
```

---

## 3. Lock File (`incan.lock`)

For reproducible builds, Incan generates a lock file capturing exact resolved versions.

### 3.1 Format

```toml
# Auto-generated by Incan - do not edit manually
# Regenerate with: incan lock

[metadata]
incan-version = "0.2.0"
generated = "2025-12-16T10:30:00Z"

[[rust.package]]
name = "tokio"
version = "1.35.1"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "abc123def456..."
features = ["full"]

[[rust.package]]
name = "serde"
version = "1.0.195"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "def789abc012..."
features = ["derive"]

[[rust.package]]
name = "my_internal_crate"
version = "1.0.0"
source = "git+https://github.com/company/crate?tag=v1.0.0#commit-sha"
```

### 3.2 CLI Commands

| Command                | Description                                       |
|------------------------|---------------------------------------------------|
| `incan build`          | Build; uses lock file if present, creates if not  |
| `incan lock`           | Regenerate lock file from current dependencies    |
| `incan update`         | Update dependencies to latest compatible versions |
| `incan update <crate>` | Update specific crate only                        |

---

## 4. Resolution Rules

When resolving a Rust crate dependency, the following precedence applies (highest to lowest):

```text
1. incan.toml [rust.dependencies]  → explicit project config wins
2. Inline version annotation       → import rust::foo @ "1.0"
3. Known-good defaults             → curated list in compiler
4. Error                           → unknown crate without version
```

### 4.1 Known-Good Defaults

The compiler maintains a curated list of common crates with tested version/feature combinations. These serve as **convenient defaults**, not restrictions:

```rust
// In compiler (simplified)
static KNOWN_GOOD: &[(&str, &str)] = &[
    ("serde", r#"{ version = "1.0", features = ["derive"] }"#),
    ("tokio", r#"{ version = "1", features = ["rt-multi-thread", "macros"] }"#),
    ("reqwest", r#"{ version = "0.11", features = ["json"] }"#),
    // ... etc
];
```

Users can **override** these defaults via `incan.toml` or inline annotations.

### 4.2 Conflict Resolution

```incan
# This is an error - conflicting versions
import rust::tokio @ "1.0"   # Inline says 1.0
# incan.toml says tokio = "2.0"
```

Error message:

```bash
error: conflicting versions for `tokio`

  --> src/main.incn:3
    import rust::tokio @ "1.0"

  --> incan.toml:12
    tokio = "2.0"

Remove the inline version to use incan.toml, or update incan.toml to match.
```

---

## 5. Error Messages

### 5.1 Unknown Crate Without Version

```bash
error: unknown Rust crate `my_obscure_lib`

  --> src/main.incn:5
    import rust::my_obscure_lib

This crate isn't in the known-good list. Specify a version:

    import rust::my_obscure_lib @ "1.0"

Or add it to incan.toml:

    [rust.dependencies]
    my_obscure_lib = "1.0"

Tip: Check https://crates.io/crates/my_obscure_lib for available versions.
```

### 5.2 Feature Not Found

```bash
error: feature `nonexistent` not found in crate `tokio`

  --> src/main.incn:3
    import rust::tokio @ "1.0" with ["nonexistent"]

Available features: full, rt, rt-multi-thread, macros, time, sync, net, ...
```

### 5.3 Version Not Found

```bash
error: version `99.0` of `serde` does not exist

  --> src/main.incn:3
    import rust::serde @ "99.0"

Latest version: 1.0.195
Tip: Check https://crates.io/crates/serde/versions for available versions.
```

---

## 6. Implementation Phases

### Phase 1: Inline Versions (Minimal Viable)

- Add `@ "version"` syntax to parser
- Pass version to `ProjectGenerator`
- Fall back to `*` with warning for unknown crates without version (temporary)

### Phase 2: Features Support

- Add `with ["feature"]` syntax
- Update codegen to emit features in Cargo.toml

### Phase 3: Project Configuration

- Parse `incan.toml` if present
- Merge with inline annotations per resolution rules
- Add `incan init` to create starter `incan.toml`

### Phase 4: Lock File

- Generate `incan.lock` on first build
- Use locked versions on subsequent builds
- Add `incan lock`, `incan update` commands

### Phase 5: Advanced Sources

- Git dependencies (`git = "..."`)
- Path dependencies (`path = "..."`)
- Optional dependencies

---

## 7. Examples

### 7.1 Simple Usage (Phase 1+)

```incan
# Known-good crate - just works
import rust::serde

# Unknown crate - must specify version
import rust::obscure_parser @ "2.1"

# Known-good with different version
import rust::tokio @ "1.35"  # Override default

def main() -> None:
    print("Hello, Rust ecosystem!")
```

### 7.2 Full Project (Phase 3+)

**incan.toml:**

```toml
[project]
name = "web_service"
version = "0.1.0"

[rust.dependencies]
axum = "0.7"
tokio = { version = "1", features = ["full"] }
sqlx = { version = "0.7", features = ["runtime-tokio", "postgres"] }
tracing = "0.1"
```

**src/main.incn:**

```incan
# No version needed - defined in incan.toml
from rust::axum import Router, Json
from rust::sqlx import PgPool
import rust::tracing

async def main() -> None:
    tracing::info("Starting server...")
    app = Router::new()
    # ...
```

### 7.3 Mixed Inline and Config

**incan.toml:**

```toml
[project]
name = "mixed_example"
version = "0.1.0"

[rust.dependencies]
serde = { version = "1.0", features = ["derive"] }
```

**src/main.incn:**

```incan
# From incan.toml
import rust::serde

# Inline - not in incan.toml
import rust::uuid @ "1.0" with ["v4"]

# Override known-good default for one-off use
from rust::chrono @ "0.4" with ["serde", "clock"] import DateTime, Utc
```

---

## 8. Comparison with pyproject.toml

| pyproject.toml                    | incan.toml                     | Notes                             |
|-----------------------------------|--------------------------------|-----------------------------------|
| `[project]`                       | `[project]`                    | Same structure                    |
| `dependencies = [...]`            | `[rust.dependencies]`          | Namespaced for Rust               |
| `[project.optional-dependencies]` | `[rust.dependencies.optional]` | Similar                           |
| `[tool.pytest]`                   | N/A                            | Tool-specific config not in scope |
| `requires-python`                 | `requires-incan`               | Minimum version                   |

---

## 9. Open Questions

1. **Workspace support**: Should `incan.toml` support workspaces with multiple packages?
2. **Private registries**: How to authenticate with private Cargo registries?
3. **Version ranges in toml**: Allow `tokio = ">=1.30, <2.0"` or require Cargo syntax?
4. **Auto-update policy**: Should `incan update` respect semver or allow major updates?

---

## 10. Checklist

### Implementing Phase 1: Inline Versions

- [ ] Parser: `@ "version"` syntax
- [ ] Codegen: pass version to ProjectGenerator
- [ ] Error: unknown crate without version (warning + `*` fallback)
- [ ] Docs: update rust_interop.md

### Implementing Phase 2: Features

- [ ] Parser: `with ["features"]` syntax
- [ ] Codegen: emit features in Cargo.toml
- [ ] Error: unknown feature

### Implementing Phase 3: Project Configuration

- [ ] Parser: `incan.toml` format
- [ ] CLI: `incan init` command
- [ ] Resolution: merge inline + toml + defaults
- [ ] Error: version conflicts

### Implementing Phase 4: Lock File

- [ ] Generate `incan.lock`
- [ ] CLI: `incan lock` command
- [ ] CLI: `incan update` command
- [ ] Build: use lock file when present

### Implementing Phase 5: Advanced Sources

- [ ] Git dependencies
- [ ] Path dependencies
- [ ] Optional dependencies
- [ ] Dev dependencies
