# Projects today (current reality)

This page explains how “projects” work in Incan today, and what is planned next.

## The current model

Incan source files (`.incn`) are the source of truth.

When you run:

```bash
incan build path/to/main.incn
```

Incan generates a Rust project and builds it via Cargo.

--8<-- "_snippets/callouts/no_install_fallback.md"

## Where outputs go

On build, Incan prints the generated project directory and the compiled binary path. In practice, outputs live under:

- Generated Rust project: `target/incan/<name>/`
- Built binary: `target/incan/<name>/target/release/<name>`

See: [CLI reference](../reference/cli_reference.md).

## What gets regenerated

The generated Rust project under `target/incan/` is tool-managed output. Treat it as **generated**:

- It is safe to delete: `rm -rf target/incan/`
- Manual edits inside `target/incan/<name>/` may be overwritten on the next build

## Dependencies (today)

- For `rust::` imports, the toolchain uses a curated “known-good” mapping for versions/features.
- If a crate is not in the known-good list, you’ll need to request adding it (issue/PR) until the dependency system is
  expanded.

See: [Rust Interop](../../language/how-to/rust_interop.md) and [RFC 013](../../RFCs/013_rust_crate_dependencies.md).

## What’s planned

Future work aims to make the project lifecycle explicit and reproducible:

- `incan.toml` project config + lockfiles: [RFC 013](../../RFCs/013_rust_crate_dependencies.md)
- “Project lifecycle” commands (`incan init/new/doctor`): [RFC 015](../../RFCs/015_hatch_like_tooling.md)
