# Browser page-load benchmarks

`scripts/bench-browser` runs Chromium through Playwright, using the same Nix packages as `scripts/ui-compare`. It writes raw samples, environment metadata, and grouped median/p95 results. It does not start servers, modify their settings, or generate data.

```bash
scripts/bench-browser \
  --rust http://127.0.0.1:8091 \
  --rails http://127.0.0.1:3046 \
  --out bench/browser-comparison
```

Use production builds with matching snapshots, settings, PostgreSQL versions and compression. Stop imports, builds and other load first. The default fixtures use the public Meta corpus; pass `--fixtures PATH` for another dataset. Each fixture specifies a path, a visible content selector, a name and a kind (`list`, `topic`, or `search`). An absent topic or empty search result is a failed sample, not a fast page.

Twenty samples per target/scenario run serially. Target order reverses every round. `--samples 1 --timeout-ms 5000` gives a quick smoke test. Omitting `--rails` is supported for harness development; it is not a comparative benchmark. Output directories must not already exist.

## What is measured

- Cold visits use a fresh browser context, with no previous HTTP cache or cookies. Chromium itself and the server are already running. This is not a cold database or OS-cache test.
- Warm visits reuse that context after its cold visit. Navigation passes through `about:blank` to force a new document while retaining its HTTP cache.
- Latest-to-topic clicks select the first visible topic title. Topic-to-latest clicks use the visible Latest link. The harness reports whether each transition produced a new document or stayed in the same document. Those are separate summary groups. It does not label full document navigations as SPA transitions.
- `ready_ms` ends when the fixture's first content element is visible, has text and has crossed two animation frames. This measures content readiness, not complete hydration or all images loaded. The same selector is used for both implementations.
- TTFB and FCP come from browser Performance entries. `lcp_observed_ms` is the latest observed LCP candidate at readiness plus `--settle-ms` (default 500). It is not a finalized field LCP measurement. Document timings are null for same-document transitions.
- `transfer_bytes` sums CDP `Network.loadingFinished.encodedDataLength` for requests started in the measurement window. This includes completed background requests and excludes unfinished transfers. Cache hits may transfer zero bytes. It is not total bytes eventually loaded by the page.
- `script_ms` is the CDP ScriptDuration delta through the same measurement window. It includes scripting, not total CPU or rendering work. A counter reset is reported as null.

Paint, byte and scripting collection continues for the fixed settling window after readiness. There is no network-idle wait, CPU/network throttling, screenshot capture or concurrent page load. Browser and application caches are deliberately different concepts here.

## Authentication and safety

By default visits are anonymous. Optionally set `BENCH_USERNAME` and `BENCH_PASSWORD` for a disposable account present in both snapshots. API login happens once per target before measurement, and its cookies are reused in memory across fresh contexts to avoid login throttling. Every measured topic visit then checks that clicking Reply exposes an editable composer; it does not type or submit. This check is outside timing. Authenticated cold runs have session cookies and must be compared separately from anonymous cold runs.

Credentials and cookies are never written to artifacts. Do not put credentials in URLs. Only benchmark trusted local/test instances: browsing can update views and authenticated read state. Use an existing regular account, not an administrator. No trace or screenshot artifacts are captured because they can expose private page content.

## Interpreting results

`metadata.json` records browser version, targets, fixtures, viewport and sample settings. `samples.jsonl` is flushed after every sample so partial runs remain inspectable. `summary.json` groups by target, scenario, cold/warm/click mode, and actual document navigation type. Every metric includes its valid sample count; p95 uses nearest rank. Failures cause a nonzero exit and appear in raw output and summary.

`validation.json` records failures and cross-server click-target mismatches. If the two Latest lists choose different topic paths in a round, the run exits nonzero. Inspect failures before comparing numbers. This harness does not assert cross-server content equivalence; retain the HTTP benchmark's response parity checks. Compare medians and p95 across repeated runs, and record server/database revisions separately alongside the artifacts.

Initial validation used the local Rust demo: all ten scenarios passed, then a two-round test pointing both target labels at that same demo passed forty samples and verified alternating order. An intentionally nonexistent topic produced two failed samples and exit status 1. An authenticated smoke test on the disposable Rust fixture instance also passed all ten scenarios, including editable reply composers after cold and warm visits to topics 5 and 35. That check caught missing JSON/XHR login headers and per-context login throttling; both were corrected in the harness. These are harness checks, not Rust/Rails performance results. A real Rails comparison still needs validation against matching running instances.
