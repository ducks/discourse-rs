//! features/text-post-process.js and the rules pushed on its ruler: text
//! outside links is searched for patterns that become nodes. Mentions
//! (features/mentions.js) so far.

use markdown_it::common::utils::is_punct_char;

use markdown_it::parser::inline::Text;
use markdown_it::plugins::cmark::inline::autolink::Autolink;
use markdown_it::plugins::cmark::inline::link::Link;
use markdown_it::{Node, NodeValue, Renderer};
use regex::Regex;
use std::sync::LazyLock as Lazy;

use super::RenderSettings;

/// `mentionRegex(false)`: `\w` is ASCII in JavaScript.
static MENTION: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"@([0-9A-Za-z_][0-9A-Za-z_.-]{0,58}[0-9A-Za-z])|@([0-9A-Za-z_])").unwrap()
});

/// `<span class="mention">@name</span>`; PrettyText.cleanup turns the ones
/// that name a user or group into links.
#[derive(Debug)]
pub struct Mention {
    pub username: String,
}

impl NodeValue for Mention {
    fn render(&self, _: &Node, fmt: &mut dyn Renderer) {
        fmt.open("span", &[("class", "mention".into())]);
        fmt.text(&format!("@{}", self.username));
        fmt.close("span");
    }
}

/// `allowedBoundary`: whitespace or punctuation.
fn boundary(c: char) -> bool {
    c.is_whitespace() || is_punct_char(c)
}

/// `hasAllowedBoundaries`
fn allowed_boundaries(content: &str, start: usize, end: usize) -> bool {
    let before = content[..start].chars().next_back();
    let after = content[end..].chars().next();
    before.is_none_or(boundary) && after.is_none_or(boundary)
}

/// `textPostProcess`: the text split around its matches, or None when
/// nothing matched.
fn split(content: &str) -> Option<Vec<Node>> {
    let mut result: Option<Vec<Node>> = None;
    let mut pos = 0;
    for caps in MENTION.captures_iter(content) {
        let whole = caps.get(0).unwrap();
        if !allowed_boundaries(content, whole.start(), whole.end()) {
            continue;
        }
        let nodes = result.get_or_insert_with(Vec::new);
        if whole.start() > pos {
            nodes.push(Node::new(Text {
                content: content[pos..whole.start()].to_string(),
            }));
        }
        let username = caps.get(1).or(caps.get(2)).unwrap().as_str();
        nodes.push(Node::new(Mention {
            username: username.to_string(),
        }));
        pos = whole.end();
    }
    if let Some(nodes) = result.as_mut() {
        if pos < content.len() {
            nodes.push(Node::new(Text {
                content: content[pos..].to_string(),
            }));
        }
    }
    result
}

/// The `text-post-process` core rule: textReplace with skipAllLinks, so
/// nothing inside a link is touched.
pub fn apply(root: &mut Node, settings: &RenderSettings) {
    if !settings.mentions {
        return;
    }
    fn visit(node: &mut Node) {
        if node.is::<Link>() || node.is::<Autolink>() {
            return;
        }
        let mut i = 0;
        while i < node.children.len() {
            let replaced = node.children[i]
                .cast::<Text>()
                .and_then(|text| split(&text.content));
            match replaced {
                Some(nodes) => {
                    let count = nodes.len();
                    node.children.splice(i..=i, nodes);
                    i += count;
                }
                None => {
                    visit(&mut node.children[i]);
                    i += 1;
                }
            }
        }
    }
    visit(root);
}
