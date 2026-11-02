# Rust rerun, October 30, 2026

Rust only, main `1e057a14` (release), the October 24 protocol unchanged:
`meta_rust` on the same PostgreSQL 18.6 container (system identifier verified),
`scripts/bench-corpus` with the same fixture, five seconds per row, concurrency
16, three rounds, server on 8091 with the October 24 environment. Load average
1.26 at the start; the desktop session idle. Rails is unchanged since October 24,
so the paired October 24 report remains the Rails baseline. Zero errors in all samples.

| Uncached req/s, median of 3 | Oct 24 | Oct 30 |
|---|---:|---:|
| Latest JSON | 2,173 | 2,177 |
| Categories JSON | 845 | 826 |
| Short topic JSON | 1,245 | 1,181 |
| Twenty-post topic JSON | 1,051 | 1,018 |
| Topic HTML | 490 | 535 |
| Search, common term | 125 | 122 |
| Search, selective | 504 | 496 |
| Search, no match | 1,661 | 988 |

Within the October 24 rounds' spread except topic HTML, 9% faster after the
topic-page work. Startup (five runs, medians): the binary grew from 185 to
268 MB (about 30 MB of vendored math assets, the rest the chat round and
plugins), exec to first 200 from 87 to 113 ms, RSS 50 to 56 MB idle and 80
to 89 MB after the warm-up.

**Uncached search here partly measures the rate limiter.** The snapshot raises
the four anonymous search limits to 1,000,000, and every search runs
`discourse_rs.rate_limit_perform`, which rewrites the key's list of stamps under
its row lock: the longer the list, the slower each search, and every anonymous
search queues on the global key. Search rows fall round over round on both
dates (no match 1,677 / 1,661 / 1,038 on October 24, 1,658 / 988 / 674 now), a
server restart doesn't help, and `pg_stat_activity` under load shows every
connection waiting on that function's row locks. Rails keeps the list in Redis,
where a push costs the same however long it is. Stamps more than a window old
are now dropped (exactly Rails' answers), which stops the growth across rounds,
but a minute's window at these rates is still about 21,500 stamps; ROADMAP 285
stores them as per-second counts. At Rails' default limits (150 a minute at
most for search) the lists stay small. Raw output and the investigation:
`bench/20261030-rust/` of the main checkout (`summary.md`).
