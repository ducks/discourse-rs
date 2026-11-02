# Rust vs Rails benchmarks

One comparison table from the October 24, 2026 paired run: release Rust
`85672018` (v20261024.0.0) against production Rails `bf55a44`. Both used
separate copies of the same 1,089-topic, 20,576-post corpus on one
PostgreSQL 18.6 instance. These are historical measurements on a shared
laptop, not measurements of the current VPS or current demo build.

HTTP cells show **throughput; p50 / p99 latency**. Higher req/s is better;
lower latency and memory are better. HTTP results are medians of three
runs at concurrency 16. Browser cells show **median / p95 content-ready
time**, from 20 samples. First-request rows are single requests after a
process restart with warm database and OS caches.

| Area | Measurement | Rust | Rails |
|---|---|---:|---:|
| HTTP uncached | Latest JSON | 2,173 req/s; 7.4 / 9.9 ms | 122 req/s; 124.2 / 159.0 ms |
| HTTP uncached | Categories JSON | 845 req/s; 19.1 / 23.9 ms | 203 req/s; 79.0 / 97.8 ms |
| HTTP uncached | One-post topic JSON | 1,245 req/s; 12.9 / 17.1 ms | 151 req/s; 106.9 / 129.7 ms |
| HTTP uncached | Twenty-post topic JSON | 1,051 req/s; 15.4 / 20.0 ms | 112 req/s; 146.3 / 192.5 ms |
| HTTP uncached | Twenty-post topic HTML | 490 req/s; 33.3 / 41.6 ms | 74 req/s; 211.6 / 439.6 ms |
| HTTP uncached | Search: Discourse | 125 req/s; 135.0 / 163.9 ms | 65 req/s; 240.8 / 367.7 ms |
| HTTP uncached | Search: postgres | 504 req/s; 31.3 / 42.7 ms | 172 req/s; 94.3 / 110.3 ms |
| HTTP uncached | Search: no matches | 1,661 req/s; 9.4 / 14.5 ms | 691 req/s; 22.6 / 30.7 ms |
| HTTP cached | Latest JSON | 15,665 req/s; 0.9 / 2.3 ms | 3,716 req/s; 3.9 / 10.1 ms |
| HTTP cached | Categories JSON | 16,406 req/s; 0.9 / 2.2 ms | 4,170 req/s; 3.7 / 6.2 ms |
| HTTP cached | One-post topic JSON | 17,187 req/s; 0.8 / 2.1 ms | 4,429 req/s; 3.5 / 5.2 ms |
| HTTP cached | Twenty-post topic JSON | 16,336 req/s; 0.9 / 2.2 ms | 3,926 req/s; 3.9 / 6.0 ms |
| HTTP cached | Twenty-post topic HTML | 14,345 req/s; 1.0 / 2.5 ms | 2,154 req/s; 6.1 / 18.4 ms |
| HTTP cached | Search: Discourse | 16,335 req/s; 0.9 / 2.2 ms | 3,650 req/s; 4.0 / 10.0 ms |
| HTTP cached | Search: postgres | 15,577 req/s; 0.9 / 2.3 ms | 3,633 req/s; 4.3 / 6.1 ms |
| HTTP cached | Search: no matches | 15,943 req/s; 0.9 / 2.2 ms | 3,923 req/s; 3.9 / 6.1 ms |
| First request | Latest JSON | 15.2 ms | 157.2 ms |
| First request | Categories JSON | 9.9 ms | 34.1 ms |
| First request | One-post topic JSON | 15.4 ms | 54.9 ms |
| First request | Twenty-post topic JSON | 10.1 ms | 53.3 ms |
| First request | Twenty-post topic HTML | 45.0 ms | 296.0 ms |
| First request | Search: Discourse | 83.2 ms | 99.7 ms |
| First request | Search: postgres | 16.1 ms | 32.8 ms |
| First request | Search: no matches | 4.9 ms | 9.5 ms |
| Browser ready | Latest, cold | 169 / 185 ms | 753 / 806 ms |
| Browser ready | Latest, warm | 104 / 105 ms | 712 / 759 ms |
| Browser ready | One-post topic, cold | 143 / 154 ms | 670 / 718 ms |
| Browser ready | One-post topic, warm | 88 / 104 ms | 641 / 683 ms |
| Browser ready | Twenty-post topic, cold | 170 / 186 ms | 859 / 919 ms |
| Browser ready | Twenty-post topic, warm | 104 / 136 ms | 819 / 897 ms |
| Browser ready | Search, cold | 184 / 256 ms | 719 / 832 ms |
| Browser ready | Search, warm | 122 / 188 ms | 690 / 789 ms |
| Browser ready | Latest to topic, click | 108 / 126 ms | 191 / 256 ms |
| Browser ready | Topic to latest, click | 73 / 91 ms | 188 / 192 ms |
| Memory | Idle RSS | 50 MiB | 5,144 MiB |
| Memory | Idle PSS | 48 MiB | 872 MiB |
| Memory | Median RSS under load | 210 MiB | 6,488 MiB |
| Memory | Peak RSS under load | 216 MiB | 7,296 MiB |
| Memory | After run, RSS / PSS | 216 / 214 MiB | 6,611 / 3,289 MiB |
| Startup | Exec to first HTTP 200 | 87 ms (median of five) | 9.8 s (one observation) |
| Startup | All workers ready | One process | 17.2 s |
| Distribution | Binary size | 185 MiB; 161 MiB stripped | Ruby application; no comparable single binary |

The [full run report](docs/bench-20261024.md) records the protocol, raw
artifacts, browser transfer and script timings, and response spot checks.
All 96 HTTP samples had zero non-2xx responses or connection errors,
excluding duration-deadline cancellations. Selected core response fields
matched; that check is not full serializer parity.

Read the table with these differences in mind:

- Cache hits measure cached response delivery. Keep them separate from
  uncached application work.
- Rust renders topic HTML; Rails serves an Ember shell that then loads
  the page. The browser content-ready rows cover more of that work.
- Rails used eight workers. Its RSS sums all 11 Pitchfork processes and
  double-counts shared pages; PSS apportions them. Neither application's
  footprint includes PostgreSQL, Redis, the browser, or the OS.
- Browser “cold” means a fresh browser context, not a cold server. Rails'
  early splash-screen paint is not content-ready time. Rails served its
  own precompiled assets without nginx; compression also differed.
- Search used artificially raised limits. Rust's timestamp-list rate
  limiter accumulated contention during sustained search; these rows
  include that cost. They are not a production capacity estimate.

A Rust-only October 30 rerun is recorded in
[the rerun notes](docs/bench-rust-20261030.md). It measured 113 ms startup,
56 MiB idle RSS, and 89 MiB after warm-up. No fresh Rails run accompanied
it, so those figures are not mixed into the paired table above.

Earlier datasets and optimization experiments remain in the
[benchmark history](docs/bench-history.md) and
[public corpus report](docs/bench-public-meta-20261019.md).

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
