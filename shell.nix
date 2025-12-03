{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    # Rust toolchain
    rustc
    cargo
    rustfmt
    clippy

    # Database
    postgresql
    diesel-cli

    # Build dependencies
    pkg-config
    openssl
  ];

  shellHook = ''
    echo "=== Discourse-rs Development Environment ==="
    echo "Rust version: $(rustc --version)"
    echo "Cargo version: $(cargo --version)"
    echo "Diesel CLI: $(diesel --version)"
    echo ""
    echo "Ready to build a forum in Rust!"
  '';
}
