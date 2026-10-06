//! features/bbcode-block.js and bbcode-inline.js: `[tag attr=value]...[/tag]`
//! as a block around other blocks, or inline. The tags themselves come
//! from the features that register them: quote, code, excerpt, wrap, grid,
//! b/i/u/s, email, img, and the bundled plugins' details and spoiler.
//!
//! A tag that a bundled plugin registers but that is not ported here is
//! refused rather than left as text, since Rails would have rendered it.

use std::sync::LazyLock;

use markdown_it::parser::block::{BlockRule, BlockState};
use markdown_it::parser::inline::{InlineRule, InlineState, Text};
use markdown_it::plugins::cmark::block::fence::{CodeFence, FenceScanner};
use markdown_it::plugins::cmark::block::paragraph::Paragraph;
use markdown_it::{MarkdownIt, Node, NodeValue, Renderer};
use regex::Regex;

use super::context::Context;
use super::element::{BlockText, Element, Holder};
use super::{RenderSettings, poll, quotes, untrimmed};

/// `QUOTATION_MARKS`, as opening and closing characters.
const QUOTATION_MARKS: [(char, char); 9] = [
    ('"', '"'),
    ('\'', '\''),
    ('«', '»'),
    ('“', '”'),
    ('”', '”'),
    ('‘', '’'),
    ('„', '“'),
    ('‚', '’'),
    ('‹', '›'),
];

fn all_quotation_marks() -> String {
    QUOTATION_MARKS.iter().flat_map(|(a, b)| [*a, *b]).collect()
}

static CLOSING_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\[/([-0-9A-Za-z_]+)\]").unwrap());

/// The old `[quote=name, post:1]` form, without quotation marks.
static QUOTE_OR_DETAILS_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)^\[(quote|details)=(\s*[^{}].+?)\]",
        regex::escape(&all_quotation_marks())
    ))
    .unwrap()
});

/// A valid opening tag, up to but not including its `]`.
static OPENING_TAG: LazyLock<Regex> = LazyLock::new(|| {
    let quoted: Vec<String> = QUOTATION_MARKS
        .iter()
        .map(|(a, b)| format!("{a}[^{b}]+{b}"))
        .collect();
    Regex::new(&format!(
        r"(?i)\[(?:(?:[-0-9A-Za-z_]+(?:=(?:{}|[^\s\]]+))?)+\s*)+",
        quoted.join("|")
    ))
    .unwrap()
});

/// `key=value` pairs of an opening tag, the value possibly quoted.
static ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| {
    let quoted: Vec<String> = QUOTATION_MARKS
        .iter()
        .map(|(a, b)| format!("{a}([^{b}]+){b}"))
        .collect();
    Regex::new(&format!(
        r"(?i)([-0-9A-Za-z_]+)(?:=(?:{}|([^\s\]]+)))?",
        quoted.join("|")
    ))
    .unwrap()
});

/// What `parseBBCodeTag` returns.
#[derive(Debug, Clone)]
pub struct TagInfo {
    pub tag: String,
    pub closing: bool,
    /// Bytes of source the tag takes, brackets included.
    pub length: usize,
    /// In the order written; the tag's own value under `_default`.
    pub attrs: Vec<(String, String)>,
}

impl TagInfo {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// `trailingSpaceOnly`: nothing but whitespace up to the end of the line.
fn trailing_space_only(rest: &str) -> bool {
    for c in rest.chars() {
        if c == '\n' {
            return true;
        }
        if !c.is_whitespace() {
            return false;
        }
    }
    true
}

/// `parseBBCodeTag(src, start, max, multiline)` on `text = src[start..max]`.
pub fn parse_tag(text: &str, multiline: bool) -> Option<TagInfo> {
    // A closing tag never has attributes.
    if let Some(m) = CLOSING_TAG.captures(text) {
        let whole = m.get(0).unwrap();
        if multiline && !trailing_space_only(&text[whole.end()..]) {
            return None;
        }
        return Some(TagInfo {
            tag: m[1].to_lowercase(),
            closing: true,
            length: whole.end(),
            attrs: Vec::new(),
        });
    }
    if let Some(m) = QUOTE_OR_DETAILS_TAG.captures(text) {
        let whole = m.get(0).unwrap();
        if multiline && !trailing_space_only(&text[whole.end()..]) {
            return None;
        }
        return Some(TagInfo {
            // As written: `[QUOTE=x]` finds no rule.
            tag: m[1].to_string(),
            closing: false,
            length: whole.end(),
            attrs: vec![("_default".to_string(), m[2].to_string())],
        });
    }
    // The pattern is not anchored: its first match anywhere is taken, then
    // read as if it started the text.
    let bbcode = OPENING_TAG.find(text)?.as_str();
    if text.len() <= bbcode.len() || !text[bbcode.len()..].starts_with(']') {
        return None;
    }
    let mut tag: Option<String> = None;
    let mut attrs: Vec<(String, String)> = Vec::new();
    for m in ATTRIBUTE.captures_iter(bbcode) {
        let key = m.get(1)?.as_str();
        let value = (2..m.len()).find_map(|i| m.get(i)).map(|v| v.as_str());
        if tag.is_none() {
            // The tag's name sits right after the bracket.
            if m.get(0).unwrap().start() != 1 {
                return None;
            }
            tag = Some(key.to_lowercase());
            if let Some(value) = value {
                attrs.push(("_default".to_string(), value.trim().to_string()));
            }
        } else {
            let value = value.map(str::trim).unwrap_or("").to_string();
            match attrs.iter_mut().find(|(k, _)| k == key) {
                Some(entry) => entry.1 = value,
                None => attrs.push((key.to_string(), value)),
            }
        }
    }
    let tag = tag?;
    let length = bbcode.len() + 1;
    if multiline && !trailing_space_only(&text[length..]) {
        return None;
    }
    Some(TagInfo {
        tag,
        closing: false,
        length,
        attrs,
    })
}

/// `applyDataAttributes`: the attributes as `data-*`, sorted by key, the
/// tag's own value under `default_name`.
pub fn data_attributes(
    attrs: &[(String, String)],
    default_name: Option<&str>,
) -> Vec<(String, String)> {
    let mut named: Vec<(String, String)> = Vec::new();
    let mut set = |key: String, value: String| match named.iter_mut().find(|(k, _)| *k == key) {
        Some(entry) => entry.1 = value,
        None => named.push((key, value)),
    };
    for (key, value) in attrs.iter().filter(|(k, _)| k != "_default") {
        set(key.clone(), value.clone());
    }
    if let (Some(name), Some((_, value))) = (
        default_name,
        attrs.iter().find(|(k, v)| k == "_default" && !v.is_empty()),
    ) {
        set(name.to_string(), value.clone());
    }
    named.sort_by(|a, b| a.0.cmp(&b.0));
    named
        .into_iter()
        .filter_map(|(key, value)| {
            // `key.replace(/[^a-z0-9-]/gi, "")`, then camelCaseToDash.
            let clean: String = key
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            let mut dashed = String::new();
            let chars: Vec<char> = clean.chars().collect();
            for (i, c) in chars.iter().enumerate() {
                // A dash after any letter that an uppercase one follows.
                if c.is_ascii_uppercase() && i > 0 && chars[i - 1].is_ascii_alphabetic() {
                    dashed.push('-');
                }
                dashed.push(c.to_ascii_lowercase());
            }
            // The value goes through escapeHtml here and again when the
            // attribute is written.
            let value = markdown_it::common::utils::escape_html(&value).into_owned();
            (!value.is_empty() && dashed.len() > 1).then(|| (format!("data-{dashed}"), value))
        })
        .collect()
}

/// The block tags, by who registers them.
enum BlockTag {
    Excerpt,
    Code,
    Quote,
    Wrap,
    Grid,
    Details,
    Spoiler,
    Poll,
}

/// Tags the bundled plugins register and this port does not render.
const UNPORTED_BLOCK_TAGS: [&str; 8] = [
    "chat",
    "calendar",
    "timezones",
    "event",
    "preview",
    "hidden",
    "graphviz",
    "policy",
];

fn block_tag(tag: &str, settings: &RenderSettings, ctx: &Context) -> Option<BlockTag> {
    Some(match tag {
        "excerpt" => BlockTag::Excerpt,
        "code" => BlockTag::Code,
        "quote" => BlockTag::Quote,
        "wrap" => BlockTag::Wrap,
        "grid" => BlockTag::Grid,
        "details" => BlockTag::Details,
        "spoiler" if settings.spoiler => BlockTag::Spoiler,
        "poll" if settings.poll => BlockTag::Poll,
        _ => {
            if UNPORTED_BLOCK_TAGS.contains(&tag) {
                ctx.refuse("a bbcode block of a plugin that is not ported (chat, events, graphviz, policy)");
            }
            return None;
        }
    })
}

/// Where a block's closing tag is.
struct CloseTag {
    /// The line of a closing tag on its own line.
    line: Option<usize>,
    /// The byte offset in the source of one on the opening line.
    start: usize,
    length: usize,
}

/// `findInlineCloseTag`: a closing tag that ends the opening line.
fn find_inline_close(src: &str, tag: &str, start: usize, max: usize) -> Option<CloseTag> {
    let mut possible = false;
    let indices: Vec<(usize, char)> = src[start..max].char_indices().collect();
    // `for (j = max - 1; j > start; j--)`
    for (offset, c) in indices.into_iter().rev() {
        if offset == 0 {
            break;
        }
        if !possible {
            if c == ']' {
                possible = true;
                continue;
            }
            if !c.is_whitespace() {
                break;
            }
        } else if c == '[' {
            let at = start + offset;
            if let Some(info) = parse_tag(&src[at..max], false)
                && info.closing
                && info.tag == tag
            {
                return Some(CloseTag {
                    line: None,
                    start: at,
                    length: info.length,
                });
            }
        }
    }
    None
}

/// `findBlockCloseTag`: the matching closing tag on a line of its own,
/// counting nested tags of the same name.
fn find_block_close(state: &BlockState, tag: &str, start_line: usize) -> Option<CloseTag> {
    let mut nesting = 0;
    let mut line = start_line;
    loop {
        // An unclosed block is not closed by the end of the document.
        if line >= state.line_max {
            return None;
        }
        let offsets = &state.line_offsets[line];
        let (start, max) = (offsets.first_nonspace, offsets.line_end);
        if start < max && state.line_indent(line) < 0 {
            return None;
        }
        if state.src[start..max].starts_with('[')
            && state.line_indent(line) < 4
            && let Some(info) = parse_tag(&state.src[start..max], true)
            && info.tag == tag
        {
            if info.closing {
                if nesting == 0 {
                    return Some(CloseTag {
                        line: Some(line),
                        start,
                        length: info.length,
                    });
                }
                nesting -= 1;
            } else {
                nesting += 1;
            }
        }
        line += 1;
    }
}

struct BlockBbcode;
impl BlockRule for BlockBbcode {
    fn check(state: &mut BlockState) -> Option<()> {
        let line = state.get_line(state.line);
        if !line.starts_with('[') {
            return None;
        }
        let info = parse_tag(line, false)?;
        if info.closing {
            return None;
        }
        let settings = state.md.ext.get::<RenderSettings>()?;
        let ctx = state.md.ext.get::<Context>()?;
        block_tag(&info.tag, settings, ctx).map(|_| ())
    }

    fn run(state: &mut BlockState) -> Option<(Node, usize)> {
        let start_line = state.line;
        let offsets = state.line_offsets[start_line].clone();
        let (start, max) = (offsets.first_nonspace, offsets.line_end);
        let src = state.src;
        if !src[start..max].starts_with('[') {
            return None;
        }
        let info = parse_tag(&src[start..max], false)?;
        if info.closing {
            return None;
        }
        let md = state.md;
        let settings = md.ext.get::<RenderSettings>()?;
        let ctx = md.ext.get::<Context>()?;
        let kind = block_tag(&info.tag, settings, ctx)?;

        // A closing tag at the end of the same line, else one further down.
        let close = match find_inline_close(src, &info.tag, start + info.length, max) {
            Some(close) => close,
            None => {
                if !trailing_space_only(&src[start + info.length..max]) {
                    return None;
                }
                find_block_close(state, &info.tag, start_line + 1)?
            }
        };
        // Content after an inline closing tag: the inline rules' case.
        if close.line.is_none() && close.start + close.length < max {
            return None;
        }
        let next_line = close.line.unwrap_or(start_line);
        let lines = next_line + 1 - start_line;

        if let BlockTag::Code = kind {
            let content = if close.line.is_none() {
                src[start + info.length..close.start].to_string()
            } else {
                state.get_lines(start_line + 1, next_line, 0, false).0
            };
            let node = Node::new(CodeFence {
                info: String::new(),
                marker: '`',
                marker_len: 3,
                content,
                lang_prefix: "language-",
            });
            return Some((node, lines));
        }

        // The content: the lines between the tags as blocks, or what sits
        // between them on the one line as a paragraph.
        // A poll inside a poll stays plain content.
        let nested_poll = if let BlockTag::Poll = kind {
            let depth = state.root_ext.get_or_insert_default::<poll::PollDepth>();
            depth.0 += 1;
            depth.0 > 1
        } else {
            false
        };
        let mut holder = Node::new(Holder);
        if close.line.is_some() {
            let old_node = std::mem::replace(&mut state.node, holder);
            let old_line_max = state.line_max;
            state.line = start_line + 1;
            state.line_max = next_line;
            md.block.tokenize(state);
            state.line = start_line;
            state.line_max = old_line_max;
            holder = std::mem::replace(&mut state.node, old_node);
        } else {
            let content_start = start + info.length;
            // Untrimmed, as JS hands it to the inline parser.
            let mut paragraph = Node::new(Paragraph);
            paragraph.children = untrimmed::inline(
                src[content_start..close.start].to_string(),
                vec![(0, content_start)],
            );
            holder.children.push(paragraph);
        }
        let children = std::mem::take(&mut holder.children);
        if let BlockTag::Poll = kind {
            state.root_ext.get_or_insert_default::<poll::PollDepth>().0 -= 1;
        }

        let wrap = |tag: &str, attrs: Vec<(String, String)>, children: Vec<Node>| {
            let mut node = Node::new(Element {
                tag: tag.to_string(),
                attrs,
                block: true,
            });
            node.children = children;
            node
        };
        let class = |name: &str| vec![("class".to_string(), name.to_string())];
        let node = match kind {
            BlockTag::Code => unreachable!("handled above"),
            BlockTag::Excerpt => wrap("div", class("excerpt"), children),
            BlockTag::Spoiler => wrap("div", class("spoiler"), children),
            BlockTag::Quote => quotes::build(&info, children, settings, ctx),
            BlockTag::Poll => poll::build(&info, children, nested_poll, settings, ctx),
            BlockTag::Wrap => {
                let mut attrs = class("d-wrap");
                attrs.extend(data_attributes(&info.attrs, Some("wrap")));
                wrap("div", attrs, children)
            }
            BlockTag::Grid => {
                let mut attrs = class("d-image-grid");
                // Only `mode`, and only grid or carousel.
                if let Some(mode) = info.attr("mode").filter(|m| !m.is_empty()) {
                    let mode = if matches!(mode, "grid" | "carousel") {
                        mode
                    } else {
                        "grid"
                    };
                    attrs.extend(data_attributes(
                        &[("mode".to_string(), mode.to_string())],
                        None,
                    ));
                }
                wrap("div", attrs, children)
            }
            BlockTag::Details => {
                let mut attrs = Vec::new();
                if info.attr("open") == Some("") {
                    attrs.push(("open".to_string(), String::new()));
                }
                let mut summary = Element::block("summary", &[]);
                summary.children.push(Node::new(BlockText(
                    info.attr("_default").unwrap_or("").to_string(),
                )));
                let mut all = vec![summary];
                all.extend(children);
                wrap("details", attrs, all)
            }
        };
        Some((node, lines))
    }
}

/// An inline tag that wraps what is between it and its closing tag
/// (`[b]`, `[spoiler]`): a delimiter until the pass below pairs it.
#[derive(Debug)]
struct Delimiter {
    tag: String,
    closing: bool,
    /// The tag as written, which is what an unpaired one stays.
    raw: String,
}

impl NodeValue for Delimiter {
    fn render(&self, _: &Node, fmt: &mut dyn Renderer) {
        fmt.text(&self.raw);
    }
}

/// The class of an inline wrapping tag.
fn inline_wrap(tag: &str, settings: &RenderSettings) -> Option<&'static str> {
    match tag {
        "b" => Some("bbcode-b"),
        "i" => Some("bbcode-i"),
        "u" => Some("bbcode-u"),
        "s" => Some("bbcode-s"),
        "spoiler" if settings.spoiler => Some("spoiler"),
        _ => None,
    }
}

struct InlineBbcode;
impl InlineRule for InlineBbcode {
    const MARKER: char = '[';

    fn run(state: &mut InlineState) -> Option<(Node, usize)> {
        let text = &state.src[state.pos..state.pos_max];
        if !text.starts_with('[') {
            return None;
        }
        let info = parse_tag(text, false)?;
        let settings = state.md.ext.get::<RenderSettings>()?;
        let ctx = state.md.ext.get::<Context>()?;

        if inline_wrap(&info.tag, settings).is_some() {
            return Some((
                Node::new(Delimiter {
                    tag: info.tag.clone(),
                    closing: info.closing,
                    raw: text[..info.length].to_string(),
                }),
                info.length,
            ));
        }
        if !matches!(info.tag.as_str(), "code" | "url" | "email" | "img" | "wrap") {
            return None;
        }
        // The rules that take their content raw: everything up to the
        // first closing tag.
        if info.closing {
            return None;
        }
        let close = format!("[/{}]", info.tag);
        let lower = text.to_ascii_lowercase();
        let end = lower[info.length..].find(&close)? + info.length;
        let content = &text[info.length..end];
        let length = end + close.len();
        let node = match info.tag.as_str() {
            "code" => {
                let mut code = Element::inline("code", &[]);
                code.children
                    .push(Node::new(BlockText(content.to_string())));
                code
            }
            "wrap" => {
                let mut attrs = vec![("class".to_string(), "d-wrap".to_string())];
                attrs.extend(data_attributes(&info.attrs, Some("wrap")));
                let mut span = Node::new(Element {
                    tag: "span".to_string(),
                    attrs,
                    block: false,
                });
                if !content.is_empty() {
                    span.children.push(Node::new(Text {
                        content: content.to_string(),
                    }));
                }
                span
            }
            "email" => {
                let email = info
                    .attr("_default")
                    .filter(|e| !e.is_empty())
                    .unwrap_or(content);
                let mut link = Element::inline(
                    "a",
                    &[
                        ("href", &format!("mailto:{email}")),
                        ("data-bbcode", "true"),
                    ],
                );
                link.children.push(Node::new(Text {
                    content: content.to_string(),
                }));
                link
            }
            "img" => {
                if content.starts_with("upload://") {
                    ctx.refuse("[img] bbcode with an upload:// url");
                }
                // An image token without children: no alt text, so the
                // image renderer marks it as presentation.
                Node::new(super::element::RawHtml(format!(
                    "<img src=\"{}\" alt=\"\" role=\"presentation\">",
                    markdown_it::common::utils::escape_html(content)
                )))
            }
            _ => {
                ctx.refuse("[url] bbcode");
                return None;
            }
        };
        Some((node, length))
    }
}

/// `processBBCode`: pairs the wrapping delimiters among siblings, each
/// closing tag with the nearest open one of its kind, and wraps what is
/// between them. Unpaired ones become the text they were.
pub fn pair(root: &mut Node, settings: &RenderSettings) {
    fn visit(node: &mut Node, settings: &RenderSettings) {
        let mut i = 0;
        while i < node.children.len() {
            let closing = node.children[i]
                .cast::<Delimiter>()
                .filter(|d| d.closing)
                .map(|d| d.tag.clone());
            if let Some(tag) = closing {
                let open = (0..i).rev().find(|&j| {
                    node.children[j]
                        .cast::<Delimiter>()
                        .is_some_and(|d| !d.closing && d.tag == tag)
                });
                if let (Some(open), Some(class)) = (open, inline_wrap(&tag, settings)) {
                    let mut inner: Vec<Node> = node.children.drain(open..=i).collect();
                    inner.pop();
                    inner.remove(0);
                    let mut span = Element::inline("span", &[("class", class)]);
                    span.children = inner;
                    node.children.insert(open, span);
                    i = open + 1;
                    continue;
                }
            }
            i += 1;
        }
        for child in node.children.iter_mut() {
            if let Some(delimiter) = child.cast::<Delimiter>() {
                let content = delimiter.raw.clone();
                child.replace(Text { content });
            } else {
                visit(child, settings);
            }
        }
    }
    visit(root, settings);
}

pub fn add(md: &mut MarkdownIt) {
    md.block.add_rule::<BlockBbcode>().after::<FenceScanner>();
    md.inline.add_rule::<InlineBbcode>();
}

pub fn allow(list: &mut crate::pretty_text::sanitizer::AllowList, settings: &RenderSettings) {
    list.allow(&[
        "span.bbcode-b",
        "span.bbcode-i",
        "span.bbcode-u",
        "span.bbcode-s",
        // d-wrap
        "div.d-wrap",
        "span.d-wrap",
        "span[data-*]",
        // image-grid
        "div.d-image-grid",
        "div.d-image-grid[data-mode]",
        // discourse-details
        "summary",
        "summary[title]",
        "details",
        "details[open]",
        "details.elided",
    ]);
    if settings.spoiler {
        list.allow(&["span.spoiler", "div.spoiler"]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tags() {
        let close = parse_tag("[/quote] rest", false).unwrap();
        assert!(close.closing && close.tag == "quote" && close.length == 8);
        assert!(parse_tag("[/quote] rest", true).is_none());

        let old = parse_tag("[quote=user1, post:2, topic:35]", false).unwrap();
        assert_eq!(old.tag, "quote");
        assert_eq!(old.attr("_default"), Some("user1, post:2, topic:35"));

        let quoted = parse_tag("[quote=\"user1, post:2\"]text", false).unwrap();
        assert_eq!(quoted.attr("_default"), Some("user1, post:2"));
        assert_eq!(quoted.length, "[quote=\"user1, post:2\"]".len());

        let wrap = parse_tag("[wrap=foo bar=1]", false).unwrap();
        assert_eq!(wrap.tag, "wrap");
        assert_eq!(
            wrap.attrs,
            vec![
                ("_default".into(), "foo".into()),
                ("bar".into(), "1".into())
            ]
        );

        assert!(parse_tag("[not closed", false).is_none());
        assert!(parse_tag("[ spaced]", false).is_none());
    }

    #[test]
    fn data_attributes_are_sorted_and_dashed() {
        let attrs = vec![
            ("_default".to_string(), "foo".to_string()),
            ("someKey".to_string(), "1".to_string()),
            ("a".to_string(), "dropped".to_string()),
            ("bar".to_string(), "<x>".to_string()),
        ];
        assert_eq!(
            data_attributes(&attrs, Some("wrap")),
            vec![
                ("data-bar".to_string(), "&lt;x&gt;".to_string()),
                ("data-some-key".to_string(), "1".to_string()),
                ("data-wrap".to_string(), "foo".to_string()),
            ]
        );
    }
}
