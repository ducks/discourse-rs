//! Port of `PrettyText.cleanup` (lib/pretty_text.rb), the step of
//! `PrettyText.cook` after the markdown: the HTML parsed as an HTML5
//! fragment, links given their `rel`, hidden direction marks in code made
//! visible, video placeholders given their thumbnail, mentions of users
//! and groups turned into links, scripts removed, and the fragment
//! serialized again (twice, as Nokogiri then Loofah do).
//!
//! The document is reference counted and so cannot be held across a
//! query: what needs the database (mentions, thumbnails) is collected
//! from one parse, looked up, and applied to a second.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use html5ever::serialize::{SerializeOpts, TraversalScope, serialize};
use html5ever::tendril::TendrilSink;
use html5ever::{
    Attribute, LocalName, ParseOpts, QualName, local_name, namespace_url, ns, parse_fragment,
};
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom, SerializableHandle};
use sqlx::PgConnection;

use super::CookError;
use crate::Unsupported;
use crate::config::Config;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::url::Urls;

/// `DANGEROUS_BIDI_CHARACTERS`
const BIDI: [char; 9] = [
    '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}',
    '\u{2069}',
];

pub(crate) fn parse(html: &str) -> RcDom {
    let context = QualName::new(None, ns!(html), local_name!("body"));
    parse_fragment(RcDom::default(), ParseOpts::default(), context, vec![])
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .expect("reading from a byte slice cannot fail")
}

/// `Nokogiri::HTML5(html)`: a whole document.
pub(crate) fn parse_document(html: &str) -> RcDom {
    html5ever::parse_document(RcDom::default(), ParseOpts::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .expect("reading from a byte slice cannot fail")
}

/// The fragment's nodes: the children of the `html` element the parser
/// puts them in.
/// The text of a whole fragment (`fragment.text`).
pub(super) fn dom_text(dom: &RcDom) -> String {
    text(&fragment_root(dom))
}

pub(crate) fn fragment_root(dom: &RcDom) -> Handle {
    dom.document
        .children
        .borrow()
        .first()
        .cloned()
        .unwrap_or_else(|| dom.document.clone())
}

pub(crate) fn to_html(dom: &RcDom) -> String {
    let root = fragment_root(dom);
    let mut out = Vec::new();
    for child in root.children.borrow().iter() {
        let handle: SerializableHandle = child.clone().into();
        serialize(
            &mut out,
            &handle,
            SerializeOpts {
                traversal_scope: TraversalScope::IncludeNode,
                ..Default::default()
            },
        )
        .expect("writing to a vector cannot fail");
    }
    String::from_utf8(out).expect("the serializer writes UTF-8")
}

pub(crate) fn element_name(node: &Handle) -> Option<&str> {
    match &node.data {
        NodeData::Element { name, .. } => Some(&name.local),
        _ => None,
    }
}

pub(crate) fn attr(node: &Handle, name: &str) -> Option<String> {
    match &node.data {
        NodeData::Element { attrs, .. } => attrs
            .borrow()
            .iter()
            .find(|a| &*a.name.local == name)
            .map(|a| a.value.to_string()),
        _ => None,
    }
}

/// Nokogiri's `node[name] = value`: replaced in place, else appended.
pub(super) fn set_attr(node: &Handle, name: &str, value: &str) {
    if let NodeData::Element { attrs, .. } = &node.data {
        let mut attrs = attrs.borrow_mut();
        match attrs.iter_mut().find(|a| &*a.name.local == name) {
            Some(existing) => existing.value = value.into(),
            None => attrs.push(Attribute {
                name: QualName::new(None, ns!(), LocalName::from(name)),
                value: value.into(),
            }),
        }
    }
}

pub(crate) fn has_class(node: &Handle, class: &str) -> bool {
    attr(node, "class").is_some_and(|c| c.split_whitespace().any(|c| c == class))
}

/// Every element in document order.
fn elements(root: &Handle, out: &mut Vec<Handle>) {
    for child in root.children.borrow().iter() {
        if matches!(child.data, NodeData::Element { .. }) {
            out.push(child.clone());
        }
        elements(child, out);
    }
}

pub(super) fn all_elements(dom: &RcDom) -> Vec<Handle> {
    let mut out = Vec::new();
    elements(&fragment_root(dom), &mut out);
    out
}

/// `node.text`: the text of every descendant.
pub(crate) fn text(node: &Handle) -> String {
    let mut out = String::new();
    fn walk(node: &Handle, out: &mut String) {
        if let NodeData::Text { contents } = &node.data {
            out.push_str(&contents.borrow());
        }
        for child in node.children.borrow().iter() {
            walk(child, out);
        }
    }
    walk(node, &mut out);
    out
}

fn new_text(content: &str) -> Handle {
    Node::new(NodeData::Text {
        contents: RefCell::new(content.into()),
    })
}

fn new_element(tag: &str, attrs: &[(&str, &str)]) -> Handle {
    Node::new(NodeData::Element {
        name: QualName::new(None, ns!(html), LocalName::from(tag)),
        attrs: RefCell::new(
            attrs
                .iter()
                .map(|(k, v)| Attribute {
                    name: QualName::new(None, ns!(), LocalName::from(*k)),
                    value: (*v).into(),
                })
                .collect(),
        ),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    })
}

fn append(parent: &Handle, child: Handle) {
    child.parent.set(Some(Rc::downgrade(parent)));
    parent.children.borrow_mut().push(child);
}

/// Puts `new` where `old` is in its parent.
fn replace(old: &Handle, new: Handle) {
    let Some(parent) = old.parent.take().and_then(|p| p.upgrade()) else {
        return;
    };
    new.parent.set(Some(Rc::downgrade(&parent)));
    let mut children = parent.children.borrow_mut();
    if let Some(i) = children.iter().position(|c| Rc::ptr_eq(c, old)) {
        children[i] = new;
    }
}

fn remove(node: &Handle) {
    let Some(parent) = node.parent.take().and_then(|p| p.upgrade()) else {
        return;
    };
    parent
        .children
        .borrow_mut()
        .retain(|c| !Rc::ptr_eq(c, node));
}

/// `Addressable::URI.encode_component` with its default character class:
/// everything but RFC 3986's reserved and unreserved characters is
/// percent-encoded, `%` included.
pub(super) fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let keep = c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=".contains(c);
        if keep {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// The host Ruby's `URI()` finds in an encoded href: None without an
/// authority, Err where the parser raises.
pub(super) fn uri_host(href: &str) -> Result<Option<String>, ()> {
    // A second `#`, or brackets outside an IPv6 host, are not RFC 3986.
    if href.matches('#').count() > 1 {
        return Err(());
    }
    let after_scheme = {
        let scheme_end = href.find(':').filter(|&i| {
            let scheme = &href[..i];
            scheme
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                && !href[..i].contains(['/', '?', '#'])
        });
        match scheme_end {
            Some(i) => &href[i + 1..],
            None => href,
        }
    };
    let Some(rest) = after_scheme.strip_prefix("//") else {
        if href.contains(['[', ']']) {
            return Err(());
        }
        return Ok(None);
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if rest[authority_end..].contains(['[', ']']) {
        return Err(());
    }
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(v6) = host_port.strip_prefix('[') {
        format!("[{}]", v6.split(']').next().ok_or(())?)
    } else {
        if host_port.contains(['[', ']']) {
            return Err(());
        }
        let (host, port) = host_port.split_once(':').unwrap_or((host_port, ""));
        if !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err(());
        }
        host.to_string()
    };
    Ok((!host.is_empty()).then_some(host))
}

/// `add_rel_attributes_to_user_content`
pub(super) fn add_rel_attributes(
    dom: &RcDom,
    add_nofollow: bool,
    site_host: &str,
    allowlist: &[String],
) {
    for link in all_elements(dom) {
        if element_name(&link) != Some("a") {
            continue;
        }
        let href = attr(&link, "href").unwrap_or_default();
        if attr(&link, "target").as_deref() == Some("_blank") {
            set_attr(&link, "rel", "noopener");
        }
        match uri_host(&encode_component(&href)) {
            Ok(host) => {
                let same_domain = host.as_deref().is_none_or(|host| {
                    host == site_host
                        || host.ends_with(&format!(".{site_host}"))
                        || allowlist
                            .iter()
                            .any(|u| host == u || host.ends_with(&format!(".{u}")))
                });
                if add_nofollow && !same_domain {
                    set_attr(&link, "rel", "noopener nofollow ugc");
                }
            }
            // A nofollow anyway.
            Err(()) => set_attr(&link, "rel", "noopener nofollow ugc"),
        }
    }
}

/// `strip_hidden_unicode_bidirectional_characters`: each direction mark
/// in code shown as a warning span.
fn mark_bidi_characters(dom: &RcDom, title: &str) {
    if !text(&fragment_root(dom)).contains(BIDI) {
        return;
    }
    for code in all_elements(dom) {
        if !matches!(element_name(&code), Some("code") | Some("pre")) {
            continue;
        }
        let mut texts = Vec::new();
        fn collect(node: &Handle, out: &mut Vec<Handle>) {
            for child in node.children.borrow().iter() {
                if matches!(child.data, NodeData::Text { .. }) {
                    out.push(child.clone());
                } else {
                    collect(child, out);
                }
            }
        }
        collect(&code, &mut texts);
        for text_node in texts {
            let NodeData::Text { contents } = &text_node.data else {
                continue;
            };
            let content = contents.borrow().to_string();
            if !content.contains(BIDI) {
                continue;
            }
            let Some(parent) = text_node.parent.take().and_then(|p| p.upgrade()) else {
                continue;
            };
            let mut pieces: Vec<Handle> = Vec::new();
            let mut run = String::new();
            for c in content.chars() {
                if BIDI.contains(&c) {
                    if !run.is_empty() {
                        pieces.push(new_text(&std::mem::take(&mut run)));
                    }
                    let span = new_element("span", &[("class", "bidi-warning"), ("title", title)]);
                    append(&span, new_text(&format!("<U+{:X}>", c as u32)));
                    pieces.push(span);
                } else {
                    run.push(c);
                }
            }
            if !run.is_empty() {
                pieces.push(new_text(&run));
            }
            let mut children = parent.children.borrow_mut();
            if let Some(i) = children.iter().position(|c| Rc::ptr_eq(c, &text_node)) {
                for piece in &pieces {
                    piece.parent.set(Some(Rc::downgrade(&parent)));
                }
                children.splice(i..=i, pieces);
            }
        }
    }
}

/// What a mention resolves to (`lookup_mentions`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum MentionType {
    User,
    Group,
    GroupMentionable,
}

/// `.video-placeholder-container` sources that point at an upload.
fn video_sources(dom: &RcDom) -> Vec<String> {
    all_elements(dom)
        .into_iter()
        .filter(|e| has_class(e, "video-placeholder-container"))
        .filter_map(|e| attr(&e, "data-video-src"))
        .filter(|src| src != "/404")
        .collect()
}

/// `File.basename(src, File.extname(src))` and `File.extname(src)`.
fn basename_and_ext(src: &str) -> (&str, &str) {
    let base = src.rsplit('/').next().unwrap_or(src);
    match base.rfind('.').filter(|&i| i > 0 && i + 1 < base.len()) {
        Some(i) => (&base[..i], &base[i..]),
        None => (base, ""),
    }
}

/// Whether the cooked HTML has anything for the mention step: its
/// `<span class="mention">` texts without the `@`.
fn mention_names(dom: &RcDom) -> Vec<String> {
    all_elements(dom)
        .into_iter()
        .filter(|e| element_name(e) == Some("span") && has_class(e, "mention"))
        .map(|e| text(&e).chars().skip(1).collect())
        .collect()
}

/// `lookup_mentions`: users by name, every group, mentionable groups by
/// name; later types win, in the order `group`, `group-mentionable`,
/// `user`.
async fn lookup_mentions(
    conn: &mut PgConnection,
    names: &[String],
    user_id: Option<i64>,
) -> Result<HashMap<String, MentionType>, CookError> {
    let mut mentions = HashMap::new();
    if names.is_empty() {
        return Ok(mentions);
    }
    let names: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    let (admin, moderator): (bool, bool) = match user_id {
        Some(id) => sqlx::query_as("SELECT admin, moderator FROM users WHERE id = $1")
            .bind(i32::try_from(id).unwrap_or(0))
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or((false, false)),
        None => (false, false),
    };
    // Group.alias_levels
    let levels: Vec<i32> = if admin {
        vec![99, 1, 2, 3, 4]
    } else if moderator {
        vec![99, 2, 3, 4]
    } else {
        vec![99]
    };
    let rows: Vec<(String, String)> = sqlx::query_as(
        "(SELECT 'user' AS type, username_lower AS name FROM users \
           WHERE username_lower = ANY($1) AND staged = false) \
         UNION \
         (SELECT 'group' AS type, lower(name) AS name FROM groups) \
         UNION \
         (SELECT 'group-mentionable' AS type, lower(name) AS name FROM groups \
           WHERE lower(name) = ANY($1) AND ( \
             groups.mentionable_level = ANY($2) \
             OR (groups.mentionable_level = 3 \
                 AND groups.id IN (SELECT group_id FROM group_users WHERE user_id = $3)) \
             OR (groups.mentionable_level = 4 \
                 AND groups.id IN (SELECT group_id FROM group_users WHERE user_id = $3 AND owner IS TRUE)))) \
         ORDER BY type",
    )
    .bind(&names)
    .bind(&levels)
    .bind(user_id.and_then(|id| i32::try_from(id).ok()))
    .fetch_all(&mut *conn)
    .await?;
    for (kind, name) in rows {
        let kind = match kind.as_str() {
            "user" => MentionType::User,
            "group" => MentionType::Group,
            _ => MentionType::GroupMentionable,
        };
        mentions.insert(name, kind);
    }
    Ok(mentions)
}

/// `add_mentions`: a mention that names a user or a group becomes a link.
fn add_mentions(dom: &RcDom, mentions: &HashMap<String, MentionType>, base_path: &str) {
    for span in all_elements(dom) {
        if element_name(&span) != Some("span") || !has_class(&span, "mention") {
            continue;
        }
        let shown = text(&span);
        let name: String = shown.chars().skip(1).collect::<String>().to_lowercase();
        let Some(kind) = mentions.get(&name) else {
            continue;
        };
        let NodeData::Element { attrs, .. } = &span.data else {
            continue;
        };
        let link = Node::new(NodeData::Element {
            name: QualName::new(None, ns!(html), local_name!("a")),
            attrs: RefCell::new(attrs.borrow().clone()),
            template_contents: RefCell::new(None),
            mathml_annotation_xml_integration_point: false,
        });
        // format_username is the identity.
        append(&link, new_text(&shown));
        let encoded = encode_component(&name);
        match kind {
            MentionType::User => {
                set_attr(&link, "href", &format!("{base_path}/u/{encoded}"));
            }
            MentionType::GroupMentionable => {
                set_attr(&link, "class", "mention-group notify");
                set_attr(&link, "href", &format!("{base_path}/groups/{encoded}"));
            }
            MentionType::Group => {
                set_attr(&link, "class", "mention-group");
                set_attr(&link, "href", &format!("{base_path}/groups/{encoded}"));
            }
        }
        replace(&span, link);
    }
}

/// `add_rel_attributes_to_user_content` with the site's settings: what
/// cleanup does to links, and the post processor's `enforce_nofollow`.
pub(super) fn rel_settings(
    settings: &SiteSettings,
    config: &Config,
) -> Result<(String, Vec<String>), CookError> {
    let urls = Urls { config, settings };
    let allowlist: Vec<String> = settings
        .get("exclude_rel_nofollow_domains")?
        .to_s()
        .split('|')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let site_host = uri_host(&urls.base_url()?)
        .ok()
        .flatten()
        .unwrap_or_default();
    Ok((site_host, allowlist))
}
/// `PrettyText.cleanup(html, user_id:, omit_nofollow:)`
pub async fn cleanup(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    config: &Config,
    i18n: &I18n,
    html: &str,
    user_id: Option<i64>,
    omit_nofollow: bool,
) -> Result<String, CookError> {
    if settings.get("block_hotlinked_media")?.truthy() {
        return Err(Unsupported("block_hotlinked_media in cooking").into());
    }
    let mentions_enabled = settings.get("enable_mentions")?.truthy();

    // What needs the database, from a first parse.
    let (names, videos) = {
        let dom = parse(html);
        let names = if mentions_enabled {
            mention_names(&dom)
        } else {
            Vec::new()
        };
        (names, video_sources(&dom))
    };
    let mentions = lookup_mentions(conn, &names, user_id).await?;
    let urls = Urls { config, settings };
    let mut thumbnails: HashMap<String, (String, String)> = HashMap::new();
    for src in &videos {
        let (sha1, ext) = basename_and_ext(src);
        let pattern = format!(
            "{}.%",
            sha1.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let url: Option<String> = sqlx::query_scalar(
            "SELECT url FROM uploads WHERE original_filename LIKE $1 ORDER BY id DESC LIMIT 1",
        )
        .bind(pattern)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(url) = url {
            if config.globals.cdn_url().is_some() || config.globals.s3_cdn_url().is_some() {
                return Err(Unsupported("video thumbnails behind a CDN").into());
            }
            let base62 = super::helpers::base62_sha1(sha1)
                .ok_or(Unsupported("a video upload whose name is not a sha1"))?;
            thumbnails.insert(
                src.clone(),
                (urls.absolute(&url)?, format!("{base62}{ext}")),
            );
        }
    }

    let add_nofollow = !omit_nofollow && settings.get("add_rel_nofollow_to_user_content")?.truthy();
    let (site_host, allowlist) = rel_settings(settings, config)?;
    let bidi_title = i18n
        .t("post.hidden_bidi_character")
        .unwrap_or(
            "Bidirectional control characters can change the order in which text is rendered.",
        )
        .to_string();

    let dom = parse(html);
    add_rel_attributes(&dom, add_nofollow, &site_host, &allowlist);
    mark_bidi_characters(&dom, &bidi_title);
    for video in all_elements(&dom) {
        if !has_class(&video, "video-placeholder-container") {
            continue;
        }
        if let Some((thumbnail, base62)) =
            attr(&video, "data-video-src").and_then(|s| thumbnails.get(&s))
        {
            set_attr(&video, "data-thumbnail-src", thumbnail);
            set_attr(&video, "data-video-base62-sha1", base62);
        }
    }
    if mentions_enabled {
        add_mentions(&dom, &mentions, config.globals.relative_url_root());
    }
    let first = to_html(&dom);

    // Loofah's pass: scripts out, serialized once more.
    let dom = parse(&first);
    for script in all_elements(&dom) {
        if element_name(&script) == Some("script") {
            remove(&script);
        }
    }
    Ok(to_html(&dom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_as_ruby_uri_finds_them() {
        assert_eq!(
            uri_host("https://example.com/x"),
            Ok(Some("example.com".into()))
        );
        assert_eq!(
            uri_host("http://user@host:3000/p?q#f"),
            Ok(Some("host".into()))
        );
        assert_eq!(
            uri_host("//cdn.example.com/a"),
            Ok(Some("cdn.example.com".into()))
        );
        assert_eq!(uri_host("/latest"), Ok(None));
        assert_eq!(uri_host("mailto:a@example.com"), Ok(None));
        assert_eq!(uri_host("#a#b"), Err(()));
    }

    #[test]
    fn encodes_like_addressable() {
        assert_eq!(encode_component("a b%20é"), "a%20b%2520%C3%A9");
        assert_eq!(
            encode_component("https://x.com/?a=1&b#c"),
            "https://x.com/?a=1&b#c"
        );
    }

    #[test]
    fn reserializes_as_html5() {
        let dom = parse("<p>a &quot;b&quot; <img alt src=x></p>");
        assert_eq!(to_html(&dom), "<p>a \"b\" <img alt=\"\" src=\"x\"></p>");
    }
}
