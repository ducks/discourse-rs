// The options menu's Insert table (ComposerService#insertTable, ROADMAP
// 196r6): a four-column table with its default headings and an empty row
// put in as a block, in the markdown editor as markdown and in the rich
// editor as a table with its editing controls. The reference's
// composer-insert-table-test.js and composer_insert_table_spec.rb.
const { test, expect, login, topic, sql, label, member } = require("./helpers");

const TABLE = [
  "| Column 1 | Column 2 | Column 3 | Column 4 |",
  "| --- | --- | --- | --- |",
  "|  |  |  |  |",
].join("\n");

const textarea = (page) => page.locator("#reply-control textarea.d-editor-input");
const rich = (page) => page.locator("#reply-control .ProseMirror.d-editor-input");

async function insertTable(page) {
  await page.locator("#reply-control .toolbar-menu__options-trigger").click();
  await page.locator(".toolbar-menu__options-content button[data-name='toggle-spreadsheet']").click();
}

// A member of the test's own in a composition mode (0 markdown, 1 rich).
async function writer(context, mode) {
  const m = member();
  sql("UPDATE user_options SET composition_mode = :'p1' WHERE user_id = :'p2'", mode, m.id);
  await login(context, m.name);
  return m;
}

test.describe("Composer - Insert table | Markdown editor", () => {
  test("inserts a Markdown table into a reply", async ({ page, context }) => {
    await writer(context, 0);
    const t = await topic(context, label("Insert table topic"), label("A topic to reply to with a table"));
    await page.goto(t.url);
    await page.locator("#topic-footer-buttons .create").click();
    await expect(textarea(page)).toBeVisible();
    await textarea(page).fill("Some text");
    await insertTable(page);

    await expect(textarea(page), "the table starts on its own line, ready to edit").toHaveValue(
      `Some text\n\n${TABLE}\n`,
    );
  });

  test("inserts a Markdown table when editing a post", async ({ page, context }) => {
    await writer(context, 0);
    const t = await topic(context, label("Insert table edit topic"), label("A post to edit with a table"));
    await page.goto(t.url);
    const post = page.locator(`article[data-post-id="${t.posts[0]}"]`);
    const showMore = post.locator(".post-action-menu__show-more");
    if (await showMore.isVisible()) {
      await showMore.click();
    }
    await post.locator(".post-action-menu__edit").click();
    await expect(textarea(page)).not.toHaveValue("");
    await insertTable(page);

    await expect
      .poll(() => textarea(page).inputValue(), { message: "the table is added to the post's Markdown" })
      .toContain(TABLE);
  });

  // composer_insert_table_spec.rb's "inserts a Markdown table"
  test("inserts a Markdown table into a new topic", async ({ page, context }) => {
    await writer(context, 0);
    await page.goto("/latest");
    await page.locator("#create-topic").click();
    await expect(textarea(page)).toBeVisible();
    await textarea(page).fill("Some text");
    await insertTable(page);

    await expect(textarea(page)).toHaveValue(`Some text\n\n${TABLE}\n`);
  });
});

test.describe("Composer - Insert table | rich editor", () => {
  test("inserts a table with editing controls", async ({ page, context }) => {
    await writer(context, 1);
    await page.goto("/latest");
    await page.locator("#create-topic").click();
    await expect(rich(page)).toBeVisible();
    await insertTable(page);

    await expect(rich(page).locator("table th"), "the table has four columns").toHaveCount(4);
    await expect(rich(page).locator("table th").first(), "the columns have default headings").toHaveText("Column 1");
    await expect(
      rich(page).locator(".composer-table__append").first(),
      "the structural editing controls are available",
    ).toBeAttached();
  });
});

test("does not offer to edit tables in published posts", async ({ page, context }) => {
  await writer(context, 0);
  const t = await topic(
    context,
    label("Published table topic"),
    `${label("A table")}\n\n|Make | Model | Year|\n|--- | --- | ---|\n|Toyota | Supra | 1998|`,
  );
  await page.goto(t.url);

  await expect(page.locator(`article[data-post-id="${t.posts[0]}"] .cooked`)).toContainText("Toyota");
  await expect(page.locator(".btn-edit-table")).toHaveCount(0);
});
