# Demo servers

Two local servers for showing the port, both on the dev cluster
(`.nix-postgres`, port 5442), each with its job worker running. Without
it a post is never processed (no lightbox or optimized images, no video
thumbnail) and nobody is notified of a reply or mention. No SMTP is
configured, so email jobs fail with the reason, and a job kind that isn't
ported fails at once and logs why. The scheduled check_new_features
fetches development's `http://localhost:3000/new-features.json`, which
the SSRF guard refuses.

Both sites allow video and audio uploads: `authorized_extensions` adds
`mp4|mov|webm|mp3|m4a|ogg` to the defaults (a stock Discourse refuses
them). The admin's site settings can't save a list setting yet (164), so
it was written to `site_settings` directly, typed as Rails stores a list
(data_type 8):

```bash
nix-shell --run "psql -d discourse_rs_demo -c \"INSERT INTO site_settings
  (name, data_type, value, created_at, updated_at) VALUES
  ('authorized_extensions', 8,
   'jpg|jpeg|png|gif|heic|heif|webp|avif|svg|jxl|mp4|mov|webm|mp3|m4a|ogg',
   now(), now()) ON CONFLICT (name) DO UPDATE SET value = EXCLUDED.value\""
```

(and the same for `discourse_rs_meta_public`), then restart the units.

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
cp deploy/demo/drs-demo@.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now drs-demo@8080 drs-demo@8092
```

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
