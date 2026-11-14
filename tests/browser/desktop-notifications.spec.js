// Desktop notifications (static/js/desktop-notifications.js: Ember's
// lib/desktop-notifications, the desktop-notifications service's
// browser half and PushNotificationSelect in static/js/preferences.js):
// the preference's states, and an alert shown through the Notification
// API only in the primary tab, while enabled and out of do not disturb,
// its click going to the post.
//
// Headless Chromium shows no notifications: window.Notification is a
// stub recording what is constructed, its permission kept across the
// test's page loads in localStorage. Each test works on a member of its
// own; a new member is notified of more than the test's mentions (a
// badge granted), so the tests look for their own notification.
const { test, expect, login, api, topic, sql, member, label } = require("./helpers");

const STORE = "discourse_desktop_notifications_notifications-disabled";
const TRACKER = "discourse_desktop_notifications_focus-tracker";

// permission: what the browser holds as the test starts; answer: what a
// request for it gets; null permission: no Notification at all.
async function stub(context, permission, answer, enabled) {
  await context.grantPermissions(["notifications"]);
  await context.addInitScript(
    ([permission, answer, enabled, STORE]) => {
      if (permission === null) {
        delete window.Notification;
        return;
      }
      if (localStorage.getItem("__permission") === null) {
        localStorage.setItem("__permission", permission);
        if (enabled) {
          localStorage.setItem(STORE, enabled);
        }
      }
      window.__shown = [];
      window.__asked = 0;
      class Stub extends EventTarget {
        constructor(title, options) {
          super();
          this.title = title;
          this.options = options || {};
          this.closed = false;
          window.__shown.push(this);
        }
        close() {
          this.closed = true;
        }
        static get permission() {
          return localStorage.getItem("__permission");
        }
        static requestPermission(callback) {
          window.__asked += 1;
          localStorage.setItem("__permission", answer);
          if (callback) {
            callback(answer);
          }
          return Promise.resolve(answer);
        }
      }
      window.Notification = Stub;
    },
    [permission, answer, enabled || null, STORE],
  );
}

// What the page constructed: [{ title, body, icon, tag, closed }].
function shown(page) {
  return page.evaluate(() =>
    window.__shown.map((n) => ({ title: n.title, body: n.options.body, icon: n.options.icon, tag: n.options.tag, closed: n.closed })),
  );
}

// The notification whose body holds the text, once it is made.
async function shownWith(page, text) {
  let found;
  await expect
    .poll(async () => {
      found = (await shown(page)).find((n) => (n.body || "").includes(text));
      return !!found;
    })
    .toBe(true);
  return found;
}

function config(page) {
  return page.evaluate(() => JSON.parse(document.querySelector('script[src*="desktop-notifications"]').dataset.config));
}

async function choose(page, value) {
  const select = page.locator(".push-notification-select");
  await select.locator("summary").click();
  await select.locator(`.select-kit-row[data-value="${value}"]`).click();
}

function saved(page, level) {
  return page.waitForRequest(
    (r) => r.method() === "PUT" && /\/u\/[^/]+\.json$/.test(new URL(r.url()).pathname) && (r.postData() || "").includes(`push_notification_level=${level}`),
  );
}

function level(m) {
  return sql("SELECT push_notification_level FROM user_options WHERE user_id = :'p1'", m.id)[0][0];
}

// A topic by user1, and replies in it mentioning the member: each a post
// of its own, so each alerts. Returns the reply's text and URL.
async function mentions(session, m) {
  const author = await session("user1");
  const t = await topic(author, label("Desktop notifications"), label("The first post of a topic for desktop notifications"));
  let number = 1;
  return {
    topic: t,
    mention: async () => {
      const text = label("a mention to notify of");
      await api(author, "POST", "/posts.json", { topic_id: String(t.id), raw: `@${m.name} ${text}` });
      number += 1;
      return { text, url: `${t.url}/${number}` };
    },
  };
}

// The header's alert for the post: the stream's batch for it is in, the
// notification-alert event just behind.
async function alerted(page, url) {
  await expect(page.locator(`#notification-alert a[href$="${url}"]`)).toHaveCount(1);
}

test("the preference enables with the browser's prompt and a confirmation, and disables", async ({ page, context }) => {
  const m = member();
  await login(context, m.name);
  await stub(context, "default", "granted");
  await page.goto(`/u/${m.name}/preferences/notifications`);
  const group = page.locator(".control-group.desktop-notifications");
  await expect(group.locator(".control-label")).toHaveText("Live notifications");
  const select = group.locator(".push-notification-select");
  await expect(group.locator(".push-notification-config")).toHaveCount(0);
  // Not subscribed: none, whatever the saved level.
  expect(level(m)).toBe("1");
  await expect(select.locator("summary")).toHaveAttribute("data-value", "none");
  await expect(select.locator(".select-kit-selected-name .name")).toHaveText("Disabled");

  // Enabled: the prompt, the confirmation, the store and the level.
  let put = saved(page, "all");
  await choose(page, "all");
  await put;
  expect(await page.evaluate(() => window.__asked)).toBe(1);
  const c = await config(page);
  expect(await shown(page)).toEqual([
    {
      title: `Notifications enabled - ${c.title}`,
      body: "Success! Notifications have been enabled.",
      icon: c.icon,
      tag: "confirm-subscription",
      closed: false,
    },
  ]);
  expect(await page.evaluate((k) => localStorage.getItem(k), STORE)).toBe("enabled");
  await expect(select.locator("summary")).toHaveAttribute("data-value", "all");
  // confirmNotification's click closes it.
  await page.evaluate(() => window.__shown[0].dispatchEvent(new Event("click")));
  expect((await shown(page))[0].closed).toBe(true);

  // Drawn enabled the next time.
  await expect.poll(() => level(m)).toBe("1");
  await page.reload();
  await expect(select.locator("summary")).toHaveAttribute("data-value", "all");
  await expect(select.locator(".select-kit-selected-name .name")).toHaveText("Enabled");

  // Disabled: the store, and the level saved as none.
  put = saved(page, "none");
  await choose(page, "none");
  await put;
  expect(await page.evaluate((k) => localStorage.getItem(k), STORE)).toBe("disabled");
  await expect.poll(() => level(m)).toBe("0");
  expect(await page.evaluate(() => window.__asked)).toBe(0);
  await page.reload();
  await expect(select.locator("summary")).toHaveAttribute("data-value", "none");

  // Enabled again without a prompt to answer: permission is granted.
  put = saved(page, "all");
  await choose(page, "all");
  await put;
  expect(await page.evaluate((k) => localStorage.getItem(k), STORE)).toBe("enabled");
  await expect.poll(() => level(m)).toBe("1");
});

test("a refused prompt leaves the preference disabled, a denied permission says so", async ({ page, context }) => {
  const m = member();
  await login(context, m.name);
  await stub(context, "default", "denied");
  await page.goto(`/u/${m.name}/preferences/notifications`);
  const group = page.locator(".control-group.desktop-notifications");
  const select = group.locator(".push-notification-select");
  const puts = [];
  page.on("request", (r) => {
    if (r.method() === "PUT") {
      puts.push(r.postData() || "");
    }
  });
  await choose(page, "all");
  await expect.poll(() => page.evaluate(() => window.__asked)).toBe(1);
  await expect(select.locator("summary")).toHaveAttribute("data-value", "none");
  expect(await page.evaluate((k) => localStorage.getItem(k), STORE)).toBeNull();

  // Denied: the button and its explanation in the select's place.
  await page.reload();
  await expect(select).toHaveCount(0);
  const button = group.locator("button.btn-default[disabled]");
  await expect(button).toHaveText("Permission denied");
  await expect(button.locator("svg.d-icon-bell-slash")).toHaveCount(1);
  await expect(group.locator("button + span")).toHaveText(
    "You denied permission for notifications. Allow notifications via your browser settings.",
  );
  await expect(group.locator(".instructions")).toContainText("You have to change this setting on every browser you use.");
  expect(puts).toEqual([]);
  expect(level(m)).toBe("1");
});

test("a browser without notifications says they are not supported", async ({ page, context }) => {
  const m = member();
  await login(context, m.name);
  await stub(context, null);
  await page.goto(`/u/${m.name}/preferences/notifications`);
  const group = page.locator(".control-group.desktop-notifications");
  await expect(group.locator(".push-notification-select")).toHaveCount(0);
  await expect(group.locator("button.btn-default[disabled]")).toHaveText("Notifications are not supported on this browser. Sorry.");
});

test("a mention is shown as a notification whose click goes to the post", async ({ page, context, session }) => {
  const m = member();
  await login(context, m.name);
  await stub(context, "granted", "granted", "enabled");
  const { topic: t, mention } = await mentions(session, m);
  await page.goto("/latest");
  const c = await config(page);
  const title = await page.evaluate((id) => fetch(`/t/${id}.json`).then((r) => r.json()).then((j) => j.title), t.id);

  const first = await mention();
  const note = await shownWith(page, first.text);
  expect(note).toEqual({
    title: `user1 mentioned you in "${title}" - ${c.title}`,
    body: `@${m.name} ${first.text}`,
    icon: c.icon,
    tag: `discourse-notification-${c.title}-${t.id}`,
    closed: false,
  });
  // site_logo_small_url, before site_logo_url (the header's).
  expect(c.icon).toMatch(/\/images\/discourse-logo-sketch-small\.png$/);
  expect(await page.locator("#site-logo").getAttribute("src")).not.toBe(c.icon);
  expect(await page.evaluate(() => window.__asked)).toBe(0);
  // One for the alert.
  expect((await shown(page)).filter((n) => (n.body || "").includes(first.text))).toHaveLength(1);

  await page.evaluate((text) => window.__shown.find((n) => (n.options.body || "").includes(text)).dispatchEvent(new Event("click")), first.text);
  await expect(page).toHaveURL(new RegExp(`${first.url}$`));
  await expect(page.locator("#post_2")).toContainText(first.text);
  expect((await shown(page)).find((n) => (n.body || "").includes(first.text)).closed).toBe(true);
});

test("nothing is shown in a tab that isn't the primary one, while disabled or in do not disturb", async ({ page, context, session }) => {
  const m = member();
  await login(context, m.name);
  await stub(context, "granted", "granted", "enabled");
  const { topic: t, mention } = await mentions(session, m);
  await page.goto("/latest");

  // Another tab focused after this one (its focus-tracker write): the
  // alert arrives, and no notification is made of it.
  await page.evaluate((key) => window.dispatchEvent(new StorageEvent("storage", { key, newValue: "another-tab" })), TRACKER);
  const other = await mention();
  await alerted(page, other.url);
  // Focused again, the primary tab: Ember shows it whether or not the
  // tab is in sight.
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  const primary = await mention();
  await shownWith(page, primary.text);

  // Disabled in this browser.
  await page.evaluate((k) => localStorage.setItem(k, "disabled"), STORE);
  const disabled = await mention();
  await alerted(page, disabled.url);
  await page.evaluate((k) => localStorage.setItem(k, "enabled"), STORE);
  const enabled = await mention();
  await shownWith(page, enabled.text);

  // Do not disturb: the stream sends no alert; the next one after it
  // ends is the first the page hears of.
  const moon = page.locator("#toggle-current-user .do-not-disturb-background");
  await api(context, "POST", "/do-not-disturb.json", { duration: "30" });
  await expect(moon).toHaveCount(1);
  const paused = await mention();
  await expect
    .poll(() => sql("SELECT count(*) FROM notifications WHERE user_id = :'p1' AND topic_id = :'p2' AND notification_type = 1", m.id, t.id)[0][0])
    .toBe("5");
  await api(context, "DELETE", "/do-not-disturb.json");
  await expect(moon).toHaveCount(0);
  const resumed = await mention();
  await shownWith(page, resumed.text);

  const bodies = (await shown(page)).map((n) => n.body || "");
  for (const quiet of [other, disabled, paused]) {
    expect(bodies.filter((b) => b.includes(quiet.text))).toEqual([]);
  }
});

test("a browser that denied permission is shown nothing, and a visitor has no script", async ({ page, context, session }) => {
  const m = member();
  await login(context, m.name);
  await stub(context, "denied", "denied", "enabled");
  const { mention } = await mentions(session, m);
  await page.goto("/latest");
  const quiet = await mention();
  await alerted(page, quiet.url);
  const again = await mention();
  await alerted(page, again.url);
  expect((await shown(page)).filter((n) => (n.body || "").includes(quiet.text))).toEqual([]);
  expect(await page.evaluate(() => window.__asked)).toBe(0);

  const visitor = await session(null);
  const anon = await visitor.newPage();
  await anon.goto("/latest");
  await expect(anon.locator('script[src*="desktop-notifications"]')).toHaveCount(0);
  expect(await anon.evaluate(() => typeof Discourse.desktopNotifications)).toBe("undefined");
});

test("an installed app's member is asked once by the consent banner", async ({ page, context }) => {
  const m = member();
  await login(context, m.name);
  await stub(context, "default", "granted");
  // capabilities.isPwa
  await context.addInitScript(() => {
    const real = window.matchMedia.bind(window);
    window.matchMedia = (query) => (query === "(display-mode: standalone)" ? { matches: true, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} } : real(query));
  });
  await page.goto("/latest");
  const banner = page.locator("#main-container > .row > .consent_banner.alert.alert-info");
  await expect(banner).toHaveCount(1);
  await expect(banner.locator("> span")).toContainText("Do you want live notifications when people reply to your posts?");
  await expect(banner.locator("span > button.btn-link")).toHaveText("Enable notifications");
  await expect(banner.locator("> button.btn-transparent.close svg.d-icon-xmark")).toHaveCount(1);

  // Kept across pages until answered.
  await page.locator('a[href$="/categories"]').first().click();
  await expect(page).toHaveURL(/\/categories$/);
  await expect(banner).toHaveCount(1);

  await banner.locator("button.btn-link").click();
  await expect(banner).toHaveCount(0);
  expect(await page.evaluate(() => window.__asked)).toBe(1);
  expect(await page.evaluate((k) => localStorage.getItem(k), STORE)).toBe("enabled");
  expect(await page.evaluate(() => localStorage.getItem("discourse_push_notifications_dismissed-prompt"))).toBe("true");
  expect((await shown(page)).map((n) => n.tag)).toEqual(["confirm-subscription"]);
  await page.reload();
  await expect(page.locator("#main-container")).toHaveCount(1);
  await expect(banner).toHaveCount(0);
});

test("the consent banner's close dismisses it for good, and a browser tab has none", async ({ page, context }) => {
  const m = member();
  await login(context, m.name);
  await stub(context, "default", "granted");
  await page.goto("/latest");
  // Not an installed app.
  await expect(page.locator("#main-container")).toHaveCount(1);
  await expect(page.locator(".consent_banner")).toHaveCount(0);

  await page.addInitScript(() => {
    const real = window.matchMedia.bind(window);
    window.matchMedia = (query) => (query === "(display-mode: standalone)" ? { matches: true, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} } : real(query));
  });
  await page.reload();
  const banner = page.locator(".consent_banner");
  await expect(banner).toHaveCount(1);
  await banner.locator("button.close").click();
  await expect(banner).toHaveCount(0);
  expect(await page.evaluate(() => window.__asked)).toBe(0);
  expect(await page.evaluate(() => localStorage.getItem("discourse_push_notifications_dismissed-prompt"))).toBe("false");
  await page.reload();
  await expect(page.locator("#main-container")).toHaveCount(1);
  await expect(banner).toHaveCount(0);
});
