{ pkgs ? import <nixpkgs> {} }:

let
  postgres = import ./nix/postgres.nix {
    inherit pkgs;
    port = 5442;
  };
in

pkgs.mkShell {
  buildInputs = with pkgs; [
    rustc
    cargo
    rustfmt
    clippy

    pkg-config
    openssl
  ] ++ postgres.buildInputs;

  shellHook = ''
    ${postgres.shellHook}

    echo "discourse-rs: $(rustc --version), $(postgres --version)"
    echo "  db_start / db_stop / db_status   local PostgreSQL"
    echo "  make db-load / make db-test      load vendored structure.sql"
    echo "  make parity RAILS_URL=...        diff against a running Discourse"
  '';
}
