//! features/quotes.js: `[quote="user, post:1, topic:2"]` as an aside with
//! a title (avatar, then the user's name or a link to the quoted topic)
//! around a blockquote.

use markdown_it::Node;

use super::RenderSettings;
use super::bbcode::TagInfo;
use super::context::Context;
use super::element::{BlockText, Element, RawHtml};
use crate::pretty_text::sanitizer::AllowList;

/// JavaScript's `parseInt(s, 10)`: the leading integer, if there is one.
fn parse_int(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let (sign, digits) = match s.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, s.strip_prefix('+').unwrap_or(s)),
    };
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    digits[..end].parse::<i64>().ok().map(|n| sign * n)
}

/// `avatarImg({ size: "tiny", avatarTemplate }, getURL)`: tiny is 24
/// pixels, drawn from the 48 pixel image (the context claims a pixel
/// ratio of 2).
fn avatar_img(template: &str, settings: &RenderSettings) -> Option<String> {
    if template.is_empty() {
        return None;
    }
    let wanted = 48;
    let raw_size = settings
        .avatar_sizes
        .iter()
        .find(|size| **size >= wanted)
        .or(settings.avatar_sizes.last())
        .copied()
        .unwrap_or(wanted);
    let path = template.replace("{size}", &raw_size.to_string());
    // getURL: a root-relative path gets the base path in front.
    let url = if path.starts_with('/') && !path.starts_with("//") && !settings.base_path.is_empty()
    {
        if path == settings.base_path || path.starts_with(&format!("{}/", settings.base_path)) {
            path
        } else {
            format!("{}{path}", settings.base_path)
        }
    } else {
        path
    };
    Some(format!(
        "<img alt='' width='24' height='24' src='{url}' class='avatar'>"
    ))
}

/// Whether `performEmojiUnescape` would touch a title: a unicode emoji,
/// or `textEmojiRegex` (`\B:[^\s:]+(?::t\d)?:?\B`) finding a `:code:`.
fn title_has_emoji(title: &str) -> bool {
    static CODE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?-u:\B):[^\s:]+(?::t[0-9])?:?(?-u:\B)").unwrap()
    });
    let chars: Vec<char> = title.chars().collect();
    let unicode = (0..chars.len()).any(|i| {
        !chars[i].is_ascii()
            && (1..=crate::emoji::DATA.unicode_max_chars.min(chars.len() - i)).any(|len| {
                let candidate: String = chars[i..i + len].iter().collect();
                crate::emoji::DATA.unicode.contains_key(&candidate)
            })
    });
    unicode || CODE.is_match(title)
}

/// The `before` and `after` of the quote rule around the parsed content.
pub fn build(info: &TagInfo, content: Vec<Node>, settings: &RenderSettings, ctx: &Context) -> Node {
    let mut username: Option<String> = None;
    let mut post_number: Option<i64> = None;
    let mut topic_id: Option<i64> = None;
    let mut full = false;
    let mut display_name: Option<String> = None;

    if let Some(quote_info) = info.attr("_default").filter(|q| !q.is_empty()) {
        // `split(/\,\s*/)`
        let mut split: Vec<&str> = Vec::new();
        let mut rest = quote_info;
        while let Some(comma) = rest.find(',') {
            split.push(&rest[..comma]);
            rest = rest[comma + 1..].trim_start();
        }
        split.push(rest);
        username = Some(split[0].to_string());
        for part in &split[1..] {
            if let Some(n) = part.strip_prefix("post:") {
                post_number = parse_int(n);
            } else if let Some(n) = part.strip_prefix("topic:") {
                topic_id = parse_int(n);
            } else if part
                .find("full:")
                .is_some_and(|at| part[at + 5..].trim_start().starts_with("true"))
            {
                full = true;
            } else if let Some(name) = part.strip_prefix("username:") {
                // With full names shown first, the name comes before
                // `post:` and may itself hold commas.
                let marker = post_number.map(|n| format!("post:{n}"));
                let upto = marker
                    .and_then(|m| split.iter().position(|s| *s == m))
                    .unwrap_or(0);
                display_name = Some(split[..upto].join(", "));
                username = Some(name.to_string());
            }
        }
    }
    // A post number of 0 or NaN is no post number.
    let post_number = post_number.filter(|n| *n != 0);
    let topic_id = topic_id.filter(|n| *n != 0);

    let name = username.as_deref().unwrap_or("");
    let avatar = avatar_img(ctx.avatar(name), settings);
    let group = ctx.primary_group(name).to_string();
    // formatUsername is the identity.
    let display_name = display_name
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| name.to_string());

    let mut attrs: Vec<(String, String)> = Vec::new();
    let class = if group.is_empty() {
        "quote no-group".to_string()
    } else {
        format!("quote group-{group}")
    };
    attrs.push(("class".into(), class));
    if !name.is_empty() {
        attrs.push(("data-username".into(), name.to_string()));
    }
    if !display_name.is_empty() && display_name != name {
        attrs.push(("data-display-name".into(), display_name.clone()));
    }
    if let Some(n) = post_number {
        attrs.push(("data-post".into(), n.to_string()));
    }
    if let Some(n) = topic_id {
        attrs.push(("data-topic".into(), n.to_string()));
    }
    if full {
        attrs.push(("data-full".into(), "true".into()));
    }
    let mut aside = Node::new(Element {
        tag: "aside".into(),
        attrs,
        block: true,
    });

    if !name.is_empty() {
        let mut title = Element::block("div", &[("class", "title")]);
        title
            .children
            .push(Element::block("div", &[("class", "quote-controls")]));
        if let Some(img) = avatar {
            title.children.push(Node::new(RawHtml(img)));
        }
        // A quote from another topic links to it instead of naming the
        // user; a topic that cannot be found leaves the title bare.
        let for_other_topic = settings.topic_id.is_some() && topic_id != settings.topic_id;
        let off_topic = post_number.is_some() && (for_other_topic || settings.force_quote_link);
        if off_topic {
            let info = topic_id.and_then(|id| ctx.topic(id));
            if let Some(info) = info {
                let mut href = info.href.clone();
                if let Some(n) = post_number.filter(|n| *n > 0) {
                    href.push_str(&format!("/{n}"));
                }
                // performEmojiUnescape on the title: only a title with
                // something it would replace is refused.
                if settings.emoji && title_has_emoji(&info.title) {
                    ctx.refuse("emoji in the title of a quoted topic");
                }
                let mut link = Element::inline("a", &[("href", &href)]);
                link.children.push(Node::new(RawHtml(info.title.clone())));
                title.children.push(link);
            }
        } else {
            title
                .children
                .push(Node::new(BlockText(format!(" {display_name}:"))));
        }
        aside.children.push(title);
    }

    let mut blockquote = Element::block("blockquote", &[]);
    blockquote.children = content;
    aside.children.push(blockquote);
    aside
}

pub fn allow(list: &mut AllowList) {
    list.allow(&["img[class=avatar]", "img[loading=lazy]"]);
    list.allow_custom(|tag, name, value| {
        tag == "aside"
            && name == "class"
            && (value == "quote no-group"
                || value
                    .strip_prefix("quote group-")
                    .is_some_and(|group| !group.is_empty()))
    });
}

#[cfg(test)]
mod tests {
    use super::parse_int;

    #[test]
    fn parse_int_like_javascript() {
        assert_eq!(parse_int("12"), Some(12));
        assert_eq!(parse_int("12abc"), Some(12));
        assert_eq!(parse_int(" 7"), Some(7));
        assert_eq!(parse_int("abc"), None);
        assert_eq!(parse_int(""), None);
    }
}
