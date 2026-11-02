// Chat::MessageProcessor and full-page chat search on a running server: a
// message with a topic link drawn again with its onebox once
// Jobs::Chat::ProcessMessage publishes it processed, in the sender's chat
// and live in another member's; an uploaded image's markdown drawn as an
// image with its lightbox (lightbox.js on the message's cooked); and
// /chat/search from the sidebar (ChatRoutesSearch, chat-search.js):
// finding a message, sorting, the query in the address, opening a result,
// and the query back with the page.
const fs = require("fs");
const path = require("path");
const { test, expect, login, api, sql, label, topic } = require("./helpers");

const CHANNEL = 2; // the seed's General channel

const prefer = (page, mode) =>
  page.evaluate((mode) => localStorage.setItem("discourse_chat_preferred_mode", JSON.stringify(mode)), mode);

test("attaching an image keeps the caption focused when the composer menu finishes closing", async ({ page, context }) => {
  await login(context, "user1");
  await page.goto(`/chat/c/general/${CHANNEL}`);
  // Keep the close animation pending while the file picker returns.
  await page.addStyleTag({ content: ".fk-d-menu.-closing { animation-name: none !important; animation-duration: 2s !important; }" });
  await page.locator(".chat-composer-dropdown__trigger-btn").click();
  const chooser = page.waitForEvent("filechooser");
  await page.locator("button[data-id='chat-upload-btn']").click();
  await (await chooser).setFiles({
    name: "caption.png",
    mimeType: "image/png",
    buffer: Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==", "base64"),
  });
  const composer = page.locator(".full-page-chat .chat-composer__input");
  await composer.fill(label("image caption"));
  const caption = await composer.inputValue();
  await expect(page.locator(".chat-composer-dropdown__list")).toHaveCount(0);
  await expect(composer).toBeFocused();
  await expect(page.locator(".chat-composer-button.-send")).toBeEnabled();
  await composer.press("Enter");
  const sent = page.locator(".chat-message-container", { hasText: caption });
  await expect(sent.locator(".chat-uploads img.chat-img-upload")).toBeVisible();
  await sent.locator(".chat-uploads img.chat-img-upload").click();
  await expect(page.locator(".pswp")).toBeVisible();
});

test("a topic link draws its onebox once processed, live in another member's chat", async ({
  page,
  context,
  session,
}) => {
  const user0 = await session("user0");
  const title = label("Onebox in chat topic");
  const linked = await topic(user0, title, label("a topic linked from chat"));
  await login(context, "user1");
  const other = await user0.newPage();
  await other.goto(`/chat/c/general/${CHANNEL}`);
  await expect(other.locator(".full-page-chat .chat-composer__input")).toBeVisible();

  await page.goto(`/chat/c/general/${CHANNEL}`);
  const marker = label("look at this");
  const url = new URL(linked.url, page.url()).toString();
  const composer = page.locator(".full-page-chat .chat-composer__input");
  await composer.fill(`${marker}\n\n${url}`);
  await composer.press("Enter");

  // The processed message: the link's onebox in place of the link.
  const sent = page.locator(".chat-message-container", { hasText: marker });
  await expect(sent.locator(".chat-cooked aside.quote")).toContainText(title, { timeout: 20_000 });
  // No reload on the other side: the processed event redraws it.
  const arrived = other.locator(".chat-message-container", { hasText: marker });
  await expect(arrived.locator(".chat-cooked aside.quote")).toContainText(title, { timeout: 20_000 });
  await expect(arrived.locator(".chat-cooked a.onebox")).toHaveCount(0);
});

test("an uploaded image's markdown draws as an image with its lightbox", async ({ page, context }) => {
  await login(context, "user1");
  const csrf = await context.request.get("/session/csrf.json", { headers: { "X-Requested-With": "XMLHttpRequest" } });
  const token = (await csrf.json()).csrf;
  const file = path.join(test.info().outputDir, "chat-image.png");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  // A 1x1 png, its bytes made unique so the upload is a new one.
  const png = Buffer.concat([
    Buffer.from(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
      "base64",
    ),
    Buffer.from(label("tail")),
  ]);
  fs.writeFileSync(file, png);
  const res = await context.request.post("/uploads.json", {
    headers: { "X-CSRF-Token": token, "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
    multipart: {
      upload_type: "chat-composer",
      synchronous: "true",
      file: { name: "chat-image.png", mimeType: "image/png", buffer: png },
    },
  });
  expect(res.ok(), await res.text()).toBeTruthy();
  const upload = await res.json();

  const alt = label("image").replace(/[^a-z0-9]+/gi, "");
  await api(context, "POST", `/chat/${CHANNEL}.json`, {
    message: `![${alt}|1x1](${upload.short_url})`,
    staged_id: label("staged"),
  });
  await page.goto(`/chat/c/general/${CHANNEL}`);
  const image = page.locator(`.chat-message-container .chat-cooked img[alt='${alt}']`);
  await expect(image).toHaveClass(/lightbox/, { timeout: 20_000 });
  await expect(image).toHaveAttribute("data-download-href", upload.short_path);
  await image.click();
  await expect(page.locator(".pswp")).toBeVisible();
});

test("search chat from the sidebar finds a message full page, keeps its query, and opens it", async ({
  page,
  context,
}) => {
  await login(context, "user1");
  const word = label("needle").replace(/[^a-z0-9]+/gi, "");
  const message = `full page search ${word}`;
  await api(context, "POST", `/chat/${CHANNEL}.json`, { message, staged_id: label("staged") });
  const [[messageId]] = sql("SELECT id FROM chat_messages WHERE message = :'p1'", message);

  await page.goto("/latest");
  await prefer(page, "FULL_PAGE_CHAT");
  await page.goto(`/chat/c/general/${CHANNEL}`);
  await page.locator("#d-sidebar a[href$='/chat/search']").click();
  await expect(page).toHaveURL(/\/chat\/search$/);
  const search = page.locator(".full-page-chat .c-routes.--search");
  await expect(search.locator(".c-navbar__title-text")).toHaveText("Search chat");
  await expect(page.locator("#d-sidebar a[href$='/chat/search']")).toHaveClass(/active/);
  const input = search.locator(".c-search .filter-input");
  await expect(input).toBeFocused();

  await input.fill(word);
  const entry = search.locator(".chat-message-search-entry", { hasText: message });
  await expect(entry).toBeVisible();
  await expect(entry.locator(".chat-channel-title")).toContainText("General");
  // The query in the address (replace: true).
  await expect(page).toHaveURL(new RegExp(`/chat/search\\?q=${word}$`));

  await search.locator(".search-sort-options-trigger").click();
  await page.locator(".search-sort-options-content [data-sort='latest']").click();
  await expect(search.locator(".search-sort-options-trigger .d-button-label")).toHaveText("Latest");
  await expect(entry).toBeVisible();

  // The query comes back with the page.
  await page.reload();
  await expect(page.locator(".c-search .filter-input")).toHaveValue(word);
  await expect(page.locator(".chat-message-search-entry", { hasText: message })).toBeVisible();

  // A result opens its message in the channel.
  await page.locator(".chat-message-search-entry", { hasText: message }).click();
  // ChatChannelNearMessage drops the target from the URL after jumping.
  await expect(page).toHaveURL(new RegExp(`/chat/c/general/${CHANNEL}$`));
  await expect(page.locator(".full-page-chat .chat-channel")).toHaveAttribute("data-target", messageId);
  await expect(page.locator(`.full-page-chat .chat-message-container[data-id='${messageId}']`)).toBeVisible();
});
