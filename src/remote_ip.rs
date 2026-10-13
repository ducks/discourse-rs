//! `request.remote_ip`: ActionDispatch::RemoteIp (actionpack 8.1, Rack
//! 3) with its default trusted proxies, the only ones Discourse uses.
//!
//! The candidates are the forwarded addresses (the `Forwarded` header's
//! `for=` when it parses, else `X-Forwarded-For`) and `Client-Ip`, nearest
//! hop first, then the peer. The client is the nearest one that is not a
//! trusted proxy; when every one is trusted, the furthest. Taking the
//! first `X-Forwarded-For` entry instead would let any client name its
//! own address past the proxy. As in Rails, a site reachable without a
//! proxy in front still takes the headers: it is deployed behind one.

use std::net::{IpAddr, SocketAddr};

use axum::http::HeaderMap;

/// `ActionDispatch::RemoteIp::IpSpoofAttackError`: `Client-Ip` and the
/// forwarded addresses disagree. Rails does not rescue it: a 500.
#[derive(Debug)]
pub struct IpSpoofAttack(String);

impl std::fmt::Display for IpSpoofAttack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IP spoofing attack?! {}", self.0)
    }
}

impl std::error::Error for IpSpoofAttack {}

/// `ActionDispatch::RemoteIp::TRUSTED_PROXIES`
fn trusted(ip: &str) -> bool {
    let Ok(ip) = ip.parse::<IpAddr>() else {
        return false;
    };
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            a == 127
                || a == 10
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 169 && b == 254)
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

/// `sanitize_ips`: only single addresses (IPAddr.new parses them and
/// nothing sneaks a netmask in).
fn sanitize(ips: Vec<String>) -> Vec<String> {
    ips.into_iter()
        .filter(|ip| ip.parse::<IpAddr>().is_ok())
        .collect()
}

/// `ips_from` and Rack's `split_header`: a list split on commas and
/// whitespace.
fn split_list(value: &str) -> Vec<String> {
    value
        .trim()
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Rack's `split_authority(...)[1]` (its AUTHORITY regexp): a
/// bracketed IPv6 address, else the shortest printable prefix (no
/// brackets) that leaves `:digits` or nothing; None when it does not
/// match.
fn authority_address(authority: &str) -> Option<String> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (address, after) = rest.split_once(']')?;
        let port_ok = after.is_empty()
            || after
                .strip_prefix(':')
                .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        let ipv6_ok = !address.is_empty()
            && address
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.');
        if port_ok && ipv6_ok {
            return Some(address.to_string());
        }
    }
    if !authority
        .chars()
        .all(|c| c.is_ascii_graphic() && c != '[' && c != ']')
    {
        return None;
    }
    for (i, c) in authority.char_indices() {
        let port = &authority[i + 1..];
        if c == ':' && !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) {
            return Some(authority[..i].to_string());
        }
    }
    Some(authority.to_string())
}

/// Ruby's `String#split(sep)`: trailing empty strings dropped.
fn ruby_split(s: &str, sep: char) -> Vec<&str> {
    let mut parts: Vec<&str> = s.split(sep).collect();
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

/// Rack's `wrap_ipv6`
fn wrap_ipv6(host: &str) -> String {
    if !host.starts_with('[') && host.matches(':').count() > 1 {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

/// `Rack::Utils.forwarded_values(header)[:for]`: None when the header is
/// missing, malformed (Rack drops it whole) or has no `for=`.
fn forwarded_for_values(header: &str) -> Option<Vec<String>> {
    let header = header.replace('\n', ";");
    let mut values = Vec::new();
    for field in ruby_split(&header, ';') {
        for pair in ruby_split(field, ',') {
            let pair = ruby_split(pair, '=')
                .into_iter()
                .map(str::trim)
                .collect::<Vec<_>>()
                .join("=");
            let (key, value) = pair.split_once('=')?;
            if !["by", "for", "host", "proto"].contains(&key.to_ascii_lowercase().as_str()) {
                return None;
            }
            let value = value.strip_prefix('"').unwrap_or(value);
            let value = value.strip_suffix('"').unwrap_or(value);
            if value.is_empty() || value.contains('"') {
                return None;
            }
            if key.eq_ignore_ascii_case("for") {
                values.push(value.to_string());
            }
        }
    }
    (!values.is_empty()).then_some(values)
}

/// Rack's `Request#forwarded_for`, by forwarded_priority
/// `[:forwarded, :x_forwarded]`.
fn forwarded_for(headers: &HeaderMap) -> Vec<String> {
    let header = |name: &str| {
        let values: Vec<&str> = headers
            .get_all(name)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect();
        (!values.is_empty()).then(|| values.join(","))
    };
    if let Some(values) = header("forwarded")
        .as_deref()
        .and_then(forwarded_for_values)
    {
        return values.iter().filter_map(|a| authority_address(a)).collect();
    }
    match header("x-forwarded-for") {
        Some(value) => split_list(&value)
            .iter()
            .filter_map(|a| authority_address(&wrap_ipv6(a)))
            .collect(),
        None => Vec::new(),
    }
}

/// `request.remote_ip` (GetIp#calculate_ip).
pub fn remote_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> Result<String, IpSpoofAttack> {
    let remote_addr = peer
        .map(|p| p.ip().to_string())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let client_header = headers.get("client-ip").and_then(|v| v.to_str().ok());
    let mut client_ips = sanitize(client_header.map(split_list).unwrap_or_default());
    client_ips.reverse();
    let mut forwarded_ips = sanitize(forwarded_for(headers));
    forwarded_ips.reverse();
    if let (Some(client), Some(_)) = (client_ips.last(), forwarded_ips.last())
        && !forwarded_ips.contains(client)
    {
        let xff = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        return Err(IpSpoofAttack(format!(
            "HTTP_CLIENT_IP={:?} HTTP_X_FORWARDED_FOR={xff:?}",
            client_header.unwrap_or_default()
        )));
    }
    let ips: Vec<String> = forwarded_ips.into_iter().chain(client_ips).collect();
    let nearest_untrusted = ips
        .iter()
        .chain(std::iter::once(&remote_addr))
        .find(|ip| !trusted(ip));
    Ok(nearest_untrusted
        .or(ips.last())
        .cloned()
        .unwrap_or(remote_addr))
}

/// Rack's `trusted_proxies` (Request.ip_filter): loopback, private
/// ranges, localhost and unix sockets, matched on the text.
fn rack_trusted_proxy(ip: &str) -> bool {
    static TRUSTED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        let octet = r"\.(25[0-5]|2[0-4][0-9]|[01]?[0-9]?[0-9])";
        regex::Regex::new(&format!(
            r"(?i)\A127({octet}){{3}}\z|\A::1\z|\Af[cd][0-9a-f]{{2}}(?::[0-9a-f]{{0,4}}){{0,7}}\z|\A10({octet}){{3}}\z|\A172\.(1[6-9]|2[0-9]|3[01])({octet}){{2}}\z|\A192\.168({octet}){{2}}\z|\Alocalhost\z|\Aunix(\z|:)"
        ))
        .expect("Rack's trusted proxies")
    });
    TRUSTED.is_match(ip)
}

/// `request.ip` (Rack::Request#ip), which the request tracker and the
/// auth cookie limiter key on: the peer unless it is a trusted proxy,
/// else the nearest untrusted forwarded address (or the furthest when all
/// are trusted), else the peer.
pub fn request_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    let remote_addr = peer
        .map(|p| p.ip().to_string())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    if !rack_trusted_proxy(&remote_addr) {
        return remote_addr;
    }
    let forwarded = forwarded_for(headers);
    if !forwarded.is_empty() {
        return forwarded
            .iter()
            .rev()
            .find(|ip| !rack_trusted_proxy(ip))
            .unwrap_or(&forwarded[0])
            .clone();
    }
    remote_addr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ip_is_racks() {
        let headers = |xff: &str| {
            let mut h = HeaderMap::new();
            h.insert("x-forwarded-for", xff.parse().unwrap());
            h
        };
        let peer = |ip: &str| Some(SocketAddr::new(ip.parse().unwrap(), 443));
        assert_eq!(request_ip(&headers("1.1.1.1"), peer("8.8.8.8")), "8.8.8.8");
        assert_eq!(
            request_ip(&headers("1.1.1.1, 2.2.2.2"), peer("127.0.0.1")),
            "2.2.2.2"
        );
        assert_eq!(
            request_ip(&headers("1.1.1.1, 10.0.0.2"), peer("10.0.0.1")),
            "1.1.1.1"
        );
        assert_eq!(
            request_ip(&headers("10.0.0.3, 10.0.0.2"), peer("10.0.0.1")),
            "10.0.0.3"
        );
        assert_eq!(
            request_ip(&HeaderMap::new(), peer("127.0.0.1")),
            "127.0.0.1"
        );
        assert!(rack_trusted_proxy("172.31.0.1") && !rack_trusted_proxy("172.32.0.1"));
        assert!(rack_trusted_proxy("fd12:3456::1") && rack_trusted_proxy("unix:/tmp/s"));
    }

    fn ip(pairs: &[(&str, &str)], peer: &str) -> Result<String, IpSpoofAttack> {
        let mut headers = HeaderMap::new();
        for (k, v) in pairs {
            headers.append(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        let peer: IpAddr = peer.parse().unwrap();
        remote_ip(&headers, Some(SocketAddr::new(peer, 443)))
    }

    #[test]
    fn a_client_cannot_name_its_own_address_past_a_proxy() {
        // The client sent "1.1.1.1"; Caddy appended what it saw.
        assert_eq!(
            ip(&[("x-forwarded-for", "1.1.1.1, 2.2.2.2")], "127.0.0.1").unwrap(),
            "2.2.2.2"
        );
    }

    #[test]
    fn trusted_hops_are_skipped() {
        assert_eq!(
            ip(
                &[("x-forwarded-for", "3.3.3.3, 10.0.0.2, 192.168.1.1")],
                "127.0.0.1"
            )
            .unwrap(),
            "3.3.3.3"
        );
    }

    #[test]
    fn all_trusted_gives_the_furthest() {
        assert_eq!(
            ip(&[("x-forwarded-for", "10.1.2.3, 127.0.0.1")], "127.0.0.1").unwrap(),
            "10.1.2.3"
        );
        assert_eq!(ip(&[], "127.0.0.1").unwrap(), "127.0.0.1");
    }

    #[test]
    fn the_headers_count_without_a_proxy_too() {
        // As in Rails: the nearest untrusted forwarded hop wins even when
        // the peer is untrusted, so a site serves behind a proxy.
        assert_eq!(
            ip(&[("x-forwarded-for", "1.1.1.1")], "5.5.5.5").unwrap(),
            "1.1.1.1"
        );
        assert_eq!(
            ip(&[("x-forwarded-for", "10.0.0.9")], "5.5.5.5").unwrap(),
            "5.5.5.5"
        );
    }

    #[test]
    fn invalid_entries_and_netmasks_are_dropped() {
        assert_eq!(
            ip(
                &[("x-forwarded-for", "4.4.4.4, junk, 6.6.6.0/24")],
                "127.0.0.1"
            )
            .unwrap(),
            "4.4.4.4"
        );
    }

    #[test]
    fn ports_and_ipv6_brackets() {
        assert_eq!(
            ip(&[("x-forwarded-for", "4.4.4.4:8080")], "127.0.0.1").unwrap(),
            "4.4.4.4"
        );
        assert_eq!(
            ip(&[("x-forwarded-for", "2001:db8::1")], "::1").unwrap(),
            "2001:db8::1"
        );
        assert_eq!(
            ip(&[("x-forwarded-for", "[2001:db8::2]:443")], "::1").unwrap(),
            "2001:db8::2"
        );
    }

    #[test]
    fn the_forwarded_header_comes_first() {
        assert_eq!(
            ip(
                &[
                    (
                        "forwarded",
                        "for=7.7.7.7;proto=https, for=\"[2001:db8::3]:80\""
                    ),
                    ("x-forwarded-for", "8.8.8.8")
                ],
                "127.0.0.1"
            )
            .unwrap(),
            "2001:db8::3"
        );
        // An unbracketed IPv6 address loses its last group as a port, and
        // the rest is no address.
        assert_eq!(
            ip(
                &[
                    ("forwarded", "for=2001:db8::1"),
                    ("x-forwarded-for", "8.8.8.8")
                ],
                "127.0.0.1"
            )
            .unwrap(),
            "127.0.0.1"
        );
        // Malformed: Rack ignores it whole and X-Forwarded-For applies.
        assert_eq!(
            ip(
                &[
                    ("forwarded", "for=7.7.7.7;bogus"),
                    ("x-forwarded-for", "8.8.8.8")
                ],
                "127.0.0.1"
            )
            .unwrap(),
            "8.8.8.8"
        );
    }

    #[test]
    fn client_ip_must_be_among_the_forwarded() {
        assert_eq!(
            ip(&[("client-ip", "9.9.9.9")], "127.0.0.1").unwrap(),
            "9.9.9.9"
        );
        assert_eq!(
            ip(
                &[("client-ip", "9.9.9.9"), ("x-forwarded-for", "9.9.9.9")],
                "127.0.0.1"
            )
            .unwrap(),
            "9.9.9.9"
        );
        assert!(
            ip(
                &[("client-ip", "9.9.9.9"), ("x-forwarded-for", "8.8.8.8")],
                "127.0.0.1"
            )
            .is_err()
        );
    }
}
