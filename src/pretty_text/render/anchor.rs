//! features/anchor.js: every heading outside a quote starts with an empty
//! link named after its text, numbered through the document.

use markdown_it::parser::block::builtin::BlockParserRule;
use markdown_it::parser::core::CoreRule;
use markdown_it::parser::extset::NodeExt;
use markdown_it::parser::inline::InlineRoot;
use markdown_it::parser::inline::builtin::InlineParserRule;
use markdown_it::plugins::cmark::block::blockquote::Blockquote;
use markdown_it::plugins::cmark::block::heading::ATXHeading;
use markdown_it::plugins::cmark::block::lheading::SetextHeader;
use markdown_it::{MarkdownIt, Node, NodeValue, Renderer};

use super::RenderSettings;
use crate::pretty_text::sanitizer::AllowList;

/// The heading's inline source, kept from before the inline parser turns
/// it into nodes: the slug is made from the markdown, not the text.
#[derive(Debug)]
struct HeadingSource(String);
impl NodeExt for HeadingSource {}

#[derive(Debug)]
struct HeadingAnchor {
    slug: String,
    label: String,
}

impl NodeValue for HeadingAnchor {
    fn render(&self, _: &Node, fmt: &mut dyn Renderer) {
        fmt.open(
            "a",
            &[
                ("name", self.slug.clone()),
                ("class", "anchor".into()),
                ("href", format!("#{}", self.slug)),
                ("aria-label", self.label.clone()),
            ],
        );
        fmt.close("a");
    }
}

fn is_heading(node: &Node) -> bool {
    node.is::<ATXHeading>() || node.is::<SetextHeader>()
}

struct KeepHeadingSource;
impl CoreRule for KeepHeadingSource {
    fn run(root: &mut Node, _: &MarkdownIt) {
        root.walk_mut(|node, _| {
            if !is_heading(node) {
                return;
            }
            let source = node
                .children
                .iter()
                .find_map(|c| c.cast::<InlineRoot>())
                .map(|inline| inline.content.clone());
            if let Some(source) = source {
                node.ext.insert(HeadingSource(source));
            }
        });
    }
}

/// The slug of anchor.js: lowercased, whitespace to dashes, everything
/// but `[A-Za-z0-9_-]` dropped, dashes collapsed and trimmed.
fn slug(source: &str) -> String {
    let mut out = String::new();
    let mut in_space = false;
    for c in source.to_lowercase().chars() {
        if c.is_whitespace() {
            if !in_space {
                out.push('-');
            }
            in_space = true;
            continue;
        }
        in_space = false;
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        }
    }
    let mut collapsed = String::with_capacity(out.len());
    for c in out.chars() {
        if c == '-' && collapsed.ends_with('-') {
            continue;
        }
        collapsed.push(c);
    }
    let slug = collapsed.trim_matches('-').to_string();
    if slug.chars().next().is_some_and(|c| !c.is_ascii_lowercase()) {
        format!("h-{slug}")
    } else {
        slug
    }
}

/// The `anchor` core rule.
pub fn apply(root: &mut Node, settings: &RenderSettings) {
    fn visit(node: &mut Node, settings: &RenderSettings, heading_id: &mut u32) {
        // Headings inside a quote get no anchor.
        if node.is::<Blockquote>()
            || node
                .cast::<super::element::Element>()
                .is_some_and(|e| e.tag == "aside")
        {
            return;
        }
        if is_heading(node) {
            let source = node
                .ext
                .get::<HeadingSource>()
                .map(|s| s.0.clone())
                .unwrap_or_default();
            *heading_id += 1;
            let slug = slug(&source);
            let mut slug = format!(
                "{}-{heading_id}",
                if slug.is_empty() { "h" } else { slug.as_str() }
            );
            if let Some(post_id) = settings.post_id {
                slug = format!("p-{post_id}-{slug}");
            }
            node.children.insert(
                0,
                Node::new(HeadingAnchor {
                    slug,
                    label: settings.heading_anchor_label.clone(),
                }),
            );
            return;
        }
        for child in node.children.iter_mut() {
            visit(child, settings, heading_id);
        }
    }
    let mut heading_id = 0;
    visit(root, settings, &mut heading_id);
}

/// Registers the rule that keeps each heading's source; it has to sit
/// between the block and the inline parser, so it is the one rule here
/// that lives in the crate's own chain.
pub fn add(md: &mut MarkdownIt) {
    md.add_rule::<KeepHeadingSource>()
        .after::<BlockParserRule>()
        .before::<InlineParserRule>();
}

pub fn allow(list: &mut AllowList) {
    list.allow(&["a[aria-label]"]);
}
