let
  # oxalica/rust-overlay, pinned: rust-toolchain.toml's version comes from
  # its manifests. Bump the rev (and sha256) to reach a newer release.
  rust-overlay = import (fetchTarball {
    url = "https://github.com/oxalica/rust-overlay/archive/368fee9beaab04ca6fe7af28db63caa9badb22fa.tar.gz";
    sha256 = "153wynqcjizxi46vh9xxf1rw5z7b59jhchma5mnxzv3iyhvwrgg6";
  });
in

{ pkgs ? import <nixpkgs> { overlays = [ rust-overlay ]; } }:

let
  rust = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
  postgres = import ./nix/postgres.nix {
    inherit pkgs;
    port = 5442;
  };
  # `make browser-test`: Playwright and its browsers.
  playwright = import ./nix/playwright.nix { inherit pkgs; };
  # The font Rails draws letter avatars with (Debian's fonts-urw-base35,
  # as on the reference), for LETTER_AVATAR_FONT.
  letterAvatarFont = pkgs.fetchurl {
    url = "https://raw.githubusercontent.com/ArtifexSoftware/urw-base35-fonts/20200910/fonts/NimbusSans-Regular.otf";
    sha256 = "020ddndnqlyxgc1xgjmvvbvpyrahf4ki0nxq18426m8mg16vw9bw";
  };
in

pkgs.mkShell {
  buildInputs = with pkgs; [
    rust
    # `make test`: tests across binaries in parallel
    cargo-nextest

    pkg-config
    openssl

    # scripts/bench
    oha
    jq
    curl

    # The S3 test server (tests/common/s3.rs, scripts/s3-test-server)
    minio

    # Plugin guests (plugins/, docs/plugin-wasm.md): inspecting modules
    # and pre-initializing them. The guests build with the toolchain's
    # wasm32-unknown-unknown target.
    wasm-tools
    wizer
  ] ++ postgres.buildInputs ++ playwright.buildInputs;

  shellHook = ''
    ${postgres.shellHook}
    ${playwright.shellHook}
    export LETTER_AVATAR_FONT="${letterAvatarFont}"

    echo "discourse-rs: $(rustc --version), $(postgres --version)"
    echo "  db_start / db_stop / db_status   local PostgreSQL"
    echo "  make db-load / make db-test      load vendored schema + seeds"
    echo "  make browser-test                the Playwright suite (tests/browser)"
    echo "  make parity RAILS_URL=...        diff against a running Discourse"
  '';
}
