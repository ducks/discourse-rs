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

    # Server management commands
    server_start() {
      echo "Starting discourse-rs server..."
      RUST_LOG=info cargo run
    }

    server_dev() {
      echo "Starting discourse-rs server in background..."
      RUST_LOG=info cargo run &
      echo "Server started with PID $!"
      echo "Use 'server_stop' to stop it"
    }

    server_stop() {
      echo "Stopping discourse-rs server..."
      pkill -f "target/debug/discourse-rs" && echo "Server stopped" || echo "No server running"
    }

    server_restart() {
      server_stop
      sleep 2
      server_dev
    }

    server_status() {
      if pgrep -f "target/debug/discourse-rs" > /dev/null; then
        echo "Server is running:"
        ps aux | grep "target/debug/discourse-rs" | grep -v grep
      else
        echo "Server is not running"
      fi
    }

    echo ""
    echo "Server commands available:"
    echo "  server_start   - Start server in foreground"
    echo "  server_dev     - Start server in background"
    echo "  server_stop    - Stop the server"
    echo "  server_restart - Restart the server"
    echo "  server_status  - Check if server is running"
    echo ""
    echo "Ready to build a forum in Rust!"
  '';
}
