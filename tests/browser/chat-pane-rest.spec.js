// The rest of the channel pane (224a) on a running server: the
// mention_without_membership notice and its invite, the in-channel filter,
// the date separator's jump, the saved scroll position across navigation,
// the decorators chat.js runs on messages (spoilers, hashtag icons, code
// block buttons), the stop streaming button, the "+" menu's local dates
// and mobile emoji, the mobile arrow's new messages count, and a hidden
// tab neither reading nor following new messages. Specs that scroll get a
// channel of their own (in General's category), filled with sql().
const { test, expect, login, api, sql, label, member } = require("./helpers");

const GENERAL = 2; // the seed's General channel
const SCROLLER = ".chat-messages-scroller";

// A new channel that user0 and user1 follow, with `count` of user0's
// messages a minute apart (or by `at`, a SQL expression of n), user1
// having read them all. Returns { id, url, ids }.
async function filledChannel(session, count, at) {
  const admin = await session("admin");
  const name = label("pane");
  const created = await api(admin, "POST", "/chat/api/channels", {
    "channel[chatable_id]": "4",
    "channel[name]": name,
  });
  const id = created.channel.id;
  for (const user of ["user0", "user1"]) {
    await api(await session(user), "POST", `/chat/api/channels/${id}/memberships/me`);
  }
  const when = at || `now() - (${count + 1} - n) * interval '1 minute'`;
  const ids = sql(
    `INSERT INTO chat_messages (chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id)
     SELECT :'p1'::int, 2, ${when}, now(), 'pane message ' || n, '<p>pane message ' || n || '</p>', 1, 2
     FROM generate_series(1, :'p2'::int) n ORDER BY n RETURNING id`,
    id,
    count,
  ).map((r) => Number(r[0]));
  sql("UPDATE chat_channels SET last_message_id = :'p2'::bigint WHERE id = :'p1'::int", id, ids[ids.length - 1]);
  sql(
    "UPDATE user_chat_channel_memberships SET last_read_message_id = :'p2'::bigint WHERE chat_channel_id = :'p1'::int AND user_id = (SELECT id FROM users WHERE username = 'user1')",
    id,
    ids[ids.length - 1],
  );
  return { id, url: `/chat/c/${created.channel.slug}/${id}`, ids };
}

// Another chat page as htmx swaps it in (the sidebar's links do), the
// page's scripts kept.
async function swapTo(page, url) {
  await page.evaluate((href) => {
    const a = document.createElement("a");
    a.href = href;
    a.hidden = true;
    document.body.appendChild(a);
    window.htmx.process(a);
    a.click();
  }, url);
  await expect.poll(() => new URL(page.url()).pathname).toBe(url);
}

async function openChannel(page, url) {
  await page.goto(url);
  await expect(page.locator(".full-page-chat .chat-composer__input")).toBeVisible();
}

const message = (page, id) => page.locator(`.chat-message-container[data-id="${id}"]`);

// A member of the test's own, never seen: a member's first request
// auto-joins them to General (Chat::AutoJoinChannels on user_seen), as
// it would user2 once any spec signs in as them.
test("a mention of a member who can't see it brings a notice whose invite notifies them", async ({ page, context }) => {
  const invitee = member();
  await login(context, "user1");
  await openChannel(page, `/chat/c/general/${GENERAL}`);
  const composer = page.locator(".chat-composer__input");

  await composer.fill(`@${invitee.name} ${label("mention")}`);
  await composer.press("Enter");
  const notice = page.locator(".chat-notices .chat-notices__notice");
  await expect(notice.locator(".mention-without-membership-notice__body__text")).toContainText(invitee.name, {
    timeout: 20_000,
  });
  await notice.locator(".mention-without-membership-notice__body__link").click();
  await expect(notice.locator(".mention-without-membership-notice__invitation-sent")).toHaveText("Invitation sent");
  expect(new URL(page.url()).pathname).toBe(`/chat/c/general/${GENERAL}`);
  await expect
    .poll(() =>
      sql(
        "SELECT count(*) FROM notifications WHERE user_id = :'p1' AND data LIKE '%chat.invitation_notification%'",
        invitee.id,
      )[0][0],
    )
    .toBe("1");
  // The notice's data.messageId is undefined in Ember (the payload says
  // message_id): the invitation names no message.
  const data = sql(
    "SELECT data FROM notifications WHERE user_id = :'p1' AND data LIKE '%chat.invitation_notification%' ORDER BY id DESC LIMIT 1",
    invitee.id,
  )[0][0];
  expect(JSON.parse(data).chat_message_id).toBeUndefined();
  // Three seconds later the notices go.
  await expect(notice).toHaveCount(0, { timeout: 6000 });

  // Another one, cleared with its button.
  await composer.fill(`@${invitee.name} ${label("mention")}`);
  await composer.press("Enter");
  await expect(notice).toHaveCount(1, { timeout: 20_000 });
  await notice.locator(".chat-notices__notice__clear").click();
  await expect(notice).toHaveCount(0);
});

test("the navbar's filter searches the channel and jumps through the results", async ({ page, context, session }) => {
  const word = label("filterword").replace(/\W/g, "");
  const user1 = await session("user1");
  const ids = [];
  for (const n of [1, 2, 3]) {
    const sent = await api(user1, "POST", `/chat/${GENERAL}.json`, {
      message: `${word} number ${n}`,
      staged_id: label("staged"),
    });
    ids.push(sent.message_id || sent.chat_message_id || sent.id);
  }
  const rows = sql("SELECT id FROM chat_messages WHERE message LIKE :'p1' ORDER BY id", `${word}%`).map((r) => r[0]);
  expect(rows.length).toBe(3);

  await login(context, "user1");
  await openChannel(page, `/chat/c/general/${GENERAL}`);
  const button = page.locator(".c-navbar__filter");
  await button.click();
  await expect(button).toHaveClass(/active/);
  const bar = page.locator(".chat-channel .chat-channel__filter-bar");
  const input = bar.locator(".filter-input");
  await expect(input).toBeFocused();
  await expect(input).toHaveAttribute("placeholder", "Search chat");

  // Latest first: the newest is the first result, highlighted in place.
  await input.pressSequentially(word, { delay: 5 });
  await expect(bar.locator(".chat-channel__filter-position")).toHaveText("1/3");
  await expect(message(page, rows[2])).toHaveClass(/-highlighted/);
  // Up is older, wrapping round; down is newer.
  await bar.locator(".chat-channel__prev-result").click();
  await expect(bar.locator(".chat-channel__filter-position-index")).toHaveText("2");
  await expect(message(page, rows[1])).toHaveClass(/-highlighted/);
  await bar.locator(".chat-channel__next-result").click();
  await bar.locator(".chat-channel__next-result").click();
  await expect(bar.locator(".chat-channel__filter-position-index")).toHaveText("3");
  await expect(message(page, rows[0])).toHaveClass(/-highlighted/);

  // Nothing found; a blank filter clears it.
  await input.fill(label("nothing").replace(/\W/g, ""));
  await expect(bar.locator(".alert.alert-info")).toHaveText("No results found");
  await expect(bar.locator(".chat-channel__filter-position")).toHaveCount(0);
  await input.fill("");
  await expect(bar.locator(".alert")).toHaveCount(0);

  // Esc closes it, as Done does.
  await input.press("Escape");
  await expect(bar).toHaveCount(0);
  await expect(button).not.toHaveClass(/active/);
  await expect(page.locator(".full-page-chat")).toBeVisible();
  await button.click();
  await bar.locator(".chat-channel__filter-done").click();
  await expect(bar).toHaveCount(0);
});

test("a date separator jumps to its day, loading the page around it when older ones are left", async ({
  page,
  context,
  session,
}) => {
  // 30 messages on each of two days and 10 yesterday, around noon UTC:
  // the page opens on the last read (the latest) with the 25 before it.
  const day = (n) => `date_trunc('day', now()) - (3 - (n - 1) / 30) * interval '1 day' + interval '12 hours' + ((n - 1) % 30) * interval '1 minute'`;
  const channel = await filledChannel(session, 70, day("n"));
  const [first] = channel.ids;
  await login(context, "user1");
  const requests = [];
  page.on("request", (r) => {
    if (r.url().includes(`/live/chat/${channel.id}/messages`)) {
      requests.push(new URL(r.url()).searchParams);
    }
  });
  await openChannel(page, channel.url);
  await expect(message(page, first + 69)).toBeAttached();
  await expect(message(page, first + 44)).toBeAttached();
  await page.waitForTimeout(700);
  await expect(message(page, first + 43)).toHaveCount(0);
  requests.length = 0;

  // Yesterday's first message is loaded, not the first of its page:
  // highlighted in place.
  const separator = (id) => page.locator(`.chat-message-separator-date[data-id="${id}"] .chat-message-separator__text`);
  await separator(first + 60).dispatchEvent("click");
  await expect(message(page, first + 60)).toHaveClass(/-highlighted/);
  expect(requests).toEqual([]);

  // The day before's first loaded is the page's first, older ones left:
  // the page around that day's start.
  await separator(first + 44).dispatchEvent("click");
  await expect(message(page, first + 30)).toBeAttached();
  await expect(message(page, first + 5)).toBeAttached();
  const asked = requests.find((q) => q.get("target_date"));
  expect(asked.get("direction")).toBe("future");
  expect(asked.get("target_date")).toMatch(/^\d{4}-\d\d-\d\dT00:00:00[+-]\d\d:\d\d$/);
  // Now that day's first isn't its page's: in place.
  requests.length = 0;
  await separator(first + 30).dispatchEvent("click");
  await expect(message(page, first + 30)).toHaveClass(/-highlighted/);
  expect(requests.filter((q) => q.get("target_date"))).toEqual([]);
});

test("a channel opened again goes back to where its scroll rested", async ({ page, context, session }) => {
  const channel = await filledChannel(session, 70);
  await login(context, "user1");
  await openChannel(page, channel.url);
  const pane = page.locator(".chat-channel");
  await expect(pane).not.toHaveClass(/chat-channel--saved-scroll-position/);

  await page.evaluate((s) => (document.querySelector(s).scrollTop = -500), SCROLLER);
  await expect(pane).toHaveClass(/chat-channel--saved-scroll-position/);
  // firstVisibleMessageId: the last message whose bottom shows.
  const saved = await page.evaluate((s) => {
    const scroller = document.querySelector(s);
    const view = scroller.getBoundingClientRect();
    const nodes = Array.from(scroller.querySelectorAll(".chat-messages-container > .chat-message-container[data-id]"));
    const found = nodes.reverse().find((n) => {
      const r = n.getBoundingClientRect();
      return r.bottom <= view.bottom + 1 && r.bottom >= view.top;
    });
    return Number(found.dataset.id);
  }, SCROLLER);
  expect(saved).toBeLessThan(channel.ids[channel.ids.length - 1]);

  // Away and back without a reload: the saved message highlighted.
  await page.evaluate(() => (window.__noReload = true));
  await swapTo(page, `/chat/c/general/${GENERAL}`);
  await expect(page.locator(`.chat-channel[data-id="${GENERAL}"]`)).toBeVisible();
  await swapTo(page, channel.url);
  await expect(message(page, saved)).toHaveClass(/-highlighted/);
  await expect(page.locator(".chat-channel")).toHaveClass(/chat-channel--saved-scroll-position/);
  expect(await page.evaluate(() => window.__noReload)).toBe(true);

  // Resting at the bottom forgets it.
  await expect(message(page, saved)).not.toHaveClass(/-highlighted/, { timeout: 4000 });
  await page.evaluate((s) => (document.querySelector(s).scrollTop = 0), SCROLLER);
  await expect(page.locator(".chat-channel")).not.toHaveClass(/chat-channel--saved-scroll-position/);
  await swapTo(page, `/chat/c/general/${GENERAL}`);
  await expect(page.locator(`.chat-channel[data-id="${GENERAL}"]`)).toBeVisible();
  await swapTo(page, channel.url);
  await expect(message(page, channel.ids[channel.ids.length - 1])).toBeVisible();
  await expect(page.locator(".chat-message-container.-highlighted")).toHaveCount(0);
});

test("messages get their spoilers, hashtag icons and code block buttons, loaded and live", async ({
  page,
  context,
  session,
}) => {
  const user0 = await session("user0");
  const marker = label("decorated");
  const spoiler = await api(user0, "POST", `/chat/${GENERAL}.json`, {
    message: `[spoiler]${marker} secret[/spoiler]`,
    staged_id: label("staged"),
  });
  void spoiler;
  await api(user0, "POST", `/chat/${GENERAL}.json`, { message: `${marker} in #general`, staged_id: label("staged") });
  await api(user0, "POST", `/chat/${GENERAL}.json`, {
    message: "```\n" + marker + " code\n```",
    staged_id: label("staged"),
  });

  await login(context, "user1");
  await openChannel(page, `/chat/c/general/${GENERAL}`);
  const byText = (text) => page.locator(".chat-message-container", { hasText: text });
  const hidden = byText(`${marker} secret`).locator(".chat-cooked .spoiled");
  await expect(hidden).toHaveAttribute("data-spoiler-state", "blurred");
  await hidden.click();
  await expect(hidden).toHaveAttribute("data-spoiler-state", "revealed");
  const hashtag = byText(`${marker} in`).locator(".chat-cooked a.hashtag-cooked");
  await expect(hashtag).toHaveAttribute("aria-label", /general/i);
  await expect(hashtag.locator(".hashtag-icon-placeholder")).toHaveCount(0);
  await expect(
    byText(`${marker} code`).locator(".chat-cooked pre.codeblock-buttons .codeblock-button-wrapper .copy-cmd"),
  ).toHaveCount(1);

  // One arriving over the bus.
  await api(user0, "POST", `/chat/${GENERAL}.json`, {
    message: `[spoiler]${marker} live[/spoiler]`,
    staged_id: label("staged"),
  });
  await expect(byText(`${marker} live`).locator(".chat-cooked .spoiled")).toHaveAttribute(
    "data-spoiler-state",
    "blurred",
    { timeout: 20_000 },
  );
});

test("the stop streaming button stops the message streaming", async ({ page, context, session }) => {
  const user1 = await session("user1");
  const user0 = await session("user0");
  const question = label("question");
  await api(user1, "POST", `/chat/${GENERAL}.json`, { message: question, staged_id: label("staged") });
  const questionId = sql("SELECT id FROM chat_messages WHERE message = :'p1'", question)[0][0];
  const answer = label("streamed answer");
  await api(user0, "POST", `/chat/${GENERAL}.json`, {
    message: answer,
    in_reply_to_id: questionId,
    staged_id: label("staged"),
  });
  sql("UPDATE chat_messages SET streaming = TRUE WHERE message = :'p1'", answer);

  // The member it answers may stop it.
  await login(context, "user1");
  await openChannel(page, `/chat/c/general/${GENERAL}`);
  const streamed = page.locator(".chat-message-container", { hasText: answer });
  await expect(streamed).toHaveClass(/-streaming/);
  await streamed.locator(".stop-streaming-btn").click();
  await expect.poll(() => sql("SELECT streaming FROM chat_messages WHERE message = :'p1'", answer)[0][0]).toBe("f");
  // The edit it publishes draws the message again.
  await expect(streamed.locator(".stop-streaming-btn")).toHaveCount(0, { timeout: 20_000 });
  await expect(streamed).not.toHaveClass(/-streaming/);
});

// Chat drafts are kept on the server: what a test leaves in user1's
// General composer comes back in the next test that opens it (and the
// typed message lands in front of it). Emptying the composer posts the
// empty draft, which deletes it.
async function dropDraft(composer) {
  await composer.fill("");
  await expect
    .poll(
      () =>
        sql(
          "SELECT count(*) FROM chat_drafts d JOIN users u ON u.id = d.user_id WHERE u.username = 'user1' AND d.chat_channel_id = :'p1'",
          GENERAL,
        ),
      { timeout: 10_000 },
    )
    .toEqual([["0"]]);
}

test("the composer's + menu inserts a local date", async ({ page, context }) => {
  await login(context, "user1");
  await openChannel(page, `/chat/c/general/${GENERAL}`);
  const composer = page.locator(".chat-composer__input");
  await composer.fill("meet at ");
  await page.locator(".chat-composer-dropdown__trigger-btn").click();
  const menu = page.locator(".chat-composer-dropdown__list");
  await expect(menu.locator(".chat-composer-dropdown__item.chat-upload-btn")).toBeVisible();
  // The emoji button is the mobile view's; GIFs aren't ported.
  await expect(menu.locator(".chat-composer-dropdown__item.emoji")).toHaveCount(0);
  await expect(menu.locator(".chat-composer-dropdown__item.gifs")).toHaveCount(0);
  await menu.locator(".chat-composer-dropdown__action-btn.local-dates").click();
  const modal = page.locator(".discourse-local-dates-create-modal");
  await expect(modal).toBeVisible();
  await modal.locator(".btn-primary").click();
  await expect(modal).toHaveCount(0);
  await expect(composer).toHaveValue(/^meet at \[date=\d{4}-\d\d-\d\d/);
  await expect(composer).toBeFocused();
  await dropDraft(composer);
});

test.describe("in the mobile view", () => {
  test.use({ viewport: { width: 400, height: 760 } });

  test("the + menu's emoji goes in the composer", async ({ page, context }) => {
    await login(context, "user1");
    await openChannel(page, `/chat/c/general/${GENERAL}`);
    await expect(page.locator("html")).toHaveClass(/mobile-view/);
    const composer = page.locator(".chat-composer__input");
    await composer.fill("well ");
    await page.locator(".chat-composer-dropdown__trigger-btn").click();
    await page.locator(".chat-composer-dropdown__action-btn.emoji").click();
    const picker = page.locator(".emoji-picker");
    await expect(picker).toBeVisible();
    await picker.locator("input.filter-input").first().fill("tada");
    await picker.locator("img.emoji[data-emoji='tada']:visible").first().click();
    await expect(composer).toHaveValue("well :tada:");
    await dropDraft(composer);
  });

  test("the arrow counts the new messages", async ({ page, context, session }) => {
    const channel = await filledChannel(session, 40);
    await login(context, "user1");
    await openChannel(page, channel.url);
    const arrow = page.locator(".chat-scroll-to-bottom__button");
    const count = arrow.locator(".chat-scroll-to-bottom__new-messages");
    await page.evaluate((s) => (document.querySelector(s).scrollTop = -600), SCROLLER);
    await expect(arrow).toHaveClass(/visible/);
    await expect(count).toHaveCount(0);

    const user0 = await session("user0");
    await api(user0, "POST", `/chat/${channel.id}.json`, { message: label("new one"), staged_id: label("staged") });
    await expect(count).toHaveText("1 new message", { timeout: 20_000 });
    await api(user0, "POST", `/chat/${channel.id}.json`, { message: label("new two"), staged_id: label("staged") });
    await expect(count).toHaveText("2 new messages", { timeout: 20_000 });

    // Read at the bottom: none left.
    await arrow.click();
    await expect(arrow).not.toHaveClass(/visible/);
    await expect(count).toHaveCount(0);
    await expect
      .poll(() =>
        page.evaluate(() => document.querySelector("template.chat-channel-extras").dataset.unreadCount),
      )
      .toBe("0");
  });

  test("a message still loading when the arrow is clicked follows to the bottom", async ({ page, context, session }) => {
    const channel = await filledChannel(session, 40);
    await login(context, "user1");
    await openChannel(page, channel.url);
    const arrow = page.locator(".chat-scroll-to-bottom__button");
    await page.evaluate((s) => (document.querySelector(s).scrollTop = -600), SCROLLER);
    await expect(arrow).toHaveClass(/visible/);

    // The new message's html held until the arrow has been clicked.
    let release;
    const held = new Promise((resolve) => (release = resolve));
    let fetching;
    const fetched = new Promise((resolve) => (fetching = resolve));
    await page.route(new RegExp(`/live/chat/${channel.id}/\\d+`), async (route) => {
      fetching();
      await held;
      await route.continue();
    });
    const user0 = await session("user0");
    const raw = label("late one");
    await api(user0, "POST", `/chat/${channel.id}.json`, { message: raw, staged_id: label("staged") });
    await fetched;
    await arrow.click();
    release();

    await expect(page.locator(".chat-message-container", { hasText: raw })).toBeVisible();
    await expect(arrow).not.toHaveClass(/visible/);
  });

  test("a message drawn live while the newer page loads is drawn once", async ({ page, context, session }) => {
    const channel = await filledChannel(session, 40);
    await login(context, "user1");
    await openChannel(page, channel.url);
    const arrow = page.locator(".chat-scroll-to-bottom__button");
    await page.evaluate((s) => (document.querySelector(s).scrollTop = -600), SCROLLER);
    await expect(arrow).toHaveClass(/visible/);
    // Scrolling up asks for the older page; one load at a time, so the
    // newer page below waits for it to land (the scroll settles in 250ms).
    await page.waitForTimeout(400);
    await page.waitForFunction(() => !document.querySelector(".chat-channel[data-id]").classList.contains("loading"));

    // The page after the last message, asked for at the bottom while
    // whether newer ones load is unknown (as after an older page), held
    // until the new message is drawn live.
    await page.evaluate(() => delete document.querySelector(".chat-channel[data-id]").dataset.canLoadMoreFuture);
    let release;
    const held = new Promise((resolve) => (release = resolve));
    let fetching;
    const fetched = new Promise((resolve) => (fetching = resolve));
    await page.route(new RegExp(`/live/chat/${channel.id}/messages\\?.*direction=future`), async (route) => {
      fetching();
      await held;
      await route.continue();
    });
    await arrow.click();
    await fetched;
    const user0 = await session("user0");
    const raw = label("raced one");
    await api(user0, "POST", `/chat/${channel.id}.json`, { message: raw, staged_id: label("staged") });
    const message = page.locator(".chat-message-container", { hasText: raw });
    await expect(message).toHaveCount(1);
    const loaded = page.waitForResponse(/\/messages\?.*direction=future/);
    release();
    await loaded;

    // Counted as the page lands: a copy removed later still showed twice.
    await page.waitForFunction(() => !document.querySelector(".chat-channel[data-id]").classList.contains("loading"));
    expect(await message.count()).toBe(1);
  });
});

test("a hidden tab neither follows nor reads new messages", async ({ page, context, session }) => {
  const channel = await filledChannel(session, 40);
  await login(context, "user1");
  const reads = [];
  page.on("request", (r) => {
    if (r.method() === "PUT" && r.url().includes(`/chat/api/channels/${channel.id}/read`)) {
      reads.push(Number(new URL(r.url()).searchParams.get("message_id")));
    }
  });
  await openChannel(page, channel.url);
  const last = channel.ids[channel.ids.length - 1];
  await expect(message(page, last)).toBeVisible();

  // The tab hidden (page visibility emulated, as the browser reports it).
  const setVisibility = (state) =>
    page.evaluate((s) => {
      Object.defineProperty(document, "visibilityState", { configurable: true, get: () => s });
      Object.defineProperty(document, "hidden", { configurable: true, get: () => s === "hidden" });
      document.dispatchEvent(new Event("visibilitychange"));
    }, state);
  await setVisibility("hidden");
  const text = label("while hidden");
  await api(await session("user0"), "POST", `/chat/${channel.id}.json`, { message: text, staged_id: label("staged") });
  const arrived = page.locator(".chat-message-container", { hasText: text });
  await expect(arrived).toBeAttached({ timeout: 20_000 });
  const id = Number(await arrived.getAttribute("data-id"));
  // It kept its place: the new message below, pending.
  await expect.poll(() => page.evaluate((s) => document.querySelector(s).scrollTop, SCROLLER)).toBeLessThan(-5);
  await expect(page.locator(".chat-scroll-to-bottom__button")).toHaveClass(/visible/);
  await page.waitForTimeout(1500);
  expect(reads).not.toContain(id);

  // Back: what shows is read, the new message only once scrolled to.
  await setVisibility("visible");
  await page.waitForTimeout(1500);
  expect(reads).not.toContain(id);
  await page.locator(".chat-scroll-to-bottom__button").click();
  await expect.poll(() => reads).toContain(id);
});
