# Binding inspection JSON schema

This page specifies the schema-versioned output of `incan inspect bindings --format json`. Use the [inspection how-to](../how-to/inspect_checked_c_bindings.md) to review a binding from the terminal and the [CLI reference](cli_reference.md#incan-inspect-bindings) for command options.

The top-level report has `schema_version: 2` and one `bindings` array:

```json
{
  "schema_version": 2,
  "bindings": [
    {
      "module": ["sqlite"],
      "identity": "sha256:checked-descriptor-digest",
      "name": "SQLite",
      "header": "sqlite3.h",
      "system_library": "sqlite3",
      "link_capability": "system_library",
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
| `identity` | string | Compiler digest of the complete checked descriptor contract. It excludes source spans and source-file locations, but includes the declared header spelling; use a portable header spelling when the identity must survive relocation. It changes when an ABI-affecting declaration fact changes. |
| `name` | string | Binding name visible to Incan source. |
| `header` | string | Header spelling declared by the binding. |
| `system_library` | string | Stable historical field for the logical native link name. Its interpretation is qualified by `link_capability`. |
| `link_capability` | string | Exact checked link kind: `system_library` for `c.system_library(...)`, or `framework` for `c.framework(...)`. |
| `source` | object | Source file and binding-declaration span. |
| `resources` | array | Opaque resource declarations and their release associations. |
| `symbols` | array | Raw symbol declarations. |
| `enums` | array | Enum carrier and native constant declarations. |
| `structs` | array | Plain C structure declarations. |

Each resource has `name`, `native`, and `release` strings. Each symbol has its Incan `name`, native linker spelling in `native`, named `parameters`, structural `return_type`, descriptor-owned `buffers`, and declared `outcomes`. A buffer record has `pointer_parameter`, `length_parameter`, and exact scalar `element` spelling; it records a checked span association rather than a guessed relationship from names or generated Rust. An outcome contains the binding-local result spelling and the output-position names in `initializes`, `updates`, and `invalidates`.

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

## Binding-use receipt

`incan inspect bindings --format receipt` is a separate, redaction-safe JSON contract with `schema_version: 2`. It contains no source anchors, header spellings, package paths, argument values, native bytes, or process-local addresses:

```json
{
  "schema_version": 2,
  "compatibility": {
    "binding_contract": "exact_descriptor_identity",
    "target_contract": "exact_locked_target_identity"
  },
  "target": {
    "target": "aarch64-apple-darwin",
    "locked_target_identity": "sha256:locked-target-digest",
    "selected_execution_identity": "sha256:optional-selected-receipt"
  },
  "bindings": [
    {
      "module": ["sqlite"],
      "name": "SQLite",
      "identity": "sha256:checked-descriptor-digest",
      "target_artifacts": ["sqlite"]
    }
  ],
  "calls": [
    {
      "binding_identity": "sha256:checked-descriptor-digest",
      "symbol": "sqlite3_open_v2",
      "owner": { "name": "open_database", "visibility": "private" }
    }
  ],
  "facades": [
    {
      "facade": { "name": "open", "visibility": "public" },
      "bridge": { "name": "open_database", "visibility": "private" },
      "calls": [
        {
          "binding_identity": "sha256:checked-descriptor-digest",
          "symbol": "sqlite3_open_v2"
        }
      ]
    }
  ]
}
```

`compatibility.binding_contract` is always `exact_descriptor_identity`: v0.5 treats any descriptor identity change as an ABI or ownership contract change. With `--target`, `compatibility.target_contract` is `exact_locked_target_identity`: a changed locked target identity is a changed selected target/artifact contract. The policy is intentionally exact until a future compiler-owned compatibility classifier can prove a narrower safe relation.

`target` is omitted without `--target`. With a target, `locked_target_identity` is the portable digest of the exact canonical locked target requirements. `selected_execution_identity` is present only when a current selected Oven interop execution receipt already exists and validates against that target. `target_artifacts` is present only when the selected target explicitly maps that exact checked module/name pair to declared artifact names; it is not inferred from a header, a library name, generated Rust, or a path. A call's `owner` is omitted when compiler checking cannot retain a named function owner; consumers must not infer one from source layout or naming. A `facades` entry is emitted only when the typechecker proves that a public function directly calls a private function in the same module and that bridge owns one or more checked raw calls. It lists the bridge's identity-linked raw calls, but does not infer transitive, imported, re-exported, method, or whole-package relationships.

The receipt establishes checked declaration and direct-call usage, compiler-proven direct façade-to-bridge edges, and optionally joins them to a target selection. When the package explicitly declares a correspondence, it reports the selected artifact names for that exact compiler-checked binding. It does **not** infer or report a generated bridge implementation, linker invocation, physical artifact path, or platform package.

## Compatibility

Consumers must check `schema_version` before interpreting the report. The declaration report and binding-use receipt are separately versioned; both currently use version 2. They are strict projections from a successful compilation analysis. Fields may be added additively within the same schema version; consumers must ignore fields they do not recognize. They do not contain a reusable target-verification receipt, resolved Oven artifact plan, generated bridge implementation, whole-package facade classification, or editor index.
