//! markdown-it's `smartquotes`, one inline block at a time. JS pairs quotes
//! within each `inline` token, and its start and end count as space; the
//! crate's rule pairs them across the whole document and takes only a
//! paragraph or a line break as a boundary. A tight list item has no
//! paragraph and a table cell never had one, so a quote closing an item or a
//! cell saw the next one's first letter and stayed straight.
//!
//! The inline parser splices each block's inline nodes into its parent, so
//! the blocks' source ranges are kept on the parent before it runs, and the
//! crate's rule runs on each run of siblings that came from one of them.
//! Image alt text is left alone: in JS it is the image token's own
//! children, which smartquotes does not visit.

use std::ops::Range;

use markdown_it::parser::block::builtin::BlockParserRule;
use markdown_it::parser::core::CoreRule;
use markdown_it::parser::extset::NodeExt;
use markdown_it::parser::inline::InlineRoot;
use markdown_it::parser::inline::builtin::InlineParserRule;
use markdown_it::plugins::cmark::inline::image::Image;
use markdown_it::plugins::extra::smartquotes::SmartQuotesRule;
use markdown_it::{MarkdownIt, Node};

/// markdown-it's default quotes, which RenderSettings insists on.
type SmartQuotes = SmartQuotesRule<'‘', '’', '“', '”'>;

/// An inline block: its source byte range, and whether that source has a
/// quote at all (`QUOTE_TEST_RE`); JS skips a block without one, even when
/// linkify has since decoded a `%27` in it into a quote.
#[derive(Debug)]
struct Span {
    range: Range<usize>,
    quoted: bool,
}

impl Span {
    fn new(range: Range<usize>, source: &str) -> Self {
        let quoted = source.contains(['\'', '"']);
        Self { range, quoted }
    }
}

/// The inline blocks a node's children came from.
#[derive(Debug, Default)]
struct InlineSpans(Vec<Span>);

impl NodeExt for InlineSpans {}

/// Makes all of `node`'s children one inline block, from `range` of the
/// source: a footnote's paragraph, built after parsing from a note's nodes.
pub fn keep_block(node: &mut Node, range: Range<usize>, source: &str) {
    node.ext.insert(InlineSpans(vec![Span::new(range, source)]));
}

struct KeepInlineSpans;

impl CoreRule for KeepInlineSpans {
    fn run(root: &mut Node, _: &MarkdownIt) {
        root.walk_mut(|node, _| {
            let spans: Vec<Span> = node
                .children
                .iter()
                .filter_map(|c| c.cast::<InlineRoot>())
                .filter_map(|inline| {
                    let (first, last) = (inline.mapping.first()?, inline.mapping.last()?);
                    let range = first.1..last.1 + (inline.content.len() - last.0);
                    Some(Span::new(range, &inline.content))
                })
                .collect();
            if !spans.is_empty() {
                node.ext.insert(InlineSpans(spans));
            }
        });
    }
}

pub fn add(md: &mut MarkdownIt) {
    md.add_rule::<KeepInlineSpans>()
        .after::<BlockParserRule>()
        .before::<InlineParserRule>();
}

pub fn run(root: &mut Node, md: &MarkdownIt) {
    root.walk_mut(|node, _| {
        let Some(InlineSpans(spans)) = node.ext.remove::<InlineSpans>() else {
            return;
        };
        let owners = owners(&node.children, &spans);
        let mut children = std::mem::take(&mut node.children)
            .into_iter()
            .zip(owners)
            .peekable();
        while let Some((child, owner)) = children.next() {
            let Some(span) = owner else {
                node.children.push(child);
                continue;
            };
            let mut block = Node::default();
            block.children.push(child);
            while let Some((next, _)) = children.next_if(|(_, o)| *o == Some(span)) {
                block.children.push(next);
            }
            if !spans[span].quoted {
                node.children.append(&mut block.children);
                continue;
            }
            let mut alts = Vec::new();
            take_alts(&mut block, &mut alts);
            SmartQuotes::run(&mut block, md);
            let mut alts = alts.into_iter();
            put_alts(&mut block, &mut alts);
            node.children.append(&mut block.children);
        }
    });
}

/// Which inline block each child came from, by where its source starts.
/// Nodes made after parsing (linkify's) have no source and belong to the
/// block of the nodes around them, or to the only block there is; a block
/// child has a source outside them all and belongs to none.
fn owners(children: &[Node], spans: &[Span]) -> Vec<Option<usize>> {
    let sourced: Vec<Option<Option<usize>>> = children
        .iter()
        .map(|c| {
            let (start, _) = c.srcmap?.get_byte_offsets();
            Some(spans.iter().position(|s| s.range.contains(&start)))
        })
        .collect();
    (0..children.len())
        .map(|i| {
            sourced[i].unwrap_or_else(|| {
                let before = sourced[..i].iter().rev().find_map(|s| *s);
                let after = || sourced[i + 1..].iter().find_map(|s| *s);
                let only = (spans.len() == 1).then_some(0);
                before.flatten().or_else(|| after().flatten()).or(only)
            })
        })
        .collect()
}

fn take_alts(node: &mut Node, alts: &mut Vec<Vec<Node>>) {
    for child in &mut node.children {
        if child.is::<Image>() {
            alts.push(std::mem::take(&mut child.children));
        } else {
            take_alts(child, alts);
        }
    }
}

fn put_alts(node: &mut Node, alts: &mut impl Iterator<Item = Vec<Node>>) {
    for child in &mut node.children {
        if child.is::<Image>() {
            child.children = alts.next().unwrap_or_default();
        } else {
            put_alts(child, alts);
        }
    }
}
