//! Nodes for the HTML Discourse's rules write through plain tokens: an
//! element with any tag and attributes, raw HTML, and text pushed at block
//! level (which the text passes do not visit).

use markdown_it::common::utils::escape_html;
use markdown_it::{Node, NodeValue, Renderer};

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
