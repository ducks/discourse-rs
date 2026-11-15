// ChatChannelRow's touch gestures in the mobile lists: a long press opens
// ChatChannelSidebarContextMenu (ChatOnLongPress), swiping a DM's row left
// past a third of the window leaves it, swiping a channel with unreads
// marks it read, and a short swipe goes back. Touches are the browser's
// own (CDP Input.dispatchTouchEvent) on a touch-enabled phone viewport.
// Every test has members and channels of its own.
const { test, expect, login, api, sql, member, label } = require("./helpers");

test.use({ viewport: { width: 400, height: 760 }, hasTouch: true, isMobile: true });

async function directMessage(context, other) {
  const dm = await api(context, "POST", "/chat/api/direct-message-channels.json", { "target_usernames[]": other.name });
  return dm.channel.id;
}

async function mobile(page, context) {
  const base = new URL(test.info().project.use.baseURL);
  await context.addCookies([{ name: "mobile_view", value: "1", domain: base.hostname, path: "/" }]);
}

// One finger on the row's middle; `to` moves it left by that many pixels
// in steps, then lifts it.
async function touch(page, row) {
  const cdp = await page.context().newCDPSession(page);
  const box = await row.boundingBox();
  const y = box.y + box.height / 2;
  const x = box.x + box.width - 40;
  const send = (type, px) =>
    cdp.send("Input.dispatchTouchEvent", { type, touchPoints: type === "touchEnd" ? [] : [{ x: px, y }] });
  await send("touchStart", x);
  return {
    async swipe(by) {
      for (let i = 1; i <= 8; i++) {
        await send("touchMove", x - (by * i) / 8);
      }
      await send("touchEnd");
    },
    end: () => send("touchEnd"),
  };
}

test("a long press on a direct message's row opens its menu, not the channel", async ({ page, context }) => {
  const me = member();
  const other = member();
  await login(context, me.name);
  const id = await directMessage(context, other);
  await mobile(page, context);
  await page.goto("/chat/direct-messages");
  await expect(page.locator("html")).toHaveClass(/discourse-touch/);
  await expect(page.locator("html")).toHaveClass(/mobile-view/);

  const row = page.locator(`.c-routes.--direct-messages .chat-channel-row[data-chat-channel-id='${id}']`);
  await expect(row).toBeVisible();
  const loaded = page.waitForResponse((r) => new URL(r.url()).pathname === `/chat/api/channels/${id}`);
  const finger = await touch(page, row);
  await loaded;
  const menu = page.locator(".chat-channel-sidebar-link-menu");
  await expect(menu).toBeVisible();
  await expect(row).not.toHaveClass(/is-long-pressed/);
  await finger.end();
  await expect(menu.locator(".chat-channel-sidebar-link-menu__leave-channel .d-button-label")).toHaveText("Close channel");
  await expect(menu.locator(".chat-channel-sidebar-link-menu__star-channel .d-button-label")).toHaveText(
    "Star channel",
  );
  // The release didn't follow the row's link.
  await expect(page).toHaveURL(/\/chat\/direct-messages$/);

  // Its items act on the channel: starring it, once the menu has
  // finished sliding in.
  await menu.evaluate((el) => Promise.all(el.getAnimations({ subtree: true }).map((a) => a.finished)));
  const starred = page.waitForResponse(
    (r) => r.request().method() === "PUT" && new URL(r.url()).pathname === `/chat/api/channels/${id}/memberships/me`,
  );
  await menu.locator(".chat-channel-sidebar-link-menu__star-channel").click();
  await starred;
  await expect
    .poll(() =>
      sql("SELECT starred FROM user_chat_channel_memberships WHERE chat_channel_id = :'p1' AND user_id = :'p2'", id, me.id),
    )
    .toEqual([["t"]]);
});

test("swiping a direct message's row leaves it, a short swipe goes back", async ({ page, context }) => {
  const me = member();
  const other = member();
  const kept = member();
  await login(context, me.name);
  const id = await directMessage(context, other);
  const keptId = await directMessage(context, kept);
  await mobile(page, context);
  await page.goto("/chat/direct-messages");

  const list = page.locator(".c-routes.--direct-messages");
  const keptRow = list.locator(`.chat-channel-row[data-chat-channel-id='${keptId}']`);
  await expect(keptRow).toBeVisible();

  // Under a third of the window: the row slides back, nothing is left.
  let finger = await touch(page, keptRow);
  await finger.swipe(80);
  await expect(keptRow.locator(".chat-channel-row__action-btn")).toHaveCount(0);
  await expect(keptRow.locator(".chat-channel-row__content")).toHaveCSS("margin-right", "0px");
  await expect(keptRow).toBeVisible();

  const row = list.locator(`.chat-channel-row[data-chat-channel-id='${id}']`);
  const left = page.waitForResponse(
    (r) =>
      r.request().method() === "DELETE" &&
      new URL(r.url()).pathname === `/chat/api/channels/${id}/memberships/me/follows`,
  );
  finger = await touch(page, row);
  await finger.swipe(220);
  await left;
  await expect(row).toHaveCount(0);
  await expect(keptRow).toBeVisible();
  expect(
    sql("SELECT following FROM user_chat_channel_memberships WHERE chat_channel_id = :'p1' AND user_id = :'p2'", id, me.id),
  ).toEqual([["f"]]);
});

test("swiping a channel with unreads marks it read", async ({ page, context, session }) => {
  const me = member();
  await login(context, me.name);
  const admin = await session("admin");
  const name = label("swipe").replace(/\s+/g, "-").slice(0, 40);
  const created = await api(admin, "POST", "/chat/api/channels.json", {
    "channel[chatable_id]": "2",
    "channel[name]": name,
  });
  const id = created.channel.id;
  await api(context, "POST", `/chat/api/channels/${id}/memberships/me.json`);
  await api(admin, "POST", `/chat/${id}.json`, { message: label("unread"), staged_id: label("staged") });
  const [[messageId]] = sql("SELECT max(id) FROM chat_messages WHERE chat_channel_id = :'p1'", id);
  await mobile(page, context);
  await page.goto("/chat/channels");

  const row = page.locator(`.c-routes.--channels .chat-channel-row[data-chat-channel-id='${id}']`);
  await expect(row).toHaveClass(/has-unread/);
  const read = page.waitForResponse(
    (r) => r.request().method() === "PUT" && new URL(r.url()).pathname === `/chat/api/channels/${id}/read`,
  );
  const finger = await touch(page, row);
  await finger.swipe(220);
  const response = await read;
  expect(new URL(response.url()).searchParams.get("message_id")).toBe(String(messageId));
  await expect(row).not.toHaveClass(/has-unread/);
  await expect(row.locator(".chat-channel-unread-indicator")).toHaveCount(0);
  await expect(row).toBeVisible();
  await expect
    .poll(() =>
      sql(
        "SELECT last_read_message_id FROM user_chat_channel_memberships WHERE chat_channel_id = :'p1' AND user_id = :'p2'",
        id,
        me.id,
      ),
    )
    .toEqual([[String(messageId)]]);
});
