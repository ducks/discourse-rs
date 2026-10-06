//! markdown-it's `text_join` core rule: escapes and entities (`text_special`)
//! become plain text, and adjacent text joins into one. It runs right
//! after the typographer and before any core rule Discourse adds, so those
//! (emoji, mentions, watched words) see `:question:&nbsp;` as one text,
//! not an emoji alone that would be drawn large.
//!
//! JS works on a flat token list where an open and a close token sit
//! between a link's text and what follows it; here they are children of
//! the link node, so joining siblings is the same thing.

use markdown_it::Node;
use markdown_it::parser::inline::{Text, TextSpecial};

pub fn apply(root: &mut Node) {
    root.walk_mut(|node, _| {
        if node.children.is_empty() {
            return;
        }
        let mut joined: Vec<Node> = Vec::with_capacity(node.children.len());
        for mut child in std::mem::take(&mut node.children) {
            if let Some(special) = child.cast::<TextSpecial>() {
                let content = special.content.clone();
                let srcmap = child.srcmap;
                child = Node::new(Text { content });
                child.srcmap = srcmap;
            }
            let content = child.cast::<Text>().map(|t| t.content.clone());
            match (
                joined.last_mut().and_then(|n| n.cast_mut::<Text>()),
                content,
            ) {
                (Some(previous), Some(content)) => previous.content.push_str(&content),
                _ => joined.push(child),
            }
        }
        node.children = joined;
    });
}
