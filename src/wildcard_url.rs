//! Ruby's `URI.parse` for absolute URLs (RFC 3986), as far as
//! WildcardUrlChecker and the user API key redirects read it, and
//! WildcardUrlChecker itself.

use std::sync::LazyLock;

use regex::Regex;

/// A URL as `URI.parse` splits it.
#[derive(Debug, Clone, PartialEq)]
pub struct Uri {
    pub scheme: String,
    pub userinfo: Option<String>,
    pub host: Option<String>,
    /// The explicit port, else the scheme's default (http 80, https 443,
    /// ftp 21, ws 80, wss 443), else none.
    pub port: Option<u16>,
    pub explicit_port: bool,
    pub path: String,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

fn allowed(s: &str, extra: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'%' {
            if !(bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
            {
                return false;
            }
            i += 3;
            continue;
        }
        if !(c.is_ascii_alphanumeric()
            || b"-._~!$&'()*+,;=".contains(&c)
            || extra.as_bytes().contains(&c))
        {
            return false;
        }
        i += 1;
    }
    true
}

/// `URI.parse(url)`; None where it raises InvalidURIError. Relative
/// references parse too (no scheme or host).
pub fn parse(url: &str) -> Option<Uri> {
    static RFC3986: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"\A(?:([A-Za-z][A-Za-z0-9+\-.]*):)?(?://([^/?#]*))?([^?#]*)(?:\?([^#]*))?(?:#(.*))?\z",
        )
        .unwrap()
    });
    let c = RFC3986.captures(url)?;
    let scheme = c.get(1).map(|m| m.as_str().to_string());
    let authority = c.get(2).map(|m| m.as_str());
    let path = c.get(3).map_or("", |m| m.as_str());
    let query = c.get(4).map(|m| m.as_str().to_string());
    let fragment = c.get(5).map(|m| m.as_str().to_string());
    if !allowed(path, ":@/")
        || !query.as_deref().is_none_or(|q| allowed(q, ":@/?"))
        || !fragment.as_deref().is_none_or(|f| allowed(f, ":@/?"))
    {
        return None;
    }
    let (userinfo, host, port) = match authority {
        None => (None, None, None),
        Some(a) => {
            let (userinfo, hostport) = match a.rsplit_once('@') {
                Some((u, h)) => (Some(u.to_string()), h),
                None => (None, a),
            };
            if userinfo.as_deref().is_some_and(|u| !allowed(u, ":")) {
                return None;
            }
            let (host, port) = if hostport.starts_with('[') {
                let end = hostport.find(']')?;
                let (h, rest) = hostport.split_at(end + 1);
                (h, rest.strip_prefix(':'))
            } else {
                match hostport.rsplit_once(':') {
                    Some((h, p)) => (h, Some(p)),
                    None => (hostport, None),
                }
            };
            if !host.starts_with('[') && !allowed(host, "") {
                return None;
            }
            let port = match port {
                Some("") | None => None,
                Some(p) if p.bytes().all(|b| b.is_ascii_digit()) => Some(p.parse::<u16>().ok()?),
                Some(_) => return None,
            };
            (userinfo, Some(host.to_string()), port)
        }
    };
    let explicit_port = port.is_some();
    let port = port.or(
        match scheme.as_deref().map(str::to_ascii_lowercase).as_deref() {
            Some("http" | "ws") => Some(80),
            Some("https" | "wss") => Some(443),
            Some("ftp") => Some(21),
            _ => None,
        },
    );
    Some(Uri {
        scheme: scheme.unwrap_or_default(),
        userinfo,
        host,
        port,
        explicit_port,
        path: path.to_string(),
        query,
        fragment,
    })
}

/// `parse_valid_url`: a scheme and a host.
fn parse_valid(url: &str) -> Option<Uri> {
    parse(url).filter(|u| !u.scheme.is_empty() && u.host.as_deref().is_some_and(|h| !h.is_empty()))
}

fn wildcard_match(pattern: &str, value: &str, wildcard: &str) -> bool {
    let escaped = regex::escape(pattern).replace(r"\*", wildcard);
    Regex::new(&format!(r"(?i)\A{escaped}\z")).is_ok_and(|r| r.is_match(value))
}

fn hosts_match(host: &str, to_check: &str) -> bool {
    let host = host.to_lowercase();
    let to_check = to_check.to_lowercase();
    if let Some(suffix) = host.strip_prefix("*.")
        && host.matches('*').count() == 1
    {
        let dotted = format!(".{suffix}");
        static VALUE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"\A[A-Za-z0-9.-]+\z").unwrap());
        return to_check.ends_with(&dotted)
            && VALUE.is_match(to_check.strip_suffix(&dotted).unwrap_or_default());
    }
    wildcard_match(&host, &to_check, "[A-Za-z0-9.-]*")
}

fn path_query(u: &Uri) -> String {
    match u.query.as_deref() {
        Some(q) if !q.is_empty() => format!("{}?{q}", u.path),
        _ => u.path.clone(),
    }
}

/// `WildcardUrlChecker.check_url(url, url_to_check)`
pub fn check_url(url: &str, url_to_check: &str) -> bool {
    let Some(to_check) = parse_valid(url_to_check) else {
        return false;
    };
    if url == "*" {
        return true;
    }
    let Some(uri) = parse_valid(url) else {
        return false;
    };
    uri.scheme.eq_ignore_ascii_case(&to_check.scheme)
        && uri.port == to_check.port
        && uri.userinfo.clone().unwrap_or_default().to_lowercase()
            == to_check.userinfo.clone().unwrap_or_default().to_lowercase()
        && hosts_match(
            uri.host.as_deref().unwrap_or_default(),
            to_check.host.as_deref().unwrap_or_default(),
        )
        && wildcard_match(&path_query(&uri), &path_query(&to_check), ".*")
        && wildcard_match(
            uri.fragment.as_deref().unwrap_or_default(),
            to_check.fragment.as_deref().unwrap_or_default(),
            ".*",
        )
}

/// `uri.query = [uri.query, extra].compact.join("&"); uri.to_s`: the url
/// with `extra` appended to its query, its fragment kept after it.
pub fn append_query(url: &str, extra: &str) -> String {
    let (before_fragment, fragment) = match url.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (url, None),
    };
    let mut out = match before_fragment.split_once('?') {
        Some((base, query)) => format!("{base}?{query}&{extra}"),
        None => format!("{before_fragment}?{extra}"),
    };
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_like_ruby() {
        let u = parse("discourse://auth_redirect").unwrap();
        assert_eq!(u.host.as_deref(), Some("auth_redirect"));
        assert_eq!(u.port, None);
        let u = parse("https://app.example.com:8443/auth?x=1#f").unwrap();
        assert_eq!(u.port, Some(8443));
        assert_eq!(u.query.as_deref(), Some("x=1"));
        assert!(parse("https://exa mple.com/").is_none());
        assert_eq!(parse("https://a.example.com/").unwrap().port, Some(443));
    }

    #[test]
    fn checks_wildcards() {
        assert!(check_url(
            "https://api.discourse.org/api/auth_redirect",
            "https://api.discourse.org/api/auth_redirect"
        ));
        assert!(check_url(
            "discourse://auth_redirect",
            "discourse://auth_redirect"
        ));
        assert!(!check_url("discourse://auth_redirect", "https://evil.com/"));
        assert!(check_url(
            "https://*.example.com/*",
            "https://app.example.com/auth"
        ));
        assert!(!check_url(
            "https://*.example.com/*",
            "https://example.com/auth"
        ));
        assert!(check_url("*", "https://anything.example/"));
        assert!(!check_url("*", "not a url"));
    }

    #[test]
    fn appends_queries() {
        assert_eq!(append_query("https://a/b", "p=1"), "https://a/b?p=1");
        assert_eq!(
            append_query("https://a/b?x=2#f", "p=1"),
            "https://a/b?x=2&p=1#f"
        );
    }
}
