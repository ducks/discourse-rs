// The reference's tests/unit/static/prosemirror/extensions/word-paste-test.js,
// its bodies run as written in the editor page (rich-editor-helper.js
// inPage) against static/js/rich-editor/extensions/word-paste.js.
const { test } = require("./helpers");
const { qunitTest } = require("./rich-editor-helper");

const qtest = (name, body) => qunitTest(test, name, body);

test.describe("rich editor word paste", () => {
  qtest("no-op when no Word list classes present", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = "<p>Regular paragraph</p><ul><li>Normal list</li></ul>";
    assert.strictEqual(transformWordHtml(html), html);
  });

  qtest("converts unordered list", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = wordListHtml([
      { marker: "·", text: "Item 1" },
      { marker: "·", text: "Item 2" },
    ]);
    const doc = createDoc(transformWordHtml(html));

    assert.strictEqual(doc.querySelectorAll("ul").length, 1);
    assert.strictEqual(doc.querySelectorAll("li").length, 2);
    assert.strictEqual(
      doc.querySelectorAll("p.MsoListParagraphCxSpFirst").length,
      0
    );
  });

  qtest("detects ordered list from various marker formats", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const markers = [
      "1.",
      "1)",
      "a.",
      "A)",
      "i.",
      "ii.",
      "L.",
      "C)",
      "D.",
      "M)",
    ];

    for (const marker of markers) {
      const html = wordListHtml([
        { marker, text: "First" },
        { marker: "2.", text: "Second" },
      ]);
      const doc = createDoc(transformWordHtml(html));
      assert.strictEqual(
        doc.querySelectorAll("ol").length,
        1,
        `marker "${marker}" creates ol`
      );
    }
  });

  qtest("sets start attribute for custom start number", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = wordListHtml([
      { marker: "5.", text: "Fifth" },
      { marker: "6.", text: "Sixth" },
    ]);
    const doc = createDoc(transformWordHtml(html));
    assert.strictEqual(doc.querySelector("ol").getAttribute("start"), "5");
  });

  qtest("handles nested lists", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = wordListHtml([
      { marker: "·", text: "Level 1", level: 1 },
      { marker: "·", text: "Level 2", level: 2 },
      { marker: "·", text: "Level 3", level: 3 },
      { marker: "·", text: "Back to 1", level: 1 },
    ]);
    const doc = createDoc(transformWordHtml(html));

    assert.strictEqual(doc.querySelectorAll("ul").length, 3);
    assert.strictEqual(doc.querySelectorAll("li").length, 4);
  });

  qtest("handles IE conditional comments", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = `
      <p class="MsoListParagraphCxSpFirst" style="mso-list:l0 level1 lfo1">
        <![if !supportLists]><span style="mso-list:Ignore">1.</span><![endif]>First
      </p>
      <p class="MsoListParagraphCxSpLast" style="mso-list:l0 level1 lfo1">
        <![if !supportLists]><span style="mso-list:Ignore">2.</span><![endif]>Second
      </p>
    `;
    const doc = createDoc(transformWordHtml(html));

    assert.strictEqual(doc.querySelectorAll("ol").length, 1);
    assert.false(doc.querySelector("li").textContent.includes("1."));
  });

  qtest("preserves inline formatting", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = wordListHtml([
      { marker: "·", text: "<b>Bold</b>" },
      { marker: "·", text: "<i>Italic</i>" },
    ]);
    const doc = createDoc(transformWordHtml(html));

    assert.true(!!doc.querySelector("li b"));
    assert.true(!!doc.querySelector("li i"));
  });

  qtest("sets data-tight attribute", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = wordListHtml([
      { marker: "·", text: "Item 1" },
      { marker: "·", text: "Item 2" },
    ]);
    const doc = createDoc(transformWordHtml(html));
    assert.strictEqual(
      doc.querySelector("ul").getAttribute("data-tight"),
      "true"
    );
  });

  qtest("handles multiple separate lists with surrounding text", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = `
      <p>Intro</p>
      ${wordListHtml([{ marker: "·", text: "Bullet" }])}
      <p>Middle</p>
      ${wordListHtml([{ marker: "1.", text: "Numbered" }])}
    `;
    const doc = createDoc(transformWordHtml(html));

    assert.strictEqual(doc.querySelectorAll("ul").length, 1);
    assert.strictEqual(doc.querySelectorAll("ol").length, 1);
  });

  qtest("transformWordLists modifies DOM in place", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const doc = createDoc(wordListHtml([{ marker: "·", text: "Item" }]));

    transformWordLists(doc.body);

    assert.strictEqual(doc.querySelectorAll("ul").length, 1);
    assert.strictEqual(
      doc.querySelectorAll("p.MsoListParagraphCxSpFirst").length,
      0
    );
  });

  qtest("converts Word Quote style to blockquote", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = `<p class=MsoQuote style='margin-left:.5in'>Quoted line</p>`;
    const doc = createDoc(transformWordHtml(html));

    assert.strictEqual(doc.querySelectorAll("blockquote").length, 1);
    assert.strictEqual(doc.querySelectorAll("p.MsoQuote").length, 0);
    assert.true(
      doc.querySelector("blockquote").textContent.includes("Quoted line")
    );
  });

  qtest("converts Word Intense Quote style to blockquote", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = `<p class=MsoIntenseQuote>Intense line</p>`;
    const doc = createDoc(transformWordHtml(html));

    assert.strictEqual(doc.querySelectorAll("blockquote").length, 1);
    assert.strictEqual(doc.querySelectorAll("p.MsoIntenseQuote").length, 0);
  });

  qtest("merges consecutive quote paragraphs into one blockquote", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = `
      <p class=MsoQuote>First line</p>
      <p class=MsoQuote>Second line</p>
      <p>Not a quote</p>
      <p class=MsoQuote>Separate quote</p>
    `;
    const doc = createDoc(transformWordHtml(html));

    const blockquotes = doc.querySelectorAll("blockquote");
    assert.strictEqual(blockquotes.length, 2);
    assert.strictEqual(blockquotes[0].querySelectorAll("p").length, 2);
    assert.strictEqual(blockquotes[1].querySelectorAll("p").length, 1);
  });

  qtest("preserves inline formatting in quotes", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = `<p class=MsoQuote>Has <b>bold</b> text</p>`;
    const doc = createDoc(transformWordHtml(html));

    assert.true(!!doc.querySelector("blockquote b"));
  });

  qtest("transformWordQuotes modifies DOM in place", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const doc = createDoc(`<p class=MsoQuote>Quoted</p>`);

    transformWordQuotes(doc.body);

    assert.strictEqual(doc.querySelectorAll("blockquote").length, 1);
    assert.strictEqual(doc.querySelectorAll("p.MsoQuote").length, 0);
  });

  qtest("converts Word Block Text style to blockquote", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = `<p class=MsoBlockText>Block text line</p>`;
    const doc = createDoc(transformWordHtml(html));

    assert.strictEqual(doc.querySelectorAll("blockquote").length, 1);
    assert.strictEqual(doc.querySelectorAll("p.MsoBlockText").length, 0);
  });

  qtest("converts Word for the web Quote style to blockquote regardless of locale", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    // PT-BR document: visible text is localized but the style id stays English
    const html = `
      <div class="OutlineElement"><p class="Paragraph" lang="PT-BR"><span
        class="TextRun"><span class="NormalTextRun"
        data-ccp-parastyle="Quote">Citação</span></span></p></div>
      <div class="OutlineElement"><p class="Paragraph" lang="PT-BR"><span
        class="TextRun"><span class="NormalTextRun">Regular text</span></span></p></div>
    `;
    const doc = createDoc(transformWordHtml(html));

    assert.strictEqual(doc.querySelectorAll("blockquote").length, 1);
    assert.true(
      doc.querySelector("blockquote").textContent.includes("Citação")
    );
    assert.false(
      doc.querySelector("blockquote").textContent.includes("Regular text")
    );
  });

  qtest("does not convert non-quote Word for the web styles", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = `<div class="OutlineElement"><p class="Paragraph"><span
      class="NormalTextRun" data-ccp-parastyle="Texto de Dica">Tip</span></p></div>`;
    const doc = createDoc(transformWordHtml(html));

    assert.strictEqual(doc.querySelectorAll("blockquote").length, 0);
  });

  qtest("strips Word comment references and comment body", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = `<p class=MsoNormal>Significant accounts
      <a class=msocomanchor href="#_msocom_1">[J1]</a>
      and assertions.</p>
      <div style='mso-element:comment-list'>
        <div style='mso-element:comment'>
          <p class=MsoCommentText>Consider removing.</p>
        </div>
      </div>`;
    const doc = createDoc(transformWordHtml(html));

    assert.strictEqual(doc.querySelectorAll("a.msocomanchor").length, 0);
    assert.strictEqual(
      doc.querySelectorAll("[style*='mso-element:comment-list']").length,
      0
    );
    assert.false(doc.body.textContent.includes("[J1]"));
    assert.false(doc.body.textContent.includes("Consider removing."));
    assert.true(doc.body.textContent.includes("Significant accounts"));
  });

  qtest("strips Word tracked deletions but keeps surrounding text", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = `<p class=MsoNormal>Keep <del>deleted text</del>this.</p>`;
    const doc = createDoc(transformWordHtml(html));

    assert.strictEqual(doc.querySelectorAll("del").length, 0);
    assert.false(doc.body.textContent.includes("deleted text"));
    assert.true(doc.body.textContent.includes("Keep"));
  });

  qtest("does not strip <del> from non-Word HTML", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = `<p>Keep <del>struck</del> text</p>`;
    const doc = createDoc(transformWordHtml(html));

    assert.strictEqual(doc.querySelectorAll("del").length, 1);
  });

  qtest("stripWordReviewMarkup modifies DOM in place", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const doc = createDoc(
      `<p>Text <a class=msocomanchor href="#_msocom_1">[1]</a></p>`
    );

    stripWordReviewMarkup(doc.body);

    assert.strictEqual(doc.querySelectorAll("a.msocomanchor").length, 0);
  });

  qtest("does not treat a generic TextRun class as Word (no content loss)", async function (assert, { wordPaste }) {
    const { stripWordReviewMarkup, transformWordHtml, transformWordLists, transformWordQuotes } = wordPaste;
    const createDoc = (html) => new DOMParser().parseFromString(html, "text/html");
    const wordListHtml = (items, { startClass = "First", endClass = "Last" } = {}) =>
      items
        .map((item, i) => {
          let cls = "MsoListParagraphCxSpMiddle";
          if (i === 0) {
            cls = `MsoListParagraphCxSp${startClass}`;
          }
          if (i === items.length - 1) {
            cls = `MsoListParagraphCxSp${endClass}`;
          }
          return `<p class="${cls}" style="mso-list:l0 level${item.level || 1} lfo1">
          <span style="mso-list:Ignore">${item.marker}</span>${item.text}
        </p>`;
        })
        .join("\n");

    const html = `<p class="TextRun">Keep <del>this</del> too</p>`;
    const doc = createDoc(transformWordHtml(html));

    assert.strictEqual(doc.querySelectorAll("del").length, 1);
  });
});
