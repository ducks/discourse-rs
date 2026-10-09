//! Chat's `chat-html-inline` rule (chat-html-inline.js): markdown-it's
//! html_inline narrowed to `<kbd>` and `<mark>` (and comments, processing
//! instructions, declarations and CDATA), the only raw HTML a chat message
//! keeps. Pushed after the other inline rules, as the plugin pushes it.

use markdown_it::parser::inline::{InlineRule, InlineState};
use markdown_it::plugins::html::html_inline::HtmlInline;
use markdown_it::{MarkdownIt, Node};
use regex::Regex;
use std::sync::LazyLock;

static HTML_TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    let names = "kbd|mark";
    let attr_name = "[a-zA-Z_:][a-zA-Z0-9:._-]*";
    let unquoted = "[^\"'=<>`\\x00-\\x20]+";
    let single_quoted = "'[^']*'";
    let double_quoted = "\"[^\"]*\"";
    let attr_value = format!("(?:{unquoted}|{single_quoted}|{double_quoted})");
    let attribute = format!("(?:\\s+{attr_name}(?:\\s*=\\s*{attr_value})?)");
    let open_tag = format!("<({names}){attribute}*\\s*/?>");
    let close_tag = format!("</({names})\\s*>");
    let comment = "<!---?>|<!--(?:[^-]|-[^-]|--[^>])*-->";
    let processing = "<[?][\\s\\S]*?[?]>";
    let declaration = "<![A-Za-z][^>]*>";
    let cdata = "<!\\[CDATA\\[[\\s\\S]*?\\]\\]>";
    Regex::new(&format!(
        "^(?:{open_tag}|{close_tag}|{comment}|{processing}|{declaration}|{cdata})"
    ))
    .expect("chat html inline pattern")
});

pub fn add(md: &mut MarkdownIt) {
    md.inline.add_rule::<ChatHtmlInline>();
}

struct ChatHtmlInline;
impl InlineRule for ChatHtmlInline {
    const MARKER: char = '<';

    fn run(state: &mut InlineState) -> Option<(Node, usize)> {
        let src = &state.src[state.pos..state.pos_max];
        let mut chars = src.chars();
        if chars.next()? != '<' || src.len() <= 2 {
            return None;
        }
        let second = chars.next()?;
        if !matches!(second, '!' | '?' | '/') && !second.is_ascii_alphabetic() {
            return None;
        }
        let content = HTML_TAG_RE.find(src)?.as_str().to_string();
        let length = content.len();
        Some((Node::new(HtmlInline { content }), length))
    }
}
