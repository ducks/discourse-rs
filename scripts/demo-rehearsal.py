#!/usr/bin/env python3
"""Dress rehearsal of docs/demo-script.md: the 38 demo steps in Chromium.

Each step clicks and types as the presenter does, as its persona, one
browser context per persona (Member and Second member stay open together
for the live updates). A step records ok or fail, a screenshot, how long it
took, and every problem seen while it ran: responses of 500 and up, page
and console errors, visible "not ported yet" or "Something went wrong"
text, images that failed to load, and an expected outcome that didn't
appear. A failure doesn't stop the run; a step that needs an earlier one's
outcome is skipped. Writes index.html and results.json to --out.
"""
import argparse
import base64
from datetime import datetime, timezone
import html
import json
import os
from pathlib import Path
import re
import sys
import time
import traceback
from urllib.parse import urlparse

from playwright.sync_api import Error as PlaywrightError
from playwright.sync_api import TimeoutError as PlaywrightTimeout
from playwright.sync_api import sync_playwright

PASSWORD = "password"
PERSONAS = {
    "Visitor": None,
    "Member": "user1",
    "Second member": "user0",
    "Moderator": "user2",
    "Admin": "admin",
    "Newcomer": None,  # signs up in step 13
}
# A topic the demo opens in step 2 (docs/demo-script.md).
LONG_TOPIC = 118
SEARCH_TERM = "discourse"
# Bundled plugins step 30 turns on (off on the demo database, as in
# Discourse) and those the plugin steps need that are on by default.
PLUGINS_TO_TURN_ON = ["assign_enabled", "policy_enabled", "user_notes_enabled", "discourse_gamification_enabled"]
PLUGINS_ON_BY_DEFAULT = ["solved_enabled", "topic_voting_enabled", "discourse_reactions_enabled"]
VISIBLE_ERRORS = re.compile(r"not ported yet[^\n]*|Something went wrong[^\n]*", re.I)
# Waits for an outcome. Live updates get longer: they ride the message bus.
WAIT = 10_000
LIVE_WAIT = 20_000
BROKEN_IMAGES = """() => [...document.images].filter(i => {
  if (!i.src || !i.complete || i.naturalWidth > 0) return false;
  const r = i.getBoundingClientRect();
  if (r.width === 0 || r.height === 0) return false;
  const s = getComputedStyle(i);
  return s.visibility !== 'hidden' && s.display !== 'none';
}).map(i => i.currentSrc || i.src)"""
# A PNG large enough to be lightboxed and a two second WebM, made by the browser itself so the
# script needs no fixtures or ffmpeg. WebM because Playwright's Chromium
# has no H.264 (docs/demo-script.md).
MAKE_PNG = """(label) => {
  const c = document.createElement('canvas'); c.width = 1600; c.height = 900;
  const g = c.getContext('2d');
  const grad = g.createLinearGradient(0, 0, 1600, 900);
  grad.addColorStop(0, '#0e76bd'); grad.addColorStop(1, '#f1592a');
  g.fillStyle = grad; g.fillRect(0, 0, 1600, 900);
  g.fillStyle = '#fff'; g.font = 'bold 80px sans-serif'; g.fillText(label, 60, 470);
  return c.toDataURL('image/png').split(',')[1];
}"""
MAKE_WEBM = """async (label) => {
  const c = document.createElement('canvas'); c.width = 320; c.height = 240;
  document.body.appendChild(c);
  const g = c.getContext('2d');
  const stream = c.captureStream(15);
  const rec = new MediaRecorder(stream, {mimeType: 'video/webm;codecs=vp8'});
  const chunks = [];
  rec.ondataavailable = e => chunks.push(e.data);
  const done = new Promise(r => rec.onstop = r);
  rec.start(100);
  const start = performance.now();
  await new Promise(resolve => {
    function frame() {
      const t = performance.now() - start;
      g.fillStyle = `hsl(${(t / 10) % 360}, 70%, 45%)`; g.fillRect(0, 0, 320, 240);
      g.fillStyle = '#fff'; g.font = 'bold 20px sans-serif'; g.fillText(label, 10, 120);
      if (t < 2000) requestAnimationFrame(frame); else resolve();
    }
    frame();
  });
  rec.stop(); await done;
  const buf = await new Blob(chunks, {type: 'video/webm'}).arrayBuffer();
  let s = ''; const bytes = new Uint8Array(buf);
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
  return btoa(s);
}"""


class StepFail(Exception):
    """The step's expected outcome didn't appear."""


class Step:
    def __init__(self, number, persona, title, needs, fn):
        self.number = number
        self.persona = persona
        self.title = title
        self.needs = needs
        self.fn = fn
        self.status = "pending"
        self.reason = ""
        self.url = ""
        self.duration = 0.0
        self.problems = []
        self.notes = []
        # Expected outcomes that didn't appear; a step may record one and go on.
        self.misses = []
        self.screenshot = None
        # The outcome a later step builds on, apart from any problems seen.
        self.achieved = False
        self.used = []
        self._responses = []

    def miss(self, what):
        self.misses.append(what)

    def problem(self, kind, text, url="", warning=False):
        entry = {"kind": kind, "text": text[:600], "url": url, "warning": warning}
        if entry not in self.problems:
            self.problems.append(entry)

    def as_json(self):
        return {
            "step": self.number,
            "persona": self.persona,
            "title": self.title,
            "status": self.status,
            "reason": self.reason,
            "url": self.url,
            "duration_s": round(self.duration, 2),
            "screenshot": self.screenshot,
            "problems": self.problems,
            "notes": self.notes,
            "needs": self.needs,
        }


class Persona:
    def __init__(self, run, name, username):
        self.run = run
        self.name = name
        self.username = username
        self.context = None
        self.page = None
        self.logged_in = False

    def open(self):
        if self.page:
            return self.page
        self.context = self.run.browser.new_context(
            viewport={"width": 1400, "height": 900},
            # Not the live demo's own colors: a fixed light scheme for comparable screenshots.
            color_scheme="light",
        )
        self.context.set_default_timeout(WAIT)
        self.page = self.context.new_page()
        self.watch(self.page)
        return self.page

    def watch(self, page):
        run = self.run

        def on_response(response):
            step = run.current
            if step is None:
                return
            if response.status >= 500:
                step._responses.append((response, self.name))
            elif response.status >= 400 and response.request.resource_type in ("xhr", "fetch", "document"):
                # A refusal may be a 4xx whose body says it isn't ported.
                step._responses.append((response, self.name))

        def on_console(message):
            if message.type != "error" or not run.current:
                return
            if message.text == "[object Event]":
                # htmx-ext-sse.js's source.onerror logs the Event when a live
                # stream drops, which a navigation away does every time. Kept,
                # but it doesn't fail the step alone.
                run.current.problem("console error", f"{self.name}: [object Event] (htmx SSE stream error, "
                                    "seen on navigating away)", page.url, warning=True)
                return
            run.current.problem("console error", f"{self.name}: {message.text}", page.url)

        def on_pageerror(error):
            if run.current:
                run.current.problem("page error", f"{self.name}: {error}", page.url)

        page.on("response", on_response)
        page.on("console", on_console)
        page.on("pageerror", on_pageerror)

    def use(self):
        """The persona's page, logged in, for the current step."""
        page = self.open()
        step = self.run.current
        if step and self not in step.used:
            step.used.append(self)
        if self.username and not self.logged_in:
            self.login(page)
        return page

    def login(self, page, from_path="/login"):
        page.goto(self.run.base + from_path)
        # A persona whose own flow already signed it in (the newcomer, after
        # signup and activation): /login sends a logged-in visitor home.
        if page.locator("#current-user").count() and page.locator("#current-user").first.is_visible():
            self.logged_in = True
            return
        if from_path != "/login":
            page.locator(".d-header .login-button, .d-header a[href='/login']").first.click()
            # The modal moves focus to the username field once it's drawn;
            # typing before that sent the password into the username field.
            page.locator("#login-account-name").first.wait_for(state="visible", timeout=WAIT)
            settle(page, 400)
        page.locator("#login-account-name").fill(self.username)
        page.locator("#login-account-password").fill(PASSWORD)
        page.locator("#login-button").click()
        try:
            page.locator("#current-user").wait_for(state="visible", timeout=WAIT)
        except PlaywrightTimeout:
            flash = page.locator(".login-fullpage .alert, #login-form .alert, .alert-error")
            said = flash.first.inner_text().strip() if flash.count() else "no message"
            raise StepFail(f"{self.name} ({self.username}) couldn't log in: {said}") from None
        self.logged_in = True


def brief(error):
    """A Playwright error's first line and the locator it was waiting for."""
    lines = str(error).splitlines()
    waiting = next((line.strip() for line in lines if "waiting for" in line), "")
    return lines[0] + (f" ({waiting})" if waiting else "")


def expect_visible(locator, what, timeout=WAIT):
    try:
        locator.first.wait_for(state="visible", timeout=timeout)
    except PlaywrightTimeout:
        raise StepFail(f"expected {what}, it didn't appear") from None
    return locator.first


def expect_gone(locator, what, timeout=WAIT):
    try:
        locator.first.wait_for(state="hidden", timeout=timeout)
    except PlaywrightTimeout:
        raise StepFail(f"expected {what}, it didn't happen") from None


def wait_until(page, js, arg, what, timeout=WAIT):
    try:
        page.wait_for_function(js, arg=arg, timeout=timeout)
    except PlaywrightTimeout:
        raise StepFail(f"expected {what}, it didn't happen") from None


def settle(page, ms=400):
    page.wait_for_timeout(ms)


class Run:
    def __init__(self, args):
        self.base = args.url.rstrip("/")
        self.out = Path(args.out)
        self.server_log = Path(args.server_log) if args.server_log else None
        self.started = datetime.now(timezone.utc)
        self.stamp = self.started.strftime("%Y%m%d%H%M%S")
        self.label = f"rehearsal {self.started.strftime('%Y-%m-%d %H:%M:%S')}Z"
        self.newcomer = f"rh{self.stamp}"
        self.watched_word = f"rhword{self.stamp}"
        self.current = None
        self.browser = None
        self.personas = {}
        self.state = {}
        self.left_behind = []
        self.cleanup = []
        self.steps = []
        self.files = {}

    # -- bookkeeping ----------------------------------------------------

    def persona(self, name):
        if name not in self.personas:
            username = self.newcomer if name == "Newcomer" else PERSONAS[name]
            self.personas[name] = Persona(self, name, username)
        return self.personas[name]

    def leave(self, what):
        if what not in self.left_behind:
            self.left_behind.append(what)

    def step(self, number, persona, title, needs=()):
        def register(fn):
            self.steps.append(Step(number, persona, title, list(needs), fn))
            return fn
        return register

    def by_number(self, number):
        return next(s for s in self.steps if s.number == number)

    def log_offset(self):
        try:
            return self.server_log.stat().st_size if self.server_log else 0
        except OSError:
            return 0

    def log_errors(self, offset):
        if not self.server_log:
            return []
        try:
            with open(self.server_log, "rb") as f:
                f.seek(offset)
                text = f.read().decode("utf-8", "replace")
        except OSError:
            return []
        text = re.sub(r"\x1b\[[0-9;]*m", "", text)
        lines = []
        for line in text.splitlines():
            level = " ERROR " if " ERROR " in line else " WARN " if " WARN " in line else None
            if not level:
                continue
            message = line.split(level, 1)[1].strip()
            if "request failed" in message:
                lines.append((message, False))
            elif message.startswith("discourse_rs::jobs") and "scheduled job" not in message \
                    and "call_discourse_hub" not in message:
                # A job the step's writes enqueued: unseen in the page, so a
                # warning (email jobs fail for want of SMTP on the demo).
                lines.append((message, True))
        return lines

    def run_step(self, step):
        missing = [n for n in step.needs if not self.by_number(n).achieved]
        if missing:
            step.status = "skipped"
            step.reason = "skipped (needs " + ", ".join(str(n) for n in missing) + ")"
            print(f"step {step.number:2}: {step.reason}")
            return
        self.current = step
        offset = self.log_offset()
        started = time.monotonic()
        try:
            step.fn(self, step)
            step.achieved = True
        except StepFail as e:
            step.misses.append(str(e))
        except PlaywrightTimeout as e:
            step.misses.append("timed out: " + brief(e))
        except PlaywrightError as e:
            step.misses.append("browser error: " + brief(e))
        except Exception as e:  # a bug in this script, reported as such
            step.misses.append(f"rehearsal script error: {type(e).__name__}: {e}")
            step.notes.append(traceback.format_exc())
        step.reason = "; ".join(step.misses)
        step.duration = time.monotonic() - started
        self.finish_step(step, offset)
        self.current = None
        print(f"step {step.number:2}: {step.status:4} {step.duration:5.1f}s {step.title}"
              + (f" -- {step.reason}" if step.reason else ""))

    def finish_step(self, step, offset):
        for persona in step.used or [self.persona(step.persona)]:
            page = persona.page
            if not page or page.is_closed():
                continue
            try:
                settle(page, 300)
                for text in VISIBLE_ERRORS.findall(page.locator("body").inner_text(timeout=3000)):
                    step.problem("visible error", f"{persona.name}: {text.strip()}", page.url)
                for src in page.evaluate(BROKEN_IMAGES):
                    step.problem("broken image", f"{persona.name}: {src}", page.url)
            except PlaywrightError as e:
                step.notes.append(f"couldn't inspect {persona.name}'s page: {e}")
        for response, who in step._responses:
            body = ""
            try:
                body = response.text()[:600]
            except PlaywrightError:
                pass
            req = response.request
            text = f"{who}: {req.method} {urlparse(response.url).path} {' '.join(body.split())[:300]}"
            if response.status >= 500 or "not ported yet" in body:
                step.problem(f"HTTP {response.status}", text, response.url)
            else:
                # A 4xx a page's script made: often expected (a refusal the
                # page shows), so kept as a warning; a page error that
                # follows it fails the step.
                step.problem(f"HTTP {response.status}", text, response.url, warning=True)
        step._responses = []
        for line, warning in self.log_errors(offset):
            step.problem("server log", line, warning=warning)
        main = self.persona(step.persona)
        page = main.page if main.page and not main.page.is_closed() else None
        if page is None and step.used:
            page = step.used[0].page
        if page and not page.is_closed():
            step.url = step.url or page.url
            name = f"step-{step.number:02}.png"
            try:
                page.screenshot(path=str(self.out / name))
                step.screenshot = name
            except PlaywrightError as e:
                step.notes.append(f"no screenshot: {e}")
        if step.misses:
            step.status = "fail"
        elif any(not p["warning"] for p in step.problems):
            step.status = "fail"
            # The first problem names the failure; a failed resource load is
            # usually the echo of an HTTP error already listed.
            serious = [p for p in step.problems if not p["warning"]]
            first = next((p for p in serious if "Failed to load resource" not in p["text"]), serious[0])
            step.reason = f"{first['kind']}: {first['text']}"
        else:
            step.status = "ok"

    # -- fixtures made in the browser -----------------------------------

    def make_files(self):
        page = self.browser.new_page()
        page.set_content("<html><body></body></html>")
        png = self.out / "fixtures" / f"rehearsal-{self.stamp}.png"
        webm = self.out / "fixtures" / f"rehearsal-{self.stamp}.webm"
        png.parent.mkdir()
        png.write_bytes(base64.b64decode(page.evaluate(MAKE_PNG, f"rehearsal {self.stamp}")))
        webm.write_bytes(base64.b64decode(page.evaluate(MAKE_WEBM, f"rehearsal {self.stamp}")))
        page.close()
        self.files = {"png": str(png), "webm": str(webm)}

    # -- output ---------------------------------------------------------

    def build(self):
        try:
            about = json.loads(self.browser_get("/about.json"))
            version = about.get("about", {}).get("version")
        except Exception:
            version = None
        binary = Path.home() / "dev/discourse-rs/tmp/demo/discourse-rs"
        target = os.readlink(binary) if binary.is_symlink() else None
        parts = []
        if target:
            parts.append(f"binary {target} (tmp/demo/discourse-rs symlink; the server doesn't expose its build)")
        if version:
            parts.append(f"reports Discourse version {version}")
        return "; ".join(parts) or "unknown"

    def browser_get(self, path):
        ctx = self.browser.new_context()
        try:
            return ctx.request.get(self.base + path).text()
        finally:
            ctx.close()

    def write(self, build):
        counts = {k: sum(1 for s in self.steps if s.status == k) for k in ("ok", "fail", "skipped")}
        results = {
            "url": self.base,
            "build": build,
            "started": self.started.isoformat(),
            "finished": datetime.now(timezone.utc).isoformat(),
            "label": self.label,
            "newcomer": self.newcomer,
            "watched_word": self.watched_word,
            "summary": counts,
            "steps": [s.as_json() for s in self.steps],
            "cleanup": self.cleanup,
            "left_behind": self.left_behind,
        }
        (self.out / "results.json").write_text(json.dumps(results, indent=2) + "\n")
        (self.out / "index.html").write_text(render_html(results))
        return counts


def render_html(r):
    e = html.escape
    rows = []
    for s in r["steps"]:
        thumb = ""
        if s["screenshot"]:
            thumb = f'<a href="{e(s["screenshot"])}"><img src="{e(s["screenshot"])}" loading="lazy" alt="step {s["step"]}"></a>'
        problems = "".join(
            f'<li class="{"warn" if p["warning"] else ""}"><b>{e(p["kind"])}</b>{" (warning)" if p["warning"] else ""} {e(p["text"])}'
            + (f' <span class="u">{e(p["url"])}</span>' if p["url"] else "") + "</li>"
            for p in s["problems"])
        notes = "".join(f"<pre>{e(n)}</pre>" for n in s["notes"])
        rows.append(f"""<tr class="{e(s['status'])}">
<td class="n">{s['step']}</td>
<td><div class="t">{e(s['title'])}</div><div class="p">{e(s['persona'])}</div></td>
<td class="s"><span class="badge">{e(s['status'])}</span><div class="d">{s['duration_s']}s</div></td>
<td>{e(s['reason'])}{f'<div class="u">{e(s["url"])}</div>' if s['url'] else ''}
{f'<ul>{problems}</ul>' if problems else ''}{f'<details><summary>notes</summary>{notes}</details>' if notes else ''}</td>
<td class="th">{thumb}</td></tr>""")
    c = r["summary"]
    left = "".join(f"<li>{e(x)}</li>" for x in r["left_behind"]) or "<li>nothing</li>"
    clean = "".join(f"<li>{e(x)}</li>" for x in r["cleanup"]) or "<li>nothing to clean</li>"
    return f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Demo rehearsal {e(r['started'][:19])}</title>
<style>
:root {{ --bg:#fff; --fg:#1d1d1d; --mute:#666; --line:#ddd; --ok:#1a7f37; --fail:#c62828; --skip:#8a6d00; --row:#fafafa; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg:#151515; --fg:#e8e8e8; --mute:#9a9a9a; --line:#333; --ok:#4cc26a; --fail:#ff6b6b; --skip:#e0b400; --row:#1c1c1c; }} }}
body {{ background:var(--bg); color:var(--fg); font:14px/1.45 system-ui, sans-serif; margin:0; padding:16px; }}
h1 {{ font-size:20px; margin:0 0 8px; }}
.meta {{ color:var(--mute); margin-bottom:12px; }}
.meta div {{ overflow-wrap:anywhere; }}
.sum {{ display:flex; gap:12px; margin:12px 0; flex-wrap:wrap; }}
.sum span {{ border:1px solid var(--line); border-radius:6px; padding:6px 12px; font-weight:600; }}
.sum .ok {{ color:var(--ok); }} .sum .fail {{ color:var(--fail); }} .sum .skipped {{ color:var(--skip); }}
.wrap {{ overflow-x:auto; }}
table {{ border-collapse:collapse; width:100%; min-width:720px; }}
td {{ border-top:1px solid var(--line); padding:8px; vertical-align:top; }}
tr:nth-child(even) td {{ background:var(--row); }}
td.n {{ font-weight:700; width:2em; }}
.t {{ font-weight:600; }} .p, .d, .u {{ color:var(--mute); font-size:12px; overflow-wrap:anywhere; }}
.badge {{ font-weight:700; text-transform:uppercase; font-size:12px; }}
tr.ok .badge {{ color:var(--ok); }} tr.fail .badge {{ color:var(--fail); }} tr.skipped .badge {{ color:var(--skip); }}
td.th img {{ width:220px; border:1px solid var(--line); border-radius:4px; }}
ul {{ margin:6px 0 0; padding-left:18px; }} li {{ overflow-wrap:anywhere; }} li.warn {{ color:var(--mute); }}
pre {{ white-space:pre-wrap; font-size:11px; }}
</style></head><body>
<h1>Demo rehearsal</h1>
<div class="meta">
<div>URL: {e(r['url'])}</div>
<div>Build: {e(r['build'])}</div>
<div>Started {e(r['started'])}, finished {e(r['finished'])}</div>
<div>Label: {e(r['label'])}; newcomer {e(r['newcomer'])}; watched word {e(r['watched_word'])}</div>
</div>
<div class="sum"><span class="ok">{c['ok']} ok</span><span class="fail">{c['fail']} fail</span><span class="skipped">{c['skipped']} skipped</span></div>
<div class="wrap"><table>
<tbody>
{''.join(rows)}
</tbody></table></div>
<h2>Cleanup</h2><ul>{clean}</ul>
<h2>Left behind</h2><ul>{left}</ul>
</body></html>
"""


# --------------------------------------------------------------------------
# Shared actions

def scroll_to_topic_end(page):
    """Scroll as a presenter does until the post stream has loaded its last
    post and drawn the footer (Ember draws it only then)."""
    footer = page.locator("#topic-footer-buttons, .topic-footer-main-buttons")
    for _ in range(60):
        if footer.count() and footer.first.is_visible():
            return
        page.mouse.wheel(0, 5000)
        settle(page, 300)
    expect_visible(footer, "the topic footer after scrolling to the end")


def open_composer_reply(page):
    page.locator("#topic-footer-buttons .create, .topic-footer-main-buttons button.create").first.click()
    return expect_visible(page.locator("#reply-control.open .d-editor-input"), "the composer to open")


def type_in_composer(page, text, delay=0):
    editor = page.locator("#reply-control .d-editor-input")
    editor.click()
    if delay:
        editor.press_sequentially(text, delay=delay)
    else:
        editor.fill(text)
    return editor


def upload_in_composer(page, path, what):
    editor = page.locator("#reply-control .d-editor-input")
    page.locator("#reply-control input[type=file]").first.set_input_files(path)
    wait_until(page,
               "() => { const v = document.querySelector('#reply-control .d-editor-input').value;"
               " return /upload:\\/\\//.test(v) && !/\\[Uploading/.test(v); }",
               None, f"the {what} to finish uploading into the composer", timeout=30_000)
    return editor.input_value()


# When each page last created a post. Discourse refuses a member's next
# topic within rate_limit_create_topic (15s) and reply within
# rate_limit_create_post (5s); a presenter talking between steps never hits
# that, so the script waits as long as they would.
LAST_POST = {}
POST_GAP = 16


def submit_composer(page):
    wait = POST_GAP - (time.monotonic() - LAST_POST.get(id(page), -POST_GAP))
    if wait > 0:
        page.wait_for_timeout(wait * 1000)
    page.locator("#reply-control .save-or-cancel .create").click()
    expect_gone(page.locator("#reply-control.open"), "the composer to close after submitting", timeout=20_000)
    LAST_POST[id(page)] = time.monotonic()


def post_with(page, marker):
    return page.locator(".topic-post", has=page.locator(".cooked", has_text=marker))


def topic_url(url):
    m = re.search(r"(/t/[^/]+/\d+)", url)
    return m.group(1) if m else url


def open_user_menu(page):
    if not page.locator(".user-menu.revamped").first.is_visible():
        page.locator("#toggle-current-user").click()
    return expect_visible(page.locator(".user-menu.revamped"), "the user menu to open")


def close_menus(page):
    page.keyboard.press("Escape")
    settle(page, 200)


def review_note(run, item):
    note = f"Note from the {run.label}"
    item.locator(".reviewable-note-form__textarea").fill(note)
    item.locator("button", has_text=re.compile(r"^\s*Add note")).click()
    expect_visible(item.locator(".reviewable-timeline", has_text=note), "the note in the review item's timeline")


def admin_user_page(run, page, username, listing="active"):
    page.goto(run.base + f"/admin/users/list/{listing}")
    search = expect_visible(page.locator(".d-filter-controls__input"), "the admin users list's filter")
    search.press_sequentially(username, delay=30)
    link = page.locator(f"#main-outlet a[href^='/admin/users/'][href$='/{username}']")
    expect_visible(link, f"{username} in the admin users list")
    # The list redraws as the filter settles; click the row that stays.
    page.wait_for_timeout(1200)
    link.first.click()
    page.wait_for_url(f"**/admin/users/*/{username}", timeout=WAIT)
    expect_visible(page.locator("#main-outlet .silence-user, #main-outlet .suspend-user"), f"{username}'s admin page")


def open_channel(run, page, path, what):
    """A channel in full-page chat, joined if the persona only previews it."""
    page.goto(run.base + path)
    join = page.locator(".chat-channel .toggle-channel-membership-button.-join")
    composer = page.locator(".chat-channel .chat-composer__input")
    expect_visible(composer.or_(join), what)
    if join.count() and join.first.is_visible():
        join.first.click()
    return expect_visible(composer, f"{what}, joined")


def send_chat(scope, text, what):
    """A message typed and sent in the composer under scope (a channel or
    the thread pane); its container once drawn."""
    composer = scope.locator(".chat-composer__input").first
    composer.click()
    composer.press_sequentially(text, delay=10)
    composer.press("Enter")
    return expect_visible(scope.locator(".chat-message-container", has_text=text), what)


def chat_message_action(message, value, what):
    """A secondary action from the message's hover toolbar (its "..." menu)."""
    page = message.page
    summary = message.locator(".chat-message-actions .more-actions-chat summary")
    row = message.locator(f".chat-message-actions .select-kit-row[data-value='{value}']")
    # The toolbar follows the pointer after a moment (and a message drawn
    # anew as it is processed takes an open menu with it): as a presenter
    # would, point at the message again until its menu holds the action.
    for attempt in range(4):
        message.scroll_into_view_if_needed()
        page.mouse.move(5, 5)
        message.hover()
        try:
            summary.first.wait_for(state="visible", timeout=3000)
            summary.first.click(timeout=3000)
            row.first.click(timeout=3000)
            return
        except PlaywrightError:  # a timeout, or the message swapped out mid-action
            opened = page.locator(".more-actions-chat[open] summary")
            try:
                if opened.count():
                    opened.first.click(timeout=2000)
            except PlaywrightError:
                pass
    raise StepFail(f"expected {what}, it didn't appear")


def toggle_plugin(page, setting, on):
    """A bundled plugin's switch on /admin/plugins (the page already open);
    whether it changed."""
    switch = page.locator(f"[data-enabled-setting='{setting}']")
    switch.first.wait_for(state="attached", timeout=WAIT)
    if (switch.first.get_attribute("aria-checked") == "true") == on:
        return False
    page.locator(".admin-plugins-list__row", has=switch).locator(".d-toggle-switch__checkbox-slider").click()
    wait_until(page, "([s, on]) => document.querySelector(`[data-enabled-setting='${s}']`).getAttribute('aria-checked') === on",
               [setting, "true" if on else "false"], f"{setting} switched {'on' if on else 'off'}")
    return True


# --------------------------------------------------------------------------
# The demo, in order (docs/demo-script.md)

def define_steps(run):
    step = run.step

    @step(1, "Visitor", "/latest, scroll, then /categories, a category, /top")
    def _(run, s):
        page = run.persona("Visitor").use()
        page.goto(run.base + "/latest")
        rows = page.locator(".topic-list .topic-list-item")
        expect_visible(rows, "the latest topic list")
        before = rows.count()
        for _ in range(6):
            page.mouse.wheel(0, 4000)
            settle(page, 400)
        try:
            page.wait_for_function("n => document.querySelectorAll('.topic-list .topic-list-item').length > n",
                                   arg=before, timeout=WAIT)
        except PlaywrightTimeout:
            more = page.locator(".more-topics a[rel=next]").count()
            s.miss(f"scrolling /latest loaded no more topics (still {rows.count()})"
                   + ("; it shows a 'Next page' link instead" if more else ""))
        page.mouse.wheel(0, -100000)
        page.locator("#navigation-bar .nav-item_categories a, #navigation-bar li.categories a").first.click()
        page.wait_for_url("**/categories", timeout=WAIT)
        cat = expect_visible(page.locator(".category-list a.category-title-link, .category-list .category-name a, table.category-list td.category a[href^='/c/']"),
                             "the category list")
        cat.click()
        page.wait_for_url("**/c/**", timeout=WAIT)
        expect_visible(page.locator(".topic-list .topic-list-item"), "the category's topic list")
        page.goto(run.base + "/top")
        expect_visible(page.locator(".topic-list .topic-list-item"), "the top topics")

    @step(2, "Visitor", f"long topic /t/{LONG_TOPIC}: scroll through the posts to the end")
    def _(run, s):
        page = run.persona("Visitor").use()
        page.goto(run.base + f"/t/{LONG_TOPIC}")
        posts = page.locator(".topic-post")
        expect_visible(posts, "the topic's posts")
        before = posts.count()
        total = page.evaluate("() => { const t = document.querySelector('.timeline-replies, .topic-timeline .timeline-scroller-content'); return t ? t.innerText : '' }")
        # The post stream loads the next chunk as its end comes into view,
        # down to the last post and the footer Ember draws only then.
        scroll_to_topic_end(page)
        s.notes.append(f"{before} posts drawn on load, {posts.count()} after scrolling to the end; timeline says {total!r}")
        # The corpus has no lightbox image or linked quote in a public topic
        # (its images are unresolved upload:// links, its quotes carry no
        # data-post), so the lightbox is shown on the member's own upload in
        # step 6.
        if posts.count() <= before:
            raise StepFail(f"scrolling /t/{LONG_TOPIC} loaded no posts past the first {before}")

    @step(3, "Visitor", f"header search for {SEARCH_TERM!r}, then the full search page with a filter")
    def _(run, s):
        page = run.persona("Visitor").use()
        page.goto(run.base + "/latest")
        button = page.locator("#search-button")
        if button.count() and button.is_visible():
            button.click()
        field = expect_visible(page.locator(".search-menu input[type=search], .search-menu input#icon-search-input, .search-menu .search-term__input input, .welcome-banner__search-menu input"),
                               "the search field")
        field.click()
        field.press_sequentially(SEARCH_TERM, delay=40)
        expect_visible(page.locator(".search-menu .results .search-link"), "suggestions in the search menu")
        # Enter searches topics and posts; the results' "More..." opens the
        # full page (as a second Enter would).
        field.press("Enter")
        expect_visible(page.locator(".search-menu .results .search-result-topic .search-link"), "topic results in the search menu")
        expect_visible(page.locator(".search-menu .results").get_by_text(re.compile(r"^\s*More")), "the results' More link").click()
        page.wait_for_url("**/search?**", timeout=WAIT)
        expect_visible(page.locator(".fps-result"), "results on the full search page")
        page.locator(".advanced-filters__toggle").click()
        status = expect_visible(page.locator("#search-status-options-header"), "the advanced filters")
        status.click()
        option = expect_visible(page.locator("#search-status-options .select-kit-row").filter(has_text=re.compile("open", re.I)),
                                "the 'Where topics: are open' option")
        option.click()
        page.locator(".search-cta").first.click()
        wait_until(page, "() => /status(:|%3A)open/.test(location.search)", None, "the filter in the search URL")
        expect_visible(page.locator(".fps-result"), "filtered results")

    @step(4, "Visitor", "a tag page, a user profile")
    def _(run, s):
        page = run.persona("Visitor").use()
        page.goto(run.base + "/latest")
        tag = expect_visible(page.locator("#d-sidebar .sidebar-section[data-section-name=tags] a.sidebar-section-link[href^='/tag/']"),
                             "a tag in the sidebar")
        tag.click()
        page.wait_for_url("**/tag/**", timeout=WAIT)
        expect_visible(page.locator(".topic-list .topic-list-item"), "the tag's topic list")
        avatar = page.locator(".topic-list .posters a[data-user-card], .topic-list .posters a").first
        avatar.click()
        card = expect_visible(page.locator("#user-card .user-card, .user-card.show, #user-card"), "the user card")
        card.locator("a.user-profile-link, .names__primary a, .user-card-avatar a").first.click()
        page.wait_for_url("**/u/**", timeout=WAIT)
        expect_visible(page.locator(".user-main .user-profile-names, .user-main .details"), "the user profile")

    @step(5, "Member", "logs in from a topic and comes back to it")
    def _(run, s):
        member = run.persona("Member")
        page = member.open()
        s.used.append(member)
        page.goto(run.base + f"/t/{LONG_TOPIC}")
        expect_visible(page.locator(".topic-post"), "the topic")
        topic = topic_url(page.url)
        page.locator(".d-header a[href='/login'], .d-header .login-button").first.click()
        # As Persona.login: the modal moves focus to the username field
        # once it's drawn, which can take the password typed before it.
        page.locator("#login-account-name").first.wait_for(state="visible", timeout=WAIT)
        settle(page, 400)
        page.locator("#login-account-name").fill(member.username)
        page.locator("#login-account-password").fill(PASSWORD)
        page.locator("#login-button").click()
        expect_visible(page.locator("#current-user"), "the member logged in")
        member.logged_in = True
        wait_until(page, "t => location.pathname.startsWith(t)", topic, f"to be back on {topic}")
        expect_visible(page.locator(".topic-post"), "the topic again")

    @step(6, "Member", "reply with formatting, @user0, an emoji and an uploaded image; it appears without a reload")
    def _(run, s):
        member = run.persona("Member")
        page = member.use()
        # Second member opens a page now, to receive step 7's notification live.
        second = run.persona("Second member").use()
        second.goto(run.base + "/latest")
        page.goto(run.base + f"/t/{LONG_TOPIC}")
        expect_visible(page.locator(".topic-post"), "the topic")
        scroll_to_topic_end(page)
        # A rerun would stack the member's replies at the end of the same
        # topic until max_consecutive_replies refuses one; a presenter picks
        # another topic then, and so does the rehearsal.
        last = page.locator(".topic-post").last.locator(".names a[data-user-card]").first.get_attribute("data-user-card")
        if last == member.username:
            page.goto(run.base + "/latest")
            rows = page.locator(".topic-list .topic-list-item").filter(
                has_not=page.locator(f".posters a.latest[data-user-card='{member.username}']")).filter(
                has_not=page.locator(".title", has_text=re.compile("rehearsal", re.I))).filter(
                has_not=page.locator(".topic-statuses .d-icon-lock, .topic-statuses .d-icon-thumbtack"))
            expect_visible(rows, "a topic in /latest the member didn't post last in").locator("a.title").first.click()
            page.wait_for_url("**/t/**", timeout=WAIT)
            expect_visible(page.locator(".topic-post"), "the topic")
            scroll_to_topic_end(page)
            s.notes.append(f"user1 posted last in /t/{LONG_TOPIC} (an earlier rehearsal), so this reply went to {topic_url(page.url)}")
        marker = f"{run.label} reply"
        page.evaluate("() => { window.__rehearsalNoReload = true }")
        open_composer_reply(page)
        editor = type_in_composer(page, f"**{marker}** with _formatting_ for @user0 ", delay=20)
        settle(page, 600)
        close_menus(page)
        editor.press("End")
        editor.press_sequentially(":tada: ", delay=20)
        settle(page, 300)
        close_menus(page)
        editor.press("End")
        editor.press("Enter")
        upload_in_composer(page, run.files["png"], "image")
        submit_composer(page)
        post = expect_visible(post_with(page, marker), "the reply in the topic", timeout=LIVE_WAIT)
        if not page.evaluate("() => window.__rehearsalNoReload === true"):
            raise StepFail("the page reloaded to show the reply")
        cooked = post.locator(".cooked")
        checks = {
            "bold text": "strong",
            "italic text": "em",
            "a mention of @user0": "a.mention",
            "an emoji": "img.emoji",
            "the uploaded image": "img:not(.emoji):not(.avatar)",
        }
        absent = [what for what, sel in checks.items() if cooked.locator(sel).count() == 0]
        run.state["reply_url"] = page.url
        run.state["reply_marker"] = marker
        run.state["reply_post_id"] = post.locator("article").first.get_attribute("data-post-id")
        run.state["topic_path"] = topic_url(page.url)
        run.leave(f"a reply '{marker}' with an uploaded image in {topic_url(page.url)} by user1")
        if absent:
            raise StepFail("the reply is missing " + ", ".join(absent))
        # Post processing (the job worker) wraps the image in a lightbox and
        # swaps the post in live; open it as the demo does.
        try:
            post.locator(".cooked a.lightbox").first.wait_for(state="attached", timeout=LIVE_WAIT)
        except PlaywrightTimeout:
            raise StepFail("the reply's image never got its lightbox wrapper (post processing)")
        lightbox = post.locator(".cooked a.lightbox").first
        lightbox.scroll_into_view_if_needed()
        lightbox.click()
        expect_visible(page.locator(".pswp"), "the lightbox to open")
        settle(page, 800)
        page.keyboard.press("Escape")
        expect_gone(page.locator(".pswp"), "the lightbox to close")

    @step(7, "Second member", "the mention notification arrives; open it, like the reply", needs=[6])
    def _(run, s):
        page = run.persona("Second member").use()
        live = True
        try:
            page.locator("#current-user .badge-notification, #current-user .unread-notifications, #current-user .d-icon-at").first.wait_for(state="visible", timeout=LIVE_WAIT)
        except PlaywrightTimeout:
            live = False
        if not live:
            page.reload()
        open_user_menu(page)
        topic = run.state["topic_path"]
        item = expect_visible(page.locator(f".user-menu li.notification.mentioned a[href^='{topic}']"),
                              "the mention notification in the user menu")
        item.click()
        post = expect_visible(post_with(page, run.state["reply_marker"]), "the mentioning reply")
        like = post.locator("button.toggle-like, button.like, .post-action-menu__like, .discourse-reactions-reaction-button").first
        like.click()
        expect_visible(post.locator("button.has-like, button.my-likes, .post-action-menu__like.has-like, .discourse-reactions-reaction-button.has-reacted, .discourse-reactions-counter"),
                       "the reply shown as liked")
        run.leave("user0's like on that reply")
        if not live:
            raise StepFail("the notification didn't arrive live: no unread badge on the avatar until a reload")

    @step(8, "Member", "new topic with a poll and a checklist; vote; tick a checklist item")
    def _(run, s):
        page = run.persona("Member").use()
        page.goto(run.base + "/latest")
        page.locator("#create-topic").click()
        title = expect_visible(page.locator("#reply-title"), "the new topic composer")
        topic_title = f"{run.label} poll and checklist"
        title.press_sequentially(topic_title, delay=10)
        # Labelled: Discourse refuses a body the member posted within
        # unique_posts_mins, so a second run soon after would be refused.
        body = (f"A topic for the demo {run.label}.\n\n"
                "[poll type=regular results=always]\n* Rust\n* Ruby\n* Both\n[/poll]\n\n"
                "[ ] write the demo script\n[ ] rehearse it\n")
        type_in_composer(page, body)
        submit_composer(page)
        page.wait_for_url("**/t/**", timeout=WAIT)
        expect_visible(page.locator("h1 .fancy-title, #topic-title h1, .title-wrapper h1").filter(has_text="poll and checklist"),
                       "the new topic")
        run.state["new_topic"] = topic_url(page.url)
        run.state["new_topic_title"] = topic_title
        run.leave(f"topic '{topic_title}' by user1 at {run.state['new_topic']} (step 26 unlists it)")
        poll = expect_visible(page.locator(".poll"), "the poll drawn in the topic")
        poll.locator("li[data-poll-option-id]").first.click()
        voted = poll.locator(".results, .poll-results, li.chosen, .d-icon-circle-check")
        expect_visible(voted, "the vote counted (results or the chosen option)")
        box = expect_visible(page.locator(".cooked .chcklst-box"), "the checklist")
        box.click()
        expect_visible(page.locator(".cooked .chcklst-box.checked, .cooked .chcklst-box.fa-square-check-o"), "the checklist item ticked")
        # The tick is drawn at once; a reload shows whether it was saved.
        settle(page, 1000)
        page.reload()
        expect_visible(page.locator(".cooked .chcklst-box"), "the checklist after a reload")
        if page.locator(".cooked .chcklst-box.checked, .cooked .chcklst-box.fa-square-check-o").count() == 0:
            s.miss("the checklist tick was drawn but not saved (gone after a reload)")

    @step(9, "Member", "bookmark a post, then the bookmarks list in the user menu")
    def _(run, s):
        page = run.persona("Member").use()
        page.goto(run.base + f"/t/{LONG_TOPIC}")
        expect_visible(page.locator(".topic-post"), "the topic")
        target = None
        for i in range(page.locator(".topic-post").count()):
            candidate = page.locator(".topic-post").nth(i)
            if candidate.locator(".post-controls button.bookmarked, .post-action-menu__bookmark.bookmarked").count() == 0:
                target = candidate
                break
        if target is None:
            raise StepFail(f"every post in /t/{LONG_TOPIC} is already bookmarked by user1")
        target.scroll_into_view_if_needed()
        target.hover()
        more = target.locator(".post-controls .show-more-actions, .post-action-menu__show-more")
        bookmark = target.locator(".post-controls button.bookmark, .post-controls .bookmark-menu-trigger, .post-action-menu__bookmark")
        if not bookmark.first.is_visible() and more.count():
            more.first.click()
        bookmark.first.click()
        # The bookmark menu opens; its first choice saves without a reminder.
        menu = page.locator(".bookmark-menu-content, .bookmark-options-menu, .fk-d-menu .bookmark-menu__body")
        try:
            menu.first.wait_for(state="visible", timeout=3000)
            page.keyboard.press("Escape")
        except PlaywrightTimeout:
            pass
        expect_visible(target.locator("button.bookmarked, .post-action-menu__bookmark.bookmarked, .bookmark-menu-trigger.bookmarked"),
                       "the post shown as bookmarked")
        post_number = target.locator("article").first.get_attribute("id")
        run.leave(f"user1's bookmark on {post_number} of /t/{LONG_TOPIC}")
        open_user_menu(page)
        page.locator("#user-menu-button-bookmarks").click()
        expect_visible(page.locator("#quick-access-bookmarks li.bookmark, #quick-access-bookmarks li"), "the bookmarks list in the user menu")

    @step(10, "Member", "private message to Second member; Second member replies")
    def _(run, s):
        page = run.persona("Member").use()
        page.goto(run.base + "/u/user0/summary")
        expect_visible(page.locator(".user-main"), "user0's profile")
        page.locator(".user-main button.compose-pm").first.click()
        title = page.locator("#reply-control.open #reply-title")
        try:
            title.wait_for(state="visible", timeout=4000)
        except PlaywrightTimeout:
            title = None
        if title is not None:
            pm_title = f"{run.label} private message"
            title.press_sequentially(pm_title, delay=10)
            type_in_composer(page, f"Hello from the {run.label}. Please reply.", delay=5)
            submit_composer(page)
            page.wait_for_url("**/t/**", timeout=WAIT)
            expect_visible(page.locator(".topic-post .cooked", has_text="Please reply"), "the message sent")
            run.state["pm"] = topic_url(page.url)
            run.leave(f"private message '{pm_title}' between user1 and user0 at {run.state['pm']}")
        else:
            s.miss("the Message button on user0's profile opens no composer (a new message isn't ported: "
                   "static/js/composer.js openNewMessage refuses 'the message composer')")
            # Go on with a message the two already share, from the user menu.
            open_user_menu(page)
            page.locator("#user-menu-button-messages").click()
            shared = page.locator("#quick-access-messages li a", has_text=re.compile("user0", re.I))
            try:
                shared.first.wait_for(state="visible", timeout=4000)
            except PlaywrightTimeout:
                raise StepFail("no message shared with user0 in the member's user menu to go on with") from None
            shared.first.click()
            page.wait_for_url("**/t/**", timeout=WAIT)
            expect_visible(page.locator(".topic-post"), "the shared message")
            run.state["pm"] = topic_url(page.url)
            s.notes.append(f"went on with the existing message {run.state['pm']}")
        second = run.persona("Second member").use()
        second.goto(run.base + "/latest")
        open_user_menu(second)
        second.locator("#user-menu-button-messages").click()
        item = expect_visible(second.locator(f".user-menu a[href^='{run.state['pm']}']"), "the message in Second member's user menu")
        item.click()
        expect_visible(second.locator(".topic-post"), "the message opened")
        open_composer_reply(second)
        answer = f"Reply from user0, {run.label}"
        type_in_composer(second, answer, delay=5)
        submit_composer(second)
        expect_visible(post_with(second, answer), "the reply in the message")
        run.leave(f"user0's reply '{answer}' in {run.state['pm']}")
        try:
            post_with(page, answer).first.wait_for(state="visible", timeout=LIVE_WAIT)
        except PlaywrightTimeout:
            s.miss("Member didn't see Second member's reply arrive without a reload")

    @step(11, "Member", "preferences: switch to the dark palette and back")
    def _(run, s):
        page = run.persona("Member").use()
        page.goto(run.base + "/latest")
        open_user_menu(page)
        page.locator("#user-menu-button-profile").click()
        expect_visible(page.locator("#quick-access-profile a[href$='/preferences'], .user-menu a[href$='/preferences']"), "the preferences link").click()
        page.wait_for_url("**/preferences/**", timeout=WAIT)
        page.locator("a[href$='/preferences/interface']").first.click()
        page.wait_for_url("**/preferences/interface", timeout=WAIT)
        light_bg = page.evaluate("() => getComputedStyle(document.documentElement).backgroundColor")
        palettes = page.locator(".control-group.color-scheme")
        if palettes.count() == 0:
            # Palettes show only when one is user selectable
            # (showColorSchemeSelector); an admin marks them.
            raise StepFail("no color palette chooser on the interface preferences: no palette is "
                           "user selectable (color_schemes.user_selectable)")

        def choose(pattern, what):
            palettes.locator("details.select-kit summary").first.click()
            expect_visible(page.locator(".select-kit-row").filter(has_text=pattern), what).click()
            page.locator(".save-changes").first.click()
            expect_visible(page.locator(".saved"), "the preferences saved")
            page.reload()

        choose(re.compile("dark", re.I), "a dark palette in the chooser")
        wait_until(page, "bg => getComputedStyle(document.documentElement).backgroundColor !== bg", light_bg, "the page drawn dark")
        s.notes.append(f"background {light_bg} -> {page.evaluate('() => getComputedStyle(document.documentElement).backgroundColor')}")
        choose(re.compile(r"^(?!.*dark)", re.I), "a light palette in the chooser")
        wait_until(page, "bg => getComputedStyle(document.documentElement).backgroundColor === bg", light_bg, "the page back to light")

    @step(12, "Member", "upload a video in a reply; placeholder, then it plays")
    def _(run, s):
        page = run.persona("Member").use()
        page.goto(run.base + run.state.get("new_topic", f"/t/{LONG_TOPIC}"))
        expect_visible(page.locator(".topic-post"), "the topic")
        marker = f"{run.label} video"
        open_composer_reply(page)
        editor = type_in_composer(page, f"{marker}\n\n", delay=5)
        editor.press("End")
        upload_in_composer(page, run.files["webm"], "video")
        submit_composer(page)
        post = expect_visible(post_with(page, marker), "the video reply", timeout=LIVE_WAIT)
        run.leave(f"a reply '{marker}' with an uploaded WebM in {topic_url(page.url)}")
        placeholder = expect_visible(post.locator(".video-placeholder-container, .video-placeholder-overlay"), "the video placeholder")
        placeholder.click()
        video = expect_visible(post.locator("video"), "the video player")
        try:
            page.wait_for_function("v => v.currentTime > 0 || (!v.paused && v.readyState > 2)", arg=video.element_handle(), timeout=WAIT)
        except PlaywrightTimeout:
            err = page.evaluate("v => v.error ? `${v.error.code} ${v.error.message}` : `paused=${v.paused} readyState=${v.readyState} src=${v.currentSrc}`",
                                video.element_handle())
            raise StepFail(f"the video didn't play ({err})") from None

    @step(13, "Newcomer", "sign up (no SMTP, so the account waits for activation)")
    def _(run, s):
        newcomer = run.persona("Newcomer")
        page = newcomer.open()
        s.used.append(newcomer)
        page.goto(run.base + "/latest")
        page.locator(".d-header .sign-up-button").click()
        page.wait_for_url("**/signup", timeout=WAIT)
        email = f"{run.newcomer}@example.com"
        if page.locator(".code-login-form").count():
            page.locator(".code-login-form input[type=email]").press_sequentially(email, delay=20)
            page.locator(".code-login-form__continue").click()
            code = page.locator(".code-login-form input[name=code], input[autocomplete=one-time-code], .code-login-form__code")
            try:
                code.first.wait_for(state="visible", timeout=WAIT)
            except PlaywrightTimeout:
                raise StepFail("the signup page is the emailed-code form (enable_local_logins_via_code), "
                               "and sending the code failed") from None
            raise StepFail("the signup page asks for a code sent by email, and the demo has no SMTP")
        page.locator("#new-account-email").press_sequentially(email, delay=20)
        page.locator("#new-account-username").fill("")
        page.locator("#new-account-username").press_sequentially(run.newcomer, delay=20)
        expect_visible(page.locator("#username-validation.good"), "the username accepted")
        name = page.locator("#new-account-name")
        if name.count() and name.is_visible():
            name.press_sequentially("Rehearsal Newcomer", delay=10)
        page.locator("#new-account-password").press_sequentially(f"Rehearsal-{run.stamp}-pw", delay=10)
        expect_visible(page.locator("#password-validation.good"), "the password accepted")
        page.locator(".signup-page-cta__signup").click()
        page.wait_for_url("**/u/account-created", timeout=WAIT)
        expect_visible(page.locator(".account-created"), "the account-created page")
        run.state["newcomer_password"] = f"Rehearsal-{run.stamp}-pw"
        run.leave(f"user {run.newcomer} ({email})")

    @step(14, "Member", "chat drawer from the header; send in a channel; Second member sees it arrive")
    def _(run, s):
        page = run.persona("Member").use()
        second = run.persona("Second member").use()
        page.goto(run.base + "/latest")
        page.locator(".d-header .chat-header-icon a, .d-header .chat-header-icon button").first.click()
        drawer = expect_visible(page.locator(".chat-drawer"), "the chat drawer")
        channel = expect_visible(drawer.locator(".chat-channel-row").filter(has_text="General"), "the General channel in the drawer")
        channel.click()
        composer = expect_visible(drawer.locator(".chat-composer__input"), "the chat composer")
        # Second member has the same channel open before the message goes.
        second.goto(run.base + "/latest")
        second.locator(".d-header .chat-header-icon a, .d-header .chat-header-icon button").first.click()
        sdrawer = expect_visible(second.locator(".chat-drawer"), "Second member's chat drawer")
        expect_visible(sdrawer.locator(".chat-channel-row").filter(has_text="General"), "General in Second member's drawer").click()
        expect_visible(sdrawer.locator(".chat-composer__input"), "Second member's channel open")
        message = f"{run.label} chat"
        composer.click()
        composer.press_sequentially(message, delay=15)
        composer.press("Enter")
        expect_visible(drawer.locator(".chat-message-text, .chat-message-container", has_text=message), "the message in Member's channel")
        run.leave(f"chat message '{message}' by user1 in #General")
        try:
            sdrawer.locator(".chat-message-text, .chat-message-container", has_text=message).first.wait_for(state="visible", timeout=LIVE_WAIT)
        except PlaywrightTimeout:
            raise StepFail("Second member didn't see the message arrive") from None

    @step(15, "Member", "chat DM with Second member, react; full-page chat and back to the drawer")
    def _(run, s):
        page = run.persona("Member").use()
        page.goto(run.base + "/latest")
        page.locator(".d-header .chat-header-icon a, .d-header .chat-header-icon button").first.click()
        drawer = page.locator(".chat-drawer")
        expect_visible(drawer, "the chat drawer")
        back = drawer.locator(".c-navbar__back-button")
        if back.count() and back.first.is_visible():
            back.first.click()
        # A new direct message: the drawer's DMs tab, then its new-message
        # button, or "Start a conversation" when there is none yet.
        expect_visible(drawer.locator("#c-footer-direct-messages"), "the drawer's DMs tab").click()
        dm = drawer.locator(".chat-channel-row", has_text="user0")
        creator = page.locator(".chat-message-creator, .d-modal.chat-modal-new-message")
        # The tab's list draws after a request: wait for a row or the empty
        # state before choosing (a rerun finds the DM the last run made).
        # Wait for this tab's own outcome: the channels tab's rows are still
        # drawn for a moment after the switch and would satisfy any row.
        try:
            dm.or_(drawer.locator(".empty-state__cta button, .channels-list-empty")).first.wait_for(state="visible", timeout=WAIT)
        except PlaywrightTimeout:
            pass
        if dm.count() == 0:
            drawer.locator(".c-navbar__new-dm-button, .empty-state__cta button, button:has-text('Start a conversation')").first.click()
            try:
                creator.first.wait_for(state="visible", timeout=4000)
            except PlaywrightTimeout:
                pass
            if creator.count() and creator.first.is_visible():
                creator.locator("input").first.press_sequentially("user0", delay=40)
                expect_visible(creator.locator(".chat-message-creator__list-item, .chat-message-creator__row", has_text="user0"),
                               "user0 in the new message search").click()
                start = creator.locator(".chat-message-creator__open-dm-btn, .chat-message-creator__start-btn, button:has-text('Start')")
                if start.count() and start.first.is_visible():
                    start.first.click()
            else:
                # The sidebar's "Start DM or group chat" is the other way in.
                page.locator("#d-sidebar a[data-link-name='new-chat-dm']").first.click()
                try:
                    creator.first.wait_for(state="visible", timeout=4000)
                except PlaywrightTimeout:
                    pass
                s.miss("no way to start a direct message: the drawer's 'Start a conversation' opens nothing and "
                       f"the sidebar's 'Start DM or group chat' goes to {urlparse(page.url).path}"
                       + ("" if creator.count() else " without a message creator"))
        else:
            dm.first.click()
        composer = drawer.locator(".chat-composer__input")
        if not s.misses:
            expect_visible(composer, "the direct message channel")
            message = f"{run.label} direct message"
            composer.click()
            composer.press_sequentially(message, delay=15)
            composer.press("Enter")
            sent = expect_visible(drawer.locator(".chat-message-container", has_text=message), "the direct message sent")
            run.leave(f"chat direct message '{message}' from user1 to user0 (and the DM channel if it was new)")
        else:
            # Go on in #General with the rest of the step: react, full page, back.
            page.goto(run.base + "/latest")
            page.locator(".d-header .chat-header-icon a, .d-header .chat-header-icon button").first.click()
            expect_visible(drawer, "the chat drawer")
            if back.count() and back.first.is_visible():
                back.first.click()
            channels = drawer.locator("#c-footer-channels")
            if channels.count() and channels.first.is_visible():
                channels.first.click()
            expect_visible(drawer.locator(".chat-channel-row").filter(has_text="General"), "#General").click()
            expect_visible(composer, "#General open")
            sent = drawer.locator(".chat-message-container").last
            s.notes.append("reacted in #General instead of a direct message")
        sent.hover()
        expect_visible(page.locator(".chat-message-actions .react-btn"), "the react button on hover").click()
        picker = expect_visible(page.locator(".emoji-picker"), "the emoji picker")
        box = picker.bounding_box()
        if box and box["y"] < 0:
            s.problem("layout", f"the emoji picker opens partly above the viewport (top at {box['y']:.0f}px)", page.url, warning=True)
        try:
            picker.locator("input.filter-input, input[type=search]").first.press_sequentially("tada", delay=40, timeout=3000)
            picker.locator("img.emoji[data-emoji='tada']").first.click(timeout=3000)
        except PlaywrightTimeout:
            # The search box is out of reach: the first emoji fully in view.
            index = page.evaluate("""() => [...document.querySelectorAll('.emoji-picker .emoji-picker__sections img.emoji')]
              .findIndex(e => { const r = e.getBoundingClientRect(); return r.top > 60 && r.bottom < innerHeight; })""")
            if index < 0:
                raise StepFail("no emoji in the picker within reach") from None
            picker.locator(".emoji-picker__sections img.emoji").nth(index).click()
        expect_visible(sent.locator(".chat-message-reaction"), "the reaction on the message")
        run.leave("user1's chat reaction")
        expect_visible(drawer.locator(".c-navbar__full-page-button"), "the full-page chat button").click()
        page.wait_for_url("**/chat/**", timeout=WAIT)
        expect_visible(page.locator("#main-chat-outlet .chat-composer__input, .full-page-chat .chat-composer__input"), "full-page chat")
        expect_visible(page.locator(".c-navbar__open-drawer-button"), "the button back to the drawer").click()
        expect_visible(page.locator(".chat-drawer .chat-composer__input"), "the drawer again")

    @step(16, "Member", "full-page chat: edit a message, then delete it")
    def _(run, s):
        page = run.persona("Member").use()
        channel = page.locator(".chat-channel")
        open_channel(run, page, "/chat/c/general/2", "#General in full-page chat")
        typo = f"{run.label} chat with a tpyo"
        message = send_chat(channel, typo, "the message sent")
        # The staged message is swapped for the saved one; act on that.
        wait_until(page, "t => [...document.querySelectorAll('.chat-message-container[data-id]')]"
                   ".some(m => m.innerText.includes(t) && /^\\d+$/.test(m.dataset.id))",
                   typo, "the message saved")
        chat_message_action(message, "edit", "Edit in the message's menu")
        composer = channel.locator(".chat-composer__input").first
        wait_until(page, "t => document.querySelector('.chat-channel .chat-composer__input').value === t", typo,
                   "the message's text in the composer to edit")
        fixed = f"{run.label} chat with the typo fixed"
        composer.fill(fixed)
        composer.press("Enter")
        edited = expect_visible(channel.locator(".chat-message-container", has_text=fixed), "the edited message")
        if channel.locator(".chat-message-container", has_text=typo).count():
            raise StepFail("the old text is still drawn after the edit")
        chat_message_action(edited, "delete", "Delete in the message's menu")
        expect_gone(channel.locator(".chat-message-container", has_text=fixed), "the message gone from the channel")
        page.reload()
        expect_visible(channel.locator(".chat-composer__input"), "#General again")
        if channel.locator(".chat-message-container", has_text=fixed).count():
            raise StepFail("the deleted message is back after a reload")
        run.leave(f"user1's deleted chat message '{fixed}' in #General (trashed, kept for staff)")

    @step(17, "Member", "an image in a chat message; it opens in the lightbox")
    def _(run, s):
        page = run.persona("Member").use()
        channel = page.locator(".chat-channel")
        open_channel(run, page, "/chat/c/general/2", "#General in full-page chat")
        expect_visible(channel.locator(".chat-composer-dropdown__trigger-btn"), "the composer's + button").click()
        attach = expect_visible(page.locator("button[data-id='chat-upload-btn']"), "Attach a file in the composer's menu")
        with page.expect_file_chooser() as chooser:
            attach.click()
        chooser.value.set_files(run.files["png"])
        expect_visible(channel.locator(".chat-composer-upload--image .preview-img"), "the image's preview in the composer", timeout=30_000)
        # The send button waits for the upload to finish.
        wait_until(page, "() => !document.querySelector('.chat-channel .chat-composer-button.-send').disabled", None,
                   "the upload to finish", timeout=30_000)
        text = f"{run.label} chat image"
        message = send_chat(channel, text, "the message with the image")
        image = expect_visible(message.locator(".chat-uploads img.chat-img-upload"), "the image in the message", timeout=LIVE_WAIT)
        run.state["chat_image_message"] = text
        run.leave(f"chat message '{text}' with an uploaded image by user1 in #General (step 20 moves it)")
        image.click()
        expect_visible(page.locator(".pswp"), "the lightbox to open")
        settle(page, 800)
        page.keyboard.press("Escape")
        expect_gone(page.locator(".pswp"), "the lightbox to close")

    @step(18, "Admin", "a new channel with threading from the browse page; its settings")
    def _(run, s):
        page = run.persona("Admin").use()
        page.goto(run.base + "/chat/browse")
        expect_visible(page.locator("#main-outlet button").filter(has_text=re.compile(r"^\s*New channel")), "New channel on the browse page").click()
        modal = expect_visible(page.locator(".d-modal", has=page.locator(".chat-modal-create-channel__form")), "the new channel modal")
        # The name first: a category chosen while the name is empty names
        # the channel after it.
        name = f"Rehearsal {run.stamp}"
        modal.locator("input[name=name]").press_sequentially(name, delay=20)
        modal.locator("select[name=chatable_id]").select_option(label="General")
        # The slug comes from POST /slugs.json as the name is typed, shown as
        # the slug field's placeholder.
        wait_until(page, "() => document.querySelector('.chat-modal-create-channel__form input[name=slug]').placeholder"
                   ".startsWith('rehearsal-')", None, "the slug made from the name")
        modal.locator("input[name=description]").fill(f"Made by the {run.label}")
        modal.locator("label[for=control-threading_enabled-input]").click()
        modal.locator(".d-modal__footer .create").click()
        page.wait_for_url(re.compile(r"/chat/c/[^/]+/\d+$"), timeout=WAIT)
        path = urlparse(page.url).path
        run.state["channel_path"] = path
        run.state["channel_name"] = name
        run.leave(f"chat channel '{name}' at {path} (unless cleanup deleted it)")
        channel = page.locator(".chat-channel")
        expect_visible(page.locator(".c-navbar__channel-title", has_text=name), "the new channel open")
        starter = f"{run.label}: what should the next demo show?"
        send_chat(channel, starter, "the channel's first message")
        run.state["thread_starter"] = starter
        page.locator(".c-navbar__channel-title").first.click()
        page.wait_for_url("**/info/settings", timeout=WAIT)
        # The switch's button is drawn under its slider, so attached, not visible.
        wait_until(page, "() => document.querySelector(\".c-channel-settings__threading-switch[aria-checked='true']\")",
                   None, "threading shown on in the channel's settings")
        expect_visible(page.locator(".delete-btn"), "the delete button in the channel's settings")

    @step(19, "Member", "join the new channel, reply to a message: it starts a thread; reply in the side panel", needs=[18])
    def _(run, s):
        page = run.persona("Member").use()
        page.goto(run.base + "/chat/browse")
        expect_visible(page.locator("#main-outlet a, #main-outlet .chat-channel-card").filter(has_text=run.state["channel_name"]),
                       "the new channel on the browse page").click()
        page.wait_for_url(re.compile(r"/chat/c/[^/]+/\d+"), timeout=WAIT)
        open_channel(run, page, run.state["channel_path"], "the new channel")
        channel = page.locator(".chat-channel")
        original = expect_visible(channel.locator(".chat-message-container", has_text=run.state["thread_starter"]),
                                  "the admin's message")
        original.hover()
        expect_visible(original.locator(".chat-message-actions .reply-btn"), "Reply on hover").click()
        expect_visible(channel.locator(".chat-composer-message-details, .chat-reply"), "the message being replied to above the composer")
        reply = f"{run.label} reply that starts a thread"
        composer = channel.locator(".chat-composer__input").first
        composer.press_sequentially(reply, delay=10)
        composer.press("Enter")
        indicator = expect_visible(original.locator(".chat-message-thread-indicator"), "the thread indicator under the message",
                                   timeout=LIVE_WAIT)
        indicator.click()
        page.wait_for_url(re.compile(r"/t/\d+"), timeout=WAIT)
        panel = expect_visible(page.locator(".chat-side-panel .chat-thread"), "the thread in the side panel")
        expect_visible(panel.locator(".chat-message-container", has_text=reply), "the reply in the thread")
        second = f"{run.label} second reply in the thread"
        send_chat(panel, second, "the second reply in the thread")
        expect_visible(original.locator(".chat-message-thread-indicator__replies-count", has_text="2 replies"),
                       "the indicator counting two replies", timeout=LIVE_WAIT)
        run.leave("user1's thread of two replies in the new channel")
        page.locator(".c-navbar__back-button").first.click()
        expect_visible(page.locator(".c-navbar__threads-list-button"), "the threads list button").click()
        # The list counts a thread once its reply count is updated, a job
        # Discourse runs a few seconds after a reply; a presenter talking
        # gives it that, the rehearsal looks again.
        item = page.locator(".chat-thread-list-item").filter(has_text=run.state["thread_starter"][:30])
        for attempt in range(4):
            try:
                item.first.wait_for(state="visible", timeout=3000)
                break
            except PlaywrightTimeout:
                page.reload()
        expect_visible(item, "the thread in the channel's threads list")

    @step(20, "Moderator", "select chat messages in #General and move them to the new channel", needs=[17, 18])
    def _(run, s):
        page = run.persona("Moderator").use()
        # The move lists the moderator's own public channels.
        open_channel(run, page, run.state["channel_path"], "the new channel")
        channel = page.locator(".chat-channel")
        open_channel(run, page, "/chat/c/general/2", "#General")
        def by_text(text):
            return channel.locator(".chat-message-container", has=page.locator(
                ".chat-message-text", has_text=re.compile(rf"^\s*{re.escape(text)}\s*$")))

        image = expect_visible(channel.locator(".chat-message-container", has_text=run.state["chat_image_message"]),
                               "step 17's image message")
        chat_message_action(image, "select", "Select in the message's menu")
        bar = expect_visible(page.locator(".chat-selection-management"), "the selection bar in the composer's place")
        moved = [run.state["chat_image_message"]]
        # Step 14's message too, ticked in the selection.
        earlier = by_text(f"{run.label} chat")
        if run.by_number(14).achieved and earlier.count():
            earlier.first.locator(".chat-message-selector").click()
            moved.insert(0, f"{run.label} chat")
        expect_visible(bar.locator("#chat-delete-btn", has_text=f"({len(moved)}/"), f"{len(moved)} messages selected")
        bar.locator("#chat-move-to-channel-btn").click()
        modal = expect_visible(page.locator(".chat-modal-move-message-to-channel"), "the move modal")
        modal.locator("select").select_option(label=run.state["channel_name"])
        modal.locator(".d-modal__footer .btn-primary").click()
        destination = run.state["channel_path"].rsplit("/", 1)[1]
        page.wait_for_url(re.compile(rf"/chat/c/[^/]+/{destination}(/|$)"), timeout=WAIT)
        expect_visible(page.locator(".c-navbar__channel-title", has_text=run.state["channel_name"]), "the new channel after the move")
        expect_visible(channel.locator(".chat-message-container", has_text=run.state["chat_image_message"]),
                       "the image message moved into the new channel")
        if len(moved) == 2:
            expect_visible(by_text(moved[0]), "step 14's message moved into the new channel")
        run.leave(f"{len(moved)} messages moved from #General to the new channel, and the move's notice in #General")

    @step(21, "Admin", "chat's admin page: an incoming webhook posts to the new channel", needs=[18])
    def _(run, s):
        page = run.persona("Admin").use()
        page.goto(run.base + "/admin/plugins/chat")
        expect_visible(page.locator("#main-outlet a", has_text=re.compile(r"^\s*Incoming webhooks")), "the Incoming webhooks tab").click()
        page.wait_for_url("**/admin/plugins/chat/hooks", timeout=WAIT)
        expect_visible(page.locator(".admin-incoming-webhooks-new"), "New incoming webhook").click()
        form = expect_visible(page.locator(".chat-incoming-webhook-form"), "the new webhook form")
        hook = f"Rehearsal {run.stamp}"
        form.locator("input[name=name]").fill(hook)
        form.locator("select[name=chat_channel_id]").select_option(label=run.state["channel_name"])
        form.locator("button[type=submit]").click()
        page.wait_for_url("**/admin/plugins/chat/hooks/*/edit", timeout=WAIT)
        run.state["webhook"] = hook
        run.leave(f"incoming webhook '{hook}' (unless cleanup deleted it)")
        url = expect_visible(page.locator(".chat-incoming-webhook-form [data-name=url] code"), "the webhook's URL").inner_text().strip()
        text = f"{run.label} posted by the webhook"
        answer = page.request.post(url, data={"text": text})
        if answer.status != 200:
            raise StepFail(f"POST to the webhook's URL answered {answer.status}: {answer.text()[:200]}")
        page.goto(run.base + run.state["channel_path"])
        expect_visible(page.locator(".chat-channel .chat-message-container", has_text=text), "the webhook's message in the channel",
                       timeout=LIVE_WAIT)

    @step(22, "Admin", "activate the Newcomer from the admin users list", needs=[13])
    def _(run, s):
        page = run.persona("Admin").use()
        admin_user_page(run, page, run.newcomer, listing="new")
        expect_visible(page.locator("#main-outlet button", has_text=re.compile(r"^\s*Activate Account")), "Activate Account").click()
        expect_visible(page.locator("#main-outlet button", has_text=re.compile(r"Deactivate Account")), "the account activated")

    @step(23, "Second member", "flags a post as inappropriate", needs=[6])
    def _(run, s):
        page = run.persona("Second member").use()
        page.goto(run.state["reply_url"])
        post = expect_visible(post_with(page, run.state["reply_marker"]), "the reply to flag")
        post.scroll_into_view_if_needed()
        post.hover()
        more = post.locator(".post-controls .show-more-actions, .post-action-menu__show-more")
        flag = post.locator(".post-controls button.create-flag, .post-action-menu__flag")
        if not flag.first.is_visible() and more.count():
            more.first.click()
        expect_visible(flag, "the flag button").click()
        modal = expect_visible(page.locator(".flag-modal, .d-modal.flag-modal"), "the flag modal")
        modal.locator("label, .flag-action-type").filter(has_text=re.compile("inappropriate", re.I)).first.click()
        modal.locator(".d-modal__footer .btn-primary, .modal-footer .btn-primary").first.click()
        expect_gone(page.locator(".flag-modal"), "the flag modal to close after flagging")
        run.leave("user0's flag (inappropriate) on that reply, and its review item")

    @step(24, "Moderator", "review count in header and sidebar; /review, a note, agree with the flag", needs=[23])
    def _(run, s):
        page = run.persona("Moderator").use()
        page.goto(run.base + "/latest")
        try:
            page.locator("#d-sidebar a[href='/review'] .sidebar-section-link-content-badge").first.wait_for(state="visible", timeout=WAIT)
        except PlaywrightTimeout:
            s.miss("no review count in the sidebar")
        if not page.locator("#current-user .badge-notification").first.is_visible():
            s.miss("no count on the header avatar")
        page.locator("#d-sidebar a[href='/review']").first.click()
        page.wait_for_url("**/review**", timeout=WAIT)
        item = expect_visible(page.locator(".review-item.reviewable-flagged-post").filter(has_text=run.state["reply_marker"]),
                              "the flagged reply in /review")
        review_note(run, item)
        item.locator("details summary", has_text=re.compile(r"^\s*Yes")).first.click()
        expect_visible(page.locator(".select-kit.is-expanded .select-kit-row", has_text=re.compile("Agree with flag", re.I)), "the agree choices").click()
        expect_visible(item.locator("*", has_text=re.compile(r"^\s*Flag approved\s*$")), "the flag shown approved")

    @step(25, "Admin", "watched word 'require approval'; Newcomer posts it; held; approve from /review")
    def _(run, s):
        page = run.persona("Admin").use()
        page.goto(run.base + "/admin/customize/watched_words/action/require_approval")
        expect_visible(page.locator("#watched-words-header"), "the watched word form").click()
        field = expect_visible(page.locator("#watched-words input.filter-input, .select-kit.is-expanded input.filter-input"), "the watched word input")
        field.press_sequentially(run.watched_word, delay=20)
        field.press("Enter")
        page.keyboard.press("Escape")
        page.locator("button[type=submit]", has_text=re.compile(r"^\s*Add")).first.click()
        # The list folds behind "show N words" once it has any.
        show = page.locator("label", has_text=re.compile(r"show \d+ word", re.I))
        try:
            show.first.wait_for(state="visible", timeout=WAIT)
            if not show.first.locator("input").is_checked():
                show.first.click()
        except PlaywrightTimeout:
            pass
        expect_visible(page.locator(".watched-word", has_text=run.watched_word), "the watched word listed")
        run.state["watched_word_added"] = True
        missing = [n for n in (13, 22) if not run.by_number(n).achieved]
        if missing:
            s.miss("watched word added; the Newcomer's held post is skipped (needs "
                   + ", ".join(map(str, missing)) + ")")
            return
        newcomer = run.persona("Newcomer")
        npage = newcomer.page
        npage.goto(run.base + "/login")
        npage.locator("#login-account-name").fill(run.newcomer)
        npage.locator("#login-account-password").fill(run.state["newcomer_password"])
        npage.locator("#login-button").click()
        expect_visible(npage.locator("#current-user"), "the newcomer logged in")
        s.used.append(newcomer)
        npage.locator("#create-topic").click()
        title = expect_visible(npage.locator("#reply-title"), "the newcomer's composer")
        held_title = f"{run.label} held topic"
        title.press_sequentially(held_title, delay=80)
        type_in_composer(npage, f"My first post mentions {run.watched_word}, which needs approval.", delay=80)
        npage.locator("#reply-control .save-or-cancel .create").click()
        expect_visible(npage.locator(".d-modal, .dialog-body, .post-enqueued-modal").filter(has_text=re.compile("approv|review", re.I)),
                       "the newcomer told the post awaits approval")
        run.leave(f"topic '{held_title}' by {run.newcomer}, held for approval (approved below if the step got that far)")
        page.goto(run.base + "/review")
        item = expect_visible(page.locator(".review-item.reviewable-queued-post").filter(has_text=run.watched_word), "the held post in /review")
        item.locator("button.approve-post").first.click()
        # The status badge (statuses.approved_post: "Post approved").
        expect_visible(item.locator(".review-item__status.--approved"), "the held post shown approved")

    @step(26, "Admin", "topic admin wrench: close, reopen, unlist", needs=[8])
    def _(run, s):
        page = run.persona("Admin").use()
        page.goto(run.base + run.state["new_topic"])
        expect_visible(page.locator(".topic-post"), "the rehearsal topic")

        def wrench(action, what):
            page.locator(".topic-admin-menu-trigger, .toggle-admin-menu").first.click()
            expect_visible(page.locator(f".topic-admin-menu-content .topic-admin-{action}, .topic-admin-{action}"), f"'{what}' in the wrench menu").click()

        wrench("close", "Close Topic")
        expect_visible(page.locator(".topic-status .d-icon-lock, .topic-statuses .d-icon-lock, .small-action.topic-post-visited, .small-action", has_text=re.compile("closed", re.I)),
                       "the topic shown closed")
        wrench("open", "Open Topic")
        expect_visible(page.locator(".small-action", has_text=re.compile("opened", re.I)), "the topic shown reopened")
        wrench("visible", "Unlist Topic")
        expect_visible(page.locator(".topic-statuses .d-icon-far-eye-slash, .topic-status .unlisted, .small-action", has_text=re.compile("unlisted", re.I)),
                       "the topic shown unlisted")

    @step(27, "Admin", "dashboard, a report, the staff action log")
    def _(run, s):
        page = run.persona("Admin").use()
        page.goto(run.base + "/latest")
        page.locator("#d-sidebar a[href='/admin'], .sidebar-section-link[data-link-name=admin]").first.click()
        page.wait_for_url("**/admin**", timeout=WAIT)
        expect_visible(page.locator(".dashboard, .admin-dashboard, .dashboard-next"), "the dashboard")
        page.goto(run.base + "/admin/reports")
        report = expect_visible(page.locator("#main-outlet a[href*='/admin/reports/']"), "the reports list")
        s.notes.append(f"opened the report {report.get_attribute('href')}")
        report.click()
        page.wait_for_url("**/admin/reports/**", timeout=WAIT)
        expect_visible(page.locator(".admin-report"), "a report")
        page.goto(run.base + "/admin/logs/staff_action_logs")
        expect_visible(page.locator("#main-outlet table tbody tr"), "the staff action log")
        text = page.locator("#main-outlet").inner_text().lower()
        expected = {}
        if run.by_number(26).achieved:
            expected["close"] = "step 26's close"
        if run.state.get("watched_word_added"):
            expected["watched"] = "step 25's watched word"
        if run.by_number(22).achieved:
            expected["activate"] = "step 22's activation"
        absent = [what for word, what in expected.items() if word not in text]
        if absent:
            s.miss("the staff action log doesn't show " + ", ".join(absent))

    @step(28, "Admin", "a list setting (authorized extensions), a plain one; customize a color palette")
    def _(run, s):
        page = run.persona("Admin").use()
        page.goto(run.base + "/admin/site_settings?filter=authorized_extensions")
        row = page.locator(".row.setting[data-setting='authorized_extensions']")
        expect_visible(row, "the authorized_extensions setting")
        summary = row.locator("summary").first
        s.notes.append("authorized_extensions before: " + (summary.get_attribute("data-name") or ""))
        had_txt = "txt" in (summary.get_attribute("data-name") or "").split(",")
        if had_txt:
            raise StepFail("authorized_extensions already has txt, the extension this step adds")
        summary.click()
        field = expect_visible(row.locator("input.filter-input"), "the list's filter")
        field.press_sequentially("txt", delay=40)
        field.press("Enter")
        page.keyboard.press("Escape")
        expect_visible(row.locator(".setting-controls__ok"), "the setting's save button").click()
        wait_until(page, "() => (document.querySelector(\".row.setting[data-setting='authorized_extensions'] summary\").dataset.name || '').split(',').includes('txt')"
                   " && !document.querySelector(\".row.setting[data-setting='authorized_extensions'] .setting-controls__ok\")",
                   None, "txt saved in authorized_extensions")
        run.state["ext_added"] = True
        page.reload()
        row = page.locator(".row.setting[data-setting='authorized_extensions']")
        if "txt" not in (row.locator("summary").first.get_attribute("data-name") or "").split(","):
            raise StepFail("txt was gone from authorized_extensions after a reload")
        # Back as it was: txt out again.
        row.locator("summary").first.click()
        expect_visible(row.locator(".selected-choice[data-name='txt']"), "txt in the list").click()
        page.keyboard.press("Escape")
        expect_visible(row.locator(".setting-controls__ok"), "the setting's save button").click()
        wait_until(page, "() => !(document.querySelector(\".row.setting[data-setting='authorized_extensions'] summary\").dataset.name || '').split(',').includes('txt')"
                   " && !document.querySelector(\".row.setting[data-setting='authorized_extensions'] .setting-controls__ok\")",
                   None, "txt removed again")
        run.state["ext_added"] = False
        # A plain setting: the site title, changed and put back.
        page.goto(run.base + "/admin/site_settings?filter=title")
        row = page.locator(".row.setting[data-setting='title']")
        field = expect_visible(row.locator("input#setting-title, input").first, "the title setting")
        old = field.input_value()
        run.state["old_title"] = old
        field.fill(f"{old} ({run.label})")
        expect_visible(row.locator(".setting-controls__ok"), "the title's save button").click()
        expect_gone(row.locator(".setting-controls__ok"), "the title saved")
        page.reload()
        field = expect_visible(page.locator(".row.setting[data-setting='title'] input").first, "the title setting again")
        if run.label not in field.input_value():
            raise StepFail(f"the new title didn't stick after a reload (it reads {field.input_value()!r})")
        field.fill(old)
        page.locator(".row.setting[data-setting='title'] .setting-controls__ok").first.click()
        expect_gone(page.locator(".row.setting[data-setting='title'] .setting-controls__ok"), "the title put back")
        run.state["old_title"] = None
        # A color palette: a new one, renamed, one color changed, saved.
        page.goto(run.base + "/admin/customize/colors")
        expect_visible(page.locator(".create-new-palette"), "the color palettes").click()
        expect_visible(page.locator(".d-modal .btn-primary"), "the base palette modal").click()
        name = f"Rehearsal {run.stamp}"
        expect_visible(page.locator(".admin-config-color-palettes__edit-name"), "the new palette's editor").click()
        field = expect_visible(page.locator("input[name=name]"), "the palette's name field")
        run.leave("a palette named 'New color palette' if the rename below fails")
        field.fill(name)
        page.locator(".admin-config-color-palettes__save-name").click()
        expect_visible(page.locator("#main-outlet h1, #main-outlet h2").filter(has_text=name), "the palette renamed")
        run.state["palette"] = name
        run.left_behind[:] = [x for x in run.left_behind if "New color palette" not in x]
        run.leave(f"color palette '{name}' (unless cleanup deleted it)")
        color = page.locator(".color-palette-editor__text-input").nth(2)
        color.fill("cc3366")
        color.press("Tab")
        page.locator("#main-outlet button", has_text=re.compile(r"^\s*Save changes")).first.click()
        page.wait_for_timeout(1000)
        page.reload()
        if page.locator(".color-palette-editor__text-input").count() == 0:
            page.goto(run.base + "/admin/customize/colors")
            card = expect_visible(page.locator("#main-outlet .color-palette, #main-outlet li, #main-outlet .admin-config-area-card").filter(has_text=name), "the palette in the list")
            card.locator("button", has_text="Edit").first.click()
        saved = expect_visible(page.locator(".color-palette-editor__text-input").nth(2), "the palette's colors after a reload")
        if saved.input_value().lower() != "cc3366":
            raise StepFail(f"the changed color didn't save (it reads {saved.input_value()})")

    @step(29, "Admin", "silence a member, show their system message; unsilence")
    def _(run, s):
        page = run.persona("Admin").use()
        target = "Newcomer" if run.by_number(22).achieved else "Second member"
        username = run.persona(target).username
        s.notes.append(f"silencing {target} ({username})")
        admin_user_page(run, page, username)
        if page.locator(".silence-user").count() == 0 and page.locator("#main-outlet button", has_text=re.compile(r"^\s*Unsilence")).count():
            raise StepFail(f"{username} is silenced already (a newcomer's first post auto-silences them as a fast typer), "
                           "so there is no Silence button")
        page.locator(".silence-user").first.click()
        modal = expect_visible(page.locator(".d-modal"), "the silence modal")
        until = modal.locator("details.future-date-input-selector summary, .future-date-input details summary, details summary")
        if until.count():
            until.first.click()
            expect_visible(page.locator(".select-kit-row", has_text=re.compile("tomorrow|hour|later", re.I)), "a silence duration").click()
        reason = modal.locator("input.silence-reason, input[name=reason], .penalty-reason input, input[type=text]").first
        reason.fill(f"Demo {run.label}")
        modal.locator(".d-modal__footer .btn-danger, button.perform-penalize, .d-modal__footer .btn-primary").first.click()
        expect_gone(page.locator(".d-modal"), "the silence modal to close")
        expect_visible(page.locator("button", has_text=re.compile(r"^\s*Unsilence", re.I)), "the user shown silenced")
        run.state["silenced"] = username
        run.leave(f"a silence and unsilence of {username} in the staff log, and the system message they got")
        tpage = run.persona(target).use()
        tpage.goto(run.base + "/latest")
        # The notification comes from a job a moment after the silence, and the
        # menu fetches its list when it opens: reload until it's there, as a
        # presenter switching windows would see it.
        notice = tpage.locator(".user-menu li", has_text=re.compile("temporarily on hold", re.I))
        for _ in range(5):
            open_user_menu(tpage)
            try:
                notice.first.wait_for(state="visible", timeout=2000)
                break
            except PlaywrightTimeout:
                tpage.reload()
        open_user_menu(tpage)
        # The menu's messages tab shows only to members who may send PMs, which
        # a new or silenced account can't; the menu opens on the notifications
        # tab, which lists the system message for everyone. Clicking that tab
        # while it's active would follow its linkWhenActive to the
        # notifications page, as Ember does, so it isn't clicked.
        messages_tab = tpage.locator("#user-menu-button-messages")
        if messages_tab.count() and messages_tab.first.is_visible():
            messages_tab.first.click()
        # SystemMessage :silenced_by_staff, "Account temporarily on hold".
        expect_visible(tpage.locator(".user-menu li", has_text=re.compile("temporarily on hold", re.I)), f"the silence message in {username}'s user menu").click()
        expect_visible(tpage.locator(".topic-post .cooked", has_text=re.compile("temporarily placed on hold", re.I)), "the system message")
        page.locator("button", has_text=re.compile(r"^\s*Unsilence", re.I)).first.click()
        confirm = page.locator(".dialog-footer .btn-primary, .d-modal__footer .btn-primary")
        try:
            confirm.first.wait_for(state="visible", timeout=2000)
            confirm.first.click()
        except PlaywrightTimeout:
            pass
        expect_visible(page.locator(".silence-user"), "the user unsilenced")
        run.state["silenced"] = None

    @step(30, "Admin", "/admin/plugins: turn on Assign, Policy, User Notes and Gamification")
    def _(run, s):
        page = run.persona("Admin").use()
        page.goto(run.base + "/latest")
        page.locator("#d-sidebar a[href='/admin'], .sidebar-section-link[data-link-name=admin]").first.click()
        page.wait_for_url("**/admin**", timeout=WAIT)
        page.goto(run.base + "/admin/plugins")
        expect_visible(page.locator(".admin-plugins-list__row"), "the plugins list")
        turned = run.state.setdefault("plugins_turned_on", [])
        for setting in PLUGINS_TO_TURN_ON:
            if toggle_plugin(page, setting, True):
                turned.append(setting)
            else:
                s.notes.append(f"{setting} was on already")
        # On by default, as on the demo database.
        off = [setting for setting in PLUGINS_ON_BY_DEFAULT
               if page.locator(f"[data-enabled-setting='{setting}']").first.get_attribute("aria-checked") != "true"]
        if off:
            s.miss("off, though on by default: " + ", ".join(off))
        page.reload()
        not_kept = [setting for setting in PLUGINS_TO_TURN_ON
                    if page.locator(f"[data-enabled-setting='{setting}']").first.get_attribute("aria-checked") != "true"]
        if not_kept:
            raise StepFail("switched on but off after a reload: " + ", ".join(not_kept))

    @step(31, "Admin", "a new category with the Support and Ideas types (Solved, Topic Voting)")
    def _(run, s):
        page = run.persona("Admin").use()
        page.goto(run.base + "/categories")
        expect_visible(page.locator("#create-category"), "New Category on /categories").click()
        page.wait_for_url("**/new-category**", timeout=WAIT)
        name = f"Rehearsal ideas {run.stamp}"
        expect_visible(page.locator("input[name=name]"), "the category form").fill(name)
        for kind in ("support", "ideas"):
            page.locator(f".category-type-option[data-category-type='{kind}'] label").click()
            wait_until(page, "k => document.querySelector(`.category-type-option[data-category-type='${k}'] input`).checked",
                       kind, f"the {kind} type ticked")
        page.locator("#main-outlet button", has_text=re.compile(r"^\s*Create category")).click()
        page.wait_for_url(re.compile(r"/c/[^/]+/\d+"), timeout=WAIT)
        run.state["category_path"] = urlparse(page.url).path
        run.state["category_name"] = name
        run.leave(f"category '{name}' at {run.state['category_path']} with the Support and Ideas types")
        # Ideas brings topic voting's Votes and My Votes lists.
        expect_visible(page.locator("#navigation-bar").get_by_text(re.compile(r"^\s*Votes\s*$")),
                       "the category's Votes list in its navigation")

    @step(32, "Member", "ask in that category; Second member votes for it and answers", needs=[31])
    def _(run, s):
        page = run.persona("Member").use()
        page.goto(run.base + run.state["category_path"])
        expect_visible(page.locator("#create-topic"), "New Topic in the category").click()
        title = expect_visible(page.locator("#reply-title"), "the new topic composer")
        question = f"{run.label} how do I move a forum to discourse-rs?"
        title.press_sequentially(question, delay=10)
        type_in_composer(page, f"Asked during the {run.label}: what does moving an existing forum take?")
        submit_composer(page)
        page.wait_for_url("**/t/**", timeout=WAIT)
        expect_visible(page.locator("#topic-title").filter(has_text="how do I move a forum"), "the question")
        run.state["question"] = topic_url(page.url)
        run.leave(f"topic '{question}' by user1 at {run.state['question']}")
        expect_visible(page.locator(".voting-wrapper"), "the vote button on the question")
        second = run.persona("Second member").use()
        second.goto(run.base + run.state["question"])
        voting = expect_visible(second.locator(".voting-wrapper"), "the vote button for Second member")
        voting.locator(".voting-wrapper__button").click()
        wait_until(second, "() => document.querySelector('.voting-wrapper .voting-wrapper__count-text').innerText.trim() === '1'",
                   None, "the vote counted")
        run.leave("user0's vote on the question")
        open_composer_reply(second)
        answer = f"Answer from user0, {run.label}: import it, then point DNS at the new server."
        type_in_composer(second, answer, delay=5)
        submit_composer(second)
        expect_visible(post_with(second, answer), "the answer in the topic")
        run.state["answer"] = answer
        run.leave(f"user0's answer in {run.state['question']}")
        second.reload()
        voted = expect_visible(second.locator(".voting-wrapper"), "the vote button after a reload")
        if voted.get_attribute("data-user-voted") != "true":
            raise StepFail("the vote wasn't kept after a reload")

    @step(33, "Member", "accept the answer as the solution; the title shows it solved", needs=[32])
    def _(run, s):
        page = run.persona("Member").use()
        page.goto(run.base + run.state["question"])
        page.evaluate("() => { window.__rehearsalNoReload = true }")
        # By its id: once accepted, the first post quotes the answer too.
        answer_id = expect_visible(post_with(page, run.state["answer"]), "the answer").locator("article").first.get_attribute("data-post-id")
        answer = page.locator(f"article[data-post-id='{answer_id}']")
        expect_visible(answer.locator(".post-action-menu__solved-unaccepted"), "the answer's Solution button").click()
        expect_visible(answer.locator(".post-action-menu__solved-accepted"), "the answer shown as the solution")
        expect_visible(page.locator("#topic-title .topic-status.--solved"), "the solved status in the title")
        if not page.evaluate("() => window.__rehearsalNoReload === true"):
            raise StepFail("the page reloaded to show the solution")
        run.leave("the accepted solution on the question")
        page.reload()
        expect_visible(page.locator("#topic-title .topic-status.--solved"), "the solved status after a reload")

    @step(34, "Second member", "a reaction other than a like, from the reaction picker", needs=[32])
    def _(run, s):
        page = run.persona("Second member").use()
        page.goto(run.base + run.state["question"])
        first = expect_visible(page.locator("article[data-post-id]"), "the question's first post")
        post_id = first.get_attribute("data-post-id")
        actions = page.locator(f"#discourse-reactions-actions-{post_id}-right")
        expect_visible(actions.locator(".discourse-reactions-reaction-button"), "the reaction button").hover()
        picker = expect_visible(actions.locator(".discourse-reactions-picker.is-expanded"), "the reaction picker on hover")
        picker.locator(".pickable-reaction.clap").click()
        wait_until(page, "id => document.querySelector(`#discourse-reactions-actions-${id}-right`).dataset.currentReaction === 'clap'",
                   post_id, "the post shown with Second member's clap")
        expect_visible(page.locator(f"#discourse-reactions-list-emoji-{post_id}-clap"), "the clap in the post's reactions")
        run.leave("user0's clap reaction on the question")

    @step(35, "Admin", "assign the question to the Moderator from the footer; it redraws in place", needs=[30, 32])
    def _(run, s):
        page = run.persona("Admin").use()
        page.goto(run.base + run.state["question"])
        expect_visible(page.locator(".topic-post"), "the question")
        page.evaluate("() => { window.__rehearsalNoReload = true }")
        scroll_to_topic_end(page)
        expect_visible(page.locator("#topic-footer-button-assign"), "the footer's Assign button").click()
        modal = expect_visible(page.locator(".d-modal.assign"), "the assign modal")
        modal.locator("#assignee").fill(PERSONAS["Moderator"])
        modal.locator(".assign-submit").click()
        expect_gone(modal, "the assign modal to close")
        expect_visible(page.locator("#topic-footer-buttons .reassign-trigger"), "the footer's reassign menu")
        expect_visible(page.locator("#post_1 .assigned-to", has_text=PERSONAS["Moderator"]), "the assignee under the first post")
        if not page.evaluate("() => window.__rehearsalNoReload === true"):
            raise StepFail("the page reloaded to show the assignment")
        run.leave(f"the question assigned to {PERSONAS['Moderator']}")

    @step(36, "Admin", "a policy in a reply; Second member accepts it", needs=[30, 32])
    def _(run, s):
        page = run.persona("Admin").use()
        page.goto(run.base + run.state["question"])
        expect_visible(page.locator(".topic-post"), "the question")
        scroll_to_topic_end(page)
        open_composer_reply(page)
        marker = f"Accept the house rules ({run.label})."
        type_in_composer(page, f'[policy group=trust_level_1 accept="I agree" revoke="Withdraw"]\n{marker}\n[/policy]')
        submit_composer(page)
        post = expect_visible(post_with(page, marker), "the policy reply", timeout=LIVE_WAIT)
        expect_visible(post.locator(".policy-footer"), "the policy drawn with its footer")
        run.leave(f"the admin's policy reply in {run.state['question']}")
        second = run.persona("Second member").use()
        policy = second.locator(".topic-post", has=second.locator(".cooked .policy", has_text=marker)).locator(".cooked .policy").first
        accept = policy.locator("button.accept")
        # The post's policy is saved as the post is processed, a moment
        # after it shows; a presenter switching windows gives it that.
        for attempt in range(3):
            second.goto(run.base + run.state["question"])
            expect_visible(policy, "the policy for Second member")
            try:
                accept.first.wait_for(state="visible", timeout=4000)
                break
            except PlaywrightTimeout:
                continue
        expect_visible(accept, "the policy's I agree button").click()
        expect_visible(policy.locator("button.revoke, button", has_text="Withdraw"), "Withdraw once accepted")
        expect_visible(policy.locator(".toggle-accepted .user-count", has_text=re.compile(r"^\s*1\s*$")), "one acceptance counted")
        run.leave("user0's acceptance of the policy")

    @step(37, "Moderator", "a staff note on Second member from their profile", needs=[30])
    def _(run, s):
        page = run.persona("Moderator").use()
        page.goto(run.base + f"/u/{PERSONAS['Second member']}/summary")
        expect_visible(page.locator(".show-user-notes-btn"), "the user notes button on the profile").click()
        modal = expect_visible(page.locator(".d-modal .user-notes-modal-body"), "the user notes modal")
        note = f"Note from the {run.label}"
        modal.locator("textarea[name=content]").fill(note)
        modal.locator("button[type=submit]").click()
        expect_visible(modal.locator(".user-note", has_text=note), "the note listed in the modal")
        run.leave(f"user2's note on {PERSONAS['Second member']}")
        page.keyboard.press("Escape")
        expect_gone(page.locator(".d-modal .user-notes-modal-body"), "the modal closed")
        page.reload()
        expect_visible(page.locator(".show-user-notes-btn"), "the user notes button again").click()
        expect_visible(page.locator(".d-modal .user-note", has_text=note), "the note kept after a reload")
        page.keyboard.press("Escape")

    @step(38, "Admin", "Gamification: recalculate the scores, then the leaderboard", needs=[30])
    def _(run, s):
        page = run.persona("Admin").use()
        page.goto(run.base + "/admin/plugins/discourse-gamification/leaderboards")
        expect_visible(page.locator(".leaderboard-admin__btn-recalculate"), "Recalculate scores").click()
        modal = expect_visible(page.locator(".d-modal", has=page.locator(".recalculate-modal__body")), "the recalculate modal")
        left = modal.locator(".recalculate-modal__footer-text").inner_text()
        s.notes.append(left)
        # Five recalculations a day (a rehearsal run spends one).
        if re.match(r"\s*0\b", left):
            s.notes.append("no recalculation left today; the leaderboard shows the scores as they are")
            modal.locator("#cancel-section").click()
        else:
            modal.locator("#apply-section").click()
            expect_visible(modal.get_by_text(re.compile("successfully recalculated", re.I)), "the scores recalculated",
                           timeout=LIVE_WAIT)
            page.keyboard.press("Escape")
        page.goto(run.base + "/leaderboard")
        expect_visible(page.locator(".podium .winner__name"), "the leaderboard's podium")
        names = page.locator(".podium .winner__name").all_inner_texts()
        s.notes.append("podium: " + ", ".join(n.strip() for n in names))


def cleanup(run):
    """What the UI allows: the watched word, a silence or setting left in place, the palette, the chat webhook and channel, the plugins turned on."""
    admin = run.personas.get("Admin")
    if not admin or not admin.page:
        return
    page = admin.page
    run.current = None

    def attempt(what, fn, left):
        try:
            fn()
            run.cleanup.append(what)
        except Exception as e:
            run.cleanup.append(f"couldn't: {what}: {brief(e) if isinstance(e, PlaywrightError) else e}")
            run.leave(left)

    if run.state.get("watched_word_added"):
        def word():
            page.goto(run.base + "/admin/customize/watched_words/action/require_approval")
            show = page.locator(".watched-words-detail input[type=checkbox], label:has-text('show') input")
            if show.count() and not show.last.is_checked():
                show.last.check()
            entry = expect_visible(page.locator(".watched-word", has_text=run.watched_word), "the watched word")
            entry.locator(".delete-word-record").click()
            expect_gone(page.locator(".watched-word", has_text=run.watched_word), "the watched word deleted")
        attempt(f"deleted the watched word {run.watched_word}", word, f"watched word {run.watched_word} (require approval)")
    if run.state.get("silenced"):
        username = run.state["silenced"]

        def unsilence():
            admin_user_page(run, page, username)
            page.locator("button", has_text=re.compile(r"^\s*Unsilence", re.I)).first.click()
            confirm = page.locator(".dialog-footer .btn-primary")
            if confirm.count():
                confirm.first.click()
            expect_visible(page.locator(".silence-user"), f"{username} unsilenced")
        attempt(f"unsilenced {username}", unsilence, f"{username} still silenced")
    if run.state.get("ext_added"):
        def ext():
            page.goto(run.base + "/admin/site_settings?filter=authorized_extensions")
            row = page.locator(".row.setting[data-setting='authorized_extensions']")
            row.locator("summary").first.click()
            row.locator(".selected-choice[data-name='txt']").click()
            page.keyboard.press("Escape")
            row.locator(".setting-controls__ok").click()
            expect_gone(row.locator(".setting-controls__ok"), "authorized_extensions saved")
        attempt("took txt back out of authorized_extensions", ext, "txt still in authorized_extensions")
    if run.state.get("old_title"):
        def title():
            page.goto(run.base + "/admin/site_settings?filter=title")
            page.locator(".row.setting[data-setting='title'] input").first.fill(run.state["old_title"])
            page.locator(".row.setting[data-setting='title'] .setting-controls__ok").click()
            expect_gone(page.locator(".row.setting[data-setting='title'] .setting-controls__ok"), "the title saved")
        attempt("put the site title back", title, "the site title still carries the rehearsal label")
    if run.state.get("palette"):
        name = run.state["palette"]

        def palette():
            page.goto(run.base + "/admin/customize/colors")
            page.locator(".d-filter-controls__input").fill(name)
            page.wait_for_timeout(500)
            card = expect_visible(page.locator("#main-outlet .color-palette, #main-outlet li, #main-outlet .admin-config-area-card").filter(has_text=name), "the palette")
            card.locator("button", has_text="Edit").first.click()
            expect_visible(page.locator(".delete-palette"), "the palette's Delete").click()
            expect_visible(page.locator(".dialog-footer .btn-danger, .dialog-footer .btn-primary"), "the delete confirmation").click()
            page.wait_for_timeout(1000)
            page.goto(run.base + "/admin/customize/colors")
            page.locator(".d-filter-controls__input").fill(name)
            page.wait_for_timeout(500)
            if page.locator("#main-outlet").get_by_text(name, exact=True).count():
                raise StepFail("it is still listed after Delete")
            run.left_behind[:] = [x for x in run.left_behind if not x.startswith("color palette")]
        attempt(f"deleted the palette {name}", palette, f"color palette '{name}'")
    if run.state.get("webhook"):
        hook = run.state["webhook"]

        def webhook():
            page.goto(run.base + "/admin/plugins/chat/hooks")
            row = expect_visible(page.locator(".incoming-chat-webhooks-row", has_text=hook), "the webhook")
            row.locator(".admin-chat-incoming-webhooks-delete").click()
            expect_visible(page.locator(".dialog-footer .btn-danger, .dialog-footer .btn-primary"), "the delete confirmation").click()
            expect_gone(page.locator(".incoming-chat-webhooks-row", has_text=hook), "the webhook deleted")
            run.left_behind[:] = [x for x in run.left_behind if not x.startswith(f"incoming webhook '{hook}'")]
        attempt(f"deleted the incoming webhook {hook}", webhook, f"incoming webhook '{hook}'")
    if run.state.get("channel_path"):
        path = run.state["channel_path"]
        name = run.state["channel_name"]

        def channel():
            page.goto(run.base + path + "/info/settings")
            expect_visible(page.locator(".delete-btn"), "the channel's Delete").click()
            expect_visible(page.locator("#channel-delete-confirm-name"), "the delete confirmation").fill(name)
            page.locator("#chat-confirm-delete-channel").click()
            # The modal says the deletion started, then goes to /chat.
            wait_until(page, "() => !location.pathname.includes('/info/settings')", None,
                       "the page to leave the deleted channel")
            answer = page.request.get(run.base + "/chat/api/channels/" + path.rsplit("/", 1)[1] + ".json")
            if answer.status != 404:
                raise StepFail(f"the channel still answers {answer.status} after Delete")
            run.left_behind[:] = [x for x in run.left_behind if not x.startswith(f"chat channel '{name}'")]
            run.leave(f"the deleted channel '{name}' (trashed) with its messages")
        attempt(f"deleted the chat channel {name}", channel, f"chat channel '{name}' at {path}")
    if run.state.get("plugins_turned_on"):
        def plugins():
            page.goto(run.base + "/admin/plugins")
            expect_visible(page.locator(".admin-plugins-list__row"), "the plugins list")
            for setting in run.state["plugins_turned_on"]:
                toggle_plugin(page, setting, False)
        attempt("turned off " + ", ".join(run.state["plugins_turned_on"]), plugins,
                "plugins left on: " + ", ".join(run.state["plugins_turned_on"]))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--url", default="http://127.0.0.1:8080")
    parser.add_argument("--out", required=True, help="output directory, must not exist")
    parser.add_argument("--server-log", help="the server's log file, to attach its request errors to each step")
    parser.add_argument("--only", help="comma-separated step numbers to run (the rest are skipped)")
    parser.add_argument("--headed", action="store_true")
    args = parser.parse_args()
    out = Path(args.out)
    if out.exists():
        sys.exit(f"{out} exists; give a new directory")
    out.mkdir(parents=True)
    run = Run(args)
    define_steps(run)
    only = {int(n) for n in args.only.split(",")} if args.only else None
    print(f"rehearsing {run.base} as '{run.label}' into {out}")
    with sync_playwright() as p:
        run.browser = p.chromium.launch(headless=not args.headed)
        run.make_files()
        build = run.build()
        for s in run.steps:
            if only and s.number not in only:
                s.status = "skipped"
                s.reason = "skipped (not in --only)"
                continue
            run.run_step(s)
        cleanup(run)
        counts = run.write(build)
        run.browser.close()
    print(f"{counts['ok']} ok, {counts['fail']} fail, {counts['skipped']} skipped: {out / 'index.html'}")
    return 0 if counts["fail"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
