# Checked C binding inspection

`incan inspect bindings` exposes the C declaration contracts that the compiler has already accepted from the source module graph. It is the inspection surface for a binding’s Incan name, header spelling, logical system-library capability, symbols, exact C types, enum constants, plain structures, and source anchor. It consumes the same checked descriptor used by the compiler; it does not scrape `.incn` source or generated Rust to reconstruct those facts.

Use the command when a tool, a review, or a package author needs to see what the checked declaration says before considering target preparation or a build.

```bash
incan inspect bindings
incan inspect bindings src/main.incn --format text
incan inspect bindings . --format json --features sqlite --sdk-profile minimal
```

`PATH` may be an Incan source file or a project directory. For a project directory, the command uses `src/main.incn` when present and otherwise `src/lib.incn`; imports reachable from that entrypoint are analysed through the ordinary checked module graph. `text` is the default concise terminal summary. `json` is the tool-facing report and has `schema_version: 1`.

## JSON contract

The report has this shape:

```json
{
  "schema_version": 1,
  "bindings": [
    {
      "module": ["sqlite"],
      "name": "SQLite",
      "header": "sqlite3.h",
      "system_library": "sqlite3",
      "source": {
        "file": "/workspace/src/sqlite.incn",
        "start": 0,
        "end": 234,
        "start_line": 1,
        "start_column": 1,
        "end_line": 12,
        "end_column": 1
      },
      "symbols": [],
      "enums": [],
      "structs": []
    }
  ]
}
```

Bindings are sorted by module path and binding name. A symbol reports its source name, native linker spelling, named parameters, and return type. Enum reports retain the declared scalar carrier and native constant spellings. Structure reports retain their native C spelling and declared field contracts.

Each reported type is structural rather than generated-Rust text. A scalar has `kind: "scalar"` and its canonical C vocabulary `spelling`, such as `c.i32`. A pointer has `kind: "pointer"`, a `mutable` flag, and a nested `pointee`. A plain structure has `kind: "struct"` and a binding-local `name`; `kind: "void"` represents a C `void` result.

The `source` object carries inclusive `start` and exclusive `end` byte offsets, plus 1-based line and column positions for display. Tooling should retain both forms: byte offsets anchor an editor buffer precisely, while line and column values are easier to present to people.

## Strict checked boundary

The command typechecks the selected source graph before emitting a report. A malformed or unsupported binding therefore returns ordinary compiler diagnostics instead of partial descriptor facts. Feature and SDK-profile flags use the same project selection rules as the other semantic inspection commands, so a feature-conditioned declaration is inspected in the same selected source view that the compiler checks.

This is a declaration inspection surface, not a reusable build receipt. The shared checked compilation path runs the ordinary host-target Clang probe before a report is emitted, so an invalid header, signature, enum carrier, enum value, or plain layout fails with the normal compiler diagnostic. The JSON report intentionally contains the source declaration facts rather than a toolchain-keyed verification receipt; it does not report the selected toolchain identity or publish a cross-invocation verification cache entry.

The command also does not read `incan.lock`, resolve/download native artifacts, bake C or C++ shims, classify private bridges or public façades, or provide LSP navigation and hover data. Those facts have distinct package-graph or editor lifecycles and will be projected by later RFC 116 tooling work rather than inferred from a declaration report.

For the language contract, see [the `std.interop` reference](../../language/reference/stdlib/interop.md). For C versus Rust interop and the architecture around the binding, see [How checked C interop is structured](../../language/explanation/checked_c_interop.md).
