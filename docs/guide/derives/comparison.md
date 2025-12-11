# Comparison Derives

This page covers the `Eq`, `Ord`, and `Hash` derives for comparing and hashing your types.

## Eq (Equality)

**What it does**: Enables equality comparison with `==` and `!=`.

**Python equivalent**: `__eq__`, `__ne__`

**Rust generated**: `#[derive(Eq, PartialEq)]`

### Basic Eq Usage

```incan
@derive(Debug, Eq)
model User:
    id: int
    name: str

def main() -> None:
    user1 = User(id=1, name="Alice")
    user2 = User(id=1, name="Alice")
    user3 = User(id=2, name="Bob")

    if user1 == user2:
        println("Same user!")  # ✓ Prints

    if user1 != user3:
        println("Different users!")  # ✓ Prints
```

### How Equality Works

By default, `Eq` compares **all fields**. Two instances are equal only if every field matches:

```incan
@derive(Debug, Eq)
model Point:
    x: int
    y: int

def main() -> None:
    p1 = Point(x=10, y=20)
    p2 = Point(x=10, y=20)  # Equal - same x and y
    p3 = Point(x=10, y=99)  # Not equal - different y

    println(f"{p1 == p2}")  # true
    println(f"{p1 == p3}")  # false
```

### Custom Equality with `__eq__`

Sometimes you want to compare only certain fields (e.g., compare entities by ID):

```incan
@derive(Debug, Clone)  # Note: NOT deriving Eq
model User:
    id: int
    name: str
    cache_key: str  # Internal field, shouldn't affect equality

    def __eq__(self, other: User) -> bool:
        return self.id == other.id

def main() -> None:
    user1 = User(id=1, name="Alice", cache_key="abc")
    user2 = User(id=1, name="Alice", cache_key="xyz")  # Different cache_key

    println(f"{user1 == user2}")  # true - same id
```

---

## Ord

**What it does**: Enables ordering comparisons (`<`, `<=`, `>`, `>=`) and sorting.

**Python equivalent**: `__lt__`, `__le__`, `__gt__`, `__ge__`

**Rust generated**: `#[derive(Ord, PartialOrd, Eq, PartialEq)]`

### Basic Ord Usage

```incan
@derive(Debug, Eq, Ord)
model Score:
    points: int
    player: str

def main() -> None:
    high = Score(points=100, player="Alice")
    low = Score(points=50, player="Bob")

    if high > low:
        println("High score wins!")  # ✓ Prints

    if low <= high:
        println("Low is less or equal")  # ✓ Prints
```

### Sorting

`Ord` enables sorting collections:

```incan
@derive(Debug, Eq, Ord)
model Player:
    score: int
    name: str

def main() -> None:
    players = [
        Player(score=50, name="Charlie"),
        Player(score=100, name="Alice"),
        Player(score=75, name="Bob")
    ]

    sorted_players = sorted(players)
    for p in sorted_players:
        println(f"{p.name}: {p.score}")
    # Charlie: 50
    # Bob: 75
    # Alice: 100
```

### Field Ordering

Fields are compared in **declaration order**. The first field is most significant:

```incan
@derive(Debug, Eq, Ord)
model Version:
    major: int  # Compared first
    minor: int  # Then this
    patch: int  # Then this

def main() -> None:
    v1 = Version(major=1, minor=9, patch=0)
    v2 = Version(major=2, minor=0, patch=0)

    println(f"{v1 < v2}")  # true - major 1 < major 2
```

### Custom Ordering with `__lt__`

```incan
@derive(Debug, Clone, Eq)  # Note: NOT deriving Ord
model Task:
    priority: int  # Lower number = higher priority
    name: str

    def __lt__(self, other: Task) -> bool:
        # Reverse: lower priority number comes first
        return self.priority < other.priority

def main() -> None:
    tasks = [
        Task(priority=3, name="Low"),
        Task(priority=1, name="High"),
        Task(priority=2, name="Medium")
    ]

    sorted_tasks = sorted(tasks)
    for t in sorted_tasks:
        println(f"{t.name}")
    # High
    # Medium
    # Low
```

---

## Hash

**What it does**: Enables using your type as a `Dict` key or in a `Set`.

**Python equivalent**: `__hash__`

**Rust generated**: `#[derive(Hash)]`

### Basic Hash Usage

```incan
@derive(Debug, Eq, Hash)
model UserId:
    id: int

def main() -> None:
    # Use in Set
    seen: Set[UserId] = set()
    seen.add(UserId(id=1))
    seen.add(UserId(id=2))
    seen.add(UserId(id=1))  # Duplicate, won't be added

    println(f"Unique IDs: {len(seen)}")  # 2

    # Use as Dict key
    names: Dict[UserId, str] = {}
    names[UserId(id=1)] = "Alice"
    names[UserId(id=2)] = "Bob"

    println(names[UserId(id=1)])  # Alice
```

### Hash Requires Eq

`Hash` always requires `Eq` - if two values are equal, they must have the same hash.
Incan handles this automatically:

```incan
@derive(Debug, Hash)  # Eq is auto-included
model Tag:
    name: str
```

### Custom Hash with `__hash__`

If you define custom `__eq__`, you must also define matching `__hash__`:

```incan
@derive(Debug, Clone)
model CacheEntry:
    key: str
    value: str
    timestamp: int  # Shouldn't affect equality or hash

    def __eq__(self, other: CacheEntry) -> bool:
        return self.key == other.key

    def __hash__(self) -> int:
        # Must be consistent with __eq__
        return hash(self.key)
```

> **Important**: If `a == b`, then `hash(a)` must equal `hash(b)`. Violating this breaks Sets and Dicts.

---

## Comparison Quick Reference

| Derive | Operators Enabled | Use Case |
|--------|-------------------|----------|
| `Eq` | `==`, `!=` | Check if two instances are the same |
| `Ord` | `<`, `<=`, `>`, `>=`, sorting | Order/rank instances |
| `Hash` | Dict keys, Set members | Store in hash-based collections |

## See Also

- [Derives Overview](./derives_and_traits.md) - Complete reference
- [Dunder Overrides](./derives_custom.md) - Custom comparison logic
