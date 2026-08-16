```bash
incan new hello --yes
cd hello
incan oven bake --project .
incan run
incan test
incan build --release
```

This is the canonical first-contact loop: scaffold one project, prepare its receipt-bound debug and release plans once, run its entry point, execute its tests, and produce a native release build. Normal `run`, `test`, and `build` commands then reuse the sealed plans without invoking Cargo.
