# `std.interop`: checked C bindings

`std.interop` activates the first checked C binding vocabulary. This page is the exact contract: it lists accepted
declaration forms, current execution limits, and verification behavior. Start with the [tutorial](../../tutorials/checked_c_binding.md),
use the [how-to guide](../../how-to/checked_c_bindings.md) for modelling and diagnostics, and read the
[architecture explanation](../../explanation/checked_c_interop.md) before choosing C over Rust interop.

The surface lets a module declare a small, explicit C ABI contract and call its supported scalar free functions without
writing a Rust wrapper first. The compiler verifies declared signatures, enum carriers, and listed plain-structure
layouts with Clang before generating Rust.

This is a deliberately narrow foundation. It is useful for direct scalar C functions and ABI verification; resource ownership, output positions, bundled artifacts, shims, and platform packaging have separate RFC 116 slices.

## Activate the vocabulary

Import the C namespace explicitly. The import activates `binding` only in that module; it does not make C syntax a global language keyword.

```incan
from std.interop import c
```

`binding` is vocabulary surface. It lowers to an ordinary private class decorated with `@c.binding(...)` and extending `BindingDeclaration`. That keeps the ABI declaration inspectable as ordinary language data while making the source read like the contract it describes.

## Declare a binding

Each binding supplies one explicit header and logical system-library link name. `symbol`, `enum`, and `struct` bodies are declarations, not executable method bodies.

```incan
from std.interop import c

binding LibC:
    header = "stdlib.h"
    link = c.system_library("c")

    symbol absolute(value: c.i32) -> c.i32:
        native = "abs"

    enum Status:
        OK: c.i32 = EXIT_SUCCESS

def absolute(value: int) -> int:
    unsafe:
        return LibC.absolute(value)

def is_success(status: int) -> bool:
    return status == LibC.Status.OK
```

The raw call remains inside `unsafe:`. A public façade can live in the same module and call the private binding; the façade is where an API gives native failure codes, argument validity, and domain meanings their ordinary Incan shape.

The compiler emits the private `extern "C"` declaration from the checked binding descriptor. It does not rediscover the function signature from generated Rust.

## Checked declaration facts

The foundation accepts these declaration forms:

- Exact C scalar spellings: `c.i8`, `c.u8`, `c.i16`, `c.u16`, `c.i32`, `c.u32`, `c.i64`, `c.u64`, `c.Size`, `c.c_char`, and `c.c_int`.
- Read-only and mutable pointer descriptions: `c.ConstPtr[T]` and `c.MutPtr[T]`.
- `enum` variants with one explicit scalar carrier and a native constant name.
- Plain `struct` declarations with an explicit native C type name and listed fields.

For the executable subset, Incan currently carries scalar values as `int`. Generated wrappers range-check every conversion to and from the exact C scalar type; a value that cannot be represented by Incan `int` is not silently truncated. A verified enum constant is available as an ordinary integer expression such as `LibC.Status.OK`. Pointer and by-value structure contracts are verified declarations in this slice, but calls using them stay rejected until the later ownership and view design is implemented.

## Verification and diagnostics

Before code generation, the compiler renders a non-executable C probe from the binding descriptor and invokes a Clang-compatible toolchain for the selected host ABI. The probe checks the exact free-function signature, every enum constant's declared carrier, and the size, alignment, and field offsets of every listed plain structure. A mismatch is reported at the binding declaration before native execution.

Headers and native names are explicit in source. The verifier neither scans arbitrary headers to infer an API nor searches for a library that happens to provide a symbol. The logical library name records the link capability; target artifact selection and locking are intentionally deferred to the native-artifact slice.

The repository verifies the pure checked-ABI fixture in Linux x86-64 and macOS arm64 Clang target modes. A normal project invocation checks its host target; cross-target toolchain provisioning and deployable target plans are not yet part of this slice.

## Not included yet

Do not use this surface for:

- owned C resources, `Out` / `InOut` parameters, release rules, or borrowed views;
- callbacks, variadics, unions, bitfields, pointer arithmetic, casts, dereferences, or dynamic symbol lookup;
- native artifact downloads, vendored libraries, C/C++ shim compilation, or `incan.pub` publication;
- Android, Xcode, Gradle, or signing handoff artifacts.

Those boundaries will build on the checked descriptor rather than adding a second source of ABI truth.

## Related guidance

- [Write your first checked C binding](../../tutorials/checked_c_binding.md)
- [Work with checked C bindings](../../how-to/checked_c_bindings.md)
- [How checked C interop is structured](../../explanation/checked_c_interop.md)
