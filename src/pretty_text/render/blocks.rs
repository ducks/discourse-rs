//! markdown-it's `inline` tokens, after the fact. The core rules that work
//! on inline content (linkify, smartquotes) go one inline token at a time:
//! its source decides whether a rule runs at all, and its ends bound what a
//! rule sees. The crate's inline parser splices each block's inline nodes
//! into its parent and a tight list item has no paragraph left, so the
//! blocks' source ranges are kept on the parent before the inline parser
//! runs, and `each_block` hands a rule each run of siblings that came from
//! one of them.

use std::ops::Range;

use markdown_it::parser::block::builtin::BlockParserRule;
use markdown_it::parser::core::CoreRule;
use markdown_it::parser::extset::NodeExt;
use markdown_it::parser::inline::InlineRoot;
use markdown_it::parser::inline::builtin::InlineParserRule;
use markdown_it::{MarkdownIt, Node};

/// An inline block: its source byte range and its content.
#[derive(Debug)]
struct Span {
    range: Range<usize>,
    source: String,
}

/// The inline blocks a node's children came from.
#[derive(Debug, Default)]
struct InlineSpans(Vec<Span>);

impl NodeExt for InlineSpans {}

/// Makes all of `node`'s children one inline block, from `range` of the
/// source: a footnote's paragraph, built after parsing from a note's nodes.
pub fn keep_block(node: &mut Node, range: Range<usize>, source: &str) {
    let source = source.to_string();
    node.ext.insert(InlineSpans(vec![Span { range, source }]));
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
                    let source = inline.content.clone();
                    Some(Span { range, source })
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
        .after::<super::link_pipes::RestoreLinkPipes>()
        .before::<InlineParserRule>();
}

/// Runs `f` on each inline block: the run of siblings that came from it,
/// as the children of a scratch node, with the block's source.
pub fn each_block(root: &mut Node, mut f: impl FnMut(&mut Node, &str)) {
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
            f(&mut block, &spans[span].source);
            node.children.append(&mut block.children);
        }
        node.ext.insert(InlineSpans(spans));
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
