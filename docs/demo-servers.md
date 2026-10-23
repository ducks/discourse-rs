# Demo servers

Two local servers for showing the port, both on the dev cluster
(`.nix-postgres`, port 5442) with background jobs off:

| Port | Database | Data |
|---|---|---|
| 8080 | `discourse_rs_demo` | the cleaned large corpus: 1,089 topics, 20,576 posts, 220 users |
| 8092 | `discourse_rs_meta_public` | the public Meta baseline: 46 sampled topics, 524 posts (85 topics and 572 posts with the seed's rows) |

Log in as `admin` / `password` on either. Both datasets are described in
[synthetic-forum.md](synthetic-forum.md) and
[bench-public-meta-20261019.md](bench-public-meta-20261019.md).

Nothing about them lives in the repo's code or in a unit file, so a reboot
or a re-created cluster loses them. This page is how to bring them back.

## Loading the databases

The snapshots are untracked, under `bench/` and `backups/synthetic/` in the
main checkout. They were dumped by PostgreSQL 18's `pg_dump`, so two lines
PostgreSQL 16's psql rejects are dropped on the way in: the `\restrict`
and `\unrestrict` meta-commands and `SET transaction_timeout`.

```bash
nix-shell --run '
  strip() { grep -v -E "^\\\\(un)?restrict |^SET transaction_timeout" "$1"; }
  createdb -U postgres -O "$USER" discourse_rs_demo
  strip bench/20261019-clean-corpus/forum.sql \
    | psql -X -q -v ON_ERROR_STOP=1 -d discourse_rs_demo
  createdb -U postgres -O "$USER" discourse_rs_meta_public
  strip backups/synthetic/discourse_synthetic_meta_baseline/forum.sql \
    | psql -X -q -v ON_ERROR_STOP=1 -d discourse_rs_meta_public
'
```

`bench/20261019-clean-corpus/checksums.txt` has the large snapshot's
SHA-256. Loading takes under a minute.

## Running them

Copy a release build to a path of its own, so the next `cargo build`
doesn't replace the binary under a running server, and start each as a
user unit:

```bash
D=~/dev/discourse-rs/tmp/demo
SHA=$(git rev-parse --short HEAD)
mkdir -p $D && cp target/release/discourse-rs $D/discourse-rs-$SHA
for spec in "8080 discourse_rs_demo" "8092 discourse_rs_meta_public"; do
  set -- $spec
  mkdir -p $D/$1/tmp $D/$1/public
  systemd-run --user --unit=drs-demo-$1 \
    --working-directory=/home/ducks/dev/discourse-rs \
    -E DATABASE_URL="postgresql://localhost:5442/$2?host=/home/ducks/dev/discourse-rs/.nix-postgres" \
    -E BIND_ADDR=127.0.0.1:$1 -E DISCOURSE_HOSTNAME=127.0.0.1 -E UNICORN_PORT=$1 \
    -E RAILS_ENV=development -E DISCOURSE_RS_JOBS=off -E RUST_LOG=info \
    -E TMP_DIR=$D/$1/tmp -E PUBLIC_DIR=$D/$1/public \
    $D/discourse-rs-$SHA
done
```

`journalctl --user -u drs-demo-8080` has the logs;
`systemctl --user stop drs-demo-8080` stops one. To update them, stop
both, copy the new build and start them again.

The units are transient: they survive the shell that started them, not a
reboot.

## The cluster's settings

The dev cluster's `ALTER SYSTEM` settings are not in `nix/postgres.nix`.
The full test suite needs more connections than the default 100, so a
re-created cluster needs these again:

```bash
nix-shell --run "
  psql -U postgres -d postgres \
    -c \"ALTER SYSTEM SET max_connections = '300'\" \
    -c \"ALTER SYSTEM SET fsync = 'off'\" \
    -c \"ALTER SYSTEM SET synchronous_commit = 'off'\" \
    -c \"ALTER SYSTEM SET full_page_writes = 'off'\"
  pg_ctl restart -w -l \$PGDATA/logfile -o \"-k \$PGDATA -p 5442\"
"
```

With `fsync` off an unclean stop can corrupt the cluster. On 2026-10-22 a
full disk did, and it would not start again (`could not locate a valid
checkpoint record`). Re-creating it means `initdb` (the shell hook does it
when `.nix-postgres` is missing), the settings above, `make db-load`,
`make db-test` and the two demo databases.
