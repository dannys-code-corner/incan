# RFC 123: Package executable representation

- **Status:** Draft
- **Created:** 2026-09-05
- **Author(s):** Danny Meijer (@dannymeijer)
- **Related:**
    - RFC 118 (Oven API and operational core)
    - RFC 120 (canonical source symbol identity)
    - RFC 097 (Rust-hosted Incan caller)
    - #989 (executable public-boundary evidence)
    - #1260 (package and local-module import execution)
    - #1261 (facade identity on the replacement route)
- **Issue:** —
- **RFC PR:** —
- **Written against:** v0.6
- **Shipped in:** —

## Summary

A published Incan package describes what it exports but not how any of it runs. Its manifest carries signatures, checked public metadata, and canonical identities, which is exactly enough for a consumer to typecheck a call into the package and nothing more. The only executable form it ships is a compiled Rust library, which is machine code for one particular backend rather than a representation any other route can interpret. This RFC proposes that a package additionally publish a versioned executable representation of its public surface, keyed by the same canonical identities its manifest already exports, produced by the compilation that declares them.

## Core model

1. **A package's public surface has one identity space.** Canonical identity is minted by the compilation that declares a symbol, and a consumer resolves against those identities rather than against spellings.
2. **Execution is a projection of that surface, not a second description of it.** Whatever a consumer executes for an imported declaration must be reachable from the identity it already resolved, not from a name, a path, or a re-derivation.
3. **A representation is produced once, by the declaring compilation.** Any route that reconstructs a package's executable form from the package's source produces a second identity space, which by construction cannot be compared to the first.
4. **Representations are versioned and refusable.** A consumer that cannot interpret a package's representation must say so, in terms of the package and version it could not use, before it produces a result.

## Motivation

Incan is acquiring execution routes that are not "compile everything to Rust and link it". The replacement backend already executes a program's own modules directly, including calls that leave the module they are written in. It cannot follow the same call into a dependency, and the obstacle is not the call — it is that nothing in the dependency's published form can be executed by anything except the Rust toolchain.

This shows up as an asymmetry a user can see. Splitting a project into two modules keeps it executable. Extracting one of those modules into a package makes the same code unreachable on that route, with no change to the code itself. The boundary that stops execution is a packaging decision, not a semantic one, which is the wrong place for such a boundary to sit.

It also blocks the evidence #989 asks for. Proving that a public contract is preserved across a boundary requires executing across that boundary; a test that can only execute inside one compilation cannot distinguish a preserved contract from a coincidence.

The tempting shortcut is worse than the problem. A consumer can often see a path dependency's source, and could re-derive an executable form from it. Doing so runs a second analysis, and a second analysis mints its own identities: the same declaration acquires a module-scoped identity in the consumer while the manifest continues to carry the package-scoped one. Those identities cannot be compared, so every check built on identity silently weakens to a comparison of spellings. Removing exactly that failure mode is what RFC 120 exists for, and reintroducing it to make execution work would trade a visible limitation for an invisible one.

## Goals

- Define what a package publishes so a non-Rust route can execute its public surface.
- Keep identity minting in the declaring compilation, so a consumer never re-derives what the package already established.
- Version the representation independently of the manifest, so a consumer can refuse an interpretation it does not support without refusing the package.
- Make an unusable or absent representation an explicit, source-owned refusal rather than a silent fallback.
- Let a package ship no representation at all and remain a valid package for the routes that do not need one.

## Non-Goals

- Defining the representation's encoding. This RFC fixes the contract, not the format.
- Replacing the compiled Rust library. Packages continue to ship one, and it remains what the Rust-linking route uses.
- Making a package's private implementation executable. Only the public surface is in scope.
- Cross-version execution guarantees. A representation produced by one compiler release need not be interpretable by another; it must only say so clearly.
- Rust-hosted callers, which RFC 097 owns, and the interop surface a package may expose to Rust.

## Guide-level explanation

Building a library produces its manifest and its compiled artifact as it does today, and additionally an executable representation of the declarations it exports. The author does not write it or name it; it is a product of the same build that produced the manifest, and it describes the same public surface.

Consuming a package is unchanged. An import resolves against the package's public contract exactly as before, and a call into it typechecks the same way. What changes is that a route which does not link Rust can now execute that call, because the package shipped something it can execute.

When it cannot, the failure is specific. A consumer that finds no representation, or one it cannot interpret, reports the package and version it could not use and what it needed, rather than reporting that a call is unsupported. The distinction matters to whoever has to act: the first is a packaging or version problem, the second would suggest a language limitation that does not exist.

A package that ships no representation stays usable. Routes that link Rust behave as they always have; only the routes that need a representation refuse, and only for the calls that actually cross into that package.

## Reference-level explanation

A package **may** publish an executable representation of its public surface. A package that does not **must** remain valid for every route that does not require one.

A published representation **must** identify every declaration it covers by the canonical identity that declaration carries in the package's own manifest. A representation **must not** identify a declaration by source spelling, by module path, or by any generated Rust name.

A representation **must** be produced by the compilation that declares the symbols it covers. A consumer **must not** synthesise a representation for a dependency, including when that dependency's source is available to it.

A representation **must** carry a version distinct from the manifest's. A consumer **must** refuse a representation whose version it does not support, and that refusal **must not** invalidate the manifest or the package.

A representation **must** be self-consistent with the manifest it accompanies: every identity it covers **must** appear in that manifest's public surface, and a representation **must not** cover a declaration the package does not export.

A consumer that requires a representation and cannot obtain a usable one **must** refuse before producing any result or publishing an execution receipt. The refusal **must** name the package, the version, and the requirement that was not met. It **must not** silently select another route, and it **must not** report the condition as an unsupported language construct.

A consumer **must** resolve an imported declaration to its representation through canonical identity alone. Where a package's declaration is a projection of another — an alias, a re-export, or a facade — the consumer **must** resolve to the declaration the identity names rather than to the projection's spelling.

A representation **must not** expose a package's private declarations, its compiler-session state, or its generated Rust layout. A consumer **must not** derive compatibility from any of those, and **must** derive it from the declared version and requirements alone.

## Design details

The representation is a third product of a library build, beside the manifest and the compiled artifact. It is keyed by canonical identity, which is already the manifest's currency, so the two are joined by the identity space rather than by file naming or ordering.

Versioning is separate from the manifest deliberately. A consumer that understands a package's exports but not its representation is in a recoverable position: it can typecheck, report precisely, and continue on a route that does not need the representation. Collapsing the two versions would turn an execution-route limitation into a packaging incompatibility.

Coverage is permitted to be partial. A package may publish a representation for some of its public surface and not the rest, and a consumer refuses per call rather than per package. This keeps the contract usable while the set of executable constructs grows, and it avoids an all-or-nothing gate on a package that exports one construct a route cannot yet execute.

Projections resolve to their target. A consumer calling through an alias, a re-export, or a facade resolves to the declaration the canonical identity names, so a chain of projections does not multiply representations and a facade does not need one of its own.

The compiled Rust artifact is unaffected. It remains the product the Rust-linking route consumes, and nothing here changes how that route selects or verifies it.

## Alternatives considered

**Re-derive the representation in the consumer.** Rejected. A second analysis mints a second identity space, so a re-derived declaration cannot be compared to the manifest's. Every identity-based check would degrade to spelling comparison, which is the failure RFC 120 removed.

**Extend the manifest to carry executable content.** Rejected. The manifest is the public contract and is read for typechecking, inspection, and compatibility. Loading executable content for every consumer that only wants a signature makes the common path pay for the rare one, and couples two things that need to version independently.

**Ship the representation as a separate package.** Rejected. Two packages describing one public surface can drift, and a consumer would have to reconcile two identity spaces at exactly the boundary this RFC exists to keep single.

**Require every package to publish one.** Rejected. It would make packages invalid for routes that never needed a representation, and would gate publication on a route's current capability.

**Interpret the compiled artifact.** Rejected. It is the output of one backend for one target and profile, and reading it back would make every route depend on that backend's layout — the opposite of a route-neutral contract.

## Drawbacks

A package build produces more, and a published package is larger, for a benefit only some routes use. The partial-coverage rule and the optionality of the representation limit that cost but do not remove it.

Two independently versioned products describing one surface can disagree. The self-consistency rule constrains what a valid package may publish, but a consumer must still handle the case, and "the manifest says this exists but the representation does not cover it" becomes a state that has to be reported well.

A representation is a second thing to keep correct as the language grows. A construct that becomes executable is not executable across a package boundary until the representation covers it, and that lag is visible to users as a route difference.

## Implementation architecture

*Non-normative.*

The natural producer is the same library build that emits the manifest, because it already holds the checked public surface and the identities the representation must use. The natural consumer boundary is wherever a route resolves an imported declaration, so that a missing or unusable representation is discovered at resolution rather than part-way through execution.

Coverage is likely to grow along the same axis as executable constructs generally, which suggests recording coverage explicitly rather than inferring it from what happens to be present.

## Layers affected

- **Typechecker / Symbol resolution**: resolving an imported declaration to a published representation through canonical identity, and reporting when none is usable.
- **Emission**: producing the representation for a library's public surface as part of the same build that produces its manifest.
- **Tooling**: publishing and locating the representation beside a package's other products, and surfacing version and coverage in inspection output.

## Unresolved questions

- Should a representation's coverage be declared explicitly, or inferred from the identities it contains?
- Should a consumer be able to require a representation, so that a package lacking one fails at resolution rather than at the first call that needs it?
- What is the right granularity for the representation's version: one version for the whole representation, or per covered construct class?
- Should a package be permitted to publish a representation for a declaration whose executable form differs from what the Rust-linking route would produce, or must the two agree by construction?

<!-- Rename this section to "Design Decisions" once all questions have been resolved.
     An RFC cannot move from Draft to Planned until no unresolved questions remain. -->
