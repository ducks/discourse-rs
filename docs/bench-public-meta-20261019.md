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

## Larger corpus follow-up

The same six URLs were measured against identical PostgreSQL 18.6 copies with
1,089 topics, 20,576 posts, and 220 users. The added data contains 1,000 topics
and 20,000 posts, reusing sampled public bodies with synthetic authors. It includes
400 five-post, 300 ten-post, 200 twenty-five-post, and 100 hundred-post threads.
This increases volume and repeated search matches, not vocabulary diversity.
The original topic URLs remain unchanged, so the topic measurements still fetch
the original short and twenty-post threads, not the new hundred-post threads.

Rust uses the release binary with the search-prefetch change (`e1b67f15`);
Rails remains at `bf55a44`. All other benchmark settings above were retained:
three alternating rounds, five seconds per sample, concurrency 16, 24 warmups,
no HTTP keep-alive or compression, and separate cached/bypass runs. Both database
copies were analyzed. Generation and compilation were finished before measurement.
Both configured connections verified PostgreSQL 18.6 on the same server instance.

Uncached medians, with latency columns showing Rust / Rails in milliseconds:

| Endpoint | Rust req/s | Rails req/s | p50 ms | p99 ms |
|---|---:|---:|---:|---:|
| Latest JSON | 2,064 | 126 | 7.8 / 127.9 | 10.5 / 191.6 |
| Categories JSON | 819 | 202 | 19.7 / 79.4 | 24.5 / 95.7 |
| One-post topic JSON | 1,274 | 140 | 12.6 / 113.1 | 16.1 / 132.7 |
| Twenty-post topic JSON | 1,062 | 106 | 15.2 / 153.2 | 18.7 / 169.7 |
| Twenty-post topic HTML | 517 | 70 | 31.4 / 225.6 | 37.9 / 400.6 |
| Search JSON | 87 | 56 | 191.0 / 298.4 | 232.7 / 369.5 |

All 72 measurements had zero HTTP/connection errors, excluding requests aborted
at the duration deadline. Initial response projections matched for ordered latest
topics/users, category counters, cooked topic posts, and search posts/topics.
These checks cover selected core fields rather than full serializer parity.
Warmup headers confirmed response-cache hits for all cache-eligible endpoints.

Search is the clearest scaling bottleneck: Rust fell from 408 req/s in the small
corpus prefetch follow-up to 87 here; Rails fell from 112 to 56. This comparison
does not isolate dataset size perfectly because runs occurred at different times
on a shared desktop. Repeated text also makes this a high-match workload. Query
profiling is needed to attribute the slowdown; it is not a browser measurement.

### Search profile and corpus artifact

Follow-up profiling found that the importer appends an indexed attribution URL,
`[Original post on Meta](https://meta.discourse.org/...)`, to every copied post.
The expansion generator copied those footers too: 20,524 posts contain them.
Only 8,265 of those bodies contain the literal substring `discourse` before the
footer (this substring count is not an exact full-text match count).
Consequently the benchmark query matches 20,528 of 20,572 indexed posts. Treat
this search row as a near-universal-match stress case, not representative search
throughput or evidence of poor scaling for all terms.

A warmed traced Rust request took 84.6 ms; the main search SQL accounted for
67.1 ms and all 26 SQL calls for 74.7 ms. Three standalone EXPLAIN ANALYZE runs
gave median main-query execution times of 74.4 ms for `Discourse`, 1.09 ms for
`postgres`, 0.33 ms for `janitor`, and 0.24 ms for a no-match term. These are serial
query timings, not concurrent HTTP latency. The selective terms use the existing
GIN index. For `Discourse`, PostgreSQL scans the search data, joins eligible posts,
and computes ranking aggregates over about 20,500 matches / 1,048 topics before
selecting 51 results. The recorded plan has no disk reads, sort spills, or JIT.

Before claiming representative search throughput, keep attribution in the corpus
manifest rather than searchable post text, regenerate the search index through
the normal indexing path, and benchmark multiple terms of differing selectivity.
No dataset or application behavior was changed during this investigation.
Raw SQL, plans, and supplementary per-term HTTP probes are retained under
`bench/20261019-search-profile-large/`.

The [per-round TSV](benchmarks/20261019-large-public-meta.tsv) retains all results.
Raw responses, server version proofs, metadata, and summary JSON are under
`bench/20261019-large-benchmark/`. The source snapshot SHA-256 is
`c675d98d28e79d1a9c78842d561d62f2e23e5ff6c6d5f6a27b212e7bbf7f2769`.

## Cleaned corpus rerun

The attribution artifact was removed in a fresh copy, preserving all 20,524
source URLs in a separate manifest. Each affected post was rebaked with Rails
and indexed with `SearchIndexer.index(post, force: true)`. Verification checked
nonempty cooked content and removal of attribution from cooked/search text.
The normal indexing call skips posts without dirty tracked attributes after
rebake's direct column updates, so the forced path is required. The successful
pass took 770 seconds; all 20,524 affected posts were indexed.

Two fresh databases were restored from the cleaned SQL snapshot and analyzed on
the same PostgreSQL 18.6 server. They retain 1,089 topics, 20,576 posts, and 220
users. Of 20,572 indexed posts, 7,472 match `Discourse:*ABCD`, 39 match
`postgres:*ABCD`, and none match `zzzznonexistentbenchmark:*ABCD`. These counts
precede search visibility filtering and topic grouping. Neither copy contains
the attribution label in search raw data.

The original six URLs plus selective and no-match searches ran with the same
three-round protocol and server builds as the larger corpus run. No generation,
cleanup, or compilation ran during measurements. All 96 samples had zero
HTTP/connection errors, excluding requests aborted at the duration deadline.
Core response projections matched for latest, categories, both topic JSON
responses, and all three searches. Both servers omit `topics` for no matches.
All 16 first-round cache-eligible warmups reported response-cache hits.

Uncached medians; latency columns show Rust / Rails in milliseconds:

| Endpoint | Rust req/s | Rails req/s | p50 ms | p99 ms |
|---|---:|---:|---:|---:|
| Latest JSON | 2,012 | 121 | 7.9 / 133.9 | 11.2 / 194.2 |
| Categories JSON | 824 | 197 | 19.4 / 79.6 | 26.2 / 108.7 |
| One-post topic JSON | 1,206 | 141 | 13.1 / 113.6 | 17.9 / 142.9 |
| Twenty-post topic JSON | 1,073 | 108 | 15.0 / 148.9 | 19.6 / 169.1 |
| Twenty-post topic HTML | 541 | 73 | 30.1 / 220.5 | 36.4 / 352.7 |
| Search: Discourse | 127 | 70 | 133.5 / 231.8 | 159.0 / 301.2 |
| Search: postgres | 570 | 181 | 28.1 / 89.1 | 35.9 / 109.2 |
| Search: no matches | 1,653 | 689 | 9.5 / 23.1 | 14.8 / 30.9 |

The cleaned common-term median improves from 87 to 127 req/s for Rust and 56 to
70 for Rails. This supersedes the contaminated search row for this corpus, but
still represents a deliberately repetitive dataset. Individual no-match Rust
runs ranged from 1,015 to 1,667 req/s on the shared desktop; do not treat these
short-run medians as stable production capacity.

The importer now keeps attribution in its existing manifest instead of appending
it to post bodies. Expansion strips legacy footers and records generated-to-source
post mappings in its result file. The cleanup script is
`scripts/clean-corpus-attribution.rb`; source mappings are retained locally in
`bench/20261019-clean-corpus/attribution.json`. Snapshot SHA-256:
`bcc9ced228ed9d267e2f8340b6a6a940a351078e2fdb4985a0376eb8dda7abba`.
Raw results and connection proofs are under `bench/20261019-clean-benchmark/`.
See the [per-round TSV](benchmarks/20261019-clean-public-meta.tsv) and
`scripts/fixtures/public-meta-clean-bench.json` for the endpoint list.

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
