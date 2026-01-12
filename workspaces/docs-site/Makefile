# Docs site workspace Makefile
# ============================

.PHONY: docs  ## docs - Build and serve the documentation site locally
docs:
	@python -m mkdocs --version >/dev/null 2>&1 || $(MAKE) docs-install
	@$(MAKE) docs-serve

.PHONY: docs-install  ## docs - Install docs site dependencies (MkDocs + Material)
docs-install:
	@echo "\033[1mInstalling docs dependencies...\033[0m"
	@python -m pip install --upgrade pip
	@python -m pip install -r requirements-docs.txt
	@echo "\033[32m✓ Docs dependencies installed\033[0m"

.PHONY: docs-build  ## docs - Build docs site (MkDocs strict)
docs-build:
	@echo "\033[1mBuilding docs site...\033[0m"
	@python -m mkdocs build --strict
	@echo "\033[32m✓ Docs site built\033[0m"

.PHONY: docs-serve  ## docs - Serve docs site locally (MkDocs)
docs-serve:
	@echo "\033[1mServing docs site at http://127.0.0.1:8000 ...\033[0m"
	@python -m mkdocs serve -a 127.0.0.1:8000

.PHONY: docs-lint  ## docs - Lint markdown docs (markdownlint-cli2 via npx)
# Override to customize lint scope, e.g.:
#   make docs-lint DOCS_LINT_GLOBS="docs/**/*.md"
# By default we exclude the generated language reference.
DOCS_LINT_GLOBS ?= docs/**/*.md !docs/reference/language.md
docs-lint:
	@echo "\033[1mLinting docs markdown...\033[0m"
	@NPM_CONFIG_CACHE="$(PWD)/.npm-cache" npx --yes markdownlint-cli2 $(DOCS_LINT_GLOBS)
	@echo "\033[32m✓ Docs markdown lint passed\033[0m"
