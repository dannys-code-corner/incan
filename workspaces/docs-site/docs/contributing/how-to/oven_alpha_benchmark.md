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
revision, release archive identity, `incan --version`, OS/architecture, exact source fixture, profile, storage limits,
and whether the store started empty. Keep the generated `report.json` and per-phase logs with the release evidence.

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
  --workload test \
  --source tests/fixtures/test_assert_canary.incn \
  --incan-home /tmp/incan-oven-test-home \
  --output /tmp/incan-oven-test-evidence \
  --cargo-guard-dir /tmp/incan-oven-cargo-guard \
  --repetitions 2
```

For build or run, set `--workload build` or `--workload run` and point `--source` at a core-envelope program. On
Linux, use a task-specific directory below `/tmp`; the store must start empty so its first materialization is
attributable. The default policy is the normal developer policy: 2 GiB aggregate physical allocation, 1 GiB physical
allocation per compatibility domain, and 768 MiB logical artifact bytes per domain. Pass explicit byte overrides only
when recording a different policy.

## Read the report

`report.json` contains:

- the machine and toolchain identity;
- `first_materialization` and each `warm_repeat_N` elapsed duration and exit status;
- the required Cargo-guard probe status and verdict that successful normal stages did not launch Cargo;
- bounded-store inspection with physical allocation separate from logical artifact bytes, reclaimable bytes, and
  lease-protected bytes; and
- one log file per phase, including verbose Oven timing for a test workload.

Use `first_materialization` for the supported compiler-shipped unit's initial user-machine cost and the warm repeats
for the unchanged normal-command goal. If the first normal command fails, a warm command launches the guard, a plan
changes unexpectedly, or store reporting loses the physical/logical distinction, treat the result as a failure rather
than averaging it into a performance claim.
