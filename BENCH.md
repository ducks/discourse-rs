# Benchmarks

The latest run is the [v20261024.0.0 release benchmark](docs/bench-20261024.md):
release Rust `85672018` against production Rails `bf55a44` on the cleaned public
Meta corpus (1,089 topics, 20,576 posts, 220 users), separate copies of the same
snapshot on **one PostgreSQL 18.6 instance**, the same protocol as October 19.
Uncached anonymous req/s, Rust / Rails: latest JSON 2,173 / 122, categories
845 / 203, twenty-post topic JSON 1,051 / 112, topic HTML 490 / 74, and search
125 / 65 (common term), 504 / 172 (selective), 1,661 / 691 (no match). Zero
errors in all 96 samples; core response fields matched. It also records the
first browser comparison (a cold Latest visit is content-ready in 169 ms vs
753 ms) and memory: 50 MB idle and about 210 MB under load for Rust, 5.0 GiB
summed RSS idle and 6.3 GiB under load for Rails' eight workers. This is still a
small forum on one shared laptop, not a production-capacity test.

Against the October 19
[cleaned corpus rerun](docs/bench-public-meta-20261019.md#cleaned-corpus-rerun)
(Rust 127 / 570 / 1,653 search, latest 2,012), the unchanged Rails side drifted
up to about 7%; Rust is within that band except topic HTML, about 9% slower
after a week of topic-page work. The original small-corpus baseline is retained
below.

The earlier search term `Discourse` matched almost every generated post because
copied attribution URLs contained `meta.discourse.org`. Those footers have now
been removed and the posts recooked/reindexed. Source URLs remain in a separate
manifest. The earlier search row is superseded; see the
[profile and corpus artifact](docs/bench-public-meta-20261019.md#search-profile-and-corpus-artifact).

See the [public corpus baseline](docs/bench-public-meta-20261019.md) for setup,
correctness checks, first-pass latency, and import timing. Older tables, profiles,
and experiment notes are in the [benchmark history](docs/bench-history.md).

## Reading the results

- Compare rows from the same run. Hardware load, code, settings, and datasets
  changed between sessions; gains from separate runs cannot be multiplied.
- Use a release Rust build and production-mode Rails with the same data.
  Historical `rails-dev` results include development overhead and should not be
  used to claim a speedup over deployed Discourse.
- Separate anonymous cache hits, cache misses, and logged-in requests. Both
  implementations now have an anonymous cache; older runs predate the Rust
  implementation. Historical `cached` and `uncached` labels refer to Rails.
- Check `non-2xx` before throughput. Rate-limited search rows measure rejection
  throughput, not successful searches. Loopback requests can also be exempt from
  address-based limits even when rate limiting is enabled.
- HTML requests do different work: Rust renders the page; Rails serves an Ember
  shell that fetches topic data afterwards. HTTP latency is not a browser
  page-load comparison.
- These were local tests: client, servers, and PostgreSQL shared one machine,
  with HTTP keep-alive disabled. Several optimization runs competed with builds.

## Public corpus baseline, October 19

Release Rust `ef9eec98` and production Rails `bf55a44`, anonymous requests,
concurrency 16, three five-second rounds. The table gives median throughput with
response caching bypassed on **both** servers. All 72 measured rows, including
cache-eligible requests, had zero HTTP or connection errors.

| Endpoint | Rust req/s | Rails req/s | Rust p50, ms | Rails p50, ms |
|---|---:|---:|---:|---:|
| Latest JSON | 2,238 | 124 | 7.1 | 130.9 |
| Categories JSON | 1,277 | 239 | 12.6 | 63.5 |
| One-post topic JSON | 1,392 | 136 | 11.5 | 117.3 |
| Twenty-post topic JSON | 1,109 | 105 | 14.6 | 153.9 |
| Search JSON | 276 | 112 | 57.9 | 145.4 |

That is about 2.5x to 18x the Rails throughput on these JSON endpoints. The selected
core response fields matched. Both database connections were verified against the
same PostgreSQL 18.6 instance, settings, locale, and TCP endpoint. This remains a
local deployment comparison: the applications have different runtime/worker
architectures. No compression or keep-alive; no logged-in or mixed-traffic workload.

Both implementations served verified anonymous cache hits much faster:
roughly 15,000–16,000 JSON req/s for Rust and 3,000–4,000 for Rails. Those numbers
mostly describe response-cache delivery. See the
[full results](docs/bench-public-meta-20261019.md) for per-round data, p99,
HTML results, setup limitations, and first-pass latency.

## Historical production Rails comparison

October 13: discourse-rs `eafbcea`, release build, production mode, fresh parity
seed. The Rails reference was unchanged from the October 9 setup: Discourse
`bf55a44`, precompiled assets, eight Pitchfork workers, and nginx.

Each row ran for five seconds at concurrency 16, logged in as `user1` on both
sides, without anonymous caching. Rust rate limiting was enabled, although the
loopback address was exempt from request limits. These selected JSON endpoints
all returned zero non-2xx responses:

| Endpoint | Rust req/s | Rails req/s | Rust p50, ms | Rails p50, ms |
|---|---:|---:|---:|---:|
| `/site.json` | 2,481 | 309 | 6.1 | 50.8 |
| `/latest.json` | 2,769 | 188 | 5.5 | 84.8 |
| `/categories.json` | 2,357 | 266 | 6.4 | 59.7 |
| `/t/welcome-to-discourse/5.json` | 1,021 | 100 | 15.6 | 162.0 |
| `/u/system.json` | 3,680 | 227 | 4.0 | 70.4 |

These are historical endpoint measurements, not results for today's build.
Later features added overhead: the October 15 Rust comparison recorded
`/latest.json` falling from 2,645 to 2,419 req/s and topic HTML from 153 to 106
req/s. October 16 addressed specific costs; it was not a new Rails comparison.

See the [October 13 full run](docs/bench-history.md#2026-10-13-after-messages-and-chat-dms-rate-limits-on)
for anonymous-cache results, p99 latency, errors, and the limiter investigation.

## Latest optimization results

October 16 investigated logged-in HTML topic pages. The page used 114 distinct
statements, exceeding SQLx's default cache of 100 per connection. Repeated query
planning dominated PostgreSQL's work. An inline SVG sprite also added 116 KB to
each HTML response and increased compression work.

Each row below summarizes its own comparison. All used the parity seed and
concurrency 16; the machine was shared with other builds.

| Change | Measurement | Before | After |
|---|---|---:|---:|
| Batch page and navigation queries | Queries for a five-post topic page | 187 | 145 |
| Raise statement cache to 1,000 per connection | Topic HTML req/s, repeated-run median | 81 | 216 |
| Serve a cacheable SVG sprite | Topic HTML req/s, three-run median | 351 | 409 |
| Same sprite change | Topic HTML size, raw / Brotli | 204 / 50 KB | 88 / 16 KB |

Reducing query count alone barely helped while the statement cache was thrashing.
The larger cache produced the biggest improvement in this investigation. After
that, query reductions and smaller HTML responses provided further gains.

The sprite run found unchanged first contentful paint with a warm cache and
roughly 20 ms extra on a first visit. These are separate experiments, not one
combined before/after result.

See the [October 16 profiles and tables](docs/bench-history.md#2026-10-16-the-topic-pages-queries-and-where-its-time-goes)
for revisions, load conditions, latency, and excluded noisy measurements.

## Reproduce a comparison

Use the repository's `nix-shell` for Rust, PostgreSQL, `oha`, `curl`, and `jq`.
Prepare two separate databases from the same snapshot and start both servers
before running the benchmark. Use production mode, equivalent site settings,
and a release Rust binary; do not benchmark the development demo as production.

```bash
nix-shell
make release-build

# Anonymous requests; Rails is measured with and without its anonymous cache.
make bench TARGETS="rs=http://127.0.0.1:8080 rails=http://127.0.0.1:3043" \
  BENCH_ARGS="-d 10s -c 16 -o bench/comparison-anonymous"

# Logged in; use a disposable local account present in both databases.
scripts/bench -d 10s -c 16 -u benchuser:local-test-password \
  -o bench/comparison-user \
  rs=http://127.0.0.1:8080 rails=http://127.0.0.1:3043
```

[`scripts/bench`](scripts/bench) discovers a topic, category, user, and tag from
the first target, then measures the same endpoints on each server. Preserve IDs
across the databases. Raw `oha` results are saved in the output directory; the
printed table reports req/s, p50, p99, and non-2xx responses or connection errors.

For binary size, time to the first healthy response, and idle/warm RSS:

```bash
# Supply the intended DATABASE_URL and server configuration first.
make bench-startup
```

[`scripts/bench-startup`](scripts/bench-startup) starts its own process on
`127.0.0.1:8090` by default and stops it afterwards. It measures server startup,
not database provisioning or browser page load. Old build/test timings in the
history describe much smaller versions of the project.

## Next useful measurement

Profile search on the expanded corpus: uncached throughput fell substantially
on both implementations. Add logged-in traffic and mixed endpoint loads; measure
browser navigation separately. The original endpoint set still selects a
twenty-post topic, so benchmark the generated hundred-post threads separately.

The measured 1,000-post generation batch on PostgreSQL 18.6 took 102.0 seconds
through Rails cooking and indexing. At that rate, 20,000 posts would take about
34 minutes and 200,000 about 5.7 hours. Those are extrapolations, with background
jobs skipped.

Search profiling found 164 SQL queries for 48 results, including 48 tag queries,
48 solved-state queries, and 48 category-field queries. Calling the topic
serializer's existing batch-prefetch method reduced this to 26 queries. A separate
three-round before/after run measured median uncached search throughput of
283 → 408 req/s (+45%), with p50 latency of 56.7 → 38.7 ms. Responses matched
apart from search-log IDs, with zero errors. The baseline tables above remain
unchanged; see the [follow-up measurements](docs/bench-public-meta-20261019.md#search-prefetch-follow-up).
