// discourse-poll's widget actions in poll.js: PollButtonsDropdown's close
// and open, asked first (toggleStatus, an on_close poll's results showing
// once closed, with the votes Ember refreshes the route for), and its
// breakdown (PollBreakdownModal: the Percentage and Count tabs, the
// options with their swatches and highlight, the group chooser over
// /polls/grouped_poll_results and a pie per group, drawn with Chart.js and
// chartjs-plugin-datalabels). Each test works on a topic and members of
// its own; the breakdown test turns poll_groupable_user_fields on for its
// run, which only adds the breakdown to other specs' poll menus.
const { test, expect, login, api, topic, sql, label, member } = require("./helpers");

async function vote(context, postId, name, digest) {
  await api(context, "PUT", "/polls/vote", { post_id: String(postId), poll_name: name, "options[]": digest });
}

function digests(postId, name) {
  return sql(
    "SELECT po.html, po.digest FROM poll_options po JOIN polls p ON p.id = po.poll_id \
     WHERE p.post_id = :'p1' AND p.name = :'p2' ORDER BY po.id",
    postId,
    name,
  ).reduce((all, [html, digest]) => Object.assign(all, { [html]: digest }), {});
}

function status(postId) {
  return sql("SELECT status, closed_by_id IS NOT NULL FROM polls WHERE post_id = :'p1'", postId);
}

// A poll author of the test's own, also in trust_level_4: composer-actions
// narrows poll_create_allowed_groups to 14 for one test, and a member only
// in the default 11 would be refused a poll while that test runs.
function pollAuthor() {
  const author = member();
  sql(
    "INSERT INTO group_users (group_id, user_id, notification_level, created_at, updated_at) \
     VALUES (14, :'p1', 2, now(), now()) ON CONFLICT DO NOTHING",
    author.id,
  );
  return author;
}

test("the author closes and opens a poll once confirmed", async ({ page, context, session }) => {
  await login(context, pollAuthor().name);
  const t = await topic(context, label("A poll to close"), "Results when it closes.\n\n[poll results=on_close]\n* Now\n* Later\n[/poll]");
  const voter = await session("user2");
  await vote(voter, t.posts[0], "poll", digests(t.posts[0], "poll").Later);

  await page.goto(t.url);
  const widget = page.locator(".poll-outer[data-poll-name='poll']");
  await expect(widget.locator(".results-on-close")).toBeVisible();
  // The dropdown's one action, drawn as its button.
  const toggle = widget.locator(".poll-buttons-dropdown .toggle-status");
  await expect(toggle).toHaveText("Close");

  // No keeps it open.
  await toggle.click();
  const dialog = page.locator("#dialog-holder");
  await expect(dialog.locator(".dialog-body")).toHaveText("Are you sure you want to close this poll?");
  await expect(dialog.locator(".dialog-footer .btn")).toHaveText(["Yes", "No"]);
  await dialog.locator(".dialog-footer .btn-default").click();
  await expect(dialog).toHaveCount(0);
  expect(status(t.posts[0])).toEqual([["0", "f"]]);

  // Yes closes it: the results show, with the votes the close reveals.
  await toggle.click();
  await dialog.locator(".dialog-footer .btn-primary").click();
  await expect(widget.locator(".discourse-poll-regular-results")).toBeVisible();
  await expect(widget.locator(".results li").first()).toContainText("100%");
  await expect(widget.locator(".results li").first()).toContainText("Later");
  await expect(widget.locator(".poll-info_closed-by")).toBeVisible();
  expect(status(t.posts[0])).toEqual([["1", "t"]]);
  // With the tally toggle beside it, Open is behind the gear.
  await widget.locator(".poll-buttons-dropdown .widget-dropdown-header").click();
  const menu = page.locator(".fk-d-menu .dropdown-menu");
  await expect(menu.locator("[data-poll-action]")).toHaveText(["Display tally", "Open"]);

  // Opened again: the results wait for the close once more.
  await menu.locator(".toggle-status").click();
  await expect(menu.locator(".toggle-status")).toHaveCount(0);
  await expect(dialog.locator(".dialog-body")).toHaveText("Are you sure you want to open this poll?");
  await dialog.locator(".dialog-footer .btn-primary").click();
  await expect(widget.locator(".results-on-close")).toBeVisible();
  await expect(widget.locator(".discourse-poll-regular-results")).toHaveCount(0);
  await expect(widget.locator(".poll-buttons-dropdown .toggle-status")).toHaveText("Close");
  expect(status(t.posts[0])).toEqual([["0", "f"]]);
});

test("the breakdown modal shows each group's pie by the field chosen", async ({ page, context, session }) => {
  const admin = await session("admin");
  await api(admin, "PUT", "/admin/site_settings/poll_groupable_user_fields", {
    poll_groupable_user_fields: "city|favorite_team",
  });
  try {
    await login(context, pollAuthor().name);
    const t = await topic(context, label("Pets by city"), "Which pet?\n\n[poll]\n# Pets\n* Cats\n* Dogs\n* Fish\n[/poll]");
    const options = digests(t.posts[0], "poll");
    const voters = [member(), member(), member()];
    sql(
      "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) \
       VALUES (:'p1', 'city', 'new york', now(), now()), (:'p2', 'city', 'new york', now(), now()), \
              (:'p3', 'city', 'boston', now(), now())",
      voters[0].id,
      voters[1].id,
      voters[2].id,
    );
    for (const [m, pick] of [[voters[0], "Cats"], [voters[1], "Dogs"], [voters[2], "Dogs"]]) {
      await vote(await session(m.name), t.posts[0], "poll", options[pick]);
    }
    // The author's own vote: a member without the field, "No Data".
    await vote(context, t.posts[0], "poll", options.Dogs);

    await page.goto(t.url);
    const widget = page.locator(".poll-outer[data-poll-name='poll']");
    await widget.locator(".poll-buttons-dropdown .widget-dropdown-header").click();
    await page.locator(".fk-d-menu .show-breakdown").click();

    const modal = page.locator(".d-modal.poll-breakdown.has-tabs");
    await expect(modal.locator(".d-modal__title-text")).toHaveText("Poll results");
    const tabs = modal.locator(".d-modal__header .modal-tabs .modal-tab");
    await expect(tabs).toHaveText(["Percentage", "Count"]);
    await expect(modal.locator(".modal-tab.percentage")).toHaveClass("modal-tab percentage is-active");
    await expect(modal.locator(".poll-breakdown-title")).toHaveText("Pets");
    await expect(modal.locator(".poll-breakdown-total-votes")).toHaveText("4 votes");
    const rows = modal.locator(".poll-breakdown-options .poll-breakdown-option");
    await expect(rows.locator(".poll-breakdown-option-text")).toHaveText(["Cats", "Dogs", "Fish"]);
    await expect(rows.locator(".poll-breakdown-option-count")).toHaveText(["25.0%", "75.0%", "0.0%"]);
    await expect(rows.first().locator(".poll-breakdown-option-color")).toHaveAttribute("style", /^background: \S.*;$/);
    await expect(modal.locator(".poll-breakdown-body-header-label")).toHaveText("Breakdown");
    await expect(modal.locator(".poll-breakdown-dropdown .select-kit-header .name")).toHaveText("city");

    // A pie per group, over the options with votes.
    const charts = modal.locator(".poll-breakdown-charts .poll-breakdown-chart-container");
    await expect(charts).toHaveCount(3);
    const labels = await charts.locator(".poll-breakdown-chart-label").allTextContents();
    expect(labels.sort()).toEqual(["Boston", "New York", "No Data"]);
    const pie = (group) =>
      modal
        .locator(".poll-breakdown-chart-container", { has: page.locator(".poll-breakdown-chart-label", { hasText: group }) })
        .locator("canvas.poll-breakdown-chart-chart");
    const chartOf = (group, fn) => pie(group).evaluate(fn);
    await expect.poll(() => chartOf("New York", (c) => !!window.Chart.getChart(c))).toBe(true);
    expect(
      await chartOf("New York", (c) => {
        const chart = window.Chart.getChart(c);
        return {
          type: chart.config.type,
          data: chart.data.datasets[0].data,
          colors: chart.data.datasets[0].backgroundColor.length,
          aspectRatio: chart.options.aspectRatio,
          datalabels: chart.config.plugins.some((p) => p.id === "datalabels"),
          // The formatter as configured (chart.options resolves it).
          label: chart.config._config.options.plugins.datalabels.formatter(1),
        };
      })
    ).toEqual({ type: "pie", data: [1, 1], colors: 2, aspectRatio: 1.1, datalabels: true, label: "50.0%" });
    expect(await chartOf("No Data", (c) => window.Chart.getChart(c).data.datasets[0].data)).toEqual([1]);

    // Count: the votes, in the sidebar and on the slices.
    await modal.locator(".modal-tab.count").click();
    await expect(modal.locator(".modal-tab.count")).toHaveClass("modal-tab count is-active");
    await expect(modal.locator(".modal-tab.percentage")).toHaveClass("modal-tab percentage");
    await expect(rows.locator(".poll-breakdown-option-count")).toHaveText(["1", "3", "0"]);
    expect(await chartOf("New York", (c) => window.Chart.getChart(c).config._config.options.plugins.datalabels.formatter(1))).toBe(1);

    // Hovering an option highlights its row only: Ember's render throws
    // at the swatch's hover color (chart.js/auto has no Chart.helpers),
    // so the swatch and the pies stay as they were.
    const swatch = await rows.nth(1).locator(".poll-breakdown-option-color").getAttribute("style");
    await rows.nth(1).hover();
    await expect(rows.nth(1)).toHaveAttribute("style", "background: rgba(0, 0, 0, 0.1);");
    await expect(rows.nth(0)).not.toHaveAttribute("style");
    await expect(rows.nth(1).locator(".poll-breakdown-option-color")).toHaveAttribute("style", swatch);
    expect(await chartOf("New York", (c) => window.Chart.getChart(c).getActiveElements().length)).toBe(0);
    await modal.locator(".poll-breakdown-body-header-label").hover();
    await expect(rows.nth(1)).not.toHaveAttribute("style");
    // A slice under the pointer highlights its option's row.
    const box = await pie("Boston").boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height * 0.75);
    await expect(rows.nth(1)).toHaveAttribute("style", "background: rgba(0, 0, 0, 0.1);");
    // Off the slice but on the chart: Chart.js' onHover with nothing
    // active (leaving the canvas calls no onHover).
    await page.mouse.move(box.x + 2, box.y + box.height / 2);
    await expect(rows.nth(1)).not.toHaveAttribute("style");

    // Another vote comes in: the sidebar follows the poll.
    const late = member();
    await vote(await session(late.name), t.posts[0], "poll", options.Fish);
    await expect(modal.locator(".poll-breakdown-total-votes")).toHaveText("5 votes");
    await expect(rows.locator(".poll-breakdown-option-count")).toHaveText(["1", "3", "1"]);

    // Grouped by another field: fetched again, no one has it.
    const request = page.waitForRequest((r) => r.url().includes("/polls/grouped_poll_results.json") && r.url().includes("user_field_name=favorite_team"));
    await modal.locator(".poll-breakdown-dropdown .select-kit-header").click();
    await expect(modal.locator(".poll-breakdown-dropdown .select-kit-row")).toHaveText(["city", "Favorite team"]);
    await modal.locator(".poll-breakdown-dropdown .select-kit-row[data-value='favorite_team']").click();
    await request;
    await expect(modal.locator(".poll-breakdown-dropdown .select-kit-header .name")).toHaveText("Favorite team");
    await expect(charts).toHaveCount(1);
    await expect(charts.locator(".poll-breakdown-chart-label")).toHaveText("No Data");
    await expect.poll(() => chartOf("No Data", (c) => window.Chart.getChart(c) && window.Chart.getChart(c).data.datasets[0].data)).toEqual([1, 3, 1]);

    await page.keyboard.press("Escape");
    await expect(modal).toHaveCount(0);
  } finally {
    await api(admin, "PUT", "/admin/site_settings/poll_groupable_user_fields", { poll_groupable_user_fields: "" });
  }
});
