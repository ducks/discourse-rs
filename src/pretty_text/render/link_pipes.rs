//! features/table.js: table rows are split on `|` before inline markdown
//! is parsed, so a pipe inside a link or an image (`![alt|690x400](...)`)
//! would end a cell. Before parsing, those pipes become a NUL, which is
//! the same length; after the block parser they are pipes again in what
//! the inline parser and the code blocks read.

use std::borrow::Cow;

use markdown_it::parser::block::builtin::BlockParserRule;
use markdown_it::parser::core::{CoreRule, Root};
use markdown_it::parser::extset::{InlineRootExtSet, RootExtSet};
use markdown_it::parser::inline::builtin::InlineParserRule;
use markdown_it::parser::inline::{InlineRoot, InlineState};
use markdown_it::plugins::cmark::block::code::CodeBlock;
use markdown_it::plugins::cmark::block::fence::CodeFence;
use markdown_it::plugins::html::html_block::HtmlBlock;
use markdown_it::{MarkdownIt, Node};

const PLACEHOLDER: char = '\0';

/// markdown-it's `parseLinkLabel`: where the label opened at `start` ends.
fn parse_link_label(state: &mut InlineState, start: usize, disable_nested: bool) -> Option<usize> {
    let old_pos = state.pos;
    state.pos = start + 1;
    let mut level = 1;
    let mut found = false;
    while state.pos < state.pos_max {
        let marker = state.src.as_bytes()[state.pos];
        if marker == b']' {
            level -= 1;
            if level == 0 {
                found = true;
                break;
            }
        }
        let prev_pos = state.pos;
        state.md.inline.skip_token(state);
        if marker == b'[' {
            if prev_pos == state.pos - 1 {
                level += 1;
            } else if disable_nested {
                state.pos = old_pos;
                return None;
            }
        }
    }
    let end = found.then_some(state.pos);
    state.pos = old_pos;
    end
}

/// `linkEndWithPipe`: the end of the link or image starting at `start`,
/// which `skipToken` has just passed, if it has a pipe in it.
fn link_end_with_pipe(state: &mut InlineState, start: usize, is_image: bool) -> Option<usize> {
    let inline_end = state.pos;
    let label_end = parse_link_label(state, start + usize::from(is_image), !is_image)?;
    let destination = label_end + 1;
    let end = match state.src.as_bytes().get(destination) {
        Some(b'(') if inline_end > destination + 1 => inline_end,
        // Reference definitions are not collected until block parsing.
        Some(b'[') => parse_link_label(state, destination, false)? + 1,
        _ => return None,
    };
    state.src[start..end].contains('|').then_some(end)
}

/// `protectLinePipes`
fn protect_line(md: &MarkdownIt, line: &str) -> String {
    if !line.contains('|') || !line.contains('[') {
        return line.to_string();
    }
    let mut root_ext = RootExtSet::new();
    let mut inline_ext = InlineRootExtSet::new();
    let mut state = InlineState::new(
        line.to_string(),
        vec![(0, 0)],
        md,
        &mut root_ext,
        &mut inline_ext,
        Node::default(),
    );
    let mut output = String::new();
    let mut copied_until = 0;
    while state.pos < state.pos_max {
        let start = state.pos;
        let is_image = line[start..].starts_with("![");
        md.inline.skip_token(&mut state);
        if !is_image && !line[start..].starts_with('[') {
            continue;
        }
        if let Some(end) = link_end_with_pipe(&mut state, start, is_image) {
            output.push_str(&line[copied_until..start]);
            output.push_str(&line[start..end].replace('|', "\0"));
            copied_until = end;
            state.pos = end;
        }
    }
    if copied_until == 0 {
        return line.to_string();
    }
    output.push_str(&line[copied_until..]);
    output
}

/// `protectLinkPipes`, after `normalize` has replaced the source's own
/// NULs (so every NUL left is a placeholder).
pub fn protect<'a>(md: &MarkdownIt, raw: &'a str) -> Cow<'a, str> {
    let raw: Cow<str> = if raw.contains(PLACEHOLDER) {
        Cow::Owned(raw.replace(PLACEHOLDER, "\u{FFFD}"))
    } else {
        Cow::Borrowed(raw)
    };
    if !raw.contains('|') || !raw.contains('[') {
        return raw;
    }
    let lines: Vec<String> = raw.split('\n').map(|line| protect_line(md, line)).collect();
    Cow::Owned(lines.join("\n"))
}

/// `restoreLinkPipes`: the pipes back in every block's content.
pub struct RestoreLinkPipes;

impl CoreRule for RestoreLinkPipes {
    fn run(root: &mut Node, _: &MarkdownIt) {
        let restore = |s: &mut String| {
            if s.contains(PLACEHOLDER) {
                *s = s.replace(PLACEHOLDER, "|");
            }
        };
        root.walk_mut(|node, _| {
            if let Some(root) = node.cast_mut::<Root>() {
                restore(&mut root.content);
            } else if let Some(inline) = node.cast_mut::<InlineRoot>() {
                restore(&mut inline.content);
            } else if let Some(fence) = node.cast_mut::<CodeFence>() {
                restore(&mut fence.content);
                restore(&mut fence.info);
            } else if let Some(code) = node.cast_mut::<CodeBlock>() {
                restore(&mut code.content);
            } else if let Some(html) = node.cast_mut::<HtmlBlock>() {
                restore(&mut html.content);
            }
        });
    }
}

pub fn add(md: &mut MarkdownIt) {
    md.add_rule::<RestoreLinkPipes>()
        .after::<BlockParserRule>()
        .before::<InlineParserRule>();
}
