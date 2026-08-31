# RFC 120: canonical source symbol identity

- **Status:** In Progress
- **Created:** 2026-08-19
- **Author(s):** Danny Meijer (@dannymeijer)
- **Related:**
    - RFC 022 (stdlib namespacing and soft keywords)
    - RFC 025 (multi-instantiation trait dispatch)
    - RFC 083 (symbol and method aliases)
    - #546 (`pub`/`rust`/`std` namespace-root import syntax)
    - #652 / #987 (v0.6 backend cutover and its parity corpus)
    - #699 (v0.4 symbol-identity pass)
    - #1072 (plain-assignment scope-lookup bug)
    - #1116 (builtin-function shadowing contract)
    - #1125 (destructuring `for` patterns in Body IR)
    - #1132 (statement-level tuple unpack of a non-tuple value)
    - #1174 (recoverable emitted-name projection)
- **Issue:** [#1042](https://github.com/encero-systems/incan/issues/1042)
- **RFC PR:** —
- **Written against:** v0.5
- **Shipped in:** —

## Summary

Every named source object — a local declaration, an import, an alias, a re-export, a generic binder, or a member — gets one canonical symbol identity, established once at its declaration site. A resolved reference carries that identity through every later stage: HIR, Body IR, diagnostics, the LSP, Oven inspection, codegraph export, and backend emission. Source spelling and any emitted Rust name are projections of that identity for a given consumer; neither is ever the source of truth for what a reference means. Where backend emission creates a linker-visible symbol for an Incan-origin declaration, that projection must also be recoverable to the canonical identity it represents without a separate side-car artifact.

## Motivation

The direct-HIR v0.6 backend cutover removes generated Rust as the normal semantic handoff between compiler stages. Up to now, several tools and stages have been able to lean on emitted Rust names, directly or indirectly, to recover what a piece of source actually referred to. That option goes away once the replacement backend is the normal path: a diagnostic, an LSP hover, a codegraph edge, and the compiled artifact all need to agree on what a name means using only the compiler's own identity model, not by comparing generated output.

Today that model is incomplete. The earlier v0.4 symbol-identity pass intentionally excluded new namespace features — it settled identity for what existed then, not for how imports, aliases, re-exports, generic binders, and members should all resolve onto one shared identity space. Without that, it is possible for two compiler stages to reasonably disagree about whether two references mean the same thing, which is exactly the class of defect this RFC exists to close off before the backend cutover makes generated Rust an unavailable fallback.

This RFC deliberately does not introduce new binding syntax. `let name = value` and `mut name = value` already exist in Incan (see `scopes_and_name_resolution.md`) as the explicit forms for introducing a new binding in the current scope, including one that deliberately shadows an outer binding; plain assignment already documents that it should reassign the nearest active binding rather than create a new one.

Two gaps between that documented contract and the current implementation exist, and both are bugs against an already-decided contract rather than new design questions. Plain assignment inside a nested block creates an undiagnosed shadow instead of reassigning the outer binding (see Related). And the typechecker does not read the binding form at all: assignment checking inspects `BindingKind` only to decide mutability, so `let x = ...` and plain `x = ...` are checked identically, and `let`'s shadowing semantics are realized solely by the `let` in the emitted Rust. That second gap is the sharpest available statement of why this RFC comes before the cutover: a documented source-level binding contract currently has no frontend representation at all, and would simply disappear the moment generated Rust stops being the semantic handoff. This RFC assumes both gaps get fixed to match the documented contract, and builds canonical identity on top of the binding model as already specified.

## Guide-level explanation

Canonical identity is mostly invisible when things work — that is the point. It becomes visible whenever more than one compiler surface needs to agree about what a name refers to.

```incan
# lib.incn
def helper() -> int:
    return 42

# main.incn
from lib import helper as h

def main() -> int:
    return h()
```

`h` is a binding, not a second declaration. Hovering `h()` in an editor, a diagnostic naming that call, and a codegraph edge for that call all resolve to the same canonical identity — the `helper` declared in `lib.incn` — never a separately tracked identity for the alias `h`. If `helper` is later renamed at its declaration site and `main.incn`'s import is updated to match, every tool that already resolved through the canonical identity sees one consistent rename, not two unrelated symbols that happen to share a spelling.

The same guarantee extends to re-exports and generic binders:

```incan
# api.incn
from lib import helper as h
# re-export: `h` is now also part of api's public surface

# consumer.incn
from api import h as run

def main() -> int:
    return run()
```

`run` in `consumer.incn`, `h` in `api.incn`, and `helper` in `lib.incn` are three bindings to one canonical identity. A diagnostic raised while type-checking `run()` names the original declaration's identity, not an intermediate alias hop; an LSP "go to definition" on any of the three spellings lands on the same declaration.

## Reference-level explanation

### Canonical identity

Every named source declaration is assigned one canonical symbol identity at its declaration site. That identity does not change based on how the declaration is later referenced. This covers: a function, model, class, trait, enum (and each of its variants), field, method, generic type parameter, a module-level or block-level binding introduced by `let` or plain assignment under its documented reassignment rules, a `for` loop variable, a `with ... as` binding, and an `except ... as` binding. All of these are ordinary bindings into a lexical namespace; none gets a separate identity model just because of which statement form introduced it.

A decorator's own name (for example `derive` in `@derive(CliArgs)`, or a trait name passed as its argument) is an ordinary reference and resolves through the same canonical identity as any other reference; a decorator application does not introduce a new kind of binding itself.

An import, an alias, and a re-export are bindings to an existing canonical symbol. None of the three creates a second canonical identity for the thing they name. A local declaration and an imported declaration differ only in how their binding enters scope, not in what kind of identity they carry.

A generic binder (a type parameter) has its own canonical identity, scoped to the declaration that introduces it, distinct from any concrete type that instantiates it.

A member (a field or method reached via `.`) resolves to the canonical identity of its declaration on the owning type, not to a fresh identity per access site.

### Namespace table

Incan has three namespaces, distinguished by *how* a name is looked up rather than by what kind of thing it names. This is deliberately not the type/value split rejected under Alternatives: a model name and a function name share one namespace here, exactly as ordinary Python-like lexical lookup expects.

| Namespace | Binds | Lookup | Scope unit |
| --- | --- | --- | --- |
| **Ordinary lexical** | module-level declarations (function, model, class, trait, enum, newtype, `rusttype`, type alias, `const`, `static`, partial); imports, aliases, and re-exports; bare enum variant names; generic binders; parameters and receivers (`self`, `cls`); locals introduced by `let`, `mut`, or a first plain assignment; `for` variables; `with ... as`; `except ... as` | innermost scope outward through the scope chain, then the builtin fallback tier | module, function/method, closure, block, comprehension — and, for a generic binder, the declaration that introduces it |
| **Members** | fields, methods, computed properties, method aliases, and qualified enum variants | `.`-directed from a resolved owner type; never through the scope chain | the owning nominal type, including its inherited surface |
| **Module and package paths** | project module paths, the `std` root, `rust::` crate roots and item paths, and `pub::` library roots | path-directed from a namespace root | the compilation's module and package graph |

Beneath the ordinary lexical namespace sits one **builtin fallback tier**: core builtin functions such as `len`, `sum`, and `zip`, plus builtin type spellings and their registry aliases. It is consulted only after the whole scope chain misses, so a real lexical binding — declared or imported — wins over an unprotected ambient builtin. That is settled behavior with a documented contract and tests (see Related). Rebinding an unprotected builtin-function spelling is therefore **not** a collision, and the shared mechanism below must not begin reporting one; `std.builtins.<name>` remains the explicit way to reach the core builtin from a scope that has rebound the spelling. The protected `print` and `println` builtins are exceptions: attempts to rebind either name are compile-time errors.

Namespaces do not shadow one another. A field named `items` never shadows a local named `items`, and a module path segment never shadows either. Generic binders are ordinary lexical bindings with a declaration-bounded scope rather than a fourth namespace: what makes a binder distinct is its identity kind and the scope it is bounded to, not a separate lookup rule. Today they are registered as builtin-kinded type symbols, which is why a binder and a concrete type are currently indistinguishable to anything downstream.

### One shared namespace/binding-resolution mechanism

Today, collision and ambiguity detection for named things is scattered: `duplicate_library_export`, `duplicate_rust_module`, `duplicate_trait_instantiation`, `duplicate_alias`, `pub_library_import_name_collision`, and `pub_library_module_member_ambiguous` each independently implement their own narrow check for their own construct. They are scattered across compiler layers, not merely across files: the library-export check is decided in the build command, the duplicate-`rust`-module check is raised in the parser, and the rest are spread through declaration checking, import collection, and expression checking. That fragmentation is itself a source of the defects this RFC exists to close: two constructs can reasonably disagree about whether a collision is a collision, because each runs its own logic instead of consulting one shared answer — and a check living in the CLI cannot agree with one living in the frontend by construction.

This RFC requires one shared namespace/binding-resolution mechanism that every binding kind — local declarations, imports, aliases, re-exports, library exports, trait instantiations, generic binders, and members — registers into and is checked against. One check is deliberately exempt: the duplicate-`rust`-module check runs in the parser, before any resolver exists to consult, and keeps its own narrow implementation (see Design decisions). Collision and ambiguity detection is a property of that one mechanism, not something each construct's typechecker code independently decides. A specific diagnostic's wording may still vary by construct for a good user-facing message (an ambiguous `pub::` import and an ambiguous generic instantiation don't need identical phrasing), but the underlying question — "does this binding collide with, or ambiguously resolve against, an already-active binding for this spelling in this namespace" — is answered once, by one mechanism, for every construct.

### Namespace rule

At every program point, one spelling resolves to at most one active binding in a given ordinary lexical namespace. This RFC does not redefine how a binding becomes active — that is the existing `let` / `mut` / plain-assignment / import contract in `scopes_and_name_resolution.md`, once the known scope-lookup bug against that contract is fixed (see Related). This RFC defines what a binding carries once it is resolved (its canonical identity) and requires that "is this binding active, and does it collide with another" be answered by the one shared mechanism above, not reimplemented per construct.

### Canonical identity payload

A canonical identity is a compiler-owned value, not a rendered string. It carries exactly the data a downstream stage needs to answer "do these two references mean the same thing" without comparing spellings.

| Field | Purpose | Notes |
| --- | --- | --- |
| `namespace` | Which of the three namespaces above the binding lives in | Keeps a member and a local that share one spelling from ever comparing equal |
| `origin` | The module, package, `rust::` crate, or builtin registry owning the declaration | An import, alias, or re-export carries its target's `origin`, never the importing module's |
| `declaration_name` | The spelling at the **declaration** site | Never the spelling at the reference site, so `run`, `h`, and `helper` in the guide-level example all carry `helper` |
| `kind` | Declaration category: function, model, class, newtype, `rusttype`, enum, variant, trait, field, method, property, `const`, `static`, local, parameter, receiver, generic binder, module, Rust item, builtin | Today's recorded target kinds cover functions and four type kinds; every other kind resolves to nothing |
| `scope_discriminant` | Distinguishes bindings that are not unique within their `origin` | Required for locals, parameters, receivers, generic binders, and any block-scoped binding; module-level declarations are already unique within their origin |
| `declaration_span` | Provenance anchor | One span, at the one declaration site |

Two properties are load-bearing. Identity equality must be decidable without string comparison of source spellings or emitted names. And identity must be stable across the stages of **one** compilation; it is not required to be stable across edits to the source, because `declaration_span` moves when the file changes.

The `scope_discriminant` is the piece today's model is missing most concretely. A local's identity is currently built from module path plus spelling, so two `x` bindings in sibling blocks of one module collapse to the same identity — while a declaration's identity is built from the *referencing* module plus spelling, so an alias splits away from the declaration it names. Those are the two halves of one defect: identities that should differ collapse, and identities that should coincide split.

### Recoverable emitted-name projection

Emission has two deliberately separate contracts:

1. **Semantic resolution is one-way.** No compiler phase may determine what source binding a reference means by parsing an emitted Rust name. The resolver, HIR, Body IR, diagnostics, LSP, Oven facts, and codegraph use compiler-owned canonical identity facts. An emitted name never becomes semantic authority.
2. **Artifact observation is recoverable.** When an Incan-origin declaration becomes a linker-visible emitted symbol, its emitted identifier carries a versioned, reversible encoding of its canonical identity. A backtrace or artifact inspector can decode that payload to the declaration identity without loading generated Rust, re-resolving source, or consulting a source-map sidecar that could drift from the artifact.

The initial projection format is an Incan-owned `incan-v1` payload, encoded with a Rust-identifier-safe reversible alphabet and carried in the emitted item's unmangled identifier. It contains the complete canonical identity payload — namespace, origin, declaration name, kind, scope discriminant, and declaration span — plus a format version. A decoded payload is therefore the identity for the artifact that carries it, rather than a lookup key that needs a source map or another companion artifact. Generic specialization data is included whenever it is necessary to distinguish linker-visible instantiations of one declaration.

The implementation must prove that the selected Rust toolchain's v0 mangling and demangling preserve the complete `incan-v1` payload. #1174 records the intended toolchain dependency and must add the missing DD-0002 toolchain decision before implementation relies on a particular compiler version. Until that record and its fixture exist, this RFC states the requirement, not an unverified claim about a Rust release.

This applies only to Incan-origin declarations that materialize as linker-visible symbols. Locals and other source declarations that do not materialize as an emitted symbol are not counterexamples; a future backtrace locates them through their nearest recoverable Incan-origin frame. A frame with no Incan origin is classified as runtime, host, or interop and may be collapsed at normal verbosity or shown at an explicit verbose setting. It is never guessed to be an Incan declaration.

The payload may be compacted only through another reversible encoding that passes the same independent decoding fixture. There is no release-mode setting that silently removes recoverability. The binary-size and emitted-name-length impact must be measured on a representative release artifact before the cutover gate is declared complete.

### Propagation through the pipeline

A resolved reference's canonical identity must be preserved, not recomputed from spelling, at every stage that consumes it:

- **HIR / Body IR** — a reference node carries the canonical identity, not only a source span or spelling.
- **Diagnostics** — an error or warning naming a symbol names its canonical identity's declaration site, regardless of which binding (local, import, alias, re-export) the offending reference used.
- **LSP** — "go to definition," "find references," and hover all resolve through canonical identity, so every binding to one declaration reports the same definition site.
- **Oven inspection / codegraph export** — an edge or fact referencing a symbol identifies it by canonical identity, so tooling can determine that two differently-spelled references mean the same thing without string comparison.
- **Backend emission** — the emitted Rust name for a canonical identity is a projection for that backend; it must not become a second identity another stage compares against, and every linker-visible Incan-origin projection must carry the recoverable `incan-v1` payload.

Every one of those stages recomputes from spelling today, which is what makes this a sequence of real slices rather than a threading exercise. Declaration-level HIR derives its ids from module path plus name, and gives an import declaration no name at all. Body IR resolves identifiers through a flat name-to-local map and synthesizes an opaque external local, with unknown ownership, for any name it cannot find — so a name the resolver resolved perfectly can still arrive at lowering unresolved. The LSP answers "go to definition" with a first-match scan over the current module's declarations, consulting no typechecker output, and therefore cannot follow an import at all. Codegraph keys its records on a `(module path, name, kind)` string triple and silently drops an edge whose triple is unregistered.

### Diagnostics

A duplicate declaration or import that would introduce a second active binding for the same spelling in the same namespace at the same program point, without going through an explicit `let` or `mut` shadowing form, is a diagnostic, raised by the one shared namespace/binding-resolution mechanism rather than by construct-specific checks. An import whose target cannot be resolved to exactly one canonical identity — because more than one visible declaration could satisfy it — is a diagnostic naming the candidates, raised by that same mechanism.

Existing construct-specific diagnostics that already implement a narrower version of this check (for example `duplicate_alias`, `duplicate_trait_instantiation`, `duplicate_library_export`) are expected to become call sites of the shared mechanism rather than parallel implementations; their user-facing wording may stay construct-specific, but the collision logic behind them should not. `pub_library_import_name_collision` remains tied to the separate `pub`/`rust`/`std` namespace-root import syntax (see Related) and is out of this RFC's scope to consolidate.

## Design details

### Binding-form decisions

These are the decisions the implementation must encode. None changes accepted source syntax; each states which binding an existing form targets and what identity it produces.

- **`let name = value`** introduces a new binding in the current scope and gives it a fresh canonical identity, even when an outer binding for that spelling is active. This is the explicit shadowing form. It must become a frontend-modeled decision, because the typechecker currently reads the binding form only for mutability.
- **`mut name = value`** behaves identically to `let` for binding and identity purposes, and additionally marks the new binding mutable. It is equally a shadowing form. It is **not** "make the existing `name` mutable": it declares a new binding that happens to be mutable, and reading it as a modifier on an already-active name inverts what it does to identity. Any claim that `let` is the *only* form introducing a binding over an active one is imprecise and must not be carried into implementation or tests.
- **Plain `name = value`** resolves outward through the scope chain and reassigns the nearest active binding, preserving that binding's canonical identity and requiring both `mut` and a compatible type. It introduces a new binding, with a new identity, only when no active binding for that spelling exists anywhere in the chain. It is never a silent shadow.
- **A duplicate declaration or import** — a second binding for one spelling in one namespace, in the same scope, through a form that is not `let` or `mut` — is a diagnostic from the shared mechanism. Rebinding a core builtin-function spelling is explicitly not this case.
- **An ambiguous import** — an import whose target resolves to more than one visible canonical identity — is a diagnostic naming the candidate declarations by identity and span, from that same mechanism.
- **A bare enum variant name** binds into the ordinary lexical namespace without displacing an already-active binding for that spelling. The variant keeps its own canonical identity and stays reachable through its qualified member path; only the bare spelling defers. This preserves existing behavior and does not weaken the namespace rule.

### Interaction with the scope-lookup bug

This RFC's identity model assumes plain assignment, `let`, and `mut` behave as decided above once the known scope-lookup bug (see Related) is fixed. That fix has two halves and they must land together. Making plain assignment walk outward is the half the bug report names; honoring `let` and `mut` as binding-introducing forms is the half that keeps documented shadowing working, because a lookup that walks outward *without* that change would turn every in-block `let` into a reassignment of the outer binding.

The identity model as *specified* is independent of that fix: it applies equally to whichever binding is active under the corrected contract. The identity work as *delivered* is not, and is sequenced behind it (see Implementation plan). An identity assigned to a binding the checker resolved to the wrong scope is a precise name for the wrong thing, and no conformance fixture exercising shadowing can pass before the fix lands.

### Reconciliation with in-flight source-semantics work

- **#1072 (assignment semantics)** lands after #1132 and before every slice in this RFC, and must cover both halves explicitly: plain assignment finds the nearest enclosing binding and reassigns it, and `let` and `mut` introduce a binding that may shadow an active outer one. It owns that fix; this RFC contributes the requirement that both halves ship together, because they edit one assignment-checking path and either half alone leaves the documented contract broken in a different way.
- **#1132 (statement-level tuple unpack of a non-tuple value)** finishes first and owns the statement-checker lane until it does. Its fix corrects an arity guard that a non-tuple fallback renders unreachable; it is independent of identity in semantics and adjacent to it in code. The interaction to respect afterwards is that the statement-level unpack carries its own binding form, so the `let`/`mut`/plain-assignment decision must be applied to the multi-name unpack path rather than left to diverge from single-name assignment.
- **#1125 (destructuring `for` patterns)** has already landed the loop half. Its loop-pattern bindings are ordinary lexical bindings and need identities like any other, and Body IR already binds them through the flat name-to-local map that the Body IR slice replaces.
- **#1116 (builtin-function shadowing)** has already settled the lexical contract for unprotected builtins: a real local or imported binding wins over the ambient builtin, and `std.builtins.<name>` is the explicit escape hatch for reaching the core builtin from a scope that has rebound the spelling. That contract becomes a conformance case for the identity work rather than only a thing not to regress — the rebound spelling and the qualified builtin must resolve to two different canonical identities, and the rebinding itself must report nothing. The protected `print` and `println` names are excluded: attempts to rebind them are compile-time errors.

### Interaction with existing features

- **Imports and modules** — RFC 022's stdlib namespacing and canonical `std` root are unaffected; this RFC's identity model applies to `std` symbols the same as any other declaration.
- **Aliases** — RFC 083 already establishes that an alias preserves its target's semantic identity for imports, diagnostics, documentation, and metadata rather than acting as a copy or wrapper. This RFC generalizes that same principle to the full identity model rather than introducing a competing one.
- **Generic binders** — RFC 025's multi-instantiation trait dispatch resolves a call to one of several adopted instantiations by argument/return type; this RFC's canonical identity for a generic binder is what that dispatch resolves against, not a per-instantiation identity.
- **Rust interop** — an emitted Rust name remains a backend-specific projection of a canonical identity; this RFC does not change what interop code can call, only how the compiler tracks what a reference means before emission. Native runtime, host, and third-party Rust frames have no Incan identity unless the compiler emitted an Incan-origin projection for them; inspection classifies those frames rather than inventing source provenance.

### Compatibility / migration

This RFC does not change accepted source syntax and is not expected to be a breaking change for correctly-typed existing programs. Programs that were relying on the undocumented, buggy shadowing behavior described above may see new diagnostics once that bug is fixed to match its own documented contract; that migration story belongs to the bug fix, not this RFC.

## Alternatives considered

### Let the backend or generated Rust resolve collisions

Rejected because source meaning would then vary by backend, and tooling could not inspect one authoritative result — exactly the dependency on generated Rust the v0.6 backend cutover is removing.

### Reopen the earlier v0.4 identity pass unchanged

Rejected. That pass was completed and explicitly excluded new namespace syntax; its scope does not cover imports, aliases, re-exports, or generic binders as one identity space.

### Use Rust-style separate type and value namespaces

Rejected because it introduces a second mental model that does not match Incan's Python-like ordinary lexical lookup, and nothing about the canonical-identity problem requires splitting the namespace to solve.

### Emit a side-car artifact-to-identity map instead

Rejected as the primary recovery mechanism. A side-car can be missing or describe a different artifact, reintroducing a second source of truth precisely where user-facing artifact inspection needs an answer. Compiler facts may enrich a decoded identity with spans or source context, but decoding the emitted symbol itself must establish its identity.

## Drawbacks

- Threading one canonical identity through every pipeline stage (HIR, Body IR, diagnostics, LSP, codegraph, backend) is real engineering work across most of the compiler, not confined to one layer.
- Conformance fixtures for the full matrix of local/import/alias/re-export/generic-binder/member combinations add meaningful test surface.
- A reversible emitted-name payload lengthens symbol names and can increase artifact size. v0.6 accepts that cost only with a recorded measurement and a fixture proving that mangling and demangling preserve recovery.

## Layers affected

- **Parser** — must attach enough declaration-site information for every named source object to be assigned a canonical identity later, including the binding form it already records; must not itself resolve identity.
- **Typechecker / resolver** — must resolve every import, alias, re-export, generic binder, and member reference to the canonical identity of its underlying declaration; must consolidate collision and ambiguity detection into one shared namespace/binding-resolution mechanism rather than construct-specific checks, and migrate existing construct-specific diagnostics to call sites of that mechanism — including the duplicate-library-export check that is decided in the build command today, and excluding the two checks recorded as exemptions under Design decisions.
- **HIR / Body IR** — must carry canonical identity on reference nodes rather than recomputable spelling alone.
- **Diagnostics** — must report a symbol's canonical declaration site regardless of which binding a reference used to reach it.
- **LSP** — must resolve "go to definition," "find references," and hover through canonical identity.
- **Codegraph / Oven inspection** — must key symbol edges and facts by canonical identity, not string spelling.
- **Backend emission** — must treat an emitted Rust name as a projection of canonical identity, never a second identity another stage compares against; must encode the versioned, reversible `incan-v1` identity payload in every linker-visible Incan-origin symbol; and must prove the selected Rust toolchain preserves it through v0 mangling and demangling.

## Inspectability and tooling surface

- **Artifact or metadata:** codegraph export already reports declarations, references, and calls; this RFC requires those records to carry canonical identity so two differently-spelled references to one declaration are visibly the same fact, not two. A linker-visible Incan-origin symbol independently carries a recoverable `incan-v1` projection, so an artifact observer can establish its identity even when no codegraph or source-map side-car is present.
- **Inspection command:** `incan inspect codegraph --format jsonl` is the existing surface; no new command is introduced.
- **Diagnostics:** duplicate-binding and ambiguous-import diagnostics name the conflicting declarations by canonical identity and source span.
- **Provenance:** every canonical identity anchors to the source span of its one declaration site.
- **Not implicit:** an alias, re-export, or generic instantiation never silently becomes a second identity; a rename at a declaration site is visible as one consistent change everywhere the compiler reports that identity.

## Implementation plan

This plan is the governing delivery map for the work. Two changes land before any slice below, in this order, and neither belongs to this RFC.

1. **#1132 — statement-level tuple unpack of a non-tuple value.** It finishes first because it owns the current statement-checker lane. Its fix is independent of identity in semantics, but it edits the same assignment and unpack region that the binding-form work rewrites, so running the two concurrently buys nothing and costs a merge.
2. **#1072 — assignment semantics, both halves.** Plain assignment must find the nearest enclosing binding and reassign it; `let` and `mut` must introduce a binding, shadowing an active outer one where present. `mut` is not "make an existing binding mutable" — it declares a new mutable binding, and reading it as a mutability modifier on an existing name is the misreading that must not reach implementation or tests. Owning modules: `src/frontend/typechecker/check_stmt.rs` and `src/frontend/symbols.rs`, applying the same rule to the multi-name unpack path. Conformance: the `reassigns_outer`, `shadows_in_block`, and `shadow_vs_reassign` examples from `scopes_and_name_resolution.md` promoted to executable fixtures, plus both repro cases in #1072.

The canonical-identity slices follow #1072 because correct binding semantics are their foundation: an identity assigned to a binding the checker resolved to the wrong scope is a precise name for the wrong thing.

The slices below are then dependency-ordered and deliberately narrow. Each names the modules that own it and the conformance evidence that proves it. Slices 1-3 are frontend-only and land before anything downstream consumes an identity; slices 4-8 take one consumer each, so a regression in a consumer is attributable to one slice.

### Slice 1: Canonical identity type and declaration-site assignment

Introduce the identity value and assign one at every declaration site, changing no behavior and no diagnostic. Owning modules: `src/frontend/symbols.rs` for assignment at symbol definition, and `crates/incan_semantics_core/src/facts.rs` for the identity type itself, alongside the existing compiler node identity. Conformance: unit coverage proving that two same-spelled bindings in sibling blocks receive different identities, that a generic binder's identity differs from the concrete type instantiating it, and that a module-level declaration's identity is independent of how often it is referenced.

### Slice 2: Reference-side identity recording

Generalize the existing source-target recording so every resolved reference records an identity, rather than only the calls and type uses that carry one today. Owning modules: `src/frontend/typechecker/mod.rs` for the recording and symbol-origin helpers, and `src/frontend/typechecker/type_info.rs` for the recorded target shape. The existing string-shaped target stays as a projection through this slice so codegraph does not have to move in the same change. Conformance: the RFC's own alias and re-export examples, asserting all three spellings record one identity, plus a case proving an imported and a local reference to one declaration compare equal.

### Slice 3: One shared binding-registration and collision mechanism

Give symbol definition a single entry point that answers "does this binding collide with, or ambiguously resolve against, an already-active binding for this spelling in this namespace", and migrate the construct-specific checks to call it. Owning modules: `src/frontend/symbols.rs` for the mechanism; `src/frontend/typechecker/check_decl.rs` for the duplicate-alias and duplicate-trait-instantiation checks; `src/cli/commands/build.rs` for the duplicate-library-export check, which currently lives in the CLI rather than the frontend and should move with it. Explicitly excluded: the duplicate-`rust`-module check, which is raised in `crates/incan_syntax/src/parser/core.rs` and cannot consult a resolver that has not run, and the `pub`-library import-collision check, which stays with #546. Conformance: each migrated diagnostic keeps its existing wording and span under its existing tests, plus a case proving a rebound unprotected builtin-function spelling still reports nothing while rebinding protected `print` or `println` remains a compile-time error.

### Slice 4: HIR carries identity

Stop deriving declaration ids from module path plus spelling, and give import declarations the name and identity they currently lack. Owning modules: `src/frontend/hir.rs` and `crates/incan_semantics_core/src/facts.rs`. Conformance: a snapshot proving an aliased import and its target declaration share one identity.

### Slice 5: Body IR resolves by identity

Replace name-keyed local resolution so a resolved reference cannot degrade into a synthesized external local with unknown ownership. Owning module: `src/frontend/body_ir.rs`. Conformance: a case proving a shadowed spelling binds the correct local in each scope, and that no reference the resolver resolved arrives as an external local. This slice needs the 0.6 backend line and cannot be developed against a base predating Body IR.

### Slice 6: Diagnostics name declaration sites

Make a diagnostic that names a symbol report its canonical declaration site regardless of which binding the offending reference used. Owning modules: the diagnostics catalog under `crates/incan_syntax/src/diagnostics/`, and the typechecker call sites that build symbol-naming messages. Conformance: a diagnostic raised through an alias naming the original declaration's span.

### Slice 7: LSP resolves through identity

Retain the checked fact snapshot on the LSP document state and answer definition, references, and hover from it, instead of the current first-match declaration scan that consults no typechecker output. Owning module: `src/lsp/backend.rs`. Conformance: definition from each of the three spellings in the re-export example landing on one declaration.

### Slice 8: Codegraph and one-way backend projection

Key codegraph records on identity rather than the `(module path, name, kind)` string triple, and add a guard that no compiler phase recovers a source binding by reading an emitted name. Owning modules: `src/cli/commands/codegraph.rs`, plus the emission path for the projection guard. Conformance: a `jsonl` export in which two differently-spelled references to one declaration are visibly one fact, and a case proving an edge is no longer dropped when its triple is unregistered.

### Slice 9: Recoverable emitted-name projection (#1174)

Define and emit the versioned `incan-v1` payload for every linker-visible Incan-origin declaration, then add the independent artifact decoder that an inspector and a future user-facing backtrace consume. Owning modules: the direct-HIR emission path and the artifact-inspection boundary; the exact toolchain version and v0-mangling guarantee must be recorded in DD-0002 before this slice relies on them. Conformance: a release-mode artifact fixture covering an ordinary declaration and a generic specialization, decode after v0 mangling and demangling, classify native runtime and interop frames as non-Incan, and record the release-artifact size delta. This slice does not implement the backtrace UX itself; it provides the contract that consumer needs.

### Cutover conformance

The identity guarantees that must not regress at the v0.6 backend cutover belong in the backend-parity corpus rather than only in frontend unit tests, so a replacement backend cannot silently lose them. The matrix worth pinning is the cross-product of binding entry (local, import, alias, re-export), namespace (lexical, member, path), and scope nesting (module, function, block), plus explicit shadowing with `let` and with `mut`, one generic-binder case, and #1116's builtin contract: a rebound unprotected builtin-function spelling and the same name reached through `std.builtins.<name>` must carry two different canonical identities across the cutover. The protected `print` and `println` builtins are excluded because attempts to rebind them are compile-time errors. It also includes a release-artifact decode of the `incan-v1` payload after v0 mangling and demangling, plus classification fixtures proving non-Incan frames are never reported as source declarations.

## Progress Checklist

Items tick as their PRs merge. The slice structure above remains the governing map; this checklist is its trackable projection.

### Predecessors (not owned by this RFC)

- [x] #1132 statement-level tuple unpack lands first.
- [x] #1072 assignment semantics, both halves, land together.

### Identity core (Slices 1–2)

- [ ] One compiler-owned identity mint at symbol definition, covering module declarations, locals, consts, statics, parameters, receivers, and generic binders, with scope discriminants.
- [ ] Import, alias, and re-export bindings carry their resolved target's identity; unproven bindings carry none.
- [ ] Builtin registry identities: every alias spelling records the one canonical registry identity.
- [ ] Reference-side identity recording keyed by reference span, with the string-shaped source target retained as a projection.
- [ ] Method declarations carry member-namespace identities; field/property declaration identities generalized.
- [ ] Conformance: sibling-scope distinctness, alias/re-export equality, `let`/`mut` shadowing, builtin-rebinding distinctness, duplicate-declaration identity distinctness.

### One shared binding mechanism (Slice 3)

- [ ] Single binding-registration/collision entry point; duplicate declarations and imports become diagnostics.
- [ ] `duplicate_alias`, `duplicate_trait_instantiation`, and the CLI-owned `duplicate_library_export` checks migrate to call sites of the shared mechanism.
- [ ] Rebound builtin-function spellings keep reporting nothing.

### Consumers (Slices 4–8)

- [ ] HIR declarations carry canonical identity; single-binding imports carry their target's identity. (Identity fact delivered; the id-derivation rework and per-binding import declarations remain.)
- [ ] Body IR callable targets consume the typechecker-minted identity instead of re-deriving one. (Delivered for direct calls; local resolution by identity remains.)
- [ ] Body IR resolves locals by identity so no resolver-resolved reference degrades to an external local.
- [ ] Diagnostics name canonical declaration sites regardless of the referencing binding.
- [ ] LSP definition/references/hover resolve through identity.
- [ ] Codegraph keys records on identity rather than the string triple.

### Recoverable projection (Slice 9)

- [ ] `incan-v1` emitted-name payload with DD-0002 toolchain record and decode fixtures (#1174).

### Cutover conformance

- [ ] Identity matrix rows land in the backend-parity corpus.

## Design decisions

- **Canonical identity covers every ordinary binding form, not just the originally-proposed set:** locals, imports, aliases, re-exports, generic binders, and members, plus `for` loop variables, `with ... as` bindings, and `except ... as` bindings. All are ordinary bindings into a lexical namespace; none earns a separate identity model just because of which statement form introduced it. A decorator's own name reference resolves through the same general mechanism rather than needing special-casing.
- **Collision and ambiguity detection consolidates into one shared namespace/binding-resolution mechanism, not construct-specific diagnostics:** today's fragmented approach (`duplicate_library_export`, `duplicate_rust_module`, `duplicate_trait_instantiation`, `duplicate_alias`, `pub_library_import_name_collision`, `pub_library_module_member_ambiguous`, each implemented independently, and spread across the parser, the frontend, and the build command rather than merely across files) is itself a source of the defects this RFC exists to close — different constructs can disagree about what counts as a collision because each runs its own logic, and a check living in the CLI cannot agree with a frontend check by construction. Every binding kind registers into and is checked against one shared mechanism; existing construct-specific diagnostics migrate to call sites of it, keeping their own user-facing wording where that's genuinely useful, but not their own collision logic. `pub_library_import_name_collision` stays separate, tied to the `pub`/`rust`/`std` namespace-root import syntax (see Related), and the duplicate-`rust`-module check stays in the parser for the reason recorded below.
- **Delivery order is settled, and the assignment fix's scope with it.** #1132 finishes first because it owns the statement-checker lane; #1072 follows and must cover both halves of assignment semantics; the canonical-identity slices follow #1072. The two halves of #1072 are one change, not a sequencing choice: because assignment checking ignores the binding form today, making plain assignment walk outward without also honoring `let` and `mut` would convert every in-block `let` into a reassignment. This RFC records that order as its governing map without claiming ownership of the two issues that precede it.
- **`mut` is a shadowing form too, not only `let`.** The originating issue's phrasing that `let` is the only ordinary mechanism introducing a same-spelling binding over an active one does not match the documented binding model, which gives `mut name = value` the same binding-introducing behavior plus mutability. Implementation and conformance fixtures follow the documented model.
- **Rebinding an unprotected builtin-function spelling is not a collision.** The builtin fallback tier sits beneath the whole scope chain, so a declared or imported binding legitimately wins. The protected `print` and `println` builtins are explicit exceptions: attempts to rebind either name are compile-time errors. The consolidation slice must preserve both parts of that settled, tested behavior.
- **The duplicate-`rust`-module check stays in the parser.** It is raised before any resolver has run and cannot consult a shared binding mechanism without inverting the pipeline. It keeps its own narrow check, and the consolidation decision above is amended to exclude it rather than pretending it can migrate.
- **The duplicate-library-export check moves out of the CLI.** It is a namespace collision decided today in the build command rather than the frontend, which is why it can disagree with frontend checks. Migrating it to the shared mechanism is a layering correction, not only a deduplication.
- **Canonical identity is stable within one compilation, not across source edits.** Every identity anchors to its declaration span, so an edit that moves a declaration changes its identity. Consumers that need cross-edit continuity, such as an editor session, must re-resolve rather than cache an identity across versions.
- **Recoverable projection serves artifact observation, never source semantics.** A compiler phase never parses an emitted name to resolve a source reference. An artifact observer may decode the versioned `incan-v1` payload because that operation answers the distinct question of which compiler-owned identity a completed artifact exposes. A side-car may enrich that decoded answer, but cannot be necessary to establish it.
