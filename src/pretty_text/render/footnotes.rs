//! The footnote plugin (plugins/footnote), which is markdown-it-footnote
//! 3.0.3: `[^label]` with a `[^label]: text` definition, and inline
//! `^[text]`. References are numbered in the order they appear; the notes
//! are listed after the document, each ending with links back to its
//! references.

use std::collections::HashMap;

use markdown_it::common::sourcemap::SourcePos;
use markdown_it::parser::block::{BlockRule, BlockState};
use markdown_it::parser::extset::RootExt;
use markdown_it::parser::inline::{InlineRule, InlineState, Text};
use markdown_it::plugins::cmark::block::paragraph::Paragraph;
use markdown_it::plugins::cmark::block::reference::ReferenceScanner;
use markdown_it::plugins::html::html_inline::HtmlInline;
use markdown_it::{MarkdownIt, Node, NodeValue, Renderer};

use super::RenderSettings;
use super::element::{Holder, RawHtml};
use crate::pretty_text::sanitizer::AllowList;

/// `env.footnotes`: the labels defined, and the notes in the order they
/// were first referenced.
#[derive(Debug, Default)]
struct Footnotes {
    /// label -> its place in `list` once referenced.
    refs: HashMap<String, Option<usize>>,
    list: Vec<Note>,
}
impl RootExt for Footnotes {}

#[derive(Debug)]
struct Note {
    /// None for an inline note.
    label: Option<String>,
    /// How often it is referenced.
    count: usize,
}

/// `[^label]: ...` with the blocks of the note as children; taken out of
/// the document by `tail`.
#[derive(Debug)]
struct Definition {
    label: String,
}
impl NodeValue for Definition {}

/// A reference. An inline note carries its content as children until
/// `tail` moves it to the list.
#[derive(Debug)]
struct Reference {
    id: usize,
    sub_id: usize,
    inline: bool,
}

impl NodeValue for Reference {
    fn render(&self, _: &Node, fmt: &mut dyn Renderer) {
        let n = self.id + 1;
        let (ref_id, caption) = if self.sub_id > 0 {
            (
                format!("{n}:{}", self.sub_id),
                format!("[{n}:{}]", self.sub_id),
            )
        } else {
            (n.to_string(), format!("[{n}]"))
        };
        fmt.text_raw(&format!(
            "<sup class=\"footnote-ref\"><a href=\"#fn{n}\" id=\"fnref{ref_id}\">{caption}</a></sup>"
        ));
    }
}

fn enabled(md: &MarkdownIt) -> bool {
    md.ext.get::<RenderSettings>().is_some_and(|s| s.footnotes)
}

/// `[^label]:` at the start of a line: the label, and the offset right
/// after the colon.
fn definition_start(line: &str) -> Option<(&str, usize)> {
    let rest = line.strip_prefix("[^")?;
    if line.len() < 4 {
        return None;
    }
    let end = rest.find([' ', ']'])?;
    // No spaces in a label, and no empty one.
    if !rest[end..].starts_with(']') || end == 0 {
        return None;
    }
    let after = 2 + end + 1;
    line[after..]
        .starts_with(':')
        .then(|| (&rest[..end], after + 1))
}

struct DefinitionScanner;
impl BlockRule for DefinitionScanner {
    fn check(state: &mut BlockState) -> Option<()> {
        if !enabled(state.md) {
            return None;
        }
        definition_start(state.get_line(state.line)).map(|_| ())
    }

    fn run(state: &mut BlockState) -> Option<(Node, usize)> {
        Self::check(state)?;
        let start_line = state.line;
        let (label, after_colon) = {
            let (label, after) = definition_start(state.get_line(start_line))?;
            (label.to_string(), after)
        };
        state
            .root_ext
            .get_or_insert_default::<Footnotes>()
            .refs
            .entry(label.clone())
            .or_insert(None);

        // The note's first line starts after the colon; the lines that
        // follow belong to it while they are indented by four.
        let old_offsets = state.line_offsets[start_line].clone();
        let content_start = old_offsets.first_nonspace + after_colon;
        let line_end = old_offsets.line_end;
        let mut pos = content_start;
        let mut indent = 0;
        for c in state.src[content_start..line_end].chars() {
            match c {
                ' ' => indent += 1,
                '\t' => indent += 4 - indent % 4,
                _ => break,
            }
            pos += 1;
        }
        state.line_offsets[start_line].line_start = content_start;
        state.line_offsets[start_line].first_nonspace = pos;
        state.blk_indent += 4;
        let indent = (indent as usize) as i32;
        state.line_offsets[start_line].indent_nonspace = if indent < state.blk_indent as i32 {
            indent + state.blk_indent as i32
        } else {
            indent
        };

        let old_node = std::mem::replace(&mut state.node, Node::new(Definition { label }));
        state.md.block.tokenize(state);
        let node = std::mem::replace(&mut state.node, old_node);
        let consumed = state.line - start_line;

        state.blk_indent -= 4;
        state.line_offsets[start_line] = old_offsets;
        state.line = start_line;
        Some((node, consumed.max(1)))
    }
}

/// `parseLinkLabel`: the offset of the `]` that closes the bracket at
/// `start`, skipping over whatever the inline rules take as one token.
fn label_end(state: &mut InlineState, start: usize) -> Option<usize> {
    let old_pos = state.pos;
    let mut level = 1;
    let mut found = None;
    state.pos = start + 1;
    while let Some(c) = state.src[state.pos..state.pos_max].chars().next() {
        if c == ']' {
            level -= 1;
            if level == 0 {
                found = Some(state.pos);
                break;
            }
        }
        let before = state.pos;
        state.md.inline.skip_token(state);
        if c == '[' && before == state.pos - 1 {
            level += 1;
        }
    }
    state.pos = old_pos;
    found
}

struct InlineNoteScanner;
impl InlineRule for InlineNoteScanner {
    const MARKER: char = '^';

    fn run(state: &mut InlineState) -> Option<(Node, usize)> {
        if !enabled(state.md) {
            return None;
        }
        let start = state.pos;
        if start + 2 >= state.pos_max || !state.src[start..state.pos_max].starts_with("^[") {
            return None;
        }
        let label_start = start + 2;
        let end = label_end(state, start + 1)?;

        let footnotes = state.root_ext.get_or_insert_default::<Footnotes>();
        let id = footnotes.list.len();
        footnotes.list.push(Note {
            label: None,
            count: 0,
        });

        // Discourse's change to the library: an unclosed html link right
        // before the note is closed first.
        let after_open_link = state.node.children.last().is_some_and(|last| {
            last.cast::<HtmlInline>()
                .map(|h| h.content.as_str())
                .or(last.cast::<Text>().map(|t| t.content.as_str()))
                .is_some_and(|content| content.contains("<a"))
        });

        // The note's text, parsed as inline content of its own.
        let reference = Node::new(Reference {
            id,
            sub_id: 0,
            inline: true,
        });
        let outer = std::mem::replace(&mut state.node, reference);
        let (old_pos, old_max) = (state.pos, state.pos_max);
        state.pos = label_start;
        state.pos_max = end;
        state.md.inline.tokenize(state);
        state.pos = old_pos;
        state.pos_max = old_max;
        let reference = std::mem::replace(&mut state.node, outer);

        let node = if after_open_link {
            let mut both = Node::new(Holder);
            both.children.push(Node::new(RawHtml("</a>".into())));
            both.children.push(reference);
            both
        } else {
            reference
        };
        Some((node, end + 1 - start))
    }
}

struct ReferenceScannerRule;
impl InlineRule for ReferenceScannerRule {
    const MARKER: char = '[';

    fn run(state: &mut InlineState) -> Option<(Node, usize)> {
        if !enabled(state.md) {
            return None;
        }
        let text = &state.src[state.pos..state.pos_max];
        if text.len() < 3 || !text.starts_with("[^") {
            return None;
        }
        let rest = &text[2..];
        let end = rest.find([' ', '\n', ']'])?;
        if !rest[end..].starts_with(']') || end == 0 {
            return None;
        }
        let label = rest[..end].to_string();
        let footnotes = state.root_ext.get_mut::<Footnotes>()?;
        let slot = *footnotes.refs.get(&label)?;
        let id = match slot {
            Some(id) => id,
            None => {
                let id = footnotes.list.len();
                footnotes.list.push(Note {
                    label: Some(label.clone()),
                    count: 0,
                });
                footnotes.refs.insert(label, Some(id));
                id
            }
        };
        let sub_id = footnotes.list[id].count;
        footnotes.list[id].count += 1;
        Some((
            Node::new(Reference {
                id,
                sub_id,
                inline: false,
            }),
            2 + end + 1,
        ))
    }
}

/// The list of notes after the document.
#[derive(Debug)]
struct NoteList;
impl NodeValue for NoteList {
    fn render(&self, node: &Node, fmt: &mut dyn Renderer) {
        fmt.cr();
        fmt.text_raw("<hr class=\"footnotes-sep\">\n<section class=\"footnotes\">\n<ol class=\"footnotes-list\">\n");
        fmt.contents(&node.children);
        fmt.text_raw("</ol>\n</section>\n");
    }
}

#[derive(Debug)]
struct NoteItem {
    id: usize,
    /// The back-links, written at the end of the note's last paragraph.
    anchors: String,
}

impl NodeValue for NoteItem {
    fn render(&self, node: &Node, fmt: &mut dyn Renderer) {
        fmt.text_raw(&format!(
            "<li id=\"fn{}\" class=\"footnote-item\">",
            self.id + 1
        ));
        let last = node.children.len().saturating_sub(1);
        let ends_with_paragraph = node.children.last().is_some_and(|c| c.is::<Paragraph>());
        for (i, child) in node.children.iter().enumerate() {
            // markdown-it writes no newline between the item and a
            // paragraph that starts it.
            if child.is::<Paragraph>() {
                fmt.text_raw("<p>");
                fmt.contents(&child.children);
                if i == last {
                    fmt.text_raw(&self.anchors);
                }
                fmt.text_raw("</p>\n");
            } else {
                fmt.contents(std::slice::from_ref(child));
            }
        }
        if !ends_with_paragraph {
            fmt.text_raw(&self.anchors);
        }
        fmt.text_raw("</li>\n");
    }
}

/// `footnote_tail`: takes the definitions out of the document and appends
/// the list of the notes that are referenced.
pub fn tail(root: &mut Node) {
    use markdown_it::parser::core::Root;
    let Some(footnotes) = root
        .cast_mut::<Root>()
        .and_then(|r| r.ext.remove::<Footnotes>())
    else {
        return;
    };

    // The definitions' blocks, by label.
    let mut definitions: HashMap<String, Vec<Node>> = HashMap::new();
    fn take_definitions(node: &mut Node, out: &mut HashMap<String, Vec<Node>>) {
        let mut i = 0;
        while i < node.children.len() {
            if let Some(label) = node.children[i]
                .cast::<Definition>()
                .map(|d| d.label.clone())
            {
                let mut definition = node.children.remove(i);
                out.insert(label, std::mem::take(&mut definition.children));
            } else {
                take_definitions(&mut node.children[i], out);
                i += 1;
            }
        }
    }
    take_definitions(root, &mut definitions);

    // The inline notes' content, by id.
    let mut inline: HashMap<usize, (Option<SourcePos>, Vec<Node>)> = HashMap::new();
    root.walk_mut(|node, _| {
        if let Some((id, true)) = node.cast::<Reference>().map(|r| (r.id, r.inline)) {
            inline.insert(id, (node.srcmap, std::mem::take(&mut node.children)));
        }
    });
    let source = root
        .cast::<Root>()
        .map(|r| r.content.clone())
        .unwrap_or_default();

    if footnotes.list.is_empty() {
        return;
    }
    let mut list = Node::new(NoteList);
    for (id, note) in footnotes.list.iter().enumerate() {
        let n = id + 1;
        let anchors: String = (0..note.count.max(1))
            .map(|sub| {
                let target = if sub > 0 {
                    format!("{n}:{sub}")
                } else {
                    n.to_string()
                };
                format!(
                    " <a href=\"#fnref{target}\" class=\"footnote-backref\">\u{21a9}\u{FE0E}</a>"
                )
            })
            .collect();
        let mut item = Node::new(NoteItem { id, anchors });
        match &note.label {
            Some(label) => item.children = definitions.remove(label).unwrap_or_default(),
            None => {
                let mut paragraph = Node::new(Paragraph);
                let (srcmap, children) = inline.remove(&id).unwrap_or_default();
                paragraph.children = children;
                // Its own inline block in JS, quoted by the note's source.
                if let Some((start, end)) = srcmap.map(|s| s.get_byte_offsets()) {
                    let text = source.get(start..end).unwrap_or_default();
                    super::smartquotes::keep_block(&mut paragraph, start..end, text);
                }
                item.children.push(paragraph);
            }
        }
        list.children.push(item);
    }
    root.children.push(list);
}

pub fn add(md: &mut MarkdownIt) {
    md.block
        .add_rule::<DefinitionScanner>()
        .before::<ReferenceScanner>();
    // Added last, so the link and image rules get a bracket first.
    md.inline.add_rule::<InlineNoteScanner>();
    md.inline.add_rule::<ReferenceScannerRule>();
}

pub fn allow(list: &mut AllowList) {
    list.allow(&[
        "ol.footnotes-list",
        "hr.footnotes-sep",
        "li.footnote-item",
        "a.footnote-backref",
        "sup.footnote-ref",
    ]);
    list.allow_custom(|tag, name, value| {
        // `/^fn(ref)?\d+$/`
        (tag == "a" || tag == "li")
            && name == "id"
            && value
                .strip_prefix("fn")
                .map(|rest| rest.strip_prefix("ref").unwrap_or(rest))
                .is_some_and(|digits| {
                    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
                })
    });
}
