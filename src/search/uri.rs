//! What search needs of Ruby's URI (3.4, URI::RFC3986_Parser):
//! `prepare_data`'s rewrite of a URL in the term without its query string,
//! and `Discourse.route_for`'s topic for search_for_id.

/// A parsed URI reference.
#[derive(Debug, PartialEq, Eq)]
struct Uri<'a> {
    scheme: Option<&'a str>,
    userinfo: Option<&'a str>,
    host: Option<&'a str>,
    port: Option<&'a str>,
    path: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}

fn pct_ok(s: &str, allowed: impl Fn(u8) -> bool) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if i + 2 >= b.len() || !b[i + 1].is_ascii_hexdigit() || !b[i + 2].is_ascii_hexdigit() {
                return false;
            }
            i += 3;
            continue;
        }
        if !allowed(b[i]) {
            return false;
        }
        i += 1;
    }
    true
}

/// `[!$&-.0-;=@-Z_a-z~]`: pchar without percent-encoding.
fn pchar(c: u8) -> bool {
    matches!(c, b'!' | b'$' | b'&'..=b'.' | b'0'..=b';' | b'=' | b'@'..=b'Z' | b'_' | b'a'..=b'z' | b'~')
}

fn reg_name_char(c: u8) -> bool {
    matches!(c, b'!' | b'$' | b'&'..=b'.' | b'0'..=b'9' | b';' | b'=' | b'A'..=b'Z' | b'_' | b'a'..=b'z' | b'~')
}

fn userinfo_char(c: u8) -> bool {
    matches!(c, b'!' | b'$' | b'&'..=b'.' | b'0'..=b';' | b'=' | b'A'..=b'Z' | b'_' | b'a'..=b'z' | b'~')
}

fn path_ok(path: &str) -> bool {
    path.split('/').all(|seg| pct_ok(seg, pchar))
}

fn host_ok(host: &str) -> bool {
    if let Some(inner) = host.strip_prefix('[') {
        let Some(inner) = inner.strip_suffix(']') else {
            return false;
        };
        if inner.parse::<std::net::Ipv6Addr>().is_ok() {
            return true;
        }
        // IPvFuture: v<hex>.<chars>
        let Some(rest) = inner.strip_prefix(['v', 'V']) else {
            return false;
        };
        let Some((hex, tail)) = rest.split_once('.') else {
            return false;
        };
        return !hex.is_empty()
            && hex.bytes().all(|c| c.is_ascii_hexdigit())
            && !tail.is_empty()
            && tail.bytes().all(userinfo_char);
    }
    pct_ok(host, reg_name_char)
}

/// `URI.parse`: an absolute URI or a relative reference, or `None` where
/// Ruby raises InvalidURIError.
fn parse(s: &str) -> Option<Uri<'_>> {
    if !s.is_ascii() {
        return None;
    }
    let (rest, fragment) = match s.split_once('#') {
        Some((r, f)) => (r, Some(f)),
        None => (s, None),
    };
    if let Some(f) = fragment
        && !pct_ok(f, |c| pchar(c) || c == b'/' || c == b'?')
    {
        return None;
    }
    let (rest, query) = match rest.split_once('?') {
        Some((r, q)) => (r, Some(q)),
        None => (rest, None),
    };
    // The query takes anything but `#`, percent signs only as escapes.
    if let Some(q) = query
        && !pct_ok(q, |c| c != b'%' && c != b'#')
    {
        return None;
    }
    // A scheme: a letter, then letters, digits, + - ., before the first `:`
    // that comes before any `/`.
    let mut scheme = None;
    let mut hier = rest;
    if let Some(colon) = rest.find(':')
        && !rest[..colon].contains('/')
    {
        let candidate = &rest[..colon];
        let b = candidate.as_bytes();
        if !b.is_empty()
            && b[0].is_ascii_alphabetic()
            && b.iter()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
        {
            scheme = Some(candidate);
            hier = &rest[colon + 1..];
        } else {
            // path-noscheme: no `:` in the first segment.
            return None;
        }
    }
    let mut uri = Uri {
        scheme,
        userinfo: None,
        host: None,
        port: None,
        path: hier,
        query,
        fragment,
    };
    if let Some(after) = hier.strip_prefix("//") {
        let end = after.find('/').unwrap_or(after.len());
        let authority = &after[..end];
        uri.path = &after[end..];
        let host_port = match authority.split_once('@') {
            Some((userinfo, hp)) => {
                if !pct_ok(userinfo, userinfo_char) {
                    return None;
                }
                uri.userinfo = Some(userinfo);
                hp
            }
            None => authority,
        };
        let (host, port) = if host_port.starts_with('[') {
            let close = host_port.find(']')?;
            let (h, p) = host_port.split_at(close + 1);
            match p.strip_prefix(':') {
                Some(port) => (h, Some(port)),
                None if p.is_empty() => (h, None),
                None => return None,
            }
        } else {
            match host_port.split_once(':') {
                Some((h, p)) => (h, Some(p)),
                None => (host_port, None),
            }
        };
        if !host_ok(host) {
            return None;
        }
        if let Some(p) = port
            && !p.bytes().all(|c| c.is_ascii_digit())
        {
            return None;
        }
        uri.host = Some(host);
        uri.port = port.filter(|p| !p.is_empty());
    }
    if !path_ok(uri.path) {
        return None;
    }
    Some(uri)
}

/// `URI::Generic#to_s` without the query.
fn to_s_without_query(uri: &Uri<'_>) -> String {
    let mut out = String::new();
    if let Some(scheme) = uri.scheme {
        out.push_str(scheme);
        out.push(':');
    }
    if let Some(host) = uri.host {
        out.push_str("//");
        if let Some(userinfo) = uri.userinfo {
            out.push_str(userinfo);
            out.push('@');
        }
        out.push_str(host);
        if let Some(port) = uri.port {
            let default = match uri.scheme.map(str::to_ascii_lowercase).as_deref() {
                Some("http") => Some(80),
                Some("https") => Some(443),
                _ => None,
            };
            let number = port.trim_start_matches('0');
            let number: u64 = if number.is_empty() {
                0
            } else {
                number.parse().unwrap_or(u64::MAX)
            };
            if Some(number) != default {
                out.push(':');
                out.push_str(&number.to_string());
            }
        }
    }
    out.push_str(uri.path);
    if let Some(fragment) = uri.fragment {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

/// `prepare_data`'s gsub over `\S+`: a word that is a URL (optionally in
/// double quotes) loses its query string, and its quotes, when Ruby can
/// parse it.
pub fn strip_url_queries(data: &str) -> String {
    static URL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"\A"?((https?://)[^ \t\r\n\f\v]+)"?\z"#).expect("static pattern")
    });
    let mut out = String::with_capacity(data.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        let replaced = URL
            .captures(word)
            .and_then(|c| parse(c.get(1)?.as_str()).map(|u| to_s_without_query(&u)));
        out.push_str(replaced.as_deref().unwrap_or(word));
        word.clear();
    };
    for c in data.chars() {
        // Ruby's \s: ASCII whitespace.
        if matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0c' | '\x0b') {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// `Discourse.route_for(term)` as search_for_id reads it: the topic id of
/// a topics#show route (`(route[:id] || route[:topic_id]).to_i`), for a
/// relative URL or one on this host under the base path.
pub fn topic_id_for(term: &str, hostname: &str, base_path: &str) -> Option<i64> {
    let uri = parse(term)?;
    if let Some(host) = uri.host
        && !(host == hostname && uri.path.starts_with(base_path))
    {
        return None;
    }
    // path.slice!(base_path): its first occurrence.
    let path = match uri.path.find(base_path) {
        Some(i) if !base_path.is_empty() => {
            format!("{}{}", &uri.path[..i], &uri.path[i + base_path.len()..])
        }
        _ => uri.path.to_string(),
    };
    let path = path.trim_start_matches('/');
    let rest = path.strip_prefix("t/")?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    // The format suffixes topics#show answers to.
    let strip_format = |s: &str| -> Option<(String, bool)> {
        for ext in [".json", ".html", ".md"] {
            if let Some(base) = s.strip_suffix(ext) {
                return Some((base.to_string(), true));
            }
        }
        if s.contains('.') {
            return None;
        }
        Some((s.to_string(), false))
    };
    let parts: Vec<&str> = rest.split('/').collect();
    let id = match parts.as_slice() {
        // t/:id (no `.md`: that format belongs to the topic_id routes).
        [one] if !one.is_empty() => {
            let (base, _) = strip_format(one)?;
            if one.ends_with(".md") && !digits(&base) {
                return None;
            }
            base
        }
        ["id_for" | "external_id", _] => return None,
        [a, b] => {
            if digits(a) && (digits(b) || *b == "last" || *b == "summary") {
                a.to_string()
            } else {
                let (base, _) = strip_format(b)?;
                if !digits(&base) {
                    return None;
                }
                base
            }
        }
        [_, b, c] => {
            if !digits(b) {
                return None;
            }
            let (base, _) = strip_format(c)?;
            if !(digits(&base) || matches!(base.as_str(), "last" | "print" | "summary")) {
                return None;
            }
            b.to_string()
        }
        _ => return None,
    };
    Some(crate::ruby::to_i(&id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_queries_as_ruby_does() {
        let cases = [
            ("HTTPS://Ex.COM:443/a?b=1#c", "HTTPS://Ex.COM:443/a?b=1#c"),
            ("http://x.com/t/slug/35?u=user1", "http://x.com/t/slug/35"),
            ("http://x.com?", "http://x.com"),
            ("http://x.com/a|b?c", "http://x.com/a|b?c"),
            ("http://x.com/\u{e9}?x", "http://x.com/\u{e9}?x"),
            ("http://[::1]:80/a?q", "http://[::1]/a"),
            ("http://user:pw@x.com/p?q=1", "http://user:pw@x.com/p"),
            ("http://x.com/%zz?a", "http://x.com/%zz?a"),
            ("\"http://x.com/a?b\"", "http://x.com/a"),
            ("http://x.com:abc/?q", "http://x.com:abc/?q"),
            ("http://x.com/a?b#", "http://x.com/a#"),
            ("http://x.com/#frag?x", "http://x.com/#frag?x"),
            ("http://x.com/a?b|c", "http://x.com/a"),
            ("http://x.com/a?%zz", "http://x.com/a?%zz"),
            ("http://x.com/a\"b?c", "http://x.com/a\"b?c"),
            ("http://x.com/a?b#c|d", "http://x.com/a?b#c|d"),
            ("http://x_y.com/a?b", "http://x_y.com/a"),
            ("https://x.com:443/a?b", "https://x.com/a"),
            ("http://x.com:443/a?b", "http://x.com:443/a"),
            ("http://x.com:/a?b", "http://x.com/a"),
            ("http://x.com/a?b?c", "http://x.com/a"),
            ("http://x.com/a#b#c", "http://x.com/a#b#c"),
            ("http://x.com/a'b?c", "http://x.com/a'b"),
            ("http://[x]/a?b", "http://[x]/a?b"),
            ("see http://x.com/a?b now", "see http://x.com/a now"),
        ];
        for (input, want) in cases {
            assert_eq!(strip_url_queries(input), want, "{input}");
        }
    }

    #[test]
    fn finds_topics_as_route_for_does() {
        let t = |s: &str| topic_id_for(s, "localhost", "");
        for (input, want) in [
            ("/t/35", Some(35)),
            ("/t/abc", Some(0)),
            ("/t/35abc", Some(35)),
            ("/t/x/35", Some(35)),
            ("/t/x/35/2", Some(35)),
            ("/t/35/2", Some(35)),
            ("/t/35/last", Some(35)),
            ("/t/x/35/last", Some(35)),
            ("/t/x/35/print", Some(35)),
            ("/t/x/35/summary", Some(35)),
            ("/t/35/summary", Some(35)),
            ("/t/x/35.json", Some(35)),
            ("/t/35.json", Some(35)),
            ("/t/x/35.md", Some(35)),
            ("/t/35.md", Some(35)),
            ("/t/x/35.rss", None),
            ("/t/id_for/35", None),
            ("/t/external_id/35", None),
            ("/t/x/35/wordpress", None),
            ("/t/x/y", None),
            ("/t/x/35/abc", None),
            ("/t/x/35/2/3", None),
            ("/t/35/posts", None),
            ("/t/35.html", Some(35)),
            ("/t/", None),
            ("/t", None),
            ("http://localhost/t/x/35", Some(35)),
            ("https://localhost/t/x/35", Some(35)),
            ("http://LOCALHOST/t/x/35", None),
            ("//localhost/t/x/35", Some(35)),
            ("t/x/35", Some(35)),
            ("http://localhost:9/t/x/35", Some(35)),
            ("/t/x/0035", Some(35)),
            ("/t/-5", Some(-5)),
            ("/t/x/35?a=b", Some(35)),
            ("/t/x/35#p", Some(35)),
            ("http://other.example/t/x/35", None),
        ] {
            assert_eq!(t(input), want, "{input}");
        }
    }
}
