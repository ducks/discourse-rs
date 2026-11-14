// The username step of the invite page's code signup (CodeLoginForm's
// signup-details step with CodeLoginUsernameField, drawn by invite.js):
// the availability check as the visitor types (GET /u/check_username
// with the email), its errors, the avatar preview and the Continue
// button. Each test works on an invite, an address and names of its own.
const { test, expect, api, sql, label, captureCodes, codeFor } = require("./helpers");

// A username no other run or test takes.
function username(what) {
  return label(what).replace(/[^a-z0-9]/g, "");
}

// Takes a visitor through an invite link of their own to the username
// step. Returns the address.
async function usernameStep(page, local) {
  const key = username("uk");
  const email = `${local || username("ue")}@example.com`;
  sql(
    "INSERT INTO invites (invite_key, email, invited_by_id, created_at, updated_at, emailed_status, \
       max_redemptions_allowed, redemption_count, expires_at) \
     SELECT :'p1', NULL, id, now(), now(), 0, 5, 0, now() + interval '30 days' FROM users WHERE username = 'user1'",
    key,
  );
  await page.goto(`/invites/${key}`);
  await page.locator("#code-login-email").fill(email);
  await page.locator(".code-login-form__continue").click();
  await expect(page.locator(".code-login-form__code-step")).toBeVisible();
  await page.locator(".d-otp-input").fill(codeFor(email));
  await expect(page.locator(".code-login-form__signup-details-step")).toBeVisible();
  return email;
}

// The check for a name, as the page asks it.
function checked(page, name) {
  return page.waitForResponse((r) => {
    const url = new URL(r.url());
    return url.pathname.endsWith("/u/check_username.json") && url.searchParams.get("username") === name;
  });
}

// The avatar drawn from a template, at any pixel ratio.
function avatarSrc(template) {
  const [before, after] = template.split("{size}").map((part) => part.replace(/[.*+?^$()|[\]\\{}]/g, "\\$&"));
  return new RegExp(before + "\\d+" + after + "$");
}

test.beforeAll(captureCodes);

test("the username step starts empty, its button disabled", async ({ page }) => {
  await usernameStep(page);
  const input = page.locator("#code-login-username");
  await expect(input).toHaveValue("");
  await expect(input).toBeFocused();
  await expect(input).toHaveAttribute("aria-describedby", "code-login-username-instructions code-login-username-error");
  await expect(page.locator(".code-login-form__continue-to-site")).toBeDisabled();
  await expect(page.locator("#code-login-username-error")).toHaveText("");
  await expect(page.locator(".code-login-form__avatar-placeholder")).toBeVisible();
  // enable_random_usernames is off: no dice.
  await expect(page.locator(".code-login-form__username-regen")).toHaveCount(0);
});

test("a name too short or with invalid characters shows the check's errors", async ({ page }) => {
  await usernameStep(page);
  const input = page.locator("#code-login-username");
  const error = page.locator("#code-login-username-error");
  const button = page.locator(".code-login-form__continue-to-site");

  let answer = checked(page, "a");
  await input.fill("a");
  expect((await (await answer).json()).errors).toEqual(["must be at least 3 characters"]);
  await expect(error).toHaveText("must be at least 3 characters");
  await expect(input).toHaveAttribute("aria-invalid", "true");
  await expect(button).toBeDisabled();

  answer = checked(page, "bad name!");
  await input.fill("bad name!");
  // Typing clears the error until the answer comes.
  await answer;
  await expect(error).toHaveText("must only include numbers, letters, dashes, dots, and underscores");
  await expect(button).toBeDisabled();
  await expect(page.locator(".code-login-form__avatar-placeholder")).toBeVisible();

  // An emptied field has no error and asks nothing.
  let asked = 0;
  page.on("request", (r) => {
    if (r.url().includes("/u/check_username")) {
      asked += 1;
    }
  });
  await input.fill("");
  await expect(error).toHaveText("");
  await expect(input).not.toHaveAttribute("aria-invalid", "true");
  // Past the debounce: the next check is the only one asked.
  const name = username("un");
  answer = checked(page, name);
  await input.fill(name);
  await answer;
  expect(asked).toBe(1);
});

test("an available name enables the button and the avatar follows the name", async ({ page }) => {
  const email = await usernameStep(page);
  const input = page.locator("#code-login-username");
  const button = page.locator(".code-login-form__continue-to-site");
  const avatar = page.locator(".code-login-form__avatar");

  const first = username("ua");
  let answer = checked(page, first);
  await input.fill(first);
  let response = await answer;
  // The email goes along, as User.checkUsername sends it.
  expect(new URL(response.url()).searchParams.get("email")).toBe(email);
  let body = await response.json();
  expect(body.available).toBe(true);
  await expect(button).toBeEnabled();
  await expect(page.locator("#code-login-username-error")).toHaveText("");
  await expect(input).not.toHaveAttribute("aria-invalid", "true");
  await expect(avatar.locator("img.avatar")).toHaveAttribute("src", avatarSrc(body.avatar_template));
  await expect(avatar.locator("img.avatar")).toHaveAttribute("width", "144");

  // Another name: the button waits for its check, the avatar is its own.
  const second = "z" + username("ub");
  answer = checked(page, second);
  await input.fill(second);
  await expect(button).toBeDisabled();
  body = await (await answer).json();
  expect(body.avatar_template).not.toBe("");
  await expect(button).toBeEnabled();
  await expect(avatar.locator("img.avatar")).toHaveAttribute("src", avatarSrc(body.avatar_template));
});

test("a taken name shows it isn't available, with the suggestion", async ({ page }) => {
  await usernameStep(page);
  const input = page.locator("#code-login-username");
  const answer = checked(page, "user1");
  await input.fill("user1");
  const body = await (await answer).json();
  expect(body.available).toBe(false);
  expect(body.suggestion).toMatch(/^user1\d+$/);
  await expect(page.locator("#code-login-username-error")).toHaveText(
    `That username isn't available. Try ${body.suggestion}.`,
  );
  await expect(input).toHaveAttribute("aria-invalid", "true");
  // The suggestion is offered, not applied.
  await expect(input).toHaveValue("user1");
  await expect(page.locator(".code-login-form__continue-to-site")).toBeDisabled();
  await expect(page.locator(".code-login-form__avatar-placeholder")).toBeVisible();
});

test("continuing creates the account with the checked username", async ({ page }) => {
  const email = await usernameStep(page);
  const name = username("uc");
  const answer = checked(page, name);
  await page.locator("#code-login-username").fill(name);
  await answer;
  const button = page.locator(".code-login-form__continue-to-site");
  await expect(button).toBeEnabled();
  expect(sql("SELECT COUNT(*) FROM users WHERE username = :'p1'", name)[0][0]).toBe("0");
  await button.click();
  await expect(page.locator("#current-user")).toBeVisible();
  expect(
    sql(
      "SELECT COUNT(*) FROM users u JOIN user_emails e ON e.user_id = u.id AND e.primary \
       WHERE u.username = :'p1' AND e.email = :'p2' AND u.active",
      name,
      email,
    )[0][0],
  ).toBe("1");
});

test("the username instructions show until an error takes their place", async ({ page, session }) => {
  const admin = await session("admin");
  await api(admin, "PUT", "/admin/site_settings/show_signup_form_username_instructions", {
    show_signup_form_username_instructions: "true",
  });
  try {
    await usernameStep(page);
    const instructions = page.locator("#code-login-username-instructions");
    await expect(instructions).toHaveText("Unique, no spaces, short");
    await expect(instructions).toHaveClass("more-info code-login-form__field-instructions");
    const input = page.locator("#code-login-username");
    let answer = checked(page, "a");
    await input.fill("a");
    await answer;
    await expect(page.locator("#code-login-username-error")).not.toHaveText("");
    await expect(instructions).toHaveCount(0);
    const name = username("ui");
    answer = checked(page, name);
    await input.fill(name);
    await answer;
    await expect(instructions).toBeVisible();
  } finally {
    await api(admin, "PUT", "/admin/site_settings/show_signup_form_username_instructions", {
      show_signup_form_username_instructions: "false",
    });
  }
});

test("a username suggested from the email is prefilled and checked", async ({ page, session }) => {
  const admin = await session("admin");
  await api(admin, "PUT", "/admin/site_settings/use_email_for_username_and_name_suggestions", {
    use_email_for_username_and_name_suggestions: "true",
  });
  try {
    const local = username("up");
    const answer = checked(page, local);
    await usernameStep(page, local);
    const body = await (await answer).json();
    expect(body.available).toBe(true);
    await expect(page.locator("#code-login-username")).toHaveValue(local);
    await expect(page.locator(".code-login-form__continue-to-site")).toBeEnabled();
    await expect(page.locator(".code-login-form__avatar img.avatar")).toHaveAttribute(
      "src",
      avatarSrc(body.avatar_template),
    );
  } finally {
    await api(admin, "PUT", "/admin/site_settings/use_email_for_username_and_name_suggestions", {
      use_email_for_username_and_name_suggestions: "false",
    });
  }
});
