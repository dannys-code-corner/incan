# 10. Models vs classes

Incan has two “types with fields”:

- `model`: data-first (great for DTOs, configs, payloads)
- `class`: behavior-first (methods + possible mutation)

!!! tip "Coming from Python?"
    A `model` is closest to a Python `@dataclass` (or a Pydantic `Basemodel`) in spirit: you declare fields, and you get
    a constructor automatically.

    In Incan you **don’t** write an `__init__`/init method. You construct values with keyword arguments matching the declared
    fields (as shown below), and you can give fields default values in the declaration when needed. For validation or alternate 
    construction, write a separate helper/factory function (often returning `Result`).

## A simple model

```incan
model User:
    name: str
    age: int

def main() -> None:
    u = User(name="Alice", age=30)
    println(f"{u.name} age={u.age}")
```

## A simple class

```incan
class Counter:
    value: int

    def increment(mut self) -> None:
        self.value += 1

def main() -> None:
    c = Counter(value=0)
    c.increment()
    c.increment()
    println(f"value={c.value}")  # outputs: value=2
```

## Try it

1. Create a `model Product` with `name` and `price`.
2. Create a `class Cart` with a list of products and a method to compute a total.
3. Print the total.

??? example "One possible solution"

    ```incan
    model Product:
        name: str
        price: float

    class Cart:
        items: list[Product]

        def total(self) -> float:
            total = 0.0
            for item in self.items:
                total = total + item.price
            return total

    def main() -> None:
        cart = Cart(items=[
            Product(name="Book", price=10.0),
            Product(name="Pen", price=2.5),
        ])
        println(f"total={cart.total()}")
    ```

## Where to learn more

- Full guide and decision table: [Models & Classes](../../explanation/models_and_classes.md)

## Next

Back: [9. Enums and better `match`](09_enums.md)

Next chapter: [11. Traits and derives](11_traits_and_derives.md)
