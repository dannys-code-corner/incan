# Binding inspection JSON schema

This page specifies the schema-versioned output of `incan inspect bindings --format json`. Use the [inspection how-to](../how-to/inspect_checked_c_bindings.md) to review a binding from the terminal and the [CLI reference](cli_reference.md#incan-inspect-bindings) for command options.

The top-level report has `schema_version: 1` and one `bindings` array:

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
      "resources": [],
      "symbols": [],
      "enums": [],
      "structs": []
    }
  ]
}
```

Bindings are sorted by module path, binding name, and source file. Members and parameters retain declaration order.

## Binding fields

| Field | Type | Meaning |
| --- | --- | --- |
| `module` | array of strings | Logical module path containing the declaration. |
| `name` | string | Binding name visible to Incan source. |
| `header` | string | Header spelling declared by the binding. |
| `system_library` | string | Logical system-library capability named by `c.system_library(...)`. |
| `source` | object | Source file and binding-declaration span. |
| `resources` | array | Opaque resource declarations and their release associations. |
| `symbols` | array | Raw symbol declarations. |
| `enums` | array | Enum carrier and native constant declarations. |
| `structs` | array | Plain C structure declarations. |

Each resource has `name`, `native`, and `release` strings. Each symbol has its Incan `name`, native linker spelling in `native`, named `parameters`, structural `return_type`, and declared `outcomes`. An outcome contains the binding-local result spelling and the output-position names in `initializes`, `updates`, and `invalidates`.

An enum has a binding-local `name`, a canonical scalar `carrier`, and `variants` containing Incan `name` and C constant `native` spellings. A structure has a binding-local `name`, its C spelling in `native`, and named structural `fields`.

## Structural types

Types use a `kind` discriminator and preserve the checked C vocabulary rather than generated-Rust text:

| `kind` | Additional fields | Meaning |
| --- | --- | --- |
| `scalar` | `spelling` | Canonical C vocabulary spelling such as `c.i32`. |
| `pointer` | `mutable`, `pointee` | Required C pointer and its nested pointee type. |
| `struct` | `name` | Binding-local plain structure. |
| `resource` | `access`, `resource` | Opaque resource with `owned`, `borrowed`, or `borrowed_mut` access. |
| `output` | `mode`, `value` | Compiler-managed `out` or `in_out` storage and its nested value type. |
| `nullable` | `value` | Nullable form of the nested resource type. |
| `void` | none | C `void` result. |

## Source spans

The `source` object carries `file`, inclusive `start` and exclusive `end` byte offsets, and 1-based `start_line`, `start_column`, `end_line`, and `end_column` positions. Consumers should retain both coordinate forms: byte offsets anchor the inspected source buffer, while line and column values are appropriate for display.

## Compatibility

Consumers must check `schema_version` before interpreting the report. Schema version 1 is a strict declaration projection from a successful compilation analysis. It does not contain a reusable target-verification receipt, resolved Oven artifact plan, generated bridge/façade relationship, or editor index.
