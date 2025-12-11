# Serialization Derives

This page covers `Serialize` and `Deserialize` for JSON support.

## Overview

Incan provides built-in JSON serialization through two derives:

| Derive | Function | Direction |
|--------|----------|-----------|
| `Serialize` | `json_stringify(value)` | Incan → JSON string |
| `Deserialize` | `json_parse[T](string)` | JSON string → Incan |

**Python equivalent**: `json.dumps()` and `json.loads()`

**Rust generated**: `#[derive(serde::Serialize)]` and `#[derive(serde::Deserialize)]`

---

## Serialize

**What it does**: Enables converting your type to a JSON string.

### Basic Serialize Usage

```incan
@derive(Debug, Serialize)
model User:
    name: str
    age: int
    active: bool

def main() -> None:
    user = User(name="Alice", age=30, active=true)
    json_str = json_stringify(user)
    println(json_str)
    # {"name":"Alice","age":30,"active":true}
```

### Nested Types

Serialize works with nested structures (all types must have `Serialize`):

```incan
@derive(Debug, Serialize)
model Address:
    city: str
    zip: str

@derive(Debug, Serialize)
model Person:
    name: str
    address: Address

def main() -> None:
    person = Person(
        name = "Alice",
        address = Address(city="NYC", zip="10001")
    )
    json_str = json_stringify(person)
    println(json_str)
    # {"name":"Alice","address":{"city":"NYC","zip":"10001"}}
```

### Collections

Lists, Dicts, and Options serialize automatically:

```incan
@derive(Debug, Serialize)
model Team:
    name: str
    members: List[str]
    metadata: Dict[str, str]

def main() -> None:
    team = Team(
        name = "Engineering",
        members = ["Alice", "Bob", "Charlie"],
        metadata = {"location": "NYC", "floor": "3"}
    )
    println(json_stringify(team))
    # {"name":"Engineering","members":["Alice","Bob","Charlie"],"metadata":{"location":"NYC","floor":"3"}}
```

---

## Deserialize

**What it does**: Enables parsing your type from a JSON string.

### Basic Deserialize Usage

```incan
@derive(Debug, Deserialize)
model User:
    name: str
    age: int

def main() -> None:
    json_str = "{\"name\": \"Alice\", \"age\": 30}"
    result: Result[User, str] = json_parse(json_str)
    
    match result:
        case Ok(user):
            println(f"Loaded: {user.name}, age {user.age}")
        case Err(e):
            println(f"Parse error: {e}")
```

### Error Handling

`json_parse` returns a `Result` because parsing can fail:

```incan
@derive(Debug, Deserialize)
model Config:
    port: int
    host: str

def main() -> None:
    # Invalid JSON
    bad_json = "not valid json"
    result = json_parse[Config](bad_json)
    
    match result:
        case Ok(config):
            println(f"Config: {config.host}:{config.port}")
        case Err(error):
            println(f"Failed to parse: {error}")
```

### Missing Fields

By default, all fields must be present. Use `Default` for optional fields:

```incan
@derive(Debug, Deserialize, Default)
model Settings:
    theme: str = "light"
    font_size: int = 12

def main() -> None:
    # JSON missing "font_size"
    json_str = "{\"theme\": \"dark\"}"
    result = json_parse[Settings](json_str)
    
    match result:
        case Ok(settings):
            println(f"Theme: {settings.theme}")  # dark
            println(f"Font: {settings.font_size}")  # 12 (default)
        case Err(e):
            println(f"Error: {e}")
```

---

## Both Together

Most types need both `Serialize` and `Deserialize`:

```incan
@derive(Debug, Serialize, Deserialize)
model ApiResponse:
    status: int
    message: str
    data: str = ""

def main() -> None:
    # Create and serialize
    response = ApiResponse(status=200, message="OK", data="Hello")
    json_out = json_stringify(response)
    println(f"Sent: {json_out}")

    # Receive and deserialize
    json_in = "{\"status\": 404, \"message\": \"Not Found\", \"data\": \"\"}"
    result = json_parse[ApiResponse](json_in)
    
    match result:
        case Ok(resp):
            println(f"Received: {resp.status} {resp.message}")
        case Err(e):
            println(f"Error: {e}")
```

---

## Type Mappings

How Incan types map to JSON:

| Incan Type | JSON Type | Example |
|------------|-----------|---------|
| `str` | string | `"hello"` |
| `int` | number | `42` |
| `float` | number | `3.14` |
| `bool` | boolean | `true` |
| `List[T]` | array | `[1, 2, 3]` |
| `Dict[str, T]` | object | `{"key": "value"}` |
| `Option[T]` | value or `null` | `null` |
| `model`/`class` | object | `{"field": "value"}` |

---

## Common Patterns

### API Client

```incan
@derive(Debug, Serialize)
model CreateUserRequest:
    name: str
    email: str

@derive(Debug, Deserialize)
model CreateUserResponse:
    id: int
    name: str
    created_at: str

def create_user(name: str, email: str) -> Result[CreateUserResponse, str]:
    request = CreateUserRequest(name=name, email=email)
    request_json = json_stringify(request)
    
    # ... send HTTP request, get response_json ...
    response_json = "{\"id\": 123, \"name\": \"Alice\", \"created_at\": \"2024-01-01\"}"
    
    return json_parse[CreateUserResponse](response_json)
```

### Configuration Files

```incan
@derive(Debug, Serialize, Deserialize, Default)
model AppConfig:
    database_url: str = "localhost:5432"
    max_connections: int = 10
    debug: bool = false

def load_config(json_str: str) -> AppConfig:
    result = json_parse[AppConfig](json_str)
    match result:
        case Ok(config):
            return config
        case Err(_):
            return AppConfig()  # Fall back to defaults
```

---

## Quick Reference

| Function | Input | Output |
|----------|-------|--------|
| `json_stringify(value)` | Any `Serialize` type | `str` |
| `json_parse[T](string)` | `str` | `Result[T, str]` |

## See Also

- [Derives Overview](./derives_and_traits.md) - Complete reference
- [Default Derive](./copying_default.md#default) - For optional fields
