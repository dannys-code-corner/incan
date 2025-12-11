# Error Handling in Incan

Incan takes a **Rust-like approach** to error handling, not Python's exception model. This is a deliberate design choice that makes error handling explicit, type-safe, and predictable.

## Why Not Exceptions?

Python uses exceptions for error handling:

```python
# Python - exceptions can come from anywhere
def process_user(user_id: int) -> User:
    user = fetch_user(user_id)      # May raise NetworkError
    validated = validate(user)       # May raise ValidationError
    return save(validated)           # May raise DatabaseError
```

Problems with exceptions:

- **Hidden control flow**: Any function can throw anything
- **No type information**: Can't know what errors a function may raise
- **Easy to forget**: Unhandled exceptions crash at runtime
- **Performance cost**: Exception unwinding is expensive

Incan uses **explicit Result types** instead:

```incan
# Incan - errors are visible in the type signature
def process_user(user_id: int) -> Result[User, ProcessError]:
    user = fetch_user(user_id)?       # Returns Result[User, NetworkError]
    validated = validate(user)?        # Returns Result[User, ValidationError]
    return save(validated)             # Returns Result[User, DatabaseError]
```

Benefits:

- **Explicit**: Error paths visible in function signatures
- **Type-safe**: Compiler ensures errors are handled
- **Predictable**: No surprise exceptions
- **Efficient**: No stack unwinding overhead

---

## Core Types

### Result[T, E]

`Result` represents an operation that can succeed (`Ok`) or fail (`Err`):

```incan
enum Result[T, E]:
    Ok(T)      # Success with value
    Err(E)     # Failure with error
```

Usage:

```incan
def divide(a: int, b: int) -> Result[int, str]:
    if b == 0:
        return Err("division by zero")
    return Ok(a / b)

def main() -> None:
    match divide(10, 2):
        case Ok(value): println(f"Result: {value}")   # Result: 5
        case Err(msg): println(f"Error: {msg}")
```

### Option[T]

`Option` represents a value that may or may not exist:

```incan
enum Option[T]:
    Some(T)    # Value present
    None       # No value
```

Usage:

```incan
def find_user(id: int) -> Option[User]:
    if id in users:
        return Some(users[id])
    return None

def main() -> None:
    match find_user(42):
        case Some(user): println(f"Found: {user.name}")
        case None: println("User not found")
```

---

## The `?` Operator

The `?` operator provides concise error propagation. It's the Incan equivalent of Python's implicit exception bubbling, but explicit and type-checked.

### How It Works

```incan
# Without ? - verbose
def process() -> Result[Data, Error]:
    match fetch_data():
        case Ok(data):
            match validate(data):
                case Ok(valid): return Ok(valid)
                case Err(e): return Err(e)
        case Err(e): return Err(e)

# With ? - concise
def process() -> Result[Data, Error]:
    data = fetch_data()?      # If Err, return early
    valid = validate(data)?   # If Err, return early
    return Ok(valid)
```

### Rules for `?`

1. **Must return Result**: Functions using `?` must return `Result[T, E]`
2. **Error types must match**: The error in `?` must be compatible with the return error
3. **Compile-time checked**: The compiler ensures proper error handling

```incan
# This won't compile - main returns None, not Result
def main() -> None:
    data = fetch_data()?  # Error: can't use ? in non-Result function
```

### Python Comparison

| Python | Incan |
|--------|-------|
| `try/except` wraps code | `match` on Result |
| Exceptions propagate implicitly | `?` propagates explicitly |
| `raise` throws exception | `return Err(e)` |
| Runtime error if unhandled | Compile error if unhandled |

```python
# Python - implicit propagation
def process(path: str) -> str:
    content = read_file(path)  # May raise IOError
    return parse(content)       # May raise ParseError
```

```incan
# Incan - explicit propagation
def process(path: str) -> Result[str, ProcessError]:
    content = read_file(path)?   # Propagate if Err
    return parse(content)         # Also returns Result
```

---

## The Error Trait

For custom error types, implement the `Error` trait:

```incan
trait Error:
    def message(self) -> str:
        """Return a human-readable error message"""
        ...
    
    def source(self) -> Option[str]:
        """Optional: Return the underlying cause"""
        return None
```

### Creating Custom Errors

**Simple error with enum:**

```incan
enum MathError:
    DivisionByZero
    Overflow
    InvalidInput(str)

def divide(a: int, b: int) -> Result[int, MathError]:
    if b == 0:
        return Err(MathError.DivisionByZero)
    return Ok(a / b)
```

**Rich error with Error trait:**

```incan
model ValidationError with Error:
    field: str
    message: str
    
    def message(self) -> str:
        return f"Validation failed for '{self.field}': {self.message}"

def validate_age(age: int) -> Result[int, ValidationError]:
    if age < 0:
        return Err(ValidationError(
            field="age",
            message="cannot be negative"
        ))
    return Ok(age)
```

### Error Chaining

Use `source()` to chain errors:

```incan
model DatabaseError with Error:
    query: str
    cause: Option[str]
    
    def message(self) -> str:
        return f"Database query failed: {self.query}"
    
    def source(self) -> Option[str]:
        return self.cause
```

---

## Common Patterns

### Pattern 1: Match for Control Flow

```incan
def handle_result() -> None:
    match fetch_user(42):
        case Ok(user):
            println(f"Found: {user.name}")
            process_user(user)
        case Err(e):
            println(f"Error: {e.message()}")
            log_error(e)
```

### Pattern 2: Propagate with `?`

```incan
def process_all() -> Result[Summary, ProcessError]:
    users = fetch_users()?
    validated = validate_all(users)?
    saved = save_all(validated)?
    return Ok(Summary(count=len(saved)))
```

### Pattern 3: Transform Errors with map_err

```incan
def load_config(path: str) -> Result[Config, AppError]:
    # Convert IoError to AppError
    content = read_file(path).map_err(|e| AppError.Io(e))?
    # Convert ParseError to AppError
    config = parse_config(content).map_err(|e| AppError.Parse(e))?
    return Ok(config)
```

### Pattern 4: Default Values with unwrap_or

```incan
def get_setting(key: str) -> str:
    # Return default if None
    return settings.get(key).unwrap_or("default_value")
```

### Pattern 5: Option to Result

```incan
def require_user(id: int) -> Result[User, str]:
    # Convert Option to Result
    return find_user(id).ok_or("user not found")
```

---

## Unwrap and Panic

### What is `unwrap()`?

`unwrap()` extracts the value from `Option` or `Result`, but **panics if there's no value**:

```incan
# Option
value = Some(42)
x = value.unwrap()     # x = 42 ✓

empty = None
y = empty.unwrap()     # PANIC! Program crashes ✗
```

```incan
# Result
ok = Ok(42)
x = ok.unwrap()        # x = 42 ✓

err = Err("oops")
y = err.unwrap()       # PANIC! Program crashes ✗
```

Think of `Option`/`Result` as a **wrapped gift box**:

- `Some(42)` or `Ok(42)` = box with a value inside
- `None` or `Err(e)` = empty box (or box with error)
- `unwrap()` = "rip open the box, give me what's inside, or crash if empty"

### What is Panic?

A **panic** is an unrecoverable error that crashes the program. Unlike `Result` errors which can be handled, panics are for situations that should never happen:

```incan
# Panics are for programmer errors, not user errors
def get_first(items: List[T]) -> T:
    if len(items) == 0:
        panic("get_first called on empty list")  # Bug in caller's code
    return items[0]
```

### Safe Alternatives to `unwrap()`

| Method | What It Does | Use When |
|--------|--------------|----------|
| `unwrap()` | Extract or panic | You're 100% sure it's Some/Ok |
| `unwrap_or(default)` | Extract or use default | You have a fallback value |
| `unwrap_or_else(fn)` | Extract or compute default | Default is expensive to create |
| `expect("msg")` | Extract or panic with message | Debugging, clearer panic message |
| `match` | Handle both cases | You need to handle the error |

```incan
# Instead of unwrap(), prefer:

# unwrap_or - provide a default
name = user.name.unwrap_or("Anonymous")

# unwrap_or_else - compute default lazily  
config = load_config().unwrap_or_else(|| Config.default())

# expect - better panic message for debugging
user = users.get(id).expect(f"User {id} must exist")

# match - handle the error properly
match fetch_data():
    case Ok(data): process(data)
    case Err(e): log_error(e)
```

### When to Use `unwrap()`

Use `unwrap()` only when:

1. **You've already checked**: `if value.is_some(): value.unwrap()`
2. **It's truly impossible to fail**: Constants, compile-time known values
3. **In tests**: Quick assertions where panic is acceptable
4. **Prototyping**: Temporary code you'll fix later

```incan
# OK - we just checked
if result.is_ok():
    data = result.unwrap()

# OK - in a test
def test_parse():
    result = parse("42").unwrap()  # Panic = test failure
    assert(result == 42)

# BAD - user input can fail
def handle_input(s: str) -> int:
    return parse(s).unwrap()  # Don't do this!
```

---

## Structured Error Types

Prefer structured errors over strings:

```incan
# Bad - stringly typed errors
def process() -> Result[Data, str]:
    return Err("something went wrong")  # Hard to handle programmatically

# Good - structured errors
enum ProcessError:
    NetworkFailure(url: str, status: int)
    ValidationFailed(field: str, reason: str)
    NotFound(resource: str)
    
def process() -> Result[Data, ProcessError]:
    return Err(ProcessError.NetworkFailure(
        url="https://api.example.com",
        status=503
    ))
```

Structured errors enable:

- **Pattern matching**: Handle specific cases
- **Type safety**: Compiler catches typos
- **Rich context**: Include relevant data
- **Programmatic handling**: Not just for humans

### Errors That Carry Recoverable Data

Some errors should return data that would otherwise be lost. For example, `SendError[T]` from channels contains the value that failed to send:

```incan
model SendError[T] with Error:
    """Error when sending on a closed channel"""
    value: T
    
    def message(self) -> str:
        return "channel closed: receiver dropped"
```

Usage - access the `.value` field directly:

```incan
match await tx.send(important_data):
    case Ok(_): println("Sent!")
    case Err(e):
        # Data isn't lost - recover it via .value field
        println(f"Failed to send: {e.value:?}")
        save_for_retry(e.value)
```

This pattern is useful when:

- The operation consumes a value (like channel send)
- The caller might want to retry or handle the data differently
- Losing the data silently would be problematic

---

## Best Practices

### 1. Be Explicit About Failure Modes

```incan
# Document what can go wrong
def connect(host: str, port: int) -> Result[Connection, ConnectionError]:
    """
    Connect to a remote host.
    
    Errors:
    - ConnectionError.Refused: Host rejected connection
    - ConnectionError.Timeout: Connection timed out
    - ConnectionError.DnsFailure: Could not resolve hostname
    """
    ...
```

### 2. Use Specific Error Types

```incan
# Bad - generic error type
def process() -> Result[Data, str]: ...

# Good - domain-specific error enum
def process() -> Result[Data, ProcessError]: ...
```

### 3. Handle Errors at Appropriate Levels

```incan
# Low-level: return errors
def read_config_file(path: str) -> Result[str, IoError]:
    return read_file(path)

# Mid-level: transform and propagate
def load_config(path: str) -> Result[Config, ConfigError]:
    content = read_config_file(path).map_err(ConfigError.Io)?
    return parse_config(content)

# High-level: handle and recover
def main() -> None:
    match load_config("app.toml"):
        case Ok(config): start_app(config)
        case Err(e):
            println(f"Failed to load config: {e.message()}")
            println("Using default configuration")
            start_app(Config.default())
```

### 4. Don't Panic for Expected Failures

```incan
# Bad - panics on expected failure
def get_user(id: int) -> User:
    return users[id]  # Panics if not found!

# Good - returns Option for expected absence
def get_user(id: int) -> Option[User]:
    return users.get(id)
```

---

## Comparison: Python vs Incan

| Aspect | Python | Incan |
|--------|--------|-------|
| **Error mechanism** | Exceptions | Result/Option types |
| **Error visibility** | Hidden in implementation | In type signature |
| **Propagation** | Implicit (bubbles up) | Explicit (`?` operator) |
| **Handling** | `try/except` blocks | `match` expressions |
| **Unhandled errors** | Runtime crash | Compile error |
| **Performance** | Stack unwinding | Zero-cost until checked |
| **Multiple errors** | Exception hierarchy | Error enums |

### Translating Python Patterns

**Try/Except → Match:**

```python
# Python
try:
    result = risky_operation()
except SomeError as e:
    handle_error(e)
```

```incan
# Incan
match risky_operation():
    case Ok(result): use_result(result)
    case Err(e): handle_error(e)
```

**Raise → Return Err:**

```python
# Python
def validate(x: int) -> int:
    if x < 0:
        raise ValueError("must be positive")
    return x
```

```incan
# Incan
def validate(x: int) -> Result[int, ValidationError]:
    if x < 0:
        return Err(ValidationError(message="must be positive"))
    return Ok(x)
```

**Bare except → Wildcard match:**

```python
# Python (bad practice)
try:
    result = operation()
except:
    pass  # Swallow all errors
```

```incan
# Incan - at least you're explicit about ignoring
match operation():
    case Ok(result): use_result(result)
    case Err(_): pass  # Explicitly ignoring error
```

---

## Summary

| Concept | What It Does | When to Use |
|---------|--------------|-------------|
| `Result[T, E]` | Success or failure | Operations that can fail |
| `Option[T]` | Value or nothing | Optional values |
| `?` operator | Propagate errors | Chain fallible operations |
| `Error` trait | Custom error types | Rich error information |
| `match` | Handle all cases | Make decisions on Result/Option |

Incan's error handling is:

- **Explicit**: Errors are part of the type signature
- **Type-safe**: Compiler ensures handling
- **Efficient**: No runtime overhead for error path tracking
- **Predictable**: No surprise exceptions

This is a core design principle: **errors should surface as `Result`, not hide**.

---

## See Also

- [Example: Error Handling](../examples/intermediate/error_handling.incn)
- [Error Trait Definition](../stdlib/traits/error.incn)
- [Async Error Handling](./async_programming.md#error-handling)
