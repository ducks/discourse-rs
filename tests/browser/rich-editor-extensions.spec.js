// The rich editor's everyday extensions (ROADMAP 196r2), with the
// reference's integration tests (frontend/discourse/tests/integration/
// components/prosemirror-editor/): emoji, mention, hashtag, link, quote,
// code-block, html-inline and html-block, and prosemirror-editor-test.gjs's
// registry cases. Table-driven cases go through testMarkdown here; the
// others run their reference body as written in the page
// (rich-editor-helper.js qunitTest). The reference's pretender routes are
// Playwright routes.
const { test, expect } = require("./helpers");
const {
  openEditorPage,
  testMarkdown,
  setupRichEditor,
  editorValue,
  respond,
  qunitTest,
} = require("./rich-editor-helper");

const qtest = (name, body, opts) => qunitTest(test, name, body, opts);

// The emoji images' ?v= (pretty-text/emoji/version's IMAGE_VERSION).
function imageVersion(page) {
  return page.evaluate(() => window.richSpec.engine.emoji("tada").url.split("?v=")[1]);
}

// [name, markdown, html, markdown out], each its own test.
function cases(group, entries, opts = {}) {
  for (const [name, [markdown, html, expected]] of Object.entries(entries)) {
    test(`${group}: ${name}`, async ({ page }) => {
      if (opts.before) {
        await opts.before(page);
      }
      await openEditorPage(page);
      const v = await imageVersion(page);
      let expectedHtml = typeof html === "function" ? html : html.replaceAll("${v}", v);
      if (opts.cleanHtml && typeof expectedHtml === "string") {
        const want = expectedHtml;
        expectedHtml = async (_, result) => expect(opts.cleanHtml(result.html)).toBe(want);
      }
      await testMarkdown(page, markdown, expectedHtml, expected, opts.setup || {});
    });
  }
}

test.describe("rich editor emoji", () => {
  cases("emoji", {
    emoji: [
      "Hey :tada:!",
      '<p>Hey <img class="emoji" alt=":tada:" title=":tada:" src="/images/emoji/twitter/tada.png?v=${v}" contenteditable="false" draggable="true">!</p>',
      "Hey :tada:!",
    ],
    "emoji in heading": [
      "# Heading :information_source:",
      '<h1>Heading <img class="emoji" alt=":information_source:" title=":information_source:" src="/images/emoji/twitter/information_source.png?v=${v}" contenteditable="false" draggable="true"></h1>',
      "# Heading :information_source:",
    ],
    "emoji after a heading": [
      "# Heading\n\n:tada:",
      '<h1>Heading</h1><p><img class="emoji only-emoji" alt=":tada:" title=":tada:" src="/images/emoji/twitter/tada.png?v=${v}" contenteditable="false" draggable="true"></p>',
      "# Heading\n\n:tada:",
    ],
    "single emoji in paragraph gets only-emoji class": [
      ":tada:",
      '<p><img class="emoji only-emoji" alt=":tada:" title=":tada:" src="/images/emoji/twitter/tada.png?v=${v}" contenteditable="false" draggable="true"></p>',
      ":tada:",
    ],
    "three emojis in paragraph get only-emoji class": [
      ":tada: :smile: :heart:",
      '<p><img class="emoji only-emoji" alt=":tada:" title=":tada:" src="/images/emoji/twitter/tada.png?v=${v}" contenteditable="false" draggable="true"> <img class="emoji only-emoji" alt=":smile:" title=":smile:" src="/images/emoji/twitter/smile.png?v=${v}" contenteditable="false" draggable="true"> <img class="emoji only-emoji" alt=":heart:" title=":heart:" src="/images/emoji/twitter/heart.png?v=${v}" contenteditable="false" draggable="true"></p>',
      ":tada: :smile: :heart:",
    ],
    "more than three emojis don't get only-emoji class": [
      ":tada: :smile: :heart: :+1:",
      '<p><img class="emoji" alt=":tada:" title=":tada:" src="/images/emoji/twitter/tada.png?v=${v}" contenteditable="false" draggable="true"> <img class="emoji" alt=":smile:" title=":smile:" src="/images/emoji/twitter/smile.png?v=${v}" contenteditable="false" draggable="true"> <img class="emoji" alt=":heart:" title=":heart:" src="/images/emoji/twitter/heart.png?v=${v}" contenteditable="false" draggable="true"> <img class="emoji" alt=":+1:" title=":+1:" src="/images/emoji/twitter/+1.png?v=${v}" contenteditable="false" draggable="true"></p>',
      ":tada: :smile: :heart: :+1:",
    ],
    "emoji with text doesn't get only-emoji class": [
      "Hello :tada:",
      '<p>Hello <img class="emoji" alt=":tada:" title=":tada:" src="/images/emoji/twitter/tada.png?v=${v}" contenteditable="false" draggable="true"></p>',
      "Hello :tada:",
    ],
  });

  test("typing :code: and an emoticon makes emoji", async ({ page }) => {
    await openEditorPage(page);
    await setupRichEditor(page, "");
    const editor = page.locator("#rich-spec-host .ProseMirror");
    await editor.click();
    await page.keyboard.type("Hi :tada: and :) ");
    await expect(editor.locator("img.emoji")).toHaveCount(2);
    expect(await editorValue(page)).toBe("Hi :tada: and :slight_smile: ");
  });
});

const MENTIONS = {
  users: ["eviltrout", "john"],
  user_reasons: {},
  groups: {
    support: { user_count: 1 },
    unmentionable: { user_count: 5 },
  },
  group_reasons: { unmentionable: "not_mentionable" },
  max_users_notified_per_group_mention: 100,
};

test.describe("rich editor mentions", () => {
  cases(
    "mention",
    {
      mention: [
        "@eviltrout",
        '<p><a class="mention" data-name="eviltrout" contenteditable="false" draggable="true">@eviltrout</a></p>',
        "@eviltrout",
      ],
      "text with mention": [
        "Hello @eviltrout",
        '<p>Hello <a class="mention" data-name="eviltrout" contenteditable="false" draggable="true">@eviltrout</a></p>',
        "Hello @eviltrout",
      ],
      "mention after heading": [
        "## Hello\n\n@eviltrout",
        '<h2>Hello</h2><p><a class="mention" data-name="eviltrout" contenteditable="false" draggable="true">@eviltrout</a></p>',
        "## Hello\n\n@eviltrout",
      ],
      "group mention": [
        "Maybe @support can help",
        '<p>Maybe <a class="mention" data-name="support" contenteditable="false" draggable="true">@support</a> can help</p>',
        "Maybe @support can help",
      ],
      "group and user mention": [
        "Hey @john, I think @support can help here",
        '<p>Hey <a class="mention" data-name="john" contenteditable="false" draggable="true">@john</a>, I think <a class="mention" data-name="support" contenteditable="false" draggable="true">@support</a> can help here</p>',
        "Hey @john, I think @support can help here",
      ],
      "invalid mention": ["Hello @invalid, how are you?", "<p>Hello @invalid, how are you?</p>", "Hello @invalid, how are you?"],
      "here mention": [
        "Hello @here, how are you?",
        '<p>Hello <a class="mention" data-name="here" contenteditable="false" draggable="true">@here</a>, how are you?</p>',
        "Hello @here, how are you?",
      ],
    },
    { before: (page) => respond(page, "**/composer/mentions?*", MENTIONS), setup: { settle: 300 } }
  );

  test("asks /composer/mentions with each name and the topic", async ({ page }) => {
    let asked = null;
    await page.route("**/composer/mentions?*", (route) => {
      asked = new URL(route.request().url()).searchParams;
      route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(MENTIONS) });
    });
    await openEditorPage(page);
    await setupRichEditor(page, "@john and @support", { topicId: 7, settle: 300 });
    expect(asked.getAll("names[]")).toEqual(["john", "support"]);
    expect(asked.get("topic_id")).toBe("7");
  });

  test("typing a mention makes the node", async ({ page }) => {
    await respond(page, "**/composer/mentions?*", MENTIONS);
    await openEditorPage(page);
    await setupRichEditor(page, "");
    const editor = page.locator("#rich-spec-host .ProseMirror");
    await editor.click();
    await page.keyboard.type("Hey @john ");
    await expect(editor.locator("a.mention[data-name='john']")).toHaveCount(1);
    expect(await editorValue(page)).toBe("Hey @john ");
  });
});

const HASHTAGS = {
  categories: [
    { type: "category", ref: "product", style_type: "square", id: 2 },
    { type: "category", ref: "coffee", emoji: "coffee", style_type: "emoji", id: 3 },
    { type: "category", ref: "discuss", icon: "comment", style_type: "icon", id: 4 },
    { type: "category", ref: "welcome", text: "hello :wave:", style_type: "square", id: 5 },
  ],
  tags: [{ type: "tag", ref: "dev", icon: "tag", id: 1 }],
};

test.describe("rich editor hashtags", () => {
  cases(
    "hashtag",
    {
      hashtag: [
        "#product",
        '<p><a class="hashtag-cooked" data-name="product" data-processed="true" contenteditable="false" draggable="true"><span class="hashtag-category-square hashtag-color--category-2"></span>product</a></p>',
        "#product",
      ],
      "text with hashtag": [
        "Hello #product",
        '<p>Hello <a class="hashtag-cooked" data-name="product" data-processed="true" contenteditable="false" draggable="true"><span class="hashtag-category-square hashtag-color--category-2"></span>product</a></p>',
        "Hello #product",
      ],
      "hashtag after heading": [
        "## Hello\n\n#product",
        '<h2>Hello</h2><p><a class="hashtag-cooked" data-name="product" data-processed="true" contenteditable="false" draggable="true"><span class="hashtag-category-square hashtag-color--category-2"></span>product</a></p>',
        "## Hello\n\n#product",
      ],
      "invalid hashtag": ["Hello #invalid, how are you?", "<p>Hello #invalid, how are you?</p>", "Hello #invalid, how are you?"],
      "with regular tags": [
        "Hello #dev",
        '<p>Hello <a class="hashtag-cooked" data-name="dev" data-processed="true" contenteditable="false" draggable="true"><svg class="fa d-icon d-icon-tag svg-icon fa-width-auto hashtag-color--tag-1 svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#tag"></use></svg>dev</a></p>',
        "Hello #dev",
      ],
      "hashtag with emoji": [
        "Time for #coffee",
        '<p>Time for <a class="hashtag-cooked" data-name="coffee" data-processed="true" contenteditable="false" draggable="true"><span class="hashtag-category-emoji hashtag-color--category-3"><img width="20" height="20" src="/images/emoji/twitter/coffee.png?v=${v}" title="coffee" alt="coffee" class="emoji"></span>coffee</a></p>',
        "Time for #coffee",
      ],
      "hashtag with icon": [
        "Lets #discuss",
        '<p>Lets <a class="hashtag-cooked" data-name="discuss" data-processed="true" contenteditable="false" draggable="true"><span class="hashtag-category-icon hashtag-color--category-4"><svg class="fa d-icon d-icon-comment svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#comment"></use></svg></span>discuss</a></p>',
        "Lets #discuss",
      ],
      "hashtag with emoji in text": [
        "#welcome",
        '<p><a class="hashtag-cooked" data-name="welcome" data-processed="true" contenteditable="false" draggable="true"><span class="hashtag-category-square hashtag-color--category-5"></span>hello <img width="20" height="20" src="/images/emoji/twitter/wave.png?v=${v}" title="wave" alt="wave" class="emoji"></a></p>',
        "#welcome",
      ],
      "anchor in link matching hashtag is not converted": [
        "[Docs](https://example.com/docs/#product)",
        '<p><a href="https://example.com/docs/#product">Docs</a></p>',
        "[Docs](https://example.com/docs/#product)",
      ],
    },
    { before: (page) => respond(page, "**/hashtags?*", HASHTAGS), setup: { settle: 300 } }
  );

  // The reference's case is a channel hashtag in safe mode; chat's channel
  // type may well be registered here, so the type is one nothing knows.
  test("hashtag: renders without crashing when the server returns a type that isn't registered on the client", async ({
    page,
  }) => {
    await respond(page, "**/hashtags?*", {
      widgets: [{ type: "widget", ref: "unknown-type-channel", icon: "comment", id: 1 }],
    });
    await openEditorPage(page);
    await testMarkdown(
      page,
      "Hello #unknown-type-channel",
      '<p>Hello <a class="hashtag-cooked" data-name="unknown-type-channel" data-processed="true" contenteditable="false" draggable="true">unknown-type-channel</a></p>',
      "Hello #unknown-type-channel",
      { settle: 300 }
    );
  });
});

test.describe("rich editor links", () => {
  cases("link", {
    "basic link": [
      "[Example](https://example.com)",
      '<p><a href="https://example.com">Example</a></p>',
      "[Example](https://example.com)",
    ],
    "link with title": [
      '[Example](https://example.com "Example Title")',
      '<p><a href="https://example.com" title="Example Title">Example</a></p>',
      '[Example](https://example.com "Example Title")',
    ],
    autolink: [
      "<https://example.com>",
      '<p><a href="https://example.com" data-markup="autolink">https://example.com</a></p>',
      "<https://example.com>",
    ],
    "auto links with ~": [
      "<https://example.com/~user> https://example.com/~user",
      '<p><a href="https://example.com/~user" data-markup="autolink">https://example.com/~user</a> <a href="https://example.com/~user" data-markup="linkify">https://example.com/~user</a></p>',
      "<https://example.com/~user> https://example.com/~user",
    ],
    "attachment link": [
      "[File|attachment](https://example.com/file.pdf)",
      '<p><a href="https://example.com/file.pdf" class="attachment">File</a></p>',
      "[File|attachment](https://example.com/file.pdf)",
    ],
    "attachment link with hash upload": [
      "[File|attachment](upload://some-hash)",
      '<p><a href="/404" class="attachment" data-orig-href="upload://some-hash">File</a></p>',
      "[File|attachment](upload://some-hash)",
    ],
    "link with anchor": [
      "[Docs](https://example.com/docs/#section)",
      '<p><a href="https://example.com/docs/#section">Docs</a></p>',
      "[Docs](https://example.com/docs/#section)",
    ],
  });

  qtest("pasting rich content over a selection: a lone URL links the selection", async function (assert, { helpers, pmState }) {
    const { setupRichEditor, settled } = helpers;
    const { TextSelection } = pmState;
    const [editor] = await setupRichEditor(assert, "Hello world");
    const { view } = editor;
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 7, 12)));
    view.pasteHTML("<span>https://discourse.org</span>");
    await settled();

    assert.strictEqual(editor.value, "Hello [world](https://discourse.org)");
  });

  qtest(
    "pasting rich content over a selection: text with a URL among other words replaces the selection",
    async function (assert, { helpers, pmState }) {
      const { setupRichEditor, settled } = helpers;
      const { TextSelection } = pmState;
      const [editor] = await setupRichEditor(assert, "Hello world");
      const { view } = editor;
      view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 7, 12)));
      view.pasteHTML("<span>see https://discourse.org</span>");
      await settled();

      assert.strictEqual(editor.value, "Hello see https://discourse.org");
    }
  );

  test("typing a bare url links it, and the link toolbar opens on it", async ({ page }) => {
    await openEditorPage(page);
    await setupRichEditor(page, "");
    await page.locator("#rich-spec-host").scrollIntoViewIfNeeded();
    const editor = page.locator("#rich-spec-host .ProseMirror");
    await editor.click();
    await page.keyboard.type("see https://example.com now");
    await expect(editor.locator('a[data-markup="linkify"]')).toHaveAttribute("href", "https://example.com");
    expect(await editorValue(page)).toBe("see https://example.com now");
    // The toolbars typing opened and left close animated, then focus the
    // view, which writes its selection back to the DOM: an arrow key whose
    // selectionchange is still pending would be undone.
    await expect(page.locator('[data-identifier="composer-link-toolbar"]')).toHaveCount(0);

    // Back into the link: the floating toolbar, without unlink for an
    // automatic link, with the visit link.
    await page.keyboard.press("ArrowLeft");
    for (let i = 0; i < 6; i++) {
      await page.keyboard.press("ArrowLeft");
    }
    const toolbar = page.locator('[data-identifier="composer-link-toolbar"]:not(.-closing)');
    await expect(toolbar.locator(".composer-link-toolbar__copy")).toBeVisible();
    await expect(toolbar.locator(".composer-link-toolbar__unlink")).toHaveCount(0);
    await expect(toolbar.locator("a.composer-link-toolbar__visit")).toHaveAttribute("href", "https://example.com");
    await expect(toolbar.locator(".composer-link-toolbar__edit")).toBeVisible();
    // No Load preview: a top-level url inline gets no onebox.
    await expect(toolbar.locator(".composer-link-toolbar__show-preview")).toHaveCount(0);
  });

  // The link toolbar's Edit: UpsertHyperlink through the modal service,
  // its addText replacing the link.
  test("the link toolbar's Edit opens UpsertHyperlink and replaces the link", async ({ page }) => {
    await openEditorPage(page);
    await setupRichEditor(page, "a [text](https://example.com) b");
    await page.locator("#rich-spec-host").scrollIntoViewIfNeeded();
    const editor = page.locator("#rich-spec-host .ProseMirror");
    await editor.locator("a").click();
    const toolbar = page.locator('[data-identifier="composer-link-toolbar"]:not(.-closing)');
    await toolbar.locator(".composer-link-toolbar__edit").click();
    const model = await page.evaluate(() => {
      const { name, opts } = window.richSpec.activeModal;
      return { name, editing: opts.model.editing, linkText: opts.model.linkText, linkUrl: opts.model.linkUrl };
    });
    expect(model).toEqual({ name: "upsert-hyperlink", editing: true, linkText: "text", linkUrl: "https://example.com" });
    await page.evaluate(() =>
      window.richSpec.activeModal.opts.model.toolbarEvent.addText("[new text](https://example.org)"),
    );
    await expect(editor.locator("a")).toHaveAttribute("href", "https://example.org");
    expect(await editorValue(page)).toBe("a [new text](https://example.org) b");
  });

  test("the link toolbar unlinks a markdown link and closes off it", async ({ page }) => {
    await openEditorPage(page);
    await setupRichEditor(page, "a [text](https://example.com) b");
    await page.locator("#rich-spec-host").scrollIntoViewIfNeeded();
    await page.evaluate(async () => {
      const { TextSelection } = await import("prosemirror-state");
      const view = window.richSpec.editor.view;
      view.focus();
      view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 5)));
    });
    const toolbar = page.locator('[data-identifier="composer-link-toolbar"]:not(.-closing)');
    await expect(toolbar.locator(".composer-link-toolbar__visit .d-button-label")).toHaveText("example.com");
    await toolbar.locator(".composer-link-toolbar__unlink").click();
    expect(await editorValue(page)).toBe("a text b");
    await expect(toolbar).toHaveCount(0);
  });
});

test.describe("rich editor quotes", () => {
  cases("quote", {
    "basic quote": [
      `[quote]\nThis is a quote.\n\n[/quote]\n\nThis is not`,
      `<aside class="quote" data-full="false"><blockquote><p>This is a quote.</p></blockquote></aside><p>This is not</p>`,
      `[quote]\nThis is a quote.\n\n[/quote]\n\nThis is not`,
    ],
    "quote with username": [
      `[quote="User"]\nQuoted text.\n\n[/quote]`,
      `<aside class="quote" data-username="User" data-full="false"><div class="title" contenteditable="false">User:</div><blockquote><p>Quoted text.</p></blockquote></aside>`,
      `[quote="User"]\nQuoted text.\n\n[/quote]\n\n`,
    ],
    "quote with topic ID": [
      `[quote="User, topic:456"]\nQuoted from a topic.\n\n[/quote]`,
      `<aside class="quote" data-username="User" data-topic="456" data-full="false"><div class="title" contenteditable="false">User:</div><blockquote><p>Quoted from a topic.</p></blockquote></aside>`,
      `[quote="User, topic:456"]\nQuoted from a topic.\n\n[/quote]\n\n`,
    ],
    "quote with topic ID and post number": [
      `[quote="User, post:123, topic:456"]\nFull quote example.\n\n[/quote]`,
      `<aside class="quote" data-username="User" data-post="123" data-topic="456" data-full="false"><div class="title" contenteditable="false">User:</div><blockquote><p>Full quote example.</p></blockquote></aside>`,
      `[quote="User, post:123, topic:456"]\nFull quote example.\n\n[/quote]\n\n`,
    ],
    "quote with display name and username": [
      `[quote="Full Name, post:123, topic:456, username:user123"]\nQuoted with display name.\n\n[/quote]`,
      `<aside class="quote" data-username="user123" data-post="123" data-topic="456" data-full="false" data-display-name="Full Name"><div class="title" contenteditable="false">Full Name:</div><blockquote><p>Quoted with display name.</p></blockquote></aside>`,
      `[quote="Full Name, post:123, topic:456, username:user123"]\nQuoted with display name.\n\n[/quote]\n\n`,
    ],
    "quote with display name containing comma": [
      `[quote="Last, First, post:123, topic:456, username:user123"]\nComma name.\n\n[/quote]`,
      `<aside class="quote" data-username="user123" data-post="123" data-topic="456" data-full="false" data-display-name="Last, First"><div class="title" contenteditable="false">Last, First:</div><blockquote><p>Comma name.</p></blockquote></aside>`,
      `[quote="Last, First, post:123, topic:456, username:user123"]\nComma name.\n\n[/quote]\n\n`,
    ],
    "quote with full:true": [
      `[quote="User, post:1, topic:2, full:true"]\nFull quote.\n\n[/quote]`,
      `<aside class="quote" data-username="User" data-post="1" data-topic="2" data-full="true"><div class="title" contenteditable="false">User:</div><blockquote><p>Full quote.</p></blockquote></aside>`,
      `[quote="User, post:1, topic:2, full:true"]\nFull quote.\n\n[/quote]\n\n`,
    ],
  });

  // markdownOptions' lookup is a function, which the page builds itself.
  qtest("avatar from the host editor's topic", async function (assert, { helpers }) {
    const { setupRichEditor } = helpers;
    const [editor] = await setupRichEditor(
      assert,
      `[quote="Full Name, post:123, topic:456, username:quoted_user"]\nQuoted text.\n\n[/quote]`,
      {
        markdownOptions: {
          lookupAvatarTemplateByPostNumber: (postNumber, topicId) =>
            postNumber === 123 && topicId === 456 ? "/images/avatar/{size}.png" : null,
        },
      }
    );

    assert
      .dom("aside.quote .title img.avatar")
      .hasAttribute("src", /^\/images\/avatar\/\d+\.png$/, "the avatar is rendered at a resolved size");
    assert.dom("aside.quote .title").hasText("Full Name:", "the display name is shown");

    assert.strictEqual(
      editor.value,
      `[quote="Full Name, post:123, topic:456, username:quoted_user"]\nQuoted text.\n\n[/quote]\n\n`,
      "avatar does not leak into the serialized markdown"
    );
  });

  qtest("no avatar when the quoted post is not in the topic", async function (assert, { helpers }) {
    const { setupRichEditor } = helpers;
    await setupRichEditor(assert, `[quote="User, post:99, topic:11"]\nQuoted text.\n\n[/quote]`, {
      markdownOptions: { lookupAvatarTemplateByPostNumber: () => undefined },
    });

    assert.dom("aside.quote .title img.avatar").doesNotExist("no avatar is rendered for an unresolved post");
    assert.dom("aside.quote .title").hasText("User:", "the username is still shown");
  });

  qtest("quote without a username renders no title", async function (assert, { helpers }) {
    const { setupRichEditor, settled } = helpers;
    const [editor] = await setupRichEditor(assert, `[quote="removed_user"]\nQuoted text.\n\n[/quote]`);

    const { view } = editor;
    view.dispatch(view.state.tr.setNodeMarkup(0, null, { ...view.state.doc.nodeAt(0).attrs, username: null }));
    await settled();

    assert.dom("aside.quote .title").doesNotExist("the header is removed with the username");
    assert.dom("aside.quote blockquote").hasText("Quoted text.", "the quoted content is kept");
  });

  test("quote can be selected and removed", async ({ page }) => {
    await openEditorPage(page);
    await setupRichEditor(page, `[quote="User"]\nQuoted text.\n\n[/quote]`);
    await page.locator("aside.quote .title").click();
    await expect(page.locator("aside.quote")).toHaveClass(/ProseMirror-selectednode/);
    await page.keyboard.press("Backspace");
    await expect(page.locator("aside.quote")).toHaveCount(0);
  });

  qtest("the header is not rebuilt while typing in the quote", async function (assert, { helpers }) {
    const { setupRichEditor, settled, find } = helpers;
    const [editor] = await setupRichEditor(assert, `[quote="User, post:1, topic:2"]\nText.\n\n[/quote]`, {
      markdownOptions: { lookupAvatarTemplateByPostNumber: () => "/a/{size}.png" },
    });

    const title = find("aside.quote .title");
    const avatar = find("aside.quote .title img");

    const { view } = editor;
    view.dispatch(view.state.tr.insertText("x", 3));
    await settled();

    assert.strictEqual(title, find("aside.quote .title"), "the title element is reused");
    assert.strictEqual(avatar, find("aside.quote .title img"), "the avatar element is reused");
  });

  qtest("a hostile avatar template cannot inject markup", async function (assert, { helpers }) {
    const { setupRichEditor } = helpers;
    await setupRichEditor(assert, `[quote="User, post:1, topic:2"]\nQuoted text.\n\n[/quote]`, {
      markdownOptions: {
        lookupAvatarTemplateByPostNumber: () => `/a/{size}.png" onerror="throw new Error('injected')`,
      },
    });

    assert.dom("aside.quote .title img.avatar").doesNotHaveAttribute("onerror", "the template cannot add attributes");
    assert.dom("aside.quote .title script").doesNotExist("the template cannot add elements");
  });

  cases(
    "quote",
    {
      "quote survives repeated node view teardown": [
        `[quote="User"]\nQuoted text.\n\n[/quote]`,
        `<aside class="quote" data-username="User" data-full="false"><div class="title" contenteditable="false">User:</div><blockquote><p>Quoted text.</p></blockquote></aside>`,
        `[quote="User"]\nQuoted text.\n\n[/quote]\n\n`,
      ],
    },
    { setup: { multiToggle: true } }
  );
});

test.describe("rich editor code blocks", () => {
  // The languages are the site's highlight.js bundle's.
  async function select(page, lang = "") {
    const languages = await page.evaluate(async () => {
      const hljs = await window.Discourse.highlightSyntax.ensureHighlightJs();
      return hljs.listLanguages().sort((a, b) => a.localeCompare(b));
    });
    return (
      `<select contenteditable="false" class="code-language-select"><option>${lang}</option>` +
      languages.map((l) => `<option>${l}</option>`).join("") +
      "</select>"
    );
  }

  const CASES = {
    "basic code block": [
      "```plaintext\nconsole.log('Hello, world!');\n```",
      (s) => `<pre><code>console.log('Hello, world!');</code>${s("plaintext")}</pre>`,
      "```plaintext\nconsole.log('Hello, world!');\n```",
    ],
    "basic code block without a language": [
      "```\nconsole.log('Hello, world!');\n```",
      (s) => `<pre><code>console.log('Hello, world!');</code>${s()}</pre>`,
      "```\nconsole.log('Hello, world!');\n```",
    ],
    "code block within list item": [
      "- ```plaintext\n  console.log('Hello, world!');\n  ```",
      (s) => `<ul><li><pre><code>console.log('Hello, world!');</code>${s("plaintext")}</pre></li></ul>`,
      "* ```plaintext\n  console.log('Hello, world!');\n  ```",
    ],
    "code block with language": [
      '```javascript\nconsole.log("Hello, world!");\n```',
      (s) =>
        `<pre><code><span class="hljs-variable language_">console</span>.<span class="hljs-title function_">log</span>(<span class="hljs-string">"Hello, world!"</span>);</code>${s()}</pre>`,
      '```javascript\nconsole.log("Hello, world!");\n```',
    ],
    "code block with 4 spaces": [
      "    print('Hello, world!')",
      (s) => `<pre><code>print('Hello, world!')</code>${s()}</pre>`,
      "```\nprint('Hello, world!')\n```",
    ],
    "code block with 4 spaces within list item": [
      "-     print('Hello, world!')",
      (s) => `<ul><li><pre><code>print('Hello, world!')</code>${s()}</pre></li></ul>`,
      "* ```\n  print('Hello, world!')\n  ```",
    ],
  };

  for (const [name, [markdown, html, expected]] of Object.entries(CASES)) {
    test(`code-block: ${name}`, async ({ page }) => {
      await openEditorPage(page);
      const languages = await select(page);
      // The empty option names a language the bundle does not have.
      const s = (lang = "") =>
        languages.replace("<option></option>", `<option>${languages.includes(`<option>${lang}</option>`) ? "" : lang}</option>`);
      await testMarkdown(page, markdown, html(s), expected, { multiToggle: true, settle: 300 });
    });
  }

  test("code-block: code block without a language is highlighted with default_code_lang", async ({ page }) => {
    await openEditorPage(page);
    const s = await select(page);
    await testMarkdown(
      page,
      '```\nconsole.log("Hello, world!");\n```',
      `<pre><code><span class="hljs-variable language_">console</span>.<span class="hljs-title function_">log</span>(<span class="hljs-string">"Hello, world!"</span>);</code>${s}</pre>`,
      '```\nconsole.log("Hello, world!");\n```',
      { siteSettings: { default_code_lang: "javascript" }, settle: 300 }
    );
  });

  qtest("pasting code with line wrappers keeps a single code block", async function (assert, { helpers }) {
    const { setupRichEditor, settled } = helpers;
    const [editor] = await setupRichEditor(assert, "");

    editor.view.pasteHTML(
      '<pre class="language-bash"><code><div><span>first</span><br></div><div><br></div><div>  last<br></div></code></pre>'
    );
    await settled();

    assert.dom(".ProseMirror pre").exists({ count: 1 }, "keeps one code block");
    assert.strictEqual(
      editor.view.state.doc.firstChild.textContent,
      "first\n\n  last",
      "preserves line breaks and indentation in the pasted code"
    );
    assert.strictEqual(editor.view.state.doc.firstChild.attrs.params, "bash", "preserves the code language");
  });

  qtest("language selector options are alphabetically sorted", async function (assert, { helpers }) {
    const { testMarkdown } = helpers;
    await testMarkdown(
      assert,
      "```\nconsole.log('Hello, world!');\n```",
      () => {
        const options = [...document.querySelectorAll(".code-language-select option")]
          .slice(1)
          .map((option) => option.textContent);

        assert.deepEqual(
          options,
          [...options].sort((a, b) => a.localeCompare(b))
        );
      },
      "```\nconsole.log('Hello, world!');\n```",
      { multiToggle: true, settle: 300 }
    );
  });

  qtest("Tab indents selected code block lines with two spaces", async function (assert, { helpers, pmState }) {
    const { setupRichEditor, triggerKeyEvent } = helpers;
    const { TextSelection } = pmState;
    const markdown =
      '```csharp\n      this.board.acl = [\n        {\n          type: "group",\n        },\n      ];\n```';
    const [editor] = await setupRichEditor(assert, markdown);
    const { view } = editor;
    const codeBlock = view.state.doc.firstChild;

    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 7, codeBlock.content.size + 1)));
    await triggerKeyEvent(".ProseMirror", "keydown", "Tab");

    assert.strictEqual(
      view.state.doc.firstChild.textContent,
      '        this.board.acl = [\n          {\n            type: "group",\n          },\n        ];',
      "all partially selected lines are indented"
    );
    assert.strictEqual(
      view.state.doc.textBetween(view.state.selection.from, view.state.selection.to),
      'this.board.acl = [\n          {\n            type: "group",\n          },\n        ];',
      "the code remains selected after indenting"
    );
  });

  qtest("Shift-Tab removes up to two spaces from selected code block lines", async function (assert, { helpers, pmState }) {
    const { setupRichEditor, triggerKeyEvent } = helpers;
    const { TextSelection } = pmState;
    const markdown = "```\n  one\n two\nthree\n```";
    const [editor] = await setupRichEditor(assert, markdown);
    const { view } = editor;
    const codeBlock = view.state.doc.firstChild;

    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 1, codeBlock.content.size + 1)));
    await triggerKeyEvent(".ProseMirror", "keydown", "Tab", { shiftKey: true });

    assert.strictEqual(view.state.doc.firstChild.textContent, "one\ntwo\nthree", "each selected line is outdented as far as possible");
  });

  // Outside a code block Tab is d-editor's keymap (indentSelection's
  // sinkListItem), which the helper's editor has as the composer's does.
  qtest("Tab still nests list items", async function (assert, { helpers, pmState }) {
    const { setupRichEditor, triggerKeyEvent } = helpers;
    const { TextSelection } = pmState;
    const [editor] = await setupRichEditor(assert, "* first\n* second");
    const { view } = editor;

    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 12)));
    await triggerKeyEvent(".ProseMirror", "keydown", "Tab");

    assert.dom(".ProseMirror > ul > li > ul > li").hasText("second", "the second item is nested under the first");

    await triggerKeyEvent(".ProseMirror", "keydown", "Tab", {
      shiftKey: true,
    });

    assert.dom(".ProseMirror > ul > li").exists({ count: 2 }, "Shift-Tab lifts the nested item");
  });
});

test.describe("rich editor inline HTML", () => {
  // assert.dom checks, run in the page after the editor opens.
  const outlined = (selector, title) => async (page) => {
    const node = page.locator(`.ProseMirror ${selector}`);
    await expect(node).toHaveClass(/composer-html-inline-node/);
    await expect(node).toHaveAttribute("title", title);
  };

  cases("html-inline", {
    "basic inline HTML tags": ["<kbd>Ctrl</kbd>", "<p><kbd>Ctrl</kbd> </p>", "<kbd>Ctrl</kbd> "],
    "nested inline HTML": ["<sup><small>text</small></sup>", "<p><sup><small>text</small></sup> </p>", "<sup><small>text</small></sup> "],
    "inline marks using HTML": [
      "<strong>bold</strong> and <em>italic</em>",
      "<p><strong>bold</strong> and <em>italic</em></p>",
      "**bold** and *italic*",
    ],
    "mixed HTML inline and marks": [
      "<kbd><strong>Ctrl+B</strong></kbd>",
      "<p><kbd><strong>Ctrl+B</strong></kbd> </p>",
      "<kbd>**Ctrl+B**</kbd> ",
    ],
    "multiple inline elements": [
      "Text with <sub>subscript</sub> and <sup>superscript</sup>",
      "<p>Text with <sub>subscript</sub> and <sup>superscript</sup> </p>",
      "Text with <sub>subscript</sub> and <sup>superscript</sup> ",
    ],
    "several blocks ending in inline HTML": [
      "a <sup>1</sup>\n\nb <mark>m</mark>\n\nc <del>d</del>",
      "<p>a <sup>1</sup> </p><p>b <mark>m</mark> </p><p>c <del>d</del> </p>",
      "a <sup>1</sup> \n\nb <mark>m</mark> \n\nc <del>d</del> ",
    ],
    "HTML mark aliases": ["<b>bold</b> and <i>italic</i>", "<p><strong>bold</strong> and <em>italic</em></p>", "**bold** and *italic*"],
    "semantic HTML elements": [
      "<ins>inserted</ins> and <del>deleted</del>",
      "<p><ins>inserted</ins> and <del>deleted</del> </p>",
      "<ins>inserted</ins> and <del>deleted</del> ",
    ],
    "link HTML with href attribute": [
      '<a href="https://example.com">text</a>',
      '<p><a href="https://example.com">text</a></p>',
      "[text](https://example.com)",
    ],
    "link HTML with href and title": [
      '<a href="https://example.com" title="Title">x</a>',
      '<p><a href="https://example.com" title="Title">x</a></p>',
      '[x](https://example.com "Title")',
    ],
    "unmatched opening inline tag auto-closes": ["<kbd>x", "<p><kbd>x</kbd> </p>", "<kbd>x</kbd> "],
    "unmatched opening link tag auto-closes": [
      '<a href="https://example.com">x',
      '<p><a href="https://example.com">x</a></p>',
      "[x](https://example.com)",
    ],
    "image HTML with src, alt, width and height": [
      '<img src="https://example.com/image.png" alt="Alt text" width="100" height="200">',
      async (page) => {
        const img = page.locator("#rich-spec-host p img:not(.ProseMirror-separator)");
        await expect(img).toHaveAttribute("src", "https://example.com/image.png");
        await expect(img).toHaveAttribute("alt", "Alt text");
        await expect(img).toHaveAttribute("width", "100");
        await expect(img).toHaveAttribute("height", "200");
      },
      "![Alt text|100x200](https://example.com/image.png)",
    ],
    "image HTMl with text after": [
      '<img src="https://example.com/image.png" alt="Alt text"> after',
      async (page) => {
        const img = page.locator("#rich-spec-host p img:not(.ProseMirror-separator)");
        await expect(img).toHaveAttribute("src", "https://example.com/image.png");
        await expect(img).toHaveAttribute("alt", "Alt text");
        await expect(page.locator("#rich-spec-host p")).toHaveText(/after/);
      },
      "![Alt text](https://example.com/image.png) after",
    ],
    "ruby annotation round-trip": [
      "<ruby>漢字<rp>(</rp><rt>かんじ</rt><rp>)</rp></ruby>",
      "<p><ruby>漢字<rp>(</rp><rt>かんじ</rt><rp>)</rp></ruby> </p>",
      "<ruby>漢字<rp>(</rp><rt>かんじ</rt><rp>)</rp></ruby> ",
    ],
    "span with lang attribute": ['<span lang="ja">日本語</span>', outlined("span[lang]", '<span lang="ja">'), '<span lang="ja">日本語</span> '],
    "span with an empty data attribute": [
      "a <span data-fish>fishy</span> word",
      outlined("span[data-fish]", "<span data-fish>"),
      "a <span data-fish>fishy</span> word",
    ],
    "span with an explicitly empty attribute is written bare": [
      'a <span data-fish="">fishy</span> word',
      outlined("span[data-fish]", "<span data-fish>"),
      "a <span data-fish>fishy</span> word",
    ],
    "span without attributes": ["<span>plain</span>", outlined("span", "<span>"), "<span>plain</span> "],
    "span with a valued data attribute": [
      '<span data-mood="happy">text</span>',
      outlined("span[data-mood]", '<span data-mood="happy">'),
      '<span data-mood="happy">text</span> ',
    ],
    "br tag converts to hard break": ["Line1<br>Line2", "<p>Line1<br>Line2</p>", "Line1\nLine2"],
    "self-closing br tag converts to hard break": ["Line1<br/>Line2", "<p>Line1<br>Line2</p>", "Line1\nLine2"],
    "self-closing br tag with space converts to hard break": ["Line1<br />Line2", "<p>Line1<br>Line2</p>", "Line1\nLine2"],
    "br tag with attributes converts to hard break": ['Line1<br class="x">Line2', "<p>Line1<br>Line2</p>", "Line1\nLine2"],
    "stray closing tag is ignored": ["a</li>b", "<p>ab</p>", "ab"],
    "closing an outer tag closes the inner ones": ["<kbd><sup>x</kbd> y", "<p><kbd><sup>x</sup></kbd> y</p>", "<kbd><sup>x</sup></kbd> y"],
    "anchor without href keeps its name and own title": [
      'see <a name="top" title="Top">here</a>',
      outlined("a[name]", "Top"),
      'see <a name="top" title="Top">here</a> ',
    ],
    "mark tags keep being marks when they carry attributes": [
      '<strong class="x">bold</strong> <code class="lang-js">c</code>',
      "<p><strong>bold</strong> <code>c</code></p>",
      "**bold** `c`",
    ],
    "tags the cook strips, embedded media and block-level tags outside a table are dropped": [
      'a <blink>b</blink> <iframe src="https://example.com"></iframe> <div>c</div> <ul><li>d</li></ul> e',
      "<p>a b  c d e</p>",
      "a b  c d e",
    ],
  });

  // The cases inside tables need the table extension (rich-editor-table.spec.js
  // runs them with it).

  qtest("inline HTML inserted after load is outlined", async function (assert, { helpers }) {
    const { setupRichEditor, settled } = helpers;
    const [editor] = await setupRichEditor(assert, "a");
    const { state, dispatch } = editor.view;
    const span = state.schema.nodes.html_inline.create({ tag: "span" }, state.schema.text("b"));

    dispatch(state.tr.insert(2, span));
    await settled();

    assert.dom(".ProseMirror span").hasClass("composer-html-inline-node");
  });

  qtest("pasted phrasing tags keep allow-listed attributes", async function (assert, { helpers }) {
    const { setupRichEditor, settled } = helpers;
    const [editor] = await setupRichEditor(assert, "");

    editor.view.pasteHTML('x <abbr title="T">a</abbr> <kbd>k</kbd> <blink>b</blink>');
    await settled();

    assert.strictEqual(editor.value, 'x <abbr title="T">a</abbr> <kbd>k</kbd> b');
  });

  test("typing <kbd> makes inline HTML", async ({ page }) => {
    await openEditorPage(page);
    await setupRichEditor(page, "");
    const editor = page.locator("#rich-spec-host .ProseMirror");
    await editor.click();
    await page.keyboard.type("press <kbd>");
    await page.keyboard.type("K");
    await expect(editor.locator("kbd")).toHaveText("K");
  });
});

test.describe("rich editor HTML blocks", () => {
  cases("html-block", {
    "unclosed html block": [
      "<div>Hello World\n\n",
      '<pre class="html-block"><code>&lt;div&gt;Hello World</code></pre>',
      "<div>Hello World\n\n",
    ],
    "html block with attributes": [
      'Hey\n\n<div class="test">Hello World</div>\nYou',
      '<p>Hey</p><pre class="html-block"><code>&lt;div class="test"&gt;Hello World&lt;/div&gt;\nYou</code></pre>',
      'Hey\n\n<div class="test">Hello World</div>\nYou\n\n',
    ],
    "html block with multiple lines": [
      "<div>\n  <p>Hello</p>\n  <p>World</p>\n</div>",
      '<pre class="html-block"><code>&lt;div&gt;\n  &lt;p&gt;Hello&lt;/p&gt;\n  &lt;p&gt;World&lt;/p&gt;\n&lt;/div&gt;</code></pre>',
      "<div>\n  <p>Hello</p>\n  <p>World</p>\n</div>\n\n",
    ],
    "html block multiple times": [
      "<div>1</div>\n\nA\n\n<div>2</div>",
      '<pre class="html-block"><code>&lt;div&gt;1&lt;/div&gt;</code></pre><p>A</p><pre class="html-block"><code>&lt;div&gt;2&lt;/div&gt;</code></pre>',
      "<div>1</div>\n\nA\n\n<div>2</div>\n\n",
    ],
    "html block with formatting that would be escaped outside it": [
      "<div>A **bold** or *italic* text</div>",
      '<pre class="html-block"><code>&lt;div&gt;A **bold** or *italic* text&lt;/div&gt;</code></pre>',
      "<div>A **bold** or *italic* text</div>\n\n",
    ],
  },
  // The site's highlight.js bundle has HTML (xml), which the reference's
  // test bundle lacks: its highlighting is set aside.
  { cleanHtml: (html) => html.replace(/<span class="hljs-[^"]*">|<\/span>/g, "") });

  qtest("Tab and Shift-Tab indent selected HTML block lines", async function (assert, { helpers, pmState }) {
    const { setupRichEditor, triggerKeyEvent } = helpers;
    const { TextSelection } = pmState;
    const [editor] = await setupRichEditor(assert, "<div>\n  <p>Hello</p>\n</div>");
    const { view } = editor;
    const htmlBlock = view.state.doc.firstChild;

    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 1, htmlBlock.content.size + 1)));
    await triggerKeyEvent(".ProseMirror", "keydown", "Tab");

    assert.strictEqual(view.state.doc.firstChild.textContent, "  <div>\n    <p>Hello</p>\n  </div>", "all selected HTML lines are indented");

    await triggerKeyEvent(".ProseMirror", "keydown", "Tab", { shiftKey: true });

    assert.strictEqual(view.state.doc.firstChild.textContent, "<div>\n  <p>Hello</p>\n</div>", "all selected HTML lines are outdented");
  });
});

test.describe("rich editor paste", () => {
  qtest("markdown pasted as text is parsed", async function (assert, { helpers }) {
    const { setupRichEditor, settled } = helpers;
    const [editor] = await setupRichEditor(assert, "");
    const event = new ClipboardEvent("paste", { clipboardData: new DataTransfer(), bubbles: true, cancelable: true });
    event.clipboardData.setData("text/plain", "# Title\n\nsome **bold** and :tada:");
    editor.view.dom.dispatchEvent(event);
    await settled();

    assert.dom(".ProseMirror h1").hasText("Title");
    assert.dom(".ProseMirror strong").hasText("bold");
    assert.dom(".ProseMirror img.emoji").exists();
    assert.strictEqual(editor.value, "# Title\n\nsome **bold** and :tada:");
  });

  qtest("several lines pasted into an empty list item make items", async function (assert, { helpers, pmState }) {
    const { setupRichEditor, settled } = helpers;
    const { TextSelection } = pmState;
    const [editor] = await setupRichEditor(assert, "* one\n* ");
    const { view } = editor;
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, view.state.doc.content.size - 3)));
    const event = new ClipboardEvent("paste", { clipboardData: new DataTransfer(), bubbles: true, cancelable: true });
    event.clipboardData.setData("text/plain", "two\n\nthree");
    view.dom.dispatchEvent(event);
    await settled();

    assert.dom(".ProseMirror ul > li").exists({ count: 3 });
  });

  qtest("Word's list paragraphs pasted become a list", async function (assert, { helpers }) {
    const { setupRichEditor, settled } = helpers;
    const [editor] = await setupRichEditor(assert, "");
    editor.view.pasteHTML(
      '<p class="MsoListParagraphCxSpFirst" style="mso-list:l0 level1 lfo1"><span style="mso-list:Ignore">·</span>One</p>' +
        '<p class="MsoListParagraphCxSpLast" style="mso-list:l0 level1 lfo1"><span style="mso-list:Ignore">·</span>Two</p>'
    );
    await settled();

    assert.strictEqual(editor.value, "* One\n* Two");
  });
});

test.describe("rich editor extension registry", () => {
  // prosemirror-editor-test.gjs's registry cases. "supports registered
  // nodeSpec/parser/serializer" parses [wrap]: it is with wrap's own cases
  // (rich-editor-plugins.spec.js).
  qtest("supports registered markSpec/parser/serializer", async function (assert, { helpers, extensions }) {
    const { testMarkdown } = helpers;
    extensions.registerRichEditorExtension({
      markSpec: {
        hashtag: {
          parseDOM: [{ tag: "span.hashtag-raw" }],
          toDOM: () => ["span", { class: "hashtag-raw" }],
        },
      },
      parse: {
        span_open(state, token, tokens, i) {
          if (token.attrGet("class") === "hashtag-raw") {
            tokens[i + 1].content = tokens[i + 1].content.slice(1);
            state.openMark(state.schema.marks.hashtag.create());
            return true;
          }
        },
        span_close(state) {
          state.closeMark(state.schema.marks.hashtag);
        },
      },
      serializeMark: { hashtag: { open: "#", close: "" } },
    });
    try {
      // the hashtag node's own parse spec comes first and would take it
      const registered = extensions.getExtensions();
      const hashtagNode = registered.find((e) => e.nodeSpec && e.nodeSpec.hashtag);
      extensions.clearRichEditorExtensions();
      registered.filter((e) => e !== hashtagNode).forEach(extensions.registerRichEditorExtension);
      await testMarkdown(
        assert,
        "Hello #tag #test",
        '<p>Hello <span class="hashtag-raw">tag</span> <span class="hashtag-raw">test</span></p>',
        "Hello #tag #test"
      );
    } finally {
      extensions.resetRichEditorExtensions();
    }
  });

  qtest("supports registered nodeViews and plugins with array, object or function", async function (assert, { editor, extensions, pmState }) {
    const state = {};
    const host = document.createElement("div");
    document.body.appendChild(host);
    const created = editor.createEditor({
      container: host,
      engine: window.richSpec.engine,
      extensions: [
        {
          nodeViews: {
            paragraph: class CustomNodeView {
              constructor() {
                this.dom = document.createElement("p");
                this.dom.className = "custom-p";
                state.updated = true;
              }
            },
          },
        },
        { plugins: [{ view: () => ((state.plugin1 = true), {}) }] },
        { plugins: { view: () => ((state.plugin2 = true), {}) } },
        { plugins: ({ pmState: { Plugin } }) => new Plugin({ view: () => ((state.plugin3 = true), {}) }) },
      ].concat(extensions.getExtensions()),
      value: "",
    });

    assert.true(state.updated, "it calls the update method of the custom node view");
    assert.dom(".custom-p").exists("it renders the custom node view for p");
    assert.true(state.plugin1, "plugin1's view fn was called");
    assert.true(state.plugin2, "plugin2's view fn was called");
    assert.true(state.plugin3, "plugin3's view fn was called");
    created.destroy();
    host.remove();
    assert.ok(pmState.Plugin);
  });
});
