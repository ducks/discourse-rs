# discourse-rs

A Rust port of the [Discourse](https://github.com/discourse/discourse) backend,
aiming for feature parity: same database, same JSON API, so the real Ember
frontend can run against it.

The pre-rewrite Actix/Diesel version is tagged `v20260610.0.1`.

## Principles

- **Discourse owns the schema.** `vendor/discourse/` holds files copied
  verbatim from a pinned Discourse commit (`vendor/discourse/DISCOURSE_REF`):
  `db/structure.sql`, `config/site_settings.yml`, `server.en.yml` and the base colors.
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

`seed/fresh_install.sql` is a data snapshot of a freshly provisioned Discourse (a
dv agent at the vendored commit), taken with `make snapshot-dv AGENT=<name>`.
It is the database the parity golden files were recorded from, so tests and
recordings see the same data.

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
changes never leak in. Re-snapshot the seed from an agent at the same commit
and reload the databases afterwards.

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
(`tests/parity.rs`) on the snapshot database with the recorded environment.

Recording workflow: create a fresh dv agent, vendor its commit, snapshot its
database, write its env to `parity/environment`, then `make parity-record`.
Live comparison needs both sides on the same database and configuration:
run discourse-rs against a `make db-load` of the snapshot with
`env $(grep -v ^# parity/environment) cargo run`.

## Ported

| Route | Discourse source | Notes |
|---|---|---|
| `GET /srv/status` | `forums_controller.rb` | |
| `GET /site/basic-info` | `site_controller.rb#basic_info` | S3 upload CDN not supported (explicit 500) |
| `GET /site` | `site_controller.rb#site`, `Site.json_for`, `SiteSerializer`, `SiteCategorySerializer` | anonymous only; plugin-added keys not yet (see `parity/cases`) |
| `GET /latest` | `list_controller.rb#latest`, `TopicQuery#list_latest`, `TopicListSerializer`, `TopicListItemSerializer` | anonymous only; `page`, `per_page`, `order`, `ascending`; plugin keys not yet |
| `GET /t/:slug/:id(/:post_number)` | `topics_controller.rb#show`, `TopicView`, `TopicViewSerializer`, `PostSerializer` | anonymous only; slug and page redirects, 404 JSON; suggested topics deterministic (Rails randomizes); plugin keys not yet |
| `GET /c/:slug_path/:id(/l/latest)` | `list_controller.rb#category_default`, `#category_latest` | anonymous only; subcategory scoping, `/none`, category pins and sort, slug redirects; `top` and `hot` not yet |
| `GET /categories` | `categories_controller.rb#index`, `CategoryList`, `CategoryDetailedSerializer` | anonymous only; featured topics; no pagination, parent, tag filter or `subcategory_list` yet |

Supporting ports: SiteSetting (YAML defaults, `locale_default`, typed DB
rows, GlobalSetting shadowing, upcoming-change promotion), server-side I18n
(en), UrlHelper/GlobalPath URL generation, SiteIconManager,
ColorScheme (hex_for_name, ColorSchemeSerializer with ColorMath), the
anonymous Guardian, FlagSerializer, sidebar sections, user themes,
Site#categories (plain-paragraph descriptions only), TopicQuery (latest),
TopicPostersSummary, avatar templates (letter avatar colors, uploaded,
system), Emoji unicode lookup from the discourse-emojis gem, TopicView
(paged and near-post chunks, timeline lookup, participants, flags summary).

Behaviors the port hits but hasn't implemented return an explicit 500
(`Unsupported`) rather than a guess: watched words, tag visibility rules,
category descriptions with markup (ExcerptParser), computed fancy titles
(HtmlPrettify), post link counts, hidden posts, topic timers, thumbnails,
user fields, enabled auth providers, user-selectable color schemes, group
flair uploads, S3 CDN.

Not yet: plugin settings files and plugin registries, upcoming-change
default overrides, `mandatory_values`, themeable settings, response
headers, any authenticated user.

## Pages

Anonymous readers get server-rendered HTML (askama templates in
`templates/`, one stylesheet in `static/`) built from the same documents the
JSON endpoints return: `/` and `/latest` (the topic list, paged), `/c/...`
(a category's list with its subcategories), `/categories` (the index with
featured topics), and `/t/:slug/:id` (the topic with its posts). Requests ending in `.json` get the
API document instead. The structure follows Discourse's crawler views.

`PUBLIC_DIR` (default `public`) is served at `/images` and `/uploads`: point
it at a Discourse `public/` directory or a restored backup's.

## Serving a backup

```bash
scripts/restore-backup site-backup.tar.gz mysite            # -> backups/mysite/
PUBLIC_DIR=backups/mysite DISCOURSE_SRC=~/discourse/discourse \
  DATABASE_URL=postgresql://localhost:5442/mysite?host=$PGDATA cargo run
```

`restore-backup` loads the backup's `dump.sql.gz` into a fresh database and
unpacks its `uploads/` where `PUBLIC_DIR` serves them. `DISCOURSE_SRC` (a
Discourse checkout with its bundle installed) supplies the stock images and
the emoji set a backup doesn't carry.

discourse-rs refuses a database behind the vendored schema. For an older
backup, vendor Discourse at the commit that matches the backup's migration
version (`meta.json`):

```bash
scripts/vendor-discourse ~/discourse/discourse \
  $(scripts/discourse-commit-for-migration ~/discourse/discourse <version>)
```
