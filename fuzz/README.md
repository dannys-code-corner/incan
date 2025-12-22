# Fuzzing for Incan

This directory contains fuzz targets for the Incan compiler using `cargo-fuzz`.

## Setup

Install cargo-fuzz:

```bash
cargo install cargo-fuzz
```

## Running Fuzz Tests

Fuzz the parser (lexer + parser pipeline):

```bash
cargo fuzz run parse
```

Run with more threads:

```bash
cargo fuzz run parse -- -workers=4
```

Run for a specific duration:

```bash
cargo fuzz run parse -- -max_total_time=60
```

## Targets

- `parse` - Fuzzes the lexer and parser with arbitrary input strings

## Crash Artifacts

If fuzzing finds a crash, the input will be saved to `fuzz/artifacts/parse/`.
You can reproduce the crash with:

```bash
cargo fuzz run parse fuzz/artifacts/parse/crash-<hash>
```

## Corpus

The fuzzer builds a corpus of interesting inputs in `fuzz/corpus/parse/`.
You can add seed inputs to this directory to guide fuzzing toward specific areas.
