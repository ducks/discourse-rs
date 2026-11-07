//! What the rich editor asks the markdown engine besides its node tree
//! (docs/rich-editor.md, "The engine"), for the parts of
//! static/prosemirror/lib/markdown-it.js and pretty-text the extensions
//! read: the engine's linkify (`getLinkify`), its allow list
//! (`getAllowLister`), and the emoji images and emoticons. Offsets are in
//! UTF-16 code units, as JavaScript counts them.

use serde_json::{Value, json};

use super::RenderSettings;
use super::linkify::{LinkMatch, LinkifyIt};
use crate::Unsupported;

/// The engine's linkify: the site's TLDs whether or not linkify is on.
fn linkify(settings: &RenderSettings) -> Result<std::sync::Arc<LinkifyIt>, Unsupported> {
    LinkifyIt::for_tlds(&settings.link_matcher_tlds)
        .map_err(|_| Unsupported("markdown_linkify_tlds that do not compile"))
}

fn utf16_offset(text: &str, byte: usize) -> usize {
    text[..byte].encode_utf16().count()
}

fn match_json(text: &str, link: &LinkMatch) -> Value {
    json!({
        "index": utf16_offset(text, link.index),
        "lastIndex": utf16_offset(text, link.last_index),
        "raw": link.raw,
        "url": link.url,
    })
}

/// `linkify.match(text)`: every link, or null for none.
pub fn link_matches(text: &str, settings: &RenderSettings) -> Result<Value, Unsupported> {
    let links = linkify(settings)?.link_matches(text);
    if links.is_empty() {
        return Ok(Value::Null);
    }
    Ok(Value::Array(
        links.iter().map(|link| match_json(text, link)).collect(),
    ))
}

/// `linkify.matchAtStart(text)`: the link starting the text, or null.
pub fn link_at_start(text: &str, settings: &RenderSettings) -> Result<Value, Unsupported> {
    Ok(linkify(settings)?
        .link_at_start(text)
        .map_or(Value::Null, |link| match_json(text, &link)))
}

/// `getAllowList().tagList`'s tags.
pub fn html_tags(settings: &RenderSettings) -> Value {
    json!(super::allow_list(settings).tags())
}

/// Whether the allow list keeps the attribute on the tag
/// (`isAllowListedAttr` or a custom check); null for a tag not on it.
pub fn html_attr_allowed(tag: &str, name: &str, value: &str, settings: &RenderSettings) -> Value {
    super::allow_list(settings)
        .allows_attr(tag, name, value)
        .map_or(Value::Null, Value::Bool)
}

/// `buildEmojiUrl(code)` and `isCustomEmoji(code)` for a code that
/// exists (`emojiExists`), else null.
pub fn emoji(code: &str, settings: &RenderSettings) -> Value {
    super::emoji::image(code, settings).map_or(
        Value::Null,
        |(url, custom)| json!({ "url": url, "custom": custom }),
    )
}

/// lib/text's `emojiUnescape(text)`: the text with its emoji as images
/// (custom ones and the deny list included, not lazy), or as it is with
/// emoji off.
pub fn emoji_unescape(text: &str, settings: &RenderSettings) -> Value {
    if !settings.emoji {
        return json!(text);
    }
    json!(crate::emoji_escape::perform_emoji_unescape(
        text,
        &crate::emoji_escape::UnescapeOptions {
            emoji_set: &settings.emoji_set,
            emoji_base_path: &settings.emoji_base_path,
            shortcuts: settings.emoji_shortcuts,
            inline: settings.inline_emoji,
            lazy: false,
            custom: Some(&settings.custom_emoji),
            denied: &settings.emoji_deny_list,
        },
    ))
}

/// pretty-text/emoji/data's `translations`: emoticon to emoji name.
pub fn emoji_translations() -> Value {
    json!(crate::emoji::DATA.translations)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::RenderSettings;
    use super::*;

    fn settings() -> RenderSettings {
        serde_json::from_value::<RenderSettings>(json!({
            "breaks": true, "linkify_tlds": null, "link_matcher_tlds": ["com", "org"],
            "typographer": false, "mentions": true, "emoji": true, "emoji_shortcuts": true,
            "inline_emoji": true, "emoji_set": "twitter", "emoji_base_path": "/images/emoji",
            "custom_emoji": {"party_parrot": "/uploads/parrot.gif"},
            "default_code_lang": "auto", "heading_anchor_label": "Heading anchor",
            "allowed_iframes": [], "allowed_href_schemes": [], "checklist": true,
            "footnotes": true, "poll": true, "poll_maximum_options": 20,
            "poll_voters_label": "voters", "local_dates": true,
            "local_dates_email_format": "YYYY-MM-DDTHH:mm:ss[Z]",
            "local_dates_email_timezone": "Etc/UTC", "spoiler": true, "policy": false,
            "avatar_sizes": [24, 48], "base_path": "", "secure_uploads": false,
            "topic_id": null, "force_quote_link": false, "post_id": null, "chat": false,
        }))
        .unwrap()
        .compiled()
        .unwrap()
    }

    #[test]
    fn links_are_found_with_linkify_off() {
        let found = link_matches("see example.com and https://a.org/x", &settings()).unwrap();
        assert_eq!(
            found,
            json!([
                {"index": 4, "lastIndex": 15, "raw": "example.com", "url": "http://example.com"},
                {"index": 20, "lastIndex": 35, "raw": "https://a.org/x", "url": "https://a.org/x"},
            ])
        );
        assert_eq!(link_matches("no links", &settings()).unwrap(), Value::Null);
    }

    #[test]
    fn offsets_count_utf16_units() {
        let found = link_matches("😀 example.com", &settings()).unwrap();
        assert_eq!(found[0]["index"], 3);
        assert_eq!(found[0]["lastIndex"], 14);
    }

    #[test]
    fn a_link_at_the_start() {
        let s = settings();
        assert_eq!(
            link_at_start("https://example.com/a b", &s).unwrap()["url"],
            "https://example.com/a"
        );
        // Fuzzy links need a schema at the start.
        assert_eq!(link_at_start("example.com", &s).unwrap(), Value::Null);
        assert_eq!(
            link_at_start("text https://example.com", &s).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn the_allow_list() {
        let s = settings();
        let tags = html_tags(&s);
        assert!(tags.as_array().unwrap().contains(&json!("kbd")));
        assert!(!tags.as_array().unwrap().contains(&json!("blink")));
        assert_eq!(html_attr_allowed("abbr", "title", "T", &s), json!(true));
        assert_eq!(html_attr_allowed("div", "data-x", "1", &s), json!(true));
        assert_eq!(html_attr_allowed("div", "onclick", "y()", &s), json!(false));
        assert_eq!(html_attr_allowed("blink", "class", "x", &s), Value::Null);
        // quotes.js's custom check
        assert_eq!(
            html_attr_allowed("aside", "class", "quote no-group", &s),
            json!(true)
        );
    }

    #[test]
    fn emoji_images() {
        let s = settings();
        let tada = emoji("tada", &s);
        assert!(
            tada["url"]
                .as_str()
                .unwrap()
                .starts_with("/images/emoji/twitter/tada.png?v=")
        );
        assert_eq!(tada["custom"], false);
        assert!(
            emoji("thumbsup:t3", &s)["url"]
                .as_str()
                .unwrap()
                .starts_with("/images/emoji/twitter/thumbsup/3.png")
        );
        assert_eq!(emoji("party_parrot", &s)["custom"], true);
        assert_eq!(emoji("not_an_emoji", &s), Value::Null);
        assert_eq!(emoji_translations()[":)"], "slight_smile");
        assert_eq!(
            emoji_unescape("hello :wave: :party_parrot:", &s),
            format!(
                "hello <img width=\"20\" height=\"20\" src='/images/emoji/twitter/wave.png?v={v}' title='wave' alt='wave' class='emoji'> \
                 <img width=\"20\" height=\"20\" src='/uploads/parrot.gif?v={v}' title='party_parrot' alt='party_parrot' class='emoji emoji-custom'>",
                v = crate::emoji::image_version()
            )
        );
    }
}
