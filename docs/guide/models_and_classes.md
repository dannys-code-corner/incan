# Models and Classes in Incan

Incan provides two ways to define types with fields: `model` and `class`. Understanding when to use each is key to writing idiomatic Incan code.

## Quick Comparison

| Aspect | `model` | `class` |
|--------|---------|---------|
| **Purpose** | Data containers | Objects with behavior |
| **Focus** | Fields first | Methods first |
| **Inheritance** | ❌ Cannot inherit | ✅ `extends` supported |
| **Traits** | ✅ `with Trait` | ✅ `with Trait` |
| **Python analogy** | `@dataclass`, Pydantic `BaseModel` | Traditional `class` |
| **Rust analogy** | `struct` (plain) | `struct` + `impl` block |

## When to Use Which

| Use Case | Choose | Why |
|----------|--------|-----|
| Config, settings | `model` | Pure data, no behavior needed |
| DTO, API payload | `model` | Data transfer, serialization focus |
| Database record | `model` | Represents stored data |
| Service with methods | `class` | Has operations/behavior |
| Stateful controller | `class` | Methods that modify state |
| Needs inheritance | `class` | Only `class` supports `extends` |

## Why No `struct` Keyword?

If you're coming from Rust, you might wonder: "Where's `struct`?"

Incan uses `model` instead because:

1. **Python familiarity** — Python developers know `class`, not `struct`
2. **Clearer semantics** — `model` says "this is data", not "this is a memory layout"
3. **Tooling conventions** — ORMs, validators, serializers all use "model" terminology

Under the hood, both `model` and `class` compile to Rust `struct`s.

### Visibility (`pub`)

Control whether declarations are importable from other modules.

- Items are **private by default**, just like in Rust.
- Prefix a declaration with `pub` (publicly visible) to make it importable from other modules.
- For `model` and `class`, `pub` also makes fields public by default.

```incan
model User:           # private by default
    name: str

pub model PublicUser: # public model, fields are public by default
    name: str

class Service:        # private by default
    repo: Repo
    def work(self):
        ...
```

> Note: this is an example of how Incan is more rust-like than Python. In Python, all fields are public by default.

## Model: Data-First

Models are for **data containers** — types where the fields are the primary concern.

```incan
# Simple data model
model Point:
    x: int
    y: int

# With derives for common behaviors
@derive(Eq, Hash, Serialize)
model User:
    id: int
    name: str
    email: str

# With default values
model Config:
    host: str = "localhost"
    port: int = 8080
    debug: bool = false
```

### What Models Get Automatically

Every model automatically derives `Debug` and `Clone`:

```incan
# These are equivalent:
model User:
    name: str

@derive(Debug, Clone)
model User:
    name: str
```

### Models Cannot Inherit

```incan
model Base:
    id: int

# ❌ This is NOT allowed
model Child extends Base:  # Error!
    name: str

# ✅ Use composition instead
model Child:
    base: Base
    name: str
```

## Class: Behavior-First

Classes are for **objects with behavior** — types where methods are the primary concern.

```incan
trait Loggable:
    def log(self, msg: str): ...

class UserService with Loggable:
    repo: UserRepository
    logger_name: str

    def log(self, msg: str) -> None:
        println(f"[{self.logger_name}] {msg}")

    def create_user(self, name: str, email: str) -> Result[User, Error]:
        self.log(f"Creating user: {name}")
        user = User(id=next_id(), name=name, email=email)
        return self.repo.save(user)

    def find_user(self, id: int) -> Option[User]:
        return self.repo.find_by_id(id)
```

### Classes Support Inheritance

```incan
class Animal:
    name: str
    
    def speak(self) -> str:
        return "..."

class Dog extends Animal:
    breed: str
    
    def speak(self) -> str:
        return "Woof!"

class Cat extends Animal:
    indoor: bool
    
    def speak(self) -> str:
        return "Meow!"
```

### Mutable Methods

Use `mut self` when a method modifies the object:

```incan
class Counter:
    value: int

    def get(self) -> int:
        return self.value

    def increment(mut self) -> None:
        self.value = self.value + 1

    def add(mut self, n: int) -> None:
        self.value = self.value + n
```

## Both Can Implement Traits

```incan
trait Describable:
    def describe(self) -> str: ...

# Model implementing a trait
model Product with Describable:
    name: str
    price: float

    def describe(self) -> str:
        return f"{self.name}: ${self.price}"

# Class implementing a trait
class Employee with Describable:
    name: str
    title: str

    def describe(self) -> str:
        return f"{self.name}, {self.title}"
```

## Design Philosophy

Incan's type system follows the principle: **Python's vocabulary, Rust's guarantees**.

```text
     Python                  Incan                   Rust
┌──────────────┐       ┌──────────────┐       ┌──────────────┐
│  @dataclass  │  →    │    model     │   →   │    struct    │
│    class     │  →    │    class     │   →   │ struct+impl  │
│  ABC/Protocol│  →    │    trait     │   →   │    trait     │
└──────────────┘       └──────────────┘       └──────────────┘
   Ergonomics            Bridge               Performance
```

You think in Python terms, but get Rust's:

- Zero-cost abstractions
- Memory safety without GC (Garbage Collection)
- Compile-time guarantees

## See Also

- [Derives and Traits](derives_and_traits.md) — Adding behaviors with `@derive`
- [Error Handling](error_handling.md) — Using `Result` and `Option`
- [Example: models_vs_classes.incn](../../examples/intermediate/models_vs_classes.incn)
