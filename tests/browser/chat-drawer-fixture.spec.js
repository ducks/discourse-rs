// chat-drawer.js over fixture pages (223b and 223d's harnesses, kept):
// the resizer's minimums and the composer's resize events, which the
// server's pages make hard to reach, and the browser's back and forward
// to a chat URL with the drawer preferred. Every request is answered by
// page.route; nothing reaches the server or the network.
const { test, expect, asset } = require("./helpers");

const FIXTURE = "http://fixture.test";

test.describe("the drawer's resizer and the composer's resize events (223b)", () => {
  test.beforeEach(async ({ page }) => {
    await page.route("**/*", (route) => {
      const url = route.request().url();
      if (url.includes("/live/chat/drawer")) {
        return route.fulfill({
          status: 200,
          contentType: "text/html",
          headers: { "x-chat-drawer-routes": "chat.channels" },
          body: '<div class="c-drawer-routes --channels"><p>channels</p></div>',
        });
      }
      if (url.endsWith("/page.js")) {
        return route.fulfill({ contentType: "text/javascript", body: asset("js/page.js") });
      }
      if (url.endsWith("/chat-drawer.js")) {
        return route.fulfill({ contentType: "text/javascript", body: asset("js/chat-drawer.js") });
      }
      return route.fulfill({
        status: 200,
        contentType: "text/html",
        body: `<!doctype html><html><head>
<style>
.chat-drawer-outlet-container{position:fixed;right:15px;bottom:0;display:flex;align-items:flex-end;}
.chat-drawer{min-width:250px!important;min-height:300px!important;width:400px}
.chat-drawer-container{position:relative;height:100%}
.chat-drawer-resizer{position:absolute;top:-5px;left:-5px;width:15px;height:15px}
#reply-control{position:fixed;bottom:0;left:0;width:300px;height:200px}
</style>
<script src="/page.js"></script></head><body>
<div id="main-outlet"></div>
<div id="reply-control" class="closed"></div>
<div class="below-footer-outlet chat-drawer-outlet"><div class="chat-drawer-outlet-container"></div></div>
<script src="/chat-drawer.js" data-homepage="/latest" data-scripts=""></script>
</body></html>`,
      });
    });
  });

  const stored = (page) =>
    page.evaluate(() => [
      localStorage.getItem("discourse_chat_drawer_size_width"),
      localStorage.getItem("discourse_chat_drawer_size_height"),
    ]);

  async function drag(page, dx, dy) {
    const box = await page.locator(".chat-drawer-resizer").boundingBox();
    await page.mouse.move(box.x + 7, box.y + 7);
    await page.mouse.down();
    await page.mouse.move(box.x + 7 + dx, box.y + 7 + dy);
    await page.mouse.up();
  }

  test("a drag sets and stores the size, kept over the minimums", async ({ page }) => {
    await page.goto(`${FIXTURE}/latest`);
    // "-" opens the drawer.
    await page.keyboard.press("-");
    await expect(page.locator(".chat-drawer .c-drawer-routes")).toBeAttached();
    await expect(page.locator(".chat-drawer")).toHaveAttribute("style", "width: 400px;height: 530px;");

    await drag(page, -100, -50);
    expect(await stored(page)).toEqual(["500", "580"]);
    await expect(page.locator(".chat-drawer")).toHaveAttribute("style", /width: 500px/);
    await expect(page.locator(".chat-drawer")).toHaveAttribute("style", /height: 580px/);

    // Under the minimum: the drawer stays as it was, the store keeps
    // ChatDrawerSize's minimums.
    await drag(page, 400, 400);
    expect(await stored(page)).toEqual(["250", "300"]);
    await expect(page.locator(".chat-drawer")).toHaveAttribute("style", /width: 500px/);

    // A toggle computes the style again from the store.
    await page.evaluate(() => {
      const b = document.createElement("button");
      b.className = "c-navbar__toggle-drawer-button";
      document.querySelector(".c-drawer-routes").appendChild(b);
      b.click();
    });
    await expect(page.locator(".chat-drawer")).toHaveAttribute("style", "width: 250px;height: 300px;");
  });

  test("the composer's resize events turn transitions off and check the offset", async ({ page }) => {
    await page.goto(`${FIXTURE}/latest`);
    await page.keyboard.press("-");
    await expect(page.locator(".chat-drawer .c-drawer-routes")).toBeAttached();
    const outlet = page.locator(".chat-drawer-outlet-container");
    const fire = (name) =>
      page.evaluate((name) => {
        const c = document.getElementById("reply-control");
        c.classList.remove("closed");
        c.dispatchEvent(new CustomEvent(name, { bubbles: true }));
      }, name);

    await fire("composer:resize-started");
    await expect(outlet).toHaveClass(/clear-transitions/);
    await fire("composer:div-resizing");
    await expect
      .poll(() => outlet.evaluate((o) => o.style.getPropertyValue("--composer-right")))
      .toBe("15px");
    await fire("composer:resize-ended");
    await expect(outlet).not.toHaveClass(/clear-transitions/);
  });
});

test.describe("back and forward to a chat URL (223d)", () => {
  function fixturePage(path) {
    const chat = path.startsWith("/chat");
    const main = chat
      ? '<div class="full-page-chat"><div class="main-chat-outlet"><button class="c-navbar__open-drawer-button">drawer</button></div></div>'
      : `<h1>${path}</h1><a id="to-topic" href="/t/topic/1">topic</a> <a id="to-chat" href="/chat/c/general/2">chat</a> <a id="to-latest" href="/latest">latest</a>`;
    return `<!doctype html><html><head><meta name="htmx-config" content='{"historyCacheSize": 0}'>
<script src="/page.js"></script><script src="/htmx.min.js" defer></script></head>
<body hx-boost="true"><div id="main-outlet">${main}</div>
<div id="reply-control" class="closed"></div>
<div class="below-footer-outlet chat-drawer-outlet" id="chat-drawer-outlet" hx-preserve><div class="chat-drawer-outlet-container"></div></div>
<script src="/chat-drawer.js" data-homepage="/latest" data-scripts=""></script>
</body></html>`;
  }

  let requests;
  test.beforeEach(async ({ page }) => {
    requests = [];
    await page.route("**/*", (route) => {
      const url = new URL(route.request().url());
      if (url.pathname === "/live/chat/drawer") {
        requests.push("drawer " + url.searchParams.get("url"));
        return route.fulfill({
          status: 200,
          contentType: "text/html",
          headers: { "x-chat-drawer-routes": "chat.channel", "x-chat-drawer-channel": "2" },
          body: '<div class="c-drawer-routes --channel"><textarea id="channel-composer"></textarea></div>',
        });
      }
      if (url.pathname === "/page.js") {
        return route.fulfill({ contentType: "text/javascript", body: asset("js/page.js") });
      }
      if (url.pathname === "/htmx.min.js") {
        return route.fulfill({ contentType: "text/javascript", body: asset("vendor/htmx.min.js") });
      }
      if (url.pathname === "/chat-drawer.js") {
        return route.fulfill({ contentType: "text/javascript", body: asset("js/chat-drawer.js") });
      }
      requests.push("page " + url.pathname);
      return route.fulfill({ status: 200, contentType: "text/html; charset=utf-8", body: fixturePage(url.pathname) });
    });
  });

  const prefer = (page, mode) =>
    page.evaluate((mode) => localStorage.setItem("discourse_chat_preferred_mode", JSON.stringify(mode)), mode);

  test("back opens the chat URL in the drawer, forward restores the forum page", async ({ page }) => {
    await page.goto(`${FIXTURE}/latest`);
    await prefer(page, "FULL_PAGE_CHAT");
    await page.click("#to-chat");
    await expect(page.locator(".full-page-chat")).toBeAttached();
    // Full-page chat's open drawer button: back to the forum, the drawer
    // at the chat URL.
    await page.click(".c-navbar__open-drawer-button");
    await expect(page.locator(".chat-drawer #channel-composer")).toBeAttached();
    expect(new URL(page.url()).pathname).toBe("/latest");

    // Close the drawer, then go back to the chat URL.
    await page.evaluate(() => {
      const b = document.createElement("button");
      b.className = "c-navbar__close-drawer-button";
      document.querySelector(".c-drawer-routes").appendChild(b);
      b.click();
    });
    await expect(page.locator(".chat-drawer")).toHaveCount(0);
    requests.length = 0;
    await page.goBack({ waitUntil: "commit" });
    await expect(page.locator(".chat-drawer #channel-composer")).toBeAttached();
    // The address bar on the chat URL as Ember leaves it, the forum page
    // still shown, and only the drawer's route asked for.
    expect(new URL(page.url()).pathname).toBe("/chat/c/general/2");
    await expect(page.locator("h1")).toBeAttached();
    await expect(page.locator(".full-page-chat")).toHaveCount(0);
    expect(requests).toEqual(["drawer /chat/c/general/2"]);

    requests.length = 0;
    await page.goForward({ waitUntil: "commit" });
    await page.waitForFunction(() => location.pathname === "/latest");
    await expect.poll(() => requests).toContain("page /latest");

    // Full page preferred: back loads full-page chat and closes the drawer.
    await prefer(page, "FULL_PAGE_CHAT");
    await page.goBack({ waitUntil: "commit" });
    await expect(page.locator(".full-page-chat")).toBeAttached();
    await expect(page.locator(".chat-drawer")).toHaveCount(0);
  });

  test("a forum page swapped in takes the focus from the drawer's composer", async ({ page }) => {
    await page.goto(`${FIXTURE}/latest`);
    await prefer(page, "DRAWER_CHAT");
    await page.keyboard.press("-");
    await expect(page.locator(".chat-drawer #channel-composer")).toBeAttached();
    await page.focus("#channel-composer");
    await page.evaluate(() => document.getElementById("to-topic").click());
    await expect(page.locator("h1", { hasText: "/t/topic/1" })).toBeAttached();
    expect(await page.evaluate(() => document.activeElement === document.body)).toBe(true);
    await expect(page.locator(".chat-drawer #channel-composer")).toBeAttached();
  });
});
