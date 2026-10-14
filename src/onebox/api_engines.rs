//! The engines over a site's JSON API or its HTML (the JSON and HTML
//! engine mixins, and the ones that fetch on their own): xkcd, Hacker
//! News, Wikimedia Commons, Stack Exchange, Gfycat, Pastebin, PDFs,
//! Wikipedia, Amazon, X/Twitter and Threads posts, Google Play apps,
//! PubMed and Google Maps.

use std::rc::Rc;
use std::sync::LazyLock;

use markup5ever_rcdom::{Handle, NodeData};
use serde_json::{Map, Value, json};

use super::embed_engines::{CloneOk, clone_fail, rendered, rs};
use super::engines::{Ctx, Fail, Rendered, layout, link, placeholder};
use super::fetch::{Fetch, FetchError, FetchOpts};
use super::generic::{Kind, Normalizer, decode, helpers_truncate, open_graph};
use super::html_doc::{fetch_html_doc, parse};
use super::matcher::Engine;
use super::normalize_url_for_output;
use super::uri::RubyUri;
use crate::Unsupported;
use crate::pretty_text::cleanup::{attr, element_name, text};
use crate::pretty_text::select::{parent, select, select_first};

macro_rules! template {
    ($name:literal) => {
        include_str!(concat!(
            "../../vendor/discourse/lib/onebox/templates/",
            $name,
            ".mustache"
        ))
    };
}

fn raised(m: impl Into<String>) -> Fail {
    Fail::Raised(m.into())
}

fn nil(what: &str) -> Fail {
    raised(format!("NoMethodError: undefined method for nil ({what})"))
}

/// The JSON engine mixin's `raw`: `MultiJson.load(URI.parse(url).open)`.
async fn json_raw<F: Fetch>(fetch: &mut F, url: &str) -> Result<Value, Fail> {
    let body = match fetch.open(url, &[]).await? {
        Ok(b) => b,
        Err(FetchError::Refused(u)) => return Err(u.into()),
        Err(FetchError::Web(m)) => return Err(Fail::Web(m)),
        Err(FetchError::Other(m)) => return Err(raised(m)),
    };
    if crate::ruby::is_blank(&body) {
        return Ok(Value::Null);
    }
    serde_json::from_str(&body).map_err(|e| raised(format!("MultiJson::ParseError: {e}")))
}

/// `raw[key]`, raising on nil.
fn at<'v>(v: &'v Value, key: &str) -> Result<&'v Value, Fail> {
    match v {
        Value::Object(o) => Ok(o.get(key).unwrap_or(&Value::Null)),
        Value::Null => Err(nil(key)),
        _ => Err(Unsupported("API values of another shape").into()),
    }
}

/// The Layout over data, as HTML and as the placeholder.
fn layout_both(name: &str, view: &str, data: Result<Value, Fail>) -> Result<Rendered, Fail> {
    if let Err(Fail::App(_)) = &data {
        return data.map(|_| Rendered::default());
    }
    let html = data.as_ref().map_err(clone_fail).and_then(|d| match d {
        // Layout.new(name, nil) raises on nil's deep_symbolize_keys.
        Value::Null => Err(nil("deep_symbolize_keys")),
        d => Ok(Some(layout(name, view, d))),
    });
    let p = html.clone_ok();
    rendered(html, p)
}

// ---- JSON APIs ----

async fn xkcd<F: Fetch>(url: &str, fetch: &mut F) -> Result<Value, Fail> {
    static M: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"xkcd\.com/(?<comic_id>\d+)").unwrap());
    let id = M
        .captures(url)
        .map(|c| c["comic_id"].to_string())
        .ok_or_else(|| nil("match"))?;
    let raw = json_raw(fetch, &format!("https://xkcd.com/{id}/info.0.json")).await?;
    Ok(json!({
        "link": url,
        "title": at(&raw, "safe_title")?,
        "image": at(&raw, "img")?,
        "description": at(&raw, "alt")?,
    }))
}

async fn hackernews<F: Fetch>(url: &str, fetch: &mut F) -> Result<Value, Fail> {
    static M: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?m)^https?://news\.ycombinator\.com/item\?id=(?<item_id>\d+)").unwrap()
    });
    let id = M
        .captures(url)
        .map(|c| c["item_id"].to_string())
        .ok_or_else(|| nil("match"))?;
    let raw = json_raw(
        fetch,
        &format!("https://hacker-news.firebaseio.com/v0/item/{id}.json"),
    )
    .await?;
    let kind = at(&raw, "type")?.as_str().map(str::to_string);
    if !matches!(kind.as_deref(), Some("story" | "comment")) {
        return Ok(Value::Null);
    }
    let title = match at(&raw, "title")? {
        Value::String(t) => json!(helpers_truncate(t, 80)),
        Value::Null => Value::Null,
        _ => return Err(Unsupported("Hacker News titles of another shape").into()),
    };
    let time = at(&raw, "time")?
        .as_i64()
        .ok_or(Unsupported("Hacker News times that aren't integers"))?;
    let mut data = Map::new();
    data.insert("link".into(), json!(url));
    data.insert("title".into(), title);
    data.insert(
        "favicon".into(),
        json!("https://news.ycombinator.com/y18.svg"),
    );
    data.insert(
        "timestamp".into(),
        json!(super::ruby_time::at(time)?.strftime("%-l:%M %p - %-d %b %Y")?),
    );
    data.insert("author".into(), at(&raw, "by")?.clone());
    match at(&raw, "text")? {
        Value::Null | Value::Bool(false) => {}
        Value::String(t) => {
            data.insert(
                "description".into(),
                json!(decode(&helpers_truncate(t, 400))?),
            );
        }
        _ => return Err(Unsupported("Hacker News texts of another shape").into()),
    }
    if kind.as_deref() == Some("story") {
        data.insert("data_1".into(), at(&raw, "score")?.clone());
        data.insert("data_2".into(), at(&raw, "descendants")?.clone());
    }
    Ok(Value::Object(data))
}

async fn wikimedia<F: Fetch>(url: &str, fetch: &mut F) -> Result<Value, Fail> {
    static M: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?m)^https?://commons\.wikimedia\.org/wiki/(?<name>File:.+)").unwrap()
    });
    let name = M
        .captures(url)
        .map(|c| c["name"].to_string())
        .ok_or_else(|| nil("match"))?;
    let raw = json_raw(
        fetch,
        &format!(
            "https://en.wikipedia.org/w/api.php?action=query&titles={name}&prop=imageinfo&iilimit=50&iiprop=timestamp|user|url&iiurlwidth=500&format=json"
        ),
    )
    .await?;
    let pages = at(at(&raw, "query")?, "pages")?;
    let first = match pages {
        Value::Object(o) => o.values().next().ok_or_else(|| nil("first"))?,
        _ => return Err(nil("pages")),
    };
    let info = match at(first, "imageinfo")? {
        Value::Array(a) => a.first().ok_or_else(|| nil("imageinfo"))?,
        _ => return Err(nil("imageinfo")),
    };
    Ok(json!({
        "link": at(info, "descriptionurl")?,
        "title": at(first, "title")?,
        "image": at(info, "url")?,
        "thumbnail": at(info, "thumburl")?,
    }))
}

async fn stack_exchange<F: Fetch>(url: &str, fetch: &mut F) -> Result<Value, Fail> {
    static M: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?m)^https?://(?:(?:(?<subsubdomain>\w*)\.)?(?<subdomain>\w*)\.)?(?<domain>stackexchange\.com|stackoverflow\.com|superuser\.com|serverfault\.com|askubuntu\.com|stackapps\.com|mathoverflow\.net)/((?:questions|q)/(?<question_id>\d*)(/.*/(?<answer_id1>\d*))?|(a/(?<answer_id2>\d*)))",
        )
        .unwrap()
    });
    let c = M.captures(url).ok_or_else(|| nil("match"))?;
    let host = RubyUri::parse(url).and_then(|u| u.host).unwrap_or_default();
    let answer = c
        .name("answer_id2")
        .or_else(|| c.name("answer_id1"))
        .map(|m| m.as_str().to_string());
    let api = match answer {
        Some(a) => format!(
            "https://api.stackexchange.com/2.2/answers/{a}?site={host}&filter=!.FjueITQdx6-Rq3Ue9PWG.QZ2WNdW"
        ),
        None => format!(
            "https://api.stackexchange.com/2.2/questions/{}?site={host}&filter=!5-duuxrJa-iw9oVvOA(JNimB5VIisYwZgwcfNI",
            c.name("question_id").map_or("", |m| m.as_str())
        ),
    };
    let raw = json_raw(fetch, &api).await?;
    let mut result = match at(&raw, "items")? {
        Value::Array(a) => match a.first() {
            Some(Value::Object(o)) => o.clone(),
            None => return Ok(Value::Null),
            Some(_) => return Err(Unsupported("Stack Exchange items of another shape").into()),
        },
        _ => return Err(nil("items")),
    };
    let created = match result.get("creation_date") {
        Some(Value::Number(n)) if n.is_i64() => n.as_i64().unwrap(),
        Some(Value::Null) | None => 0,
        _ => return Err(Unsupported("Stack Exchange dates of another shape").into()),
    };
    result.insert(
        "creation_date".into(),
        json!(super::ruby_time::at(created)?.strftime("%I:%M%p - %d %b %y %Z")?),
    );
    let tags = match result.get("tags") {
        Some(Value::Array(t)) => t
            .iter()
            .take(4)
            .map(rs)
            .collect::<Result<Vec<_>, _>>()?
            .join(", "),
        _ => return Err(nil("tags")),
    };
    result.insert("tags".into(), json!(tags));
    let is_answer = result.contains_key("answer_id");
    let is_question = result.contains_key("question_id");
    result.insert("is_answer".into(), json!(is_answer));
    result.insert("is_question".into(), json!(is_question));
    Ok(Value::Object(result))
}

async fn pastebin<F: Fetch>(url: &str, fetch: &mut F) -> Result<Value, Fail> {
    // `case uri when %r{/raw/}` never matches a URI: the first segment.
    let path = RubyUri::parse(url).map(|u| u.path).unwrap_or_default();
    static KEY: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"/([^/]+)").unwrap());
    let key = KEY
        .captures(&path)
        .map(|c| c[1].to_string())
        .unwrap_or_default();
    let opts = FetchOpts {
        redirect_limit: 1,
        ..Default::default()
    };
    let response = match fetch
        .fetch_response(&format!("http://pastebin.com/raw/{key}"), &opts)
        .await?
    {
        Ok(b) => b,
        Err(FetchError::Refused(u)) => return Err(u.into()),
        Err(_) => String::new(),
    };
    let mut lines: Vec<&str> = response.split('\n').collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    Ok(json!({
        "title": "pastebin.com",
        "link": link(url)?,
        "content": lines.iter().take(10).copied().collect::<Vec<_>>().join("\n"),
        "truncated?": lines.len() > 10,
    }))
}

/// `Onebox::Helpers.pretty_filesize`
fn pretty_filesize(size: i64) -> String {
    const UNITS: [&str; 7] = ["B", "KB", "MB", "GB", "TB", "PB", "EB"];
    if size < 2 * 1024 {
        return format!("{size} B");
    }
    let f = size as f64;
    for i in 2..=7 {
        if f < 2.0 * 1024f64.powi(i) {
            return format!("{:.2} {}", f / 1024f64.powi(i - 1), UNITS[i as usize - 1]);
        }
    }
    format!("{:.2} {}", f / 1024f64.powi(6), UNITS[6])
}

async fn pdf<F: Fetch>(url: &str, fetch: &mut F) -> Result<Value, Fail> {
    let size = match fetch.content_length(url).await? {
        Ok(s) => s,
        Err(FetchError::Refused(u)) => return Err(u.into()),
        Err(_) => return Err(raised(format!("Unable to read pdf file: {url}"))),
    };
    let path = RubyUri::parse(url).map(|u| u.path).unwrap_or_default();
    let base = path
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();
    let base = if path == "/" || path.is_empty() {
        path.clone()
    } else {
        base
    };
    Ok(json!({
        "link": link(url)?,
        "title": base,
        "filesize": size.map(|s| pretty_filesize(crate::ruby::to_i(&s))),
    }))
}

async fn gfycat<F: Fetch>(url: &str, fetch: &mut F) -> Result<(String, String), Fail> {
    static M: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?m)^https?://gfycat\.com/(gifs/detail/)?(?<name>.+)").unwrap()
    });
    let name = M
        .captures(url)
        .map(|c| c["name"].to_string())
        .ok_or_else(|| nil("match"))?;
    let opts = FetchOpts {
        redirect_limit: 10,
        ..Default::default()
    };
    let response = match fetch.fetch_response(url, &opts).await? {
        Ok(b) => Some(b),
        Err(FetchError::Refused(u)) => return Err(u.into()),
        Err(_) => None,
    };
    let script = {
        let dom = parse(response.as_deref().unwrap_or(""));
        select_first(&dom.document, "script[type=\"application/ld+json\"]").map(|s| text(&s))
    };
    let og: Map<String, Value> = match script {
        Some(json) => match serde_json::from_str::<Value>(&json) {
            Ok(Value::Object(o)) => o,
            Ok(Value::Null) | Err(_) if crate::ruby::is_blank(&json) => Map::new(),
            Ok(_) => return Err(Unsupported("Gfycat JSON-LD of another shape").into()),
            Err(e) => return Err(raised(format!("MultiJson::ParseError: {e}"))),
        },
        None => Map::new(),
    };
    let s = |v: Option<&Value>| rs(v.unwrap_or(&Value::Null));
    let title = match og.get("headline") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => "No Title".to_string(),
        v => s(v)?,
    };
    let author = s(og.get("author"))?;
    let keywords = match og.get("keywords") {
        Some(Value::String(k)) => Some(
            k.split(',')
                .map(|k| format!("<a href='https://gfycat.com/gifs/search/{k}'>#{k}</a>"))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        None | Some(Value::Null) => None,
        Some(_) => return Err(Unsupported("Gfycat keywords of another shape").into()),
    };
    let (mut webm, mut mp4, mut poster, mut width, mut height) = (
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
    );
    if let Some(video) = og.get("video").filter(|v| !v.is_null()) {
        let content = normalize_url_for_output(&s(video.get("contentUrl"))?);
        let stem = match content.rfind('.') {
            Some(i)
                if !content[i..].contains('/') && i > content.rfind('/').map_or(0, |j| j + 1) =>
            {
                &content[..i]
            }
            _ => &content[..],
        };
        webm = format!("{stem}.webm");
        mp4 = format!("{stem}.mp4");
        poster = normalize_url_for_output(&s(video.get("thumbnailUrl"))?);
        width = s(video.get("width"))?;
        height = s(video.get("height"))?;
    }
    let html = format!(
        "          <aside class=\"onebox gfycat\">\n            <header class=\"source\">\n              <img src=\"https://gfycat.com/static/favicons/favicon-96x96.png\" class=\"site-icon\" width=\"64\" height=\"64\">\n              <a href=\"{url}\" target=\"_blank\" rel=\"nofollow ugc noopener\">Gfycat.com</a>\n            </header>\n\n            <article class=\"onebox-body\">\n              <h4>\n                {title} by\n                <a href=\"https://gfycat.com/@{author}\" target=\"_blank\" rel=\"nofollow ugc noopener\">\n                  <span>{author}</span>\n                </a>\n              </h4>\n\n              <div class=\"video\" style=\"--aspect-ratio: {width}/{height}\">\n                <video controls loop muted poster=\"{poster}\">\n                  <source id=\"webmSource\" src=\"{webm}\" type=\"video/webm\">\n                  <source id=\"mp4Source\" src=\"{mp4}\" type=\"video/mp4\">\n                  <img title=\"Sorry, your browser doesn't support HTML5 video.\" src=\"{poster}\">\n                </video>\n              </div>\n\n              <p>\n                <span class=\"label1\">{}</span>\n              </p>\n            </article>\n\n            <div style=\"clear: both\"></div>\n          </aside>\n",
        keywords.unwrap_or_default()
    );
    let placeholder = format!(
        "          <a href=\"{url}\">\n            <img src=\"{poster}\" width=\"{width}\" height=\"{height}\"><br/>\n            {name}\n          </a>\n"
    );
    Ok((html, placeholder))
}

// ---- HTML pages ----

/// `node.text.strip.presence`
fn text_at(root: &Handle, selector: &str) -> Option<String> {
    select_first(root, selector)
        .map(|n| crate::ruby::strip(&text(&n)).to_string())
        .filter(|t| !crate::ruby::is_blank(t))
}

/// The element children of a node.
fn child_elements(node: &Handle) -> Vec<Handle> {
    crate::pretty_text::select::element_children(node)
}

/// `CGI.unescapeHTML`
fn cgi_unescape_html(s: &str) -> String {
    static ENT: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"&(apos|amp|quot|gt|lt|#[0-9]+|#[xX][0-9A-Fa-f]+);").unwrap()
    });
    ENT.replace_all(s, |c: &regex::Captures| {
        let e = &c[1];
        match e {
            "apos" => "'".to_string(),
            "amp" => "&".to_string(),
            "quot" => "\"".to_string(),
            "gt" => ">".to_string(),
            "lt" => "<".to_string(),
            _ => {
                let n = if let Some(h) = e.strip_prefix("#x").or_else(|| e.strip_prefix("#X")) {
                    u32::from_str_radix(h, 16).ok()
                } else {
                    e[1..].parse().ok()
                };
                n.and_then(char::from_u32)
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| c[0].to_string())
            }
        }
    })
    .into_owned()
}

/// Amazon's `//div[@id=..]//X//tr[n]//td`-style lookups: the first match
/// of `steps` (a descendant step with a tag, an optional id and an
/// optional position among its parent's same-named children).
fn xpath_first(
    root: &Handle,
    steps: &[(&str, Option<&str>, Option<usize>, bool)],
) -> Option<Handle> {
    fn walk(
        node: &Handle,
        steps: &[(&str, Option<&str>, Option<usize>, bool)],
        out: &mut Vec<Handle>,
    ) {
        let Some(((tag, id, pos, descendant), rest)) = steps.split_first() else {
            out.push(node.clone());
            return;
        };
        let candidates: Vec<Handle> = if *descendant {
            crate::pretty_text::select::all_descendants(node)
        } else {
            child_elements(node)
        };
        for c in candidates {
            if element_name(&c) != Some(tag) {
                continue;
            }
            if let Some(id) = id
                && attr(&c, "id").as_deref() != Some(*id)
            {
                continue;
            }
            if let Some(p) = pos {
                let Some(par) = parent(&c) else { continue };
                let same: Vec<Handle> = child_elements(&par)
                    .into_iter()
                    .filter(|s| element_name(s) == Some(tag))
                    .collect();
                if !same.get(p - 1).is_some_and(|s| Rc::ptr_eq(s, &c)) {
                    continue;
                }
            }
            walk(&c, rest, out);
        }
    }
    let mut out = Vec::new();
    walk(root, steps, &mut out);
    // Document order, without repeats.
    let order = crate::pretty_text::select::all_descendants(root);
    order
        .into_iter()
        .find(|n| out.iter().any(|o| Rc::ptr_eq(o, n)))
}

struct Amazon {
    data: Value,
    errors: Vec<String>,
}

async fn amazon<F: Fetch>(url: &str, fetch: &mut F) -> Result<Amazon, Fail> {
    static ASIN: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?im)/[dg]p/(?:product/|video/detail/)?([A-Z0-9]+)(?:[/?]|$)").unwrap()
    });
    static TLD: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?m)^https?://(?:www\.)?(?:smile\.)?(amazon|amzn)\.(?<tld>com|ca|de|it|es|fr|co\.jp|co\.uk|cn|in|com\.br|com\.mx|nl|pl|sa|sg|se|com\.tr|ae)/",
        )
        .unwrap()
    });
    let asin = ASIN.captures(url).map(|c| c[1].to_string());
    let tld = TLD.captures(url).map(|c| c["tld"].to_string());
    let constructed = |asin: &str| -> Result<String, Fail> {
        Ok(format!(
            "https://www.amazon.{}/dp/{asin}",
            tld.clone().ok_or_else(|| nil("match"))?
        ))
    };
    let fetch_url = match &asin {
        Some(a) => constructed(a)?,
        None => url.to_string(),
    };
    let opts = FetchOpts {
        raise_error_when_response_too_large: false,
        ..Default::default()
    };
    let body = match fetch.fetch_response(&fetch_url, &opts).await? {
        Ok(b) => b,
        Err(FetchError::Refused(u)) => return Err(u.into()),
        Err(_) => String::new(),
    };
    let dom = parse(&body);
    let root = dom.document.clone();
    let link_url = match &asin {
        None => url.to_string(),
        Some(a) => {
            let canonical = select_first(&root, "link[rel=\"canonical\"]")
                .and_then(|l| attr(&l, "href"))
                .unwrap_or_default();
            let tail = format!("/dp/{}", a.to_lowercase());
            if canonical.to_lowercase().ends_with(&tail) {
                canonical
            } else {
                constructed(a)?
            }
        }
    };
    let og = Normalizer {
        kind: Kind::OpenGraph,
        data: open_graph(&root),
    };
    let description = match og.value("description")? {
        Value::Null => text_at(&root, "#productDescription").or_else(|| {
            select_first(&root, "meta[name=description]").and_then(|m| attr(&m, "content"))
        }),
        v => Some(rs(&v)?),
    };
    let title = cgi_unescape_html(&rs(&og.value("title")?)?);
    let image = match og.value("image")? {
        Value::Null => {
            let landing = select_first(&root, "#landingImage");
            let present = |v: Option<String>| v.filter(|s| !crate::ruby::is_blank(s));
            let from_dynamic = || -> Result<Option<String>, Fail> {
                let Some(json) = select_first(&root, "#ebooksImgBlkFront")
                    .and_then(|n| attr(&n, "data-a-dynamic-image"))
                else {
                    return Ok(None);
                };
                match serde_json::from_str::<Value>(&json) {
                    Ok(Value::Object(o)) => Ok(o.keys().next().cloned()),
                    Ok(_) => Err(Unsupported("Amazon image data of another shape").into()),
                    Err(e) => Err(raised(format!("JSON::ParserError: {e}"))),
                }
            };
            match present(select_first(&root, "#main-image").and_then(|n| attr(&n, "data-a-hires")))
                .or_else(|| present(landing.as_ref().and_then(|l| attr(l, "data-old-hires"))))
                .or_else(|| present(landing.as_ref().and_then(|l| attr(l, "src"))))
            {
                Some(i) => json!(i),
                None => json!(from_dynamic()?),
            }
        }
        v => v,
    };
    let price = text_at(&root, "#corePrice_feature_div .a-price .a-offscreen")
        .or_else(|| text_at(&root, ".mediaMatrixListItem.a-active .a-color-price"))
        .filter(|a| a.chars().any(|c| ('1'..='9').contains(&c)));
    let mut data = Map::new();
    data.insert("link".into(), json!(link_url));
    data.insert("title".into(), json!(title));
    data.insert("image".into(), image);
    data.insert(
        "description".into(),
        json!(description.map(|d| cgi_unescape_html(&helpers_truncate(&d, 250)))),
    );
    data.insert("price".into(), json!(price));
    // book_details
    let details: Option<Vec<(&str, Value)>> = if select_first(&root, "#dp.book_mobile").is_some() {
        let row = |n: usize| {
            xpath_first(
                &root,
                &[
                    ("div", Some("productDetails_secondary_view_div"), None, true),
                    (
                        "table",
                        Some("productDetails_techSpec_section_1"),
                        None,
                        true,
                    ),
                    ("tr", None, Some(n), true),
                    ("td", None, None, true),
                ],
            )
            .map(|n| crate::ruby::strip(&text(&n)).to_string())
            .filter(|t| !crate::ruby::is_blank(t))
        };
        Some(vec![
            ("title", json!(text_at(&root, "h1#title"))),
            ("isbn_asin_text", json!("ISBN")),
            ("isbn_asin", json!(row(8))),
            ("publisher", json!(row(1))),
            ("published", json!(row(2))),
        ])
    } else if select_first(&root, "#dp.ebooks_mobile").is_some() {
        let row = |n: usize| {
            xpath_first(
                &root,
                &[
                    ("div", Some("detailBullets_secondary_view_div"), None, true),
                    ("ul", None, None, true),
                    ("li", None, Some(n), true),
                    ("span", None, None, false),
                    ("span", None, Some(2), false),
                ],
            )
            .map(|n| crate::ruby::strip(&text(&n)).to_string())
            .filter(|t| !crate::ruby::is_blank(t))
        };
        Some(vec![
            ("title", json!(text_at(&root, "#ebooksTitle"))),
            ("isbn_asin_text", json!("ASIN")),
            ("isbn_asin", json!(row(4))),
            ("publisher", json!(row(2))),
            ("published", json!(row(1))),
        ])
    } else {
        None
    };
    if let Some(details) = details {
        let rating = text_at(&root, "#averageCustomerReviews_feature_div .a-icon")
            .or_else(|| text_at(&root, "#acrCustomerReviewLink .a-icon"));
        let has_price = data.get("price").is_some_and(|p| !p.is_null());
        for (k, v) in details {
            let v = if k == "published" {
                match v {
                    Value::String(p) => json!(format!("{p}{}", if has_price { ", " } else { "" })),
                    v => v,
                }
            } else {
                v
            };
            data.insert(k.into(), v);
        }
        data.insert(
            "by_info".into(),
            json!(text_at(&root, "#byline, #bylineInfo")),
        );
        data.insert("rating".into(), json!(rating.map(|r| format!("{r}, "))));
    }
    let mut errors = Vec::new();
    for k in ["title", "description"] {
        if data.get(k).is_none_or(super::generic::blank) {
            errors.push(k.to_string());
        }
    }
    Ok(Amazon {
        data: Value::Object(data),
        errors,
    })
}

async fn wikipedia<F: Fetch>(url: &str, fetch: &mut F) -> Result<Value, Fail> {
    static HASH: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"#([^/?]+)").unwrap());
    static REFS: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\[\d+\]").unwrap());
    static BREAK: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"p|text|div|ul").unwrap());
    let html = fetch_html_doc(fetch, url, &[]).await?;
    let dom = parse(html.as_deref().unwrap_or(""));
    let root = dom.document.clone();
    let mut paras: Vec<Handle> = Vec::new();
    let mut section_title: Option<String> = None;
    let all_p = || select(&root, "p");
    match HASH.captures(url).map(|c| c[1].to_string()) {
        None => paras = all_p(),
        Some(hash) => {
            let id = super::uri::cgi_unescape(&hash)
                .ok_or(Unsupported("fragments that aren't UTF-8"))?;
            let headers: Vec<Handle> = crate::pretty_text::select::all_descendants(&root)
                .into_iter()
                .filter(|n| {
                    attr(n, "id").as_deref() == Some(id.as_str())
                        && matches!(
                            element_name(n),
                            Some("h1" | "h2" | "h3" | "h4" | "h5" | "h6")
                        )
                })
                .collect();
            if headers.is_empty() {
                paras = all_p();
            } else {
                section_title = Some(headers.iter().map(text).collect());
                let mut cur = parent(&headers[0]).ok_or_else(|| nil("parent"))?;
                let mut first_p = false;
                loop {
                    let next = next_sibling(&cur).ok_or_else(|| nil("next_sibling"))?;
                    let name = node_name(&next);
                    if !BREAK.is_match(&name) && first_p {
                        break;
                    }
                    cur = next;
                    if name == "p" || name == "ul" {
                        first_p = true;
                        paras.push(cur.clone());
                    }
                }
            }
        }
    }
    let max = 500usize;
    let mut text_out = String::new();
    let mut cnt = 0;
    if !paras.is_empty() {
        while text_out.chars().count() < max && cnt <= 3 {
            if cnt >= paras.len() {
                break;
            }
            if cnt != 0 {
                text_out.push(' ');
            }
            let p = &paras[cnt];
            let paragraph: String = if element_name(p) == Some("ul") {
                let items: Vec<String> = select(p, "li")
                    .iter()
                    .enumerate()
                    .map(|(i, li)| format!("{}.{}", i + 1, text(li)))
                    .collect();
                items.join(" |\n ").chars().take(max + 1).collect()
            } else {
                text(p).chars().take(max + 1).collect()
            };
            text_out.push_str(&REFS.replace_all(&paragraph, ""));
            cnt += 1;
        }
    }
    if text_out.chars().count() > max {
        text_out = format!("{}...", text_out.chars().take(max + 1).collect::<String>());
    }
    let h1: String = select(&root, "html body h1").iter().map(text).collect();
    let title = match &section_title {
        Some(s) => format!("{h1} | {s}"),
        None => h1,
    };
    let mut result = Map::new();
    result.insert("link".into(), json!(link(url)?));
    result.insert("title".into(), json!(title));
    result.insert("description".into(), json!(text_out));
    for img in select(&root, ".infobox-image img") {
        let src = attr(&img, "src");
        if !src.as_deref().is_some_and(|s| s.contains("Question_book")) {
            result.insert("image".into(), json!(src));
            break;
        }
    }
    Ok(Value::Object(result))
}

/// `node.next_sibling`, text and comment nodes included.
fn next_sibling(node: &Handle) -> Option<Handle> {
    let p = parent(node)?;
    let children = p.children.borrow();
    let i = children.iter().position(|c| Rc::ptr_eq(c, node))?;
    children.get(i + 1).cloned()
}

/// Nokogiri's `node.name`.
fn node_name(node: &Handle) -> String {
    match &node.data {
        NodeData::Element { name, .. } => name.local.to_string(),
        NodeData::Text { .. } => "text".into(),
        NodeData::Comment { .. } => "comment".into(),
        _ => "#other".into(),
    }
}

/// `og:` metas as TwitterStatusOnebox reads them: every one, the last
/// winning, stripped.
fn og_metas(root: &Handle) -> Map<String, Value> {
    static OG: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?im)^og:").unwrap());
    let mut data = Map::new();
    for m in select(root, "meta") {
        let Some(property) = attr(&m, "property") else {
            continue;
        };
        if !OG.is_match(&property) {
            continue;
        }
        let content = crate::ruby::strip(&attr(&m, "content").unwrap_or_default()).to_string();
        let key = property.replacen("og:", "", usize::MAX).replace(':', "_");
        data.insert(key, json!(content));
    }
    data
}

async fn twitter_status<F: Fetch>(url: &str, ctx: &Ctx<'_>, fetch: &mut F) -> Result<Value, Fail> {
    if ctx.settings.twitter_credentials {
        return Err(Unsupported("X posts through the Twitter API").into());
    }
    let opts = FetchOpts {
        allow_cross_domain_cookies: true,
        ..Default::default()
    };
    let response = match fetch.fetch_response(url, &opts).await? {
        Ok(b) => b,
        Err(FetchError::Refused(u)) => return Err(u.into()),
        // get_twitter_data returns nil, which `[:description]` raises on.
        Err(_) => return Err(nil("twitter_data")),
    };
    let og = {
        let dom = parse(&response);
        og_metas(&dom.document)
    };
    let get = |k: &str| og.get(k).and_then(Value::as_str).map(str::to_string);
    static QUOTES: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?is)“(.+?)”").unwrap());
    static SCREEN: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\(@([^\)\(]*)\) on X").unwrap());
    let tweet = get("description").map(|d| QUOTES.replace_all(&d, "$1").into_owned());
    let title = get("title");
    let screen_name = title
        .as_deref()
        .filter(|t| !crate::ruby::is_blank(t))
        .and_then(|t| SCREEN.captures(t).map(|c| c[1].to_string()));
    let avatar = get("image").filter(|i| i.contains("profile_images"));
    let mut data = Map::new();
    data.insert("link".into(), json!(link(url)?));
    data.insert("tweet".into(), json!(tweet));
    data.insert("timestamp".into(), Value::Null);
    data.insert("title".into(), json!(title));
    data.insert("screen_name".into(), json!(screen_name));
    data.insert("avatar".into(), json!(avatar));
    for k in [
        "likes",
        "retweets",
        "is_reply",
        "quoted_text",
        "quoted_full_name",
        "quoted_screen_name",
        "quoted_link",
    ] {
        data.insert(k.into(), Value::Null);
    }
    Ok(Value::Object(data))
}

async fn threads<F: Fetch>(url: &str, fetch: &mut F) -> Result<Value, Fail> {
    let html = fetch_html_doc(fetch, url, &[]).await?;
    let (link_href, favicon, og) = {
        let dom = parse(html.as_deref().unwrap_or(""));
        let root = dom.document.clone();
        let first_href = |sel: &str| -> Result<Value, Fail> {
            let l = select(&root, sel)
                .into_iter()
                .next()
                .ok_or_else(|| nil(sel))?;
            Ok(json!(attr(&l, "href")))
        };
        (
            first_href("link[rel='canonical']")?,
            first_href("link[rel='icon']")?,
            open_graph(&root),
        )
    };
    let og_s = |k: &str| -> Result<String, Fail> {
        og.get(k)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| nil(k))
    };
    static MENTION: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"@([\w\d]+)").unwrap());
    let description = og_s("description")?;
    let rest: Vec<&str> = description.split(". ").skip(1).collect();
    let description_text = MENTION
        .replace_all(
            &rest.join(". "),
            "<a href='https://www.threads.net/@$1'>@$1</a>",
        )
        .into_owned();
    let og_title = og_s("title")?;
    let title = og_title.split(" (@").next().unwrap_or("").to_string();
    let screen_name = og_title
        .split(" (@")
        .skip(1)
        .collect::<Vec<_>>()
        .join(" (@")
        .split(") on Threads")
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let screen = screen_name.clone().unwrap_or_default();
    let avatar = match fetch
        .fetch_response(
            &format!("https://www.threads.net/@{screen}"),
            &FetchOpts::default(),
        )
        .await?
    {
        Ok(b) => {
            let dom = parse(&b);
            open_graph(&dom.document)
                .get("image")
                .cloned()
                .unwrap_or(Value::Null)
        }
        Err(FetchError::Refused(u)) => return Err(u.into()),
        Err(_) => Value::Null,
    };
    let likes = description
        .split(' ')
        .find(|s| !s.is_empty())
        .map(str::to_string);
    let replies = description
        .split(", ")
        .skip(1)
        .collect::<Vec<_>>()
        .join(", ")
        .split(" repl")
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let image = og.get("image").cloned().unwrap_or(Value::Null);
    let image_s = image.as_str().ok_or_else(|| nil("image"))?;
    let avatar_s = avatar.as_str().ok_or_else(|| nil("avatar"))?;
    let image = if image_s.split('?').next() == avatar_s.split('?').next() {
        Value::Null
    } else {
        image.clone()
    };
    Ok(json!({
        "favicon": favicon,
        "link": link_href,
        "description": description_text,
        "image": image,
        "title": title,
        "screen_name": screen_name,
        "avatar": avatar,
        "likes": likes,
        "replies": replies,
    }))
}

async fn google_play<F: Fetch>(url: &str, fetch: &mut F) -> Result<Value, Fail> {
    let html = fetch_html_doc(fetch, url, &[]).await?;
    let dom = parse(html.as_deref().unwrap_or(""));
    let root = dom.document.clone();
    let first_attr = |sel: &str, a: &str| -> Option<Option<String>> {
        select(&root, sel).into_iter().next().map(|n| attr(&n, a))
    };
    let price = match first_attr("meta[itemprop=price]", "content") {
        Some(Some(p)) => Some(p),
        Some(None) => None,
        None => Some("Free".to_string()),
    };
    let title = first_attr("meta[property='og:title']", "content")
        .ok_or_else(|| nil("og:title"))?
        .ok_or_else(|| nil("content"))?
        .replace(" - Apps on Google Play", "");
    let image =
        first_attr("meta[property='og:image']", "content").ok_or_else(|| nil("og:image"))?;
    let description = first_attr("meta[name=description]", "content")
        .ok_or_else(|| nil("description"))?
        .ok_or_else(|| nil("content"))?;
    let mut d: String = description.chars().take(501).collect();
    d.pop();
    Ok(json!({
        "link": link(url)?,
        "title": title,
        "image": normalize_url_for_output(image.as_deref().unwrap_or("")),
        "description": format!("{d}..."),
        "price": if price.as_deref() == Some("0") { Some("Free".to_string()) } else { price },
    }))
}

async fn pubmed<F: Fetch>(url: &str, fetch: &mut F) -> Result<Value, Fail> {
    static PMID: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"www\.ncbi\.nlm\.nih\.gov/pubmed/(?<pmid>[0-9]+)").unwrap()
    });
    let xml_url = RubyUri::parse(url)
        .and_then(|u| u.merge("?report=xml&format=text"))
        .map(|u| u.to_s())
        .ok_or(Unsupported("PubMed URLs URI can't join"))?;
    let body = match fetch.open(&xml_url, &[]).await? {
        Ok(b) => b,
        Err(FetchError::Refused(u)) => return Err(u.into()),
        Err(FetchError::Web(m)) => return Err(Fail::Web(m)),
        Err(FetchError::Other(m)) => return Err(raised(m)),
    };
    let doc =
        super::xml::parse(&body).ok_or(Unsupported("PubMed pages that aren't well-formed XML"))?;
    let pre: String = doc
        .descendants_named("pre")
        .iter()
        .map(|p| p.text())
        .collect();
    let xml = super::xml::parse(&format!("<root>{pre}</root>"))
        .ok_or(Unsupported("PubMed records that aren't well-formed XML"))?;
    let texts =
        |n: &str| -> Vec<String> { xml.descendants_named(n).iter().map(|e| e.text()).collect() };
    let initials = texts("Initials");
    let last = texts("LastName");
    if last.len() < initials.len() {
        return Err(raised(
            "TypeError: no implicit conversion of nil into String",
        ));
    }
    let mut authors: Vec<String> = initials
        .iter()
        .zip(&last)
        .map(|(i, l)| format!("{i} {l}"))
        .collect();
    if authors.len() > 1 {
        let n = authors.len();
        authors[n - 2] = format!("{} and {}", authors[n - 2], authors[n - 1]);
        authors.pop();
    }
    static WS_LINE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?m)^\s+$").unwrap());
    let mut parts: Vec<String> = Vec::new();
    for pd in xml.descendants_named("PubDate") {
        for c in &pd.children {
            let s = match c {
                super::xml::Node::Text(t) => t.clone(),
                super::xml::Node::Element(e) => e.text(),
            };
            if WS_LINE.is_match(&s) {
                continue;
            }
            parts.extend(s.split_whitespace().map(str::to_string));
        }
    }
    parts.sort();
    parts.reverse();
    Ok(json!({
        "title": texts("ArticleTitle").concat(),
        "authors": authors.join(", "),
        "journal": texts("Title").concat(),
        "abstract": texts("AbstractText").concat(),
        "date": parts.join(" "),
        "link": url,
        "pmid": PMID.captures(url).map(|c| c["pmid"].to_string()).ok_or_else(|| nil("match"))?,
    }))
}

// ---- Google Maps ----

struct Maps {
    url: String,
}

fn maps_type(url: &str) -> Option<(&'static str, Option<regex::Captures<'_>>)> {
    static SHORT: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?m)^(https?:)?//goo\.gl/maps/").unwrap());
    static CUSTOM: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?m)^(?:https?:)?//www\.google(?:\.(?:\w{2,}))+/maps/d/(?:edit|viewer|embed)\?mid=.+$").unwrap()
    });
    static STREET: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?m)^(?:https?:)?//www\.google(?:\.(?:\w{2,}))+/maps[^@]+@(?<lon>-?[\d.]+),(?<lat>-?[\d.]+),(?:\d+)a,(?<zoom>[\d.]+)y,(?<heading>[\d.]+)h,(?<pitch>[\d.]+)t.+?data=.*?!1s(?<pano>[^!]{22})").unwrap()
    });
    static STANDARD: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?m)^(?:https?:)?//www\.google(?:\.(?:\w{2,}))+/maps").unwrap()
    });
    static CANONICAL: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?m)^(?:https?:)?//maps\.google(?:\.(?:\w{2,}))+/maps\?").unwrap()
    });
    if SHORT.is_match(url) {
        return Some(("short", None));
    }
    if CUSTOM.is_match(url) {
        return Some(("custom", None));
    }
    if let Some(c) = STREET.captures(url) {
        return Some(("streetview", Some(c)));
    }
    if STANDARD.is_match(url) {
        return Some(("standard", None));
    }
    if CANONICAL.is_match(url) {
        return Some(("canonical", None));
    }
    None
}

/// `follow_redirect!`: the URL a HEAD request leads to.
async fn follow_redirect<F: Fetch>(url: &str, fetch: &mut F) -> Result<String, Fail> {
    match fetch.head(url).await? {
        Ok((200, _)) => Ok(RubyUri::parse(url)
            .map(|u| u.to_s())
            .unwrap_or_else(|| url.to_string())),
        Ok((301 | 302, location)) => location.ok_or_else(|| raised("ArgumentError: no Location")),
        Ok((code, _)) => Err(raised(format!(
            "ArgumentError: unexpected response code {code}"
        ))),
        Err(FetchError::Refused(u)) => Err(u.into()),
        Err(_) => Err(raised("ArgumentError: malformed url or unresolveable")),
    }
}

/// Ruby's `Float#round(4).to_s` for a parsed decimal.
fn round4(f: f64) -> Result<String, Fail> {
    let r = (f * 10_000.0).round() / 10_000.0;
    Ok(super::embed_engines::float_s(r)?)
}

async fn google_maps<F: Fetch>(url: &str, fetch: &mut F) -> Result<Maps, Fail> {
    let mut url = url.to_string();
    let Some((mut kind, _)) = maps_type(&url) else {
        return Err(raised("ArgumentError: does not match any known pattern"));
    };
    if kind == "short" {
        url = follow_redirect(&url, fetch).await?;
        kind = maps_type(&url)
            .ok_or_else(|| raised("ArgumentError: does not match"))?
            .0;
    }
    if kind == "standard" {
        for _ in 0..9 {
            url = follow_redirect(&url, fetch).await?;
            kind = maps_type(&url)
                .ok_or_else(|| raised("ArgumentError: does not match"))?
                .0;
            if kind != "standard" {
                break;
            }
        }
    }
    match kind {
        "standard" => {
            static AT: LazyLock<regex::Regex> = LazyLock::new(|| {
                regex::Regex::new(
                    r"@(?<lon>[\d.-]+),(?<lat>[\d.-]+),(?<zoom>\d+)(\.\d+)?(?<mz>[mz])",
                )
                .unwrap()
            });
            static PLACE: LazyLock<regex::Regex> =
                LazyLock::new(|| regex::Regex::new(r"/place/([^/\?]+)").unwrap());
            static CID: LazyLock<regex::Regex> = LazyLock::new(|| {
                regex::Regex::new(r"!3m1!1s0x([0-9a-fA-F]{16}):0x([0-9a-fA-F]{16})").unwrap()
            });
            let m = AT
                .captures(&url)
                .ok_or_else(|| raised(format!("unexpected standard url {url}")))?;
            let zoom = if &m["mz"] == "z" {
                m["zoom"].to_string()
            } else {
                let z: f64 = m["zoom"].parse().unwrap_or(0.0);
                ((57280048.0 / z).log2().round() as i64).to_string()
            };
            let mut out = format!(
                "https://maps.google.com/maps?ll={},{}&z={zoom}&output=embed&dg=ntvb",
                &m["lon"], &m["lat"]
            );
            if let Some(p) = PLACE.captures(&url) {
                out.push_str(&format!("&q={}", &p[1]));
            }
            if let Some(c) = CID.captures(&url) {
                let cid = u128::from_str_radix(&format!("{}{}", &c[1], &c[2]), 16).unwrap_or(0);
                out.push_str(&format!("&cid={cid}"));
            }
            Ok(Maps { url: out })
        }
        "custom" => {
            let mut uri = RubyUri::parse(&url).ok_or_else(|| raised("URI::InvalidURIError"))?;
            static LAST: LazyLock<fancy_regex::Regex> =
                LazyLock::new(|| fancy_regex::Regex::new(r"(?<=^/maps/d/)\w+$").unwrap());
            uri.path = LAST.replace(&uri.path, "embed").into_owned();
            Ok(Maps { url: uri.to_s() })
        }
        "streetview" => {
            let (_, c) = maps_type(&url).unwrap();
            let c = c.unwrap();
            let f = |k: &str| -> f64 { c[k].parse::<f64>().unwrap_or(0.0) };
            let out = format!(
                "https://www.google.com/maps/embed?pb=!3m2!2sen!4v0!6m8!1m7!1s{}!2m2!1d{}!2d{}!3f{}!4f{}!5f{}",
                &c["pano"],
                super::embed_engines::float_s(f("lon"))?,
                super::embed_engines::float_s(f("lat"))?,
                round4(f("heading"))?,
                round4(f("pitch") / 10.0)?,
                round4(f("zoom") / 100.0)?
            );
            Ok(Maps { url: out })
        }
        "canonical" => {
            let query = RubyUri::parse(&url)
                .and_then(|u| u.query)
                .unwrap_or_default();
            let mut q: Vec<(String, String)> = Vec::new();
            for (k, v) in form_urlencoded::parse(query.as_bytes()) {
                match q.iter_mut().find(|(e, _)| *e == k) {
                    Some(e) => e.1 = v.into_owned(),
                    None => q.push((k.into_owned(), v.into_owned())),
                }
            }
            let get = |q: &Vec<(String, String)>, k: &str| {
                q.iter().find(|(e, _)| e == k).map(|(_, v)| v.clone())
            };
            if get(&q, "ll").is_none() {
                let sll = get(&q, "sll").ok_or_else(|| {
                    raised("ArgumentError: canonical url lacks location argument")
                })?;
                url.push_str(&format!("&ll={sll}"));
                q.push(("ll".into(), sll));
            }
            if get(&q, "z").is_none() && get(&q, "spn").is_none() {
                let sspn = get(&q, "sspn").ok_or_else(|| {
                    raised("ArgumentError: canonical url has incomplete query arguments")
                })?;
                url.push_str(&format!("&spn={sspn}"));
                q.push(("spn".into(), sspn));
            }
            Ok(Maps {
                url: url.replacen("output=classic", "output=embed", 1),
            })
        }
        _ => Err(raised(format!("unexpected url type {kind}"))),
    }
}

/// The JSON and HTML engines' previews.
pub async fn preview<F: Fetch>(
    engine: &Engine,
    url: &str,
    ctx: &Ctx<'_>,
    fetch: &mut F,
) -> Result<Rendered, Fail> {
    let always = if engine.name == "StackExchangeOnebox" {
        // `always_https?`: hosts of at most three labels.
        let host = RubyUri::parse(url).and_then(|u| u.host).unwrap_or_default();
        host.split('.').count() <= 3
    } else {
        engine.always_https
    };
    let url = if always {
        super::engines::https(url)
    } else {
        url.to_string()
    };
    match engine.name {
        "XkcdOnebox" => layout_both("xkcd", template!("xkcd"), xkcd(&url, fetch).await),
        "HackernewsOnebox" => layout_both(
            "hackernews",
            template!("hackernews"),
            hackernews(&url, fetch).await,
        ),
        "WikimediaOnebox" => layout_both(
            "wikimedia",
            template!("wikimedia"),
            wikimedia(&url, fetch).await,
        ),
        "StackExchangeOnebox" => layout_both(
            "stackexchange",
            template!("stackexchange"),
            stack_exchange(&url, fetch).await,
        ),
        "PastebinOnebox" => layout_both(
            "pastebin",
            template!("pastebin"),
            pastebin(&url, fetch).await,
        ),
        "PdfOnebox" => layout_both("pdf", template!("pdf"), pdf(&url, fetch).await),
        "WikipediaOnebox" => layout_both(
            "wikipedia",
            template!("wikipedia"),
            wikipedia(&url, fetch).await,
        ),
        "GooglePlayAppOnebox" => layout_both(
            "googleplayapp",
            template!("googleplayapp"),
            google_play(&url, fetch).await,
        ),
        "PubmedOnebox" => layout_both("pubmed", template!("pubmed"), pubmed(&url, fetch).await),
        "ThreadsStatusOnebox" => layout_both(
            "threadsstatus",
            template!("threadsstatus"),
            threads(&url, fetch).await,
        ),
        "GfycatOnebox" => match gfycat(&url, fetch).await {
            Ok((h, p)) => rendered(Ok(Some(h)), Ok(Some(p))),
            Err(Fail::App(e)) => Err(Fail::App(e)),
            Err(e) => {
                let e2 = clone_fail(&e);
                rendered(Err(e), Err(e2))
            }
        },
        "TwitterStatusOnebox" => {
            let data = twitter_status(&url, ctx, fetch).await;
            if let Err(Fail::App(_)) = &data {
                return data.map(|_| Rendered::default());
            }
            let mut errors = Vec::new();
            let html = data.as_ref().map_err(clone_fail).map(|d| {
                for k in ["title", "tweet"] {
                    if d.get(k).is_none_or(super::generic::blank) {
                        errors.push(k.to_string());
                    }
                }
                if errors.is_empty() {
                    Some(layout("twitterstatus", template!("twitterstatus"), d))
                } else {
                    Some(String::new())
                }
            });
            let p = html.clone_ok();
            let mut r = rendered(html, p)?;
            r.errors = errors;
            if let Ok(d) = data {
                r.verified_data = Some(d);
            }
            Ok(r)
        }
        "AmazonOnebox" => {
            let a = amazon(&url, fetch).await;
            if let Err(Fail::App(_)) = &a {
                return a.map(|_| Rendered::default());
            }
            match a {
                Ok(a) => {
                    let view = layout("amazon", template!("amazon"), &a.data);
                    let html = if a.errors.is_empty() {
                        view.clone()
                    } else {
                        String::new()
                    };
                    let mut r = rendered(Ok(Some(html)), Ok(Some(view)))?;
                    r.errors = a.errors;
                    r.verified_data = Some(a.data);
                    Ok(r)
                }
                Err(e) => {
                    let e2 = clone_fail(&e);
                    rendered(Err(e), Err(e2))
                }
            }
        }
        "GoogleMapsOnebox" => match google_maps(&url, fetch).await {
            Ok(m) => {
                let html = format!(
                    "<div class='maps-onebox'><iframe src=\"{}\" width=\"690\" height=\"400\" frameborder=\"0\" style=\"border:0\">{}</iframe></div>",
                    link(&m.url)?,
                    placeholder("map")
                );
                rendered(Ok(Some(html)), Ok(Some(placeholder("map"))))
            }
            // The engine raises as it is built: Preview#to_s doesn't rescue that.
            Err(Fail::Web(m)) => Err(Fail::Raised(m)),
            Err(e) => Err(e),
        },
        _ => Err(Unsupported("onebox engines not ported").into()),
    }
}
