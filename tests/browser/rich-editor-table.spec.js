// The rich editor's table extension (ROADMAP 196r2): the reference's
// frontend/discourse/tests/integration/components/prosemirror-editor/
// table-test.js and the table cases of tests/unit/lib/to-markdown-test.js,
// on the editor rich-editor-helper.js opens. Each case drives the editor in
// the page as the reference does, with ProseMirror commands and synthetic
// DOM events; window.tableSpec holds the reference's helpers.
const { test, expect } = require("./helpers");
const { openEditorPage, setupRichEditor, testMarkdown, editorValue } = require("./rich-editor-helper");

const TABLE = `| h1 | h2 | h3 |\n| --- | --- | --- |\n| a1 | a2 | a3 |\n| b1 | b2 | b3 |`;
const UNEVEN_TABLE = `| short | a much longer heading | x |\n| --- | --- | --- |\n| one | a much longer value | three |`;
const TABLE_OUT = `| h1 | h2 | h3 |\n|----|----|----|\n| a1 | a2 | a3 |\n| b1 | b2 | b3 |\n\n`;

// In the page: table-test.js's helpers.
function specHelpers() {
  const wait = (ms = 30) => new Promise((resolve) => setTimeout(resolve, ms));
  const modules = {};
  window.tableSpec = {
    wait,
    async load() {
      const [commands, grid, menu, state, history, gapcursor] = await Promise.all([
        import("discourse/rich-editor/table/commands"),
        import("discourse/rich-editor/table/grid"),
        import("discourse/rich-editor/table/menu"),
        import("prosemirror-state"),
        import("prosemirror-history"),
        import("prosemirror-gapcursor"),
      ]);
      Object.assign(modules, { commands, grid, menu, state, history, gapcursor });
      return modules;
    },
    modules,
    get view() {
      return window.richSpec.editor.view;
    },
    get value() {
      return window.richSpec.value;
    },
    find(selector) {
      return document.querySelector(selector);
    },
    findAll(selector) {
      return [...document.querySelectorAll(selector)];
    },
    locateTable(view = this.view) {
      let node = null;
      let pos = null;
      view.state.doc.descendants((child, at) => {
        if (child.type.name === "table") {
          node = child;
          pos = at;
          return false;
        }
      });
      return { node, pos, start: pos + 1, grid: modules.grid.tableGrid(node) };
    },
    cellPos(row, col) {
      const { start, grid } = this.locateTable();
      return start + grid.rows[row].cells[col].offset;
    },
    async selectCell(row, col) {
      const view = this.view;
      const pos = this.cellPos(row, col);
      view.dispatch(view.state.tr.setSelection(modules.state.TextSelection.create(view.state.doc, pos + 1)));
      await wait();
    },
    async setSelection(from, to) {
      const view = this.view;
      view.dispatch(view.state.tr.setSelection(modules.state.TextSelection.create(view.state.doc, from, to)));
      await wait();
    },
    async apply(command) {
      const view = this.view;
      const result = command(view.state, view.dispatch, view);
      await wait();
      return result;
    },
    async hoverCell(cell) {
      const rect = cell.getBoundingClientRect();
      cell.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: rect.left + 2, clientY: rect.top + 2 }));
      await wait();
    },
    async pressKey(key, modifiers = {}) {
      const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...modifiers });
      this.view.dom.dispatchEvent(event);
      await wait();
      return event.defaultPrevented;
    },
    async pressGrip(grip, { activate = false, click = activate, moveBy = 0, pointerType = "mouse" } = {}) {
      const rect = grip.getBoundingClientRect();
      const at = {
        bubbles: true,
        cancelable: true,
        button: 0,
        pointerId: 2,
        pointerType,
        isPrimary: true,
        clientX: rect.left + rect.width / 2,
        clientY: rect.top + rect.height / 2,
      };
      grip.dispatchEvent(new PointerEvent("pointerdown", at));
      const end = { ...at, clientY: at.clientY + moveBy };
      if (moveBy) {
        document.dispatchEvent(new PointerEvent("pointermove", end));
      }
      document.dispatchEvent(new PointerEvent(activate || click ? "pointerup" : "pointercancel", end));
      if (click) {
        grip.dispatchEvent(new MouseEvent("click", { ...end, detail: 1 }));
      }
      await wait();
    },
    async dragGrip(grip, target, beforeDrop) {
      const from = grip.getBoundingClientRect();
      const to = target.getBoundingClientRect();
      const at = (x, y) => ({ bubbles: true, cancelable: true, button: 0, pointerId: 1, isPrimary: true, clientX: x, clientY: y });
      grip.dispatchEvent(new PointerEvent("pointerdown", at(from.left + 1, from.top + 1)));
      document.dispatchEvent(new PointerEvent("pointermove", at(to.left + to.width / 2 + 1, to.top + to.height / 2 + 1)));
      await wait();
      const checks = beforeDrop ? await beforeDrop() : null;
      document.dispatchEvent(new PointerEvent("pointerup", at(to.left + to.width / 2 + 1, to.top + to.height / 2 + 1)));
      // The browser sends this after a short drag; a drag must not also open a menu.
      grip.dispatchEvent(new MouseEvent("click", { ...at(to.left, to.top), detail: 1 }));
      await wait();
      return checks;
    },
    async dragAppend(button, distance) {
      const rect = button.getBoundingClientRect();
      const column = button.classList.contains("--column");
      const start = { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
      const at = (x, y) => ({ bubbles: true, cancelable: true, button: 0, pointerId: 3, isPrimary: true, clientX: x, clientY: y });
      const end = column ? { x: start.x + distance, y: start.y } : { x: start.x, y: start.y + distance };
      button.dispatchEvent(new PointerEvent("pointerdown", at(start.x, start.y)));
      document.dispatchEvent(new PointerEvent("pointermove", at(end.x, end.y)));
      document.dispatchEvent(new PointerEvent("pointerup", at(end.x, end.y)));
      button.dispatchEvent(new MouseEvent("click", { ...at(end.x, end.y), detail: 1 }));
      await wait();
    },
    // The editor's table less its own chrome, which is not the document.
    tableMarkup() {
      const table = document.querySelector(".ProseMirror .composer-table table").cloneNode(true);
      table.querySelectorAll(".composer-table__grip").forEach((el) => el.remove());
      table
        .querySelectorAll(".is-current-row, .is-current-column, .is-structural-target, .--axis-row, .--axis-column")
        .forEach((cell) => cell.removeAttribute("class"));
      return table.outerHTML;
    },
    menu() {
      return document.querySelector('.fk-d-menu[data-identifier="composer-table-menu"]');
    },
    async waitFor(predicate, timeout = 2000) {
      const started = Date.now();
      while (!predicate()) {
        if (Date.now() - started > timeout) {
          throw new Error("timed out waiting");
        }
        await wait(20);
      }
    },
  };
}

async function open(page) {
  await openEditorPage(page);
  await page.addScriptTag({ content: `(${specHelpers.toString()})()` });
  await page.evaluate(() => window.tableSpec.load());
}

// In the page with the spec's helpers as `s` and its modules as `m`.
function inPage(page, fn, arg) {
  return page.evaluate(`(${fn.toString()})(window.tableSpec, window.tableSpec.modules, ${JSON.stringify(arg === undefined ? null : arg)})`);
}

test.describe("rich editor tables", () => {
  test.beforeEach(async ({ page }) => {
    await open(page);
  });

  for (const [name, [markdown, html, expectedMarkdown]] of Object.entries({
    "basic table": [
      "| Header 1 | Header 2 |\n| --- | --- |\n| Cell 1 | Cell 2 |",
      `<table><thead><tr><th>Header 1</th><th>Header 2</th></tr></thead><tbody><tr><td>Cell 1</td><td>Cell 2</td></tr></tbody></table>`,
      `| Header 1 | Header 2 |\n|----|----|\n| Cell 1 | Cell 2 |\n\n`,
    ],
    "table with alignment": [
      `| Left | Center | Right |\n| :--- | :---: | ---: |\n| A | B | C |`,
      `<table><thead><tr><th style="text-align: left;">Left</th><th style="text-align: center;">Center</th><th style="text-align: right;">Right</th></tr></thead><tbody><tr><td style="text-align: left;">A</td><td style="text-align: center;">B</td><td style="text-align: right;">C</td></tr></tbody></table>`,
      `| Left | Center | Right |\n|:---|:---:|---:|\n| A | B | C |\n\n`,
    ],
    "table with br in cells": [
      `| Line1<br>Line2 | Cell 2 |\n| --- | --- |\n| Cell 3 | Cell 4 |`,
      `<table><thead><tr><th>Line1<br>Line2</th><th>Cell 2</th></tr></thead><tbody><tr><td>Cell 3</td><td>Cell 4</td></tr></tbody></table>`,
      `| Line1<br>Line2 | Cell 2 |\n|----|----|\n| Cell 3 | Cell 4 |\n\n`,
    ],
    "table with pipes inside a link": [
      `| Link | Note |\n| --- | --- |\n| [x|y](https://example.com "title|value") | ok |`,
      `<table><thead><tr><th>Link</th><th>Note</th></tr></thead><tbody><tr><td><a href="https://example.com" title="title|value">x|y</a></td><td>ok</td></tr></tbody></table>`,
      `| Link | Note |\n|----|----|\n| [x\\|y](https://example.com "title\\|value") | ok |\n\n`,
    ],
  })) {
    test(name, async ({ page }) => {
      await testMarkdown(
        page,
        markdown,
        async () => {
          expect(await page.evaluate(() => window.tableSpec.tableMarkup()), "table markup should match").toBe(html);
        },
        expectedMarkdown
      );
    });
  }

  test("tables without text keep their emoji and images", async ({ page }) => {
    const { value } = await setupRichEditor(
      page,
      `before\n\n| :smile: | |\n| --- | --- |\n| | ![alt](/images/avatar.png) |\n\nafter`
    );
    expect(value).toBe(`before\n\n| :smile: |  |\n|----|----|\n|  | ![alt](/images/avatar.png) |\n\nafter`);
  });

  test("tables with no content are dropped", async ({ page }) => {
    const { value } = await setupRichEditor(page, `before\n\n| | |\n| --- | --- |\n| <br> | |\n\nafter`);
    expect(value).toBe(`before\n\nafter`);
  });

  test("table within quotes", async ({ page }) => {
    await testMarkdown(
      page,
      `> \n> | Header 1 | Header 2 |\n> | --- | --- |\n> | Cell 1 | Cell 2 |\n`,
      async () => {
        await expect(page.locator(".ProseMirror blockquote .composer-table table")).toHaveCount(1);
        expect(await page.evaluate(() => window.tableSpec.tableMarkup())).toBe(
          `<table><thead><tr><th>Header 1</th><th>Header 2</th></tr></thead><tbody><tr><td>Cell 1</td><td>Cell 2</td></tr></tbody></table>`
        );
      },
      `> \n> | Header 1 | Header 2 |\n> |----|----|\n> | Cell 1 | Cell 2 |\n\n`
    );
  });

  test("renders a grip for every row and column", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    await expect(page.locator(".composer-table__grip.--column")).toHaveCount(3);
    await expect(page.locator(".composer-table__grip.--row")).toHaveCount(3);
    await expect(page.locator(".composer-table__append.--column")).toHaveCount(1);
    await expect(page.locator(".composer-table__append.--row")).toHaveCount(1);
  });

  test("adds and deletes columns", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const values = await inPage(page, async (s, m) => {
      await s.selectCell(1, 1);
      await s.apply(m.commands.addColumn(1));
      const added = s.value;
      await s.selectCell(1, 2);
      await s.apply(m.commands.deleteColumn());
      return [added, s.value];
    });
    expect(values[0], "inserts an empty column after the selected one").toBe(
      `| h1 | h2 |  | h3 |\n|----|----|----|----|\n| a1 | a2 |  | a3 |\n| b1 | b2 |  | b3 |\n\n`
    );
    expect(values[1], "removes the column again").toBe(TABLE_OUT);
  });

  test("adds and deletes rows", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const values = await inPage(page, async (s, m) => {
      await s.selectCell(1, 0);
      await s.apply(m.commands.addRow(-1));
      const added = s.value;
      await s.selectCell(1, 0);
      await s.apply(m.commands.deleteRow());
      return [added, s.value];
    });
    expect(values[0]).toBe(`| h1 | h2 | h3 |\n|----|----|----|\n|  |  |  |\n| a1 | a2 | a3 |\n| b1 | b2 | b3 |\n\n`);
    expect(values[1]).toBe(TABLE_OUT);
  });

  test("deleting the header row promotes the next one", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const value = await inPage(page, async (s, m) => {
      await s.selectCell(0, 0);
      await s.apply(m.commands.deleteRow());
      return s.value;
    });
    expect(value).toBe(`| a1 | a2 | a3 |\n|----|----|----|\n| b1 | b2 | b3 |\n\n`);
  });

  test("alignment survives a header promotion", async ({ page }) => {
    await setupRichEditor(page, `| h1 | h2 |\n| --- | :---: |\n| a1 | a2 |\n| b1 | b2 |`);
    const value = await inPage(page, async (s, m) => {
      await s.selectCell(0, 0);
      await s.apply(m.commands.deleteRow());
      return s.value;
    });
    expect(value).toBe(`| a1 | a2 |\n|----|:---:|\n| b1 | b2 |\n\n`);
  });

  test("alignment travels with a moved column", async ({ page }) => {
    await setupRichEditor(page, `| h1 | h2 |\n| --- | :---: |\n| a1 | a2 |`);
    const value = await inPage(page, async (s, m) => {
      await s.selectCell(1, 0);
      await s.apply(m.commands.moveColumn(1, 0));
      return s.value;
    });
    expect(value).toBe(`| h2 | h1 |\n|:---:|----|\n| a2 | a1 |\n\n`);
  });

  test("sets alignment for the whole column", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const value = await inPage(page, async (s, m) => {
      await s.selectCell(2, 1);
      await s.apply(m.commands.setColumnAlignment("center"));
      return s.value;
    });
    expect(value).toBe(`| h1 | h2 | h3 |\n|----|:---:|----|\n| a1 | a2 | a3 |\n| b1 | b2 | b3 |\n\n`);
    await expect(page.locator(".ProseMirror .composer-table tbody tr:first-child td:nth-child(2)")).toHaveCSS(
      "text-align",
      "center"
    );
  });

  test("reorders rows and columns", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const values = await inPage(page, async (s, m) => {
      await s.selectCell(1, 0);
      await s.apply(m.commands.moveRow(1, 3));
      const rows = s.value;
      await s.selectCell(1, 0);
      await s.apply(m.commands.moveColumn(0, 3));
      return [rows, s.value];
    });
    expect(values[0]).toBe(`| h1 | h2 | h3 |\n|----|----|----|\n| b1 | b2 | b3 |\n| a1 | a2 | a3 |\n\n`);
    expect(values[1]).toBe(`| h2 | h3 | h1 |\n|----|----|----|\n| b2 | b3 | b1 |\n| a2 | a3 | a1 |\n\n`);
  });

  test("Tab walks the cells and appends a row at the end", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s, m) => {
      await s.selectCell(0, 0);
      await s.apply(m.commands.goToNextCell(1));
      const text = s.view.state.selection.$from.parent.textContent;
      await s.selectCell(2, 2);
      await s.apply(m.commands.goToNextCell(1));
      return [text, s.value];
    });
    expect(result[0]).toBe("h2");
    expect(result[1]).toBe(`| h1 | h2 | h3 |\n|----|----|----|\n| a1 | a2 | a3 |\n| b1 | b2 | b3 |\n|  |  |  |\n\n`);
  });

  test("a caret in a cell targets both of its axes", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const classNames = await inPage(page, async (s, m) => {
      await s.selectCell(1, 1);
      return m.menu
        .cellMenuItems(s.view, m.menu.menuTargetFor(s.view.state))
        .filter((item) => !item.divider)
        .map((item) => item.className);
    });
    expect(classNames).toEqual([
      "composer-table-menu__insert-above",
      "composer-table-menu__insert-below",
      "composer-table-menu__insert-left",
      "composer-table-menu__insert-right",
      "composer-table-menu__duplicate-row",
      "composer-table-menu__duplicate-column",
      "composer-table-menu__move-row-up",
      "composer-table-menu__move-row-down",
      "composer-table-menu__move-column-left",
      "composer-table-menu__move-column-right",
      "composer-table-menu__align-left",
      "composer-table-menu__align-center",
      "composer-table-menu__align-right",
      "composer-table-menu__clear-contents",
      "composer-table-menu__delete-row",
      "composer-table-menu__delete-column",
      "composer-table-menu__delete-table",
    ]);
  });

  test("the keyboard opens the rendered cell menu and clears its cell", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    await inPage(page, async (s) => {
      await s.selectCell(1, 1);
      await s.pressKey("Enter", { altKey: true });
    });
    await expect(page.locator('.fk-d-menu[data-identifier="composer-table-menu"]')).toHaveCount(1);
    await expect(page.locator(".composer-table-menu__duplicate-row")).not.toHaveAttribute("aria-keyshortcuts");
    await expect(page.locator(".composer-table-menu__align-left")).toHaveAttribute("aria-pressed", "false");
    await expect(page.locator(".composer-table-menu__align-center")).toHaveAttribute("aria-pressed", "false");
    await page.locator(".composer-table-menu__clear-contents").click();
    await expect.poll(() => editorValue(page)).toBe(`| h1 | h2 | h3 |\n|----|----|----|\n| a1 |  | a3 |\n| b1 | b2 | b3 |\n\n`);
  });

  test("the header row is offered a header above instead of a plain row", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const classNames = await inPage(page, async (s, m) => {
      await s.selectCell(0, 0);
      return m.menu.cellMenuItems(s.view, m.menu.menuTargetFor(s.view.state)).map((item) => item.className);
    });
    expect(classNames).not.toContain("composer-table-menu__insert-above");
    expect(classNames).toContain("composer-table-menu__insert-header-above");
  });

  test("inserting a header above demotes the one it replaces", async ({ page }) => {
    await setupRichEditor(page, `| left | right |\n| :--- | ---: |\n| a1 | a2 |`);
    const value = await inPage(page, async (s, m) => {
      await s.selectCell(0, 0);
      await s.apply(m.commands.addHeaderRow());
      return s.value;
    });
    expect(value).toBe(`|  |  |\n|:---|---:|\n| left | right |\n| a1 | a2 |\n\n`);
  });

  test("an inserted header row undoes in one step", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const value = await inPage(page, async (s, m) => {
      await s.selectCell(0, 0);
      await s.apply(m.commands.addHeaderRow());
      await s.apply(m.history.undo);
      return s.value;
    });
    expect(value).toBe(TABLE_OUT);
  });

  test("a caret outside a table has no target", async ({ page }) => {
    await setupRichEditor(page, `text\n\n${TABLE}`);
    const target = await inPage(page, async (s, m) => {
      await s.setSelection(2);
      return m.menu.menuTargetFor(s.view.state);
    });
    expect(target).toBeNull();
  });

  test("a grip highlights its row without replacing the caret", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s) => {
      await s.selectCell(2, 1);
      const caret = s.view.state.selection.from;
      await s.pressGrip(s.findAll(".composer-table__grip.--row")[1]);
      return {
        targets: s.findAll(".ProseMirror .composer-table .is-structural-target").length,
        same: s.view.state.selection.from === caret,
      };
    });
    expect(result.targets, "one row").toBe(3);
    expect(result.same, "the document selection remains a caret").toBe(true);
  });

  for (const kind of ["row", "column"]) {
    test(`clicking a ${kind} grip opens its rendered action menu`, async ({ page }) => {
      await setupRichEditor(page, TABLE);
      await inPage(
        page,
        async (s, m, kind) => {
          s.findAll(`.composer-table__grip.--${kind}`)[1].click();
          await s.waitFor(() => s.menu());
        },
        kind
      );
      await expect(page.locator(`.composer-table-menu__duplicate-${kind}`)).toHaveCount(1);
      await expect(page.locator(".composer-table-menu__clear-contents")).toHaveCount(1);
      await expect(page.locator(`.composer-table__grip.--${kind}`).nth(1)).toHaveAttribute("aria-expanded", "true");
      const bounds = await page
        .locator('.fk-d-menu[data-identifier="composer-table-menu"]')
        .evaluate((el) => el.getBoundingClientRect());
      expect([Math.round(bounds.left), Math.round(bounds.top)]).not.toEqual([0, 0]);
    });
  }

  test("tapping a grip opens the table actions on mobile", async ({ page }) => {
    await page.evaluate(() => document.documentElement.classList.add("mobile-view"));
    await setupRichEditor(page, TABLE);
    await inPage(page, async (s) => {
      await s.pressGrip(s.findAll(".composer-table__grip.--row")[1], { activate: true, moveBy: 6, pointerType: "touch" });
    });
    await expect(page.locator('.fk-d-menu-modal[data-identifier="composer-table-menu"]')).toHaveCount(1);
    await expect(page.locator(".composer-table-menu__duplicate-row")).toHaveCount(1);
  });

  test("a mobile grip can reopen its drawer after closing", async ({ page }) => {
    await page.evaluate(() => document.documentElement.classList.add("mobile-view"));
    await setupRichEditor(page, TABLE);
    const selector = '.fk-d-menu-modal[data-identifier="composer-table-menu"]';
    for (let attempt = 0; attempt < 3; attempt++) {
      await inPage(page, async (s) => {
        await s.pressGrip(s.findAll(".composer-table__grip.--row")[1], { activate: true, pointerType: "touch", click: true });
      });
      await expect(page.locator(selector), `tap ${attempt + 1} opens the drawer`).toHaveCount(1);
      await inPage(page, (s, m) => m.menu.shownMenu().close());
      await expect(page.locator(selector), "the drawer closes").toHaveCount(0);
    }
  });

  test("an active mobile grip does not expose vertical table scrolling", async ({ page }) => {
    await page.evaluate(() => document.documentElement.classList.add("mobile-view"));
    await setupRichEditor(page, TABLE);
    const overflowY = await inPage(page, async (s) => {
      await s.pressGrip(s.findAll(".composer-table__grip.--column")[1]);
      return getComputedStyle(s.find(".composer-table")).overflowY;
    });
    expect(overflowY).toBe("hidden");
  });

  test("touch dragging the table body scrolls it horizontally", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s) => {
      const scroller = s.find(".composer-table");
      const inner = s.find(".composer-table__inner");
      const cell = s.find(".composer-table td");
      Object.assign(scroller.style, { width: "12rem" });
      Object.assign(inner.style, { width: "36rem" });
      scroller.scrollLeft = 0;
      const at = (x, y) => ({
        bubbles: true,
        cancelable: true,
        button: 0,
        pointerId: 11,
        pointerType: "touch",
        isPrimary: true,
        clientX: x,
        clientY: y,
      });
      cell.dispatchEvent(new PointerEvent("pointerdown", at(160, 50)));
      document.dispatchEvent(new PointerEvent("pointermove", at(60, 52)));
      const overflowX = getComputedStyle(scroller).overflowX;
      const scrolled = scroller.scrollLeft > 0;
      document.dispatchEvent(new PointerEvent("pointerup", at(60, 52)));
      return { overflowX, scrolled, panning: scroller.classList.contains("is-panning") };
    });
    expect(result.overflowX).toBe("auto");
    expect(result.scrolled).toBe(true);
    expect(result.panning).toBe(false);
  });

  test("table chrome reserves only the touch gestures it uses", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const actions = await inPage(page, async (s) => ({
      table: getComputedStyle(s.find(".composer-table table")).touchAction,
      grips: s.findAll(".composer-table__grip").map((el) => getComputedStyle(el).touchAction),
      row: getComputedStyle(s.find(".composer-table__append.--row")).touchAction,
      column: getComputedStyle(s.find(".composer-table__append.--column")).touchAction,
    }));
    expect(actions.table).toBe("pan-y pinch-zoom");
    actions.grips.forEach((action) => expect(action).toBe("none"));
    expect(actions.row).toBe("pan-x pinch-zoom");
    expect(actions.column).toBe("pan-y pinch-zoom");
  });

  test("an assistive click opens a grip without pointer events", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    await inPage(page, async (s) => {
      s.findAll(".composer-table__grip.--column")[1].dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await s.waitFor(() => s.menu());
    });
    await expect(page.locator(".composer-table-menu__duplicate-column")).toHaveCount(1);
  });

  test("a canceled grip drag does not swallow the next click", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const opened = await inPage(page, async (s) => {
      const grip = s.findAll(".composer-table__grip.--row")[1];
      const bounds = grip.getBoundingClientRect();
      const at = (y) => ({ bubbles: true, cancelable: true, button: 0, pointerId: 9, isPrimary: true, clientX: bounds.left + 1, clientY: y });
      grip.dispatchEvent(new PointerEvent("pointerdown", at(bounds.top + 1)));
      document.dispatchEvent(new PointerEvent("pointermove", at(bounds.top + 10)));
      document.dispatchEvent(new PointerEvent("pointercancel", at(bounds.top + 10)));
      await s.wait();
      const early = !!s.menu();
      s.findAll(".composer-table__grip.--row")[1].dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await s.waitFor(() => s.menu());
      return early;
    });
    expect(opened, "canceling does not open a menu").toBe(false);
    await expect(page.locator(".composer-table-menu__duplicate-row")).toHaveCount(1);
  });

  test("duplicating a row and a column copies their contents", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const values = await inPage(page, async (s, m) => {
      await s.selectCell(1, 0);
      await s.apply(m.commands.duplicateRow());
      const row = s.value;
      await s.selectCell(1, 1);
      await s.apply(m.commands.duplicateColumn());
      return [row, s.value];
    });
    expect(values[0]).toBe(`| h1 | h2 | h3 |\n|----|----|----|\n| a1 | a2 | a3 |\n| a1 | a2 | a3 |\n| b1 | b2 | b3 |\n\n`);
    expect(values[1]).toBe(
      `| h1 | h2 | h2 | h3 |\n|----|----|----|----|\n| a1 | a2 | a2 | a3 |\n| a1 | a2 | a2 | a3 |\n| b1 | b2 | b2 | b3 |\n\n`
    );
  });

  test("deleting the table removes it from the document", async ({ page }) => {
    await setupRichEditor(page, `before\n\n${TABLE}`);
    const value = await inPage(page, async (s, m) => {
      await s.selectCell(1, 1);
      await s.apply(m.commands.deleteTable());
      return s.value;
    });
    expect(value).toBe("before");
    await expect(page.locator(".ProseMirror .composer-table")).toHaveCount(0);
  });

  test("Tab moves between cells through the keymap", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const texts = await inPage(page, async (s) => {
      await s.selectCell(0, 0);
      await s.pressKey("Tab");
      const forward = s.view.state.selection.$from.parent.textContent;
      await s.pressKey("Tab", { shiftKey: true });
      return [forward, s.view.state.selection.$from.parent.textContent];
    });
    expect(texts).toEqual(["h2", "h1"]);
  });

  test("Shift+Tab at the first cell remains available to the browser", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const prevented = await inPage(page, async (s) => {
      await s.selectCell(0, 0);
      return s.pressKey("Tab", { shiftKey: true });
    });
    expect(prevented).toBe(false);
  });

  test("a cell that ends up the wrong type is corrected", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const type = await inPage(page, async (s) => {
      const view = s.view;
      const pos = s.cellPos(1, 0);
      const cell = view.state.doc.nodeAt(pos);
      view.dispatch(view.state.tr.setNodeMarkup(pos, view.state.schema.nodes.table_header_cell, cell.attrs));
      await s.wait();
      return view.state.doc.nodeAt(s.cellPos(1, 0)).type.name;
    });
    expect(type).toBe("table_cell");
  });

  // Merged cells and header columns are expressible in a post through the
  // sanitized HTML subset, which the editor keeps verbatim rather than
  // flattening into a pipe table that cannot hold them.
  for (const [name, markdown] of Object.entries({
    colspan: `<table>\n<thead>\n<tr><th>A</th><th colspan="2">B</th></tr>\n</thead>\n<tbody>\n<tr><td>1</td><td>2</td><td>3</td></tr>\n</tbody>\n</table>`,
    rowspan: `<table>\n<tbody>\n<tr><td rowspan="2">A</td><td>1</td></tr>\n<tr><td>2</td></tr>\n</tbody>\n</table>`,
    "header column": `<table>\n<tbody>\n<tr><th>Row A</th><td>1</td></tr>\n<tr><th>Row B</th><td>2</td></tr>\n</tbody>\n</table>`,
  })) {
    test(`an HTML table with ${name} survives untouched`, async ({ page }) => {
      const { value } = await setupRichEditor(page, markdown);
      expect(value).toBe(`${markdown}\n\n`);
      await expect(page.locator(".ProseMirror .composer-table")).toHaveCount(0);
      await expect(page.locator(".ProseMirror .html-block")).toHaveCount(1);
    });
  }

  test("hovering a cell reveals only its own row and column handle", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s) => {
      const before = s.findAll(".composer-table__grip.is-visible").length;
      const cell = s.findAll(".ProseMirror .composer-table td").find((td) => td.textContent === "b2");
      await s.hoverCell(cell);
      const shown = s.findAll(".composer-table__grip.is-visible");
      const cellOf = (grip) => grip.closest("th, td").textContent;
      return {
        before,
        count: shown.length,
        column: cellOf(shown.find((g) => g.classList.contains("--column"))),
        row: cellOf(shown.find((g) => g.classList.contains("--row"))),
      };
    });
    expect(result).toEqual({ before: 0, count: 2, column: "h2", row: "b1" });
  });

  test("a caret keeps pointer-specific handles quiet", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    await inPage(page, (s) => s.selectCell(2, 1));
    await expect(page.locator(".composer-table__grip.is-visible")).toHaveCount(0);
    await expect(page.locator(".composer-table.is-active")).toHaveCount(1);
    await expect(page.locator(".composer-table .is-current-row")).toHaveCount(1);
    await expect(page.locator(".composer-table .is-current-column")).toHaveCount(1);
  });

  test("only the caret's handles are exposed to assistive technology", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s) => {
      await s.selectCell(2, 1);
      const grips = s.findAll(".composer-table__grip");
      const exposed = grips.filter((grip) => grip.getAttribute("aria-hidden") === "false");
      return {
        count: grips.length,
        labels: exposed.map((grip) => grip.getAttribute("aria-label")).sort(),
        hidden: grips.filter((grip) => !exposed.includes(grip)).every((grip) => grip.getAttribute("aria-hidden") === "true"),
      };
    });
    expect(result.count).toBe(6);
    expect(result.labels).toEqual(["Column 2 actions", "Row 3 actions"]);
    expect(result.hidden).toBe(true);
  });

  test("a disabled editor hides table chrome and rejects its events", async ({ page }) => {
    await setupRichEditor(page, TABLE, { disabled: true });
    const result = await inPage(page, async (s, m) => {
      const before = s.value;
      const grip = s.find(".composer-table__grip.--row");
      const append = s.find(".composer-table__append.--row");
      const hidden = grip.hasAttribute("hidden") && append.hasAttribute("hidden");
      grip.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      append.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      const target = m.menu.menuTargetFor(s.view.state);
      if (target) {
        m.menu
          .cellMenuItems(s.view, target)
          .find((item) => item.className === "composer-table-menu__insert-below")
          .action();
      }
      await s.pressKey("Enter", { altKey: true });
      await s.wait();
      return { hidden, same: s.value === before, menu: !!s.menu() };
    });
    expect(result).toEqual({ hidden: true, same: true, menu: false });
  });

  test("a structural target marks the sides that face out of it", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const edges = await inPage(page, async (s) => {
      await s.pressGrip(s.findAll(".composer-table__grip.--row")[1]);
      return s
        .findAll(".ProseMirror .composer-table .is-structural-target")
        .map((cell) => [...cell.classList].filter((name) => name.startsWith("--edge")).sort().join(" "));
    });
    expect(edges).toEqual(["--edge-start", "", "--edge-end"]);
  });

  test("clicking an append bar after dragging it still adds one", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s) => {
      const columns = () => s.findAll(".ProseMirror .composer-table th").length;
      const button = s.find(".composer-table__append.--column");
      const rect = button.getBoundingClientRect();
      const at = (x) => ({ bubbles: true, cancelable: true, button: 0, pointerId: 4, isPrimary: true, clientX: x, clientY: rect.top + rect.height / 2 });
      const origin = rect.left + rect.width / 2;
      button.dispatchEvent(new PointerEvent("pointerdown", at(origin)));
      document.dispatchEvent(new PointerEvent("pointermove", at(origin + 600)));
      document.dispatchEvent(new PointerEvent("pointerup", at(origin + 600)));
      await s.wait();
      const afterDrag = columns();
      button.dispatchEvent(new PointerEvent("pointerdown", at(origin)));
      document.dispatchEvent(new PointerEvent("pointerup", at(origin)));
      button.dispatchEvent(new MouseEvent("click", at(origin)));
      await s.wait();
      return { afterDrag, after: columns(), markdown: s.value.split("\n")[0].split("|").length - 2 };
    });
    expect(result.afterDrag).toBeGreaterThan(3);
    expect(result.after).toBe(result.afterDrag + 1);
    expect(result.markdown).toBe(result.afterDrag + 1);
  });

  test("dragging an append bar back removes empty rows or columns", async ({ page }) => {
    await setupRichEditor(page, `| h1 | h2 |\n| --- | --- |\n| a1 | a2 |`);
    const result = await inPage(page, async (s) => {
      const columns = () => s.value.split("\n")[0].split("|").length - 2;
      const before = columns();
      await s.dragAppend(s.find(".composer-table__append.--column"), 400);
      const added = columns();
      await s.dragAppend(s.find(".composer-table__append.--column"), -400);
      return { before, added, after: columns() };
    });
    expect(result.added).toBeGreaterThan(result.before);
    expect(result.after).toBe(result.before);
  });

  test("dragging back never removes a row or column with content", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s) => {
      const before = s.value;
      await s.dragAppend(s.find(".composer-table__append.--column"), -600);
      const afterColumns = s.value;
      await s.dragAppend(s.find(".composer-table__append.--row"), -600);
      return [before, afterColumns, s.value];
    });
    expect(result[1]).toBe(result[0]);
    expect(result[2]).toBe(result[0]);
  });

  test("dragging back preserves non-text cell content", async ({ page }) => {
    await setupRichEditor(page, `| h1 | |\n| --- | --- |\n| | ![image](https://example.com/image.png) |`);
    const result = await inPage(page, async (s) => {
      const before = s.value;
      await s.dragAppend(s.find(".composer-table__append.--column"), -600);
      const afterColumns = s.value;
      await s.dragAppend(s.find(".composer-table__append.--row"), -600);
      return [before, afterColumns, s.value];
    });
    expect(result[1]).toBe(result[0]);
    expect(result[2]).toBe(result[0]);
  });

  test("a row can be dragged into and out of the header", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const values = await inPage(page, async (s, m) => {
      await s.selectCell(2, 0);
      await s.apply(m.commands.moveRow(2, 0));
      const up = s.value;
      await s.selectCell(0, 0);
      await s.apply(m.commands.moveRow(0, 3));
      return [up, s.value];
    });
    expect(values[0]).toBe(`| b1 | b2 | b3 |\n|----|----|----|\n| h1 | h2 | h3 |\n| a1 | a2 | a3 |\n\n`);
    expect(values[1]).toBe(TABLE_OUT);
  });

  test("crossing the header re-types the cells it moves", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const types = await inPage(page, async (s, m) => {
      await s.selectCell(1, 0);
      await s.apply(m.commands.moveRow(1, 0));
      const out = [];
      s.view.state.doc.descendants((node) => {
        if (node.type.spec.tableRole === "row") {
          out.push([...node.children].map((cell) => cell.type.name).join(","));
        }
      });
      return out;
    });
    expect(types[0]).toBe("table_header_cell,table_header_cell,table_header_cell");
    expect(types[1]).toBe("table_cell,table_cell,table_cell");
  });

  test("the append bars grow the table", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const values = await inPage(page, async (s) => {
      s.find(".composer-table__append.--column").click();
      await s.wait();
      const column = s.value;
      s.find(".composer-table__append.--row").click();
      await s.wait();
      return [column, s.value];
    });
    expect(values[0]).toBe(`| h1 | h2 | h3 |  |\n|----|----|----|----|\n| a1 | a2 | a3 |  |\n| b1 | b2 | b3 |  |\n\n`);
    expect(values[1]).toBe(
      `| h1 | h2 | h3 |  |\n|----|----|----|----|\n| a1 | a2 | a3 |  |\n| b1 | b2 | b3 |  |\n|  |  |  |  |\n\n`
    );
  });

  test("dragging an append bar adds multiple rows or columns", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const value = await inPage(page, async (s) => {
      s.find(".composer-table table").getBoundingClientRect = () => ({ width: 300, height: 90 });
      await s.dragAppend(s.find(".composer-table__append.--column"), 200);
      await s.dragAppend(s.find(".composer-table__append.--row"), 60);
      return s.value;
    });
    expect(value).toBe(
      `| h1 | h2 | h3 |  |  |\n|----|----|----|----|----|\n| a1 | a2 | a3 |  |  |\n| b1 | b2 | b3 |  |  |\n|  |  |  |  |  |\n|  |  |  |  |  |\n\n`
    );
  });

  test("a narrow table does not over-count a column drag", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const value = await inPage(page, async (s) => {
      s.find(".composer-table table").getBoundingClientRect = () => ({ width: 90, height: 90 });
      await s.dragAppend(s.find(".composer-table__append.--column"), 100);
      return s.value;
    });
    expect(value).toBe(`| h1 | h2 | h3 |  |\n|----|----|----|----|\n| a1 | a2 | a3 |  |\n| b1 | b2 | b3 |  |\n\n`);
  });

  test("a long append drag is capped", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const columns = await inPage(page, async (s) => {
      s.find(".composer-table table").getBoundingClientRect = () => ({ width: 300, height: 90 });
      await s.dragAppend(s.find(".composer-table__append.--column"), 10_000);
      return s.value.split("\n")[0].split("|").length - 2;
    });
    expect(columns).toBe(11);
  });

  test("an append drag autoscrolls at the table edge", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s) => {
      const scroller = s.find(".composer-table");
      const button = s.find(".composer-table__append.--column");
      const inner = s.find(".composer-table__inner");
      Object.assign(scroller.style, { width: "200px", overflowX: "auto" });
      inner.style.width = "600px";
      scroller.scrollLeft = 0;
      const bounds = scroller.getBoundingClientRect();
      const at = (x) => ({ bubbles: true, cancelable: true, button: 0, pointerId: 8, isPrimary: true, clientX: x, clientY: 50 });
      button.dispatchEvent(new PointerEvent("pointerdown", at(bounds.left + bounds.width / 2)));
      document.dispatchEvent(new PointerEvent("pointermove", at(bounds.right - 2)));
      await s.waitFor(() => s.find(".composer-table__append.--column .composer-table__append-count").textContent === "+2");
      const scrolled = scroller.scrollLeft > 0;
      document.dispatchEvent(new PointerEvent("pointerup", at(bounds.right - 2)));
      await s.wait();
      return { scrolled, columns: s.value.split("\n")[0].split("|").length - 2 };
    });
    expect(result.scrolled).toBe(true);
    expect(result.columns).toBeGreaterThanOrEqual(5);
  });

  test("a row reorder autoscrolls at the composer edge", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s) => {
      const scroller = s.find(".composer-table");
      const inner = s.find(".composer-table__inner");
      const grip = s.findAll(".composer-table__grip.--row")[1];
      Object.assign(scroller.style, { height: "70px", overflowY: "auto" });
      inner.style.height = "300px";
      const gripBounds = grip.getBoundingClientRect();
      const bounds = scroller.getBoundingClientRect();
      const at = (y) => ({
        bubbles: true,
        cancelable: true,
        button: 0,
        pointerId: 9,
        isPrimary: true,
        clientX: gripBounds.left + gripBounds.width / 2,
        clientY: y,
      });
      grip.dispatchEvent(new PointerEvent("pointerdown", at(gripBounds.top + gripBounds.height / 2)));
      document.dispatchEvent(new PointerEvent("pointermove", at(bounds.bottom - 2)));
      await s.waitFor(() => scroller.scrollTop > 0);
      const scrolled = scroller.scrollTop > 0;
      document.dispatchEvent(new PointerEvent("pointerup", at(bounds.bottom - 2)));
      await s.wait();
      return { scrolled, value: s.value };
    });
    expect(result.scrolled).toBe(true);
    expect(result.value).toBe(`| h1 | h2 | h3 |\n|----|----|----|\n| b1 | b2 | b3 |\n| a1 | a2 | a3 |\n\n`);
  });

  test("a text selection across cells remains ordinary text selection", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s, m) => {
      await s.setSelection(s.cellPos(1, 0) + 1, s.cellPos(1, 2) + 2);
      s.view.focus();
      return {
        text: s.view.state.selection instanceof m.state.TextSelection,
        native: window.getSelection().toString().replace(/\s/g, ""),
        targets: s.findAll(".ProseMirror .composer-table .is-structural-target").length,
      };
    });
    expect(result).toEqual({ text: true, native: "a1a2a", targets: 0 });
  });

  test("Enter consumes the two-space line break syntax inside a cell", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const value = await inPage(page, async (s) => {
      await s.setSelection(s.cellPos(1, 0) + 3);
      s.view.dispatch(s.view.state.tr.insertText("  "));
      await s.pressKey("Enter");
      return s.value;
    });
    expect(value).toBe(`| h1 | h2 | h3 |\n|----|----|----|\n| a1<br> | a2 | a3 |\n| b1 | b2 | b3 |\n\n`);
  });

  test("arrow keys move between cells without a gap cursor", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s, m) => {
      const endOfCell = s.cellPos(1, 0) + 3;
      await s.setSelection(endOfCell);
      s.view.focus();
      const GapCursor = m.gapcursor.GapCursor;
      const between = GapCursor.valid(s.view.state.doc.resolve(endOfCell + 1));
      const rowStart = GapCursor.valid(s.view.state.doc.resolve(s.cellPos(1, 0)));
      const prevented = await s.pressKey("ArrowRight", { keyCode: 39 });
      return { between, rowStart, prevented, text: s.view.state.selection instanceof m.state.TextSelection };
    });
    expect(result).toEqual({ between: false, rowStart: false, prevented: false, text: true });
  });

  test("Enter inserts a line break that survives Markdown round-tripping", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const value = await inPage(page, async (s) => {
      await s.setSelection(s.cellPos(1, 1) + 2);
      await s.pressKey("Enter");
      return s.value;
    });
    expect(value).toBe(`| h1 | h2 | h3 |\n|----|----|----|\n| a1 | a<br>2 | a3 |\n| b1 | b2 | b3 |\n\n`);
    // Toggling to markdown and back: the editor opened again over its value.
    const width = await page.evaluate(async (markdown) => {
      await window.richSpec.create(markdown, {});
      return window.tableSpec.locateTable().grid.width;
    }, value);
    await expect(page.locator(".ProseMirror tbody tr:first-child td:nth-child(2) br")).toHaveCount(1);
    expect(width).toBe(3);
  });

  test("Backspace and Delete at cell edges keep cells apart", async ({ page }) => {
    await setupRichEditor(page, `| h1 | | h3 |\n| --- | --- | --- |\n| a1 | | a3 |\n| | | |`);
    const broken = await inPage(page, async (s) => {
      const original = s.view.state.doc;
      const failures = [];
      for (const key of ["Backspace", "Delete"]) {
        for (const row of [0, 1, 2]) {
          for (const col of [0, 1, 2]) {
            const { start, grid } = s.locateTable();
            const cell = grid.rows[row].cells[col];
            const pos = start + cell.offset + 1 + (key === "Delete" ? cell.node.content.size : 0);
            await s.setSelection(pos);
            await s.pressKey(key, { keyCode: key === "Backspace" ? 8 : 46 });
            await s.pressKey(key, { keyCode: key === "Backspace" ? 8 : 46 });
            if (!s.view.state.doc.eq(original)) {
              failures.push(`${key} ${row},${col}`);
            }
          }
        }
      }
      return failures;
    });
    expect(broken).toEqual([]);
  });

  test("Delete across cells preserves their boundaries", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s) => {
      await s.setSelection(s.cellPos(1, 0) + 2, s.cellPos(1, 2) + 2);
      await s.pressKey("Delete");
      return { value: s.value, width: s.locateTable().grid.width };
    });
    expect(result.value).toBe(`| h1 | h2 | h3 |\n|----|----|----|\n| a |  | 3 |\n| b1 | b2 | b3 |\n\n`);
    expect(result.width).toBe(3);
  });

  test("mobile deletion across table sections preserves their structure", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s) => {
      await s.setSelection(s.cellPos(0, 2) + 2, s.cellPos(1, 0) + 2);
      s.view.dom.dispatchEvent(
        new InputEvent("beforeinput", { bubbles: true, cancelable: true, inputType: "deleteContentBackward" })
      );
      await s.wait();
      return {
        value: s.value,
        types: s.locateTable().grid.rows.map((row) => row.cells.map((cell) => cell.node.type.name)),
      };
    });
    expect(result.value).toBe(`| h1 | h2 | h |\n|----|----|----|\n| 1 | a2 | a3 |\n| b1 | b2 | b3 |\n\n`);
    expect(result.types).toEqual([
      ["table_header_cell", "table_header_cell", "table_header_cell"],
      ["table_cell", "table_cell", "table_cell"],
      ["table_cell", "table_cell", "table_cell"],
    ]);
  });

  for (const backwards of [false, true]) {
    for (const shiftKey of [false, true]) {
      test(`${shiftKey ? "Shift+Enter" : "Enter"} across rows preserves cells (${backwards ? "backward" : "forward"})`, async ({
        page,
      }) => {
        await setupRichEditor(page, TABLE);
        const result = await inPage(
          page,
          async (s, m, { backwards, shiftKey }) => {
            const from = s.cellPos(1, 0) + 2;
            const to = s.cellPos(2, 0) + 2;
            await s.setSelection(backwards ? to : from, backwards ? from : to);
            await s.pressKey("Enter", { shiftKey });
            const grid = s.locateTable().grid;
            const out = {
              height: grid.height,
              widths: grid.rows.map((row) => row.cells.length),
              last: grid.rows[1].cells[0].node.lastChild && grid.rows[1].cells[0].node.lastChild.type.name,
              suffix: grid.rows[2].cells[0].node.textContent,
            };
            await s.apply(m.history.undo);
            out.restored = s.locateTable().grid.rows[1].cells[0].node.textContent;
            s.view.dispatch(s.view.state.tr.insertText("X"));
            out.after = s.locateTable().grid.height;
            return out;
          },
          { backwards, shiftKey }
        );
        expect(result).toEqual({
          height: 3,
          widths: [3, 3, 3],
          last: "hard_break",
          suffix: "1",
          restored: "a1",
          after: 3,
        });
      });
    }
  }

  test("editing a trailing cell cancels an append-removal drag", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s, m) => {
      await s.selectCell(1, 2);
      await s.apply(m.commands.addColumn(1));
      const button = s.find(".composer-table__append.--column");
      const bounds = button.getBoundingClientRect();
      const start = { bubbles: true, button: 0, pointerId: 8, clientX: bounds.left, clientY: bounds.top };
      const end = { ...start, clientX: bounds.left - 200 };
      button.dispatchEvent(new PointerEvent("pointerdown", start));
      document.dispatchEvent(new PointerEvent("pointermove", end));
      s.view.dispatch(s.view.state.tr.insertText("keep", s.cellPos(1, 3) + 1));
      document.dispatchEvent(new PointerEvent("pointerup", end));
      return {
        text: s.locateTable().grid.rows[1].cells[3].node.textContent,
        ghost: !!s.find(".composer-table__ghost.--visible"),
      };
    });
    expect(result).toEqual({ text: "keep", ghost: false });
  });

  test("a captured command rejects a changed table position", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s, m) => {
      const view = s.view;
      const target = s.locateTable();
      view.dispatch(view.state.tr.insert(0, view.state.schema.nodes.paragraph.create(null, view.state.schema.text("before"))));
      const doc = view.state.doc;
      return { handled: m.commands.moveRow(1, 3, target)(view.state, view.dispatch), same: view.state.doc === doc };
    });
    expect(result).toEqual({ handled: false, same: true });
  });

  test("header moves keep subsequent typing in the moved row", async ({ page }) => {
    await setupRichEditor(page, TABLE + "\n\nafter");
    const text = await inPage(page, async (s, m) => {
      await s.selectCell(1, 0);
      await s.apply(m.commands.moveRow(1, 0));
      s.view.dispatch(s.view.state.tr.insertText("X"));
      return s.locateTable().grid.rows[0].cells[0].node.textContent;
    });
    expect(text).toBe("Xa1");
  });

  test("pasting a table keeps subsequent typing inside the destination", async ({ page }) => {
    await setupRichEditor(page, TABLE + "\n\nafter");
    const text = await inPage(page, async (s) => {
      await s.selectCell(1, 0);
      s.view.pasteHTML("<table><tbody><tr><td>pasted</td></tr></tbody></table>");
      s.view.dispatch(s.view.state.tr.insertText("X"));
      return s.locateTable().grid.rows[1].cells[0].node.textContent;
    });
    expect(text).toBe("Xpasted");
  });

  test("a caret in table chrome returns to editable cell content", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s) => {
      await s.selectCell(2, 0);
      s.view.focus();
      window.getSelection().collapse(s.find(".composer-table__inner"), 0);
      document.dispatchEvent(new Event("selectionchange"));
      await s.wait(100);
      const selection = window.getSelection();
      const element =
        selection.anchorNode.nodeType === Node.ELEMENT_NODE ? selection.anchorNode : selection.anchorNode.parentElement;
      const inCell = !!element.closest("th, td");
      document.execCommand("insertText", false, "sss");
      await s.wait(100);
      return {
        inCell,
        typed: s.view.state.doc.textContent.includes("sss"),
        first: s.find(".composer-table__inner").firstChild.tagName,
      };
    });
    expect(result).toEqual({ inCell: true, typed: true, first: "TABLE" });
  });

  test("cell mutation handling still reads text edits", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const text = await inPage(page, async (s) => {
      const cell = s.find(".composer-table td");
      [...cell.childNodes].find((node) => node.nodeType === Node.TEXT_NODE).textContent = "edited";
      await s.wait(100);
      return s.locateTable().grid.rows[1].cells[0].node.textContent;
    });
    expect(text).toBe("edited");
  });

  test("an assistive append activation works after a drag ends elsewhere", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const widths = await inPage(page, async (s) => {
      const button = s.find(".composer-table__append.--column");
      const bounds = button.getBoundingClientRect();
      const start = { bubbles: true, button: 0, pointerId: 8, clientX: bounds.left, clientY: bounds.top };
      button.dispatchEvent(new PointerEvent("pointerdown", start));
      document.dispatchEvent(new PointerEvent("pointermove", { ...start, clientX: bounds.left + 200 }));
      document.dispatchEvent(new PointerEvent("pointerup", { ...start, clientX: bounds.left + 200 }));
      const width = s.locateTable().grid.width;
      button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      return [width, s.locateTable().grid.width];
    });
    expect(widths[1]).toBe(widths[0] + 1);
  });

  for (const direction of ["ltr", "rtl"]) {
    test(`row grips follow ${direction} table direction on an RTL page`, async ({ page }) => {
      await page.evaluate(() => (document.documentElement.dir = "rtl"));
      await setupRichEditor(page, TABLE);
      const offset = await inPage(
        page,
        async (s, m, direction) => {
          s.view.dom.dir = direction;
          const grip = s.find(".composer-table__grip.--row");
          const cell = grip.closest("th");
          const gripBounds = grip.getBoundingClientRect();
          const edge = cell.getBoundingClientRect()[direction === "rtl" ? "right" : "left"];
          return Math.abs(gripBounds.left + gripBounds.width / 2 - edge);
        },
        direction
      );
      expect(offset).toBeLessThan(1);
    });
  }

  test("editable changes synchronize all table controls", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const mismatches = await inPage(page, async (s) => {
      const out = [];
      for (const editable of [false, true]) {
        s.view.setProps({ editable: () => editable });
        s.findAll(".composer-table__grip, .composer-table__append").forEach((control) => {
          if (control.hidden !== !editable || control.disabled !== !editable) {
            out.push(`${control.className} editable=${editable}`);
          }
        });
      }
      return out;
    });
    expect(mismatches).toEqual([]);
  });

  test("dragging keeps the editable cells intact between pointer events", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s) => {
      const cell = s.findAll(".composer-table td")[0];
      const during = await s.dragGrip(s.findAll(".composer-table__grip.--row")[1], s.findAll(".composer-table tr")[2], () => ({
        same: s.findAll(".composer-table td")[0] === cell,
        moving: cell.classList.contains("is-moving"),
        transformed: cell.style.transform !== "",
      }));
      return { during, after: s.locateTable().grid.rows[2].cells[0].node.textContent };
    });
    expect(result).toEqual({ during: { same: true, moving: true, transformed: true }, after: "a1" });
  });

  test("a document change cancels a pending reorder", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s) => {
      await s.dragGrip(s.findAll(".composer-table__grip.--row")[1], s.findAll(".composer-table tr")[2], () => {
        s.view.dispatch(s.view.state.tr.insertText("X", s.cellPos(1, 0) + 1));
      });
      return {
        text: s.locateTable().grid.rows[1].cells[0].node.textContent,
        avatar: !!document.querySelector(".composer-table__drag-avatar"),
      };
    });
    expect(result).toEqual({ text: "Xa1", avatar: false });
  });

  // code-block's highlighter is a plugin that resolves after the editor
  // opens, and adding a plugin replaces every plugin view.
  test("a plugin resolving mid-drag keeps the reorder", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s, m) => {
      const cell = s.findAll(".composer-table td")[0];
      const during = await s.dragGrip(s.findAll(".composer-table__grip.--row")[1], s.findAll(".composer-table tr")[2], async () => {
        s.view.updateState(s.view.state.reconfigure({ plugins: s.view.state.plugins.concat(new m.state.Plugin({})) }));
        await s.wait();
        return {
          moving: cell.classList.contains("is-moving"),
          avatar: !!document.querySelector(".composer-table__drag-avatar"),
        };
      });
      return { during, value: s.value, avatar: !!document.querySelector(".composer-table__drag-avatar") };
    });
    expect(result).toEqual({
      during: { moving: true, avatar: true },
      value: `| h1 | h2 | h3 |\n|----|----|----|\n| b1 | b2 | b3 |\n| a1 | a2 | a3 |\n\n`,
      avatar: false,
    });
  });

  test("destroying the editor mid-drag drops the drag", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s) => {
      const view = s.view;
      return await s.dragGrip(s.findAll(".composer-table__grip.--row")[1], s.findAll(".composer-table tr")[2], async () => {
        view.destroy();
        await s.wait();
        return { avatar: !!document.querySelector(".composer-table__drag-avatar") };
      });
    });
    expect(result).toEqual({ avatar: false });
  });

  test("typing across cells is one safe undoable edit", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s, m) => {
      await s.setSelection(s.cellPos(0, 2) + 2, s.cellPos(1, 0) + 2);
      s.view.dom.dispatchEvent(
        new KeyboardEvent("keypress", { bubbles: true, cancelable: true, charCode: 88, key: "X", keyCode: 88 })
      );
      await s.wait();
      const typed = s.value;
      const caret = s.view.state.selection.empty;
      await s.apply(m.history.undo);
      return { typed, caret, undone: s.value };
    });
    expect(result.typed).toBe(`| h1 | h2 | hX |\n|----|----|----|\n| 1 | a2 | a3 |\n| b1 | b2 | b3 |\n\n`);
    expect(result.caret).toBe(true);
    expect(result.undone).toBe(TABLE_OUT);
  });

  test("pasting text across cells preserves the grid", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const value = await inPage(page, async (s) => {
      await s.setSelection(s.cellPos(1, 0) + 2, s.cellPos(1, 1) + 2);
      s.view.pasteHTML("<p>Z</p>");
      await s.wait();
      return s.value;
    });
    expect(value).toBe(`| h1 | h2 | h3 |\n|----|----|----|\n| aZ | 2 | a3 |\n| b1 | b2 | b3 |\n\n`);
  });

  test("cutting across cells preserves the grid and is undoable", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const result = await inPage(page, async (s, m) => {
      const clipboardData = new DataTransfer();
      await s.setSelection(s.cellPos(1, 0) + 2, s.cellPos(1, 1) + 2);
      s.view.dom.dispatchEvent(new ClipboardEvent("cut", { bubbles: true, cancelable: true, clipboardData }));
      await s.wait();
      const cut = s.value;
      await s.apply(m.history.undo);
      return { text: clipboardData.getData("text/plain"), cut, undone: s.value };
    });
    expect(result.text).toBe("1\n\na");
    expect(result.cut).toBe(`| h1 | h2 | h3 |\n|----|----|----|\n| a | 2 | a3 |\n| b1 | b2 | b3 |\n\n`);
    expect(result.undone).toBe(TABLE_OUT);
  });

  test("a row left short of cells is padded back out", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const value = await inPage(page, async (s) => {
      const pos = s.cellPos(1, 2);
      const cell = s.view.state.doc.nodeAt(pos);
      s.view.dispatch(s.view.state.tr.delete(pos, pos + cell.nodeSize));
      await s.wait();
      return s.value;
    });
    expect(value).toBe(`| h1 | h2 | h3 |\n|----|----|----|\n| a1 | a2 |  |\n| b1 | b2 | b3 |\n\n`);
  });

  test("pasting a table into a cell fills the grid instead of nesting", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const value = await inPage(page, async (s) => {
      await s.selectCell(1, 1);
      s.view.pasteHTML("<table><tbody><tr><td>x1</td><td>x2</td></tr><tr><td>y1</td><td>y2</td></tr></tbody></table>");
      await s.wait();
      return s.value;
    });
    expect(value).toBe(`| h1 | h2 | h3 |\n|----|----|----|\n| a1 | x1 | x2 |\n| b1 | y1 | y2 |\n\n`);
  });

  test("pasting a table larger than the grid grows it", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const value = await inPage(page, async (s) => {
      await s.selectCell(2, 2);
      s.view.pasteHTML("<table><tbody><tr><td>x1</td><td>x2</td></tr><tr><td>y1</td><td>y2</td></tr></tbody></table>");
      await s.wait();
      return s.value;
    });
    expect(value).toBe(
      `| h1 | h2 | h3 |  |\n|----|----|----|----|\n| a1 | a2 | a3 |  |\n| b1 | b2 | x1 | x2 |\n|  |  | y1 | y2 |\n\n`
    );
  });

  test("pasting a table alongside other content keeps the other content", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const rules = await inPage(page, async (s) => {
      await s.selectCell(1, 1);
      s.view.pasteHTML("<hr><table><tbody><tr><td>x1</td></tr></tbody></table>");
      await s.wait();
      let count = 0;
      s.view.state.doc.descendants((node) => {
        if (node.type.name === "horizontal_rule") {
          count++;
        }
      });
      return count;
    });
    expect(rules).toBe(1);
  });

  test("structural edits undo in one step", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const value = await inPage(page, async (s, m) => {
      await s.selectCell(1, 1);
      await s.apply(m.commands.addColumn(1));
      await s.apply(m.history.undo);
      return s.value;
    });
    expect(value).toBe(TABLE_OUT);
  });

  test("an imported table with no rows is removed safely", async ({ page }) => {
    await setupRichEditor(page, "before");
    const first = await inPage(page, async (s) => {
      const { schema, tr } = s.view.state;
      const emptyTable = schema.nodes.table.create(null, schema.nodes.table_body.create());
      s.view.dispatch(tr.replaceWith(0, s.view.state.doc.content.size, emptyTable));
      await s.wait();
      return s.view.state.doc.firstChild.type.name;
    });
    await expect(page.locator(".composer-table")).toHaveCount(0);
    expect(first).not.toBe("table");
  });

  test("dragging a row grip past another row reorders them", async ({ page }) => {
    await setupRichEditor(page, TABLE);
    const value = await inPage(page, async (s) => {
      await s.dragGrip(s.findAll(".composer-table__grip.--row")[1], s.findAll(".ProseMirror .composer-table table tr")[2]);
      return s.value;
    });
    expect(value).toBe(`| h1 | h2 | h3 |\n|----|----|----|\n| b1 | b2 | b3 |\n| a1 | a2 | a3 |\n\n`);
  });

  for (const [direction, [from, to, expected]] of Object.entries({
    right: [0, 2, `| a much longer heading | x | short |\n|----|----|----|\n| a much longer value | three | one |\n\n`],
    left: [2, 0, `| x | short | a much longer heading |\n|----|----|----|\n| three | one | a much longer value |\n\n`],
  })) {
    test(`dragging a column grip to the ${direction} reorders unequal cells`, async ({ page }) => {
      await setupRichEditor(page, UNEVEN_TABLE);
      const result = await inPage(
        page,
        async (s, m, { from, to }) => {
          const during = await s.dragGrip(
            s.findAll(".composer-table__grip.--column")[from],
            s.findAll(".ProseMirror .composer-table table th")[to],
            () => {
              const avatar = document.querySelector(".composer-table__drag-avatar.--column");
              return { avatar: !!avatar, x: !!avatar && avatar.style.transform.startsWith("translateX(") };
            }
          );
          return { during, value: s.value };
        },
        { from, to }
      );
      expect(result.during).toEqual({ avatar: true, x: true });
      expect(result.value).toBe(expected);
    });
  }

  // to-markdown-test.js's table cases.
  for (const [name, cases] of Object.entries({
    "converts table tags": [
      [
        `<address>Discourse Avenue</address><b>laboris</b>
    <table>
      <thead> <tr><th>Heading 1</th><th>Head 2</th></tr> </thead>
        <tbody>
          <tr><td>Lorem</td><td>ipsum</td></tr>
          <tr><td><b>dolor</b></td> <td><i>sit amet</i></td> </tr>

          </tbody>
  </table>
    `,
        `Discourse Avenue\n\n**laboris**\n\n| Heading 1 | Head 2 |\n|----|----|\n| Lorem | ipsum |\n| **dolor** | *sit amet* |`,
      ],
      [
        `<table>
              <tr><th>Heading 1</th><th>Head 2</th></tr>
              <tr><td><a href="http://example.com"><img src="http://example.com/image|large.png" alt="Lorem" width="45" height="45" title="wide|image"></a></td><td>ipsum</td></tr>
              <tr><td>x | y</td><td><code>a|b</code></td></tr>
              <tr><td><a class="attachment" href="http://example.com/file|v.pdf">file.pdf</a></td><td><ruby lang="ja|latin">字</ruby></td></tr>
            </table>`,
        `| Heading 1 | Head 2 |\n|----|----|\n| [![Lorem\\|45x45](http://example.com/image\\|large.png "wide\\|image")](http://example.com) | ipsum |\n| x \\| y | \`a\\|b\` |\n| [file.pdf\\|attachment](http://example.com/file\\|v.pdf) | <ruby lang="ja\\|latin">字</ruby> |`,
      ],
    ],
    "table with br in header is still a valid table": [
      [
        `<table>
      <thead> <tr><th>Headi<br><br>ng 1</th><th>Head 2</th></tr> </thead>
        <tbody>
          <tr><td>Lorem</td><td>ipsum</td></tr>
          <tr><td><a href="http://example.com"><img src="http://dolor.com/image.png" /></a></td> <td><i>sit amet</i></td></tr></tbody>
  </table>
    `,
        `| Headi<br><br>ng 1 | Head 2 |\n|----|----|\n| Lorem | ipsum |\n| [![](http://dolor.com/image.png)](http://example.com) | *sit amet* |`,
      ],
    ],
    "normalizes table without thead": [
      [
        `<table>
      <tbody>
        <tr><td>Title</td></tr>
        <tr><td>Column A</td><td>Column B</td></tr>
        <tr><td>Value 1</td><td>Value 2</td></tr>
      </tbody>
    </table>`,
        `| Title |  |\n|----|----|\n| Column A | Column B |\n| Value 1 | Value 2 |`,
      ],
      [`<table><tr><td>Lorem</td><td><strong>sit amet</strong></td></tr></table>`, `| Lorem | **sit amet** |\n|----|----|`],
    ],
    "keeps tables whose cells hold only images": [
      [
        `<table><tr><th><img src="http://example.com/a.png" alt="a"></th><th></th></tr><tr><td></td><td><img src="http://example.com/b.png" alt="b"></td></tr></table>`,
        `| ![a](http://example.com/a.png) |  |\n|----|----|\n|  | ![b](http://example.com/b.png) |`,
      ],
    ],
    "skips tables without content": [
      [`<p>before</p><table><tr><td><br></td><td> </td></tr></table><p>after</p>`, `before\n\nafter`],
    ],
    "converts single-column tables": [
      [
        `<table>
      <thead> <tr><th>Heading 1</th></tr> </thead>
        <tbody>
          <tr><td>Lorem</td></tr>
          <tr><td><i>sit amet</i></td></tr></tbody>
  </table>
    `,
        `| Heading 1 |\n|----|\n| Lorem |\n| *sit amet* |`,
      ],
    ],
  })) {
    test(`to-markdown ${name}`, async ({ page }) => {
      for (const [html, markdown] of cases) {
        const result = await page.evaluate(async (html) => {
          // The engine the extensions read, which the reference's is from the
          // start (lib/markdown-it.js).
          (await import("discourse/rich-editor/engine")).setEngine(window.richSpec.engine);
          const toMarkdown = (await import("discourse/rich-editor/to-markdown")).default;
          return toMarkdown(html);
        }, html);
        expect(result, `markdown for ${JSON.stringify(html)}`).toBe(markdown);
      }
    });
  }
});
