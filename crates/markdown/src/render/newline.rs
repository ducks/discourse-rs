//! features/newline.js: markdown-it's newline rule, which also notes when
//! the next line starts with spaces. The onebox rule needs that: a link
//! on an indented line is not a onebox.

use markdown_it::parser::extset::NodeExt;
use markdown_it::parser::inline::{InlineRule, InlineState};
use markdown_it::plugins::cmark::inline::newline::{Hardbreak, NewlineScanner, Softbreak};
use markdown_it::{MarkdownIt, Node};

/// `token.leading_space` on a break.
#[derive(Debug)]
pub struct LeadingSpace;
impl NodeExt for LeadingSpace {}

struct Newline;
impl InlineRule for Newline {
    const MARKER: char = '\n';

    fn check(state: &mut InlineState) -> Option<usize> {
        state.src[state.pos..state.pos_max]
            .starts_with('\n')
            .then_some(1)
    }

    fn run(state: &mut InlineState) -> Option<(Node, usize)> {
        if !state.src[state.pos..state.pos_max].starts_with('\n') {
            return None;
        }
        // Two or more trailing spaces make a hard break; the spaces go
        // either way.
        let trailing = state
            .trailing_text_get()
            .chars()
            .rev()
            .take_while(|c| *c == ' ')
            .count();
        state.trailing_text_pop(trailing);
        let mut node = if trailing >= 2 {
            Node::new(Hardbreak)
        } else {
            Node::new(Softbreak)
        };
        // Skip the spaces that start the next line.
        let mut len = 1;
        let rest = &state.src[state.pos + 1..state.pos_max];
        let spaces = rest.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        if spaces > 0 {
            node.ext.insert(LeadingSpace);
            len += spaces;
        }
        state.pos -= trailing;
        Some((node, len + trailing))
    }
}

pub fn add(md: &mut MarkdownIt) {
    md.inline.remove_rule::<NewlineScanner>();
    md.inline.add_rule::<Newline>();
}
