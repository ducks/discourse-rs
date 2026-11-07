// The rich editor's core (ROADMAP 196r1), its modules loaded from the
// server's /assets/ through the import map static/js/rich-editor.js writes,
// on a page of the server's (anonymous, no composer needed).
//
// The cases are the reference's: frontend/discourse/tests/integration/
// components/prosemirror-editor/prosemirror-markdown-test.js (the editor
// with prosemirror-markdown's defaults alone, clearRichEditorExtensions),
// strikethrough-test.js and underline-test.js (with the extensions), each
// [markdown, the editor's HTML, the markdown it writes back], checked as
// tests/helpers/rich-editor-helper.gjs testMarkdown does: the HTML less
// ProseMirror's own artifacts, and the markdown serialized by an edit.
// Some of tests/unit/lib/to-markdown-test.js's cases follow, those the
// core extensions cover.
//
// Three ways in:
// - "serializes": the document built from the expected HTML by the
//   schema's parseDOM rules, edited in as the member would paste it. Runs
//   now: schema, view and serializer.
// - "maps": the document from a node tree written here as the markdown
//   crate's export is to write it (docs/rich-editor.md). Runs now: the
//   parser's mapping layer (rich-editor/parser.js tokensFromTree).
// - "round-trips": markdown through the export itself (markdown.wasm's
//   rich_editor_tree), skipped until a build has it (196r1 phase B).
const { test, expect, asset } = require("./helpers");

// The node tree, as markdown-it's token stream nests (docs/rich-editor.md).
const t = (content) => ({ type: "text", content });
const inline = (...children) => ({ type: "inline", children });
const p = (...children) => ({ type: "paragraph", tag: "p", children: [inline(...children)] });
// A tight list item's paragraph, which markdown-it hides.
const ph = (...children) => ({ type: "paragraph", tag: "p", hidden: true, children: [inline(...children)] });
const softbreak = { type: "softbreak", tag: "br" };
const bq = (...children) => ({ type: "blockquote", tag: "blockquote", markup: ">", children });
const hr = (markup) => ({ type: "hr", tag: "hr", markup });
const h = (level, ...children) => ({
  type: "heading",
  tag: "h" + level,
  markup: "#".repeat(level),
  children: [inline(...children)],
});
const fence = (info, content) => ({ type: "fence", tag: "code", markup: "```", info, content });
const li = (markup, ...children) => ({ type: "list_item", tag: "li", markup, children });
const ul = (markup, ...items) => ({ type: "bullet_list", tag: "ul", markup, children: items });
const ol = (start, ...items) => ({
  type: "ordered_list",
  tag: "ol",
  markup: ".",
  ...(start === 1 ? {} : { attrs: [["start", String(start)]] }),
  children: items,
});
const img = (src, alt, title) => ({
  type: "image",
  tag: "img",
  nesting: 0,
  attrs: [["src", src], ["alt", ""], ...(title ? [["title", title]] : [])],
  content: alt,
  children: [t(alt)],
});
const em = (markup, ...children) => ({ type: "em", tag: "em", markup, children });
const strong = (markup, ...children) => ({ type: "strong", tag: "strong", markup, children });
const s = (...children) => ({ type: "s", tag: "s", markup: "~~", children });
const link = (href, title, ...children) => ({
  type: "link",
  tag: "a",
  attrs: [["href", href], ...(title ? [["title", title]] : [])],
  children,
});
const code = (content) => ({ type: "code_inline", tag: "code", markup: "`", content });
const bbcode = (name, ...children) => ({
  type: "bbcode_" + name,
  tag: "span",
  attrs: [["class", "bbcode-" + name]],
  markup: "[" + name + "]",
  children,
});
const doc = (...children) => ({ type: "doc", children });

const HR = '<div contenteditable="false" draggable="true"><hr></div>';

// prosemirror-markdown-test.js: [markdown, tree, html, markdown out]
const DEFAULTS = {
  "paragraphs and hard breaks": [
    ["Hello", doc(p(t("Hello"))), "<p>Hello</p>", "Hello"],
    ["Hello\nWorld", doc(p(t("Hello"), softbreak, t("World"))), "<p>Hello<br>World</p>", "Hello\nWorld"],
    ["Hello\n\nWorld", doc(p(t("Hello")), p(t("World"))), "<p>Hello</p><p>World</p>", "Hello\n\nWorld"],
  ],
  blockquotes: [
    ["> Hello", doc(bq(p(t("Hello")))), "<blockquote><p>Hello</p></blockquote>", "> Hello"],
    [
      "> Hello\n> World",
      doc(bq(p(t("Hello"), softbreak, t("World")))),
      "<blockquote><p>Hello<br>World</p></blockquote>",
      "> Hello\n> World",
    ],
    [
      "> Hello\n\n> World",
      doc(bq(p(t("Hello"))), bq(p(t("World")))),
      "<blockquote><p>Hello</p></blockquote><blockquote><p>World</p></blockquote>",
      "> Hello\n\n> World",
    ],
  ],
  "horizontal rule": [
    ["Hey\n\n---", doc(p(t("Hey")), hr("---")), "<p>Hey</p>" + HR, "Hey\n\n---"],
    ["***", doc(hr("***")), HR, "---"],
  ],
  "heading (level 1-6)": [
    ["# Hello", doc(h(1, t("Hello"))), "<h1>Hello</h1>", "# Hello"],
    ["# Hello\nWorld", doc(h(1, t("Hello")), p(t("World"))), "<h1>Hello</h1><p>World</p>", "# Hello\n\nWorld"],
    ["## Hello", doc(h(2, t("Hello"))), "<h2>Hello</h2>", "## Hello"],
    ["### Hello", doc(h(3, t("Hello"))), "<h3>Hello</h3>", "### Hello"],
    ["#### Hello", doc(h(4, t("Hello"))), "<h4>Hello</h4>", "#### Hello"],
    ["##### Hello", doc(h(5, t("Hello"))), "<h5>Hello</h5>", "##### Hello"],
    ["###### Hello", doc(h(6, t("Hello"))), "<h6>Hello</h6>", "###### Hello"],
  ],
  "code block": [
    ["```\nHello\n```", doc(fence("", "Hello\n")), "<pre><code>Hello</code></pre>", "```\nHello\n```"],
    [
      "```\nHello\nWorld\n```",
      doc(fence("", "Hello\nWorld\n")),
      "<pre><code>Hello\nWorld</code></pre>",
      "```\nHello\nWorld\n```",
    ],
    [
      "```\nHello\n\nWorld\n```",
      doc(fence("", "Hello\n\nWorld\n")),
      "<pre><code>Hello\n\nWorld</code></pre>",
      "```\nHello\n\nWorld\n```",
    ],
    [
      "```ruby\nHello\n```\n\nWorld",
      doc(fence("ruby", "Hello\n"), p(t("World"))),
      '<pre data-params="ruby"><code>Hello</code></pre><p>World</p>',
      "```ruby\nHello\n```\n\nWorld",
    ],
  ],
  "ordered lists": [
    ["1. Hello", doc(ol(1, li(".", ph(t("Hello"))))), `<ol data-tight="true"><li><p>Hello</p></li></ol>`, "1. Hello"],
    [
      "1. Hello\n2. World",
      doc(ol(1, li(".", ph(t("Hello"))), li(".", ph(t("World"))))),
      `<ol data-tight="true"><li><p>Hello</p></li><li><p>World</p></li></ol>`,
      "1. Hello\n2. World",
    ],
    [
      "5. Hello\n\n6. World",
      doc(ol(5, li(".", p(t("Hello"))), li(".", p(t("World"))))),
      `<ol start="5"><li><p>Hello</p></li><li><p>World</p></li></ol>`,
      "5. Hello\n\n6. World",
    ],
  ],
  "bullet lists": [
    ["* Hello", doc(ul("*", li("*", ph(t("Hello"))))), '<ul data-tight="true"><li><p>Hello</p></li></ul>', "* Hello"],
    [
      "* Hello\n* World",
      doc(ul("*", li("*", ph(t("Hello"))), li("*", ph(t("World"))))),
      '<ul data-tight="true"><li><p>Hello</p></li><li><p>World</p></li></ul>',
      "* Hello\n* World",
    ],
    [
      "* Hello\n\n* World",
      doc(ul("*", li("*", p(t("Hello"))), li("*", p(t("World"))))),
      "<ul><li><p>Hello</p></li><li><p>World</p></li></ul>",
      "* Hello\n\n* World",
    ],
  ],
  images: [
    [
      "![alt](src)",
      doc(p(img("src", "alt"))),
      '<p><img src="src" alt="alt" contenteditable="false" draggable="true"></p>',
      "![alt](src)",
    ],
    [
      '![alt](src "title")',
      doc(p(img("src", "alt", "title"))),
      '<p><img src="src" alt="alt" title="title" contenteditable="false" draggable="true"></p>',
      '![alt](src "title")',
    ],
  ],
  em: [
    ["*Hello*", doc(p(em("*", t("Hello")))), "<p><em>Hello</em></p>", "*Hello*"],
    ["_Hello_", doc(p(em("_", t("Hello")))), "<p><em>Hello</em></p>", "*Hello*"],
  ],
  strong: [
    ["**Hello**", doc(p(strong("**", t("Hello")))), "<p><strong>Hello</strong></p>", "**Hello**"],
    ["__Hello__", doc(p(strong("__", t("Hello")))), "<p><strong>Hello</strong></p>", "**Hello**"],
  ],
  link: [
    ["[text](href)", doc(p(link("href", null, t("text")))), '<p><a href="href">text</a></p>', "[text](href)"],
    [
      '[text](href "title")',
      doc(p(link("href", "title", t("text")))),
      '<p><a href="href" title="title">text</a></p>',
      '[text](href "title")',
    ],
  ],
  code: [
    [
      "Hel`lo wo`rld",
      doc(p(t("Hel"), code("lo wo"), t("rld"))),
      "<p>Hel<code>lo wo</code>rld</p>",
      "Hel`lo wo`rld",
    ],
  ],
  "all marks": [
    [
      "___[`Hello`](https://example.com)___",
      doc(p(em("_", strong("__", link("https://example.com", null, code("Hello")))))),
      '<p><em><strong><a href="https://example.com"><code>Hello</code></a></strong></em></p>',
      "***[`Hello`](https://example.com)***",
    ],
  ],
  // The typographer is off in the rich editor whatever
  // enable_markdown_typographer says: the document mirrors the source.
  "typographer disabled": [
    ["hello -- world", doc(p(t("hello -- world"))), "<p>hello -- world</p>", "hello -- world"],
    ["hello --- world", doc(p(t("hello --- world"))), "<p>hello --- world</p>", "hello --- world"],
    ["hello... world", doc(p(t("hello... world"))), "<p>hello... world</p>", "hello... world"],
    ["+-5", doc(p(t("+-5"))), "<p>+-5</p>", "+-5"],
    ["(tm)", doc(p(t("(tm)"))), "<p>(tm)</p>", "(tm)"],
    ["(pa)", doc(p(t("(pa)"))), "<p>(pa)</p>", "(pa)"],
    ['"hello" world', doc(p(t('"hello" world'))), '<p>"hello" world</p>', '"hello" world'],
    ["it's here", doc(p(t("it's here"))), "<p>it's here</p>", "it's here"],
    ["`hello -- world`", doc(p(code("hello -- world"))), "<p><code>hello -- world</code></p>", "`hello -- world`"],
  ],
};

// strikethrough-test.js and underline-test.js, with the extensions.
const EXTENSIONS = {
  strikethrough: [
    ["[s]Hello[/s]", doc(p(bbcode("s", t("Hello")))), "<p><s>Hello</s></p>", "~~Hello~~"],
    ["~~Hello~~", doc(p(s(t("Hello")))), "<p><s>Hello</s></p>", "~~Hello~~"],
    [
      "Hey [s]wrod[/s] ~~uord~~ World",
      doc(p(t("Hey "), bbcode("s", t("wrod")), t(" "), s(t("uord")), t(" World"))),
      "<p>Hey <s>wrod</s> <s>uord</s> World</p>",
      "Hey ~~wrod~~ ~~uord~~ World",
    ],
  ],
  "strikethrough with other marks": [
    [
      "___[s][`Hello`](https://example.com)[/s]___",
      doc(p(em("_", strong("__", bbcode("s", link("https://example.com", null, code("Hello"))))))),
      '<p><em><strong><s><a href="https://example.com"><code>Hello</code></a></s></strong></em></p>',
      "***~~[`Hello`](https://example.com)~~***",
    ],
  ],
  underline: [
    ["[u]Hello[/u]", doc(p(bbcode("u", t("Hello")))), "<p><u>Hello</u></p>", "[u]Hello[/u]"],
    [
      "Hello [u]World[/u]",
      doc(p(t("Hello "), bbcode("u", t("World")))),
      "<p>Hello <u>World</u></p>",
      "Hello [u]World[/u]",
    ],
    [
      "[u]Hello[/u] [u]World[/u]",
      doc(p(bbcode("u", t("Hello")), t(" "), bbcode("u", t("World")))),
      "<p><u>Hello</u> <u>World</u></p>",
      "[u]Hello[/u] [u]World[/u]",
    ],
  ],
  "underline with other marks": [
    [
      "___[u][`Hello`](https://example.com)[/u]___",
      doc(p(em("_", strong("__", bbcode("u", link("https://example.com", null, code("Hello"))))))),
      '<p><em><strong><u><a href="https://example.com"><code>Hello</code></a></u></strong></em></p>',
      "***[u][`Hello`](https://example.com)[/u]***",
    ],
  ],
};

// to-markdown-test.js's cases the core extensions cover: [html, markdown]
const TO_MARKDOWN = {
  "converts styles between normal words": [
    ["Line with <s>styles</s> <b><i>between</i></b> words.", "Line with ~~styles~~ ***between*** words."],
    ["A <b>bold </b>word", "A **bold** word"],
    ["A <b>bold</b>, word", "A **bold**, word"],
  ],
  "converts inline nested styles": [
    [
      "<em>Italicised line with <strong>some random</strong> <b>bold</b> words.</em>",
      "*Italicised line with **some random** **bold** words.*",
    ],
    [
      `<i class="fa">Italicised line
     with <b title="strong">some<br>
     random</b> <s>bold</s> words.</i>`,
      "*Italicised line with **some\nrandom** ~~bold~~ words.*",
    ],
  ],
  "only treats bold font weights as strong": [
    [`some <span style="font-weight: 500;">medium</span> text`, "some medium text"],
    [`some <span style="font-weight: 700;">bold</span> text`, "some **bold** text"],
    [`some <span style="font-weight: 1000;">black</span> text`, "some **black** text"],
    [`some <span style="font-weight: 650.5;">variable</span> text`, "some **variable** text"],
    [`some <span style="font-weight: bold;">bold</span> text`, "some **bold** text"],
  ],
  "converts heading tags": [
    [
      `
    <h1>Heading 1</h1>
    <h2>Heading 2</h2>

    \t  <h3>Heading 3</h3>


    <h4>Heading 4</h4>



  <h5>Heading 5</h5>




  <h6>Heading 6</h6>
    `,
      `# Heading 1\n\n## Heading 2\n\n### Heading 3\n\n#### Heading 4\n\n##### Heading 5\n\n###### Heading 6`,
    ],
  ],
  "converts ul list tag": [
    [
      `
    <ul>
      <li>Item 1</li>
      <li>
        Item 2
        <ul>
          <li>Sub Item 1</li>
          <li>Sub Item 2</li>
          <li>Sub Item 3<ul><li>Sub <i>Sub</i> Item 1</li><li>Sub <b>Sub</b> Item 2</li></ul></li>
        </ul>
      </li>
      <li>Item 3</li>
    </ul>
    `,
      `* Item 1\n* Item 2\n  * Sub Item 1\n  * Sub Item 2\n  * Sub Item 3\n    * Sub *Sub* Item 1\n    * Sub **Sub** Item 2\n* Item 3`,
    ],
  ],
  "converts blockquote tag": [
    ["<blockquote>Lorem ipsum</blockquote>", "> Lorem ipsum"],
    ["<blockquote>Lorem ipsum</blockquote><blockquote><p>dolor sit amet</p></blockquote>", "> Lorem ipsum\n\n> dolor sit amet"],
  ],
  "converts ol list tag": [
    [
      `Testing
    <ol>
      <li>Item 1</li>
      <li>
        Item 2
        <ol start="100">
          <li>Sub Item 1</li>
          <li>Sub Item 2</li>
        </ol>
      </li>
      <li>Item 3</li>
    </ol>
    `,
      `Testing\n\n1. Item 1\n2. Item 2\n   100. Sub Item 1\n   101. Sub Item 2\n3. Item 3`,
    ],
  ],
  "converts loose lists (with paragraphs inside items)": [
    [`<ul><li><p>Item 1</p></li><li><p>Item 2</p></li></ul>`, `* Item 1\n\n* Item 2`],
    [`<ol><li><p>First</p></li><li><p>Second</p></li></ol>`, `1. First\n\n2. Second`],
  ],
  "converts ol with custom start attribute": [[`<ol start="5"><li>Fifth</li><li>Sixth</li></ol>`, `5. Fifth\n6. Sixth`]],
};

// The page: the loader as the composer's pages will carry it, and the
// editor module it loads.
async function openEditorPage(page) {
  await page.goto("/");
  await page.addScriptTag({ content: asset("js/rich-editor.js") });
  await page.evaluate(() => window.Discourse.richEditor.load());
}

// In the page: an editor (defaults alone, or the core extensions) holding
// the document `build` makes, then testMarkdown's checks. `build` is
// "html" (parseDOM over the expected HTML), "tree" (the node tree) or
// "markdown" (the export).
function check(page, build, input, defaultsOnly) {
  return page.evaluate(
    async ({ build, input, defaultsOnly }) => {
      const editorModule = await window.Discourse.richEditor.load();
      const extensionsModule = await import("discourse/rich-editor/extensions");
      const { DOMParser } = await import("prosemirror-model");
      const extensions = defaultsOnly ? [] : extensionsModule.getExtensions();

      const host = document.createElement("div");
      host.className = "rich-editor-spec";
      document.body.appendChild(host);
      let value = null;
      const parseTree = build === "html" ? null : build === "tree" ? () => input : window.richEditorSpecTree;
      const editor = editorModule.createEditor({
        container: host,
        extensions,
        parseTree,
        value: build === "html" ? "" : build === "tree" ? "(the tree)" : input,
        onChange: (markdown) => {
          value = markdown;
        },
      });
      const view = editor.view;

      if (build === "html") {
        // The document as a paste brings it: an edit, so it serializes.
        const dom = document.createElement("div");
        dom.innerHTML = input;
        const parsed = DOMParser.fromSchema(editor.schema).parse(dom);
        view.dispatch(view.state.tr.replaceWith(0, view.state.doc.content.size, parsed.content));
      }

      // rich-editor-helper.gjs: a paragraph inserted and deleted again, so
      // the value is what an edit serializes.
      const tr = view.state.tr;
      tr.insert(tr.doc.content.size, view.state.schema.node("paragraph", null, view.state.schema.text("X")));
      tr.delete(tr.doc.content.size - 3, tr.doc.content.size);
      view.dispatch(tr);

      const html = view.dom.innerHTML
        .replace(' class="ProseMirror-selectednode"', "")
        .replace('<img class="ProseMirror-separator" alt="">', "")
        .replace('<br class="ProseMirror-trailingBreak">', "")
        .replace(' class=""', "")
        .replace(/<p>(<br class="ProseMirror-trailingBreak">)?<\/p>$/, "")
        .replace('<span class="fake-cursor ProseMirror-widget" contenteditable="false"></span>', "");

      editor.destroy();
      host.remove();
      return { html, value };
    },
    { build, input, defaultsOnly }
  );
}

test.describe("rich editor", () => {
  test("serves its modules through the import map", async ({ page }) => {
    await openEditorPage(page);
    const map = await page.evaluate(() => {
      const tag = document.querySelector('script[type="importmap"][data-rich-editor]');
      return tag && JSON.parse(tag.textContent);
    });
    expect(map).not.toBeNull();
    for (const url of Object.values(map.imports)) {
      const res = await page.request.get(url);
      expect(res.status(), url).toBe(200);
      expect(res.headers()["content-type"], url).toContain("javascript");
    }
    // prosemirror-markdown loaded with the stub in markdown-it's place.
    const stub = await page.evaluate(async () => {
      const { defaultMarkdownParser } = await import("prosemirror-markdown");
      try {
        defaultMarkdownParser.parse("x");
        return "parsed";
      } catch (e) {
        return e.message;
      }
    });
    expect(stub).toContain("markdown-it is not loaded");
  });

  test("opens empty, with the placeholder, and serializes what is typed", async ({ page }) => {
    await openEditorPage(page);
    await page.evaluate(async () => {
      const { createEditor } = await window.Discourse.richEditor.load();
      const host = document.createElement("div");
      host.id = "rich-editor-typing";
      document.body.appendChild(host);
      window.richEditorTyped = [];
      window.richEditorTyping = createEditor({
        container: host,
        value: "",
        placeholder: "Type here",
        className: "d-editor-input",
        onChange: (markdown) => window.richEditorTyped.push(markdown),
      });
    });
    const editor = page.locator("#rich-editor-typing .ProseMirror.d-editor-input");
    await expect(editor.locator("p[data-placeholder='Type here']")).toHaveCount(1);
    await editor.click();
    // The input rules: "## " a heading, "**bold**" a strong mark, "* " a list.
    await page.keyboard.type("## Title");
    await page.keyboard.press("Enter");
    await page.keyboard.type("some **bold** words");
    await page.keyboard.press("Enter");
    await page.keyboard.type("* one");
    await page.keyboard.press("Enter");
    await page.keyboard.type("two");
    await expect(editor.locator("h2")).toHaveText("Title");
    await expect(editor.locator("p strong")).toHaveText("bold");
    await expect(editor.locator("ul li")).toHaveCount(2);
    const typed = await page.evaluate(() => window.richEditorTyped[window.richEditorTyped.length - 1]);
    expect(typed).toBe("## Title\n\nsome **bold** words\n\n* one\n* two");
    // Undo (prosemirror-history, Mod-z) takes the last typing back.
    await page.keyboard.press("Control+z");
    await expect(editor).not.toContainText("two");
  });

  test("refuses markdown it cannot parse yet, and without the export anything but empty", async ({ page }) => {
    await openEditorPage(page);
    const result = await page.evaluate(async () => {
      const { createEditor } = await window.Discourse.richEditor.load();
      const host = document.createElement("div");
      document.body.appendChild(host);
      const refused = [];
      createEditor({
        container: host,
        value: ":smile:",
        parseTree: () => ({ type: "doc", children: [{ type: "paragraph", children: [{ type: "inline", children: [{ type: "emoji", content: "smile" }] }] }] }),
        onUnsupported: (e) => refused.push(e.name),
      });
      let missing = null;
      try {
        createEditor({ container: host, value: "Hello" });
      } catch (e) {
        missing = e.message;
      }
      let broken = null;
      try {
        createEditor({ container: host, value: "Hello", parseTree: () => ({ type: "paragraph" }) });
      } catch (e) {
        broken = e.name;
      }
      return { refused, missing, broken };
    });
    expect(result.refused).toEqual(["UnsupportedTokenError"]);
    expect(result.missing).toContain("node tree export is not loaded");
    expect(result.broken).toBe("NodeTreeError");
  });

  for (const [suite, defaultsOnly] of [
    [DEFAULTS, true],
    [EXTENSIONS, false],
  ]) {
    for (const [name, cases] of Object.entries(suite)) {
      test(`serializes ${name}`, async ({ page }) => {
        await openEditorPage(page);
        for (const [markdown, , html, out] of cases) {
          const result = await check(page, "html", html, defaultsOnly);
          expect(result.html, `HTML for ${JSON.stringify(markdown)}`).toBe(html);
          expect(result.value, `markdown for ${JSON.stringify(markdown)}`).toBe(out);
        }
      });

      test(`maps ${name}`, async ({ page }) => {
        await openEditorPage(page);
        for (const [markdown, tree, html, out] of cases) {
          const result = await check(page, "tree", tree, defaultsOnly);
          expect(result.html, `HTML for ${JSON.stringify(markdown)}`).toBe(html);
          expect(result.value, `markdown for ${JSON.stringify(markdown)}`).toBe(out);
        }
      });

      test(`round-trips ${name}`, async ({ page }) => {
        await openEditorPage(page);
        // markdown.wasm's node tree export (crates/markdown/src/wasm.rs,
        // docs/rich-editor.md), configured with the site's settings as the
        // composer's preview is.
        const exported = await page.evaluate(async () => {
          const [wasm, settings] = await Promise.all([
            WebAssembly.instantiateStreaming(fetch("/assets/markdown.wasm"), {}),
            fetch("/assets/markdown-settings.json").then((r) => r.text()),
          ]);
          const exports = wasm.instance.exports;
          if (typeof exports.rich_editor_tree !== "function") {
            return false;
          }
          const call = (fn, text) => {
            const bytes = new TextEncoder().encode(text);
            const ptr = exports.alloc(bytes.length);
            new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
            const status = fn(ptr, bytes.length);
            exports.dealloc(ptr, bytes.length);
            const out = new TextDecoder().decode(
              new Uint8Array(exports.memory.buffer, exports.output_ptr(), exports.output_len())
            );
            if (status !== 0) {
              throw new Error(out);
            }
            return out;
          };
          call(exports.configure, settings);
          window.richEditorSpecTree = (raw) => JSON.parse(call(exports.rich_editor_tree, raw));
          return true;
        });
        test.skip(!exported, "markdown.wasm has no rich_editor_tree export yet (ROADMAP 196r1, phase B)");
        for (const [markdown, , html, out] of cases) {
          const result = await check(page, "markdown", markdown, defaultsOnly);
          expect(result.html, `HTML for ${JSON.stringify(markdown)}`).toBe(html);
          expect(result.value, `markdown for ${JSON.stringify(markdown)}`).toBe(out);
        }
      });
    }
  }

  for (const [name, cases] of Object.entries(TO_MARKDOWN)) {
    test(`to-markdown ${name}`, async ({ page }) => {
      await openEditorPage(page);
      for (const [html, markdown] of cases) {
        const result = await page.evaluate(async (html) => {
          const toMarkdown = (await import("discourse/rich-editor/to-markdown")).default;
          return toMarkdown(html);
        }, html);
        expect(result, `markdown for ${JSON.stringify(html)}`).toBe(markdown);
      }
    });
  }
});
