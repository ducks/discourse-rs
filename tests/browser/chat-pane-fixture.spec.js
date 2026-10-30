// chat.js's channel pane over a fixture channel of 200 messages (224's
// harness, kept): the near-message URL's target highlighted and the URL
// replaced, older pages on scrolling up to the first message, newer pages
// on scrolling down, the scroll to bottom arrow both ways, jumps to a
// loaded and an unloaded message, and the error of a target that can't
// load. The page and /live/chat/2/messages are answered by page.route
// with the markup the server draws; nothing reaches the network.
const { test, expect, asset } = require("./helpers");

const FIXTURE = "http://fixture.test";
const TOTAL = 200;

function msg(id, extra) {
  const at = new Date(Date.UTC(2026, 9, 1, 0, id)).toISOString();
  const reply = id % 7 === 0 ? `<a class="chat-reply" href="/chat/c/general/2/${id - 60}">reply</a>` : "";
  return `<div class="chat-message-container -persisted -processed${extra || ""}" data-id="${id}" data-created-at="${at}" data-user-id="2" data-username="u" data-avatar-template="" data-message="m${id}" data-excerpt="m${id}"><div class="chat-message-text"><div class="chat-cooked"><p>message ${id}</p>${reply}</div></div></div>`;
}

function range(a, b) {
  const r = [];
  for (let i = Math.max(1, a); i <= Math.min(TOTAL, b); i++) r.push(i);
  return r;
}

function channelPage(target, { loaded = true, scripts = "" } = {}) {
  const shown = target ? range(target - 25, target + 25) : range(TOTAL - 49, TOTAL);
  const past = shown[0] > 1;
  const future = shown[shown.length - 1] < TOTAL;
  return `<!doctype html><html lang="en"><head><script src="/page.js"></script>
<style>
.full-page-chat{height:600px;display:flex;flex-direction:column}
.main-chat-outlet{height:100%}
.chat-channel{display:flex;flex-direction:column;height:100%;position:relative}
.chat-messages-scroller{flex:1;overflow-y:scroll;display:flex;flex-direction:column-reverse;}
.chat-messages-container{display:flex;flex-direction:column;position:relative}
.chat-message-container{height:40px}
.chat-message-separator-date{position:absolute;width:100%;pointer-events:none}
.chat-scroll-to-bottom__button{display:none}.chat-scroll-to-bottom__button.visible{display:block}
</style></head><body>
<div class="full-page-chat" data-base-path=""><div class="main-chat-outlet">
<div class="chat-channel${loaded ? " --loaded" : ""}" data-id="2" data-label-today="Today" data-label-yesterday="Yesterday" data-label-last-visit="last visit" data-format-time="h:mm a" data-format-tiny="h:mm" data-format-title="MMMM D, YYYY h:mm a" data-format-date="MMMM D, YYYY" data-last-read="0" data-following="false" data-viewer-id="3" data-staff="false" data-can-moderate="false" data-label-deleted-one="1 deleted" data-label-deleted-other="%{count} deleted" data-direct-message="false" data-can-load-more-past="${past}" data-can-load-more-future="${future}" data-label-all-loaded="Showing all messages" data-label-not-found="Not found"${target ? ` data-target="${target}"` : ""}>
<div class="chat-messages-scroller"><div class="chat-messages-container">${shown.map((i) => msg(i)).join("")}</div></div>
<div class="chat-scroll-to-bottom"><button class="btn no-text btn-flat chat-scroll-to-bottom__button" type="button">v</button></div>
</div></div></div>
${scripts}<script src="/chat.js"></script></body></html>`;
}

let requests;

test.beforeEach(async ({ page }) => {
  requests = [];
  await page.setViewportSize({ width: 1200, height: 800 });
  await page.route("**/*", (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/page.js") {
      return route.fulfill({ contentType: "text/javascript; charset=utf-8", body: asset("js/page.js") });
    }
    if (url.pathname === "/chat.js") {
      return route.fulfill({ contentType: "text/javascript; charset=utf-8", body: asset("js/chat.js") });
    }
    if (url.pathname === "/dialog.js") {
      // popupAjaxError, recorded.
      return route.fulfill({
        contentType: "text/javascript; charset=utf-8",
        body: "window.Discourse=window.Discourse||{};Discourse.dialog={popupAjaxError:function(t){window.popped=t;}};",
      });
    }
    if (url.pathname === "/live/chat/2/messages") {
      const q = url.searchParams;
      requests.push(url.search);
      const t = q.get("target_message_id") ? Number(q.get("target_message_id")) : null;
      const d = q.get("direction");
      let ids;
      let past = null;
      let future = null;
      let anchor = "";
      if (d === "past") {
        ids = range(t - 50, t - 1);
        past = ids[0] > 1;
        anchor = `<template class="chat-message-anchor" data-id="${t}">${msg(t, " redrawn")}</template>`;
      } else if (d === "future") {
        ids = range(t + 1, t + 50);
        future = ids[ids.length - 1] < TOTAL;
      } else if (t) {
        if (t > TOTAL) return route.fulfill({ status: 404, body: "" });
        ids = range(t - 25, t + 25);
        past = ids[0] > 1;
        future = ids[ids.length - 1] < TOTAL;
      } else {
        ids = range(TOTAL - 49, TOTAL);
        past = true;
        future = false;
      }
      return route.fulfill({
        status: 200,
        contentType: "text/html",
        headers: { "x-chat-can-load-more-past": String(past), "x-chat-can-load-more-future": String(future) },
        body: ids.map((i) => msg(i)).join("") + anchor,
      });
    }
    const m = /^\/chat\/c\/general\/2(?:\/(\d+))?$/.exec(url.pathname);
    if (m) {
      const target = m[1] ? Number(m[1]) : null;
      if (target > TOTAL) {
        // The unloaded channel with the target's error to pop.
        return route.fulfill({
          contentType: "text/html",
          body: channelPage(null, { loaded: false, scripts: '<script src="/dialog.js"></script>' }),
        });
      }
      return route.fulfill({ contentType: "text/html", body: channelPage(target) });
    }
    return route.fulfill({ status: 404, body: "" });
  });
});

const scroller = ".chat-messages-scroller";
const toTop = (page) =>
  page.evaluate((s) => {
    const el = document.querySelector(s);
    el.scrollTop = -(el.scrollHeight - el.clientHeight);
  }, scroller);
const arrow = (page) => page.locator(".chat-scroll-to-bottom__button");
const ids = (page) =>
  page.evaluate(() =>
    Array.from(document.querySelectorAll(".chat-messages-container > .chat-message-container")).map((n) =>
      Number(n.dataset.id),
    ),
  );

test("the near-message URL, then older pages up to the first message", async ({ page }) => {
  await page.goto(`${FIXTURE}/chat/c/general/2/100`);
  // ChatChannelNearMessage: the URL replaced with the channel's, the
  // target in view and highlighted for two seconds.
  await expect.poll(() => new URL(page.url()).pathname).toBe("/chat/c/general/2");
  const target = page.locator('.chat-message-container[data-id="100"]');
  await expect(target).toHaveClass(/-highlighted/);
  expect(
    await page.evaluate(() => {
      const s = document.querySelector(".chat-messages-scroller").getBoundingClientRect();
      const m = document.querySelector('.chat-message-container[data-id="100"]').getBoundingClientRect();
      return m.top >= s.top && m.bottom <= s.bottom;
    }),
  ).toBe(true);
  // Newer messages to load: the arrow shows.
  await expect(arrow(page)).toHaveClass(/visible/);
  await expect(target).not.toHaveClass(/-highlighted/, { timeout: 4000 });

  // Scrolling up loads the page before the first shown; that first one
  // comes back drawn again and is swapped in keeping its classes.
  requests.length = 0;
  await toTop(page);
  await expect(page.locator('.chat-message-container[data-id="50"]')).toBeAttached();
  expect(requests[0]).toBe("?target_message_id=75&direction=past&newest=true");
  const anchor = page.locator('.chat-message-container[data-id="75"]');
  await expect(anchor).not.toHaveClass(/redrawn/);
  await expect(anchor).toHaveClass(/-persisted/);

  // On to the start with the mouse wheel, as a reader does: a scroll that
  // lands inside the 50ms throttle after a page's reflow is dropped, as in
  // ChatScrollableList, and the wheel's own events (it listens to both)
  // carry on at the top. The first message, the all loaded line, every
  // message once and in order.
  const box = await page.locator(scroller).boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await expect(async () => {
    await page.mouse.wheel(0, -4000);
    await expect(page.locator('.chat-message-container[data-id="1"]')).toBeAttached({ timeout: 300 });
  }).toPass({ intervals: [300], timeout: 15_000 });
  await expect(page.locator(".all-loaded-message")).toBeAttached();
  const order = await ids(page);
  expect(order[0]).toBe(1);
  expect(order.every((v, i) => i === 0 || v === order[i - 1] + 1)).toBe(true);

  // Newer pages aren't known after paging back (null): the arrow scrolls
  // to the bottom, which then pages forward.
  requests.length = 0;
  await arrow(page).click();
  await expect(page.locator('.chat-message-container[data-id="126"]')).toBeAttached();
  expect(requests[0]).toBe("?target_message_id=125&direction=future&newest=true");
});

test("the arrow loads the latest page, jumps load around an unloaded target or stay in place", async ({ page }) => {
  await page.goto(`${FIXTURE}/chat/c/general/2/100`);
  await expect(page.locator('.chat-message-container[data-id="100"]')).toHaveClass(/-highlighted/);

  // Newer known to load: the arrow brings the latest page, at the bottom.
  requests.length = 0;
  await arrow(page).click();
  await expect(page.locator('.chat-message-container[data-id="200"]')).toBeAttached();
  expect(requests[0]).toBe("?newest=true");
  await expect
    .poll(() => page.evaluate((s) => Math.abs(document.querySelector(s).scrollTop), scroller))
    .toBeLessThan(2);

  // A reply line to a message not loaded: the page around it, highlighted,
  // the URL unchanged.
  requests.length = 0;
  await page.evaluate(() => document.querySelector('.chat-message-container[data-id="196"] .chat-reply').click());
  await expect(page.locator('.chat-message-container[data-id="136"]')).toHaveClass(/-highlighted/);
  expect(requests[0]).toBe("?target_message_id=136&newest=true");
  expect(new URL(page.url()).pathname).toBe("/chat/c/general/2");

  // One already loaded: in place, no request.
  requests.length = 0;
  await page.evaluate(() => {
    const a = document.createElement("a");
    a.href = "/chat/c/general/2/120";
    document.querySelector('.chat-message-container[data-id="150"] .chat-cooked').appendChild(a);
    a.click();
  });
  await expect(page.locator('.chat-message-container[data-id="120"]')).toHaveClass(/-highlighted/);
  expect(requests).toEqual([]);

  // Scrolling to the bottom loads the newer page.
  requests.length = 0;
  await page.evaluate((s) => (document.querySelector(s).scrollTop = 0), scroller);
  await expect(page.locator('.chat-message-container[data-id="162"]')).toBeAttached();
  expect(requests.some((r) => r.startsWith("?target_message_id=161&direction=future"))).toBe(true);
});

test("a target that can't load pops its error over the unloaded channel", async ({ page }) => {
  await page.goto(`${FIXTURE}/chat/c/general/2/999`);
  await expect.poll(() => page.evaluate(() => window.popped)).toBe("Not found");
});
