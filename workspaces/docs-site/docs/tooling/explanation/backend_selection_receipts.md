# Backend selection & execution receipts

The v0.6 replacement-backend cutover ([#652](https://github.com/encero-systems/incan/issues/652)) introduces a second compiler backend: the Body IR replacement backend tracked by [#653](https://github.com/encero-systems/incan/issues/653), alongside the current Rust-source-emission backend (`IrCodegen`, `src/backend/ir/`) that this document calls the legacy backend. #988 provides the first deliberately partial direct-execution profile. Every request still declares the intended backend, records the backend that actually ran, and refuses unsupported source visibly rather than quietly falling back to legacy. `src/backend/selection.rs` (#986) is the compiler-owned boundary that makes that possible.

This is a different axis from Oven's own receipt, described in [Oven Alpha](oven_alpha.md): Oven's receipt selects how an already-generated artifact is compiled (its legacy-Cargo-vs-direct-`rustc` build boundary), and never influences which compiler backend produced that artifact in the first place. Legacy builds hand generated source to Oven; the #988 direct replacement profile does not create generated source or an Oven plan.

## The two records

**`BackendSelection`** is a versioned, content-identified record of what was decided *before* execution: the requested backend, its implementation revision, the compatibility profile it declares for this compilation, a content identity of the source being compiled, why that backend was selected (the compiler-owned default, or an explicit `--backend` request), and the declared fallback policy. It is built by `select_backend` before any codegen starts.

**`BackendExecutionReceipt`** is a versioned, content-identified record of what actually happened *after* execution: the backend that actually ran, whether a declared fallback occurred, the shadow-comparison outcome (always present and explicit, even when the comparison could not run), the diagnostic-contract version in force, and a content identity of the produced output. It embeds the `BackendSelection` it is bound to.

Both types are plain, I/O-free data — building and executing them does not touch `IrCodegen` or any other execution machinery directly. Both carry a content-derived `sha256:` identity, verified with `verify_identity()`, the same pattern Oven's own receipt uses: a later stage that only holds a serialized copy does not need to re-derive trust from the fields themselves.

## No silent fallback

Every successful `incan build` exposes a selection and a receipt, including the default path: selecting the legacy backend with no flags produces an explicit `selection_reason: "default"` record rather than an implicit, unrecorded choice. A source-current completed-output reuse is eligible only when its immutable Loaf already carries and verifies that same implicit-default receipt; it republishes the verified receipt after materialization rather than inventing a new execution record.

The first #988 replacement profile produces a visible outcome, never a silent legacy execution:

- `incan build --backend replacement --backend-fallback refuse` typechecks one source-only module containing exactly the selected zero-argument `main` free function, lowers it to Body IR, and executes that body directly. Its receipt records `executed_backend: "replacement"`, `fallback_outcome: "not_needed"`, and an output identity over the actual Body-IR execution result; this path does not construct generated Rust or an Oven plan.
- The supported profile is deliberately partial: scalar arithmetic, scalar local bindings with no repeated user-binding name, returns, compiler-owned string concatenation, normalized range/while branches and loops, and assertions. Packages, imports, Rust interop, additional free functions, callable values, generators, destructuring, all aggregates, projections, and any repeated user-binding spelling (lexical shadowing or reassignment) are refused visibly with the original Incan source span. A refusal emits no new receipt; it does not remove a receipt written by an earlier successful build.
- The #988 CLI surface accepts only `--backend-fallback refuse`. It has no receipt-bound legacy execution path for an unavailable source profile, so users must choose `--backend legacy` explicitly when the source is outside the profile.

A `--shadow` request without a source-observable legacy entrypoint remains `{"unavailable": {"reason": "..."}}`, never `"matched"`. It is deliberately non-green: generated-Rust token shape is not a semantic comparison.

Any explicit backend request, fallback policy, or shadow request bypasses completed-output reuse and takes the source-based path appropriate to the declared backend. This prevents a cached default result from being presented as the outcome of a different declared selection.

## CLI surface

`incan build` accepts three flags:

- `--backend <legacy|replacement>` — declare the backend for this build. Defaults to `legacy`.
- `--backend-fallback <refuse>` — declare that an unavailable backend must stop visibly. Omitting this flag means refuse.
- `--shadow` — request a comparison against the replacement backend alongside normal execution.

A successful build publishes its receipt to `.incan/backend/receipt.json` in the project root (parallel to Oven's own `.incan/oven/receipt.json`), and embeds it as the `backend` field of `incan build --report json` output. The #988 direct path has the explicit `incan.replacement_execution.v0` report schema: alongside `status`, `mode`, and `entrypoint`, its `replacement_execution` payload projects the Body-IR result, receipt-bound output identity, snapshot, canonical ownership reads, and runtime requirements. It intentionally has no generated-artifact or Oven fields. An eligible completed-output reuse republishes its verified sealed receipt at the same path. Inspect a persisted receipt directly with:

```bash
incan inspect backend-selection --receipt .incan/backend/receipt.json
incan inspect backend-selection --receipt .incan/backend/receipt.json --format json
```

`inspect backend-selection` reads the receipt, calls `verify_identity()` on it (and, transitively, on the selection it embeds), and refuses to render a receipt whose recorded identity does not match its own content — the same tamper/staleness detection Oven's `inspect oven` performs on its receipt.

## Provenance for Oven and other clients

Oven and other clients can key provenance on the pre-execution `BackendSelection.identity` and attach the post-execution `BackendExecutionReceipt` to their own outputs, without reading private HIR or Body IR structures: the receipt is a public, versioned projection of "what backend produced this," independent of how either backend represents a program internally. `diagnostic_contract_version` on the receipt ties it to the diagnostics schema (`crates/incan_syntax/src/diagnostics/stable.rs::DIAGNOSTIC_SCHEMA_VERSION`) in force when it was produced, so a consumer can tell whether a receipt's diagnostics are still interpretable under its own contract version.
