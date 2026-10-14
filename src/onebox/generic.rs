//! AllowlistedGenericOnebox (lib/onebox/engine/allowlisted_generic_onebox.rb)
//! over StandardEmbed (standard_embed.rb): another site's page as its
//! OpenGraph, Twitter, oEmbed and JSON-LD (Movie) data, its favicon and
//! description and the section its fragment names, rendered as an
//! article, an image, a video, an embed or a player card.

use std::sync::LazyLock;

use markup5ever_rcdom::Handle;
use serde_json::{Map, Value, json};

use super::engines::{Ctx, Fail, Rendered, https_host, layout, link};
use super::fetch::Fetch;
use super::html_doc::{parse, root};
use super::normalize_url_for_output;
use super::ruby_time;
use super::uri::{RubyUri, absolute_url, cgi_unescape};
use crate::Unsupported;
use crate::pretty_text::cleanup::{attr, element_name, text};
use crate::pretty_text::select::{
    ancestors, element_children, is_element, parent, previous_elements, select, select_first,
};

const TEMPLATE: &str =
    include_str!("../../vendor/discourse/lib/onebox/templates/allowlistedgeneric.mustache");

/// `UrlHelper::MAX_URL_LENGTH`
const MAX_URL_LENGTH: usize = 2000;

// ---- Ruby string helpers ----

/// `blank?` for a JSON value: nil, false, an empty or whitespace string,
/// an empty list or hash.
pub(super) fn blank(v: &Value) -> bool {
    match v {
        Value::Null | Value::Bool(false) => true,
        Value::String(s) => crate::ruby::is_blank(s),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

pub(super) fn present(v: Option<&Value>) -> bool {
    v.is_some_and(|v| !blank(v))
}

pub(super) fn str_of(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

/// `Onebox::Helpers.truncate(string, length)`
pub fn helpers_truncate(s: &str, length: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= length {
        return s.to_string();
    }
    // rindex(" ", length): the last space at or before `length`.
    let upto = length.min(chars.len() - 1);
    let cut = (0..=upto)
        .rev()
        .find(|i| chars[*i] == ' ')
        .unwrap_or(length);
    let mut out: String = chars[..cut].iter().collect();
    out.push_str("...");
    out
}

/// ActiveSupport's `String#truncate(length, separator: " ", omission:)`
pub fn active_support_truncate(s: &str, length: usize, omission: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= length {
        return s.to_string();
    }
    let room = length.saturating_sub(omission.chars().count());
    let upto = room.min(chars.len() - 1);
    let stop = (0..=upto).rev().find(|i| chars[*i] == ' ').unwrap_or(room);
    let mut out: String = chars[..stop.min(chars.len())].iter().collect();
    out.push_str(omission);
    out
}

/// `value.to_s` for a parsed JSON value, where Ruby's matches.
pub(super) fn ruby_to_s(v: &Value) -> Result<String, Unsupported> {
    Ok(match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                n.to_string()
            } else {
                let f = n.as_f64().unwrap_or(0.0);
                let s = n.to_string();
                let digits = s.trim_start_matches('-').replace('.', "");
                if f != 0.0 && !(1e-4..1e16).contains(&f.abs()) || digits.len() > 15 {
                    return Err(Unsupported("oEmbed numbers Ruby prints differently"));
                }
                if s.contains('.') { s } else { format!("{s}.0") }
            }
        }
        Value::Array(_) | Value::Object(_) => {
            return Err(Unsupported("oEmbed values that are lists or hashes"));
        }
    })
}

pub(super) fn decode(s: &str) -> Result<String, Unsupported> {
    super::entities::decode(s)
}

/// `Sanitize.fragment(value)` with the default config: the text, escaped.
pub(super) fn sanitize_default(s: &str) -> Result<String, Unsupported> {
    crate::sanitize::fragment(s, &crate::sanitize::config::Config::default_config())
        .map_err(|_| Unsupported("onebox values past Gumbo's limits"))
}

// ---- Onebox::Normalizer and its kinds ----

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    OpenGraph,
    Oembed,
    JsonLd,
}

/// What `public_send(key)` would call besides `method_missing`: Object's
/// and the normalizer's own methods, which this port doesn't call.
const OBJECT_METHODS: &[&str] = &[
    "hash",
    "class",
    "display",
    "data",
    "get",
    "method",
    "methods",
    "send",
    "freeze",
    "frozen?",
    "dup",
    "clone",
    "tap",
    "then",
    "inspect",
    "to_s",
    "object_id",
    "itself",
    "presence",
    "present?",
    "blank?",
    "try",
    "title_attr",
    "secure_image_url",
    "to_json",
    "as_json",
    "to_param",
    "to_query",
    "instance_values",
    "instance_variables",
    "extend",
    "singleton_class",
];

pub(super) struct Normalizer {
    pub(super) kind: Kind,
    pub(super) data: Map<String, Value>,
}

impl Normalizer {
    /// `sanitize_value(value, length, sanitize)`
    fn sanitize_value(
        v: &Value,
        length: Option<usize>,
        sanitize: bool,
    ) -> Result<String, Unsupported> {
        let mut value = decode(&ruby_to_s(v)?)?;
        if sanitize {
            value = decode(&sanitize_default(&value)?)?;
        }
        let mut value = crate::ruby::strip(&value).to_string();
        if let Some(l) = length {
            value = helpers_truncate(&value, l);
        }
        Ok(value)
    }

    /// `get(attr, length, sanitize)`
    pub(super) fn get(
        &self,
        key: &str,
        length: Option<usize>,
        sanitize: bool,
    ) -> Result<Value, Unsupported> {
        let Some(value) = self.data.get(key).filter(|v| !blank(v)) else {
            return Ok(Value::Null);
        };
        if let Value::Array(items) = value {
            return Ok(Value::Array(
                items
                    .iter()
                    .map(|v| Self::sanitize_value(v, length, sanitize).map(Value::String))
                    .collect::<Result<_, _>>()?,
            ));
        }
        Ok(Value::String(Self::sanitize_value(
            value, length, sanitize,
        )?))
    }

    /// `normalizer.public_send(key)`; Err(Fail::Raised) where Ruby raises.
    pub(super) fn value(&self, key: &str) -> Result<Value, Fail> {
        if OBJECT_METHODS.contains(&key) || key.ends_with(['?', '!', '=']) {
            return Err(Unsupported("onebox data keys that name Ruby methods").into());
        }
        match (self.kind, key) {
            (Kind::OpenGraph | Kind::Oembed, "title") => {
                return Ok(self.get("title", Some(80), true)?);
            }
            (Kind::Oembed, "html") => return Ok(self.get("html", None, false)?),
            _ => {}
        }
        // method_missing
        let value = self.get(key, None, true)?;
        if blank(&value) {
            return Ok(Value::Null);
        }
        if key.ends_with("width") || key.ends_with("height") {
            return match value {
                Value::String(s) => Ok(json!(crate::ruby::to_i(&s))),
                _ => Err(Fail::Raised(
                    "NoMethodError: undefined method `to_i' for an Array".into(),
                )),
            };
        }
        if key.ends_with("url") || key.ends_with("image") || key.ends_with("video") {
            return match value {
                Value::String(s) => {
                    let url = normalize_url_for_output(&s);
                    Ok(if url.is_empty() {
                        Value::Null
                    } else {
                        Value::String(url)
                    })
                }
                _ => Err(Fail::Raised(
                    "NoMethodError: undefined method `gsub!' for an Array".into(),
                )),
            };
        }
        Ok(value)
    }
}

/// `OpenGraph#extract(doc)`
pub(super) fn open_graph(root: &Handle) -> Map<String, Value> {
    static KEY: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)\A(?:og|article|product):(.+)\z").unwrap());
    const COLLECTIONS: [&str; 3] = ["article_section", "article_section_color", "article_tag"];
    let mut data = Map::new();
    for m in select(root, "meta") {
        let key = attr(&m, "property")
            .and_then(|p| KEY.captures(&p).map(|c| c[1].to_string()))
            .or_else(|| attr(&m, "name").and_then(|n| KEY.captures(&n).map(|c| c[1].to_string())));
        let Some(key) = key else {
            continue;
        };
        let value = attr(&m, "content")
            .or_else(|| attr(&m, "value"))
            .unwrap_or_default();
        if crate::ruby::is_blank(&value) {
            continue;
        }
        let key = key.replace(['-', ':'], "_");
        data.entry(key.clone()).or_insert_with(|| json!(value));
        if COLLECTIONS.contains(&key.as_str()) {
            let list = data.entry(format!("{key}s")).or_insert_with(|| json!([]));
            if let Value::Array(a) = list {
                a.push(json!(value));
            }
        }
    }
    if let Some(title) = select_first(root, "title") {
        let t = text(&title);
        if !crate::ruby::is_blank(&t) {
            data.entry("title").or_insert_with(|| json!(t));
        }
    }
    data
}

/// `StandardEmbed#get_twitter`
pub(super) fn twitter(root: &Handle) -> Map<String, Value> {
    static KEY: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?im)^twitter:(.+)$").unwrap());
    let mut data = Map::new();
    for m in select(root, "meta") {
        let key = attr(&m, "property")
            .and_then(|p| KEY.captures(&p).map(|c| c[1].to_string()))
            .or_else(|| attr(&m, "name").and_then(|n| KEY.captures(&n).map(|c| c[1].to_string())));
        let Some(key) = key else {
            continue;
        };
        let value = attr(&m, "content")
            .or_else(|| attr(&m, "value"))
            .unwrap_or_default();
        if !crate::ruby::is_blank(&value) && value != "0 minutes" {
            data.entry(key.replace(['-', ':'], "_"))
                .or_insert_with(|| json!(value));
        }
    }
    data
}

/// `JsonLd#extract(doc)`: a Movie's data, else nothing.
pub(super) fn json_ld(root: &Handle) -> Result<Map<String, Value>, Fail> {
    for script in select(root, "script[type=\"application/ld+json\"]") {
        let Ok(parsed) = serde_json::from_str::<Value>(&text(&script)) else {
            continue;
        };
        let movie = match &parsed {
            Value::Array(items) => {
                let mut found = None;
                for item in items {
                    match item {
                        Value::Object(o) => {
                            if o.get("@type").and_then(Value::as_str) == Some("Movie") {
                                found = Some(o.clone());
                                break;
                            }
                        }
                        Value::String(_) => {}
                        _ => return Err(Unsupported("JSON-LD lists of other values").into()),
                    }
                }
                match found {
                    Some(m) => m,
                    None => return Ok(Map::new()),
                }
            }
            Value::Object(o) => {
                if o.get("@type").and_then(Value::as_str) != Some("Movie") {
                    continue;
                }
                o.clone()
            }
            Value::String(_) => continue,
            _ => return Err(Unsupported("JSON-LD that isn't an object or a list").into()),
        };
        return movie_data(&movie);
    }
    Ok(Map::new())
}

/// `Onebox::Movie#to_h`
fn movie_data(m: &Map<String, Value>) -> Result<Map<String, Value>, Fail> {
    let rating = match m.get("aggregateRating") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::Object(r)) => r.get("ratingValue").cloned().unwrap_or(Value::Null),
        Some(_) => return Err(Unsupported("JSON-LD ratings that aren't objects").into()),
    };
    let duration = match m.get("duration") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Value::Null,
        Some(Value::String(d)) => match ruby_time::duration(d)? {
            ruby_time::Parsed::Time(f) => json!(f.hours_minutes()),
            ruby_time::Parsed::Raised => {
                return Err(Fail::Raised(format!(
                    "ArgumentError: no time information in {d:?}"
                )));
            }
        },
        Some(_) => return Err(Unsupported("JSON-LD durations that aren't strings").into()),
    };
    if matches!(m.get("image"), Some(Value::Array(_) | Value::Object(_))) {
        return Err(Unsupported("JSON-LD images that are lists or objects").into());
    }
    let mut out = Map::new();
    out.insert("name".into(), m.get("name").cloned().unwrap_or(Value::Null));
    out.insert(
        "image".into(),
        m.get("image").cloned().unwrap_or(Value::Null),
    );
    out.insert(
        "description".into(),
        m.get("description").cloned().unwrap_or(Value::Null),
    );
    out.insert("rating".into(), rating);
    out.insert(
        "genres".into(),
        m.get("genre").cloned().unwrap_or(Value::Null),
    );
    out.insert("duration".into(), duration);
    Ok(out)
}

/// `StandardEmbed#get_favicon`
pub(super) fn favicon(root: &Handle, url: &str) -> Option<String> {
    let link = select_first(
        root,
        "link[rel=\"shortcut icon\"], link[rel=\"icon shortcut\"], link[rel=\"shortcut\"], link[rel=\"icon\"]",
    )?;
    let href = attr(&link, "href")?;
    let href = crate::ruby::strip(&href);
    if crate::ruby::is_blank(href) {
        return None;
    }
    let absolute = absolute_url(Some(href), url)?;
    (absolute.len() <= MAX_URL_LENGTH).then_some(absolute)
}

/// `StandardEmbed#get_description`
pub(super) fn description(root: &Handle) -> Option<String> {
    select_first(root, "meta[name='description']")
        .and_then(|m| attr(&m, "content"))
        .or_else(|| {
            select_first(root, "meta[name='Description']").and_then(|m| attr(&m, "content"))
        })
}

/// `node.text.strip`
fn text_strip(node: &Handle) -> String {
    crate::ruby::strip(&text(node)).to_string()
}

fn heading(node: &Handle) -> bool {
    element_name(node).is_some_and(|n| {
        n.len() == 2
            && n.starts_with(['h', 'H'])
            && ('1'..='6').contains(&n.chars().nth(1).unwrap())
    })
}

/// `find_anchor_target(fragment)`: the element with the id, else the
/// anchor with the name (XPath string literals don't decode `&quot;`).
fn anchor_target(root: &Handle, fragment: &str) -> Option<Handle> {
    let escaped = fragment.replace('"', "&quot;");
    let all = crate::pretty_text::select::all_descendants(root);
    all.iter()
        .find(|n| attr(n, "id").as_deref() == Some(escaped.as_str()))
        .or_else(|| {
            all.iter().find(|n| {
                element_name(n) == Some("a") && attr(n, "name").as_deref() == Some(escaped.as_str())
            })
        })
        .cloned()
}

/// `find_nearest_heading(element)`
fn nearest_heading(element: &Handle) -> Option<String> {
    let mut current = Some(element.clone());
    while let Some(c) = current.filter(is_element) {
        if let Some(prev) = previous_elements(&c).first()
            && heading(prev)
        {
            return Some(text_strip(prev));
        }
        current = parent(&c);
        if let Some(p) = &current
            && is_element(p)
            && heading(p)
        {
            return Some(text_strip(p));
        }
    }
    None
}

/// `find_section_title(fragment)`
fn section_title(root: &Handle, fragment: &str) -> Option<String> {
    let target = anchor_target(root, fragment)?;
    if heading(&target) {
        return Some(text_strip(&target));
    }
    if let Some(code) = select_first(&target, "code, .docstring-binding") {
        let c = text_strip(&code);
        if !crate::ruby::is_blank(&c) {
            return Some(c);
        }
    }
    if let Some(h) = select_first(&target, "h1, h2, h3, h4, h5, h6") {
        return Some(text_strip(&h));
    }
    nearest_heading(&target)
}

/// The `p` elements after the node in document order, outside it
/// (`following::p`).
fn following_p(node: &Handle) -> Option<Handle> {
    let mut current = node.clone();
    loop {
        let p = parent(&current)?;
        let siblings: Vec<Handle> = p.children.borrow().clone();
        let i = siblings
            .iter()
            .position(|s| std::rc::Rc::ptr_eq(s, &current))?;
        for s in &siblings[i + 1..] {
            if element_name(s) == Some("p") {
                return Some(s.clone());
            }
            if let Some(found) = select_first(s, "p") {
                return Some(found);
            }
        }
        current = p;
    }
}

/// `extract_description_from_target(target)`
fn section_description(root: &Handle, fragment: &str) -> Option<String> {
    let target = anchor_target(root, fragment)?;
    let context = ancestors(&target, "article, section, details, .docstring")
        .into_iter()
        .next()
        .or_else(|| parent(&target))
        .unwrap_or_else(|| target.clone());
    if let Some(p) = select_first(&context, "p:not(.admonition-header)") {
        let t = text_strip(&p);
        if !crate::ruby::is_blank(&t) {
            return Some(t);
        }
    }
    let next = element_children(&parent(&target)?)
        .into_iter()
        .skip_while(|s| !std::rc::Rc::ptr_eq(s, &target))
        .skip(1)
        .find(|s| element_name(s) == Some("p"))
        .or_else(|| following_p(&target))?;
    let t = text_strip(&next);
    (!crate::ruby::is_blank(&t)).then_some(t)
}

/// `gsub(/\s+/, " ").strip`
pub(super) fn squish(s: &str) -> String {
    static WS: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"[ \t\r\n\x0b\x0c]+").unwrap());
    crate::ruby::strip(&WS.replace_all(s, " ")).to_string()
}

/// What the page itself gives, read before the oEmbed fetch.
pub(super) struct Page {
    pub(super) og: Map<String, Value>,
    pub(super) twitter: Map<String, Value>,
    pub(super) oembed_links: (Option<String>, Option<String>),
    pub(super) json_ld: Result<Map<String, Value>, Fail>,
    pub(super) favicon: Option<String>,
    pub(super) description: Option<String>,
    pub(super) section_title: Option<String>,
    pub(super) section_description: Option<String>,
}

pub(super) fn read_page(html: Option<&str>, url: &str) -> Result<Page, Fail> {
    let dom = parse(html.unwrap_or(""));
    let root = root(&dom);
    let fragment = match RubyUri::parse(url).and_then(|u| u.fragment) {
        Some(f) if !crate::ruby::is_blank(&f) => {
            Some(cgi_unescape(&f).ok_or(Unsupported("URL fragments that aren't UTF-8 unescaped"))?)
        }
        _ => None,
    };
    let link = |t: &str| {
        select(&root, "link")
            .into_iter()
            .find(|l| attr(l, "type").as_deref() == Some(t) && attr(l, "href").is_some())
            .and_then(|l| attr(&l, "href"))
    };
    Ok(Page {
        og: open_graph(&root),
        twitter: twitter(&root),
        oembed_links: (link("application/json+oembed"), link("text/json+oembed")),
        json_ld: json_ld(&root),
        favicon: favicon(&root, url),
        description: description(&root),
        section_title: fragment
            .as_deref()
            .filter(|f| !crate::ruby::is_blank(f))
            .and_then(|f| section_title(&root, f)),
        section_description: fragment
            .as_deref()
            .filter(|f| !crate::ruby::is_blank(f))
            .and_then(|f| section_description(&root, f)),
    })
}

/// `StandardEmbed.oembed_providers`
pub(super) static OEMBED_PROVIDERS: LazyLock<Vec<(regex::Regex, &str)>> = LazyLock::new(|| {
    [
        (r"www\.meetup\.com/", "http://api.meetup.com/oembed"),
        (r"www\.mixcloud\.com/", "https://www.mixcloud.com/oembed/"),
        (r"vimeo\.com/", "https://vimeo.com/api/oembed.json"),
        (r"nytimes\.com/", "https://www.nytimes.com/svc/oembed/json/"),
        (r"youtube\.com|youtu\.be", "https://www.youtube.com/oembed"),
    ]
    .into_iter()
    .map(|(r, e)| (regex::Regex::new(r).unwrap(), e))
    .collect()
});

/// `set_from_normalizer_data(normalizer, skip_dimensions:)`
pub(super) fn set_from(
    raw: &mut Map<String, Value>,
    normalizer: &Normalizer,
    skip_dimensions: bool,
) -> Result<(), Fail> {
    for key in normalizer.data.keys() {
        if skip_dimensions && (key == "width" || key == "height") {
            continue;
        }
        let v = normalizer.value(key)?;
        if v.is_null() {
            continue;
        }
        if !raw.get(key).is_some_and(|e| !e.is_null()) {
            raw.insert(key.clone(), v);
        }
    }
    Ok(())
}

/// `Onebox::Oembed.new(response).data`
pub(super) fn oembed_data(response: &str) -> Result<Map<String, Value>, Fail> {
    let parsed: Value = serde_json::from_str(response)
        .map_err(|e| Fail::Raised(format!("MultiJson::ParseError: {e}")))?;
    let Value::Object(mut data) = parsed else {
        return Err(Fail::Raised(
            "NoMethodError: oEmbed that isn't an object".into(),
        ));
    };
    // Never use oembed from WordPress 4.4 (it's broken).
    match data.get("html") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => {}
        Some(Value::String(h)) => {
            if h.contains("wp-embedded-content") {
                data.remove("html");
            }
        }
        Some(_) => return Err(Unsupported("oEmbed html that isn't a string").into()),
    }
    Ok(data)
}

pub(super) fn host_matches(host: &str, list: &[&str]) -> bool {
    list.iter()
        .any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

/// `AllowlistedGenericOnebox.allowed_twitter_labels`
const TWITTER_LABELS: [&str; 6] = ["brand", "price", "usd", "cad", "reading time", "likes"];

fn allowed_label(label: &str) -> bool {
    let lower = label.to_lowercase();
    TWITTER_LABELS.iter().any(|l| lower.contains(l))
}

/// The engine's state once `data` has run.
struct Generic<'a> {
    host: String,
    data: Map<String, Value>,
    errors: Vec<String>,
    ctx: &'a Ctx<'a>,
    layout: Option<String>,
}

/// `AllowlistedGenericOnebox#data`
fn data(url: &str, raw: Map<String, Value>) -> Result<(Map<String, Value>, Vec<String>), Fail> {
    let mut d = Map::new();
    d.insert("link".into(), json!(link(url)?));
    for (k, v) in raw {
        d.insert(k, v);
    }
    let title = match d.get("title") {
        Some(Value::String(t)) if !crate::ruby::is_blank(t) => {
            json!(decode(&helpers_truncate(t, 80))?)
        }
        Some(v) if !blank(v) => return Err(Unsupported("onebox titles that are lists").into()),
        _ => Value::Null,
    };
    d.insert("title".into(), title);
    if !d.get("description").is_some_and(|v| !v.is_null()) {
        let summary = d.get("summary").cloned().unwrap_or(Value::Null);
        d.insert("description".into(), summary);
    }
    if present(d.get("description")) {
        let desc = d["description"]
            .as_str()
            .ok_or(Unsupported("onebox descriptions that are lists"))?
            .to_string();
        d.insert(
            "description".into(),
            json!(decode(&helpers_truncate(&desc, 250))?),
        );
    }
    if present(d.get("site_name")) {
        let s = d["site_name"]
            .as_str()
            .ok_or(Unsupported("onebox site names that are lists"))?
            .to_string();
        d.insert("domain".into(), json!(decode(&helpers_truncate(&s, 80))?));
    } else if present(d.get("domain")) {
        let mut domain = d["domain"]
            .as_str()
            .ok_or(Unsupported("onebox domains that are lists"))?
            .to_string();
        static SCHEME: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new(r"(?m)^https?://").unwrap());
        if !SCHEME.is_match(&domain) {
            domain = format!("http://{domain}");
        }
        let host = RubyUri::parse(&domain).map(|u| {
            let h = u.host.unwrap_or_default();
            h.strip_prefix("www.").map(str::to_string).unwrap_or(h)
        });
        d.insert("domain".into(), host.map_or(Value::Null, Value::String));
    }
    // prefer secure URLs
    let image = ["image_secure_url", "image_url", "thumbnail_url", "image"]
        .iter()
        .find_map(|k| d.get(*k).filter(|v| !v.is_null()).cloned())
        .unwrap_or(Value::Null);
    let image = match image {
        Value::Null => None,
        Value::String(s) => Some(s),
        _ => return Err(Unsupported("onebox images that are lists").into()),
    };
    let image = absolute_url(image.as_deref(), url);
    let image = normalize_url_for_output(&decode(image.as_deref().unwrap_or(""))?);
    d.insert(
        "image".into(),
        if crate::ruby::is_blank(&image) {
            Value::Null
        } else {
            json!(image)
        },
    );
    let video = ["video_secure_url", "video_url", "video"]
        .iter()
        .find_map(|k| d.get(*k).filter(|v| !v.is_null()).cloned())
        .unwrap_or(Value::Null);
    d.insert(
        "video".into(),
        if blank(&video) { Value::Null } else { video },
    );
    if present(d.get("article_published_time")) {
        let t = d["article_published_time"].clone();
        d.insert("published_time".into(), t);
    }
    if present(d.get("published_time")) {
        let t = d["published_time"]
            .as_str()
            .ok_or(Unsupported("published times that are lists"))?
            .to_string();
        match ruby_time::parse(&t)? {
            ruby_time::Parsed::Time(f) => {
                d.insert("article_published_time".into(), json!(f.short()));
                d.insert("article_published_time_title".into(), json!(f.long()));
            }
            ruby_time::Parsed::Raised => {
                return Err(Fail::Raised(format!("ArgumentError: {t:?}")));
            }
        }
    }
    // Twitter labels
    let label = |d: &Map<String, Value>, k: &str| str_of(d.get(k)).map(str::to_string);
    if let (Some(l1), Some(d1)) = (label(&d, "label1"), label(&d, "data1"))
        && !crate::ruby::is_blank(&l1)
        && !crate::ruby::is_blank(&d1)
        && allowed_label(&l1)
    {
        d.insert("label_1".into(), json!(helpers_truncate(&l1, 50)));
        d.insert("data_1".into(), json!(helpers_truncate(&d1, 50)));
    }
    if let (Some(l2), Some(d2)) = (label(&d, "label2"), label(&d, "data2"))
        && !crate::ruby::is_blank(&l2)
        && !crate::ruby::is_blank(&d2)
        && allowed_label(&l2)
    {
        if present(d.get("label_1")) {
            d.insert("label_2".into(), json!(helpers_truncate(&l2, 50)));
            d.insert("data_2".into(), json!(helpers_truncate(&d2, 50)));
        } else {
            d.insert("label_1".into(), json!(helpers_truncate(&l2, 50)));
            d.insert("data_1".into(), json!(helpers_truncate(&d2, 50)));
        }
    }
    if !present(d.get("label_1"))
        && let (Some(amount), Some(currency)) =
            (label(&d, "price_amount"), label(&d, "price_currency"))
        && !crate::ruby::is_blank(&amount)
        && !crate::ruby::is_blank(&currency)
    {
        d.insert("label_1".into(), json!("Price"));
        d.insert(
            "data_1".into(),
            json!(helpers_truncate(
                &format!(
                    "{} {}",
                    crate::ruby::strip(&currency),
                    crate::ruby::strip(&amount)
                ),
                50
            )),
        );
    }
    let mut errors = Vec::new();
    for (k, v) in &d {
        if k == "video" || k == "description" {
            continue;
        }
        if v.is_null() || v.as_str() == Some("") {
            errors.push(k.clone());
        }
    }
    Ok((d, errors))
}

impl Generic<'_> {
    fn s(&self, k: &str) -> Option<&str> {
        str_of(self.data.get(k))
    }

    fn present(&self, k: &str) -> bool {
        present(self.data.get(k))
    }

    /// `data[k] =~ regex`
    fn matches(&self, k: &str, re: &str) -> bool {
        self.s(k)
            .is_some_and(|v| regex::Regex::new(re).unwrap().is_match(v))
    }

    fn has_text(&self) -> bool {
        self.present("title")
    }

    fn has_image(&self) -> bool {
        self.present("image")
    }

    fn is_article(&self) -> bool {
        (self.matches("type", "article") || self.matches("asset_type", "article"))
            && self.has_text()
    }

    fn is_image(&self) -> bool {
        self.matches("type", "photo|image")
            && !self.matches("type", "photostream")
            && self.has_image()
    }

    fn is_video(&self) -> bool {
        self.matches("type", r"(?m)^video[/.]")
            && self.s("video_type") == Some("video/mp4")
            && self.present("video")
    }

    fn trusted_html_provider(&self) -> bool {
        let hosts: &[&str] = match self.s("provider_name") {
            Some("Flickr") => &["flickr.com"],
            Some("Meetup") => &["meetup.com"],
            _ => return false,
        };
        host_matches(&self.host, hosts)
    }

    fn is_embedded(&self) -> bool {
        let Some(html) = self.s("html").filter(|h| !crate::ruby::is_blank(h)) else {
            return false;
        };
        if self.trusted_html_provider() {
            return true;
        }
        let dom = crate::pretty_text::cleanup::parse(html);
        let iframes = select(&crate::pretty_text::cleanup::fragment_root(&dom), "iframe");
        !iframes.is_empty()
            && iframes.iter().all(|i| {
                attr(i, "src").is_some_and(|src| self.ctx.iframes.iter().any(|r| r.is_match(&src)))
            })
    }

    fn is_card(&self) -> bool {
        self.s("card") == Some("player")
            && self
                .s("player")
                .is_some_and(|p| self.ctx.iframes.iter().any(|r| r.is_match(p)))
    }

    fn force_article_html(&self) -> bool {
        host_matches(&self.host, &["imdb.com"]) && self.has_text()
    }

    fn article_html(&mut self) -> String {
        if !self.data.get("image").is_none_or(Value::is_null) {
            for (thumb, image, plain) in [
                ("thumbnail_width", "image_width", "width"),
                ("thumbnail_height", "image_height", "height"),
            ] {
                if self.data.get(thumb).is_none_or(Value::is_null) {
                    let v = [image, plain]
                        .iter()
                        .find_map(|k| self.data.get(*k).filter(|v| !v.is_null()).cloned())
                        .unwrap_or(Value::Null);
                    self.data.insert(thumb.into(), v);
                }
            }
        }
        if self.layout.is_none() {
            self.layout = Some(layout(
                "allowlistedgeneric",
                TEMPLATE,
                &Value::Object(self.data.clone()),
            ));
        }
        self.layout.clone().unwrap()
    }

    fn value_s(&self, k: &str) -> String {
        match self.data.get(k) {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
        }
    }

    fn image_html(&self) -> Option<String> {
        if !self.has_image() {
            return None;
        }
        let src = normalize_url_for_output(self.s("image").unwrap_or(""));
        let alt = ["description", "title"]
            .iter()
            .find(|k| !self.data.get(**k).is_none_or(Value::is_null))
            .map(|k| self.value_s(k))
            .unwrap_or_default();
        let first = |keys: [&str; 3]| {
            keys.iter()
                .find(|k| !self.data.get(**k).is_none_or(Value::is_null))
                .map(|k| self.value_s(k))
                .unwrap_or_default()
        };
        let width = first(["image_width", "thumbnail_width", "width"]);
        let height = first(["image_height", "thumbnail_height", "height"]);
        Some(format!(
            "<img src='{src}' alt='{alt}' width='{width}' height='{height}' class='onebox'>"
        ))
    }

    fn video_html(&self) -> String {
        let video = normalize_url_for_output(self.s("video").unwrap_or(""));
        let image = normalize_url_for_output(self.s("image").unwrap_or(""));
        format!(
            "          <video\n            title='{}'\n            width='{}'\n            height='{}'\n            style='max-width:100%'\n            poster='{image}'\n            controls=''\n          >\n            <source src='{video}'>\n          </video>\n",
            self.value_s("title"),
            self.value_s("video_width"),
            self.value_s("video_height"),
        )
    }

    fn card_html(&self) -> String {
        let src = normalize_url_for_output(self.s("player").unwrap_or(""));
        let width = if self.data.get("player_width").is_none_or(Value::is_null) {
            "100%".to_string()
        } else {
            self.value_s("player_width")
        };
        format!(
            "<iframe src=\"{src}\"\n        width=\"{width}\"\n        height=\"{}\"\n        scrolling=\"no\"\n        frameborder=\"0\">\n</iframe>\n",
            self.value_s("player_height")
        )
    }

    fn embedded_html(&self) -> String {
        use crate::pretty_text::cleanup::{fragment_root, node_html, set_attr, to_html};
        let dom = crate::pretty_text::cleanup::parse(self.s("html").unwrap_or(""));
        let root = fragment_root(&dom);
        if self.trusted_html_provider() {
            for img in select(&root, "img") {
                set_attr(&img, "class", "thumbnail");
            }
            return to_html(&dom);
        }
        let mut out = String::new();
        for iframe in select(&root, "iframe") {
            if let markup5ever_rcdom::NodeData::Element { attrs, .. } = &iframe.data {
                attrs.borrow_mut().retain(|a| &*a.name.local != "style");
            }
            let width = if self.data.get("width").is_none_or(Value::is_null) {
                "100%".to_string()
            } else {
                self.value_s("width")
            };
            set_attr(&iframe, "width", &width);
            if self.present("height") {
                set_attr(&iframe, "height", &self.value_s("height"));
            }
            set_attr(&iframe, "scrolling", "no");
            set_attr(&iframe, "frameborder", "0");
            out.push_str(&node_html(&iframe));
        }
        out
    }

    /// `generic_html`
    fn generic_html(&mut self) -> Option<String> {
        if self.is_article() || self.force_article_html() {
            return Some(self.article_html());
        }
        if self.is_video() {
            return Some(self.video_html());
        }
        if self.is_image() {
            return self.image_html();
        }
        if self.is_embedded() {
            return Some(self.embedded_html());
        }
        if self.is_card() {
            return Some(self.card_html());
        }
        if self.has_text() {
            return Some(self.article_html());
        }
        None
    }

    /// `to_html`: rewrite_https(generic_html)
    fn html(&mut self) -> Option<String> {
        let html = self.generic_html()?;
        Some(
            if host_matches(
                &self.host,
                &[
                    "slideshare.net",
                    "dailymotion.com",
                    "livestream.com",
                    "imgur.com",
                    "flickr.com",
                ],
            ) {
                html.replace("http://", "https://")
            } else {
                html
            },
        )
    }

    /// `placeholder_html`
    fn placeholder_html(&mut self) -> Option<String> {
        if self.is_article() || self.force_article_html() {
            return Some(self.article_html());
        }
        if self.is_image() {
            return self.image_html();
        }
        if self.is_video() || self.is_card() {
            return Some(super::engines::placeholder("video"));
        }
        if self.is_embedded() {
            return Some(super::engines::placeholder("generic"));
        }
        self.html()
    }
}

/// AllowlistedGenericOnebox's preview: its HTML, placeholder, errors and
/// verified data.
pub async fn preview<F: Fetch>(url: &str, ctx: &Ctx<'_>, fetch: &mut F) -> Result<Rendered, Fail> {
    let host = RubyUri::parse(url)
        .and_then(|u| u.host)
        .ok_or(Unsupported("onebox URLs URI can't parse"))?;
    // always_https? for the https hosts.
    let url = if https_host(&host) {
        super::engines::https(url)
    } else {
        url.to_string()
    };
    let mut embed = super::embed::Embed::new(&url, ctx);
    let raw_error = match embed.raw(fetch).await {
        Ok(_) => None,
        Err(Fail::App(e)) => return Err(Fail::App(e)),
        Err(e) => Some(e),
    };
    let raw = embed.raw_so_far();
    let mut rendered = Rendered::default();
    // `data` is memoized only once it returns; `raw` from its first call.
    let mut generic: Option<Generic> = None;
    let build = |raw: &Map<String, Value>| -> Result<Result<Generic, String>, Fail> {
        match data(&url, raw.clone()) {
            Ok((data, errors)) => Ok(Ok(Generic {
                host: host.clone(),
                data,
                errors,
                ctx,
                layout: None,
            })),
            Err(Fail::Raised(m)) => Ok(Err(m)),
            Err(Fail::Web(m)) => Ok(Err(m)),
            Err(Fail::App(e)) => Err(Fail::App(e)),
        }
    };
    // to_html
    match raw_error {
        Some(Fail::Raised(m)) => rendered.html_raised = Some(m),
        Some(_) => {}
        None => match build(&raw)? {
            Ok(mut g) => {
                rendered.html = g.html();
                generic = Some(g);
            }
            Err(m) => rendered.html_raised = Some(m),
        },
    }
    // placeholder_html
    if generic.is_none() {
        match build(&raw)? {
            Ok(g) => generic = Some(g),
            Err(m) => rendered.placeholder_raised = Some(m),
        }
    }
    if let Some(mut g) = generic {
        rendered.placeholder = g.placeholder_html();
        rendered.errors = g.errors.clone();
        rendered.verified_data = Some(Value::Object(g.data));
    }
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_as_the_helpers() {
        assert_eq!(helpers_truncate("hello world foo", 11), "hello world...");
        assert_eq!(helpers_truncate("hello world foo", 8), "hello...");
        assert_eq!(helpers_truncate("abcdefghij", 5), "abcde...");
        assert_eq!(helpers_truncate("short", 50), "short");
        assert_eq!(
            active_support_truncate("hello world foo", 13, "..."),
            "hello..."
        );
        assert_eq!(
            active_support_truncate("hello world foo", 12, "\u{2026}"),
            "hello world\u{2026}"
        );
    }
}
