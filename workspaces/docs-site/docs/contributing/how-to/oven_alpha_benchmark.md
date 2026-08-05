# Oven Alpha benchmark protocol

Use this protocol to measure the DX-recovery lane: a compiler-shipped Oven unit's first materialization into an empty
developer store, then repeated normal Oven `build`, `run`, or `test` commands. It is separate from generated-program
runtime benchmarks and never uses Cargo as part of the benchmark workflow.

The harness is deliberately strict. It starts with an empty `INCAN_HOME`, records the first normal command (which
materializes a verified compiler-shipped unit and produces the caller-owned output), then records unchanged normal
command repeats. The first command is not labelled warm. A required failing `cargo` executable is probed to confirm
that it exits with status 97, then prepended to `PATH`; a successful normal stage therefore proves that it did not
launch Cargo.

## Reference-machine requirements

Run the same supported workload on one documented macOS machine and one documented Linux machine. Record the checkout
revision, release archive/artifact identity, `incan --version`, OS/architecture, exact source fixture and digest,
profile, storage limits, and whether the store started empty. Keep the generated `report.json` and per-phase logs with
the release evidence. The harness requires an archive or CI-artifact identity rather than silently treating an
arbitrary local binary as a comparable measurement.

The documented Alpha envelope is intentionally finite. At present the release archive ships exactly two units: a
release core closure and a debug foundation closure for `std.testing`, `std.fs`, and `std.json` (which may satisfy a
narrower compatible debug-core request). An unsupported provider/dependency closure must fail explicitly; do not turn
the benchmark into a manual `legacy_cargo` publication to make it pass.

## Run a guarded test workload

Extract the candidate release archive and use the matching Incan checkout for the harness and its fixtures; those
benchmark assets are deliberately not shipped inside the toolchain archive. Then create a task-specific failing Cargo
guard outside the repository. The guard proves that the tested normal command cannot accidentally invoke Cargo.

```bash
mkdir -p /tmp/incan-oven-cargo-guard
printf '#!/bin/sh\necho unexpected Cargo launch >&2\nexit 97\n' > /tmp/incan-oven-cargo-guard/cargo
chmod +x /tmp/incan-oven-cargo-guard/cargo

bash scripts/bench_oven_alpha.sh \
  --incan /path/to/extracted/incan/bin/incan \
  --release-identity 'incan-VERSION-TARGET.tar.gz sha256:...' \
  --checkout-revision "$(git rev-parse HEAD)" \
  --workload test \
  --source tests/fixtures/test_assert_canary.incn \
  --incan-home /tmp/incan-oven-test-home \
  --output /tmp/incan-oven-test-evidence \
  --cargo-guard-dir /tmp/incan-oven-cargo-guard \
  --repetitions 2
```

To prove that reuse is not tied to the original checkout, create a clean worktree at the same revision and pass its
byte-identical fixture as `--clean-worktree-source`. The harness rejects a different source digest rather than calling
two unrelated commands a reuse measurement:

```bash
git worktree add --detach /tmp/incan-oven-benchmark-clean HEAD
# Add --clean-worktree-source /tmp/incan-oven-benchmark-clean/tests/fixtures/test_assert_canary.incn
```

For build or run, set `--workload build` or `--workload run` and use the checkout's release-core fixture
`tests/fixtures/oven_alpha_release_core.incn`; a `std.testing` fixture is debug-only and is intentionally not a
release-build benchmark. On Linux, use a task-specific directory below `/tmp`; the store must start empty so its first
materialization is attributable. The default policy is the normal developer policy: 2 GiB aggregate physical
allocation, 1 GiB physical allocation per compatibility domain, and 768 MiB logical artifact bytes per domain. Pass
explicit byte overrides only when recording a different policy.

## Read the report

`report.json` contains:

- the machine and toolchain identity;
- `first_materialization`, each `warm_repeat_N`, and (when requested) `clean_worktree_reuse` elapsed duration and
  exit status;
- the required Cargo-guard probe status and verdict that successful normal stages did not launch Cargo;
- storage junctions before materialization, after first materialization, after every warm repeat, and after the
  clean-worktree repeat. Each keeps its own bounded-store inspection (physical allocation separate from logical
  artifact bytes, reclaimable bytes, and lease-protected bytes) plus raw store/output disk totals; and
- one log file per phase, including verbose Oven timing for a test workload.

Use `first_materialization` for the supported compiler-shipped unit's initial user-machine cost and the warm repeats
for the unchanged normal-command goal. If the first normal command fails, a warm command launches the guard, a plan
changes unexpectedly, or store reporting loses the physical/logical distinction, treat the result as a failure rather
than averaging it into a performance claim.

## Measure the complete compiler repository suite

The repository suite uses the same bounded-store policy but has an additional, explicitly named preparation seam:
`scripts/run_oven_compiler_suite.sh`. Invoke it with the current checkout, prewarmed SDK-provider store, compiler-owned
native-unit layout, and the limits used by `make test-oven`; the `test-oven` target is the canonical argument list.
The Makefile pins the publisher split used by CI: stable Cargo prepares native units and nightly Cargo supplies the
publisher-only `-Z unit-graph` capability for the stored compiler suite. Normal Oven consumers remain direct-rustc.
For a local full run, use `make test-verbose`; override `INCAN_TEST_PREWARM_TOOLCHAIN` or
`INCAN_TEST_SUITE_TOOLCHAIN` only when reproducing a documented alternate toolchain matrix.
For benchmark evidence, invoke the script directly with a retained, unique `--output` directory instead of the
Make target's disposable success directory.

After building the debug compiler and running `INCAN_TEST_COMPILER_ALREADY_BUILT=1 make test-prewarm-oven-native-units`,
the first reference run is:

```bash
bash scripts/run_oven_compiler_suite.sh \
  --incan "$PWD/target/debug/incan" \
  --compiler-root "$PWD" \
  --store /tmp/incan-oven-suite-store \
  --output /tmp/incan-oven-suite-cold \
  --sdk-provider-path-file "$PWD/target/incan_test_sdk_provider_path" \
  --sdk-provider-store "$PWD/target/incan_test_sdk_provider_store" \
  --toolchain-data-root "$PWD/target" \
  --generated-cargo-target-dir "$PWD/target/incan_generated_shared_target" \
  --domain reference-compiler-suite \
  --max-physical-bytes 3221225472 \
  --max-domain-physical-bytes 2415919104 \
  --max-domain-logical-bytes 2684354560 \
  --feature lsp \
  --temp-root /tmp/incan_oven_suite_tmp
```

For the reuse measurement, repeat that command unchanged except for a fresh `--output`
directory such as `/tmp/incan-oven-suite-warm`; do not remove or alter the store.

The resulting `suite-evidence.json` separates compiler revision/toolchain identity from four execution/storage facts:

- `publisher.prepare.cargo_version` is preserved in the caller-owned report alongside
  `publisher-result.json`: `not-run-existing-suite` proves this invocation reused the compatible immutable suite;
- `named_legacy_publisher` is the declared cold preparation or receipt-reuse cost of the temporary,
  receipt-bound publisher;
- `cargo_free_direct_rustc_replay` is the prepared full-suite result and must report zero Cargo-guard invocations;
- `store_snapshot_initial`, `store_snapshot_after_named_legacy_publisher`, and
  `store_snapshot_after_cargo_free_direct_rustc_replay` retain the storage state at every junction. Their reports
  separately record logical artifacts, physical allocation, reclaimability, active leases, limits, and raw
  store/output disk totals.

Repeat the same script with the same store and a fresh caller-output directory. The repeated publisher must select
the existing receipt-compatible suite rather than start Cargo, while the direct-Rustc replay still executes every
reported root. Keep both `suite-evidence.json` and `publisher-result.json`, the `storage-junctions.json` sequence,
and their `phases.tsv` files. Run this cold/reuse pair on the documented macOS and Linux reference machines; do not
combine the publisher duration with the prepared replay or present a source-incompatible native-unit refusal as a
benchmark result.
