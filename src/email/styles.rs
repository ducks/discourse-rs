//! Port of lib/email/styles.rb: the inline styles an email's HTML gets,
//! since mail clients drop `<style>` blocks. Rails selects with Nokogiri's
//! CSS engine; the selectors it uses are matched here by a small engine
//! (tags, classes, attributes, `:not`, `:first-child`, `:last-child`,
//! descendant and child combinators).

use std::cell::RefCell;
use std::rc::Rc;

use html5ever::serialize::{SerializeOpts, TraversalScope, serialize};
use html5ever::tendril::StrTendril;
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom, SerializableHandle};

use crate::pretty_text::cleanup::{attr, element_name, parse_document, set_attr, text};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

// ---- selectors ----

#[derive(Debug, Clone)]
enum AttrOp {
    Exists,
    Equals(String),
    EndsWith(String),
}

#[derive(Debug, Clone, Default)]
struct Compound {
    tag: Option<String>,
    classes: Vec<String>,
    attrs: Vec<(String, AttrOp)>,
    not: Vec<Compound>,
    first_child: bool,
    last_child: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Combinator {
    Descendant,
    Child,
}

/// A complex selector, rightmost compound first, each with the combinator
/// to its left.
type Complex = Vec<(Compound, Option<Combinator>)>;

fn parse_compound(s: &str) -> (Compound, &str) {
    let mut c = Compound::default();
    let mut rest = s;
    let ident_end = |r: &str| {
        r.find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
            .unwrap_or(r.len())
    };
    if let Some(r) = rest.strip_prefix('*') {
        rest = r;
    } else {
        let end = ident_end(rest);
        if end > 0 {
            c.tag = Some(rest[..end].to_ascii_lowercase());
            rest = &rest[end..];
        }
    }
    loop {
        if let Some(r) = rest.strip_prefix('.') {
            let end = ident_end(r);
            c.classes.push(r[..end].to_string());
            rest = &r[end..];
        } else if let Some(r) = rest.strip_prefix('[') {
            let close = r.find(']').expect("selectors are written closed");
            let inner = &r[..close];
            let unquote = |v: &str| v.trim_matches(|ch| ch == '"' || ch == '\'').to_string();
            let entry = if let Some((name, value)) = inner.split_once("$=") {
                (name.to_string(), AttrOp::EndsWith(unquote(value)))
            } else if let Some((name, value)) = inner.split_once('=') {
                (name.to_string(), AttrOp::Equals(unquote(value)))
            } else {
                (inner.to_string(), AttrOp::Exists)
            };
            c.attrs.push(entry);
            rest = &r[close + 1..];
        } else if let Some(r) = rest.strip_prefix(":not(") {
            let close = r.find(')').expect("selectors are written closed");
            let (inner, _) = parse_compound(&r[..close]);
            c.not.push(inner);
            rest = &r[close + 1..];
        } else if let Some(r) = rest.strip_prefix(":first-child") {
            c.first_child = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix(":last-child") {
            c.last_child = true;
            rest = r;
        } else {
            break;
        }
    }
    (c, rest)
}

fn parse_complex(s: &str) -> Complex {
    let mut parts: Vec<(Compound, Option<Combinator>)> = Vec::new();
    let mut rest = s.trim();
    let mut pending: Option<Combinator> = None;
    while !rest.is_empty() {
        let (c, r) = parse_compound(rest);
        assert!(r.len() < rest.len(), "unsupported selector: {s}");
        parts.push((c, pending.take()));
        let trimmed = r.trim_start();
        if let Some(r) = trimmed.strip_prefix('>') {
            pending = Some(Combinator::Child);
            rest = r.trim_start();
        } else if trimmed.len() < r.len() || !trimmed.is_empty() {
            pending = Some(Combinator::Descendant);
            rest = trimmed;
        } else {
            rest = trimmed;
        }
    }
    // Rightmost first, each carrying the combinator that joins it to the
    // compound on its left.
    let mut out = Vec::new();
    for i in (0..parts.len()).rev() {
        let combinator = parts[i].1;
        out.push((parts[i].0.clone(), combinator));
    }
    out
}

fn parse_selector(s: &str) -> Vec<Complex> {
    s.split(',').map(parse_complex).collect()
}

fn element_children(node: &Handle) -> Vec<Handle> {
    node.children
        .borrow()
        .iter()
        .filter(|c| matches!(c.data, NodeData::Element { .. }))
        .cloned()
        .collect()
}

fn parent(node: &Handle) -> Option<Handle> {
    let weak = node.parent.take();
    let parent = weak.as_ref().and_then(|w| w.upgrade());
    node.parent.set(weak);
    parent
}

fn matches_compound(node: &Handle, c: &Compound) -> bool {
    let Some(name) = element_name(node) else {
        return false;
    };
    if c.tag.as_deref().is_some_and(|t| t != name) {
        return false;
    }
    if !c.classes.is_empty() {
        let class = attr(node, "class").unwrap_or_default();
        let tokens: Vec<&str> = class.split_whitespace().collect();
        if !c.classes.iter().all(|k| tokens.contains(&k.as_str())) {
            return false;
        }
    }
    for (name, op) in &c.attrs {
        let Some(value) = attr(node, name) else {
            return false;
        };
        let ok = match op {
            AttrOp::Exists => true,
            AttrOp::Equals(v) => &value == v,
            AttrOp::EndsWith(v) => !v.is_empty() && value.ends_with(v.as_str()),
        };
        if !ok {
            return false;
        }
    }
    if c.not.iter().any(|n| matches_compound(node, n)) {
        return false;
    }
    if c.first_child || c.last_child {
        let Some(p) = parent(node) else {
            return false;
        };
        let siblings = element_children(&p);
        if c.first_child && !siblings.first().is_some_and(|s| Rc::ptr_eq(s, node)) {
            return false;
        }
        if c.last_child && !siblings.last().is_some_and(|s| Rc::ptr_eq(s, node)) {
            return false;
        }
    }
    true
}

fn matches_complex(node: &Handle, parts: &[(Compound, Option<Combinator>)]) -> bool {
    let Some(((compound, combinator), rest)) = parts.split_first() else {
        return true;
    };
    if !matches_compound(node, compound) {
        return false;
    }
    match combinator {
        None => rest.is_empty(),
        Some(Combinator::Child) => parent(node).is_some_and(|p| matches_complex(&p, rest)),
        Some(Combinator::Descendant) => {
            let mut ancestor = parent(node);
            while let Some(a) = ancestor {
                if matches_complex(&a, rest) {
                    return true;
                }
                ancestor = parent(&a);
            }
            false
        }
    }
}

fn all_elements(node: &Handle, out: &mut Vec<Handle>) {
    for child in node.children.borrow().iter() {
        if matches!(child.data, NodeData::Element { .. }) {
            out.push(child.clone());
        }
        all_elements(child, out);
    }
}

// ---- the document ----

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
        let selectors = parse_selector(selector);
        let mut all = Vec::new();
        all_elements(&self.dom.document, &mut all);
        all.into_iter()
            .filter(|n| selectors.iter().any(|s| matches_complex(n, s)))
            .collect()
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
                set_attr(&node, k, v);
            }
        }
    }

    /// What this port refuses rather than restyles: the onebox and quote
    /// rewrites (`aside` to `blockquote`, iframes to links, user oneboxes).
    fn refuse_unported(&self) -> Result<(), Unsupported> {
        if !self
            .select("aside, article, header, iframe, .user-onebox")
            .is_empty()
        {
            return Err(Unsupported("quotes, oneboxes and iframes in emails"));
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
            if let Some(href) = attr(&a, "href") {
                if href.starts_with('/') && !href.starts_with("//") {
                    set_attr(&a, "href", &format!("{base_url}{href}"));
                }
            }
            if let Some(href) = attr(&a, "href").filter(|h| h.starts_with("//")) {
                set_attr(&a, "href", &format!("{scheme}:{href}"));
            }
        }
        Ok(())
    }

    /// `format_html`
    pub fn format_html(&self, settings: &SiteSettings) -> Result<(), AppError> {
        if settings.get("email_custom_css")?.presence().is_some()
            || settings
                .get("email_custom_css_compiled")?
                .presence()
                .is_some()
        {
            return Err(Unsupported("email_custom_css").into());
        }
        let link_color = settings.get("email_link_color")?.to_s();
        let accent_bg = settings.get("email_accent_bg_color")?.to_s();
        let accent_fg = settings.get("email_accent_fg_color")?.to_s();
        let locale = settings.get("default_locale")?.to_s();
        if matches!(locale.as_str(), "ar" | "fa_IR" | "he" | "ug" | "ur") {
            return Err(Unsupported("right-to-left email styles").into());
        }

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
        if let Some(footer) = self.select(".footer.highlight").first() {
            if let Some(a) = self.select_within(footer, "a").first() {
                set_attr(
                    a,
                    "style",
                    &format!(
                        "background-color: {accent_bg}; color: {accent_fg}; border-top: 4px solid {accent_bg}; border-right: 6px solid {accent_bg}; border-bottom: 4px solid {accent_bg}; border-left: 6px solid {accent_bg}; display: inline-block; font-weight: bold;"
                    ),
                );
            }
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
        self.style("body", Some("line-height: 1.4; text-align:left;"), &[]);
        self.style("body", None, &[("dir", "ltr")]);
        self.style(".with-dir", Some("text-align:left;"), &[("dir", "ltr")]);
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
        Ok(())
    }

    /// The style-only part of `onebox_styles` (the rewrites are refused).
    fn onebox_styles(&self) {
        self.style("blockquote", Some("border-left: 5px solid #e9e9e9; background-color: #f8f8f8; margin-left: 0; padding: 12px;"), &[]);
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
            ".onebox-avatar-inline",
            Some("width: 20px; height: 20px; float: none; vertical-align: middle;"),
            &[],
        );
    }

    fn select_within(&self, root: &Handle, selector: &str) -> Vec<Handle> {
        let selectors = parse_selector(selector);
        let mut all = Vec::new();
        all_elements(root, &mut all);
        all.into_iter()
            .filter(|n| selectors.iter().any(|s| matches_complex(n, s)))
            .collect()
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
                if let Some(p) = parent(&img) {
                    if element_name(&p) == Some("td") {
                        set_attr(&p, "style", "vertical-align: top;");
                    }
                }
                remove(&img);
                continue;
            }
            if let Some(title) = attr(&img, "title") {
                if src.contains("/emoji/") || src.contains("/_emoji/") {
                    if let Some(p) = parent(&img) {
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
