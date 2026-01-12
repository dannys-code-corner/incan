# CI & automation (projects / CLI-first)

This page collects the canonical, CI-friendly commands for **Incan projects** (using the `incan` CLI).

If you’re running CI for the **Incan compiler/tooling repository**, see: [CI & automation (repository)](../../contributing/how-to/ci_and_automation.md).

## Recommended commands

### Type check (fast gate)

Type-check a program without building/running it (default action when no subcommand is provided):

```bash
incan path/to/main.incn
```

### Format (CI mode)

Check formatting without modifying files:

```bash
incan fmt --check .
```

See also: [Formatting](formatting.md) and [CLI reference](../reference/cli_reference.md).

### Tests

Run all tests:

```bash
incan test .
```

See also: [Testing](testing.md) and [CLI reference](../reference/cli_reference.md).

### Run an incn file

Run a program and use its exit code as the CI result:

```bash
incan run path/to/main.incn
```

## GitHub Actions example

```yaml
- name: Type check
  run: incan path/to/main.incn

- name: Format (CI)
  run: incan fmt --check .

- name: Tests
  run: incan test .
```
