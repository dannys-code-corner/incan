# Incan language reference

> Generated file. Do not edit by hand.
>
> Regenerate with: `cargo run -p incan_core --bin generate_lang_reference`

## Contents

- [Keywords](#keywords)
- [Builtin exceptions](#builtin-exceptions)
- [Builtin functions](#builtin-functions)
- [Derives](#derives)
- [Operators](#operators)
- [Punctuation](#punctuation)
- [Builtin types](#builtin-types)
- [Surface constructors](#surface-constructors)
- [Surface functions](#surface-functions)
- [Surface math](#surface-math)
- [Surface string methods](#surface-string-methods)
- [Surface types](#surface-types)
- [Surface methods](#surface-methods)

## Keywords

| Id | Canonical | Aliases | Category | Usage | RFC | Since | Stability |
|----|---|---|---|---|---|---|---|
| If | `if` |  | ControlFlow | Statement, Expression | RFC 000 |  | Stable |
| Else | `else` |  | ControlFlow | Statement | RFC 000 |  | Stable |
| Elif | `elif` |  | ControlFlow | Statement | RFC 000 |  | Stable |
| Match | `match` |  | ControlFlow | Statement, Expression | RFC 000 |  | Stable |
| Case | `case` |  | ControlFlow | Statement | RFC 000 |  | Stable |
| While | `while` |  | ControlFlow | Statement | RFC 000 |  | Stable |
| For | `for` |  | ControlFlow | Statement | RFC 000 |  | Stable |
| Break | `break` |  | ControlFlow | Statement | RFC 000 |  | Stable |
| Continue | `continue` |  | ControlFlow | Statement | RFC 000 |  | Stable |
| Return | `return` |  | ControlFlow | Statement | RFC 000 |  | Stable |
| Yield | `yield` |  | ControlFlow | Statement, Expression | RFC 001 |  | Stable |
| Pass | `pass` |  | ControlFlow | Statement | RFC 000 |  | Stable |
| Def | `def` | `fn` | Definition | Statement | RFC 000 |  | Stable |
| Async | `async` |  | Definition | Modifier | RFC 000 |  | Stable |
| Await | `await` |  | Definition | Expression | RFC 000 |  | Stable |
| Class | `class` |  | Definition | Statement | RFC 000 |  | Stable |
| Model | `model` |  | Definition | Statement | RFC 000 |  | Stable |
| Trait | `trait` |  | Definition | Statement | RFC 000 |  | Stable |
| Enum | `enum` |  | Definition | Statement | RFC 000 |  | Stable |
| Type | `type` |  | Definition | Statement | RFC 000 |  | Stable |
| Newtype | `newtype` |  | Definition | Statement | RFC 000 |  | Stable |
| With | `with` |  | Definition | Modifier | RFC 000 |  | Stable |
| Extends | `extends` |  | Definition | Modifier | RFC 000 |  | Stable |
| Pub | `pub` |  | Definition | Modifier | RFC 000 |  | Stable |
| Import | `import` |  | Import | Statement | RFC 000 |  | Stable |
| From | `from` |  | Import | Statement | RFC 000 |  | Stable |
| As | `as` |  | Import | Modifier | RFC 000 |  | Stable |
| Rust | `rust` |  | Import | Modifier | RFC 005 |  | Stable |
| Python | `python` |  | Import | Modifier | RFC 000 |  | Stable |
| Super | `super` |  | Import | Expression | RFC 000 |  | Stable |
| Crate | `crate` |  | Import | Expression | RFC 005 |  | Stable |
| Const | `const` |  | Binding | Statement | RFC 008 |  | Stable |
| Let | `let` |  | Binding | Statement | RFC 000 |  | Stable |
| Mut | `mut` |  | Binding | Modifier | RFC 000 |  | Stable |
| SelfKw | `self` |  | Binding | ReceiverOnly | RFC 000 |  | Stable |
| True | `true` | `True` | Literal | Expression | RFC 000 |  | Stable |
| False | `false` | `False` | Literal | Expression | RFC 000 |  | Stable |
| None | `None` |  | Literal | Expression | RFC 000 |  | Stable |
| And | `and` |  | Operator | Operator | RFC 000 |  | Stable |
| Or | `or` |  | Operator | Operator | RFC 000 |  | Stable |
| Not | `not` |  | Operator | Operator | RFC 000 |  | Stable |
| In | `in` |  | Operator | Operator | RFC 000 |  | Stable |
| Is | `is` |  | Operator | Operator | RFC 000 |  | Stable |

### Examples

Only keywords with examples are listed here.

## Builtin exceptions

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| ValueError | `ValueError` |  | Raised when an operation receives a value of the right type but an invalid value. | RFC 000 |  | Stable |
| TypeError | `TypeError` |  | Raised when an operation receives a value of an inappropriate type. | RFC 000 |  | Stable |
| ZeroDivisionError | `ZeroDivisionError` |  | Raised when dividing or taking modulo by zero (Python-like numeric semantics). | RFC 000 |  | Stable |
| IndexError | `IndexError` |  | Raised when an index is out of bounds (e.g. string/list indexing). | RFC 000 |  | Stable |
| KeyError | `KeyError` |  | Raised when a dict key is missing. | RFC 000 |  | Stable |
| JsonDecodeError | `JSONDecodeError` |  | Raised when parsing JSON fails (Python-like). | RFC 000 |  | Stable |

### Examples

Only exceptions with examples are listed here.

#### `ValueError`

```incan
def main() -> None:
    print("abc"[0:3:0])  # step 0

```

Panics at runtime with `ValueError: slice step cannot be zero`.

```incan
def main() -> None:
    _ = int("abc")

```

Panics at runtime with `ValueError: cannot convert 'abc' to int`.

```incan
def main() -> None:
    _ = float("abc")

```

Panics at runtime with `ValueError: cannot convert 'abc' to float`.

```incan
def main() -> None:
    # range step cannot be zero (Python-like)
    for i in range(0, 5, 0):
        print(i)

```

Panics at runtime with `ValueError: range() arg 3 must not be zero`.

#### `TypeError`

```incan
def main() -> None:
    # Example: JSON serialization failures (e.g. NaN/Inf) raise TypeError
    _ = json_stringify(nan)

```

Panics at runtime with a `TypeError: ... is not JSON serializable` message.

#### `ZeroDivisionError`

```incan
def main() -> None:
    print(1 / 0)

```

Panics at runtime with `ZeroDivisionError: float division by zero`.

#### `IndexError`

```incan
def main() -> None:
    print("a"[99])

```

Panics at runtime with `IndexError: ...`.

```incan
def main() -> None:
    xs: list[int] = [1, 2, 3]
    print(xs[99])

```

Panics at runtime with `IndexError: index 99 out of range for list of length 3`.

#### `KeyError`

```incan
def main() -> None:
    d: Dict[str, int] = {"a": 1}
    print(d["b"])

```

Panics at runtime with `KeyError: 'b' not found in dict`.

#### `JsonDecodeError`

```incan
@derive(Deserialize)
model User:
    name: str

def main() -> None:
    bad: str = "{"
    match User.from_json(bad):
        case Ok(u): print(u.name)
        case Err(e): print(e)

```

`from_json` returns `Result[T, str]`; on failure the error string is prefixed with `JSONDecodeError: ...`.

## Builtin functions

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Print | `print` | `println` | Print values to stdout. | RFC 000 |  | Stable |
| Len | `len` |  | Return the length of a collection/string. | RFC 000 |  | Stable |
| Sum | `sum` |  | Sum a numeric iterable/collection. | RFC 000 |  | Stable |
| Str | `str` |  | Convert a value to a string. | RFC 000 |  | Stable |
| Int | `int` |  | Convert a value to an integer. | RFC 000 |  | Stable |
| Float | `float` |  | Convert a value to a float. | RFC 000 |  | Stable |
| Abs | `abs` |  | Absolute value (numeric). | RFC 000 |  | Stable |
| Range | `range` |  | Create a range of integers. | RFC 000 |  | Stable |
| Enumerate | `enumerate` |  | Enumerate an iterable into (index, value) pairs. | RFC 000 |  | Stable |
| Zip | `zip` |  | Zip iterables element-wise into tuples. | RFC 000 |  | Stable |
| ReadFile | `read_file` |  | Read a file from disk into a string/bytes. | RFC 000 |  | Stable |
| WriteFile | `write_file` |  | Write a string/bytes to a file on disk. | RFC 000 |  | Stable |
| JsonStringify | `json_stringify` |  | Serialize a value to JSON. | RFC 000 |  | Stable |
| Sleep | `sleep` |  | Sleep for a duration. | RFC 000 |  | Stable |

## Derives

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Debug | `Debug` |  | Derive Rust-style debug formatting. | RFC 000 |  | Stable |
| Display | `Display` |  | Derive user-facing string formatting. | RFC 000 |  | Stable |
| Eq | `Eq` |  | Derive equality comparisons. | RFC 000 |  | Stable |
| Ord | `Ord` |  | Derive ordering comparisons. | RFC 000 |  | Stable |
| Hash | `Hash` |  | Derive hashing support (for map/set keys). | RFC 000 |  | Stable |
| Clone | `Clone` |  | Derive deep cloning. | RFC 000 |  | Stable |
| Copy | `Copy` |  | Derive copy semantics for simple value types. | RFC 000 |  | Stable |
| Default | `Default` |  | Derive a default value constructor. | RFC 000 |  | Stable |
| Serialize | `Serialize` |  | Derive serialization support (e.g. JSON). | RFC 000 |  | Stable |
| Deserialize | `Deserialize` |  | Derive deserialization support (e.g. JSON). | RFC 000 |  | Stable |

## Operators

### Notes

- **Precedence**: Higher binds tighter (e.g. `*` > `+`). Values are relative and must be consistent with the parser.
- **Associativity**: How operators of the same precedence group (left-to-right vs right-to-left).
- **Fixity**: Whether the operator is used as a prefix unary operator or an infix binary operator.
- **KeywordSpelling**: Whether the operator token is spelled as a reserved word (e.g. `and`, `not`).

| Id | Spellings | Precedence | Associativity | Fixity | KeywordSpelling | RFC | Since | Stability |
|---|---|---:|---|---|---|---|---|---|
| Plus | `+` | 50 | Left | Infix | false | RFC 000 |  | Stable |
| Minus | `-` | 50 | Left | Infix | false | RFC 000 |  | Stable |
| Star | `*` | 60 | Left | Infix | false | RFC 000 |  | Stable |
| StarStar | `**` | 70 | Right | Infix | false | RFC 000 |  | Stable |
| Slash | `/` | 60 | Left | Infix | false | RFC 000 |  | Stable |
| SlashSlash | `//` | 60 | Left | Infix | false | RFC 000 |  | Stable |
| Percent | `%` | 60 | Left | Infix | false | RFC 000 |  | Stable |
| EqEq | `==` | 40 | Left | Infix | false | RFC 000 |  | Stable |
| NotEq | `!=` | 40 | Left | Infix | false | RFC 000 |  | Stable |
| Lt | `<` | 40 | Left | Infix | false | RFC 000 |  | Stable |
| LtEq | `<=` | 40 | Left | Infix | false | RFC 000 |  | Stable |
| Gt | `>` | 40 | Left | Infix | false | RFC 000 |  | Stable |
| GtEq | `>=` | 40 | Left | Infix | false | RFC 000 |  | Stable |
| Eq | `=` | 10 | Left | Infix | false | RFC 000 |  | Stable |
| PlusEq | `+=` | 10 | Left | Infix | false | RFC 000 |  | Stable |
| MinusEq | `-=` | 10 | Left | Infix | false | RFC 000 |  | Stable |
| StarEq | `*=` | 10 | Left | Infix | false | RFC 000 |  | Stable |
| SlashEq | `/=` | 10 | Left | Infix | false | RFC 000 |  | Stable |
| SlashSlashEq | `//=` | 10 | Left | Infix | false | RFC 000 |  | Stable |
| PercentEq | `%=` | 10 | Left | Infix | false | RFC 000 |  | Stable |
| DotDot | `..` | 30 | Left | Infix | false | RFC 000 |  | Stable |
| DotDotEq | `..=` | 30 | Left | Infix | false | RFC 000 |  | Stable |
| And | `and` | 35 | Left | Infix | true | RFC 000 |  | Stable |
| Or | `or` | 35 | Left | Infix | true | RFC 000 |  | Stable |
| Not | `not` | 45 | Left | Prefix | true | RFC 000 |  | Stable |
| In | `in` | 35 | Left | Infix | true | RFC 000 |  | Stable |
| Is | `is` | 35 | Left | Infix | true | RFC 000 |  | Stable |

## Punctuation

| Id | Canonical | Aliases | Category | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Comma | `,` |  | Separator | RFC 000 |  | Stable |
| Colon | `:` |  | Separator | RFC 000 |  | Stable |
| Question | `?` |  | Marker | RFC 000 |  | Stable |
| At | `@` |  | Marker | RFC 000 |  | Stable |
| Dot | `.` |  | Access | RFC 000 |  | Stable |
| ColonColon | `::` |  | Access | RFC 000 |  | Stable |
| Arrow | `->` |  | Arrow | RFC 000 |  | Stable |
| FatArrow | `=>` |  | Arrow | RFC 000 |  | Stable |
| Ellipsis | `...` |  | Marker | RFC 000 |  | Stable |
| LParen | `(` |  | Delimiter | RFC 000 |  | Stable |
| RParen | `)` |  | Delimiter | RFC 000 |  | Stable |
| LBracket | `[` |  | Delimiter | RFC 000 |  | Stable |
| RBracket | `]` |  | Delimiter | RFC 000 |  | Stable |
| LBrace | `{` |  | Delimiter | RFC 000 |  | Stable |
| RBrace | `}` |  | Delimiter | RFC 000 |  | Stable |

## Builtin types

### String-like

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Str | `str` |  | Builtin UTF-8 string type. | RFC 000 |  | Stable |
| Bytes | `bytes` |  | Builtin byte buffer type. | RFC 000 |  | Stable |
| FrozenStr | `frozenstr` | `FrozenStr` | Immutable/const-friendly string type. | RFC 009 |  | Stable |
| FrozenBytes | `frozenbytes` | `FrozenBytes` | Immutable/const-friendly bytes type. | RFC 009 |  | Stable |
| FString | `fstring` | `FString` | Formatted string result type. | RFC 000 |  | Stable |


### Numerics

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Int | `int` | `i64`, `i32` | Builtin signed integer type. | RFC 000 |  | Stable |
| Float | `float` | `f64`, `f32` | Builtin floating-point type. | RFC 000 |  | Stable |
| Bool | `bool` |  | Builtin boolean type. | RFC 000 |  | Stable |


### Collections / generic bases

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| List | `List` | `list` | Growable list (generic sequence) type. | RFC 000 |  | Stable |
| Dict | `Dict` | `dict`, `HashMap` | Key/value map type. | RFC 000 |  | Stable |
| Set | `Set` | `set` | Unordered set type. | RFC 000 |  | Stable |
| Tuple | `Tuple` | `tuple` | Fixed-length heterogeneous tuple type. | RFC 000 |  | Stable |
| Option | `Option` | `option` | Optional value type (`Some`/`None`). | RFC 000 |  | Stable |
| Result | `Result` | `result` | Result type (`Ok`/`Err`). | RFC 000 |  | Stable |
| FrozenList | `FrozenList` | `frozenlist` | Immutable/const-friendly list type. | RFC 009 |  | Stable |
| FrozenDict | `FrozenDict` | `frozendict` | Immutable/const-friendly dict type. | RFC 009 |  | Stable |
| FrozenSet | `FrozenSet` | `frozenset` | Immutable/const-friendly set type. | RFC 009 |  | Stable |

## Surface constructors

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Ok | `Ok` |  | Construct an `Ok(T)` variant (Result success). | RFC 000 |  | Stable |
| Err | `Err` |  | Construct an `Err(E)` variant (Result failure). | RFC 000 |  | Stable |
| Some | `Some` |  | Construct a `Some(T)` variant (Option present). | RFC 000 |  | Stable |
| None | `None` |  | Construct a `None` variant (Option absent). | RFC 000 |  | Stable |

## Surface functions

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| SleepMs | `sleep_ms` |  | Sleep for N milliseconds. | RFC 000 |  | Stable |
| Timeout | `timeout` |  | Run an async operation with a timeout. | RFC 000 |  | Stable |
| TimeoutMs | `timeout_ms` |  | Run an async operation with a timeout in milliseconds. | RFC 000 |  | Stable |
| SelectTimeout | `select_timeout` |  | Select between futures with a timeout. | RFC 000 |  | Stable |
| YieldNow | `yield_now` |  | Yield execution back to the async scheduler. | RFC 000 |  | Stable |
| Spawn | `spawn` |  | Spawn an async task. | RFC 000 |  | Stable |
| SpawnBlocking | `spawn_blocking` |  | Spawn a blocking task on a dedicated thread pool. | RFC 004 |  | Stable |
| Channel | `channel` |  | Create a bounded channel (sender, receiver). | RFC 000 |  | Stable |
| UnboundedChannel | `unbounded_channel` |  | Create an unbounded channel (sender, receiver). | RFC 000 |  | Stable |
| Oneshot | `oneshot` |  | Create a oneshot channel (sender, receiver). | RFC 000 |  | Stable |

## Surface math

### Functions

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Sqrt | `sqrt` |  | Square root. | RFC 000 |  | Stable |
| Abs | `abs` |  | Absolute value. | RFC 000 |  | Stable |
| Floor | `floor` |  | Floor (round down). | RFC 000 |  | Stable |
| Ceil | `ceil` |  | Ceil (round up). | RFC 000 |  | Stable |
| Pow | `pow` |  | Power function. | RFC 000 |  | Stable |
| Exp | `exp` |  | Exponentiation (e^x). | RFC 000 |  | Stable |
| Log | `log` |  | Natural logarithm. | RFC 000 |  | Stable |
| Log10 | `log10` |  | Base-10 logarithm. | RFC 000 |  | Stable |
| Log2 | `log2` |  | Base-2 logarithm. | RFC 000 |  | Stable |
| Sin | `sin` |  | Sine. | RFC 000 |  | Stable |
| Cos | `cos` |  | Cosine. | RFC 000 |  | Stable |
| Tan | `tan` |  | Tangent. | RFC 000 |  | Stable |
| Asin | `asin` |  | Arcsine. | RFC 000 |  | Stable |
| Acos | `acos` |  | Arccosine. | RFC 000 |  | Stable |
| Atan | `atan` |  | Arctangent. | RFC 000 |  | Stable |
| Sinh | `sinh` |  | Hyperbolic sine. | RFC 000 |  | Stable |
| Cosh | `cosh` |  | Hyperbolic cosine. | RFC 000 |  | Stable |
| Tanh | `tanh` |  | Hyperbolic tangent. | RFC 000 |  | Stable |
| Atan2 | `atan2` |  | Two-argument arctangent. | RFC 000 |  | Stable |


### Constants

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Pi | `pi` |  | The constant π. | RFC 000 |  | Stable |
| E | `e` |  | The constant e. | RFC 000 |  | Stable |
| Tau | `tau` |  | The constant τ (2π). | RFC 000 |  | Stable |
| Inf | `inf` |  | Positive infinity. | RFC 000 |  | Stable |
| Nan | `nan` |  | Not a number (NaN). | RFC 000 |  | Stable |

## Surface string methods

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Upper | `upper` |  | Convert to uppercase. | RFC 009 |  | Stable |
| Lower | `lower` |  | Convert to lowercase. | RFC 009 |  | Stable |
| Strip | `strip` |  | Strip leading and trailing whitespace. | RFC 009 |  | Stable |
| Replace | `replace` |  | Replace occurrences of a substring. | RFC 009 |  | Stable |
| Join | `join` |  | Join an iterable/list of strings with this separator. | RFC 009 |  | Stable |
| ToString | `to_string` |  | Return a string representation (identity for strings). | RFC 009 |  | Stable |
| SplitWhitespace | `split_whitespace` |  | Split on Unicode whitespace. | RFC 009 |  | Stable |
| Split | `split` |  | Split on a separator substring. | RFC 009 |  | Stable |
| Contains | `contains` |  | Return true if the substring occurs within the string. | RFC 009 |  | Stable |
| StartsWith | `startswith` | `starts_with` | Return true if the string starts with a prefix. | RFC 009 |  | Stable |
| EndsWith | `endswith` | `ends_with` | Return true if the string ends with a suffix. | RFC 009 |  | Stable |
| Len | `len` |  | Return the length (in Unicode scalars). | RFC 009 |  | Stable |
| IsEmpty | `is_empty` |  | Return true if the length is zero. | RFC 009 |  | Stable |

## Surface types

| Id | Canonical | Aliases | Kind | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|---|
| Mutex | `Mutex` |  | Generic | Async/runtime mutex. | RFC 000 |  | Stable |
| RwLock | `RwLock` |  | Generic | Async/runtime read-write lock. | RFC 000 |  | Stable |
| Semaphore | `Semaphore` |  | Named | Async/runtime semaphore. | RFC 000 |  | Stable |
| Barrier | `Barrier` |  | Named | Async/runtime barrier. | RFC 000 |  | Stable |
| JoinHandle | `JoinHandle` |  | Generic | Handle to a spawned task. | RFC 000 |  | Stable |
| Sender | `Sender` |  | Generic | Bounded channel sender. | RFC 000 |  | Stable |
| Receiver | `Receiver` |  | Generic | Bounded channel receiver. | RFC 000 |  | Stable |
| UnboundedSender | `UnboundedSender` |  | Generic | Unbounded channel sender. | RFC 000 |  | Stable |
| UnboundedReceiver | `UnboundedReceiver` |  | Generic | Unbounded channel receiver. | RFC 000 |  | Stable |
| OneshotSender | `OneshotSender` |  | Generic | Oneshot channel sender. | RFC 000 |  | Stable |
| OneshotReceiver | `OneshotReceiver` |  | Generic | Oneshot channel receiver. | RFC 000 |  | Stable |
| Vec | `Vec` |  | Generic | Rust interop `Vec<T>`. | RFC 005 |  | Stable |
| HashMap | `HashMap` |  | Generic | Rust interop `HashMap<K, V>`. | RFC 005 |  | Stable |

## Surface methods

### float methods

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Sqrt | `sqrt` |  | Square root. | RFC 009 |  | Stable |
| Abs | `abs` |  | Absolute value. | RFC 009 |  | Stable |
| Floor | `floor` |  | Round down to the nearest integer (as float). | RFC 009 |  | Stable |
| Ceil | `ceil` |  | Round up to the nearest integer (as float). | RFC 009 |  | Stable |
| Round | `round` |  | Round to the nearest integer (as float). | RFC 009 |  | Stable |
| Sin | `sin` |  | Sine. | RFC 009 |  | Stable |
| Cos | `cos` |  | Cosine. | RFC 009 |  | Stable |
| Tan | `tan` |  | Tangent. | RFC 009 |  | Stable |
| Exp | `exp` |  | Exponentiation (e^x). | RFC 009 |  | Stable |
| Ln | `ln` |  | Natural logarithm. | RFC 009 |  | Stable |
| Log2 | `log2` |  | Base-2 logarithm. | RFC 009 |  | Stable |
| Log10 | `log10` |  | Base-10 logarithm. | RFC 009 |  | Stable |
| IsNan | `is_nan` |  | Return true if this value is NaN. | RFC 009 |  | Stable |
| IsInfinite | `is_infinite` |  | Return true if this value is ±infinity. | RFC 009 |  | Stable |
| IsFinite | `is_finite` |  | Return true if this value is finite. | RFC 009 |  | Stable |
| Powi | `powi` |  | Raise to an integer power. | RFC 009 |  | Stable |
| Powf | `powf` |  | Raise to a float power. | RFC 009 |  | Stable |


### List methods

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Append | `append` |  | Append an element to the end of the list. | RFC 009 |  | Stable |
| Pop | `pop` |  | Remove and return the last element. | RFC 009 |  | Stable |
| Contains | `contains` |  | Return true if the list contains a value. | RFC 009 |  | Stable |
| Swap | `swap` |  | Swap two elements by index. | RFC 009 |  | Stable |
| Reserve | `reserve` |  | Reserve capacity for at least N more elements. | RFC 009 |  | Stable |
| ReserveExact | `reserve_exact` |  | Reserve capacity for exactly N more elements. | RFC 009 |  | Stable |
| Remove | `remove` |  | Remove and return the element at the given index. | RFC 009 |  | Stable |
| Count | `count` |  | Count occurrences of a value. | RFC 009 |  | Stable |
| Index | `index` |  | Return the index of a value (or error if not found). | RFC 009 |  | Stable |


### Dict methods

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Keys | `keys` |  | Return an iterable/list of keys. | RFC 009 |  | Stable |
| Values | `values` |  | Return an iterable/list of values. | RFC 009 |  | Stable |
| Get | `get` |  | Get a value by key, optionally with a default. | RFC 009 |  | Stable |
| Insert | `insert` |  | Insert or overwrite a key/value pair. | RFC 009 |  | Stable |


### Set methods

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Contains | `contains` |  | Return true if the set contains a value. | RFC 009 |  | Stable |


### FrozenList methods

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Len | `len` |  | Return the number of elements. | RFC 009 |  | Stable |
| IsEmpty | `is_empty` |  | Return true if the list is empty. | RFC 009 |  | Stable |


### FrozenDict methods

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Len | `len` |  | Return the number of entries. | RFC 009 |  | Stable |
| IsEmpty | `is_empty` |  | Return true if the dict is empty. | RFC 009 |  | Stable |
| ContainsKey | `contains_key` |  | Return true if the dict contains a key. | RFC 009 |  | Stable |


### FrozenSet methods

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Len | `len` |  | Return the number of elements. | RFC 009 |  | Stable |
| IsEmpty | `is_empty` |  | Return true if the set is empty. | RFC 009 |  | Stable |
| Contains | `contains` |  | Return true if the set contains a value. | RFC 009 |  | Stable |


### FrozenBytes methods

| Id | Canonical | Aliases | Description | RFC | Since | Stability |
|---|---|---|---|---|---|---|
| Len | `len` |  | Return the number of bytes. | RFC 009 |  | Stable |
| IsEmpty | `is_empty` |  | Return true if the byte string is empty. | RFC 009 |  | Stable |


