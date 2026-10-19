# Benchmarks

The latest recorded measurements are from October 16, 2026. They show substantial
endpoint throughput gains over Rails on the small parity seed and identify useful
optimizations within discourse-rs. They do not establish production capacity or
performance on a large forum.

The public Meta corpus collected on October 19 has not been benchmarked yet.
All original tables, profiles, and experiment notes are in the
[benchmark history](docs/bench-history.md).

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

## Latest production Rails comparison

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

Run both current implementations against the public Meta baseline with matching
settings and no competing builds. Record revisions, hardware, PostgreSQL settings,
worker/pool sizes, cache state, authentication, compression, and rate-limit behavior.
Repeat runs and retain error counts alongside throughput and latency.

Include long and short topics, search, and both anonymous and logged-in requests.
Measure browser navigation separately from HTTP endpoints. That is the evidence
needed before presenting a current, realistic performance comparison.
