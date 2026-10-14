//! lib/final_destination.rb's `get`: reading a page of another site,
//! following its redirects, without letting a URL reach the site's own
//! network (SSRFDetector). Each request connects to an address resolved
//! and checked here, so a name that resolves elsewhere a second time
//! (DNS rebinding) can't slip past the check.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use sqlx::PgConnection;

use crate::AppError;
use crate::site_settings::SiteSettings;
use crate::url::Urls;

/// `FinalDestination::DEFAULT_USER_AGENT`
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

/// `SSRFDetector::PRIVATE_IPV4_RANGES` and `PRIVATE_IPV6_RANGES`.
const PRIVATE_RANGES: &[&str] = &[
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.0.0.0/24",
    "192.0.0.0/29",
    "192.0.0.8/32",
    "192.0.0.170/32",
    "192.0.0.171/32",
    "192.0.2.0/24",
    "192.168.0.0/16",
    "192.175.48.0/24",
    "198.18.0.0/15",
    "198.51.100.0/24",
    "203.0.113.0/24",
    "240.0.0.0/4",
    "255.255.255.255/32",
    "::1/128",
    "::/128",
    "64:ff9b:1::/48",
    "100::/64",
    "2001::/23",
    "2001:2::/48",
    "2001:db8::/32",
    "fc00::/7",
    "fe80::/10",
];

/// An `IPAddr.new("a/b")` range.
#[derive(Debug, Clone, Copy)]
struct Range {
    net: IpAddr,
    prefix: u8,
}

impl Range {
    fn parse(s: &str) -> Option<Range> {
        let s = s.trim();
        let (addr, prefix) = match s.split_once('/') {
            Some((a, p)) => (a, p.parse::<u8>().ok()?),
            None => (s, if s.contains(':') { 128 } else { 32 }),
        };
        Some(Range {
            net: addr.parse().ok()?,
            prefix,
        })
    }

    fn contains(&self, ip: IpAddr) -> bool {
        match (self.net, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let bits = u32::from(self.prefix.min(32));
                let mask = if bits == 0 {
                    0
                } else {
                    u32::MAX << (32 - bits)
                };
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let bits = u32::from(self.prefix.min(128));
                let mask = if bits == 0 {
                    0
                } else {
                    u128::MAX << (128 - bits)
                };
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

/// `ip.native`: an IPv4-mapped IPv6 address as its IPv4 one.
fn native(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

/// `SSRFDetector.ip_allowed?`
fn ip_allowed(ip: IpAddr, blocked: &[Range]) -> bool {
    let ip = native(ip);
    let private = PRIVATE_RANGES
        .iter()
        .filter_map(|r| Range::parse(r))
        .any(|r| r.contains(ip));
    !private && !blocked.iter().any(|r| r.contains(ip))
}

fn setting_list(settings: &SiteSettings, name: &str) -> Result<Vec<String>, AppError> {
    Ok(settings
        .get(name)?
        .to_s()
        .split(['|', '\n'])
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .collect())
}

fn hostname_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = if let Some(stripped) = host.strip_prefix('[') {
        stripped.split(']').next()?
    } else {
        host.split(':').next()?
    };
    Some(host.to_lowercase())
}

/// A URL's parts as `URI.parse` gives them, for http and https.
#[derive(Debug, Clone, PartialEq)]
pub struct Uri {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    /// The path and query (`request_uri`).
    pub request_uri: String,
}

impl Uri {
    pub fn parse(url: &str) -> Option<Uri> {
        let (scheme, rest) = url.split_once("://")?;
        let scheme = scheme.to_lowercase();
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..end];
        if authority.contains('@') || authority.starts_with('[') {
            return None;
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().ok()?),
            None => (
                authority,
                match scheme.as_str() {
                    "http" => 80,
                    "https" => 443,
                    _ => return None,
                },
            ),
        };
        if host.is_empty() {
            return None;
        }
        let tail = &rest[end..];
        let tail = tail.split('#').next().unwrap_or("");
        let request_uri = if tail.is_empty() || tail.starts_with('?') {
            format!("/{tail}")
        } else {
            tail.to_string()
        };
        Some(Uri {
            scheme,
            host: host.to_lowercase(),
            port,
            request_uri,
        })
    }

    fn to_url(&self) -> String {
        let default = (self.scheme == "http" && self.port == 80)
            || (self.scheme == "https" && self.port == 443);
        if default {
            format!("{}://{}{}", self.scheme, self.host, self.request_uri)
        } else {
            format!(
                "{}://{}:{}{}",
                self.scheme, self.host, self.port, self.request_uri
            )
        }
    }
}

/// What reading a page gave.
pub enum Read {
    /// The page's beginning, with its content type, from where it was
    /// read (after redirects).
    Page {
        body: Vec<u8>,
        content_type: Option<String>,
        uri: Uri,
    },
    /// Nothing to read: an invalid or blocked address, a page that asked
    /// not to be oneboxed, an error, too many redirects.
    Nothing,
}

/// FinalDestination's options for `get`.
pub struct Options {
    pub timeout: Duration,
    pub max_redirects: u32,
    /// `initial_https_redirect_ignore_limit`
    pub https_redirect_ignore_limit: bool,
    pub stop_at_blocked_pages: bool,
    /// Extra request headers.
    pub headers: Vec<(String, String)>,
}

/// The settings FinalDestination reads.
pub struct Fetcher<'a> {
    pub settings: &'a SiteSettings,
    pub urls: Urls<'a>,
}

impl Fetcher<'_> {
    /// `SSRFDetector.allowed_internal_hosts`: the CDNs, the site itself and
    /// allowed_internal_hosts.
    fn allowed_internal_hosts(&self) -> Result<Vec<String>, AppError> {
        let mut hosts = Vec::new();
        if let Some(cdn) = self.urls.config.globals.cdn_url()
            && let Some(h) = hostname_of(cdn)
        {
            hosts.push(h);
        }
        if let Some(h) = hostname_of(&self.urls.base_url_no_prefix()?) {
            hosts.push(h);
        }
        hosts.extend(setting_list(self.settings, "allowed_internal_hosts")?);
        Ok(hosts)
    }

    /// `validate_uri_format`: http on port 80 (or an allowed internal
    /// host serving the S3 endpoint), https on 443, never an IP address.
    fn valid(&self, uri: &Uri) -> Result<bool, AppError> {
        if uri.scheme != "http" && uri.scheme != "https" {
            return Ok(false);
        }
        if uri.scheme == "http" && uri.port != 80 && !self.http_port_ok(uri)? {
            return Ok(false);
        }
        if uri.scheme == "https" && uri.port != 443 {
            return Ok(false);
        }
        Ok(uri.host.parse::<IpAddr>().is_err())
    }

    /// `http_port_ok?`: another port only for an allowed internal host
    /// that is the S3 endpoint's.
    fn http_port_ok(&self, uri: &Uri) -> Result<bool, AppError> {
        let allowed = setting_list(self.settings, "allowed_internal_hosts")?;
        let endpoint = self.settings.get("s3_endpoint")?.to_s().to_string();
        if allowed.is_empty() || endpoint.trim().is_empty() {
            return Ok(false);
        }
        let Some(endpoint) = Uri::parse(&endpoint) else {
            return Ok(false);
        };
        Ok(allowed.iter().any(|h| {
            h.to_lowercase() == uri.host
                && uri.port == endpoint.port
                && uri.host.ends_with(&endpoint.host)
        }))
    }

    /// `Onebox::DomainChecker.is_blocked?`
    pub fn blocked_domain(&self, host: &str) -> Result<bool, AppError> {
        let host = host.to_lowercase();
        Ok(setting_list(self.settings, "blocked_onebox_domains")?
            .iter()
            .any(|b| {
                let b = b.to_lowercase();
                host == b || host.ends_with(&format!(".{b}"))
            }))
    }

    /// `SSRFDetector.lookup_and_filter_ips`: the host's addresses that
    /// are allowed, none when lookup fails or all are refused.
    async fn lookup(&self, uri: &Uri) -> Result<Vec<SocketAddr>, AppError> {
        let resolved: Vec<SocketAddr> =
            match tokio::net::lookup_host((uri.host.as_str(), uri.port)).await {
                Ok(addrs) => addrs.collect(),
                Err(_) => return Ok(Vec::new()),
            };
        if self
            .allowed_internal_hosts()?
            .iter()
            .any(|h| h.eq_ignore_ascii_case(&uri.host))
        {
            return Ok(resolved);
        }
        let blocked: Vec<Range> = setting_list(self.settings, "blocked_ip_blocks")?
            .iter()
            .filter_map(|r| Range::parse(r))
            .collect();
        Ok(resolved
            .into_iter()
            .filter(|a| ip_allowed(a.ip(), &blocked))
            .collect())
    }

    /// `FinalDestination#get`: the page's beginning, up to `max_bytes`, or
    /// until `done` says the bytes read so far are enough.
    pub async fn get(
        &self,
        conn: &mut PgConnection,
        url: &str,
        opts: &Options,
        max_bytes: impl Fn(&Uri) -> usize,
        done: impl Fn(&[u8]) -> bool,
    ) -> Result<Read, AppError> {
        let Some(mut uri) = Uri::parse(url) else {
            return Ok(Read::Nothing);
        };
        let mut redirects = opts.max_redirects;
        let mut ignore_limit = opts.https_redirect_ignore_limit;
        let mut cookie: Option<String> = None;
        loop {
            // A domain known to redirect to https.
            if uri.port == 80
                && crate::owned_schema::cached_get(conn, &format!("HTTPS_DOMAIN_{}", uri.host))
                    .await?
                    .is_some()
            {
                uri.scheme = "https".into();
                uri.port = 443;
            }
            if !self.valid(&uri)? {
                return Ok(Read::Nothing);
            }
            if opts.stop_at_blocked_pages && self.blocked_domain(&uri.host)? {
                return Ok(Read::Nothing);
            }
            let addrs = self.lookup(&uri).await?;
            let Some(addr) = addrs.first() else {
                // SSRFError: RetrieveTitle gives up quietly.
                return Ok(Read::Nothing);
            };
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(opts.timeout)
                .resolve(&uri.host, *addr)
                .build()?;
            let mut request = client
                .get(uri.to_url())
                .header("User-Agent", DEFAULT_USER_AGENT)
                .header("Accept", "*/*")
                .header("Accept-Language", "*");
            for (name, value) in &opts.headers {
                request = request.header(name.as_str(), value.as_str());
            }
            // Cookies a redirect set come along; Authorization never does.
            if let Some(c) = &cookie {
                request = request.header("Cookie", c.as_str());
            }
            let response = match tokio::time::timeout(opts.timeout, request.send()).await {
                Ok(Ok(r)) => r,
                _ => return Ok(Read::Nothing),
            };
            if opts.stop_at_blocked_pages
                && response
                    .headers()
                    .get("Discourse-No-Onebox")
                    .is_some_and(|v| v == "1")
            {
                return Ok(Read::Nothing);
            }
            let status = response.status();
            if status.is_redirection() {
                let set_cookie = response
                    .headers()
                    .get_all("set-cookie")
                    .iter()
                    .filter_map(|v| v.to_str().ok())
                    .map(|c| c.split(';').next().unwrap_or("").trim().to_string())
                    .collect::<Vec<_>>();
                let Some(location) = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
                else {
                    return Ok(Read::Nothing);
                };
                let location = if location.starts_with('/') {
                    format!("{}://{}{}", uri.scheme, uri.host, location)
                } else {
                    location
                };
                let Some(next) = Uri::parse(&location) else {
                    return Ok(Read::Nothing);
                };
                // The first http to https redirect of the same URL is free.
                if redirects == opts.max_redirects
                    && ignore_limit
                    && uri.scheme == "http"
                    && next.scheme == "https"
                    && uri.host == next.host
                    && uri.request_uri == next.request_uri
                {
                    redirects += 1;
                    ignore_limit = false;
                }
                if redirects == 0 {
                    return Ok(Read::Nothing);
                }
                if uri.port == 80 && next.port == 443 && next.scheme == "https" {
                    crate::owned_schema::cached_setex(
                        conn,
                        &format!("HTTPS_DOMAIN_{}", next.host),
                        86_400,
                        "1",
                    )
                    .await?;
                }
                cookie = (!set_cookie.is_empty()).then(|| set_cookie.join("; "));
                uri = next;
                redirects -= 1;
                continue;
            }
            if !status.is_success() {
                return Ok(Read::Nothing);
            }
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let limit = max_bytes(&uri);
            let mut body = Vec::new();
            let mut response = response;
            loop {
                let chunk = match tokio::time::timeout(opts.timeout, response.chunk()).await {
                    Ok(Ok(Some(chunk))) => chunk,
                    Ok(Ok(None)) => break,
                    // A read timeout: what was read stands only if done.
                    _ => break,
                };
                body.extend_from_slice(&chunk);
                if done(&body) || body.len() > limit {
                    break;
                }
            }
            return Ok(Read::Page {
                body,
                content_type,
                uri,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_and_blocked_addresses() {
        let none = [];
        assert!(!ip_allowed("10.1.2.3".parse().unwrap(), &none));
        assert!(!ip_allowed("127.0.0.1".parse().unwrap(), &none));
        assert!(!ip_allowed("::1".parse().unwrap(), &none));
        assert!(!ip_allowed("::ffff:192.168.1.1".parse().unwrap(), &none));
        assert!(ip_allowed("93.184.216.34".parse().unwrap(), &none));
        let blocked = [Range::parse("93.184.0.0/16").unwrap()];
        assert!(!ip_allowed("93.184.216.34".parse().unwrap(), &blocked));
    }

    #[test]
    fn parses_uris_as_uri_parse() {
        let u = Uri::parse("https://Example.com/a/b?x=1#frag").unwrap();
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, 443);
        assert_eq!(u.request_uri, "/a/b?x=1");
        assert_eq!(Uri::parse("http://x.test:8099").unwrap().request_uri, "/");
        assert!(Uri::parse("ftp://x.test/").is_none());
    }
}
