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
