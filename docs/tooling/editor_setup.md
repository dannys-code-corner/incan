# Editor & IDE Setup

> Note: Incan-specific tooling is in development. This guide will be updated as the tooling improves.

## Syntax Highlighting

### VS Code / Cursor (Recommended)

The Incan extension provides full syntax highlighting for `.incn` files.

**Installation:**

1. Copy the `editors/vscode/` folder to your VS Code extensions directory:

   ```bash
   # macOS/Linux
   cp -r editors/vscode ~/.vscode/extensions/incan-language
   
   # Or for Cursor
   cp -r editors/vscode ~/.cursor/extensions/incan-language
   ```

2. Restart VS Code/Cursor

3. Open any `.incn` file - syntax highlighting should work automatically

**Features:**

- Full syntax highlighting for all Incan constructs
- Keywords: `model`, `class`, `trait`, `enum`, `newtype`, `async`, `await`
- Rust-style operators: `?`, `::`
- F-string interpolation highlighting
- Type annotations
- Decorators (`@derive`, `@skip`, etc.)
- Markdown code block highlighting for `incan` language
- **LSP integration** - Diagnostics, hover, go-to-definition (requires incan-lsp)

See [`editors/vscode/README.md`](../../editors/vscode/README.md) for full details.

**Alternative**: Use Python highlighting

If you don't want to install the extension:

1. Open Settings (`Cmd/Ctrl` + `,`)
2. Search for "files.associations"
3. Add: `"*.incn": "python"`

> Note: Although most of the syntax for Incan is supported by Python, using Python highlighting will not provide the full Incan experience. It's a good way to 'get started' quickly without installing the extension.

### Vim/Neovim

Add to your config:

```vim
" Associate .incn files with Python syntax
autocmd BufNewFile,BufRead *.incn set filetype=python
```

### JetBrains IDEs (PyCharm, IntelliJ)

1. Go to Settings → Editor → File Types
2. Find "Python" and add `*.incn` to registered patterns

## Language Server (LSP)

The Incan Language Server provides IDE integration:

- **Real-time diagnostics** - See errors as you type
- **Hover information** - View types and signatures
- **Go-to-definition** - Jump to symbol definitions
- **Completions** - Keywords and symbols

See [Language Server](lsp.md) for setup instructions.

## Format on Save

### VS Code / Cursor

Once the Incan extension is available, format-on-save will be supported.

For now, you can set up a task or use a file watcher:

```json
// .vscode/tasks.json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "Format Incan",
      "type": "shell",
      "command": "incan fmt ${file}",
      "group": "build",
      "presentation": {
        "reveal": "silent"
      }
    }
  ]
}
```

## Recommended Extensions

- **Error Lens** - Inline error display
- **TODO Highlight** - Track TODOs in code

## Next Steps

- [Formatting Guide](formatting.md) - Code style and `incan fmt`
- [Language Guide](../guide/) - Learn Incan syntax and features
- [Examples](../../examples/) - Sample programs
