# 12. Newtypes (stronger types)

A **newtype** is a “zero-cost wrapper” around another type. It lets you create **distinct types** that the compiler won’t
let you accidentally mix up.

## Why newtypes?

If two values share the same underlying type (like `int`), it’s easy to mix them up by accident.

With newtypes, the compiler helps you keep them straight:

```incan
type UserId = newtype int
type ProductId = newtype int

def get_user_name(user_id: UserId) -> str:
    return f"user={user_id.0}"

def get_product_name(product_id: ProductId) -> str:
    return f"product={product_id.0}"

def main() -> None:
    user_id = UserId(42)
    product_id = ProductId(100)

    println(get_user_name(user_id))
    println(get_product_name(product_id))

    # If you accidentally swap them, the compiler can catch it:
    # println(get_user_name(product_id))
```

!!! tip "Coming from Python?"
    This is similar to creating a tiny wrapper class just to avoid mixing up values, but with less runtime overhead and
    better static checking.

## Constructing and unwrapping

You construct a newtype by calling it like a function: `UserId(42)`.

To access the wrapped value, use `.0`:

```incan
def main() -> None:
    user_id = UserId(42)
    println(f"{user_id.0}")
```

## Validation with `Result`

Newtypes are great for values that should satisfy an invariant (like “must be positive”).

One common pattern is to provide a constructor helper that returns `Result`:

```incan
type UserId = newtype int

def make_positive_id(n: int) -> Result[UserId, str]:
    if n > 0:
        return Ok(UserId(n))
    return Err("ID must be positive")
```

You can then use `match` (or `?` if you’re inside a function returning `Result`) to handle failures.

## Methods on newtypes

Newtypes can also define methods:

```incan
enum EmailError:
    MissingAt

type Email = newtype str:
    def from_str(v: str) -> Result[Email, EmailError]:
        """Validate an email address by checking for the presence of an @ symbol"""
        if "@" not in v:
            return Err(EmailError.MissingAt)
        return Ok(Email(v.lower()))

def main() -> None:
    match Email.from_str("Alice@Example.com"):
        Ok(email) => println(f"email={email.0}")
        Err(_) => println("invalid email")
```

## Try it

1. Define `type CartItems = newtype List[str]`.
2. Write `def non_empty(items: List[str]) -> Result[CartItems, str]` that rejects empty lists.
3. In `main()`, call it with both `[]` and `["a"]` and print either the error or the first item.

??? example "One possible solution"

    ```incan
    type CartItems = newtype List[str]

    def non_empty(items: List[str]) -> Result[CartItems, str]:
        if len(items) == 0:
            return Err("must not be empty")
        return Ok(CartItems(items))

    def main() -> None:
        match non_empty([]):
            Ok(items) => println(f"first={items.0[0]}")
            Err(e) => println(f"error: {e}")

        match non_empty(["a"]):
            Ok(items) => println(f"first={items.0[0]}")
            Err(e) => println(f"error: {e}")
    ```

## Where to learn more

- Longer example in the repository: `examples/intermediate/newtypes.incn`

## Next

Back: [11. Traits and derives](11_traits_and_derives.md)

Next: return to [Start here](../../../start_here/index.md) or continue in the [Language Guide](../../index.md).
