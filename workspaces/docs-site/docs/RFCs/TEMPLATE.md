# RFC Template

> Use this template for RFCs in `docs/RFCs/`. Keep the RFC focused: one coherent proposal, with clear motivation,
> semantics, and implementation strategy.

## Title

RFC NNN: \<short descriptive title\>

## Status

- Status: Draft | In Progress | Implemented | Deferred
- Author(s): \<name/handle\>
- Issue: \<link to issue\>
- RFC PR: \<link to PR\>

## Summary

One paragraph describing what this RFC proposes.

## Motivation

Explain the problem and why it matters:

- What’s painful/confusing today?
- Who benefits?
- Why is this better than the status quo?

## Guide-level explanation (how users think about it)

Explain the feature as a user would understand it. Include examples.

```incan
# Example code
```

## Reference-level explanation (precise rules)

Define exact semantics, typing rules, and edge cases.

- Syntax changes (grammar-ish description, if needed)
- Type checking rules
- Runtime behavior
- Errors / diagnostics

## Design details

### Syntax

Describe new/changed syntax.

### Semantics

Describe behavior precisely.

### Interaction with existing features

How this composes with:

- async/await
- traits/derives
- imports/modules
- error handling (Result/Option)
- Rust interop

### Compatibility / migration

- Is this breaking?
- If yes, provide a migration strategy and examples.

## Alternatives considered

List plausible alternatives and why they’re worse.

## Drawbacks

What does this cost (complexity, performance, mental model)?

## Implementation plan

Concrete steps (expected touchpoints):

- Frontend changes (lexer/parser/AST/typechecker)
- Backend changes (IR/lowering/emission)
- Stdlib/runtime changes
- Tooling changes (fmt/test/LSP)
- Tests to add (unit/integration/fixtures)

## Unresolved questions

Open questions to decide before implementation lands.
