# Troubleshooting

This page collects common “it didn’t work” fixes when getting started with Incan from this repository.

## `incan: command not found` after `make install`

- `make install` installs `incan` into `~/.cargo/bin`.
- Ensure `~/.cargo/bin` is on your `PATH`.

Check:

```bash
ls -la ~/.cargo/bin/incan
echo "$PATH"
which incan || true
```

## I didn’t run `make install` (no-install fallback)

If you’re using the no-install fallback, run commands from the repository root and invoke:

```bash
./target/release/incan ...
```

If `./target/release/incan` does not exist yet, build it first:

```bash
make release
```

## Builds are slow the first time

The first `make release` (or first `incan build`) will compile Rust dependencies and can take a few minutes.

## Cargo needs internet access for dependencies

Some builds may download Rust crates via Cargo on first run. Ensure your environment can reach crates.io (or your configured
proxy/mirror).

## macOS: toolchain/linker issues

If you see errors about a missing C toolchain or linker, install Xcode Command Line Tools:

```bash
xcode-select --install
```

## Still stuck?

If you’re still stuck, please [open an issue](https://github.com/dannys-code-corner/incan/issues) and include:

- your OS and architecture
- the exact commands you ran
- the full error output
