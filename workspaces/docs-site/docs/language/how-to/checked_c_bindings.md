# Work with checked C bindings

Use this guide when you already understand the first checked C binding tutorial and need to model a small C header,
keep the raw boundary private, or diagnose a rejected declaration.

## Keep the binding local and expose a façade

Do not publish a raw binding merely because it has a useful native name. Leave the binding unexported and export only
ordinary Incan functions or models that give the C API application meaning:

```incan
from std.interop import c

binding LibC:
    header = "stdlib.h"
    link = c.system_library("c")

    symbol absolute(value: c.i32) -> c.i32:
        native = "abs"

pub def magnitude(value: int) -> int:
    unsafe:
        return LibC.absolute(value)
```

The `binding` vocabulary desugars to a `@c.binding(...)` declaration class extending `BindingDeclaration`. That is an
implementation detail with a useful consequence: normal module visibility rules still apply, and no special compiler
rule turns every native name into public API.

## Choose exact C types

Use the C namespace in the raw declaration, even when a type looks similar to Incan `int`:

| C declaration need | Binding type |
| --- | --- |
| Exact signed or unsigned width | `c.i8` through `c.i64`, or `c.u8` through `c.u64` |
| C `int` | `c.c_int` |
| C `char` | `c.c_char` |
| Target-sized byte count | `c.Size` |
| Required immutable or mutable pointer | `c.ConstPtr[T]` or `c.MutPtr[T]` |
| Nullable pointer | `Option[c.ConstPtr[T]]` or `Option[c.MutPtr[T]]` |

The executable subset currently admits scalar free functions only. Pointer and structure declarations are still useful:
the compiler verifies their declared shape, but rejects a call that would require unimplemented ownership or view rules.

## Declare constants and a plain layout

Use a binding enum for a C macro or named constant. Every variant uses the same explicit C scalar carrier:

```incan
binding Fixture:
    header = "fixture.h"
    link = c.system_library("fixture")

    enum Status:
        OK: c.i32 = FIXTURE_OK
        Retry: c.i32 = FIXTURE_RETRY
```

Use a binding structure only for a named plain C layout whose native spelling and fields you can state exactly:

```incan
binding Fixture:
    header = "fixture.h"
    link = c.system_library("fixture")

    struct Pair:
        native = "fixture_pair"
        left: c.i32 = left
        right: c.i32 = right
```

Clang checks each requested field offset, size, and alignment for the selected host target. It does not infer omitted
fields or discover a structure from the header. By-value structure and pointer calls are deliberately unavailable in
this slice, so do not use a structure declaration as an assertion that you can already pass it across the boundary.

## Interpret common failures

| Failure | What to check |
| --- | --- |
| `@c.binding requires from std.interop import c` | Import `c` in the declaring module. An alias is allowed; a global activation is not. |
| C symbol has an unsupported parameter or return type | Use one of the current exact scalar types, or wait for the pointer/resource slice instead of weakening the source type. |
| Clang rejects the signature or layout | Compare the header's exact spelling, calling shape, field order, and scalar category with the binding. Do not change the declaration to make generated Rust compile. |
| Native enum carrier mismatch | Keep one declared `c.*` carrier for all variants and verify what the header exposes after macro expansion. |
| Missing system library at final link | `c.system_library("name")` records a logical system capability; this slice does not download, vendor, or lock a library for you. |

## Decide whether C is the right boundary

Choose this surface when the library's supported boundary is a compact C ABI and the part you need fits the verified
scalar subset. Prefer [Rust interop](rust_interop.md) when a maintained Rust crate already offers the safe, resource,
callback, or asynchronous API you need. A C ABI may still be the right eventual boundary for a library implemented in
another language; the implementation language is not the deciding factor.

If the header depends on callbacks, variadics, unions, bitfields, macros that cannot be represented as constants, or
nontrivial lifetime rules, do not fake a scalar declaration. A checked C or C++ shim is the intended later adapter;
it is not available in this first release slice.

See [how checked C interop is structured](../explanation/checked_c_interop.md) for the source-of-truth and toolchain
boundary, and the [`std.interop` reference](../reference/stdlib/interop.md) for precise accepted syntax.
