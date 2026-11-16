# Demo servers

Two local servers for showing the port, both on the dev cluster
(`.nix-postgres`, port 5442), each with its job worker running. Without
it a post is never processed (no lightbox or optimized images, no video
thumbnail) and nobody is notified of a reply or mention. No SMTP is
configured, so email jobs fail with the reason, and a job kind that isn't
ported fails at once and logs why. The scheduled check_new_features
fetches development's `http://localhost:3000/new-features.json`, which
the SSRF guard refuses.

Validated November 1, 2026: both units run the release binary built from
1819a61f, after all 44 demo rehearsal steps passed on a fresh copy of the
8080 database. The binary is `tmp/demo/discourse-rs-1819a61f`; its SHA-256
is `8440b5ebcc9ea74d26da68e8f1778f59e0e6397d9167fd3fe9df8ca09625686d`.
The local report is `tmp/rehearsal/release-20261101-focus/report/index.html`.
The previous `discourse-rs-v20261024.0.1` binary is retained for rollback.

Both sites allow video and audio uploads: `authorized_extensions` adds
`mp4|mov|webm|mp3|m4a|ogg` to the defaults (a stock Discourse refuses
them). These values were originally written to `site_settings` directly,
typed as Rails stores a list (data_type 8). The admin list editor now
supports saving them too:

```bash
nix-shell --run "psql -d discourse_rs_demo -c \"INSERT INTO site_settings
  (name, data_type, value, created_at, updated_at) VALUES
  ('authorized_extensions', 8,
   'jpg|jpeg|png|gif|heic|heif|webp|avif|svg|jxl|mp4|mov|webm|mp3|m4a|ogg',
   now(), now()) ON CONFLICT (name) DO UPDATE SET value = EXCLUDED.value\""
```

(and the same for `discourse_rs_meta_public`), then restart the units.

Set `max_oneboxes_per_post` to 5 for the walkthrough. The benchmark
snapshot has it set to 0, so otherwise chat links stay plain links even
with message processing enabled.

| Port | Database | Data |
|---|---|---|
| 8080 | `discourse_rs_demo` | the cleaned large corpus: 1,089 topics, 20,576 posts, 220 users |
| 8092 | `discourse_rs_meta_public` | the public Meta baseline: 46 sampled topics, 524 posts (85 topics and 572 posts with the seed's rows) |

Log in as `admin` / `password` on either. Both datasets are described in
[synthetic-forum.md](synthetic-forum.md) and
[bench-public-meta-20261019.md](bench-public-meta-20261019.md).

Their databases live only in the cluster, so a re-created cluster loses
them. This page is how to bring them back.

## Loading the databases

The snapshots are untracked, under `bench/` and `backups/synthetic/` in the
main checkout. They were dumped by PostgreSQL 18's `pg_dump`. The
`\restrict` and `\unrestrict` meta-commands and `SET transaction_timeout`
are dropped on the way in, so they load into older servers too.

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

Each is an instance of the user unit `deploy/demo/drs-demo@.service`,
named by its port. The unit runs what is in `~/dev/discourse-rs/tmp/demo`
(ignored), not the checkout: `discourse-rs`, a symlink to a copied release
build, so the next `cargo build` doesn't replace the binary under a
running server, and `<port>.env` with the database (copied from
`deploy/demo/`). Its tmp and public directories are
`~/.local/state/drs-demo/<port>/`.

Installing, from a checkout with a release build:

```bash
D=~/dev/discourse-rs/tmp/demo
mkdir -p $D ~/.config/systemd/user
cp target/release/discourse-rs $D/discourse-rs-$(git rev-parse --short HEAD)
ln -sfn discourse-rs-$(git rev-parse --short HEAD) $D/discourse-rs
cp deploy/demo/8080.env deploy/demo/8092.env $D/
# A secret of their own, so logins survive a restart (without one each
# process makes up a random secret). Kept out of the repo.
for p in 8080 8092; do
  echo "DISCOURSE_SECRET_KEY_BASE=$(openssl rand -hex 64)" >> $D/$p.env
  chmod 600 $D/$p.env
done
cp deploy/demo/drs-demo@.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now drs-demo@8080 drs-demo@8092
```

The unit sets `DISCOURSE_SRC=~/discourse/discourse`: a Discourse checkout
with its bundle installed supplies the stock images (the logo, category and
avatar fallbacks), the emoji set and the fonts. Without it those answer 404
and the header shows a broken logo.

Showing a newer build: copy it next to the old one, point the symlink at
it and `systemctl --user restart drs-demo@8080 drs-demo@8092`.
`journalctl --user -u drs-demo@8080` has the logs.

Enabled user units start with the user's session. Without lingering
(`loginctl show-user $USER -p Linger`; on NixOS
`users.users.<name>.linger = true`) that means at login, not at boot.

## The cluster's settings

The full test suite needs more connections than the default 100, and runs
faster without durability. `nix/postgres.nix` writes these settings into
`postgresql.conf` when it creates the cluster. A cluster created before
that needs them set by hand:

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

## Upgrading the cluster's PostgreSQL

The shell runs the PostgreSQL major Discourse requires (18, from the
nixpkgs `flake.lock` pins), and warns when `.nix-postgres` was made by an
older one. `pg_upgrade` moves it across, keeping every database. With the
old server's `bin` (the `postgresql-and-plugins` one, so pgvector is
there; `ls /nix/store | grep postgresql-and-plugins-16`) as `$OLDB`, in
the main checkout:

```bash
PGDATA=$PWD/.nix-postgres $OLDB/pg_ctl stop -m fast
mv .nix-postgres .nix-postgres-16
nix-shell --run '
  set -e
  # The shell hook initdb-ed an empty cluster with the new defaults; its data
  # checksums do not match the old cluster, so start over without them.
  rm -rf "$PGDATA"
  initdb -D "$PGDATA" --locale=C.UTF-8 --encoding=UTF8 -U postgres --no-data-checksums
  cp .nix-postgres-16/postgresql.auto.conf "$PGDATA/"
  mkdir -p tmp/pg-upgrade && cd tmp/pg-upgrade
  pg_upgrade -U postgres -b '"$OLDB"' -B "$(dirname "$(command -v postgres)")" \
    -d ../../.nix-postgres-16 -D "$PGDATA" -o "-k $PWD" -O "-k $PWD"
  db_start
  psql -U postgres -d postgres -f update_extensions.sql
  vacuumdb -U postgres --all --analyze-in-stages --missing-stats-only
  # The libc collation version moves with the new glibc: rebuild, then
  # record it.
  reindexdb -U postgres --all
  for db in $(psql -U postgres -d postgres -Atc "SELECT datname FROM pg_database WHERE datallowconn"); do
    psql -U postgres -d postgres -c "ALTER DATABASE \"$db\" REFRESH COLLATION VERSION"
  done
'
```

Restart the demo units afterwards. `.nix-postgres-16` is the old cluster
untouched; delete it once the new one is in use.
