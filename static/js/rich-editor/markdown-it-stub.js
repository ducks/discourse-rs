// What the import map resolves 'markdown-it' to. prosemirror-markdown
// imports markdown-it and calls MarkdownIt("commonmark", { html: false })
// at module load for its defaultMarkdownParser, which the rich editor never
// uses: it reads defaultMarkdownParser.tokens (the parse specs) and parses
// with the markdown crate's node tree (rich-editor/parser.js). So this is
// only enough for that call, and a parse through it says where to look.
export default function MarkdownIt() {
  return {
    parse: function () {
      throw new Error(
        "markdown-it is not loaded: the rich editor parses with the markdown crate (rich-editor/parser.js)"
      );
    },
  };
}
