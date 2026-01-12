```incan
def main() -> None:
    match parse_port("8080"):
        Ok(port) => println(f"port={port}")
        Err(e) => println(f"error: {e}")
```
