# discourse-rs

A Rust port of the [Discourse](https://github.com/discourse/discourse) backend,
aiming for feature parity: same database, same JSON API, so the real Ember
frontend can run against it.

The pre-rewrite Actix/Diesel version is tagged `v20260610.0.1`.

## Principles

- **Discourse owns the schema.** `schema/structure.sql` is vendored verbatim
  from a pinned Discourse commit (`schema/DISCOURSE_REF`). discourse-rs has
  no migrations and refuses to start against a database missing any of them.
- **Parity is measured, not claimed.** Every ported endpoint gets a case in
  `parity/cases`, diffed against real Discourse responses.
- **Port behavior from the Rails source**, citing the file it came from.

## Stack

axum, tokio, sqlx (Postgres 16 + pgvector), clap, serde.

## Setup

```bash
nix-shell          # toolchain, Postgres on port 5442, DATABASE_URL/TEST_DATABASE_URL
db_start
make db-load       # structure.sql -> discourse_rs_development (FORCE=1 to replace)
make db-test       # structure.sql -> discourse_rs_test
cargo run          # http://127.0.0.1:8080  (BIND_ADDR to change)
make test
```

To run against a real Discourse database, point `DATABASE_URL` at it. Extra
migrations (newer Discourse, third-party plugins) are logged and tolerated.

## Schema

```bash
make vendor-schema DISCOURSE=~/discourse/discourse [REF=<sha>]
```

Copies `db/structure.sql` as committed at REF (default HEAD), so uncommitted
local plugin migrations never leak in. Reload the databases afterwards.

## Parity

`parity/cases` lists requests, one per line:

```
GET /latest.json ignore=/topic_list/topics/*/bumped_at
```

`ignore` takes JSON pointers (`*` matches any element or key) for fields that
legitimately differ between runs. JSON is compared structurally; status and
media type must match.

```bash
make parity RAILS_URL=http://127.0.0.1:3000   # live: Rails vs discourse-rs
make parity-record RAILS_URL=...              # save Rails responses to parity/golden/
make parity-check                             # running discourse-rs vs golden
```

`cargo test` also replays every golden file against the in-process router
(`tests/parity.rs`), so regressions fail without Rails or a server running.

Parity only means something when Rails and discourse-rs read the same
database: point discourse-rs at the Rails database, or restore a dump of it
into the local one.

## Ported

| Route | Discourse source |
|---|---|
| `GET /srv/status` | `app/controllers/forums_controller.rb` |
