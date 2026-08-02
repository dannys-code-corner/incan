---
title: Oven Alpha
hide:
  - toc
---

<!-- markdownlint-disable MD033 -->

<section class="inc-oven-hero" aria-labelledby="oven-hero-title" markdown="1">

<div class="inc-oven-hero__copy" markdown="1">

<p class="inc-oven-hero__kicker">Experimental · Oven Alpha</p>

<h1 id="oven-hero-title"><span>Bound the build.</span><span>Keep the proof.</span></h1>

<p class="inc-oven-hero__lead">Oven is Incan’s receipt-bound native consumer path: a verified closure is stored under policy, then direct <code>rustc</code> tests and binaries run without Cargo on the consumer path.</p>

<div class="inc-oven-hero__actions" markdown="1">
[Read the Alpha boundary](#what-this-does-not-claim){ .md-button .md-button--primary }
[CLI reference](../reference/cli_reference.md){ .md-button }
</div>

<p class="inc-oven-hero__truth"><strong>Alpha status:</strong> the shipped core and <code>std.testing</code> compatibility units let ordinary <code>incan build</code>, <code>incan test</code>, and <code>incan run</code> use the stored direct-<code>rustc</code> consumer path. A cold unsupported compatibility domain stops with an explicit receipt-bound preparation request; it never falls back to Cargo. The envelope remains deliberately narrow and is not yet release-complete.</p>

</div>

<div class="inc-oven-hero__image" role="img" aria-label="A cybernetic alpaca baker tending a glowing oven in a mountain workshop"></div>

<div class="inc-oven-hero__proofs" aria-label="Oven Alpha guarantees">
<div><strong>Receipt-bound</strong><span>Exact source, target, and compiler identity.</span></div>
<div><strong>Policy-bounded</strong><span>Physical disk use and logical artifact bytes stay distinct.</span></div>
<div><strong>Lease-safe</strong><span>Active work cannot be pruned underneath a consumer.</span></div>
</div>

</section>

Oven Alpha is the native execution path Incan is moving toward. It establishes ownership and safety boundaries that a faster Cargo cache cannot provide:

- a portable, content-verified receipt for one frozen compatibility import;
- a bounded Oven-owned immutable artifact store;
- explicit direct-`rustc` test and binary-run consumers with no Cargo process or inherited Cargo environment; and
- native libtest inventory before exact test selection.

It is Alpha infrastructure, not a claim that every Rust ecosystem dependency shape is already supported. The release archive currently ships two direct-rustc units: core release and a debug foundation for `std.testing`, `std.fs`, and `std.json`. The foundation may serve a narrower core-debug provider request only when compiler, target, profile, runtime inputs, and requested provider modules/facets meet the explicit capability rule; it never authorizes an arbitrary Rust dependency. Compiler-supplied `rust::std` is permitted because it creates no Cargo dependency closure. Checked transitive Rust crates of a selected standard provider are captured only by its explicit release-only publisher; every caller-owned inline Rust crate remains an Alpha materialization boundary. The maintained normal path is Oven-owned `build`, `test`, and `run`; Cargo is not an alternative normal backend. An explicitly named, temporary `incan oven legacy-cargo prepare` publisher may prepare a bounded compatibility closure, but it is never a normal-command fallback. Oven Alpha does not yet resolve every build script or procedural macro shape, replace `incan.lock`, distribute Loaves, or provide an IncQL/DataFusion session. Each compatibility envelope must be proven against real workloads before it becomes supported.

## Compatibility receipt

`incan oven import` reads a root `Cargo.toml` and `Cargo.lock` as frozen compatibility evidence. It does not invoke Cargo, inspect a Cargo target directory, or infer target/toolchain facts from the host. Target, profile, features, and generated source inputs are explicit receipt inputs. The toolchain input must be the complete output of the exact compiler's `--version`; the consumer later executes that path with `--version` and rejects a different identity.

```bash
RUSTC="$(rustup which rustc)"
TOOLCHAIN="$($RUSTC --version)"

incan oven import \
  --project examples/frozen-package \
  --target aarch64-apple-darwin \
  --toolchain "$TOOLCHAIN" \
  --profile release \
  --source direct-rustc-source=target/oven/generated-tests.rs
```

The command writes `.incan/oven/receipt.json` below `--project` unless `--output` is supplied. The receipt records digests, not local source paths. Every later plan publication and execution recomputes its identity before trusting it.

The current compatibility envelope is intentionally narrow: one non-virtual root Cargo package with a lock file. Cargo metadata is an import input only. A publisher-side compatibility workflow may be called `legacy_cargo`, but Oven Alpha does not hide such a publisher behind a consumer command and does not offer it as normal developer workflow.

## Artifact plans and direct execution

A publisher supplies a JSON `OvenRustcArtifactManifest` and an immutable artifact root. The plan names every dependency search directory, `--extern` artifact, native search directory, and digest. `incan oven plan publish` checks every named path and digest against the receipt intent, then atomically copies that exact declared regular-file closure into the immutable `direct_rustc_plan` entry.

The publisher root is read only at publication. The stored plan and copied closure share one policy-bounded entry, and `incan oven test` and `incan oven run` resolve every declared artifact from that entry while holding its lease; neither has a `--artifact-root` runtime input. This is deliberately narrower than a general engine/Loaf materializer: the publisher must already have produced a complete direct-`rustc` manifest, and Alpha neither resolves dependencies nor discovers a closure itself.

```bash
incan oven plan publish \
  --receipt examples/frozen-package/.incan/oven/receipt.json \
  --manifest publisher/direct-rustc-plan.json \
  --artifact-root publisher/artifacts \
  --domain datafusion-aarch64-apple-darwin-release
```

On a cold native bake, `incan oven test` selects that exact stored plan while holding an active lease, rechecks the receipt, source digest, manifest intent, artifact paths, and artifact digests, verifies that `--rustc --version` exactly matches the receipt, and invokes that compiler for the receipt target. An unchanged caller-owned output is reused only after its sidecar receipt, selected-plan payload, source digest, and toolchain identity verify; it still holds the entry lease, but does not re-walk an immutable closure that was verified at publication. `incan oven store inspect` performs that full materialized-closure accounting verification. The executor clears inherited Cargo process variables for both compiler and test-binary processes.

```bash
incan oven test \
  --receipt examples/frozen-package/.incan/oven/receipt.json \
  --plan sha256:... \
  --rustc "$(rustup which rustc)" \
  --source target/oven/generated-tests.rs \
  --source-evidence direct-rustc-source \
  --output target/oven/generated-tests \
  --crate-name generated_tests \
  --exact package::smoke
```

`incan oven run` exposes the same receipt, stored plan, and copied closure used by ordinary `incan run` for a supported Alpha envelope. It compiles one caller-owned Rust binary, then runs that binary while the selected entry lease remains live. Arguments after `--` are forwarded only to the compiled binary. The explicit command is useful for inspecting the low-level contract; it is not a second backend or a Cargo fallback.

```bash
incan oven run \
  --receipt examples/frozen-package/.incan/oven/receipt.json \
  --plan sha256:... \
  --rustc "$(rustup which rustc)" \
  --source target/oven/generated-main.rs \
  --source-evidence direct-rustc-main \
  --output target/oven/generated-main \
  --crate-name generated_main \
  -- --application-flag
```

The direct executor is intentionally fail-closed. A changed receipt, source, plan, path, or digest is rejected before compilation. A selected artifact of the wrong kind or receipt intent is rejected instead of being treated as a generic cache hit.

## Bounded storage policy

The default store is `$INCAN_HOME/oven/store/v1`; without `INCAN_HOME`, it is `~/.incan/oven/store/v1`. This is distinct from `$INCAN_HOME/cache/generated-cargo/v1` and does not prune project source, caller-owned outputs, a Cargo target directory, or SDK-provider state.

Oven enforces policy at publication, not as a post-hoc inspection convenience:

| Policy | Default | Meaning |
| --- | ---: | --- |
| Aggregate physical allocation | 2 GiB | Allocated filesystem blocks retained by all published Oven artifacts in the everyday developer store. |
| Per-domain physical allocation | 1 GiB | Allocated blocks retained by one compatibility domain. |
| Per-domain logical artifact bytes | 768 MiB | Plan content plus manifest-declared copied-file bytes retained by one compatibility domain. |

Set `INCAN_OVEN_MAX_PHYSICAL_BYTES`, `INCAN_OVEN_MAX_DOMAIN_PHYSICAL_BYTES`, or `INCAN_OVEN_MAX_DOMAIN_LOGICAL_BYTES` to whole-byte values, or pass the matching `--max-…-bytes` flags for an explicit Alpha invocation. The per-domain physical allowance cannot exceed the aggregate allowance. A compiler-suite publisher that needs a larger envelope must pass it explicitly; its output is not a normal developer-store default.

The release archive has a separate `INCAN_OVEN_NATIVE_UNIT_MAX_BYTES` budget for its compiler-shipped seed payload. Its
default is 320 MiB of **logical** uncompressed bytes across all shipped units; package evidence records the logical
payload, the filesystem's physical allocation, and the cap. The cap is deliberately applied to logical bytes so
deduplication or filesystem compression cannot turn a larger distribution into an apparently compliant one. It prevents
the small documented Alpha envelope from silently turning into a multi-gigabyte distribution.

The seed count and byte values are release-package measurements, not everyday developer-store occupancy or a full
compiler-suite performance result; both logical and physical values must be remeasured for another target or release.

Separate native compatibility units may share byte-identical immutable artifacts through hard links in the release
layout. This reduces the physical archive/install footprint without merging their identities or relaxing validation;
the logical policy continues to charge every manifest-declared artifact path.

`incan oven store inspect` reports both values separately:

- **logical artifact bytes** are the immutable plan length plus the copied manifest-declared file lengths recorded in the entry manifest;
- **physical allocation** is measured filesystem allocation for the store-owned files. On Unix this uses allocated blocks, so it is deliberately not presented as logical artifact bytes.

```bash
incan oven store inspect --format json
incan oven store prune --format json
```

Before accepting a new artifact, Oven reserves conservative physical space, evicts least-recently-used inactive artifacts where that can satisfy aggregate or pending-domain policy, writes and synchronizes a same-filesystem staging directory, measures it, rechecks admission, and atomically publishes it. The manager lock also reclaims only stale compiler-owned staging directories before a report or operation, so interrupted partial publications do not become unreported physical usage.

If one compatibility domain exceeds either of its allowances, Oven first considers only inactive artifacts in that same domain for the per-domain constraint. If the incoming payload alone is too large, or retained entries needed to satisfy policy have active leases, publication fails with a capacity error. It never evicts an active artifact, silently exceeds the allowance, or removes caller-owned output. Aggregate pressure can evict any inactive Oven artifact by least-recently-used order, but active leases still win.

## Native test selection

`incan oven test --exact NAME` does not pass a potentially empty filter through as success. It runs the native test binary with `--list --format terse`, verifies every requested exact name against that inventory, then executes each exact test. A missing name fails before test execution. The higher-level Incan collector still owns Incan-language markers, fixtures, reports, and workspace scheduling; its supported native batches select this Oven scheduler rather than a Cargo consumer.

## What this does not claim

This Alpha establishes explicit end-to-end non-Cargo test and binary-run consumer seams with a bounded store-owned direct-`rustc` file closure. It is not yet a general direct-`rustc` build for arbitrary Rust packages, a Cargo-compatible resolver, support for Cargo build scripts/proc macros, a general engine/Loaf materializer, or the five-minute full-suite target. Performance claims need a measured representative Oven workload over a supported project closure.

The repository’s [Oven Alpha benchmark harness](../../contributing/how-to/oven_alpha_benchmark.md) records that cold publication separately from repeated normal-command measurements on macOS and Linux. A single local result is evidence for its declared machine and workload only; it is not a release-wide performance claim.
