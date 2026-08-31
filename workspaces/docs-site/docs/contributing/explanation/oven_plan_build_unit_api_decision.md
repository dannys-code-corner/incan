# Oven plan/build-unit/artifact/receipt API — extraction decision

This is the design-decision record required by [#1094](https://github.com/encero-systems/incan/issues/1094): how Oven's plan-selection, build-unit, artifact, and receipt logic leaves `src/cli/commands/build.rs` and becomes part of Oven's own operational API. It is a decision document, not an extraction changelog — no implementation code moves as part of landing this document. The actual code move is filed as an independent follow-up (see [Follow-up work](#follow-up-work)).

## Status

Decided, pending maintainer review via the PR that adds this document. Cross-slice prerequisite for [Slice 5](https://github.com/encero-systems/incan/issues/1141) ("Oven planning/API seams" is explicitly in its scope) and [Slice 6](https://github.com/encero-systems/incan/issues/1142) (names the shared planning API as [#1010](https://github.com/encero-systems/incan/issues/1010)'s direct blocking prerequisite). Coordinates with, and does not duplicate, [#1095](https://github.com/encero-systems/incan/issues/1095) (`test_runner/execution.rs` shim-vs-core blur — explicitly scoped to apply this decision's API once it lands).

## Problem

Per the [Oven and toolchain ownership map](oven_toolchain_ownership_map.md), `src/cli/commands/build.rs` is 19,276 lines (17,328 when #1094 was filed; the file has grown further while this decision was pending). It backs `incan build`, `incan run`, and `incan inspect rust`. RFC 118 requires these to be a "project-scale convenience" that delegates shallowly to Oven's plan, scheduler, bake, and receipt — but roughly 90% of the file's non-test code *is* that plan-selection, build-unit, artifact, and receipt logic itself, not a thin delegation call. The same blur exists in `src/cli/test_runner/execution.rs`, which calls into `build.rs`'s plan-selection functions but also independently reconstructs build-unit inputs rather than delegating cleanly.

RFC 118 states the target shape directly:

> **Oven API and operational core** — must own planning, resolver/provider choice, policy, target selection, scheduling, artifacts, receipts, caches, stores, leases, and crash-safe publication through language-neutral owned snapshots and opaque handles.

and:

> Require project-scale `incan run` and `incan test` conveniences to delegate shallowly to the same Oven plan, scheduler, bake, `*.loaf` asset, and receipt as the canonical Oven operation.

This document decides where that ownership boundary sits concretely, so `build.rs` and `execution.rs` can both become thin callers against it.

## Non-goals (hard boundaries carried from #1094)

- **No code move in this change.** This document is the decision; the move is a separate, independently reviewable follow-up.
- **Cargo, generated Rust, and `build.rs` gain no semantic authority.** The decision moves logic that is already Oven-owned into an Oven-owned home; it does not grant new authority to any of these.
- **No second planner.** Every plan-selection function this document discusses already belongs to Oven conceptually (see [Current state](#current-state)); this is a relocation of existing Oven logic, not a new planning system alongside Oven's.
- **Does not preempt [#1008](https://github.com/encero-systems/incan/issues/1008) (Loaf envelope) or [#1029](https://github.com/encero-systems/incan/issues/1029) (authority contract).** Both are named as required future inputs to the API boundary this document defines (see [Inputs this decision does not invent](#inputs-this-decision-does-not-invent)), not designed here.

## Current state

### `src/oven` already exists as the Oven operational core

`src/oven.rs` plus `src/oven/{store,rustc,loaf,legacy_cargo,native_test,interop,process,compiler_suite_env}.rs` (39,437 lines total) is declared via `pub mod oven;` in `src/lib.rs`. It already owns `OvenReceipt`, `OvenStore`/`OvenStoreEntry`/`OvenStoreLease`, `OvenRustcArtifactPlan`, `OvenRustcArtifactManifest`, `OvenToolchainLoaf`, and related low-level plan/receipt/store types. 328 existing `crate::oven::` call sites span 19 files across the compiler. `src/oven.rs`'s own module doc already states its charter:

> This small compiler-owned Rust kernel is temporary implementation debt scoped to the tracked Oven Alpha (#1005, #975)... It must remain a narrow, removable boundary rather than growing into a Rust orchestration layer for the product workflow.

**This means the extraction target is not a new module decision.** `src/oven` is the one Oven API and operational core layer RFC 118 defines. The question this document answers is where *inside or beside* that existing layer the plan-selection/build-unit composition logic belongs, not whether to invent a second Oven module.

### `build.rs` is already a de facto API, just filed under the wrong module

25 functions in `build.rs` are already `pub(crate)`, and 12 of them are already called from outside `build.rs` today — from `src/cli/commands/lock.rs`, `src/cli/commands/oven.rs`, and `src/cli/test_runner/execution.rs`:

| Function | Line | Already called from |
|---|---|---|
| `select_oven_direct_rustc_plan` | 6026 | `execution.rs:2616` |
| `oven_build_unit_inputs` | 3600 | `lock.rs:1261` |
| `bake_oven_project_targets` | 13330 | `oven.rs:411` |
| `rematerialize_caller_owned_libraries` | 5246 | `execution.rs:2664` |
| `promoted_oven_test_dependencies` | 6199 | `lock.rs:989`, `execution.rs:1125` |
| `oven_caller_owned_libraries` | 3727 | `execution.rs:2313` |
| `append_oven_interop_execution_build_inputs` | 3619 | `execution.rs:2351` |
| `declared_rust_libraries_missing_from_selected_plan` | 5378 | `execution.rs:2652` |
| `compiler_selected_path_authority` | 4756 | `execution.rs:2659` |
| `has_caller_owned_project_libraries` | 3717 | `execution.rs:2661` |
| `replace_caller_owned_package_libraries` | 5271 | `execution.rs:2677` |
| `oven_native_provider_records` | 3667 | `execution.rs:2797` |
| `load_current_project_registry_source_authorities` | 8147 | `execution.rs:89` |

These are the five functions the issue names verbatim, plus their seven direct dependents. The relocation this document decides on is exactly these signatures moving module, not a redesign — the call sites already exist and already cross a module boundary; only *which* module owns the callee changes.

### Function inventory: what is Oven-owned vs. CLI-only

Classifying every top-level function in `build.rs` (verified against source, not just the ownership map's prose):

**Oven-owned (moves)** — roughly 190 functions, ~90% of non-test code. Grouped by theme:

- Plan selection/composition (`build.rs:1110`–`2201`): `select_receipt_direct_rustc_execution_plan`, `select_packaged_direct_rustc_execution_plan`, `select_receipt_project_extension_execution_plan`, `project_extension_execution_plan_from_selected`, `select_published_project_extension_plan`, `select_published_project_plan`, `select_packaged_provider_plan`, `compose_packaged_provider_plan`, `compose_direct_packaged_provider_plan`, plus `OvenDirectRustcPlanSelection`/`OvenPackagedProviderExecutionPlan` and their impl methods.
- The five functions named in #1094 and their direct helpers (`build.rs:3600`–`6634`, see table above).
- Core project preparation: `prepare_oven_project` (`build.rs:5463`, ~480 lines) and `prepare_library_project` (`build.rs:9639`, ~1050 lines) — the two largest functions in the file, and the actual end-to-end build-unit construction core.
- Caller-owned library/receipt handling (`build.rs:3796`–`5442`): registry-leaf authority, caller-owned provider graph rematerialization, packaged-provider profile checking.
- Loaf/test-dependency machinery (`build.rs:5946`–`6547`): generated-project compatibility planning, test-dependency envelope preparation.
- Artifact/output path & materialization (`build.rs:6717`–`9038`, `12121`–`13292`): output path resolution, project-output publication, packaged-library Loaf manifest read/write, `bake_oven_project`/`bake_oven_library`.
- Source/lock digesting for receipt authority (`build.rs:11554`–`11940`): `ProjectSourceAuthorityDigester` and its dependents.
- `OvenProjectBakeAuthorityContext` impl (`build.rs:12847`–`13061`).

**CLI-only (stays)**: `build_file`/`build_library` (top-level entry points — arg validation, delegate, print, exit-code mapping), `run_file`/`run_inline_source`, `inspect_backend_selection`/`inspect_rust`, flag/option validation (`reject_normal_cargo_controls`, `ensure_backend_request_available`), user-facing diagnostic rendering (`classify_rust_extern_build_failure`, `format_rust_extern_wrapped_diagnostics`), progress/timing output (`print_build_progress`, `record_timing`), `incan run -c <source>` inline-command handling.

**Ambiguous — flagged, decided explicitly below**:

- `build_file_report`/`build_library_report` (`build.rs:9519`, `13681`) — the largest orchestration entry points. They currently call almost everything in the Oven-owned bucket inline while also assembling the CLI-facing `BuildReport` struct. **Decision**: these split. The plan-select → materialize/bake → receipt sequence becomes calls into the new API; only `BuildReport` field assembly stays in `build.rs`. This is exactly the "thin delegation call" RFC 118 requires the command surface to become.
- `backend_shadow_comparison`, `select_and_resolve_backend`, `finalize_backend_receipt`, `write_backend_receipt` (`build.rs:2346`–`8551`) — these operate on `BackendExecutionReceipt` (`src/backend/selection.rs`), a **different receipt concept** from `OvenReceipt` (codegen-backend-choice, not Oven build/publish). **Decision**: these stay CLI-side and are explicitly out of scope for this extraction. Do not conflate the two receipt systems into one type during the follow-up move — this is the single highest-risk naming collision found in this inventory.
- The provider-metadata projection block (`build.rs:10695`–`11520`: `synchronize_projected_provider_dependencies`, `compiled_provider_metadata`, and ~12 related functions) produces `CompiledProviderMetadata`, which is defined in `src/library_manifest/model.rs`, not in `build.rs` or `src/oven`. **Decision**: explicitly out of scope for this extraction — it is provider/library-manifest semantic logic, not Oven plan/build-unit/receipt logic, and no existing #1034 sub-issue covers its placement. Not filing a new issue for it as part of this decision (see [Follow-up work](#follow-up-work)); flagged for the maintainer to decide whether it warrants one.
- `LibraryReexportResolver` (`build.rs:3051`–`3251`) — semantic re-export resolution over checked AST. **Decision**: stays with frontend/library-manifest concerns; out of scope for the same reason as above.

### The concrete duplication this decision resolves

`execution.rs:2792` defines `oven_test_build_unit_inputs`, which is structurally identical to `build.rs:3600`'s `oven_build_unit_inputs` — same three steps (native provider records → dependency-spec digest → `runtime_build_unit_inputs`) — but is a separate copy rather than a call to it. The only real difference is the error type: `build.rs`'s version returns `CliResult<_>` (wraps `CliError`); `execution.rs`'s version returns `Result<_, String>` (the test runner's batch-result plumbing). **This mismatch in error-handling convention between the CLI-command layer and the test-runner layer is why the duplicate exists.** It is concrete, verified evidence — not a hypothetical risk — for [Error and diagnostic responsibility](#error-and-diagnostic-responsibility) below.

`execution.rs` also directly imports low-level `src/oven` primitives (`runtime_build_unit_inputs`, `direct_rustc_compile_environment`, `OvenStore`, `OvenStoreLimits`) and opens the store itself via `commands::oven::open_default_oven_store()`, bypassing `build.rs` for some things while calling through it for others. That inconsistency is itself evidence for consolidating the plan/build-unit surface into one place both callers use uniformly, rather than leaving each caller to decide per-function whether to go through `build.rs` or around it.

## Decision

### Module home

The Oven-owned functions and types inventoried above move into a new submodule of the existing `src/oven` module — **`src/oven/plan.rs`** (exact name is an implementation-follow-up detail; the constraint decided here is "inside `src/oven`, not a new sibling module"). Rationale:

- RFC 118 defines exactly one "Oven API and operational core" layer. `src/oven` is that layer today. Creating a second Rust module for "the plan/build-unit API" would fragment the one operational authority RFC 118's "one operational authority" principle requires — every project operation must produce the same Oven plan, policy decision, target, asset, and receipt regardless of which command reached it, which is easiest to guarantee when there's one module, not two, that can decide those things.
- The low-level types this logic already composes (`OvenRustcArtifactPlan`, `OvenRustcArtifactManifest`, `OvenReceipt`, `OvenStore`) already live in `src/oven`. The plan-selection/build-unit layer misplaced in `build.rs` is a composition layer *above* those types, not an unrelated concern — it belongs beside what it already depends on.
- `src/oven.rs`'s existing module doc frames the kernel as intentionally narrow. Adding a `plan` submodule does not change that charter — it relocates logic that is already conceptually Oven's, it does not grow Oven's scope into new responsibility.

### API surface

The target API is the same 12 already-`pub(crate)`, already-cross-file-consumed functions in the table above, relocated with their signatures preserved wherever possible, plus the supporting functions they depend on (the ~190-function Oven-owned bucket). This is deliberately not a from-scratch API redesign: the call sites already exist and already cross a module boundary from three different files (`lock.rs`, `oven.rs`, `execution.rs`); the follow-up changes *which* module owns the callee, not the call shape at each existing site. Concretely, callers migrate from:

```rust
use crate::cli::commands::build::{select_oven_direct_rustc_plan, oven_build_unit_inputs, /* ... */};
```

to:

```rust
use crate::oven::plan::{select_direct_rustc_plan, build_unit_inputs, /* ... */};
```

`build_file_report`/`build_library_report` in `build.rs` become the primary consumers of this API for the `build`/`run`/`inspect rust` path; `execution.rs` becomes a direct consumer for the `test` path, deleting its duplicate `oven_test_build_unit_inputs` in favor of calling the relocated `build_unit_inputs` directly (see [Error and diagnostic responsibility](#error-and-diagnostic-responsibility) for how the error-type mismatch that caused the duplicate gets resolved).

### Ownership

- **Oven's plan module (`src/oven/plan.rs`) owns**: plan selection and composition, build-unit input construction, target materialize/bake, caller-owned library/provider-graph rematerialization, and construction of the receipt-shaped return types (`OvenBakeProjectTarget`, `OvenStoredProjectOutput`, `OvenStoredProjectExtensionExecutionPlan`, and the ~15 related ad hoc structs currently at `build.rs:593`–`1101`). These structs move alongside the functions that construct them — they are the parameter/return types of the moved functions, not a new receipt kind requiring separate design.
- **`build.rs` and `execution.rs` own**: argument parsing and validation, calling the Oven plan API, mapping its domain errors to their own presentation (`CliError`/`ExitCode` for `build.rs`, batch-result strings for `execution.rs`), rendering progress/diagnostics text, and assembling their own report structs (`BuildReport`) from the API's return values.
- **`src/backend/selection.rs` (unchanged, explicitly out of scope)**: `BackendExecutionReceipt` and the codegen-backend-choice logic that produces it stay where they are. This decision does not touch them, and the follow-up move must not merge `BackendExecutionReceipt` into `OvenReceipt` or vice versa.

### Error and diagnostic responsibility

The Oven plan API returns one domain error type (a `thiserror` enum, e.g. `OvenPlanError`), not `CliResult`/`CliError` and not `Result<_, String>`. Each caller converts that domain error into its own local presentation at the call site: `build.rs` maps it into `CliError`/`ExitCode` as it does today; `execution.rs` maps it into its existing `Result<_, String>` batch-result convention. This directly removes the reason `oven_test_build_unit_inputs` was duplicated instead of calling `oven_build_unit_inputs` — the duplicate existed because the shared logic lived inside a `CliError`-typed file, so a caller with a different error convention had to either wrap that dependency or reimplement it, and reimplementation won. A domain-error-returning API removes that fork: both callers now do the same one-line `.map_err(...)` at their own boundary instead of choosing between wrapping and duplicating.

Diagnostics (rustc/build failure formatting, user-facing text) stay CLI-side: `classify_rust_extern_build_failure` and `format_rust_extern_wrapped_diagnostics` are presentation over Oven's domain errors, not part of the domain error itself, and remain in `build.rs`/`execution.rs` respectively (or their own presentation-layer module if the follow-up finds that cleaner — not decided here).

### Receipt flow

`OvenReceipt` (`src/oven.rs`) remains the single Oven receipt authority; nothing here creates a second one. The plan/build-unit API's job is to select, materialize, and bake toward that receipt — its return types feed a receipt, they are not a competing receipt format. `BackendExecutionReceipt` remains a separate, unrelated concept (codegen backend selection, not Oven build/publish) and is explicitly not touched by this decision or its follow-up.

### Migration seams (phased, for the follow-up issue to sequence)

1. Move the ~190 Oven-owned functions and their ad hoc types from `build.rs` into `src/oven/plan.rs`, preserving the 12 already-`pub(crate)` signatures so `lock.rs`/`oven.rs`/`execution.rs` need only an import-path change at first, not a behavior change. Domain error type introduced here.
2. `execution.rs` deletes `oven_test_build_unit_inputs` and calls the relocated `build_unit_inputs` directly, mapping the new domain error to its existing `Result<_, String>` convention at the call site.
3. `build_file_report`/`build_library_report` are split so plan-select/materialize/bake/receipt calls go through the new API and only `BuildReport` field assembly remains local to `build.rs`.
4. `promoted_oven_test_dependencies` (currently only in the named-functions table) and any remaining direct `src/oven` primitive imports in `execution.rs` (`runtime_build_unit_inputs`, `direct_rustc_compile_environment`, `OvenStore`, `OvenStoreLimits`) are reconciled so `execution.rs` consistently goes through the plan API rather than sometimes bypassing it — this closes the inconsistency noted in [Current state](#current-state).

This phasing is a recommendation for the follow-up issue's own planning, not a commitment made by this document — the follow-up owns its own sequencing.

## Inputs this decision does not invent

Per #1094's explicit hard boundary, the API's request/response shape must accept these two prerequisites' outputs once they land, rather than this decision inventing substitutes:

- **[#1008](https://github.com/encero-systems/incan/issues/1008) — Loaf envelope.** The plan API's request types accept whatever project/target/profile selection `Loaf.toml`/`Oven.lock` define once #1008 lands. Today, `build.rs` builds these from `incan.toml`-derived `ProjectRequirements`; that is an existing input shape this decision does not redesign. The new module boundary is exactly where #1008's eventual input-shape change lands without requiring a second edit to `build.rs`/`execution.rs`.
- **[#1029](https://github.com/encero-systems/incan/issues/1029) — authority context.** The API's request/response types should be able to carry whatever authority-context/capability-grant handles #1029 defines for governed execution. `build.rs` already has authority-shaped concepts today (`OvenProjectBakeAuthorityContext`, `caller_owned_provider_registry_leaf_authority`, `ProjectSourceAuthorityDigester`) — flagged here as the most likely seam #1029's contract will thread through, but this decision does not assume or design that contract's exact shape.

## Follow-up work

- **[#1266](https://github.com/encero-systems/incan/issues/1266)** (new, narrowly scoped, independently closable, filed as a sub-issue of #1094): relocate the ~190 Oven-owned functions and their types from `build.rs` into `src/oven/plan.rs` per [Migration seams](#migration-seams-phased-for-the-follow-up-issue-to-sequence) above, preserving the 12 already-`pub(crate)` call sites' signatures and introducing the shared domain error type. See the issue itself for its full acceptance contract.
- **[#1095](https://github.com/encero-systems/incan/issues/1095)** (existing, not duplicated here): already scoped to apply this decision's API to `execution.rs` and remove its duplicated build-unit construction once the code-move issue above lands. No change needed to its own scope text; it is now unblocked by this decision.
- **Not filed as part of this decision**: the provider-metadata projection block (`build.rs:10695`–`11520`, `CompiledProviderMetadata`) and `LibraryReexportResolver` (`build.rs:3051`–`3251`) are explicitly out of scope for the Oven plan/build-unit extraction — they are provider/library-manifest semantic logic that happens to sit in `build.rs`, not Oven plan/receipt logic. No existing #1034 sub-issue covers their placement. Flagged for the maintainer to decide whether a separate ownership decision is warranted; not unilaterally filed here since it is tangential to #1094's scope.

## #1010 readiness

[#1010](https://github.com/encero-systems/incan/issues/1010)'s plan step 1 currently reads "Sequence the command work as follows: 1. #1094 and #1097 decide and extract the shared Oven plan/build-unit API..." Once this document and its follow-up code-move issue (#1266) are accepted, #1010's dependency wording should record that the *decision* half of that step is complete for the `build.rs`/`execution.rs` axis (the boundary is `src/oven/plan.rs`, API surface is the 12-function table above plus supporting functions, domain error is a new `thiserror` enum), while the *extraction* half remains tracked by #1266 and #1095. #1097 (the `commands/common.rs` decomposition decision) is a separate, still-open decision this document does not resolve.
