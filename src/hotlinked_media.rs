//! lib/hotlinked_media.rb and `PostHotlinkedMedia.normalize_src`: the
//! media in cooked HTML that may be hotlinked from another site, the src
//! each is downloaded from, and the normalized url a post's
//! post_hotlinked_media rows are keyed by (Addressable's heuristic parse
//! and normalization).

use std::sync::LazyLock;

use markup5ever_rcdom::Handle;
use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::Unsupported;
use crate::pretty_text::cleanup::{all_elements, attr, element_name, has_class, parse};
use crate::pretty_text::cooked_post_processor::ancestors;

/// `PrettyText::BLOCKED_HOTLINKED_SRC_ATTR`
pub const BLOCKED_HOTLINKED_SRC_ATTR: &str = "data-blocked-hotlinked-src";

/// `HotlinkedMedia.extract_candidates(html)`: `img[src]`, any element with
/// a blocked hotlinked src and `a.lightbox[href]`, less avatars and the
/// images inside a lightbox (whose link is the candidate), in document
/// order. Each is given as its `download_src_for` source attribute.
pub fn extract_candidates(html: &str) -> Vec<String> {
    let dom = parse(html);
    let mut out = Vec::new();
    for e in all_elements(&dom) {
        let name = element_name(&e);
        let img = name == Some("img");
        let candidate = (img && attr(&e, "src").is_some())
            || attr(&e, BLOCKED_HOTLINKED_SRC_ATTR).is_some()
            || (name == Some("a") && has_class(&e, "lightbox") && attr(&e, "href").is_some());
        if !candidate {
            continue;
        }
        if img && has_class(&e, "avatar") {
            continue;
        }
        if img && attr(&e, "src").is_some() && in_lightbox(&e) {
            continue;
        }
        // `node["src"] || node[BLOCKED_HOTLINKED_SRC_ATTR] || node["href"]`
        let src = attr(&e, "src")
            .or_else(|| attr(&e, BLOCKED_HOTLINKED_SRC_ATTR))
            .or_else(|| attr(&e, "href"))
            .unwrap_or_default();
        out.push(src);
    }
    out
}

/// `.lightbox img`: an ancestor with the lightbox class.
fn in_lightbox(node: &Handle) -> bool {
    ancestors(node).iter().any(|a| has_class(a, "lightbox"))
}

/// `HotlinkedMedia.download_src_for(node)` from the node's source
/// attribute: normalized keeping its scheme, protocol-relative ones given
/// the site's.
pub fn download_src_for(original_src: &str, force_https: bool) -> Result<String, Unsupported> {
    if crate::ruby::is_blank(original_src) {
        return Ok(original_src.to_string());
    }
    if original_src.starts_with("//") {
        let scheme = if force_https { "https" } else { "http" };
        return Ok(format!("{scheme}:{original_src}"));
    }
    normalize_src(original_src, false)
}

/// `PostHotlinkedMedia.normalize_src(src, reset_scheme:)`:
/// `Addressable::URI.heuristic_parse(src).normalize!`, its scheme dropped
/// unless asked to keep it; the src as given where Addressable raises.
pub fn normalize_src(src: &str, reset_scheme: bool) -> Result<String, Unsupported> {
    let Some(mut uri) = heuristic_parse(src)? else {
        return Ok(src.to_string());
    };
    let Some(normalized) = uri.normalize()? else {
        return Ok(src.to_string());
    };
    uri = normalized;
    if reset_scheme {
        uri.scheme = None;
        if !uri.valid() {
            return Ok(src.to_string());
        }
    }
    Ok(uri.to_s())
}

/// An `Addressable::URI` as its components.
#[derive(Debug, Clone, Default)]
struct AddrUri {
    scheme: Option<String>,
    user: Option<String>,
    password: Option<String>,
    host: Option<String>,
    port: Option<u32>,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

/// `Addressable::URI::URIREGEX`
static URIREGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A(([^:/?#]+):)?(//([^/?#]*))?([^?#]*)(\?([^#]*))?(#(.*))?\z").unwrap()
});
static HTTP_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\Ahttp:/+").unwrap());
static HTTPS_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\Ahttps:/+").unwrap());
static IP_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}").unwrap());
static NUMERIC_HOST_PORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A\d+\..*:\d+\z").unwrap());
static HOST_LIKE_SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A[^/?#.]+\.[^/?#]+\z").unwrap());
static MAIL_AT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b@\b").unwrap());
static HOST_IN_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A([^/]+\.[^/]*)").unwrap());
/// `NORMPATH`: a relative path with a colon in its first segment.
static NORMPATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A[^/:]*:").unwrap());

/// Ruby's `String#strip`: whitespace and NULs from both ends.
fn ruby_strip(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_ascii_whitespace() || c == '\0' || c == '\x0b')
}

/// `Addressable::URI.heuristic_parse(uri)` with the default http hint;
/// None where it raises InvalidURIError.
fn heuristic_parse(src: &str) -> Result<Option<AddrUri>, Unsupported> {
    if src.contains(['\n', '\r']) {
        return Err(Unsupported("hotlinked media urls spanning lines"));
    }
    let lower = src.to_ascii_lowercase();
    if lower.starts_with("feed:") || lower.starts_with("file:") {
        return Err(Unsupported("feed and file urls as hotlinked media"));
    }
    let mut uri = ruby_strip(src).to_string();
    if HTTP_PREFIX.is_match(&uri) {
        uri = HTTP_PREFIX.replace(&uri, "http://").into_owned();
    } else if HTTPS_PREFIX.is_match(&uri) {
        uri = HTTPS_PREFIX.replace(&uri, "https://").into_owned();
    } else if IP_PREFIX.is_match(&uri) || NUMERIC_HOST_PORT.is_match(&uri) {
        uri = format!("http://{uri}");
    }
    // The authority's backslashes as slashes and spaces encoded.
    if let Some(m) = URIREGEX.captures(&uri).and_then(|c| c.get(4))
        && !m.as_str().is_empty()
    {
        let fixed = m.as_str().replace('\\', "/").replace(' ', "%20");
        uri = format!("{}{fixed}{}", &uri[..m.start()], &uri[m.end()..]);
    }
    let Some(mut parsed) = parse_uri(&uri) else {
        return Ok(None);
    };
    if parsed
        .scheme
        .as_deref()
        .is_some_and(|s| HOST_LIKE_SCHEME.is_match(s))
    {
        let Some(again) = parse_uri(&format!("http://{uri}")) else {
            return Ok(None);
        };
        parsed = again;
    }
    if parsed.path.contains('.') {
        if MAIL_AT.is_match(&parsed.path) {
            if parsed.scheme.is_none() {
                parsed.scheme = Some("mailto".into());
            }
        } else if let Some(host) = HOST_IN_PATH
            .captures(&parsed.path)
            .map(|c| c[1].to_string())
        {
            parsed.path = parsed.path[host.len()..].to_string();
            parsed.host = Some(host);
            if parsed.scheme.is_none() {
                parsed.scheme = Some("http".into());
            }
        }
        if !parsed.valid() {
            return Ok(None);
        }
    }
    Ok(Some(parsed))
}

/// `Addressable::URI.parse`; None where it raises.
fn parse_uri(uri: &str) -> Option<AddrUri> {
    let c = URIREGEX.captures(uri)?;
    let mut parsed = AddrUri {
        scheme: c.get(2).map(|m| m.as_str().to_string()),
        path: c.get(5).map_or("", |m| m.as_str()).to_string(),
        query: c.get(7).map(|m| m.as_str().to_string()),
        fragment: c.get(9).map(|m| m.as_str().to_string()),
        ..Default::default()
    };
    if let Some(authority) = c.get(4).map(|m| m.as_str()) {
        parsed.set_authority(authority)?;
    }
    parsed.valid().then_some(parsed)
}

/// `userinfo@` at the start of an authority: up to its last `@` outside
/// brackets.
static USERINFO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A([^\[\]]*)@").unwrap());
static PORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r":([^:@\[\]]*?)\z").unwrap());
static INVALID_HOST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"[<>{}/\\?#@"\s]"#).unwrap());
static BRACKETED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A\[(.*)\]\z").unwrap());
static IP_LITERAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A[a-zA-Z0-9\-._~!$&'()*+,;=:]*\z").unwrap());

impl AddrUri {
    /// `authority=`: None where the port is not a number.
    fn set_authority(&mut self, authority: &str) -> Option<()> {
        let userinfo = USERINFO.captures(authority).map(|c| c[1].to_string());
        if let Some(info) = &userinfo {
            let info = ruby_strip(info);
            self.user = Some(info.split(':').next().unwrap_or("").to_string());
            self.password = info.split_once(':').map(|(_, p)| p.to_string());
        }
        let rest = USERINFO.replace(authority, "");
        let host = PORT.replace(&rest, "").into_owned();
        let port = PORT.captures(&rest).map(|c| c[1].to_string());
        self.host = Some(host);
        self.port = match port.filter(|p| !p.is_empty()) {
            Some(p) => {
                let p = unencode(&p, "");
                if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let n: u32 = p.parse().ok()?;
                (n != 0).then_some(n)
            }
            None => None,
        };
        Some(())
    }

    fn has_authority(&self) -> bool {
        self.host.is_some()
    }

    /// `validate`: whether Addressable takes the URI.
    fn valid(&self) -> bool {
        if let Some(scheme) = &self.scheme
            && default_port(&scheme.to_ascii_lowercase()).is_some()
            && self.host.as_deref().unwrap_or("").is_empty()
            && self.path.is_empty()
        {
            return false;
        }
        if self.host.is_none()
            && (self.port.is_some() || self.user.is_some() || self.password.is_some())
        {
            return false;
        }
        if !self.path.is_empty() && !self.path.starts_with('/') && self.has_authority() {
            return false;
        }
        if self.path.starts_with("//") && !self.has_authority() {
            return false;
        }
        if let Some(host) = &self.host {
            if INVALID_HOST.is_match(host) {
                return false;
            }
            if let Some(c) = BRACKETED.captures(host)
                && !IP_LITERAL.is_match(&c[1])
            {
                return false;
            }
        }
        true
    }

    /// `normalize`: None where the normalized URI is invalid.
    fn normalize(&self) -> Result<Option<AddrUri>, Unsupported> {
        let scheme = self
            .scheme
            .as_deref()
            .map(|s| normalize_component(&ruby_strip(s).to_ascii_lowercase(), scheme_char, ""));
        let https = scheme.as_deref().is_some_and(|s| s.contains("http"));
        let (user, password) = match (&self.user, &self.password) {
            (None, None) => (None, None),
            (user, password) => {
                let password = password.as_deref().and_then(|p| {
                    let blank_user = user.as_deref().is_none_or(|u| ruby_strip(u).is_empty());
                    if https && ruby_strip(p).is_empty() && blank_user {
                        None
                    } else {
                        Some(normalize_component(ruby_strip(p), unreserved, ""))
                    }
                });
                let user = user.as_deref().and_then(|u| {
                    let blank_password = self
                        .password
                        .as_deref()
                        .is_none_or(|p| ruby_strip(p).is_empty());
                    if https && ruby_strip(u).is_empty() && blank_password {
                        None
                    } else {
                        Some(normalize_component(ruby_strip(u), unreserved, ""))
                    }
                });
                (user, password)
            }
        };
        let host = match &self.host {
            None => None,
            Some(h) if ruby_strip(h).is_empty() => Some(String::new()),
            Some(h) => {
                let decoded = unencode(&ruby_strip(h).to_lowercase(), "");
                let mut ascii = if decoded.is_ascii() {
                    decoded
                } else {
                    match url::Host::parse(&decoded) {
                        Ok(url::Host::Domain(d)) => d,
                        _ => return Err(Unsupported("hotlinked media hosts IDNA refuses")),
                    }
                };
                let b = ascii.as_bytes();
                if b.len() >= 2 && b[b.len() - 1] == b'.' && b[b.len() - 2] != b'.' {
                    ascii.pop();
                }
                Some(normalize_component(&ascii, host_char, ""))
            }
        };
        let port = self
            .port
            .filter(|p| scheme.as_deref().and_then(default_port) != Some(*p));
        let mut path = self.path.clone();
        if self.scheme.is_none() && NORMPATH.is_match(&path) {
            path = path.replacen(':', "%2F", 1);
        }
        let mut path = ruby_strip(&path)
            .split('/')
            .map(|segment| normalize_component(segment, pchar, ""))
            .collect::<Vec<_>>()
            .join("/");
        path = normalize_path(&path);
        if path.is_empty() && matches!(scheme.as_deref(), Some("http" | "https" | "ftp" | "tftp")) {
            path = "/".into();
        }
        let query = self.query.as_deref().and_then(|q| {
            let q = q
                .split('&')
                .map(normalize_query_pair)
                .collect::<Vec<_>>()
                .join("&");
            (!q.is_empty()).then_some(q)
        });
        let fragment = self.fragment.as_deref().and_then(|f| {
            let f = normalize_component(f, fragment_char, "");
            (!f.is_empty()).then_some(f)
        });
        let normalized = AddrUri {
            scheme,
            user,
            password,
            host,
            port,
            path,
            query,
            fragment,
        };
        Ok(normalized.valid().then_some(normalized))
    }

    /// `to_s`
    fn to_s(&self) -> String {
        let mut s = String::new();
        if let Some(scheme) = &self.scheme {
            s.push_str(scheme);
            s.push(':');
        }
        if let Some(host) = &self.host {
            s.push_str("//");
            match (&self.user, &self.password) {
                (Some(u), Some(p)) => s.push_str(&format!("{u}:{p}@")),
                (Some(u), None) => s.push_str(&format!("{u}@")),
                (None, _) => {}
            }
            s.push_str(host);
            if let Some(port) = self.port {
                s.push_str(&format!(":{port}"));
            }
        }
        s.push_str(&self.path);
        if let Some(q) = &self.query {
            s.push('?');
            s.push_str(q);
        }
        if let Some(f) = &self.fragment {
            s.push('#');
            s.push_str(f);
        }
        s
    }
}

/// `URI.port_mapping`
fn default_port(scheme: &str) -> Option<u32> {
    Some(match scheme {
        "http" => 80,
        "https" => 443,
        "ftp" => 21,
        "tftp" => 69,
        "sftp" | "ssh" | "svn+ssh" => 22,
        "telnet" => 23,
        "nntp" => 119,
        "gopher" => 70,
        "wais" => 210,
        "ldap" => 389,
        "prospero" => 1525,
        _ => return None,
    })
}

fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~".contains(&b)
}

fn sub_delim(b: u8) -> bool {
    b"!$&'()*+,;=".contains(&b)
}

fn pchar(b: u8) -> bool {
    unreserved(b) || sub_delim(b) || b":@".contains(&b)
}

fn scheme_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-+.".contains(&b)
}

fn host_char(b: u8) -> bool {
    unreserved(b) || sub_delim(b) || b"[:]".contains(&b)
}

fn fragment_char(b: u8) -> bool {
    pchar(b) || b"/?".contains(&b)
}

fn reserved_and_unreserved(b: u8) -> bool {
    unreserved(b) || sub_delim(b) || b":/?#[]@".contains(&b)
}

/// `Addressable::URI.unencode(s, String, leave_encoded)`, as bytes.
fn unencode_bytes(s: &str, leave_encoded: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            let b = u8::from_str_radix(&s[i + 1..i + 3], 16).expect("two hex digits");
            if leave_encoded.as_bytes().contains(&b) {
                out.extend_from_slice(&bytes[i..i + 3]);
            } else {
                out.push(b);
            }
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn unencode(s: &str, leave_encoded: &str) -> String {
    String::from_utf8_lossy(&unencode_bytes(s, leave_encoded)).into_owned()
}

fn encode_bytes(bytes: &[u8], keep: impl Fn(u8) -> bool) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if keep(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `Addressable::URI.normalize_component(component, class, leave_encoded)`:
/// decoded, NFC-normalized and encoded again outside the class (outside
/// RESERVED_AND_UNRESERVED for bytes that are not UTF-8).
fn normalize_component(component: &str, class: fn(u8) -> bool, leave_encoded: &str) -> String {
    let unencoded = unencode_bytes(component, leave_encoded);
    match String::from_utf8(unencoded) {
        Ok(s) => {
            let nfc: String = s.nfc().collect();
            encode_bytes(nfc.as_bytes(), class)
        }
        Err(e) => encode_bytes(e.as_bytes(), reserved_and_unreserved),
    }
}

/// A query pair under `NormalizeCharacterClasses::QUERY`, `%2B` left
/// encoded (and upcased).
fn normalize_query_pair(pair: &str) -> String {
    let unencoded = unencode_bytes(pair, "+");
    let keep = |b: u8| b.is_ascii_alphanumeric() || b"-._~!$'()*+,=:@/?".contains(&b);
    let encode = |bytes: &[u8]| {
        let mut out = String::new();
        for (i, &b) in bytes.iter().enumerate() {
            let encoded_plus = b == b'%'
                && bytes.get(i + 1) == Some(&b'2')
                && matches!(bytes.get(i + 2), Some(b'B' | b'b'));
            if keep(b) || encoded_plus {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out.replace("%2b", "%2B")
    };
    match String::from_utf8(unencoded) {
        Ok(s) => {
            let nfc: String = s.nfc().collect();
            encode(nfc.as_bytes())
        }
        Err(e) => encode_bytes(e.as_bytes(), reserved_and_unreserved),
    }
}

static RULE_2A: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/\./|/\.\z").unwrap());
static RULE_2B_2C: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/([^/]*)/\.\./|/([^/]*)/\.\.\z").unwrap());
static RULE_2D: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A\.\.?/?").unwrap());
static RULE_PREFIXED_PARENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A/\.\.?/|\A(/\.\.?)+/?\z").unwrap());

/// `gsub!`: the replaced string, or None when nothing matched.
fn gsub(re: &Regex, s: &str, with: &str) -> Option<String> {
    re.is_match(s).then(|| re.replace_all(s, with).into_owned())
}

/// `Addressable::URI.normalize_path`: dot segments removed (RFC 3986
/// 5.2.4, and its non-standard prefixed parents), one rule a round.
fn normalize_path(path: &str) -> String {
    let mut path = path.to_string();
    loop {
        if let Some(p) = gsub(&RULE_2A, &path, "/") {
            path = p;
            continue;
        }
        if let Some(c) = RULE_2B_2C.captures(&path) {
            let parent = c.get(1).map(|m| m.as_str().to_string());
            let current = c.get(2).map(|m| m.as_str().to_string());
            let dotted = |s: &Option<String>| matches!(s.as_deref(), Some("." | ".."));
            if !dotted(&parent) || !dotted(&current) {
                let re = Regex::new(&format!(
                    r"/{}/\.\./|(/{}/\.\.\z)",
                    regex::escape(parent.as_deref().unwrap_or("")),
                    regex::escape(current.as_deref().unwrap_or(""))
                ))
                .expect("escaped segments");
                if let Some(p) = gsub(&re, &path, "/") {
                    path = p;
                    continue;
                }
            }
        }
        if let Some(p) = gsub(&RULE_2D, &path, "") {
            path = p;
            continue;
        }
        if let Some(p) = gsub(&RULE_PREFIXED_PARENT, &path, "/") {
            path = p;
            continue;
        }
        return path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Addressable 2.9.0's answers (`normalize_src(src)` and with
    /// `reset_scheme: false`), from the reference.
    #[test]
    fn normalizes_as_addressable() {
        let cases = [
            (
                "http://test.localhost",
                "//test.localhost/",
                "http://test.localhost/",
            ),
            (
                "https://test.localhost/",
                "//test.localhost/",
                "https://test.localhost/",
            ),
            ("//test.localhost", "//test.localhost", "//test.localhost"),
            (
                "/uploads/default/original/1X/abc.png",
                "/uploads/default/original/1X/abc.png",
                "/uploads/default/original/1X/abc.png",
            ),
            (
                "HTTP://Example.COM:80/a b/%7euser/%2F?q=a b&x=%2B+#frag ment",
                "//example.com/a%20b/~user/%2F?q=a%20b&x=%2B+#frag%20ment",
                "http://example.com/a%20b/~user/%2F?q=a%20b&x=%2B+#frag%20ment",
            ),
            (
                "https://example.com:443/x.png",
                "//example.com/x.png",
                "https://example.com/x.png",
            ),
            (
                "https://example.com:8080/x.png",
                "//example.com:8080/x.png",
                "https://example.com:8080/x.png",
            ),
            (
                "http://example.com/a/../b/./c.png",
                "//example.com/b/c.png",
                "http://example.com/b/c.png",
            ),
            (
                "example.com/x.png",
                "//example.com/x.png",
                "http://example.com/x.png",
            ),
            ("images/foo.png", "images/foo.png", "images/foo.png"),
            ("foo.png/x", "//foo.png/x", "http://foo.png/x"),
            (
                "http://ex.com/ü.png",
                "//ex.com/%C3%BC.png",
                "http://ex.com/%C3%BC.png",
            ),
            (
                "http://Bücher.example/x.png",
                "//xn--bcher-kva.example/x.png",
                "http://xn--bcher-kva.example/x.png",
            ),
            (
                "http://ex.com/a%2fb",
                "//ex.com/a%2Fb",
                "http://ex.com/a%2Fb",
            ),
            ("http://ex.com/%41", "//ex.com/A", "http://ex.com/A"),
            (
                "http://user:p@ss@ex.com/",
                "//user:p%40ss@ex.com/",
                "http://user:p%40ss@ex.com/",
            ),
            ("http://ex.com.", "//ex.com/", "http://ex.com/"),
            (
                "http://ex.com/x?b=1&a=2",
                "//ex.com/x?b=1&a=2",
                "http://ex.com/x?b=1&a=2",
            ),
            ("", "", ""),
            (
                "data:image/png;base64,AAA",
                "image/png;base64,AAA",
                "data:image/png;base64,AAA",
            ),
            (
                "http://ex.com/x[1].png",
                "//ex.com/x%5B1%5D.png",
                "http://ex.com/x%5B1%5D.png",
            ),
            (
                "http://ex.com/x|y.png",
                "//ex.com/x%7Cy.png",
                "http://ex.com/x%7Cy.png",
            ),
            (
                "https:/ex.com/a.png",
                "//ex.com/a.png",
                "https://ex.com/a.png",
            ),
            ("http://ex.com?x", "//ex.com/?x", "http://ex.com/?x"),
            ("http://ex.com#", "//ex.com/", "http://ex.com/"),
            ("/a:b", "/a:b", "/a:b"),
            (
                "http://ex.com/a;b=c",
                "//ex.com/a;b=c",
                "http://ex.com/a;b=c",
            ),
            ("mailto:a@b.com", "a@b.com", "mailto:a@b.com"),
            (
                "http://ex.com/%e2%9c%93",
                "//ex.com/%E2%9C%93",
                "http://ex.com/%E2%9C%93",
            ),
        ];
        for (src, reset, kept) in cases {
            assert_eq!(normalize_src(src, true).unwrap(), reset, "{src}");
            assert_eq!(
                normalize_src(src, false).unwrap(),
                kept,
                "{src} keeping its scheme"
            );
        }
    }

    #[test]
    fn extracts_candidates_as_rails() {
        let html = r#"<p><img src="/a.png"><img class="avatar" src="/b.png">
            <a class="lightbox" href="/full.png"><img src="/thumb.png"></a>
            <img data-blocked-hotlinked-src="http://x.test/c.png">
            <video><source data-blocked-hotlinked-src="http://x.test/v.mp4"></video>
            <img alt="none"></p>"#;
        assert_eq!(
            extract_candidates(html),
            [
                "/a.png",
                "/full.png",
                "http://x.test/c.png",
                "http://x.test/v.mp4"
            ]
        );
    }
}
