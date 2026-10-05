//! features/html-img.js: lines that are only an `<img>` tag are a
//! paragraph, so the images render inline (`<p><img ...></p>`), where
//! CommonMark would make them an HTML block. The rule sits before
//! html_block and cannot interrupt a paragraph (it joins only the fence
//! chain); four spaces of indent make the line code instead.

use markdown_it::parser::block::{BlockRule, BlockState};
use markdown_it::parser::inline::{InlineRoot, Text};
use markdown_it::plugins::cmark::block::paragraph::Paragraph;
use markdown_it::plugins::html::html_block::HtmlBlockScanner;
use markdown_it::{MarkdownIt, Node, NodeValue};

/// `/^<img.*\\?>\s*$/i`: the tag and nothing after it but space.
fn is_img_line(line: &str) -> bool {
    let line = line.trim_end();
    line.as_bytes()
        .get(..4)
        .is_some_and(|start| start.eq_ignore_ascii_case(b"<img"))
        && line.ends_with('>')
}

/// The line from its first non-space character, as `bMarks + tShift`.
fn line_text<'a>(state: &'a BlockState, line: usize) -> &'a str {
    let offsets = &state.line_offsets[line];
    &state.src[offsets.first_nonspace..offsets.line_end]
}

struct HtmlImgScanner;

impl BlockRule for HtmlImgScanner {
    // Interrupts nothing: a paragraph's lines are not checked against it.
    fn check(_: &mut BlockState) -> Option<()> {
        None
    }

    fn run(state: &mut BlockState) -> Option<(Node, usize)> {
        let start_line = state.line;
        if state.line_indent(start_line) >= state.md.max_indent {
            return None;
        }
        if !is_img_line(line_text(state, start_line)) {
            return None;
        }
        let mut content = String::new();
        let mut mapping = Vec::new();
        let mut next_line = start_line;
        while next_line < state.line_max {
            let text = line_text(state, next_line);
            if text.trim().is_empty() || !is_img_line(text) {
                break;
            }
            if next_line > start_line {
                content.push('\n');
            }
            mapping.push((content.len(), state.line_offsets[next_line].first_nonspace));
            content.push_str(text);
            next_line += 1;
        }
        let trailing = content[content.trim_end_matches([' ', '\t']).len()..].to_string();
        let mut node = Node::new(Paragraph);
        node.children
            .push(Node::new(InlineRoot::new(content, mapping)));
        if !trailing.is_empty() {
            node.children.push(Node::new(Trailing(trailing)));
        }
        Some((node, next_line - start_line))
    }
}

/// The space after the last tag. JS does not trim the block's content, so
/// it stays as text (`<img ...> </p>`, and in a tight item `... </li>`);
/// the crate's inline parser trims every content. It is a node beside the
/// content rather than a mark on the paragraph, which a tight list drops.
#[derive(Debug)]
pub struct Trailing(String);

impl NodeValue for Trailing {}

/// Turns each kept trailing space into the text it is in JS.
pub fn restore_trailing(root: &mut Node) {
    root.walk_mut(|node, _| {
        if let Some(Trailing(space)) = node.cast::<Trailing>() {
            let content = space.clone();
            node.replace(Text { content });
        }
    });
}

pub fn add(md: &mut MarkdownIt) {
    md.block
        .add_rule::<HtmlImgScanner>()
        .before::<HtmlBlockScanner>();
}

#[cfg(test)]
mod tests {
    use super::is_img_line;

    #[test]
    fn img_lines_as_discourse_matches_them() {
        assert!(is_img_line(r#"<img src="a.png" width="10">"#));
        assert!(is_img_line("<IMG src=x>  "));
        assert!(is_img_line(r#"<img src="a.png" />"#));
        assert!(!is_img_line(r#"<img src="a.png"> and text"#));
        assert!(!is_img_line("<image>x"));
        assert!(!is_img_line("<div>"));
        assert!(!is_img_line("aé>"));
    }
}
