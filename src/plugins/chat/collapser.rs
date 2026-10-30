//! A message's text as ChatMessageText draws it when `isCollapsible`:
//! ChatMessageCollapser (components/chat-message-collapser.gjs) with its
//! Collapser headers, and ChatUpload for each upload. A message with
//! uploads shows its cooked, then the uploads behind one header; else
//! each top-level element of the cooked (domFromString's children) is
//! drawn on its own, an image, image onebox or gallery behind a header
//! of its link. Lazy videos (only from oneboxes) are refused.

use markup5ever_rcdom::{Handle, NodeData};
use serde_json::Value;

use crate::post_view::d_icon;
use crate::pretty_text::cleanup::{attr, element_name, fragment_root, has_class, node_html, parse};
use crate::topic_list_view::escape;
use crate::{AppError, Unsupported};

/// What the collapser reads besides the message.
pub(crate) struct Opts<'a> {
    pub base_path: &'a str,
    /// The page's url, which a link's `.href` and an image's `.src`
    /// resolve against.
    pub page_url: &'a str,
    /// max_image_width and max_image_height.
    pub max_image: (f64, f64),
    pub t: &'a (dyn Fn(&str) -> String + Sync),
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// `isImage` (discourse/lib/uploads)
fn is_image(name: &str) -> bool {
    has_extension(
        name,
        &[
            "png", "webp", "jpg", "jpeg", "gif", "svg", "ico", "heic", "heif", "avif", "jxl",
        ],
    )
}

/// `isVideo`
fn is_video(name: &str) -> bool {
    has_extension(
        name,
        &["mov", "mp4", "webm", "m4v", "3gp", "ogv", "avi", "mpeg"],
    )
}

/// `isAudio`
fn is_audio(name: &str) -> bool {
    has_extension(
        name,
        &[
            "mp3", "ogg", "oga", "opus", "wav", "m4a", "m4b", "m4p", "m4r", "aac", "flac",
        ],
    )
}

fn has_extension(name: &str, exts: &[&str]) -> bool {
    let lower = name.to_lowercase();
    exts.iter().any(|e| lower.ends_with(&format!(".{e}")))
}

/// `getURLWithCDN` without a CDN: a local path under the base path.
fn url_with_cdn(base_path: &str, url: &str) -> String {
    if url.starts_with('/')
        && !url.starts_with("//")
        && (base_path.is_empty() || !url.starts_with(&format!("{base_path}/")))
    {
        format!("{base_path}{url}")
    } else {
        url.to_string()
    }
}

/// A JS number as `String(n)` writes it, for the integral and short
/// fractional sizes ChatUpload computes.
fn js_number(n: f64) -> String {
    if n.is_finite() && n == n.trunc() && n.abs() < 1e21 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// ChatUpload
fn upload_html(upload: &Value, o: &Opts) -> String {
    let name = s(&upload["original_filename"]);
    if is_image(name) {
        // size: shrunk to fit, never blown up.
        let width = upload["width"].as_f64();
        let height = upload["height"].as_f64();
        // null * ratio is 0 in JS, and null / x Infinity.
        let w = width.unwrap_or(0.0);
        let h = height.unwrap_or(0.0);
        let ratio = 1f64.min(o.max_image.0 / w).min(o.max_image.1 / h);
        let thumb_width = w * ratio;
        let thumb_height = h * ratio;
        let image_url = url_with_cdn(
            o.base_path,
            upload["thumbnail"]["url"]
                .as_str()
                .unwrap_or_else(|| s(&upload["url"])),
        );
        let dominant = upload["dominant_color"].as_str();
        let mut out = String::from("<img class=\"chat-img-upload lightbox\"");
        if let Some(c) = dominant {
            out.push_str(&format!(" data-dominant-color=\"{}\"", escape(c)));
        }
        out.push_str(&format!(
            " data-download-href=\"{}\" data-large-src=\"{}\" data-orig-src=\"{}\"",
            escape(s(&upload["short_path"])),
            escape(&url_with_cdn(o.base_path, s(&upload["url"]))),
            escape(s(&upload["short_url"]))
        ));
        if let Some(h) = height {
            out.push_str(&format!(" data-target-height=\"{}\"", js_number(h)));
        }
        if let Some(w) = width {
            out.push_str(&format!(" data-target-width=\"{}\"", js_number(w)));
        }
        out.push_str(&format!(
            " height=\"{}\" loading=\"lazy\" src=\"{}\"",
            js_number(thumb_height),
            escape(&image_url)
        ));
        // imageStyle: the dominant color until the image loads.
        if let Some(c) = dominant.filter(|c| !c.is_empty()) {
            out.push_str(&format!(" style=\"background-color: #{};\"", escape(c)));
        }
        out.push_str(&format!(
            " tabindex=\"0\" width=\"{}\">",
            js_number(thumb_width)
        ));
        out
    } else if is_video(name) {
        // videoSourceUrl (chat.js adds Safari's #t=0.001).
        let src = upload["optimized_video"]["url"]
            .as_str()
            .unwrap_or_else(|| s(&upload["url"]));
        format!(
            "<video class=\"chat-video-upload\" controls=\"\" height=\"150\" preload=\"metadata\"><source src=\"{}\"></video>",
            escape(&url_with_cdn(o.base_path, src))
        )
    } else if is_audio(name) {
        format!(
            "<audio class=\"chat-audio-upload\" controls=\"\" preload=\"metadata\"><source src=\"{}\"></audio>",
            escape(&url_with_cdn(o.base_path, s(&upload["url"])))
        )
    } else {
        format!(
            "<a class=\"chat-other-upload\" data-orig-href=\"{}\" href=\"{}\">{}</a>",
            escape(s(&upload["short_url"])),
            escape(&url_with_cdn(o.base_path, s(&upload["url"]))),
            escape(name)
        )
    }
}

/// Collapser: the header with its toggle (open at first), then the body.
fn collapser(header: &str, body: &str) -> String {
    format!(
        "<div class=\"chat-message-collapser-header\">{header}<button class=\"btn no-text btn-icon chat-message-collapser-button chat-message-collapser-opened\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button></div><div class=\"chat-message-collapser-body\">{body}</div>",
        d_icon("angle-down", None)
    )
}

fn cooked_div(html: &str) -> String {
    format!("<div class=\"chat-cooked\">{html}</div>")
}

fn first_element_child(e: &Handle) -> Option<Handle> {
    e.children
        .borrow()
        .iter()
        .find(|c| matches!(c.data, NodeData::Element { .. }))
        .cloned()
}

fn is(e: &Option<Handle>, tag: &str) -> bool {
    e.as_ref().and_then(element_name) == Some(tag)
}

fn lazy_video(e: &Handle) -> bool {
    has_class(e, "lazy-video-container")
}

fn animated_image(e: &Handle) -> bool {
    let first = e.children.borrow().first().cloned();
    is(&first, "img")
        && first
            .as_ref()
            .is_some_and(|i| has_class(i, "animated") && has_class(i, "onebox"))
}

fn external_image_onebox(e: &Handle) -> bool {
    let a = first_element_child(e);
    is(&a, "a")
        && a.as_ref().is_some_and(|a| has_class(a, "onebox"))
        && is(&a.as_ref().and_then(first_element_child), "img")
}

fn image_onebox(e: &Handle) -> bool {
    animated_image(e) || external_image_onebox(e)
}

fn youtube_thumbnail_onebox(e: &Handle) -> bool {
    external_image_onebox(e)
        && first_element_child(e)
            .and_then(|a| first_element_child(&a))
            .is_some_and(|i| has_class(&i, "youtube-thumbnail"))
}

fn image(e: &Handle) -> bool {
    element_name(e) == Some("p")
        && first_element_child(e)
            .is_some_and(|i| element_name(&i) == Some("img") && !has_class(&i, "emoji"))
}

fn gallery(e: &Handle) -> bool {
    let a = first_element_child(e);
    is(&a, "a")
        && a.as_ref()
            .and_then(first_element_child)
            .is_some_and(|b| has_class(&b, "outer-box"))
}

/// domFromString: the cooked's top-level elements, with the document
/// that holds them (an RcDom empties its nodes when dropped).
fn elements(cooked: &str) -> (markup5ever_rcdom::RcDom, Vec<Handle>) {
    let dom = parse(cooked.trim());
    let elements = fragment_root(&dom)
        .children
        .borrow()
        .iter()
        .filter(|c| matches!(c.data, NodeData::Element { .. }))
        .cloned()
        .collect();
    (dom, elements)
}

/// `isCollapsible(cooked, uploads)`
pub(crate) fn is_collapsible(cooked: &str, uploads: &[Value]) -> bool {
    if !uploads.is_empty() {
        return true;
    }
    let (_dom, elements) = elements(cooked);
    elements
        .iter()
        .any(|e| lazy_video(e) || image_onebox(e) || image(e) || gallery(e))
}

/// An element's `.href` or `.src`: the attribute resolved against the
/// page's url.
fn resolved(o: &Opts, value: &str) -> String {
    url::Url::parse(o.page_url)
        .and_then(|base| base.join(value))
        .map(|u| u.to_string())
        .unwrap_or_else(|_| value.to_string())
}

fn small_link(link: &str, title: &str) -> String {
    format!(
        "<a target=\"_blank\" class=\"chat-message-collapser-link-small\" rel=\"noopener noreferrer\" href=\"{link}\">{title}</a>"
    )
}

/// ChatMessageCollapser, within chat-message-text.
pub(crate) fn collapser_html(
    cooked: &str,
    uploads: &[Value],
    o: &Opts,
) -> Result<String, AppError> {
    let mut out = String::from("<div class=\"chat-message-collapser\">");
    if !uploads.is_empty() {
        out.push_str(&cooked_div(cooked));
        // uploadsHeader
        let name = if uploads.len() == 1 {
            let u = &uploads[0];
            u["optimized_video"]["original_filename"]
                .as_str()
                .unwrap_or_else(|| s(&u["original_filename"]))
                .to_string()
        } else {
            (o.t)("chat.uploaded_files.other").replace("%{count}", &uploads.len().to_string())
        };
        let header = format!(
            "<span class=\"chat-message-collapser-link-small\">{}</span>",
            escape(&name)
        );
        let body: String = uploads.iter().map(|u| upload_html(u, o)).collect();
        out.push_str(&collapser(
            &header,
            &format!("<div class=\"chat-uploads\">{body}</div>"),
        ));
        out.push_str("</div>");
        return Ok(out);
    }
    let (_dom, elements) = elements(cooked);
    // cookedBodies: the first kind present decides which elements fold.
    #[derive(PartialEq)]
    enum Kind {
        ImageOnebox,
        Image,
        Gallery,
    }
    let kind = if elements.iter().any(lazy_video) {
        return Err(Unsupported("lazy videos in chat messages").into());
    } else if elements.iter().any(image_onebox) {
        Some(Kind::ImageOnebox)
    } else if elements.iter().any(image) {
        Some(Kind::Image)
    } else if elements.iter().any(gallery) {
        Some(Kind::Gallery)
    } else {
        None
    };
    for e in &elements {
        let header = match kind {
            Some(Kind::ImageOnebox) if image_onebox(e) => {
                let raw = if animated_image(e) {
                    e.children
                        .borrow()
                        .first()
                        .and_then(|i| attr(i, "src"))
                        .unwrap_or_default()
                } else {
                    first_element_child(e)
                        .and_then(|a| attr(&a, "href"))
                        .unwrap_or_default()
                };
                let link = escape(&resolved(o, &raw));
                let title = if youtube_thumbnail_onebox(e) {
                    (o.t)("chat.youtube")
                } else {
                    link.clone()
                };
                Some(small_link(&link, &title))
            }
            Some(Kind::Image) if image(e) => {
                let img = first_element_child(e);
                let link = escape(&resolved(
                    o,
                    &img.as_ref()
                        .and_then(|i| attr(i, "src"))
                        .unwrap_or_default(),
                ));
                let alt = escape(
                    &img.as_ref()
                        .and_then(|i| attr(i, "alt"))
                        .unwrap_or_default(),
                );
                let title = if alt.is_empty() { link.clone() } else { alt };
                Some(small_link(&link, &title))
            }
            Some(Kind::Gallery) if gallery(e) => {
                let a = first_element_child(e);
                let link = escape(&resolved(
                    o,
                    &a.as_ref().and_then(|a| attr(a, "href")).unwrap_or_default(),
                ));
                let title = a
                    .as_ref()
                    .and_then(first_element_child)
                    .map(|b| crate::pretty_text::cleanup::text(&b))
                    .unwrap_or_default();
                // The title box is taken out of the body.
                if let Some(a) = &a {
                    let mut children = a.children.borrow_mut();
                    if let Some(pos) = children
                        .iter()
                        .position(|c| matches!(c.data, NodeData::Element { .. }))
                    {
                        children.remove(pos);
                    }
                }
                Some(small_link(&link, &escape(&title)))
            }
            _ => None,
        };
        let body = cooked_div(&node_html(e));
        match header {
            Some(header) => out.push_str(&collapser(&header, &body)),
            None => out.push_str(&body),
        }
    }
    out.push_str("</div>");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t(key: &str) -> String {
        match key {
            "chat.uploaded_files.other" => "%{count} files".into(),
            "chat.youtube" => "YouTube".into(),
            k => k.into(),
        }
    }

    fn opts() -> Opts<'static> {
        Opts {
            base_path: "",
            page_url: "http://test.localhost/chat/c/general/2",
            max_image: (690.0, 500.0),
            t: &t,
        }
    }

    fn png(width: i64, height: i64) -> Value {
        json!({
            "id": 7,
            "url": "/uploads/default/original/1X/abc.png",
            "original_filename": "cat.png",
            "width": width,
            "height": height,
            "short_url": "upload://abc.png",
            "short_path": "/uploads/short-url/abc.png",
            "dominant_color": "AABBCC",
            "thumbnail": null,
        })
    }

    #[test]
    fn one_image_upload_folds_under_its_name() {
        let html = collapser_html("", &[png(1380, 500)], &opts()).unwrap();
        assert_eq!(
            html,
            format!(
                "<div class=\"chat-message-collapser\"><div class=\"chat-cooked\"></div><div class=\"chat-message-collapser-header\"><span class=\"chat-message-collapser-link-small\">cat.png</span><button class=\"btn no-text btn-icon chat-message-collapser-button chat-message-collapser-opened\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button></div><div class=\"chat-message-collapser-body\"><div class=\"chat-uploads\"><img class=\"chat-img-upload lightbox\" data-dominant-color=\"AABBCC\" data-download-href=\"/uploads/short-url/abc.png\" data-large-src=\"/uploads/default/original/1X/abc.png\" data-orig-src=\"upload://abc.png\" data-target-height=\"500\" data-target-width=\"1380\" height=\"250\" loading=\"lazy\" src=\"/uploads/default/original/1X/abc.png\" style=\"background-color: #AABBCC;\" tabindex=\"0\" width=\"690\"></div></div></div>",
                d_icon("angle-down", None)
            )
        );
    }

    #[test]
    fn several_uploads_count_and_draw_by_type() {
        let video = json!({"original_filename": "clip.MP4", "url": "/uploads/v.mp4"});
        let audio = json!({"original_filename": "song.mp3", "url": "/uploads/s.mp3"});
        let other = json!({"original_filename": "notes <1>.pdf", "url": "/uploads/n.pdf", "short_url": "upload://n.pdf"});
        let o = Opts {
            base_path: "/forum",
            ..opts()
        };
        let html = collapser_html("<p>hi</p>", &[video, audio, other], &o).unwrap();
        assert!(html.contains("<div class=\"chat-cooked\"><p>hi</p></div>"));
        assert!(html.contains("<span class=\"chat-message-collapser-link-small\">3 files</span>"));
        assert!(html.contains("<video class=\"chat-video-upload\" controls=\"\" height=\"150\" preload=\"metadata\"><source src=\"/forum/uploads/v.mp4\"></video>"));
        assert!(html.contains("<audio class=\"chat-audio-upload\" controls=\"\" preload=\"metadata\"><source src=\"/forum/uploads/s.mp3\"></audio>"));
        assert!(html.contains("<a class=\"chat-other-upload\" data-orig-href=\"upload://n.pdf\" href=\"/forum/uploads/n.pdf\">notes &lt;1&gt;.pdf</a>"));
    }

    #[test]
    fn small_images_are_not_blown_up() {
        let html = collapser_html("", &[png(100, 50)], &opts()).unwrap();
        assert!(html.contains("height=\"50\" loading=\"lazy\""));
        assert!(html.contains("width=\"100\">"));
    }

    #[test]
    fn a_cooked_image_folds_under_its_alt_or_link() {
        let cooked = "<p>look</p>\n<p><img src=\"/uploads/x.png\" alt=\"kitten\"></p>";
        assert!(is_collapsible(cooked, &[]));
        let html = collapser_html(cooked, &[], &opts()).unwrap();
        assert!(html.starts_with("<div class=\"chat-message-collapser\"><div class=\"chat-cooked\"><p>look</p></div><div class=\"chat-message-collapser-header\"><a target=\"_blank\" class=\"chat-message-collapser-link-small\" rel=\"noopener noreferrer\" href=\"http://test.localhost/uploads/x.png\">kitten</a>"));
        assert!(html.ends_with("<div class=\"chat-message-collapser-body\"><div class=\"chat-cooked\"><p><img src=\"/uploads/x.png\" alt=\"kitten\"></p></div></div></div>"));
    }

    #[test]
    fn emoji_and_plain_text_do_not_fold() {
        assert!(!is_collapsible(
            "<p><img class=\"emoji\" src=\"/e.png\"> hi</p>",
            &[]
        ));
        assert!(!is_collapsible("<p>hi</p>", &[]));
    }

    #[test]
    fn a_youtube_onebox_is_titled_youtube() {
        let cooked = "<div class=\"youtube-onebox\"><a class=\"onebox\" href=\"https://youtu.be/x\"><img class=\"youtube-thumbnail\" src=\"https://i.ytimg.com/x.jpg\"></a></div>";
        let html = collapser_html(cooked, &[], &opts()).unwrap();
        assert!(html.contains("href=\"https://youtu.be/x\">YouTube</a>"));
    }

    #[test]
    fn a_gallery_loses_its_title_box_to_the_header() {
        let cooked = "<div><a href=\"https://imgur.com/a/x\"><span class=\"outer-box\">My album</span><img src=\"https://i.imgur.com/x.png\"></a></div>";
        let html = collapser_html(cooked, &[], &opts()).unwrap();
        assert!(html.contains("href=\"https://imgur.com/a/x\">My album</a>"));
        assert!(html.contains("<div class=\"chat-cooked\"><div><a href=\"https://imgur.com/a/x\"><img src=\"https://i.imgur.com/x.png\"></a></div></div>"));
    }
}
