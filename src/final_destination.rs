//! lib/final_destination.rb's `get`: reading a page of another site,
//! following its redirects, without letting a URL reach the site's own
//! network (SSRFDetector). Each request connects to an address resolved
//! and checked here, so a name that resolves elsewhere a second time
//! (DNS rebinding) can't slip past the check.

use std::net::{IpAddr, SocketAddr};
use std::sync::LazyLock;
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

/// Ruby's `URI::RFC3986_Parser`: what `URI.parse` takes without raising
/// InvalidURIError (its IP-literal hosts loosened to any bracketed text).
static RFC3986: LazyLock<regex::Regex> = LazyLock::new(|| {
    let hex = "%[0-9A-Fa-f]{2}";
    let seg = format!("(?:{hex}|[!$&-.0-9:;=@A-Z_a-z~/])");
    let seg_no_slash = format!("(?:{hex}|[!$&-.0-9:;=@A-Z_a-z~])");
    let authority = format!(
        r"(?:(?:{hex}|[!$&-.0-9:;=A-Z_a-z~])*@)?(?:\[[^\]]*\]|(?:{hex}|[!$&-.0-9;=A-Z_a-z~])*)(?::[0-9]*)?"
    );
    let tail = format!(r"(?:\?[^#]*)?(?:#(?:{hex}|[!$&-.0-9:;=@A-Z_a-z~/?])*)?");
    let absolute = format!(
        r"[A-Za-z][+\-.0-9A-Za-z]*:(?://{authority}(?:/{seg}*)?|/(?:{seg_no_slash}{seg}*)?|{seg_no_slash}{seg}*|)"
    );
    let relative = format!(
        r"(?://{authority}(?:/{seg}*)?|/{seg}*|(?:{hex}|[!$&-.0-9;=@A-Z_a-z~])+(?:/{seg}*)?|)"
    );
    regex::Regex::new(&format!(r"\A(?:{absolute}|{relative}){tail}\z")).unwrap()
});

/// Whether `URI.parse` takes the URL.
pub fn ruby_uri_valid(url: &str) -> bool {
    RFC3986.is_match(url)
}

/// A URL's parts as `URI.parse` gives them, for http and https.
#[derive(Debug, Clone, PartialEq)]
pub struct Uri {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    /// The path and query (`request_uri`).
    pub request_uri: String,
    pub fragment: Option<String>,
}

impl Uri {
    pub fn parse(url: &str) -> Option<Uri> {
        if !ruby_uri_valid(url) {
            return None;
        }
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
        let (tail, fragment) = match tail.split_once('#') {
            Some((t, f)) => (t, Some(f.to_string())),
            None => (tail, None),
        };
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
            fragment,
        })
    }

    /// `uri.path`
    pub fn path(&self) -> &str {
        self.request_uri.split('?').next().unwrap_or("")
    }

    /// `uri.to_s`, with its fragment.
    pub fn to_s(&self) -> String {
        match &self.fragment {
            Some(f) => format!("{}#{f}", self.to_url()),
            None => self.to_url(),
        }
    }

    /// The URL requested: no fragment.
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
    /// The last response's error status (4xx, 5xx).
    Failed { status: u16 },
}

/// What `lookup_and_filter_ips` found.
pub enum Lookup {
    Allowed(Vec<IpAddr>),
    /// DisallowedIpError: every address is private or blocked.
    Disallowed,
    /// LookupFailedError
    Failed,
}

/// A POST to make.
pub struct Post<'a> {
    pub url: &'a str,
    pub headers: &'a [(String, String)],
    pub body: &'a [u8],
    pub verify_certificate: bool,
    pub timeout: Duration,
    /// `Discourse.user_agent`, which Net::HTTP's patched headers start with.
    pub user_agent: &'a str,
}

/// What a POST gave.
pub enum Posted {
    Response {
        status: u16,
        /// Lower-cased, in the order received.
        headers: Vec<(String, String)>,
        body: String,
    },
    /// The SSRF check refused every address (DisallowedIpError).
    Blocked,
    /// Any other failure, with its message.
    Failed(String),
}

/// The host and port of an absolute http(s) URL.
fn post_target(url: &str) -> Option<(String, u16)> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let host = parsed.host_str()?.trim_matches(['[', ']']).to_string();
    let port = parsed.port_or_known_default()?;
    Some((host, port))
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
    pub(crate) async fn lookup(&self, uri: &Uri) -> Result<Vec<SocketAddr>, AppError> {
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

    /// `SSRFDetector.lookup_and_filter_ips(name)` as it raises: the
    /// allowed addresses, or why there are none.
    pub async fn lookup_and_filter_ips(&self, host: &str) -> Result<Lookup, AppError> {
        let resolved: Vec<IpAddr> = match host.parse::<IpAddr>() {
            Ok(ip) => vec![ip],
            Err(_) => match tokio::net::lookup_host((host, 80)).await {
                Ok(addrs) => addrs.map(|a| a.ip()).collect(),
                Err(_) => return Ok(Lookup::Failed),
            },
        };
        if resolved.is_empty() {
            return Ok(Lookup::Failed);
        }
        if self
            .allowed_internal_hosts()?
            .iter()
            .any(|h| h.eq_ignore_ascii_case(host))
        {
            return Ok(Lookup::Allowed(resolved));
        }
        let blocked: Vec<Range> = setting_list(self.settings, "blocked_ip_blocks")?
            .iter()
            .filter_map(|r| Range::parse(r))
            .collect();
        let allowed: Vec<IpAddr> = resolved
            .into_iter()
            .filter(|ip| ip_allowed(*ip, &blocked))
            .collect();
        Ok(if allowed.is_empty() {
            Lookup::Disallowed
        } else {
            Lookup::Allowed(allowed)
        })
    }

    /// A POST through `FinalDestination::FaradayAdapter` (WebHookEmitter):
    /// the connection made only to an address the SSRF check allows, the
    /// headers sent as given, no redirects followed.
    pub async fn post(&self, req: &Post<'_>) -> Result<Posted, AppError> {
        self.send(reqwest::Method::POST, req).await
    }

    /// A request of any method through `FinalDestination::FaradayAdapter`
    /// (the OAuth strategies' token and user info requests), as `post`
    /// makes it.
    pub async fn send(&self, method: reqwest::Method, req: &Post<'_>) -> Result<Posted, AppError> {
        let Some((host, port)) = post_target(req.url) else {
            return Ok(Posted::Failed(format!("bad URI(is not URI?): {}", req.url)));
        };
        let addrs = match self.lookup_and_filter_ips(&host).await? {
            Lookup::Allowed(addrs) => addrs,
            Lookup::Disallowed => return Ok(Posted::Blocked),
            Lookup::Failed => {
                return Ok(Posted::Failed("FinalDestination: lookup failed".into()));
            }
        };
        let addrs: Vec<SocketAddr> = addrs.iter().map(|ip| SocketAddr::new(*ip, port)).collect();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(req.timeout)
            .timeout(req.timeout)
            .http1_title_case_headers()
            .danger_accept_invalid_certs(!req.verify_certificate)
            .resolve_to_addrs(&host, &addrs)
            .build()?;
        let mut request = client.request(method, req.url);
        for (name, value) in req.headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let response = match request.body(req.body.to_vec()).send().await {
            Ok(r) => r,
            Err(e) => {
                let cause = if e.is_timeout() {
                    "execution expired".to_string()
                } else if e.is_connect() {
                    "Connection refused".to_string()
                } else {
                    e.to_string()
                };
                return Ok(Posted::Failed(format!(
                    "Failed to open TCP connection to {host}:{port} ({cause})"
                )));
            }
        };
        let status = response.status().as_u16();
        // Net::HTTP's headers, each name once, its values joined; a
        // response carries the User-Agent Discourse's patch gives every
        // header set first.
        let mut headers: Vec<(String, String)> = vec![("user-agent".into(), req.user_agent.into())];
        for (name, value) in response.headers() {
            let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
            match headers.iter_mut().find(|(n, _)| n == name.as_str()) {
                Some((_, v)) => {
                    v.push_str(", ");
                    v.push_str(&value);
                }
                None => headers.push((name.as_str().to_string(), value)),
            }
        }
        let body = match response.bytes().await {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(e) => return Ok(Posted::Failed(e.to_string())),
        };
        Ok(Posted::Response {
            status,
            headers,
            body,
        })
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
                return Ok(Read::Failed {
                    status: status.as_u16(),
                });
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

/// `FinalDestination::MAX_REQUEST_SIZE_BYTES`
const MAX_REQUEST_SIZE_BYTES: usize = 5_242_880;
/// `FinalDestination::MAX_REQUEST_TIME_SECONDS`
const MAX_REQUEST_TIME: Duration = Duration::from_secs(10);

/// Where `resolve` ended (`fd.status`).
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Resolved,
    BlockedPage,
    InvalidAddress,
    TooManyRedirects,
    /// A response that is neither the page nor a redirect.
    Failure {
        status_code: u16,
        bot_challenge: bool,
    },
    /// A timeout, which `resolve` swallows (status left `:ready`).
    Timeout,
}

/// What `resolve` found out.
pub struct Resolution {
    pub status: Status,
    /// What `resolve` returned: the page's URI once resolved.
    pub uri: Option<Uri>,
    /// `fd.hostname`: the host it was last at.
    pub hostname: String,
    pub content_type: Option<String>,
    pub cookie: Option<String>,
}

/// FinalDestination's options for `resolve`.
#[derive(Clone)]
pub struct ResolveOptions {
    pub timeout: Duration,
    /// `max_redirects` (nil is 5).
    pub max_redirects: u32,
    /// `initial_https_redirect_ignore_limit`
    pub https_redirect_ignore_limit: bool,
    pub stop_at_blocked_pages: bool,
    /// URLs whose host and path prefix count as resolved without a request
    /// (the site's own base URL comes first).
    pub ignore_redirects: Vec<String>,
    pub force_get_hosts: Vec<String>,
    pub force_custom_user_agent_hosts: Vec<String>,
    pub preserve_fragment_url_hosts: Vec<String>,
    /// The user agent for force_custom_user_agent_hosts
    /// (`Onebox::Helpers.user_agent`).
    pub custom_user_agent: String,
    /// Extra request headers.
    pub headers: Vec<(String, String)>,
}

/// An error `resolve` lets through, as Excon raises it (a refused
/// connection, a body too big): Oneboxer.onebox_raw rescues it.
#[derive(Debug)]
pub struct Raised(pub String);

/// `hostname_matches?(url)`: a `*.` host matches its subdomains.
fn hostname_matches(uri: &Uri, url: &str) -> bool {
    let Some(other) = Uri::parse(url) else {
        return false;
    };
    match other.host.strip_prefix("*.") {
        Some(rest) => uri.host.ends_with(rest),
        None => uri.host == other.host,
    }
}

impl Fetcher<'_> {
    /// `FinalDestination#resolve`: where the URL ends up, following
    /// redirects with HEAD requests (GET for force_get_hosts), without
    /// reading the page.
    pub async fn resolve(
        &self,
        conn: &mut PgConnection,
        url: &str,
        opts: &ResolveOptions,
    ) -> Result<Result<Resolution, Raised>, AppError> {
        let Some(mut uri) = Uri::parse(url) else {
            return Ok(Ok(Resolution {
                status: Status::InvalidAddress,
                uri: None,
                hostname: String::new(),
                content_type: None,
                cookie: None,
            }));
        };
        let max = i64::from(opts.max_redirects);
        let mut limit = max;
        let mut ignore_limit = opts.https_redirect_ignore_limit;
        let mut ignored: Vec<(String, String)> = Vec::new();
        if limit > 0 {
            let mut urls = vec![self.urls.base_url()?];
            urls.extend(opts.ignore_redirects.iter().cloned());
            for u in urls {
                if let Some(i) = Uri::parse(&u) {
                    let path = i.path().to_string();
                    ignored.push((i.host, path));
                }
            }
        }
        let force_get = opts
            .force_get_hosts
            .iter()
            .any(|h| hostname_matches(&uri, h));
        let preserve_fragment = opts
            .preserve_fragment_url_hosts
            .iter()
            .any(|h| hostname_matches(&uri, h));
        let user_agent = if opts
            .force_custom_user_agent_hosts
            .iter()
            .any(|h| hostname_matches(&uri, h))
        {
            opts.custom_user_agent.clone()
        } else {
            DEFAULT_USER_AGENT.to_string()
        };
        let mut cookie: Option<String> = None;
        let done = |status, uri: &Uri, ret: Option<Uri>, content_type, cookie: &Option<String>| {
            Ok(Ok(Resolution {
                status,
                hostname: uri.host.clone(),
                uri: ret,
                content_type,
                cookie: cookie.clone(),
            }))
        };
        loop {
            if uri.port == 80
                && crate::owned_schema::cached_get(conn, &format!("HTTPS_DOMAIN_{}", uri.host))
                    .await?
                    .is_some()
            {
                uri.scheme = "https".into();
                uri.port = 443;
            }
            if limit < 0 {
                return done(Status::TooManyRedirects, &uri, None, None, &cookie);
            }
            if !self.valid(&uri)? {
                return done(Status::InvalidAddress, &uri, None, None, &cookie);
            }
            if opts.stop_at_blocked_pages && self.blocked_domain(&uri.host)? {
                return done(Status::BlockedPage, &uri, None, None, &cookie);
            }
            let path = uri.path().to_string();
            if ignored.iter().any(|(host, prefix)| {
                *host == uri.host && path.starts_with(if prefix.is_empty() { "/" } else { prefix })
            }) {
                let ret = uri.clone();
                return done(Status::Resolved, &uri, Some(ret), None, &cookie);
            }
            let Some(addr) = self.lookup(&uri).await?.into_iter().next() else {
                return done(Status::InvalidAddress, &uri, None, None, &cookie);
            };
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(opts.timeout)
                .resolve(&uri.host, addr)
                .build()?;
            let mut headers = vec![
                ("User-Agent".to_string(), user_agent.clone()),
                ("Accept".to_string(), "*/*".to_string()),
                ("Accept-Language".to_string(), "*".to_string()),
            ];
            headers.extend(opts.headers.iter().cloned());
            if let Some(c) = &cookie {
                headers.push(("Cookie".into(), c.clone()));
            }
            let method = if force_get {
                reqwest::Method::GET
            } else {
                reqwest::Method::HEAD
            };
            let mut request = client.request(method.clone(), uri.to_url());
            for (name, value) in &headers {
                request = request.header(name.as_str(), value.as_str());
            }
            let response = match tokio::time::timeout(opts.timeout, request.send()).await {
                Err(_) => return done(Status::Timeout, &uri, None, None, &cookie),
                Ok(Err(e)) if e.is_timeout() => {
                    return done(Status::Timeout, &uri, None, None, &cookie);
                }
                Ok(Err(e)) => return Ok(Err(Raised(e.to_string()))),
                Ok(Ok(r)) => r,
            };
            let response_headers = response.headers().clone();
            let status = response.status().as_u16();
            // The status the redirect is read from: small_get's, when it ran.
            let mut response_status = status;
            // The response block: the body (a GET's) within its limits.
            if method == reqwest::Method::GET {
                let started = std::time::Instant::now();
                let mut size = 0;
                let mut response = response;
                loop {
                    match tokio::time::timeout(opts.timeout, response.chunk()).await {
                        Err(_) => return done(Status::Timeout, &uri, None, None, &cookie),
                        Ok(Err(e)) => return Ok(Err(Raised(e.to_string()))),
                        Ok(Ok(None)) => break,
                        Ok(Ok(Some(chunk))) => {
                            size += chunk.len();
                            if size > MAX_REQUEST_SIZE_BYTES {
                                return Ok(Err(Raised(format!(
                                    "response size too big: {}",
                                    uri.to_s()
                                ))));
                            }
                            if started.elapsed() > MAX_REQUEST_TIME {
                                return Ok(Err(Raised(format!(
                                    "connect timeout reached: {}",
                                    uri.to_s()
                                ))));
                            }
                        }
                    }
                }
            }
            let header = |name: &str| {
                response_headers
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            };
            if opts.stop_at_blocked_pages
                && (self.blocked_domain(&uri.host)?
                    || header("Discourse-No-Onebox").as_deref() == Some("1"))
            {
                return done(Status::BlockedPage, &uri, None, None, &cookie);
            }
            let mut cookies: Vec<String> = response_headers
                .get_all("set-cookie")
                .iter()
                .filter_map(|v| v.to_str().ok().map(str::to_string))
                .collect();
            let mut location = header("location");
            match status {
                200 => {
                    let ret = uri.clone();
                    return done(
                        Status::Resolved,
                        &uri,
                        Some(ret),
                        header("content-type"),
                        &cookie,
                    );
                }
                103 | 400 | 405 | 406 | 409 | 500 | 501 => {
                    // small_get: a GET's status and headers, its body unread.
                    let mut request = client.get(uri.to_url());
                    for (name, value) in &headers {
                        request = request.header(name.as_str(), value.as_str());
                    }
                    let small = match tokio::time::timeout(opts.timeout, request.send()).await {
                        Err(_) => return done(Status::Timeout, &uri, None, None, &cookie),
                        Ok(Err(e)) if e.is_timeout() => {
                            return done(Status::Timeout, &uri, None, None, &cookie);
                        }
                        Ok(Err(e)) => return Ok(Err(Raised(e.to_string()))),
                        Ok(Ok(r)) => r,
                    };
                    if opts.stop_at_blocked_pages {
                        let dont = small
                            .headers()
                            .get_all("discourse-no-onebox")
                            .iter()
                            .filter_map(|v| v.to_str().ok())
                            .collect::<String>()
                            == "1";
                        if dont || self.blocked_domain(&uri.host)? {
                            return done(Status::BlockedPage, &uri, None, None, &cookie);
                        }
                    }
                    response_status = small.status().as_u16();
                    if response_status == 200 {
                        let ret = uri.clone();
                        return done(Status::Resolved, &uri, Some(ret), None, &cookie);
                    }
                    cookies = small
                        .headers()
                        .get_all("set-cookie")
                        .iter()
                        .filter_map(|v| v.to_str().ok().map(str::to_string))
                        .collect();
                    location = {
                        let all: Vec<&str> = small
                            .headers()
                            .get_all("location")
                            .iter()
                            .filter_map(|v| v.to_str().ok())
                            .collect();
                        (!all.is_empty()).then(|| all.concat())
                    };
                }
                _ => {}
            }
            let location = location.filter(|_| (300..400).contains(&response_status));
            if !cookies.is_empty() {
                cookie = Some(
                    cookies
                        .iter()
                        .map(|c| c.split(';').next().unwrap_or("").trim().to_string())
                        .collect::<Vec<_>>()
                        .join("; "),
                );
            }
            if let Some(location) = location {
                let next_location = if preserve_fragment
                    && let Some(f) = uri.fragment.as_deref().filter(|f| !f.is_empty())
                {
                    format!("{location}#{f}")
                } else {
                    location.clone()
                };
                let next_location = if next_location.starts_with('/') {
                    format!("{}://{}{}", uri.scheme, uri.host, next_location)
                } else {
                    next_location
                };
                if let Some(redirect) = Uri::parse(&location)
                    && redirect.host == uri.host
                    && (redirect.path().contains("/login") || redirect.path().contains("/session"))
                {
                    let ret = uri.clone();
                    return done(Status::Resolved, &uri, Some(ret), None, &cookie);
                }
                let Some(next) = Uri::parse(&next_location) else {
                    return done(Status::InvalidAddress, &uri, None, None, &cookie);
                };
                if limit == max
                    && ignore_limit
                    && uri.scheme == "http"
                    && next.scheme == "https"
                    && uri.to_s().strip_prefix("http://") == next.to_s().strip_prefix("https://")
                {
                    limit += 1;
                    ignore_limit = false;
                }
                limit -= 1;
                if uri.port == 80 && next.port == 443 && next.scheme == "https" {
                    crate::owned_schema::cached_setex(
                        conn,
                        &format!("HTTPS_DOMAIN_{}", next.host),
                        86_400,
                        "1",
                    )
                    .await?;
                }
                uri = next;
                continue;
            }
            let bot_challenge = header("x-amzn-waf-action").is_some_and(|v| !v.trim().is_empty())
                || header("cf-mitigated").as_deref() == Some("challenge");
            return done(
                Status::Failure {
                    status_code: status,
                    bot_challenge,
                },
                &uri,
                None,
                None,
                &cookie,
            );
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
