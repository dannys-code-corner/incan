# Incan Language Server (LSP)

The Incan Language Server provides IDE integration for real-time feedback while coding.

## Features

| Feature              | Description                                        |
| -------------------- | -------------------------------------------------- |
| **Diagnostics**      | Real-time errors, warnings, and hints as you type  |
| **Hover**            | View function signatures, types, and documentation |
| **Go-to-Definition** | Jump to symbol definitions (Cmd/Ctrl + Click)      |
| **Completions**      | Autocomplete for keywords and symbols              |

## Installation

### 1. Build the LSP Server

```bash
cd /path/to/incan-programming-language
make lsp
```

### 2. Add to PATH

Add the binary to your PATH:

```bash
# Add to .bashrc, .zshrc, or your shell profile
export PATH="$PATH:/path/to/incan-programming-language/target/release"
```

Or symlink it:

```bash
sudo ln -s /path/to/incan-programming-language/target/release/incan-lsp /usr/local/bin/
```

### 3. Install VS Code Extension

See [Editor Setup](editor_setup.md) for VS Code/Cursor extension installation.

## Usage

Once installed, the LSP activates automatically when you open `.incn` files.

### Real-time Diagnostics

Errors appear as you type with helpful hints:

```bash
type error: Type mismatch: expected 'Result[str, str]', found 'str'
  --> file.incn:8:5

note: In Incan, functions that can fail return Result[T, E]
hint: Wrap the value with Ok(...) to return success
```

### Hover Information

Hover over any symbol to see its type:

```incan
def process(data: List[str]) -> Result[int, Error]
```

### Go-to-Definition

- **VS Code/Cursor**: Cmd+Click (macOS) or Ctrl+Click (Windows/Linux)
- **Keyboard**: F12 or Ctrl+Click

Works for:

- Functions
- Models
- Classes
- Traits
- Enums
- Newtypes

### Completions

Trigger completions with Ctrl+Space or by typing:

- `.` for field/method access
- `:` for type annotations

Suggestions include:

- Incan keywords (`def`, `model`, `class`, etc.)
- Symbols from current file
- Built-in types (`Result`, `Option`, etc.)

## Configuration

### VS Code Settings

```json
{
  "incan.lsp.enabled": true,
  "incan.lsp.path": "/path/to/incan-lsp"
}
```

| Setting             | Default | Description                                   |
| ------------------- | ------- | --------------------------------------------- |
| `incan.lsp.enabled` | `true`  | Enable/disable the language server            |
| `incan.lsp.path`    | `""`    | Custom path to incan-lsp (uses PATH if empty) |

## Troubleshooting

### LSP Not Starting

1. **Check binary exists:**

      ```bash
      which incan-lsp
      # or
      incan-lsp --version
      ```

2. **Check VS Code output:**
      - View → Output → Select "Incan Language Server"

3. **Verify extension is active:**
      - Extensions panel → Search "Incan" → Check it's enabled

### No Diagnostics

- Ensure the file has `.incn` extension
- Check for syntax errors that prevent parsing
- Try reloading the window (Cmd/Ctrl + Shift + P → "Reload Window")

### Hover Not Working

- LSP must successfully parse the file first
- Check for diagnostics/errors in the file
- Ensure cursor is on a symbol (function name, type name, etc.)

## See also

- Architecture: [LSP architecture](../explanation/lsp_architecture.md)
- Reference: [LSP protocol support](../reference/lsp_protocol_support.md)
