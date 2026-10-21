#!/usr/bin/env python3
"""Serial browser measurements; no throughput load or server provisioning."""
import argparse
import json
import math
import os
from pathlib import Path
import statistics
import sys
import time
from urllib.parse import urlparse

from playwright.sync_api import sync_playwright

INIT = """() => {
  window.__bench = {id: crypto.randomUUID(), lcp: null};
  new PerformanceObserver(list => {
    for (const entry of list.getEntries()) window.__bench.lcp = entry.startTime;
  }).observe({type: 'largest-contentful-paint', buffered: true});
}"""
TIMINGS = """() => {
  const nav = performance.getEntriesByType('navigation')[0];
  const fcp = performance.getEntriesByName('first-contentful-paint')[0];
  return {document_id: window.__bench.id,
    ttfb_ms: nav ? nav.responseStart - nav.startTime : null,
    fcp_ms: fcp?.startTime ?? null, lcp_observed_ms: window.__bench.lcp};
}"""
METRICS = ["ready_ms", "ttfb_ms", "fcp_ms", "lcp_observed_ms", "transfer_bytes", "script_ms"]
AUTH_COOKIES = {}


def percentile(values, fraction):
    return sorted(values)[max(0, math.ceil(len(values) * fraction) - 1)]


def authenticate_request(context, base):
    user, password = os.environ.get("BENCH_USERNAME"), os.environ.get("BENCH_PASSWORD")
    if not user and not password:
        return False
    if not user or not password:
        raise ValueError("Set both BENCH_USERNAME and BENCH_PASSWORD")
    csrf_response = context.request.get(base + "/session/csrf.json")
    if not csrf_response.ok:
        raise RuntimeError("CSRF request failed")
    response = context.request.post(base + "/session", form={"login": user, "password": password},
                                    headers={"X-CSRF-Token": csrf_response.json()["csrf"],
                                             "X-Requested-With": "XMLHttpRequest",
                                             "Accept": "application/json"})
    if not response.ok or not response.json().get("user"):
        raise RuntimeError("Login failed; no credentials or response saved")
    return True


def authenticate(context, base):
    try:
        if base in AUTH_COOKIES:
            context.add_cookies(AUTH_COOKIES[base])
            return True
        authenticated = authenticate_request(context, base)
        if authenticated:
            # Reuse one session in memory, avoiding login throttling across samples.
            AUTH_COOKIES[base] = context.cookies()
        return authenticated
    except Exception:
        # Request exceptions can include authentication headers in their call log.
        raise RuntimeError("Authentication setup failed; check account and credential environment") from None


def ready(page, fixture):
    page.locator(fixture["ready"]).first.wait_for(state="visible")
    page.wait_for_function("selector => document.querySelector(selector)?.textContent.trim().length > 0",
                           arg=fixture["ready"])
    # Wait for a rendering opportunity after the content is present, not network idle.
    page.evaluate("() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))")


class Meter:
    def __init__(self, context, page):
        self.cdp = context.new_cdp_session(page)
        self.cdp.send("Network.enable")
        self.cdp.send("Performance.enable")
        self.pending = set()
        self.transferred = 0
        self.cdp.on("Network.requestWillBeSent", self.started)
        self.cdp.on("Network.loadingFinished", self.finished)

    def started(self, event):
        self.pending.add(event["requestId"])

    def finished(self, event):
        if event["requestId"] in self.pending:
            self.transferred += event["encodedDataLength"]
            self.pending.remove(event["requestId"])

    def script(self):
        metrics = self.cdp.send("Performance.getMetrics")["metrics"]
        return next(m["value"] for m in metrics if m["name"] == "ScriptDuration")

    def reset(self):
        self.pending.clear()
        self.transferred = 0
        self.script_start = self.script()

    def result(self):
        elapsed = self.script() - self.script_start
        return {"transfer_bytes": self.transferred,
                "script_ms": elapsed * 1000 if elapsed >= 0 else None}


def measure(page, meter, action, fixture, settle_ms, previous_id=None):
    meter.reset()
    start = time.monotonic()
    response = action()
    if response is not None and hasattr(response, "status") and response.status >= 400:
        raise RuntimeError(f"Document returned HTTP {response.status}")
    ready(page, fixture)
    elapsed = (time.monotonic() - start) * 1000
    page.wait_for_timeout(settle_ms)
    timings = page.evaluate(TIMINGS)
    document_id = timings.pop("document_id")
    navigation = "document" if document_id != previous_id else "same-document"
    if navigation == "same-document":
        # Navigation Timing and paint entries still describe the old document.
        timings = {k: None for k in timings}
    return {"ready_ms": elapsed, "navigation": navigation, **timings, **meter.result()}


def check_reply(page):
    button = page.locator(".topic-post .reply").first
    button.wait_for(state="visible")
    button.click()
    editor = page.locator("#reply-control textarea, #reply-control [contenteditable=true]").first
    editor.wait_for(state="visible")
    if not editor.is_editable():
        raise RuntimeError("Reply editor is not editable")
    # No typing or submitting; this is a capability check outside timing.


def run_visits(browser, base, fixture, args, emit):
    context = browser.new_context(viewport={"width": 1280, "height": 900}, service_workers="block")
    try:
        authenticated = authenticate(context, base)
        context.add_init_script("(" + INIT + ")()")
        page = context.new_page()
        page.set_default_timeout(args.timeout_ms)
        meter = Meter(context, page)
        for temperature in ("cold", "warm"):
            if temperature == "warm":
                # Preserve this context's HTTP cache but avoid same-URL SPA/no-op navigation.
                page.goto("about:blank")
            try:
                result = measure(page, meter, lambda: page.goto(base + fixture["path"], wait_until="domcontentloaded"),
                                 fixture, args.settle_ms)
                if authenticated and fixture["kind"] == "topic":
                    check_reply(page)
                    result["reply_usable"] = True
                emit({"scenario": fixture["name"], "mode": temperature, "ok": True, **result})
            except Exception as error:
                emit({"scenario": fixture["name"], "mode": temperature, "ok": False, "error": str(error)})
    except Exception as error:
        emit({"scenario": fixture["name"], "mode": "setup", "ok": False, "error": str(error)})
    finally:
        context.close()


def run_clicks(browser, base, fixture, args, emit):
    context = browser.new_context(viewport={"width": 1280, "height": 900}, service_workers="block")
    try:
        authenticate(context, base)
        context.add_init_script("(" + INIT + ")()")
        page = context.new_page()
        page.set_default_timeout(args.timeout_ms)
        page.goto(base + fixture["path"], wait_until="domcontentloaded")
        ready(page, fixture)
        link = page.locator(".topic-list a.title").first
        href = link.get_attribute("href")
        topic_id = href.split("/")[3]
        meter = Meter(context, page)
        previous = page.evaluate("window.__bench.id")
        result = measure(page, meter, lambda: link.click(),
                         {"ready": ".topic-post .cooked"}, args.settle_ms, previous)
        if topic_id not in urlparse(page.url).path.split("/"):
            raise RuntimeError("Topic click did not reach the selected topic")
        emit({"scenario": "latest-to-topic", "mode": "click", "ok": True,
              "topic_path": href, **result})
        previous = page.evaluate("window.__bench.id")
        # Use the visible Latest link instead of history, which may restore BFCache.
        back = page.locator('a[href="/latest"]:visible').first
        result = measure(page, meter, lambda: back.click(), fixture, args.settle_ms, previous)
        emit({"scenario": "topic-to-latest", "mode": "click", "ok": True, **result})
    except Exception as error:
        emit({"scenario": "click-journey", "mode": "click", "ok": False, "error": str(error)})
    finally:
        context.close()


def summarize(rows):
    groups = {}
    for row in rows:
        key = (row["target"], row["scenario"], row["mode"], row.get("navigation", "failed"))
        groups.setdefault(key, []).append(row)
    result = []
    for (target, scenario, mode, navigation), samples in groups.items():
        summary = dict(target=target, scenario=scenario, mode=mode, navigation=navigation,
                       samples=len(samples), failures=sum(not r["ok"] for r in samples))
        for metric in METRICS:
            values = [r[metric] for r in samples if r["ok"] and r.get(metric) is not None]
            summary[metric] = {"n": len(values), "median": statistics.median(values),
                               "p95": percentile(values, .95)} if values else None
        result.append(summary)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", required=True)
    parser.add_argument("--rails", help="Omit only for a smoke test, not a comparison")
    parser.add_argument("--fixtures", type=Path, default=Path(__file__).parent / "fixtures/browser-bench.json")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=20)
    parser.add_argument("--timeout-ms", type=int, default=30000)
    parser.add_argument("--settle-ms", type=int, default=500)
    parser.add_argument("--skip-clicks", action="store_true")
    args = parser.parse_args()
    if args.samples < 1 or args.settle_ms < 0:
        parser.error("samples must be positive and settle-ms nonnegative")
    targets = [(name, url.rstrip("/")) for name, url in (("rust", args.rust), ("rails", args.rails)) if url]
    for _, url in targets:
        parsed = urlparse(url)
        if parsed.scheme not in ("http", "https") or parsed.username or parsed.password or parsed.query:
            parser.error("Use an HTTP(S) base URL without credentials or query parameters")
    fixtures = json.loads(args.fixtures.read_text())
    if not fixtures or any(not f.get("ready") or not f.get("name") or
                           not f.get("path", "").startswith("/") or
                           f.get("path", "").startswith("//") or
                           f.get("kind") not in ("list", "topic", "search") for f in fixtures):
        parser.error("Fixtures need a name, local path, ready selector and list/topic/search kind")
    args.out.mkdir(parents=True, exist_ok=False)
    rows = []
    with sync_playwright() as playwright, (args.out / "samples.jsonl").open("w") as raw:
        browser = playwright.chromium.launch()
        metadata = {"targets": dict(targets), "samples": args.samples, "viewport": [1280, 900],
                    "chromium": browser.version, "fixtures": fixtures, "settle_ms": args.settle_ms,
                    "authenticated": bool(os.environ.get("BENCH_USERNAME")),
                    "note": "Cold browser contexts; server/OS caches are not reset. No concurrency or throttling."}
        (args.out / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
        for sample in range(args.samples):
            for target, base in (targets if sample % 2 == 0 else list(reversed(targets))):
                def emit(row):
                    row.update(target=target, sample=sample + 1)
                    rows.append(row)
                    raw.write(json.dumps(row) + "\n")
                    raw.flush()
                    print(f"{sample + 1} {target} {row['scenario']} {row['mode']}: "
                          + (f"{row['ready_ms']:.1f} ms" if row["ok"] else "FAILED"), flush=True)
                for fixture in fixtures:
                    run_visits(browser, base, fixture, args, emit)
                latest = next((f for f in fixtures if f["kind"] == "list"), None)
                if latest and not args.skip_clicks:
                    run_clicks(browser, base, latest, args, emit)
        browser.close()
    (args.out / "summary.json").write_text(json.dumps(summarize(rows), indent=2) + "\n")
    failures = sum(not r["ok"] for r in rows)
    comparison_errors = []
    for sample in range(1, args.samples + 1):
        paths = {r["topic_path"] for r in rows if r["sample"] == sample and r.get("topic_path")}
        if len(paths) > 1:
            failures += 1
            message = f"Round {sample}: click targets differ across servers; comparison invalid"
            comparison_errors.append(message)
            print(message, file=sys.stderr)
    (args.out / "validation.json").write_text(json.dumps({"failures": failures,
        "comparison_errors": comparison_errors}, indent=2) + "\n")
    print(f"{len(rows)} samples, {failures} failures; results: {args.out}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
