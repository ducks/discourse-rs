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
