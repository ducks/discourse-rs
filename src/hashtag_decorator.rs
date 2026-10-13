//! lib/hashtag-decorator.js's `decorateHashtags`, run on a post's cooked
//! HTML before it is sent (Ember runs it as it decorates the post): each
//! `a.hashtag-cooked`'s icon placeholder becomes its hashtag type's icon
//! (`generateIconHTML`), and the link gets an aria-label of its text.

use crate::pretty_text::cleanup::{
    all_elements, all_elements_within, attr, fragment_root, has_class, parse, replace_with,
    set_attr, text, to_html,
};

/// What the icons are made of: the site's hashtag icons and the emoji
/// images' location.
pub struct Icons {
    /// `site.hashtag_icons.tag`, none without tagging.
    pub tag: Option<String>,
    /// `Emoji.url_for`'s `<base>/images/emoji/<set>` (or the external
    /// emoji URL with the set), without the trailing slash.
    pub emoji_base: String,
    /// The emoji images' `?v=`.
    pub emoji_version: String,
}

fn escape(s: &str) -> String {
    crate::discourse_diff::escape_html(s)
}

/// `dReplaceEmoji(":<name>:")` as the post decorations leave it (lazy
/// loading with its aspect ratio).
fn emoji(icons: &Icons, name: &str) -> String {
    let path = match name.rsplit_once(":t") {
        Some((base, tone))
            if tone.len() == 1 && ('1'..='6').contains(&tone.chars().next().unwrap_or('0')) =>
        {
            format!("{base}/{tone}")
        }
        _ => name.to_string(),
    };
    format!(
        "<img width=\"20\" height=\"20\" src=\"{}/{}.png?v={}\" title=\"{n}\" alt=\"{n}\" class=\"emoji\" loading=\"lazy\" style=\"aspect-ratio: 20 / 20;\">",
        icons.emoji_base,
        escape(&path),
        icons.emoji_version,
        n = escape(name),
    )
}

/// The hashtag type's `generateIconHTML`, None for a type with no class.
fn icon_html(
    icons: &Icons,
    kind: &str,
    id: i64,
    style_type: &str,
    data_icon: Option<&str>,
    data_emoji: Option<&str>,
) -> Option<String> {
    match kind {
        "category" => {
            let inner = match style_type {
                "icon" => data_icon
                    .filter(|i| !i.is_empty())
                    .map(|i| crate::topic_list_view::icon(i, None))
                    .unwrap_or_default(),
                "emoji" => data_emoji
                    .filter(|e| !e.is_empty())
                    .map(|e| emoji(icons, e))
                    .unwrap_or_default(),
                _ => String::new(),
            };
            Some(format!(
                "<span class=\"hashtag-category-{style_type} hashtag-color--category-{id}\">{inner}</span>"
            ))
        }
        // TagHashtagType: the site's tag icon (the link's own when its
        // style is icon).
        "tag" => {
            let icon = if style_type == "icon" {
                data_icon.map(str::to_string)
            } else {
                icons.tag.clone()
            };
            Some(crate::topic_list_view::icon(
                icon.as_deref().unwrap_or(""),
                Some(&format!("hashtag-color--tag-{id}")),
            ))
        }
        _ => None,
    }
}

/// `decorateHashtags` over cooked HTML; unchanged without hashtags.
pub fn decorate(cooked: &str, icons: &Icons) -> String {
    if !cooked.contains("hashtag-cooked") {
        return cooked.to_string();
    }
    let dom = parse(cooked);
    // The icons' own documents, kept until the cooked is serialized: an
    // RcDom clears its nodes' children when dropped.
    let mut fragments = Vec::new();
    for link in all_elements(&dom) {
        if !has_class(&link, "hashtag-cooked") {
            continue;
        }
        let placeholder = all_elements_within(&link)
            .into_iter()
            .find(|e| has_class(e, "hashtag-icon-placeholder"));
        let kind = attr(&link, "data-type").unwrap_or_default();
        if let Some(placeholder) = placeholder {
            let style = attr(&link, "data-style-type")
                .filter(|s| ["square", "icon", "emoji"].contains(&s.as_str()))
                .unwrap_or_else(|| "square".to_string());
            // parseInt of data-id.
            let id = attr(&link, "data-id")
                .map(|v| crate::ruby::to_i(&v))
                .unwrap_or_default();
            let html = icon_html(
                icons,
                &kind,
                id,
                &style,
                attr(&link, "data-icon").as_deref(),
                attr(&link, "data-emoji").as_deref(),
            );
            if let Some(html) = html {
                let fragment = parse(html.trim());
                let first = fragment_root(&fragment).children.borrow().first().cloned();
                if let Some(node) = first {
                    node.parent.set(None);
                    replace_with(&placeholder, vec![node]);
                }
                fragments.push(fragment);
            }
        }
        set_attr(&link, "aria-label", text(&link).trim());
    }
    to_html(&dom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icons() -> Icons {
        Icons {
            tag: Some("tag".into()),
            emoji_base: "/images/emoji/twitter".into(),
            emoji_version: "15".into(),
        }
    }

    const PLACEHOLDER: &str = "<span class=\"hashtag-icon-placeholder\"><svg class=\"fa d-icon d-icon-square-full svg-icon svg-node\"><use href=\"#square-full\"></use></svg></span>";

    #[test]
    fn decorates_as_ember() {
        let cooked = format!(
            "<p><a class=\"hashtag-cooked\" href=\"/c/general/4\" data-type=\"category\" data-slug=\"general\" data-id=\"4\" data-style-type=\"emoji\" data-emoji=\"blue_book\">{PLACEHOLDER}<span>General</span></a> <a class=\"hashtag-cooked\" href=\"/c/general/sub-general/34\" data-type=\"category\" data-slug=\"sub-general\" data-id=\"34\" data-style-type=\"square\" data-ref=\"general:sub-general\">{PLACEHOLDER}<span>General &gt; Sub General</span></a> <a class=\"hashtag-cooked\" href=\"/tag/guide/2\" data-type=\"tag\" data-slug=\"guide\" data-id=\"2\" data-style-type=\"icon\" data-icon=\"tag\">{PLACEHOLDER}<span>guide</span></a></p>"
        );
        assert_eq!(
            decorate(&cooked, &icons()),
            "<p><a class=\"hashtag-cooked\" href=\"/c/general/4\" data-type=\"category\" data-slug=\"general\" data-id=\"4\" data-style-type=\"emoji\" data-emoji=\"blue_book\" aria-label=\"General\"><span class=\"hashtag-category-emoji hashtag-color--category-4\"><img width=\"20\" height=\"20\" src=\"/images/emoji/twitter/blue_book.png?v=15\" title=\"blue_book\" alt=\"blue_book\" class=\"emoji\" loading=\"lazy\" style=\"aspect-ratio: 20 / 20;\"></span><span>General</span></a> <a class=\"hashtag-cooked\" href=\"/c/general/sub-general/34\" data-type=\"category\" data-slug=\"sub-general\" data-id=\"34\" data-style-type=\"square\" data-ref=\"general:sub-general\" aria-label=\"General > Sub General\"><span class=\"hashtag-category-square hashtag-color--category-34\"></span><span>General &gt; Sub General</span></a> <a class=\"hashtag-cooked\" href=\"/tag/guide/2\" data-type=\"tag\" data-slug=\"guide\" data-id=\"2\" data-style-type=\"icon\" data-icon=\"tag\" aria-label=\"guide\"><svg class=\"fa d-icon d-icon-tag svg-icon fa-width-auto hashtag-color--tag-2 svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#tag\"></use></svg><span>guide</span></a></p>"
        );
    }

    #[test]
    fn leaves_cooked_without_hashtags_alone() {
        assert_eq!(decorate("<p>a &amp; b</p>", &icons()), "<p>a &amp; b</p>");
    }
}
