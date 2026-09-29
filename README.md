# discourse-rs

A Rust port of the [Discourse](https://github.com/discourse/discourse) backend,
aiming for feature parity: same database, same JSON API, so the real Ember
frontend can run against it.

The pre-rewrite Actix/Diesel version is tagged `v20260610.0.1`.

## Principles

- **Discourse owns the schema.** `vendor/discourse/` holds files copied
  verbatim from a pinned Discourse commit (`vendor/discourse/DISCOURSE_REF`):
  `db/structure.sql`, `config/site_settings.yml`, and the base colors.
  discourse-rs has no migrations and refuses to start against a database
  missing any of them.
- **Parity is measured, not claimed.** Every ported endpoint gets a case in
  `parity/cases`, diffed against real Discourse responses.
- **Port behavior from the Rails source**, citing the file it came from, and
  port the matching request specs as tests.

## Stack

axum, tokio, sqlx (Postgres 16 + pgvector), clap, serde.

## Setup

```bash
nix-shell          # toolchain, Postgres on port 5442, DATABASE_URL/TEST_DATABASE_URL
db_start
make db-load       # schema + seeds -> discourse_rs_development (FORCE=1 to replace)
make db-test       # schema + seeds -> discourse_rs_test (template for tests)
cargo run          # http://127.0.0.1:8080  (BIND_ADDR to change)
make test
```

`seed/*.sql` holds the rows Discourse's `db/fixtures` create on a fresh
install, added as slices need them.

Configuration mirrors Discourse: `RAILS_ENV` (default development, as in
Rails) changes URL generation the same way, and every `DISCOURSE_*` env var is
a GlobalSetting, shadowing the site setting of the same name
(`DISCOURSE_HOSTNAME`, `DISCOURSE_CDN_URL`, `DISCOURSE_TITLE`, ...).

To run against a real Discourse database, point `DATABASE_URL` at it. Extra
migrations (newer Discourse, third-party plugins) are logged and tolerated.

Tests clone the `discourse_rs_test` template into a private database per test,
so they can write freely and run in parallel.

## Vendoring

```bash
make vendor-discourse DISCOURSE=~/discourse/discourse [REF=<sha>]
```

Copies the files as committed at REF (default HEAD), so uncommitted local
changes never leak in, and regenerates `seed/010_uploads.sql`. Reload the
databases afterwards.

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
(`tests/parity.rs`) on a fresh seeded database with production defaults.

Parity only means something when both sides read the same database and
configuration: point discourse-rs at the Rails database with the same
`RAILS_ENV` and `DISCOURSE_*` settings.

## Ported

| Route | Discourse source | Notes |
|---|---|---|
| `GET /srv/status` | `forums_controller.rb` | |
| `GET /site/basic-info` | `site_controller.rb#basic_info` | S3 upload CDN not supported (explicit 500) |

Supporting ports: SiteSetting (YAML defaults, `locale_default`, typed DB
rows, GlobalSetting shadowing), UrlHelper/GlobalPath URL generation,
SiteIconManager, ColorScheme.hex_for_name. Not yet: plugin settings files,
upcoming-change default overrides, `mandatory_values`, themeable settings,
response headers.
