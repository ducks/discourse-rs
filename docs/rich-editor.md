# The composer's rich editor

Discourse's composer has two editors: the markdown textarea and the rich
editor, ProseMirror under `frontend/discourse/app/static/prosemirror/`
(ROADMAP 196). The port's rich editor is that code as plain ES modules in
`static/js/rich-editor/`, ProseMirror vendored under
`static/vendor/prosemirror/` (static/vendor/README), loaded through an
import map with no bundler or build step, and parsing markdown with the
port's own markdown crate (`crates/markdown`) so the editor and cooked posts
share one parser.

The rich editor is the default composition mode, as Discourse ships it
(`default_composition_mode` 1, new members' `composition_mode`): the
composer and the profile's About me open in the member's
`composition_mode` (user_options, 1 is rich), and the toggle and Ctrl+M
switch and save it. It became the default once the round-trip tests
passed (196r5, below); what it still refuses opens in markdown with the
reference's alert.

## Files

| File | From the reference |
|---|---|
| `static/js/rich-editor.js` | `lib/load-rich-editor.js`: the import map and `Discourse.richEditor.load()` |
| `rich-editor/editor.js` | `components/prosemirror-editor.gts`, `core/plugin.js`, `core/commands.js` |
| `rich-editor/extensions.js` | `lib/composer/rich-editor-extensions.ts`, `extensions/register-default.ts` and the small core extensions: code (codemark), heading, strikethrough, strong, underline, trailing paragraph, ordered and bullet lists, hard break, placeholder |
| `rich-editor/extensions/*.js` | `extensions/`: emoji, mention, hashtag, link, link-toolbar, quote, code-block, html-inline, html-block, trailing-inline-space, markdown-paste, word-paste, table, image, upload-placeholder, grid, override-drag-ghost, wrap, onebox, onebox-toolbar, preview-source, preview-toolbar |
| `rich-editor/text-manipulation.js` | `lib/text-manipulation.ts`: the TextManipulation the composer's toolbar acts through (below) |
| `static/js/editor-toolbar.js` | `lib/composer/toolbar.ts`'s Toolbar, d-editor's newToolbarEvent and keymap, and the toolbar half of `lib/textarea-text-manipulation.ts`, shipped in front of composer.js |
| `static/js/upsert-hyperlink.js` | `components/modal/upsert-hyperlink.gjs`, shipped in front of composer.js |
| `rich-editor/plugins.js` | each bundled plugin's `api.registerRichEditorExtension`: the extensions of the plugins the composer's context lists, registered once, after the defaults, in the plugins' load order |
| `rich-editor/plugins/*.js` | the plugins' `lib/rich-editor-extension.js`: checklist, discourse-details (details), discourse-graphviz (graphviz, a preview block), discourse-local-dates (local-dates, with `local-date-node-view.js`), discourse-math (math), discourse-policy (policy, from its initializer), footnote, poll (with `components/poll-node-view.gjs` and `poll-info.gjs` in `poll-node-view.js`), spoiler-alert (spoiler) |
| `rich-editor/wrap-utils.js`, `rich-editor/wrap-modal.js` | `lib/wrap-utils.js`, `components/wrap-attributes-modal.gjs` |
| `rich-editor/node-views/*.js` | `components/`: image (with media and video dispatch), video, grid, upload placeholder, image alt text input and wrap, and `components/composer/preview-node-view.gjs` with `lib/composer/preview-block.js` (preview), as plain DOM node views |
| `rich-editor/table/*.js` | `lib/table/` |
| `rich-editor/engine.js` | `lib/markdown-it.js`: the markdown engine the extensions ask (below) |
| `rich-editor/toolbar.js` | `lib/composer/toolbar.ts` ToolbarBase, `components/composer/toolbar-buttons.gjs`, `lib/roving-button-bar.js`, iconHTML and a success toast, for the extensions' floating toolbars |
| `rich-editor/schema.js` | `core/schema.js` |
| `rich-editor/parser.js` | `core/parser.js`, plus the node tree mapping below |
| `rich-editor/serializer.js` | `core/serializer.js`, `lib/expel-boundary-punctuation.ts` |
| `rich-editor/keymap.js` | `core/keymap.js` |
| `rich-editor/inputrules.js` | `core/inputrules.js` |
| `rich-editor/plugin-utils.js` | `lib/plugin-utils.js`, with lib/text.js's buildBBCodeAttrs and serializeBBCodeAttr and bbcode-block.js's parseBBCodeTag |
| `rich-editor/to-markdown.js` | `to-markdown.js` |
| `rich-editor/markdown-it-stub.js` | none: what `markdown-it` resolves to |
| `static/css/rich-editor.css` | `common/rich-editor/rich-editor.scss` (core rules and the everyday extensions'), `common/components/composer-toggle-switch.scss` |
| `static/css/rich-editor-table.css` | `common/rich-editor/composer-table.scss` |
| `static/css/rich-editor-media.css` | `rich-editor.scss`'s image, upload placeholder, video and grid rules, `composer-image-gallery.scss` |
| `src/composer/rich_editor.rs` | what getContext gives the extensions: site settings, strings, the hashtag order |
| `src/routes/composer_mentions.rs` | `ComposerController#mentions` (GET /composer/mentions), which the mention extension asks |

Extensions keep the reference's shape (nodeSpec, markSpec, parse,
serializeNode, serializeMark, inputRules, keymap, plugins, nodeViews,
commands, state, transformParsedHTML, each a value or a function of the
plugin params), so the plugins' extensions (196r4) ported one by one without
changing the core.
Node views are classes (extractNodeViews builds them with `new`) or
functions returning one; the Glimmer component descriptors have no
counterpart, so the components are plain DOM node views.

## Loading

Every file is a manifest entry (`static/manifest.json`), served by the
`/assets/{*path}` fallback: `prosemirror/<package>.js`,
`prosemirror/prosemirror-codemark/<file>.js`, `rich-editor/<module>.js` and
`rich-editor.js`. No Rust route is needed.

`rich-editor.js` is a classic script. When it runs it appends
`<script type="importmap" data-rich-editor>` mapping:

- each package's bare name (`prosemirror-model`) to its asset,
- prosemirror-codemark's extensionless relative imports, by url
  (`<base>/assets/prosemirror/prosemirror-codemark/plugin` to `plugin.js`),
- `discourse/rich-editor/<module>` to `rich-editor/<module>.js`,
- `markdown-it` to `rich-editor/markdown-it-stub.js`.

prosemirror-markdown imports markdown-it and builds a default parser with it
at module load; the stub answers that call and throws if anything parses
through it. The editor never does: it reads `defaultMarkdownParser.tokens`
(the parse specs) and tokenizes through the node tree.

The script tag may carry `data-assets`, a JSON object of asset name to url
with its `?v=` digest, and `data-base-path`; names it lacks load from
`<base>/assets/<name>` (revalidated, not cached for good). The import map has
to be in the document before the page imports any module (lightbox.js and
highlight-syntax.js import theirs), so the tag goes in the head before
those scripts. `Discourse.richEditor.load()` imports
`discourse/rich-editor/editor` the first time and resolves to that module.

## The node tree

The parser takes markdown as a node tree in JSON, which the markdown crate
writes (`render::rich_editor_tree`, `crates/markdown/src/render/tree.rs`)
and its WebAssembly build exports as `rich_editor_tree`. The tree is markdown-it's token stream
nested: a pair of `X_open` / `X_close` tokens is a node of type `X` with
children, any other token a node without. `tokensFromTree`
(`rich-editor/parser.js`) flattens it back into the tokens Discourse's
markdown-it produces for the same source, and prosemirror-markdown's
MarkdownParser runs every extension's parse spec over them unchanged. So
the contract is: for any markdown, the tree flattens to the token stream of

    buildEngine({ siteSettings: { ...siteSettings, enable_markdown_typographer: false } },
                ["onebox", "watched-words", "censored"]).parse(text)

(`static/prosemirror/lib/markdown-it.js` at the vendored commit): the
typographer off, and the onebox, watched words and censored features left
out, with the site's other settings as the preview's.

### Shape

```json
{ "type": "doc", "children": [ <node>, ... ] }
```

A node is an object:

| Field | Type | |
|---|---|---|
| `type` | string, required | the token type, less `_open` / `_close` |
| `tag` | string | `token.tag` |
| `attrs` | array of `[name, value]` strings | `token.attrs`, in order; omitted for none |
| `markup` | string | `token.markup` |
| `info` | string | `token.info` |
| `content` | string | `token.content` |
| `hidden` | boolean | `token.hidden`; true on a tight list item's paragraph |
| `meta` | any JSON | `token.meta`, where a rule sets it (footnote ids, math type) |
| `fields` | object | properties a rule sets on its token besides markdown-it's own, copied onto the open token (the poll's `poll_attrs`) |
| `bare` | boolean | true on a container whose open and close tokens are both `type`, nesting 1 and -1 (bbcode-block's `wrap_bbcode`) |
| `map` | `[startLine, endLine]` | `token.map`, optional |
| `children` | array of nodes | see below |
| `nesting` | `0` | only on a leaf that has `children` |

- A node with `children` and no `nesting` is a container: it flattens to
  `type_open`, its children, `type_close` (with `bare`, `type` twice). The
  close token copies the open token's tag, markup, hidden and block.
- `{"type": "inline", "content": <source>, "children": [...]}` is
  markdown-it's `inline` token: a block's inline content, wherever
  markdown-it has one (paragraphs, headings, table cells), its children the
  inline nodes. Required: a paragraph's children are one `inline` node,
  never text directly.
- A node without `children`, or with `"nesting": 0`, is a single token; with
  `nesting: 0` its children are the token's `children` (an image's alt).
- `block` and `level` are not written: nodes outside an `inline` are block
  tokens, inside it inline ones, and levels follow the nesting.

String fields default to `""`, `hidden` to false, `meta` to null. A tree
that breaks the shape (a node without a type, attrs that are not string
pairs, `inline` inside inline content, a root that is not `doc`) throws
`NodeTreeError`: a bug in the export, never in the post. A token no parse
spec handles throws `UnsupportedTokenError`, which the composer answers as
the reference does (`composer.unsupported_token`, back to markdown).

### Core types (196r1)

| type | tag | attrs | other fields |
|---|---|---|---|
| `paragraph` | `p` | | `hidden` in a tight list |
| `heading` | `h1`..`h6` | | `markup`: the `#`s, or `=` / `-` for setext |
| `blockquote` | `blockquote` | | `markup` `>` |
| `bullet_list` | `ul` | | `markup` `*`, `-` or `+` |
| `ordered_list` | `ol` | `start`, only when not 1 | `markup` `.` or `)` |
| `list_item` | `li` | | `markup` as the list's; `info` the item's number in an ordered list |
| `code_block` | `code` | | `content`, with its last newline |
| `fence` | `code` | | `info`, `markup`, `content` with its last newline |
| `hr` | `hr` | | `markup` |
| `text` | | | `content` |
| `softbreak`, `hardbreak` | `br` | | |
| `em`, `strong` | `em`, `strong` | | `markup` `*`, `_`, `**`, `__` |
| `s` | `s` | | `markup` `~~` |
| `link` | `a` | `href`, then `title` if any | `markup` `linkify` or `autolink` and `info` `auto` for those |
| `image` | `img` | `src`, `alt` (empty), then `title` if any | `nesting: 0`, `content` the alt text, `children` the alt's inline nodes |
| `code_inline` | `code` | | `markup` the backticks, `content` |
| `bbcode_b`, `bbcode_i`, `bbcode_u`, `bbcode_s` | `span` | `class` `bbcode-b` (and so on) | `markup` `[b]` (and so on) |

Discourse's other tokens are written the same way where the crate already
knows the token: html_block, html_inline, html_raw, emoji (with
getEmojiTokenByName's attrs), tables (`table`, `thead`, `tbody`, `tr`, `th`,
`td` with their `text-align` style), `[code]` as `code_inline`, `[url]` and
`[email]` as `link`, `[img]` as `image` (src the content, an empty alt,
no children, as bbcode-inline pushes it).

### The everyday extensions' types (196r2)

| type | tag | attrs | from |
|---|---|---|---|
| `mention` | `span` | `class` `mention`; its child is the `@name` text | mentions.js, as text-post-process runs it |
| `span` | `span` | `class` `hashtag-raw`; its child is the `#slug` text | hashtag-autocomplete.js: the editor's engine looks nothing up, so every hashtag is raw |
| `bbcode` | `aside` | `class` `quote no-group`, `data-username`, `data-display-name`, `data-post`, `data-topic`, `data-full` | quotes.js, then a `bbcode` `blockquote` around the content |
| `quote_header` | `div` | `class` `title`; a `quote_controls` div, then the ` name:` text | quotes.js, for a quote naming a user |
| `bbcode` | `div` | `class` `d-image-grid`, `data-mode` | image-grid.js |

upload-protocol runs as in the client, with no lookup: an `img`'s or an
`a`'s first `upload://` attribute (an image, a link, a linkified or
bbcode url) becomes getURL of `/images/transparent.png` (`/404` for an
image whose alt says `|video` or `|audio`, `/404` for a link), the short
url going to `data-orig-src` or `data-orig-href`. HTML naming an upload is
refused: upload-protocol rewrites its `img` through the xss filter, which
the tree does not port. The engine's options have no topic, so a quote
of another topic names its user rather than linking to the topic, as the
reference's buildEngine does.

### The plugins' types and [wrap] (196r4)

| type | tag | attrs | from |
|---|---|---|---|
| `bbcode` | `details`, then `summary` | `open` (empty) where `[details open]` | discourse-details' rule; the summary's text is a `text` token among the block tokens |
| `wrap_bbcode` (`bare`) | `div` | `class` `spoiler` | spoiler-alert's block rule (bbcode-block's `wrap: "div.spoiler"`) |
| `bbcode_spoiler` | `span` | `class` `spoiler` | spoiler-alert's inline rule, `markup` `[spoiler]` |
| `wrap_bbcode` (`bare`) | `div` | `class` `policy`, each attribute as `data-<key>`, sorted, `version` 1 unless given | discourse-policy's `wrap` |
| `wrap_bbcode` (`bare`) | `div` | `class` `excerpt` | core's `[excerpt]`, which no extension takes in the reference either |
| `wrap` | `div` or `span` | `class` `d-wrap`, applyDataAttributes' `data-*` | d-wrap.js; the inline one holds its content as one `text` token |
| `check` | `span` | `class` `chcklst-box ...`, `data-chk-src` | checklist's core rule, with no children |
| `footnote_ref` | | | `meta` `{id}` for `^[...]`, `{id, subId, label}` for `[^label]` |
| `footnote_block`, `footnote` | | | footnote_tail: the notes after the document, each `footnote` with `meta` `{id, label}`, its blocks, and a `footnote_anchor` (`meta` `{id, subId, label}`) per reference before its last paragraph's close |
| `math_inline`, `math_block` | | | discourse-math; `content` the math, `meta` `{mathType}` (`tex` or `asciimath`) on the inline one |
| `span` | `span` | `class` `discourse-local-date`, its `data-*` | discourse-local-dates; its child the date's text, and a range's spans either side of a `→` text |
| `poll` | `div` | `class` `poll`, `data-poll-*` | the poll rule, `fields` `{poll_attrs}`: `{name: "poll", status: "open", ...}` and every attribute written |
| `poll_container`, `poll_title` | `div` | `class` `poll-container`, `poll-title` | the title's `inline` in `poll_title`; each option's `list_item` with its `data-poll-option-id` |
| `bullet_list` (a number poll's) | `ul` | | addNumberListItems: each item a hidden `paragraph` around a `text` token |
| `poll_info`, `poll_info_counts`, `poll_info_counts_count`, `poll_info_number`, `poll_info_label` | `div`, `span` | their classes | addPollInfo, the label poll_voters_label |

Where a rule writes raw HTML, its node says the tokens the HTML stands
for (`element::JsTree` on a RawHtml node, `JsWrap` on a Holder whose first
and last children are the rule's own tags, `JsFields` for a token's own
properties, `JsBare` for a `wrap_bbcode` Element); the cook's HTML is
unchanged.

discourse-graphviz's `[graphviz]` (196r3) is its rule's `graphviz` token:
`div`, `data-engine` (an engine it knows, else `dot`) and the source
between the tags as `content`, a JsTree on the raw HTML it cooks to.

What no extension takes yet is still refused: HTML naming an upload
(above), and what the crate writes as `html_raw` with no parse spec:
chat's `[chat]` transcripts (chat's extension is not ported). discourse-events' `[event]` and the calendar's
blocks are refused by the crate itself, the plugins not being ported
(ROADMAP 202), which the composer answers as an unsupported token. An
`[excerpt]` is refused where both spoiler-alert and discourse-policy are
on, as in the reference: with one of them its handler declining drops the
token, as the reference's parser does. Text may be split or merged
differently from markdown-it's (the parser merges adjacent text), so the
test is the document a tree gives, not equality with markdown-it's tokens.

tests/browser/rich-editor.spec.js writes the trees for each of the
reference's cases (its `doc(...)` builders): those are the expected shape.

### What the export rebuilds

The crate's tree is not markdown-it's token stream as it stands; tree.rs
rebuilds it from `render::parse`, the cook's parse and core rules up to
emoji:

1. A tight list's paragraphs are spliced away (markdown-it 0.6
   `ListScanner::mark_tight_paragraphs`): each item's text goes back in a
   `paragraph` with `hidden: true`.
2. `InlineRoot` is replaced by its children once inline rules run: each
   block's inline content goes back in an `inline` node, grouped by the
   inline blocks `render::blocks` keeps (`inline_blocks`), its content the
   block's source (trimmed for an ATX heading, as heading.mjs trims).
3. `render::element::Element` keeps only a tag and attrs. Where a rule builds
   one for a JS token (bbcode-inline's `[b]` is `bbcode_b`, a `span` with
   `class="bbcode-b"`), the rule records the token's type and markup in the
   `element::JsToken` node extension; the tag is never guessed from.
4. The tree is the parse, before what only rendering adds: heading anchors
   (`render::anchor`, an empty link pair in JS that gives the editor
   nothing, so left out), the sanitizer, text post-processing for HTML.
   The upload pass is left out too: this port runs upload-protocol together
   with the image and link renderers (images become their HTML), so tree.rs
   applies upload-protocol's client half to the tokens itself (above).
5. The settings: typographer off; onebox, watched words and censored off
   whatever the site's settings say; no topic id and no forced quote link.

## The engine

The reference's extensions ask its markdown-it engine more than a parse
(`lib/markdown-it.js`): `getLinkify()` to find links as the member types or
pastes and in the link toolbar, `getAllowLister()` for the inline HTML it
keeps, and pretty-text's emoji data for images and emoticons. The port's
engine is markdown.wasm configured with the site's settings
(`rich-editor/engine.js` `wasmEngine`, `crates/markdown/src/render/editor.rs`),
set by createEditor's `engine` option:

| engine.js | export | the reference's |
|---|---|---|
| `tree(markdown)` | `rich_editor_tree` | `parse(text)` |
| `linkify.match`, `.test`, `.matchAtStart` | `linkify_match`, `linkify_match_at_start` | linkify-it's, with markdown_linkify_tlds whether linkify is on or not (RenderSettings' `link_matcher_tlds`); offsets in UTF-16 units |
| `allowList.tags`, `.attrAllowed(tag, name, value)` | `html_allow_list`, `html_attr_allowed` | `getAllowList().tagList`, `isAllowListedAttr` or a custom check |
| `emoji(code)` | `emoji_image` | `buildEmojiUrl` and `isCustomEmoji` of a code that exists |
| `emojiTranslations` | `emoji_translations` | pretty-text/emoji/data's `translations` |
| `emojiUnescape(text)` | `emoji_unescape` | lib/text's `emojiUnescape` (custom emoji and the deny list, not lazy) |

`isWhiteSpace` and `isBoundary` are markdown-it's own, written out in
plugin-utils.js.

## Tests

`tests/browser/rich-editor-helper.js` is the reference's
`tests/helpers/rich-editor-helper.gjs`: a page with the loader, the editor's
modules and the engine, `testMarkdown` and `setupRichEditor` (the editor's
HTML less ProseMirror's artifacts, and the markdown an edit serializes,
with every default extension, and the plugins' a page or test names), and
`qunitTest`, which runs a reference QUnit body as written in the page with
a QUnit-like `assert` (and `assert.dom`) and the @ember/test-helpers it
calls (click, at fireEvent's point just inside an element's top left
corner, triggerKeyEvent, settled). The context's `modal` records the last
modal shown as `richSpec.activeModal`, where the reference's tests read the
modal service's. A page may change the engine's render settings (math and
policy are off on the test site). The reference's pretender routes are
Playwright routes.

- `rich-editor.spec.js` ports `prosemirror-markdown-test.js` (the editor
  with prosemirror-markdown's defaults alone), `strikethrough-test.js` and
  `underline-test.js`, three ways: `serializes` (the document built from the
  expected HTML, so the schema, view and serializer), `maps` (from the
  case's node tree, so `tokensFromTree` and the parse specs) and
  `round-trips` (markdown through markdown.wasm's `rich_editor_tree`), with
  bbcode-inline's [b], [i], [code], [url], [email] and [img] round-tripped
  too,
  the import map's urls, typing, undo and the refusals.
- `rich-editor-extensions.spec.js` ports the emoji, mention, hashtag, link,
  quote, code-block, html-inline and html-block tests and
  prosemirror-editor-test.gjs's registry cases, the link toolbar and the
  paste extensions.
- `rich-editor-table.spec.js` ports `table-test.js` and the to-markdown
  table cases; `rich-editor-media.spec.js` `image-test.js`, the grid cases
  of prosemirror-editor-test.gjs, the to-markdown image cases and an upload
  through the composer in rich mode.
- `rich-editor-to-markdown.spec.js` and `rich-editor-word-paste.spec.js` run
  `to-markdown-test.js` (less its table and image cases) and
  `word-paste-test.js` as written.
- `rich-editor-plugins.spec.js` ports the plugins' rich-editor-extension
  tests (checklist, discourse-details, spoiler-alert, poll with its unit
  cases, discourse-math, discourse-local-dates, discourse-policy),
  prosemirror-editor-test.gjs's [wrap] registry case, and from their system
  specs what the QUnit tests leave out: footnote's, checklist's and
  spoiler's typing, prosemirror_wrap_spec.rb (less its toolbar case), and
  the refusals. composer-actions.spec.js runs poll-ui-builder-test's
  editing cases on the builder, and composer.spec.js the plugins in the
  composer: details and a poll opened, Edit poll and Edit Wrap.

- `rich-editor-toolbar.spec.js` (196r3) runs `text-manipulation-test.gjs`,
  the checklist's and footnote's text-manipulation tests,
  `onebox-test.js`, `preview-node-view-test.gjs` and discourse-graphviz's
  rich-editor-extension-test.js as written in the page (the helper's
  `tm` gives text-manipulation-test's setupEditor and friends,
  `settledRequests` waits out the page's requests where the reference's
  settled waits for its ajax, and create-pretender's /onebox and
  /inline-onebox answers are routes), and on the composer the toolbar's
  cases of prosemirror_toolbar_spec.rb, prosemirror_keymap_spec.rb,
  prosemirror_links_spec.rb, prosemirror_onebox_spec.rb,
  prosemirror_onebox_toolbar_spec.rb, composer_spoiler_spec.rb and
  prosemirror_wrap_spec.rb, composer-hyperlink-test.js on the markdown
  editor, and the emoji, upload and options menu buttons in both editors.
  The helper's editor has d-editor's Tab keys, so rich-editor-extensions
  runs code-block-test's "Tab still nests list items".

Cases the port cannot run say why where they would be: onebox-test's
markdown-it case (the node tree's contract) and graphviz's "constrains a
diagram with tall source" (the editor page loads no site stylesheets);
spoiler's `composer:apply-surround` app event cases have no app events
service to send them, and edit_graphviz_in_rich_editor_spec.rb runs on
the editor page, as the test site has graphviz off. The site's highlight.js bundle has languages
the reference's test bundle lacks, so the code block cases list the
bundle's and the HTML block cases set its highlighting aside.

Run them with `scripts/browser-test rich-editor`. The Rust tests in tree.rs
check the trees (the core cases after the editor's text merging, the
extensions' tokens, the upload placeholders and what the overrides leave
out) and editor.rs the engine's answers; tests/composer_mentions.rs covers
GET /composer/mentions.

## In the composer

- `templates/layout.html` puts `rich-editor.js` in the head of every page of
  a logged-in member, before lightbox.js, classic and not deferred, with
  `data-assets` (`assets::rich_editor_urls`: every `prosemirror/` and
  `rich-editor/` asset's digested url) and `data-base-path`. An htmx
  navigation can bring the composer to a document that has already
  imported modules, so the map is there from the first page.
- `src/composer/view.rs` draws ComposerToggleSwitch first in
  `.d-editor-button-bar` (d-editor's showEditorModeToggle: no editor mode is
  forced in the composer), in the mode it opens in (the member's
  `composition_mode`, setupEditorMode): `--rte` and `aria-checked="true"` for 1,
  `--markdown` otherwise, `--active` on the fab-markdown or `a` icon,
  labelled with formatShortcut("ctrl+m") for a keyboard outside Apple's
  platforms. `#reply-control` carries `data-composition-mode`,
  `data-label-switch-to-markdown`, `-rich-text` and their `-no-shortcut`
  forms (with `%{keyboardShortcut}` left in), `data-label-key-ctrl`,
  `data-label-key-control` and `data-label-unsupported-token`;
  `.d-editor-container` takes `--rich-editor-enabled` or
  `--markdown-editor-enabled`, and `data-allow-preview` is false in rich
  mode.
- `static/js/composer.js` follows d-editor's toggleRichEditor. The toggle,
  Ctrl+M (in the textarea, or the editor's keymap) or opening the composer
  in mode 1 loads `Discourse.richEditor` and the preview's renderer (whose
  `tree` is `rich_editor_tree`), then `createEditor` with the engine and the
  context below in
  `.d-editor-textarea-wrapper` over the textarea's value. The textarea
  stays, hidden: each edit writes its markdown there and fires `input`, so
  drafts, presence and the save read it as before, and wherever composer.js
  writes the textarea itself (the text it opens with, a draft) the
  editor's `setValue` follows; the toolbar, a quote and a template go in
  through its TextManipulation (below). A refusal
  alerts `composer.unsupported_token` and switches back to markdown. Each switch redraws the container, the preview and the
  toggle (relabelled for the platform, or the `_no_shortcut` forms without
  a fine pointer) and saves `composition_mode` with PUT
  /u/:username.json after a second of quiet.
- `src/composer/rich_editor.rs` writes `data-rich-editor-context` on
  `#reply-control`: the client site settings the extensions read
  (default_code_lang, here_mention, max_image_width and so on), the strings
  they show (each plural key's forms as `key.one`, `key.other`), the
  current user's staff flag, `site.hashtag_configurations`'
  `topic-composer` order and `plugins`, the bundled plugins on that have
  an extension (by their `enabled_site_setting`). composer.js registers
  those (`registerPluginExtensions`), loading local dates' moment first
  when it is one, and its `richEditorContext` hands the editor
  that as getContext's `siteSettings`, `site`, `currentUser` and `i18n(key,
  params)`, with the composer's `topicId`, an `appEvents` emitter the upload
  code and the extensions share, `capabilities.viewport.sm` and
  d-editor's `replaceToolbar` (an extension's toolbar in the button bar's
  place, with a back button) and `modal.show(name, { model })` for the node
  views' modals: Edit Wrap (`wrap-modal.js`), the poll builder in its Edit
  poll mode (poll-builder.js `show(config, composer, { poll, onSave })`),
  Edit math (discourse-math.js `Discourse.mathEdit`), the policy builder
  over a node's values (policy.js `Discourse.policyBuilder.edit`, GET
  /live/policy-builder?edit=1 drawing it) and the insert date modal over a
  date's (local-dates-create.js `show(insert, initialValues)`). The
  editor's engine is `wasmEngine` over the preview's markdown.wasm, built
  once.
- In rich mode an upload goes in as the upload placeholder node (or an
  image node with `placeholder`), as text-manipulation's placeholder
  handler does: its progress, then the upload's markdown in its place, or
  nothing when it is cancelled from the node, the cancel link or by
  deleting it. Pasted files and the editor's own `data:` images upload
  through the same code.
- The profile's About me (preferences/profile.gjs' DEditor, no mode
  forced) is the same: `src/preferences_view/profile.rs` draws the toggle
  first in its button bar and the container in the viewer's
  `composition_mode`, and `static/js/preferences.js` opens the editor
  there through `Discourse.composerRichEditor` (composer.js: the
  composer's engine, extensions and context with no topic, a toolbar slot
  of its own for replaceToolbar, the toggle's drawing, the mode's save and
  the refusal's alert). Its textarea stays, hidden, as the composer's
  does, and Save Changes sends it; the toggle, Ctrl+M and a refusal save
  `composition_mode`, and a closed composer follows the switch, as Ember's
  opens anew in the saved mode. Its toolbar is the composer's
  (`Discourse.composerRichEditor.toolbar`), with no emoji button: its
  page draws no picker template.
- The toolbar (196r3, `static/js/editor-toolbar.js`): toolbar.ts's
  buttons, drawn by the server (`composer::view::editor_toolbar`: bold,
  italic, the heading menu, link, blockquote, code, upload, the list
  menu, emoji where enable_emoji is on, the options menu), act on
  d-editor's newToolbarEvent over the open editor's TextManipulation:
  the rich editor's (`editor.textManipulation`,
  `rich-editor/text-manipulation.js`) or the textarea's (editor-toolbar.js'
  port of textarea-text-manipulation.ts' toolbar half). The rich editor
  reports its toolbar state after each transaction (`onStateChange`), and
  the buttons draw `--active` and the heading and list icons from it; the
  menus mark their active options. The buttons' and the menus' shortcuts
  (mod+B, I, K, E, Shift+7/8/9, Alt+0 to 5, the options' Shift+D, M, .)
  are the rich editor's keymap and the textarea's keydown, and Tab and
  Shift+Tab indent (nest and lift list items in the rich editor). Link
  and mod+K open UpsertHyperlink (`static/js/upsert-hyperlink.js`); the
  link toolbar's Edit opens it through the context's modal service
  (`upsert-hyperlink`). Emoji opens the emoji picker (emoji-picker.js,
  loaded the first time) and the pick goes in through `emojiSelected`.
  The options menu's items act on the toolbar event once the menu has
  closed (details, footnote, spoiler's toggleSpoiler, graphviz, local
  dates, math, the poll and policy builders, toggle-wrap's
  WrapAttributesModal over insertWrap, updateWrap and removeWrap, the
  touch view's format-code); checklist adds its item to the list menu.
  Insert table and Change reply to are not ported. What a button inserts
  that the editor refuses alerts and goes back to markdown, as an opening
  does.
- Oneboxes (196r3): the onebox extension turns a linkified link the
  cursor has left into its preview or inline title, loaded through
  `Discourse.oneboxes` (oneboxes.js' `html` and `inlineTitles`, the
  preview's queue and caches) with the topic and a new topic's category,
  within max_oneboxes_per_post; the onebox toolbar removes a preview, and
  the link toolbar's Load preview forces one. A preview block
  (`node-views/preview.js`, discourse-graphviz's) shows its source or its
  drawing (graphviz.js' `Discourse.graphvizDiagram`), the preview toolbar
  swapping them and running the block's controls (Expand diagram,
  `Discourse.graphvizFullscreen`).
- GET /composer/mentions (`src/routes/composer_mentions.rs`) answers the
  mention extension: an @name that is no user or visible group, nor
  @here, goes back to text, and the reasons are triggered as
  `composer-messages:create`, which nothing shows yet: the composer's
  messages (ComposerMessages) are not ported.

What the composer and the editor do not do yet, as the reference does:

- The quote node view draws the quoted post's avatar from
  `markdownOptions.lookupAvatarTemplateByPostNumber`, which the composer
  does not give (its preview has no quote avatars either).
- A11y announcements (tables, videos) go through `getContext().a11y`; the
  port's pages have no live regions (A11y's), so the composer gives none
  and they are skipped.
- The strings' plural forms are picked by English's rule (one for 1, zero
  where the locale has it, other otherwise).

The default (196r5): Ember opens the composer in the member's stored mode
(d-editor's setupEditorMode), and Discourse ships `default_composition_mode`
1, so the seed's members, an imported site's and new signups open in the
rich editor. Until the round-trip tests passed the port held that back
(`composer::OPEN_IN_STORED_MODE`, off: always markdown); 196r5 removed the
switch once they did. The round-trip tests are each case that goes from
markdown through markdown.wasm's `rich_editor_tree` and the editor to the
markdown an edit serializes: rich-editor.spec.js' `round-trips` cases
(prosemirror-markdown-test, strikethrough, underline, and bbcode-inline's
[b], [i], [code], [url], [email] and [img], which the reference's tests leave
out), and every `testMarkdown` and `setupRichEditor` case of
rich-editor-extensions, -plugins, -table and -media.spec.js
(rich-editor-helper.js opens the editor over markdown through the
engine). What the editor refuses, it refuses whatever the mode: the
composer, an edit and the About me open in markdown with the reference's
alert and save `composition_mode` 0, as the alert's toggleRichEditor does
(composer.spec.js). The browser suite's members stay on markdown
(`tests/browser/setup.sql`), as the specs type into and read the
textarea; composer.spec.js gives its own the mode each case needs.
