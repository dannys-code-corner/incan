# Incan Toolchain npm Package

Incan is a statically typed, Pythonic programming language for writing clear high-level application code that compiles to native Rust. Learn more at [incan.io](https://incan.io).

This package provides `incan` and `incan-lsp` command shims for the Incan toolchain. The install itself is a small reference shim and runs no npm lifecycle script; the first `incan` invocation provisions the matching checksum-verified toolchain archive for the current host from the GitHub release, exactly like the pip package. Later invocations reuse that installation.

```bash
npm install -g @incan/toolchain
incan --version
```

The command shims run the provisioned toolchain commands directly. Supported npm hosts are Linux x64, macOS x64, and macOS arm64.

`install-incan` remains available for explicit installer flows that need a custom manifest, cache location, or archive override. The script-free `incan` and `incan-lsp` shims do not provision Rust during `npm install`; make sure `rustup`, `cargo`, `rustc`, and the `wasm32-wasip1` target are available before building projects.
