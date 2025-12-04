{ pkgs ? import <nixpkgs> {} }:

let
  postgres = import ./nix/postgres.nix {
    inherit pkgs;
    dbName = "discourse_rs_development";
    port = 5432;
  };
in

pkgs.mkShell {
  buildInputs = with pkgs; [
    # Rust toolchain
    rustc
    cargo
    rustfmt
    clippy

    # Database
    diesel-cli

    # Build dependencies
    pkg-config
    openssl
  ] ++ postgres.buildInputs;

  shellHook = ''
    echo "=== Discourse-rs Development Environment ==="
    echo "Rust version: $(rustc --version)"
    echo "Cargo version: $(cargo --version)"
    echo "Diesel CLI: $(diesel --version)"
    echo ""
    ${postgres.shellHook}
    echo ""
    echo "Ready to build a forum in Rust!"
  '';
}
