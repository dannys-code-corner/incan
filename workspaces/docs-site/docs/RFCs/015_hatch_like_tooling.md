# RFC 015: Hatch-like Tooling (Project Lifecycle CLI)

**Status:** Planned
**Created:** 2025-12-23  

## Summary

Introduce a first-class, batteries-included project lifecycle CLI—similar in spirit to Python’s Hatch—for:

- **Versioning**: `incan version <major|minor|patch|alpha|beta|rc|dev>` (with optional `--dry-run`)
- **Project scaffolding**: `incan init` (in-place) and `incan new <name>` (new directory)
- **Matrix testing** (tox/nox-style): `incan test --matrix ...` with declarative environments
- Additional “hatch-like” ergonomics where it fits Incan’s workflow (format/lint/release/build/publish).

This RFC defines the CLI surface, the project metadata format, and the implementation boundaries
so we don’t bake policy into ad-hoc scripts.

## Motivation

Incan is a compiler + runtime ecosystem, but day-to-day developer experience is heavily shaped by tooling:

- Starting a new project should be **one command**.
- Bumping versions should be **correct and consistent** across the compiler, generated projects, and any package metadata.
- Running tests should support **repeatable environments** and **matrix execution**,
    without forcing users to learn Cargo internals.
- Release workflows should be **scriptable** and **standard** across projects.

Python’s Hatch demonstrates that a single tool can cover the project lifecycle. This RFC adapts the useful parts to Incan.

## Goals

- Provide an ergonomic, consistent, and scriptable CLI for common workflows:
    - `init`, `new`, `version`, `test`, `fmt`, `lint`, `build`, (future: `publish`)
- Define a single source of truth for project metadata (name, version, dependencies, entrypoint).
- Enable matrix testing via a declarative config.
- Keep generated projects deterministic and reproducible.
- Avoid “magic”: every generated file is readable and intended to be edited.

## Non-Goals (initial)

- Implement a public package registry client (publish/install) in this RFC (can be a follow-up RFC).
- Replace Cargo for Rust-level dependency resolution (we can orchestrate Cargo, not reinvent it).
- Provide virtualenv-style isolation identical to Python (we’ll use explicit env configs and reproducible commands instead).

## Terminology

- **Project**: An Incan repository containing Incan sources and metadata.
- **Environment**: A named test/build configuration (toolchain, flags, features, timeouts).
- **Matrix**: Running an environment set across multiple dimensions (e.g., debug/release, features on/off).

## Project Metadata

Add `incan.toml` at repo root (similar to `pyproject.toml`), as the canonical metadata source.

Example:

```toml
[project]
name = "hello_incan"
version = "0.1.0-alpha.1"
entrypoint = "src/main.incn"

[project.dependencies]
# Rust interop dependencies (RFC 013)
rand = "0.8"
serde = { version = "1.0", features = ["derive"] }

[tool.incan]
formatter = { line_length = 100 }

[tool.incan.test]
timeout_secs = 5

[[tool.incan.env]]
name = "unit"
command = ["incan", "test"]

[[tool.incan.env]]
name = "smoke"
command = ["incan", "smoke-test"]
```

Notes:

- `version` is SemVer-compatible with pre-release tags.
- Rust dependencies integrate with RFC 013 rules.
- Environments are explicit; no implicit “dev shell” assumptions.

## CLI Design

### `incan new <name>`

Create a new directory containing a minimal Incan project scaffold:

- `incan.toml`
- `src/main.incn` (hello world)
- `README.md`
- `.gitignore`

Flags:

- `--bin` / `--lib` (default: `--bin`)
- `--dir <path>` (default: `./<name>`)
- `--force` (overwrite)

### `incan init`

Initialize `incan.toml` (and `src/main.incn`) in the current directory.

Flags:

- `--force` (overwrite metadata)
- `--detect` (attempt to infer: entrypoint, existing version strings, etc.)

### `incan version <bump>`

Update the project version in `incan.toml` and any derived files that must match.

Bumps:

- `major`, `minor`, `patch`
- `alpha`, `beta`, `rc`, `dev`

Rules:

- `major/minor/patch` operate on the release core and clear pre-release unless `--keep-prerelease`.
- `alpha/beta/rc/dev`:
    - If no prerelease exists, append `-<tag>.1`
    - If same prerelease exists, increment numeric suffix
    - If different prerelease exists, switch tag and reset to `.1`

Flags:

- `--dry-run`
- `--set <version>` (explicit override)
- `--keep-prerelease`
- `--message <msg>` (for future integration with changelog/commit tooling)

Output should print:

- old version
- new version
- modified files

### `incan test`

Default test runner entrypoint.

Behavior:

- If `tool.incan.env` exists, running `incan test` executes the default environment (or `unit`).
- Otherwise, runs Incan’s standard checks: `cargo test` for compiler/runtime crates
    and `incan test` for Incan-level tests (if present).

Flags:

- `--env <name>` (select an env)
- `--matrix <expr>` (see below)
- `--list-envs`
- `--timeout <secs>`
- `--jobs <n>` (parallel env execution)
- `--fail-fast`

### Matrix execution

Matrix is defined either in `incan.toml` or via CLI:

Config:

```toml
[tool.incan.matrix]
axes = { profile = ["debug", "release"], features = ["default", "json"] }

[[tool.incan.matrix.include]]
profile = "release"
features = "json"
env = "unit"
```

CLI expression (initial, simple):

- `--matrix profile=debug,release features=default,json`

Execution strategy:

- Expand combinations
- Run each as a named environment with injected variables:
    - `INCAN_PROFILE`, `INCAN_FEATURES`, `INCAN_TIMEOUT`

## Additional Commands (Recommended)

These exist today in Makefiles across many repos.
This RFC explicitly prefers **CLI-native** equivalents so projects do not need Make as a dependency.

- `incan fmt` / `incan fmt --check`
- `incan lint` (clippy-like checks for compiler + emitted code)
- `incan smoke-test` (build + tests + examples + benchmark smoke-check, mirroring current repo conventions)
- `incan doctor` (environment diagnostics: toolchain version, cargo, PATH, permissions)

## Implementation Plan

### Phase 1: Metadata + scaffolding

- Add `incan.toml` parsing (serde + toml)
- Implement `incan new`, `incan init`
- Teach codegen/project generation to consult `incan.toml` when present

### Phase 2: Version command

- Implement SemVer parsing + bump logic
- Apply updates to `incan.toml`
- Optional: update any secondary manifests (only if this repo’s policy requires it)

### Phase 3: Environment + matrix runner

- Parse env definitions
- Implement `incan test --env`
- Implement `incan test --matrix`
- Add `--jobs` + `--fail-fast`

### Phase 4: Polish + docs

- Add guide pages: `docs/tooling/project_lifecycle.md`
- Provide “new project” tutorial for Python users
- Add examples of matrix testing

## Alternatives Considered

- **Rely solely on Makefile targets**: simple but inconsistent across repos, hard to compose and introspect;
    also adds an extra tool dependency we don’t need.
- **Embed everything in Cargo**: good for Rust, but Incan’s source-of-truth isn’t Cargo.toml;
    also doesn’t cover project scaffolding or Incan-centric metadata.
- **Adopt an existing tool (justfile, cargo-make)**: helps execution but doesn’t solve metadata/version semantics.

## Open Questions

1. Should `incan.toml` be mandatory for new projects, or optional?
2. Should `incan version` also update the compiler crate versions (workspace crates), or only project metadata?
3. How do we want to represent “dev” versions (e.g., `-dev.1` vs `-dev+<sha>`)?
4. Do we need a lockfile for `incan.toml` dependencies beyond Cargo.lock (likely no, but clarify)?

## Checklist

- [ ] Define `incan.toml` schema and document it
- [ ] Implement `incan new` and `incan init`
- [ ] Implement `incan version` bump logic
- [ ] Implement env runner and matrix expansion
- [ ] Docs + examples
