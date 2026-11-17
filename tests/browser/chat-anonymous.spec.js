// Discourse 00f21b36e14: anonymous visitors read the public channels
// (chat_allowed_groups holds anonymous_users by default). A visitor sees a
// member's message in General with the log in and sign up card where the
// composer would be, finds the channel in the sidebar, lands on the
// channels list from /chat, and signs up from the card.
const { test, expect, api, label } = require("./helpers");

const CHANNEL = 2; // the seed's General channel

test("an anonymous visitor reads a public channel and signs up from it", async ({ page, session }) => {
  const message = label("anonymous reader");
  await api(await session("user0"), "POST", `/chat/${CHANNEL}.json`, { message, staged_id: label("staged") });

  await page.goto(`/chat/c/general/${CHANNEL}`);
  await expect(page.locator(".full-page-chat .chat-message-container", { hasText: message })).toBeVisible();
  const card = page.locator(".chat-channel-preview-card.--anon");
  await expect(card).toBeVisible();
  await expect(card.locator(".chat-channel-preview-card__title")).toHaveText("Join #General to start chatting");
  await expect(page.locator(".chat-composer__input")).toHaveCount(0);
  await expect(page.locator(".chat-header-icon")).toHaveCount(0);
  await expect(
    page.locator(".sidebar-section[data-section-name='chat-channels'] .sidebar-section-link", { hasText: "General" }),
  ).toBeVisible();

  await card.getByRole("button", { name: "Sign up" }).click();
  await expect(page).toHaveURL(/\/signup$/);
});

test("an anonymous visitor's /chat is the channels list", async ({ page }) => {
  await page.goto("/chat");
  await expect(page).toHaveURL(/\/chat\/channels$/);
  const row = page.locator(".c-routes.--channels .chat-channel-row", { hasText: "General" });
  await expect(row).toBeVisible();
  await row.click();
  await expect(page).toHaveURL(new RegExp(`/chat/c/general/${CHANNEL}$`));
  await expect(page.locator(".chat-channel-preview-card.--anon")).toBeVisible();
});
