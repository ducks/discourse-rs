# Bench

`make bench TARGETS="rs=URL rails=URL"` compares request rates and
latencies per endpoint (scripts/bench, oha); `make bench-startup` measures
the release binary's size, cold start and RSS (scripts/bench-startup).
Runs are dated below, newest first. Read the caveats before the numbers.

Caveats that apply to every run:

- The reference must be a production-mode Discourse serving the same
  backup. A `dv` development agent runs with code reloading, mini-profiler
  and unprecompiled assets, and is several times slower than production on
  the same machine; rows labelled `rails-dev` are that, and only show the
  order of magnitude.
- Rails caches anonymous GETs in redis for 60 s (AnonymousCache). Each
  Rails endpoint is measured twice: with the cache (`cached`) and with the
  `_bypass_cache` cookie (`uncached`). In development the two agree, which
  suggests the cache is not active there.
- Rails rate-limits anonymous search at 2/s per IP: the `/search.json`
  rows for Rails count 429s in non-2xx and measure the limiter.
- HTML rows compare different products: discourse-rs renders the page,
  Rails serves the Ember shell (the topics arrive by XHR afterwards).
- Same box, same Postgres, client and servers all local; keep-alive off so
  each request carries a connection.

## 2026-10-09, fixing what the run above found

Same box, seed and c=2, discourse-rs only, release builds at each step.

Logged-in /srv/status regression: on the same seed database the
2026-10-01 build (5e1cd1a) serves 4970 req/s logged in and the build
before the fix 3850, with the same four queries per request. perf under
load showed half the CPU in malloc and free from SiteSettings::load
cloning the whole resolved settings map on every cache hit. The map now
sits behind an Arc (perf/logged-in-floor):

| endpoint, logged in (median of 3) | 2026-10-01 build | before | after |
|---|---:|---:|---:|
| /srv/status | 4990 | 3899 | 6550 |
| /latest.json | 963 | 680 | 963 |

Skipping the connection the session layer took after each logged-in
response to look for notifications to clear measured within noise.

Caching and compression (perf/asset-caching, perf/compression), Chromium,
median of five, the page and everything it loads:

| visit | before | after |
|---|---:|---:|
| /latest, cold cache: document | 75 KB | 18 KB |
| /latest, cold cache: total | 772 KB | 497 KB |
| a topic after /latest, same session: total | about 280 KB | 27 KB |

What is left of a cold visit is mostly the Inter font (344 KB woff2,
cached for good once fetched). The anonymous /srv/status floor sits
around 18000 req/s against about 20000 for the 2026-10-01 build; not
chased further.

## 2026-10-09, after topic voting

Release binary at ac53196, rustc 1.99, the seed fixtures
(seed/fresh_install.sql) loaded fresh into discourse_rs_screens; Rails is
the `rs-parity` dv agent (development mode, Discourse bf55a44) on the same
seed. Same laptop (16 cores, powersave governor), c=2, 5 s, keep-alive off.
The data set is the parity fixtures, not the Faker backup of the earlier
runs, so absolute numbers aren't comparable with those.

Process:

| measure | value | 2026-09-30 |
|---|---:|---:|
| binary size | 53 MB | 14 MB |
| exec to first 200 (median of 3) | 75 ms | 40 ms |
| RSS idle | 32 MB | 17 MB |
| RSS after 100 requests | 44 MB | 26 MB |

The binary is 27 MB of code and 12 MB of read-only data (the vendored
schema, locales, settings, emoji and icon sprites it embeds), plus about
9 MB of symbols; the startup and RSS growth follow from what it loads.

Build and tests (warm unless said):

| measure | value |
|---|---:|
| `cargo build --release`, clean | 1m 58s |
| `cargo build --release`, one-file change | 70s |
| `cargo build` (dev), one-file change | 5s |
| `make clippy` | 4s |
| `make test` (330 tests) | 100s |
| `make test` after `cargo clean` | 3m 17s |
| `make parity-test` (486 goldens and write cases) | 3m 45s |

Requests, anonymous:

| endpoint | target | req/s | p50 ms | p99 ms | non-2xx |
|---|---|---:|---:|---:|---:|
| /srv/status | rs | 19179 | 0.1 | 0.2 | 0 |
| /srv/status | rails-dev (cached) | 1240 | 1.4 | 3.7 | 0 |
| /srv/status | rails-dev (uncached) | 1314 | 1.4 | 2.7 | 0 |
| /site.json | rs | 769 | 2.5 | 3.9 | 0 |
| /site.json | rails-dev (cached) | 271 | 4.9 | 17 | 0 |
| /site.json | rails-dev (uncached) | 260 | 5.2 | 15.7 | 0 |
| /latest.json | rs | 849 | 2.3 | 4 | 0 |
| /latest.json | rails-dev (cached) | 33 | 57.2 | 168 | 0 |
| /latest.json | rails-dev (uncached) | 33 | 58.2 | 129.4 | 0 |
| /latest | rs | 422 | 4.7 | 7.3 | 0 |
| /latest | rails-dev (cached) | 17 | 109 | 390.9 | 0 |
| /latest | rails-dev (uncached) | 18 | 110 | 247.6 | 0 |
| /c/general/4.json | rs | 568 | 3.5 | 5.5 | 0 |
| /c/general/4.json | rails-dev (cached) | 28 | 69.1 | 89 | 0 |
| /c/general/4.json | rails-dev (uncached) | 31 | 63.7 | 89.6 | 0 |
| /categories.json | rs | 754 | 2.6 | 4.1 | 0 |
| /categories.json | rails-dev (cached) | 44 | 43.8 | 99 | 0 |
| /categories.json | rails-dev (uncached) | 45 | 43.1 | 103.3 | 0 |
| /t/welcome-to-discourse/5.json | rs | 399 | 4.9 | 7.5 | 0 |
| /t/welcome-to-discourse/5.json | rails-dev (cached) | 17 | 111.6 | 161.8 | 0 |
| /t/welcome-to-discourse/5.json | rails-dev (uncached) | 17 | 110 | 156.7 | 0 |
| /t/welcome-to-discourse/5 | rs | 306 | 6.4 | 10.7 | 0 |
| /t/welcome-to-discourse/5 | rails-dev (cached) | 10 | 178.8 | 351.8 | 0 |
| /t/welcome-to-discourse/5 | rails-dev (uncached) | 11 | 178.8 | 211.3 | 0 |
| /u/system.json | rs | 1514 | 1.3 | 2.7 | 0 |
| /u/system.json | rails-dev (cached) | 44 | 43.8 | 109.2 | 0 |
| /u/system.json | rails-dev (uncached) | 48 | 41.2 | 91.1 | 0 |
| /search.json?q=Welcome | rs | 638 | 3.1 | 4.6 | 0 |
| /search.json?q=Welcome | rails-dev (cached) | 259 | 6.9 | 28.5 | 1284 |
| /search.json?q=Welcome | rails-dev (uncached) | 288 | 6.4 | 18.4 | 1436 |
| /tag/guide.json | rs | 709 | 2.8 | 4.3 | 0 |
| /tag/guide.json | rails-dev (cached) | 37 | 52.1 | 118.1 | 0 |
| /tag/guide.json | rails-dev (uncached) | 33 | 57.3 | 139.3 | 0 |

Requests, logged in as user1:

| endpoint | target | req/s | p50 ms | p99 ms | non-2xx |
|---|---|---:|---:|---:|---:|
| /srv/status | rs (user) | 3408 | 0.6 | 1 | 0 |
| /srv/status | rails-dev (user) | 1049 | 1.7 | 4.5 | 0 |
| /site.json | rs (user) | 558 | 3.5 | 6 | 0 |
| /site.json | rails-dev (user) | 39 | 49.2 | 117.7 | 0 |
| /latest.json | rs (user) | 722 | 2.6 | 5.7 | 0 |
| /latest.json | rails-dev (user) | 25 | 79.2 | 99.4 | 0 |
| /latest | rs (user) | 305 | 6.3 | 11.4 | 0 |
| /latest | rails-dev (user) | 14 | 139.8 | 263.8 | 0 |
| /c/general/4.json | rs (user) | 546 | 3.5 | 7.5 | 0 |
| /c/general/4.json | rails-dev (user) | 22 | 88.6 | 145 | 0 |
| /categories.json | rs (user) | 653 | 3 | 4.7 | 0 |
| /categories.json | rails-dev (user) | 34 | 59.5 | 70.8 | 0 |
| /t/welcome-to-discourse/5.json | rs (user) | 293 | 6.5 | 12.6 | 0 |
| /t/welcome-to-discourse/5.json | rails-dev (user) | 12 | 165.9 | 187.4 | 0 |
| /t/welcome-to-discourse/5 | rs (user) | 184 | 10.4 | 19 | 0 |
| /t/welcome-to-discourse/5 | rails-dev (user) | 7 | 251.5 | 407.2 | 0 |
| /u/system.json | rs (user) | 1004 | 1.9 | 3.9 | 0 |
| /u/system.json | rails-dev (user) | 28 | 71.6 | 85.5 | 0 |
| /search.json?q=Welcome | rs (user) | 554 | 3.5 | 5.5 | 0 |
| /search.json?q=Welcome | rails-dev (user) | 137 | 12.7 | 52 | 654 |
| /tag/guide.json | rs (user) | 546 | 3.5 | 8.2 | 0 |
| /tag/guide.json | rails-dev (user) | 25 | 79.3 | 98.3 | 0 |
| /unread.json | rs (user) | 716 | 2.7 | 4.6 | 0 |
| /unread.json | rails-dev (user) | 31 | 64.3 | 74.3 | 0 |
| /new.json | rs (user) | 686 | 2.8 | 5.2 | 0 |
| /new.json | rails-dev (user) | 29 | 66.8 | 103.8 | 0 |

The HTML pages no longer sit at ~1 req/s (the anomaly noted in earlier
sessions): /latest serves 422 req/s anonymous, topics 306. Logged-in
/srv/status is 3408 req/s against 4817 on 2026-10-01 while the anonymous
floor held (19179 against 19447); worth a look.

Payloads (bytes, neither side compresses: Rails dev has no nginx in
front, discourse-rs has no compression layer):

| path | rs | rails-dev |
|---|---:|---:|
| /site.json | 15660 | 18521 |
| /latest.json | 6294 | 6294 |
| /categories.json | 6461 | 7712 |
| /t/parity-fixture-replies-and-posters/35.json | 11261 | 12185 |
| /u/user1.json | 2952 | 3014 |
| /search.json?q=fixture | 4067 | 4068 |

JSON sizes differ where the reference's unported plugins add keys
(chat, reactions, ...). rs's HTML pages are 75 to 86 KB, 42 KB of it the
inline icon sprite; gzip -9 takes them to about 19 KB.

Page loads in Chromium, median of five after a warm-up, a fresh context
each (cold cache); "content" is when the list or first post is in the
DOM, in ms from navigation start:

| page | target | requests | document | all resources | DOMContentLoaded | content |
|---|---|---:|---:|---:|---:|---:|
| /latest | rs | 18 | 73 KB | 749 KB | 46 | 62 |
| /c/general/4 | rs | 18 | 73 KB | 749 KB | 42 | 59 |
| /t/parity-fixture-replies-and-posters/35 | rs | 18 | 84 KB | 711 KB | 62 | 72 |
| /categories | rs | 17 | 74 KB | 751 KB | 47 | 65 |
| /u/user1/summary | rs | 13 | 69 KB | 681 KB | 46 | 67 |
| /latest as=user1 | rs | 21 | 88 KB | 808 KB | 64 | 79 |
| /t/parity-fixture-replies-and-posters/35 as=user1 | rs | 22 | 122 KB | 800 KB | 97 | 106 |
| /latest | rails-dev | 104 | 109 KB | 15449 KB | 608 | 1097 |
| /c/general/4 | rails-dev | 104 | 109 KB | 15463 KB | 549 | 843 |
| /t/parity-fixture-replies-and-posters/35 | rails-dev | 104 | 115 KB | 15525 KB | 614 | 988 |
| /categories | rails-dev | 104 | 110 KB | 15536 KB | 575 | 876 |
| /u/user1/summary | rails-dev | 103 | 92 KB | 15384 KB | 488 | 1105 |
| /latest as=user1 | rails-dev | 107 | 103 KB | 15700 KB | 594 | 916 |
| /t/parity-fixture-replies-and-posters/35 as=user1 | rails-dev | 102 | 115 KB | 15521 KB | 565 | 933 |

Rails dev serves its JavaScript unbundled and unminified (about 15 MB a
page), so its column shows the order of magnitude only. rs's 750 KB is
mostly the Inter font (344 KB, cached immutable) and discourse.css
(193 KB).

Found while measuring: /assets/*.css and *.js carry no Cache-Control,
ETag or Last-Modified, so every page load fetches them again (about
270 KB); and responses aren't compressed. Both are cheap to fix and
matter more than any request-rate row above for a real visitor.

## 2026-09-30, first run

Framework 16 laptop, local Postgres 16, the Faker backup (482 posts, 58
topics). discourse-rs `target/release` at 0015a8d; Rails is the `rs-backup`
dv agent (development mode, Discourse bf55a44) on the same data.

Duration 5s, concurrency 8, keepalive off, 2026-09-30T19:55Z

| endpoint | target | req/s | p50 ms | p99 ms | non-2xx |

| measure | value |
|---|---:|
| binary size | 14 MB (15071288 bytes) |
| exec to first 200 | 40 ms |
| RSS idle | 17 MB |
| RSS after 100 requests | 26 MB |

| endpoint | target | req/s | p50 ms | p99 ms | non-2xx |
|---|---|---:|---:|---:|---:|
| /srv/status | rs | 7125 | 1 | 3.6 | 0 |
| /srv/status | rails-dev (cached) | 384 | 20.3 | 32.1 | 0 |
| /srv/status | rails-dev (uncached) | 388 | 20.3 | 29 | 0 |
| /site.json | rs | 176 | 42.7 | 75.5 | 0 |
| /site.json | rails-dev (cached) | 86 | 85.6 | 178.9 | 0 |
| /site.json | rails-dev (uncached) | 100 | 72.6 | 142.3 | 0 |
| /latest.json | rs | 92 | 86.5 | 121.3 | 0 |
| /latest.json | rails-dev (cached) | 9 | 1017.3 | 1325 | 0 |
| /latest.json | rails-dev (uncached) | 8 | 1049.4 | 1824.9 | 0 |
| /latest | rs | 70 | 107 | 249.8 | 0 |
| /latest | rails-dev (cached) | 5 | 1635 | 2292 | 0 |
| /latest | rails-dev (uncached) | 3 | 2537.4 | 3913.8 | 0 |
| /c/general/4.json | rs | 154 | 48.1 | 101 | 0 |
| /c/general/4.json | rails-dev (cached) | 13 | 672.8 | 884 | 0 |
| /c/general/4.json | rails-dev (uncached) | 11 | 707 | 1217.1 | 0 |
| /categories.json | rs | 365 | 20.5 | 40.3 | 0 |
| /categories.json | rails-dev (cached) | 11 | 799.6 | 1072.3 | 0 |
| /categories.json | rails-dev (uncached) | 10 | 814.7 | 1449.6 | 0 |
| /t/welcome-to-discourse/5.json | rs | 167 | 43.5 | 111.8 | 0 |
| /t/welcome-to-discourse/5.json | rails-dev (cached) | 8 | 1186.5 | 1420.8 | 0 |
| /t/welcome-to-discourse/5.json | rails-dev (uncached) | 6 | 1411 | 2536.1 | 0 |
| /t/welcome-to-discourse/5 | rs | 154 | 46.9 | 95.1 | 0 |
| /t/welcome-to-discourse/5 | rails-dev (cached) | 5 | 1640.4 | 2370.1 | 0 |
| /t/welcome-to-discourse/5 | rails-dev (uncached) | 3 | 2365.9 | 3777.2 | 0 |
| /u/system.json | rs | 576 | 13.4 | 38.7 | 0 |
| /u/system.json | rails-dev (cached) | 19 | 416.6 | 602.5 | 0 |
| /u/system.json | rails-dev (uncached) | 17 | 440.3 | 903.7 | 0 |
| /search.json?q=Welcome | rs | 253 | 28.6 | 72.5 | 0 |
| /search.json?q=Welcome | rails-dev (cached) | 98 | 70.3 | 273.1 | 473 |
| /search.json?q=Welcome | rails-dev (uncached) | 118 | 66.2 | 124.7 | 580 |
| /tag/design.json | rs | 199 | 36.7 | 88.4 | 0 |
| /tag/design.json | rails-dev (cached) | 12 | 669.4 | 958.1 | 0 |
| /tag/design.json | rails-dev (uncached) | 11 | 683.6 | 1405.3 | 0 |


The non-2xx column of that run counted the requests oha aborts at the
deadline (8 per row); corrected above. The 429s on Rails' search rows are
the rate limiter.

What the numbers say about discourse-rs itself: /srv/status shows the
server's floor (10k req/s), while /latest.json at ~90 req/s and 85 ms p50
under 8 connections is the serializer's per-topic queries and the
per-request site settings load, not the framework. Those are the first
things to profile.

## 2026-09-30, investigating the /latest.json curve

The first run's 92 req/s at 85 ms p50 was measured with 8 connections;
measuring the scaling curve gave 137 req/s at 7 ms with one connection,
231 at two, and then a fall: 94 at eight and nothing completing at
sixteen.

Two causes, one ours and one the machine's:

- The `login_required` gate acquired a pool connection to read the
  setting and held it while the handler ran, so every request occupied
  two of the pool's ten connections; at sixteen clients they starved each
  other until sqlx's 30 s acquire timeout. Fixed: the gate returns its
  connection before calling the handler. Also removed one of the two
  site-settings loads per request.
- The laptop clamps its clocks under multi-core load: 4.8 GHz with one
  busy core, ~920 MHz on every core with eight (amd-pstate `powersave`,
  platform profile `balanced`), with a third of the time in the kernel at
  that clock. Postgres alone shows the same curve (`pgbench` on
  `SELECT 1`: 53k tps at one client, 102k at two, 46k at eight). CPU per
  request grew tenfold on both sides between c=1 and c=8 for the same
  work. Numbers above two connections on this machine measure the power
  manager, not the port; run the harness with the `performance` governor
  or on a machine that holds its clocks.

What is ours to fix, measured at one connection where the clocks hold:
a /latest.json request is 99 queries (30 topics times tags, thumbnail
and first-post like count, plus the page query, users, groups, tags and
settings) in about 5 ms, 4.7 ms of it in the serializer's per-topic
round-trips. Batching those three per-topic lookups into one query each
would take most of that out; it is the roadmap's performance item.

## 2026-10-01, after batching the list serializer's per-topic queries

TopicListSerializer now prefetches the visible tags, the share thumbnail
or image upload, and the first post's like count for the whole page in
one query each (`prefetch`); the per-topic methods read from that and
only query on their own for topics outside the page (topic view's
suggested topics, search, profiles). /latest.json went from 99 queries
to 11, and at one connection from 137 req/s (7 ms p50) to 332 req/s
(2.8 ms).

Harness run at two connections, the range where this laptop keeps its
clocks (see the investigation above). Same setup as the first run:
release binary at the merge of perf/batch-topic-queries, the rs-backup dev
agent on the same data.

Duration 5s, concurrency 2, keepalive off, 2026-10-01T06:25Z

| endpoint | target | req/s | p50 ms | p99 ms | non-2xx |
|---|---|---:|---:|---:|---:|
| /srv/status | rs | 19778 | 0.1 | 0.2 | 0 |
| /srv/status | rails-dev (cached) | 1025 | 1.8 | 3.8 | 0 |
| /srv/status | rails-dev (uncached) | 1040 | 1.8 | 3.7 | 0 |
| /site.json | rs | 442 | 4.4 | 6.8 | 0 |
| /site.json | rails-dev (cached) | 186 | 7.7 | 23.6 | 0 |
| /site.json | rails-dev (uncached) | 204 | 7.2 | 21.1 | 0 |
| /latest.json | rs | 595 | 3.3 | 5.5 | 0 |
| /latest.json | rails-dev (cached) | 17 | 112.7 | 247.3 | 0 |
| /latest.json | rails-dev (uncached) | 16 | 115 | 286.1 | 0 |
| /latest | rs | 575 | 3.3 | 6 | 0 |
| /latest | rails-dev (cached) | 9 | 188.2 | 367.7 | 0 |
| /latest | rails-dev (uncached) | 9 | 196.5 | 456.2 | 0 |
| /c/general/4.json | rs | 659 | 3 | 4.8 | 0 |
| /c/general/4.json | rails-dev (cached) | 23 | 85.6 | 117.2 | 0 |
| /c/general/4.json | rails-dev (uncached) | 27 | 73.7 | 92.2 | 0 |
| /categories.json | rs | 768 | 2.5 | 4.1 | 0 |
| /categories.json | rails-dev (cached) | 22 | 85.2 | 165.3 | 0 |
| /categories.json | rails-dev (uncached) | 24 | 80.7 | 108.1 | 0 |
| /t/welcome-to-discourse/5.json | rs | 489 | 4 | 6.2 | 0 |
| /t/welcome-to-discourse/5.json | rails-dev (cached) | 14 | 136.7 | 167 | 0 |
| /t/welcome-to-discourse/5.json | rails-dev (uncached) | 13 | 147.9 | 269.3 | 0 |
| /t/welcome-to-discourse/5 | rs | 392 | 5 | 7.3 | 0 |
| /t/welcome-to-discourse/5 | rails-dev (cached) | 8 | 224.5 | 423.2 | 0 |
| /t/welcome-to-discourse/5 | rails-dev (uncached) | 9 | 225.1 | 314.5 | 0 |
| /u/system.json | rs | 1654 | 1.2 | 2 | 0 |
| /u/system.json | rails-dev (cached) | 40 | 48.9 | 75.4 | 0 |
| /u/system.json | rails-dev (uncached) | 43 | 46.6 | 61.7 | 0 |
| /search.json?q=Welcome | rs | 568 | 3.4 | 6.1 | 0 |
| /search.json?q=Welcome | rails-dev (cached) | 273 | 6.7 | 12.9 | 1358 |
| /search.json?q=Welcome | rails-dev (uncached) | 283 | 6.8 | 11.2 | 1414 |
| /tag/design.json | rs | 542 | 3.6 | 6.3 | 0 |
| /tag/design.json | rails-dev (cached) | 26 | 75.4 | 94 | 0 |
| /tag/design.json | rails-dev (uncached) | 27 | 71.8 | 101 | 0 |


The Rails column also moved up from the first run (its /latest.json 8
to 17 req/s): that is the clock clamp lifting at two connections, not a
change on the Rails side. The topic page (/t/...) is the next per-post
candidate, at 489 req/s against the lists' 600-770.

## 2026-10-01, logged in

The first logged-in run, with sessions slice 2 on `feat/logged-in-reads`
(topic_users joins, muting, the guardian's secure categories and can_*
predicates, the viewer block in the HTML shell). `scripts/bench -u
user1:password` logs in once per target and sends the `_t` cookie, so
Rails' anonymous cache is out of the picture and both sides resolve the
session on every request; `/unread.json` and `/new.json` join the set.
Same box and data as above, release binary at c6845d3, c=2.

Session resolution is the new fixed cost: `/srv/status` fell from
19778 req/s anonymous to 2709 logged in, which is the token lookup, the
user row, the group memberships and the silenced check (three or four
queries) per request. Rails pays the same shape (1040 -> 1121, it was
already paying it). Caching the token-to-user step for a few seconds is
the obvious next win; nothing else in the table moved by more than the
per-user work itself (the `tu` join and the per-topic read-state keys
cost `/latest.json` 595 -> 428 req/s).

Duration 5s, concurrency 2, keepalive off, logged in as user1, 2026-10-01T15:04Z

| endpoint | target | req/s | p50 ms | p99 ms | non-2xx |
|---|---|---:|---:|---:|---:|
| /srv/status | rs (user) | 2709 | 0.7 | 1.5 | 0 |
| /srv/status | rails-dev (user) | 1121 | 1.6 | 4 | 0 |
| /site.json | rs (user) | 366 | 5.3 | 8 | 0 |
| /site.json | rails-dev (user) | 37 | 51.8 | 132.1 | 0 |
| /latest.json | rs (user) | 428 | 4.5 | 8 | 0 |
| /latest.json | rails-dev (user) | 13 | 143 | 204 | 0 |
| /latest | rs (user) | 399 | 4.9 | 7.7 | 0 |
| /latest | rails-dev (user) | 8 | 221.4 | 430 | 0 |
| /c/general/4.json | rs (user) | 480 | 4 | 9.2 | 0 |
| /c/general/4.json | rails-dev (user) | 18 | 107.5 | 189.7 | 0 |
| /categories.json | rs (user) | 510 | 3.8 | 6.4 | 0 |
| /categories.json | rails-dev (user) | 17 | 115.3 | 142.4 | 0 |
| /t/welcome-to-discourse/5.json | rs (user) | 314 | 6.1 | 9.7 | 0 |
| /t/welcome-to-discourse/5.json | rails-dev (user) | 10 | 194.9 | 389.9 | 0 |
| /t/welcome-to-discourse/5 | rs (user) | 318 | 6.1 | 9.5 | 0 |
| /t/welcome-to-discourse/5 | rails-dev (user) | 7 | 269.9 | 347.9 | 0 |
| /u/system.json | rs (user) | 800 | 2.4 | 4.5 | 0 |
| /u/system.json | rails-dev (user) | 21 | 92.6 | 123.7 | 0 |
| /search.json?q=Welcome | rs (user) | 403 | 4.8 | 7.9 | 0 |
| /search.json?q=Welcome | rails-dev (user) | 113 | 14.5 | 77.5 | 535 |
| /tag/design.json | rs (user) | 474 | 4 | 9.7 | 0 |
| /tag/design.json | rails-dev (user) | 19 | 102.2 | 133.4 | 0 |
| /unread.json | rs (user) | 519 | 3.7 | 6.2 | 0 |
| /unread.json | rails-dev (user) | 20 | 96.4 | 121.1 | 0 |
| /new.json | rs (user) | 246 | 7.9 | 11.2 | 0 |
| /new.json | rails-dev (user) | 14 | 135.1 | 211.7 | 0 |

## 2026-10-01, build and test times

Measured on the same laptop (powersave governor, see the first
investigation), rustc 1.86, sqlx 0.8, 170 tests at the time.

| measure | value |
|---|---:|
| `cargo build --release`, clean | 4m 53s |
| `cargo build --release`, one-file change | 1m 15s |
| `cargo build` (dev), one-file change | 4s |
| `cargo test`, warm cache | 90s |

The test time is dominated by the integration tests cloning the template
database once per test (`CREATE DATABASE ... TEMPLATE`), about 1s each,
and the parity replay (146 cases in-process, ~12s). `make lint` runs
fmt-check, clippy with `-D warnings` and the tests.

## 2026-10-01, after sessions slice 3

Release binary at ebb0d37 (the merge of feat/notifications-messages plus
docs), the rs-backup dev agent on the same data, c=2, after a reboot with
nothing else running. Slice 3 brought two changes that touch every
request: the guardian is built in the session layer with the connection
it already holds (one checkout and two fewer queries per logged-in
request), and resolved site settings are cached behind a table
fingerprint (one small query instead of the table and the resolve, two
or three times per request).

Logged in, against the slice 2 run above: every row but /site.json moved
up, by 2% to 41%. /srv/status 2709 -> 3277 req/s, /u/system.json 800 ->
1129, /unread.json 519 -> 667, /tag/design.json 474 -> 597,
/search.json 403 -> 500, /t/...json 314 -> 367, /latest.json 428 -> 438.
The Rails column stayed where it was (/srv/status 1121 -> 1072), so the
box is comparable between the two runs.

Duration 5s, concurrency 2, keepalive off, logged in as user1, 2026-10-01T21:47Z

| endpoint | target | req/s | p50 ms | p99 ms | non-2xx |
|---|---|---:|---:|---:|---:|
| /srv/status | rs (user) | 3277 | 0.6 | 1.6 | 0 |
| /srv/status | rails-dev (user) | 1072 | 1.6 | 4.3 | 0 |
| /site.json | rs (user) | 364 | 5.4 | 8 | 0 |
| /site.json | rails-dev (user) | 37 | 49.9 | 115.2 | 0 |
| /latest.json | rs (user) | 438 | 4.4 | 9.3 | 0 |
| /latest.json | rails-dev (user) | 12 | 150.8 | 332.2 | 0 |
| /latest | rs (user) | 437 | 4.5 | 6.9 | 0 |
| /latest | rails-dev (user) | 8 | 217.2 | 460.7 | 0 |
| /c/general/4.json | rs (user) | 542 | 3.5 | 7.1 | 0 |
| /c/general/4.json | rails-dev (user) | 16 | 115.7 | 303.9 | 0 |
| /categories.json | rs (user) | 569 | 3.4 | 5.6 | 0 |
| /categories.json | rails-dev (user) | 18 | 103 | 163.3 | 0 |
| /t/welcome-to-discourse/5.json | rs (user) | 367 | 5.3 | 8.6 | 0 |
| /t/welcome-to-discourse/5.json | rails-dev (user) | 10 | 181 | 327.8 | 0 |
| /t/welcome-to-discourse/5 | rs (user) | 348 | 5.6 | 8.3 | 0 |
| /t/welcome-to-discourse/5 | rails-dev (user) | 7 | 254 | 404.3 | 0 |
| /u/system.json | rs (user) | 1129 | 1.7 | 3.3 | 0 |
| /u/system.json | rails-dev (user) | 24 | 82.3 | 166.2 | 0 |
| /search.json?q=Welcome | rs (user) | 500 | 3.9 | 6.2 | 0 |
| /search.json?q=Welcome | rails-dev (user) | 123 | 13.3 | 70.2 | 588 |
| /tag/design.json | rs (user) | 597 | 3.2 | 7 | 0 |
| /tag/design.json | rails-dev (user) | 21 | 92.1 | 115.4 | 0 |
| /unread.json | rs (user) | 667 | 2.9 | 4.6 | 0 |
| /unread.json | rails-dev (user) | 22 | 87.7 | 106.2 | 0 |
| /new.json | rs (user) | 282 | 7 | 9.8 | 0 |
| /new.json | rails-dev (user) | 16 | 119.8 | 202.8 | 0 |

Anonymous, the first such run since the session layer exists (the last
one was the batching run above, before sessions slice 1). The content
endpoints are within the noise of 5 s runs either way, but the floor
moved: /srv/status fell from 19778 to 5255 req/s. That is the session
layer on a request with no cookie: `session::current::layer` checks out
a connection and loads the settings before the handler and again after
it, so two checkouts, two fingerprint queries and two clones of the
resolved settings for a request that has no session to resolve or
rotate. Before the fingerprint cache those were two full table loads.
A request without a `_t` cookie needs neither; skipping both is the next
cheap win, and the logged-in floor (3277) would gain the second one too
if the response side reused the settings the request side loaded.

Duration 5s, concurrency 2, keepalive off, 2026-10-01T21:44Z

| endpoint | target | req/s | p50 ms | p99 ms | non-2xx |
|---|---|---:|---:|---:|---:|
| /srv/status | rs | 5255 | 0.4 | 0.7 | 0 |
| /srv/status | rails-dev (cached) | 1261 | 1.4 | 2.9 | 0 |
| /srv/status | rails-dev (uncached) | 1298 | 1.4 | 2.9 | 0 |
| /site.json | rs | 429 | 4.6 | 6.8 | 0 |
| /site.json | rails-dev (cached) | 223 | 6.1 | 21.3 | 0 |
| /site.json | rails-dev (uncached) | 253 | 5.4 | 17.5 | 0 |
| /latest.json | rs | 660 | 2.9 | 4.9 | 0 |
| /latest.json | rails-dev (cached) | 18 | 104.6 | 248.6 | 0 |
| /latest.json | rails-dev (uncached) | 18 | 108.6 | 259.8 | 0 |
| /latest | rs | 570 | 3.4 | 5.6 | 0 |
| /latest | rails-dev (cached) | 10 | 183.6 | 470.9 | 0 |
| /latest | rails-dev (uncached) | 10 | 184.6 | 402.8 | 0 |
| /c/general/4.json | rs | 616 | 3.2 | 5.2 | 0 |
| /c/general/4.json | rails-dev (cached) | 29 | 67.2 | 124.6 | 0 |
| /c/general/4.json | rails-dev (uncached) | 26 | 74.6 | 111 | 0 |
| /categories.json | rs | 729 | 2.7 | 4.3 | 0 |
| /categories.json | rails-dev (cached) | 23 | 81.6 | 228.1 | 0 |
| /categories.json | rails-dev (uncached) | 24 | 80.8 | 136.7 | 0 |
| /t/welcome-to-discourse/5.json | rs | 430 | 4.5 | 7.4 | 0 |
| /t/welcome-to-discourse/5.json | rails-dev (cached) | 13 | 143.1 | 215.1 | 0 |
| /t/welcome-to-discourse/5.json | rails-dev (uncached) | 13 | 145.4 | 197.4 | 0 |
| /t/welcome-to-discourse/5 | rs | 430 | 4.6 | 6.3 | 0 |
| /t/welcome-to-discourse/5 | rails-dev (cached) | 9 | 212.8 | 307.6 | 0 |
| /t/welcome-to-discourse/5 | rails-dev (uncached) | 10 | 194.8 | 377.7 | 0 |
| /u/system.json | rs | 1490 | 1.3 | 2.5 | 0 |
| /u/system.json | rails-dev (cached) | 48 | 41.2 | 60.1 | 0 |
| /u/system.json | rails-dev (uncached) | 49 | 40.8 | 54.1 | 0 |
| /search.json?q=Welcome | rs | 640 | 3 | 4.7 | 0 |
| /search.json?q=Welcome | rails-dev (cached) | 280 | 6.4 | 21.5 | 1390 |
| /search.json?q=Welcome | rails-dev (uncached) | 303 | 6.3 | 10.8 | 1512 |
| /tag/design.json | rs | 756 | 2.5 | 4.4 | 0 |
| /tag/design.json | rails-dev (cached) | 28 | 70.7 | 88.1 | 0 |
| /tag/design.json | rails-dev (uncached) | 28 | 71.2 | 85.9 | 0 |

## 2026-10-01, the session layer skips requests without a cookie

The fix for the floor found above (perf/cookieless-session): the layer
only checks out a connection and loads the settings when the request
carries `_t`, and the response side reuses those settings instead of
loading them again, taking a second connection only for a live session
(rotation, last-seen). discourse-rs alone, same box and data, c=2, 5 s.

| endpoint | before | after |
|---|---:|---:|
| /srv/status, anonymous | 5255 | 19447 |
| /u/system.json, anonymous | 1490 | 1960 |
| /categories.json, anonymous | 729 | 838 |
| /c/general/4.json, anonymous | 616 | 752 |
| /latest.json, anonymous | 660 | 715 |
| /srv/status, logged in | 3277 | 4817 |

The anonymous floor is back where it was before sessions (19778). The
other logged-in rows and the topic page did not move beyond the noise:
their cost is the handler's own queries. The login gate still loads the
settings once per non-exempt request, which is what is left of the
per-request overhead for anonymous content.
