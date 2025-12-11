# Enums in Incan

Incan enums are **algebraic data types** (ADTs) — a powerful concept from functional programming that Python lacks. They let you define types with a fixed set of variants, where each variant can carry different data.

## Why Enums? (Python Comparison)

Python has `Enum` but it's limited — it's essentially just a class with named constants:

```python
# Python Enum - just named integers/strings
from enum import Enum

class Color(Enum):
    RED = 1
    GREEN = 2
    BLUE = 3

# Can't attach different data to each variant
# Can't do exhaustive pattern matching
```

For complex cases, Python developers resort to:

```python
# Python workaround - classes with isinstance checks
class Shape:
    pass

class Circle(Shape):
    def __init__(self, radius: float):
        self.radius = radius

class Rectangle(Shape):
    def __init__(self, width: float, height: float):
        self.width = width
        self.height = height

# No compiler help - easy to forget a case
def area(shape: Shape) -> float:
    if isinstance(shape, Circle):
        return 3.14159 * shape.radius ** 2
    elif isinstance(shape, Rectangle):
        return shape.width * shape.height
    # Forgot Triangle? Python won't tell you!
```

Incan enums solve all of this:

```incan
enum Shape:
    Circle(float)                    # Variant with data
    Rectangle(float, float)
    Triangle(float, float, float)

def area(shape: Shape) -> float:
    match shape:
        Circle(r) => return 3.14159 * r * r
        Rectangle(w, h) => return w * h
        Triangle(a, b, c) =>
            # Heron's formula
            s = (a + b + c) / 2
            return sqrt(s * (s - a) * (s - b) * (s - c))
        # Compiler errors if you miss a variant!
```

---

## Basic Syntax

### Simple Enum (No Data)

```incan
enum Status:
    Pending
    Active
    Completed
    Cancelled
```

Usage:

```incan
status = Status.Active

match status:
    Pending => println("Waiting...")
    Active => println("In progress")
    Completed => println("Done!")
    Cancelled => println("Aborted")
```

### Enum with Data (Variants)

Each variant can carry different types and amounts of data:

```incan
enum Message:
    Quit                           # No data
    Move(int, int)                 # Two ints (x, y)
    Write(str)                     # A string
    ChangeColor(int, int, int)     # RGB values
```

Usage:

```incan
msg = Message.Move(10, 20)

match msg:
    Quit => println("Goodbye")
    Move(x, y) => println(f"Moving to ({x}, {y})")
    Write(text) => println(f"Message: {text}")
    ChangeColor(r, g, b) => println(f"RGB({r}, {g}, {b})")
```

---

## Generic Enums

Enums can be generic — parameterized over types:

```incan
enum Option[T]:
    Some(T)
    None

enum Result[T, E]:
    Ok(T)
    Err(E)
```

> Note: These are Incan's built-in types for handling optional values and errors.

### Custom Generic Enum

```incan
enum Tree[T]:
    Leaf(T)
    Node(Tree[T], Tree[T])

# A binary tree of integers
tree = Node(
    Leaf(1),
    Node(Leaf(2), Leaf(3))
)
```

---

## Pattern Matching

The `match` expression is how you work with enums. It's exhaustive — the compiler ensures you handle all variants.

### Basic Match

```incan
enum Direction:
    North
    South
    East
    West

def describe(dir: Direction) -> str:
    match dir:
        North => return "Going up"
        South => return "Going down"
        East => return "Going right"
        West => return "Going left"
```

### Extracting Data

```incan
enum ApiResponse:
    Success(str, int)        # (data, status_code)
    Error(str)               # error message
    Loading

def handle(response: ApiResponse) -> None:
    match response:
        Success(data, code) =>
            println(f"Got {code}: {data}")
        Error(msg) =>
            println(f"Failed: {msg}")
        Loading =>
            println("Please wait...")
```

### Wildcard Pattern

Use `_` to match any remaining variants:

```incan
match status:
    Active => println("Working on it")
    _ => println("Not active")  # Matches Pending, Completed, Cancelled
```

> **Warning**: Wildcards can hide bugs when you add new variants. Prefer explicit matches.

### Guards

Add conditions to patterns:

```incan
enum Temperature:
    Celsius(float)
    Fahrenheit(float)

def describe(temp: Temperature) -> str:
    match temp:
        Celsius(c) if c > 30 => return "Hot (Celsius)"
        Celsius(c) if c < 10 => return "Cold (Celsius)"
        Celsius(_) => return "Moderate (Celsius)"
        Fahrenheit(f) if f > 86 => return "Hot (Fahrenheit)"
        Fahrenheit(f) if f < 50 => return "Cold (Fahrenheit)"
        Fahrenheit(_) => return "Moderate (Fahrenheit)"
```

---

## Common Patterns

### Pattern 1: State Machines

Enums excel at modeling states:

```incan
enum ConnectionState:
    Disconnected
    Connecting(str)          # URL being connected to
    Connected(Connection)
    Error(str)

def handle_state(state: ConnectionState) -> ConnectionState:
    match state:
        Disconnected =>
            return ConnectionState.Connecting("https://api.example.com")
        Connecting(url) =>
            match try_connect(url):
                Ok(conn) => return ConnectionState.Connected(conn)
                Err(e) => return ConnectionState.Error(e)
        Connected(conn) =>
            # Stay connected
            return state
        Error(msg) =>
            println(f"Error: {msg}")
            return ConnectionState.Disconnected
```

### Pattern 2: Command/Action Types

```incan
enum Command:
    Create(str, str)         # (name, content)
    Update(int, str)         # (id, new_content)
    Delete(int)              # id
    List

def execute(cmd: Command) -> Result[str, str]:
    match cmd:
        Create(name, content) =>
            return create_item(name, content)
        Update(id, content) =>
            return update_item(id, content)
        Delete(id) =>
            return delete_item(id)
        List =>
            return Ok(list_items())
```

### Pattern 3: Error Hierarchies

```incan
enum DatabaseError:
    ConnectionFailed(str)
    QueryFailed(str, int)    # (query, error_code)
    NotFound(str)            # table/record name
    PermissionDenied

enum AppError:
    Database(DatabaseError)  # Nested enum!
    Validation(str)
    NotAuthenticated

def handle_error(err: AppError) -> str:
    match err:
        Database(db_err) =>
            match db_err:
                ConnectionFailed(host) => return f"Can't reach {host}"
                QueryFailed(q, code) => return f"Query error {code}: {q}"
                NotFound(name) => return f"Not found: {name}"
                PermissionDenied => return "Access denied"
        Validation(msg) => return f"Invalid: {msg}"
        NotAuthenticated => return "Please log in"
```

### Pattern 4: Expression Trees

```incan
enum Expr:
    Number(int)
    Add(Expr, Expr)
    Mul(Expr, Expr)
    Neg(Expr)

def eval(expr: Expr) -> int:
    match expr:
        Number(n) => return n
        Add(a, b) => return eval(a) + eval(b)
        Mul(a, b) => return eval(a) * eval(b)
        Neg(e) => return -eval(e)

# (3 + 4) * -2 = -14
expr = Mul(Add(Number(3), Number(4)), Neg(Number(2)))
result = eval(expr)  # -14
```

---

## Enums vs Models vs Classes

| Use Case | Enum | Model | Class |
|----------|------|-------|-------|
| Fixed set of variants | ✓ | | |
| Data that can be one of several shapes | ✓ | | |
| Exhaustive handling required | ✓ | | |
| Simple data container (DTO, config) | | ✓ | |
| Serialization focus (`@derive`) | | ✓ | |
| Validation and defaults | | ✓ | |
| Inheritance/polymorphism needed | | | ✓ |
| Mutable state with methods | | | ✓ |
| Open extension (new types later) | | | ✓ |

```incan
# Enum: closed set, exhaustive matching
enum PaymentMethod:
    CreditCard(str, str)     # number, expiry
    PayPal(str)              # email
    BankTransfer(str, str)   # account, routing

# Model: data-first, serialization
@derive(Serialize, Deserialize)
model PaymentRequest:
    method: PaymentMethod
    amount: float
    currency: str = "USD"

# Class: behavior-first, inheritance
class PaymentProcessor:
    def process(self, amount: float) -> Result[Receipt, Error]:
        ...
```

See also: [Models and Classes Guide](./models_and_classes.md)

---

## Built-in Enums

Incan provides these enums in the standard library:

### Option[T]

Represents an optional value:

```incan
enum Option[T]:
    Some(T)
    None
```

See: [Error Handling Guide](./error_handling.md)

### Result[T, E]

Represents success or failure:

```incan
enum Result[T, E]:
    Ok(T)
    Err(E)
```

See: [Error Handling Guide](./error_handling.md)

### Ordering

Comparison result:

```incan
enum Ordering:
    Less
    Equal
    Greater
```

---

## Comparison: Python vs Incan

| Feature | Python | Incan |
|---------|--------|-------|
| Basic enum | `Enum` class | `enum` keyword |
| Data in variants | Not supported | ✓ Full support |
| Generic enums | Not supported | ✓ `Option[T]` |
| Exhaustive matching | No | ✓ Compiler enforced |
| Pattern matching | `match` (3.10+), limited | ✓ Full destructuring |
| Type safety | Runtime only | Compile-time |

### Translating Python Patterns

**Python class hierarchy → Incan enum:**

```python
# Python
class Event:
    pass

class Click(Event):
    def __init__(self, x: int, y: int):
        self.x, self.y = x, y

class KeyPress(Event):
    def __init__(self, key: str):
        self.key = key
```

```incan
# Incan
enum Event:
    Click(int, int)
    KeyPress(str)
```

**Python union types → Incan enum:**

```python
# Python 3.10+
def process(value: int | str | None) -> str:
    match value:
        case int(n): return f"Number: {n}"
        case str(s): return f"String: {s}"
        case None: return "Nothing"
```

```incan
# Incan - more explicit
enum Value:
    Number(int)
    Text(str)
    Empty

def process(value: Value) -> str:
    match value:
        Number(n) => return f"Number: {n}"
        Text(s) => return f"String: {s}"
        Empty => return "Nothing"
```

---

## Summary

| Concept | Description |
|---------|-------------|
| `enum` | Define a type with fixed variants |
| Variants | Each case of an enum, optionally with data |
| Generic enum | Enum parameterized over types: `Option[T]` |
| `match` | Exhaustive pattern matching on enums |
| Destructuring | Extract data from variants: `Some(x) =>` |

Enums are one of Incan's most powerful features — use them for:

- Modeling states and state machines
- Error types with rich context
- Command/message types
- Any "one of these things" scenario

The compiler guarantees you handle all cases, eliminating a whole class of bugs that plague Python code.

---

## See Also

- [Error Handling](./error_handling.md) — Using `Result` and `Option`
- [Pattern Matching RFC](../RFCs/000_core_rfc.md) — Match expression grammar
- [Models and Classes](./models_and_classes.md) — When to use class vs enum
