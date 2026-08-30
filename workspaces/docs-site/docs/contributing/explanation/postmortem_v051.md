# Post-mortem: why v0.5.1 had to exist

This is a blameless post-mortem of the v0.5.0 release. Within two days of shipping, the first person to use an installed toolchain filed the first defect wave; within five days, the first real third-party Rust workload had falsified five separate assumptions about the interop surface. The patch train that followed ([#1191](https://github.com/encero-systems/incan/issues/1191), released as v0.5.1) fixed nineteen tracked defects and, beneath them, four more latent toolchain defects that only appeared as each layer above them was peeled away. This document explains how a release with a large, genuinely good test suite shipped in that state, and what the project changed so the next release cannot fail the same way.

## The one-sentence root cause

v0.5.0's validation stopped exactly where its users started: every gate exercised the compiler from the source tree on the machine that built it, while the product being shipped was an installed archive on someone else's machine — and those two differ by several whole mechanism layers that no amount of source-tree testing can observe.

## Timeline

| Date | Event |
| --- | --- |
| Aug 22 | v0.5.0 ships after six release candidates. The IncQL acceptance gate gains the ability to run against an installed Incan home on this same day. |
| Aug 24 | First defect wave filed from installed-toolchain use: released archives omit the stdlib sources ([#1192](https://github.com/encero-systems/incan/issues/1192)), inspection sources fail to reconcile ([#1189](https://github.com/encero-systems/incan/issues/1189)), rebake is not idempotent across the lock migration ([#1194](https://github.com/encero-systems/incan/issues/1194)), a silent numeric miscompile ([#1200](https://github.com/encero-systems/incan/issues/1200)), and more. The v0.5.1 tracker [#1191](https://github.com/encero-systems/incan/issues/1191) opens. |
| Aug 27 | First real third-party workload (a bare Bevy application) falsifies five interop assumptions in one program: owned-mutable Rust parameters ([#1216](https://github.com/encero-systems/incan/issues/1216)), re-export type identity ([#1217](https://github.com/encero-systems/incan/issues/1217)), tuple-struct constructors ([#1218](https://github.com/encero-systems/incan/issues/1218)), `f32` width preservation ([#1219](https://github.com/encero-systems/incan/issues/1219)), and mutable query payloads ([#1223](https://github.com/encero-systems/incan/issues/1223)). |
| Aug 28–30 | The v0.5.1 train fixes the tracked set, then discovers and fixes four latent packaged-toolchain defects underneath — each invisible until the one above it was fixed: sealed closures stripped Rust 1.98 split-metadata sidecars; cohort substitution trusted filenames across machines; a digest-stamping pass overwrote retained records; and finally two rustc invariants (the compiler folds its own install path into the strict version hash, and refuses two crates with one `StableCrateId`) that made the original artifact-sharing model unlinkable across machines at all. |
| Aug 30 | v0.5.1 ships, validated for the first time on the published artifacts themselves, on a machine that did not build them. |

## The five compounding causes

### 1. Validation ended at the artifact boundary

The suite was genuinely strong — roughly 3,900 tests, and during the v0.5.1 train it caught every regression introduced across six release-candidate cycles. But [#1192](https://github.com/encero-systems/incan/issues/1192) is not a subtle bug: released archives were missing the standard-library sources entirely, which means no released archive ever ran a stdlib-importing project on a clean machine before shipping. Six v0.5.0 release candidates iterated on "the suite is green," which validates the compiler. Nobody's gate validated the product — the thing a user actually downloads.

### 2. Same-machine masking is structural, not accidental

This is the deepest lesson. Even packaged validation passes on the machine that built the archives and fails everywhere else: split-metadata sidecars, path-salted version hashes, standard-library source spans, `StableCrateId` collisions. During the v0.5.1 train, five consecutive release candidates each fixed a layer, each passed complete local validation — including local packaged validation — and each failed on published artifacts, because the local base closure and the local project build always agreed by accident of shared machine state. This defect class is undetectable by any single-machine process, however diligent. v0.5.0's first cross-machine execution was, effectively, a user.

### 3. No adversarial consumer in the gate

The interop surface was proven against curated examples and the IncQL consumer — which is first-party-shaped: its patterns mirror what the compiler grew up with. A real ecosystem crate is hostile in ways an in-house corpus never is. Bevy — a ~250-crate closure, dense with proc macros, public re-exports, tuple structs, `f32` arithmetic, and mutable generic queries — falsified five separate language-surface assumptions within days of release. Every one of those five was expressible in a twenty-line program.

### 4. Warm development state hid the cold paths

Development machines carry primed stores, receipts, caches, and compatible toolchain vintages. The non-idempotent rebake ([#1194](https://github.com/encero-systems/incan/issues/1194)), the inspection-authority fragility ([#1229](https://github.com/encero-systems/incan/issues/1229)), and the repeated state-contamination incidents during the train are all one symptom: warm paths were exercised thousands of times, cold-start paths approximately never. Every developer bake is warm; every user's first bake is cold.

### 5. Acceptance criteria were written after the release

[#1191](https://github.com/encero-systems/incan/issues/1191)'s eventual criterion — "emits a Bevy system that compiles natively, with source *and artifact* evidence" — is precisely the sentence that, written before v0.5.0, would have caught the entire stack. The v0.5.0 release candidates iterated toward a green suite rather than toward a user-visible definition of done expressed in terms of shipped artifacts.

## Why this is not a carelessness story

Nothing here was sloppy. The suite's quality is what made it possible to fix six defect layers in one weekend without introducing a single regression. The failure mode is the oldest one in release engineering — *test what you build, not what you ship* — amplified by a product whose core value proposition (sealed, prebuilt library closures) is exactly the mechanism that behaves differently across a machine boundary. The most fundamental defect in the stack required two machines to even observe.

## What changed during v0.5.1

- A published-artifact validation protocol: fresh prefix, fresh `INCAN_HOME`, install from the release manifest, then the acceptance lanes.
- A bare-Bevy acceptance fixture whose one program exercises all five interop repairs and prints a marker on success.
- The IncQL gate runs against an installed home, not just a source checkout.
- Deterministic path remapping and standard-library span unification make baked units machine-independent where rustc permits it, and the composition fails closed by artifact name where it does not.
- State-fragility amplifiers are tracked for 0.6: fail-closed inspection authority ([#1229](https://github.com/encero-systems/incan/issues/1229)) and reservation-time store reclamation ([#1230](https://github.com/encero-systems/incan/issues/1230)).

## Actions for 0.6

1. **Cross-machine validation in CI.** A release-candidate job installs the built archive on a *different* runner than the one that built it and runs the Bevy and IncQL acceptance lanes there. This single job would have caught the entire rc7–rc12 defect stack automatically.
2. **Acceptance criteria before the train.** Milestone acceptance is written as user-visible behaviors on shipped artifacts — with the evidence form named — before the first release candidate is cut, not reconstructed afterward.
3. **A cold-start lane in the release workflow.** Empty home, empty store, first bake — the path every user actually takes first.
4. **A hostile-consumer corpus.** Keep Bevy and IncQL as permanent acceptance fixtures and add at least one more ecosystem-heavy crate family, chosen for the interop patterns the current corpus does not cover.
5. **The unified closure.** The interim identity-salt model ships in v0.5.1; the wrapper-based base-artifact injection that removes its duplication cost is designed into the 0.6 cargo rework ([#1241](https://github.com/encero-systems/incan/issues/1241)).
