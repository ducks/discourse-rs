.PHONY: help version-bump release build test clean clippy fmt fmt-check lint install-hooks \
	db-load db-test parity parity-check parity-record vendor-discourse

# Auto-generate version from today's date with auto-incrementing patch
# Format: YYYYMMDD.0.X where X increments if releasing multiple times per day
define get_next_version
$(shell \
	TODAY=$$(date +%Y%m%d); \
	LATEST=$$(git tag -l "v$$TODAY.*" 2>/dev/null | sort -V | tail -1); \
	if [ -z "$$LATEST" ]; then \
		echo "$$TODAY.0.0"; \
	else \
		PATCH=$$(echo "$$LATEST" | sed 's/.*\.0\.\([0-9]*\)/\1/'); \
		echo "$$TODAY.0.$$((PATCH + 1))"; \
	fi \
)
endef

VERSION := $(get_next_version)

DEV_DB ?= discourse_rs_development
TEST_DB ?= discourse_rs_test
DISCOURSE ?= $(HOME)/discourse/discourse
RS_URL ?= http://127.0.0.1:8080

help:
	@echo "discourse-rs Makefile"
	@echo ""
	@echo "Usage:"
	@echo "  make release                       - Auto-version and release (recommended)"
	@echo "  make release VERSION=20260125.0.0  - Release with specific version"
	@echo "  make build                         - Build release binary"
	@echo "  make test                          - Run tests (needs make db-test once)"
	@echo "  make clippy                        - Run clippy"
	@echo "  make fmt / fmt-check               - Format / check formatting"
	@echo "  make lint                          - All CI checks (fmt-check, clippy, tests)"
	@echo "  make install-hooks                 - Install a pre-push hook running 'make lint'"
	@echo "  make clean                         - Clean build artifacts"
	@echo ""
	@echo "Schema:"
	@echo "  make vendor-discourse [DISCOURSE=path] [REF=sha] - Vendor schema + settings from Discourse"
	@echo "  make db-load [FORCE=1]             - Load schema + seeds into $(DEV_DB)"
	@echo "  make db-test                       - (Re)load schema + seeds into $(TEST_DB)"
	@echo ""
	@echo "Parity (both servers must read the same database):"
	@echo "  make parity RAILS_URL=...          - Diff discourse-rs against running Discourse"
	@echo "  make parity-record RAILS_URL=...   - Save Rails responses to parity/golden"
	@echo "  make parity-check                  - Diff discourse-rs against parity/golden"
	@echo ""
	@echo "Next version will be: $(VERSION)"

# Bump version in Cargo.toml and commit on a branch
version-bump:
	@echo "Next version: $(VERSION)"
	@echo "Creating release branch for version $(VERSION)..."
	@git checkout -b release/v$(VERSION)
	@echo "Bumping version to $(VERSION)..."
	@sed -i 's/^version = .*/version = "$(VERSION)"/' Cargo.toml
	@echo "Updating Cargo.lock..."
	@cargo check --quiet 2>/dev/null || true
	@git add Cargo.toml Cargo.lock
	@git commit -m "chore: bump version to $(VERSION)"
	@echo ""
	@echo "Created branch release/v$(VERSION)"
	@echo "Version bumped to $(VERSION)"
	@echo "Commit created"

# Merge to main, tag, push. No cargo publish; discourse-rs isn't on
# crates.io yet; add it back here when it is.
release: version-bump
	@echo "Merging into main..."
	@git checkout main
	@git merge --no-ff release/v$(VERSION) -m "Merge branch 'release/v$(VERSION)'"
	@echo "Creating tag v$(VERSION) on main..."
	@git tag -a v$(VERSION) -m "Release v$(VERSION)"
	@echo "Pushing to origin..."
	@git push origin main
	@git push origin v$(VERSION)
	@echo ""
	@echo "Released v$(VERSION)"
	@echo "  - Merged release/v$(VERSION) into main"
	@echo "  - Tagged v$(VERSION)"
	@echo "  - Pushed to GitHub"

# Build release binary
build:
	cargo build --release

# Integration tests read TEST_DATABASE_URL (set by shell.nix); run
# `make db-test` once, and again after vendoring a new schema.
test:
	cargo test

# Run clippy with warnings-as-errors (mirrors CI)
clippy:
	cargo clippy --all-targets -- -D warnings

# Clean build artifacts
clean:
	cargo clean

# Run rustfmt to format the code
fmt:
	cargo fmt

# Check that rustfmt is satisfied without modifying files (mirrors CI)
fmt-check:
	cargo fmt -- --check

# Run all the checks CI runs, in order. Cheap to run locally before pushing.
lint: fmt-check clippy test

# Install a pre-push hook that runs `make lint` before any push, so CI
# failures from formatting / clippy / tests are caught locally.
install-hooks:
	@mkdir -p .git/hooks
	@printf '#!/usr/bin/env bash\nset -e\nexec make lint\n' > .git/hooks/pre-push
	@chmod +x .git/hooks/pre-push
	@echo "Installed pre-push hook -> make lint"

vendor-discourse:
	scripts/vendor-discourse $(DISCOURSE) $(REF)

db-load:
	scripts/db-load $(if $(FORCE),--force) $(DEV_DB)

db-test:
	scripts/db-load --force $(TEST_DB)

parity:
	@test -n "$(RAILS_URL)" || { echo "set RAILS_URL, e.g. make parity RAILS_URL=http://127.0.0.1:3000"; exit 2; }
	cargo run --quiet --bin parity -- live --rails $(RAILS_URL) --rs $(RS_URL)

parity-record:
	@test -n "$(RAILS_URL)" || { echo "set RAILS_URL, e.g. make parity-record RAILS_URL=http://127.0.0.1:3000"; exit 2; }
	cargo run --quiet --bin parity -- record --rails $(RAILS_URL)

parity-check:
	cargo run --quiet --bin parity -- check --rs $(RS_URL)
