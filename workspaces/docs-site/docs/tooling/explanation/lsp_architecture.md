# LSP architecture

This page explains how the Incan Language Server works internally.

## High-level design

The LSP is built with [tower-lsp](https://github.com/ebkalderon/tower-lsp) and reuses Incan's compiler frontend.

```mermaid
flowchart LR
  editor["Editor<br/>(VS Code)"]

  subgraph lsp["incan-lsp"]
    direction TB
    lexer["Lexer"]
    parser["Parser"]
    tc["TypeChecker"]
    lexer --> parser --> tc
  end

  editor <--> |stdio| lexer
```

On each file change, the LSP runs the compiler pipeline and reports:

- lexer errors (tokenization failures)
- parser errors (syntax errors)
- type errors (type mismatches, unknown symbols, etc.)
- checked public API metadata hover previews when typechecking succeeds
- checked `std.registry` membership hover and decorator navigation when typechecking succeeds
- checked C binding and raw-call hover/navigation facts when typechecking succeeds
- Contract-backed model emit through `workspace/executeCommand` command `incan.metadata.model.emit`

The LSP keeps checked API metadata, registry-description facts, and checked C binding descriptors in memory for hover and definition navigation. It projects the same successful typecheck artifact rather than reparsing `@describe` syntax, reconstructing a C declaration from generated Rust, or querying linker state, so editor details cannot silently diverge from compiler validation. Full checked API metadata package retrieval remains a CLI surface through `incan tools metadata api`. Contract model emit can inspect project bundle metadata, bundle JSON files, or `.incnlib` artifacts through the explicit `incan.metadata.model.emit` command.
