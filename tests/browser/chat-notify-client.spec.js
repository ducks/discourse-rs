// Chat's client notifications (static/js/chat-drawer.js: the chat-audio
// initializer over ChatAudioManager and chat-alert-dedup,
// ChatNotificationManager, chat-setup's addDocumentTitleCounter): a
// chat alert (/chat/notification-alert, the stream's
// chat-notification-alert event) plays the member's chat_sound unless
// the header indicator preference leaves it out, is shown as a desktop
// notification unless the member looks at its channel, and the urgent
// count (mentions, a DM's unread messages) joins the tab title's count
// in the notifications mode. None of it in do not disturb.
//
// The sounds are synthesized (Web Audio, no files): window.AudioContext
// is a stub recording each oscillator's type and frequency, a sound
// being the frequencies of one play. window.Notification is a stub too,
// as in desktop-notifications.spec.js. Each test works on a member of
// its own and a DM or a channel of its own; a new member is notified of
// more than the test's messages (a badge granted), so the title is
// compared with what the page carries.
const { test, expect, login, api, sql, member, label } = require("./helpers");

const STORE = "discourse_desktop_notifications_notifications-disabled";
const ALERTS = "discourse_chat_alerts_handled";

// ChatAudioManager's sequences, as the stub records them.
const SOFT_INCOMING = "sine:349.23,sine:466.16";
const SOFT_MENTION = "sine:349.23,sine:440,sine:523.25";

async function stub(context) {
  await context.grantPermissions(["notifications"]);
  await context.addInitScript(
    ([STORE]) => {
      localStorage.setItem(STORE, "enabled");
      window.__shown = [];
      class Stub extends EventTarget {
        constructor(title, options) {
          super();
          this.title = title;
          this.options = options || {};
          window.__shown.push(this);
        }
        close() {}
        static get permission() {
          return "granted";
        }
        static requestPermission(callback) {
          if (callback) {
            callback("granted");
          }
          return Promise.resolve("granted");
        }
      }
      window.Notification = Stub;

      // One entry per play: the oscillators started in the same task.
      window.__sounds = [];
      let playing = null;
      const param = () => ({
        value: 0,
        setValueAtTime(value) {
          this.value = value;
        },
        linearRampToValueAtTime() {},
        exponentialRampToValueAtTime() {},
      });
      class Context {
        constructor() {
          this.state = "running";
          this.currentTime = 0;
          this.destination = {};
        }
        resume() {
          return Promise.resolve();
        }
        createGain() {
          return { gain: param(), connect: (to) => to };
        }
        createOscillator() {
          const oscillator = {
            type: "sine",
            frequency: param(),
            connect: (to) => to,
            start() {
              if (!playing) {
                playing = [];
                window.__sounds.push(playing);
                setTimeout(() => (playing = null), 0);
              }
              playing.push(`${oscillator.type}:${oscillator.frequency.value}`);
            },
            stop() {},
          };
          return oscillator;
        }
      }
      window.AudioContext = Context;
    },
    [STORE],
  );
}

async function setHidden(page, hidden) {
  await page.evaluate((hidden) => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => (hidden ? "hidden" : "visible") });
    Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });
    document.dispatchEvent(new Event("visibilitychange"));
  }, hidden);
}

function options(m, values) {
  sql(`UPDATE user_options SET ${values} WHERE user_id = :'p1'`, m.id);
}

// A DM between user0 and the member, and user0's messages in it.
async function directMessages(session, m) {
  const user0 = await session("user0");
  const dm = await api(user0, "POST", "/chat/api/direct-message-channels.json", { "target_usernames[]": m.name });
  const id = dm.channel.id;
  return {
    id,
    send: async (what) => {
      const message = label(what);
      await api(user0, "POST", `/chat/${id}.json`, { message, staged_id: label("staged") });
      return message;
    },
  };
}

function sounds(page) {
  return page.evaluate(() => window.__sounds.map((s) => s.join(",")));
}

// The desktop notification whose body holds the text, once it is made:
// the alert's handlers ran before it.
async function shownWith(page, text) {
  let found;
  await expect
    .poll(async () => {
      found = (await page.evaluate(() => window.__shown.map((n) => ({ title: n.title, body: n.options.body, tag: n.options.tag })))).find((n) =>
        (n.body || "").includes(text),
      );
      return !!found;
    })
    .toBe(true);
  return found;
}

// The title counting the notifications and chat's urgent count the page
// carries, the urgent count at least `min`.
async function counted(page, plain, min) {
  await expect
    .poll(() =>
      page.evaluate(
        ([plain, min]) => {
          const unread = Number(document.querySelector("#toggle-current-user img.avatar").dataset.allUnread);
          const urgent = Number(document.getElementById("chat-tracking-state").dataset.urgentCount);
          return urgent >= min && document.title === `(${unread + urgent}) ${plain}`;
        },
        [plain, min],
      ),
    )
    .toBe(true);
}

// The message's alert, once NotifyWatching has published it (the job
// runs after the request, whatever the member's do not disturb).
async function alerted(m, message) {
  await expect
    .poll(
      () =>
        sql(
          "SELECT count(*) FROM discourse_rs_bus.messages WHERE channel = '/chat/notification-alert/' || :'p1' AND position(:'p2' in data->>'excerpt') > 0",
          m.id,
          message,
        )[0][0],
    )
    .toBe("1");
}

async function urgent(page, min) {
  await expect.poll(() => page.evaluate(() => Number(document.getElementById("chat-tracking-state").dataset.urgentCount))).toBeGreaterThanOrEqual(min);
}

test("a DM in a hidden tab counts in the title, plays the member's sound and is shown", async ({ page, context, session }) => {
  const m = member();
  options(m, "chat_sound = 'soft', title_count_mode_key = 0");
  const dm = await directMessages(session, m);
  await stub(context);
  await login(context, m.name);
  await page.goto("/latest");
  const plain = await page.title();
  expect(plain).not.toMatch(/^\(/);
  expect(await page.evaluate(() => document.querySelector("script[src*='/chat-drawer']").dataset.chatSound)).toBe("soft");

  await setHidden(page, true);
  const first = await dm.send("a direct message for the title");
  // allChannelUrgentCount: a DM's unread messages.
  await counted(page, plain, 1);
  const shown = await shownWith(page, first);
  expect(shown.title).toContain("user0");
  expect(shown.tag).toMatch(/^discourse-notification-.*-0$/);
  expect(await sounds(page)).toEqual([SOFT_INCOMING]);
  // confirmChatAlert: played here, for the other tabs.
  const handled = await page.evaluate((key) => JSON.parse(localStorage.getItem(key)), ALERTS);
  expect(handled).toHaveLength(1);
  expect(handled[0].played).toBe(true);

  // Shown again: the count goes (setFocus).
  await setHidden(page, false);
  await expect(page).toHaveTitle(plain);
});

test("the contextual title leaves chat out, and the indicator preference decides the sound alone", async ({ page, context, session }) => {
  const m = member();
  // only_mentions: a DM's alert makes no sound.
  options(m, "chat_sound = 'soft', title_count_mode_key = 1, chat_header_indicator_preference = 3");
  const dm = await directMessages(session, m);
  await stub(context);
  await login(context, m.name);
  await page.goto("/latest");
  const plain = await page.title();
  await setHidden(page, true);
  const first = await dm.send("a direct message without a sound");
  await urgent(page, 1);
  await shownWith(page, first);
  expect(await sounds(page)).toEqual([]);
  await expect(page).toHaveTitle(plain);

  // The notifications mode counts it whatever the indicator shows
  // (getDocumentTitleCount asks for no preference): never.
  options(m, "title_count_mode_key = 0, chat_header_indicator_preference = 2");
  await page.goto("/categories");
  const categories = await page.title();
  expect(await page.evaluate(() => document.querySelector("script[src*='/chat-drawer']").dataset.indicatorPreference)).toBe("never");
  await setHidden(page, true);
  const second = await dm.send("a direct message the indicator hides");
  await counted(page, categories, 2);
  await shownWith(page, second);
  expect(await sounds(page)).toEqual([]);
});

test("a mention plays the mention's sound, and no sound without chat_sound", async ({ page, context, session }) => {
  const m = member();
  options(m, "chat_sound = 'soft', title_count_mode_key = 0, chat_header_indicator_preference = 1");
  const admin = await session("admin");
  const created = await api(admin, "POST", "/chat/api/channels.json", {
    "channel[chatable_id]": "2",
    "channel[name]": label("notify").replace(/\s+/g, "-").slice(0, 40),
  });
  const channel = created.channel.id;
  const them = await session(m.name);
  await api(them, "POST", `/chat/api/channels/${channel}/memberships/me.json`);
  const mention = async (what) => {
    const text = label(what);
    await api(admin, "POST", `/chat/${channel}.json`, { message: `@${m.name} ${text}`, staged_id: label("staged") });
    return text;
  };

  await stub(context);
  await login(context, m.name);
  await page.goto("/latest");
  const plain = await page.title();
  await setHidden(page, true);
  // dm_and_mentions lets a mention through, as its sequence.
  await shownWith(page, await mention("a mention with a sound"));
  expect(await sounds(page)).toEqual([SOFT_MENTION]);
  // The mention is urgent, and a notification of its own.
  await counted(page, plain, 1);

  options(m, "chat_sound = NULL");
  await page.goto("/categories");
  expect(await page.evaluate(() => document.querySelector("script[src*='/chat-drawer']").dataset.chatSound)).toBeUndefined();
  await shownWith(page, await mention("a mention without a sound"));
  expect(await sounds(page)).toEqual([]);
});

test("do not disturb silences chat's alerts and the title", async ({ page, context, session }) => {
  const m = member();
  options(m, "chat_sound = 'soft', title_count_mode_key = 0");
  const dm = await directMessages(session, m);
  await stub(context);
  await login(context, m.name);
  await page.goto("/latest");
  const plain = await page.title();
  const moon = page.locator("#toggle-current-user .do-not-disturb-background");
  await api(context, "POST", "/do-not-disturb.json", { duration: "30" });
  await expect(moon).toBeVisible();

  await setHidden(page, true);
  const paused = await dm.send("a direct message while paused");
  await urgent(page, 1);
  await alerted(m, paused);
  await expect(page).toHaveTitle(plain);

  // Out of it, the next one is heard and shown; the paused one never was.
  await api(context, "DELETE", "/do-not-disturb.json");
  await expect(moon).toHaveCount(0);
  const heard = await dm.send("a direct message once back");
  await shownWith(page, heard);
  expect(await sounds(page)).toEqual([SOFT_INCOMING]);
  expect(await page.evaluate((text) => window.__shown.filter((n) => (n.options.body || "").includes(text)).length, paused)).toBe(0);
  await counted(page, plain, 2);
});

// The server alerts a member looking at the channel only when the job
// outruns their reading it (user_has_seen_message?), so the alert is
// handed to the page as the stream would.
test("an alert for the channel in sight is claimed without a notification or a sound", async ({ page, context, session }) => {
  const m = member();
  options(m, "chat_sound = 'soft'");
  const dm = await directMessages(session, m);
  await stub(context);
  await login(context, m.name);
  await page.goto(`/chat/c/-/${dm.id}`);
  await expect(page.locator(".chat-channel[data-id]")).toHaveAttribute("data-id", String(dm.id));
  const alert = (channel, id) =>
    page.evaluate(
      (data) => document.dispatchEvent(new CustomEvent("htmx:sseBeforeMessage", { cancelable: true, detail: { type: "chat-notification-alert", data: JSON.stringify(data) } })),
      { notification_type: 30, chat_message_id: id, channel_id: channel, is_direct_message_channel: true, excerpt: `in channel ${channel}`, translated_title: "A message" },
    );
  const handled = () => page.evaluate((key) => JSON.parse(localStorage.getItem(key)) || [], ALERTS);
  await alert(dm.id, 4001);
  // claimChatAlert, not played.
  expect((await handled()).map((entry) => [entry.key, entry.played])).toEqual([[4001, undefined]]);
  expect(await sounds(page)).toEqual([]);
  expect(await page.evaluate(() => window.__shown.filter((n) => (n.options.body || "").includes("in channel")).length)).toBe(0);

  // Another channel's is shown and heard, once across the tabs' claims.
  await alert(dm.id + 100000, 4002);
  await shownWith(page, `in channel ${dm.id + 100000}`);
  await expect.poll(async () => (await handled()).map((entry) => [entry.key, entry.played])).toEqual([[4001, undefined], [4002, true]]);
  expect(await sounds(page)).toEqual([SOFT_INCOMING]);
  await alert(dm.id + 100000, 4002);
  expect(await sounds(page)).toEqual([SOFT_INCOMING]);
});
