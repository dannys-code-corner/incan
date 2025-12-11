# Incan Language Guide

Learn how to write Incan code — from basics to advanced patterns.

## Core Concepts

| Guide | Description |
|-------|-------------|
| [Error Handling](error_handling.md) | Result, Option, and the `?` operator |
| [Error Messages](error_messages.md) | Understanding and fixing compiler errors |
| [Imports & Modules](imports_and_modules.md) | Module system and built-in functions |

## Features

| Guide | Description |
|-------|-------------|
| [Derives & Traits](derives_and_traits.md) | Derive macros and trait system |
| [Async Programming](async_programming.md) | Async/await with Tokio |
| [Rust Interop](rust_interop.md) | Using Rust crates from Incan |
| [Web Framework](web_framework.md) | Building web apps with Axum |

## Derives Reference

Detailed documentation for each derive:

| Guide | Derives |
|-------|---------|
| [String Representation](derives/string_representation.md) | `Debug`, `Display` |
| [Comparison](derives/comparison.md) | `Eq`, `Ord`, `Hash` |
| [Copying & Default](derives/copying_default.md) | `Clone`, `Copy`, `Default` |
| [Serialization](derives/serialization.md) | `Serialize`, `Deserialize` |
| [Custom Behavior](derives/custom_behavior.md) | Overriding derived behavior |

## Quick Examples

### Error Handling

```incan
def read_config(path: str) -> Result[Config, IoError]:
    let content = read_file(path)?  # Propagate errors with ?
    let config = parse_json(content)?
    return Ok(config)
```

### Async/Await

```incan
async def fetch_users() -> Result[List[User], HttpError]:
    let response = await http_get("/api/users")
    return response.json()
```

### Derives

```incan
@derive(Debug, Eq, Clone, Serialize)
model User:
    id: int
    name: str
    email: str
```

## See Also

- [Getting Started](../tooling/getting_started.md) - Installation and setup
- [Examples](../../examples/) - Sample programs
- [RFCs](../RFCs/) - Design proposals
