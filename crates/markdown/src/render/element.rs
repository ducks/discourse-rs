//! Nodes for the HTML Discourse's rules write through plain tokens: an
//! element with any tag and attributes, raw HTML, and text pushed at block
//! level (which the text passes do not visit).

use markdown_it::common::utils::escape_html;
use markdown_it::parser::extset::NodeExt;
use markdown_it::{Node, NodeValue, Renderer};

/// The markdown-it token an Element stands for, where the rule that built
/// it pushes one the rich editor parses (docs/rich-editor.md): its type
/// less `_open` / `_close`, and its markup. The tag alone cannot tell
/// `[b]`'s `bbcode_b` from any other span. A kind without the suffixes
/// (`wrap_bbcode`) is written with `bare` (JsWrap).
#[derive(Debug)]
pub struct JsToken {
    pub kind: String,
    pub markup: String,
}

/// Marks an Element whose JsToken is a pair of tokens of the same type.
#[derive(Debug)]
pub struct JsBare;

impl NodeExt for JsBare {}

impl NodeExt for JsToken {}

impl JsToken {
    pub fn on(node: &mut Node, kind: &str, markup: &str) {
        node.ext.insert(JsToken {
            kind: kind.to_string(),
            markup: markup.to_string(),
        });
    }
}

/// A token pair whose open and close tokens are both `kind`, nesting 1
/// and -1 (bbcode-block's `wrap_bbcode`), where the node renders as other
/// nodes: a Holder whose first and last children are the rule's own raw
/// HTML tags, which the editor's tree leaves out.
#[derive(Debug)]
pub struct JsWrap {
    pub kind: String,
    pub tag: String,
    pub attrs: Vec<(String, String)>,
}

impl NodeExt for JsWrap {}

/// Properties a rule sets on its token beyond markdown-it's own (the poll's
/// `poll_attrs`), for the editor's tree.
#[derive(Debug)]
pub struct JsFields(pub serde_json::Map<String, serde_json::Value>);

impl NodeExt for JsFields {}

/// The editor's tree nodes (docs/rich-editor.md) for what a rule writes as
/// raw HTML from tokens of its own: the poll's generated options and info.
#[derive(Debug)]
pub struct JsTree(pub Vec<serde_json::Value>);

impl NodeExt for JsTree {}

/// A `Token` opened and closed around its children. Rendered as
/// markdown-it's `renderToken` does: a block tag is followed by a newline
/// when it has content and when it closes.
#[derive(Debug)]
pub struct Element {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub block: bool,
}

impl Element {
    pub fn block(tag: &str, attrs: &[(&str, &str)]) -> Node {
        Node::new(Element {
            tag: tag.to_string(),
            attrs: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            block: true,
        })
    }

    pub fn inline(tag: &str, attrs: &[(&str, &str)]) -> Node {
        let mut node = Self::block(tag, attrs);
        node.cast_mut::<Element>().unwrap().block = false;
        node
    }
}

impl NodeValue for Element {
    fn render(&self, node: &Node, fmt: &mut dyn Renderer) {
        // renderToken starts a block on a new line after a hidden token (a
        // tight item's text), and the crate's `<li>` leaves its newline to
        // the block that opens it.
        if self.block {
            fmt.cr();
        }
        let mut open = format!("<{}", self.tag);
        for (name, value) in &self.attrs {
            open.push_str(&format!(
                " {}=\"{}\"",
                escape_html(name),
                escape_html(value)
            ));
        }
        open.push('>');
        if self.block && !node.children.is_empty() {
            open.push('\n');
        }
        fmt.text_raw(&open);
        fmt.contents(&node.children);
        fmt.text_raw(&format!("</{}>", self.tag));
        if self.block {
            fmt.text_raw("\n");
        }
    }
}

/// `html_inline` / `html_raw` content written by a rule.
#[derive(Debug)]
pub struct RawHtml(pub String);

impl NodeValue for RawHtml {
    fn render(&self, _: &Node, fmt: &mut dyn Renderer) {
        fmt.text_raw(&self.0);
    }
}

/// A `text` token pushed among block tokens: escaped on output, and not
/// part of any inline content, so mentions, emoji and the like leave it
/// alone.
#[derive(Debug)]
pub struct BlockText(pub String);

impl NodeValue for BlockText {
    fn render(&self, _: &Node, fmt: &mut dyn Renderer) {
        fmt.text(&self.0);
    }
}

/// Holds the nodes a rule had parsed before it knows where they go.
#[derive(Debug)]
pub struct Holder;
impl NodeValue for Holder {
    fn render(&self, node: &Node, fmt: &mut dyn Renderer) {
        fmt.contents(&node.children);
    }
}

/// The text of inline code. markdown-it's `code_inline` token has its
/// content and no children, so no pass over text (typographer, mentions,
/// emoji, linkify) ever sees it; the crate keeps it as a text node, which
/// they would.
#[derive(Debug)]
pub struct CodeText(pub String);

impl NodeValue for CodeText {
    fn render(&self, _: &Node, fmt: &mut dyn Renderer) {
        fmt.text(&self.0);
    }
}

/// Turns the text inside inline code into `CodeText`.
pub fn seal_inline_code(root: &mut Node) {
    use markdown_it::parser::inline::Text;
    use markdown_it::plugins::cmark::inline::backticks::CodeInline;
    root.walk_mut(|node, _| {
        if node.is::<CodeInline>() {
            for child in node.children.iter_mut() {
                if let Some(text) = child.cast::<Text>() {
                    let content = text.content.clone();
                    child.replace(CodeText(content));
                }
            }
        }
    });
}
