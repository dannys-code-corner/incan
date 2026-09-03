# RFC 122: Composable model construction with fieldmethods and `new`

- **Status:** Draft
- **Created:** 2026-09-03
- **Author(s):** Danny Meijer (@dannymeijer)
- **Related:**
    - RFC 017 (validated newtypes with implicit coercion)
    - RFC 021 (model field metadata and schema-safe aliases)
    - RFC 084 (RHS partial callable presets)
    - RFC 087 (reusable field contracts and structural model composition)
    - RFC 109 (receiver chain combinators)
- **Issue:** [#1295](https://github.com/encero-systems/incan/issues/1295)
- **RFC PR:** —
- **Written against:** v0.6
- **Shipped in:** —

## Summary

This RFC introduces composable model construction through model-only `@fieldmethod` hooks, fluent constructor-partial chains, and a contextual `new` expression that finalizes a construction chain without a trailing call. Every model field receives a synthesized fieldmethod unless the model authors one, models may add typed computed fieldmethods for virtual construction inputs such as `hours`, and every authored fieldmethod declares the stored fields it may bind. Fieldmethods customize pending construction rather than mutate completed model values, and constructor presets remain reusable callable values rather than becoming types or subclasses. The result is a declarative alternative to inheritance and hand-written builders: `Labrador = Dog.paws(4).coat_color("brown")` defines a reusable constructor specialization, while `dog = new Labrador` produces a concrete `Dog` through the model's canonical validation and newtype-conversion path.

## Core model

1. **A model constructor has a field-aware fluent surface:** each accessible model field is also callable on that model's constructor and constructor partials.
2. **Fieldmethods customize pending construction:** `@fieldmethod` is valid only in a model and runs against a model-construction context, not a completed model instance.
3. **Ordinary fieldmethods are synthesized:** when no authored fieldmethod exists for a field, the compiler provides one that binds the supplied value to that field.
4. **Authored fieldmethods declare their write targets:** bare `@fieldmethod` infers the same-named stored field, while `@fieldmethod(minutes)` or `@fieldmethod(days, seconds, nanoseconds)` explicitly names the stored fields the method may bind.
5. **Authored fieldmethods may be field-backed or computed:** a method named for one of its targets replaces that field's synthesized behavior, while a distinct name introduces a typed virtual construction input over its declared targets.
6. **Fluent chains produce constructor partials:** `Dog.paws(4).coat_color("brown")` is a reusable callable construction plan whose result type remains `Dog`.
7. **`new` finalizes one construction:** `new Dog.paws(4)` opens a construction expression, applies its effective fieldmethods in the model-defined construction schedule, invokes the model's canonical constructor exactly once, and returns that constructor's result.
8. **Later calls to the same construction input win:** a later call replaces an earlier preset for the same fieldmethod identity, and superseded fieldmethod bodies do not execute. Stored defaults initialize the context before the surviving fieldmethods run.
9. **Construction preserves type boundaries:** fieldmethod parameters use the declared field type, RFC 017 checked newtype coercion may be inserted at its existing approved sites, and unrelated primitive widening or parsing is not introduced.
10. **Constructor specialization is not inheritance:** a partial derived from `Dog` is a callable that constructs `Dog`; it is not a type, subtype, model declaration, or source of inherited behavior.

## Motivation

Incan models are data-first values with declared fields, keyword-only construction, defaults, validation, schema metadata, and no inheritance. RFC 084 already lets authors publish constructor presets such as `BronzeReader = partial Reader(layer="bronze")`, which covers simple default specialization without subclasses. It does not provide a field-oriented fluent surface, a place to normalize an individual pending field, or a construction expression that can be read as a left-to-right model recipe without ending in an otherwise meaningless `()`.

Static factory methods do not solve the complete problem. A static `TimeDelta.days(4)` can construct one value, but after that call the receiver is a completed value, so `.hours(3)` must either mutate or copy that value and can no longer participate in required-field checking as pending construction. A static method also cannot deliberately share a field's name under an ordinary single member namespace without creating a field/member collision.

Hand-written builder models can express pending construction, but they duplicate every model's fields, types, defaults, validation boundaries, documentation, and editor surface. Inheritance can specialize constructors in some object-oriented languages, but it introduces subtype relationships, method-resolution rules, and fragile behavioral coupling when the desired result is only a reusable configuration recipe.

The desired surface is smaller and more direct:

```incan
Labrador = Dog
    .paws(4)
    .tail_length(20)
    .coat_color("brown")
    .size(DogSize.Medium)

Stewie = new Labrador
Pedro = new Labrador.coat_color("black")
```

`Labrador` is a typed constructor specialization. `Stewie` and `Pedro` are `Dog` values. No subclass, duplicate model, mutable half-object, or trailing materialization call is involved.

## Goals

- Add a model-only `@fieldmethod` method kind for controlling how declared and virtual model inputs participate in construction.
- Synthesize an ordinary fieldmethod for every model field that does not define one explicitly.
- Require authored fieldmethods to declare a compiler-checked stored-field write set, with a same-name shorthand for field-backed methods.
- Allow authored computed fieldmethods to introduce typed virtual construction inputs that update pending model fields.
- Allow model constructors and model-constructor partials to be specialized through fluent fieldmethod chains.
- Allow declaration-safe fieldmethod chains to define top-level constructor partials without pretending that they execute during module initialization.
- Add a contextual `new` expression that finalizes a model construction chain without a trailing `()` or `.build()`.
- Route ordinary model constructor arguments and validated model constructor arguments through the same fieldmethod behavior.
- Preserve RFC 084's overrideable-default behavior and define a deterministic model-owned execution schedule when fieldmethods contain logic.
- Preserve model field visibility, aliases, defaults, validation, newtype coercion, checked API metadata, documentation, and diagnostics through fluent construction.
- Make constructor partials compose without introducing inheritance or nominal subtyping.

## Non-Goals

- Adding inheritance, subclasses, method inheritance, constructor inheritance, or method-resolution ordering.
- Treating a constructor partial such as `Labrador` as a type or permitting it in type position.
- Defining automatic serialization, deserialization, or a stable wire representation for model-constructor partials.
- Adding fieldmethods to classes, traits, enums, or newtypes.
- Calling fieldmethods on completed model instances or turning them into copy-update methods.
- Replacing ordinary instance methods, static methods, computed properties, RFC 109 receiver combinators, or whole-model validation.
- Introducing implicit primitive parsing or widening such as `str -> int` or `int -> float`.
- Changing RFC 017's explicit `from_underlying` recovery surface or its failure behavior for implicit checked coercion.
- Defining frozen constructor bindings in the initial surface; the construction-plan representation must leave room for them without making them part of this RFC.
- Allowing arbitrary top-level execution under the appearance of a constructor chain.
- Extending keyword `new` to class allocation or identity-bearing object construction in this RFC.

## Guide-level explanation

### Ordinary fluent model construction

Every model field is available as a fieldmethod on the model constructor. An authored fieldmethod may also add logic while declaring which stored field it controls:

```incan
model TimeInterval:
    minutes: int = 0

    @fieldmethod(minutes)
    def hours(mod, hours: int):
        mod.minutes += hours * 60

    @fieldmethod(minutes)
    def minutes(mod, minutes: int):
        mod.minutes += minutes

    @fieldmethod(minutes)
    def weeks(mod, weeks: int):
        mod.minutes += weeks * 7 * 24 * 60
```

`hours` and `weeks` are computed construction inputs, while `minutes` is both a stored field and an authored field-backed input. All three declare `minutes` as their write target. The `minutes` declaration replaces its ordinary synthesized setter, so different units can contribute to the same stored representation. Sharing a target is explicit at the model definition and does not make the logical inputs aliases of one another. A concrete interval can therefore be written as one construction expression:

```incan
interval = new TimeInterval
    .hours(2)
    .minutes(30)

assert interval.minutes == 150

fortnight = new TimeInterval.weeks(2)
assert fortnight.minutes == 20_160
```

The chain after `new` does not call methods on an already completed `TimeInterval`. It accumulates pending fieldmethod inputs. At the end of the complete `new` expression, the effective fieldmethods update one pending model state and that state is finalized once.

The same expression may be written on one line when that is clearer:

```incan
interval = new TimeInterval.hours(2).minutes(30)
```

### Reusable constructor specializations

Without `new`, a fieldmethod chain produces a reusable model-constructor partial:

```incan
type Paws = newtype int[ge=0, le=4]
type TailLength = newtype int[ge=0]

enum DogSize:
    Small
    Medium
    Large

model Dog:
    paws: Paws = 4
    tail_length: TailLength
    coat_color: str
    size: DogSize

Labrador = Dog
    .paws(4)
    .tail_length(20)
    .coat_color("brown")
    .size(DogSize.Medium)

Chihuahua = Dog
    .tail_length(10)
    .coat_color("white")
    .size(DogSize.Small)
```

The capitalized names are callable constructor partials, not new model types:

```incan
Stewie: Dog = new Labrador
Pedro: Dog = new Chihuahua.coat_color("black")
```

Ordinary invocation remains available because constructor partials remain callables:

```incan
Stewie = Labrador()
Pedro = Chihuahua(coat_color="black")
```

The `new` spelling is intended for fluent model construction and avoids the empty trailing call. The ordinary callable spelling remains useful when code treats a preset uniformly with other callable values.

### Authored fieldmethods

An authored fieldmethod replaces the synthesized binding behavior for one field:

```incan
model Dog:
    paws: Paws = 4
    tail_length: TailLength
    coat_color: str
    size: DogSize

    @fieldmethod
    def coat_color(mod, value: str):
        normalized = value.strip().lower()

        if normalized == "yellow":
            normalized = "golden"

        mod.coat_color = normalized
```

Bare `@fieldmethod` is shorthand for `@fieldmethod(coat_color)` because the method name resolves to a stored field. `mod` is a construction-context receiver. Assignment to `mod.coat_color` commits a value to the pending `coat_color` field and does not recursively invoke the fieldmethod. The method's construction-context result is implicit, so no `return mod` is written and callers retain the fluent chain:

```incan
GoldenDog = Dog.coat_color("  YELLOW  ")
dog = new GoldenDog.tail_length(18).size(DogSize.Medium)
```

Fieldmethods are not called on completed model instances:

```incan
dog.coat_color             # field read
dog.coat_color("black")    # compile-time error
```

This keeps model values data-first and prevents the construction surface from becoming implicit mutation or copy-update behavior.

### Computed construction inputs

An authored fieldmethod whose name is not a stored field introduces a virtual construction input. This is useful when user-facing construction vocabulary should be normalized into a smaller stored representation:

```incan
model DateTimeInterval:
    years: int = 0
    months: int = 0
    days: int = 0
    seconds: int = 0
    nanoseconds: int = 0

    @fieldmethod(days, seconds, nanoseconds)
    def hours(mod, value: int):
        normalized = normalize_day_time(
            days=mod.days,
            seconds=mod.seconds + value * 3_600,
            nanoseconds=mod.nanoseconds,
        )
        mod.days = normalized.days
        mod.seconds = normalized.seconds
        mod.nanoseconds = normalized.nanoseconds
```

`hours` is visible on the construction surface but is not stored on completed values:

```incan
interval = new DateTimeInterval.hours(30)
assert interval.days == 1
assert interval.seconds == 21_600

interval.hours       # compile-time error: no stored field or instance method
```

The decorator arguments are canonical stored-field names and form the method's declared write set. The compiler rejects a write to any other field, so the effect of a virtual input remains visible at its declaration. Computed fieldmethods may update multiple pending fields. Their purpose is still model construction; they do not add instance methods or hidden storage.

### Direct constructors use the same hooks

An authored fieldmethod is part of the field's construction contract, not special behavior limited to fluent syntax. Direct keyword construction must therefore use it:

```incan
dog = Dog(
    tail_length=20,
    coat_color="  YELLOW  ",
    size=DogSize.Medium,
)

assert dog.coat_color == "golden"
```

Field-backed constructor arguments and virtual inputs use the same hooks:

```incan
interval = DateTimeInterval(days=14, hours=20, minutes=3, seconds=42)
```

Constructor arguments are collected as effective construction inputs rather than executed in caller source order. Model defaults initialize pending stored state first, effective field-backed inputs run in canonical field declaration order, and effective computed fieldmethods then run in their model declaration order. A later explicit input replaces an earlier preset for the same fieldmethod identity. An authored field-backed method decides whether its surviving value replaces, normalizes, or contributes to the initialized default state.

### Last binding wins

Constructor partials behave as overrideable defaults, consistent with RFC 084:

```incan
BrownLabrador = Dog.coat_color("brown").paws(4)
BlackLabrador = BrownLabrador.coat_color("black")
dog = new BlackLabrador
```

Only the final pending input for `coat_color` is applied. The superseded `"brown"` fieldmethod invocation does not execute during construction. Replacing an input changes its effective value without changing the fieldmethod's model-defined execution slot.

Duplicate keywords inside one argument list remain errors:

```incan
Dog(coat_color="brown", coat_color="black")  # compile-time error
```

Last-one-in-wins applies across constructor partials, chained fieldmethod calls, and final call-site overrides that target the same construction-input identity. Stored defaults are initialization state rather than competing construction inputs. Different fieldmethods such as `hours`, `minutes`, and `weeks` all execute when supplied, even though they update the same stored field. Their overlap is intentional because the model author declared the same target on each method. The rule does not legalize duplicate syntax in one argument list.

The construction-input identity is the fieldmethod name, not its declared write targets. For the earlier `TimeInterval`, `hours` and `minutes` therefore both contribute regardless of their order:

```incan
morning = new TimeInterval.hours(2).minutes(30)
afternoon = new TimeInterval.minutes(30).hours(2)

assert morning.minutes == 150
assert afternoon.minutes == 150
```

Repeating the same logical input replaces it instead of accumulating it:

```incan
interval = new TimeInterval.hours(1).hours(2)
assert interval.minutes == 120
```

Only the final `hours` hook executes. Because repetition inside one fluent expression is easy to miss and seldom clearer than deleting the earlier call, the compiler warns at both call sites:

```text
warning: `hours` is called twice in this construction chain
  `hours(1)` is discarded; only `hours(2)` survives
```

This warning does not apply when a later construction layer deliberately overrides a named constructor partial, such as `new DefaultInterval.hours(2)`. The later value occupies the same model-defined execution slot. This distinction lets different units contribute to shared storage while preserving predictable override behavior for presets and call-site customization.

Repeating one fieldmethod never accumulates values. Even if its authored body uses `+=`, `append`, or another update operation, replacement occurs before fieldmethod execution and only the surviving body runs. A collection-oriented construction input should therefore accept the complete collection:

```incan
TaggedItem = Item.tags([urgent, external])
```

It should not rely on repeated calls:

```incan
TaggedItem = Item.tag(urgent).tag(external)  # warning; only `external` survives
```

### Composable inputs and alternatives

Distinct fieldmethods are appropriate when their effects may be combined, including when they intentionally share write targets. `hours`, `minutes`, and `weeks` are separate inputs because a duration may include all three.

Mutually exclusive choices should instead share one construction-input identity whose value is represented by an enum, union, or another type that captures the alternatives. Defining separate fieldmethods for incompatible choices would cause both methods to execute when both are supplied; overlapping write targets do not imply exclusivity or precedence.

### Newtypes remain distinct

A synthesized or authored fieldmethod receives the declared field type:

```incan
type Days = newtype int

model DateTimeInterval:
    days: Days = 0
```

The fluent call may still use the underlying integer because a function argument is already an RFC 017 checked-coercion site:

```incan
interval = new DateTimeInterval.days(-7)
```

Conceptually, the compiler inserts the same checked conversion used by `Days(-7)` and `Days.from_underlying(-7)`. The method's parameter type remains `Days`; `int` does not become identical to `Days`, and the proposal adds no primitive widening. Authors who need recoverable validation and custom diagnostics retain the explicit `Result` form:

```incan
days = Days.from_underlying(external_value)?
interval = new DateTimeInterval.days(days)
```

### Validated models

Keyword `new` must not bypass a model's canonical validation contract. When a model derives `Validate`, finalizing its construction expression has the same result type and validation behavior as `TypeName.new(...)`:

```incan
@derive(Validate)
model PortRange:
    start: int
    end: int

    def validate(self) -> Result[Self, str]:
        if self.start > self.end:
            return Err("start must not exceed end")
        return Ok(self)

def configured_range() -> Result[PortRange, str]:
    return new PortRange.start(7000).end(8000)
```

For an ordinary model, a `new` expression has type `Model`. For a model whose canonical validated constructor has type `(...) -> Result[Model, E]`, the `new` expression has type `Result[Model, E]`. Existing raw-construction restrictions remain in force.

## Reference-level explanation

### Terminology

A **model constructor** is the compiler-known callable surface that constructs a particular model. A **model-constructor partial** is a callable construction plan derived from a model constructor with zero or more pending construction inputs. A **construction context** is the temporary, compiler-owned model state against which fieldmethods run before final model construction. A **write target** is a canonical stored field that an authored fieldmethod declares and must bind. A **field-backed fieldmethod** owns the construction input for a same-named write target. A **computed fieldmethod** introduces a virtual construction input over one or more differently named write targets. A **pending input** is a captured fieldmethod invocation and its source provenance. **Finalization** applies the effective fieldmethods to the construction context and invokes the model's canonical constructor exactly once.

Constructor partials are values with a projected callable surface. They are not nominal types and must not be accepted in type annotations, inheritance positions, trait-adoption positions, or pattern type positions.

### `@fieldmethod` declarations

`@fieldmethod` must be accepted only on a method declared directly inside a model body. Applying it to a class, trait, enum, newtype, extension surface, free function, nested function, or ordinary method must be rejected.

The declaration form is:

```text
FieldMethodDecl      ::= FieldMethodDecorator Newline Visibility? "def" FieldMethodName "(" "mod" "," ValueParam ")" ":" Suite
FieldMethodDecorator ::= "@fieldmethod" | "@fieldmethod" "(" FieldTargetList ")"
FieldTargetList      ::= FieldName ("," FieldName)* ","?
ValueParam           ::= Ident ":" TypeExpr
```

Each decorator argument must resolve directly to a canonical field declared or structurally composed into the owning model. Aliases and string field names are not accepted in the decorator, duplicate targets are errors, and a missing target is an error. The target list is part of the model's checked construction API.

Bare `@fieldmethod` is permitted only when the method name resolves to a canonical stored field and infers that field as its single write target. The explicit spelling `@fieldmethod(field_name)` is equivalent. When the method name is a stored field, its target list must include that field; the declaration is field-backed and replaces that field's synthesized fieldmethod. Otherwise, an explicit non-empty target list is required, and the declaration is computed and introduces a virtual construction input with the method's name. A computed fieldmethod name must not collide with another constructor input, field alias, static member, ordinary method, computed property, or reserved constructor member.

A field-backed fieldmethod's value parameter type must be exactly the declared model field type after ordinary type-alias normalization. A broader primitive, narrower unrelated newtype, or merely assignable widening must be rejected. A computed fieldmethod may declare another concrete value type because it introduces its own construction input. RFC 017 checked conversion may adapt an underlying value at a call site where that RFC already permits it, but it does not make the supplied and declared types identical.

`mod` is a reserved receiver spelling for construction contexts. It must not be annotated with `self`, `cls`, `mut`, or a user-written construction-context type. The receiver permits construction-context field assignment by definition; it is not a mutable borrow of a completed model.

A fieldmethod must not declare positional parameters beyond its one value parameter, keyword parameters, variadic parameters, or a user-chosen return type. Its successful result is the same construction context and is returned implicitly. `return mod`, returning another value, or returning a replacement model must be rejected.

Every normally completing control-flow path through a fieldmethod must bind every declared write target before completion. It must not assign, mutate, or update a stored field outside that target set. These rules make the decorator an exact construction effect rather than documentation, preserve required-field reasoning, and prevent a supplied construction input from silently leaving one of its declared outputs absent. A fieldmethod may read other definitely bound fields and perform deterministic synchronous computation before committing its bindings.

Fieldmethods must not be async, generators, context managers, trait requirements, overloaded methods, static methods, class methods, computed properties, or decorator targets in this RFC.

### Synthesized fieldmethods

For every model field without a same-name authored fieldmethod, the compiler must synthesize behavior equivalent to:

```incan
@fieldmethod(field_name)
def field_name(mod, value: FieldType):
    mod.field_name = value
```

The synthesized fieldmethod's visibility must follow the field's constructor visibility. A caller that cannot provide a private field to the ordinary constructor must not bind it through a synthesized or authored fieldmethod.

A synthesized or authored field-backed fieldmethod and its field intentionally share canonical field identity and spelling. This is not a collision. Computed fieldmethods and ordinary instance members, static methods, computed properties, and unrelated declarations do not receive this exception and remain subject to existing member collision rules.

For a code-spellable model field alias, fluent construction may use either the canonical field name or the alias under the same access rules as keyword construction. Both spellings must resolve to the same canonical fieldmethod identity. An authored fieldmethod is declared under the canonical field name only.

### Receiver-directed member resolution

Fieldmethod syntax must resolve according to the receiver category:

- `ModelName.fieldmethod(value)` resolves on the model constructor surface and returns a model-constructor partial.
- `ConstructorPartial.fieldmethod(value)` resolves on the partial's target model surface and returns another model-constructor partial.
- Inside a `new` construction expression, `.fieldmethod(value)` extends the pending construction plan.
- `model_value.field` remains an ordinary field read.
- `model_value.fieldmethod(value)` must not resolve to a fieldmethod.

The compiler must resolve a fieldmethod through canonical model and fieldmethod identity and retain its canonical write-target identities; a field-backed fieldmethod must additionally retain its canonical same-named field identity. Lowering and emission must not guess from a member's spelling or rediscover targets by inspecting the body.

### Fluent constructor-partial expressions

The conceptual grammar is:

```text
ModelPartialExpr  ::= ModelPartialBase FieldMethodCall+
ModelPartialBase  ::= ModelName | ModelPartialName | ExplicitPartialExpr
FieldMethodCall   ::= LineContinuation? "." FieldMethodName "(" ArgumentExpr ")"
```

Applying a fieldmethod to a model constructor or model-constructor partial must return another constructor partial for the same target model. It must capture the supplied argument according to RFC 084's top-level or local preset-expression rules.

At module top level, an assignment whose complete right-hand side is a statically resolvable model-constructor fieldmethod chain is a declaration-safe constructor-partial declaration. It must not execute authored fieldmethod bodies or construct a model during module initialization. Every supplied top-level argument must satisfy RFC 084's declaration-safe preset-expression rules.

Inside executable code, fieldmethod argument expressions must be evaluated exactly once, left to right, when the partial expression is evaluated, matching RFC 084's local partial capture behavior. Authored fieldmethod bodies execute only when the partial is finalized or ordinarily invoked.

Explicit RFC 084 syntax remains valid and interoperable:

```incan
Labrador = partial Dog(paws=4, coat_color="brown")
BlackLabrador = Labrador.coat_color("black")
```

The projected callable must retain every stored field not effectively preset as a required or defaulted keyword parameter according to the model constructor surface. Field-backed and computed inputs supplied by the partial remain optional keyword overrides, consistent with RFC 084. An unsupplied computed fieldmethod is optional because it does not represent required stored state.

### Binding merge and evaluation order

Each construction plan must retain at most one effective value for each canonical construction-input identity together with its capture and replacement provenance. Input collection is declarative: caller source order must not determine fieldmethod execution order.

When a later input targets the same canonical field-backed or computed fieldmethod identity as an earlier preset, the later input must replace the earlier effective value without moving that fieldmethod's model-defined execution slot. The superseded fieldmethod body must not execute. Its already-captured local argument value remains subject to ordinary evaluation and destruction rules; replacing an input does not retroactively skip evaluation that already occurred when a local partial captured it.

When the same fieldmethod identity appears more than once within one syntactic fluent chain, the compiler must emit a warning identifying the earlier and surviving calls. The warning must state that the earlier binding is replaced. It must not claim that the earlier argument expression was skipped, because executable local chains evaluate captured arguments left to right even when a later call supersedes the binding. Overriding an input inherited from a separately named or separately supplied constructor partial must not produce this warning.

Replacement must occur before fieldmethod execution, so an effective fieldmethod body executes at most once per finalized construction. Repeated calls to one fieldmethod must not accumulate, merge, append, or otherwise combine their arguments, regardless of the operations in the authored body.

At finalization, model defaults must initialize pending stored fields first. Effective field-backed fieldmethods must then execute in canonical stored-field declaration order. After every effective field-backed input has executed, effective computed fieldmethods must execute in their declaration order within the model. Distinct fieldmethods must all execute even when their declared write targets overlap; the compiler must not infer mutual exclusion, replacement, or precedence merely from target overlap. Assignment by one fieldmethod to a field that is subsequently assigned by another effective fieldmethod follows ordinary scheduled behavior: a direct assignment replaces the earlier value, while an update such as `+=` observes and updates the earlier value.

Argument expressions must still be evaluated exactly once in their ordinary left-to-right source order when an executable local partial or construction expression captures them. Capture order and fieldmethod execution order are separate: captured values are placed into the construction plan, then only effective fieldmethod bodies run according to the model-defined schedule at finalization.

Duplicate keywords within one constructor, partial argument list, or fieldmethod call must remain compile-time errors. The override rule applies between construction layers and fluent calls, not within a single argument list.

### Construction-context access

Assignment to `mod.field` inside a fieldmethod is a primitive construction-context binding operation. It must not recursively dispatch to that field's authored or synthesized fieldmethod. A fieldmethod may bind only its decorator-declared write targets. It may read any field on its owning model subject to the construction-state rules below; this is what lets a computed input normalize into multiple stored fields without granting an undeclared whole-model mutation surface.

A field with a declared default is bound before effective fieldmethods execute and may be read through its ordinary field type. A field-backed fieldmethod may also read fields bound by earlier field-backed slots. A computed fieldmethod runs after all effective field-backed inputs and may read their bound fields plus targets unconditionally bound by earlier computed slots. The exact source spelling for safely inspecting a potentially unbound field is left as an unresolved design question; an unguarded read that is not definitely bound at that point in the model-defined schedule must be rejected rather than fabricate a value.

Construction-context state must never escape a fieldmethod, be stored in a model field, be returned as a first-class value, be passed to an unconstrained callable, or remain observable after finalization.

### `new` construction expressions

`new` is a contextual expression introducer when followed by a model constructor or model-constructor partial. It is not a universal allocation operator in this RFC.

The conceptual grammar is:

```text
NewModelExpr ::= "new" ModelPartialBase FieldMethodCall*
```

`new ModelName` must open a construction plan seeded with the model defaults. `new PartialName` must open a construction plan seeded with the partial's effective bindings. A `new` target must be a bare model constructor or model-constructor partial followed only by fieldmethod calls; call arguments immediately after the target are not permitted.

A newline followed by an indented leading `.` must continue the same `new` or model-partial expression through Incan's existing fluent-chain continuation grammar. The formatter must use four-space indentation for continued fieldmethod chains and must not add an empty trailing call.

The full syntactic expression boundary finalizes a `new` expression. Finalization must verify that every required accessible field has an effective binding, apply checked field conversions and effective authored fieldmethods, invoke the target model's canonical constructor exactly once, and return exactly that constructor's result type.

An ordinary model's canonical constructor returns the model value. A model deriving `Validate` retains its generated `TypeName.new(...) -> Result[TypeName, E]` canonical validated constructor; the contextual `new` expression must therefore return `Result[TypeName, E]` and must not expose or use forbidden raw construction.

Keyword `new` and the existing member name `new` occupy different syntactic positions. `new Model.field(value)` begins a construction expression; `Model.new(field=value)` remains an ordinary associated validated-constructor call.

Callable construction remains a separate spelling. `Model(field=value)` and `PartialName(field=value)` invoke callables; `new Model.field(value)` and `new PartialName.field(value)` finalize fluent construction plans. Mixed syntax such as `new Model(field=value).field(other)` must be rejected.

Using `new` with a class, newtype, enum, arbitrary function partial, instance, or non-constructor callable must be rejected under this RFC.

### Direct constructor integration

Ordinary model construction through `ModelName(...)`, validated construction through `ModelName.new(...)`, and invocation of an RFC 084 model-constructor partial must route each effective construction input through the same authored or synthesized fieldmethod contract used by fluent construction.

This requirement applies to direct keyword arguments, effective partial presets, field-backed inputs, and computed inputs. A computed fieldmethod introduces an optional constructor keyword of the same name, so `DateTimeInterval(hours=12)` and `new DateTimeInterval.hours(12)` use the same `hours` fieldmethod. The constructor keyword must have the computed fieldmethod's declared input type and no implicit default value beyond absence. This prevents a model author from defining normalization that fluent syntax enforces while ordinary construction bypasses it.

Stored defaults initialize the construction context directly before authored fieldmethods run; they are not fieldmethod invocations and therefore do not execute authored logic merely because the caller omitted a field. An explicit value for a defaulted field does invoke its field-backed fieldmethod. This distinction keeps a declared default stable while ensuring every caller-supplied value uses the authored construction contract.

Adding an authored fieldmethod may therefore change how future constructions of that model bind that field. This is an intentional source-level API change and must be reflected in checked API metadata and compatibility tooling.

### Required fields and static checking

The compiler must reject a `new` expression when its statically known construction plan leaves a required field unbound. A fieldmethod call guarantees every declared write target is bound because every normally completing path must commit each target. A computed fieldmethod may therefore satisfy required fields through its checked target set.

Runtime construction must not silently synthesize missing required values. If the compiler cannot prove that every declared target is bound on every normally completing path, the fieldmethod declaration itself must be rejected rather than degrading required-field checking at call sites.

Ordinary constructor-partial expressions may remain incomplete because their purpose is to produce a callable requiring the remaining fields. Tooling must display the projected required and optional keyword surface.

### Validation and newtype conversion

Field-backed fieldmethod call arguments must typecheck against the exact declared field type. Computed fieldmethod call arguments must typecheck against their explicitly declared input type. The compiler may insert RFC 017 validated-newtype coercion when the supplied expression has an approved underlying type and the call is an approved coercion site. It must not treat primitive widening, parsing, unrelated conversion traits, or arbitrary constructors as implicit fieldmethod coercion.

Checked newtype conversion must occur before the authored fieldmethod body observes the value. If implicit conversion fails, RFC 017's ordinary failure behavior applies. Authors requiring recoverable boundary diagnostics should perform explicit `from_underlying` conversion before supplying the typed result to a fieldmethod.

Whole-model validation must run only after all effective fieldmethods have completed and the concrete model value has been assembled. Fieldmethods must not bypass, replace, or suppress model validation.

### Visibility, aliases, and composed fields

Field-backed fieldmethods must preserve the visibility of their corresponding fields. A computed fieldmethod uses its declared method visibility and may deliberately expose a public logical input whose declared targets are private stored fields; the model owns that construction boundary and callers never receive direct access to those fields. Public constructor partials must not expose private fieldmethod surfaces or capture private preset values in ways RFC 084 forbids.

Fields introduced by RFC 087 structural model composition receive synthesized fieldmethods on the target model like locally declared fields. Authored fieldmethods, including computed fieldmethods, are not inherited from the spread source. If the target model wants authored construction behavior for a composed field, it must declare that behavior on the target model itself.

Aliases must continue to preserve canonical field identity, wire identity, diagnostics provenance, and privacy. Fluent aliases are code spellings only; they do not introduce additional pending fields.

### Diagnostics

The compiler must provide targeted diagnostics for at least:

- `@fieldmethod` used outside a model;
- bare `@fieldmethod` used on a method whose name is not a canonical stored field;
- missing, duplicate, aliased, or unknown decorator write target;
- write to a stored field outside the fieldmethod's declared target set;
- control flow that may complete without binding every declared write target;
- field-backed fieldmethod value parameter differs from the declared field type;
- computed fieldmethod name collides with a field alias or existing member;
- computed fieldmethod lacks an explicit input type or explicit write-target list;
- malformed construction-context receiver or unsupported additional parameters;
- explicit return value;
- fieldmethod access through a completed model instance;
- inaccessible private fieldmethod surface;
- top-level fluent preset contains an expression that is not declaration-safe;
- `new` target is not a model constructor or model-constructor partial;
- `new` construction leaves required fields unbound;
- a model-constructor partial is used as a nominal type;
- unsupported construction-context escape or potentially unbound field read;
- collision between an authored fieldmethod and an ordinary member that cannot be resolved by receiver category;
- implicit conversion would require primitive widening, parsing, or a non-RFC-017 conversion.

The compiler must additionally warn when one syntactic fluent chain supplies the same canonical fieldmethod identity more than once. The warning should label both calls and identify which binding survives.

Diagnostics should identify the target model, construction-input identity, construction layer that supplied or replaced the input, and canonical field or fieldmethod declaration when authored behavior is involved.

## Design details

### Why fieldmethods are model-only

Models have a compiler-known data shape, keyword constructor surface, defaults, schema metadata, and no inheritance. Those properties make their pending field set statically projectable. Classes may also store fields, but they are behavior- and identity-oriented and may require initialization protocols that are not equivalent to declarative field binding. Restricting fieldmethods to models is therefore based on model construction semantics, not on the inaccurate claim that classes have no stored fields.

### Why the field and fieldmethod may share a name

The two surfaces have different receivers. `dog.paws` reads a field from a completed `Dog`; `Dog.paws(4)` binds the canonical `paws` field on a `Dog` constructor plan. The compiler already knows whether the receiver is a model value, model constructor, or constructor partial, so it can resolve the meaning once and preserve canonical identity through lowering. Ordinary user-defined members do not gain a general permission to collide with fields.

### Why fieldmethods declare write targets

The fieldmethod name identifies a logical construction input; its decorator arguments identify the stored representation that input is allowed and required to produce. Those identities are deliberately separate. In `@fieldmethod(minutes) def hours(...)`, a later `hours` input replaces an earlier `hours` input, while a distinct `minutes` input still executes even though both affect the same stored field.

Requiring an exact target set keeps construction behavior inspectable without inferring semantics from arbitrary method bodies. The compiler can reject accidental writes, prove which required fields become bound, expose useful API and editor metadata, and later enforce frozen bindings against canonical field identities. Canonical identifiers rather than strings preserve rename safety and avoid a Pydantic-style runtime field-name lookup surface.

### Why computed fieldmethods exist

Not every useful construction input should become stored state. A normalized interval may store days, seconds, and nanoseconds while accepting hours, minutes, milliseconds, and microseconds during construction. Requiring every fluent name to match a stored field would either make the motivating interface impossible or pollute the model's stable data shape with redundant units. Computed fieldmethods keep those inputs typed and model-owned while making their lack of instance storage and their stored-field effects explicit.

A computed fieldmethod remains narrower than a general builder method: it accepts one typed construction input, returns the construction context implicitly, cannot be called on completed values, and must bind its complete declared target set on every normally completing path.

### Why `mod` is not `self` or `cls`

`self` denotes an existing value receiver, and `cls` denotes a type-oriented receiver. A fieldmethod has neither: it operates on temporary construction state whose lifetime ends at finalization. A distinct `mod` receiver makes that boundary visible and prevents an incomplete model from masquerading as a valid instance.

### Why finalization is implicit only under `new`

A fluent chain without `new` is valuable precisely because it remains reusable. Automatically finalizing every complete-looking chain would make meaning depend on whether all required fields happened to be bound and would prevent stable constructor specializations. `new` states the author's intent to produce one concrete result, so the expression boundary can finalize without a trailing `()`.

### Why `new` is contextual

Many languages use `new` to mark construction, although they commonly construct before subsequent method calls. Here `new` marks the whole model-construction expression and finalizes it at the expression boundary. It does not promise heap allocation or object identity. Keeping it contextual also avoids reserving the spelling where it is unrelated to model construction and allows existing `TypeName.new(...)` validated-constructor members to coexist.

Incan already parses an indented leading-dot chain as continuation of its receiver expression, so multiline `new` expressions reuse an established layout rule rather than introducing a colon-delimited block or a new continuation form.

### Why `new` does not accept constructor call arguments

Allowing `new Dog(coat_color="brown").paws(4)` would combine callable constructor syntax with fluent construction syntax inside one expression. It would add a second spelling for inputs under `new` and blur whether the parenthesized call constructs immediately or merely seeds the pending plan. This RFC keeps the forms separate: use `Dog(coat_color="brown")` or `Labrador(coat_color="black")` for callable construction, and use `new Dog.coat_color("brown")` or `new Labrador.coat_color("black")` for fluent construction.

### Why later bindings replace earlier fieldmethod execution

RFC 084 treats presets as overrideable defaults. If an overridden fieldmethod still executed, normalization, assertions, or other construction logic for a value that does not survive could affect the final model. Replacing only the pending value while retaining the method's stable execution slot makes the semantic plan match the visible last-one-in-wins configuration story without turning caller order into execution order.

### Why execution order belongs to the model

Constructor partials describe effective named inputs, not an imperative operation log. If caller source order controlled fieldmethod execution, ordinary keyword order could change meaning and a reusable computed preset could fail merely because a required field is supplied only by its eventual caller. The model-defined schedule first establishes effective stored-field inputs, then applies computed construction logic in authored declaration order. Callers can therefore reorder fluent or keyword inputs without changing the constructed value.

Distinct fieldmethods may intentionally share targets. `hours`, `minutes`, and `weeks` can all contribute to one stored `minutes` field because the author explicitly declared that target on every method. Their execution order is the order of their declarations in the model, not the order chosen by each caller.

### Determinism and effects

Fieldmethods define repeatable model construction semantics and should be deterministic, synchronous, and free of externally observable effects. They may normalize, clamp, or derive values through existing deterministic language mechanisms. Input rejection belongs to checked newtypes before the fieldmethod runs or to whole-model validation after all fieldmethods complete; a fieldmethod does not introduce a third independent error channel. I/O, clock access, randomness, global mutation, and asynchronous work should remain outside model field binding. The exact compiler enforcement mechanism is non-normative, but the contract should match the predictability expected from RFC 017 checked construction and data-first models.

### Comparison with Python and Pydantic

Python's [`dataclasses`](https://docs.python.org/3/library/dataclasses.html) generate an initializer from annotated fields. `InitVar` and `__post_init__` can accept construction-only inputs and derive stored fields, while [`functools.partial`](https://docs.python.org/3/library/functools.html#functools.partial) can publish a callable with overrideable preset keywords. These mechanisms can reproduce pieces of the proposed behavior, but they remain separate runtime conventions: the partial does not acquire a compiler-known field surface, `__post_init__` receives one completed initializer call rather than a typed fluent plan, and specialization is commonly expressed through wrappers or inheritance.

[Pydantic field and model validators](https://docs.pydantic.dev/latest/concepts/validators/) can validate or transform input and express cross-field checks. The decorator form uses string field names and class methods, field validators observe already validated fields in declaration order, and defaults do not run validators unless configured. Pydantic can also derive new runtime model types with [`create_model`](https://docs.pydantic.dev/latest/examples/dynamic_models/), including inheritance of validators and computed fields.

Fieldmethods are neither validators nor dynamic model types. Their role is to build one statically known pending model value: decorator targets are canonical compiler-resolved field identities, constructor partials remain callables returning the original model type, fieldmethods run before the model's canonical whole-model validation, and no inheritance or metaclass protocol is introduced.

### Interaction with RFC 084

RFC 084 remains the general callable-preset mechanism. This RFC adds a field-aware projection for model constructors and their partials. Explicit `partial Model(field=value)` and fluent `Model.field(value)` must converge on the same construction-plan metadata, projected callable signature, override rules, and final constructor behavior.

General function, class, newtype, and method partials do not gain fieldmethods or contextual `new`. This RFC intentionally specializes only the model-constructor target kind.

### Constructor partials are code values

A model-constructor partial is a callable construction plan and may capture local values and authored construction behavior. This RFC does not give such plans a stable data schema or wire identity and does not make them automatically serializable. Applications that persist user-defined templates should represent those templates as ordinary models and reconstruct the appropriate construction plan or concrete value when executing them.

### Interaction with RFC 109

RFC 109's `tap` and `then` operate on completed receiver values. Fieldmethods operate on pending model construction. A fieldmethod chain must not resolve through general receiver combinators, and a completed model must not acquire its fieldmethod construction surface through `tap` or `then`.

### Compatibility and migration

The syntax is additive for models that do not declare members colliding with the new contextual forms. Every existing model gains synthesized constructor fieldmethods, but completed instance member resolution remains unchanged.

An authored `@fieldmethod` changes all future construction paths for that field, including direct constructors and partial invocation. Adding, removing, changing, or reordering a fieldmethod in a way that changes its model-defined execution slot is therefore an API-significant change and should be visible to compatibility tooling.

Introducing contextual `new` may affect code that currently uses `new` as an identifier immediately before a model constructor expression. The lexer and parser must preserve ordinary identifier use outside the precise contextual form and issue migration diagnostics for genuinely ambiguous cases.

## Alternatives considered

1. **Static factory methods named after fields.** A static factory can create an initial value but cannot preserve pending required-field state through a chain. It also overloads ordinary associated behavior for what is fundamentally field binding.
2. **Instance methods that copy or mutate a completed model.** This constructs too early, makes intermediate invalid states possible or forces defaults for required fields, and confuses construction with value mutation.
3. **RFC 084 partials only.** `partial Dog(paws=4)` remains sufficient for simple presets, but it does not provide authored per-field construction behavior or the intended readable fluent surface.
4. **Require a final `()` or `.build()`.** This is mechanically explicit but visually redundant when the expression already begins with an explicit construction marker. `new` provides the finalization signal at the beginning, where readers establish the expression's intent.
5. **Object-initializer braces or an indented colon block.** `new Dog { paws=4 }` or `new Dog: ...` would delimit construction clearly, but they create a second field-assignment grammar and do not naturally reuse callable constructor partials.
6. **Hand-written builder models.** Builders remain possible for protocols more complex than field binding, but requiring one for ordinary model specialization duplicates type and validation surfaces.
7. **Inheritance.** Subclasses can carry constructor defaults in some languages, but they also create nominal subtyping and inherited behavior. Constructor partials solve specialization without importing that hierarchy.
8. **Return a transformed value from each fieldmethod.** A user-written return type would let one call leave the target model's construction surface and would weaken static required-field tracking. The construction-context result is therefore implicit and fixed.
9. **Execute every overwritten fieldmethod in chain order.** This preserves a literal operation log but lets superseded defaults affect the result through hidden behavior. Effective-binding replacement better matches RFC 084's override semantics.
10. **Infer write targets from the method body.** Body inspection can detect assignments in a local implementation, but it makes the public construction contract implicit, weakens checked API metadata, and creates brittle behavior across helper calls and separate compilation. Decorator targets keep the permitted and guaranteed field effects explicit.
11. **Allow constructor arguments inside `new`.** `new Dog(coat_color="brown").paws(4)` is superficially convenient, but mixes callable invocation and fluent construction, duplicates the fieldmethod spelling available under `new`, and weakens the expression's single construction grammar.

## Drawbacks

- `new` introduces a new contextual expression and requires the existing fluent newline-continuation rule to work in new expression and top-level preset positions.
- The same spelling denotes a field on completed values and a fieldmethod on constructor receivers, increasing the importance of precise receiver-directed diagnostics and tooling.
- Model declaration order becomes semantically significant for fieldmethods that read or bind overlapping fields, so moving a field or computed fieldmethod may be an API-significant behavioral change.
- Fluent chains look method-like even though they collect declarative inputs and do not promise caller-ordered fieldmethod execution.
- Omitting the explicit `partial` marker for top-level fluent model presets creates a narrow new exception to the declaration-only module surface.
- Routing every construction path through authored fieldmethods makes those hooks powerful and API-significant; apparently local normalization changes can affect all callers.
- Keeping captured local argument evaluation separate from eventual fieldmethod execution requires the construction plan to preserve both values and source provenance.
- The word `new` can imply allocation or identity in other languages even though Incan models are values and this RFC promises neither.
- Static completeness becomes more complex if fieldmethods may conditionally inspect or bind other pending fields.

## Implementation architecture

This section is non-normative. A coherent implementation should represent fluent model construction, RFC 084 model-constructor partials, direct constructor arguments, and `new` expressions as one typed construction-plan abstraction. The plan should preserve the target model's canonical identity, captured argument values, effective fieldmethod inputs, capture provenance, replacement provenance, model-defined execution slots, fieldmethod and field identities, projected callable signature, and canonical finalizer.

Fieldmethod resolution should occur in the compiler frontend against canonical model and fieldmethod identity, with canonical field identity attached to field-backed methods. Lowering should consume resolved construction-plan operations rather than rediscovering fieldmethods from source names. The final backend representation may use a generated wrapper, builder-like temporary, or direct constructor expansion, provided it evaluates captured expressions exactly once, invokes only effective fieldmethods, constructs the model exactly once, and preserves the canonical validation result.

Checked API metadata should expose constructor partial provenance, authored versus synthesized fieldmethods, canonical write-target sets, projected required/defaulted fields, and whether the target uses ordinary or validated finalization. Tooling should consume the same metadata for completion, hover, signature help, and diagnostics.

## Layers affected

- **Lexer / Parser / AST**: contextual `new`, `@fieldmethod`, construction-context receivers, and model-constructor chains that reuse the existing indented leading-dot continuation surface.
- **Formatter**: stable multiline formatting for model partials and `new` construction expressions through the existing fluent-chain layout, without a trailing call.
- **Typechecker / Symbol resolution**: receiver-directed fieldmethod identity, canonical write-target sets, field-backed and computed construction inputs, exact field-type contracts, required-field projection, definite assignment, visibility, aliases, partial typing, override ordering, and RFC 017 coercion integration.
- **IR Lowering / Emission**: one declarative construction plan, superseded-binding elimination, exactly-once argument evaluation, model-defined fieldmethod scheduling, and canonical constructor finalization.
- **Validation / Derives**: preservation of `TypeName.new(...) -> Result[...]` for validated models and whole-model validation after effective field binding.
- **Checked API / Compatibility metadata**: authored and synthesized fieldmethod surfaces, canonical write-target sets, constructor-partial signatures, construction behavior provenance, and API-diff visibility.
- **LSP / Tooling**: fieldmethod completion on constructors and partials, field reads on instances, signature help, hover distinctions, rename by canonical field identity, and targeted diagnostics.
- **Documentation**: model construction, callable presets, validated models, newtypes, formatting, and migration guidance.

## Unresolved questions

- What typed source surface lets a fieldmethod inspect a potentially unbound field while distinguishing bound, defaulted, and missing state?
- What static or checked-effect mechanism should enforce the deterministic, synchronous fieldmethod contract until Incan has a general effect system?
- Should tooling expose a source-spellable type for model-constructor partials, or should they remain ordinary projected callables with richer metadata?
- If frozen constructor bindings are introduced later, what syntax marks a binding as non-overridable and at which construction layers can freezing be applied?

<!-- Rename this section to "Design Decisions" once all questions have been resolved.
     An RFC cannot move from Draft to Planned until no unresolved questions remain. -->
