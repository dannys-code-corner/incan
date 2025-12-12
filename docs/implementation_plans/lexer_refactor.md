# Implementation Plan: Lexer Refactoring

**GitHub Issue:** [#2 - chore: simplify lexer frontend](https://github.com/dannys-code-corner/incan/issues/2)  
**Status:** Complete  
**Priority:** Low

## Overview

Refactor `src/frontend/lexer.rs` (~1000 lines) into a cleaner, modular structure without changing user-facing behavior.

## Phases

### Phase 1: Quick Wins (no structural changes)

**Goal:** Improve readability without moving code around.

- [x] Add section comments for visual navigation
- [x] Add `phf` dependency to `Cargo.toml`
- [x] Replace keyword match statement with `phf_map`
- [x] Extract operator matching into helper method

**Estimated effort:** 1-2 hours  
**Risk:** Very low — localized changes

### Phase 2: Module Split

**Goal:** Break the monolithic file into focused modules.

```
src/frontend/lexer/
├── mod.rs          # Re-exports, Lexer struct, tokenize()
├── tokens.rs       # TokenKind, Token, FStringPart
├── scan.rs         # scan_token() and dispatch logic
├── strings.rs      # String/fstring/byte-string scanning
├── numbers.rs      # Number literal scanning
└── indent.rs       # Indentation handling (INDENT/DEDENT)
```

- [ ] Create `lexer/` directory structure
- [ ] Move `TokenKind`, `Token`, `FStringPart` to `tokens.rs`
- [ ] Move string scanning functions to `strings.rs`
- [ ] Move number scanning to `numbers.rs`
- [ ] Move indentation logic to `indent.rs`
- [ ] Update imports across codebase
- [ ] Run tests to verify no regressions

**Estimated effort:** 2-3 hours  
**Risk:** Medium — many files change, but tests will catch issues

### Phase 3: Reduce String Duplication

**Goal:** Unify shared logic in `scan_string`, `scan_byte_string`, `scan_fstring`.

- [ ] Identify common patterns (escape handling, quote matching)
- [ ] Extract shared escape sequence handler
- [ ] Create unified string scanning with mode enum
- [ ] Simplify individual functions to use shared code

**Estimated effort:** 1-2 hours  
**Risk:** Medium — string parsing is sensitive

### Phase 4: Polish

**Goal:** Final cleanup and documentation.

- [ ] Update `src/frontend/DEVNOTES.md` with new structure
- [ ] Add doc comments to public functions
- [ ] Review and clean up any remaining duplication

**Estimated effort:** 30 minutes  
**Risk:** Very low

## Testing Strategy

Each phase must pass:

```bash
cargo test                    # All unit tests
cargo test --test integration # Integration tests
incan run examples/basic/*.incn   # Smoke test examples
```

## Success Criteria

- [ ] All existing tests pass
- [ ] No user-facing behavior changes
- [ ] Lexer code is easier to navigate (modules < 300 lines each)
- [ ] New contributors can understand structure quickly

## Rollback Plan

If issues arise:
- Each phase is in a separate commit
- Easy to revert individual phases
- Original code preserved in git history

