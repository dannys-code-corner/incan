# GPU Target Capability and Graphics Contracts

| Field | Value |
| --- | --- |
| Status | Deferred from v0.6 / RFC 092 |
| Tentative revisit | v0.8 |
| Tracking issue | #1041 |

GPU support is optional for Incan's current release thesis. It is not required for self-hosting, Oven authority, the direct-HIR backend cutover, or the first interactive runtime target. This document records the decision to defer GPU target capability and graphics contracts from RFC 092 and preserves the integration constraints needed when the work is revisited.

## Decision

GPU target capability and graphics contracts are deferred from v0.6. The follow-up should be a separately designed feature, not an extension of the current RFC 092 scope. Work should begin with a concrete runtime-consumer spike rather than speculative GPU infrastructure.

## Non-goals for now

- No GPU codegen.
- No renderer or UI framework.
- No graphics framework integration owned by the compiler.
- No shader pipeline, shader translation, or shader authoring surface.
- No device abstraction or typed GPU resource handles yet.

## Revisit trigger

Reopen this design when a concrete runtime consumer needs one of:

- GPU capability discovery or advertisement.
- GPU resource lifetime or sharing semantics.
- Graphics interop with an existing runtime or host API.

The revisit should identify the first consumer before defining compiler or stdlib surface.

## Integration constraints

When GPU support is revisited, it must satisfy these constraints:

- Target and capability advertisement must remain optional and queryable.
- GPU artifacts and receipts must reuse the existing artifact, receipt, and diagnostic contracts.
- Capability gating must not force host or runtime coupling into the core compiler.
- The first public surface must explicitly decide between metadata-only capability reporting and typed resource handles.
- Compiler authority should stop at target capability, diagnostics, and artifact references; rendering and product semantics belong downstream.

## Acceptance checklist for the follow-up design

Before implementation, the follow-up design should answer:

1. Which concrete consumer or runtime scenario requires GPU capability?
2. Is the first surface metadata-only capability reporting, or does it include typed resource handles?
3. How is GPU target availability advertised, queried, and disabled?
4. How are unavailable devices, unsupported capabilities, and runtime failures reported through existing diagnostics?
5. How do GPU artifacts and receipts integrate with existing build reports and receipt contracts?
6. How does the design avoid adding a graphics framework or renderer to the compiler?
7. What feature-gating and compatibility story is required for optional GPU targets?

## Open questions

- Device identity and capability metadata schema.
- Relationship between GPU capabilities and existing target triples or runtime profiles.
- Ownership of shader artifacts and compilation receipts.
- Safety boundaries for typed handles if they are introduced.
- Interop story for Rust graphics crates without making them compiler authority.
