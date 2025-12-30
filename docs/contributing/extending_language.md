# Extending Incan: Builtins vs New Syntax

This document is for **contributors** who want to add new language features.

Incan is implemented as a **multi-stage compiler**:

- Frontend: Lexer → Parser → AST → Typechecker (`typechecker/`)
- Backend: Lowering (AST → IR) → Emitter (IR → Rust)

That separation is intentional (clarity, correctness, debuggability), but it means that adding *new syntax* typically touches multiple stages.

---

## Rule of Thumb

**Prefer a library/builtin over a new keyword.**

Add a new **keyword / syntax form** only when the feature:

- **Introduces control-flow** that cannot be expressed as a call (e.g. `match`, `yield`, `await`, `?`)
- Requires **special typing rules** that would be awkward or misleading as a function
- Needs **non-standard evaluation** of its operands (short-circuiting, implicit returns, pattern binding, etc.)

If the feature is “some behavior” (logging, printing, tracing, helpers), it should usually be:

- A **stdlib function** (preferred), or
- A **compiler builtin** (when it must lower to special Rust code).

---

## Path A: Add a Function (Stdlib or Compiler Builtin)

### A1) Stdlib function (no new syntax)

Use this when the behavior can live in runtime support crates (e.g. `incan_stdlib`), without compiler special casing.

Typical work:

- Add runtime implementation in `crates/incan_stdlib/`
- Expose it via the prelude if appropriate
- Document it in the language guide

This avoids changing the lexer/parser/AST/IR.

### A2) Compiler builtin function (special lowering/emission)

Use this when you want a function-call surface syntax, but it must emit a particular Rust pattern.

Incan already has enum-dispatched builtins in IR (`BuiltinFn`) and emission logic in `emit/expressions/builtins.rs`.

**End-to-end checklist:**

- **Frontend symbol table**: add the builtin name and signature so it typechecks
  - `src/frontend/symbols.rs` → `SymbolTable::add_builtins()`
- **IR builtin enum**: add a new variant and name mapping
  - `src/backend/ir/expr.rs` → `enum BuiltinFn` + `BuiltinFn::from_name()`
- **Lowering**: ensure calls to that name lower to `IrExprKind::BuiltinCall`
  - `src/backend/ir/lower/expr.rs` uses `BuiltinFn::from_name(name)` for identifiers
- **Emission**: emit the Rust code for the new builtin
  - `src/backend/ir/emit/expressions/builtins.rs` → `emit_builtin_call()`
- **Docs/tests**: add/adjust as needed

This path is often **much cheaper** than adding new syntax, while still letting you control the generated Rust.

### A3) Compiler builtin method (special method lowering/emission)

Use this when you want to add a method on existing types (e.g. `list.some_method()`) that needs special Rust emission.

Incan has enum-dispatched methods in IR (`MethodKind`) and emission logic in `emit/expressions/methods.rs`.

**End-to-end checklist:**

- **IR method enum**: add a new variant and name mapping
  - `src/backend/ir/expr.rs` → `enum MethodKind` + `MethodKind::from_name()`
- **Lowering**: automatic (uses `MethodKind::from_name(name)` for all method calls)
  - `src/backend/ir/lower/expr.rs` already handles this
- **Emission**: emit the Rust code for the new method
  - `src/backend/ir/emit/expressions/methods.rs` → `emit_known_method_call()`
- **Docs/tests**: add/adjust as needed

Unknown methods pass through as regular Rust method calls, so you don't break Rust interop by adding known methods.

---

## Path B: Add a New Keyword / Syntax Form

Use this only when the feature is genuinely syntactic/control-flow.

**End-to-end checklist (typical):**

- **Lexer**
  - `crates/incan_syntax/src/lexer/*`
    - Add a `KeywordId` to `crates/incan_core/src/lang/keywords.rs`
    - Ensure tokenization emits `TokenKind::Keyword(KeywordId::YourKeyword)`
    - Update lexer parity tests (keyword/operator/punctuation registry parity)
- **Parser**
  - `crates/incan_syntax/src/parser.rs`
    - Parse the syntax and build a new AST node (usually an `Expr` or `Statement` variant)
- **AST**
  - `crates/incan_syntax/src/ast.rs`
    - Add the new `Expr::<YourNode>` or `Statement::<YourNode>` variant
- **Formatter**
  - `src/format/formatter.rs`
    - Teach the formatter how to print the new node
- **Typechecker**
  - `src/frontend/typechecker/`
    - `check_decl.rs` – add type-level rules (models, classes, traits)
    - `check_stmt.rs` – add statement-level rules (assignments, control flow)
    - `check_expr/*.rs` – add expression-level rules (calls, operators, match)
- **(Optional) Scanners**
  - `src/backend/ir/scanners.rs`
    - Ensure feature detection traverses the new node if relevant
- **Lowering (AST → IR)**
  - `src/backend/ir/lower/*.rs`
    - Lower the new AST node into an IR representation
- **IR (if needed)**
  - `src/backend/ir/expr.rs` / `stmt.rs` / `decl.rs`
    - Add a new `IrExprKind`/`IrStmtKind` variant if the feature is not expressible via existing IR
- **Emitter (IR → Rust)**
  - `src/backend/ir/emit/**/*.rs`
    - Emit correct Rust for the new IR node
- **Editor tooling (optional but recommended)**
  - `editors/vscode/*` keyword highlighting / indentation patterns
- **Docs + tests**
  - Add a guide snippet and at least one parse/typecheck/codegen regression test

---

## Practical guidance

- If you find yourself adding a keyword to achieve *“a function with a special implementation”*, pause and consider making it a **builtin function** instead.
- If you add a new AST/IR enum variant, rely on Rust’s exhaustiveness errors as your checklist: the compiler will tell you which match arms you need to update.
