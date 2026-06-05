{ pkgs, dbName ? "discourse_rs_development", port ? 5432 }:

{
  buildInputs = with pkgs; [
    postgresql_16
  ];

  shellHook = ''
    export PGDATA="$PWD/.nix-postgres"
    export PGHOST="$PGDATA"
    export PGPORT="${toString port}"
    export DATABASE_URL="postgresql://localhost:${toString port}/${dbName}?host=$PGDATA"

    # Initialize database if it doesn't exist
    if [ ! -d "$PGDATA" ]; then
      echo "Initializing PostgreSQL database..."
      initdb --locale=C.UTF-8 --encoding=UTF8 -U postgres
    fi

    db_start() {
      if pg_ctl status > /dev/null 2>&1; then
        echo "PostgreSQL is already running on port ${toString port}"
      else
        pg_ctl start -l "$PGDATA/logfile" -o "-k $PGDATA -p ${toString port}"
        echo "PostgreSQL started on port ${toString port}"

        # Create user role if it doesn't exist
        sleep 1
        psql -U postgres -d postgres -tc "SELECT 1 FROM pg_roles WHERE rolname = '$USER'" | grep -q 1 || \
          psql -U postgres -d postgres -c "CREATE ROLE $USER WITH LOGIN SUPERUSER CREATEDB"

        # Create database if it doesn't exist
        createdb ${dbName} 2>/dev/null || echo "Database ${dbName} already exists"
      fi
    }

    db_stop() {
      pg_ctl stop
      echo "PostgreSQL stopped"
    }

    db_status() {
      pg_ctl status
    }

    db_test_setup() {
      # Create + migrate the test database. Safe to re-run; existing DB is
      # left alone and `diesel migration run` is idempotent.
      createdb discourse_rs_test 2>/dev/null || echo "Database discourse_rs_test already exists"
      TEST_DATABASE_URL="postgresql://localhost:${toString port}/discourse_rs_test?host=$PGDATA" \
        diesel migration run --database-url "postgresql://localhost:${toString port}/discourse_rs_test?host=$PGDATA"
    }

    echo ""
    echo "PostgreSQL commands available:"
    echo "  db_start       - Start PostgreSQL on port ${toString port}"
    echo "  db_stop        - Stop PostgreSQL"
    echo "  db_status      - Check PostgreSQL status"
    echo "  db_test_setup  - Create + migrate the test database (discourse_rs_test)"

    # The integration test harness reads this. Tests run with their own
    # connection pool against the test DB; the dev DB is never touched.
    export TEST_DATABASE_URL="postgresql://localhost:${toString port}/discourse_rs_test?host=$PGDATA"
  '';
}
