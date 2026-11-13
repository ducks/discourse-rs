// discourse-lazy-videos' client (static/js/lazy-videos.js): the
// .lazy-video-container a YouTube, Vimeo or TikTok onebox cooked becomes
// LazyVideo's thumbnail button, and a click, Enter or Space swaps in
// LazyIframe's player. The onebox engines fetch the providers, so each
// post's cooked HTML is written by SQL in the shape Rails cooks
// (lib/discourse_lazy_videos/lazy_*.rb), and every request to a provider
// is answered here: nothing is loaded from them.
const { test, expect, login, api, topic, sql, label } = require("./helpers");

const PROVIDERS = /^https:\/\/([a-z0-9-]+\.)*(youtube\.com|vimeo\.com|vimeocdn\.com|tiktok\.com|tiktokcdn\.com)\//;
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=",
  "base64"
);
const ALLOW = "accelerometer; autoplay; encrypted-media; gyroscope; picture-in-picture";

const YOUTUBE = `<div class="youtube-onebox lazy-video-container" data-video-id="kPRA0W1kECg" data-video-title="15 Sorting Algorithms in 6 Minutes" data-video-start-time="3m54s" data-video-list-id="RDkPRA0W1kECg" data-provider-name="youtube">
  <a href="https://www.youtube.com/watch?v=kPRA0W1kECg&amp;list=RDkPRA0W1kECg&amp;t=3m54s" target="_blank" class="video-thumbnail" rel="noopener nofollow ugc">
    <img class="youtube-thumbnail" src="https://img.youtube.com/vi/kPRA0W1kECg/maxresdefault.jpg" title="15 Sorting Algorithms in 6 Minutes" data-dominant-color="00ffff" width="690" height="388">
  </a>
</div>
`;
// No playlist, the start in seconds, no dominant color.
const YOUTUBE_PLAIN = `<div class="youtube-onebox lazy-video-container" data-video-id="dQw4w9WgXcQ" data-video-title="Rick &amp; &quot;Roll&quot;" data-video-start-time="42" data-video-list-id="" data-provider-name="youtube">
  <a href="https://www.youtube.com/watch?v=dQw4w9WgXcQ&amp;t=42" target="_blank" class="video-thumbnail" rel="noopener nofollow ugc">
    <img class="youtube-thumbnail" src="https://img.youtube.com/vi/dQw4w9WgXcQ/maxresdefault.jpg" title="Rick &amp; &quot;Roll&quot;" width="690" height="388">
  </a>
</div>
`;
const YOUTUBE_NO_START = YOUTUBE_PLAIN.replace('data-video-start-time="42"', 'data-video-start-time=""')
  .replace("&amp;t=42", "")
  .replaceAll("dQw4w9WgXcQ", "aqz-KE-bpKQ");
const vimeo = (id, page) => `<div class="vimeo-onebox lazy-video-container" data-video-id="${id}" data-video-title="A Vimeo video" data-provider-name="vimeo">
  <a href="https://vimeo.com/${page}" target="_blank" rel="noopener nofollow ugc">
    <img class="vimeo-thumbnail" src="https://i.vimeocdn.com/video/1_640" title="A Vimeo video" width="640" height="360">
  </a>
</div>
`;
const TIKTOK = `<div class="tiktok-onebox lazy-video-container" data-video-id="6718335390845095173" data-video-title="A TikTok video" data-provider-name="tiktok">
  <a href="https://www.tiktok.com/@scout2015/video/6718335390845095173" target="_blank" rel="noopener nofollow ugc">
    <img class="tiktok-thumbnail" src="https://p16.tiktokcdn.com/obj/1.jpeg" title="A TikTok video" width="576" height="1024">
  </a>
</div>
`;

// A topic by user1 whose first post's cooked HTML is the one given, open
// in the page as the member given. Returns { t, post, requests }.
async function open(page, context, session, cooked, username = "user2") {
  const requests = [];
  await context.route(PROVIDERS, (route) => {
    const request = route.request();
    requests.push(request.url());
    return request.resourceType() === "image"
      ? route.fulfill({ status: 200, contentType: "image/png", body: PNG })
      : route.fulfill({ status: 200, contentType: "text/html", body: "<!doctype html><title>player</title>" });
  });
  const user1 = await session("user1");
  const t = await topic(user1, label("Lazy videos"), label("A topic for the lazy videos spec"));
  sql("UPDATE posts SET cooked = :'p1' WHERE id = :'p2'", cooked, String(t.posts[0]));
  await login(context, username);
  await page.goto(t.url);
  return { t, post: page.locator(`article[data-post-id="${t.posts[0]}"]`), requests };
}

function attributes(locator) {
  return locator.evaluate((el) => Object.fromEntries(el.getAttributeNames().map((n) => [n, el.getAttribute(n)])));
}

function iframeAttributes(src, title) {
  return { allow: ALLOW, allowfullscreen: "", frameborder: "0", scrolling: "no", seamless: "seamless", src, title };
}

test("a YouTube video is a thumbnail button until it is clicked", async ({ page, context, session }) => {
  const { post, requests } = await open(page, context, session, YOUTUBE);

  const wrapper = post.locator(".cooked > p.lazy-video-wrapper");
  await expect(wrapper).toHaveCount(1);
  const container = wrapper.locator("> .lazy-video-container");
  expect(await attributes(container)).toEqual({
    class: "lazy-video-container youtube-onebox",
    "data-provider-name": "youtube",
    "data-video-id": "kPRA0W1kECg",
    "data-video-list-id": "RDkPRA0W1kECg",
    "data-video-start-time": "3m54s",
    "data-video-title": "15 Sorting Algorithms in 6 Minutes",
  });
  const thumbnail = container.locator("> .video-thumbnail");
  expect(await attributes(thumbnail)).toEqual({
    class: "video-thumbnail youtube",
    "aria-label": "Play 15 Sorting Algorithms in 6 Minutes",
    role: "button",
    style: "background-color: #00ffff;",
    tabindex: "0",
  });
  expect(await attributes(thumbnail.locator("> img"))).toEqual({
    class: "youtube-thumbnail",
    loading: "lazy",
    src: "https://img.youtube.com/vi/kPRA0W1kECg/maxresdefault.jpg",
    title: "15 Sorting Algorithms in 6 Minutes",
  });
  // The provider's icon, from lazy-videos.scss.
  const icon = thumbnail.locator("> .icon.youtube-icon");
  await expect(icon).toHaveCSS("width", "68px");
  await expect(icon).toHaveCSS("background-image", /^url\("data:image\/svg\+xml;charset=utf8,%3Csvg/);
  const title = container.locator("> .title-container > .title-wrapper > a.title-link");
  await expect(title).toHaveText("15 Sorting Algorithms in 6 Minutes");
  expect(await attributes(title)).toEqual({
    class: "title-link",
    href: "https://www.youtube.com/watch?v=kPRA0W1kECg&list=RDkPRA0W1kECg&t=3m54s",
    rel: "noopener noreferrer",
    target: "_blank",
    title: "15 Sorting Algorithms in 6 Minutes",
  });
  await expect(container.locator("iframe")).toHaveCount(0);
  expect(requests.filter((url) => url.includes("/embed/"))).toEqual([]);

  await thumbnail.click();
  await expect(container).toHaveClass("lazy-video-container youtube-onebox video-loaded");
  await expect(container.locator("> *")).toHaveCount(1);
  const src = "https://www.youtube.com/embed/kPRA0W1kECg?autoplay=1&rel=0&list=RDkPRA0W1kECg&start=234";
  expect(await attributes(container.locator("> iframe"))).toEqual(
    iframeAttributes(src, "15 Sorting Algorithms in 6 Minutes")
  );
  await expect.poll(() => requests).toContain(src);
});

test("Enter and Space on the thumbnail load the video", async ({ page, context, session }) => {
  const { post } = await open(page, context, session, YOUTUBE_PLAIN + YOUTUBE_NO_START);

  const containers = post.locator(".lazy-video-wrapper > .lazy-video-container");
  await expect(containers).toHaveCount(2);
  const first = containers.nth(0);
  const thumbnail = first.locator(".video-thumbnail");
  await expect(thumbnail).toHaveAttribute("aria-label", 'Play Rick & "Roll"');
  // No dominant color, no style.
  expect(await thumbnail.getAttribute("style")).toBeNull();
  await expect(first).toHaveAttribute("data-video-list-id", "");

  await thumbnail.focus();
  await page.keyboard.press("a");
  await expect(first.locator("iframe")).toHaveCount(0);
  await page.keyboard.press("Enter");
  await expect(first.locator("iframe")).toHaveAttribute(
    "src",
    "https://www.youtube.com/embed/dQw4w9WgXcQ?autoplay=1&rel=0&start=42"
  );
  await expect(first.locator("iframe")).toHaveAttribute("title", 'Rick & "Roll"');

  const second = containers.nth(1);
  await second.locator(".video-thumbnail").focus();
  await page.keyboard.press("Space");
  await expect(second.locator("iframe")).toHaveAttribute(
    "src",
    "https://www.youtube.com/embed/aqz-KE-bpKQ?autoplay=1&rel=0"
  );
  await expect(second).toHaveClass(/video-loaded/);
});

test("a Vimeo video loads its player, an unlisted one with its hash", async ({ page, context, session }) => {
  const { post } = await open(
    page,
    context,
    session,
    vimeo("226053498", "226053498") + vimeo("912345678?h=0a1b2c3d4e", "912345678/0a1b2c3d4e")
  );

  const containers = post.locator(".lazy-video-wrapper > .lazy-video-container.vimeo-onebox");
  await expect(containers).toHaveCount(2);
  const first = containers.nth(0);
  // The attributes the cooked HTML doesn't carry are left out.
  expect(await attributes(first)).toEqual({
    class: "lazy-video-container vimeo-onebox",
    "data-provider-name": "vimeo",
    "data-video-id": "226053498",
    "data-video-title": "A Vimeo video",
  });
  await expect(first.locator(".video-thumbnail.vimeo > img.vimeo-thumbnail")).toHaveAttribute(
    "src",
    "https://i.vimeocdn.com/video/1_640"
  );
  await expect(first.locator(".icon.vimeo-icon")).toHaveCSS("width", "77px");
  await expect(first.locator("a.title-link")).toHaveAttribute("href", "https://vimeo.com/226053498");

  await first.locator(".video-thumbnail").click();
  expect(await attributes(first.locator("> iframe"))).toEqual(
    iframeAttributes("https://player.vimeo.com/video/226053498?autoplay=1", "A Vimeo video")
  );

  const second = containers.nth(1);
  await second.locator(".video-thumbnail").click();
  await expect(second.locator("> iframe")).toHaveAttribute(
    "src",
    "https://player.vimeo.com/video/912345678?h=0a1b2c3d4e&autoplay=1"
  );
});

test("a provider is left alone while its setting is off", async ({ page, context, session }) => {
  // lazy_tiktok_enabled is off by default.
  const { t, post } = await open(page, context, session, TIKTOK + YOUTUBE);

  await expect(post.locator(".lazy-video-wrapper > .youtube-onebox")).toHaveCount(1);
  const cooked = post.locator(".cooked > .tiktok-onebox.lazy-video-container");
  await expect(cooked).toHaveCount(1);
  await expect(cooked.locator("> a > img.tiktok-thumbnail")).toHaveCount(1);
  await expect(cooked.locator(".video-thumbnail")).toHaveCount(0);
  await expect(post.locator(".lazy-video-wrapper")).toHaveCount(1);

  const admin = await session("admin");
  await api(admin, "PUT", "/admin/site_settings/lazy_tiktok_enabled", { lazy_tiktok_enabled: "true" });
  try {
    await page.goto(t.url);
    const container = post.locator(".lazy-video-wrapper > .lazy-video-container.tiktok-onebox");
    await expect(container).toHaveCount(1);
    await expect(container.locator(".video-thumbnail.tiktok")).toHaveAttribute("aria-label", "Play A TikTok video");
    await expect(container.locator(".icon.tiktok-icon")).toHaveCSS("width", "58px");
    await expect(container.locator("a.title-link")).toHaveAttribute(
      "href",
      "https://www.tiktok.com/@scout2015/video/6718335390845095173"
    );
    // TikTok's player isn't fluid.
    await expect(container).toHaveCSS("width", "332px");

    await container.locator(".video-thumbnail").click();
    expect(await attributes(container.locator("> iframe"))).toEqual(
      iframeAttributes("https://www.tiktok.com/embed/v2/6718335390845095173", "A TikTok video")
    );
  } finally {
    await api(admin, "PUT", "/admin/site_settings/lazy_tiktok_enabled", { lazy_tiktok_enabled: "false" });
  }
});

test("a post drawn again has one thumbnail button", async ({ page, context, session }) => {
  const { t, post } = await open(page, context, session, YOUTUBE);

  const container = post.locator(".lazy-video-wrapper > .lazy-video-container");
  await expect(container).toHaveCount(1);
  await container.locator(".video-thumbnail").click();
  await expect(container.locator("iframe")).toHaveCount(1);

  // The post drawn again (GET /live/post/:id, as the live stream and the
  // post's actions do): its content is a new element, decorated once.
  await post.locator(".cooked").evaluate((el) => el.setAttribute("data-before-redraw", ""));
  await page.evaluate(
    (id) =>
      window.htmx.ajax(
        "GET",
        document.querySelector('meta[name="discourse-base-uri"]').content + "/live/post/" + id,
        { swap: "none" }
      ),
    t.posts[0]
  );
  await expect(page.locator(".cooked[data-before-redraw]")).toHaveCount(0);
  await expect(post.locator(".lazy-video-wrapper")).toHaveCount(1);
  await expect(post.locator(".lazy-video-container")).toHaveCount(1);
  await expect(container.locator(".video-thumbnail.youtube")).toHaveCount(1);
  await expect(container.locator("iframe")).toHaveCount(0);

  // Decorating the content again leaves it as it is.
  await post.evaluate((el) => window.Discourse.decorateCooked.apply(el.querySelector(".lazy-video-wrapper")));
  await expect(post.locator(".lazy-video-wrapper")).toHaveCount(1);
  await expect(post.locator(".lazy-video-container .lazy-video-container")).toHaveCount(0);

  await container.locator(".video-thumbnail").click();
  await expect(container.locator("iframe")).toHaveAttribute(
    "src",
    "https://www.youtube.com/embed/kPRA0W1kECg?autoplay=1&rel=0&list=RDkPRA0W1kECg&start=234"
  );
});
