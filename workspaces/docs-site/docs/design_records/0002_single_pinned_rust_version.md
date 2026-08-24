---
id: DD-0002
title: Pin one Rust version and one generated-project edition
status: Draft
type: design-decision
date: 2026-08-24
review_target: v0.7 planning
sources:
  - https://github.com/encero-systems/incan/issues/1168
  - https://github.com/encero-systems/incan/pull/1157
  - RFC 119
---

# DD-0002: Pin one Rust version and one generated-project edition

## Context

Incan provisions and ships its own Rust. The installer places an Incan-owned `rustup` home on the machine and never reconfigures the user's default toolchain, precisely so that a user's Rust and Incan's Rust cannot drift into each other. That ownership is only as good as the version discipline behind it.

Today the project carries three independent Rust version knobs, and a fourth for the language edition emitted into generated projects. They were introduced separately, for different audiences, and nobody has to reconcile them:

| Knob | Where it lives | Who it affects |
| --- | --- | --- |
| Shipped toolchain | the release workflow that packages the toolchain | every user, indirectly — it is the compiler that builds their generated Rust |
| Minimum supported Rust version | `rust-version` in the workspace manifest, plus a matching CI job | only people who build the Incan compiler from source with their own Rust |
| CI build and test toolchains | the `stable`-valued toolchain selectors used by the test harness | the build |
| Generated-project edition | the generated-project default in the project generator | every generated project, and therefore every user program |

Two of these are not pinned at all. The release workflow builds with a floating stable channel, so "the exact Rust release this toolchain was built against" is whatever stable happened to be current on the build date; two builds of the same tag can ship different compilers. The CI selectors float the same way.

The generated-project edition is stale rather than unpinned: generated projects still default to the 2021 edition while the compiler's own workspace moved to 2024 some time ago, and several Oven paths already hardcode 2024. Emitted user code is the only thing left on the older edition, and nothing requires it to be — the whole point of shipping our own Rust is that we are not held to an older language contract by someone else's installed toolchain.

The minimum supported Rust version existed to protect a build-from-source installation path. v0.6 removes that path as part of moving off Cargo, which leaves the field constraining contributors and nobody else.

## Decision

Incan pins **one** Rust version across everything that builds or ships, and **one** edition for generated projects.

- The shipped toolchain, the minimum supported Rust version, and the CI build and test toolchains all name **Rust 1.98.0** exactly.
- Generated projects are emitted for the **2024 edition**.
- Both values are changed deliberately, in one place each, and are expected to hold for the foreseeable future. Raising either is a decision, not a consequence of time passing.

Pinning the minimum supported version alongside the others is deliberate rather than incidental. Once the build-from-source installation path is gone, that field constrains contributors only, and contributors are on the pinned toolchain already. Keeping it lower would preserve a distinction that no longer protects anyone while leaving four numbers where one will do.

`review_target` above is a review point for whether the pin should move, not a commitment to move it.

## Consequences

- A release is reproducible with respect to its Rust compiler: the same tag built twice ships the same compiler.
- There is one number to reason about. A version mismatch between the shipped toolchain, the compiler's declared minimum, and CI becomes impossible by construction rather than by vigilance.
- Contributors need Rust 1.98.0 to build the compiler.
- Generated projects compile under 2024 language rules. That edition changes more than syntax — reserved words, opaque-return lifetime capture, `static mut` references, and tail-expression temporary scopes all differ — so moving the emitted edition is a real behavior change for generated code, not a cosmetic bump.
- **Reserved-word escaping must lead the edition change.** The 2024 edition reserved `gen`, and the compiler's keyword-escape registry does not carry it, so an Incan declaration named `gen` currently emits an unescaped Rust `gen` that is valid under 2021 and a hard error under 2024. A program that compiles today would break on the edition move. The registry must be corrected first; that correction stands on its own merit regardless of when the edition moves.
- Pinning makes the compiler's symbol-mangling scheme dependable. Rust 1.97 made v0 mangling the default, and v0 preserves generic parameter values rather than hiding them behind a hash. Work that reads symbols back — recovering a canonical source identity from an emitted name, or reconciling an artifact against compiler-owned declarations — can rely on that only because the release is pinned. Under a floating channel the mangling scheme is whatever the build date supplied, and any such work would need to detect and tolerate both schemes.
- Two nightly dependencies survive this decision and are not mismatches under it:
  - `cargo +nightly fmt` remains required, because the project's formatting configuration uses unstable comment and documentation wrapping options. It formats the compiler's own Rust source; it is never shipped and never compiles user code.
  - The nightly publisher toolchain used by the test harness exists only to exercise Cargo's nightly-only metadata flags. Its justification disappears with the move off Cargo, so it should be retired rather than pinned.

## Non-goals

- Making Cargo invalid as a project format. Cargo remains a supported compatibility and adoption mode; this record is about which Rust the toolchain itself is built with and ships.
- Changing the Oven build path, receipt contracts, or provider planning.
- Committing to track new Rust stable releases on any schedule. The pin is expected to be stable, and moving it is a fresh decision.
- Promising that generated Rust is a stable contract. It is not, and moving the emitted edition does not make it one.
- Defining the toolchain provisioning mechanism, which is Oven's and the installer's, not this record's.

## Revisit condition

Review this decision when any of the following holds:

- the compiler needs a language or standard-library feature that the pinned release does not provide, in which case the record should name that feature;
- a security advisory affects the pinned release;
- a newer edition stabilizes and offers something generated code actually benefits from; or
- the platform support matrix changes in a way the pinned release cannot serve.

Routine availability of a newer stable release is explicitly **not** a revisit condition. Drift is the failure mode this record exists to prevent.

## Provenance

This record derives from the v0.6 direction of moving off Cargo, from [RFC 119](../RFCs/119_oven_native_rust_build_facets_and_cargo_interoperation.md)'s treatment of Rust facets and Cargo interoperation, and from the audit recorded in [issue #1168](https://github.com/encero-systems/incan/issues/1168). The version-knob inventory above was gathered while reviewing [PR #1157](https://github.com/encero-systems/incan/pull/1157), which named the generated-project edition constant without changing its value. It records the accepted pinning policy and does not supersede those authorities.

## References

- [Issue #1168: bump generated projects to Rust edition 2024 and pin the shipped Rust release](https://github.com/encero-systems/incan/issues/1168)
- [Issue #1174: tighten RFC 120 so the emitted-name projection is recoverable](https://github.com/encero-systems/incan/issues/1174) — depends on v0 being dependable
- [Issue #1182: reconcile artifact symbols against compiler-owned declarations](https://github.com/encero-systems/incan/issues/1182) — same dependency
- [PR #1157: source-observable shadow comparison](https://github.com/encero-systems/incan/pull/1157) — surfaced the edition constant and the unescaped `gen` gap
- [RFC 119: Oven-native Rust build facets and Cargo interoperation](../RFCs/119_oven_native_rust_build_facets_and_cargo_interoperation.md)
