// Demo steps 6 to 8: the composer (composer.js) opened from a topic and
// from the topic list, the preview while writing, posting, and the post
// arriving on the page without a reload; the mention reaching the other
// member's header live. Its tag chooser: a category's minimum, recently
// used tags first, and a picked tag kept in the draft.
const { test, expect, login, api, topic, sql, label, member } = require("./helpers");

test("a reply written in the composer appears without a reload and notifies the mention live", async ({
  page,
  context,
  session,
}) => {
  const marker = label("composer reply");
  const user0 = await session("user0");
  const t = await topic(user0, label("Composer topic for a reply"), label("A topic user1 replies to, in the browser suite"));
  await login(context, "user1");

  // user3, mentioned, waits on another page. Its unread notifications in
  // the seed are all low priority, so the avatar shows their count (a
  // high priority one, as user0 has, shows its icon instead).
  const user3 = await session("user3");
  const other = await user3.newPage();
  await other.goto("/latest");
  await expect(other.locator("#current-user")).toBeVisible();
  const unread = other.locator("#current-user .badge-notification.unread-notifications");
  const before = (await unread.count()) ? Number(await unread.innerText()) : 0;

  await page.goto(t.url);
  await expect(page.locator(".topic-post")).toHaveCount(1);
  await page.evaluate(() => (window.__noReload = true));
  await page.locator("#topic-footer-buttons .create").click();
  const editor = page.locator("#reply-control.open .d-editor-input");
  await expect(editor).toBeVisible();
  await expect(editor).toBeFocused();
  // composer-open on html, so the page can scroll clear of it (276).
  await expect(page.locator("html")).toHaveClass(/composer-open/);

  await editor.fill(`**${marker}** with _formatting_ for @user3 :tada:`);
  const preview = page.locator("#reply-control .d-editor-preview");
  await expect(preview.locator("strong")).toHaveText(marker);
  await expect(preview.locator("em")).toHaveText("formatting");
  await expect(preview.locator("img.emoji[title=':tada:'], img.emoji[alt=':tada:']")).toHaveCount(1);

  await page.locator("#reply-control .save-or-cancel .create").click();
  await expect(page.locator("#reply-control.open")).toHaveCount(0);
  await expect(page.locator("html")).not.toHaveClass(/composer-open/);

  const post = page.locator(".topic-post", { has: page.locator(".cooked strong", { hasText: marker }) });
  await expect(post).toBeVisible();
  await expect(page.locator(".topic-post")).toHaveCount(2);
  const cooked = post.locator(".cooked");
  await expect(cooked.locator("em")).toHaveText("formatting");
  await expect(cooked.locator("a.mention")).toHaveText("@user3");
  await expect(cooked.locator("img.emoji")).toHaveCount(1);
  expect(await page.evaluate(() => window.__noReload)).toBe(true);

  const rows = sql(
    "SELECT u.username, p.post_number FROM posts p JOIN users u ON u.id = p.user_id WHERE p.topic_id = :'p1' AND p.raw LIKE '%' || :'p2' || '%'",
    t.id,
    marker,
  );
  expect(rows).toEqual([["user1", "2"]]);

  // The mention, over the message bus: the unread count on user3's avatar,
  // and the notification in the user menu.
  await expect(unread).toHaveText(String(before + 1), { timeout: 20_000 });
  await other.locator("#toggle-current-user").click();
  await expect(other.locator(`.user-menu li.notification.mentioned a[href^='${t.url}']`)).toBeVisible();
});

test("a new topic from the topic list opens on its own page", async ({ page, context }) => {
  await login(context, "user1");
  const title = label("Browser suite new topic");
  await page.goto("/latest");
  await page.locator("#create-topic").click();
  const titleInput = page.locator("#reply-title");
  await expect(titleInput).toBeVisible();
  await expect(titleInput).toBeFocused();
  await titleInput.fill(title);
  await page.locator("#reply-control .d-editor-input").fill(`The first post of ${title}, with a [link](https://example.com).`);
  await expect(page.locator("#reply-control .d-editor-preview a[href='https://example.com']")).toHaveText("link");
  await page.locator("#reply-control .save-or-cancel .create").click();

  await expect(page).toHaveURL(/\/t\/[^/]+\/\d+/);
  await expect(page.locator("#topic-title h1")).toContainText(title);
  await expect(page.locator(".topic-post .cooked").first()).toContainText(`The first post of ${title}`);
  const rows = sql("SELECT count(*) FROM topics WHERE title = :'p1' AND archetype = 'regular'", title);
  expect(rows).toEqual([["1"]]);
});

test("discarding a written draft asks first, and keeps it when declined", async ({ page, context }) => {
  await login(context, "user1");
  const title = label("A draft to discard");
  await page.goto("/latest");
  await page.locator("#create-topic").click();
  await page.locator("#reply-title").fill(title);
  await page.locator("#reply-control .d-editor-input").fill("Some words that would be lost.");

  const questions = [];
  page.once("dialog", (d) => {
    questions.push(d.message());
    d.dismiss();
  });
  await page.locator("#reply-control .discard-button").click();
  await expect.poll(() => questions.length).toBe(1);
  await expect(page.locator("#reply-control.open")).toBeVisible();
  await expect(page.locator("#reply-title")).toHaveValue(title);

  page.once("dialog", (d) => {
    questions.push(d.message());
    d.accept();
  });
  await page.locator("#reply-control .discard-button").click();
  await expect(page.locator("#reply-control.open")).toHaveCount(0);
  expect(questions[1]).toBe(questions[0]);
  // Opened again, the composer starts empty: the draft went with it.
  await page.locator("#create-topic").click();
  await expect(page.locator("#reply-title")).toHaveValue("");
});

// composer-body's keyDown and the composer service's hitEsc. Not in the
// reference's tests but for select-kit-accessibility-test.js' "Escape
// does not dismiss composer"; the message closed first is in
// composer-messages.spec.js.
test.describe("Esc in the composer", () => {
  const control = (page) => page.locator("#reply-control");
  const editor = (page) => page.locator("#reply-control textarea.d-editor-input");

  async function replyTo(page, context, session) {
    const writer = member();
    sql("UPDATE user_options SET composition_mode = 0 WHERE user_id = :'p1'", writer.id);
    const user0 = await session("user0");
    const t = await topic(user0, label("Composer Esc topic"), label("A topic replied to, then Esc"));
    await login(context, writer.name);
    await page.goto(t.url);
    await page.locator("#topic-footer-buttons .create").click();
    await expect(editor(page)).toBeFocused();
    return { writer, t };
  }

  const drafts = (writer, t) =>
    sql(
      "SELECT count(*) FROM drafts WHERE user_id = :'p1' AND draft_key = :'p2'",
      writer.id,
      `topic_${t.id}`,
    );

  test("minimizes the open composer, saving what was written", async ({ page, context, session }) => {
    const { writer, t } = await replyTo(page, context, session);
    const questions = [];
    page.on("dialog", (d) => {
      questions.push(d.message());
      d.dismiss();
    });
    await editor(page).fill("Some words kept while the composer is minimized.");

    await editor(page).press("Escape");
    await expect(control(page)).toHaveClass(/\bdraft\b/);
    await expect(control(page)).not.toHaveClass(/\bopen\b/);
    await expect(page.locator("html")).not.toHaveClass(/composer-open/);
    expect(
      await page.evaluate(() => document.documentElement.style.getPropertyValue("--composer-height")),
    ).toBe("40px");
    await expect.poll(() => drafts(writer, t)).toEqual([["1"]]);

    // Minimized, Esc does nothing more.
    await control(page).dispatchEvent("keydown", { key: "Escape" });
    await expect(control(page)).toHaveClass(/\bdraft\b/);

    await // Any click on the minimized bar opens it again.
    await control(page).click({ position: { x: 30, y: 10 } });
    await expect(control(page)).toHaveClass(/\bopen\b/);
    await expect(editor(page)).toHaveValue("Some words kept while the composer is minimized.");
    expect(questions, "nothing is asked: Esc is not a discard").toEqual([]);
  });

  test("minimizes an empty composer too, discarding nothing", async ({ page, context }) => {
    await login(context, member().name);
    await page.goto("/latest");
    await page.locator("#create-topic").click();
    await expect(page.locator("#reply-title")).toBeFocused();

    await page.locator("#reply-title").press("Escape");
    await expect(control(page)).toHaveClass(/\bdraft\b/);
    await expect(control(page)).not.toHaveClass(/\bclosed\b/);

    await // Any click on the minimized bar opens it again.
    await control(page).click({ position: { x: 30, y: 10 } });
    await expect(control(page)).toHaveClass(/\bopen\b/);
    await expect(page.locator("#reply-title")).toBeVisible();
  });

  test("leaves fullscreen for the open composer, its editor focused", async ({ page, context, session }) => {
    await replyTo(page, context, session);
    await editor(page).fill("Written in fullscreen.");
    await control(page).locator(".toggle-fullscreen").click();
    await expect(control(page)).toHaveClass(/\bfullscreen\b/);
    await expect(page.locator("html")).toHaveClass(/fullscreen-composer/);

    // From anywhere within the composer: here its fullscreen toggle.
    await control(page).locator(".toggle-fullscreen").press("Escape");
    await expect(control(page)).toHaveClass(/\bopen\b/);
    await expect(control(page)).not.toHaveClass(/\bfullscreen\b/);
    await expect(page.locator("html")).not.toHaveClass(/fullscreen-composer/);
    await expect(editor(page)).toBeFocused();
    await expect(editor(page)).toHaveValue("Written in fullscreen.");

    await page.keyboard.press("Escape");
    await expect(control(page)).toHaveClass(/\bdraft\b/);
  });

  test("closes an open autocomplete first", async ({ page, context, session }) => {
    await replyTo(page, context, session);
    const menu = page.locator(".fk-d-menu[data-identifier='d-autocomplete'] .autocomplete");
    await editor(page).pressSequentially("Hello @use");
    await expect(menu).toBeVisible();

    await editor(page).press("Escape");
    await expect(menu).toHaveCount(0);
    await expect(control(page)).toHaveClass(/\bopen\b/);

    await editor(page).press("Escape");
    await expect(control(page)).toHaveClass(/\bdraft\b/);
  });

  // select-kit-accessibility-test.js: "Hitting Escape dismisses the tag
  // chooser", "Escape does not dismiss composer".
  test("is the tag chooser's and the category chooser's own", async ({ page, context }) => {
    await login(context, member().name);
    await page.goto("/latest");
    await page.locator("#create-topic").click();
    await expect(page.locator("#reply-title")).toBeFocused();

    const tags = control(page).locator("details.mini-tag-chooser");
    await tags.locator("summary").click();
    await expect(tags.locator(".filter-input")).toBeVisible();
    await tags.locator(".filter-input").press("Escape");
    await expect(tags.locator(".filter-input")).toBeHidden();
    await expect(control(page).locator(".composer-fields")).toBeVisible();
    await expect(control(page)).toHaveClass(/\bopen\b/);

    await control(page).locator("details.category-chooser summary").press("Escape");
    await expect(control(page)).toHaveClass(/\bopen\b/);
  });
});

// The composer's tag chooser (composer.js miniTagChooser): a new topic
// in a category with a minimum tag count is refused without a tag, with
// the minimum in the chooser and the validation's tip, and posts with
// its tags once one is picked.
test("a new topic in a category requiring a tag posts once one is picked", async ({ page, context, session }) => {
  const admin = await session("admin");
  const name = label("Tags required");
  const created = await api(admin, "POST", "/categories.json", {
    name,
    color: "0088CC",
    text_color: "FFFFFF",
    minimum_required_tags: "1",
  });
  const categoryId = String(created.category.id);
  const author = member();
  await login(context, author.name);
  const title = label("A tagged topic from the composer");
  await page.goto("/latest");
  await page.locator("#create-topic").click();
  await page.locator("#reply-title").fill(title);
  await page.locator("#reply-control .d-editor-input").fill(`The first post of ${title}, which needs a tag.`);
  await page.locator("#reply-control details.category-chooser summary").click();
  await page.locator("#reply-control details.category-chooser .filter-input").fill(name);
  await page.locator(`#reply-control .category-row[data-value="${categoryId}"]`).click();
  const tags = page.locator("#reply-control details.mini-tag-chooser");
  await expect(tags).toHaveAttribute("category-id", categoryId);
  await expect(tags.locator("summary .formatted-selection")).toHaveText("select at least 1 tag…");

  await page.locator("#reply-control .save-or-cancel .create").click();
  const tip = page.locator("#reply-control .tags-input .popup-tip");
  await expect(tip).toHaveClass(/\bbad\b/);
  await expect(tip).toHaveText("You must choose at least 1 tag");
  await expect(page.locator("#reply-control.open")).toBeVisible();
  expect(sql("SELECT count(*) FROM topics WHERE title = :'p1'", title)).toEqual([["0"]]);

  const search = page.waitForRequest(
    (r) => r.url().includes("/tags/filter/search") && new URL(r.url()).searchParams.get("categoryId") === categoryId,
  );
  await tags.locator("summary").click();
  await search;
  await tags.locator(".filter-input").fill("howto");
  await tags.locator('.select-kit-row[data-name="howto"]').click();
  await expect(tags.locator("summary .formatted-selection")).toHaveText("howto");
  await expect(tip).toHaveClass(/\bhide\b/);
  await tags.locator(".filter-input").press("Escape");

  await page.locator("#reply-control .save-or-cancel .create").click();
  await expect(page).toHaveURL(/\/t\/[^/]+\/\d+/);
  await expect(page.locator("#topic-title h1")).toContainText(title);
  expect(
    sql(
      "SELECT t.category_id, string_agg(g.name, ',') FROM topics t JOIN topic_tags tt ON tt.topic_id = t.id \
         JOIN tags g ON g.id = tt.tag_id WHERE t.title = :'p1' GROUP BY t.category_id",
      title,
    ),
  ).toEqual([[categoryId, "howto"]]);
});

// prioritizeRecentTags (composer-container passes it, the upcoming change
// prioritize_recently_used_tags decides): with no filter, the member's
// recently used tags come first.
test("the composer's tag chooser puts recently used tags first", async ({ page, context, session }) => {
  const admin = await session("admin");
  const author = member();
  await login(context, author.name);
  // The tag the search lists last without recent tags first.
  const plain = await context.request.get("/tags/filter/search.json?q=&limit=5&filterForInput=true&categoryId=4", {
    headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
  });
  const names = (await plain.json()).results.map((r) => r.name);
  expect(names.length).toBeGreaterThan(1);
  const recent = names[names.length - 1];
  await api(context, "POST", "/posts.json", {
    title: label("A topic tagged recently"),
    raw: label("A topic whose tag the composer puts first"),
    category: "4",
    "tags[]": recent,
  });
  await api(admin, "PUT", "/admin/config/upcoming-changes/toggle.json", {
    setting_name: "prioritize_recently_used_tags",
    enabled: "true",
  });
  try {
    await page.goto("/latest");
    await page.locator("#create-topic").click();
    const tags = page.locator("#reply-control details.mini-tag-chooser");
    const search = page.waitForRequest(
      (r) => r.url().includes("/tags/filter/search") && new URL(r.url()).searchParams.get("prioritizeRecentTags") === "true",
    );
    await tags.locator("summary").click();
    await search;
    await expect(tags.locator(".select-kit-row").first()).toHaveAttribute("data-name", recent);
  } finally {
    await api(admin, "PUT", "/admin/config/upcoming-changes/toggle.json", {
      setting_name: "prioritize_recently_used_tags",
      enabled: "false",
    });
  }
});

// A picked tag goes into the new topic's draft and comes back when the
// draft is opened again from the drafts menu.
test("a tag picked in a new topic survives its draft", async ({ page, context }) => {
  const author = member();
  await login(context, author.name);
  const title = label("A tagged draft");
  await page.goto("/latest");
  await page.locator("#create-topic").click();
  await page.locator("#reply-title").fill(title);
  await page.locator("#reply-control .d-editor-input").fill("A draft whose tag should come back.");
  const tags = page.locator("#reply-control details.mini-tag-chooser");
  await tags.locator("summary").click();
  await tags.locator(".filter-input").fill("guide");
  const saved = page.waitForResponse(
    (r) => r.url().includes("/drafts.json") && r.request().method() === "POST" && (r.request().postData() || "").includes("guide"),
  );
  await tags.locator('.select-kit-row[data-name="guide"]').click();
  await expect(tags.locator("summary .formatted-selection")).toHaveText("guide");
  expect((await saved).ok()).toBeTruthy();
  const [[data]] = sql(
    "SELECT data FROM drafts WHERE user_id = :'p1' AND draft_key LIKE 'new_topic%'",
    author.id,
  );
  expect(JSON.parse(data).tags).toEqual([{ id: 2, name: "guide" }]);

  await page.goto("/latest");
  await page.locator(".topic-drafts-menu-trigger").click();
  await page.locator(".topic-drafts-item button", { hasText: title }).click();
  await expect(page.locator("#reply-title")).toHaveValue(title);
  await expect(tags.locator("summary .formatted-selection")).toHaveText("guide");
});

// The rich editor (ROADMAP 196r1, docs/rich-editor.md): d-editor's toggle
// and Ctrl+M switch the composer between the markdown textarea and the
// ProseMirror editor, the textarea keeps the markdown the editor writes,
// and the mode is the member's composition_mode, saved after a second.
test.describe("the rich editor", () => {
  const rich = (page) => page.locator("#reply-control .ProseMirror.d-editor-input");
  const textarea = (page) => page.locator("#reply-control textarea.d-editor-input");
  const toggle = (page) => page.locator("#reply-control .composer-toggle-switch");
  const savedMode = (page, name, mode) =>
    page.waitForResponse(
      (r) =>
        r.url().endsWith(`/u/${name}.json`) &&
        r.request().method() === "PUT" &&
        (r.request().postData() || "") === `composition_mode=${mode}`,
    );

  async function replyTo(page, context, session, mode) {
    const writer = member();
    sql("UPDATE user_options SET composition_mode = :'p1' WHERE user_id = :'p2'", mode, writer.id);
    const user0 = await session("user0");
    const t = await topic(user0, label("Rich editor topic"), label("A topic the rich editor replies to"));
    await login(context, writer.name);
    await page.goto(t.url);
    await page.locator("#topic-footer-buttons .create").click();
    await expect(page.locator("#reply-control.open")).toBeVisible();
    return { writer, t };
  }

  // composer-body hears the Escape the editor's keymap took for
  // selectParentNode: the first one minimizes the composer.
  test("Esc in the rich editor minimizes the composer", async ({ page, context, session }) => {
    await replyTo(page, context, session, 1);
    await expect(rich(page)).toBeFocused();
    await rich(page).pressSequentially("Kept in the minimized composer");

    await rich(page).press("Escape");
    await expect(page.locator("#reply-control")).toHaveClass(/\bdraft\b/);

    await // Any click on the minimized bar opens it again.
    await page.locator("#reply-control").click({ position: { x: 30, y: 10 } });
    await expect(page.locator("#reply-control")).toHaveClass(/\bopen\b/);
    await expect(rich(page)).toHaveText("Kept in the minimized composer");
  });

  test("switches to rich and back, the markdown being what was typed", async ({ page, context, session }) => {
    const { writer } = await replyTo(page, context, session, 0);
    await expect(toggle(page)).toHaveAttribute("aria-checked", "false");
    await expect(toggle(page)).toHaveClass(/--markdown/);
    await expect(toggle(page)).toHaveAttribute("title", "Switch to new rich text editor (Ctrl M)");
    await expect(textarea(page)).toBeVisible();
    await expect(page.locator("#reply-control .d-editor-preview-wrapper")).toBeVisible();

    const toRich = savedMode(page, writer.name, 1);
    await toggle(page).click();
    await expect(rich(page)).toBeVisible();
    await expect(textarea(page)).toBeHidden();
    await expect(toggle(page)).toHaveAttribute("aria-checked", "true");
    await expect(toggle(page)).toHaveClass(/--rte/);
    await expect(toggle(page)).toHaveAttribute("title", "Switch to standard Markdown editor (Ctrl M)");
    await expect(page.locator("#reply-control .d-editor-container")).toHaveClass(/--rich-editor-enabled/);
    await expect(page.locator("#reply-control .d-editor-preview-wrapper")).toBeHidden();
    await expect(rich(page)).toBeFocused();

    // The input rules: "## " a heading, "**bold**" a strong mark, "* " a list.
    await page.keyboard.type("## Title");
    await page.keyboard.press("Enter");
    await page.keyboard.type("some **bold** words");
    await page.keyboard.press("Enter");
    await page.keyboard.type("* one");
    await page.keyboard.press("Enter");
    await page.keyboard.type("two");
    await expect(rich(page).locator("h2")).toHaveText("Title");
    await expect(rich(page).locator("ul li")).toHaveCount(2);
    expect((await toRich).ok()).toBeTruthy();
    expect(sql("SELECT composition_mode FROM user_options WHERE user_id = :'p1'", writer.id)).toEqual([["1"]]);

    // Ctrl+M from the editor goes back, the textarea holding the markdown.
    const toMarkdown = savedMode(page, writer.name, 0);
    await page.keyboard.press("Control+m");
    await expect(rich(page)).toHaveCount(0);
    await expect(textarea(page)).toBeVisible();
    await expect(textarea(page)).toHaveValue("## Title\n\nsome **bold** words\n\n* one\n* two");
    await expect(page.locator("#reply-control .d-editor-container")).toHaveClass(/--markdown-editor-enabled/);
    await expect(page.locator("#reply-control .d-editor-preview h2")).toHaveText("Title");
    expect((await toMarkdown).ok()).toBeTruthy();
    expect(sql("SELECT composition_mode FROM user_options WHERE user_id = :'p1'", writer.id)).toEqual([["0"]]);

    // And Ctrl+M from the textarea opens it again on that markdown.
    await textarea(page).focus();
    await page.keyboard.press("Control+m");
    await expect(rich(page).locator("h2")).toHaveText("Title");
    await expect(rich(page).locator("p strong")).toHaveText("bold");
    await expect(rich(page).locator("ul li")).toHaveCount(2);
  });

  // d-editor's setupEditorMode: the composer opens in the stored mode,
  // Discourse's default of 1 the rich editor.
  test("opens in the rich editor for a stored mode 1, and the toggle switches and saves", async ({
    page,
    context,
    session,
  }) => {
    const { writer, t } = await replyTo(page, context, session, 1);
    await expect(page.locator("#reply-control")).toHaveAttribute("data-composition-mode", "1");
    await expect(rich(page)).toBeVisible();
    await expect(textarea(page)).toBeHidden();
    await expect(toggle(page)).toHaveAttribute("aria-checked", "true");
    await expect(page.locator("#reply-control .d-editor-container")).toHaveClass(/--rich-editor-enabled/);
    await expect(page.locator("#reply-control .d-editor-preview-wrapper")).toBeHidden();

    const saved = savedMode(page, writer.name, 0);
    await toggle(page).click();
    await expect(rich(page)).toHaveCount(0);
    await expect(textarea(page)).toBeVisible();
    await expect(page.locator("#reply-control .d-editor-preview-wrapper")).toBeVisible();
    expect((await saved).ok()).toBeTruthy();
    expect(sql("SELECT composition_mode FROM user_options WHERE user_id = :'p1'", writer.id)).toEqual([["0"]]);

    // A fresh page opens in the mode saved.
    await page.goto(t.url);
    await page.locator("#topic-footer-buttons .create").click();
    await expect(textarea(page)).toBeVisible();
    await expect(rich(page)).toHaveCount(0);
    await expect(toggle(page)).toHaveAttribute("aria-checked", "false");
  });

  // What the editor still refuses (a chat transcript with an archive's
  // reactions, as the reference refuses it; docs/rich-editor.md) opens in
  // markdown for a stored mode 1: the alert, the raw kept, and the mode
  // saved as 0, as the alert's toggleRichEditor does.
  test("an edit of what the editor refuses falls back to markdown for a stored mode 1", async ({
    page,
    context,
  }) => {
    const writer = member();
    sql("UPDATE user_options SET composition_mode = 1 WHERE user_id = :'p1'", writer.id);
    await login(context, writer.name);
    const raw =
      label("A transcript the rich editor refuses") +
      '\n\n[chat quote="user2;1;2026-10-01T00:00:00Z" channel="General" channelId="2" reactions="+1:user1"]\nhi\n[/chat]';
    const t = await topic(context, label("Rich editor refusal topic"), raw);
    await page.goto(t.url);
    const post = page.locator(`article[data-post-id="${t.posts[0]}"]`);
    const showMore = post.locator(".post-action-menu__show-more");
    if (await showMore.isVisible()) {
      await showMore.click();
    }
    const saved = savedMode(page, writer.name, 0);
    await post.locator(".post-action-menu__edit").click();
    const dialog = page.locator(".dialog-container");
    await expect(dialog.locator(".dialog-body")).toHaveText(
      "The rich text editor doesn’t support all features used in this post; switching you to the Markdown editor.",
    );
    await expect(textarea(page)).toBeVisible();
    await expect(textarea(page)).toHaveValue(raw);
    await expect(rich(page)).toHaveCount(0);
    await expect(toggle(page)).toHaveAttribute("aria-checked", "false");
    await expect(page.locator("#reply-control .d-editor-container")).toHaveClass(/--markdown-editor-enabled/);
    expect((await saved).ok()).toBeTruthy();
    expect(sql("SELECT composition_mode FROM user_options WHERE user_id = :'p1'", writer.id)).toEqual([["0"]]);
    await dialog.locator(".dialog-footer .btn-primary").click();
    await expect(dialog).toBeHidden();
  });

  // The profile's About me (preferences/profile.gjs' DEditor, no mode
  // forced): the member's composition mode, the toggle switching and
  // saving it, and Save Changes sending the markdown the editor wrote.
  test("the About me editor opens in the stored mode and saves the editor's markdown", async ({ page, context }) => {
    const writer = member();
    sql("UPDATE user_options SET composition_mode = 1 WHERE user_id = :'p1'", writer.id);
    sql("UPDATE user_profiles SET bio_raw = 'Some **bold** words' WHERE user_id = :'p1'", writer.id);
    await login(context, writer.name);
    await page.goto(`/u/${writer.name}/preferences/profile`);
    const bio = page.locator(".pref-bio");
    const bioRich = bio.locator(".ProseMirror.d-editor-input");
    const bioArea = bio.locator("textarea.d-editor-input");
    const bioToggle = bio.locator(".composer-toggle-switch");
    await expect(bioRich.locator("p strong")).toHaveText("bold");
    await expect(bioArea).toBeHidden();
    await expect(bioToggle).toHaveAttribute("aria-checked", "true");
    await expect(bio.locator(".d-editor-container")).toHaveClass(/--rich-editor-enabled/);
    await expect(bio.locator(".d-editor-preview-wrapper")).toBeHidden();

    await bioRich.click();
    await page.keyboard.press("Control+End");
    await page.keyboard.press("Enter");
    await page.keyboard.type("## About");
    await expect(bioRich.locator("h2")).toHaveText("About");
    const put = page.waitForResponse(
      (r) => r.url().endsWith(`/u/${writer.name}.json`) && (r.request().postData() || "").includes("bio_raw"),
    );
    await page.locator(".user-preferences .save-changes").click();
    expect((await put).ok()).toBeTruthy();
    expect(
      sql(
        "SELECT bio_raw = 'Some **bold** words' || chr(10) || chr(10) || '## About' FROM user_profiles WHERE user_id = :'p1'",
        writer.id,
      ),
    ).toEqual([["t"]]);

    const saved = savedMode(page, writer.name, 0);
    await bioToggle.click();
    await expect(bioRich).toHaveCount(0);
    await expect(bioArea).toBeVisible();
    await expect(bioArea).toHaveValue("Some **bold** words\n\n## About");
    await expect(bio.locator(".d-editor-preview h2")).toHaveText("About");
    expect((await saved).ok()).toBeTruthy();
    expect(sql("SELECT composition_mode FROM user_options WHERE user_id = :'p1'", writer.id)).toEqual([["0"]]);

    // Ctrl+M from the textarea opens it again, and the composer follows.
    const savedRich = savedMode(page, writer.name, 1);
    await bioArea.focus();
    await page.keyboard.press("Control+m");
    await expect(bioRich.locator("h2")).toHaveText("About");
    expect((await savedRich).ok()).toBeTruthy();
    await expect(page.locator("#reply-control")).toHaveAttribute("data-composition-mode", "1");
  });

  test("the About me editor falls back to markdown on what the editor refuses", async ({ page, context }) => {
    const writer = member();
    sql("UPDATE user_options SET composition_mode = 1 WHERE user_id = :'p1'", writer.id);
    // calendar and events are not ported (ROADMAP 202): refused
    sql(
      "UPDATE user_profiles SET bio_raw = '[event start=\"2030-01-01 10:00\"]' || chr(10) || '[/event]' WHERE user_id = :'p1'",
      writer.id,
    );
    await login(context, writer.name);
    const saved = savedMode(page, writer.name, 0);
    await page.goto(`/u/${writer.name}/preferences/profile`);
    const bio = page.locator(".pref-bio");
    const dialog = page.locator(".dialog-container");
    await expect(dialog.locator(".dialog-body")).toHaveText(
      "The rich text editor doesn’t support all features used in this post; switching you to the Markdown editor.",
    );
    await expect(bio.locator("textarea.d-editor-input")).toBeVisible();
    await expect(bio.locator("textarea.d-editor-input")).toHaveValue('[event start="2030-01-01 10:00"]\n[/event]');
    await expect(bio.locator(".ProseMirror")).toHaveCount(0);
    await expect(bio.locator(".composer-toggle-switch")).toHaveAttribute("aria-checked", "false");
    expect((await saved).ok()).toBeTruthy();
    expect(sql("SELECT composition_mode FROM user_options WHERE user_id = :'p1'", writer.id)).toEqual([["0"]]);
  });

  test("goes back to markdown with the alert on what it cannot hold yet", async ({ page, context, session }) => {
    const { writer } = await replyTo(page, context, session, 0);
    // calendar and events are not ported (ROADMAP 202): refused
    await textarea(page).fill('[event start="2030-01-01 10:00"]\n[/event]');
    const saved = savedMode(page, writer.name, 0);
    await toggle(page).click();
    const dialog = page.locator(".dialog-container");
    await expect(dialog.locator(".dialog-body")).toHaveText(
      "The rich text editor doesn’t support all features used in this post; switching you to the Markdown editor.",
    );
    await expect(textarea(page)).toBeVisible();
    await expect(textarea(page)).toHaveValue('[event start="2030-01-01 10:00"]\n[/event]');
    await expect(rich(page)).toHaveCount(0);
    await expect(toggle(page)).toHaveAttribute("aria-checked", "false");
    expect((await saved).ok()).toBeTruthy();
    await dialog.locator(".dialog-footer .btn-primary").click();
    await expect(dialog).toBeHidden();
  });

  // 196r4: the site's plugins' extensions, from the composer's context,
  // and the node views' modals through the composer's.
  test("opens the plugins' constructs, and Edit poll saves the builder's settings", async ({ page, context, session }) => {
    await replyTo(page, context, session, 0);
    await textarea(page).fill("[details=more]\nhello there\n[/details]\n\n[poll type=multiple]\n* a\n* b\n[/poll]");
    await toggle(page).click();
    await expect(rich(page).locator("details summary")).toHaveText("more");
    await expect(rich(page).locator(".composer-poll-node__content li")).toHaveCount(2);

    await rich(page).locator(".composer-poll-node__edit").click();
    const modal = page.locator(".d-modal.poll-ui-builder");
    await expect(modal).toBeVisible();
    await expect(modal.locator(".poll-options")).toHaveCount(0);
    await modal.locator(".poll-public .d-toggle-switch__checkbox-slider").click();
    await modal.locator(".insert-poll").click();
    await expect(modal).toHaveCount(0);
    await expect(rich(page).locator(".poll")).toHaveAttribute("data-poll-public", "true");

    await toggle(page).click();
    await expect(textarea(page)).toHaveValue(
      '[details="more"]\nhello there\n\n[/details]\n\n[poll type=multiple public=true]\n* a\n* b\n\n[/poll]\n\n'
    );
  });

  test("a wrap's indicator opens Edit Wrap", async ({ page, context, session }) => {
    await replyTo(page, context, session, 0);
    await textarea(page).fill("[wrap=outer]\nhello\n[/wrap]");
    await toggle(page).click();
    await rich(page).locator(".d-wrap-indicator").click();
    const modal = page.locator(".wrap-attributes-modal");
    await expect(modal.getByLabel("Wrap name")).toHaveValue("outer");
    await modal.getByLabel("Wrap name").fill("inner");
    await modal.getByRole("button", { name: "Apply" }).click();
    await expect(modal).toHaveCount(0);
    await expect(rich(page).locator(".d-wrap-indicator")).toHaveText("[wrap=inner]");
    await toggle(page).click();
    await expect(textarea(page)).toHaveValue("[wrap=inner]\nhello\n\n[/wrap]\n\n");
  });

  test("posts the markdown the rich editor writes", async ({ page, context, session }) => {
    const { writer, t } = await replyTo(page, context, session, 0);
    await toggle(page).click();
    await expect(rich(page)).toBeFocused();
    const marker = label("rich reply");
    await page.keyboard.type(`A **bold** reply, ${marker}`);
    await page.keyboard.press("Enter");
    await page.keyboard.type("> quoted");
    await expect(rich(page).locator("blockquote p")).toHaveText("quoted");
    await page.locator("#reply-control .save-or-cancel .create").click();
    await expect(page.locator("#reply-control.open")).toHaveCount(0);
    const post = page.locator(".topic-post .cooked", { hasText: marker });
    await expect(post.locator("strong")).toHaveText("bold");
    await expect(post.locator("blockquote")).toHaveText("quoted");
    // The raw is the serializer's markdown (compared in SQL: it spans lines).
    const raw = `A **bold** reply, ${marker}\n\n> quoted`;
    expect(
      sql("SELECT p.raw = :'p3' FROM posts p WHERE p.topic_id = :'p1' AND p.user_id = :'p2'", t.id, writer.id, raw),
    ).toEqual([["t"]]);
  });
});
