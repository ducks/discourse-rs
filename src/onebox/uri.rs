//! The URI helpers the onebox engines lean on, as Ruby's `URI` (RFC3986
//! parser, `URI::Generic#merge` and `#to_s`) and Addressable
//! (`Onebox::Helpers.uri_encode`) give them.

use std::sync::LazyLock;

use crate::final_destination::ruby_uri_valid;

/// A URI as `URI.parse` splits it.
#[derive(Debug, Clone, PartialEq)]
pub struct RubyUri {
    pub scheme: Option<String>,
    pub userinfo: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub path: String,
    pub opaque: Option<String>,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

static SPLIT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"\A(?:([A-Za-z][A-Za-z0-9+\-.]*):)?(?://([^/?#]*))?([^?#]*)(?:\?([^#]*))?(?:#(.*))?\z",
    )
    .unwrap()
});

fn default_port(scheme: Option<&str>) -> Option<u16> {
    match scheme? {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        "ftp" => Some(21),
        "ldap" => Some(389),
        "ldaps" => Some(636),
        _ => None,
    }
}

impl RubyUri {
    /// `URI.parse(s)`; None where it raises InvalidURIError.
    pub fn parse(s: &str) -> Option<RubyUri> {
        if !ruby_uri_valid(s) {
            return None;
        }
        let c = SPLIT.captures(s)?;
        let scheme = c.get(1).map(|m| m.as_str().to_ascii_lowercase());
        let mut uri = RubyUri {
            scheme,
            userinfo: None,
            host: None,
            port: None,
            path: String::new(),
            opaque: None,
            query: c.get(4).map(|m| m.as_str().to_string()),
            fragment: c.get(5).map(|m| m.as_str().to_string()),
        };
        let path = c.get(3).map_or("", |m| m.as_str());
        match c.get(2) {
            Some(authority) => {
                let authority = authority.as_str();
                let (userinfo, hostport) = match authority.rsplit_once('@') {
                    Some((u, h)) => (Some(u.to_string()), h),
                    None => (None, authority),
                };
                let (host, port) = if hostport.starts_with('[') {
                    match hostport.rfind(']') {
                        Some(i) => (&hostport[..=i], hostport[i + 1..].strip_prefix(':')),
                        None => (hostport, None),
                    }
                } else {
                    match hostport.rsplit_once(':') {
                        Some((h, p)) => (h, Some(p)),
                        None => (hostport, None),
                    }
                };
                uri.userinfo = userinfo;
                uri.host = Some(host.to_string());
                uri.port = match port.filter(|p| !p.is_empty()) {
                    Some(p) => Some(p.parse().ok()?),
                    None => None,
                };
                uri.path = path.to_string();
            }
            None => {
                if uri.scheme.is_some() && !path.starts_with('/') {
                    uri.opaque = Some(path.to_string());
                    uri.query = None;
                    // An opaque part keeps its query.
                    if let Some(q) = c.get(4) {
                        uri.opaque = Some(format!("{path}?{}", q.as_str()));
                    }
                } else {
                    uri.path = path.to_string();
                }
            }
        }
        if uri.port.is_none() {
            uri.port = default_port(uri.scheme.as_deref());
        }
        Some(uri)
    }

    pub fn absolute(&self) -> bool {
        self.scheme.is_some()
    }

    /// `uri.to_s`
    pub fn to_s(&self) -> String {
        let mut s = String::new();
        if let Some(scheme) = &self.scheme {
            s.push_str(scheme);
            s.push(':');
        }
        if let Some(opaque) = &self.opaque {
            s.push_str(opaque);
        } else {
            if self.host.is_some() || matches!(self.scheme.as_deref(), Some("file" | "postgres")) {
                s.push_str("//");
            }
            if let Some(u) = &self.userinfo {
                s.push_str(u);
                s.push('@');
            }
            if let Some(h) = &self.host {
                s.push_str(h);
            }
            if let Some(p) = self.port
                && Some(p) != default_port(self.scheme.as_deref())
            {
                s.push(':');
                s.push_str(&p.to_string());
            }
            if (self.host.is_some() || self.port.is_some())
                && !self.path.is_empty()
                && !self.path.starts_with('/')
            {
                s.push('/');
            }
            s.push_str(&self.path);
            if let Some(q) = &self.query {
                s.push('?');
                s.push_str(q);
            }
        }
        if let Some(f) = &self.fragment {
            s.push('#');
            s.push_str(f);
        }
        s
    }

    /// `uri.merge(rel)`; None where it raises.
    pub fn merge(&self, rel: &str) -> Option<RubyUri> {
        let rel = RubyUri::parse(rel)?;
        if rel.absolute() {
            return Some(rel);
        }
        if !self.absolute() {
            return None;
        }
        let mut base = self.clone();
        let authority = rel.host.is_some();
        if rel.path.is_empty() && !authority && rel.query.is_none() {
            if rel.fragment.is_some() {
                base.fragment = rel.fragment;
            }
            return Some(base);
        }
        base.query = None;
        base.fragment = None;
        if authority {
            base.userinfo = rel.userinfo;
            base.host = rel.host;
            base.port = rel.port;
            base.path = rel.path;
        } else if base.opaque.is_none() {
            base.path = merge_path(&base.path, &rel.path);
        }
        if rel.query.is_some() {
            base.query = rel.query;
        }
        if rel.fragment.is_some() {
            base.fragment = rel.fragment;
        }
        Some(base)
    }
}

/// `path.split("/", -1)`
fn split_path(path: &str) -> Vec<String> {
    if path.is_empty() {
        Vec::new()
    } else {
        path.split('/').map(str::to_string).collect()
    }
}

/// `URI::Generic#merge_path`
fn merge_path(base: &str, rel: &str) -> String {
    let mut base_path = split_path(base);
    let mut rel_path = split_path(rel);
    if base_path.last().is_some_and(|l| l == "..") {
        base_path.push(String::new());
    }
    while let Some(i) = base_path.iter().position(|s| s == "..") {
        // slice!(i - 1, 2); at 0 Ruby's -1 is the last element.
        if i == 0 {
            let last = base_path.len() - 1;
            base_path.truncate(last);
        } else {
            base_path.drain(i - 1..(i + 1).min(base_path.len()));
        }
    }
    if rel_path.first().is_some_and(String::is_empty) {
        base_path.clear();
        rel_path.remove(0);
    }
    if rel_path.last().is_some_and(|l| l == "." || l == "..") {
        rel_path.push(String::new());
    }
    rel_path.retain(|s| s != ".");
    let mut tmp: Vec<String> = Vec::new();
    for x in rel_path {
        if x == ".." && !(tmp.is_empty() || tmp.last().is_some_and(|l| l == "..")) {
            tmp.pop();
        } else {
            tmp.push(x);
        }
    }
    let mut add_trailer_slash = !tmp.is_empty();
    if base_path.is_empty() {
        base_path = vec![String::new()];
    } else if add_trailer_slash {
        base_path.pop();
    }
    let mut rest = tmp.into_iter();
    while let Some(x) = rest.next() {
        if x == ".." {
            if base_path.len() > 1 {
                base_path.pop();
            }
        } else {
            base_path.push(x);
            base_path.extend(rest.by_ref());
            add_trailer_slash = false;
            break;
        }
    }
    if add_trailer_slash {
        base_path.push(String::new());
    }
    base_path.join("/")
}

/// `Onebox::Helpers.get_absolute_image_url(src, url)`:
/// `URI.parse(url).merge(src).to_s`, the source as it is where that
/// raises.
pub fn absolute_url(src: Option<&str>, url: &str) -> Option<String> {
    let src = src?;
    Some(
        RubyUri::parse(url)
            .and_then(|u| u.merge(src))
            .map_or_else(|| src.to_string(), |u| u.to_s()),
    )
}

/// `Addressable::URI.encode_component(component, class)`: every byte
/// outside the class as `%XX`.
pub fn encode_component(component: &str, allowed: impl Fn(u8) -> bool) -> String {
    let mut out = String::new();
    for b in component.bytes() {
        if allowed(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~".contains(&b)
}

fn sub_delim(b: u8) -> bool {
    b"!$&'()*+,;=".contains(&b)
}

static ADDRESSABLE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\A(?:([^:/?#]+):)?(?://([^/?#]*))?([^?#]*)(?:\?([^#]*))?(?:#(.*))?\z")
        .unwrap()
});

/// `Onebox::Helpers.uri_encode(url)`: each part of the URL percent-encoded
/// with its own character class; None for a URL Addressable can't split.
pub fn uri_encode(url: &str) -> Option<String> {
    if url.contains(['\n', '\r']) {
        return None;
    }
    let c = ADDRESSABLE.captures(url)?;
    let mut out = String::new();
    if let Some(scheme) = c.get(1) {
        out.push_str(&encode_component(scheme.as_str(), |b| {
            b.is_ascii_alphanumeric() || b"-+.".contains(&b)
        }));
        out.push(':');
    }
    if let Some(authority) = c.get(2) {
        out.push_str("//");
        out.push_str(&encode_component(authority.as_str(), |b| {
            unreserved(b) || sub_delim(b) || b":@[]".contains(&b)
        }));
    }
    out.push_str(&encode_component(
        c.get(3).map_or("", |m| m.as_str()),
        |b| unreserved(b) || sub_delim(b) || b":@/%".contains(&b),
    ));
    if let Some(query) = c.get(4) {
        out.push('?');
        out.push_str(&encode_component(query.as_str(), |b| {
            b.is_ascii_alphanumeric() || b"-._~$&*,=:@?%+".contains(&b)
        }));
    }
    if let Some(fragment) = c.get(5) {
        out.push('#');
        out.push_str(&encode_component(fragment.as_str(), |b| {
            b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:/?%".contains(&b)
        }));
    }
    Some(out)
}

/// `CGI.unescape`: `+` as a space, `%XX` decoded; None where the result
/// isn't UTF-8.
pub fn cgi_unescape(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit) =>
            {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).ok()?);
                i += 3;
                continue;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merge(base: &str, rel: &str) -> String {
        RubyUri::parse(base).unwrap().merge(rel).unwrap().to_s()
    }

    #[test]
    fn merges_as_ruby() {
        assert_eq!(
            merge("https://Ex.com:443/a/b/c?x#y", "../d?z"),
            "https://Ex.com/a/d?z"
        );
        assert_eq!(
            merge("HTTPS://Ex.com/a/", "//cdn.x.com/f.ico"),
            "https://cdn.x.com/f.ico"
        );
        assert_eq!(
            merge("https://ex.com/a/b", "favicon.ico"),
            "https://ex.com/a/favicon.ico"
        );
        assert_eq!(
            merge("https://ex.com", "favicon.ico"),
            "https://ex.com/favicon.ico"
        );
        assert_eq!(merge("https://ex.com/a?b", "?c"), "https://ex.com/a?c");
        assert_eq!(merge("https://ex.com/a?b", "#c"), "https://ex.com/a?b#c");
        assert_eq!(
            merge("https://ex.com/a/b/", "./../../../x"),
            "https://ex.com/x"
        );
        assert!(
            RubyUri::parse("https://ex.com/")
                .unwrap()
                .merge("fav icon.ico")
                .is_none()
        );
        assert_eq!(
            absolute_url(Some("http://other/x.png"), "https://ex.com/").unwrap(),
            "http://other/x.png"
        );
    }

    #[test]
    fn encodes_as_addressable() {
        assert_eq!(
            uri_encode("https://ex.com/a b/é?q=a/b|c[d]&x=é#frag@x/y#z").unwrap(),
            "https://ex.com/a%20b/%C3%A9?q=a%2Fb%7Cc%5Bd%5D&x=%C3%A9#frag%40x/y%23z"
        );
        assert_eq!(uri_encode("https://ex.com").unwrap(), "https://ex.com");
        assert_eq!(uri_encode("https://ex.com?").unwrap(), "https://ex.com?");
    }
}
