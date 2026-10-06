//! features/upload-protocol.js with the image and link renderers of
//! engine.js: `upload://` short urls resolved to the upload (or to a
//! placeholder that remembers the short url), image alt text split into
//! the alt and its `|100x200`, `|video`, `|audio`, `|thumbnail` and
//! `|key=value` suffixes, and `|attachment` on a link.

use markdown_it::Node;
use markdown_it::parser::inline::{Text, TextSpecial};
use markdown_it::plugins::cmark::inline::backticks::CodeInline;
use markdown_it::plugins::cmark::inline::emphasis::{Em, Strong};
use markdown_it::plugins::cmark::inline::image::Image;
use markdown_it::plugins::cmark::inline::link::Link;
use markdown_it::plugins::cmark::inline::newline::{Hardbreak, Softbreak};
use markdown_it::plugins::extra::strikethrough::Strikethrough;
use markdown_it::plugins::html::html_block::HtmlBlock;
use markdown_it::plugins::html::html_inline::HtmlInline;

use super::RenderSettings;
use super::context::Context;
use super::element::{CodeText, Element, RawHtml};
use crate::pretty_text::sanitizer::AllowList;

const UPLOAD: &str = "upload://";

/// `reconstructLiteralLabel`: the label's source back from its parsed
/// nodes, emphasis markers as the characters they were. A filename with
/// underscores is not italics.
fn literal_label(nodes: &[Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        if let Some(text) = node.cast::<Text>() {
            out.push_str(&text.content);
        } else if let Some(text) = node.cast::<CodeText>() {
            out.push_str(&text.0);
        } else if let Some(text) = node.cast::<TextSpecial>() {
            out.push_str(&text.content);
        } else if let Some(code) = node.cast::<CodeInline>() {
            let marker = code.marker.to_string().repeat(code.marker_len);
            out.push_str(&marker);
            out.push_str(&literal_label(&node.children));
            out.push_str(&marker);
        } else if let Some(em) = node.cast::<Em>() {
            out.push(em.marker);
            out.push_str(&literal_label(&node.children));
            out.push(em.marker);
        } else if let Some(strong) = node.cast::<Strong>() {
            let marker = strong.marker.to_string().repeat(2);
            out.push_str(&marker);
            out.push_str(&literal_label(&node.children));
            out.push_str(&marker);
        } else if let Some(strike) = node.cast::<Strikethrough>() {
            let marker = strike.marker.to_string().repeat(2);
            out.push_str(&marker);
            out.push_str(&literal_label(&node.children));
            out.push_str(&marker);
        } else if node.is::<Softbreak>() || node.is::<Hardbreak>() {
            out.push(' ');
        } else {
            out.push_str(&literal_label(&node.children));
        }
    }
    out
}

/// `extractDataAttribute`: `key=value` as a `data-key` attribute.
fn data_attribute(segment: &str) -> Option<(String, String)> {
    let (key, value) = segment.split_once('=')?;
    let key = format!("data-{key}").to_lowercase();
    // `/^[A-Za-z]+[\w\-\:\.]*$/`
    let mut chars = key.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'));
    valid.then(|| (key, value.to_string()))
}

/// `IMG_SIZE_REGEX`: `WxH`, optionally followed by `, 75%`, `, 100x` or
/// `, x50`. Returns the width and height to write.
fn image_size(segment: &str) -> Option<(i64, i64)> {
    let (size, scale) = match segment.split_once(',') {
        Some((size, scale)) => (size.trim_end(), Some(scale.trim_start())),
        None => (segment, None),
    };
    let dimension = |s: &str| -> Option<i64> {
        (!s.is_empty() && !s.starts_with('0') && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    let (w, h) = size.split_once('x')?;
    let (mut width, mut height) = (dimension(w)?, dimension(h)?);
    if let Some(scale) = scale {
        // `(x?)([1-9][0-9]{0,2}?)([%x]?)`
        let (leading_x, rest) = match scale.strip_prefix('x') {
            Some(rest) => (true, rest),
            None => (false, scale),
        };
        let (number, unit) = match rest.strip_suffix(['%', 'x']) {
            Some(number) => (number, rest.chars().last()),
            None => (rest, None),
        };
        if number.is_empty() || number.len() > 3 {
            return None;
        }
        let n = dimension(number)? as f64;
        match unit {
            Some('%') => {
                width = (width as f64 * (n / 100.0)) as i64;
                height = (height as f64 * (n / 100.0)) as i64;
            }
            Some('x') => {
                let ratio = n / width as f64;
                width = n as i64;
                height = (height as f64 * ratio) as i64;
            }
            None if leading_x => {
                let ratio = n / height as f64;
                height = n as i64;
                width = (width as f64 * ratio) as i64;
            }
            _ => {}
        }
    }
    Some((width, height))
}

fn known_suffix(segment: &str) -> bool {
    matches!(segment, "video" | "audio" | "thumbnail")
        || image_size(segment).is_some()
        || data_attribute(segment).is_some()
}

/// xss `escapeAttrValue`, which the media HTML is written with... by the
/// time it reaches the sanitizer; the token attributes themselves are
/// already escaped the markdown-it way.
fn escape(value: &str) -> String {
    markdown_it::common::utils::escape_html(value).into_owned()
}

/// `renderImageOrPlayableMedia` after the upload rule set the `src`.
fn image(node: &Node, image: &Image, settings: &RenderSettings, ctx: &Context) -> Node {
    let from_upload = image.url.starts_with(UPLOAD);
    // The alt text: the label's literal source for an upload, else the
    // label rendered as text.
    let label = if from_upload {
        literal_label(&node.children)
    } else {
        // renderInlineAsText: the text, inline code's included.
        let mut text = String::new();
        node.walk(|n, _| {
            if let Some(t) = n.cast::<Text>() {
                text.push_str(&t.content);
            } else if let Some(t) = n.cast::<CodeText>() {
                text.push_str(&t.0);
            } else if n.is::<Softbreak>() {
                text.push('\n');
            }
        });
        text
    };
    let split: Vec<&str> = label.split('|').collect();
    let mut suffix_start = split.len();
    while suffix_start > 1 && known_suffix(split[suffix_start - 1]) {
        suffix_start -= 1;
    }
    let alt = split[..suffix_start].join("|");
    let suffixes = &split[suffix_start..];
    let media = matches!(suffixes.first(), Some(&"video") | Some(&"audio"));

    // upload-protocol: the upload's url, or a placeholder and the short
    // url kept aside.
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut extra: Vec<(String, String)> = Vec::new();
    let src = if from_upload {
        match ctx.upload(&image.url) {
            Some(upload) => {
                extra.push(("data-base62-sha1".into(), upload.base62_sha1.clone()));
                upload.url.clone()
            }
            None => {
                extra.push(("data-orig-src".into(), image.url.clone()));
                // `token.content.match(/\|video|\|audio/)`
                if label.contains("|video") || label.contains("|audio") {
                    format!("{}/404", settings.base_path)
                } else {
                    format!("{}/images/transparent.png", settings.base_path)
                }
            }
        }
    } else {
        image.url.clone()
    };

    if media {
        let orig = extra
            .iter()
            .find(|(k, _)| k == "data-orig-src")
            .map(|(_, v)| format!("data-orig-src=\"{}\"", escape(v)))
            .unwrap_or_default();
        let src = escape(&src);
        let html = if suffixes[0] == "video" {
            format!(
                "<div class=\"video-placeholder-container\" data-video-src=\"{src}\" {orig}>\n  </div>"
            )
        } else {
            format!(
                "<audio preload=\"metadata\" controls>\n    <source src=\"{src}\" {orig}>\n    <a href=\"{src}\">{src}</a>\n  </audio>"
            )
        };
        return Node::new(RawHtml(html));
    }

    attrs.push(("src".into(), src));
    attrs.push(("alt".into(), String::new()));
    if let Some(title) = &image.title {
        attrs.push(("title".into(), title.clone()));
    }
    attrs.extend(extra);
    let has = |attrs: &[(String, String)], name: &str| attrs.iter().any(|(k, _)| k == name);
    for suffix in suffixes {
        if let Some((width, height)) = image_size(suffix) {
            if !has(&attrs, "width") {
                attrs.push(("width".into(), width.to_string()));
            }
            if !has(&attrs, "height") {
                attrs.push(("height".into(), height.to_string()));
            }
        } else if let Some(data) = data_attribute(suffix) {
            attrs.push(data);
        } else if *suffix == "thumbnail" {
            attrs.push(("data-thumbnail".into(), "true".into()));
        }
    }
    if alt.is_empty() {
        attrs.push(("role".into(), "presentation".into()));
    } else if let Some(entry) = attrs.iter_mut().find(|(k, _)| k == "alt") {
        entry.1 = alt;
    }
    let html: String = attrs
        .iter()
        .map(|(k, v)| format!(" {k}=\"{}\"", escape(v)))
        .collect();
    Node::new(RawHtml(format!("<img{html}>")))
}

/// `renderAttachment` after the upload rule set the `href`: the link as
/// an element, `|attachment` and `|key=value` taken out of its last text.
fn link(node: &mut Node, settings: &RenderSettings, ctx: &Context) -> Option<Node> {
    let link = node.cast::<Link>()?;
    let from_upload = link.url.starts_with(UPLOAD);
    let (url, title) = (link.url.clone(), link.title.clone());

    // literalize_upload_labels: an upload link's label is one literal text.
    if from_upload {
        let literal = literal_label(&node.children);
        node.children = vec![Node::new(Text { content: literal })];
    }
    // The last text of the label carries the markers.
    let last_text = node
        .children
        .iter()
        .rposition(|c| c.cast::<Text>().is_some_and(|t| !t.content.is_empty()));
    let mut attachment = false;
    let mut data: Vec<(String, String)> = Vec::new();
    if let Some(i) = last_text {
        let content = node.children[i].cast::<Text>().unwrap().content.clone();
        let mut kept: Vec<&str> = Vec::new();
        for segment in content.split('|') {
            if segment == "attachment" {
                attachment = true;
            } else if let Some(pair) = data_attribute(segment) {
                data.push(pair);
            } else {
                kept.push(segment);
            }
        }
        if !kept.is_empty() {
            node.children[i].cast_mut::<Text>().unwrap().content = kept.join("|");
        }
    }
    if !from_upload && !attachment && data.is_empty() {
        return None;
    }

    let mut attrs: Vec<(String, String)> = Vec::new();
    if attachment {
        attrs.push(("class".into(), "attachment".into()));
    }
    let mut orig: Option<(String, String)> = None;
    let href = if from_upload {
        match ctx.upload(&url) {
            Some(upload) => {
                let secure = settings.secure_uploads
                    && (upload.url.contains("secure-media-uploads")
                        || upload.url.contains("secure-uploads"));
                if secure {
                    upload.url.clone()
                } else {
                    upload.short_path.clone()
                }
            }
            None => {
                orig = Some(("data-orig-href".into(), url.clone()));
                format!("{}/404", settings.base_path)
            }
        }
    } else {
        url
    };
    attrs.push(("href".into(), href));
    if let Some(title) = title {
        attrs.push(("title".into(), title));
    }
    attrs.extend(orig);
    attrs.extend(data);
    let mut element = Node::new(Element {
        tag: "a".into(),
        attrs,
        block: false,
    });
    element.children = std::mem::take(&mut node.children);
    Some(element)
}

/// `findUploadsInHtml`: in raw html, an `<img>` whose `src` is a short url
/// gets the upload's url and sha1, or the placeholder and the short url
/// kept aside, as an image written in markdown does. The JS finds them
/// with js-xss, which also re-serializes every other attribute; the
/// sanitizer serializes them all again, so only the `src` is rewritten
/// here. Returns the new html when something changed.
fn html_uploads(html: &str, settings: &RenderSettings, ctx: &Context) -> Option<String> {
    if !html.contains(UPLOAD) {
        return None;
    }
    let lower = html.to_ascii_lowercase();
    let mut out = String::new();
    let mut copied = 0;
    let mut from = 0;
    while let Some(found) = lower[from..].find("<img") {
        let tag_start = from + found;
        from = tag_start + 4;
        if !lower[from..].starts_with(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>') {
            continue;
        }
        let Some((src, short_url, end)) = upload_src(&html[from..]) else {
            continue;
        };
        let (start, stop) = (from + src.start, from + src.end);
        let attrs = match ctx.upload(&short_url) {
            Some(upload) => format!(
                "src=\"{}\" data-base62-sha1=\"{}\"",
                escape(&upload.url),
                escape(&upload.base62_sha1)
            ),
            None => format!(
                "src=\"{}\" data-orig-src=\"{}\"",
                escape(&format!("{}/images/transparent.png", settings.base_path)),
                escape(&short_url)
            ),
        };
        out.push_str(&html[copied..start]);
        out.push_str(&attrs);
        copied = stop;
        from += end;
    }
    (copied > 0).then(|| out + &html[copied..])
}

/// The whole `src=...` attribute of the tag at the start of `rest` when
/// its value is a short url, that value, and where the tag ends, as
/// offsets into `rest`.
fn upload_src(rest: &str) -> Option<(std::ops::Range<usize>, String, usize)> {
    let bytes = rest.as_bytes();
    let mut i = 0;
    let mut found = None;
    while i < bytes.len() {
        match bytes[i] {
            b'>' => return found.map(|(range, value)| (range, value, i + 1)),
            c if c.is_ascii_whitespace() || c == b'/' => i += 1,
            _ => {
                let name_start = i;
                while i < bytes.len()
                    && !matches!(bytes[i], b'=' | b'>' | b'/')
                    && !bytes[i].is_ascii_whitespace()
                {
                    i += 1;
                }
                let name = &rest[name_start..i];
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if bytes.get(i) != Some(&b'=') {
                    continue;
                }
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                let value_start = i;
                match bytes.get(i) {
                    Some(&quote @ (b'"' | b'\'')) => {
                        i += 1;
                        while i < bytes.len() && bytes[i] != quote {
                            i += 1;
                        }
                        i = (i + 1).min(bytes.len());
                    }
                    _ => {
                        while i < bytes.len() && bytes[i] != b'>' && !bytes[i].is_ascii_whitespace()
                        {
                            i += 1;
                        }
                    }
                }
                let value = rest[value_start..i].trim_matches(|c| c == '"' || c == '\'');
                if found.is_none() && name.eq_ignore_ascii_case("src") && value.starts_with(UPLOAD)
                {
                    found = Some((name_start..i, value.to_string()));
                }
            }
        }
    }
    None
}

/// The `upload-protocol` core rule and the renderers that follow it.
pub fn run(root: &mut Node, settings: &RenderSettings, ctx: &Context) {
    fn visit(node: &mut Node, settings: &RenderSettings, ctx: &Context) {
        for child in node.children.iter_mut() {
            visit(child, settings, ctx);
        }
        if let Some(img) = node.cast::<Image>() {
            let replacement = image(node, img, settings, ctx);
            *node = replacement;
        } else if node.is::<Link>() {
            if let Some(replacement) = link(node, settings, ctx) {
                *node = replacement;
            }
        } else if let Some(html) = node.cast_mut::<HtmlInline>() {
            if let Some(rewritten) = html_uploads(&html.content, settings, ctx) {
                html.content = rewritten;
            }
        } else if let Some(html) = node.cast_mut::<HtmlBlock>()
            && let Some(rewritten) = html_uploads(&html.content, settings, ctx)
        {
            html.content = rewritten;
        }
    }
    visit(root, settings, ctx);
}

pub fn allow(list: &mut AllowList) {
    list.allow(&[
        "img[data-orig-src]",
        "img[data-base62-sha1]",
        "a[data-orig-href]",
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_sizes() {
        assert_eq!(image_size("100x200"), Some((100, 200)));
        assert_eq!(image_size("100x200, 50%"), Some((50, 100)));
        assert_eq!(image_size("100x200,50x"), Some((50, 100)));
        assert_eq!(image_size("100x200, x50"), Some((25, 50)));
        assert_eq!(image_size("0x200"), None);
        assert_eq!(image_size("wide"), None);
    }

    #[test]
    fn data_attributes() {
        assert_eq!(
            data_attribute("Foo=bar"),
            Some(("data-foo".to_string(), "bar".to_string()))
        );
        assert_eq!(data_attribute("no pair"), None);
        assert_eq!(data_attribute("a b=c"), None);
    }
}
