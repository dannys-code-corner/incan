# Glossary

This page defines common terms used throughout the docs.

## Type

A type describes what a value is (for example: `int`, `str`, `bool`) and what operations are valid on it.

See also: [Language reference (generated)](language.md).

## Function

A function is a named block of code you can call. In Incan:

```incan
def add(a: int, b: int) -> int:
    return a + b
```

## Module

A module is a `.incn` file that contains code and definitions (functions, models, constants, etc.).

## Import

An import brings definitions from another module into the current file.

See:

- Explanation: [Imports and modules](imports_and_modules.md)
- How-to: [Imports and modules (how-to)](../how-to/imports_and_modules.md)
- Reference: [Imports and modules (reference)](imports_and_modules.md)

## Result

`Result[T, E]` represents either success (`Ok(T)`) or failure (`Err(E)`), and is commonly used for typed error handling.

See: [Error Handling](../explanation/error_handling.md).

## Option

`Option[T]` represents either “some value” (`Some(T)`) or “no value” (`None`).

See: [Error Handling](../explanation/error_handling.md).

## Async

Async code lets a program do other work while waiting on I/O (network, disk, timers).

See: [Async Programming](../how-to/async_programming.md).

## rustup

`rustup` is the Rust toolchain installer and version manager. It installs `rustc` and `cargo`.

## cargo

`cargo` is Rust’s build tool and package manager. Incan uses Cargo under the hood when building generated Rust projects.

## PATH

`PATH` is an environment variable that controls which directories your shell searches for executable commands (like `incan`).

## make

`make` runs Makefile targets.
This repository provides canonical commands like `make install`, `make release`, and `make smoke-test`.

## crate

In these docs, “crate” is used in the Rust sense (“a compiled unit/package”)
and sometimes as shorthand for “the project/module root”.
See: [Imports and modules](imports_and_modules.md).
