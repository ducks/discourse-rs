//! The checklist plugin's rule (plugins/checklist, discourse-markdown/
//! checklist.js): `[ ]`, `[x]`, `[X]` and `[]` in text become checkbox
//! spans. Each carries where its marker is in the source, as
//! `line:nth-marker-on-that-line`, so that Rails can toggle it in the raw
//! without parsing markdown again.

use std::collections::HashMap;
use std::sync::LazyLock;

use markdown_it::parser::block::builtin::BlockParserRule;
use markdown_it::parser::core::{CoreRule, Root};
use markdown_it::parser::extset::NodeExt;
use markdown_it::parser::inline::builtin::InlineParserRule;
use markdown_it::parser::inline::{InlineRoot, InlineRule, InlineState, Text};
use markdown_it::plugins::html::html_block::HtmlBlock;
use markdown_it::plugins::html::html_inline::HtmlInline;
use markdown_it::{MarkdownIt, Node, NodeValue, Renderer};
use regex::Regex;

use super::RenderSettings;
use super::element::{BlockText, Element};
use crate::pretty_text::sanitizer::AllowList;

static MARKER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[ xX]?\]").unwrap());
static RAW_MARKUP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bchcklst-box\b|\bdata-chk-src\b").unwrap());

/// A marker the inline rule found, at this offset of its block's inline
/// content.
#[derive(Debug)]
struct Candidate {
    marker: String,
    offset: usize,
}

impl NodeValue for Candidate {
    fn render(&self, _: &Node, fmt: &mut dyn Renderer) {
        fmt.text(&self.marker);
    }
}

struct CandidateScanner;
impl InlineRule for CandidateScanner {
    const MARKER: char = '[';

    fn run(state: &mut InlineState) -> Option<(Node, usize)> {
        if !state.md.ext.get::<RenderSettings>()?.checklist {
            return None;
        }
        let rest = &state.src[state.pos..state.pos_max];
        let marker = if rest.starts_with("[]") {
            "[]"
        } else if ["[ ]", "[x]", "[X]"].iter().any(|m| rest.starts_with(m)) {
            &rest[..3]
        } else {
            return None;
        };
        Some((
            Node::new(Candidate {
                marker: marker.to_string(),
                offset: state.pos,
            }),
            marker.len(),
        ))
    }
}

/// A block's inline content and the source line it starts on, kept from
/// before the inline parser consumes it.
#[derive(Debug)]
struct InlineSource {
    content: String,
    base_line: Option<usize>,
}
impl NodeExt for InlineSource {}

struct KeepInlineSource;
impl CoreRule for KeepInlineSource {
    fn run(root: &mut Node, md: &MarkdownIt) {
        if !md.ext.get::<RenderSettings>().is_some_and(|s| s.checklist) {
            return;
        }
        let source = root
            .cast::<Root>()
            .map(|r| r.content.clone())
            .unwrap_or_default();
        root.walk_mut(|node, _| {
            let content = node
                .children
                .iter()
                .find_map(|c| c.cast::<InlineRoot>())
                .map(|inline| inline.content.clone());
            if let Some(content) = content {
                let base_line = node.srcmap.map(|map| {
                    let (start, _) = map.get_byte_offsets();
                    source[..start.min(source.len())].matches('\n').count()
                });
                node.ext.insert(InlineSource { content, base_line });
            }
        });
    }
}

fn classes(marker: &str) -> &'static str {
    match marker {
        "[x]" => "checked fa fa-square-check-o",
        "[X]" => "checked permanent fa fa-square-check",
        _ => "fa fa-square-o",
    }
}

/// `markerLocations`: per marker offset in the content, the source line it
/// is on and how many markers that line had before it. The counts run on
/// across blocks, as other rules may have consumed markers of a line.
fn marker_locations(
    content: &str,
    base_line: usize,
    counts: &mut HashMap<usize, usize>,
) -> HashMap<usize, (usize, usize, String)> {
    let mut locations = HashMap::new();
    let mut line = base_line;
    let mut scanned = 0;
    for found in MARKER.find_iter(content) {
        line += content[scanned..found.start()].matches('\n').count();
        scanned = found.end();
        let nth = counts.entry(line).or_insert(0);
        locations.insert(found.start(), (line, *nth, found.as_str().to_string()));
        *nth += 1;
    }
    locations
}

/// The `checklist` core rule.
pub fn run(root: &mut Node, settings: &RenderSettings) {
    if !settings.checklist {
        return;
    }
    let source = root
        .cast::<Root>()
        .map(|r| r.content.clone())
        .unwrap_or_default();
    if !source.contains('[') {
        neutralize(root);
        return;
    }
    neutralize(root);
    let source_lines: Vec<&str> = source.split('\n').collect();
    let mut counts: HashMap<usize, usize> = HashMap::new();

    fn visit(
        node: &mut Node,
        source_lines: &[&str],
        counts: &mut HashMap<usize, usize>,
        nested: bool,
    ) {
        // A block with inline content starts a new set of locations.
        let locations = node.ext.get::<InlineSource>().and_then(|inline| {
            inline
                .base_line
                .map(|line| marker_locations(&inline.content, line, counts))
        });
        let is_block = node.ext.contains::<InlineSource>();
        for child in node.children.iter_mut() {
            let candidate = child
                .cast::<Candidate>()
                .map(|c| (c.marker.clone(), c.offset));
            let Some((marker, offset)) = candidate else {
                // Inside emphasis, a link and the like, a marker is text.
                visit(
                    child,
                    source_lines,
                    counts,
                    !child.ext.contains::<InlineSource>() && (nested || is_block),
                );
                continue;
            };
            if nested && !is_block {
                child.replace(Text { content: marker });
                continue;
            }
            let mut attrs = vec![(
                "class".to_string(),
                format!("chcklst-box {}", classes(&marker)),
            )];
            // The location only counts if the source line really has that
            // marker in that place; a permanent `[X]` gets none.
            let verified = locations
                .as_ref()
                .filter(|_| marker != "[X]")
                .and_then(|l| l.get(&offset))
                .filter(|(line, nth, found)| {
                    let line_markers: Vec<&str> = source_lines
                        .get(*line)
                        .map(|text| MARKER.find_iter(text).map(|m| m.as_str()).collect())
                        .unwrap_or_default();
                    line_markers.get(*nth) == Some(&found.as_str())
                });
            if let Some((line, nth, _)) = verified {
                attrs.push(("data-chk-src".to_string(), format!("{line}:{nth}")));
            }
            *child = Node::new(Element {
                tag: "span".into(),
                attrs,
                block: false,
            });
        }
    }
    visit(root, &source_lines, &mut counts, false);
}

/// `neutralizeRawChecklistMarkup`: raw html that imitates a checkbox is
/// shown as the text it is.
fn neutralize(root: &mut Node) {
    root.walk_mut(|node, _| {
        if let Some(html) = node.cast::<HtmlInline>() {
            if RAW_MARKUP.is_match(&html.content) {
                let content = html.content.clone();
                node.replace(Text { content });
            }
        } else if let Some(html) = node.cast::<HtmlBlock>()
            && RAW_MARKUP.is_match(&html.content)
        {
            let content = html.content.clone();
            node.replace(BlockText(content));
        }
    });
}

pub fn add(md: &mut MarkdownIt) {
    md.inline.add_rule::<CandidateScanner>();
    md.add_rule::<KeepInlineSource>()
        .after::<BlockParserRule>()
        .after::<super::link_pipes::RestoreLinkPipes>()
        .before::<InlineParserRule>();
}

pub fn allow(list: &mut AllowList) {
    list.allow(&[
        "span.chcklst-stroked",
        "span.chcklst-box fa fa-square-o",
        "span.chcklst-box checked fa fa-square-check-o",
        "span.chcklst-box checked permanent fa fa-square-check",
        "span[data-chk-src]",
    ]);
}
