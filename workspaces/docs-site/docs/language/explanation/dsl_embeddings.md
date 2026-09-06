# Embedded language fragments

A library you import can add block keywords whose bodies are not Incan. A markup library gives you `html:`, a styling library `css:`, a script-shaped library `pattern:`. Inside those blocks you write something that looks like another language:

```incan
from pub::webkit import render

def page(title: str) -> None:
    html:
        <h1>{title}</h1>
```

This page explains what that actually is, because the honest answer is narrower than it looks and the difference matters when you hit its edges.

## It is not the language it resembles

`<h1>{title}</h1>` is not HTML. Incan has a fixed catalogue of six **submodes** — small grammars owned by the compiler — and a library chooses which one its block claims. `Markup` happens to accept tags, attributes, text, entity references and comments, which covers a useful subset of HTML-shaped syntax and stops there. Namespaces, doctypes and unquoted attribute values are not accepted, and never will be as part of that submode.

The same applies to every other submode. `Style` takes selector lists and a declaration block, but not nested rules or `@media`. `TypePosition` takes qualified names, generics, nullables, arrays and unions, but not bounds or function types.

That boundary is deliberate rather than unfinished. Accepting a real external grammar would mean tracking a specification the compiler does not own, across versions it cannot pin, and would turn every gap into a compatibility bug. A fixed small grammar can be specified exactly and can say no clearly.

**A library that describes its blocks as "HTML support" or "CSS support" is overclaiming.** What it has is a submode, and the accepted constructs are listed in the [vocab authoring guide](../../contributing/how-to/authoring_vocab_crates.md#the-submode-catalog).

## Unrecognized syntax is an error, never a reinterpretation

Anything a submode does not accept is a parse error at the point it appears. It is not silently treated as text, not passed through to a runtime to reject later, and not reinterpreted as ordinary Incan.

This is the property that makes the feature usable. A fragment that compiles has been understood by the compiler, and a fragment that has not been understood does not compile.

## Holes are real Incan

`{title}` is an **expression hole**: an escape back into ordinary Incan, typechecked exactly as it would be anywhere else.

```incan
def page(title: str) -> None:
    html:
        <h1>{title}</h1>
        <p>{subtitle}</p>
```

If `subtitle` is not in scope, that is an ordinary unresolved-name error on that line — not something discovered later by the library's own machinery. A hole whose type does not fit its use is an ordinary type error. The fragment is foreign; what you interpolate into it is not.

## Blocks belong to the library that declared them

`html:` only means anything in a file that imports the library declaring it. Outside such a file, `html` is an ordinary identifier and `<h1>` is not valid syntax anywhere.

Two libraries cannot both claim the same block in the same place. If they try, the compiler rejects the combination as ambiguous rather than picking one, so which library you imported first never changes what your code means.

## What the compiler does not decide

The compiler parses a fragment, typechecks its holes, and hands the library a typed artifact. It assigns **no runtime meaning** to the fragment's own content — tag names, selectors, regex patterns and type shapes mean whatever the owning library decides they mean.

A library that has not yet supplied that step still gives you a fragment that parses and typechecks; only building it to a binary refuses, with a message naming exactly what is missing. That refusal is deliberate: guessing at what a DSL's syntax should do at runtime is precisely the kind of silent behavior this design exists to avoid.

## Formatting

`incan fmt` currently reproduces a fragment's original text exactly. It does not reformat inside a fragment, so whatever layout you write is the layout you keep.

## See also

- [Authoring vocab crates](../../contributing/how-to/authoring_vocab_crates.md) — the library-author side, including the full submode catalogue
- [RFC 081](../../RFCs/081_language_shaped_dsl_embeddings.md) — the specification
