# Backend-cutover parity corpus

This turns the [backend behavior inventory](backend_behavior_inventory.md) (issue [#646](https://github.com/encero-systems/incan/issues/646)) into an executable corpus for issue [#987](https://github.com/encero-systems/incan/issues/987). The inventory is a record; this corpus is runnable. Each case is a stable, identified claim about current compiler behavior that the [0.6 backend cutover](https://github.com/encero-systems/incan/issues/652) must either preserve, intentionally migrate, or drop with an owning issue.

Status: seed corpus for the 0.6 backend-cutover foundation. It is narrow and source-only by design — see [Scope](#scope) below.

## Where it lives

- Schema and validation: `tests/support/parity_corpus.rs`
- Seed cases and CI-summary tests: `tests/parity_corpus_tests.rs`
- Run it: `cargo test --test parity_corpus_tests`
- CI-readable output: written to `<CARGO_TARGET_DIR or target>/parity-corpus/summary.json` by the `ci_summary_serializes_with_the_fields_655_needs_and_is_written_to_a_stable_path` test.

## Scope

Per #987's own plan, this corpus starts with a narrow, source-only seed (direct parser/typechecker, generated-project-run, and codegen-snapshot lanes) before growing into package/import, Rust-interop, vocab, and downstream lanes. Those later lanes need stable compiler/ABI contracts and receipt-aware comparisons; they are intentionally absent from the seed rather than implied green. Issue [#987](https://github.com/encero-systems/incan/issues/987) owns choosing and adding those cases when the relevant boundaries are available.

This corpus compares by semantic and user-visible outcome — typecheck acceptance/rejection, diagnostic presence, runtime helper results — never by generated Rust token shape. A [`GeneratedArtifactBehavior`](#categories) case may only assert that generation succeeds and produces syntactically valid Rust; it must not assert a byte-exact snapshot as the contract. See [Rust-source backend deprecation policy](rust_source_backend_deprecation.md) for why: "parity means 'same supported source behavior,' not 'same emitted tokens.'"

## Categories

Same seven categories as the [behavior inventory](backend_behavior_inventory.md#categories):

| Category | Meaning |
| --- | --- |
| Supported language contract | Documented or intentionally exposed source-level Incan semantics. |
| Stdlib/runtime behavior | Behavior owned by `crates/incan_stdlib`, `crates/incan_core`, or `.incn` stdlib source. |
| Rust interop behavior | Behavior crossing `rust::` imports, rust-inspect metadata, or generated Cargo projects. |
| Generated-artifact behavior | Behavior visible mainly through generated Rust shape or `target/incan/**` layout. |
| Diagnostic behavior | Error/warning codes, spans, JSON schema facts, and text diagnostics. |
| Accidental accepted behavior | Accepted only because a parser/typechecker/lowering path happens to allow it, with no documented contract. |
| Bug-compatible behavior | Preserved only because current users may rely on a workaround, or fixing it needs a larger migration. |

## Evidence lanes

Same six lanes as the inventory's `Evidence lanes` table: direct parser/typechecker tests, codegen snapshots, generated-project runs, package/import boundaries, vocab/test-batch lanes, and downstream proof lanes (IncQL/Hees.ai).

## Disposition

Every case carries exactly one of three dispositions — #987 names these explicitly, and the schema does not allow a fourth "undecided" state:

| Disposition | Meaning |
| --- | --- |
| `preserved` | The 0.6 cutover must keep this behavior working as-is. |
| `intentional_migration` | The behavior will change deliberately during cutover; carries `owning_issue` and `migration_note`. |
| `unsupported` | The behavior is not guaranteed to survive cutover as-is; carries `owning_issue` and `migration_note`. |

A case that has not been triaged yet still has to pick `unsupported` (or `intentional_migration`) with a real owning issue and a migration note describing what triage is still needed — it cannot skip the decision.

## Receipt awareness (#986)

#987's own scope calls for "receipt-aware reference/replacement or shadow comparisons where both paths are available," referencing the backend-selection receipt type from the sibling issue [#986](https://github.com/encero-systems/incan/issues/986). As of this corpus's initial seed, #986 has not landed a stable receipt type. Every case therefore carries `receipt: { "state": "pending_schema", "blocking_issue": 986 }` — a documented placeholder, not a guessed shape.

This is why the summary's top-level `receipt_schema_available` is `false` and no case can reach `overall_state: "green"` yet: reaching green requires both a matched behavior outcome *and* an available receipt-aware comparison. Every seed case today lands at `non_green_pending_receipt` instead, even though its own frontend-observable behavior was confirmed. This is intentional — see [Explicit non-green states](#explicit-non-green-states).

## Explicit non-green states

An unavailable, skipped, or incompatible comparison must never be silently counted as parity. The corpus enforces this on two axes:

- `behavior_outcome` — the result of actually running the case's `evaluate()` function against the current compiler: `match` (green), `mismatch` (regression signal), `skipped` (not run yet, with a reason), or `incompatible` (not comparable, with a reason).
- `overall_state` — folds `behavior_outcome` together with receipt availability into the field a consumer should read first: `green` (both axes confirmed — not reachable until #986 lands), `non_green_pending_receipt` (behavior confirmed, receipt-aware comparison unavailable), or `non_green_behavior` (the behavior check itself did not match).

A consumer reading only `behavior_outcome` could report false green parity before #986 lands. Read `overall_state`.

## CI-summary JSON shape

```json
{
  "schema_version": 1,
  "total_cases": 6,
  "green": 0,
  "non_green_pending_receipt": 6,
  "non_green_behavior": 0,
  "receipt_schema_available": false,
  "cases": [
    {
      "id": "parity-987-0001",
      "title": "Match expressions over enums must be exhaustive",
      "category": "supported_language_contract",
      "lane": "direct_parser_typechecker",
      "evidence": "tests/parity_corpus_tests.rs::case_supported_match_exhaustiveness",
      "disposition_kind": "preserved",
      "behavior_outcome": { "state": "match" },
      "receipt": { "state": "pending_schema", "blocking_issue": 986 },
      "overall_state": "non_green_pending_receipt"
    }
  ]
}
```

`schema_version` bumps whenever a consumer (including the #655 compatibility report) would need to notice a field-shape change. `id` values are permanent once assigned — a future revision deletes and re-adds a case rather than renumbering it.

## Adding a case

1. Pick the narrowest category and evidence lane that actually proves the behavior; use more than one lane if the behavior crosses a boundary.
2. Write an `evaluate() -> ComparisonOutcome` function that probes the *current* compiler directly — lex/parse/typecheck a snippet, call a runtime helper, or generate and inspect Rust shape — rather than asserting against a stored fixture. A behavior change should show up as `Mismatch` the next time the test runs.
3. Pick a disposition. Non-`preserved` dispositions need a real, non-zero owning issue and a non-empty migration note; `validate_corpus` rejects anything else.
4. Assign the next stable `parity-987-NNNN` ID; do not reuse or renumber an existing one.
5. Add the case to `seed_corpus()` in `tests/parity_corpus_tests.rs`.

## Related

- [Backend behavior inventory](backend_behavior_inventory.md) — the closed #646 inventory this corpus executes.
- [Rust-source backend deprecation policy](rust_source_backend_deprecation.md) — why generated Rust is not semantic authority.
