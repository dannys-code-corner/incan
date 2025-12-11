# Custom Behavior: Dunders, Traits, and Reflection

This page covers advanced customization: overriding derived behavior with dunder methods, using traits with defaults, and accessing reflection information.

## Dunder Method Overrides

When auto-generated derives don't fit your needs, use **dunder methods** (Python-style `__method__` names) to provide custom implementations.

### Available Dunders

| Dunder | Overrides Derive | Format | Purpose |
|--------|------------------|--------|---------|
| `__eq__` | `Eq` | — | Custom equality comparison |
| `__hash__` | `Hash` | — | Custom hashing |
| `__lt__` | `Ord` | — | Custom ordering |
| `__str__` | `Display` | `{value}` | Custom string output |

> **Note**: `Debug` (`{value:?}` format) is always auto-generated and cannot be overridden with `__repr__`. Use `__str__` for custom string formatting.

### Custom Equality (`__eq__`)

Compare only specific fields instead of all fields:

```incan
@derive(Debug, Clone)
model User:
    id: int
    name: str
    cache_key: str  # Internal field - shouldn't affect equality

    def __eq__(self, other: User) -> bool:
        return self.id == other.id  # Only compare IDs

def main() -> None:
    user1 = User(id=1, name="Alice", cache_key="abc123")
    user2 = User(id=1, name="Alice", cache_key="xyz789")

    println(f"{user1 == user2}")  # true - same ID
```

### Custom Hash (`__hash__`)

**Important**: If you define `__eq__`, you must define matching `__hash__`:

```incan
@derive(Debug, Clone)
model Document:
    id: int
    content: str
    version: int  # Shouldn't affect equality/hash

    def __eq__(self, other: Document) -> bool:
        return self.id == other.id

    def __hash__(self) -> int:
        return hash(self.id)  # Must match __eq__

def main() -> None:
    docs: Set[Document] = set()
    docs.add(Document(id=1, content="Hello", version=1))
    docs.add(Document(id=1, content="Updated", version=2))  # Same ID = duplicate

    println(f"Unique docs: {len(docs)}")  # 1
```

### Custom Ordering (`__lt__`)

Control sort order:

```incan
@derive(Debug, Clone, Eq)
model Task:
    priority: int  # 1 = highest
    name: str

    def __lt__(self, other: Task) -> bool:
        # Lower priority number = higher priority = comes first
        return self.priority < other.priority

def main() -> None:
    tasks = [
        Task(priority=3, name="Low priority"),
        Task(priority=1, name="High priority"),
        Task(priority=2, name="Medium priority")
    ]

    for task in sorted(tasks):
        println(f"{task.priority}: {task.name}")
    # 1: High priority
    # 2: Medium priority
    # 3: Low priority
```

### Custom Display (`__str__`)

Human-readable output:

```incan
@derive(Debug, Clone)
model Money:
    cents: int
    currency: str

    def __str__(self) -> str:
        dollars = self.cents / 100
        return f"{self.currency}{dollars:.2f}"

def main() -> None:
    price = Money(cents=1999, currency="$")
    println(f"Price: {price}")     # Price: $19.99
    println(f"Debug: {price:?}")   # Debug: Money { cents: 1999, currency: "$" }
```

---

## Traits with Default Implementations

Define reusable behaviors that types get automatically.

### Defining a Trait

```incan
trait Describable:
    def describe(self) -> str:
        return "An object"  # Default implementation
```

### Using a Trait

Use `with TraitName` to get the default behavior:

```incan
trait Describable:
    def describe(self) -> str:
        return "An object"

class Product with Describable:
    name: str
    price: float

def main() -> None:
    product = Product(name="Laptop", price=999.99)
    println(product.describe())  # "An object"
```

### Overriding Trait Methods

Provide your own implementation when the default doesn't fit:

```incan
trait Describable:
    def describe(self) -> str:
        return "An object"

class Book with Describable:
    title: str
    author: str

    def describe(self) -> str:
        return f"'{self.title}' by {self.author}"

def main() -> None:
    book = Book(title="1984", author="Orwell")
    println(book.describe())  # "'1984' by Orwell"
```

### Multiple Traits

A class can use multiple traits:

```incan
trait Named:
    def get_name(self) -> str:
        return "Unknown"

trait Printable:
    def print_info(self) -> None:
        println("Printable object")

class Item with Named, Printable:
    id: int

    def get_name(self) -> str:
        return f"Item-{self.id}"

def main() -> None:
    item = Item(id=42)
    println(item.get_name())  # "Item-42"
    item.print_info()         # "Printable object"
```

---

## Reflection Methods

All models and classes automatically get **reflection methods** for introspection.

### `__fields__()`

Returns a list of field names:

```incan
@derive(Debug)
model Config:
    host: str
    port: int
    debug: bool

def main() -> None:
    config = Config(host="localhost", port=8080, debug=true)
    
    fields = config.__fields__()
    println(f"Fields: {fields}")  # ["host", "port", "debug"]
    
    for field in fields:
        println(f"  - {field}")
```

### `__class_name__()`

Returns the type name as a string:

```incan
@derive(Debug)
model User:
    name: str

@derive(Debug)
model Admin:
    name: str
    level: int

def log_type(obj: any) -> None:
    println(f"Type: {obj.__class_name__()}")

def main() -> None:
    user = User(name="Alice")
    admin = Admin(name="Bob", level=5)
    
    log_type(user)   # Type: User
    log_type(admin)  # Type: Admin
```

### Use Cases for Reflection

**Debugging and logging**:

```incan
@derive(Debug)
model Request:
    method: str
    path: str
    body: str

def log_request(req: Request) -> None:
    println(f"[{req.__class_name__()}]")
    for field in req.__fields__():
        println(f"  {field}")

def main() -> None:
    req = Request(method="GET", path="/api", body="")
    log_request(req)
    # [Request]
    #   method
    #   path
    #   body
```

**Generic utilities**:

```incan
def count_fields(obj: any) -> int:
    return len(obj.__fields__())

def main() -> None:
    # Works with any model/class
    println(f"User has {count_fields(User(name='x'))} fields")
```

---

## Quick Reference

### Dunder Methods

| Method | Signature | Overrides |
|--------|-----------|-----------|
| `__eq__` | `(self, other: T) -> bool` | `Eq` |
| `__hash__` | `(self) -> int` | `Hash` |
| `__lt__` | `(self, other: T) -> bool` | `Ord` |
| `__str__` | `(self) -> str` | `Display` |

### Reflection Methods (Automatic)

| Method | Returns | Description |
|--------|---------|-------------|
| `__fields__()` | `List[str]` | Field names |
| `__class_name__()` | `str` | Type name |

## See Also

- [Derives Overview](./derives_and_traits.md) - Complete reference
- [Comparison Derives](./comparison.md) - `Eq`, `Ord`, `Hash` details
- [String Representation](./string_representation.md) - `Debug`, `Display` details
