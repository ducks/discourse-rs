# Multisite: the single-site assumptions

The sweep behind ROADMAP item 208 (`feat/multisite`): every place the port
assumes one forum per process, classified by what multisite needs from it.
Taken on main at 0237e3c (2026-10-15). File references are under src/
unless they name another directory.

The design this assumes: one `AppState` and one `Router` per site, built
by the same startup main.rs runs today; a thin top-level service picks the
site from the request (RailsMultisite::Middleware's role) and calls its
router, so the 871 `state.pool` and 268 `state.bus` call sites do not
change; one job worker per site. Without a multisite.yml the process has
one site and behaves exactly as now.

## What Rails does

From the reference source and rails_multisite 7.0.0 (the gem Discourse
pins), the conventions the port follows:

- config/multisite.yml (`DISCOURSE_MULTISITE_CONFIG_PATH`, else
  `GlobalSetting.multisite_config_path`; config/application.rb:121) maps a
  db key to a connection spec: `adapter`, `database`, `host`, `port`,
  `username`, `password`, `pool`, `host_names`. The key `default` is
  refused (connection_management.rb `load_config!`): the default site is
  database.yml's, which Discourse builds from GlobalSetting (`DISCOURSE_DB_*`,
  `DISCOURSE_HOSTNAME`).
- RailsMultisite::Middleware runs first (200-first_middlewares.rb:101).
  The host is `request.host`, or the `__ws` param when the request's host
  is an asset hostname (`GlobalSetting.cdn_hostnames`, else the default
  site's `host_names`), and then the request's cookies are dropped. An
  unknown host is a 404 `not found`, except `/srv/status`, which runs on
  `default` (freedom_patches/rails_multisite.rb `db_lookup`).
- Cookie salts get the host appended
  (`RailsMultisite::CookieSalt.update_cookie_salts`: `"#{salt} #{host}"`
  for the signed and encrypted cookie salts), so a cookie from one host
  does not decrypt on another, even for two hostnames of one site.
- `current_db` is the db key; `Discourse.current_hostname` is
  `force_hostname` or the site's first host name (lib/discourse.rb:669).
- Uploads live at `uploads/#{current_db}` (file_store/base_store.rb:41),
  backups at `public/backups/#{current_db}`
  (backup_restore/local_backup_store.rb:6-9).
- Redis keys are namespaced by `current_db` (discourse_redis.rb:226),
  except RateLimiter's `global: true` limiters (`GLOBAL::` keys, without
  namespace; rate_limiter.rb:163-171), which the request tracker uses for
  per-IP and health check limits (request_tracker/rate_limiters/ip.rb:10).
- Jobs carry `current_site_id`; a job without one, as scheduled jobs run,
  runs once per db (jobs/base.rb:288-300). Sidekiq's pause is per site
  (sidekiq/pausable.rb:35). CachedCounting and Scheduler::Defer queue the
  db with the work (cached_counting.rb:165, scheduler/defer.rb:58).
- Routes with a hostname segment select the site by it, not by the Host
  header: `/highlight-js/:hostname/`, `/user_avatar/:hostname/`,
  `/svg-sprite/:hostname/` (`with_hostname(params[:hostname])`, e.g.
  highlight_js_controller.rb:16).
- `/uploads/:site/...` is served for any site: nginx serves local files,
  and UploadsController#show switches to `params[:site]`
  (uploads_controller.rb:127-133).

## 1. Statics and in-process state

294 `static` items in src/ and crates/markdown (3 task-local, 2
thread-local in the wasm build), 253 of them compiled regexes. Of the 41
others:

Per site (wrong as soon as two sites share the process):

| Where | What | Why |
|---|---|---|
| request_tracking.rs:493 `COUNTS` | CachedCounting's queue | counts from every site in one map |
| request_tracking.rs:494 `FLUSHER` | the minute's flush task | spawned once with the first pool it sees (:508), so every site's counts land in that site's database |
| request_tracking.rs:956 `QUEUE` | BrowserPageviewEvent's pending list (Redis, namespaced) | `flush_queued` inserts whatever is queued into the calling site's database |
| jobs/periodical.rs:20 `NEXT_CATEGORY_ID` | `category-featured-topic:next-category-id` (Redis, namespaced) | a category id from one site used as the batch start on another |
| jobs/periodical.rs:15 `CALLS` | `should_update_long_topics?` | Rails' class variable counts across sites (one execute per db); per site is the sane port, see "Unsure" |
| highlight_js.rs:42 `CACHE` | HighlightJs.version's cache | keyed by its inputs, so correct, but one slot thrashes between sites; Rails keys it by `current_db` (highlight_js.rb:33) |

Shared, correct as process-wide:

| Where | Why |
|---|---|
| request_tracking.rs:613 `PENDING`, :614 `DONE` | Scheduler::Defer's drain at shutdown; each deferred future carries its own `AppState` |
| rate_limiter.rs:37 `POOLS` | already keyed by the database's connect options; one 4-connection pool per site (see 6) |
| readonly.rs:62 `FORCED` | `GlobalSetting.pg_force_readonly_mode`, process-wide in Rails too |
| clock.rs:13 `PINNED`, :14 `SHIFT` | tests only; two-site tests share one clock, which is fine |
| i18n.rs:51 `VALUES` | interned override strings; grows with every site's distinct overrides but never wrong |
| letter_avatar.rs:322 `FONTS`, :407 `LOCK` | letter avatars are not per site in Rails either (tmp/letter_avatars) |
| crates/markdown/src/render/linkify.rs:193 `COMPILED` | keyed by the TLD list |

Per request or per thread: request_tracking.rs:250 `TRACKED`,
mobile.rs:44-45 `MOBILE_VIEW` and `NARROW_DESKTOP` (task-locals),
crates/markdown/src/wasm.rs:14-15 (the browser's wasm build).

Immutable, from vendored data (shared): assets.rs:368, color_scheme.rs:15
and :865, setting_enums.rs:18, site_texts.rs:99, svg_sprite.rs:28,
admin_site_settings.rs:783, html.rs:320, emojis.rs:34, :317 and :382,
routes/emojis.rs:30, routes/user_avatars.rs:32 (the HTTP client),
staff_action_logs.rs:39, api_key_scopes.rs:186, onebox/entities.rs:19,
onebox/matcher.rs:54 and :807, crates/markdown/src/emoji.rs:26 and :178,
and the 253 regexes.

State inside structs `AppState` shares through an `Arc`. One AppState per
site separates these only if each site builds its own; cloning one `Arc`
into every site's state would leak across sites:

| Where | What | Needs |
|---|---|---|
| site_settings.rs:331 `Definitions.cache` | the resolved settings and their fingerprint (`max(updated_at)`/count of site_settings and custom_emojis, :889) | per site. Two sites with equal fingerprints (two fresh sites: no rows, no emoji) would be served each other's resolution. Split the cache out of the parsed definitions so the definitions stay shared |
| i18n.rs:37 `I18n.overrides` | TranslationOverride rows and their fingerprint, refreshed per request (lib.rs `translation_overrides`) | per site; the vendored strings (several MB) stay shared |
| session/current.rs:33 `Keys.last_seen_gate` | `should_update_last_seen?`'s gate, keyed by (user id, day) | per site: user 5 on one site gates user 5 on another |
| session/cookie.rs:47-56 `derive_key` | the cookie key, from a fixed salt `authenticated encrypted cookie` | per request host: Rails appends `" #{host}"` to the salt under multisite. Precompute one key per host name (PBKDF2 at 1000 rounds is too slow per request) |
| rate_limiter.rs:65 `MemoryStore` | the request tracker's lists | split: `GLOBAL::` keys (the per-IP and health check limiters, request_tracker.rs:211) process-wide, the rest (per-user and crawler limits, request_tracker.rs:262) per site |
| anonymous_cache.rs:86 `store` | anonymous responses | per site (its key already holds the Host header, anonymous_cache.rs:169-173) |
| search/mod.rs:193 `SearchLogCache` | SearchLog's per-IP dedupe | per site |

## 2. URLs, hostnames, paths and keys from global config

The `default` db hard-coded (each becomes the site's db key):

| Where | What |
|---|---|
| file_store.rs:15 `CURRENT_DB` (used :92, :117, :132, :162) | upload URLs and paths, optimized images (`path_for_kind`, :189) included |
| upload_references.rs:21 `CURRENT_DB` (:188) | which `src` is a local upload |
| backup_restore/store.rs:13 `CURRENT_DB` (:46) | `public/backups/default` |
| backup_restore/creator.rs:240, :295, :394, :519 | `tmp/backups/default/<ts>`, the uploads to archive, `meta.json`'s `db_name`, the archive's `uploads/default` |
| backup_restore/restorer.rs:276, :851, :924, :952 | `tmp/restores/default`, the uploads' target, the URL remap from the backup's db name |
| reports/pageviews.rs:505, :523 | the storage report's backups and uploads directories, spelled `"default"` without the constant |

The hostname from `GlobalSettings::hostname` (config.rs:119,
`DISCOURSE_HOSTNAME`) instead of the site:

| Where | What |
|---|---|
| url.rs:50-58 `Urls::current_hostname` | the funnel: about 45 call sites (base URLs, Message-IDs, onebox, avatars, emails, cooked post processing) read the hostname through it, so a per-site Config fixes them all |
| jobs/rollups.rs:172-177 | browser pageview rollups' own-host filter; Rails uses every host name of the site plus `current_hostname` (browser_pageview_entry_url_daily_rollup.rb:81) |
| problem_checks.rs:345 | the `host_names` check; also skips `force_hostname`, which Rails' `Discourse.current_hostname` honours |
| reports/pageviews.rs:432 | top referrers' own host; same `force_hostname` gap |

Hostnames the request names, which the dispatcher must honour:

| Where | What |
|---|---|
| routes/highlight_js.rs:12-16 | `/highlight-js/:hostname/` serves "whatever the hostname" today; under multisite the path's hostname picks the site |
| routes/user_avatars.rs:134-136 | `/user_avatar/:hostname/` likewise |
| theme_writes.rs:1035 | theme JavaScript URLs carry `?__ws=<hostname>`; the dispatcher honours `__ws` on asset hostnames |

Keys and names that follow the site once each site has its own database
(no change): the `discourse_rs` schema's tables (owned_schema.rs:10-61:
jobs, server sessions, expiring keys, cached values, badge queue,
presence, rate limits), pg-bus's `discourse_rs_bus` schema and NOTIFY
channel (bus.rs:21; LISTEN/NOTIFY channels are per database), the
read-only keys (readonly.rs), the backup running and shutdown flags
(backup_restore/mod.rs:22-23), Sidekiq's pause key (jobs/mod.rs:721),
`last_job_perform_at` (jobs/mod.rs:214), the anonymous cache key,
Message-IDs and outbound message ids (email/sender.rs:143, :254-289, from
the site's base URL), and mail From (the site's `notification_email`).
This rests on one rule multisite.yml must enforce: no two sites share a
database.

Shared, correct as process-wide (GlobalSetting in Rails too): `cdn_url`
and `s3_cdn_url` (url.rs:102, :125, :132-136; the path carries the db),
`relative_url_root`, the SMTP mailer (email/mod.rs:104), the letter avatar
cache (letter_avatar.rs:346-349) and proxy cache
(routes/user_avatars.rs:68), `/uploads` served for every db
(routes/mod.rs:2273, nginx's role), `/images` (routes/mod.rs:145-150), the
cookie names `_t` and `_forum_session` (session/current.rs:24-26; host-only
cookies, no Domain), the OS `HOSTNAME` for the health check limiter
(request_tracker.rs:323).

## 3. Config

Config::from_vars (config.rs:154-216) and main.rs read:

Per site under multisite.yml:

- `DATABASE_URL` (config.rs:158): the default site's; other sites get
  `database`, `host`, `port`, `username`, `password` from their entry.
- `DISCOURSE_HOSTNAME` (`globals.hostname`, config.rs:119): the default
  site's; other sites' `host_names` (the first is `current_hostname`, all
  route).
- `DISCOURSE_DB_POOL` (main.rs:36): the default; a site's `pool` overrides.
- New: the site's db key (`current_db`), from the YAML key.

Process-wide: `BIND_ADDR`, `RAILS_ENV`, `UNICORN_PORT`, `PUBLIC_DIR`
(uploads and backups under it get the db key, as Rails' public/ does),
`DISCOURSE_SRC`, `TMP_DIR`, `LETTER_AVATAR_CDN`, `LETTER_AVATAR_FONT`,
`DISCOURSE_RS_RATE_LIMITS`, `TRACK_REQUESTS`, `DISCOURSE_RS_JOBS`,
`RUST_LOG`, and every other `DISCOURSE_*` global: `secret_key_base` (one
secret; the cookie salt carries the host), `cdn_url`, `s3_*`, `smtp_*`,
`relative_url_root`, `max_reqs_*`, `pg_force_readonly_mode`,
`developer_emails`, `asset_url_salt`, `allowed_theme_repos`, `maxmind_*`,
`enable_performance_http_headers`, `cluster_name`, and the globals that
shadow site settings (they shadow on every site, as in Rails). New:
`DISCOURSE_MULTISITE_CONFIG_PATH`.

## 4. Jobs

The queue is `discourse_rs.jobs` in the site's own database
(owned_schema.rs:11), so a job is a site's by where it is stored: no
payload needs a site id, and none crosses sites. `jobs::work`
(jobs/mod.rs:732) takes one `AppState`: it checks the pause key, drains
due jobs with `FOR UPDATE SKIP LOCKED` (:132), then runs the 88
`SCHEDULED` jobs (:228) when due, their last run kept in the worker's
memory (:734). One worker per site is all the fan-out needed; it matches
Rails running a scheduled job once per db.

What multiplies: each worker polls once a second (a claim and a pause
check, 2 queries per site per second idle), and every scheduled job runs
when its worker starts, so N sites run 88 jobs each at boot. Stagger the
first runs per site. `DISCOURSE_RS_JOBS=off` (main.rs:112) "leaves the
queue to a separate process", but no such process or subcommand exists;
it stays a per-process switch. `stamp_performed` (jobs/mod.rs:214) is per
site where Rails keeps one process-wide value; harmless.

Deferred work and counters spawned from requests carry the site's state
(request_tracking.rs:618 `later`), except the three statics in section 1.

## 5. Other single-site assumptions

main.rs builds one site: schema verify (:48), owned schema migrate (:62),
pg-bus (:63), secret and mailer (:70-80), `MemoryStore::load` (:92), the
site icons' optimized copies at boot (:94-108, SiteIconManager), the
worker (:110-120), and at shutdown the rate limit lists saved, deferred
work drained and counts flushed into one pool (:130-137). Each becomes a
per-site step; a site that fails to start is either fatal or skipped with
an error (Rails' `safe_each_connection` keeps going).

Backups and restore: section 2's paths; `database_args`
(backup_restore/creator.rs:142) already takes the site's pool, and the
restorer's remap already handles a backup from another db name.

scripts/restore-backup restores one backup into `<dest>` and prints
`PUBLIC_DIR=$dest`: a public directory per database. Under multisite the
sites share PUBLIC_DIR, so the script takes the site's db key, puts the
backup's `uploads/<old db>` at `public/uploads/<site>` and rewrites
`/uploads/<old db>/` in the dump as Rails' restorer does.

deploy/: env.example names one `DATABASE_URL` and `DISCOURSE_HOSTNAME`;
the Caddyfile has one host block; the unit says the server "writes no
files" though uploads, backups and TMP_DIR are written (already true; a
multisite.yml path adds a file to read); install's closing hint names the
single-site variables.

No MCP server and no CLI subcommands in the server binary; src/bin/parity.rs
compares one site per run, which stays. tests/common/mod.rs:211-221 builds
one AppState over one cloned database; two-site tests build two.

## 6. Connection budget

Per site:

- the pool: `DISCOURSE_DB_POOL`, default 10 (main.rs:36-46), no
  `idle_timeout` set, so sqlx's 10 minutes: an idle site holds what it
  used for 10 minutes;
- the rate limiter's own pool: 4, lazy, 30 s idle (rate_limiter.rs:32-60);
- pg-bus's LISTEN connection: 1, outside the pool (`PgListener`), always
  held; behind pgbouncer each site needs its own `listen_url`;
- backups: pg_dump or psql, 1 while running.

So up to 15 per site at load, 1 when idle once the timeouts pass; at
Postgres' default `max_connections` of 100, six busy sites exhaust it.
Rails' equivalent is one Active Record pool per site per process
(multisite.yml `pool`). Plan: a site's `pool` from multisite.yml, a short
`idle_timeout` on every site's pool, and a startup warning when the sum of
`pool + 5` over the sites exceeds `SHOW max_connections`.

One more shared limit: NOTIFY commits take a cluster-wide lock (pg-bus's
own docs), so every site on one Postgres shares publish throughput.

## Unsure

- `CALLS` (jobs/periodical.rs:15): Rails' `@call_count` is per process
  and bumps once per db, so with N sites "every 24th run" lands on
  rotating sites. Per site is proposed; porting the quirk is possible.
- `last_job_perform_at`: process-wide in Rails, per site here; the
  sidekiq_check problem check reads it per site either way.
- `GLOBAL::` rate limit lists: `MemoryStore::save` writes them but
  `load` (rate_limiter.rs:77-78, `MEMORY_KEYS`) only matches keys that
  start `l-rate-limit3:`, so per-IP lists are not restored at restart
  today. Where the process-wide store is saved under multisite (the
  default site's database) is a choice 208a makes.
- Two host names of one site get different cookie keys (the salt takes
  the request's host); that is Rails' behaviour, kept.
