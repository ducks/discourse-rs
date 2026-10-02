//! The poll plugin's rule (plugins/poll, discourse-markdown/poll.js):
//! `[poll ...]` around a list becomes the poll's container, each option
//! carrying the md5 of its text as its id, followed by the voters count.

use markdown_it::Node;
use markdown_it::parser::extset::RootExt;
use markdown_it::parser::inline::InlineRoot;
use markdown_it::plugins::cmark::block::heading::ATXHeading;
use markdown_it::plugins::cmark::block::lheading::SetextHeader;
use markdown_it::plugins::cmark::block::list::{BulletList, ListItem, OrderedList};

use super::RenderSettings;
use super::bbcode::TagInfo;
use super::context::Context;
use super::element::{Element, Holder, RawHtml};
use super::quotes::parse_int;
use crate::pretty_text::sanitizer::AllowList;

/// How many `[poll]` blocks are open around the one being parsed: a poll
/// inside a poll is not a poll.
#[derive(Debug, Default)]
pub struct PollDepth(pub usize);
impl RootExt for PollDepth {}

/// `ALLOWED_ATTRIBUTES`, in the order they are written.
const ALLOWED_ATTRIBUTES: [&str; 13] = [
    "chartType",
    "close",
    "groups",
    "max",
    "min",
    "name",
    "order",
    "public",
    "results",
    "status",
    "step",
    "type",
    "dynamic",
];
const DEFAULT_MAXIMUM_OPTIONS: i64 = 20;

/// `md5(JSON.stringify([text]))`
fn option_id(text: &str) -> String {
    let json = serde_json::to_string(&[text]).unwrap_or_default();
    format!("{:x}", md5::compute(json.as_bytes()))
}

/// The list's tokens as the JS sees them, flat: an item opens, the inline
/// content of its paragraphs, an item closes.
enum Token {
    ItemOpen,
    ItemClose,
    Inline(String),
}

fn flatten(node: &Node, out: &mut Vec<Token>) {
    if node.is::<ListItem>() {
        out.push(Token::ItemOpen);
        for child in &node.children {
            flatten(child, out);
        }
        out.push(Token::ItemClose);
    } else if let Some(inline) = node.cast::<InlineRoot>() {
        out.push(Token::Inline(inline.content.clone()));
    } else {
        for child in &node.children {
            flatten(child, out);
        }
    }
}

/// Gives every list item its `data-poll-option-id`: the hash of the text
/// between its opening and the first item closing after it.
fn add_option_ids(content: &mut [Node]) {
    let mut tokens = Vec::new();
    for node in content.iter() {
        flatten(node, &mut tokens);
    }
    let mut ids = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if !matches!(token, Token::ItemOpen) {
            continue;
        }
        let Some(close) = (i + 1..tokens.len()).find(|j| matches!(tokens[*j], Token::ItemClose))
        else {
            ids.push(None);
            continue;
        };
        let text: Vec<&str> = tokens[i..=close]
            .iter()
            .filter_map(|t| match t {
                Token::Inline(content) => Some(content.as_str()),
                _ => None,
            })
            .collect();
        ids.push(Some(option_id(&text.join(" "))));
    }
    let mut ids = ids.into_iter();
    fn assign(node: &mut Node, ids: &mut impl Iterator<Item = Option<String>>) {
        if node.is::<ListItem>()
            && let Some(Some(id)) = ids.next()
        {
            node.attrs.push(("data-poll-option-id", id));
        }
        for child in node.children.iter_mut() {
            assign(child, ids);
        }
    }
    for node in content.iter_mut() {
        assign(node, &mut ids);
    }
}

/// The `before` and `after` of the poll rule around the parsed content.
/// `nested` is a poll inside another, which stays plain content.
pub fn build(
    info: &TagInfo,
    mut content: Vec<Node>,
    nested: bool,
    settings: &RenderSettings,
    ctx: &Context,
) -> Node {
    let plain = |content: Vec<Node>| {
        let mut holder = Node::new(Holder);
        holder.children = content;
        holder
    };
    if nested {
        return plain(content);
    }
    // { name: "poll", status: "open", ...attrs }
    let attr = |name: &str| -> Option<&str> {
        info.attr(name).filter(|v| !v.is_empty()).or(match name {
            "name" if info.attr("name").is_none() => Some("poll"),
            "status" if info.attr("status").is_none() => Some("open"),
            _ => None,
        })
    };

    // A heading at the start is the poll's title.
    let mut title: Option<Vec<Node>> = None;
    if content
        .first()
        .is_some_and(|n| n.is::<ATXHeading>() || n.is::<SetextHeader>())
    {
        let mut heading = content.remove(0);
        title = Some(std::mem::take(&mut heading.children));
    }

    let mut generated: Option<String> = None;
    if attr("type") == Some("number") {
        if !content.is_empty() {
            // A number poll with content of its own is no poll.
            return plain(content);
        }
        let maximum = Some(settings.poll_maximum_options)
            .filter(|m| *m >= 1)
            .unwrap_or(DEFAULT_MAXIMUM_OPTIONS);
        let min = attr("min").and_then(parse_int).unwrap_or(1);
        let max = attr("max").and_then(parse_int).unwrap_or(maximum);
        let step = attr("step")
            .and_then(parse_int)
            .filter(|s| *s >= 1)
            .unwrap_or(1);
        if min <= max {
            let mut list = String::from("<ul>\n");
            let mut i = min;
            let mut count = 0;
            while i <= max && count < maximum + 1 {
                list.push_str(&format!(
                    "<li data-poll-option-id=\"{}\">{i}</li>\n",
                    option_id(&i.to_string())
                ));
                i += step;
                count += 1;
            }
            list.push_str("</ul>\n");
            generated = Some(list);
        }
    }

    let mut attrs: Vec<(String, String)> = vec![("class".into(), "poll".into())];
    for name in ALLOWED_ATTRIBUTES {
        if let Some(value) = attr(name) {
            attrs.push((format!("data-poll-{name}"), value.to_string()));
        }
    }
    let mut poll = Node::new(Element {
        tag: "div".into(),
        attrs,
        block: true,
    });

    // Anything but a list first leaves the JS with an opened poll and
    // nothing in it.
    if content
        .first()
        .is_some_and(|n| !n.is::<BulletList>() && !n.is::<OrderedList>())
    {
        ctx.refuse("a poll whose content does not start with a list");
        return poll;
    }

    let mut container = Element::block("div", &[("class", "poll-container")]);
    if let Some(title) = title {
        // The title's inline content sits right inside its div.
        container
            .children
            .push(Node::new(RawHtml("<div class=\"poll-title\">".into())));
        container.children.extend(title);
        container
            .children
            .push(Node::new(RawHtml("</div>\n".into())));
    }
    add_option_ids(&mut content);
    container.children.extend(content);
    if let Some(list) = generated {
        container.children.push(Node::new(RawHtml(list)));
    }
    poll.children.push(container);
    poll.children.push(Node::new(RawHtml(format!(
        "<div class=\"poll-info\">\n<div class=\"poll-info_counts\">\n<div class=\"poll-info_counts-count\">\n\
         <span class=\"info-number\">0</span>\n<span class=\"info-label\">{}</span>\n</div>\n</div>\n</div>\n",
        markdown_it::common::utils::escape_html(&settings.poll_voters_label)
    ))));
    poll
}

pub fn allow(list: &mut AllowList) {
    list.allow(&[
        "a.button.cast-votes",
        "a.button.toggle-results",
        "div.poll-buttons",
        "div.poll-container",
        "div.poll-info_counts-count",
        "div.poll-info_counts",
        "div.poll-info",
        "div.poll-title",
        "div.poll",
        "div[data-*]",
        "li[data-*]",
        "span.info-label",
        "span.info-number",
        "span.info-text",
    ]);
}

#[cfg(test)]
mod tests {
    use super::option_id;

    #[test]
    fn option_ids_are_md5_of_the_json_array() {
        // What Rails recorded for `* Option A`.
        assert_eq!(option_id("Option A"), "5b8ee5ba2a43e258f93dbef9264bf1ad");
    }
}
