# RFC 006: Python-Style Generators

**Status:** Planned  
**Created:** 2024-12-10

## Summary

Add Python-style generator functions to Incan, compiled to Rust 2024's `gen` blocks,
enabling lazy iteration with familiar `yield` syntax.

## Motivation

Python developers expect generator functions for:

1. **Lazy evaluation** - Process large/infinite sequences without loading all into memory
2. **Streaming pipelines** - Chain transformations without intermediate allocations
3. **Stateful iteration** - Encapsulate complex iteration logic elegantly
4. **Familiarity** - `yield` in a function is a well-understood Python pattern

With Rust 2024's `gen` blocks now stable, Incan can offer Python-style generators with zero-cost abstractions.

## Design

### Basic Syntax

A function becomes a generator when it:

1. Contains `yield` expressions
2. Returns `Iterator[T]`

```incan
# Python-style generator function
def count_up(start: int, end: int) -> Iterator[int]:
    mut i = start
    while i < end:
        yield i
        i += 1

# Usage: lazy iteration
for n in count_up(0, 1_000_000):
    if n > 100:
        break
    println(n)
```

### Infinite Generators

```incan
def fibonacci() -> Iterator[int]:
    mut a, b = 0, 1
    while true:
        yield a
        a, b = b, a + b

# Take first 10 Fibonacci numbers
let fibs = fibonacci().take(10).collect()
```

### Generators with Filtering

```incan
def even_numbers() -> Iterator[int]:
    mut n = 0
    while true:
        yield n
        n += 2

def primes() -> Iterator[int]:
    mut n = 2
    while true:
        if is_prime(n):
            yield n
        n += 1
```

### Generator Expressions (Comprehension-like)

```incan
# Existing comprehension (eager, collects to List)
squares = [x * x for x in range(10)]

# Generator expression (lazy, returns Iterator)
squares_lazy = (x * x for x in range(10))
```

## Distinction from Fixtures

The `yield` keyword is already used for test fixtures (RFC 001). The distinction:

| Context             | Return Type   | Behavior                        |
| ------------------- | ------------- | ------------------------------- |
| `@fixture` function | Any `T`       | Setup → yield value → teardown  |
| Generator function  | `Iterator[T]` | Lazy iteration, multiple yields |

```incan
# FIXTURE: yield once for setup/teardown
@fixture
def database() -> Database:
    db = Database.connect("test.db")
    yield db        # Single yield: test runs here
    db.close()      # Teardown

# GENERATOR: yield multiple values lazily
def read_lines(path: str) -> Iterator[str]:
    let file = File.open(path)?
    for line in file.lines():
        yield line  # Multiple yields: lazy iteration
```

The compiler distinguishes by:

- `@fixture` decorator → fixture semantics
- `Iterator[T]` return type (no `@fixture`) → generator semantics

## Compilation Strategy

Incan generators compile to Rust 2024 `gen` blocks:

**Incan:**

```incan
def count_up(start: int, end: int) -> Iterator[int]:
    mut i = start
    while i < end:
        yield i
        i += 1
```

**Generated Rust:**

```rust
fn count_up(start: i64, end: i64) -> impl Iterator<Item = i64> {
    gen {
        let mut i = start;
        while i < end {
            yield i;
            i += 1;
        }
    }
}
```

### Key Points

1. **`gen` block** wraps the function body
2. **`impl Iterator<Item = T>`** return type
3. **State machine** generated automatically by rustc
4. **Zero allocation** for the iterator itself

## Iterator Methods

Generators return standard iterators, so all iterator methods work:

```incan
def naturals() -> Iterator[int]:
    mut n = 0
    while true:
        yield n
        n += 1

# Chaining works naturally
result = (
    naturals()
    .filter((n) => (n % 2) == 0)  # filter even numbers
    .map((n) => n * n)            # square the numbers
    .take(5)
    .collect()
) # result = [0, 4, 16, 36, 64]
```

## Implementation Plan

### Phase 1: Parser Changes

1. Detect `yield` in function body
2. Check return type is `Iterator[T]`
3. Mark function as generator in AST

```rust
// AST addition
pub struct FunctionDecl {
    // ... existing fields ...
    pub is_generator: bool,  // NEW: detected from yield + Iterator return
}
```

### Phase 2: Type Checker

1. Verify `yield` expressions match `Iterator[T]` element type
2. Error if `yield` used without `Iterator` return type (unless `@fixture`)
3. Error if `Iterator` return type without `yield`

### Phase 3: Codegen

1. Wrap generator function bodies in `gen { }`
2. Transform `yield expr` to Rust `yield expr`
3. Return type → `impl Iterator<Item = T>`

### Phase 4: Generator Expressions

Lower `(expr for x in iter)` to anonymous generator:

```incan
squares = (x * x for x in range(10))
```

Becomes:

```rust
squares = gen {
    for x in (0..10) {
        yield x * x;
    }
};
```

## Examples

### File Processing

```incan
def read_csv_rows(path: str) -> Iterator[List[str]]:
    """Lazily read CSV rows without loading entire file."""
    file = File.open(path)?
    for line in file.lines():
        yield line.split(",")

# Process million-row CSV with constant memory
for row in read_csv_rows("huge.csv"):
    process(row)
```

### Pagination

```incan
def paginate(items: List[T], page_size: int) -> Iterator[List[T]]:
    """Yield pages of items."""
    mut start = 0
    while start < len(items):
        let end = min(start + page_size, len(items))
        yield items[start..end]
        start = end

for page in paginate(users, 10):
    display_page(page)
```

### Tree Traversal

```incan
def walk_tree(node: Node) -> Iterator[Node]:
    """Depth-first traversal."""
    yield node
    for child in node.children:
        for descendant in walk_tree(child):
            yield descendant
```

## Alternatives Considered

### 1. Explicit `gen` Keyword

```incan
def fibonacci() -> Iterator[int]:
    return gen:
        mut a, b = 0, 1
        while true:
            yield a
            a, b = b, a + b
```

**Rejected**: More verbose, less Pythonic. The `Iterator[T]` return type already signals generator intent.

### 2. Separate `generator` Keyword

```incan
generator fibonacci() -> int:  # Note: no Iterator wrapper
    mut a, b = 0, 1
    while true:
        yield a
        a, b = b, a + b
```

**Rejected**: Deviates from Python, which uses regular `def` for generators.

### 3. No Generators (Comprehensions Only)

Rely on list comprehensions and explicit iterator construction.

**Rejected**: Forces eager evaluation, no good story for infinite sequences.

## Open Questions

1. **Send/receive**: Should generators support `yield` receiving values (like Python's `gen.send()`)?
    Likely deferred — Rust's `gen` blocks don't support this yet.

2. **Async generators**: `async def foo() -> AsyncIterator[T]` with `yield`.
    Requires `async gen` blocks (not yet in Rust). Defer to future RFC.

3. **Early return**: What happens with `return` inside a generator? Propose: terminates iteration (like Python).

## Checklist

- [ ] Parser: detect `yield` + `Iterator[T]` return type → mark as generator
- [ ] Type checker: validate yield types match Iterator element type
- [ ] Codegen: wrap body in `gen { }`, emit `impl Iterator<Item = T>`
- [ ] Generator expressions: `(expr for x in iter)` syntax
- [ ] Iterator methods work on generators (`.map()`, `.filter()`, `.take()`, etc.)
- [ ] Error messages for common mistakes (yield without Iterator, etc.)
- [ ] Examples and documentation
- [ ] Integration with existing comprehensions

## References

- [Python Generators](https://docs.python.org/3/howto/functional.html#generators)
- [Rust RFC 3513: gen blocks](https://rust-lang.github.io/rfcs/3513-gen-blocks.html)
- [Rust 1.85 gen blocks stabilization](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html)
