//! A block right after a tight list item's text. markdown-it hides that
//! paragraph, and only `renderToken` starts a block with a newline when
//! the token before is hidden; the fence and html_block rules write their
//! output as is, so it follows the text directly (`<li>a<pre>`). The
//! crate's renderers and ours open every block on a new line, so those two
//! are marked while the item's text is still an InlineRoot beside them.
//! (An indented code block cannot follow the text: the blank line it
//! needs makes the list loose.)

use markdown_it::parser::block::builtin::BlockParserRule;
use markdown_it::parser::core::CoreRule;
use markdown_it::parser::extset::NodeExt;
use markdown_it::parser::inline::InlineRoot;
use markdown_it::parser::inline::builtin::InlineParserRule;
use markdown_it::plugins::cmark::block::fence::CodeFence;
use markdown_it::plugins::html::html_block::HtmlBlock;
use markdown_it::{MarkdownIt, Node};

use super::element::RawHtml;
use super::html_img;

/// The block follows a tight item's text: no newline before it.
#[derive(Debug, Default)]
pub struct AfterText;

impl NodeExt for AfterText {}

struct MarkAfterText;

impl CoreRule for MarkAfterText {
    fn run(root: &mut Node, _: &MarkdownIt) {
        root.walk_mut(|node, _| {
            for i in 1..node.children.len() {
                let block = &node.children[i];
                let before = &node.children[i - 1];
                if (block.is::<CodeFence>() || block.is::<HtmlBlock>())
                    && (before.is::<InlineRoot>() || before.is::<html_img::Trailing>())
                {
                    node.children[i].ext.insert(AfterText);
                }
            }
        });
    }
}

pub fn add(md: &mut MarkdownIt) {
    md.add_rule::<MarkAfterText>()
        .after::<BlockParserRule>()
        .before::<InlineParserRule>();
}

/// A marked html_block becomes its content alone, as JS writes it; run
/// last, after every pass that reads html blocks.
pub fn apply(root: &mut Node) {
    root.walk_mut(|node, _| {
        if node.ext.get::<AfterText>().is_some()
            && let Some(html) = node.cast::<HtmlBlock>()
        {
            let content = html.content.clone();
            node.replace(RawHtml(content));
        }
    });
}
