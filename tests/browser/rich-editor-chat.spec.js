// chat's rich editor extension (ROADMAP 196r6): a [chat] transcript as
// the chat node over chat-transcript.js' tokens, with the reference's
// tests: the plugin's rich-editor-extension-test.gjs run as written in the
// page (rich-editor-helper.js qunitTest) and
// rich_editor_extension_transcript_spec.rb's pasted transcripts in the
// composer, the markdown from the port's Chat::TranscriptService (POST
// /chat/:id/quote).
const { test, expect, login, api, sql, label, member } = require("./helpers");
const { qunitTest } = require("./rich-editor-helper");

const opts = { plugins: ["chat"], render: { chat_transcript: true } };
const qtest = (name, body) => qunitTest(test, name, body, opts);

test.describe("Integration | Component | prosemirror-editor - chat transcript extension", () => {
  qtest("single message from single user transcript", async function (assert, { helpers }) {
    const { setupRichEditor } = helpers;
    const singleMessageSingleUserMarkdown = `[chat quote="hunter;29856;2025-03-20T07:13:04Z" channel="design gems :tada:" channelId=95]
haha **ok** _cool_
[/chat]
`;
    const [{ value }] = await setupRichEditor(assert, singleMessageSingleUserMarkdown);
    const rootElement = document.querySelector(".ProseMirror .chat-transcript");
    assert.dom(".chat-transcript-messages", rootElement).hasHtml("<p>haha <strong>ok</strong> <em>cool</em></p>");
    assert.dom(".chat-transcript-user .chat-transcript-username", rootElement).hasText("hunter");
    assert.dom(".chat-transcript-user .chat-transcript-datetime", rootElement).exists();
    assert
      .dom(".chat-transcript-group__channel", rootElement)
      .hasText("design gems")
      .hasAttribute("href", "/chat/c/-/95");
    assert.dom(".chat-transcript-group__channel img[title='tada']").exists();

    assert.strictEqual(value, singleMessageSingleUserMarkdown);
  });

  qtest("escapes HTML in transcript usernames", async function (assert, { helpers }) {
    const { setupRichEditor } = helpers;
    const transcriptWithHtmlUsername = `[chat quote="<b>marker</b>;29856;2025-03-20T07:13:04Z"]
haha
[/chat]
`;
    await setupRichEditor(assert, transcriptWithHtmlUsername);

    const rootElement = document.querySelector(".ProseMirror .chat-transcript");
    assert
      .dom(".chat-transcript-user .chat-transcript-username", rootElement)
      .hasText("<b>marker</b>", "renders transcript username markup as text");
    assert
      .dom(".chat-transcript-user .chat-transcript-username b", rootElement)
      .doesNotExist("does not render markup from transcript usernames");
  });

  qtest("multiple messages from multiple different users", async function (assert, { helpers }) {
    const { setupRichEditor } = helpers;
    const multiMessagesMultiUserMarkdown = `[chat quote="martin;29853;2025-03-20T07:12:55Z" channel="design gems :tada:" channelId=95 multiQuote=true chained=true]
test
[/chat]
[chat quote="hunter;29856;2025-03-20T07:13:04Z" chained=true]
haha **ok** _cool_
[/chat]
`;
    const [{ value }] = await setupRichEditor(assert, multiMessagesMultiUserMarkdown);

    assert.dom(".chat-transcript").exists({ count: 2 });
    assert.dom(".chat-transcript-messages").exists({ count: 2 });
    assert.dom(".chat-transcript-user").exists({ count: 2 });
    assert.dom(".chat-transcript:nth-of-type(1)").hasClass("chat-transcript-chained");
    assert.dom(".chat-transcript-group__header").exists({ count: 1 });
    assert.dom(".chat-transcript:nth-of-type(1) .chat-transcript-group__channel").hasText("design gems");
    assert.dom(".chat-transcript-group__channel img[title='tada']").exists();

    let rootElement = document.querySelector(".ProseMirror .chat-transcript:nth-of-type(1)");
    assert.dom(".chat-transcript-messages", rootElement).hasHtml("<p>test</p>");
    assert.dom(".chat-transcript-user .chat-transcript-username", rootElement).hasText("martin");

    rootElement = document.querySelector(".ProseMirror .chat-transcript:nth-of-type(2)");
    assert.dom(".chat-transcript-messages", rootElement).hasHtml("<p>haha <strong>ok</strong> <em>cool</em></p>");
    assert.dom(".chat-transcript-user .chat-transcript-username", rootElement).hasText("hunter");

    assert.strictEqual(value, multiMessagesMultiUserMarkdown);
  });

  qtest("transcript named by an unlinked source", async function (assert, { helpers }) {
    const { setupRichEditor } = helpers;
    const sourceMarkdown = `[chat quote="martin;1;2025-03-20T07:12:55Z" source="Weekly sync" sourceType=meeting chained=true]
test
[/chat]
[chat quote="hunter;2;2025-03-20T07:13:04Z" chained=true]
haha
[/chat]
`;
    const [{ value }] = await setupRichEditor(assert, sourceMarkdown);

    assert.dom(".chat-transcript-group__header").exists({ count: 1 });
    assert.dom("span.chat-transcript-group__channel").hasText("Weekly sync");
    assert.dom(".chat-transcript-group__label").hasText("Transcript");

    assert.strictEqual(value, sourceMarkdown);
  });

  qtest("messages in a thread", async function (assert, { helpers }) {
    const { setupRichEditor } = helpers;
    const threadMessagesMarkdown = `[chat quote="martin;29854;2025-03-20T07:12:57Z" channel="design gems :tada:" channelId=95 multiQuote=true threadId=124 threadTitle="Some cool thread title"]
thread op message

[chat quote="martin;29857;2025-03-24T07:08:01Z"]
thread other message
[/chat]

[/chat]
`;
    const [{ value }] = await setupRichEditor(assert, threadMessagesMarkdown);
    assert.dom(".chat-transcript-group__channel").hasText("design gems");
    assert.dom(".chat-transcript-group__channel img[title='tada']").exists();
    assert.dom(".chat-transcript details").doesNotExist("the thread is shown expanded");

    let rootElement = document.querySelector(".ProseMirror .chat-transcript > .chat-transcript-thread");
    assert.dom(".chat-transcript-thread-header svg.d-icon-discourse-threads", rootElement).exists();
    assert
      .dom(".chat-transcript-thread-header .chat-transcript-thread-header__title", rootElement)
      .hasText("Some cool thread title");

    assert
      .dom(".chat-transcript-messages", rootElement)
      .hasHtml("<p>thread op message</p>", "the thread op message is inside the thread element");
    assert.dom(".chat-transcript-user .chat-transcript-username", rootElement).hasText("martin");

    rootElement = document.querySelector(".ProseMirror .chat-transcript > .chat-transcript");
    assert
      .dom(".chat-transcript-messages", rootElement)
      .hasHtml("<p>thread other message</p>", "the other thread messages are nested in the thread's transcript");
    assert.dom(".chat-transcript-user .chat-transcript-username", rootElement).hasText("martin");

    assert.strictEqual(value, threadMessagesMarkdown);
  });
});

// rich_editor_extension_transcript_spec.rb: a transcript the port's
// TranscriptService writes, pasted into a rich composer.
test.describe("chat transcripts in rich editor", () => {
  const rich = (page) => page.locator("#reply-control .ProseMirror.d-editor-input");

  // A channel of its own in General's category, its messages posted by
  // the admin. Returns { id, name, messageIds }.
  async function channelWithMessages(context, name, messages) {
    const created = await api(context, "POST", "/chat/api/channels", {
      "channel[name]": name,
      "channel[chatable_id]": "4",
    });
    const channel = created.channel;
    const messageIds = [];
    for (let i = 0; i < messages.length; i++) {
      const sent = await api(context, "POST", `/chat/${channel.id}.json`, {
        message: messages[i],
        staged_id: `s${i}`,
      });
      messageIds.push(sent.message_id);
    }
    return { id: channel.id, name: channel.title || name, messageIds };
  }

  // Chat::TranscriptService through POST /chat/:id/quote (message_ids[]).
  async function transcript(context, channel, ids) {
    const csrf = await context.request.get("/session/csrf.json", {
      headers: { "X-Requested-With": "XMLHttpRequest" },
    });
    const res = await context.request.post(`/chat/${channel.id}/quote.json`, {
      headers: {
        "X-CSRF-Token": (await csrf.json()).csrf,
        "X-Requested-With": "XMLHttpRequest",
        Accept: "application/json",
        "Content-Type": "application/x-www-form-urlencoded",
      },
      data: ids.map((id) => `message_ids%5B%5D=${id}`).join("&"),
    });
    expect(res.ok(), await res.text()).toBeTruthy();
    return (await res.json()).markdown;
  }

  async function openRichComposer(page, context) {
    const writer = member();
    sql("UPDATE user_options SET composition_mode = 1 WHERE user_id = :'p1'", writer.id);
    await login(context, writer.name);
    await page.goto("/new-topic");
    await expect(page.locator("#reply-control.open")).toBeVisible();
    await expect(rich(page)).toBeVisible();
    await rich(page).click();
  }

  // cdp.copy_paste: the markdown as the clipboard's text, pasted.
  async function paste(page, markdown) {
    await rich(page).evaluate((el, text) => {
      const clipboardData = new DataTransfer();
      clipboardData.setData("text/plain", text);
      el.dispatchEvent(new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData }));
    }, markdown);
  }

  test("works for single messages", async ({ page, context, session }) => {
    const admin = await session("admin");
    const message = label("A transcript message");
    const channel = await channelWithMessages(admin, label("Transcript channel").replace(/[^a-z0-9 ]/gi, ""), [message]);
    const markdown = await transcript(admin, channel, channel.messageIds);
    await openRichComposer(page, context);
    await paste(page, markdown);

    await expect(rich(page).locator(".chat-transcript").filter({ hasText: channel.name })).toHaveCount(1);
    await expect(rich(page).locator(".chat-transcript .chat-transcript-user .chat-transcript-username")).toHaveText(
      "admin",
    );
    await expect(rich(page).locator(".chat-transcript .chat-transcript-messages")).toContainText(message);
  });

  test("works for channel name with quotation marks in its name", async ({ page, context, session }) => {
    const admin = await session("admin");
    const message = label("A quoted channel's message");
    const name = `Channel with a "special" name ${label("q").replace(/[^a-z0-9]/gi, "")}`;
    const channel = await channelWithMessages(admin, name, [message]);
    const markdown = await transcript(admin, channel, channel.messageIds);
    await openRichComposer(page, context);
    await paste(page, markdown);

    await expect(rich(page).locator(".chat-transcript").filter({ hasText: channel.name })).toHaveCount(1);
    await expect(rich(page).locator(".chat-transcript .chat-transcript-user .chat-transcript-username")).toHaveText(
      "admin",
    );
    await expect(rich(page).locator(".chat-transcript .chat-transcript-messages")).toContainText(message);
  });

  test("works for multiple messages", async ({ page, context, session }) => {
    const admin = await session("admin");
    const user1 = await session("user1");
    const first = label("The first message");
    const channel = await channelWithMessages(admin, label("Transcript channel").replace(/[^a-z0-9 ]/gi, ""), [first]);
    // message_2 by another member, as the spec's Fabricate(:chat_message)
    sql(
      "INSERT INTO user_chat_channel_memberships (user_id, chat_channel_id, following, notification_level, created_at, updated_at) \
       SELECT id, :'p1', true, 2, now(), now() FROM users WHERE username = 'user1'",
      channel.id,
    );
    const second = label("The second message");
    const sent = await api(user1, "POST", `/chat/${channel.id}.json`, { message: second, staged_id: "s9" });
    const markdown = await transcript(admin, channel, [channel.messageIds[0], sent.message_id]);
    await openRichComposer(page, context);
    await paste(page, markdown);

    await expect(rich(page).locator(".chat-transcript.chat-transcript-chained")).toHaveCount(2);
    await expect(rich(page).locator(".chat-transcript .chat-transcript-group__channel")).toHaveText(channel.name);
    await expect(
      rich(page).locator(".chat-transcript:nth-of-type(1) .chat-transcript-user .chat-transcript-username"),
    ).toHaveText("admin");
    await expect(
      rich(page).locator(".chat-transcript:nth-of-type(2) .chat-transcript-user .chat-transcript-username"),
    ).toHaveText("user1");
    await expect(rich(page).locator(".chat-transcript:nth-of-type(1) .chat-transcript-messages")).toContainText(first);
    await expect(rich(page).locator(".chat-transcript:nth-of-type(2) .chat-transcript-messages")).toContainText(second);
  });

  test.describe("sanitizing", () => {
    test("sanitizes thread title", async ({ page, context, session }) => {
      const admin = await session("admin");
      const channel = await channelWithMessages(admin, label("Thread channel").replace(/[^a-z0-9 ]/gi, ""), [
        label("The thread's original message"),
      ]);
      sql("UPDATE chat_channels SET threading_enabled = true WHERE id = :'p1'", channel.id);
      const replies = [];
      for (let i = 0; i < 2; i++) {
        const sent = await api(admin, "POST", `/chat/${channel.id}.json`, {
          message: label(`A thread reply ${i}`),
          in_reply_to_id: String(channel.messageIds[0]),
          staged_id: `t${i}`,
        });
        replies.push(sent.message_id);
      }
      sql(
        "UPDATE chat_threads SET title = 'Thread <video src=_ onloadstart=confirm(document.domain)>' \
         WHERE id = (SELECT thread_id FROM chat_messages WHERE id = :'p1')",
        replies[0],
      );
      const markdown = await transcript(admin, channel, replies);
      await openRichComposer(page, context);
      let dialogs = 0;
      page.on("dialog", (dialog) => {
        dialogs++;
        dialog.dismiss();
      });
      await paste(page, markdown);

      await expect(rich(page).locator(".chat-transcript").first()).toBeVisible();
      expect(dialogs).toBe(0);
      await expect(rich(page).locator(".chat-transcript video")).toHaveCount(0);
    });

    test("sanitizes channel title", async ({ page, context, session }) => {
      const admin = await session("admin");
      const name = `Channel <video src=_ onloadstart=confirm(document.domain)> ${label("x").replace(/[^a-z0-9]/gi, "")}`;
      const channel = await channelWithMessages(admin, name, [label("A message")]);
      const markdown = await transcript(admin, channel, channel.messageIds);
      await openRichComposer(page, context);
      let dialogs = 0;
      page.on("dialog", (dialog) => {
        dialogs++;
        dialog.dismiss();
      });
      await paste(page, markdown);

      await expect(rich(page).locator(".chat-transcript")).toHaveCount(1);
      expect(dialogs).toBe(0);
      await expect(rich(page).locator(".chat-transcript video")).toHaveCount(0);
    });

    test("keeps a channel ID which closes the href inside the href", async ({ page, context }) => {
      await openRichComposer(page, context);

      const channelId = `" onmouseover="alert(1)" x="`;
      const markdown = `[chat quote="hunter;29856;2025-03-20T07:13:04Z" channel="design" channelId='${channelId}' multiQuote=true]
haha
[/chat]
`;
      await paste(page, markdown);

      await expect(rich(page).locator(".chat-transcript-group__channel")).toHaveCount(1);

      const attributes = await page.evaluate(() =>
        Object.fromEntries(
          Array.from(
            document.querySelector(".d-editor-input.ProseMirror .chat-transcript-group__channel").attributes,
          ).map((attribute) => [attribute.name, attribute.value]),
        ),
      );

      expect(attributes).toEqual({
        class: "hashtag-cooked chat-transcript-group__channel",
        href: "/chat/c/-/%22%20onmouseover%3D%22alert(1)%22%20x%3D%22",
      });
    });
  });
});
