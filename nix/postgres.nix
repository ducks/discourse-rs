{ pkgs, port ? 5442 }:

let
  # Discourse's structure.sql creates hstore, pg_trgm, unaccent (contrib) and
  # vector (pgvector, used by the bundled discourse-ai tables).
  postgresql = pkgs.postgresql_16.withPackages (ps: [ ps.pgvector ]);
in
{
  buildInputs = [ postgresql ];

  shellHook = ''
    # One cluster per clone: a git worktree uses its main checkout's, so
    # entering a worktree's shell doesn't initdb a second, empty one.
    common_dir="$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
    if [ -n "$common_dir" ]; then
      export PGDATA="$(dirname "$common_dir")/.nix-postgres"
    else
      export PGDATA="$PWD/.nix-postgres"
    fi
    unset common_dir
    export PGHOST="$PGDATA"
    export PGPORT="${toString port}"
    export DATABASE_URL="postgresql://localhost:${toString port}/discourse_rs_development?host=$PGDATA"
    export TEST_DATABASE_URL="postgresql://localhost:${toString port}/discourse_rs_test?host=$PGDATA"

    if [ ! -d "$PGDATA" ]; then
      echo "Initializing PostgreSQL data dir..."
      initdb --locale=C.UTF-8 --encoding=UTF8 -U postgres
    fi

    db_start() {
      if pg_ctl status > /dev/null 2>&1; then
        echo "PostgreSQL is already running on port ${toString port}"
        return
      fi
      pg_ctl start -w -l "$PGDATA/logfile" -o "-k $PGDATA -p ${toString port}"
      psql -U postgres -d postgres -tc "SELECT 1 FROM pg_roles WHERE rolname = '$USER'" | grep -q 1 || \
        psql -U postgres -d postgres -c "CREATE ROLE \"$USER\" WITH LOGIN SUPERUSER CREATEDB"
    }

    db_stop() {
      pg_ctl stop
    }

    db_status() {
      pg_ctl status
    }
  '';
}
