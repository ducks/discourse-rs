// The composer's toolbar wired to the rich editor (ROADMAP 196r3):
// rich-editor/text-manipulation.js against the reference's
// text-manipulation-test.gjs and the checklist's and footnote's
// rich-editor-text-manipulation-test.gjs, run as written in the page
// (rich-editor-helper.js qunitTest, `tm` its setupEditor and friends), and
// prosemirror_toolbar_spec.rb's and prosemirror_keymap_spec.rb's toolbar
// cases on the composer (static/js/editor-toolbar.js).
const { test, expect, login, sql, member } = require("./helpers");
const { qunitTest } = require("./rich-editor-helper");

const qtest = (name, body, opts) => qunitTest(test, name, body, opts);

test.describe("TextManipulation: applySurround", () => {
  qtest("surrounds formatted lines independently in multiline mode", async function (assert, { tm }) {
    const { setupEditor, setContent, selectAll } = tm;
    const state = await setupEditor();
    setContent(state, "**one**\n_two_");
    selectAll(state);
    state.textManipulation.applySurroundSelection("<kbd>", "</kbd>", "code_text", { multiline: true });
    assert.dom(".ProseMirror kbd").exists({ count: 2 }, "each line has its own wrapper");
    assert.dom(".ProseMirror kbd strong, .ProseMirror strong kbd").hasText("one", "bold survives");
    assert.dom(".ProseMirror kbd em, .ProseMirror em kbd").hasText("two", "italic survives");
    assert.false(state.textManipulation.view.state.selection.empty, "the formatted content stays selected");
  });

  for (const [name, head, tail] of [
    ["bare delimiters", "```", "```"],
    ["delimiters with newlines", "```\n", "\n```"],
  ]) {
    qtest(
      `useBlockMode separates multiline content with ${name}`,
      async function (assert, { tm }, [head, tail]) {
        const { setupEditor, setContent, selectAll } = tm;
        const state = await setupEditor();
        setContent(state, "first\nsecond");
        selectAll(state);

        state.textManipulation.applySurroundSelection(head, tail, "code_text", {
          multiline: false,
          useBlockMode: true,
        });

        assert
          .dom(".ProseMirror pre code")
          .hasText("first\nsecond", "both lines are inside the code block without extra blank lines");
      },
      { args: [head, tail] },
    );
  }

  qtest("useBlockMode keeps single-line code inline", async function (assert, { tm }) {
    const { setupEditor, setContent } = tm;
    const state = await setupEditor();
    setContent(state, "before secret after");
    state.textManipulation.selectText(8, 6);

    state.textManipulation.applySurroundSelection("```", "```", "wrap_text", {
      multiline: false,
      useBlockMode: true,
    });

    assert.dom(".ProseMirror p code").hasText("secret", "the selection is wrapped");
    assert.dom(".ProseMirror br").doesNotExist("no boundary breaks are added");
    assert.dom(".ProseMirror p").hasText("before secret after", "surrounding text is preserved");
  });

  qtest(
    "multiline surround preserves text outside the selection and undoes in one step",
    async function (assert, { tm, pmState, pmHistory }) {
      const { setupEditor, setContent, getMarkdown } = tm;
      const { TextSelection } = pmState;
      const { closeHistory, undo } = pmHistory;
      const state = await setupEditor();
      setContent(state, "hello one\ntwo world");
      const { view } = state.textManipulation;
      view.dispatch(closeHistory(view.state.tr.setSelection(TextSelection.create(view.state.doc, 7, 14))));
      state.textManipulation.applySurroundSelection("<kbd>", "</kbd>", "code_text", { multiline: true });
      assert.dom(".ProseMirror kbd:first-of-type").hasText("one", "only the selected first line is wrapped");
      assert.dom(".ProseMirror kbd:last-of-type").hasText("two", "only the selected last line is wrapped");
      undo(view.state, view.dispatch);
      assert.strictEqual(getMarkdown(state).trim(), "hello one\ntwo world", "one undo restores both lines");
    },
  );

  qtest("multiline surround preserves the containing blockquote", async function (assert, { tm }) {
    const { setupEditor, setContent, selectAll } = tm;
    const state = await setupEditor();
    setContent(state, "> one\n> two");
    selectAll(state);
    state.textManipulation.applySurroundSelection("<kbd>", "</kbd>", "code_text", { multiline: true });
    assert.dom(".ProseMirror blockquote kbd").exists({ count: 2 }, "both lines stay inside the quote");
  });

  qtest("applies a multiline heading prefix across hard breaks", async function (assert, { tm }) {
    const { setupEditor, setContent, selectAll, getMarkdown } = tm;
    const state = await setupEditor();
    setContent(state, "one\ntwo");
    selectAll(state);
    state.textManipulation.applySurroundSelection("# ", "", "heading_text", { multiline: true });
    assert.dom(".ProseMirror h1").exists({ count: 2 }, "each selected line becomes a heading");
    assert.strictEqual(getMarkdown(state).trim(), "# one\n\n# two", "both lines retain their content without literal prefixes");
  });

  qtest("multiline heading prefixes preserve unselected boundary text", async function (assert, { tm }) {
    const { setupEditor, setContent } = tm;
    const state = await setupEditor();
    setContent(state, "hello\nworld\nafter");
    state.textManipulation.selectText(4, 5);
    state.textManipulation.applySurroundSelection("# ", "", "heading_text", { multiline: true });
    assert.dom(".ProseMirror > p:first-child").hasText("hel# lo", "a prefix in the middle of a line stays literal");
    assert.dom(".ProseMirror h1").hasText("world", "a heading includes the rest of its line");
    assert.dom(".ProseMirror > p:last-child").hasText("after", "the following line is preserved");
    assert.dom(".ProseMirror br").doesNotExist("block boundaries replace the original hard breaks");
  });

  qtest("honors a caller-adjusted surround selection", async function (assert, { tm }) {
    const { setupEditor, setContent, selectAll } = tm;
    const state = await setupEditor();
    setContent(state, "hello world");
    selectAll(state);
    const selected = state.textManipulation.getSelected();
    selected.start = 7;
    selected.value = "world";
    state.textManipulation.applySurround(selected, "<small>", "</small>", "wrap_text");
    assert.dom(".ProseMirror small").hasText("world", "only the supplied range is wrapped");
    assert.dom(".ProseMirror p").hasText("hello world", "the preceding text is retained");
  });

  for (const [head, tail] of [
    ["hello ", " suffix"],
    ["prefix ", " hello"],
    ["hello ", " hello"],
  ]) {
    qtest(
      `restores the original content selection with repeated wrapper text (${head}, ${tail})`,
      async function (assert, { tm }, [head, tail]) {
        const { setupEditor, setContent, selectAll } = tm;
        const state = await setupEditor();
        setContent(state, "hello");
        selectAll(state);

        state.textManipulation.applySurroundSelection(head, tail, "wrap_text", { multiline: false });

        const { view } = state.textManipulation;
        view.dispatch(view.state.tr.insertText("replacement"));

        assert
          .dom(".ProseMirror p")
          .hasText(`${head}replacement${tail}`, "typing replaces the original content and preserves the wrapper");
      },
      { args: [head, tail] },
    );
  }

  qtest("restores the content selection with a repeated inline prefix (# hello )", async function (assert, { tm }) {
    const { setupEditor, setContent } = tm;
    const head = "# hello ";
    const state = await setupEditor();
    setContent(state, "before hello after");
    state.textManipulation.selectText(8, 5);

    state.textManipulation.applySurroundSelection(head, " suffix", "wrap_text", { multiline: false });

    const { view } = state.textManipulation;
    view.dispatch(view.state.tr.insertText("replacement"));

    assert
      .dom(".ProseMirror p")
      .hasText(`before ${head}replacement suffix after`, "typing preserves the literal inline prefix and surrounding text");
  });

  qtest("preserves formatting and selection after an inline surround", async function (assert, { tm }) {
    const { setupEditor, setContent } = tm;
    const state = await setupEditor();
    setContent(state, "hello **world**");
    state.textManipulation.selectText(7, 5);
    state.textManipulation.applySurroundSelection("<small>", "</small>", "wrap_text", { multiline: false });
    assert.dom(".ProseMirror small strong, .ProseMirror strong small").hasText("world", "existing formatting is preserved");
    const { view } = state.textManipulation;
    assert.strictEqual(
      view.state.doc.textBetween(view.state.selection.from, view.state.selection.to),
      "world",
      "the content remains selected",
    );
  });

  qtest("selects an image created from a selected URL", async function (assert, { tm, helpers }) {
    const { setupEditor, setContent, selectAll } = tm;
    const state = await setupEditor();
    setContent(state, "https://example.com/image.png");
    selectAll(state);
    state.textManipulation.applySurroundSelection('<img src="', '" alt="">', "wrap_text", { multiline: false });
    await helpers.settled();
    assert.dom(".ProseMirror img").hasAttribute("src", "https://example.com/image.png", "the selected URL becomes an image");
    assert.strictEqual(
      state.textManipulation.view.state.selection.node && state.textManipulation.view.state.selection.node.type.name,
      "image",
      "the image is selected",
    );
  });

  for (const [edge, markdown] of [
    ["leading", "![a](https://example.com/image.png) hello"],
    ["trailing", "hello ![a](https://example.com/image.png)"],
  ]) {
    qtest(
      `preserves a selected ${edge} image when surrounding text`,
      async function (assert, { tm, helpers }, markdown) {
        const { setupEditor, setContent, selectAll } = tm;
        const state = await setupEditor();
        setContent(state, markdown);
        selectAll(state);
        state.textManipulation.applySurroundSelection("<small>", "</small>", "wrap_text", { multiline: false });
        await helpers.settled();
        const { view } = state.textManipulation;
        assert.dom(".ProseMirror small img").exists("the image is inside the wrapper");
        const { from, to } = view.state.selection;
        assert.strictEqual(view.state.doc.textBetween(from, to).trim(), "hello", "the text remains selected");
        view.dispatch(view.state.tr.insertText("replacement"));
        assert.dom(".ProseMirror img").doesNotExist("replacing the selection removes the image too");
        assert.dom(".ProseMirror").hasText("replacement", "typing replaces the complete selected content");
      },
      { args: markdown },
    );
  }

  qtest("trims leading whitespace in toolbar selections", async function (assert, { tm }) {
    const { setupEditor, setContent } = tm;
    const state = await setupEditor();
    setContent(state, "hello world");
    state.textManipulation.selectText(6, 6);
    const selected = state.textManipulation.getSelected(true);
    state.textManipulation.applySurround(selected, "<small>", "</small>", "wrap_text");
    assert.dom(".ProseMirror small").hasText("world", "leading whitespace stays outside the wrapper");
    assert.strictEqual(selected.start, 7, "the selection starts after whitespace");
  });

  qtest("keeps a selected block whole when trimming leading whitespace", async function (assert, { tm, pmState }) {
    const { setupEditor, setContent } = tm;
    const { NodeSelection } = pmState;
    const state = await setupEditor();
    setContent(state, "```\n  indented\n```");
    const { view } = state.textManipulation;
    view.dispatch(view.state.tr.setSelection(NodeSelection.create(view.state.doc, 0)));
    const selected = state.textManipulation.getSelected(true);
    assert.strictEqual(selected.start, 0, "the selection is not trimmed");
    assert.strictEqual(selected.end, view.state.doc.firstChild.nodeSize, "the selection still covers the block");
  });

  for (const [name, head] of [
    ["bold", "**"],
    ["strikethrough", "~~"],
  ]) {
    qtest(
      `toggles the ${name} mark via parser detection`,
      async function (assert, { tm }, [name, head]) {
        const { setupEditor, setContent, selectAll, getMarkdown } = tm;
        const state = await setupEditor();
        setContent(state, "hello world");
        selectAll(state);
        state.textManipulation.applySurroundSelection(head, head, `${name}_text`);
        assert.strictEqual(getMarkdown(state).trim(), `${head}hello world${head}`, "the mark is applied");

        selectAll(state);
        state.textManipulation.applySurroundSelection(head, head, `${name}_text`);
        assert.strictEqual(getMarkdown(state).trim(), "hello world", "the mark is removed");
      },
      { args: [name, head] },
    );
  }

  qtest("preserves combined marks in the fallback", async function (assert, { tm }) {
    const { setupEditor, setContent, selectAll } = tm;
    const state = await setupEditor();
    setContent(state, "hello world");
    selectAll(state);

    state.textManipulation.applySurroundSelection("**_", "_**", "bold_text");

    assert.dom(".ProseMirror strong em, .ProseMirror em strong").hasText("hello world", "both marks are applied");
  });

  for (const [head, tail, selector, text] of [
    ["> **", "**", "blockquote strong", "hello world"],
    ["> prefix ", " suffix", "blockquote", "prefix hello world suffix"],
    ["> > ", "", "blockquote blockquote", "hello world"],
  ]) {
    qtest(
      `preserves the content of the "${head}" block wrapper`,
      async function (assert, { tm }, [head, tail, selector, text]) {
        const { setupEditor, setContent, selectAll } = tm;
        const state = await setupEditor();
        setContent(state, "hello world");
        selectAll(state);

        state.textManipulation.applySurroundSelection(head, tail, "blockquote_text");

        assert.dom(`.ProseMirror ${selector}`).hasText(text, "the wrapper keeps its content");
        assert.dom(".ProseMirror > blockquote:first-child").exists("the replaced paragraph is not left behind");
      },
      { args: [head, tail, selector, text] },
    );
  }

  qtest("uses the inline variant of block-capable markup inside a paragraph", async function (assert, { tm, pmState }) {
    const { setupEditor, setContent, countNodes } = tm;
    const { TextSelection } = pmState;
    const state = await setupEditor();
    setContent(state, "hello world");
    const { view } = state.textManipulation;
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 7, 12)));

    state.textManipulation.applySurroundSelection("[wrap=note]", "[/wrap]", "wrap_text");

    assert.strictEqual(countNodes(state, "wrap_inline"), 1);
    assert.strictEqual(countNodes(state, "wrap_block"), 0);
    // The wrap's indicator is drawn at once here (the reference's Glimmer
    // node view has not rendered yet when it reads the text).
    assert.dom(".ProseMirror p").hasText("hello world[wrap=note]");
  });

  qtest("uses the block variant of block-capable markup for a whole paragraph", async function (assert, { tm }) {
    const { setupEditor, setContent, selectAll, countNodes } = tm;
    const state = await setupEditor();
    setContent(state, "hello world");
    selectAll(state);

    state.textManipulation.applySurroundSelection("[wrap=note]", "[/wrap]", "wrap_text");

    assert.strictEqual(countNodes(state, "wrap_block"), 1);
    assert.strictEqual(countNodes(state, "wrap_inline"), 0);
    assert.dom(".ProseMirror p").hasText("hello world");
  });

  qtest("wraps a multi-paragraph selection as a block", async function (assert, { tm }) {
    const { setupEditor, setContent, selectAll, countNodes } = tm;
    const state = await setupEditor();
    setContent(state, "first\n\nsecond");
    selectAll(state);

    state.textManipulation.applySurroundSelection("[wrap=note]", "[/wrap]", "wrap_text");

    assert.strictEqual(countNodes(state, "wrap_block"), 1);
    assert.strictEqual(
      state.textManipulation.view.state.doc.firstChild.childCount,
      2,
      "both paragraphs are inside the wrapper",
    );
  });

  qtest("keeps a blockquote around a fully selected quoted paragraph", async function (assert, { tm, pmState }) {
    const { setupEditor, setContent, countNodes } = tm;
    const { TextSelection } = pmState;
    const state = await setupEditor();
    setContent(state, "> quoted");
    const { view } = state.textManipulation;
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 2, 8)));

    state.textManipulation.applySurroundSelection("[wrap=note]", "[/wrap]", "wrap_text");

    assert.strictEqual(countNodes(state, "blockquote"), 1);
    assert.strictEqual(countNodes(state, "wrap_block"), 1);
    assert.dom(".ProseMirror blockquote p").hasText("quoted");
  });

  qtest("keeps a fully selected table cell when wrapping it as a block", async function (assert, { tm, pmState }) {
    const { setupEditor, setContent, countNodes } = tm;
    const { TextSelection } = pmState;
    const state = await setupEditor();
    setContent(state, "| a | b |\n|---|---|\n| one | two |");
    const { view } = state.textManipulation;
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 14, 17)));

    state.textManipulation.applySurroundSelection("[wrap=note]", "[/wrap]", "wrap_text");

    const rowSizes = [];
    view.state.doc.descendants((node) => {
      if (node.type.name === "table_row") {
        rowSizes.push(node.childCount);
      }
    });
    assert.true(
      rowSizes.every((size) => size === 2),
      "every row keeps both cells",
    );
    assert.strictEqual(countNodes(state, "wrap_block"), 1);
  });

  for (const [where, pos] of [
    ["after", 12],
    ["before", 1],
  ]) {
    qtest(
      `selects the placeholder of a block inserted ${where} a paragraph`,
      async function (assert, { tm, pmState }, pos) {
        const { setupEditor, setContent, countNodes } = tm;
        const { TextSelection } = pmState;
        const state = await setupEditor();
        setContent(state, "hello world");
        const { view } = state.textManipulation;
        view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, pos)));

        state.textManipulation.applySurroundSelection("\n[wrap=note]\n", "\n[/wrap]\n", "wrap_text");

        const { from, to } = view.state.selection;
        assert.strictEqual(view.state.doc.textBetween(from, to), "Wrap content");
        assert.strictEqual(countNodes(state, "wrap_block"), 1);
        assert.dom(".ProseMirror > p").hasText("hello world", "the paragraph is kept");
      },
      { args: pos },
    );
  }

  qtest("falls back to round-trip for unrecognized markup", async function (assert, { tm }) {
    const { setupEditor, setContent, selectAll, getMarkdown } = tm;
    const state = await setupEditor();
    setContent(state, "hello world");
    selectAll(state);

    const sel = state.textManipulation.getSelected();
    state.textManipulation.applySurround(sel, "<big>", "</big>", "big_text");

    assert.strictEqual(getMarkdown(state).trim(), "<big>hello world</big>");
  });

  qtest("toggles stored marks when nothing is selected", async function (assert, { tm }) {
    const { setupEditor, setContent, placeCursor, getMarkdown } = tm;
    const state = await setupEditor();
    setContent(state, "hello world");
    placeCursor(state);
    const { view } = state.textManipulation;

    state.textManipulation.applySurroundSelection("**", "**", "bold_text");
    view.dispatch(view.state.tr.insertText("bold "));

    assert.dom(".ProseMirror strong").hasText("bold", "typing is bold");
    assert.false(getMarkdown(state).includes("strong text"), "no placeholder is inserted");

    state.textManipulation.applySurroundSelection("**", "**", "bold_text");
    view.dispatch(view.state.tr.insertText("plain "));

    assert.dom(".ProseMirror strong").hasText("bold", "the stored mark is toggled off again");
  });

  qtest("inserts and selects a placeholder when nothing is selected in the fallback", async function (assert, { tm }) {
    const { setupEditor, setContent, placeCursor } = tm;
    const state = await setupEditor();
    setContent(state, "hello world");
    placeCursor(state);
    const { view } = state.textManipulation;

    state.textManipulation.applySurroundSelection("**_", "_**", "bold_text");

    const { from, to } = view.state.selection;
    assert.strictEqual(view.state.doc.textBetween(from, to), "strong text", "the placeholder is selected");
    assert.dom(".ProseMirror strong em, .ProseMirror em strong").hasText("strong text");
  });
});

test.describe("TextManipulation: applyList", () => {
  qtest("honors inline mode for a custom prefix across paragraphs", async function (assert, { tm }) {
    const { setupEditor, setContent, selectAll, getMarkdown } = tm;
    const state = await setupEditor();
    setContent(state, "one\n\ntwo");
    selectAll(state);
    state.textManipulation.applyList(state.textManipulation.getSelected(), "? ", "list_item", { multiline: false });
    assert.strictEqual(getMarkdown(state).trim(), "? one\n\ntwo", "the prefix is applied once");
  });

  qtest("honors a caller-adjusted list selection", async function (assert, { tm }) {
    const { setupEditor, setContent, selectAll } = tm;
    const state = await setupEditor();
    setContent(state, "hello world");
    selectAll(state);
    const selected = state.textManipulation.getSelected();
    selected.start = 7;
    state.textManipulation.applyList(selected, "? ", "list_item");
    assert.dom(".ProseMirror p").hasText("hello ? world", "only the supplied range is prefixed");
  });

  for (const [name, exampleKey, applied] of [
    ["bullet list", "list_item", "* hello world"],
    ["ordered list", "list_item", "1. hello world"],
    ["blockquote", "blockquote_text", "> hello world"],
  ]) {
    qtest(
      `applies and removes a ${name} via parser detection`,
      async function (assert, { tm }, [name, exampleKey, applied]) {
        const { setupEditor, setContent, selectAll, placeCursor, getMarkdown } = tm;
        const head = {
          "bullet list": "* ",
          "ordered list": (i) => (!i ? "1. " : `${parseInt(i, 10) + 1}. `),
          blockquote: "> ",
        }[name];
        const state = await setupEditor();
        setContent(state, "hello world");
        selectAll(state);
        state.textManipulation.applyList(state.textManipulation.getSelected(), head, exampleKey);
        assert.strictEqual(getMarkdown(state).trim(), applied, "it is applied");

        placeCursor(state);
        state.textManipulation.applyList(state.textManipulation.getSelected(), head, exampleKey);
        assert.strictEqual(getMarkdown(state).trim(), "hello world", "it is removed");
      },
      { args: [name, exampleKey, applied] },
    );
  }

  qtest("creates a list item for each selected line", async function (assert, { tm }) {
    const { setupEditor, setContent, selectAll, getMarkdown } = tm;
    const state = await setupEditor();
    setContent(state, "first line\nsecond line");
    selectAll(state);

    state.textManipulation.applyList(state.textManipulation.getSelected(), "* ", "list_item");

    assert.dom(".ProseMirror li").exists({ count: 2 }, "each line becomes a separate list item");
    assert.strictEqual(getMarkdown(state).trim(), "* first line\n* second line", "both lines are preserved");
  });

  qtest("switching list type is undone in one step", async function (assert, { tm, pmState, pmHistory }) {
    const { setupEditor, setContent, getMarkdown } = tm;
    const { TextSelection } = pmState;
    const { closeHistory, undo } = pmHistory;
    const state = await setupEditor();
    setContent(state, "* first\n* second");
    const { view } = state.textManipulation;
    const list = view.state.doc.firstChild;
    const from = TextSelection.atStart(list).from + 1;
    const to = TextSelection.atEnd(list).to + 1;
    view.dispatch(closeHistory(view.state.tr.setSelection(TextSelection.create(view.state.doc, from, to))));

    state.textManipulation.applyList(state.textManipulation.getSelected(), "1. ", "list_item");
    assert.strictEqual(getMarkdown(state).trim(), "1. first\n2. second", "switching preserves both items");
    assert.dom(".ProseMirror ol li").exists({ count: 2 }, "both items change type");
    undo(view.state, view.dispatch);
    assert.strictEqual(getMarkdown(state).trim(), "* first\n* second", "one undo restores the original list");
  });

  qtest("switches from ordered list to bullet list", async function (assert, { tm }) {
    const { setupEditor, setContent, placeCursor } = tm;
    const state = await setupEditor();
    setContent(state, "1. hello world");
    placeCursor(state);

    state.textManipulation.applyList(state.textManipulation.getSelected(), "* ", "list_item");

    assert.dom(".ProseMirror ul li").hasText("hello world");
    assert.dom(".ProseMirror ol").doesNotExist();
  });

  qtest("falls back to markdown round-trip for unrecognized list head", async function (assert, { tm }) {
    const { setupEditor, setContent, selectAll, getMarkdown } = tm;
    const state = await setupEditor();
    setContent(state, "hello world");
    selectAll(state);

    const sel = state.textManipulation.getSelected();
    state.textManipulation.applyList(sel, "? ", "list_item");

    assert.strictEqual(getMarkdown(state).trim(), "? hello world");
    assert.strictEqual(state.textManipulation.view.state.doc.childCount, 1, "the replaced paragraph is not left behind");
  });

  qtest("passes the previous head to function heads in the fallback", async function (assert, { tm }) {
    const { setupEditor, setContent, selectAll, getMarkdown } = tm;
    const state = await setupEditor();
    setContent(state, "first\n\nsecond");
    selectAll(state);

    let calls = 0;
    state.textManipulation.applyList(
      state.textManipulation.getSelected(),
      (previous) => {
        calls++;
        return previous === undefined ? "?1 " : `?${parseInt(previous.slice(1), 10) + 1} `;
      },
      "list_item",
    );

    assert.strictEqual(
      getMarkdown(state).trim(),
      "?1 first\n\n?2 second",
      "each head derives from the previous one and blank lines do not advance it",
    );
    assert.strictEqual(calls, 2, "the head is computed once per line");
  });

  for (const [head, exampleKey, selector, text] of [
    ["* [x] ", "list_item", "li", "[x] hello world"],
    ["> prefix ", "blockquote_text", "blockquote", "prefix hello world"],
  ]) {
    qtest(
      `preserves additional content in the "${head}" prefix`,
      async function (assert, { tm }, [head, exampleKey, selector, text]) {
        const { setupEditor, setContent, selectAll } = tm;
        const state = await setupEditor();
        setContent(state, "hello world");
        selectAll(state);

        state.textManipulation.applyList(state.textManipulation.getSelected(), head, exampleKey);

        assert.dom(`.ProseMirror ${selector}`).hasText(text, "the prefix content is retained");
      },
      // The test site's markdown has the checklist rule, which the
      // reference's core bundle lacks: "[x]" stays text without it.
      { args: [head, exampleKey, selector, text], render: { checklist: false } },
    );
  }

  qtest("preserves the starting number of an ordered list", async function (assert, { tm }) {
    const { setupEditor, setContent, selectAll } = tm;
    const state = await setupEditor();
    setContent(state, "hello world");
    selectAll(state);

    state.textManipulation.applyList(state.textManipulation.getSelected(), "3. ", "list_item");

    assert.dom(".ProseMirror ol").hasAttribute("start", "3", "the parsed starting number is retained");
  });

  qtest("selects the inserted placeholder rather than an earlier copy of its text", async function (assert, { tm, pmState }) {
    const { setupEditor, setContent } = tm;
    const { TextSelection } = pmState;
    const state = await setupEditor();
    setContent(state, "List item and more");
    const { view } = state.textManipulation;
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 19)));

    state.textManipulation.applyList(state.textManipulation.getSelected(), "? ", "list_item");

    const { from, to } = view.state.selection;
    assert.strictEqual(view.state.doc.textBetween(from, to), "List item");
    assert.strictEqual(from, 21, "the new occurrence is selected");
  });
});

// The plugins' rich-editor-text-manipulation-test.gjs.
test.describe("TextManipulation: the plugins' markup", () => {
  for (const [name, markdown] of [
    ["paragraph", "**one**\n\ntwo"],
    ["line", "**one**\ntwo"],
  ]) {
    qtest(
      `applies a checklist prefix to each selected ${name}`,
      async function (assert, { helpers }, markdown) {
        const { setupRichEditor, settled } = helpers;
        let textManipulation;
        await setupRichEditor(assert, markdown, {
          onSetup: (value) => (textManipulation = value),
        });
        const { view } = textManipulation;
        textManipulation.selectText(1, view.state.doc.content.size - 2);
        textManipulation.applyList(textManipulation.getSelected(), "* [ ] ", "list_item", { multiline: true });
        await settled();
        assert.dom(".ProseMirror li .chcklst-box").exists({ count: 2 }, "each item has a checkbox");
        assert.dom(".ProseMirror li strong").hasText("one", "formatting is retained");
        assert.dom(".ProseMirror li:last-child p").hasText("two", "the second paragraph is retained");
      },
      { plugins: ["checklist"], args: markdown },
    );
  }

  qtest(
    "surrounds selected text with an editable footnote",
    async function (assert, { helpers }) {
      const { setupRichEditor, settled } = helpers;
      let textManipulation;
      const onSetup = (value) => {
        textManipulation = value;
      };
      await setupRichEditor(assert, "note", { onSetup });
      textManipulation.selectText(1, 4);
      textManipulation.applySurroundSelection("^[", "]", "wrap_text", { multiline: false });
      await settled();
      assert.dom(".footnote").exists("a footnote is inserted");
      assert.strictEqual(
        textManipulation.view.state.selection.node && textManipulation.view.state.selection.node.type.name,
        "footnote",
        "the atomic node is selected to open its editor",
      );
      assert.dom(".footnote-tooltip .ProseMirror").hasText("note", "the selected text can be edited in the footnote");
    },
    { plugins: ["footnote"] },
  );

  qtest(
    "keeps text around an existing footnote selected after surrounding",
    async function (assert, { helpers }) {
      const { setupRichEditor, settled } = helpers;
      let textManipulation;
      const onSetup = (value) => {
        textManipulation = value;
      };
      await setupRichEditor(assert, "before ^[note] after", { onSetup });
      const { view } = textManipulation;
      textManipulation.selectText(1, view.state.doc.content.size - 2);
      textManipulation.applySurroundSelection("<small>", "</small>", "wrap_text", { multiline: false });
      await settled();
      assert.strictEqual(
        view.state.doc.textBetween(view.state.selection.from, view.state.selection.to).trimEnd(),
        "before note after",
        "the entire original content remains selected",
      );
      assert.dom(".footnote-tooltip").doesNotExist("formatting the range does not open the existing footnote");
    },
    { plugins: ["footnote"] },
  );
});

// prosemirror_toolbar_spec.rb and prosemirror_keymap_spec.rb on the
// composer: a new topic's, in the rich editor (composition_mode 1).
test.describe("the composer's toolbar in the rich editor", () => {
  const rich = (page) => page.locator("#reply-control .ProseMirror.d-editor-input");
  const button = (page, name) => page.locator(`#reply-control .d-editor-button-bar .toolbar__button.${name}`);
  const menuOption = (page, menu, name) =>
    page.locator(`.toolbar-menu__${menu}-content:not(.-closing) [data-name='${name}']`);
  const mod = process.platform === "darwin" ? "Meta" : "Control";

  async function openComposer(page, context) {
    const writer = member();
    sql("UPDATE user_options SET composition_mode = 1 WHERE user_id = :'p1'", writer.id);
    await login(context, writer.name);
    await page.goto("/latest");
    await page.locator("#create-topic").click();
    await expect(page.locator("#reply-control.open")).toBeVisible();
    await expect(rich(page)).toBeVisible();
    await rich(page).click();
    await expect(rich(page)).toBeFocused();
    return writer;
  }

  async function expandMenu(page, menu) {
    await button(page, menu).click();
    await expect(page.locator(`.toolbar-menu__${menu}-content:not(.-closing)`)).toBeVisible();
  }

  test("updates the toolbar state following the cursor position", async ({ page, context }) => {
    await openComposer(page, context);
    for (const name of ["bold", "italic", "heading", "link", "list", "code", "blockquote"]) {
      await expect(button(page, `${name}.--active`)).toHaveCount(0);
    }

    await page.keyboard.type("> - [***many `styles`***](https://example.com)");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("ArrowLeft");

    for (const name of ["bold", "italic", "link", "list", "code", "blockquote"]) {
      await expect(button(page, `${name}.--active`)).toHaveCount(1);
    }
  });

  test("applies a bullet and an ordered list from the list menu", async ({ page, context }) => {
    await openComposer(page, context);
    await page.keyboard.type("A list item");

    await expandMenu(page, "list");
    await menuOption(page, "list", "list-bullet").click();
    await expect(rich(page).locator("ul li")).toHaveText("A list item");
    await expect(button(page, "list.--active")).toHaveCount(1);
    await expect(button(page, "list").locator(".d-icon-list-ul")).toHaveCount(1);

    await expandMenu(page, "list");
    await expect(menuOption(page, "list", "list-bullet")).toHaveClass(/--active/);
    await menuOption(page, "list", "list-ordered").click();
    await expect(rich(page).locator("ol li")).toHaveText("A list item");
    await expect(rich(page).locator("ul")).toHaveCount(0);
    await expect(button(page, "list").locator(".d-icon-list-ol")).toHaveCount(1);
  });

  test("splits multi-line plain text into list items", async ({ page, context }) => {
    await openComposer(page, context);
    await page.keyboard.type("apple");
    await page.keyboard.press("Shift+Enter");
    await page.keyboard.type("banana");
    await page.keyboard.press("Shift+Enter");
    await page.keyboard.type("coconut");
    await page.keyboard.press(`${mod}+a`);

    await expandMenu(page, "list");
    await menuOption(page, "list", "list-bullet").click();

    await expect(rich(page).locator("ul li")).toHaveText(["apple", "banana", "coconut"]);
  });

  test("updates the heading button's state and icon with the heading level", async ({ page, context }) => {
    await openComposer(page, context);
    await page.keyboard.type("## This is a test");
    await page.keyboard.press("Enter");
    await page.keyboard.type("#### And this is another test");
    await expect(button(page, "heading.--active")).toHaveCount(1);
    await expect(button(page, "heading").locator(".d-icon-discourse-h4")).toHaveCount(1);

    await expandMenu(page, "heading");
    await expect(menuOption(page, "heading", "heading-4")).toHaveClass(/--active/);
    await page.keyboard.press("Escape");
    await expect(page.locator(".toolbar-menu__heading-content")).toHaveCount(0);

    await rich(page).locator("h2").click();
    await expect(button(page, "heading").locator(".d-icon-discourse-h2")).toHaveCount(1);

    await rich(page).click();
    await page.keyboard.press(`${mod}+a`);
    await expect(button(page, "heading.--active")).toHaveCount(0);
    await expect(button(page, "heading").locator(".d-icon-discourse-text")).toHaveCount(1);
    await expandMenu(page, "heading");
    await expect(menuOption(page, "heading", "heading-2")).not.toHaveClass(/--active/);
    await expect(menuOption(page, "heading", "heading-4")).not.toHaveClass(/--active/);
  });

  test("changes the heading level, resets it to a paragraph and inserts one on an empty line", async ({
    page,
    context,
  }) => {
    await openComposer(page, context);
    await expandMenu(page, "heading");
    await menuOption(page, "heading", "heading-2").click();
    await expect(rich(page)).toBeFocused();
    await page.keyboard.type("This is a test");
    await expect(rich(page).locator("h2", { hasText: "This is a test" })).toHaveCount(1);

    await expandMenu(page, "heading");
    await menuOption(page, "heading", "heading-3").click();
    await expect(rich(page).locator("h3")).toHaveText("This is a test");

    await expandMenu(page, "heading");
    await menuOption(page, "heading", "heading-paragraph").click();
    await expect(rich(page).locator("p", { hasText: "This is a test" })).toHaveCount(1);
    await expect(rich(page).locator("h3")).toHaveCount(0);
  });

  test("applies and removes small text, and switches between it and a heading", async ({ page, context }) => {
    await openComposer(page, context);
    await page.keyboard.type("This is a test");
    await page.keyboard.press(`${mod}+a`);
    await page.keyboard.press(`${mod}+Alt+5`);
    await expect(rich(page).locator("p small")).toHaveText("This is a test");

    await rich(page).locator("small").click();
    await expandMenu(page, "heading");
    await expect(menuOption(page, "heading", "heading-small")).toHaveClass(/--active/);
    await expect(menuOption(page, "heading", "heading-paragraph")).not.toHaveClass(/--active/);
    await menuOption(page, "heading", "heading-small").click();
    await expect(rich(page).locator("small")).toHaveCount(0);
    await expect(rich(page).locator("p", { hasText: "This is a test" })).toHaveCount(1);

    await page.keyboard.press(`${mod}+a`);
    await expandMenu(page, "heading");
    await menuOption(page, "heading", "heading-2").click();
    await expect(rich(page).locator("h2", { hasText: "This is a test" })).toHaveCount(1);

    await page.keyboard.press(`${mod}+a`);
    await expandMenu(page, "heading");
    await menuOption(page, "heading", "heading-small").click();
    await expect(rich(page).locator("p small")).toHaveText("This is a test");
    await expect(rich(page).locator("h2")).toHaveCount(0);

    await page.keyboard.press(`${mod}+a`);
    await expandMenu(page, "heading");
    await menuOption(page, "heading", "heading-2").click();
    await expect(rich(page).locator("h2", { hasText: "This is a test" })).toHaveCount(1);
    await expect(rich(page).locator("small")).toHaveCount(0);
  });

  test("the buttons apply bold, italic, a blockquote and code", async ({ page, context }) => {
    await openComposer(page, context);
    await button(page, "bold").click();
    await page.keyboard.type("bold");
    await expect(rich(page).locator("strong")).toHaveText("bold");
    await button(page, "bold").click();
    await page.keyboard.type(" plain ");
    await button(page, "italic").click();
    await page.keyboard.type("leaning");
    await expect(rich(page).locator("em")).toHaveText("leaning");
    await expect(button(page, "italic.--active")).toHaveCount(1);
    await expect(button(page, "bold.--active")).toHaveCount(0);

    await page.keyboard.press("Enter");
    await page.keyboard.type("quoted");
    await button(page, "blockquote").click();
    await expect(rich(page).locator("blockquote p")).toHaveText("quoted");

    await page.keyboard.press("Enter");
    await page.keyboard.press("Enter");
    await button(page, "code").click();
    await expect(rich(page).locator("pre code")).toHaveCount(1);
    await expect(button(page, "code.--active")).toHaveCount(1);
  });

  // prosemirror_keymap_spec.rb's shortcuts
  test("the toolbar's shortcuts: bold, italic, lists, blockquote and headings", async ({ page, context }) => {
    await openComposer(page, context);
    await page.keyboard.press(`${mod}+b`);
    await page.keyboard.type("This is bold");
    await expect(rich(page).locator("strong")).toHaveText("This is bold");
    await page.keyboard.press(`${mod}+b`);
    await page.keyboard.press(`${mod}+i`);
    await page.keyboard.type(" and italic");
    await expect(rich(page).locator("em")).toHaveText("and italic");

    await page.keyboard.press("Enter");
    await page.keyboard.type("Item 1");
    await page.keyboard.press(`${mod}+Shift+7`);
    await expect(rich(page).locator("ol li")).toHaveText("Item 1");
    await page.keyboard.press(`${mod}+Shift+8`);
    await expect(rich(page).locator("ul li")).toHaveText("Item 1");
    await page.keyboard.press(`${mod}+Shift+8`);
    await expect(rich(page).locator("ul")).toHaveCount(0);

    await page.keyboard.press("Enter");
    await page.keyboard.type("This is a blockquote");
    await page.keyboard.press(`${mod}+Shift+9`);
    await expect(rich(page).locator("blockquote")).toHaveText("This is a blockquote");
    await page.keyboard.press(`${mod}+Shift+9`);
    await expect(rich(page).locator("blockquote")).toHaveCount(0);

    for (const i of [1, 2, 3, 4]) {
      await page.keyboard.press("Enter");
      await page.keyboard.type(`Heading ${i}`);
      await page.keyboard.press(`${mod}+Alt+${i}`);
      await expect(rich(page).locator(`h${i}`)).toHaveText(`Heading ${i}`);
    }
    await page.keyboard.press(`${mod}+Alt+0`);
    await expect(rich(page).locator("h4")).toHaveCount(0);
  });

  test("lets the member nest and lift list items with Tab", async ({ page, context }) => {
    await openComposer(page, context);
    await page.keyboard.type("* First");
    await page.keyboard.press("Enter");
    await page.keyboard.type("Second");

    await page.keyboard.press("Tab");
    await expect(rich(page).locator("ul > li > ul > li")).toHaveText("Second");
    await expect(rich(page)).toBeFocused();

    await page.keyboard.press("Shift+Tab");
    await expect(rich(page).locator("ul > li > ul")).toHaveCount(0);
    await expect(rich(page).locator("ul > li")).toHaveText(["First", "Second"]);
  });

  test("the menus and buttons act on the textarea in markdown mode", async ({ page, context }) => {
    await openComposer(page, context);
    await page.locator("#reply-control .composer-toggle-switch").click();
    const textarea = page.locator("#reply-control textarea.d-editor-input");
    await expect(textarea).toBeVisible();
    await expect(button(page, "heading").locator(".d-icon-discourse-text")).toHaveCount(1);
    await textarea.fill("A title");
    await textarea.press(`${mod}+a`);
    await expandMenu(page, "heading");
    await menuOption(page, "heading", "heading-2").click();
    await expect(textarea).toHaveValue("## A title");
    await textarea.press("End");
    await textarea.press("Enter");
    await textarea.press("Enter");
    await textarea.pressSequentially("first");
    await textarea.press("Shift+Home");
    await expandMenu(page, "list");
    await menuOption(page, "list", "list-ordered").click();
    await expect(textarea).toHaveValue("## A title\n\n1. first");
    await textarea.press("End");
    await textarea.press("Enter");
    await textarea.press("Enter");
    await textarea.pressSequentially("strong");
    await textarea.press("Shift+Home");
    await textarea.press(`${mod}+b`);
    await expect(textarea).toHaveValue("## A title\n\n1. first\n\n**strong**");
    await expect(page.locator("#reply-control .d-editor-preview strong")).toHaveText("strong");
  });
});

// UpsertHyperlink: composer-hyperlink-test.js on the markdown editor, and
// prosemirror_links_spec.rb's and prosemirror_keymap_spec.rb's link cases
// in the rich editor.
test.describe("the toolbar's link modal", () => {
  const modal = (page) => page.locator(".d-modal.upsert-hyperlink-modal");
  const linkButton = (page) => page.locator("#reply-control .d-editor-button-bar .toolbar__button.link");
  const rich = (page) => page.locator("#reply-control .ProseMirror.d-editor-input");
  const textarea = (page) => page.locator("#reply-control textarea.d-editor-input");
  const mod = process.platform === "darwin" ? "Meta" : "Control";

  async function openComposer(page, context, mode) {
    const writer = member();
    sql("UPDATE user_options SET composition_mode = :'p1' WHERE user_id = :'p2'", mode, writer.id);
    await login(context, writer.name);
    await page.goto("/latest");
    await page.locator("#create-topic").click();
    await expect(page.locator("#reply-control.open")).toBeVisible();
    if (mode === 1) {
      await expect(rich(page)).toBeVisible();
      await rich(page).click();
      await expect(rich(page)).toBeFocused();
    }
  }

  test("adds a hyperlink to a reply in the markdown editor", async ({ page, context }) => {
    await openComposer(page, context, 0);
    await textarea(page).fill("This is a link to ");
    await expect(page.locator(".insert-link.d-modal__body")).toHaveCount(0);

    await linkButton(page).click();
    await expect(page.locator(".insert-link.d-modal__body")).toBeVisible();
    await expect(modal(page).locator(".d-modal__title-text")).toContainText("Insert link");
    await modal(page).locator(".link-url").fill("google.com");
    await modal(page).locator(".link-text").fill("Google");
    await modal(page).locator(".d-modal__footer .btn-primary").click();
    await expect(textarea(page)).toHaveValue("This is a link to [Google](https://google.com)");
    await expect(modal(page)).toHaveCount(0);

    await textarea(page).fill("Reset textarea contents.");
    await linkButton(page).click();
    await modal(page).locator(".link-url").fill("google.com");
    await modal(page).locator(".link-text").fill("Google");
    await modal(page).locator(".d-modal__footer .btn-transparent").click();
    await expect(modal(page)).toHaveCount(0);
    await expect(textarea(page)).toHaveValue("Reset textarea contents.");

    await textarea(page).evaluate((el) => el.setSelectionRange(0, 5));
    await linkButton(page).click();
    await expect(modal(page).locator(".link-text")).toHaveCount(0);
    await modal(page).locator(".link-url").fill("somelink.com");
    await modal(page).locator(".d-modal__footer .btn-primary").click();
    await expect(textarea(page)).toHaveValue("[Reset](https://somelink.com) textarea contents.");
  });

  test("searches topics from the url field and picks one with the keys", async ({ page, context }) => {
    await page.route(/\/search\/query\.json\?/, (route) => {
      const term = new URL(route.request().url()).searchParams.get("term");
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          topics: [{ id: 280, slug: "internationalization-localization", title: `About ${term}`, fancy_title: `About ${term}` }],
        }),
      });
    });
    await openComposer(page, context, 0);
    await textarea(page).fill("");
    await linkButton(page).click();
    const url = modal(page).locator(".link-url");
    await expect(url).toBeFocused();
    await url.fill("http://google.com");
    await expect(modal(page).locator(".internal-link-results")).toBeHidden();

    await url.fill("local");
    await expect(modal(page).locator(".internal-link-results .search-link")).toHaveText("About local");
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    await expect(modal(page).locator(".internal-link-results")).toBeHidden();
    await expect(url).toHaveValue(/http.*\/t\/internationalization-localization\/280$/);
    await expect(modal(page).locator(".link-text")).toHaveValue("About local");

    await page.keyboard.press("Escape");
    await expect(modal(page)).toHaveCount(0);
    await expect(textarea(page)).toBeFocused();
    await expect(textarea(page)).toHaveValue("");
  });

  test("keeps percent-encoded characters and encodes loose ones", async ({ page, context }) => {
    await openComposer(page, context, 0);
    await linkButton(page).click();
    await modal(page).locator(".link-url").fill("https://example.com/?q=%3D+test");
    await modal(page).locator(".link-text").fill("Link");
    await modal(page).locator(".d-modal__footer .btn-primary").click();
    await expect(textarea(page)).toHaveValue("[Link](https://example.com/?q=%3D+test)");
  });

  test("Ctrl+K opens it in the rich editor and inserts the link", async ({ page, context }) => {
    await openComposer(page, context, 1);
    await page.keyboard.press(`${mod}+k`);
    await expect(modal(page)).toBeVisible();
    await expect(modal(page).locator(".link-url")).toBeFocused();
    await page.keyboard.type("https://www.example.com");
    await page.keyboard.press("Tab");
    await page.keyboard.type("This is a link");
    await page.keyboard.press("Enter");
    await expect(modal(page)).toHaveCount(0);
    await expect(rich(page).locator("a")).toHaveText("This is a link");
    await expect(rich(page).locator("a")).toHaveAttribute("href", "https://www.example.com");
    await expect(rich(page)).toBeFocused();
  });

  test("the link toolbar's Edit updates the link, its url escaped", async ({ page, context }) => {
    await openComposer(page, context, 1);
    await page.keyboard.type("[Example](https://example.com)");
    await expect(rich(page).locator("a")).toHaveText("Example");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("ArrowLeft");
    const toolbar = page.locator('[data-identifier="composer-link-toolbar"]:not(.-closing)');
    await expect(toolbar).toBeVisible();
    await page.keyboard.press("Tab");
    await expect(toolbar.locator(".composer-link-toolbar__edit")).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(modal(page)).toBeVisible();
    await expect(modal(page).locator(".d-modal__title-text")).toContainText("Edit link");
    await expect(modal(page).locator(".link-text")).toHaveValue("Example");
    await expect(modal(page).locator(".link-url")).toHaveValue("https://example.com");
    await modal(page).locator(".link-text").fill("Updated Example");
    await modal(page).locator(".link-url").fill("https://updated-example.com?query=with space");
    await modal(page).locator(".link-text").press("Enter");
    await expect(rich(page).locator("a[href='https://updated-example.com/?query=with%20space']")).toHaveText(
      "Updated Example",
    );
    await page.locator("#reply-control .composer-toggle-switch").click();
    await expect(textarea(page)).toHaveValue("[Updated Example](https://updated-example.com/?query=with%20space)");
  });

  test("inserts a link with a malformed url in the rich editor, edited without decoding", async ({ page, context }) => {
    await openComposer(page, context, 1);
    await linkButton(page).click();
    await modal(page).locator(".link-text").fill("Encoded URL");
    await modal(page).locator(".link-url").fill("https://example.com/100%/working 1");
    await modal(page).locator(".d-modal__footer .btn-primary").click();
    await expect(rich(page).locator("a[href='https://example.com/100%25/working%201']")).toHaveText("Encoded URL");

    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("ArrowLeft");
    await page.locator('[data-identifier="composer-link-toolbar"]:not(.-closing) .composer-link-toolbar__edit').click();
    await expect(modal(page).locator(".link-text")).toHaveValue("Encoded URL");
    await expect(modal(page).locator(".link-url")).toHaveValue("https://example.com/100%25/working%201");
    await modal(page).locator(".d-modal__footer .btn-transparent").click();
    await expect(modal(page)).toHaveCount(0);
    await page.locator("#reply-control .composer-toggle-switch").click();
    await expect(textarea(page)).toHaveValue("[Encoded URL](https://example.com/100%25/working%201)");
  });
});

// enable-emoji.js' button and the composer's upload button, in both
// editors: the picked emoji goes in at the selection (emojiSelected).
test.describe("the toolbar's emoji and upload buttons", () => {
  const rich = (page) => page.locator("#reply-control .ProseMirror.d-editor-input");
  const textarea = (page) => page.locator("#reply-control textarea.d-editor-input");
  const button = (page, name) => page.locator(`#reply-control .d-editor-button-bar .toolbar__button.${name}`);

  async function openComposer(page, context, mode) {
    const writer = member();
    sql("UPDATE user_options SET composition_mode = :'p1' WHERE user_id = :'p2'", mode, writer.id);
    await login(context, writer.name);
    await page.goto("/latest");
    await page.locator("#create-topic").click();
    await expect(page.locator("#reply-control.open")).toBeVisible();
    if (mode === 1) {
      await expect(rich(page)).toBeVisible();
      await rich(page).click();
      await expect(rich(page)).toBeFocused();
    }
  }

  async function pick(page, emoji) {
    const picker = page.locator(".emoji-picker");
    await expect(picker.locator(".filter-input")).toBeFocused();
    await picker.locator(".filter-input").fill(emoji);
    await picker.locator(`.emoji-picker__section.filtered img.emoji[data-emoji="${emoji}"]`).click();
    await expect(picker).toHaveCount(0);
  }

  test("inserts the picked emoji in the rich editor", async ({ page, context }) => {
    await openComposer(page, context, 1);
    await page.keyboard.type("Gone fishing");
    await expect(button(page, "emoji")).toHaveAttribute("title", "Emoji :)");
    await button(page, "emoji").click();
    await pick(page, "fishing_pole");
    await expect(rich(page).locator("img.emoji")).toHaveAttribute("alt", ":fishing_pole:");
    await expect(rich(page)).toBeFocused();
    await page.keyboard.type("today");
    await page.locator("#reply-control .composer-toggle-switch").click();
    await expect(textarea(page)).toHaveValue("Gone fishing :fishing_pole: today");
  });

  test("inserts the picked emoji in the markdown editor", async ({ page, context }) => {
    await openComposer(page, context, 0);
    await textarea(page).fill("Gone fishing");
    await button(page, "emoji").click();
    await pick(page, "fishing_pole");
    await expect(textarea(page)).toHaveValue("Gone fishing :fishing_pole:");
  });

  test("the upload button opens the file picker in the rich editor", async ({ page, context }) => {
    await openComposer(page, context, 1);
    const chooser = page.waitForEvent("filechooser");
    await button(page, "upload").click();
    expect((await chooser).isMultiple()).toBe(true);
  });
});

// The options menu over the open editor's toolbar event: the plugins'
// items (composer_spoiler_spec.rb's toolbar cases, details, footnote),
// toggle-wrap (prosemirror_wrap_spec.rb's toolbar case), and checklist's
// list menu item.
test.describe("the toolbar's options menu", () => {
  const rich = (page) => page.locator("#reply-control .ProseMirror.d-editor-input");
  const textarea = (page) => page.locator("#reply-control textarea.d-editor-input");
  const mod = process.platform === "darwin" ? "Meta" : "Control";

  async function openComposer(page, context, mode) {
    const writer = member();
    sql("UPDATE user_options SET composition_mode = :'p1' WHERE user_id = :'p2'", mode, writer.id);
    await login(context, writer.name);
    await page.goto("/latest");
    await page.locator("#create-topic").click();
    await expect(page.locator("#reply-control.open")).toBeVisible();
    if (mode === 1) {
      await expect(rich(page)).toBeVisible();
      await rich(page).click();
      await expect(rich(page)).toBeFocused();
    }
  }

  const options = (page) => page.locator(".toolbar-menu__options-content:not(.-closing)");

  async function option(page, title) {
    await page.locator("#reply-control .toolbar-menu__options-trigger").click();
    await expect(options(page)).toBeVisible();
    await options(page).locator(`button[title='${title}']`).click();
    await expect(options(page)).toHaveCount(0);
    await expect(page.locator("#reply-control .d-editor-input:not([hidden])")).toBeFocused();
  }

  test("spoiler wraps the cursor's place inline or as a block, and removes it", async ({ page, context }) => {
    await openComposer(page, context, 1);
    await page.keyboard.type("This is a test ");
    await option(page, "Blur spoiler");
    await expect(rich(page).locator("span.spoiled")).toHaveText("This text will be blurred");
    await page.keyboard.type("spoiler");
    await expect(rich(page).locator("span.spoiled")).toHaveText("spoiler");

    await page.locator("#reply-control .toolbar-menu__options-trigger").click();
    await expect(options(page).locator("button[title='Blur spoiler']")).toHaveClass(/--active/);
    await options(page).locator("button[title='Blur spoiler']").click();
    await expect(rich(page).locator("span.spoiled")).toHaveCount(0);
    await expect(rich(page)).toContainText("This is a test spoiler");
    await expect(rich(page)).toBeFocused();

    await page.keyboard.press("End");
    await page.keyboard.press("Enter");
    await option(page, "Blur spoiler");
    await expect(rich(page).locator("div.spoiled")).toHaveText("This text will be blurred");
    await page.keyboard.type("secret");
    await expect(rich(page).locator("div.spoiled")).toHaveText("secret");
    await page.locator("#reply-control .composer-toggle-switch").click();
    await expect(textarea(page)).toHaveValue(/^This is a test spoiler\s*\n\[spoiler\]\nsecret\n\n\[\/spoiler\]\n\n$/);
  });

  test("spoiler wraps paragraphs in a block spoiler", async ({ page, context }) => {
    await openComposer(page, context, 1);
    await page.keyboard.type("First paragraph");
    await page.keyboard.press("Enter");
    await page.keyboard.type("Second paragraph");
    await page.keyboard.press(`${mod}+a`);
    await option(page, "Blur spoiler");
    await expect(rich(page).locator("div.spoiled p")).toHaveText(["First paragraph", "Second paragraph"]);
    await page.locator("#reply-control .composer-toggle-switch").click();
    await expect(textarea(page)).toHaveValue("[spoiler]\nFirst paragraph\n\nSecond paragraph\n\n[/spoiler]\n\n");
  });

  test("details and a footnote go in through the rich editor", async ({ page, context }) => {
    await openComposer(page, context, 1);
    await page.keyboard.type("Some text");
    await option(page, "Add footnote");
    await expect(rich(page).locator(".footnote")).toHaveCount(1);
    await page.keyboard.press("End");
    await page.keyboard.press("Enter");
    await option(page, "Hide details");
    await expect(rich(page).locator("details summary")).toHaveText("Summary");
    await expect(rich(page).locator("details p")).toHaveText("This text will be hidden");
    await page.locator("#reply-control .composer-toggle-switch").click();
    await expect(textarea(page)).toHaveValue(
      'Some text^[Footnotes]\n\n[details="Summary"]\nThis text will be hidden\n\n[/details]\n\n',
    );
  });

  test("details goes in around the textarea's selection", async ({ page, context }) => {
    await openComposer(page, context, 0);
    await textarea(page).fill("hidden words");
    await textarea(page).press(`${mod}+a`);
    await option(page, "Hide details");
    await expect(textarea(page)).toHaveValue('\n[details="Summary"]\nhidden words\n[/details]\n');
  });

  test("toggle-wrap edits the wrap the cursor is in", async ({ page, context }) => {
    await openComposer(page, context, 1);
    await page.keyboard.type("[wrap=example class=test]");
    await expect(rich(page).locator(".composer-wrap-node")).toHaveCount(1);
    await rich(page).locator(".composer-wrap-node").click();
    await page.locator("#reply-control .toolbar-menu__options-trigger").click();
    await expect(options(page).locator("[data-name='toggle-wrap']")).toHaveClass(/--active/);
    await options(page).locator("[data-name='toggle-wrap']").click();
    const modal = page.locator(".wrap-attributes-modal");
    await expect(modal).toBeVisible();
    await expect(modal.getByRole("textbox", { name: "Wrap name" })).toHaveValue("example");
    await expect(modal.getByRole("textbox", { name: "Key" })).toHaveValue("class");
    await expect(modal.getByRole("textbox", { name: "Value" })).toHaveValue("test");
    await modal.getByRole("textbox", { name: "Wrap name" }).fill("updated");
    await modal.getByRole("textbox", { name: "Value" }).fill("highlight");
    await modal.getByRole("button", { name: "Apply" }).click();
    await expect(modal).toHaveCount(0);
    await expect(rich(page).locator(".composer-wrap-node")).toHaveCount(1);
    await page.locator("#reply-control .composer-toggle-switch").click();
    await expect(textarea(page)).toHaveValue("[wrap=updated class=highlight]\n\n[/wrap]\n\n");
  });

  test("the list menu's checklist item makes a checklist", async ({ page, context }) => {
    await openComposer(page, context, 1);
    await page.keyboard.type("An item");
    await page.locator("#reply-control .toolbar__button.list").click();
    const menu = page.locator(".toolbar-menu__list-content:not(.-closing)");
    await menu.locator("[data-name='list-checklist']").click();
    await expect(rich(page).locator("li .chcklst-box")).toHaveCount(1);
    await expect(page.locator("#reply-control .toolbar__button.list .d-icon-list-check")).toHaveCount(1);
    await page.locator("#reply-control .composer-toggle-switch").click();
    await expect(textarea(page)).toHaveValue("* [ ] An item");
  });
});

// The onebox extension and its toolbar (prosemirror_onebox_spec.rb and
// prosemirror_onebox_toolbar_spec.rb), the link toolbar's Load preview:
// /onebox and /inline-onebox answered as the reference's stubbed sites
// cook them.
test.describe("oneboxes in the rich editor", () => {
  const rich = (page) => page.locator("#reply-control .ProseMirror.d-editor-input");
  const textarea = (page) => page.locator("#reply-control textarea.d-editor-input");
  const toolbar = (page) => page.locator('[data-identifier="composer-onebox-toolbar"]:not(.-closing)');
  const titles = { "example.com": "Example Site 1", "example2.com": "Example Site 2" };
  const titleOf = (url) => titles[new URL(url).hostname];

  async function openComposer(page, context) {
    await page.route(/\/onebox\?/, (route) => {
      const url = new URL(route.request().url()).searchParams.get("url");
      route.fulfill({
        status: 200,
        contentType: "text/html",
        body: `<aside class="onebox allowlistedgeneric" data-onebox-src="${url}"><article class="onebox-body"><h3><a href="${url}">${titleOf(url)}</a></h3><p>This is an example site</p></article></aside>`,
      });
    });
    await page.route(/\/inline-onebox\?/, (route) => {
      const urls = new URL(route.request().url()).searchParams.getAll("urls[]");
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ "inline-oneboxes": urls.map((url) => ({ url, title: titleOf(url) })) }),
      });
    });
    const writer = member();
    sql("UPDATE user_options SET composition_mode = 1 WHERE user_id = :'p1'", writer.id);
    await login(context, writer.name);
    await page.goto("/latest");
    await page.locator("#create-topic").click();
    await expect(rich(page)).toBeVisible();
    await rich(page).click();
    await expect(rich(page)).toBeFocused();
  }

  test("an inline onebox for a link within text", async ({ page, context }) => {
    await openComposer(page, context);
    await page.keyboard.type("Check out this link https://example.com/x ");
    await expect(rich(page).locator("a.inline-onebox[href='https://example.com/x']")).toHaveText("Example Site 1");
    await page.keyboard.type("in the middle of text");
    await page.locator("#reply-control .composer-toggle-switch").click();
    await expect(textarea(page)).toHaveValue("Check out this link https://example.com/x in the middle of text");
  });

  test("a full onebox for a link alone on its line, none in a code block", async ({ page, context }) => {
    await openComposer(page, context);
    await page.keyboard.type("https://example.com");
    await page.keyboard.press("Enter");
    const onebox = rich(page).locator("div.onebox-wrapper[data-onebox-src='https://example.com']");
    await expect(onebox).toContainText("Example Site 1");
    await expect(onebox).toContainText("This is an example site");
    await expect(rich(page)).toBeFocused();
    await page.keyboard.type("```");
    await page.keyboard.type("https://example2.com");
    await expect(rich(page).locator("pre code")).toHaveText("https://example2.com");
    await expect(rich(page).locator("div.onebox-wrapper")).toHaveCount(1);
    await page.locator("#reply-control .composer-toggle-switch").click();
    await expect(textarea(page)).toHaveValue("https://example.com\n\n```\nhttps://example2.com\n```");
  });

  test("the full onebox's toolbar removes the preview, and Load preview brings it back", async ({ page, context }) => {
    await openComposer(page, context);
    await page.keyboard.type("https://example.com");
    await page.keyboard.press("Enter");
    await expect(rich(page).locator("div.onebox-wrapper")).toHaveCount(1);
    await rich(page).locator("div.onebox-wrapper").click();
    await expect(toolbar(page).locator(".composer-onebox-toolbar__copy")).toBeVisible();
    await expect(toolbar(page).locator("a.composer-onebox-toolbar__visit")).toHaveAttribute("href", "https://example.com");
    await toolbar(page).locator(".composer-onebox-toolbar__remove-preview").click();
    await expect(rich(page).locator("div.onebox-wrapper")).toHaveCount(0);
    await expect(rich(page).locator("a[href='https://example.com']")).toHaveText("https://example.com");

    // Not taken again once the cursor moves off it.
    await page.keyboard.press("Enter");
    await page.keyboard.type("some more text");
    await expect(rich(page).locator("div.onebox-wrapper")).toHaveCount(0);
    await page.locator("#reply-control .composer-toggle-switch").click();
    await expect(textarea(page)).toHaveValue("<https://example.com>\n\nsome more text");
    await page.locator("#reply-control .composer-toggle-switch").click();

    await rich(page).locator("a[href='https://example.com']").click();
    const linkToolbar = page.locator('[data-identifier="composer-link-toolbar"]:not(.-closing)');
    await linkToolbar.locator(".composer-link-toolbar__show-preview").click();
    await expect(rich(page).locator("div.onebox-wrapper")).toContainText("Example Site 1");
  });

  test("the inline onebox's toolbar removes the preview, and Load preview brings it back", async ({ page, context }) => {
    await openComposer(page, context);
    await page.keyboard.type("Check https://example.com/x ");
    await expect(rich(page).locator("a.inline-onebox")).toHaveText("Example Site 1");
    await rich(page).locator("a.inline-onebox").click();
    await expect(toolbar(page).locator(".composer-onebox-toolbar__copy")).toBeVisible();
    await toolbar(page).locator(".composer-onebox-toolbar__remove-preview").click();
    await expect(rich(page).locator("a.inline-onebox")).toHaveCount(0);
    await expect(rich(page).locator("a[href='https://example.com/x']")).toHaveText("https://example.com/x");

    await rich(page).locator("a[href='https://example.com/x']").click();
    const linkToolbar = page.locator('[data-identifier="composer-link-toolbar"]:not(.-closing)');
    await linkToolbar.locator(".composer-link-toolbar__show-preview").click();
    await expect(rich(page).locator("a.inline-onebox")).toHaveText("Example Site 1");
  });
});

// onebox-test.js, run as written in the page; create-pretender's /onebox
// and /inline-onebox answers are Playwright routes. Its first case (the
// markdown-it feature left out) is the node tree's contract
// (docs/rich-editor.md), tested in tree.rs.
async function pretenderOneboxes(page, opts = {}) {
  await page.route(/\/inline-onebox\?/, async (route) => {
    if (opts.inlineGate) {
      await page.evaluate(() => window.oneboxGate);
    }
    const urls = new URL(route.request().url()).searchParams.getAll("urls[]");
    const answer = opts.inline
      ? opts.inline(urls)
      : urls.includes("http://www.example.com/has-title.html")
        ? [{ url: "http://www.example.com/has-title.html", title: "This is a great title" }]
        : [];
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ "inline-oneboxes": answer }) });
  });
  await page.route(/\/onebox\?/, (route) => {
    const url = new URL(route.request().url()).searchParams.get("url");
    let body = null;
    if (opts.onebox) {
      body = opts.onebox;
    } else if (url === "http://www.example.com/has-title.html") {
      body =
        '<aside class="onebox"><article class="onebox-body"><h3><a href="http://www.example.com/article.html">An interesting article</a></h3></article></aside>';
    } else if (url === "http://www.example.com/no-title.html") {
      body = '<aside class="onebox"><article class="onebox-body"><p>No title</p></article></aside>';
    }
    route.fulfill(body ? { status: 200, contentType: "text/html", body } : { status: 404, body: "" });
  });
}

test.describe("the onebox extension", () => {
  const before = (opts) => (page) => pretenderOneboxes(page, opts);

  qtest(
    "respects max_oneboxes_per_post site setting",
    async function (assert, { helpers, pmState }) {
      const { setupRichEditor, settledRequests } = helpers;
      const URL = "http://www.example.com/has-title.html";
      const lastParagraphStart = (doc) => {
        let pos = null;
        doc.descendants((node, nodePos) => {
          if (node.type.name === "paragraph") {
            pos = nodePos + 1;
          }
        });
        return pos;
      };
      const [editor] = await setupRichEditor(assert, "a\n\nb", { siteSettings: { max_oneboxes_per_post: 0 } });
      const { view } = editor;

      view.dispatch(view.state.tr.insertText(URL, 1, 2));
      view.dispatch(view.state.tr.setSelection(pmState.TextSelection.create(view.state.doc, lastParagraphStart(view.state.doc))));
      await settledRequests();

      assert.dom(".onebox-wrapper").doesNotExist("does not render a onebox");
      assert.dom("a.inline-onebox").doesNotExist("does not render an inline onebox");
    },
    { before: before() },
  );

  qtest(
    "each copy of a repeated onebox takes a slot",
    async function (assert, { helpers, pmState }) {
      const { setupRichEditor, settledRequests } = helpers;
      const URL = "http://www.example.com/has-title.html";
      const lastParagraphStart = (doc) => {
        let pos = null;
        doc.descendants((node, nodePos) => {
          if (node.type.name === "paragraph") {
            pos = nodePos + 1;
          }
        });
        return pos;
      };
      const [editor] = await setupRichEditor(assert, `${URL}\n\n${URL}\n\nb`, { siteSettings: { max_oneboxes_per_post: 1 } });

      editor.view.dispatch(
        editor.view.state.tr.setSelection(pmState.TextSelection.create(editor.view.state.doc, lastParagraphStart(editor.view.state.doc)))
      );
      await settledRequests();

      assert.dom(".onebox-wrapper").exists({ count: 1 });
      assert.dom(`a[href="${URL}"]`).hasText(URL);
    },
    { before: before() },
  );

  qtest(
    "demotes a onebox pushed past max_oneboxes_per_post by an earlier link",
    async function (assert, { helpers, pmState }) {
      const { setupRichEditor, settledRequests } = helpers;
      const { TextSelection } = pmState;
      const URL = "http://www.example.com/has-title.html";
      const OTHER_URL = "http://www.example.com/no-title.html";
      const lastParagraphStart = (doc) => {
        let pos = null;
        doc.descendants((node, nodePos) => {
          if (node.type.name === "paragraph") {
            pos = nodePos + 1;
          }
        });
        return pos;
      };
      const [editor] = await setupRichEditor(assert, `a\n\n${OTHER_URL}\n\nb`, { siteSettings: { max_oneboxes_per_post: 1 } });
      const { view } = editor;

      view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, lastParagraphStart(view.state.doc))));
      await settledRequests();
      assert.dom(".onebox-wrapper").hasText("No title");

      const tr = view.state.tr.insertText(URL, 1, 2);
      tr.setSelection(TextSelection.atEnd(tr.doc));
      view.dispatch(tr);
      await settledRequests();

      assert.dom(".onebox-wrapper").exists({ count: 1 });
      assert.ok(document.querySelector(".onebox-wrapper").textContent.includes("An interesting article"));
      assert.dom(`a[href="${OTHER_URL}"]`).hasText(OTHER_URL);
      assert.strictEqual(editor.value, `${URL}\n\n${OTHER_URL}\n\nb`);
    },
    { before: before() },
  );

  qtest(
    "links that never cook to a onebox do not take a slot",
    async function (assert, { helpers, pmState }) {
      const { setupRichEditor, settledRequests } = helpers;
      const URL = "http://www.example.com/has-title.html";
      const lastParagraphStart = (doc) => {
        let pos = null;
        doc.descendants((node, nodePos) => {
          if (node.type.name === "paragraph") {
            pos = nodePos + 1;
          }
        });
        return pos;
      };
      const [editor] = await setupRichEditor(assert, `# http://www.example.com\n\nfoo@example.com\n\n${URL}\n\nb`, {
        siteSettings: { max_oneboxes_per_post: 1 },
      });

      editor.view.dispatch(
        editor.view.state.tr.setSelection(pmState.TextSelection.create(editor.view.state.doc, lastParagraphStart(editor.view.state.doc)))
      );
      await settledRequests();

      assert.dom(".onebox-wrapper").exists({ count: 1 });
      assert.ok(document.querySelector(".onebox-wrapper").textContent.includes("An interesting article"));
    },
    { before: before() },
  );

  qtest(
    "keeps marks when demoting an inline onebox",
    async function (assert, { helpers }) {
      const { setupRichEditor, settledRequests } = helpers;
      const URL = "http://www.example.com/has-title.html";
      const [editor] = await setupRichEditor(assert, "see x here", { siteSettings: { max_oneboxes_per_post: 0 } });
      const { view } = editor;
      const { schema } = view.state;

      view.dispatch(
        view.state.tr.replaceWith(5, 6, [
          schema.nodes.onebox_inline.create({ url: URL, title: "Title" }, null, [schema.marks.strong.create()]),
        ])
      );
      await settledRequests();

      assert.dom("a.inline-onebox").doesNotExist();
      assert.strictEqual(editor.value, `see **${URL}** here`);
    },
    { before: before() },
  );

  qtest(
    "show preview past max_oneboxes_per_post keeps the link and shows a toast",
    async function (assert, { helpers, pmState }) {
      const { setupRichEditor, settledRequests, click } = helpers;
      const URL = "http://www.example.com/has-title.html";
      const lastParagraphStart = (doc) => {
        let pos = null;
        doc.descendants((node, nodePos) => {
          if (node.type.name === "paragraph") {
            pos = nodePos + 1;
          }
        });
        return pos;
      };
      const [editor] = await setupRichEditor(assert, `a\n\n<${URL}>`, { siteSettings: { max_oneboxes_per_post: 0 } });
      editor.view.focus();
      editor.view.dispatch(
        editor.view.state.tr.setSelection(
          pmState.TextSelection.create(editor.view.state.doc, lastParagraphStart(editor.view.state.doc) + 5)
        )
      );
      await settledRequests();

      await click(".composer-link-toolbar__show-preview");

      assert.dom(".onebox-wrapper").doesNotExist();
      assert.strictEqual(editor.value, `a\n\n<${URL}>`);
      assert.dom(".fk-d-toasts .fk-d-toast").exists({ count: 1 });
    },
    { before: before() },
  );

  qtest(
    "renders an inline onebox when the cursor moves while it loads",
    async function (assert, { helpers, pmState }) {
      const { setupRichEditor, settledRequests } = helpers;
      const { TextSelection } = pmState;
      let release;
      window.oneboxGate = new Promise((resolve) => (release = resolve));
      const wait = () => new Promise((resolve) => setTimeout(resolve, 300));

      const url = "http://www.example.com/loads-slowly.html";
      const [editor] = await setupRichEditor(assert, "see x here\n\nb");
      const { view } = editor;

      const tr = view.state.tr.insertText(url, 5, 6);
      tr.setSelection(TextSelection.atEnd(tr.doc));
      view.dispatch(tr);
      await wait();

      view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 2)));
      await wait();

      release();
      await settledRequests();

      assert.dom("a.inline-onebox").hasText("Loaded title");
    },
    {
      before: before({
        inlineGate: true,
        inline: (urls) => urls.map((url) => ({ url, title: "Loaded title" })),
      }),
    },
  );

  qtest(
    "saves an inline onebox URL unescaped",
    async function (assert, { helpers }) {
      const { setupRichEditor, settledRequests } = helpers;
      const url = "http://www.example.com/~user/has-title.html";
      const [editor] = await setupRichEditor(assert, "see x here");
      const { view } = editor;

      view.dispatch(view.state.tr.replaceWith(5, 6, view.state.schema.nodes.onebox_inline.create({ url, title: "Title" })));
      await settledRequests();

      assert.strictEqual(editor.value, `see ${url} here`);
    },
    { before: before() },
  );

  qtest(
    "saves the bracket after a bare link unescaped",
    async function (assert, { helpers }) {
      const { setupRichEditor } = helpers;
      const OTHER_URL = "http://www.example.com/no-title.html";
      const [editor] = await setupRichEditor(assert, `see [${OTHER_URL}] here`);

      assert.strictEqual(editor.value, `see \\[${OTHER_URL}] here`);
    },
    { before: before({ inline: () => [] }) },
  );

  qtest(
    "ignores nested editor states",
    async function (assert, { helpers, pmState }) {
      const { setupRichEditor } = helpers;
      const { EditorState, TextSelection } = pmState;
      const { oneboxPluginKey } = await import("discourse/rich-editor/extensions/onebox");
      const URL = "http://www.example.com/has-title.html";
      const [editor] = await setupRichEditor(assert, `> ${URL}\n>\n> b`);
      const { view } = editor;

      const inner = EditorState.create({ doc: view.state.doc.firstChild, plugins: view.state.plugins });
      const tr = inner.tr.insertText("c", inner.doc.content.size - 1);
      tr.setSelection(TextSelection.atEnd(tr.doc));
      const { state } = inner.applyTransaction(tr);

      assert.strictEqual(oneboxPluginKey.getState(state).find().length, 0);
    },
    { before: before() },
  );

  qtest(
    "holds a lone link inline while the cursor is on its line, and promotes it when the cursor leaves",
    async function (assert, { helpers, pmState }) {
      const { setupRichEditor, settledRequests } = helpers;
      const { TextSelection } = pmState;
      const URL = "http://www.example.com/has-title.html";
      const lastParagraphStart = (doc) => {
        let pos = null;
        doc.descendants((node, nodePos) => {
          if (node.type.name === "paragraph") {
            pos = nodePos + 1;
          }
        });
        return pos;
      };
      const [editor] = await setupRichEditor(assert, "a\n\nb");
      const { view } = editor;

      const text = `${URL} `;
      const tr = view.state.tr.insertText(text, 1, 2);
      tr.setSelection(TextSelection.create(tr.doc, 1 + text.length));
      view.dispatch(tr);
      await settledRequests();

      assert.dom("a.inline-onebox").exists("renders an inline onebox");
      assert.dom(".onebox-wrapper").doesNotExist("does not render a full onebox yet");

      view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, lastParagraphStart(view.state.doc))));
      await settledRequests();

      assert.dom(".onebox-wrapper").exists("promotes to a full onebox");
      assert.dom("a.inline-onebox").doesNotExist("is no longer an inline onebox");
      assert.strictEqual(editor.value, `${URL} \n\nb`, "leaves the markdown the user typed alone");
    },
    { before: before() },
  );

  qtest(
    "a top-level URL alone on its line becomes a full onebox with no empty paragraph before it",
    async function (assert, { helpers }) {
      const { setupRichEditor, settledRequests } = helpers;
      const topLevelUrl = "http://www.example.com";
      const [editor] = await setupRichEditor(assert, "x");
      const { view } = editor;

      view.dispatch(view.state.tr.insertText(`${topLevelUrl} `, 1, 2));
      await settledRequests();
      assert.dom(".onebox-wrapper").doesNotExist("stays a plain link while the cursor is on the line");

      view.dispatch(view.state.tr.split(view.state.selection.from));
      await settledRequests();

      assert.dom(".onebox-wrapper").exists("becomes a full onebox");
      assert.strictEqual(view.state.doc.firstChild.type.name, "onebox", "the onebox is the first node, no empty paragraph before it");
    },
    {
      before: before({
        onebox:
          '<aside class="onebox"><article class="onebox-body"><h3><a href="http://www.example.com">Example</a></h3></article></aside>',
      }),
    },
  );

  qtest(
    "a URL followed by a hard break and text splits into onebox + paragraph",
    async function (assert, { helpers, pmState }) {
      const { setupRichEditor, settledRequests } = helpers;
      const { NodeSelection, TextSelection } = pmState;
      const URL = "http://www.example.com/has-title.html";
      const [editor] = await setupRichEditor(assert, "x");
      const { view } = editor;
      const { schema } = view.state;

      const linkMark = schema.marks.link.create({ href: URL, markup: "linkify" });
      const tr = view.state.tr.replaceWith(1, 2, [schema.text(URL, [linkMark]), schema.nodes.hard_break.create(), schema.text("asd")]);
      tr.setSelection(TextSelection.create(tr.doc, 1 + URL.length + 1));
      view.dispatch(tr);
      await settledRequests();

      assert.dom(".onebox-wrapper").exists("becomes a full onebox");
      assert.strictEqual(view.state.doc.firstChild.type.name, "onebox", "the onebox is the first node, no empty paragraph before it");
      assert.strictEqual(editor.value, `${URL}\nasd`, "leaves the markdown the user typed alone");
      assert.false(view.state.selection instanceof NodeSelection, "the cursor lands after the onebox, not selecting the block");
    },
    { before: before() },
  );
});
