# Frontend packages and build caching

Core JS and CSS are runtime files, not Rust compiler inputs. The ordered
catalog is `static/manifest.json`; it preserves the original CSS concatenation
and Ace bundle separators. `scripts/package-frontend` reads this catalog and
writes an immutable package containing a manifest and content-addressed objects.
It uses Python's standard library and never invokes Cargo. Manifest entries
are available through the `/assets/` fallback, so adding a JS/CSS file does
not require adding a Rust route.

The server validates and loads the entire package before listening. A missing
file, invalid manifest or digest mismatch prevents startup. Asset URLs still
use `?v=<digest>`, ETags and immutable caching for known versions. Unknown
versions retain the existing revalidation behavior. Historical versions supplied
with `--previous` remain available to tabs opened before the update.

## Local development and browser tests

Without an override, a checkout reads `static/manifest.json` and its source
files at startup. Edit JS/CSS and restart the existing binary; no compilation
is needed. Templates, Rust-rendered HTML, and the generated vendor bundles in
`build.rs` still require a Rust build.

The browser harness packages the frontend into its scratch directory. To test
frontend-only changes against a previously built compatible server:

```bash
BROWSER_TEST_BINARY="$PWD/target-gate/debug/discourse-rs" \
  scripts/browser-test categories.spec.js
```

Without `BROWSER_TEST_BINARY`, the harness builds the server as before. Only
skip that step when Rust sources have not changed since that binary was built.

## Release packages

Inside `nix-shell`:

```bash
make release-build
python3 scripts/package-frontend tmp/frontend-20261102
# For an update, retain the previous package's versions:
python3 scripts/package-frontend tmp/frontend-next --previous tmp/frontend-20261102
```

Destinations must not already exist. Package creation happens in a temporary
directory and publishes only after all inputs validate. Activate complete
packages rather than overwriting files in the active directory.

`DISCOURSE_FRONTEND_DIR` selects a package explicitly. Otherwise installed
binaries look under `../share/discourse-rs/frontend` relative to their bin
directory, then fall back to the build checkout's `static` directory. Explicit
paths that are invalid fail startup rather than silently falling back.

Release tarballs include `frontend/`. `deploy/install BINARY FRONTEND` installs
a versioned copy and atomically switches its frontend symlink. It retains old
copies and does not restart the service. Packages should include `--previous`
when old tabs need assets from earlier versions. Nix builds the frontend as a
separate derivation and supplies its directory through the executable wrapper.

## Asset-only staging update

The currently deployed embedded-assets binary needs one server upgrade before
it supports this workflow. After that upgrade, configure the service with
`DISCOURSE_FRONTEND_DIR=/opt/discourse-rs/frontend-current` and use:

1. Package the new sources with `--previous` pointing to the currently deployed
   package. Copy the complete package into a new directory on the VPS.
2. Record the current symlink target, then switch `frontend-current` atomically
   using a temporary symlink and `mv -Tf` on the same filesystem.
3. Restart the service, check `/srv/status`, then run focused live browser checks.
   If validation fails, restore the previous symlink and restart.
4. Retain the old binary and asset directory together for rollback. Historical
   objects accumulate deliberately; remove old versions only when retiring the
   corresponding browser-cache compatibility window.

The app loads assets once, so changing a symlink requires a restart. This does
not yet fix the long shutdown observed with live connections on staging.

## Release cache

`scripts/release-build` uses the primary checkout's `target` directory for all
worktrees, unless `CARGO_TARGET_DIR` is explicitly supplied. It builds the
locked dependency graph with the pinned toolchain when run inside `nix-shell`.
Use consistent flags and profiles; avoid `cargo clean` for routine deployments.
Build once, test that artifact, and copy it rather than rebuilding for deployment.

The November 2 embedded-assets release took 24m 10s, including dependency
compilation. That run had no release binary in the selected target directory;
the underlying reason previous artifacts were absent has not been established.
Do not label it a Cargo cache-invalidation bug without evidence. Measure clean,
unchanged, JS-only, CSS-only and Rust-only builds separately.

## Validation of the split

An independent standalone Rust program evaluated the former embedded constants
with Rust's own `include_str!` and `concat!`. All 137 existing assets, totaling
7,234,249 bytes, matched the package byte-for-byte and MIME type. The additional
LTI submit script also matches its original source.

Packaging regression tests cover concatenation, changed digests, retained
history, corrupt/missing inputs, path escapes and immutable destination handling.
Installer checks under `umask 077` verified readable asset permissions, atomic
symlink replacement, old-release retention and environment-file preservation.

Nix evaluation confirmed that edits to JS, CSS, `site.css` and the manifest
change the frontend/wrapper derivations without changing the core Rust source
or derivation. A frontend-only Nix build passed and all 138 digests survived
fixup. This check used local Nixpkgs 25.05 with the pinned Rust overlay; the
flake's Nixpkgs 26.05 VM build has not been run for this change.

Local packaging measurements on November 2 were 0.067 seconds unchanged,
0.071 seconds after a JS-only edit and 0.074 seconds after a CSS-only edit,
including retaining the previous package's versions. These measure packaging
only, not browser checks, transfer, service restart or a Rust build. No Cargo
process was invoked by the packaging command.

Final local checks passed: workspace/all-target clippy with warnings denied,
four runtime asset tests, 36 database-backed integration tests across the moved
domains and asset serving, and seven browser tests using the packaged frontend
with `BROWSER_TEST_BINARY` (no server build in the browser harness).

A direct `cargo check --lib` comparison after the final Rust change took
75.756 seconds to check the application crate, with dependencies reused. The
subsequent unchanged, JS-only and CSS-only cases took 0.190, 0.193 and 0.186
seconds respectively, each reporting zero rebuilt crates. The temporary JS/CSS
edits were restored. These are check timings, not optimized release build times;
a fresh optimized build and the initial VPS upgrade remain separate work.
