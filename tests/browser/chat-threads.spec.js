// Chat threads on a running server (160a): the thread pane's composer
// (chat.js's ChatComposerThread) sending, editing, its emoji picker,
// autocomplete and replying indicator, uploads; the thread's messages'
// actions (react, edit, the secondary menu, no reply); its paging and
// jumps; the settings modal from the title; the side panel's resizer; the
// back link after the channel (ChatHistory); the threads list's load
// more and live updates. Each test makes its own threaded channel, so
// the other specs' General is left alone.
const { test, expect, login, api, sql, label } = require("./helpers");
const fs = require("fs");
const path = require("path");

// A threaded category channel (Site Feedback) the given members follow.
async function threadedChannel(session, members) {
  const admin = await session("admin");
  const name = label("threads").replace(/\s+/g, "-").slice(0, 40);
  const created = await api(admin, "POST", "/chat/api/channels.json", {
    "channel[chatable_id]": "2",
    "channel[name]": name,
    "channel[threading_enabled]": "true",
  });
  const id = created.channel.id;
  sql("UPDATE chat_channels SET threading_enabled = true WHERE id = :'p1'", id);
  const contexts = { admin };
  for (const username of members) {
    const context = username === "admin" ? admin : await session(username);
    await api(context, "POST", `/chat/api/channels/${id}/memberships/me.json`);
    contexts[username] = context;
  }
  return { id, slug: created.channel.slug, contexts };
}

async function send(context, channelId, params) {
  return api(context, "POST", `/chat/${channelId}.json`, { staged_id: label("staged"), ...params });
}

// A thread: `author`'s message and a reply to it by `replier`.
async function makeThread(channel, author, replier, text) {
  await send(channel.contexts[author], channel.id, { message: text });
  const [[messageId]] = sql(
    "SELECT id FROM chat_messages WHERE chat_channel_id = :'p1' AND message = :'p2'",
    channel.id,
    text,
  );
  await send(channel.contexts[replier], channel.id, { message: label("reply to"), in_reply_to_id: messageId });
  const [[threadId]] = sql("SELECT thread_id FROM chat_messages WHERE id = :'p1'", messageId);
  // The reply count Jobs::Chat::UpdateThreadReplyCount keeps, which the
  // lists filter on, without waiting for the job.
  sql("UPDATE chat_threads SET replies_count = GREATEST(replies_count, 1) WHERE id = :'p1'", threadId);
  return { messageId, threadId };
}

function threadUrl(channel, threadId) {
  return `/chat/c/${channel.slug}/${channel.id}/t/${threadId}`;
}

test("a reply sent from the thread's composer, live in another member's thread", async ({ page, context, session }) => {
  const channel = await threadedChannel(session, ["user0", "user1"]);
  const { threadId } = await makeThread(channel, "user0", "user1", label("thread start"));
  await login(context, "user1");
  const other = await channel.contexts.user0.newPage();
  await other.goto(threadUrl(channel, threadId));
  await expect(other.locator(".chat-thread #thread-composer")).toBeVisible();

  await page.goto(threadUrl(channel, threadId));
  const composer = page.locator(".chat-thread #thread-composer");
  await expect(composer).toBeFocused();
  // ChatReplyingIndicator in the other member's thread while typing.
  await composer.pressSequentially("typing ", { delay: 5 });
  await expect(other.locator(".chat-thread .chat-replying-indicator")).toContainText("user1", { timeout: 20_000 });
  const message = label("in the thread");
  await composer.fill(message);
  await composer.press("Enter");
  await expect(page.locator(".chat-thread .chat-message-container", { hasText: message })).toBeVisible();
  await expect(composer).toHaveValue("");
  await expect
    .poll(() => sql("SELECT thread_id FROM chat_messages WHERE message = :'p1'", message))
    .toEqual([[String(threadId)]]);
  // Not in the channel's pane (where the thread indicator's excerpt has it).
  await expect(page.locator(".chat-channel .chat-cooked", { hasText: message })).toHaveCount(0);
  await expect(page.locator(".chat-channel .chat-message-thread-indicator__last-reply-excerpt")).toContainText(message);
  await expect(other.locator(".chat-thread .chat-message-container", { hasText: message })).toBeVisible({
    timeout: 20_000,
  });
});

test("a thread message's actions: react, edit from the menu, no reply", async ({ page, context, session }) => {
  const channel = await threadedChannel(session, ["user0", "user1"]);
  const { threadId } = await makeThread(channel, "user0", "user1", label("actions thread"));
  const own = label("my thread message");
  const [[threadIdSent]] = [[threadId]];
  await send(channel.contexts.user1, channel.id, { message: own, thread_id: String(threadIdSent) });
  await login(context, "user1");
  await page.goto(threadUrl(channel, threadId));

  const message = page.locator(".chat-thread .chat-message-container", { hasText: own });
  await message.hover();
  const actions = page.locator(".chat-thread .chat-message-actions");
  await expect(actions).toBeVisible();
  // canReply is off in a thread.
  await expect(actions.locator(".reply-btn")).toHaveCount(0);

  await actions.locator(".react-btn").click();
  const picker = page.locator(".emoji-picker");
  await expect(picker).toBeVisible();
  await picker.locator("input.filter-input, input[type=search]").first().fill("tada");
  await picker.locator("img.emoji[data-emoji='tada']:visible").first().click();
  await expect(message.locator(".chat-message-reaction")).toBeVisible({ timeout: 20_000 });
  await expect
    .poll(() =>
      sql(
        "SELECT r.emoji FROM chat_message_reactions r JOIN chat_messages m ON m.id = r.chat_message_id WHERE m.message = :'p1'",
        own,
      ),
    )
    .toEqual([["tada"]]);

  // Edit from the secondary menu: the thread's composer takes it.
  await message.hover();
  await page.locator(".chat-thread .chat-message-actions .more-buttons summary").click();
  await page.locator(".chat-thread .more-actions-chat .select-kit-row[data-value='edit']").click();
  const composer = page.locator(".chat-thread #thread-composer");
  await expect(composer).toHaveValue(own);
  await expect(page.locator(".chat-thread .chat-composer-message-details[data-action='edit']")).toBeVisible();
  await expect(page.locator("#channel-composer")).toHaveValue("");
  const edited = label("edited in the thread");
  await composer.fill(edited);
  await composer.press("Enter");
  await expect(page.locator(".chat-thread .chat-message-container", { hasText: edited })).toBeVisible({
    timeout: 20_000,
  });
  await expect.poll(() => sql("SELECT count(*) FROM chat_messages WHERE message = :'p1'", edited)).toEqual([["1"]]);

  // Up in the empty thread composer edits the member's last message of
  // the thread; Escape drops the edit, the pane stays.
  await composer.press("ArrowUp");
  await expect(composer).toHaveValue(edited);
  await composer.press("Escape");
  await expect(composer).toHaveValue("");
  await expect(page.locator(".chat-thread")).toBeVisible();

  // Select from the menu: the thread's own selection, in place of its
  // composer, without moving (no enableMove).
  const target = page.locator(".chat-thread .chat-message-container", { hasText: edited });
  await target.hover();
  await page.locator(".chat-thread .chat-message-actions .more-buttons summary").click();
  await page.locator(".chat-thread .more-actions-chat .select-kit-row[data-value='select']").click();
  const bar = page.locator(".chat-thread .chat-selection-management");
  await expect(bar).toBeVisible();
  await expect(bar.locator("#chat-move-to-channel-btn")).toHaveCount(0);
  await expect(page.locator(".chat-channel .chat-selection-management")).toHaveCount(0);
  await expect(page.locator(".chat-channel .chat-message-selector")).toHaveCount(0);
  await expect(target).toHaveClass(/-selected/);
  await bar.locator("#chat-cancel-selection-btn").click();
  await expect(bar).toHaveCount(0);
  await expect(composer).toBeVisible();

  // Escape with nothing to drop closes the pane.
  await composer.press("Escape");
  await expect(page).toHaveURL(new RegExp(`/chat/c/${channel.slug}/${channel.id}$`));
});

test("the thread composer's emoji picker, autocomplete and uploads", async ({ page, context, session }) => {
  const channel = await threadedChannel(session, ["user0", "user1"]);
  const { threadId } = await makeThread(channel, "user0", "user1", label("composer thread"));
  await login(context, "user1");
  await page.goto(threadUrl(channel, threadId));
  const composer = page.locator(".chat-thread #thread-composer");

  // The emoji button's pick goes into the thread's composer.
  await page.locator(".chat-thread .chat-composer-button.--emoji").click();
  const picker = page.locator(".emoji-picker");
  await expect(picker).toBeVisible();
  await picker.locator("input.filter-input, input[type=search]").first().fill("tada");
  await picker.locator("img.emoji[data-emoji='tada']:visible").first().click();
  await expect(composer).toHaveValue(":tada:");
  await expect(page.locator("#channel-composer")).toHaveValue("");

  // The emoji autocomplete on the thread's composer.
  await composer.fill("");
  await composer.pressSequentially("hi :smil", { delay: 20 });
  await expect(page.locator(".autocomplete")).toBeVisible();
  await composer.press("Escape");

  // An upload into the thread's composer, sent with the reply.
  await composer.fill("");
  const file = path.join(test.info().outputDir, "thread-upload.png");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(
    file,
    Buffer.from(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
      "base64",
    ),
  );
  await page.locator("#thread-file-uploader").setInputFiles(file);
  const upload = page.locator(".chat-thread .chat-composer-upload[data-id]");
  await expect(upload).toBeVisible({ timeout: 20_000 });
  await expect(page.locator(".chat-channel .chat-composer-upload")).toHaveCount(0);
  const message = label("with a file");
  await composer.fill(message);
  await composer.press("Enter");
  await expect
    .poll(() =>
      sql(
        "SELECT m.thread_id, count(r.id) FROM chat_messages m LEFT JOIN upload_references r ON r.target_id = m.id AND r.target_type = 'ChatMessage' WHERE m.message = :'p1' GROUP BY m.thread_id",
        message,
      ),
    )
    .toEqual([[String(threadId), "1"]]);
  await expect(page.locator(".chat-thread .chat-composer-upload")).toHaveCount(0);
});

test("the thread pane pages, jumps to a message, and the settings modal sets the title", async ({ page, context, session }) => {
  const channel = await threadedChannel(session, ["user0", "user1"]);
  const { threadId } = await makeThread(channel, "user0", "user1", label("long thread"));
  const marker = label("page");
  // Seventy more replies, a page and a half past the first.
  sql(
    `INSERT INTO chat_messages (chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id, thread_id)
     SELECT :'p1', u.id, now() + n * interval '1 second', now(), :'p2' || ' ' || n, '<p>' || :'p2' || ' ' || n || '</p>', 1, u.id, :'p3'
     FROM generate_series(1, 70) AS n, users u WHERE u.username = 'user1'`,
    channel.id,
    marker,
    threadId,
  );
  sql(
    "UPDATE chat_threads SET last_message_id = (SELECT max(id) FROM chat_messages WHERE thread_id = :'p1'), replies_count = 71 WHERE id = :'p1'",
    threadId,
  );
  await login(context, "user0");
  await page.goto(threadUrl(channel, threadId));
  const pane = page.locator(".chat-thread");
  await expect(pane.locator(".chat-message-container", { hasText: `${marker} 10` }).first()).toBeVisible();
  await expect(pane.locator(".chat-message-container", { hasText: `${marker} 70` })).toHaveCount(0);

  // The arrow loads around the latest message.
  await pane.locator(".chat-messages-scroller").evaluate((s) => (s.scrollTop = -400));
  await pane.locator(".chat-scroll-to-bottom__button").click();
  await expect(pane.locator(".chat-message-container", { hasText: `${marker} 70` })).toBeVisible();
  // Scrolling up loads the older page.
  for (let i = 0; i < 20; i++) {
    await pane.locator(".chat-messages-scroller").evaluate((s) => (s.scrollTop = -s.scrollHeight));
    if ((await pane.locator(".chat-message-container", { hasText: `${marker} 5` }).count()) > 0) {
      break;
    }
    await page.waitForTimeout(300);
  }
  await expect(pane.locator(".chat-message-container", { hasText: new RegExp(`${marker} 5$`) })).toHaveCount(1);

  // A link to a message of the thread jumps in place, highlighted.
  const [[target]] = sql("SELECT id FROM chat_messages WHERE message = :'p1'", `${marker} 30`);
  await page.evaluate(
    ([href]) => {
      const a = document.createElement("a");
      a.href = href;
      a.id = "jump-link";
      a.textContent = "jump";
      document.querySelector(".chat-thread__heading, .chat-thread").prepend(a);
    },
    [`${threadUrl(channel, threadId)}/${target}`],
  );
  await page.locator("#jump-link").click();
  await expect(pane.locator(`.chat-message-container[data-id='${target}']`)).toHaveClass(/-highlighted/);
  await expect(page).toHaveURL(new RegExp(`/t/${threadId}$`));

  // The original message's author sets the title from the header.
  const title = label("Our plans");
  await page.locator(".c-routes.--channel-thread .c-navbar__title.clickable").click();
  const modal = page.locator(".chat-modal-thread-settings");
  await expect(modal).toBeVisible();
  await modal.locator(".chat-modal-thread-settings__title-input").fill(title);
  await expect(modal.locator(".thread-title-length span")).toHaveText(String(title.length));
  await modal.locator(".d-modal__footer .btn-primary").click();
  await expect(modal).toHaveCount(0);
  await expect(page.locator(".c-routes.--channel-thread .c-navbar__title-text")).toHaveText(title);
  await expect(page.locator(".chat-thread__heading-title")).toHaveText(title);
  await expect.poll(() => sql("SELECT title FROM chat_threads WHERE id = :'p1'", threadId)).toEqual([[title]]);
});

test("the title prompt on a phone opens the settings", async ({ page, context, session }) => {
  const channel = await threadedChannel(session, ["user0", "user1"]);
  const { threadId } = await makeThread(channel, "user0", "user1", label("prompted thread"));
  for (let i = 0; i < 4; i++) {
    await send(channel.contexts.user1, channel.id, { message: label("more"), thread_id: String(threadId) });
  }
  await login(context, "user0");
  // A phone's viewport, which page.js keeps the mobile_view cookie with.
  await page.setViewportSize({ width: 400, height: 800 });
  const base = new URL(test.info().project.use.baseURL);
  await context.addCookies([{ name: "mobile_view", value: "1", domain: base.hostname, path: "/" }]);
  await page.goto(threadUrl(channel, threadId));
  const toast = page.locator(".fk-d-toast.thread-toast");
  await expect(toast).toBeVisible();
  await expect
    .poll(() =>
      sql(
        "SELECT m.thread_title_prompt_seen FROM user_chat_thread_memberships m JOIN users u ON u.id = m.user_id WHERE u.username = 'user0' AND m.thread_id = :'p1'",
        threadId,
      ),
    )
    .toEqual([["t"]]);
  await toast.locator(".toast-action").click();
  await expect(toast).toHaveCount(0);
  await expect(page.locator(".chat-modal-thread-settings")).toBeVisible();
  // Seen: the next visit has no prompt.
  await page.reload();
  await expect(page.locator(".chat-thread")).toBeVisible();
  await page.waitForTimeout(300);
  await expect(page.locator(".fk-d-toast.thread-toast")).toHaveCount(0);
});

test("the side panel's resizer, the back link after the channel, the threads list", async ({ page, context, session }) => {
  const channel = await threadedChannel(session, ["user0", "user1"]);
  const first = await makeThread(channel, "user0", "user1", label("first thread"));
  // Eleven more threads: the list's second page.
  for (let i = 0; i < 11; i++) {
    await makeThread(channel, "user0", "user1", label(`more thread ${i}`));
  }
  await login(context, "user1");

  // From the channel, a thread's indicator; the back link goes to the
  // threads list (the channel was the previous route, on desktop).
  await page.goto(`/chat/c/${channel.slug}/${channel.id}`);
  await page.locator(`.chat-message-container[data-id='${first.messageId}'] .chat-message-thread-indicator`).click();
  await expect(page).toHaveURL(new RegExp(`/t/${first.threadId}$`));
  const back = page.locator(".c-routes.--channel-thread .c-navbar__back-button");
  await expect(back).toHaveAttribute("href", new RegExp(`/${channel.id}/t$`));
  await expect(back).toHaveAttribute("title", "Return to threads list");

  // The resizer: wider by the drag, kept for the next page.
  const panel = page.locator(".chat-side-panel");
  await expect(panel).toHaveAttribute("style", /width: 250px/);
  const box = await page.locator(".chat-side-panel-resizer").boundingBox();
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x - 50, y, { steps: 5 });
  await page.mouse.move(x - 100, y, { steps: 5 });
  await page.mouse.up();
  await expect(panel).toHaveAttribute("style", /width: 350px/);
  expect(await page.evaluate(() => localStorage.getItem("discourse_chat_side_panel_size_width"))).toBe("350");

  // The threads list: ten, then the rest at its end.
  await back.click();
  await expect(page).toHaveURL(new RegExp(`/${channel.id}/t$`));
  await expect(page.locator(".chat-side-panel")).toHaveAttribute("style", /width: 350px/);
  const items = page.locator(".chat-thread-list-item");
  await page.locator(".chat-thread-list__load-more").scrollIntoViewIfNeeded();
  await expect(items).toHaveCount(12);

  // Live: the first thread's original message deleted elsewhere leaves
  // the list.
  await expect(page.locator(`.chat-thread-list-item[data-thread-id='${first.threadId}']`)).toHaveCount(1);
  await api(channel.contexts.user0, "DELETE", `/chat/api/channels/${channel.id}/messages/${first.messageId}.json`);
  await expect(page.locator(`.chat-thread-list-item[data-thread-id='${first.threadId}']`)).toHaveCount(0, {
    timeout: 20_000,
  });
  await expect(items).toHaveCount(11);
});
