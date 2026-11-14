// The signup form's user fields (login.js over templates/signup.gjs's
// UserField list and UserFieldsValidationHelper): each type's control,
// value-entered, the tips a submit shows and how they follow the values,
// and the user_fields a signup sends and the server stores.
//
// The suite's site signs up by emailed code, whose page shows the
// create-account form only for an external login's data, so each test
// leaves that data (empty) in the session's storage as the login would.
//
// user_fields rows are the whole site's. The spec's own are optional, off
// the profile and not editable, so no other spec's signup or preferences
// meets them, and are removed at the end. A required field would refuse
// every other spec's signup: the required paths run on these rows with
// `required` set in the page's data only (the server's refusals are
// tests/signup_user_fields.rs). The multiselect is not a row at all (a
// staff member's profile preferences refuse one): its markup is put in the
// page as the server draws it, and the test reads what the form sends.
const { test, expect, sql, label } = require("./helpers");

const PASSWORD = "a-long-enough-pass-phrase";
const MULTI = 987654;

// A name no other run or test takes.
function unique(what) {
  return label(what).replace(/[^a-z0-9]/g, "");
}

let fields;

test.beforeAll(() => {
  const tag = unique("suf");
  const add = (what, type, position) =>
    Number(
      sql(
        "INSERT INTO user_fields (name, description, field_type_enum, editable, requirement, show_on_profile, \
           show_on_user_card, show_on_signup, searchable, position, created_at, updated_at) \
         VALUES (:'p1', :'p2', :'p3', false, 0, false, false, true, false, :'p4', now(), now()) RETURNING id",
        `${what} ${tag}`,
        `About <b>${what}</b>`,
        type,
        position,
      )[0][0],
    );
  fields = {
    tag,
    text: add("Color", 0, 9001),
    confirm: add("Rules", 1, 9002),
    dropdown: add("Pet", 2, 9003),
    textarea: add("Story", 4, 9004),
    date: add("Birthday", 5, 9005),
  };
  for (const option of ["cat", "dog"]) {
    sql(
      "INSERT INTO user_field_options (user_field_id, value, created_at, updated_at) VALUES (:'p1', :'p2', now(), now())",
      fields.dropdown,
      option,
    );
  }
});

test.afterAll(() => {
  if (!fields) {
    return;
  }
  const ids = [fields.text, fields.confirm, fields.dropdown, fields.textarea, fields.date];
  for (const id of ids) {
    sql("DELETE FROM user_custom_fields WHERE name = :'p1'", `user_field_${id}`);
    sql("DELETE FROM user_field_options WHERE user_field_id = :'p1'", id);
    sql("DELETE FROM user_fields WHERE id = :'p1'", id);
  }
});

// The page's #login-data rewritten by `change`.
function withData(html, change) {
  return html.replace(/(<script type="application\/json" id="login-data">)(.*?)(<\/script>)/s, (_, open, json, close) => {
    const data = JSON.parse(json);
    change(data);
    return open + JSON.stringify(data).replace(/</g, "\\u003c") + close;
  });
}

// The spec's fields required, as far as the page knows.
function required(html) {
  const ids = [fields.text, fields.confirm, fields.dropdown, fields.textarea, fields.date];
  return withData(html, (data) => {
    data.user_fields.forEach((f) => {
      if (ids.includes(f.id)) {
        f.required = true;
      }
    });
  });
}

// A multiselect field as the server draws one, after the site's fields.
function withMultiselect(isRequired) {
  return (html) => {
    const field = `<div class="input-group"><div class="user-field multiselect user-field-langs"><label class="control-label alt-placeholder" for="user-field-${MULTI}-input">Langs${isRequired ? "" : "(optional)"}</label><div class="controls"><details id="user-field-${MULTI}-input" data-user-field="${MULTI}" data-options="[&quot;rust&quot;,&quot;ruby&quot;,&quot;go&quot;]" class="select-kit multi-select" data-chooser="options" data-items="[]"><summary data-name="" data-value="" tabindex="0" class="select-kit-header multi-select-header"><div class="select-kit-header-wrapper"><span class="formatted-selection">(select an option)</span></div></summary><div class="select-kit-body"></div></details><div class="instructions">Languages</div></div></div></div>`;
    const strings = {
      default_header: "(select an option)",
      filter_placeholder: "Search…",
      results_one: "%{count} result",
      results_other: "%{count} results",
      delete_item: "Delete %{name}",
    };
    const marker = '<div class="user-fields">';
    const at = html.lastIndexOf(marker);
    const end = html.indexOf("</form>", at);
    // The list's own closing tag is the last before the form's (no
    // captcha connector on the suite's site).
    const close = html.lastIndexOf("</div>", end);
    html = html.slice(0, close) + field + html.slice(close);
    html = html.replace(
      '<script type="application/json" id="login-data">',
      `<script type="application/json" id="prefs-choosers-strings">${JSON.stringify(strings)}</script><script src="/assets/prefs-choosers.js"></script><script type="application/json" id="login-data">`,
    );
    return withData(html, (data) => {
      data.user_fields.push({ id: MULTI, name: "Langs", field_type: "multiselect", required: isRequired });
    });
  };
}

// Opens the create-account form with the spec's fields, the page's HTML
// passed through `patch` first.
async function openForm(page, patch) {
  if (patch) {
    await page.route(
      (url) => url.pathname === "/signup",
      async (route) => {
        const response = await route.fetch();
        await route.fulfill({ response, body: patch(await response.text()) });
      },
    );
  }
  await page.goto("/signup");
  await page.evaluate(() => sessionStorage.setItem("discourse-authentication-data", "{}"));
  // The form asks for the honeypot as it is drawn.
  const hp = page.waitForResponse((r) => new URL(r.url()).pathname === "/session/hp.json");
  await page.reload();
  await hp;
  await expect(page.locator("form#login-form .user-fields")).toBeVisible();
}

// Fills the account's own inputs, the username checked. Returns the name.
async function account(page) {
  const name = unique("su");
  await page.locator("#new-account-email").fill(`${name}@example.com`);
  const checked = page.waitForResponse((r) => {
    const url = new URL(r.url());
    return url.pathname === "/u/check_username.json" && url.searchParams.get("username") === name;
  });
  await page.locator("#new-account-username").fill(name);
  await checked;
  await expect(page.locator("#username-validation")).toHaveText("Your username is available");
  const fullName = page.locator("#new-account-name");
  if (await fullName.count()) {
    await fullName.fill("Signup Fields");
  }
  await page.locator("#new-account-password").fill(PASSWORD);
  await expect(page.locator("#password-validation")).toHaveText("Your password looks good.");
  return name;
}

function component(page, id) {
  return page.locator(`.user-fields .user-field:has([data-user-field="${id}"])`);
}

function control(page, id) {
  return page.locator(`.user-fields [data-user-field="${id}"]`);
}

// The POST /u the form sends, as its decoded pairs.
function creation(page) {
  return page.waitForRequest((r) => r.method() === "POST" && new URL(r.url()).pathname === "/u");
}

function pairs(request) {
  return [...new URLSearchParams(request.postData())].filter(([key]) => key.startsWith("user_fields"));
}

// The account's user field values as stored, a newline as a bar.
function storedFields(name) {
  return sql(
    "SELECT f.name, replace(f.value, chr(10), '|') FROM user_custom_fields f JOIN users u ON u.id = f.user_id \
     WHERE u.username = :'p1' AND f.name LIKE 'user\\_field\\_%' ORDER BY f.name, f.id",
    name,
  );
}

async function created(page, name) {
  await expect(page).toHaveURL(/\/u\/account-created/);
  await expect(page.locator(".account-created")).toBeVisible();
  expect(sql("SELECT COUNT(*) FROM users WHERE username = :'p1'", name)[0][0]).toBe("1");
}

test("each field's control takes a value, which the signup stores", async ({ page }) => {
  await openForm(page);
  const name = await account(page);

  // In position order, the site's other fields (none) before them.
  const drawn = await page
    .locator(".user-fields [data-user-field]")
    .evaluateAll((els) => els.map((el) => Number(el.dataset.userField)));
  expect(drawn).toEqual([fields.text, fields.confirm, fields.dropdown, fields.textarea, fields.date]);

  // A text field: its label, its description as HTML, value-entered.
  const text = component(page, fields.text);
  await expect(text).toHaveClass(new RegExp(`^user-field text user-field-color-${fields.tag}$`));
  await expect(text.locator("label.alt-placeholder")).toHaveText(`Color ${fields.tag}(optional)`);
  await expect(text.locator(".instructions b")).toHaveText("Color");
  await control(page, fields.text).fill("Bright red");
  await expect(text).toHaveClass(/value-entered/);
  await control(page, fields.text).fill("");
  await expect(text).not.toHaveClass(/value-entered/);
  await control(page, fields.text).fill("Bright red");

  // A confirm: a checkbox, its tip hidden.
  const confirm = component(page, fields.confirm);
  await expect(confirm.locator(".tip")).toHaveClass("tip good hidden");
  await control(page, fields.confirm).check();

  // A dropdown: the ComboBox opens under its header and a row selects.
  const dropdown = control(page, fields.dropdown);
  await expect(dropdown.locator(".select-kit-selected-name")).toHaveText("(select an option)");
  await dropdown.locator("summary").click();
  await expect(dropdown).toHaveClass(/is-expanded/);
  await expect(dropdown.locator(".select-kit-row")).toHaveText(["(select an option)", "cat", "dog"]);
  await dropdown.locator('.select-kit-row[data-value="dog"]').click();
  await expect(dropdown).not.toHaveAttribute("open", "");
  await expect(dropdown).toHaveClass(/has-selection/);
  await expect(dropdown).not.toHaveClass(/is-expanded/);
  await expect(dropdown.locator(".select-kit-selected-name")).toHaveText("dog");
  await expect(component(page, fields.dropdown)).toHaveClass(/value-entered/);
  // A click elsewhere closes it.
  await dropdown.locator("summary").click();
  await expect(dropdown.locator(".select-kit-body")).toBeVisible();
  await page.locator(".login-title").click();
  await expect(dropdown.locator(".select-kit-body")).toBeHidden();

  await control(page, fields.textarea).fill("Once\nupon a time");
  await expect(component(page, fields.textarea)).toHaveClass(/value-entered/);
  await control(page, fields.date).fill("2001-02-03");

  const sent = creation(page);
  await page.locator(".signup-page-cta__signup").click();
  expect(pairs(await sent)).toEqual([
    [`user_fields[${fields.text}]`, "Bright red"],
    [`user_fields[${fields.confirm}]`, "true"],
    [`user_fields[${fields.dropdown}]`, "dog"],
    [`user_fields[${fields.textarea}]`, "Once\nupon a time"],
    [`user_fields[${fields.date}]`, "2001-02-03"],
  ]);
  await created(page, name);
  expect(storedFields(name).sort()).toEqual(
    [
      [`user_field_${fields.text}`, "Bright red"],
      [`user_field_${fields.confirm}`, "true"],
      [`user_field_${fields.dropdown}`, "dog"],
      [`user_field_${fields.textarea}`, "Once|upon a time"],
      [`user_field_${fields.date}`, "2001-02-03"],
    ].sort(),
  );
});

test("optional fields left alone go empty and store nothing", async ({ page }) => {
  await openForm(page);
  const name = await account(page);
  // The dropdown back on its none row.
  const dropdown = control(page, fields.dropdown);
  await dropdown.locator("summary").click();
  await dropdown.locator('.select-kit-row[data-value="cat"]').click();
  await expect(dropdown.locator(".select-kit-selected-name")).toHaveText("cat");
  await dropdown.locator("summary").click();
  await dropdown.locator(".select-kit-row.none").click();
  await expect(dropdown).not.toHaveClass(/has-selection/);
  await expect(component(page, fields.dropdown)).not.toHaveClass(/value-entered/);

  const sent = creation(page);
  await page.locator(".signup-page-cta__signup").click();
  expect(pairs(await sent)).toEqual([
    [`user_fields[${fields.text}]`, ""],
    [`user_fields[${fields.confirm}]`, "false"],
    [`user_fields[${fields.dropdown}]`, ""],
    [`user_fields[${fields.textarea}]`, ""],
    [`user_fields[${fields.date}]`, ""],
  ]);
  await created(page, name);
  expect(storedFields(name)).toEqual([]);
});

test("a submit shows the required fields' tips, which follow the values", async ({ page }) => {
  await openForm(page, required);
  const name = await account(page);
  let posts = 0;
  page.on("request", (r) => {
    if (r.method() === "POST" && new URL(r.url()).pathname === "/u") {
      posts += 1;
    }
  });
  const submit = page.locator(".signup-page-cta__signup");
  const tip = (id) => component(page, id).locator(".tip");
  const title = (what) => `${what} ${fields.tag}`;

  // Nothing shows before a submit.
  await expect(page.locator(".user-fields .tip.bad")).toHaveCount(0);
  await submit.click();
  await expect(tip(fields.text)).toHaveText(`Please enter a value for "${title("Color")}"`);
  await expect(tip(fields.text)).toHaveClass("tip bad");
  await expect(tip(fields.text).locator("svg.d-icon-xmark")).toHaveCount(1);
  await expect(component(page, fields.text).locator(".instructions")).toHaveCount(0);
  await expect(tip(fields.confirm)).toHaveText(`The field "${title("Rules")}" is required`);
  await expect(tip(fields.confirm)).toHaveClass("tip bad");
  await expect(tip(fields.dropdown)).toHaveText(`Please select a value for "${title("Pet")}"`);
  await expect(tip(fields.textarea)).toHaveText(`Please select a value for "${title("Story")}"`);
  await expect(tip(fields.date)).toHaveText(`Please enter a value for "${title("Birthday")}"`);
  // The first that fails takes the focus.
  await expect(control(page, fields.text)).toBeFocused();

  // The tips follow the values from here on.
  await control(page, fields.text).fill("Bright red");
  await expect(tip(fields.text)).toHaveCount(0);
  await expect(component(page, fields.text).locator(".instructions")).toHaveText("About Color");
  await control(page, fields.text).fill("");
  await expect(tip(fields.text)).toHaveText(`Please enter a value for "${title("Color")}"`);
  // A text field repeating the password.
  await control(page, fields.text).fill(`my ${PASSWORD.toUpperCase()}!`);
  await expect(tip(fields.text)).toHaveText("Your password should not be repeated in other fields");
  await control(page, fields.text).fill("Bright red");
  await expect(tip(fields.text)).toHaveCount(0);

  await submit.click();
  await expect(control(page, fields.confirm)).toBeFocused();
  await control(page, fields.confirm).check();
  await expect(tip(fields.confirm)).toHaveClass("tip good hidden");
  await expect(tip(fields.confirm)).toHaveText("");

  await submit.click();
  const dropdown = control(page, fields.dropdown);
  await expect(dropdown.locator("summary")).toBeFocused();
  await dropdown.locator("summary").click();
  await dropdown.locator('.select-kit-row[data-value="cat"]').click();
  await expect(tip(fields.dropdown)).toHaveCount(0);
  await expect(component(page, fields.dropdown).locator(".instructions")).toHaveText("About Pet");

  await control(page, fields.textarea).fill("A story");
  await expect(tip(fields.textarea)).toHaveCount(0);
  await submit.click();
  await expect(control(page, fields.date)).toBeFocused();
  expect(posts).toBe(0);
  await control(page, fields.date).fill("1999-12-31");
  await expect(tip(fields.date)).toHaveCount(0);

  const sent = creation(page);
  await submit.click();
  await sent;
  await created(page, name);
  expect(posts).toBe(1);
  expect(storedFields(name).sort()).toEqual(
    [
      [`user_field_${fields.text}`, "Bright red"],
      [`user_field_${fields.confirm}`, "true"],
      [`user_field_${fields.dropdown}`, "cat"],
      [`user_field_${fields.textarea}`, "A story"],
      [`user_field_${fields.date}`, "1999-12-31"],
    ].sort(),
  );
});

test("a multiselect chooses among its options and sends them as a list", async ({ page }) => {
  await openForm(page, withMultiselect(true));
  const name = await account(page);
  const chooser = control(page, MULTI);
  const field = component(page, MULTI);
  const submit = page.locator(".signup-page-cta__signup");

  // Required and empty: its tip, and no request.
  await submit.click();
  await expect(field.locator(".tip")).toHaveText('Please select a value for "Langs"');
  await expect(field.locator(".instructions")).toHaveCount(0);

  await chooser.locator("summary").click();
  await expect(chooser).toHaveClass(/is-expanded/);
  await expect(chooser.locator(".select-kit-collection .select-kit-row")).toHaveText(["rust", "ruby", "go"]);
  await chooser.locator(".filter-input").fill("ru");
  await expect(chooser.locator(".select-kit-collection .select-kit-row")).toHaveText(["rust", "ruby"]);
  await chooser.locator('.select-kit-row[data-value="ruby"]').click();
  await expect(chooser.locator(".formatted-selection")).toHaveText("ruby");
  await expect(field).toHaveClass(/value-entered/);
  await expect(field.locator(".tip")).toHaveCount(0);
  await expect(field.locator(".instructions")).toHaveText("Languages");
  // The chosen option leaves the rows.
  await expect(chooser.locator(".select-kit-collection .select-kit-row")).toHaveText(["rust", "go"]);
  await chooser.locator('.select-kit-row[data-value="go"]').click();
  await chooser.locator('.select-kit-row[data-value="rust"]').click();
  await expect(chooser.locator(".formatted-selection")).toHaveText("ruby, go, rust");
  await chooser.locator('.selected-choice[data-value="go"]').click();
  await expect(chooser.locator(".formatted-selection")).toHaveText("ruby, rust");
  await page.locator(".login-title").click();
  await expect(chooser).not.toHaveAttribute("open", "");

  const sent = creation(page);
  await submit.click();
  const multi = pairs(await sent).filter(([key]) => key.includes(String(MULTI)));
  expect(multi).toEqual([
    [`user_fields[${MULTI}][]`, "ruby"],
    [`user_fields[${MULTI}][]`, "rust"],
  ]);
  // No such field on the server, which reads its own: the account is made.
  await created(page, name);
});

test("an optional multiselect emptied again sends nothing", async ({ page }) => {
  await openForm(page, withMultiselect(false));
  const name = await account(page);
  const chooser = control(page, MULTI);
  await chooser.locator("summary").click();
  await chooser.locator('.select-kit-row[data-value="rust"]').click();
  await expect(chooser.locator(".formatted-selection")).toHaveText("rust");
  await chooser.locator('.selected-choice[data-value="rust"]').click();
  await expect(chooser.locator(".formatted-selection")).toHaveText("(select an option)");
  await expect(component(page, MULTI)).not.toHaveClass(/value-entered/);
  // Enter in its filter chooses a row; it does not submit the form.
  await chooser.locator(".filter-input").fill("go");
  await chooser.locator(".filter-input").press("Enter");
  await expect(chooser.locator(".formatted-selection")).toHaveText("go");
  await chooser.locator('.selected-choice[data-value="go"]').click();
  await expect(chooser.locator(".formatted-selection")).toHaveText("(select an option)");
  await page.locator(".login-title").click();

  const sent = creation(page);
  await page.locator(".signup-page-cta__signup").click();
  // jQuery.param sends no pair for an empty list.
  expect(pairs(await sent).filter(([key]) => key.includes(String(MULTI)))).toEqual([]);
  await created(page, name);
});
