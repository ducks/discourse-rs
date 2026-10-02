//! features/text-post-process.js and the rules pushed on its ruler: text
//! outside links is searched for patterns that become nodes. Mentions
//! (features/mentions.js), then hashtags
//! (features/hashtag-autocomplete.js), tried in that order at each
//! position.

use std::sync::LazyLock;

use fancy_regex::Regex;
use markdown_it::Node;
use markdown_it::common::utils::is_punct_char;
use markdown_it::parser::inline::Text;
use markdown_it::plugins::cmark::inline::link::Link;

use super::RenderSettings;
use super::context::Context;
use super::element::Element;
use crate::pretty_text::sanitizer::AllowList;

/// `mentionRegex(false)`: `\w` is ASCII in JavaScript.
const MENTION: &str = r"@([0-9A-Za-z_][0-9A-Za-z_.-]{0,58}[0-9A-Za-z])|@([0-9A-Za-z_])";
/// hashtag-autocomplete's `MATCHER`: not after a slash.
const HASHTAG: &str = r"(?<!/)#([\x{C0}-\x{1FFF}\x{2C00}-\x{D7FF}0-9A-Za-z_:-](?:[\x{C0}-\x{1FFF}\x{2C00}-\x{D7FF}0-9A-Za-z_:.-]{0,99}[\x{C0}-\x{1FFF}\x{2C00}-\x{D7FF}0-9A-Za-z_:-])?)";

/// The ruler's rules joined as alternatives: groups 1 and 2 are a
/// mention's name, group 3 a hashtag's.
static WITH_MENTIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("{MENTION}|{HASHTAG}")).unwrap());
static HASHTAGS_ONLY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("(x^)|(x^)|{HASHTAG}")).unwrap());

fn text(content: &str) -> Node {
    Node::new(Text {
        content: content.to_string(),
    })
}

/// `<span class="mention">@name</span>`; PrettyText.cleanup turns the ones
/// that name a user or group into links.
fn mention(username: &str) -> Node {
    let mut span = Element::inline("span", &[("class", "mention")]);
    span.children.push(text(&format!("@{username}")));
    span
}

/// `addHashtag`: a link with an icon placeholder for a hashtag that
/// resolves, the text in a span for one that does not.
fn hashtag(matched: &str, slug: &str, ctx: &Context) -> Node {
    let Some(found) = ctx.hashtag(slug) else {
        let mut span = Element::inline("span", &[("class", "hashtag-raw")]);
        span.children.push(text(matched));
        return span;
    };
    let mut attrs: Vec<(String, String)> = vec![
        ("class".into(), "hashtag-cooked".into()),
        ("href".into(), found.relative_url.clone()),
        ("data-type".into(), found.kind.clone()),
        ("data-slug".into(), found.slug.clone()),
        ("data-id".into(), found.id.to_string()),
    ];
    if let Some(style) = found.style_type.as_deref().filter(|s| !s.is_empty()) {
        attrs.push(("data-style-type".into(), style.to_string()));
        match (style, &found.emoji, &found.icon) {
            ("emoji", Some(emoji), _) if !emoji.is_empty() => {
                attrs.push(("data-emoji".into(), emoji.clone()));
            }
            ("icon", _, Some(icon)) if !icon.is_empty() => {
                attrs.push(("data-icon".into(), icon.clone()));
            }
            _ => {}
        }
    }
    if found.slug != found.reference {
        attrs.push(("data-ref".into(), found.reference.clone()));
    }
    let mut link = Node::new(Element {
        tag: "a".into(),
        attrs,
        block: false,
    });
    let mut placeholder = Element::inline("span", &[("class", "hashtag-icon-placeholder")]);
    let mut svg = Element::inline(
        "svg",
        &[("class", "fa d-icon d-icon-square-full svg-icon svg-node")],
    );
    svg.children
        .push(Element::inline("use", &[("href", "#square-full")]));
    placeholder.children.push(svg);
    link.children.push(placeholder);
    let mut label = Element::inline("span", &[]);
    label.children.push(text(&found.text));
    link.children.push(label);
    link
}

/// `allowedBoundary`: whitespace or punctuation.
fn boundary(c: char) -> bool {
    c.is_whitespace() || is_punct_char(c)
}

/// `hasAllowedBoundaries`
fn allowed_boundaries(content: &str, start: usize, end: usize) -> bool {
    let before = content[..start].chars().next_back();
    let after = content[end..].chars().next();
    before.is_none_or(boundary) && after.is_none_or(boundary)
}

/// `textPostProcess`: the text split around its matches, or None when
/// nothing matched.
fn split(content: &str, matcher: &Regex, ctx: &Context) -> Option<Vec<Node>> {
    let mut result: Option<Vec<Node>> = None;
    let mut pos = 0;
    let mut search = 0;
    while search <= content.len() {
        let Ok(Some(caps)) = matcher.captures_from_pos(content, search) else {
            break;
        };
        let whole = caps.get(0).unwrap();
        search = whole.end().max(whole.start() + 1);
        if !allowed_boundaries(content, whole.start(), whole.end()) {
            continue;
        }
        let nodes = result.get_or_insert_with(Vec::new);
        if whole.start() > pos {
            nodes.push(text(&content[pos..whole.start()]));
        }
        if let Some(slug) = caps.get(3) {
            nodes.push(hashtag(whole.as_str(), slug.as_str(), ctx));
        } else if let Some(name) = caps.get(1).or(caps.get(2)) {
            nodes.push(mention(name.as_str()));
        }
        pos = whole.end();
    }
    if let Some(nodes) = result.as_mut() {
        if pos < content.len() {
            nodes.push(text(&content[pos..]));
        }
    }
    result
}

/// Any link: `textReplace` with `skipAllLinks` leaves their text alone.
pub fn is_link(node: &Node) -> bool {
    node.is::<Link>()
        || super::linkify::is_auto_link(node)
        || node.cast::<Element>().is_some_and(|e| e.tag == "a")
}

/// The `text-post-process` core rule.
pub fn apply(root: &mut Node, settings: &RenderSettings, ctx: &Context) {
    let matcher: &Regex = if settings.mentions {
        &WITH_MENTIONS
    } else {
        &HASHTAGS_ONLY
    };
    fn visit(node: &mut Node, matcher: &Regex, ctx: &Context) {
        if is_link(node) {
            return;
        }
        let mut i = 0;
        while i < node.children.len() {
            let replaced = node.children[i]
                .cast::<Text>()
                .and_then(|text| split(&text.content, matcher, ctx));
            match replaced {
                Some(nodes) => {
                    let count = nodes.len();
                    node.children.splice(i..=i, nodes);
                    i += count;
                }
                None => {
                    visit(&mut node.children[i], matcher, ctx);
                    i += 1;
                }
            }
        }
    }
    visit(root, matcher, ctx);
}

pub fn allow(list: &mut AllowList) {
    list.allow(&[
        "a.hashtag-cooked",
        "span.hashtag-raw",
        "span.hashtag-icon-placeholder",
        "svg[class=fa d-icon d-icon-square-full svg-icon svg-node]",
        "use[href=#square-full]",
        "a[data-type]",
        "a[data-slug]",
        "a[data-ref]",
        "a[data-id]",
        "a[data-style-type]",
        "a[data-icon]",
        "a[data-emoji]",
    ]);
}
