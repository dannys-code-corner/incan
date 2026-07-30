# Inspect checked C bindings

Use `incan inspect bindings` when you need to review the raw C contract accepted by the compiler without reading generated Rust or reconstructing facts from the original vocabulary syntax.

## Review a project from the terminal

Run the command from the project root:

```bash
incan inspect bindings
```

The default text output lists each checked binding with its source module, header, logical system-library capability, source location, resources, symbols, output outcomes, enums, and plain structures. This is the quickest form for reviewing whether a private binding says what its public façade expects.

Pass a source file when the binding is not reachable from the project's normal entrypoint:

```bash
incan inspect bindings src/sqlite.incn
```

When `PATH` is a directory, inspection selects `src/main.incn` when it exists and otherwise `src/lib.incn`. Local imports reachable from that entrypoint are included through the ordinary checked module graph.

## Produce JSON for another tool

Use the JSON format for an editor integration, audit tool, or reproducible review artifact:

```bash
incan inspect bindings . --format json
```

The report includes `schema_version: 1`, deterministic binding ordering, structural C types, and source anchors. Consumers should read the documented fields rather than parsing text output. See the [binding inspection JSON schema](../reference/binding_inspection_schema.md) for the exact contract.

## Match the project's selected source view

Pass the same package features and SDK profile that the build or review uses when declarations are conditional:

```bash
incan inspect bindings . --format json --features sqlite --sdk-profile minimal
```

Inspection applies the normal package-feature and SDK-profile selection rules before typechecking. The result therefore describes the selected source graph rather than every declaration that could exist under a different configuration.

## Diagnose a failed inspection

Inspection is strict. If parsing, imports, typechecking, or the host-target C probe fails, the command emits the ordinary compiler diagnostic and no partial report.

Use the diagnostic to correct the binding declaration or selected environment, then run the inspection again. Do not weaken an exact C type, ownership mode, output contract, enum carrier, or plain layout merely to obtain JSON: a partial or guessed report would defeat the purpose of the command.

## Know where the report stops

The report describes the checked language declaration. It does not read `incan.lock`, resolve or download artifacts, compile C/C++ shims, select a concrete compiler or SDK, classify a private bridge and public façade, or provide editor navigation. `incan inspect codegraph` additionally marks the checked binding and direct C calls admitted inside `unsafe:`, but it has the same boundary: those are language-contract facts, not artifact, runtime, façade, or editor receipts. Use `[oven.interop]` and `incan lock` for declared physical requirements and locked package inputs; later Oven resolution and editor projections have separate lifecycles.

For the language-side recipes, see [Work with checked C bindings](../../language/how-to/checked_c_bindings.md). For the reason these facts stay separate, see [How checked C interop is structured](../../language/explanation/checked_c_interop.md).
