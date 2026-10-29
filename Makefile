.PHONY: help version-bump release release-push build test parity-test clean clippy fmt fmt-check lint install-hooks \
	db-load db-test parity parity-check parity-record vendor-discourse snapshot-dv snapshot-clean reset-reference \
	record-pretty-text \
	bench bench-startup release-build install emoji-assets rehearsal wasm-plugins

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
PUBLIC_DIR ?= public

help:
	@echo "discourse-rs Makefile"
	@echo ""
	@echo "Usage:"
	@echo "  make release                       - Auto-version and release (recommended)"
	@echo "  make release VERSION=20260125.0.0  - Release with specific version"
	@echo "  make release-push                  - Push main and its release tag (after a failed push)"
	@echo "  make build                         - Build release binary"
	@echo "  make test                          - Run tests (needs make db-test once)"
	@echo "  make parity-test                   - Run the Rails comparison tests (slow)"
	@echo "  make clippy                        - Run clippy"
	@echo "  make fmt / fmt-check               - Format / check formatting"
	@echo "  make lint                          - fmt-check, clippy, tests (CI also runs parity-test)"
	@echo "  make install-hooks                 - Install a pre-push hook running 'make lint'"
	@echo "  sudo make install                  - Install the built binary as a systemd service (deploy/)"
	@echo "  make clean                         - Clean build artifacts"
	@echo ""
	@echo "Schema:"
	@echo "  make vendor-discourse [DISCOURSE=path] [REF=sha] - Vendor schema + settings from Discourse"
	@echo "  make db-load [FORCE=1]             - Load schema + seeds into $(DEV_DB)"
	@echo "  make db-test                       - (Re)load schema + seeds into $(TEST_DB)"
	@echo "  make emoji-assets [PUBLIC_DIR=path] - Install pinned emoji images (no Rails needed)"
	@echo "  make wasm-plugins                  - Build the WebAssembly plugin guests in plugins/"
	@echo "  make snapshot-dv AGENT=name        - Regenerate seed/fresh_install.sql from a dv agent"
	@echo "  make snapshot-clean AGENT=name     - Regenerate seed/clean_install.sql (migrate + db:seed only)"
	@echo "  make reset-reference AGENT=name    - Put a dv agent's database back to the seed (before recording)"
	@echo "  make record-pretty-text AGENT=name - Record Rails' cooking of the corpus into parity/pretty_text"
	@echo ""
	@echo "Parity (run discourse-rs with: env \$$(grep -v ^\# parity/environment) cargo run):"
	@echo "  make parity RAILS_URL=...          - Diff discourse-rs against running Discourse"
	@echo "  make parity-record RAILS_URL=...   - Save Rails responses to parity/golden"
	@echo "  make parity-check                  - Diff discourse-rs against parity/golden"
	@echo ""
	@echo "Bench (see scripts/bench for the caveats):"
	@echo "  make release-build                 - Optimized binary in target/release"
	@echo "  make bench TARGETS=\"rs=URL rails=URL\" - Compare request rates and latencies"
	@echo "  make bench-startup                 - Binary size, cold start, RSS (server env from the shell)"
	@echo ""
	@echo "Demo (docs/demo-script.md):"
	@echo "  make rehearsal RS_URL=...          - Walk the demo in Chromium; report in tmp/rehearsal/<time>/ (writes to that site)"
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
	@$(MAKE) --no-print-directory release-push

# Push main and the release tag on it. Run on its own after a release whose
# push failed: rerunning `make release` would cut another version.
release-push:
	@tag=$$(git describe --tags --exact-match main 2>/dev/null) || \
		{ echo "main is not on a tag; nothing to push (run make release)"; exit 1; }; \
	echo "Pushing main and $$tag to origin..."; \
	git push origin main && git push origin "$$tag" && \
	echo "Released $$tag"

# Build release binary
build:
	cargo build --release

# Integration tests read TEST_DATABASE_URL (set by shell.nix); run
# `make db-test` once, and again after vendoring a new schema.
test:
	cargo nextest run --workspace
	scripts/drop-stale-test-dbs

# The comparisons against recorded Rails output (.config/nextest.toml),
# left out of `make test`: the slow part of the suite. CI runs both.
parity-test:
	cargo nextest run --profile parity
	scripts/drop-stale-test-dbs

# Run clippy with warnings-as-errors (mirrors CI)
clippy:
	cargo clippy --workspace --all-targets -- -D warnings
	cargo clippy --package discourse-markdown --target wasm32-unknown-unknown -- -D warnings

# Clean build artifacts
clean:
	cargo clean

# Run rustfmt to format the code
fmt:
	cargo fmt --all

# Check that rustfmt is satisfied without modifying files (mirrors CI)
fmt-check:
	cargo fmt --all -- --check

# The checks CI runs, in order, minus parity-test (slow; CI runs it).
# Cheap to run locally before pushing.
lint: fmt-check clippy test

# Install a pre-push hook that runs `make lint` before any push, so CI
# failures from formatting / clippy / tests are caught locally.
install-hooks:
	@mkdir -p .git/hooks
	@printf '#!/usr/bin/env bash\nset -e\nexec make lint\n' > .git/hooks/pre-push
	@chmod +x .git/hooks/pre-push
	@echo "Installed pre-push hook -> make lint"

# The binary, unit, service account and /etc/discourse-rs/env (kept if it
# exists). Does not build: run `make build` first, as yourself.
install:
	deploy/install target/release/discourse-rs

emoji-assets:
	deploy/install-emoji "$(PUBLIC_DIR)"

# Third-party plugin guests (plugins/, docs/plugin-wasm.md), each built
# into its directory as plugin.wasm next to plugin.toml; PLUGINS_DIR=plugins
# then loads them. The JavaScript guest is pre-initialized with wizer.
wasm-plugins:
	cd plugins && cargo build --release --target wasm32-unknown-unknown
	cp plugins/target/wasm32-unknown-unknown/release/discourse_solved_wasm.wasm plugins/discourse-solved/plugin.wasm
	cp plugins/target/wasm32-unknown-unknown/release/echo_wasm.wasm plugins/echo/plugin.wasm
	cp plugins/target/wasm32-unknown-unknown/release/helpful_wasm.wasm plugins/helpful/plugin.wasm
	wizer --init-func wizer.initialize -o plugins/js-solved/plugin.wasm \
		plugins/target/wasm32-unknown-unknown/release/js_solved_wasm.wasm

vendor-discourse:
	scripts/vendor-discourse $(DISCOURSE) $(REF)

db-load:
	scripts/db-load $(if $(FORCE),--force) $(DEV_DB)

# The template keeps its tables as loaded (no autovacuum), so every run
# starts from the same heap layout and statistics and a failure reproduces.
# The writes cases do not depend on that state: what Rails leaves
# unordered is compared as unordered (README, Parity), and the parity
# profile passes against analyzed, vacuumed, VACUUM FULL and fresh copies.
db-test:
	scripts/db-load --force $(TEST_DB)
	psql -X -q -v ON_ERROR_STOP=1 -d $(TEST_DB) -c "DO \$$\$$ DECLARE r record; BEGIN FOR r IN SELECT schemaname, tablename FROM pg_tables WHERE schemaname NOT IN ('pg_catalog', 'information_schema') LOOP EXECUTE format('ALTER TABLE %I.%I SET (autovacuum_enabled = false, toast.autovacuum_enabled = false)', r.schemaname, r.tablename); END LOOP; END \$$\$$"
	scripts/drop-stale-test-dbs

parity:
	@test -n "$(RAILS_URL)" || { echo "set RAILS_URL, e.g. make parity RAILS_URL=http://127.0.0.1:3000"; exit 2; }
	cargo run --quiet --bin parity -- live --rails $(RAILS_URL) --rs $(RS_URL) --delay-ms 600

parity-record:
	@test -n "$(RAILS_URL)" || { echo "set RAILS_URL, e.g. make parity-record RAILS_URL=http://127.0.0.1:3000"; exit 2; }
	cargo run --quiet --bin parity -- record --rails $(RAILS_URL) --delay-ms 600

parity-check:
	cargo run --quiet --bin parity -- check --rs $(RS_URL)

snapshot-dv:
	@test -n "$(AGENT)" || { echo "set AGENT, e.g. make snapshot-dv AGENT=rs-parity"; exit 2; }
	scripts/snapshot-dv $(AGENT)

snapshot-clean:
	@test -n "$(AGENT)" || { echo "set AGENT, e.g. make snapshot-clean AGENT=rs-clean-seed"; exit 2; }
	scripts/snapshot-dv --clean $(AGENT)

reset-reference:
	@test -n "$(AGENT)" || { echo "set AGENT, e.g. make reset-reference AGENT=rs-parity"; exit 2; }
	scripts/reset-reference $(AGENT)

record-pretty-text:
	@test -n "$(AGENT)" || { echo "set AGENT, e.g. make record-pretty-text AGENT=rs-parity"; exit 2; }
	scripts/record-pretty-text $(AGENT)

release-build:
	cargo build --release

# e.g. make bench TARGETS="rs=http://127.0.0.1:8090 rails=http://127.0.0.1:3043" BENCH_ARGS="-d 10s -c 16"
bench:
	@test -n "$(TARGETS)" || { echo 'set TARGETS, e.g. make bench TARGETS="rs=http://127.0.0.1:8090 rails=http://127.0.0.1:3043"'; exit 2; }
	scripts/bench $(BENCH_ARGS) $(TARGETS)

bench-startup: release-build
	scripts/bench-startup

# The demo's dress rehearsal (docs/demo-script.md). It writes posts, chat
# messages and an account to the site at RS_URL: point it at a scratch
# copy of the demo database. SERVER_LOG, if set, is that server's log file.
rehearsal:
	scripts/demo-rehearsal --url $(RS_URL) --out tmp/rehearsal/$$(date -u +%Y%m%d-%H%M%S) \
		$(if $(SERVER_LOG),--server-log $(SERVER_LOG))
