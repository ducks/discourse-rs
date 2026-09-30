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
