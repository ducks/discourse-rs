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
changes never leak in: the schema, core's settings and locales, and the
bundled plugins' settings and client locales. Re-snapshot the seed from an
agent at the same commit and reload the databases afterwards.

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
## Bench

`make bench TARGETS="rs=URL rails=URL"` and `make bench-startup` compare the
port with a Discourse serving the same backup; BENCH.md keeps the runs and the
caveats (production-mode reference, Rails' anonymous cache, search rate limits).


## Ported

What comes next, and in what order, is in [ROADMAP.md](ROADMAP.md).

| Route| Route | Discourse source | Notes |
|---|---|---|
| `GET /srv/status` | `forums_controller.rb` | |
| `GET /site/basic-info` | `site_controller.rb#basic_info` | S3 upload CDN not supported (explicit 500) |
| `GET /site` | `site_controller.rb#site`, `Site.json_for`, `SiteSerializer`, `SiteCategorySerializer` | anonymous and logged-in (groups by visibility, permission and can_edit on categories, staff and admin keys); plugin-added keys not yet (see `parity/cases`) |
| `GET /latest` | `list_controller.rb#latest`, `TopicQuery#list_latest`, `TopicListSerializer`, `TopicListItemSerializer` | anonymous and logged-in (`topic_users` keys, muted topics/categories/tags, cleared pins, secure categories, unlisted topics for staff); `page`, `per_page`, `order`, `ascending`; plugin keys not yet |
| `GET /t/:slug/:id(/:post_number)` | `topics_controller.rb#show`, `TopicView`, `TopicViewSerializer`, `PostSerializer` | anonymous and logged-in (read state, bookmarks, `can_*` on the topic and posts, `actions_summary` with `can_act`, notices), private messages for participants (related and suggested messages, allowed users and groups); deleted replies for staff and ignored users refused; slug and page redirects, 404 JSON; suggested topics deterministic (Rails randomizes); plugin keys not yet |
| `GET /c/:slug_path/:id(/l/latest)` | `list_controller.rb#category_default`, `#category_latest` | anonymous and logged-in, every list filter (`/l/unread`, `/l/new`, ...), restricted categories for members; subcategory scoping, `/none`, `/l/top`, `/l/hot`, category pins and sort, slug redirects |
| `GET /top`, `GET /hot` | `list_controller.rb#top`, `#hot`, `TopicQuery#list_top_for`, `#list_hot` | anonymous and logged-in; period selection (`best_period_for`), `/top/:period` redirects |
| `GET /unread`, `/new`, `/unseen`, `/read`, `/posted`, `/bookmarks` (+ `.json`, `/c/.../l/<filter>`) | `list_controller.rb`, `TopicQuery#list_unread`, `#list_new` (unified new), `#list_unseen`, `#list_read`, `#list_posted`, `#list_bookmarks` | logged in only (anonymous: JSON 403, HTML 404); `enable_unified_new` with its group gate; `subset=` not ported |
| `GET /notifications(.json)` | `notifications_controller.rb#index`, `Notification.prioritized_list`, `NotificationSerializer` | logged in only; `recent` (with the `seen_notification_id` bump unless `silent`), paged with `filter`/`limit`/`offset`, `filter_by_types`, admins' `username`; the accessible-topic and disabled-badge filters; `populate_acting_user` settings, pending reviewables and DiscourseConnect not ported |
| `GET /u/:username/bookmarks(.json)`, `GET /u/:username/user-menu-bookmarks(.json)` | `users_controller.rb#bookmarks`, `#user_menu_bookmarks`, `BookmarkQuery`, `PostBookmarkable`, `TopicBookmarkable`, `UserBookmarkListSerializer` | the owner and admins; post and topic bookmarks as the viewer may see them (secure categories, the owner's private messages, whispers, hidden first posts), pinned and reminder ordering, `q`, `page`, `limit`; the user menu's unread reminder notifications with the bookmarks they are not about; the `.ics` feed, plugin bookmarkables (chat messages) and `lazy_load_categories` not ported |
| `GET /topics/private-messages{,-sent,-archive,-unread,-new,-warnings}/:username(.json)`, `GET /topics/private-messages-group/:username/:group(/archive,/new,/unread).json` | `list_controller.rb#private_messages*`, `TopicQuery::PrivateMessageLists` | logged in only; the mailbox owner vs the viewer (admins read any inbox), `message_route`'s 403/404s, participants and allowed users on items; `publish_read_state` groups and tag lists not ported |
| `GET /categories` | `categories_controller.rb#index`, `CategoryList`, `CategoryDetailedSerializer` | anonymous and logged-in (secure categories, permission, can_edit, muted topics, cleared pins); featured topics; no pagination, parent, tag filter or `subcategory_list` yet |
| `GET /tag/:name`, `GET /tag/:slug/:id(/l/:filter)`, `GET /tags/c/:slug_path/:id(/none)/:tag` | `tags_controller.rb#show_*`, `TopicQuery#filter_by_tags`, `TagSerializer`, `DiscourseTagging.visible_tags` | anonymous and logged-in (the viewer's tag groups and categories); `tags[]` intersections, synonyms, tag-group and category visibility, canonical redirects; `tags_listed_by_group` not ported |
| `GET /tags` | `tags_controller.rb#index` | anonymous and logged-in (staff counts, admins see unused tags, `pm_count`); `tags_listed_by_group` off |
| any route, `login_required` on | `application_controller.rb#redirect_to_login_if_required` | JSON 403 `not_logged_in` (topics add extras), HTML 302 to `/login` with `destination_url` cookie, `/` and `/login` render a login page; `/srv/status`, `/site/basic-info` and static files stay open; `auth_immediately` with DiscourseConnect or a single external login not ported |
| `GET /search(.json)?q=&page=`, `GET /search/query(.json)?term=&type_filter=` | `search_controller.rb#show`, `#query`, `lib/search.rb`, `GroupedSearchResults`, `SearchLog` | anonymous and logged-in (secure categories, whispers, per-user topic keys on `/search.json`, the log keyed by user); plain words and quoted phrases, relevance ranking with category priority and closed/archived penalties, per-topic aggregation, blurbs, user/category/tag/group facets, search log with the 5 s per-IP fold; advanced syntax (`in:`, `status:`, `category:`, `#`, `@`, `order:`, ...), search contexts, `search_for_id`, rate limits and pg headlines not ported |
| `GET /u/:username(.json)` (+ `/summary`, `/activity`, `/badges`...), `GET /user_actions.json` | `users_controller.rb#show`, `#summary`, `UserSerializer`, `HiddenProfileSerializer`, `UserSummary`, `UserAction.stream` | every viewer: mute/ignore/PM flags and visible groups for other members, the private block (emails, 2FA, notification buckets, auth tokens, preferences as `user_option`) for the user themself and staff, the badge post side-load for admins; badges side-loads, profile view tracking, hidden-profile rules; suspended/silenced users, bios, featured topics, user status, `include_post_count_for`, `/activity.json` feeds and `/card.json` not ported |
| `GET /robots.txt`, `GET /robots-builder.json` | `robots_txt_controller.rb` | allowed/blocked crawler agents, `allow_index_in_robots_txt`, `overridden_robots_txt`, the Sitemap line |
| `GET /sitemap.xml`, `/sitemap_:n.xml`, `/sitemap_recent.xml`, `/news.xml` | `sitemap_controller.rb`, `Sitemap` | the index regenerates the `sitemaps` rows the hourly job would; recent/news touch theirs |
| `POST /session`, `DELETE /session/:username`, `GET /session/csrf`, `GET /session/current`, `POST /login` | `session_controller.rb`, `Auth::DefaultCurrentUserProvider`, `UserAuthToken`, `CurrentUserSerializer` | local logins (PBKDF2), Rails-compatible encrypted `_t` and `_forum_session` cookies (same `secret_key_base` keeps sessions across a cutover), CSRF tokens, token rotation and expiry, logout, last-seen tracking; 2FA users get Rails' failure payload; rate limits, screened IPs, suspended-user messages, DiscourseConnect not ported |

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
(`Unsupported`) rather than a guess: watched words, computed fancy titles
(HtmlPrettify), post link counts, hidden posts, topic timers, thumbnails,
user fields, enabled auth providers, user-selectable color schemes, group
flair uploads, S3 CDN.

Not yet: plugin registries, themeable settings, response
headers.

## Cooking

Rails cooks a post by running Discourse's JavaScript markdown bundle in
V8. discourse-rs does not vendor that bundle or embed a JavaScript engine:
cooking is to be Rust rules on a Rust markdown-it, measured against Rails
like everything else. What exists so far is what surrounds the renderer:

- `pretty_text::options`: the options `PrettyText.markdown` hands the
  renderer (client site settings of core and the bundled plugins, allowed
  iframes, paths, hashtag types).
- `pretty_text::helpers`: `PrettyText::Helpers`, the lookups the rules
  make while cooking: translations, avatars, primary groups, upload URLs
  (Base62 short urls), topic titles for quotes, and hashtags (categories
  and tags under the cooking user's permissions).
- `make record-pretty-text AGENT=rs-parity` records, from Rails on the
  reference: those options, every helper call with its result, and
  `PrettyText.markdown`'s HTML for a corpus of 51 feature samples and
  the seeded posts. `tests/pretty_text.rs` holds the options and the
  helpers to that recording; the corpus is the target the renderer has
  to reach, byte for byte.

Refused with an explicit error rather than answered differently from
Rails: custom emoji, the emoji deny list, watched words, secure uploads,
uploads behind a CDN or S3, and a hashtag chat would resolve to a channel
the cooking user can see. What plugins add to the options (chat's,
discobot's iframe) is not produced.

## Pages

Anonymous readers get server-rendered HTML (askama templates in
`templates/`, one stylesheet in `static/`) built from the same documents the
JSON endpoints return: `/` and `/latest` (the topic list, paged), `/c/...`
(a category's list with its subcategories), `/tag/...` (a tag's list), `/search?q=` (results), `/u/:username` (a profile), and every page carries a canonical link, description and OpenGraph/Twitter meta like the crawler layout (with the non-canonical `noindex` header),
`/tags` (the tag index), `/categories` (the index with featured topics), and `/t/:slug/:id` (the topic with its posts). Requests ending in `.json` get the
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

## Deploying

One host, systemd, Postgres 16 with pgvector, Caddy in front. Every
release has a static x86_64 Linux binary attached, and its tarball carries
`deploy/`, so the host needs no toolchain:

```bash
tar xzf discourse-rs-v<version>-x86_64-linux.tar.gz
cd discourse-rs-v<version>-x86_64-linux
sudo deploy/install        # from a checkout: make build && sudo make install
```

That installs `/usr/local/bin/discourse-rs`, `discourse-rs.service`, the
`discourse-rs` account (systemd-sysusers) and `/etc/discourse-rs/env` from
`deploy/env.example`. Running it again upgrades the binary and keeps the
env file. Then:

1. Restore the backup (`scripts/restore-backup`, above) and put its
   `uploads/` under `/var/lib/discourse-rs/public`.
2. Give the service a database role. It reads everything and writes
   sessions, visits, search logs and sitemaps:
   `createuser discourse-rs`, then
   `GRANT pg_read_all_data, pg_write_all_data TO "discourse-rs"`.
3. Fill in `/etc/discourse-rs/env`: `DATABASE_URL`, `DISCOURSE_HOSTNAME`
   and `DISCOURSE_SECRET_KEY_BASE`. The secret must be the Rails site's
   for its sessions to survive a cutover; Rails keeps it in redis when it
   is not configured (`rails runner 'puts GlobalSetting.safe_secret_key_base'`).
4. `systemctl enable --now discourse-rs`, and add `deploy/Caddyfile` to
   Caddy's config with the real hostname.

The server binds to loopback and trusts `X-Forwarded-For`, so only the
proxy may reach it. `systemctl stop` (SIGTERM) lets in-flight requests
finish. A release binary embeds the schema vendored at its tag; a backup
older than that needs a build from a checkout vendored at the backup's
commit.
