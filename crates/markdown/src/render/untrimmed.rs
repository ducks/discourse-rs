//! Inline content JS does not trim. markdown-it's paragraph rule trims its
//! lines, but html_img and a one-line bbcode block hand theirs to the
//! inline parser as written, so the spaces at either end stay as text
//! (`<img ...> </p>`, `<p> text </p>` in a `[quote] text [/quote]`). The
//! crate's inline parser trims every content, so those spaces are kept in
//! nodes beside the InlineRoot and turned into text after parsing. Nodes
//! rather than a mark on the paragraph, which a tight list drops.

use markdown_it::Node;
use markdown_it::NodeValue;
use markdown_it::parser::inline::{InlineRoot, Text};

/// Spaces and tabs the crate's inline parser would trim off.
#[derive(Debug)]
pub struct Pad(String);

impl NodeValue for Pad {}

/// The inline content as JS leaves it: the InlineRoot with its leading and
/// trailing spaces and tabs kept beside it.
pub fn inline(content: String, mapping: Vec<(usize, usize)>) -> Vec<Node> {
    let body = content.trim_matches([' ', '\t']);
    let lead = content[..content.len() - content.trim_start_matches([' ', '\t']).len()].to_string();
    let trail = if body.is_empty() {
        String::new()
    } else {
        content[content.trim_end_matches([' ', '\t']).len()..].to_string()
    };
    let mut nodes = Vec::new();
    if !lead.is_empty() {
        nodes.push(Node::new(Pad(lead)));
    }
    nodes.push(Node::new(InlineRoot::new(content, mapping)));
    if !trail.is_empty() {
        nodes.push(Node::new(Pad(trail)));
    }
    nodes
}

/// Turns each kept pad into the text it is in JS.
pub fn restore(root: &mut Node) {
    root.walk_mut(|node, _| {
        if let Some(Pad(space)) = node.cast::<Pad>() {
            let content = space.clone();
            node.replace(Text { content });
        }
    });
}
