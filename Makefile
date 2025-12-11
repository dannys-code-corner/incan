# Incan Programming Language - Makefile
# =====================================

.PHONY: help
help: build-quiet  ## Display this help message
	@./target/debug/incan --version
	@echo ""
	@echo "\033[1mBuild:\033[0m"
	@grep -E '^.PHONY: .*?## build - .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ".PHONY: |## build - "}; {printf "  \033[36m%-18s\033[0m %s\n", $$2, $$3}'
	@echo ""
	@echo "\033[1mCode Quality:\033[0m"
	@grep -E '^.PHONY: .*?## quality - .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ".PHONY: |## quality - "}; {printf "  \033[36m%-18s\033[0m %s\n", $$2, $$3}'
	@echo ""
	@echo "\033[1mTesting:\033[0m"
	@grep -E '^.PHONY: .*?## test - .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ".PHONY: |## test - "}; {printf "  \033[36m%-18s\033[0m %s\n", $$2, $$3}'
	@echo ""
	@echo "\033[1mTooling:\033[0m"
	@grep -E '^.PHONY: .*?## tool - .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ".PHONY: |## tool - "}; {printf "  \033[36m%-18s\033[0m %s\n", $$2, $$3}'
	@echo ""
	@echo "\033[1mMiscellaneous:\033[0m"
	@grep -E '^.PHONY: .*?## misc - .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ".PHONY: |## misc - "}; {printf "  \033[36m%-18s\033[0m %s\n", $$2, $$3}'
	@echo ""

# =============================================================================
# Build
# =============================================================================

.PHONY: build  ## build - Debug build (fast compile)
build:
	@echo "\033[1mBuilding (debug)...\033[0m"
	@cargo build

.PHONY: build-quiet
build-quiet:
	@cargo build --quiet 2>/dev/null || cargo build --quiet

.PHONY: release  ## build - Release build (optimized)
release:
	@echo "\033[1mBuilding (release)...\033[0m"
	@cargo build --release

.PHONY: install  ## build - Install to ~/.cargo/bin
install:
	@echo "\033[1mInstalling incan...\033[0m"
	@cargo install --path .
	@echo "\033[32m✓ Installed to ~/.cargo/bin/incan\033[0m"

# =============================================================================
# Code Quality
# =============================================================================

.PHONY: fmt  ## quality - Format Rust code
fmt:
	@echo "\033[1mFormatting code...\033[0m"
	@cargo fmt
	@echo "\033[32m✓ Code formatted\033[0m"

.PHONY: fmt-check  ## quality - Check formatting without changes
fmt-check:
	@echo "\033[1mChecking formatting...\033[0m"
	@cargo fmt -- --check

.PHONY: lint  ## quality - Run clippy linter
lint:
	@echo "\033[1mRunning clippy...\033[0m"
	@cargo clippy -- -D warnings

.PHONY: check  ## quality - Run all quality checks (fmt + lint)
check: fmt-check lint
	@echo "\033[32m✓ All checks passed\033[0m"

# =============================================================================
# Testing
# =============================================================================

.PHONY: test  ## test - Run all tests
test:
	@echo "\033[1mRunning tests...\033[0m"
	@cargo test

.PHONY: test-verbose  ## test - Run tests with output
test-verbose:
	@echo "\033[1mRunning tests (verbose)...\033[0m"
	@cargo test -- --nocapture

.PHONY: test-one  ## test - Run specific test (TEST=name)
test-one:
ifdef TEST
	@echo "\033[1mRunning test: $(TEST)\033[0m"
	@cargo test $(TEST) -- --nocapture
else
	@echo "Usage: \033[36mmake test-one TEST=test_name\033[0m"
	@echo "Example: make test-one TEST=test_run_c_import_this"
endif

# =============================================================================
# Tooling
# =============================================================================

.PHONY: lsp  ## tool - Build the LSP server
lsp:
	@echo "\033[1mBuilding LSP server...\033[0m"
	@cargo build --release --bin incan-lsp
	@echo "\033[32m✓ LSP server built: target/release/incan-lsp\033[0m"

.PHONY: vscode-package  ## tool - Package VS Code extension
vscode-package:
	@echo "\033[1mPackaging VS Code extension...\033[0m"
	@cd editors/vscode && vsce package
	@echo "\033[32m✓ Extension packaged\033[0m"

.PHONY: watch  ## tool - Watch for changes and rebuild (requires cargo-watch)
watch:
	@echo "\033[1mWatching for changes...\033[0m"
	@cargo watch -x build

# =============================================================================
# Miscellaneous
# =============================================================================

.PHONY: run  ## misc - Build and run (debug mode)
run:
	@cargo run --

.PHONY: zen  ## misc - Print the Zen of Incan
zen:
	@cargo build --release -q 2>/dev/null
	@./target/release/incan run -c "import this"

.PHONY: clean  ## misc - Clean build artifacts
clean:
	@echo "\033[1mCleaning...\033[0m"
	@cargo clean
	@rm -rf target/incan/
	@echo "\033[32m✓ Clean\033[0m"

.PHONY: version  ## misc - Show version info
version:
	@echo "\033[1mIncan version:\033[0m"
	@cargo pkgid | cut -d# -f2
	@echo ""
	@echo "\033[1mRust version:\033[0m"
	@rustc --version
