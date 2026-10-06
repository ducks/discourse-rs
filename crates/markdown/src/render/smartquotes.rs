//! markdown-it's `smartquotes`, one inline block at a time (see blocks.rs).
//! JS pairs quotes within each `inline` token, and its start and end count
//! as space; the crate's rule pairs them across the whole document and
//! takes only a paragraph or a line break as a boundary. A tight list item
//! has no paragraph and a table cell never had one, so a quote closing an
//! item or a cell saw the next one's first letter and stayed straight.
//!
//! Image alt text is left alone: in JS it is the image token's own
//! children, which smartquotes does not visit. Inline code is not changed
//! but is read for the characters around a quote.

use markdown_it::parser::core::CoreRule;
use markdown_it::parser::extset::NodeExt;
use markdown_it::parser::inline::Text;
use markdown_it::plugins::cmark::inline::backticks::CodeInline;
use markdown_it::plugins::cmark::inline::image::Image;
use markdown_it::plugins::extra::smartquotes::SmartQuotesRule;
use markdown_it::plugins::html::html_inline::HtmlInline;
use markdown_it::{MarkdownIt, Node};

use super::blocks;
use super::element::CodeText;

/// markdown-it's default quotes, which RenderSettings insists on.
type SmartQuotes = SmartQuotesRule<'‘', '’', '“', '”'>;

/// Each inline block whose source has a quote at all (`QUOTE_TEST_RE`); JS
/// skips one without, even when linkify has since decoded a `%27` in it
/// into a quote.
pub fn run(root: &mut Node, md: &MarkdownIt) {
    blocks::each_block(root, |block, source| {
        if !source.contains(['\'', '"']) {
            return;
        }
        let mut aside = Vec::new();
        set_aside(block, &mut aside);
        SmartQuotes::run(block, md);
        put_back(block, &mut aside.into_iter());
    });
}

/// What is kept out of the crate's sight while it runs: an image's alt
/// text, or an inline code node, which stands in as html of its text. JS
/// reads a `code_inline` token's content for the characters around a
/// quote (`app.yml`'s) but never changes it; the crate does the same with
/// inline html and skips inline code altogether.
enum Aside {
    Alt(Vec<Node>),
    Code(Node),
}

fn set_aside(node: &mut Node, aside: &mut Vec<Aside>) {
    for child in &mut node.children {
        if child.is::<Image>() {
            aside.push(Aside::Alt(std::mem::take(&mut child.children)));
        } else if child.is::<CodeInline>() {
            let mut content = String::new();
            child.walk(|n, _| {
                if let Some(text) = n.cast::<CodeText>() {
                    content.push_str(&text.0);
                } else if let Some(text) = n.cast::<Text>() {
                    content.push_str(&text.content);
                }
            });
            let mut stand_in = Node::new(HtmlInline { content });
            stand_in.ext.insert(StandIn);
            let code = std::mem::replace(child, stand_in);
            aside.push(Aside::Code(code));
        } else {
            set_aside(child, aside);
        }
    }
}

/// Marks the html that stands in for inline code.
#[derive(Debug, Default)]
struct StandIn;

impl NodeExt for StandIn {}

/// Puts back what `set_aside` took, in the order it took it.
fn put_back(node: &mut Node, aside: &mut impl Iterator<Item = Aside>) {
    for child in &mut node.children {
        if child.is::<Image>() {
            if let Some(Aside::Alt(alt)) = aside.next() {
                child.children = alt;
            }
        } else if child.ext.get::<StandIn>().is_some() {
            if let Some(Aside::Code(code)) = aside.next() {
                *child = code;
            }
        } else {
            put_back(child, aside);
        }
    }
}
