// A multiselect user field on the profile preferences (preferences.js
// over prefs-choosers.js's MultiSelect: the choices drawn, saved as a
// list, emptied) and the invite form's user fields (invite.js over
// user-fields.js, the helper login.js's signup form shares: the values
// sent under user_custom_fields, the button disabled while a field
// fails).
//
// user_fields rows are the whole site's. The preferences field is a row
// of its own: optional, editable, off the profile and off signup (so no
// other spec's signup or invite form draws it), removed at the end.
//
// The invite form is another matter: the suite's site redeems invites by
// emailed code, whose page has no such form and whose server answers the
// form's PUT with a 404, and signup-user-fields.spec.js counts the fields
// shown on signup, so none can be added. Its tests put the form in the
// invite's page as the server draws it without login codes
// (invite_page_view's classic_form with signup_user_fields) and answer
// the PUT themselves, reading what the form sends; what the server draws
// and stores is tests/user_fields_rest.rs.
const { test, expect, sql, label, member, login } = require("./helpers");

const PASSWORD = "a-long-enough-pass-phrase";
const TEXT = 987001;
const CONFIRM = 987002;
const MULTI = 987003;

// A name no other run or test takes.
function unique(what) {
  return label(what).replace(/[^a-z0-9]/g, "");
}

let field;
let tag;
let invite;

test.beforeAll(() => {
  tag = unique("ufr");
  field = Number(
    sql(
      "INSERT INTO user_fields (name, description, field_type_enum, editable, requirement, show_on_profile, \
         show_on_user_card, show_on_signup, searchable, position, created_at, updated_at) \
       VALUES (:'p1', 'The <b>languages</b>', 3, true, 0, false, false, false, false, 9101, now(), now()) RETURNING id",
      `Langs ${tag}`,
    )[0][0],
  );
  for (const option of ["rust", "ruby", "go"]) {
    sql(
      "INSERT INTO user_field_options (user_field_id, value, created_at, updated_at) VALUES (:'p1', :'p2', now(), now())",
      field,
      option,
    );
  }
  invite = `ufr${tag}`.slice(0, 32);
  sql(
    "INSERT INTO invites (invite_key, invited_by_id, created_at, updated_at, emailed_status, max_redemptions_allowed, \
       redemption_count, expires_at) \
     SELECT :'p1', id, now(), now(), 0, 5, 0, now() + interval '30 days' FROM users WHERE username = 'user1'",
    invite,
  );
});

test.afterAll(() => {
  if (invite) {
    sql("DELETE FROM invites WHERE invite_key = :'p1'", invite);
  }
  if (field) {
    sql("DELETE FROM user_custom_fields WHERE name = :'p1'", `user_field_${field}`);
    sql("DELETE FROM user_field_options WHERE user_field_id = :'p1'", field);
    sql("DELETE FROM user_fields WHERE id = :'p1'", field);
  }
});

// The member's values for the spec's field, in the rows' order.
function stored(name) {
  return sql(
    "SELECT '[' || COALESCE(f.value, '') || ']' FROM user_custom_fields f JOIN users u ON u.id = f.user_id \
     WHERE u.username = :'p1' AND f.name = :'p2' ORDER BY f.id",
    name,
    `user_field_${field}`,
  ).map((row) => row[0]);
}

// Saves the profile: the PUT's pairs for the spec's field.
async function save(page, name) {
  const done = page.waitForResponse(
    (r) => r.request().method() === "PUT" && new URL(r.url()).pathname === `/u/${name}.json`,
  );
  await page.locator(".save-changes").click();
  const response = await done;
  expect(response.status()).toBe(200);
  await expect(page.locator(".save-button .saved")).toBeVisible();
  return [...new URLSearchParams(response.request().postData())].filter(([key]) =>
    key.startsWith(`user_fields[${field}]`),
  );
}

test("a member chooses a multiselect's options on their profile, which stores a row each", async ({
  context,
  page,
  session,
}) => {
  const { name } = member();
  await login(context, name);
  await page.goto(`/u/${name}/preferences/profile`);
  const chooser = page.locator(`.user-preferences [data-user-field="${field}"]`);
  const component = page.locator(`.user-preferences .user-field:has([data-user-field="${field}"])`);
  await expect(component).toHaveClass(new RegExp(`^user-field multiselect user-field-langs-${tag}$`));
  await expect(component.locator("label.alt-placeholder")).toHaveText(`Langs ${tag}(optional)`);
  await expect(component.locator(".instructions b")).toHaveText("languages");
  await expect(chooser.locator(".formatted-selection")).toHaveText("(select an option)");

  await chooser.locator("summary").click();
  await expect(chooser).toHaveClass(/is-expanded/);
  await expect(chooser.locator(".select-kit-collection .select-kit-row")).toHaveText(["rust", "ruby", "go"]);
  await chooser.locator(".filter-input").fill("ru");
  await expect(chooser.locator(".select-kit-collection .select-kit-row")).toHaveText(["rust", "ruby"]);
  await chooser.locator('.select-kit-row[data-value="ruby"]').click();
  await expect(chooser.locator(".formatted-selection")).toHaveText("ruby");
  await expect(chooser.locator(".select-kit-collection .select-kit-row")).toHaveText(["rust", "go"]);
  await chooser.locator('.select-kit-row[data-value="rust"]').click();
  await expect(chooser.locator(".formatted-selection")).toHaveText("ruby, rust");
  await expect(chooser).toHaveClass(/has-selection/);
  await chooser.locator(".filter-input").press("Escape");
  await expect(chooser).not.toHaveAttribute("open", "");

  expect(await save(page, name)).toEqual([
    [`user_fields[${field}][]`, "ruby"],
    [`user_fields[${field}][]`, "rust"],
  ]);
  expect(stored(name)).toEqual(["[ruby]", "[rust]"]);

  // The page draws the stored choices, for staff too.
  await page.reload();
  await expect(chooser.locator(".formatted-selection")).toHaveText("ruby, rust");
  await expect(chooser).toHaveClass(/has-selection/);
  const admin = await session("admin");
  const staffPage = await admin.newPage();
  await staffPage.goto(`/u/${name}/preferences/profile`);
  await expect(
    staffPage.locator(`.user-preferences [data-user-field="${field}"] .formatted-selection`),
  ).toHaveText("ruby, rust");

  // One choice removed: the other is the one row.
  await chooser.locator("summary").click();
  await expect(chooser.locator(".selected-choice")).toHaveText(["ruby", "rust"]);
  await chooser.locator('.selected-choice[data-value="ruby"]').click();
  await expect(chooser.locator(".formatted-selection")).toHaveText("rust");
  await chooser.locator(".filter-input").press("Escape");
  expect(await save(page, name)).toEqual([[`user_fields[${field}][]`, "rust"]]);
  expect(stored(name)).toEqual(["[rust]"]);

  // Emptied: the header's none again, sent as null (an empty value).
  await chooser.locator("summary").click();
  await chooser.locator('.selected-choice[data-value="rust"]').click();
  await expect(chooser.locator(".formatted-selection")).toHaveText("(select an option)");
  await expect(chooser).not.toHaveClass(/has-selection/);
  await chooser.locator(".filter-input").press("Escape");
  expect(await save(page, name)).toEqual([[`user_fields[${field}]`, ""]]);
  expect(stored(name)).toEqual(["[]"]);
  await page.reload();
  await expect(chooser.locator(".formatted-selection")).toHaveText("(select an option)");
});

test("a profile saved without touching the multiselect keeps its choices", async ({ context, page }) => {
  const { name, id } = member();
  for (const value of ["go", "rust"]) {
    sql(
      "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) VALUES (:'p1', :'p2', :'p3', now(), now())",
      id,
      `user_field_${field}`,
      value,
    );
  }
  await login(context, name);
  await page.goto(`/u/${name}/preferences/profile`);
  const chooser = page.locator(`.user-preferences [data-user-field="${field}"]`);
  await expect(chooser.locator(".formatted-selection")).toHaveText("go, rust");
  expect(await save(page, name)).toEqual([
    [`user_fields[${field}][]`, "go"],
    [`user_fields[${field}][]`, "rust"],
  ]);
  expect(stored(name)).toEqual(["[go]", "[rust]"]);
});

// The invite page with the form the server draws without login codes in
// the code form's place: email, username, password, and a text, a confirm
// and a multiselect user field, required or not as far as the page knows.
function classicForm(isRequired) {
  return (html) => {
    const optional = isRequired ? "" : "(optional)";
    const fields =
      `<div class="user-fields">` +
      `<div class="input-group"><div class="user-field text user-field-color"><div class="controls"><input id="user-field-${TEXT}-input" maxlength="2048" class="ember-text-field" type="text" value="" data-user-field="${TEXT}"><label class="control-label alt-placeholder" for="user-field-${TEXT}-input">Color${optional}</label><div class="instructions">Any color</div></div></div></div>` +
      `<div class="input-group"><div class="user-field confirm user-field-rules"><label class="control-label">Rules${optional}</label><div class="controls"><label class="control-label checkbox-label"><input id="user-field-${CONFIRM}-input" class="ember-checkbox" type="checkbox" data-user-field="${CONFIRM}"> <span>Read them</span></label><div class="tip good hidden"></div></div></div></div>` +
      `<div class="input-group"><div class="user-field multiselect user-field-langs"><label class="control-label alt-placeholder" for="user-field-${MULTI}-input">Langs${optional}</label><div class="controls"><details id="user-field-${MULTI}-input" data-user-field="${MULTI}" data-none="(select an option)" data-options="[&quot;rust&quot;,&quot;ruby&quot;,&quot;go&quot;]" class="select-kit multi-select" data-chooser="options" data-items="[]"><summary data-name="" data-value="" tabindex="0" class="select-kit-header multi-select-header"><div class="select-kit-header-wrapper"><span class="formatted-selection">(select an option)</span></div></summary><div class="select-kit-body"></div></details><div class="instructions">Languages</div></div></div></div>` +
      `</div>`;
    const form =
      `<form class="invite-signup" hx-boost="false">` +
      `<div class="input email-input input-group"><input id="new-account-email" class="ember-text-field ember-view" name="email" type="email"><label class="alt-placeholder" for="new-account-email">Email</label><div class="tip bad" id="account-email-validation"></div></div>` +
      `<div class="input username-input input-group"><input autocomplete="off" class="" id="new-account-username" maxlength="20" name="username" value="" type="text"><label class="alt-placeholder" for="new-account-username">Username</label><div class="tip bad" id="username-validation"></div></div>` +
      `<div class="input password-input input-group"><input placeholder="" autocomplete="new-password" id="new-account-password" class="ember-text-field ember-view" type="password"><label class="alt-placeholder" for="new-account-password">Password</label></div>` +
      fields +
      `<div class="invitation-cta"><button class="btn btn-primary invitation-cta__accept" type="submit"><span class="d-button-label">Accept Invitation</span></button></div>` +
      `<div class="alert alert-error" hidden></div></form>`;
    const start = html.indexOf('<div class="code-login-form"');
    const tail = "</div></div></div></section>";
    const end = html.indexOf(tail, start);
    if (start === -1 || end === -1) {
      throw new Error("the invite page has no code form to replace");
    }
    html = html.slice(0, start) + form + html.slice(end);
    const choosers = {
      default_header: "(select an option)",
      filter_placeholder: "Search…",
      results_one: "%{count} result",
      results_other: "%{count} results",
      delete_item: "Delete %{name}",
    };
    return html.replace(
      /(<script type="application\/json" id="invite-strings">)(.*?)(<\/script>)/s,
      (_, open, json, close) => {
        const strings = JSON.parse(json);
        // After the site's own, which the form here does not draw.
        strings.user_fields = [
          { id: TEXT, name: "Color", field_type: "text", required: isRequired },
          { id: CONFIRM, name: "Rules", field_type: "confirm", required: isRequired },
          { id: MULTI, name: "Langs", field_type: "multiselect", required: isRequired },
        ];
        return (
          `<script type="application/json" id="prefs-choosers-strings">${JSON.stringify(choosers)}</script><script src="/assets/prefs-choosers.js"></script>` +
          open +
          JSON.stringify(strings).replace(/</g, "\\u003c") +
          close
        );
      },
    );
  };
}

// Opens the invite with the form. Returns the PUTs the form sent, each
// answered as a redemption.
async function openInvite(page, isRequired) {
  const patch = classicForm(isRequired);
  await page.route(
    (url) => url.pathname === `/invites/${invite}`,
    async (route) => {
      const response = await route.fetch();
      await route.fulfill({ response, body: patch(await response.text()) });
    },
  );
  const sent = [];
  await page.route(
    (url) => url.pathname === `/invites/show/${invite}.json`,
    async (route) => {
      sent.push(route.request());
      await route.fulfill({ json: { success: "OK", message: "Check your email" } });
    },
  );
  await page.goto(`/invites/${invite}`);
  await expect(page.locator("form.invite-signup .user-fields")).toBeVisible();
  return sent;
}

async function account(page) {
  const name = unique("inv");
  await page.locator("#new-account-email").fill(`${name}@example.com`);
  await page.locator("#new-account-username").fill(name);
  await page.locator("#new-account-password").fill(PASSWORD);
  return name;
}

function invited(page, id) {
  return {
    component: page.locator(`form.invite-signup .user-field:has([data-user-field="${id}"])`),
    control: page.locator(`form.invite-signup [data-user-field="${id}"]`),
  };
}

function pairs(request) {
  return [...new URLSearchParams(request.postData())].filter(([key]) => key.startsWith("user_custom_fields"));
}

async function accepted(page, sent) {
  await expect(page.locator(".invite-success .success-info")).toHaveText("Check your email");
  expect(sent.length).toBe(1);
  expect(sent[0].method()).toBe("PUT");
  return new URLSearchParams(sent[0].postData());
}

test("the invite form sends its user fields' values under user_custom_fields", async ({ page }) => {
  const sent = await openInvite(page, false);
  const name = await account(page);
  const submit = page.locator(".invitation-cta__accept");
  // Optional fields: nothing keeps the button.
  await expect(submit).toBeEnabled();

  const text = invited(page, TEXT);
  await text.control.fill("Bright red");
  await expect(text.component).toHaveClass(/value-entered/);
  await text.control.fill("");
  await expect(text.component).not.toHaveClass(/value-entered/);
  await text.control.fill("Bright red");
  await invited(page, CONFIRM).control.check();

  const multi = invited(page, MULTI);
  await multi.control.locator("summary").click();
  await expect(multi.control).toHaveClass(/is-expanded/);
  await expect(multi.control.locator(".select-kit-collection .select-kit-row")).toHaveText(["rust", "ruby", "go"]);
  await multi.control.locator('.select-kit-row[data-value="ruby"]').click();
  await multi.control.locator('.select-kit-row[data-value="rust"]').click();
  await expect(multi.control.locator(".formatted-selection")).toHaveText("ruby, rust");
  await expect(multi.component).toHaveClass(/value-entered/);
  await multi.control.locator(".filter-input").press("Escape");
  await expect(multi.control).not.toHaveAttribute("open", "");

  await submit.click();
  const data = await accepted(page, sent);
  expect(data.get("username")).toBe(name);
  expect(data.get("email")).toBe(`${name}@example.com`);
  expect(pairs(sent[0])).toEqual([
    [`user_custom_fields[${TEXT}]`, "Bright red"],
    [`user_custom_fields[${CONFIRM}]`, "true"],
    [`user_custom_fields[${MULTI}][]`, "ruby"],
    [`user_custom_fields[${MULTI}][]`, "rust"],
  ]);
});

test("the invite form's fields left alone go empty, an emptied multiselect not at all", async ({ page }) => {
  let sent = await openInvite(page, false);
  await account(page);
  await page.locator(".invitation-cta__accept").click();
  await accepted(page, sent);
  // null as jQuery sends it, a confirm false.
  expect(pairs(sent[0])).toEqual([
    [`user_custom_fields[${TEXT}]`, ""],
    [`user_custom_fields[${CONFIRM}]`, "false"],
    [`user_custom_fields[${MULTI}]`, ""],
  ]);

  await page.unrouteAll({ behavior: "wait" });
  sent = await openInvite(page, false);
  await account(page);
  const multi = invited(page, MULTI);
  await multi.control.locator("summary").click();
  await multi.control.locator('.select-kit-row[data-value="go"]').click();
  await expect(multi.control.locator(".formatted-selection")).toHaveText("go");
  await multi.control.locator('.selected-choice[data-value="go"]').click();
  await expect(multi.control.locator(".formatted-selection")).toHaveText("(select an option)");
  await expect(multi.component).not.toHaveClass(/value-entered/);
  await multi.control.locator(".filter-input").press("Escape");
  await page.locator(".invitation-cta__accept").click();
  await accepted(page, sent);
  expect(pairs(sent[0])).toEqual([
    [`user_custom_fields[${TEXT}]`, ""],
    [`user_custom_fields[${CONFIRM}]`, "false"],
  ]);
});

test("a required field left empty, or a text field repeating the password, keeps the invite's button disabled", async ({
  page,
}) => {
  const sent = await openInvite(page, true);
  const submit = page.locator(".invitation-cta__accept");
  // From the start, with no tip: the fields are passed no validation.
  await expect(submit).toBeDisabled();
  await account(page);
  await expect(submit).toBeDisabled();
  await expect(page.locator(".user-fields .tip.bad")).toHaveCount(0);
  await expect(invited(page, CONFIRM).component.locator(".tip")).toHaveClass("tip good hidden");
  await expect(invited(page, TEXT).component.locator(".instructions")).toHaveText("Any color");

  await invited(page, TEXT).control.fill("Bright red");
  await expect(submit).toBeDisabled();
  await invited(page, CONFIRM).control.check();
  await expect(submit).toBeDisabled();
  const multi = invited(page, MULTI);
  await multi.control.locator("summary").click();
  await multi.control.locator('.select-kit-row[data-value="rust"]').click();
  await expect(multi.control.locator(".formatted-selection")).toHaveText("rust");
  await expect(submit).toBeEnabled();
  await multi.control.locator(".filter-input").press("Escape");

  // Each one emptied again disables it.
  await invited(page, CONFIRM).control.uncheck();
  await expect(submit).toBeDisabled();
  await invited(page, CONFIRM).control.check();
  await expect(submit).toBeEnabled();
  await invited(page, TEXT).control.fill("");
  await expect(submit).toBeDisabled();

  // A text field holding the password, whichever of the two changes.
  await invited(page, TEXT).control.fill(`my ${PASSWORD.toUpperCase()}!`);
  await expect(submit).toBeDisabled();
  await page.locator("#new-account-password").fill("another-pass-phrase-entirely");
  await expect(submit).toBeEnabled();
  await page.locator("#new-account-password").fill(PASSWORD);
  await expect(submit).toBeDisabled();
  await invited(page, TEXT).control.fill("Bright red");
  await expect(submit).toBeEnabled();
  await expect(page.locator(".user-fields .tip.bad")).toHaveCount(0);
  expect(sent.length).toBe(0);

  await submit.click();
  await accepted(page, sent);
  expect(pairs(sent[0])).toEqual([
    [`user_custom_fields[${TEXT}]`, "Bright red"],
    [`user_custom_fields[${CONFIRM}]`, "true"],
    [`user_custom_fields[${MULTI}][]`, "rust"],
  ]);
});
