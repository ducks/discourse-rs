//! Port of lib/email/styles.rb: the inline styles an email's HTML gets,
//! since mail clients drop `<style>` blocks. Rails selects with Nokogiri's
//! CSS engine; crate::pretty_text::select matches the selectors it uses.

use std::cell::RefCell;
use std::rc::Rc;

use html5ever::serialize::{SerializeOpts, TraversalScope, serialize};
use html5ever::tendril::StrTendril;
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom, SerializableHandle};

use crate::pretty_text::cleanup::{
    append, attr, element_name, fragment_root, new_element, new_text, parse, parse_document,
    replace, replace_with, set_attr, text,
};
use crate::pretty_text::select::{parent, select};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

// ---- the document ----

/// `node['xml:lang'] = value`: libxml2 sets the attribute in the XML
/// namespace, a different one from the `xml:lang` the HTML parser read as
/// a plain name, so a template carrying one serializes it twice (the next
/// parse keeps the first).
fn set_xml_lang(node: &Handle, value: &str) {
    use html5ever::{LocalName, Prefix, QualName, namespace_url, ns};
    if let NodeData::Element { attrs, .. } = &node.data {
        let mut attrs = attrs.borrow_mut();
        let existing = attrs
            .iter_mut()
            .find(|a| a.name.ns == ns!(xml) && &*a.name.local == "lang");
        match existing {
            Some(a) => a.value = value.into(),
            None => attrs.push(html5ever::Attribute {
                name: QualName::new(Some(Prefix::from("xml")), ns!(xml), LocalName::from("lang")),
                value: value.into(),
            }),
        }
    }
}

/// `ONEBOX_IMAGE_BASE_STYLE`
const ONEBOX_IMAGE_BASE_STYLE: &str =
    "max-height: 80%; max-width: 20%; height: auto; float: left; margin-right: 10px;";

/// `node.inner_html`
fn inner_html(node: &Handle) -> String {
    let mut out = Vec::new();
    let handle: SerializableHandle = node.clone().into();
    serialize(
        &mut out,
        &handle,
        SerializeOpts {
            traversal_scope: TraversalScope::ChildrenOnly(None),
            ..Default::default()
        },
    )
    .expect("writing to a vector cannot fail");
    String::from_utf8(out).expect("the serializer writes UTF-8")
}

/// `node.inner_html = html`: the children replaced by the parsed fragment.
fn set_inner_html(node: &Handle, html: &str) {
    for child in node.children.take() {
        child.parent.set(None);
    }
    let dom = parse(html);
    for child in fragment_root(&dom).children.take() {
        append(node, child);
    }
}

/// `node.name = name`: the element, its attributes and children kept,
/// under another name.
fn rename(node: &Handle, name: &str) {
    use html5ever::{LocalName, QualName, namespace_url, ns};
    let NodeData::Element { attrs, .. } = &node.data else {
        return;
    };
    let renamed = Node::new(NodeData::Element {
        name: QualName::new(None, ns!(html), LocalName::from(name)),
        attrs: RefCell::new(attrs.borrow().clone()),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    });
    for child in node.children.take() {
        append(&renamed, child);
    }
    replace(node, renamed);
}

/// The iframe addresses whose reading by Ruby's `URI()` this port is sure
/// of: `http(s)://host` or `//host` (no user, no port, lowercase scheme),
/// then a path, query and fragment of URI characters other than `'`
/// (the replacement puts the address in a single-quoted attribute).
struct IframeUri<'a> {
    scheme: Option<&'a str>,
    host: &'a str,
    rest: &'a str,
}

impl<'a> IframeUri<'a> {
    fn parse(address: &'a str) -> Option<IframeUri<'a>> {
        let (scheme, after) = if let Some(r) = address.strip_prefix("https://") {
            (Some("https"), r)
        } else if let Some(r) = address.strip_prefix("http://") {
            (Some("http"), r)
        } else {
            (None, address.strip_prefix("//")?)
        };
        let host_end = after.find(['/', '?', '#']).unwrap_or(after.len());
        let (host, rest) = after.split_at(host_end);
        let host_ok = !host.is_empty()
            && !host.starts_with(['.', '-'])
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
        let rest_ok = rest.matches('#').count() <= 1
            && rest.chars().all(|c| {
                c.is_ascii_alphanumeric() || "-._~%!$&()*+,;=:@/?#".contains(c)
            })
            // every `%` an escape
            && rest
                .split('%')
                .skip(1)
                .all(|h| h.len() >= 2 && h.as_bytes()[..2].iter().all(u8::is_ascii_hexdigit));
        (host_ok && rest_ok).then_some(IframeUri { scheme, host, rest })
    }

    /// `"#{scheme || "https"}://#{host}#{path}#{"?" + query}#{"#" + fragment}"`
    fn display(&self) -> String {
        format!(
            "{}://{}{}",
            self.scheme.unwrap_or("https"),
            self.host,
            self.rest
        )
    }
}

/// `Email::Styles`
pub struct Styles {
    dom: RcDom,
}

fn is_blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

fn remove(node: &Handle) {
    if let Some(p) = parent(node) {
        p.children.borrow_mut().retain(|c| !Rc::ptr_eq(c, node));
    }
    node.parent.set(None);
}

/// `deduplicate_style`: later declarations win unless an earlier one is
/// `!important`; the first position of a property is kept.
pub fn deduplicate_style(style: &str) -> String {
    let mut styles: Vec<(String, String)> = Vec::new();
    for part in style.split(';').filter(|p| !is_blank(p)) {
        let mut kv = part.splitn(2, ':').map(str::trim);
        let (Some(k), Some(v)) = (kv.next(), kv.next()) else {
            continue;
        };
        if k.is_empty() || v.is_empty() {
            continue;
        }
        match styles.iter_mut().find(|(key, _)| key == k) {
            Some((_, existing)) => {
                if existing.ends_with("!important") && !v.ends_with("!important") {
                    continue;
                }
                *existing = v.to_string();
            }
            None => styles.push((k.to_string(), v.to_string())),
        }
    }
    styles
        .iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect::<Vec<_>>()
        .join(";")
}

impl Styles {
    /// `Nokogiri::HTML5.parse(html)`
    pub fn new(html: &str) -> Styles {
        Styles {
            dom: parse_document(html),
        }
    }

    fn select(&self, selector: &str) -> Vec<Handle> {
        select(&self.dom.document, selector)
    }

    fn add_styles(node: &Handle, new_styles: &str) {
        match attr(node, "style").filter(|s| !is_blank(s)) {
            Some(existing) => set_attr(node, "style", &format!("{new_styles}; {existing}")),
            None => set_attr(node, "style", new_styles),
        }
    }

    fn style(&self, selector: &str, style: Option<&str>, attrs: &[(&str, &str)]) {
        for node in self.select(selector) {
            if let Some(style) = style {
                Self::add_styles(&node, style);
            }
            for (k, v) in attrs {
                if *k == "xml:lang" {
                    set_xml_lang(&node, v);
                } else {
                    set_attr(&node, k, v);
                }
            }
        }
    }

    /// What this port refuses rather than restyles, checked before anything
    /// changes: user oneboxes' table layout, iframes whose address Ruby's
    /// URI might read otherwise than this port, secure media.
    fn refuse_unported(&self) -> Result<(), Unsupported> {
        if !self.select(".user-onebox .aspect-image img").is_empty() {
            return Err(Unsupported("user oneboxes in emails"));
        }
        for iframe in self.select("iframe") {
            let Some(src) = attr(&iframe, "src").filter(|s| !is_blank(s)) else {
                continue;
            };
            let address = attr(&iframe, "data-original-href")
                .filter(|h| !is_blank(h))
                .unwrap_or(src);
            if IframeUri::parse(&address).is_none() {
                return Err(Unsupported("iframes in emails with an unusual address"));
            }
        }
        if !self
            .select("[data-stripped-secure-media], [data-stripped-secure-upload]")
            .is_empty()
        {
            return Err(Unsupported("secure media in emails"));
        }
        Ok(())
    }

    /// `format_basic`
    pub fn format_basic(&self, base_url: &str) -> Result<(), AppError> {
        self.refuse_unported()?;
        let scheme = base_url.split("://").next().unwrap_or("http");
        for node in self.select("svg, img[src$=\".svg\"]") {
            remove(&node);
        }
        for img in self.select("img") {
            if attr(&img, "class").as_deref() == Some("site-logo") {
                continue;
            }
            let class = attr(&img, "class");
            let src = attr(&img, "src");
            let emoji_src = src
                .as_deref()
                .is_some_and(|s| s.contains("/emoji/") || s.contains("/_emoji/"));
            if class.as_deref().is_some_and(|c| c.contains("emoji")) || emoji_src {
                set_attr(&img, "height", "20");
                set_attr(&img, "width", "20");
            } else {
                let to_i =
                    |name: &str| attr(&img, name).map(|v| crate::ruby::to_i(&v)).unwrap_or(0);
                if to_i("width") > 320 || to_i("height") > 480 {
                    set_attr(&img, "height", "auto");
                    set_attr(&img, "width", "auto");
                }
            }
            if let Some(src) = attr(&img, "src") {
                if src.starts_with('/') && !src.starts_with("//") {
                    set_attr(&img, "src", &format!("{base_url}{src}"));
                }
                let src = attr(&img, "src").unwrap_or_default();
                if src.starts_with("//") {
                    set_attr(&img, "src", &format!("{scheme}:{src}"));
                }
            }
        }
        let excluded = self.select("aside.onebox img, img.site-logo, img.emoji");
        for img in self.select("img[width=\"auto\"][height=\"auto\"]") {
            if excluded.iter().any(|e| Rc::ptr_eq(e, &img)) {
                continue;
            }
            if !attr(&img, "style").is_some_and(|s| s.contains("max-width")) {
                Self::add_styles(&img, "max-width: 100%;");
            }
        }
        for a in self.select("a.topic-featured-link") {
            set_attr(
                &a,
                "style",
                "color:#858585;padding:2px 8px;border:1px solid #e6e6e6;border-radius:2px;box-shadow:0 1px 3px rgba(0, 0, 0, 0.12), 0 1px 2px rgba(0, 0, 0, 0.24);",
            );
        }
        for a in self.select("a.attachment") {
            if let Some(href) = attr(&a, "href")
                && href.starts_with('/')
                && !href.starts_with("//")
            {
                set_attr(&a, "href", &format!("{base_url}{href}"));
            }
            if let Some(href) = attr(&a, "href").filter(|h| h.starts_with("//")) {
                set_attr(&a, "href", &format!("{scheme}:{href}"));
            }
        }
        Ok(())
    }

    /// `format_html`
    pub fn format_html(&self, settings: &SiteSettings) -> Result<(), AppError> {
        // custom_styles: EmailStyle#compiled_css, read before anything is
        // styled so a refusal comes first.
        let compiled_css = match settings.get("email_custom_css_compiled")?.presence() {
            Some(css) => Some(css),
            None => settings.get("email_custom_css")?.presence(),
        };
        let custom_styles = match compiled_css {
            Some(css) => super::custom_css::custom_styles(&css)?,
            None => Vec::new(),
        };
        let link_color = settings.get("email_link_color")?.to_s();
        let accent_bg = settings.get("email_accent_bg_color")?.to_s();
        let accent_fg = settings.get("email_accent_fg_color")?.to_s();
        let locale = settings.get("default_locale")?.to_s();
        if matches!(locale.as_str(), "ar" | "fa_IR" | "he" | "ug" | "ur") {
            return Err(Unsupported("right-to-left email styles").into());
        }
        let (align, dir) = ("left", "ltr");

        // correct_first_body_margin
        for p in self.select("div.body p") {
            set_attr(&p, "style", "margin-top:0; border: 0;");
        }
        // correct_footer_style
        for footer in self.select(".footer") {
            set_attr(&footer, "style", "color:#666;");
            for a in self.select_within(&footer, "a") {
                set_attr(&a, "style", "color:#666;");
            }
        }
        // correct_footer_style_highlight_first: the first link of the
        // first highlighted footer.
        if let Some(footer) = self.select(".footer.highlight").first()
            && let Some(a) = self.select_within(footer, "a").first()
        {
            set_attr(
                a,
                "style",
                &format!(
                    "background-color: {accent_bg}; color: {accent_fg}; border-top: 4px solid {accent_bg}; border-right: 6px solid {accent_bg}; border-bottom: 4px solid {accent_bg}; border-left: 6px solid {accent_bg}; display: inline-block; font-weight: bold;"
                ),
            );
        }
        // strip_hashtag_link_icons
        for hashtag in self.select(".hashtag-cooked") {
            hashtag.children.borrow_mut().clear();
            let slug = attr(&hashtag, "data-slug").unwrap_or_default();
            let node = Node::new(NodeData::Text {
                contents: RefCell::new(StrTendril::from(format!("#{slug}"))),
            });
            node.parent.set(Some(Rc::downgrade(&hashtag)));
            hashtag.children.borrow_mut().push(node);
        }
        // reset_tables
        self.style(
            "table",
            None,
            &[("cellspacing", "0"), ("cellpadding", "0"), ("border", "0")],
        );
        let html_lang = locale.replace('_', "-");
        self.style(
            "html",
            None,
            &[("lang", &html_lang), ("xml:lang", &html_lang)],
        );
        let body = format!("line-height: 1.4; text-align:{align};");
        self.style("body", Some(&body), &[]);
        self.style("body", None, &[("dir", dir)]);
        let with_dir = format!("text-align:{align};");
        self.style(".with-dir", Some(&with_dir), &[("dir", dir)]);
        self.style("blockquote > :first-child", Some("margin-top: 0;"), &[]);
        self.style("blockquote > :last-child", Some("margin-bottom: 0;"), &[]);
        self.style("blockquote > p", Some("padding: 0;"), &[]);
        let accent = format!("background-color: {accent_bg}; color: {accent_fg};");
        self.style(".with-accent-colors", Some(&accent), &[]);
        self.style("h4", Some("color: #222;"), &[]);
        self.style("h3", Some("margin: 30px 0 10px;"), &[]);
        self.style(
            "hr",
            Some("background-color: #ddd; height: 1px; border: 1px;"),
            &[],
        );
        let a = format!("text-decoration: none; font-weight: bold; color: {link_color};");
        self.style("a", Some(&a), &[]);
        self.style("ul", Some("margin: 0 0 0 10px; padding: 0 0 0 20px;"), &[]);
        self.style("li", Some("padding-bottom: 10px"), &[]);
        self.style(
            "div.summary-footer",
            Some("color:#666; font-size:95%; text-align:center; padding-top:15px;"),
            &[],
        );
        self.style("span.post-count", Some("margin: 0 5px; color: #777;"), &[]);
        self.style("pre", Some("word-wrap: break-word; max-width: 694px;"), &[]);
        self.style(
            "code",
            Some("background-color: #f9f9f9; padding: 2px 5px;"),
            &[],
        );
        self.style(
            "pre code",
            Some("display: block; background-color: #f9f9f9; overflow: auto; padding: 5px;"),
            &[],
        );
        self.style("pre.onebox code", Some("white-space: normal;"), &[]);
        self.style("pre code li", Some("white-space: pre;"), &[]);
        let featured = format!(
            "text-decoration: none; font-weight: bold; color: {link_color}; line-height:1.5em;"
        );
        self.style(".featured-topic a", Some(&featured), &[]);
        self.style(
            ".summary-email",
            Some("-moz-box-sizing:border-box;-ms-text-size-adjust:100%;-webkit-box-sizing:border-box;-webkit-text-size-adjust:100%;box-sizing:border-box;color:#0a0a0a;font-family:Arial,sans-serif;font-size:14px;font-weight:400;line-height:1.3;margin:0;min-width:100%;padding:0;width:100%"),
            &[],
        );
        self.style(".email-preview", Some("display: none;"), &[]);
        self.style(
            ".previous-discussion",
            Some("font-size: 17px; color: #444; margin-bottom:10px;"),
            &[],
        );
        self.style(
            ".notification-date",
            Some("text-align:right;color:#999999;padding-right:5px;font-family:'lucida grande',tahoma,verdana,arial,sans-serif;font-size:11px"),
            &[],
        );
        self.style(
            ".username",
            Some("font-size:13px;font-family:'lucida grande',tahoma,verdana,arial,sans-serif;text-decoration:none;font-weight:bold"),
            &[],
        );
        let username_link = format!("color:{link_color};");
        self.style(".username-link", Some(&username_link), &[]);
        self.style(".username-title", Some("color:#777;margin-left:5px;"), &[]);
        self.style(
            ".user-title",
            Some("font-size:13px;font-family:'lucida grande',tahoma,verdana,arial,sans-serif;text-decoration:none;margin-left:5px;color: #999;"),
            &[],
        );
        self.style(".post-wrapper", Some("margin-bottom:25px;"), &[]);
        self.style(".user-avatar", Some("vertical-align:top;width:55px;"), &[]);
        self.style(
            ".user-avatar img",
            None,
            &[("width", "45"), ("height", "45")],
        );
        self.style(
            "hr",
            Some("background-color: #ddd; height: 1px; border: 1px;"),
            &[],
        );
        self.style(".rtl", Some("direction: rtl;"), &[]);
        self.style("div.body", Some("padding-top:5px;"), &[]);
        self.style(
            ".whisper div.body",
            Some("font-style: italic; color: #9c9c9c;"),
            &[],
        );
        self.style(".lightbox-wrapper .meta", Some("display: none"), &[]);
        self.style(
            "div.undecorated-link-footer a",
            Some("font-weight: normal;"),
            &[],
        );
        let mso = format!("mso-border-alt: 6px solid {accent_bg}; background-color: {accent_bg};");
        self.style(".mso-accent-link", Some(&mso), &[]);
        self.style(
            ".reply-above-line",
            Some("font-size: 10px;font-family:'lucida grande',tahoma,verdana,arial,sans-serif;color: #b5b5b5;padding: 5px 0px 20px;border-top: 1px dotted #ddd;"),
            &[],
        );
        self.onebox_styles();
        // plugin_styles: chat's (Email::Styles.register_plugin_style), its
        // summary's header and messages for the dark mode styles.
        self.style(".chat-summary-header", None, &[("dm", "header")]);
        self.style(".chat-summary-content", None, &[("dm", "body")]);
        // discourse-details': elided parts out, each details its summary
        // as a paragraph.
        for details in self.select("details.elided") {
            remove(&details);
        }
        for details in self.select("details") {
            let Some(summary) = self.select_within(&details, "summary").into_iter().next() else {
                continue;
            };
            let attrs: Vec<(String, String)> = match &summary.data {
                NodeData::Element { attrs, .. } => attrs
                    .borrow()
                    .iter()
                    .map(|a| (a.name.local.to_string(), a.value.to_string()))
                    .collect(),
                _ => Vec::new(),
            };
            let pairs: Vec<(&str, &str)> = attrs
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let p = crate::pretty_text::cleanup::new_element("p", &pairs);
            for child in summary.children.take() {
                crate::pretty_text::cleanup::append(&p, child);
            }
            crate::pretty_text::cleanup::replace(&details, p);
        }
        // dark_mode_styles
        self.style(
            ".digest-header, .digest-topic, .digest-topic-title-wrapper, .digest-topic-stats, .popular-post-excerpt",
            None,
            &[("dm", "header")],
        );
        self.style(
            ".digest-content, .header-popular-posts, .spacer, .popular-post-spacer, .popular-post-meta, .digest-new-header, .digest-new-topic, .body",
            None,
            &[("dm", "body")],
        );
        self.style(
            ".with-accent-colors, .digest-content-header",
            None,
            &[("dm", "body_primary")],
        );
        self.style(".digest-topic-body", None, &[("dm", "topic-body")]);
        self.style(".summary-footer", None, &[("dm", "text-color")]);
        self.style(
            ".post-excerpt img:not(.emoji)",
            Some("max-width: 50%; max-height: 400px;"),
            &[],
        );
        self.style(
            ".post-excerpt img.emoji",
            Some("max-height: 20px; vertical-align: middle;"),
            &[],
        );
        // format_custom
        for (selector, value) in &custom_styles {
            self.style(selector, Some(value), &[]);
        }
        Ok(())
    }

    /// `onebox_styles`: quotes and oneboxes styled, each quote made a
    /// blockquote of its title and quoted text, every `aside`, `article`
    /// and `header` a `div`, and iframes links to their address. A user
    /// onebox's table layout is refused (refuse_unported).
    fn onebox_styles(&self) {
        self.style(
            "aside.quote",
            Some("padding: 12px 25px 2px 12px; margin-bottom: 10px;"),
            &[],
        );
        self.style(
            "aside.quote div.info-line",
            Some("color: #666; margin: 10px 0"),
            &[],
        );
        self.style(
            "aside.quote .avatar",
            Some("margin-right: 5px; width:20px; height:20px; vertical-align:middle;"),
            &[],
        );
        self.style(
            "aside.quote",
            Some("border-left: 5px solid #e9e9e9; background-color: #f8f8f8; margin: 0;"),
            &[],
        );
        self.style("blockquote", Some("border-left: 5px solid #e9e9e9; background-color: #f8f8f8; margin-left: 0; padding: 12px;"), &[]);
        self.style(
            "aside.onebox",
            Some("border: 5px solid #e9e9e9; padding: 12px 25px 12px 12px; margin-bottom: 10px;"),
            &[],
        );
        self.style(
            "aside.onebox header img.site-icon",
            Some("width: 16px; height: 16px; margin-right: 3px;"),
            &[],
        );
        self.style(
            "aside.onebox header a[href]",
            Some("color: #222222; text-decoration: none;"),
            &[],
        );
        self.style("aside.onebox .onebox-body", Some("clear: both"), &[]);
        self.style(
            "aside.onebox .onebox-body img:not(.onebox-avatar-inline)",
            Some(ONEBOX_IMAGE_BASE_STYLE),
            &[],
        );
        self.style(
            "aside.onebox .onebox-body img.thumbnail",
            Some("width: 60px;"),
            &[],
        );
        self.style(
            "aside.onebox .onebox-body h3, aside.onebox .onebox-body h4",
            Some("font-size: 1.17em; margin: 10px 0;"),
            &[],
        );
        self.style(".onebox-metadata", Some("color: #919191"), &[]);
        self.style(".github-info", Some("margin-top: 10px;"), &[]);
        self.style(".github-info .added", Some("color: #090;"), &[]);
        self.style(".github-info .removed", Some("color: #e45735;"), &[]);
        self.style(
            ".github-info div",
            Some("display: inline; margin-right: 10px;"),
            &[],
        );
        self.style(".github-icon-container", Some("float: left;"), &[]);
        self.style(
            ".github-icon-container *",
            Some("fill: #646464; width: 40px; height: 40px;"),
            &[],
        );
        self.style(
            ".github-body-container",
            Some("font-family: Consolas, Menlo, Monaco, \"Lucida Console\", \"Liberation Mono\", \"DejaVu Sans Mono\", \"Bitstream Vera Sans Mono\", \"Courier New\", monospace; margin-top: 1em !important;"),
            &[],
        );
        self.style(
            ".onebox-avatar-inline",
            Some("width: 20px; height: 20px; float: none; vertical-align: middle;"),
            &[],
        );
        for excerpt in self.select(".github-body-container .excerpt") {
            remove(&excerpt);
        }
        for p in self.select("aside.quote blockquote > p") {
            set_attr(&p, "style", "padding: 0;");
        }
        // Each quote, in document order: its title's and blockquotes'
        // inner HTML (every one within, a nested quote's too, as a
        // NodeSet's inner_html joins them) less the quote controls. A
        // nested quote is replaced along with its parent's content, so
        // what is done to it after shows nowhere (and Rails' copy of the
        // quote, the controls removed, reads the same as the quote).
        for quote in self.select("aside.quote") {
            let copy = quote.clone();
            for controls in select(&copy, "div.quote-controls") {
                remove(&controls);
            }
            let quoted: String = select(&copy, "blockquote").iter().map(inner_html).collect();
            // String#strip: ASCII whitespace and NUL.
            let stripped = quoted.trim_start_matches([' ', '\t', '\n', '\x0B', '\x0C', '\r', '\0']);
            let blockquote = if stripped.starts_with("<p") {
                quoted
            } else {
                format!("<p style='padding: 0;'>{quoted}</p>")
            };
            let title: String = select(&copy, "div.title").iter().map(inner_html).collect();
            set_inner_html(&quote, &format!("{title}{blockquote}"));
            rename(&quote, "blockquote");
        }
        for node in self.select("aside, article, header") {
            rename(&node, "div");
        }
        for iframe in self.select("iframe") {
            let Some(src) = attr(&iframe, "src").filter(|s| !is_blank(s)) else {
                remove(&iframe);
                continue;
            };
            let address = attr(&iframe, "data-original-href")
                .filter(|h| !is_blank(h))
                .unwrap_or(src);
            // refuse_unported let through only addresses this reads.
            let Some(uri) = IframeUri::parse(&address) else {
                remove(&iframe);
                continue;
            };
            let a = new_element("a", &[("href", &address)]);
            append(&a, new_text(&uri.display()));
            let p = new_element("p", &[]);
            append(&p, a);
            // The fragment's unclosed second `<p>`.
            replace_with(&iframe, vec![p, new_element("p", &[])]);
        }
    }

    fn select_within(&self, root: &Handle, selector: &str) -> Vec<Handle> {
        select(root, selector)
    }

    /// `to_html`: protocol-relative links get the site's scheme, relative
    /// links its origin, styles deduplicated, then serialized.
    pub fn to_html(&self, base_url: &str, base_url_no_prefix: &str) -> String {
        let (scheme, host) = match base_url.split_once("://") {
            Some((scheme, rest)) => (scheme, rest.split(['/', ':']).next().unwrap_or("")),
            None => ("http", ""),
        };
        for node in self.select("[href]") {
            let href = attr(&node, "href").unwrap_or_default();
            if href.starts_with(&format!("//{host}")) {
                set_attr(&node, "href", &format!("{scheme}:{href}"));
            }
        }
        for a in self.select("a[href]") {
            let href = attr(&a, "href").unwrap_or_default();
            if href.starts_with('/') && !href.starts_with("//") {
                set_attr(&a, "href", &format!("{base_url_no_prefix}{href}"));
            }
        }
        for node in self.select("[style]") {
            let style = attr(&node, "style").unwrap_or_default();
            set_attr(&node, "style", &deduplicate_style(&style));
        }
        self.to_s()
    }

    /// `strip_avatars_and_emojis`, for short emails.
    pub fn strip_avatars_and_emojis(&self) {
        for img in self.select("img") {
            let Some(src) = attr(&img, "src") else {
                continue;
            };
            if src.contains("_avatar") {
                if let Some(p) = parent(&img)
                    && element_name(&p) == Some("td")
                {
                    set_attr(&p, "style", "vertical-align: top;");
                }
                remove(&img);
                continue;
            }
            if let Some(title) = attr(&img, "title")
                && (src.contains("/emoji/") || src.contains("/_emoji/"))
                && let Some(p) = parent(&img)
            {
                let node = Node::new(NodeData::Text {
                    contents: RefCell::new(StrTendril::from(title)),
                });
                node.parent.set(Some(Rc::downgrade(&p)));
                let mut children = p.children.borrow_mut();
                if let Some(i) = children.iter().position(|c| Rc::ptr_eq(c, &img)) {
                    children[i] = node;
                }
            }
        }
    }

    /// The document serialized (`to_s`).
    pub fn to_s(&self) -> String {
        let mut out = Vec::new();
        let handle: SerializableHandle = self.dom.document.clone().into();
        serialize(
            &mut out,
            &handle,
            SerializeOpts {
                traversal_scope: TraversalScope::ChildrenOnly(None),
                ..Default::default()
            },
        )
        .expect("writing to a vector cannot fail");
        String::from_utf8(out).expect("the serializer writes UTF-8")
    }

    /// The text of the whole document (for tests).
    pub fn text(&self) -> String {
        text(&self.dom.document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(html: &str, selector: &str) -> usize {
        Styles::new(html).select(selector).len()
    }

    #[test]
    fn selectors() {
        let html = r#"<div class="footer highlight"><a href="/x">x</a><p>1</p><p>2</p></div><blockquote><p>q</p></blockquote><img src="a.svg"><img src="b.png" class="emoji">"#;
        assert_eq!(count(html, ".footer.highlight"), 1);
        assert_eq!(count(html, ".footer a"), 1);
        assert_eq!(count(html, "div > p:first-child"), 0);
        assert_eq!(count(html, "div > a:first-child"), 1);
        assert_eq!(count(html, "div > p:last-child"), 1);
        assert_eq!(count(html, "blockquote > p"), 1);
        assert_eq!(count(html, "img[src$=\".svg\"]"), 1);
        assert_eq!(count(html, "img:not(.emoji)"), 1);
        assert_eq!(count(html, "svg, img"), 2);
    }

    #[test]
    fn styles_deduplicate() {
        assert_eq!(
            deduplicate_style("margin-top:0; border: 0;"),
            "margin-top:0;border:0"
        );
        assert_eq!(
            deduplicate_style("color: red !important; color: blue; padding: 1px"),
            "color:red !important;padding:1px"
        );
    }
}
