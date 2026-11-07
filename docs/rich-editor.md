# The composer's rich editor

Discourse's composer has two editors: the markdown textarea and the rich
editor, ProseMirror under `frontend/discourse/app/static/prosemirror/`
(ROADMAP 196). The port's rich editor is that code as plain ES modules in
`static/js/rich-editor/`, ProseMirror vendored under
`static/vendor/prosemirror/` (static/vendor/README), loaded through an
import map with no bundler or build step, and parsing markdown with the
port's own markdown crate (`crates/markdown`) so the editor and cooked posts
share one parser.

Markdown stays the default composition mode until the round-trip tests
pass; the rich editor is reachable through the member's `composition_mode`
(user_options, 1 is rich) and the composer's toggle. Making rich the default
(Discourse's `default_composition_mode` of 1) is a separate step (196r5).

## Files

| File | From the reference |
|---|---|
| `static/js/rich-editor.js` | `lib/load-rich-editor.js`: the import map and `Discourse.richEditor.load()` |
| `rich-editor/editor.js` | `components/prosemirror-editor.gts`, `core/plugin.js`, `core/commands.js` |
| `rich-editor/extensions.js` | `lib/composer/rich-editor-extensions.ts` and the core extensions: code (codemark), heading, strikethrough, strong, underline, trailing paragraph, ordered and bullet lists, hard break, placeholder |
| `rich-editor/schema.js` | `core/schema.js` |
| `rich-editor/parser.js` | `core/parser.js`, plus the node tree mapping below |
| `rich-editor/serializer.js` | `core/serializer.js`, `lib/expel-boundary-punctuation.ts` |
| `rich-editor/keymap.js` | `core/keymap.js` |
| `rich-editor/inputrules.js` | `core/inputrules.js` |
| `rich-editor/plugin-utils.js` | `lib/plugin-utils.js` (less getLinkify and buildBBCodeAttrs, which arrive with their extensions) |
| `rich-editor/to-markdown.js` | `to-markdown.js` (less word-paste's clean-up, 196r2) |
| `rich-editor/markdown-it-stub.js` | none: what `markdown-it` resolves to |
| `static/css/rich-editor.css` | `common/rich-editor/rich-editor.scss` (core rules), `common/components/composer-toggle-switch.scss` |

Extensions keep the reference's shape (nodeSpec, markSpec, parse,
serializeNode, serializeMark, inputRules, keymap, plugins, nodeViews,
commands, state, each a value or a function of the plugin params), so the
remaining ones port one by one (196r2, 196r4) without changing the core.
Node views are classes or functions; the Glimmer component descriptors have
no counterpart.

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
| `map` | `[startLine, endLine]` | `token.map`, optional |
| `children` | array of nodes | see below |
| `nesting` | `0` | only on a leaf that has `children` |

- A node with `children` and no `nesting` is a container: it flattens to
  `type_open`, its children, `type_close`. The close token copies the open
  token's tag, markup, hidden and block.
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
`[email]` as `link`. The rest (mentions, hashtags, quotes and the other
bbcode blocks, checklists, footnotes, math, the plugins') is written as
`unported:<name>` (the Element's tag, or the crate node's name) until its
extension records its JS token (196r2, 196r4). No parse spec handles
either yet, so they refuse the post as unsupported. Text may be split or merged differently from markdown-it's
(the parser merges adjacent text), so the test is the document a tree gives,
not equality with markdown-it's tokens.

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
   with the image and link renderers (images become their HTML), so an
   `upload://` image is an image token with its short url until the image
   extension (196r2) records what upload-protocol adds.
5. The settings: typographer off; onebox, watched words and censored off
   whatever the site's settings say.

## Tests

`tests/browser/rich-editor.spec.js` ports the reference's
`prosemirror-markdown-test.js` (the editor with prosemirror-markdown's
defaults alone), `strikethrough-test.js` and `underline-test.js`, checked as
`tests/helpers/rich-editor-helper.gjs` does (the editor's HTML less
ProseMirror's artifacts, and the markdown an edit serializes), three ways:

- `serializes`: the document built from the expected HTML (parseDOM), so the
  schema, view and serializer;
- `maps`: the document from the case's node tree, so `tokensFromTree` and
  the parse specs;
- `round-trips`: markdown through markdown.wasm's `rich_editor_tree` (all
  18 pass; the case skips itself on a build without the export).

It also checks the import map serves every module, typing through the input
rules and undo, the refusals, and the to-markdown cases the core covers.
Run it with `scripts/browser-test rich-editor`. The Rust tests in tree.rs
check the same cases' trees after the editor's text merging, and what the
overrides leave out.

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
  `composition_mode`, while `OPEN_IN_STORED_MODE` is on; see below): `--rte` and `aria-checked="true"` for 1,
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
  `tree` is `rich_editor_tree`), then `createEditor` in
  `.d-editor-textarea-wrapper` over the textarea's value. The textarea
  stays, hidden: each edit writes its markdown there and fires `input`, so
  drafts, presence and the save read it as before, and wherever composer.js
  writes the textarea itself (the text it opens with, the toolbar until
  196r3, a quote, a template, upload placeholders) the editor's `setValue`
  follows. A refusal alerts `composer.unsupported_token` and switches back
  to markdown. Each switch redraws the container, the preview and the
  toggle (relabelled for the platform, or the `_no_shortcut` forms without
  a fine pointer) and saves `composition_mode` with PUT
  /u/:username.json after a second of quiet.

A temporary deviation: Ember opens the composer in the member's stored
mode (d-editor's setupEditorMode), but Discourse ships
`default_composition_mode` 1, so the seed's members, an imported site's and
new signups would all open in a rich editor that refuses a post with a
mention, an emoji, a quote or any other construct whose extension is
196r2's or 196r4's. Until then `composer::OPEN_IN_STORED_MODE`
(`src/composer/load.rs`) is off: the composer always opens in markdown
(`data-composition-mode="0"`, the preview allowed) whatever is stored. The
toggle and Ctrl+M still switch and still save `composition_mode`, and no
stored value is rewritten. 196r5 turns the switch on and removes it. The
browser suite's members are on markdown too (`tests/browser/setup.sql`);
composer.spec.js gives its own the mode each case needs.
