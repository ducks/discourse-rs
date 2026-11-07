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

The parser takes markdown as a node tree in JSON, which the markdown crate's
WebAssembly export writes (phase B). The tree is markdown-it's token stream
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

Discourse's other tokens (html_block, html_inline, emoji, mention, hashtag,
table, quotes and the other bbcode blocks, the plugins') are written the same
way, as their rules push them; their parse specs arrive with their
extensions (196r2, 196r4), and until then they refuse the post as
unsupported. Text may be split or merged differently from markdown-it's
(the parser merges adjacent text), so the test is the document a tree gives,
not equality with markdown-it's tokens.

tests/browser/rich-editor.spec.js writes the trees for each of the
reference's cases (its `doc(...)` builders): those are the expected shape.

### What the export must rebuild

The crate's tree is not markdown-it's token stream as it stands:

1. A tight list's paragraphs are spliced away (markdown-it 0.6
   `ListScanner::mark_tight_paragraphs`): each item's text goes back in a
   `paragraph` with `hidden: true`.
2. `InlineRoot` is replaced by its children once inline rules run: each
   block's inline content goes back in an `inline` node.
3. `render::element::Element` keeps only a tag and attrs. Where a rule builds
   one for a JS token (bbcode-inline's `[b]` is `bbcode_b`, a `span` with
   `class="bbcode-b"`), the token's type and markup must be recorded (a node
   extension set by the rule), not guessed from the tag.
4. The tree is the parse, before what only rendering adds: heading anchors
   (`render::anchor`, an empty link pair in JS that gives the editor
   nothing, so left out), the sanitizer, text post-processing for HTML.
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
- `round-trips`: markdown through markdown.wasm's `rich_editor_tree`,
  skipped while the build lacks the export.

It also checks the import map serves every module, typing through the input
rules and undo, the refusals, and the to-markdown cases the core covers.
Run it with `scripts/browser-test rich-editor`.

## Phase B (the Rust side of 196r1)

1. `crates/markdown`: a function returning the node tree for raw markdown
   and RenderSettings (with the overrides above), and in `src/wasm.rs`
   `rich_editor_tree(ptr, len) -> i32`, called like `preview`: the
   configured settings, the tree's JSON at `output_ptr` / `output_len`, 1 and
   a message on failure. Rust tests over the spec's cases, comparing the
   trees after the same text merging. MARKDOWN_WASM_VERSION follows the new
   bytes.
2. `src/assets.rs`: the current urls of every asset under a prefix
   (`prosemirror/`, `rich-editor/`) as a JSON object, for `data-assets`.
3. `templates/layout.html`: `<script src="rich-editor.js" data-assets="..."
   data-base-path="...">` (classic, not deferred, so its map precedes any
   module) in the head before lightbox.js, on every page of a logged-in
   member, since an htmx navigation can bring the composer to a document
   that has already imported modules.
4. `src/composer/view.rs` and `src/composer/load.rs`: `data-composition-mode`
   on `#reply-control` (the member's user_options.composition_mode; keep
   `allow_preview` false for 1), and the toggle as d-editor draws it when no
   editor mode is forced: first in `.d-editor-button-bar`,
   `<button class="composer-toggle-switch --markdown" role="switch"
   aria-checked="false" data-rich-editor="false" type="button">` with
   `composer-toggle-switch__slider` and its `__left-icon` (fab-markdown,
   `--active` in markdown mode) and `__right-icon` (`a`, `--active` in rich
   mode); the labels `composer.switch_to_markdown`,
   `composer.switch_to_rich_text` (with `%{keyboardShortcut}`, Ctrl+M) and
   their `_no_shortcut` forms, and `composer.unsupported_token`, as data
   attributes. Both icons in the sprite.
5. `static/js/composer.js` (d-editor's toggleRichEditor): the renderer loader
   also returns `tree` (`JSON.parse` of `rich_editor_tree`). Switching to
   rich, or opening with composition mode 1: `Discourse.richEditor.load()`,
   then `createEditor` in `.d-editor-textarea-wrapper` with the textarea's
   value, `parseTree`, its placeholder, `className: "d-editor-input"`,
   `keymap` Ctrl+M back, `onChange` writing the textarea's value and firing
   its `input` (drafts, presence and submit keep reading the textarea),
   `onUnsupported` alerting the string and switching back. The textarea is
   hidden, `.d-editor-container` takes `--rich-editor-enabled` for
   `--markdown-editor-enabled`, the preview hides, the toggle takes `--rte`.
   Switching back destroys the editor and shows the textarea. Each switch
   sets the preference after 1s of quiet (PUT /u/:username.json
   `composition_mode`). Where composer.js writes the textarea itself (a
   draft, a quote, the toolbar until 196r3), the editor's `setValue` follows.
6. Run the spec against the phase B build: the `round-trips` cases must
   pass, and composer.spec.js and composer-actions.spec.js stay green.
