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
in

pkgs.mkShell {
  buildInputs = with pkgs; [
    rust
    # `make test`: tests across binaries in parallel
    cargo-nextest

    pkg-config
    openssl

    # uploads: the dominant colour, as Rails reads it (`magick`, IM7)
    imagemagick

    # scripts/bench
    oha
    jq
    curl
  ] ++ postgres.buildInputs;

  shellHook = ''
    ${postgres.shellHook}

    echo "discourse-rs: $(rustc --version), $(postgres --version)"
    echo "  db_start / db_stop / db_status   local PostgreSQL"
    echo "  make db-load / make db-test      load vendored schema + seeds"
    echo "  make parity RAILS_URL=...        diff against a running Discourse"
  '';
}
