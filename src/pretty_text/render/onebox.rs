//! features/onebox.js: a linkified url alone on its line in a top-level
//! paragraph is a onebox candidate; elsewhere, one that points past a
//! site's front page is an inline onebox. Rails has no onebox cache while
//! cooking, so the links are only marked (`onebox`, or
//! `inline-onebox-loading`); the post processor fills them in.

use markdown_it::Node;
use markdown_it::parser::core::Root;
use markdown_it::plugins::cmark::block::paragraph::Paragraph;
use markdown_it::plugins::cmark::inline::newline::{Hardbreak, Softbreak};

use super::linkify::Linkified;
use super::newline::LeadingSpace;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Onebox,
    Inline,
}

fn is_break(node: &Node) -> bool {
    node.is::<Softbreak>() || node.is::<Hardbreak>()
}

/// `isTopLevel(href)`: nothing after `scheme://host/` (or `host?`).
fn is_top_level(href: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    let after_scheme = ["https://", "http://"]
        .iter()
        .filter_map(|scheme| lower.find(scheme).map(|at| at + scheme.len()))
        .min();
    let Some(start) = after_scheme else {
        return true;
    };
    let rest = &href[start..];
    // `[^\/]+[\/?]`: up to the first slash, else back to the last `?`.
    let extra = match rest.find('/') {
        Some(slash) if slash > 0 => Some(&rest[slash + 1..]),
        Some(_) => None,
        None => rest
            .rfind('?')
            .filter(|at| *at > 0)
            .map(|at| &rest[at + 1..]),
    };
    extra.is_none_or(str::is_empty)
}

/// The custom paragraph rule's `leading_space`: the paragraph's first
/// line starts with whitespace.
fn paragraph_leading_space(source: &str, paragraph: &Node) -> bool {
    let Some(map) = paragraph.srcmap else {
        return false;
    };
    let (start, _) = map.get_byte_offsets();
    let line_start = source[..start.min(source.len())]
        .rfind('\n')
        .map_or(0, |at| at + 1);
    let mut leading = false;
    for c in source[line_start..].chars() {
        if c == '\n' {
            leading = false;
        } else if c.is_whitespace() {
            leading = true;
        } else {
            break;
        }
    }
    leading
}

/// Walks one inline container in document order. `first` and `last` say
/// whether the container's own open and close tags are the paragraph's,
/// i.e. whether a link at either end has nothing but the paragraph around
/// it.
fn visit(children: &mut [Node], mode: &mut Mode, top: bool, leading_space: bool) {
    let count = children.len();
    for i in 0..count {
        if children[i].is::<Linkified>() {
            // What comes before: the paragraph's start, a break that does
            // not open an indented line, or anything else.
            if i == 0 {
                if !top || leading_space {
                    *mode = Mode::Inline;
                }
            } else {
                let before = &children[i - 1];
                if !is_break(before) || before.ext.contains::<LeadingSpace>() {
                    *mode = Mode::Inline;
                }
            }
            // What comes after: nothing, or a break.
            if i + 1 < count {
                if !is_break(&children[i + 1]) {
                    *mode = Mode::Inline;
                }
            } else if !top {
                *mode = Mode::Inline;
            }
            let link = children[i].cast_mut::<Linkified>().unwrap();
            let lower = link.url.to_ascii_lowercase();
            if !(lower.starts_with("http") || lower.starts_with("//")) {
                continue;
            }
            match *mode {
                Mode::Onebox => {
                    link.class = Some("onebox");
                    link.target_blank = true;
                }
                Mode::Inline if !is_top_level(&link.url) => {
                    link.class = Some("inline-onebox-loading");
                }
                Mode::Inline => {}
            }
        } else if !children[i].children.is_empty() {
            visit(&mut children[i].children, mode, false, false);
        }
    }
}

/// The `onebox` core rule, right after linkify.
pub fn run(root: &mut Node) {
    let source = root
        .cast::<Root>()
        .map(|r| r.content.clone())
        .unwrap_or_default();
    fn blocks(node: &mut Node, source: &str, depth: u32) {
        let has_links = node.children.iter().any(|c| {
            let mut found = false;
            c.walk(|n, _| found |= n.is::<Linkified>());
            found
        });
        if !has_links {
            return;
        }
        if node.is::<Paragraph>() {
            // Only a paragraph at the top of the document can hold a
            // onebox.
            let mut mode = if depth == 1 {
                Mode::Onebox
            } else {
                Mode::Inline
            };
            let leading = paragraph_leading_space(source, node);
            visit(&mut node.children, &mut mode, true, leading);
            return;
        }
        let inline_container = node.children.iter().any(|c| c.is::<Linkified>());
        if inline_container {
            let mut mode = Mode::Inline;
            visit(&mut node.children, &mut mode, true, false);
            return;
        }
        for child in node.children.iter_mut() {
            blocks(child, source, depth + 1);
        }
    }
    blocks(root, &source, 0);
}

#[cfg(test)]
mod tests {
    use super::is_top_level;

    #[test]
    fn top_level_urls() {
        assert!(is_top_level("http://www.example.net"));
        assert!(is_top_level("https://example.com/"));
        assert!(!is_top_level("https://example.com/path?q=1"));
        assert!(!is_top_level("https://example.com?q=1"));
        assert!(is_top_level("//example.com/x"));
    }
}
