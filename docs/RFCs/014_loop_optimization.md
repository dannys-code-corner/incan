# RFC 014: Optimize `while True` to `loop`

**Status:** Future  
**Priority:** Low

## Summary

Optimize `while True:` in Incan to emit Rust's idiomatic `loop { }` instead of `while true { }`.

## Motivation

Rust's `loop` is the idiomatic way to write infinite loops:

- Clearer intent
- Compiler knows exit is only via `break`
- Enables `break` with values

Currently Incan generates:

```rust
while true {
    // ...
}
```

Should generate:

```rust
loop {
    // ...
}
```

## Implementation

Change is in **codegen only** (`src/backend/codegen/statements.rs`):

```rust
pub(crate) fn emit_while(emitter: &mut RustEmitter, while_stmt: &WhileStmt) {
    // Optimize while True → loop
    if matches!(&while_stmt.condition.node, Expr::Literal(Literal::Bool(true))) {
        emitter.line("loop {");
        emitter.indent();
        for stmt in &while_stmt.body {
            Self::emit_statement(emitter, &stmt.node);
        }
        emitter.dedent();
        emitter.line("}");
        return;
    }
    
    // Normal while emission...
}
```

No lexer/parser changes needed.

## Alternatives to consider

1. **Add `loop` keyword** - Deviates from Python syntax
2. **Keep as-is** - `while true` works, optimizer may convert anyway
