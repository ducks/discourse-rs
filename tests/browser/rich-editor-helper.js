// The rich editor's specs share this (as the reference's
// tests/helpers/rich-editor-helper.gjs): a page with the editor's modules
// loaded through the loader's import map and the markdown engine
// (markdown.wasm, configured with the site's settings as the composer's
// preview is), and testMarkdown / setupRichEditor, which open an editor
// over some markdown with every default extension and check the HTML it
// shows and the markdown an edit serializes.
//
// The editor lives in the page; window.richSpec holds it:
//   richSpec.setup(markdown, opts) -> { html, value }
//   richSpec.editor            the createEditor() result (view, ...)
//   richSpec.value             the markdown of the last edit
//   richSpec.html()            the editor's HTML less ProseMirror's own
// Specs drive it with page.evaluate or Playwright's input on
// "#rich-spec-host .ProseMirror".
const { expect, asset } = require("./helpers");

// The reference's test site settings the extensions read: Discourse's
// defaults (config/site_settings.yml).
const SITE_SETTINGS = {
  default_code_lang: "auto",
  here_mention: "here",
  unicode_usernames: false,
  max_image_width: 690,
  max_image_height: 500,
  simultaneous_uploads: 5,
  enable_emoji_shortcuts: true,
  authorized_extensions: "jpg|jpeg|png|gif|heic|heif|webp|avif|svg",
  authorized_extensions_for_staff: "",
  avatar_sizes: "24|48|72|96|144|288",
  max_oneboxes_per_post: 50,
};

// The English strings of the keys the extensions look up (client.en.yml).
const LABELS = {
  cancel: "Cancel",
  "composer.table.default_heading": "Column %{number}",
  "composer.table.add_row": "Add row",
  "composer.table.add_column": "Add column",
  "composer.table.insert_row_above": "Insert above",
  "composer.table.insert_header_above": "Insert header above",
  "composer.table.insert_row_below": "Insert below",
  "composer.table.insert_column_left": "Insert left",
  "composer.table.insert_column_right": "Insert right",
  "composer.table.duplicate_row": "Duplicate row",
  "composer.table.duplicate_column": "Duplicate column",
  "composer.table.move_row_up": "Move row up",
  "composer.table.move_row_down": "Move row down",
  "composer.table.move_column_left": "Move column left",
  "composer.table.move_column_right": "Move column right",
  "composer.table.clear_contents": "Clear contents",
  "composer.table.delete_row": "Delete row",
  "composer.table.delete_column": "Delete column",
  "composer.table.delete_table": "Delete table",
  "composer.table.align_left": "Align left",
  "composer.table.align_center": "Align center",
  "composer.table.align_right": "Align right",
  "composer.table.row_actions": "Row %{number} actions",
  "composer.table.column_actions": "Column %{number} actions",
  "composer.table.row_inserted": "Row inserted",
  "composer.table.header_inserted": "Header row inserted",
  "composer.table.column_inserted": "Column inserted",
  "composer.table.row_duplicated": "Row duplicated",
  "composer.table.column_duplicated": "Column duplicated",
  "composer.table.row_moved_up": "Row moved up",
  "composer.table.row_moved_down": "Row moved down",
  "composer.table.column_moved_left": "Column moved left",
  "composer.table.column_moved_right": "Column moved right",
  "composer.table.contents_cleared": "Cell contents cleared",
  "composer.table.row_deleted": "Row deleted",
  "composer.table.column_deleted": "Column deleted",
  "composer.table.table_deleted": "Table deleted",
  "composer.table.alignment_left": "left",
  "composer.table.alignment_center": "center",
  "composer.table.alignment_right": "right",
  "composer.table.alignment_changed": "Column alignment changed to %{alignment}",
  "composer.table.rows_added.one": "%{count} row added",
  "composer.table.rows_added.other": "%{count} rows added",
  "composer.table.columns_added.one": "%{count} column added",
  "composer.table.columns_added.other": "%{count} columns added",
  "composer.table.rows_removed.one": "%{count} row removed",
  "composer.table.rows_removed.other": "%{count} rows removed",
  "composer.table.columns_removed.one": "%{count} column removed",
  "composer.table.columns_removed.other": "%{count} columns removed",
  "composer.image": "image",
  "composer.heading_level_small_text": "Small text",
  "upload_selector.default_image_alt_text": "image",
  "composer.link_toolbar.edit": "Edit link",
  "composer.link_toolbar.copy": "Copy link",
  "composer.link_toolbar.remove": "Remove link",
  "composer.link_toolbar.visit": "Open link in external tab",
  "composer.link_toolbar.link_copied": "Link copied!",
  "composer.link_toolbar.load_preview": "Load preview",
  "composer.link_toolbar.preview_failed": "Could not load preview",
  "composer.onebox_toolbar.copy": "Copy link",
  "composer.onebox_toolbar.remove_preview": "Remove preview",
  "composer.onebox_toolbar.visit": "Open link",
  "composer.onebox_toolbar.link_copied": "Link copied!",
  "composer.image_alt_text.aria_label": "Alt text for image",
  "composer.image_alt_text.title": "Add image description",
  "composer.image_toolbar.alt_text": "Add image description",
  "composer.image_toolbar.zoom_out": "Decrease image size",
  "composer.image_toolbar.zoom_in": "Increase image size",
  "composer.image_toolbar.remove": "Remove image",
  "composer.image_toolbar.add_to_grid": "Add to grid",
  "composer.image_toolbar.move_outside_grid": "Move outside grid",
  "composer.grid_mode_grid": "Grid",
  "composer.grid_mode_carousel": "Carousel",
  "composer.grid_mode_title": "Set image gallery style to %{mode}",
  "composer.remove_grid": "Remove grid",
  play_video: "Play video",
  cannot_render_video: "This video cannot be rendered because your browser does not support the codec.",
  invalid_video_url: "This video cannot be played because the URL is invalid or unavailable.",
  "composer.wrap_text": "Wrap content",
  "composer.wrap_modal.title": "Edit Wrap",
  "composer.wrap_modal.name_label": "Wrap name",
  "composer.wrap_modal.attributes_label": "Attributes",
  "composer.wrap_modal.add_attribute": "Add attribute",
  "composer.wrap_modal.attribute_key": "Key",
  "composer.wrap_modal.attribute_value": "Value",
  "composer.wrap_modal.remove_attribute": "Remove attribute",
  "composer.wrap_modal.apply": "Apply",
  "composer.wrap_modal.unwrap": "Unwrap",
  "composer.wrap_modal.no_attributes": "No attributes added yet.",
  "form_kit.errors.required": "Required",
  "composer.spoiler_text": "This text will be blurred",
  "discourse_math.edit_math": "Edit math",
  "discourse_local_dates.edit_date": "Edit %{date}",
  "discourse_policy.title": "Policy",
  "discourse_policy.builder.edit": "Edit policy settings",
  "discourse_policy.builder.groups.label": "Groups",
  "discourse_policy.builder.version.label": "Version",
  "discourse_policy.builder.renew.label": "Renew",
  "discourse_policy.builder.renew-start.label": "Renew Start",
  "discourse_policy.builder.reminder.label": "Reminder",
  "discourse_policy.builder.accept.label": "Accept",
  "discourse_policy.builder.revoke.label": "Revoke",
  "discourse_policy.builder.add-users-to-group.label": "Add users to group",
  "discourse_policy.builder.private.label": "Private?",
  "poll.voters.one": "voter",
  "poll.voters.other": "voters",
  "poll.dynamic.enabled_hint": "Options can be added or removed.",
  "poll.multiple.help.x_options.one": "Choose <strong>%{count} option</strong>.",
  "poll.multiple.help.x_options.other": "Choose <strong>%{count} options</strong>.",
  "poll.multiple.help.between_min_and_max_options": "Choose between <strong>%{min} and %{max} options</strong>.",
  "poll.multiple.help.at_least_min_options.one": "Choose at least <strong>%{count} option</strong>.",
  "poll.multiple.help.at_least_min_options.other": "Choose at least <strong>%{count} options</strong>.",
  "poll.multiple.help.up_to_max_options.one": "Choose up to <strong>%{count} option</strong>.",
  "poll.multiple.help.up_to_max_options.other": "Choose up to <strong>%{count} options</strong>.",
  "poll.automatic_close.closes_in": "Closes in <strong>%{timeLeft}</strong>.",
  "poll.automatic_close.age": "Closed <strong>%{age}</strong>",
  "poll.results.vote.title": "Results will be shown on <strong>vote</strong>.",
  "poll.results.closed.title": "Results will be shown once <strong>closed</strong>.",
  "poll.results.staff.title": "Results are only shown to <strong>staff</strong> members.",
  "poll.public.title": "Votes are <strong>public</strong>.",
  "poll.ui_builder.edit": "Edit poll",
  "poll.ui_builder.poll_title.placeholder": "Poll title",
  // the toolbar's example texts
  "composer.bold_text": "strong text",
  "composer.italic_text": "emphasized text",
  "composer.blockquote_text": "Blockquote",
  "composer.heading_text": "Heading",
  "composer.code_title": "Preformatted text",
  "composer.code_text": "indent preformatted text by 4 spaces",
  "composer.paste_code_text": "type or paste code here",
  "composer.list_item": "List item",
};

// In the page: the engine, the context and richSpec.
function harness() {
  const handlers = {};
  // The page's requests in flight, for settledRequests.
  let pending = 0;
  const pageFetch = window.fetch;
  window.fetch = function () {
    pending++;
    return pageFetch.apply(this, arguments).finally(() => pending--);
  };
  window.richSpec = {
    engine: null,
    editor: null,
    value: null,
    // composer.js's richEvents
    appEvents: {
      on(name, fn) {
        (handlers[name] = handlers[name] || []).push(fn);
      },
      off(name, fn) {
        handlers[name] = (handlers[name] || []).filter((h) => h !== fn);
      },
      trigger(name, ...args) {
        (handlers[name] || []).slice().forEach((fn) => fn(...args));
      },
    },

    clean(html) {
      return html
        .replace(' class="ProseMirror-selectednode"', "")
        .replace('<img class="ProseMirror-separator" alt="">', "")
        .replace('<br class="ProseMirror-trailingBreak">', "")
        .replace(' class=""', "")
        .replace(/<p>(<br class="ProseMirror-trailingBreak">)?<\/p>$/, "")
        .replace('<span class="fake-cursor ProseMirror-widget" contenteditable="false"></span>', "");
    },

    html() {
      return this.clean(this.editor.view.dom.innerHTML);
    },

    // rich-editor-helper.gjs and @ember/test-helpers, for the bodies.
    qunitHelpers() {
      const spec = this;
      const settled = () => new Promise((resolve) => setTimeout(resolve, 100));
      // @ember/test-helpers' settled where a test waits on the server
      // (oneboxes.js' queue starts a request 150ms after it is asked): no
      // request in flight across two looks 200ms apart.
      const settledRequests = async () => {
        let quiet = 0;
        for (let i = 0; i < 25 && quiet < 2; i++) {
          await new Promise((resolve) => setTimeout(resolve, 200));
          quiet = pending === 0 ? quiet + 1 : 0;
        }
      };
      // opts.onSetup gets the editor's TextManipulation, as
      // ProsemirrorEditor's @onSetup does.
      const setupRichEditor = async (assert, markdown, opts) => {
        const result = await spec.setup(markdown, opts);
        if (opts && opts.onSetup) {
          opts.onSetup(spec.editor.textManipulation);
        }
        const editor = {
          get view() {
            return spec.editor.view;
          },
          get value() {
            return spec.value;
          },
        };
        return [editor, result.html];
      };
      const testMarkdown = async (assert, markdown, expectedHtml, expectedMarkdown, opts) => {
        const [editor, html] = await setupRichEditor(assert, markdown, opts);
        if (typeof expectedHtml === "function") {
          expectedHtml(assert);
        } else {
          assert.strictEqual(html, expectedHtml, `HTML should match for "${markdown}"`);
        }
        assert.strictEqual(editor.value, expectedMarkdown, `Markdown should match for "${markdown}"`);
      };
      const find = (selector) => document.querySelector(selector);
      // @ember/test-helpers fireEvent's simulated coordinates: just inside
      // the element's top left corner.
      const click = async (selector) => {
        const el = typeof selector === "string" ? find(selector) : selector;
        const rect = el.getBoundingClientRect();
        const init = {
          bubbles: true,
          cancelable: true,
          view: window,
          button: 0,
          clientX: rect.left + 1,
          clientY: rect.top + 1,
          screenX: rect.left + 6,
          screenY: rect.top + 96,
        };
        el.dispatchEvent(new MouseEvent("mousedown", init));
        el.dispatchEvent(new MouseEvent("mouseup", init));
        el.dispatchEvent(new MouseEvent("click", init));
        await settled();
      };
      const triggerKeyEvent = async (selector, type, key, modifiers) => {
        const el = typeof selector === "string" ? find(selector) : selector;
        el.dispatchEvent(
          new KeyboardEvent(type, Object.assign({ key, bubbles: true, cancelable: true }, modifiers || {}))
        );
        await settled();
      };
      return { settled, settledRequests, setupRichEditor, testMarkdown, find, click, triggerKeyEvent };
    },

    // What the reference's QUnit bodies import (inPage).
    async modules() {
      const editor = await window.Discourse.richEditor.load();
      const engine = await import("discourse/rich-editor/engine");
      engine.setEngine(this.engine);
      const tada = this.engine.emoji("tada");
      return {
        editor,
        engine,
        richSpec: this,
        toMarkdown: (await import("discourse/rich-editor/to-markdown")).default,
        wordPaste: await import("discourse/rich-editor/extensions/word-paste"),
        codeBlock: await import("discourse/rich-editor/extensions/code-block"),
        extensions: await import("discourse/rich-editor/extensions"),
        pmState: await import("prosemirror-state"),
        pmModel: await import("prosemirror-model"),
        pmHistory: await import("prosemirror-history"),
        tm: await this.tmHelpers(),
        v: tada ? tada.url.split("?v=")[1] : "",
        helpers: this.qunitHelpers(),
      };
    },

    // text-manipulation-test.gjs' helpers: setupEditor (a ProsemirrorEditor
    // over "", its TextManipulation as state.textManipulation), setContent,
    // selectAll, placeCursor, countNodes and getMarkdown.
    async tmHelpers() {
      const spec = this;
      const { TextSelection } = await import("prosemirror-state");
      return {
        async setupEditor() {
          const editor = await spec.create("", {});
          return { textManipulation: editor.textManipulation, value: "" };
        },
        setContent(state, markdown) {
          const { view, convertFromMarkdown } = state.textManipulation;
          const doc = convertFromMarkdown(markdown);
          view.dispatch(view.state.tr.replaceWith(0, view.state.doc.content.size, doc.content));
        },
        selectAll(state) {
          const { view } = state.textManipulation;
          const { doc } = view.state;
          const from = TextSelection.atStart(doc).from;
          const to = TextSelection.atEnd(doc).to;
          view.dispatch(view.state.tr.setSelection(TextSelection.create(doc, from, to)));
        },
        placeCursor(state) {
          const { view } = state.textManipulation;
          const pos = TextSelection.atStart(view.state.doc).from;
          view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, pos)));
        },
        countNodes(state, typeName) {
          let count = 0;
          state.textManipulation.view.state.doc.descendants((node) => {
            if (node.type.name === typeName) {
              count++;
            }
          });
          return count;
        },
        getMarkdown(state) {
          const { view, convertToMarkdown } = state.textManipulation;
          return convertToMarkdown(view.state.doc);
        },
      };
    },

    destroy() {
      if (this.editor) {
        this.editor.destroy();
        this.editor = null;
      }
      const host = document.getElementById("rich-spec-host");
      if (host) {
        host.remove();
      }
    },

    async create(markdown, opts) {
      const editorModule = await window.Discourse.richEditor.load();
      const extensionsModule = await import("discourse/rich-editor/extensions");
      const plugins = opts.plugins || window.richSpecPlugins || [];
      editorModule.registerPluginExtensions(plugins);
      // composer.js's richEditorPlugins
      if (plugins.includes("local-dates") && window.Discourse.localDates) {
        await window.Discourse.localDates.loadMoment();
      }
      this.destroy();
      const host = document.createElement("div");
      host.id = "rich-spec-host";
      host.className = "d-editor-textarea-wrapper";
      // The composer's room: floating toolbars are clipped to the editor.
      if (!document.getElementById("rich-spec-style")) {
        const style = document.createElement("style");
        style.id = "rich-spec-style";
        style.textContent = "#rich-spec-host .ProseMirror { min-height: 200px; }";
        document.head.appendChild(style);
      }
      document.body.appendChild(host);
      this.value = null;
      const labels = Object.assign({}, window.richSpecLabels, opts.labels);
      this.editor = editorModule.createEditor({
        container: host,
        value: markdown,
        engine: this.engine,
        extensions: opts.defaultsOnly ? [] : extensionsModule.getExtensions(),
        className: "d-editor-input",
        disabled: opts.disabled,
        // d-editor's keymap: Tab and Shift+Tab indent (nest list items).
        keymap: {
          tab: () => !this.editor.textManipulation.indentSelection("right"),
          "shift+tab": () => !this.editor.textManipulation.indentSelection("left"),
        },
        context: Object.assign(
          {
            siteSettings: Object.assign({}, window.richSpecSiteSettings, opts.siteSettings),
            // composer.js's richEditorContext i18n
            i18n: (key, params) => {
              if (params && params.count !== undefined) {
                const count = Number(params.count);
                const form = count === 0 && key + ".zero" in labels ? "zero" : count === 1 ? "one" : "other";
                if (key + "." + form in labels) {
                  key = key + "." + form;
                }
              }
              let text = key in labels ? labels[key] : key;
              Object.keys(params || {}).forEach((name) => {
                text = text.split("%{" + name + "}").join(String(params[name]));
              });
              return text;
            },
            markdownOptions: opts.markdownOptions || {},
            topicId: opts.topicId,
            currentUser: { staff: false },
            site: { hashtag_configurations: { "topic-composer": ["category", "tag"] } },
            capabilities: { viewport: { sm: true } },
            appEvents: this.appEvents,
            // The modal service, as the reference's tests read it: the
            // last modal shown (activeModal.opts.model); opts.realModals
            // shows core's own, as composer.js does.
            modal: {
              show: (name, modalOpts) => {
                this.activeModal = { name, opts: modalOpts };
                if (opts.realModals && name === "wrap-attributes") {
                  editorModule.showWrapAttributesModal(modalOpts.model, this.editor.params.getContext().i18n);
                }
              },
            },
          },
          opts.context
        ),
        onChange: (value) => {
          this.value = value;
        },
        onUnsupported: (e) => {
          this.unsupported = e;
        },
      });
      return this.editor;
    },

    // setupRichEditor: the editor over the markdown (re-created twice more
    // with multiToggle, as toggling to markdown and back does), then a
    // paragraph inserted and deleted again so the value is what an edit
    // serializes.
    async setup(markdown, opts) {
      opts = opts || {};
      await this.create(markdown, opts);
      if (opts.multiToggle) {
        await this.create(markdown, opts);
        await this.create(markdown, opts);
      }
      const view = this.editor.view;
      const tr = view.state.tr;
      tr.insert(tr.doc.content.size, view.state.schema.node("paragraph", null, view.state.schema.text("X")));
      tr.delete(tr.doc.content.size - 3, tr.doc.content.size);
      view.dispatch(tr);
      // The extensions' async work (hljs, lookups) settles first.
      await new Promise((resolve) => setTimeout(resolve, opts.settle === undefined ? 50 : opts.settle));
      return { html: this.html(), value: this.value };
    },
  };
}

// /faq with the loader and the engine, and richSpec. A static page: the
// host is appended to the body, and on a topic list scrolling to it
// brings the list's load-more sentinel near the window, so the next page's
// rows push the editor (and a floating toolbar's reference) away.
// opts.plugins: the plugins whose extensions the editor registers
// (rich-editor/plugins.js names), as the composer's context lists them;
// opts.render: the site's render settings changed for the engine (a
// plugin the test site has off, say { math: true }).
async function openEditorPage(page, opts = {}) {
  await page.goto("/faq");
  await page.addScriptTag({ content: asset("js/rich-editor.js") });
  await page.addScriptTag({ content: `(${harness.toString()})()` });
  await page.evaluate(
    async ({ siteSettings, labels, plugins, render }) => {
      window.richSpecSiteSettings = siteSettings;
      window.richSpecLabels = labels;
      window.richSpecPlugins = plugins;
      const editorModule = await window.Discourse.richEditor.load();
      const [wasm, settings] = await Promise.all([
        WebAssembly.instantiateStreaming(fetch("/assets/markdown.wasm"), {}),
        fetch("/assets/markdown-settings.json").then((r) => r.text()),
      ]);
      const configured = JSON.stringify(Object.assign(JSON.parse(settings), render));
      const exports = wasm.instance.exports;
      const call = (fn, text) => {
        const bytes = new TextEncoder().encode(text);
        const ptr = exports.alloc(bytes.length);
        new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
        const status = fn(ptr, bytes.length);
        exports.dealloc(ptr, bytes.length);
        const out = new TextDecoder().decode(
          new Uint8Array(exports.memory.buffer, exports.output_ptr(), exports.output_len())
        );
        if (status !== 0) {
          throw new Error(out);
        }
        return out;
      };
      call(exports.configure, configured);
      window.richSpec.engine = editorModule.wasmEngine(exports, call);
    },
    { siteSettings: SITE_SETTINGS, labels: LABELS, plugins: opts.plugins || [], render: opts.render || {} }
  );
}

// setupRichEditor: { html, value } of the editor opened over markdown.
function setupRichEditor(page, markdown, opts = {}) {
  return page.evaluate(({ markdown, opts }) => window.richSpec.setup(markdown, opts), { markdown, opts });
}

// testMarkdown: the HTML (or `expectedHtml(page)`'s own checks, awaited)
// and the markdown an edit writes.
async function testMarkdown(page, markdown, expectedHtml, expectedMarkdown, opts = {}) {
  const result = await setupRichEditor(page, markdown, opts);
  if (typeof expectedHtml === "function") {
    await expectedHtml(page, result);
  } else {
    expect(result.html, `HTML should match for ${JSON.stringify(markdown)}`).toBe(expectedHtml);
  }
  expect(result.value, `Markdown should match for ${JSON.stringify(markdown)}`).toBe(expectedMarkdown);
  return result;
}

// The current markdown of the open editor, as the last edit wrote it.
function editorValue(page) {
  return page.evaluate(() => window.richSpec.value);
}

// pretender's response(): a route answering JSON.
async function respond(page, pattern, body) {
  await page.route(pattern, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(body) })
  );
}


// A reference QUnit test's body run in the page, ported as written:
// `body(assert, modules)` with a QUnit-like assert (strictEqual, deepEqual,
// true, false, ok, notOk, equal) and the modules the bodies import
// (toMarkdown, the word-paste and code-block exports, the editor and
// prosemirror modules, `v` the emoji images' ?v=). Resolves to the
// failures, which the spec expects to be none.
async function inPage(page, body, args) {
  return page.evaluate(`(async (args) => {
    const failures = [];
    const show = (value) => {
      try {
        return JSON.stringify(value);
      } catch (e) {
        return String(value);
      }
    };
    const check = (pass, message, detail) => {
      if (!pass) {
        failures.push((message || "assertion") + (detail ? ": " + detail : ""));
      }
    };
    const assert = {
      strictEqual: (actual, expected, message) =>
        check(actual === expected, message, "got " + show(actual) + ", expected " + show(expected)),
      equal: (actual, expected, message) =>
        check(actual == expected, message, "got " + show(actual) + ", expected " + show(expected)),
      notStrictEqual: (actual, expected, message) => check(actual !== expected, message, "got " + show(actual)),
      deepEqual: (actual, expected, message) =>
        check(show(actual) === show(expected), message, "got " + show(actual) + ", expected " + show(expected)),
      true: (value, message) => check(value === true, message, "got " + show(value)),
      false: (value, message) => check(value === false, message, "got " + show(value)),
      ok: (value, message) => check(!!value, message, "got " + show(value)),
      notOk: (value, message) => check(!value, message, "got " + show(value)),
    };
    const normalize = (text) => (text || "").replace(/\\s+/g, " ").trim();
    // qunit-dom's assert.dom, as far as the bodies use it
    assert.dom = (selector) => {
      const all = () => Array.from(document.querySelectorAll(selector));
      const el = () => document.querySelector(selector);
      const one = (name, message, fn) => {
        const found = el();
        check(!!found, message, selector + " not found for " + name);
        if (found) {
          fn(found);
        }
      };
      const chain = {
        exists: (opts, message) => {
          if (typeof opts === "string") {
            message = opts;
            opts = null;
          }
          const count = all().length;
          check(opts && opts.count !== undefined ? count === opts.count : count > 0, message, selector + ": " + count);
          return chain;
        },
        doesNotExist: (message) => {
          check(all().length === 0, message, selector + " exists");
          return chain;
        },
        hasClass: (name, message) => {
          one("hasClass", message, (e) => check(e.classList.contains(name), message, selector + " lacks ." + name));
          return chain;
        },
        doesNotHaveClass: (name, message) => {
          one("doesNotHaveClass", message, (e) => check(!e.classList.contains(name), message, selector + " has ." + name));
          return chain;
        },
        hasAttribute: (name, value, message) => {
          one("hasAttribute", message, (e) => {
            const actual = e.getAttribute(name);
            const pass =
              value === undefined
                ? e.hasAttribute(name)
                : value instanceof RegExp
                  ? actual !== null && value.test(actual)
                  : actual === String(value);
            check(pass, message, selector + "[" + name + "] is " + show(actual) + ", expected " + String(value));
          });
          return chain;
        },
        doesNotHaveAttribute: (name, message) => {
          one("doesNotHaveAttribute", message, (e) => check(!e.hasAttribute(name), message, selector + " has " + name));
          return chain;
        },
        hasText: (text, message) => {
          one("hasText", message, (e) =>
            check(normalize(e.textContent) === normalize(text), message, selector + " text " + show(normalize(e.textContent)))
          );
          return chain;
        },
      };
      return chain;
    };
    const modules = await window.richSpec.modules();
    await (${body.toString()})(assert, modules, args);
    return failures;
  })(${JSON.stringify(args === undefined ? null : args)})`);
}

// A Playwright test running a reference QUnit body (inPage) on the editor
// page.
function qunitTest(test, name, body, opts = {}) {
  test(name, async ({ page }) => {
    if (opts.before) {
      await opts.before(page);
    }
    await openEditorPage(page, opts);
    expect(await inPage(page, body, opts.args)).toEqual([]);
  });
}

module.exports = {
  SITE_SETTINGS,
  LABELS,
  openEditorPage,
  setupRichEditor,
  testMarkdown,
  editorValue,
  respond,
  inPage,
  qunitTest,
};
