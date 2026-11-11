// The quote toolbar (quote-button.js, components/post-text-selection.gjs)
// handing its quote to the composer as TopicController#selectText does:
// into a reply, or into a new topic where the reader may not reply.
const { test, expect, login, topic, sql, label, member } = require("./helpers");

const editor = (page) => page.locator("#reply-control.open textarea.d-editor-input");

// A topic by user1, a member of the test's own reading it with the first
// post's paragraph selected and the toolbar up.
async function selected(page, context, session, prepare) {
  const user1 = await session("user1");
  const text = label("A paragraph the reader quotes");
  const t = await topic(user1, label("Quote button"), text);
  if (prepare) {
    prepare(t);
  }
  await login(context, member().name);
  await page.goto(t.url);
  const paragraph = page.locator(`article[data-post-id="${t.posts[0]}"] .cooked p`).first();
  await expect(paragraph).toHaveText(text);
  await paragraph.evaluate((p) => {
    const range = document.createRange();
    range.selectNodeContents(p);
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
  });
  const quote = page.locator(".quote-button .insert-quote");
  await expect(quote).toBeVisible();
  const [[title]] = sql("SELECT title FROM topics WHERE id = :'p1'", t.id);
  return { t, text, title, quote };
}

test("Quote opens a reply with the quote", async ({ page, context, session }) => {
  const { t, text, quote } = await selected(page, context, session);
  await quote.click();
  await expect(page.locator("#reply-control.open.composer-action-reply")).toBeVisible();
  await expect(editor(page)).toHaveValue(`[quote="user1, post:1, topic:${t.id}, full:true"]\n${text}\n[/quote]\n\n`);
});

test("Quote in a closed topic starts a new topic continuing from the post", async ({ page, context, session }) => {
  const { t, text, title, quote } = await selected(page, context, session, (t) =>
    sql("UPDATE topics SET closed = true WHERE id = :'p1'", t.id),
  );
  await quote.click();
  await expect(page.locator("#reply-control.open.composer-action-create-topic")).toBeVisible();
  const link = new URL(t.url, page.url()).href;
  await expect(editor(page)).toHaveValue(
    `Continuing the discussion from [${title}](${link}):\n\n[quote="user1, post:1, topic:${t.id}, full:true"]\n${text}\n[/quote]\n\n`,
  );
});
