// The rich editor's images, videos, audio, uploads and grids (ROADMAP
// 196r2): the reference's
// frontend/discourse/tests/integration/components/prosemirror-editor/
// image-test.js, prosemirror-editor-test.gjs's grid cases and
// tests/unit/lib/to-markdown-test.js's image cases, on the editor the
// helper opens (rich-editor-helper.js), and an upload in the composer's
// rich mode.
const { test, expect, login, topic, label, sql, member } = require("./helpers");
const { openEditorPage, setupRichEditor, editorValue } = require("./rich-editor-helper");

const host = "#rich-spec-host";
// assert.dom("img"): the first image, not ProseMirror's separator.
const IMG = "img:not(.ProseMirror-separator)";

function requestedShortUrls(request) {
  return new URLSearchParams(request.postData() || "").getAll("short_urls[]");
}

function videoLookup(urls, base62Sha1, extension = "mp4") {
  const uploads = [
    { short_url: `upload://${base62Sha1}.${extension}`, url: `/uploads/video.${extension}` },
    { short_url: `upload://${base62Sha1}`, url: "/uploads/video-thumbnail.png" },
  ];
  return uploads.filter((upload) => urls.includes(upload.short_url));
}

// pretender.post("/uploads/lookup-urls", handler): handler(urls, count)
// gives the uploads answered.
async function lookupUrls(page, handler) {
  let count = 0;
  // The resolved media never answer: a 404 would fire the video's error.
  await page.route("**/uploads/video*", () => {});
  await page.route("**/uploads/lookup-urls", (route) => {
    count++;
    const body = handler(requestedShortUrls(route.request()), count);
    return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(body) });
  });
}

// testRenderedMarkdown: the checks, then the markdown an edit writes
// unchanged.
async function testRendered(page, markdown, assertions) {
  const result = await setupRichEditor(page, markdown);
  await assertions(page.locator(host));
  expect(result.value, `Markdown should match for ${JSON.stringify(markdown)}`).toBe(markdown);
}

async function spyAnnouncements(page) {
  await page.evaluate(() => {
    window.richSpecAnnounced = [];
  });
}

function setupWithA11y(page, markdown) {
  return page.evaluate(
    (markdown) =>
      window.richSpec.setup(markdown, {
        context: {
          a11y: {
            announce: (message, level) => window.richSpecAnnounced.push([message, level]),
          },
        },
      }),
    markdown
  );
}

function announced(page) {
  return page.evaluate(() => window.richSpecAnnounced);
}

function dispatch(page, fn, arg) {
  return page.evaluate(fn, arg);
}

test.describe("rich editor images", () => {
  test.beforeEach(async ({ page }) => {
    await openEditorPage(page);
  });

  test("basic image", async ({ page }) => {
    await testRendered(page, "![alt text](https://example.com/image.jpg)", async (el) => {
      await expect(el.locator(IMG)).toHaveCount(1);
      await expect(el.locator(IMG)).toHaveAttribute("src", "https://example.com/image.jpg");
      await expect(el.locator(IMG)).toHaveAttribute("alt", "alt text");
    });
  });

  test("image with title", async ({ page }) => {
    await testRendered(page, '![alt text](https://example.com/image.jpg "title")', async (el) => {
      await expect(el.locator(IMG)).toHaveAttribute("src", "https://example.com/image.jpg");
      await expect(el.locator(IMG)).toHaveAttribute("alt", "alt text");
      await expect(el.locator(IMG)).toHaveAttribute("title", "title");
    });
  });

  test("image with dimensions and title", async ({ page }) => {
    await testRendered(page, '![alt text|100x200](https://example.com/image.jpg "title")', async (el) => {
      const img = el.locator(IMG);
      await expect(img).toHaveAttribute("src", "https://example.com/image.jpg");
      await expect(img).toHaveAttribute("alt", "alt text");
      await expect(img).toHaveAttribute("title", "title");
      await expect(img).toHaveAttribute("width", "100");
      await expect(img).toHaveAttribute("height", "200");
    });
  });

  test("image with dimensions and scale", async ({ page }) => {
    await testRendered(page, "![alt text|100x200, 50%](https://example.com/image.jpg)", async (el) => {
      const img = el.locator(IMG);
      await expect(img).toHaveAttribute("width", "100");
      await expect(img).toHaveAttribute("height", "200");
      await expect(img).toHaveAttribute("data-scale", "50");
      expect(await img.evaluate((e) => e.style.width), "Image width style should be 50px").toBe("50px");
    });
  });

  test("image with dimensions, scale and thumbnail", async ({ page }) => {
    await testRendered(page, "![alt text|100x200, 50%|thumbnail](https://example.com/image.jpg)", async (el) => {
      const img = el.locator(IMG);
      await expect(img).toHaveAttribute("src", "https://example.com/image.jpg");
      await expect(img).toHaveAttribute("alt", "alt text");
      await expect(img).toHaveAttribute("width", "100");
      await expect(img).toHaveAttribute("height", "200");
      await expect(img).toHaveAttribute("data-scale", "50");
      await expect(img).toHaveAttribute("data-thumbnail", "true");
      expect(await img.evaluate((e) => e.style.width)).toBe("50px");
    });
  });

  test("image with parentheses in URL", async ({ page }) => {
    await testRendered(page, "![alt text](https://example.com/image\\(1\\).jpg)", async (el) => {
      await expect(el.locator(IMG)).toHaveAttribute("src", "https://example.com/image(1).jpg");
      await expect(el.locator(IMG)).toHaveAttribute("alt", "alt text");
    });
  });

  test("video player resolves its video and poster", async ({ page }) => {
    const markdown = "![Screen Recording|video](upload://videoHash.MOV)";
    let shortUrls;
    await lookupUrls(page, (urls) => {
      shortUrls = urls;
      return videoLookup(urls, "videoHash", "MOV");
    });

    const result = await setupRichEditor(page, markdown);
    const video = page.locator(`${host} .composer-video-node video`);

    await expect(video).toHaveAttribute("poster", "/uploads/video-thumbnail.png");
    await expect(video, "the video source is not loaded before playback is requested").not.toHaveAttribute("src", /.*/);
    await expect(video).toHaveAttribute("controls", "");
    await expect(video).toHaveAttribute("aria-label", "Screen Recording");
    await expect(page.locator(`${host} .composer-video-node__play`)).toHaveCount(1);
    expect(shortUrls).toEqual(["upload://videoHash.MOV", "upload://videoHash"]);
    expect(result.value).toBe(markdown);
  });

  test("video player ignores a poster that resolves to the video", async ({ page }) => {
    await lookupUrls(page, (urls) => urls.map((short_url) => ({ short_url, url: "/uploads/video.mp4" })));
    await setupRichEditor(page, "![alt text|video](upload://posterlessHash.mp4)");
    const video = page.locator(`${host} .composer-video-node video`);
    await expect(page.locator(`${host} .composer-video-node__play`)).toHaveCount(1);
    await page.waitForTimeout(200);
    await expect(video).not.toHaveAttribute("poster", /.*/);
    await expect(page.locator(`${host} .composer-video-node__error`)).toHaveCount(0);
  });

  test("lightbox shows the document's images, skipping videos and audio", async ({ page }) => {
    await lookupUrls(page, (urls) => videoLookup(urls, "galleryVideoHash"));
    await setupRichEditor(
      page,
      "![one](/images/emoji/twitter/tada.png)\n\n![clip|video](upload://galleryVideoHash.mp4)\n\n![song|audio](upload://galleryAudioHash.mp3)\n\n![two](/images/emoji/twitter/smile.png)"
    );
    await dispatch(page, async () => {
      const { NodeSelection } = await import("prosemirror-state");
      const view = window.richSpec.editor.view;
      let secondImagePos;
      view.state.doc.descendants((node, pos) => {
        if (node.attrs.alt === "two") {
          secondImagePos = pos;
        }
      });
      view.dispatch(view.state.tr.setSelection(NodeSelection.create(view.state.doc, secondImagePos)));
    });
    await page.locator(`${host} img[alt="two"]`).dispatchEvent("click");
    await expect(page.locator(".pswp--open")).toBeVisible();
    await expect(page.locator(".pswp__counter")).toHaveText("2 / 2");
    await expect(page.locator(".pswp__caption-title")).toHaveText("two");
    // PhotoSwipe ignores a close while its opening zoom still runs.
    await expect(async () => {
      await page.locator(".pswp").dispatchEvent("keydown", { key: "Escape" });
      await expect(page.locator(".pswp")).toHaveCount(0, { timeout: 1000 });
    }).toPass();
  });

  test("video player can be selected before and after playback", async ({ page }) => {
    await lookupUrls(page, (urls) => videoLookup(urls, "selectableVideoHash"));
    await setupRichEditor(page, "![alt text|video](upload://selectableVideoHash.mp4)");
    await expect(page.locator(`${host} .composer-video-node video`)).toHaveAttribute("poster", /thumbnail/);
    await page.evaluate(() => {
      const video = document.querySelector("#rich-spec-host .composer-video-node video");
      window.richSpecVideo = { clicks: 0, played: false };
      video.addEventListener("click", () => window.richSpecVideo.clicks++);
      video.play = () => {
        window.richSpecVideo.played = true;
        video.dispatchEvent(new Event("playing"));
        return Promise.resolve();
      };
    });
    const textSelection = async () => {
      await page.evaluate(async () => {
        const { TextSelection } = await import("prosemirror-state");
        const view = window.richSpec.editor.view;
        view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 2)));
      });
    };
    const nodeSelected = () =>
      page.evaluate(async () => {
        const { NodeSelection } = await import("prosemirror-state");
        return window.richSpec.editor.view.state.selection instanceof NodeSelection;
      });

    await textSelection();
    // Away from the play control, where the paused video lets the click
    // through to the editor.
    await page.locator(`${host} .composer-video-node`).click({ position: { x: 10, y: 10 } });
    expect(await nodeSelected(), "clicking the player selects the video node").toBe(true);
    expect(await page.evaluate(() => window.richSpecVideo.played)).toBe(false);
    expect(
      await page.locator(`${host} .composer-video-node video`).evaluate((v) => getComputedStyle(v).pointerEvents)
    ).toBe("none");

    await textSelection();
    await page.locator(`${host} .composer-video-node__play`).click();
    await textSelection();
    await page.locator(`${host} .composer-video-node video`).dispatchEvent("click");
    expect(await nodeSelected(), "clicking the video selects it after playback").toBe(true);
    expect(await page.evaluate(() => window.richSpecVideo.clicks)).toBe(1);
  });

  test("video play control tracks playback", async ({ page }) => {
    await lookupUrls(page, (urls) => videoLookup(urls, "playableVideoHash"));
    await setupRichEditor(page, "![alt text|video](upload://playableVideoHash.mp4)");
    await expect(page.locator(`${host} .composer-video-node video`)).toHaveAttribute("poster", /thumbnail/);
    await page.evaluate(async () => {
      const { TextSelection } = await import("prosemirror-state");
      const video = document.querySelector("#rich-spec-host .composer-video-node video");
      window.richSpecVideo = { played: false, src: null };
      video.play = () => {
        window.richSpecVideo.src = video.getAttribute("src");
        window.richSpecVideo.played = true;
        video.dispatchEvent(new Event("play"));
        return Promise.resolve();
      };
      const view = window.richSpec.editor.view;
      view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 2)));
    });

    const play = page.locator(`${host} .composer-video-node__play`);
    const video = page.locator(`${host} .composer-video-node video`);
    await play.click();
    expect(await page.evaluate(() => window.richSpecVideo)).toEqual({ played: true, src: "/uploads/video.mp4" });
    expect(
      await page.evaluate(async () => {
        const { NodeSelection } = await import("prosemirror-state");
        return window.richSpec.editor.view.state.selection instanceof NodeSelection;
      }),
      "playing the video does not select it"
    ).toBe(false);
    await expect(play, "the play control remains until playback begins").toHaveCount(1);

    await video.dispatchEvent("playing");
    await expect(play).toHaveCount(0);
    expect(await video.evaluate((v) => getComputedStyle(v).pointerEvents)).toBe("auto");

    await video.dispatchEvent("pause");
    await expect(play).toHaveCount(0);
    expect(await video.evaluate((v) => getComputedStyle(v).pointerEvents)).toBe("auto");
  });

  test("video player keeps stable 16:9 geometry", async ({ page }) => {
    await lookupUrls(page, (urls) => videoLookup(urls, "geometryVideoHash"));
    await setupRichEditor(page, "![alt text|video](upload://geometryVideoHash.mp4)");
    const rects = await page.evaluate(() => {
      const player = document.querySelector("#rich-spec-host .composer-video-node");
      const video = player.querySelector("video");
      const before = player.getBoundingClientRect();
      const videoRect = video.getBoundingClientRect();
      video.dispatchEvent(new Event("playing"));
      return { before: before.toJSON(), video: videoRect.toJSON() };
    });
    const { before, video } = rects;
    expect(before.width).toBeGreaterThan(0);
    expect(before.height).toBeGreaterThan(0);
    expect(Math.abs(before.width / before.height - 16 / 9)).toBeLessThan(0.01);
    expect([video.top, video.right, video.bottom, video.left]).toEqual([
      before.top,
      before.right,
      before.bottom,
      before.left,
    ]);
    await expect(page.locator(`${host} .composer-video-node__play`)).toHaveCount(0);
    const after = await page.locator(`${host} .composer-video-node`).evaluate((p) => p.getBoundingClientRect().toJSON());
    expect(after.width).toBe(before.width);
    expect(after.height).toBe(before.height);
  });

  test("video URL lookup failures can be retried", async ({ page }) => {
    await lookupUrls(page, (urls, count) => (count === 1 ? [] : videoLookup(urls, "retryVideoHash")));
    await spyAnnouncements(page);
    await setupWithA11y(page, "![alt text|video](upload://retryVideoHash.mp4)");
    const error = page.locator(`${host} .composer-video-node__error`);
    await expect(error).toHaveText("This video cannot be played because the URL is invalid or unavailable.");
    expect(await announced(page)).toEqual([
      ["This video cannot be played because the URL is invalid or unavailable.", "polite"],
    ]);
    await page.evaluate(() => {
      const video = document.querySelector("#rich-spec-host .composer-video-node video");
      window.richSpecVideo = { played: false, src: null };
      video.play = () => {
        window.richSpecVideo.src = video.getAttribute("src");
        window.richSpecVideo.played = true;
        video.dispatchEvent(new Event("playing"));
        return Promise.resolve();
      };
    });

    await page.locator(`${host} .composer-video-node__play`).click();
    await expect(error).toHaveCount(0);
    expect(await page.evaluate(() => window.richSpecVideo)).toEqual({ played: true, src: "/uploads/video.mp4" });
    await expect(page.locator(`${host} .composer-video-node video`)).toHaveAttribute("src", "/uploads/video.mp4");
  });

  test("video playback failures are displayed", async ({ page }) => {
    await lookupUrls(page, (urls) => videoLookup(urls, "brokenVideoHash"));
    await spyAnnouncements(page);
    await setupWithA11y(page, "![alt text|video](upload://brokenVideoHash.mp4)");
    await expect(page.locator(`${host} .composer-video-node video`)).toHaveAttribute("poster", /thumbnail/);
    await page.evaluate(() => {
      document.querySelector("#rich-spec-host .composer-video-node video").play = () => Promise.reject();
    });
    await page.locator(`${host} .composer-video-node__play`).click();
    const message = "This video cannot be rendered because your browser does not support the codec.";
    await expect(page.locator(`${host} .composer-video-node__error`)).toHaveText(message);
    expect(await announced(page)).toEqual([[message, "assertive"]]);
    await page.locator(`${host} .composer-video-node video`).dispatchEvent("error");
    expect((await announced(page)).length, "the same playback failure is not announced twice").toBe(1);
  });

  test("pending playback failures are ignored after teardown", async ({ page }) => {
    await lookupUrls(page, (urls) => videoLookup(urls, "removedVideoHash"));
    await spyAnnouncements(page);
    await setupWithA11y(page, "![alt text|video](upload://removedVideoHash.mp4)");
    await expect(page.locator(`${host} .composer-video-node video`)).toHaveAttribute("poster", /thumbnail/);
    await page.evaluate(() => {
      const video = document.querySelector("#rich-spec-host .composer-video-node video");
      window.richSpecPlayback = new Promise((started) => {
        video.play = () =>
          new Promise((_resolve, reject) => {
            window.richSpecReject = reject;
            started();
          });
      });
    });
    await page.locator(`${host} .composer-video-node__play`).click();
    await page.evaluate(async () => {
      await window.richSpecPlayback;
      const view = window.richSpec.editor.view;
      view.dispatch(view.state.tr.delete(1, 2));
      window.richSpecReject();
      await new Promise((resolve) => setTimeout(resolve, 50));
    });
    expect(await announced(page)).toEqual([]);
  });

  test("round-trips a video through the clipboard", async ({ page }) => {
    const markdown = '![alt text|video](upload://clipboardHash.mp4 "video title")';
    await lookupUrls(page, (urls) => videoLookup(urls, "clipboardHash"));
    await setupRichEditor(page, markdown);
    await page.evaluate(async () => {
      const { NodeSelection } = await import("prosemirror-state");
      const view = window.richSpec.editor.view;
      const clipboardData = new DataTransfer();
      view.dispatch(view.state.tr.setSelection(NodeSelection.create(view.state.doc, 1)));
      const { dom, text } = view.serializeForClipboard(view.state.selection.content());
      clipboardData.setData("text/html", dom.innerHTML);
      clipboardData.setData("text/plain", text);
      view.dispatch(view.state.tr.deleteSelection());
      view.dom.dispatchEvent(new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData }));
    });
    await expect(page.locator(`${host} .composer-video-node video`)).toHaveCount(1);
    expect(await editorValue(page)).toBe(markdown);
  });

  test("audio element", async ({ page }) => {
    await testRendered(page, "![alt text|audio](upload://hash)", async (el) => {
      await expect(el.locator("audio")).toHaveCount(1);
      await expect(el.locator("audio")).toHaveAttribute("preload", "metadata");
      await expect(el.locator("audio source")).toHaveAttribute("data-orig-src", "upload://hash");
    });
  });

  test("upload:// image alt text is preserved verbatim across round-trips", async ({ page }) => {
    await lookupUrls(page, () => []);
    await testRendered(page, "![_test_file_|100x100](upload://hash)", async (el) => {
      await expect(el.locator(IMG)).toHaveAttribute("alt", "_test_file_");
    });
  });

  test("dragging an image within the editor is a move, not an upload", async ({ page }) => {
    await setupRichEditor(page, "![alt text](https://example.com/image.jpg)");
    const result = await page.evaluate(() => {
      const view = window.richSpec.editor.view;
      const image = view.dom.querySelector("img");
      let reachedUploadTarget = false;
      view.dom.parentElement.addEventListener("drop", () => (reachedUploadTarget = true));
      const drag = (type, target) => {
        const dataTransfer = new DataTransfer();
        dataTransfer.items.add(new File(["x"], "image.png", { type: "image/png" }));
        target.dispatchEvent(new DragEvent(type, { dataTransfer, bubbles: true, cancelable: true }));
      };
      drag("dragstart", image);
      const owned = !!view.dragging;
      drag("drop", image);
      return { owned, reachedUploadTarget, dragging: view.dragging };
    });
    expect(result).toEqual({ owned: true, reachedUploadTarget: false, dragging: null });
  });

  test("a file dropped from outside still reaches the uploader", async ({ page }) => {
    await setupRichEditor(page, "![alt text](https://example.com/image.jpg)");
    const reached = await page.evaluate(() => {
      const view = window.richSpec.editor.view;
      let reachedUploadTarget = false;
      view.dom.parentElement.addEventListener("drop", () => (reachedUploadTarget = true));
      const dataTransfer = new DataTransfer();
      dataTransfer.items.add(new File(["x"], "image.png", { type: "image/png" }));
      view.dom
        .querySelector("img")
        .dispatchEvent(new DragEvent("drop", { dataTransfer, bubbles: true, cancelable: true }));
      return reachedUploadTarget;
    });
    expect(reached).toBe(true);
  });

  test("a selected image shows its toolbar and alt text, and scales", async ({ page }) => {
    await setupRichEditor(page, "![alt text|100x200](https://example.com/image.jpg)");
    await page.evaluate(async () => {
      const { NodeSelection } = await import("prosemirror-state");
      const view = window.richSpec.editor.view;
      view.dispatch(view.state.tr.setSelection(NodeSelection.create(view.state.doc, 1)));
    });
    const toolbar = page.locator(`${host} [data-identifier="composer-image-toolbar"]`);
    await expect(toolbar.locator(".composer-image-toolbar__zoom-out")).toHaveAttribute("title", "Decrease image size");
    await expect(toolbar.locator(".composer-image-toolbar__zoom-in")).toBeDisabled();
    await expect(page.locator(`${host} [data-identifier="composer-image-alt-text"] .image-alt-text-input__display`)).toHaveText(
      "alt text"
    );
    await toolbar.locator(".composer-image-toolbar__zoom-out").click();
    expect(await editorValue(page)).toBe("![alt text|100x200, 75%](https://example.com/image.jpg)");
    await expect(page.locator(`${host} ${IMG}`)).toHaveAttribute("style", "width: 75px");

    await page.locator(`${host} .image-alt-text-input__display`).click();
    const field = page.locator(`${host} .image-alt-text-input__field`);
    await field.fill("a new description");
    await field.press("Enter");
    expect(await editorValue(page)).toBe("![a new description|100x200, 75%](https://example.com/image.jpg)");

    await page.evaluate(async () => {
      const { NodeSelection } = await import("prosemirror-state");
      const view = window.richSpec.editor.view;
      view.dispatch(view.state.tr.setSelection(NodeSelection.create(view.state.doc, 1)));
    });
    await page.locator(`${host} .composer-image-toolbar__add-to-grid`).click();
    expect(await editorValue(page)).toBe(
      "\n[grid]\n\n![a new description|100x200, 75%](https://example.com/image.jpg)\n\n\n[/grid]\n\n"
    );
  });
});

test.describe("rich editor grids", () => {
  test.beforeEach(async ({ page }) => {
    await openEditorPage(page);
  });

  test("a grid round-trips with its mode", async ({ page }) => {
    const result = await setupRichEditor(page, "[grid mode=carousel]\n![a](https://example.com/a.jpg)\n![b](https://example.com/b.jpg)\n[/grid]");
    await expect(page.locator(`${host} .composer-image-grid .composer-image-node`)).toHaveCount(2);
    // grid.js writes a newline before the tag and one more before the close.
    expect(result.value).toBe(
      "\n[grid mode=carousel]\n\n![a](https://example.com/a.jpg)\n\n![b](https://example.com/b.jpg)\n\n\n[/grid]\n\n"
    );
    await page.locator(`${host} .composer-image-gallery__mode-btn[data-mode="grid"]`).click();
    expect(await editorValue(page)).toBe(
      "\n[grid]\n\n![a](https://example.com/a.jpg)\n\n![b](https://example.com/b.jpg)\n\n\n[/grid]\n\n"
    );
    await page.locator(`${host} .composer-image-grid__remove-btn`).click();
    expect(await editorValue(page)).toBe("![a](https://example.com/a.jpg)\n\n![b](https://example.com/b.jpg)");
  });

  test("auto-grids image filenames using generic placeholders", async ({ page }) => {
    const result = await page.evaluate(async () => {
      const editorModule = await window.Discourse.richEditor.load();
      const grid = (await import("discourse/rich-editor/extensions/grid")).default;
      const image = await import("discourse/rich-editor/extensions/image");
      const uploads = await import("discourse/rich-editor/extensions/upload-placeholder");
      const hostEl = document.createElement("div");
      hostEl.id = "rich-spec-host";
      document.body.appendChild(hostEl);
      const editor = editorModule.createEditor({
        container: hostEl,
        value: "",
        engine: window.richSpec.engine,
        extensions: [grid, image.default, uploads.default],
        context: { i18n: (key) => key, siteSettings: {}, appEvents: window.richSpec.appEvents },
      });
      window.richSpec.editor = editor;
      const uploadingText = "Uploading: %{filename}…";
      const handler = uploads.placeholderHandler(editor, uploadingText);
      const files = [1, 2, 3].map((number) => ({
        id: `image-${number}`,
        name: `IMG_${number}.HEIC`,
        data: new File(["image"], `IMG_${number}.HEIC`),
      }));
      files.forEach((file) => handler.insert(file));
      uploads.autoGridImages(
        editor,
        files.map((file) => file.name),
        uploadingText
      );
      const grids = hostEl.querySelectorAll(".composer-image-grid").length;
      const placeholders = hostEl.querySelectorAll(".composer-image-grid .upload-placeholder.--file").length;

      const shortUrls = files.map((file, index) => {
        const shortUrl = `upload://heic-grid-${index}.jpeg`;
        image.cacheShortUploadUrl(shortUrl, { url: `/images/heic-grid-${index}.jpeg` });
        handler.success(file, `![IMG_${index + 1}](${shortUrl})`);
        return shortUrl;
      });
      let replacementSrc;
      editor.view.state.doc.descendants((node) => {
        if (node.type.name === "image" && node.attrs.originalSrc === shortUrls[0]) {
          replacementSrc = node.attrs.src;
          return false;
        }
      });
      return {
        grids,
        placeholders,
        replacementSrc,
        gridsAfter: hostEl.querySelectorAll(".composer-image-grid").length,
        images: hostEl.querySelectorAll(".composer-image-grid .composer-image-node").length,
      };
    });
    expect(result).toEqual({
      grids: 1,
      placeholders: 3,
      replacementSrc: "/images/heic-grid-0.jpeg",
      gridsAfter: 1,
      images: 3,
    });
  });

  test("keeps controls available for multiple short grids", async ({ page }) => {
    await page.evaluate(async () => {
      const editorModule = await window.Discourse.richEditor.load();
      const grid = (await import("discourse/rich-editor/extensions/grid")).default;
      const image = (await import("discourse/rich-editor/extensions/image")).default;
      const hostEl = document.createElement("div");
      hostEl.id = "rich-spec-host";
      document.body.appendChild(hostEl);
      const editor = editorModule.createEditor({
        container: hostEl,
        value: "",
        engine: window.richSpec.engine,
        extensions: [grid, image],
        context: { i18n: (key) => key, siteSettings: {} },
      });
      window.richSpec.editor = editor;
      const { schema } = editor.view.state;
      const emptyGrid = () => schema.nodes.grid.createAndFill();
      const separator = schema.nodes.paragraph.create(null, schema.text("Between grids"));
      editor.view.dispatch(
        editor.view.state.tr.replaceWith(0, editor.view.state.doc.content.size, [emptyGrid(), separator, emptyGrid()])
      );
    });

    await expect(page.locator(`${host} .composer-image-grid`)).toHaveCount(2);
    await expect(page.locator('[data-identifier^="composer-image-grid-mode-"]')).toHaveCount(2);
    await expect(page.locator('[data-identifier^="composer-image-grid-remove-"]')).toHaveCount(2);
    await expect(page.locator('[data-identifier^="composer-image-grid-"][role="none"]')).toHaveCount(4);

    await expect
      .poll(
        () =>
          page.evaluate(() => {
            const modeMenus = [...document.querySelectorAll('[data-identifier^="composer-image-grid-mode-"]')];
            return modeMenus.every((modeMenu) => {
              const id = modeMenu.dataset.identifier.replace("composer-image-grid-mode-", "");
              const removeMenu = document.querySelector(`[data-identifier="composer-image-grid-remove-${id}"]`);
              return modeMenu.getBoundingClientRect().bottom < removeMenu.getBoundingClientRect().top;
            });
          }),
        { timeout: 3000 }
      )
      .toBe(true);
  });
});

// to-markdown-test.js's image cases: [html, markdown]
const TO_MARKDOWN = {
  "converts img tag": (() => {
    const url = "https://example.com/image.png";
    const base62SHA1 = "q16M6GR110R47Z9p9Dk3PMXOJoE";
    return [
      [`<img src="${url}" width="100" height="50">`, `![|100x50](${url})`],
      [`<img src="${url}" width="100" height="50" title="some title">`, `![|100x50](${url} "some title")`],
      [
        `<img src="${url}" width="100" height="50" title="some title" data-base62-sha1="${base62SHA1}">`,
        `![|100x50](upload://${base62SHA1}.png "some title")`,
      ],
      [`<div><span><img src="${url}" alt="description" width="50" height="100" /></span></div>`, `![description|50x100](${url})`],
      [`<a href="http://example.com"><img src="${url}" alt="description" /></a>`, `[![description](${url})](http://example.com)`],
      [`<a href="http://example.com">description <img src="${url}" /></a>`, `[description ![](${url})](http://example.com)`],
      [`<img alt="description" />`, ""],
      [`<a><img src="${url}" alt="description" /></a>`, `![description](${url})`],
    ];
  })(),
  "converts image lightboxes to markdown": (() => {
    const original =
      "https://d11a6trkgmumsb.cloudfront.net/uploads/default/original/1X/8hkjhk7692f6afed3cb99d43ab2abd4e30aa8cba.jpeg";
    const optimized =
      "https://d11a6trkgmumsb.cloudfront.net/uploads/default/optimized/1X/8hkjhk7692f6afed3cb99d43ab2abd4e30aa8cba_2_689x459.jpeg";
    const srcset = `${optimized}, https://d11a6trkgmumsb.cloudfront.net/uploads/default/optimized/1X/8hkjhk7692f6afed3cb99d43ab2abd4e30aa8cba_2_1033x688.jpeg 1.5x, https://d11a6trkgmumsb.cloudfront.net/uploads/default/optimized/1X/8hkjhk7692f6afed3cb99d43ab2abd4e30aa8cba_2_1378x918.jpeg 2x`;
    const a = `<a class="lightbox" href="${original}" data-download-href="https://d11a6trkgmumsb.cloudfront.net/uploads/default/8hkjhk7692f6afed3cb99d43ab2abd4e30aa8cba" title="sherlock3_sig.jpg" rel="nofollow noopener">`;
    const meta = `<div class="meta">
    <span class="filename">sherlock3_sig.jpg</span><span class="informations">5496×3664 2 MB</span><span class="expand"></span>
    </div>`;
    const markdown = `![sherlock3_sig.jpg|689x459](${original})`;
    return [
      [
        `\n    ${a}<img src="${optimized}" alt="sherlock3_sig" width="689" height="459" class="d-lazyload" srcset="${srcset}">${meta}</a>\n    `,
        markdown,
      ],
      [`${a}<img src="${optimized}" alt="sherlock3_sig" width="689" height="459" class="d-lazyload" srcset="${srcset}"></a>`, markdown],
      [
        `\n    ${a}<img src="${optimized}" data-base62-sha1="1frsimI7TOtFJyD2LLyKSHM8JWe" alt="sherlock3_sig" width="689" height="459" class="d-lazyload" srcset="${srcset}">${meta}</a>\n    `,
        "![sherlock3_sig.jpg|689x459](upload://1frsimI7TOtFJyD2LLyKSHM8JWe.jpeg)",
      ],
    ];
  })(),
  "strips base64 image URLs": [['<img src="data:image/jpeg;base64,/9j/4AAQSkZJRgABAgAAZABkAAD" />', "[image]"]],
};

test.describe("rich editor to-markdown images", () => {
  for (const [name, cases] of Object.entries(TO_MARKDOWN)) {
    test(name, async ({ page }) => {
      await openEditorPage(page);
      for (const [html, markdown] of cases) {
        const result = await page.evaluate(async (html) => {
          const toMarkdown = (await import("discourse/rich-editor/to-markdown")).default;
          return toMarkdown(html);
        }, html);
        expect(result, `markdown for ${JSON.stringify(html)}`).toBe(markdown);
      }
    });
  }
});

// An upload through the composer in rich mode (lib/uppy/composer-upload.js
// with ProsemirrorPlaceholderHandler): the placeholder is a node of the
// editor's document while the file goes, then the upload's markdown.
test.describe("uploads in the rich editor", () => {
  test("an image uploaded in rich mode replaces its placeholder", async ({ page, context, session }) => {
    const writer = member();
    sql("UPDATE user_options SET composition_mode = 0 WHERE user_id = :'p1'", writer.id);
    const user0 = await session("user0");
    const t = await topic(user0, label("Rich upload topic"), label("A topic a rich upload replies to"));
    await login(context, writer.name);
    await page.goto(t.url);
    await page.locator("#topic-footer-buttons .create").click();
    await expect(page.locator("#reply-control.open")).toBeVisible();
    await page.locator("#reply-control .composer-toggle-switch").click();
    const rich = page.locator("#reply-control .ProseMirror.d-editor-input");
    await expect(rich).toBeFocused();

    // Held until the placeholder has been seen.
    let release;
    const held = new Promise((resolve) => (release = resolve));
    await page.route("**/uploads.json*", async (route) => {
      await held;
      await route.continue();
    });

    // A 1x1 PNG.
    const png = Buffer.from(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
      "base64"
    );
    await page.locator("#file-uploader").setInputFiles({ name: "rich-upload.png", mimeType: "image/png", buffer: png });
    const placeholder = rich.locator(".composer-image-node.upload-placeholder.--image");
    await expect(placeholder).toHaveCount(1);
    await expect(placeholder.locator("img")).toHaveAttribute("alt", /rich-upload\.png/);
    // A placeholder is not part of the markdown.
    expect(await page.locator("#reply-control textarea.d-editor-input").inputValue()).toBe("");
    release();

    await expect(placeholder).toHaveCount(0);
    const image = rich.locator(".composer-image-node img");
    await expect(image).toHaveAttribute("alt", "rich-upload");
    await expect(image).toHaveAttribute("src", /\/uploads\//);
    await expect(page.locator("#reply-control textarea.d-editor-input")).toHaveValue(
      /^!\[rich-upload\|1x1\]\(upload:\/\/[a-zA-Z0-9]+\.png\)$/
    );
  });
});
