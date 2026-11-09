// The composer's autocompletes in the rich editor (ProseMirror's
// autocompleteHandler in rich-editor/text-manipulation.js under
// autocomplete.js): the reference's system spec
// composer/prosemirror_autocomplete_spec.rb, its autocomplete cases, and
// the completions as the nodes they parse into.
const { test, expect, login, api, sql, label, member } = require("./helpers");

const rich = (page) => page.locator("#reply-control .ProseMirror.d-editor-input");
const textarea = (page) => page.locator("#reply-control textarea.d-editor-input");

// open_composer: a new topic's composer for a member of the test's own on
// the rich editor, focused.
async function openComposer(page, context) {
  const writer = member();
  sql("UPDATE user_options SET composition_mode = 1 WHERE user_id = :'p1'", writer.id);
  await login(context, writer.name);
  await page.goto("/latest");
  await page.locator("#create-topic").click();
  await expect(rich(page)).toBeVisible();
  await rich(page).click();
  return writer;
}

async function typeContent(page, text) {
  for (const ch of text) {
    await page.keyboard.type(ch);
  }
}

// composer.toggle_rich_editor, with the markdown it leaves in the textarea.
async function toggleRichEditor(page) {
  await page.locator("#reply-control .composer-toggle-switch").click();
  await expect(textarea(page)).toBeVisible();
}

// A tag of the test's own (fab!(:tag)).
function makeTag() {
  const name = "tag" + Math.floor(Math.random() * 1e9).toString(36);
  const [[id]] = sql(
    "INSERT INTO tags (name, slug, created_at, updated_at) VALUES (:'p1', :'p1', now(), now()) RETURNING id",
    name
  );
  return { id, name };
}

// A mention node's text, exactly (has_css's text is case sensitive).
function mentionText(name) {
  return new RegExp("^@" + name + "$");
}

test.describe("Composer - ProseMirror - Autocomplete", () => {
  test("triggers an autocomplete on mention", async ({ page, context }) => {
    const currentUser = await openComposer(page, context);
    await typeContent(page, `@${currentUser.name}`);

    await expect(page.locator(".autocomplete.ac-user")).toBeVisible();
  });

  test("triggers an autocomplete on hashtag", async ({ page, context }) => {
    const tag = makeTag();
    await openComposer(page, context);
    await typeContent(page, `#${tag.name}`);

    await expect(page.locator(".autocomplete.hashtag-autocomplete")).toBeVisible();
  });

  test("triggers an autocomplete on emoji", async ({ page, context }) => {
    await openComposer(page, context);
    await typeContent(page, ":smile");

    await expect(page.locator(".autocomplete.ac-emoji")).toBeVisible();
  });

  test("keeps emoji autocomplete open when pressing arrow down near a horizontal rule", async ({ page, context }) => {
    await openComposer(page, context);

    await page.keyboard.press("Enter");
    await typeContent(page, "---");
    await page.keyboard.press("Enter");

    await expect(rich(page).locator("hr")).toHaveCount(1);

    await page.keyboard.press("ArrowUp");
    await page.keyboard.press("ArrowUp");
    await typeContent(page, " :smi");

    const selected = (n) => page.locator(`.autocomplete.ac-emoji li:nth-child(${n}) a.selected`);
    // settled: the term's own results, which reset the selection.
    await expect(page.locator(".autocomplete.ac-emoji .emoji-shortname").first()).toHaveText("smiley");
    await expect(selected(1)).toBeVisible();

    await page.keyboard.press("ArrowDown");

    await expect(selected(2)).toBeVisible();

    await page.keyboard.press("ArrowUp");

    await expect(selected(1)).toBeVisible();
  });

  test("strips partially written emoji when using 'more' emoji modal", async ({ page, context }) => {
    await openComposer(page, context);

    await typeContent(page, "Why :repeat_single");

    await expect(page.locator(".autocomplete.ac-emoji .emoji-shortname").first()).toHaveText("repeat_single_button");

    // "more" emoji picker
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    await page.locator(".emoji-picker__section.filtered img[data-emoji='repeat_single_button']").click();
    await toggleRichEditor(page);

    await expect(textarea(page)).toHaveValue("Why :repeat_single_button: ");
  });

  // prosemirror_autocomplete_spec.rb's "mentions": mentions typed with
  // the autocomplete open as they are, validated into mention nodes.
  test.describe("mentions", () => {
    test("validates manually typed mentions", async ({ page, context }) => {
      const currentUser = await openComposer(page, context);

      // The mention's lookup answered first: the extension skips the
      // mentions typed while one is in flight (as the reference's does),
      // so the next one would wait for an update that may not come.
      const looked = page.waitForResponse((r) => r.url().includes("/composer/mentions"));
      await typeContent(page, `Hey @${currentUser.name} `);
      await looked;

      await expect(rich(page).locator("a.mention", { hasText: mentionText(currentUser.name) })).toBeVisible();

      await typeContent(page, "and @invalid_user - how are you?");

      await expect(rich(page).locator("a.mention", { hasText: "@invalid_user" })).toHaveCount(0);

      await toggleRichEditor(page);

      await expect(textarea(page)).toHaveValue(`Hey @${currentUser.name} and @invalid_user - how are you?`);
    });

    test("validates mentions case-insensitively", async ({ page, context }) => {
      const mixed = member();
      const username = `TestUser_${mixed.name}`;
      sql("UPDATE users SET username = :'p1', username_lower = lower(:'p1') WHERE id = :'p2'", username, mixed.id);
      await openComposer(page, context);

      const looked = page.waitForResponse((r) => r.url().includes("/composer/mentions"));
      await typeContent(page, `Hey @${username.toLowerCase()} and @${username.toUpperCase()} `);
      await looked;

      await expect(rich(page).locator("a.mention", { hasText: mentionText(username.toLowerCase()) })).toBeVisible();
      await expect(rich(page).locator("a.mention", { hasText: mentionText(username.toUpperCase()) })).toBeVisible();

      await typeContent(page, "and @InvalidUser ");

      await expect(rich(page).locator("a.mention", { hasText: "@InvalidUser" })).toHaveCount(0);
    });

    // Not in the reference: a mention picked from the autocomplete is a
    // mention node, the key and the space after it as it completes.
    test("completes a picked user into a mention node", async ({ page, context }) => {
      await openComposer(page, context);

      await typeContent(page, "Hi @user3");
      await expect(page.locator(".autocomplete.ac-user .username").first()).toHaveText("user3");
      await page.keyboard.press("Enter");

      await expect(rich(page).locator("a.mention")).toHaveText("@user3");
      await typeContent(page, "there");
      await toggleRichEditor(page);
      await expect(textarea(page)).toHaveValue("Hi @user3 there");
    });

    test("completes a picked emoji into an emoji node", async ({ page, context }) => {
      await openComposer(page, context);

      await typeContent(page, "Hi :blonde_woman");
      await expect(page.locator(".autocomplete.ac-emoji .emoji-shortname").first()).toHaveText("blonde_woman");
      await page.keyboard.press("Tab");

      await expect(rich(page).locator("img.emoji[alt=':blonde_woman:']")).toHaveCount(1);
      await toggleRichEditor(page);
      await expect(textarea(page)).toHaveValue("Hi :blonde_woman: ");
    });
  });

  // prosemirror_autocomplete_spec.rb's "hashtags": a category or tag
  // picked from the autocomplete, cooked in the editor.
  test.describe("hashtags", () => {
    // A category of the test's own, typed by its whole slug (the
    // reference types `slug[0..1]` on a site of its own).
    async function category(session, attrs) {
      const admin = await session("admin");
      const slug = "c" + Math.floor(Math.random() * 1e9).toString(36);
      const created = await api(admin, "POST", "/categories.json", {
        name: label("Hashtag category"),
        slug,
        color: "0088CC",
        text_color: "FFFFFF",
        ...attrs,
      });
      return created.category;
    }

    async function pick(page, selector) {
      // the parent element, which is the actual hashtag-autocomplete__option
      await page.locator(selector).locator("xpath=..").click();
    }

    test("correctly renders category with emoji hashtags after selecting from autocomplete", async ({
      page,
      context,
      session,
    }) => {
      const cat = await category(session, { emoji: "cat", style_type: "emoji" });
      await openComposer(page, context);

      await typeContent(page, `here is the #${cat.slug}`);
      await expect(page.locator(".autocomplete.hashtag-autocomplete")).toBeVisible();

      await pick(page, `.hashtag-autocomplete .hashtag-color--category-${cat.id}`);
      await expect(
        rich(page).locator(
          `.hashtag-cooked .hashtag-category-emoji.hashtag-color--category-${cat.id} img.emoji[title='cat']`
        )
      ).toBeVisible();
    });

    test("correctly renders category with icon hashtags after selecting from autocomplete", async ({
      page,
      context,
      session,
    }) => {
      const cat = await category(session, { icon: "bell", style_type: "icon" });
      await openComposer(page, context);

      await typeContent(page, `here is the #${cat.slug}`);
      await expect(page.locator(".autocomplete.hashtag-autocomplete")).toBeVisible();

      await pick(page, `.hashtag-autocomplete .hashtag-color--category-${cat.id}`);
      await expect(
        rich(page).locator(
          `.hashtag-cooked .hashtag-category-icon.hashtag-color--category-${cat.id} svg.d-icon.d-icon-bell`
        )
      ).toBeVisible();
      await expect(rich(page).locator(".hashtag-cooked svg use[href='#bell']")).toHaveCount(1);
    });

    test("correctly renders category with square hashtags after selecting from autocomplete", async ({
      page,
      context,
      session,
    }) => {
      const cat = await category(session, {});
      await openComposer(page, context);

      await typeContent(page, `here is the #${cat.slug}`);
      await expect(page.locator(".autocomplete.hashtag-autocomplete")).toBeVisible();

      await pick(page, `.hashtag-autocomplete .hashtag-color--category-${cat.id}`);
      await expect(
        rich(page).locator(`.hashtag-cooked .hashtag-category-square.hashtag-color--category-${cat.id}`)
      ).toHaveCount(1);
    });

    test("correctly renders tag hashtags after selecting from autocomplete", async ({ page, context }) => {
      const tag = makeTag();
      await openComposer(page, context);

      await typeContent(page, `#${tag.name}`);
      await expect(page.locator(".autocomplete.hashtag-autocomplete")).toBeVisible();

      await pick(page, `.hashtag-autocomplete .hashtag-color--tag-${tag.id}`);
      await expect(rich(page).locator(`.hashtag-cooked .d-icon.d-icon-tag.hashtag-color--tag-${tag.id}`)).toBeVisible();
    });
  });
});
