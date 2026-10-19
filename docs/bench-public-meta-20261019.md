# Public corpus baseline on PostgreSQL 18, October 19, 2026

This is a local anonymous endpoint benchmark on the public Meta sample. It is a
larger and more varied fixture than the parity seed, but still a small database:
85 topics, 572 posts, and 180 users in total. Of those, 46 topics, 524 posts, and
173 fictional author accounts came from the anonymous public corpus import.
Sampled threads contain at most 20 posts; this does not test very long threads.

## Setup

- Rust: `ef9eec98`, release build with rustc 1.99.0, production mode, SQLx pool 10.
- Rails: `bf55a44c2872738f7ae2664d6fec3d6d8779190f`, Ruby 3.4, production mode,
  eight Pitchfork workers, database pool 8 per worker, precompiled assets.
- Both databases were restored separately from the same SQL snapshot and analyzed.
  Jobs were disabled. The browsing demo on port 8080 was not benchmarked or changed.
- AMD Ryzen 9 7940HS, 8 cores / 16 threads, 92 GiB RAM, shared desktop host.
  Builds and the import timing batch finished before the measured endpoint run.
- Both used **the same PostgreSQL 18.6 server**, with separate `meta_rust` and
  `meta_rails` databases restored from the same snapshot. Both connected over TCP
  to the same address and port using the same database role. Rails' old PostgreSQL
  15 service was stopped. The server reported live connections from both applications.
- Both configured connections reported instance ID `7698480746800631851`,
  256 MB shared buffers, 4 GB effective cache size, max connections 200, and JIT on.
  Both databases use `en_US.UTF-8`. The image is pinned to
  `pgvector/pgvector@sha256:2358fcba361ed2233a5ed81b5fe4ca779ccb304120ce531a3bf51c0ed7e2bc11`.
- Rust was served directly on loopback port 8091; Rails directly by Pitchfork
  through Docker's loopback port mapping on 3046, without nginx.
- `oha`: concurrency 16, five seconds per row, three rounds, alternating server
  order. Each row had 24 sequential warmup requests. HTTP keep-alive and compression
  were disabled. Both targets received the same modern Chrome user agent.
- All requests were anonymous. `eligible` permits normal response caching;
  `bypass` sends `_bypass_cache=1` to both servers. Search need not be cacheable.
- The four anonymous search rate limits were raised to 1,000,000 in both scratch
  databases so the test measured search rather than HTTP 429 throughput. Other
  rate limits remained enabled.

Snapshot SHA-256:
`7984abceea90b56fe85972a879b1b25d695a899e3e24cc694e6d187b0cdba184`.
Local raw results, response bodies, headers, and checksums are under
`bench/20261019-public-meta-pg18/run-1/` and its parent directory (ignored by Git).
The parent also retains server version/settings queries through each configured
connection and PostgreSQL's live connection table.

## Correctness checks

Rails assets were precompiled and both benchmark processes started before the
measured run. Every preflight endpoint returned HTTP 200.

The successful preflight compared ordered latest topic IDs, titles and counts,
latest user IDs, category IDs/names/counters, topic IDs/counts/post numbers/cooked
content, and search post IDs/topic IDs/blurbs/titles. These projections matched
between servers. This is not a full serializer parity test: response sizes differ
for categories and topics, which have additional fields beyond these checks.

HTML is reported separately because the implementations do different work:
Rust renders the page, whereas Rails returns the Ember application shell and
preloaded data. HTTP results do not measure interactive browser navigation.

## Warm endpoint results

Each number is the median of three per-run measurements, including the latency
percentiles. These are not percentiles pooled across runs. Both p50 and p99
columns show **Rust / Rails**, in milliseconds. There were zero HTTP/connection
errors in all 72 rows. Requests still in flight at the duration deadline are
excluded from the error count.

Response cache bypassed on both servers:

| Endpoint | Rust req/s | Rails req/s | p50 ms | p99 ms | Errors |
|---|---:|---:|---:|---:|---:|
| Categories JSON | 1,277 | 239 | 12.6 / 63.5 | 19.0 / 94.4 | 0 |
| Latest JSON | 2,238 | 124 | 7.1 / 130.9 | 9.7 / 158.0 | 0 |
| Twenty-post topic HTML | 554 | 70 | 29.3 / 230.0 | 35.4 / 411.7 | 0 |
| Twenty-post topic JSON | 1,109 | 105 | 14.6 / 153.9 | 18.6 / 186.6 | 0 |
| Search JSON | 276 | 112 | 57.9 / 145.4 | 71.8 / 168.6 | 0 |
| One-post topic JSON | 1,392 | 136 | 11.5 / 117.3 | 16.2 / 149.3 | 0 |

Cache eligible; warmup headers confirmed `X-Discourse-Cached: true` for every
endpoint on both targets, including search:

| Endpoint | Rust req/s | Rails req/s | p50 ms | p99 ms | Errors |
|---|---:|---:|---:|---:|---:|
| Categories JSON | 15,768 | 4,073 | 0.9 / 3.8 | 2.3 / 6.5 | 0 |
| Latest JSON | 15,348 | 3,984 | 0.9 / 3.9 | 2.3 / 6.8 | 0 |
| Twenty-post topic HTML | 13,450 | 2,150 | 1.1 / 6.3 | 2.6 / 20.5 | 0 |
| Twenty-post topic JSON | 15,389 | 3,308 | 0.9 / 4.3 | 2.3 / 7.8 | 0 |
| Search JSON | 15,484 | 3,528 | 0.9 / 4.3 | 2.3 / 9.8 | 0 |
| One-post topic JSON | 15,841 | 4,030 | 0.9 / 3.8 | 2.2 / 10.1 | 0 |

The [per-round TSV](benchmarks/20261019-public-meta.tsv) retains the variation.
For example, uncached latest ranged from 2,207–2,660 req/s for Rust and 112–125
for Rails. Search ranged from 275–284 and 107–113 respectively.

This run uses a frozen copy of the syntax/ShellCheck-validated benchmark script.
The earlier [mixed-version results](benchmarks/20261019-public-meta-mixed-postgres.tsv)
used PostgreSQL 16 for Rust and 15 for Rails. They are superseded by this shared
PostgreSQL 18 run and are not included in the tables above.

## First pass after process restart

Single serial samples in milliseconds, with database and OS caches already warm:

| Endpoint | Rust | Rails |
|---|---:|---:|
| Latest JSON | 25.1 | 300.6 |
| Categories JSON | 9.0 | 31.0 |
| One-post topic JSON | 19.6 | 84.0 |
| Twenty-post topic JSON | 10.5 | 64.4 |
| Twenty-post topic HTML | 44.0 | 305.9 |
| Search JSON | 58.0 | 145.9 |

These are observations of the first pass, not repeated cold-start measurements.
Subsequent endpoints benefit from initialization done by earlier requests.

## Import throughput

`scripts/time-corpus-import.rb` created 50 topics with 20 posts each in a disposable
copy of the corpus database on PostgreSQL 18.6, verified before generation. It
reused the varied public post bodies, removed
foreign quote references, created 40 users, and called Rails `PostCreator` for
each post, committing each call separately. Validation and background jobs were
skipped, remote image downloads and oneboxing were disabled, and deferred work
ran synchronously. This is a fixture-generation test, not write API throughput.

- 1,000 posts: **102.0 seconds**, **9.80 posts/second**.
- User creation: 7.8 seconds; total including category setup and final checks:
  **111.9 seconds**, excluding Rails boot and database restoration.
- All 1,000 posts had nonempty cooked HTML and search-index records.
- Linear extrapolation: about 34 minutes for 20,000 posts, about 5.7 hours for
  200,000. Larger databases, user counts, thread distributions, likes, and other
  modeled activity can change the rate. These are estimates, not measured imports.

The timed batch ran after the endpoint benchmark finished, without competing
benchmark or build work. Its
[timing results](benchmarks/20261019-import-timing.json) are retained. It did not alter either benchmark
database or the demo database.

## Search profile and next optimization

After the baseline and import runs, SQLx query logging was enabled for a separate
warmed request to `/search.json?q=Discourse` with `_bypass_cache=1`.
It returned 48 posts and 48 topics, HTTP 200, in 23.0 ms. This is a single
instrumented request, not the concurrency-16 benchmark latency.

| Work | Queries | Sum of SQLx elapsed time, ms |
|---|---:|---:|
| Tags per result topic | 48 | 3.31 |
| Solved state per result topic | 48 | 1.61 |
| Category field per result topic | 48 | 1.38 |
| Main matching-post query | 1 | 2.58 |
| All SQL | 164 | 12.11 |

SQLx durations include database round-trip time, not just database CPU execution.
The rest of request time includes application work, logging, and HTTP overhead.
The trace is retained at
`bench/20261019-public-meta-pg18/search-profile/queries.log`.

`Search::serialize` creates a `TopicListSerializer` with empty prefetched data,
then serializes each result topic. Normal topic lists call its existing
`prefetch` method first. Search therefore performs 144 individual tag/solved/
category lookups for this response. Batch prefetching is the first change to
test while preserving result content and permissions. No throughput improvement
has been claimed or implemented in this baseline.

## Search prefetch follow-up

Branch `perf/search-result-prefetch` adds the existing `TopicListSerializer::prefetch`
call before serializing search topics. A separate SQL trace counted 26 queries
instead of 164 for the same 48-result search. Each tag, solved-state, and category
field lookup is now batched. The batch loader also fetches other topic metadata,
so the reduction is 138 queries rather than 141.

The preserved `ef9eec98` release binary and the changed release binary were tested
against the same `meta_rust` database on PostgreSQL 18.6. This follow-up compares
Rust before and after; Rails was not measured again. Each process was restarted,
warmed with 24 requests, and measured for five seconds at concurrency 16 with
keep-alive disabled and response caching bypassed. Order alternated across three
rounds. Builds and tests finished before throughput measurement.

| Median of three runs | Before | After |
|---|---:|---:|
| Requests/sec | 282.5 | 408.3 |
| p50 ms | 56.7 | 38.7 |
| p99 ms | 73.8 | 52.9 |
| HTTP/connection errors | 0 | 0 |

Throughput improved 44.5%. Individual runs ranged from 277–300 req/s before and
364–419 after, so these are local measurements rather than a capacity guarantee.
Full response JSON matched across all runs after removing only
`grouped_search_result.search_log_id`. The six search tests and golden-response
parity replay passed on PostgreSQL 18, including a new regression checking that
tags, tag descriptions, and solved state remain associated with the correct topic.

Per-round results: [TSV](benchmarks/20261019-search-prefetch.tsv). Local raw results,
the comparison script, and the query trace are in
`bench/20261019-search-prefetch/`. The baseline tables above describe the original
binary and have not been replaced with these follow-up results.

## Reproducing the endpoint run

Restore the recorded snapshot into two fresh databases on the same PostgreSQL 18
instance, apply matching settings, analyze them, and start the server configurations
above. Verify `version()` and `pg_control_system().system_identifier` through both
configured connections before collecting results. Refresh Rails assets if
its plugin manifests are stale. Restart the servers and use an empty, isolated
Rails Redis cache for the first-pass samples. Do not flush a shared Redis database.

```bash
nix-shell
scripts/bench-corpus scripts/fixtures/public-meta-bench.json \
  bench/20261019-public-meta-pg18/new-run \
  rs=http://127.0.0.1:8091 rails=http://127.0.0.1:3046
```

`DURATION`, `CONCURRENCY`, and `ROUNDS` override the defaults. The endpoint list is
an array of `{ "name": "latest-json", "path": "/latest.json" }` objects. This run
used `/latest.json`, `/categories.json`, `/t/new-to-discourse-start-here/67.json`,
`/t/discourse-is-not-going-closed-source/77.json`, the same long topic without
`.json`, and `/search.json?q=Discourse`.

The first-pass file records one serial request per endpoint after process restart.
It is not a cold disk/database cache test, and it does not exercise every Rails
worker cold. Do not compare these single samples as stable cold-start percentiles.
