// Demo step 11, dark mode: the interface color selector in the sidebar
// footer (interface-color.js) flips the color definitions' stylesheets at
// once, writes forced_color_mode, and the server draws the next page in
// that mode (crate::color_mode); "auto" follows the browser's scheme.
const { test, expect, login } = require("./helpers");

const background = (page) => page.evaluate(() => getComputedStyle(document.documentElement).backgroundColor);
const media = (page) =>
  page.evaluate(() => [
    document.querySelector("link.light-scheme").media,
    document.querySelector("link.dark-scheme").media,
  ]);

async function choose(page, mode) {
  await page.locator("#d-sidebar .interface-color-selector").click();
  const menu = page.locator(".interface-color-selector-content");
  await expect(menu).toBeVisible();
  await menu.locator(`[data-mode="${mode}"]`).click();
  await expect(menu).toHaveCount(0);
}

test("the color selector switches to dark and back, and the next page keeps it", async ({ page, context }) => {
  await login(context, "user1");
  await page.goto("/latest");
  const light = await background(page);
  expect(await media(page)).toEqual(["(prefers-color-scheme: light)", "(prefers-color-scheme: dark)"]);

  await choose(page, "dark");
  expect(await media(page)).toEqual(["none", "all"]);
  await expect.poll(() => background(page)).not.toBe(light);
  const dark = await background(page);
  await expect(page.locator("#d-sidebar .interface-color-selector")).toHaveAttribute("data-current-mode", "dark");
  const cookie = (await context.cookies()).find((c) => c.name === "forced_color_mode");
  expect(cookie && cookie.value).toBe("dark");

  // The server reads the cookie: a new page is dark from the start.
  await page.reload();
  expect(await media(page)).toEqual(["none", "all"]);
  expect(await background(page)).toBe(dark);

  await choose(page, "light");
  expect(await media(page)).toEqual(["all", "none"]);
  await expect.poll(() => background(page)).toBe(light);

  // Auto follows the browser: with a dark preference, the page is dark.
  await choose(page, "auto");
  expect(await media(page)).toEqual(["(prefers-color-scheme: light)", "(prefers-color-scheme: dark)"]);
  await page.emulateMedia({ colorScheme: "dark" });
  await expect.poll(() => background(page)).toBe(dark);
  await page.emulateMedia({ colorScheme: "light" });
  await expect.poll(() => background(page)).toBe(light);
});
