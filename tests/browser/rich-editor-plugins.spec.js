// The bundled plugins' rich editor extensions and [wrap] (ROADMAP 196r4),
// with the reference's tests: each plugin's rich-editor-extension-test
// (checklist, details, spoiler-alert, poll, discourse-math,
// discourse-local-dates, discourse-policy), its system specs where the
// QUnit tests leave a behaviour out (footnote, checklist, wrap's
// prosemirror_wrap_spec.rb) and prosemirror-editor-test.gjs's [wrap]
// registry case. Table-driven cases go through testMarkdown; the others
// run their body in the page (rich-editor-helper.js qunitTest), the modal
// service's activeModal read as the reference's tests read it. The editor
// page registers the plugins a test names (the composer's context lists
// the site's); math and policy are off on the test site, so their cases
// turn them on in the engine's render settings.
const { test, expect } = require("./helpers");
const { openEditorPage, testMarkdown, setupRichEditor, editorValue, qunitTest } = require("./rich-editor-helper");

const ALL = ["checklist", "details", "local-dates", "math", "policy", "footnote", "poll", "spoiler"];
const RENDER = { math: true, policy: true };

const qtest = (name, body, opts = {}) =>
  qunitTest(test, name, body, Object.assign({ plugins: ALL, render: RENDER }, opts));

// [name, markdown, html, markdown out], each its own test.
function cases(group, entries, opts = {}) {
  for (const [name, [markdown, html, expected]] of Object.entries(entries)) {
    test(`${group}: ${name}`, async ({ page }) => {
      await openEditorPage(page, { plugins: opts.plugins || ALL, render: opts.render || RENDER });
      await testMarkdown(page, markdown, html, expected, opts.setup || {});
    });
  }
}

const editor = (page) => page.locator("#rich-spec-host .ProseMirror");

test.describe("rich editor details", () => {
  cases("details", {
    details: [
      `[details="Summary"]This text will be hidden[/details]`,
      `<details open="true"><summary>Summary</summary><p>This text will be hidden</p></details>`,
      `[details="Summary"]\nThis text will be hidden\n\n[/details]\n\n`,
    ],
    "details with open attribute": [
      `[details="Summary" open]This text will be hidden[/details]`,
      `<details open="true"><summary>Summary</summary><p>This text will be hidden</p></details>`,
      `[details="Summary"]\nThis text will be hidden\n\n[/details]\n\n`,
    ],
    "details without summary": [
      `[details]This text will be hidden[/details]`,
      `<details open="true"><summary></summary><p>This text will be hidden</p></details>`,
      `[details]\nThis text will be hidden\n\n[/details]\n\n`,
    ],
    "details without summary but with open attribute": [
      `[details open]This text will be hidden[/details]`,
      `<details open="true"><summary></summary><p>This text will be hidden</p></details>`,
      `[details]\nThis text will be hidden\n\n[/details]\n\n`,
    ],
  });

  qtest("opens and closes details on click", async function (assert, { helpers }) {
    const { setupRichEditor, click } = helpers;
    const detailsMarkdown = `[details="Summary"]This text will be hidden[/details]`;
    await setupRichEditor(assert, detailsMarkdown);

    const detailsCss = ".d-editor-input details";
    const summaryCss = `${detailsCss} summary`;
    assert.dom(detailsCss).hasAttribute("open");

    await click(summaryCss);
    assert.dom(detailsCss).doesNotHaveAttribute("open");

    // click elsewhere first to avoid a double-click being detected
    await click(`${detailsCss} p`);
    await click(summaryCss);

    assert.dom(detailsCss).hasAttribute("open");
  });
});

test.describe("rich editor spoiler", () => {
  cases("spoiler", {
    "inline spoiler": [
      "Hey [spoiler]did you know the good guys die[/spoiler] in the end?",
      '<p>Hey <span class="spoiled spoiler-blurred">did you know the good guys die</span> in the end?</p>',
      "Hey [spoiler]did you know the good guys die[/spoiler] in the end?",
    ],
    "block spoiler": [
      "hey\n\n[spoiler]\n> did you know the good guys die\n\n[/spoiler]\n\nin the end?",
      '<p>hey</p><div class="spoiled spoiler-blurred"><blockquote><p>did you know the good guys die</p></blockquote></div><p>in the end?</p>',
      "hey\n\n[spoiler]\n> did you know the good guys die\n\n[/spoiler]\n\nin the end?",
    ],
    "inline spoiler with trailing space": [
      "[spoiler]the answer is 42[/spoiler] ",
      '<p><span class="spoiled spoiler-blurred">the answer is 42</span> </p>',
      "[spoiler]the answer is 42[/spoiler] ",
    ],
  });

  // composer_spoiler_spec.rb's typing, without the toolbar (196r3)
  test("typing [spoiler] makes a spoiler, unblurred while the cursor is in it", async ({ page }) => {
    await openEditorPage(page, { plugins: ALL });
    await setupRichEditor(page, "");
    await editor(page).click();
    await page.keyboard.type("Hey [spoiler]");
    await expect(editor(page).locator("span.spoiled")).toHaveText("This text will be blurred");
    await expect(editor(page).locator("span.spoiled")).not.toHaveClass(/spoiler-blurred/);
    // trailing-inline-space's space after an inline node at the end
    expect(await editorValue(page)).toBe("Hey [spoiler]This text will be blurred[/spoiler] ");
  });
});

test.describe("rich editor wrap", () => {
  qtest(
    "supports registered nodeSpec/parser/serializer",
    async function (assert, { helpers, extensions }) {
      const { testMarkdown } = helpers;
      extensions.clearRichEditorExtensions();
      try {
        // Multiple parsers can be registered for the same node type
        extensions.registerRichEditorExtension({
          parse: { wrap_open() {}, wrap_close() {} },
        });

        extensions.registerRichEditorExtension({
          nodeSpec: {
            marquee: {
              content: "block*",
              group: "block",
              parseDOM: [{ tag: "marquee" }],
              toDOM: () => ["marquee", 0],
            },
          },
          parse: {
            wrap_open(state, token) {
              if (token.attrGet("data-wrap") === "marquee") {
                state.openNode(state.schema.nodes.marquee);
                return true;
              }
            },
            wrap_close(state) {
              if (state.top().type.name === "marquee") {
                state.closeNode();
                return true;
              }
            },
          },
          serializeNode: {
            marquee(state, node) {
              state.write("[wrap=marquee]\n");
              state.renderContent(node);
              state.write("[/wrap]\n\n");
            },
          },
        });

        extensions.registerRichEditorExtension({
          parse: { wrap_open() {}, wrap_close() {} },
        });

        await testMarkdown(
          assert,
          "[wrap=marquee]\nHello\n[wrap=marquee]\nWorld\n[/wrap]\nInner\n[/wrap]\n\nText",
          "<marquee><p>Hello</p><marquee><p>World</p></marquee><p>Inner</p></marquee><p>Text</p>",
          "[wrap=marquee]\nHello\n\n[wrap=marquee]\nWorld\n\n[/wrap]\n\nInner\n\n[/wrap]\n\nText"
        );
      } finally {
        extensions.resetRichEditorExtensions();
      }
    },
    { plugins: [] }
  );

  cases("wrap", {
    "block wrap": [
      "[wrap=toc id=1]\nHello\n[/wrap]",
      '<div class="composer-wrap-node"><div><p>Hello</p></div><button class="d-wrap-indicator btn-flat --block" contenteditable="false" type="button">[wrap=toc]</button></div>',
      "[wrap=toc id=1]\nHello\n\n[/wrap]\n\n",
    ],
    "inline wrap": [
      "Hello [wrap=inline]world[/wrap] again",
      '<p>Hello <span class="composer-wrap-node"><span>world</span><button class="d-wrap-indicator btn-flat --inline" contenteditable="false" type="button">[wrap=inline]</button></span> again</p>',
      "Hello [wrap=inline]world[/wrap] again",
    ],
    "nameless wrap with attributes": [
      "[wrap class=highlight]\nHello\n[/wrap]",
      '<div class="composer-wrap-node"><div><p>Hello</p></div><button class="d-wrap-indicator btn-flat --block" contenteditable="false" type="button">[wrap]</button></div>',
      "[wrap class=highlight]\nHello\n\n[/wrap]\n\n",
    ],
  });

  // prosemirror_wrap_spec.rb
  for (const [typed, expected] of [
    ["[wrap]", "[wrap]\n\n[/wrap]\n\n"],
    ["[wrap=example]", "[wrap=example]\n\n[/wrap]\n\n"],
    ["[wrap class=highlight]", "[wrap class=highlight]\n\n[/wrap]\n\n"],
  ]) {
    test(`typing ${typed} creates a wrap block`, async ({ page }) => {
      await openEditorPage(page);
      await setupRichEditor(page, "");
      await editor(page).click();
      await page.keyboard.type(typed);
      await expect(editor(page).locator(".composer-wrap-node")).toHaveCount(1);
      expect(await editorValue(page)).toBe(expected);
    });
  }

  test("creates inline wrap when not at start of line", async ({ page }) => {
    await openEditorPage(page);
    await setupRichEditor(page, "");
    await editor(page).click();
    await page.keyboard.type("Text before [wrap=inline]");
    await expect(editor(page).locator(".composer-wrap-node")).toHaveCount(1);
    await expect(editor(page)).toContainText("Text before");
    await expect(editor(page)).toContainText("Wrap content");
    expect(await editorValue(page)).toBe("Text before [wrap=inline]Wrap content[/wrap] ");
  });

  test("opens edit modal when clicking wrap indicator", async ({ page }) => {
    await openEditorPage(page);
    await setupRichEditor(page, "", { realModals: true });
    await editor(page).click();
    await page.keyboard.type("[wrap=outer]");
    await expect(editor(page).locator(".composer-wrap-node")).toHaveCount(1);

    await editor(page).locator(".d-wrap-indicator").click();
    const modal = page.locator(".wrap-attributes-modal");
    await expect(modal).toBeVisible();
    await expect(modal.getByLabel("Wrap name")).toHaveValue("outer");

    await modal.getByLabel("Wrap name").fill("updated");
    await modal.getByRole("button", { name: "Add attribute" }).click();
    const row = modal.locator(".wrap-modal__attribute-row");
    await row.getByLabel("Key").fill("class");
    await row.getByLabel("Value").fill("highlight");
    await modal.getByRole("button", { name: "Apply" }).click();

    await expect(editor(page).locator(".composer-wrap-node")).toHaveCount(1);
    expect(await editorValue(page)).toBe("[wrap=updated class=highlight]\n\n[/wrap]\n\n");
  });

  test("Unwrap keeps the content", async ({ page }) => {
    await openEditorPage(page);
    await setupRichEditor(page, "[wrap=outer]\nHello\n[/wrap]", { realModals: true });
    await editor(page).locator(".d-wrap-indicator").click();
    await page.locator(".wrap-attributes-modal__unwrap").click();
    await expect(editor(page).locator(".composer-wrap-node")).toHaveCount(0);
    expect(await editorValue(page)).toBe("Hello");
  });
});

test.describe("rich editor checklist", () => {
  const checked =
    '<span class="chcklst-box checked fa fa-square-check-o" contenteditable="false" draggable="true"></span>';
  const unchecked = '<span class="chcklst-box fa fa-square-o" contenteditable="false" draggable="true"></span>';

  cases("checklist", {
    "renders unchecked checkbox correctly": ["[ ] todo item", `<p>${unchecked} todo item</p>`, "[ ] todo item"],
    "renders checked checkbox correctly": [
      "[x] completed item",
      `<p>${checked} completed item</p>`,
      "[x] completed item",
    ],
    "handles multiple checkboxes in a single paragraph": [
      "[] first task [x] second task",
      `<p>${unchecked} first task ${checked} second task</p>`,
      "[ ] first task [x] second task",
    ],
    "handles checkboxes in lists": [
      "* [ ] unchecked list item\n* [x] checked list item",
      `<ul data-tight="true"><li class="has-checkbox"><p>${unchecked} unchecked list item</p></li><li class="has-checkbox"><p>${checked} checked list item</p></li></ul>`,
      "* [ ] unchecked list item\n* [x] checked list item",
    ],
    "handles checkboxes with formatting": [
      "[ ] *italics* and [x] **bold**",
      `<p>${unchecked} <em>italics</em> and ${checked} <strong>bold</strong></p>`,
      "[ ] *italics* and [x] **bold**",
    ],
    "does not render escaped checkbox": [
      "\\[x] not a checkbox",
      "<p>[x] not a checkbox</p>",
      "\\[x\\] not a checkbox",
    ],
    "handles escaped checkbox followed by real checkbox": [
      "\\[x] escaped [x] real",
      `<p>[x] escaped ${checked} real</p>`,
      "\\[x\\] escaped [x] real",
    ],
    "preserves checkbox state in ordered lists": [
      "1. [ ] unchecked\n2. [x] checked",
      `<ol data-tight="true"><li><p>${unchecked} unchecked</p></li><li><p>${checked} checked</p></li></ol>`,
      "1. [ ] unchecked\n2. [x] checked",
    ],
    "handles nested list with checkboxes": [
      "* [ ] parent\n  * [x] child",
      `<ul data-tight="true"><li class="has-checkbox"><p>${unchecked} parent</p><ul data-tight="true"><li class="has-checkbox"><p>${checked} child</p></li></ul></li></ul>`,
      "* [ ] parent\n  * [x] child",
    ],
  });

  // checklist's rich_editor_extension_spec.rb, without the toolbar's list
  // menu (196r3): typed boxes instead.
  test("toggles checked state when clicking checkbox", async ({ page }) => {
    await openEditorPage(page, { plugins: ALL });
    await setupRichEditor(page, "");
    await editor(page).click();
    await page.keyboard.type("[ ] Item 1");
    await expect(editor(page).locator(".chcklst-box:not(.checked)")).toHaveCount(1);
    await editor(page).locator(".chcklst-box").click();
    await expect(editor(page).locator(".chcklst-box.checked")).toHaveCount(1);
    // the rule's own space, then the one typed
    expect(await editorValue(page)).toBe("[x]  Item 1");
  });

  test("continues a checklist on Enter and leaves it on a second", async ({ page }) => {
    await openEditorPage(page, { plugins: ALL });
    await setupRichEditor(page, "");
    await editor(page).click();
    await page.keyboard.type("* [ ] First item");
    await page.keyboard.press("Enter");
    await expect(editor(page).locator("li .chcklst-box")).toHaveCount(2);
    await page.keyboard.press("Enter");
    await expect(editor(page).locator("li .chcklst-box")).toHaveCount(1);
    await expect(editor(page).locator("li")).toHaveCount(1);
  });

  test("Backspace after a box joins with the item before", async ({ page }) => {
    await openEditorPage(page, { plugins: ALL });
    await setupRichEditor(page, "");
    await editor(page).click();
    await page.keyboard.type("* [ ] First item");
    await page.keyboard.press("Enter");
    await page.keyboard.type("Second item");
    await expect(editor(page).locator("li .chcklst-box")).toHaveCount(2);
    await page.keyboard.press("Home");
    await page.keyboard.press("Backspace");
    await expect(editor(page).locator("li .chcklst-box")).toHaveCount(1);
    await expect(editor(page)).toContainText("First itemSecond item");
  });
});

test.describe("rich editor footnotes", () => {
  // footnote's rich_editor_extension_spec.rb: what its pastes parse to,
  // through the markdown, and its typing.
  cases(
    "footnote",
    {
      "converts inline footnotes": [
        "What is this? ^[multiple inline] ^[footnotes]",
        '<p>What is this? <div class="footnote" contenteditable="false"></div> <div class="footnote" contenteditable="false"></div></p>',
        "What is this? ^[multiple inline] ^[footnotes]",
      ],
      "converts block footnotes": [
        "Hey [^1] [^2]\n[^1]: This is inline\n[^2]: This\n\n    > not so much",
        '<p>Hey <div class="footnote" contenteditable="false"></div> <div class="footnote" contenteditable="false"></div></p>',
        "Hey ^[This is inline] [^1]\n\n[^1]: This\n\n    > not so much",
      ],
    },
    { setup: {} }
  );

  test("converts inline footnotes when typing", async ({ page }) => {
    await openEditorPage(page, { plugins: ALL });
    await setupRichEditor(page, "");
    await editor(page).click();
    await page.keyboard.type("What is this? ^[multiple inline] ^[footnotes]");
    await expect(editor(page).locator("div.footnote")).toHaveCount(2);
    expect(await editorValue(page)).toBe("What is this? ^[multiple inline] ^[footnotes]");
  });

  test("creates an inline footnote from [^n] and focuses inside", async ({ page }) => {
    await openEditorPage(page, { plugins: ALL });
    await setupRichEditor(page, "");
    await editor(page).click();
    await page.keyboard.type("[^1]Hello");
    await expect(editor(page).locator("div.footnote")).toHaveCount(1);
    expect(await editorValue(page)).toBe("^[Hello]");
  });

  qtest("selecting a footnote opens its note to edit", async function (assert, { helpers, pmState }) {
    const { setupRichEditor, settled } = helpers;
    const [editor] = await setupRichEditor(assert, "before ^[note] after");
    const { view } = editor;
    let pos = null;
    view.state.doc.descendants((node, at) => {
      if (node.type.name === "footnote") {
        pos = at;
      }
    });
    view.dispatch(view.state.tr.setSelection(pmState.NodeSelection.create(view.state.doc, pos)));
    await settled();

    assert.dom(".footnote-tooltip .ProseMirror").hasText("note", "the note is in an editor of its own");
  });
});

test.describe("rich editor math", () => {
  qtest("renders inline math and preserves markdown", async function (assert, { helpers }) {
    const { testMarkdown } = helpers;
    const markdown = "Inline $E=mc^2$ math.";

    await testMarkdown(
      assert,
      markdown,
      () => {
        assert.dom("span.composer-math-node").exists();
        assert.dom(".math-node-content").hasText("E=mc^2");
      },
      markdown
    );
  });

  qtest("renders block math and preserves markdown", async function (assert, { helpers }) {
    const { testMarkdown } = helpers;
    const markdown = "$$\nE=mc^2\n$$";
    const expectedMarkdown = "$$\nE=mc^2\n$$\n\n";

    await testMarkdown(
      assert,
      markdown,
      () => {
        assert.dom("div.composer-math-node").exists();
        assert.dom("div.composer-math-node .math-node-content").hasText("E=mc^2");
      },
      expectedMarkdown
    );
  });

  qtest("edits math via modal", async function (assert, { helpers, richSpec }) {
    const { setupRichEditor, click } = helpers;
    const markdown = "Inline $E=mc^2$ math.";
    const [editor] = await setupRichEditor(assert, markdown);

    await click(".math-node-edit-button");

    const modalModel = richSpec.activeModal && richSpec.activeModal.opts.model;
    assert.notStrictEqual(modalModel, undefined, "Opens the math edit modal");

    modalModel.onApply("a^2 + b^2 = c^2");

    assert.strictEqual(editor.value, "Inline $a^2 + b^2 = c^2$ math.", "Markdown updates after editing math");
  });

  qtest(
    "asciimath is inline math of its own type",
    async function (assert, { helpers }) {
      const { testMarkdown } = helpers;
      const markdown = "Ascii %x^2% math";

      await testMarkdown(
        assert,
        markdown,
        () => {
          assert.dom("span.composer-math-node .math-node-content").hasClass("asciimath");
          assert.dom("span.composer-math-node .math-node-content").hasText("x^2");
        },
        markdown
      );
    },
    { render: { math: true, math_asciimath: true } }
  );
});

test.describe("rich editor local dates", () => {
  for (const [name, markdown, checks] of [
    ["local date", "[date=2021-01-01 time=12:00:00]", { date: "2021-01-01", time: "12:00:00" }],
    ["local date without time", "[date=2021-01-01]", { date: "2021-01-01", time: undefined }],
    ["local date with timezone", "[date=2021-01-01 time=12:00:00 timezone=America/New_York]", { timezone: "America/New_York" }],
    ["local date with format", '[date=2021-01-01 time=12:00:00 format="YYYY-MM-DD HH:mm"]', { format: "YYYY-MM-DD HH:mm" }],
    ["local date with recurring", "[date=2021-01-01 time=12:00:00 recurring=1.weeks]", { recurring: "1.weeks" }],
    [
      "local date with timezones",
      "[date=2021-01-01 time=12:00:00 timezones=Europe/Paris|Asia/Tokyo]",
      { timezones: "Europe/Paris|Asia/Tokyo" },
    ],
    ["local date with countdown", "[date=2099-01-01 time=12:00:00 countdown=true]", { countdown: "true" }],
    [
      "local date with displayedTimezone",
      "[date=2021-01-01 time=12:00:00 displayedTimezone=Europe/London]",
      { displayedTimezone: "Europe/London" },
    ],
  ]) {
    test(`local dates: ${name}`, async ({ page }) => {
      await openEditorPage(page, { plugins: ALL });
      const result = await setupRichEditor(page, markdown);
      const dataset = await page.evaluate(() =>
        Object.assign({}, document.querySelector(".ProseMirror .discourse-local-date").dataset)
      );
      for (const [key, value] of Object.entries(checks)) {
        expect(dataset[key], key).toBe(value);
      }
      expect(result.value).toBe(markdown);
    });
  }

  test("local dates: local date with format is drawn in it", async ({ page }) => {
    await openEditorPage(page, { plugins: ALL });
    await setupRichEditor(page, '[date=2021-01-01 time=12:00:00 format="YYYY-MM-DD HH:mm"]');
    await expect(editor(page).locator(".discourse-local-date")).toContainText("2021-01-01");
  });

  const range = (key) =>
    `(() => { const r = document.querySelector(".ProseMirror .discourse-local-date-range"); return [r.querySelector('[data-range="from"]').dataset.${key}, r.querySelector('[data-range="to"]').dataset.${key}]; })()`;
  for (const [name, markdown, key, values] of [
    ["local date range", "[date-range from=2021-01-01 to=2021-01-02]", "date", ["2021-01-01", "2021-01-02"]],
    [
      "local date range with time",
      "[date-range from=2021-01-01T12:00:00 to=2021-01-02T13:00:00]",
      "time",
      ["12:00:00", "13:00:00"],
    ],
    [
      "local date range with timezone",
      "[date-range from=2021-01-01 to=2021-01-02 timezone=America/New_York]",
      "timezone",
      ["America/New_York", "America/New_York"],
    ],
    ["local date range with format", "[date-range from=2021-01-01 to=2021-01-02 format=YYYY-MM-DD]", "format", ["YYYY-MM-DD", "YYYY-MM-DD"]],
    [
      "local date range with timezones",
      "[date-range from=2021-01-01 to=2021-01-02 timezones=Europe/Paris|Asia/Tokyo]",
      "timezones",
      ["Europe/Paris|Asia/Tokyo", "Europe/Paris|Asia/Tokyo"],
    ],
    ["local date range with countdown", "[date-range from=2099-01-01 to=2099-01-02 countdown=true]", "countdown", ["true", "true"]],
    [
      "local date range with displayedTimezone",
      "[date-range from=2021-01-01 to=2021-01-02 displayedTimezone=Europe/London]",
      "displayedTimezone",
      ["Europe/London", "Europe/London"],
    ],
  ]) {
    test(`local dates: ${name}`, async ({ page }) => {
      await openEditorPage(page, { plugins: ALL });
      const result = await setupRichEditor(page, markdown);
      expect(await page.evaluate(range(key))).toEqual(values);
      if (key === "time") {
        expect(await page.evaluate(range("date"))).toEqual(["2021-01-01", "2021-01-02"]);
      }
      expect(result.value).toBe(markdown);
    });
  }

  qtest("edit seeds the modal from the node attributes", async function (assert, { helpers, richSpec }) {
    const { setupRichEditor, click } = helpers;
    await setupRichEditor(
      assert,
      '[date=2021-01-01 time=12:00:00 timezone=America/New_York format="YYYY-MM-DD" recurring=1.weeks timezones=Europe/Paris|Asia/Tokyo countdown=true displayedTimezone=Europe/London]'
    );
    await click(".composer-local-date__edit-button");
    const { model } = richSpec.activeModal.opts;

    assert.deepEqual(model.initialValues, {
      date: "2021-01-01",
      time: "12:00:00",
      toDate: null,
      toTime: null,
      timezone: "America/New_York",
      format: "YYYY-MM-DD",
      timezones: ["Europe/Paris", "Asia/Tokyo"],
      recurring: "1.weeks",
      countdown: "true",
      displayedTimezone: "Europe/London",
    });
  });

  qtest("edit seeds the modal from a range's node attributes", async function (assert, { helpers, richSpec }) {
    const { setupRichEditor, click } = helpers;
    await setupRichEditor(assert, "[date-range from=2021-01-01T12:00:00 to=2021-01-02T13:00:00 timezone=America/New_York]");
    await click(".composer-local-date__edit-button");
    const { model } = richSpec.activeModal.opts;

    assert.deepEqual(model.initialValues, {
      date: "2021-01-01",
      time: "12:00:00",
      toDate: "2021-01-02",
      toTime: "13:00:00",
      timezone: "America/New_York",
      format: null,
      timezones: [],
      recurring: null,
      countdown: null,
      displayedTimezone: null,
    });
  });

  for (const [name, markdown, markup, expected] of [
    [
      "applying an edit replaces the date node",
      "before [date=2021-01-01 time=12:00:00] after",
      "[date=2022-02-02 time=13:00:00]",
      "before [date=2022-02-02 time=13:00:00] after",
    ],
    [
      "applying an edit can turn a date into a range",
      "[date=2021-01-01 time=12:00:00]",
      "[date-range from=2021-01-01T12:00:00 to=2021-01-02T13:00:00]",
      "[date-range from=2021-01-01T12:00:00 to=2021-01-02T13:00:00]",
    ],
    [
      "applying an edit can turn a range into a date",
      "[date-range from=2021-01-01T12:00:00 to=2021-01-02T13:00:00]",
      "[date=2021-01-01 time=12:00:00]",
      "[date=2021-01-01 time=12:00:00]",
    ],
  ]) {
    qtest(
      name,
      async function (assert, { helpers, richSpec }, args) {
        const { setupRichEditor, click, settled } = helpers;
        const [editor] = await setupRichEditor(assert, args.markdown);
        await click(".composer-local-date__edit-button");
        richSpec.activeModal.opts.model.insertDate(args.markup);
        await settled();

        assert.strictEqual(editor.value, args.expected);
      },
      { args: { markdown, markup, expected } }
    );
  }
});

test.describe("rich editor policy", () => {
  qtest("policy with multiple groups", async function (assert, { helpers }) {
    const markdown = `[policy groups="staff,admins" version="2"]\nI accept the terms\n\n[/policy]\n\n`;
    const [{ value }] = await helpers.setupRichEditor(assert, markdown);

    assert.dom(".ProseMirror .policy").hasAttribute("data-groups", "staff,admins");
    assert.dom(".ProseMirror .policy").hasAttribute("data-version", "2");
    assert.dom(".ProseMirror .policy p").hasText("I accept the terms");

    assert.strictEqual(value, markdown);
  });

  qtest("policy with all attributes", async function (assert, { helpers }) {
    const markdown = `[policy group="staff" version="1" accept="true" revoke="false" reminder="daily" renew="30" renew-start="2023-01-01" add-users-to-group="members" private="true"]\nComplex policy\n\n[/policy]\n\n`;
    const [{ value }] = await helpers.setupRichEditor(assert, markdown);

    const policy = ".ProseMirror .policy";
    assert.dom(policy).hasAttribute("data-group", "staff");
    assert.dom(policy).hasAttribute("data-version", "1");
    assert.dom(policy).hasAttribute("data-accept", "true");
    assert.dom(policy).hasAttribute("data-revoke", "false");
    assert.dom(policy).hasAttribute("data-reminder", "daily");
    assert.dom(policy).hasAttribute("data-renew", "30");
    assert.dom(policy).hasAttribute("data-renew-start", "2023-01-01");
    assert.dom(policy).hasAttribute("data-add-users-to-group", "members");
    assert.dom(policy).hasAttribute("data-private", "true");
    assert.dom(`${policy} .policy-node-edit-button`).exists();
    assert.dom(`${policy} .policy-attrs dt`).exists({ count: 9 });
    assert.dom(`${policy} .policy-attrs dd`).exists({ count: 9 });
    assert.dom(`${policy} p`).hasText("Complex policy");

    assert.strictEqual(value, markdown);
  });

  qtest("policy with content around", async function (assert, { helpers }) {
    const markdown = `Hello world\n\n[policy group="everyone" version="1"]\nI accept\n\n[/policy]\n\nGoodbye world`;
    const [{ value }] = await helpers.setupRichEditor(assert, markdown);

    assert.dom(".ProseMirror .policy").hasAttribute("data-group", "everyone");
    assert.dom(".ProseMirror .policy p").hasText("I accept");

    assert.strictEqual(value, markdown);
  });

  qtest("empty policy", async function (assert, { helpers }) {
    const markdown = `[policy group="staff" version="1"]\n[/policy]\n\n`;
    const [{ value }] = await helpers.setupRichEditor(assert, markdown);

    assert.dom(".ProseMirror .policy").hasAttribute("data-group", "staff");
    assert.dom(".ProseMirror .policy").hasAttribute("data-version", "1");
    assert.dom(".ProseMirror .policy p").doesNotExist();

    assert.strictEqual(value, markdown);
  });

  qtest("policy with multiline content", async function (assert, { helpers }) {
    const markdown = `[policy group="staff" version="1"]\nFirst line\n\nSecond paragraph\n\n[/policy]\n\n`;
    const [{ value }] = await helpers.setupRichEditor(assert, markdown);

    assert.dom(".ProseMirror .policy p").exists({ count: 2 });
    assert.dom(".ProseMirror .policy p:first-child").hasText("First line");
    assert.dom(".ProseMirror .policy p:last-child").hasText("Second paragraph");

    assert.strictEqual(value, markdown);
  });

  qtest("policy filters out empty attributes", async function (assert, { helpers }) {
    const inputMarkdown = `[policy group="staff" version="1" accept="" revoke=""]\nTest\n[/policy]`;
    const expectedMarkdown = `[policy group="staff" version="1"]\nTest\n\n[/policy]\n\n`;
    const [{ value }] = await helpers.setupRichEditor(assert, inputMarkdown);

    assert.dom(".ProseMirror .policy").hasAttribute("data-group", "staff");
    assert.dom(".ProseMirror .policy").hasAttribute("data-version", "1");
    assert.dom(".ProseMirror .policy").doesNotHaveAttribute("data-accept");
    assert.dom(".ProseMirror .policy").doesNotHaveAttribute("data-revoke");
    assert.dom(".ProseMirror .policy p").hasText("Test");

    assert.strictEqual(value, expectedMarkdown);
  });

  qtest("policy renders basic structure in rich editor", async function (assert, { helpers }) {
    const markdown = `[policy group="staff" version="1"]\nI accept this policy\n\n[/policy]\n\n`;
    const [{ value }] = await helpers.setupRichEditor(assert, markdown);

    assert.dom(".ProseMirror .policy").exists();
    assert.dom(".ProseMirror .policy").hasClass("policy");
    assert.dom(".ProseMirror .policy").hasAttribute("data-group", "staff");
    assert.dom(".ProseMirror .policy").hasAttribute("data-version", "1");

    // Check that content is preserved
    assert.dom(".ProseMirror .policy p").hasText("I accept this policy");

    assert.strictEqual(value, markdown);
  });

  qtest("policy handles empty content", async function (assert, { helpers }) {
    const markdown = `[policy groups="admins" version="2"]\n[/policy]\n\n`;
    const [{ value }] = await helpers.setupRichEditor(assert, markdown);

    assert.dom(".ProseMirror .policy").exists();
    assert.dom(".ProseMirror .policy").hasAttribute("data-groups", "admins");
    assert.dom(".ProseMirror .policy").hasAttribute("data-version", "2");

    assert.strictEqual(value, markdown);
  });

  qtest("editing the policy writes the builder's settings back", async function (assert, { helpers, richSpec }) {
    const { setupRichEditor, click } = helpers;
    const [editor] = await setupRichEditor(assert, `[policy group="staff" version="1"]\nI accept\n\n[/policy]\n\n`);
    await click(".policy-node-edit-button");
    const { model } = richSpec.activeModal.opts;
    assert.deepEqual(model.policy, { version: "1", groups: "staff" }, "the builder opens on the policy's settings");

    model.onApply({ groups: "admins", version: "2", renew: "", renewStart: "", reminder: null, accept: "", revoke: "", addUsersToGroup: "", private: true });

    assert.strictEqual(editor.value, `[policy groups="admins" version="2" private="true"]\nI accept\n\n[/policy]\n\n`);
  });
});

test.describe("rich editor poll", () => {
  for (const [name, [markdown, attrs, expectedMarkdown]] of Object.entries({
    "regular poll": ["[poll]\n* Option 1\n* Option 2\n[/poll]\n\n", {}, "[poll]\n* Option 1\n* Option 2\n\n[/poll]\n\n"],
    "multiple choice poll": [
      "[poll type=multiple min=1 max=2]\n* Option 1\n* Option 2\n* Option 3\n[/poll]",
      { type: "multiple", max: "2", min: "1" },
      "[poll type=multiple max=2 min=1]\n* Option 1\n* Option 2\n* Option 3\n\n[/poll]\n\n",
    ],
    "public poll": [
      "[poll public=true]\n* Option 1\n* Option 2\n[/poll]",
      { public: "true" },
      "[poll public=true]\n* Option 1\n* Option 2\n\n[/poll]\n\n",
    ],
    "poll with name, results, close date, groups": [
      "[poll name=PollName chartType=pie results=always anonymous=true close=2021-01-01 groups=group1,group2]\n* Option 1\n* Option 2\n[/poll]",
      {
        results: "always",
        name: "PollName",
        charttype: "pie",
        close: "2021-01-01",
        groups: "group1,group2",
      },
      "[poll results=always name=PollName chartType=pie close=2021-01-01 groups=group1,group2]\n* Option 1\n* Option 2\n\n[/poll]\n\n",
    ],
    "poll with bar chart type": [
      "[poll chartType=bar]\n* Option 1\n* Option 2\n[/poll]",
      { charttype: "bar" },
      "[poll chartType=bar]\n* Option 1\n* Option 2\n\n[/poll]\n\n",
    ],
  })) {
    qtest(
      `poll: ${name}`,
      async function (assert, { helpers }, args) {
        const [editor] = await helpers.setupRichEditor(assert, args.markdown);

        assert
          .dom(".composer-poll-node__content li")
          .exists({ count: args.attrs.type === "multiple" ? 3 : 2 }, "renders the options");
        assert.dom(".composer-poll-node__content li:first-child").hasText("Option 1", "renders option text");
        assert.dom(".composer-poll-node__info .poll-info_counts").hasText("0 voters", "renders the voter count");
        assert.dom(".composer-poll-node__edit").hasText("Edit poll", "offers poll editing");
        for (const [attr, value] of Object.entries(args.attrs)) {
          assert.dom(".poll").hasAttribute(`data-poll-${attr}`, value, `preserves ${attr}`);
        }
        assert.strictEqual(editor.value, args.expectedMarkdown, "serializes the poll without its edit control");
      },
      { args: { markdown, attrs, expectedMarkdown } }
    );
  }

  qtest("numeric polls preserve their range without serializing generated options", async function (assert, { helpers }) {
    const markdown = "[poll type=number max=10 min=0 step=2]\n[/poll]\n\n";
    const [editor] = await helpers.setupRichEditor(assert, markdown, { multiToggle: true });

    assert.dom(".composer-poll-node__content li").exists({ count: 6 }, "generates all six numeric options");
    assert.dom(".composer-poll-node__content li:first-child").hasText("0", "includes zero as the lower bound");
    assert.dom(".composer-poll-node__content li:last-child").hasText("10", "includes the upper bound");
    assert.strictEqual(editor.value, markdown, "preserves the range and step across editor toggles");
  });

  qtest("numeric polls keep an authored title", async function (assert, { helpers }) {
    const markdown = "[poll type=number max=2 min=1]\n# How many?\n\n[/poll]\n\n";
    const [editor] = await helpers.setupRichEditor(assert, markdown);

    assert.dom(".composer-poll-node__content .poll-title").hasText("How many?", "renders the title");
    assert.dom(".composer-poll-node__content li").exists({ count: 2 }, "generates the options");
    assert.strictEqual(editor.value, markdown, "keeps the title and drops the generated options");
  });

  qtest("the poll summary shows settings and a zero voter count", async function (assert, { helpers }) {
    await helpers.setupRichEditor(
      assert,
      "[poll type=multiple min=1 max=2 results=on_vote close=2050-01-01 dynamic=true]\n* Option 1\n* Option 2\n\n[/poll]\n\n"
    );

    assert.dom(".composer-poll-node__info .poll-info_counts").hasText("0 voters", "shows zero voters");
    assert.dom(".composer-poll-node__info .multiple-help-text").exists("shows the selection limit");
    assert.dom(".composer-poll-node__info .is-dynamic").exists("shows that options can change after posting");
    assert.dom(".composer-poll-node__info li[title]").exists("shows the close date");
    assert
      .dom(".composer-poll-node__info .poll-info_instructions li")
      .exists({ count: 3 }, "shows the dynamic-options, selection-limit, and close-date instructions");
    assert.dom(".poll").hasAttribute("data-poll-results", "on_vote", "preserves the results visibility attribute");
    assert.dom(".composer-poll-node__info .poll-info_counts-count").exists({ count: 1 }, "renders only the voter-count element");
  });

  qtest("a multiple choice poll without bounds still phrases its hint", async function (assert, { helpers }) {
    await helpers.setupRichEditor(assert, "[poll type=multiple]\n* Option 1\n* Option 2\n\n[/poll]\n\n");

    // hasAnyText
    assert.ok(
      document.querySelector(".composer-poll-node__info .multiple-help-text").textContent.trim(),
      "no line is left as a bare icon"
    );
  });

  qtest("a poll that has already closed does not say it will close", async function (assert, { helpers }) {
    await helpers.setupRichEditor(assert, "[poll close=2021-01-01]\n* Option 1\n* Option 2\n\n[/poll]\n\n");

    assert.dom(".composer-poll-node__info .d-icon-lock").exists("shows the close date as past");
    assert.dom(".composer-poll-node__info .d-icon-far-clock").doesNotExist("rather than as a countdown");
  });

  qtest("a closed multiple choice poll claims no totals", async function (assert, { helpers }) {
    await helpers.setupRichEditor(
      assert,
      "[poll type=multiple min=1 max=2 status=closed]\n* Option 1\n* Option 2\n\n[/poll]\n\n"
    );

    assert.dom(".composer-poll-node__info .poll-info_counts-count").exists({ count: 1 }, "no total votes block");
  });

  qtest("a line break in a title survives as markup", async function (assert, { helpers }) {
    const [editor] = await helpers.setupRichEditor(assert, "[poll]\n# First<br>Second\n* Option 1\n[/poll]");

    assert.dom(".composer-poll-node__content .poll-title br").exists("keeps the break in the document");
    assert.strictEqual(
      editor.value,
      "[poll]\n# First<br>Second\n\n* Option 1\n\n[/poll]\n\n",
      "and writes it as markup, which would otherwise end the title"
    );
  });

  qtest("a title's line break does not leak into the rest of the document", async function (assert, { helpers }) {
    const [editor] = await helpers.setupRichEditor(assert, "[poll]\n# A<br>B\n* Option 1\n[/poll]\n\nOne\nTwo");

    assert.strictEqual(
      editor.value,
      "[poll]\n# A<br>B\n\n* Option 1\n\n[/poll]\n\nOne\nTwo",
      "a break outside the title is still written as a newline"
    );
  });

  qtest("a title keeps markdown that would start a block", async function (assert, { helpers }) {
    const [editor] = await helpers.setupRichEditor(assert, "[poll]\n# - pick one\n* Option 1\n[/poll]");

    assert.dom(".composer-poll-node__content .poll-title").hasText("- pick one", "renders the title as written");
    assert.strictEqual(
      editor.value,
      "[poll]\n# - pick one\n\n* Option 1\n\n[/poll]\n\n",
      "and does not escape it on the way back"
    );
  });

  qtest("an untitled poll carries an empty title to type into", async function (assert, { helpers }) {
    const markdown = "[poll]\n* Option 1\n* Option 2\n\n[/poll]\n\n";
    const [editor] = await helpers.setupRichEditor(assert, markdown);

    assert
      .dom(".composer-poll-node__content > .poll-title")
      .exists("the title node is always present")
      .hasText("", "and is empty when the poll has no title");
    assert.strictEqual(editor.value, markdown, "an empty title is left out of the markdown");
  });

  qtest("closed polls preserve status and option order", async function (assert, { helpers }) {
    const markdown = "[poll status=closed order=asc]\n* Option 1\n* Option 2\n\n[/poll]\n\n";
    const [editor] = await helpers.setupRichEditor(assert, markdown);

    assert.dom(".poll").hasAttribute("data-poll-status", "closed", "keeps the poll closed");
    assert.strictEqual(editor.value, markdown, "preserves attributes not editable in the builder");
  });

  // Unit | Lib | poll rich editor extension
  qtest("carries every attribute the markdown rule allows", async function (assert) {
    const { default: poll } = await import("discourse/rich-editor/plugins/poll");
    // the poll rule's ALLOWED_ATTRIBUTES (crates/markdown render/poll.rs)
    const ALLOWED_ATTRIBUTES = [
      "chartType",
      "close",
      "groups",
      "max",
      "min",
      "name",
      "order",
      "public",
      "results",
      "status",
      "step",
      "type",
      "dynamic",
    ];
    assert.deepEqual(
      Object.keys(poll.nodeSpec.poll.attrs).sort(),
      [...ALLOWED_ATTRIBUTES].sort(),
      "the poll node keeps up with the markdown rule"
    );
  });

  qtest("reads the settings off cooked HTML", async function (assert) {
    const { default: poll } = await import("discourse/rich-editor/plugins/poll");
    const { parseDOM } = poll.nodeSpec.poll;
    const container = document.createElement("div");
    container.innerHTML =
      '<div class="poll" data-poll-status="open" data-poll-name="poll" ' +
      'data-poll-results="always" data-poll-charttype="bar" ' +
      'data-poll-type="number" data-poll-order="asc" data-poll-step="2"></div>';

    const parsed = parseDOM[0].getAttrs(container.firstChild);

    assert.deepEqual(
      Object.fromEntries(Object.entries(parsed).filter(([, value]) => value !== null)),
      {
        type: "number",
        results: "always",
        chartType: "bar",
        order: "asc",
        step: "2",
      },
      "keeps the settings and drops the ones cooking always supplies"
    );
  });

  qtest("Edit poll opens the builder over the poll and writes its settings", async function (assert, { helpers, richSpec }) {
    const { setupRichEditor, click } = helpers;
    const [editor] = await setupRichEditor(assert, "[poll type=multiple]\n* Option 1\n* Option 2\n\n[/poll]\n\n");
    await click(".composer-poll-node__edit");
    const { model } = richSpec.activeModal.opts;
    assert.strictEqual(richSpec.activeModal.name, "poll-ui-builder");
    assert.strictEqual(model.poll.type, "multiple");
    assert.strictEqual(model.poll.optionCount, 2);

    model.onSave(Object.assign({}, editor.view.state.doc.firstChild.attrs, { public: "true", results: "on_vote" }));

    assert.strictEqual(editor.value, "[poll type=multiple results=on_vote public=true]\n* Option 1\n* Option 2\n\n[/poll]\n\n");
  });

  qtest("saving a number poll generates its options again", async function (assert, { helpers, richSpec }) {
    const { setupRichEditor, click } = helpers;
    const [editor] = await setupRichEditor(assert, "[poll type=number min=1 max=3]\n[/poll]\n\n");
    await click(".composer-poll-node__edit");
    const { model } = richSpec.activeModal.opts;

    model.onSave(Object.assign({}, editor.view.state.doc.firstChild.attrs, { max: "5" }));

    assert.dom(".composer-poll-node__content li").exists({ count: 5 });
    assert.strictEqual(editor.value, "[poll type=number max=5 min=1]\n[/poll]\n\n");
  });
});

test.describe("rich editor refusals", () => {
  // Calendar and events are not ported (ROADMAP 202): the markdown crate
  // refuses an [event] rather than guessing, and so does the editor.
  test("an [event] is refused", async ({ page }) => {
    await openEditorPage(page, { plugins: ALL });
    const error = await page.evaluate(() => {
      try {
        window.richSpec.engine.tree('[event start="2030-01-01 10:00"]\n[/event]');
        return null;
      } catch (e) {
        return e.message;
      }
    });
    expect(error).toMatch(/^not ported yet/);
  });

  // [excerpt] has no extension in the reference either: its wrap_bbcode
  // token is refused where spoiler and policy both parse wrap_bbcode (the
  // last of several handlers declining throws). With only one of them on,
  // its handler declining drops the token, as the reference's parser does.
  test("an [excerpt] is refused as the reference refuses it", async ({ page }) => {
    await openEditorPage(page, { plugins: ALL, render: RENDER });
    await setupRichEditor(page, "[excerpt]\nx\n[/excerpt]");
    expect(await page.evaluate(() => window.richSpec.unsupported && window.richSpec.unsupported.name)).toBe(
      "UnsupportedTokenError"
    );
  });

  // discourse-graphviz's extension is a preview node (preview-source.js,
  // 196r3's): its graph is written as raw HTML, which no parse spec takes.
  test("a [graphviz] is refused until the preview node is ported", async ({ page }) => {
    await openEditorPage(page, { plugins: ALL, render: { graphviz: true } });
    await setupRichEditor(page, "[graphviz]\ndigraph { a -> b }\n[/graphviz]");
    expect(await page.evaluate(() => window.richSpec.unsupported && window.richSpec.unsupported.name)).toBe(
      "UnsupportedTokenError"
    );
  });

  // chat's transcript is raw HTML too: chat's extension is not ported.
  test("a [chat] transcript is refused until chat's extension is ported", async ({ page }) => {
    await openEditorPage(page, { plugins: ALL, render: { chat_transcript: true } });
    await setupRichEditor(page, '[chat quote="user2;1;2026-10-01T00:00:00Z" channel="General" channelId="2"]\nhi\n[/chat]');
    expect(await page.evaluate(() => window.richSpec.unsupported && window.richSpec.unsupported.name)).toBe(
      "UnsupportedTokenError"
    );
  });
});
