//! `Onebox::Helpers.fetch_html_doc`: a page fetched and parsed as
//! Nokogiri.HTML (libxml2's HTML parser) reads it, following its
//! canonical link when that names another page.
//!
//! The page is parsed by html5ever with scripting off, so `<noscript>`
//! holds elements as it does for libxml2; the page is given to Nokogiri
//! as UTF-8 (fetch_response's string is), so a page that isn't UTF-8 is
//! refused.

use std::sync::LazyLock;

use html5ever::tendril::TendrilSink;
use html5ever::tree_builder::TreeBuilderOpts;
use html5ever::{ParseOpts, parse_document};
use markup5ever_rcdom::{Handle, RcDom};

use super::fetch::{Fetch, FetchError, FetchOpts};
use crate::pretty_text::cleanup::attr;
use crate::pretty_text::select::select_first;
use crate::{AppError, Unsupported};

/// `Nokogiri.HTML(html)`
pub fn parse(html: &str) -> RcDom {
    let opts = ParseOpts {
        tree_builder: TreeBuilderOpts {
            scripting_enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };
    parse_document(RcDom::default(), opts)
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .expect("reading from a byte slice cannot fail")
}

/// The document's root (what `doc.css` searches).
pub fn root(dom: &RcDom) -> Handle {
    dom.document.clone()
}

/// `Onebox::Helpers::IGNORE_CANONICAL_DOMAINS`, matched as regexes
/// against the host.
static IGNORE_CANONICAL: LazyLock<Vec<regex::Regex>> = LazyLock::new(|| {
    ["www.instagram.com", "medium.com", "youtube.com"]
        .iter()
        .map(|h| regex::Regex::new(h).unwrap())
        .collect()
});

static ADDRESSABLE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\A(?:([^:/?#]+):)?(?://([^/?#]*))?([^?#]*)(?:\?([^#]*))?(?:#(.*))?\z")
        .unwrap()
});

/// An URL as `Addressable::URI.parse(url).normalize!` leaves it.
pub struct Normalized {
    pub host: Option<String>,
    pub path: String,
    pub url: String,
}

/// `Addressable::URI.normalize_component`: `%XX` decoded (but `%2B` in a
/// query), then what's outside the class encoded again (a query's `%2B`
/// staying as it is).
fn normalize_component(
    s: &str,
    allowed: impl Fn(u8) -> bool,
    query: bool,
) -> Result<String, Unsupported> {
    let b = s.as_bytes();
    let hex = |i: usize| b.get(i).is_some_and(u8::is_ascii_hexdigit);
    let mut decoded: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && hex(i + 1) && hex(i + 2) {
            let byte = u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(0);
            if !(query && byte == b'+') {
                if byte >= 0x80 {
                    return Err(Unsupported(
                        "canonical links that need Addressable's normalizing",
                    ));
                }
                decoded.push(byte);
                i += 3;
                continue;
            }
        }
        decoded.push(b[i]);
        i += 1;
    }
    let mut out = String::new();
    for (j, &c) in decoded.iter().enumerate() {
        let kept_plus = query
            && c == b'%'
            && decoded.get(j + 1) == Some(&b'2')
            && matches!(decoded.get(j + 2), Some(b'B' | b'b'));
        if (allowed(c) && c != b'%') || kept_plus {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    Ok(out)
}

fn pchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@".contains(&b)
}

/// `Addressable::URI.parse(url).normalize!`, for the URLs whose
/// normalizing is ported: ASCII, nothing percent-encoded, no dot
/// segments, no userinfo. None where Addressable raises.
pub fn normalize(url: &str) -> Result<Option<Normalized>, Unsupported> {
    if !url.is_ascii() || url.chars().any(|c| c.is_ascii_whitespace()) {
        return Err(Unsupported(
            "canonical links that need Addressable's normalizing",
        ));
    }
    let Some(c) = ADDRESSABLE.captures(url) else {
        return Ok(None);
    };
    let scheme = c.get(1).map(|m| m.as_str().to_ascii_lowercase());
    let mut out = String::new();
    if let Some(s) = &scheme {
        out.push_str(s);
        out.push(':');
    }
    let mut host = None;
    if let Some(authority) = c.get(2) {
        let authority = authority.as_str();
        if authority.contains('@') || authority.starts_with('[') {
            return Err(Unsupported(
                "canonical links that need Addressable's normalizing",
            ));
        }
        let (h, port) = match authority.rsplit_once(':') {
            Some((h, p)) => {
                if !p.chars().all(|c| c.is_ascii_digit()) {
                    return Ok(None);
                }
                (h, p.parse::<u32>().ok())
            }
            None => (authority, None),
        };
        let mut h = h.to_ascii_lowercase();
        if h.len() > 1 && h.ends_with('.') && !h.ends_with("..") {
            h.pop();
        }
        let default = match scheme.as_deref() {
            Some("http") => Some(80),
            Some("https") => Some(443),
            Some("ftp") => Some(21),
            _ => None,
        };
        out.push_str("//");
        out.push_str(&h);
        if let Some(p) = port
            && Some(p) != default
        {
            out.push_str(&format!(":{p}"));
        }
        host = Some(h);
    }
    let raw_path = c.get(3).map_or("", |m| m.as_str());
    if raw_path
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(Unsupported(
            "canonical links that need Addressable's normalizing",
        ));
    }
    let mut path = raw_path
        .split('/')
        .map(|segment| normalize_component(segment, pchar, false))
        .collect::<Result<Vec<_>, _>>()?
        .join("/");
    if path.is_empty() && matches!(scheme.as_deref(), Some("http" | "https" | "ftp" | "tftp")) {
        path = "/".into();
    }
    out.push_str(&path);
    if let Some(q) = c.get(4) {
        let query = q
            .as_str()
            .split('&')
            .map(|pair| {
                normalize_component(
                    pair,
                    |b| b.is_ascii_alphanumeric() || b"-._~!$'()*+,=:@/?".contains(&b),
                    true,
                )
            })
            .collect::<Result<Vec<_>, _>>()?
            .join("&");
        if !query.is_empty() {
            out.push('?');
            out.push_str(&query);
        }
    }
    if let Some(f) = c.get(5) {
        let fragment = normalize_component(f.as_str(), |b| pchar(b) || b"/?".contains(&b), false)?;
        if !fragment.is_empty() {
            out.push('#');
            out.push_str(&fragment);
        }
    }
    Ok(Some(Normalized {
        host,
        path,
        url: out,
    }))
}

/// `fetch_response(url, headers:, raise_error_when_response_too_large:
/// false) rescue nil`: the body, None where it raised.
async fn fetch_or_nil<F: Fetch>(
    fetch: &mut F,
    url: &str,
    headers: &[(String, String)],
) -> Result<Option<String>, AppError> {
    let opts = FetchOpts {
        headers: headers.to_vec(),
        raise_error_when_response_too_large: false,
        ..Default::default()
    };
    Ok(match fetch.fetch_response(url, &opts).await? {
        Ok(body) => Some(body),
        Err(FetchError::Refused(u)) => return Err(u.into()),
        Err(_) => None,
    })
}

/// `Onebox::Helpers.fetch_html_doc(url, headers)`: the page's HTML (None
/// for an empty document, where the fetch failed).
pub async fn fetch_html_doc<F: Fetch>(
    fetch: &mut F,
    url: &str,
    headers: &[(String, String)],
) -> Result<Option<String>, AppError> {
    let response = fetch_or_nil(fetch, url, headers).await?;
    let Some(uri) = normalize(url)? else {
        return Err(Unsupported("onebox URLs Addressable can't parse").into());
    };
    // The canonical link, read from the parsed page (dropped before the
    // next fetch).
    let canonical = {
        let dom = parse(response.as_deref().unwrap_or(""));
        let root = root(&dom);
        let ignore_tag = select_first(&root, "meta[property=\"og:ignore_canonical\"]")
            .is_some_and(|m| attr(&m, "content").as_deref() == Some("true"));
        let host = uri.host.as_deref().unwrap_or("");
        let ignore_domain = IGNORE_CANONICAL.iter().any(|r| r.is_match(host));
        if ignore_tag || ignore_domain {
            None
        } else {
            // `//link[@rel="canonical"]/@href`: the first such link with
            // an href.
            select_first(&root, "link[rel=\"canonical\"][href]").and_then(|l| attr(&l, "href"))
        }
    };
    let Some(canonical) = canonical else {
        return Ok(response);
    };
    let Some(canonical_uri) = normalize(&canonical)? else {
        return Ok(response);
    };
    let same = format!(
        "{}{}",
        canonical_uri.host.as_deref().unwrap_or(""),
        canonical_uri.path
    ) == format!("{}{}", uri.host.as_deref().unwrap_or(""), uri.path);
    // A relative canonical link doesn't resolve.
    if same || canonical_uri.host.is_none() {
        return Ok(response);
    }
    let Some(resolved) = fetch.resolve(&canonical_uri.url).await? else {
        return Ok(response);
    };
    Ok(match fetch_or_nil(fetch, &resolved, headers).await? {
        Some(body) => Some(body),
        None => response,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_as_addressable() {
        let n = normalize("HTTPS://Example.COM:443?a=b|c#x")
            .unwrap()
            .unwrap();
        assert_eq!(n.url, "https://example.com/?a=b%7Cc#x");
        assert_eq!(n.host.as_deref(), Some("example.com"));
        assert_eq!(n.path, "/");
        let rel = normalize("/foo").unwrap().unwrap();
        assert!(rel.host.is_none());
        assert_eq!(rel.path, "/foo");
    }
}
