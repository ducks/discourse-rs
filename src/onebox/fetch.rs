//! The fetches the onebox engines make (the boundary the parity goldens
//! stub): `Onebox::Helpers.fetch_response` (FinalDestination::HTTP, its
//! redirects, cookies and size limit), `fetch_content_length`, OpenURI's
//! `URI#open`, and `FinalDestination#resolve` with Oneboxer's options,
//! which `fetch_html_doc` follows a canonical link with. [`Live`] makes
//! them on the network; [`Fixtures`] answers them from recorded pages.

use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant};

use sqlx::PgConnection;

use super::uri::RubyUri;
use crate::final_destination::{Fetcher, ResolveOptions, Status, Uri};
use crate::{AppError, Unsupported};

/// `Onebox.options.max_download_kb`
const MAX_DOWNLOAD_BYTES: usize = 2048 * 1024;
/// `Onebox.options.redirect_limit`, as Discourse sets it.
const REDIRECT_LIMIT: u32 = 3;
/// `Onebox.options.connect_timeout` and `timeout`.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TIMEOUT: Duration = Duration::from_secs(10);

/// What a fetch raised.
#[derive(Debug)]
pub enum FetchError {
    /// One of Onebox::Preview's WEB_EXCEPTIONS (Net::HTTPError, a 4xx's
    /// HTTPClientException, OpenURI::HTTPError, a timeout, a refused
    /// connection): the preview is blank.
    Web(String),
    /// Anything else (a 5xx's HTTPFatalError, a lookup the SSRF guard
    /// refuses, a body too large): Oneboxer gives up on the onebox.
    Other(String),
    /// What this port doesn't read the way Ruby does (a body that isn't
    /// UTF-8).
    Refused(Unsupported),
}

/// `fetch_response`'s keyword arguments.
#[derive(Debug, Clone)]
pub struct FetchOpts {
    pub headers: Vec<(String, String)>,
    pub redirect_limit: u32,
    pub raise_error_when_response_too_large: bool,
    pub allow_cross_domain_cookies: bool,
}

impl Default for FetchOpts {
    fn default() -> FetchOpts {
        FetchOpts {
            headers: Vec::new(),
            redirect_limit: 5,
            raise_error_when_response_too_large: true,
            allow_cross_domain_cookies: false,
        }
    }
}

/// The fetches an engine makes.
pub trait Fetch {
    /// `Onebox::Helpers.fetch_response(url, ...)`
    fn fetch_response(
        &mut self,
        url: &str,
        opts: &FetchOpts,
    ) -> impl Future<Output = Result<Result<String, FetchError>, AppError>> + Send;

    /// `FinalDestination.new(url, Oneboxer.get_final_destination_options(url)).resolve`:
    /// where the URL leads, None when it doesn't resolve.
    fn resolve(
        &mut self,
        url: &str,
    ) -> impl Future<Output = Result<Option<String>, AppError>> + Send;

    /// `URI.parse(url).open(headers).read` (OpenURI).
    fn open(
        &mut self,
        url: &str,
        headers: &[(String, String)],
    ) -> impl Future<Output = Result<Result<String, FetchError>, AppError>> + Send;

    /// `Onebox::Helpers.fetch_content_length(url)`
    fn content_length(
        &mut self,
        url: &str,
    ) -> impl Future<Output = Result<Result<Option<String>, FetchError>, AppError>> + Send;

    /// `Discourse::GithubApi.for(token:)`'s `get` or `raw_get`: the
    /// body; an error status (or the backoff) is the OpenURI::HTTPError
    /// the onebox mixin turns it into.
    fn github(
        &mut self,
        url: &str,
        token: Option<&str>,
    ) -> impl Future<Output = Result<Result<String, FetchError>, AppError>> + Send;

    /// `FinalDestination::HTTP.start(host, port).head(uri.path)`: the
    /// status and Location.
    fn head(
        &mut self,
        url: &str,
    ) -> impl Future<Output = Result<Result<(u16, Option<String>), FetchError>, AppError>> + Send;
}

/// What a response's status raises when it is neither the page nor a
/// redirect (`response.error!`).
fn status_error(code: u16) -> FetchError {
    let message = format!("{code}");
    match code {
        400..=499 => FetchError::Web(message),
        // HTTPFatalError and HTTPRetriableError aren't Net::HTTPError.
        300..=399 | 500..=599 => FetchError::Other(message),
        _ => FetchError::Web(message),
    }
}

fn client_error(e: &reqwest::Error) -> FetchError {
    if e.is_timeout() || e.is_connect() {
        FetchError::Web(e.to_string())
    } else {
        FetchError::Other(e.to_string())
    }
}

/// The fetches made on the network, through the SSRF guard.
pub struct Live<'a> {
    pub conn: &'a mut PgConnection,
    pub fetcher: Fetcher<'a>,
    /// `Onebox::Helpers.user_agent`
    pub user_agent: String,
    /// `Oneboxer.accept_language`
    pub accept_language: String,
    /// `Oneboxer.get_final_destination_options` under the default
    /// strategy; a host whose preferred strategy is force_get_and_ua gets
    /// its own.
    pub resolve_options: ResolveOptions,
}

impl Live<'_> {
    /// A client that connects only to an address the SSRF guard allows,
    /// without checking certificates (`verify_mode = VERIFY_NONE`).
    async fn client(&self, uri: &Uri) -> Result<Result<reqwest::Client, FetchError>, AppError> {
        let addrs = self.fetcher.lookup(uri).await?;
        let Some(addr) = addrs.first() else {
            return Ok(Err(FetchError::Other(format!(
                "FinalDestination: all resolved IPs were disallowed: {}",
                uri.host
            ))));
        };
        Ok(Ok(reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(TIMEOUT)
            .danger_accept_invalid_certs(true)
            .resolve(&uri.host, *addr)
            .build()?))
    }
}

fn utf8(body: Vec<u8>) -> Result<String, FetchError> {
    String::from_utf8(body)
        .map_err(|_| FetchError::Refused(Unsupported("onebox pages that aren't UTF-8")))
}

fn header(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

impl Fetch for Live<'_> {
    async fn fetch_response(
        &mut self,
        url: &str,
        opts: &FetchOpts,
    ) -> Result<Result<String, FetchError>, AppError> {
        let mut limit = opts.redirect_limit.min(REDIRECT_LIMIT);
        let mut location = url.to_string();
        let mut domain: Option<String> = None;
        let mut headers: Option<Vec<(String, String)>> = Some(opts.headers.clone());
        loop {
            if limit == 0 {
                return Ok(Err(FetchError::Web("HTTP redirect too deep".into())));
            }
            // Addressable::URI.join(domain, uri) when it has no host.
            let absolute = match RubyUri::parse(&location) {
                Some(u) if u.host.is_some() => Some(u),
                Some(_) => domain
                    .as_deref()
                    .and_then(RubyUri::parse)
                    .and_then(|d| d.merge(&location)),
                None => None,
            };
            let Some(uri) = absolute.and_then(|u| Uri::parse(&u.to_s())) else {
                return Ok(Err(FetchError::Other(format!("bad location: {location}"))));
            };
            let client = match self.client(&uri).await? {
                Ok(c) => c,
                Err(e) => return Ok(Err(e)),
            };
            let mut request_headers = headers.clone().unwrap_or_default();
            if !request_headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("User-Agent"))
            {
                request_headers.push(("User-Agent".into(), self.user_agent.clone()));
            }
            if !request_headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("Accept-Language"))
            {
                request_headers.push(("Accept-Language".into(), self.accept_language.clone()));
            }
            let mut request = client.get(uri.to_s().split('#').next().unwrap_or("").to_string());
            for (k, v) in &request_headers {
                request = request.header(k.as_str(), v.as_str());
            }
            let started = Instant::now();
            let response = match tokio::time::timeout(TIMEOUT, request.send()).await {
                Err(_) => return Ok(Err(FetchError::Web("timeout".into()))),
                Ok(Err(e)) => return Ok(Err(client_error(&e))),
                Ok(Ok(r)) => r,
            };
            let set_cookies: Vec<String> = response
                .headers()
                .get_all("set-cookie")
                .iter()
                .filter_map(|v| v.to_str().ok())
                .map(|c| c.split(';').next().unwrap_or("").trim().to_string())
                .filter(|c| !c.is_empty())
                .collect();
            let mut redirect_header: Option<Vec<(String, String)>> = None;
            if response.headers().contains_key("set-cookie") {
                let cookie = set_cookies.join("; ");
                if opts.allow_cross_domain_cookies {
                    request_headers.retain(|(k, _)| !k.eq_ignore_ascii_case("Cookie"));
                    request_headers.push(("Cookie".into(), cookie.clone()));
                }
                if let Some(next) = header(&response, "location") {
                    let host = RubyUri::parse(&next).and_then(|u| u.host);
                    if host.is_none() || host.as_deref() == Some(uri.host.as_str()) {
                        redirect_header = Some(vec![("Cookie".into(), cookie)]);
                    }
                }
            }
            let code = response.status().as_u16();
            if code != 200 {
                if ![301, 302, 303, 307, 308].contains(&code) {
                    return Ok(Err(status_error(code)));
                }
                let Some(next) = header(&response, "location") else {
                    return Ok(Err(FetchError::Other("redirect without a location".into())));
                };
                location = next;
                domain = Some(format!("{}://{}", uri.scheme, uri.host));
                headers = if opts.allow_cross_domain_cookies {
                    Some(request_headers)
                } else {
                    redirect_header
                };
                limit -= 1;
                continue;
            }
            let mut body = Vec::new();
            let mut response = response;
            loop {
                match tokio::time::timeout(TIMEOUT, response.chunk()).await {
                    Err(_) => return Ok(Err(FetchError::Web("timeout".into()))),
                    Ok(Err(e)) => return Ok(Err(client_error(&e))),
                    Ok(Ok(None)) => break,
                    Ok(Ok(Some(chunk))) => {
                        body.extend_from_slice(&chunk);
                        if body.len() > MAX_DOWNLOAD_BYTES {
                            if opts.raise_error_when_response_too_large {
                                return Ok(Err(FetchError::Other("DownloadTooLarge".into())));
                            }
                            break;
                        }
                        if started.elapsed() > TIMEOUT {
                            return Ok(Err(FetchError::Web("timeout".into())));
                        }
                    }
                }
            }
            return Ok(utf8(body));
        }
    }

    async fn resolve(&mut self, url: &str) -> Result<Option<String>, AppError> {
        let mut opts = self.resolve_options.clone();
        if let Some(uri) = Uri::parse(url)
            && crate::owned_schema::cached_get(
                self.conn,
                &format!("ONEBOXER_STRATEGY_{}", uri.host),
            )
            .await?
            .as_deref()
                == Some("force_get_and_ua")
        {
            opts.force_get_hosts = vec![format!("https://{}", uri.host)];
            opts.force_custom_user_agent_hosts = vec![format!("https://{}", uri.host)];
        }
        Ok(match self.fetcher.resolve(self.conn, url, &opts).await? {
            Ok(r) if r.status == Status::Resolved => r.uri.map(|u| u.to_s()),
            _ => None,
        })
    }

    async fn open(
        &mut self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<Result<String, FetchError>, AppError> {
        let mut location = url.to_string();
        for _ in 0..64 {
            let Some(uri) = Uri::parse(&location) else {
                return Ok(Err(FetchError::Other(format!("bad URI: {location}"))));
            };
            let client = match self.client(&uri).await? {
                Ok(c) => c,
                Err(e) => return Ok(Err(e)),
            };
            let mut request = client.get(uri.to_s().split('#').next().unwrap_or("").to_string());
            for (k, v) in headers {
                request = request.header(k.as_str(), v.as_str());
            }
            let response = match tokio::time::timeout(TIMEOUT, request.send()).await {
                Err(_) => return Ok(Err(FetchError::Web("timeout".into()))),
                Ok(Err(e)) => return Ok(Err(client_error(&e))),
                Ok(Ok(r)) => r,
            };
            let code = response.status().as_u16();
            if response.status().is_redirection()
                && let Some(next) = header(&response, "location")
            {
                let next = RubyUri::parse(&location)
                    .and_then(|u| u.merge(&next))
                    .map(|u| u.to_s())
                    .unwrap_or(next);
                // OpenURI refuses a redirect from https to http.
                if uri.scheme == "https" && next.starts_with("http:") {
                    return Ok(Err(FetchError::Other("redirection forbidden".into())));
                }
                location = next;
                continue;
            }
            if !(200..300).contains(&code) {
                return Ok(Err(FetchError::Web(format!("{code}"))));
            }
            return Ok(match response.bytes().await {
                Ok(b) => utf8(b.to_vec()),
                Err(e) => Err(client_error(&e)),
            });
        }
        Ok(Err(FetchError::Other("too many redirects".into())))
    }

    async fn content_length(
        &mut self,
        url: &str,
    ) -> Result<Result<Option<String>, FetchError>, AppError> {
        let Some(uri) = Uri::parse(url) else {
            return Ok(Err(FetchError::Other(format!("bad URI: {url}"))));
        };
        let client = match self.client(&uri).await? {
            Ok(c) => c,
            Err(e) => return Ok(Err(e)),
        };
        let request = client.head(uri.to_s().split('#').next().unwrap_or("").to_string());
        let response = match tokio::time::timeout(TIMEOUT, request.send()).await {
            Err(_) => return Ok(Err(FetchError::Web("timeout".into()))),
            Ok(Err(e)) => return Ok(Err(client_error(&e))),
            Ok(Ok(r)) => r,
        };
        if response.status().as_u16() != 200 {
            return Ok(Ok(None));
        }
        Ok(Ok(
            header(&response, "content-length").filter(|v| !v.trim().is_empty())
        ))
    }

    async fn github(
        &mut self,
        url: &str,
        token: Option<&str>,
    ) -> Result<Result<String, FetchError>, AppError> {
        let Some(uri) = Uri::parse(url) else {
            return Ok(Err(FetchError::Other(format!("bad URI: {url}"))));
        };
        // Discourse::GithubApi::ALLOWED_HOSTS
        if !matches!(
            uri.host.as_str(),
            "api.github.com" | "raw.githubusercontent.com"
        ) {
            return Ok(Err(FetchError::Other(format!(
                "Refusing to call non-GitHub host: {}",
                uri.host
            ))));
        }
        let client = match self.client(&uri).await? {
            Ok(c) => c,
            Err(e) => return Ok(Err(e)),
        };
        let mut request = client
            .get(uri.to_s().split('#').next().unwrap_or("").to_string())
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", crate::schema::user_agent());
        if let Some(t) = token.filter(|t| !t.trim().is_empty()) {
            request = request.header("Authorization", format!("Bearer {t}"));
        }
        let response = match tokio::time::timeout(Duration::from_secs(5), request.send()).await {
            Err(_) => return Ok(Err(FetchError::Web("GitHub API request failed".into()))),
            Ok(Err(e)) => return Ok(Err(FetchError::Web(e.to_string()))),
            Ok(Ok(r)) => r,
        };
        if !response.status().is_success() {
            return Ok(Err(FetchError::Web(format!(
                "GitHub API error (status {})",
                response.status().as_u16()
            ))));
        }
        Ok(match response.bytes().await {
            Ok(b) => utf8(b.to_vec()),
            Err(e) => Err(FetchError::Web(e.to_string())),
        })
    }

    async fn head(
        &mut self,
        url: &str,
    ) -> Result<Result<(u16, Option<String>), FetchError>, AppError> {
        let Some(uri) = Uri::parse(url) else {
            return Ok(Err(FetchError::Other(format!("bad URI: {url}"))));
        };
        let client = match self.client(&uri).await? {
            Ok(c) => c,
            Err(e) => return Ok(Err(e)),
        };
        // `http.head(uri.path)`: the path alone, its query left off.
        let default =
            (uri.scheme == "http" && uri.port == 80) || (uri.scheme == "https" && uri.port == 443);
        let target = if default {
            format!("{}://{}{}", uri.scheme, uri.host, uri.path())
        } else {
            format!("{}://{}:{}{}", uri.scheme, uri.host, uri.port, uri.path())
        };
        let response = match tokio::time::timeout(TIMEOUT, client.head(target).send()).await {
            Err(_) => return Ok(Err(FetchError::Web("timeout".into()))),
            Ok(Err(e)) => return Ok(Err(client_error(&e))),
            Ok(Ok(r)) => r,
        };
        Ok(Ok((
            response.status().as_u16(),
            header(&response, "location"),
        )))
    }
}

/// A recorded answer for a URL.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct Fixture {
    /// The body, from a file under parity/onebox_engines/fixtures/.
    #[serde(default)]
    pub file: Option<String>,
    /// The status when it isn't 200: a fetch of it raises as Ruby does.
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub content_length: Option<String>,
    /// Where FinalDestination resolves it (the URL itself by default;
    /// "" when it doesn't resolve).
    #[serde(default)]
    pub resolves_to: Option<String>,
    /// The Location a HEAD request answers with.
    #[serde(default)]
    pub location: Option<String>,
}

/// The fetches answered from recorded pages: an unknown URL raises as a
/// refused connection would.
pub struct Fixtures {
    pub pages: HashMap<String, Fixture>,
    pub dir: std::path::PathBuf,
}

impl Fixtures {
    /// The page a request asks for: the URL without its fragment.
    fn page(&self, url: &str) -> Option<&Fixture> {
        self.pages.get(url.split('#').next().unwrap_or(url))
    }

    fn body(&self, url: &str) -> Result<String, FetchError> {
        let Some(page) = self.page(url) else {
            return Err(FetchError::Web(format!("connection refused: {url}")));
        };
        if let Some(code) = page.status.filter(|c| *c != 200) {
            return Err(status_error(code));
        }
        let Some(file) = &page.file else {
            return Ok(String::new());
        };
        std::fs::read(self.dir.join(file))
            .map_err(|e| FetchError::Other(e.to_string()))
            .and_then(utf8)
    }
}

impl Fetch for Fixtures {
    async fn fetch_response(
        &mut self,
        url: &str,
        _opts: &FetchOpts,
    ) -> Result<Result<String, FetchError>, AppError> {
        Ok(self.body(url))
    }

    async fn resolve(&mut self, url: &str) -> Result<Option<String>, AppError> {
        Ok(match self.page(url).and_then(|p| p.resolves_to.clone()) {
            Some(to) if to.is_empty() => None,
            Some(to) => Some(to),
            None => Some(url.to_string()),
        })
    }

    async fn open(
        &mut self,
        url: &str,
        _headers: &[(String, String)],
    ) -> Result<Result<String, FetchError>, AppError> {
        Ok(self.body(url).map_err(|e| match e {
            // Any status is OpenURI::HTTPError.
            FetchError::Other(m) if m.parse::<u16>().is_ok() => FetchError::Web(m),
            e => e,
        }))
    }

    async fn content_length(
        &mut self,
        url: &str,
    ) -> Result<Result<Option<String>, FetchError>, AppError> {
        let Some(page) = self.page(url) else {
            return Ok(Err(FetchError::Web(format!("connection refused: {url}"))));
        };
        Ok(Ok((page.status.unwrap_or(200) == 200)
            .then(|| page.content_length.clone())
            .flatten()))
    }

    async fn github(
        &mut self,
        url: &str,
        _token: Option<&str>,
    ) -> Result<Result<String, FetchError>, AppError> {
        Ok(self.body(url).map_err(|e| match e {
            FetchError::Refused(u) => FetchError::Refused(u),
            // Every GitHub API error is the mixin's OpenURI::HTTPError.
            FetchError::Web(m) | FetchError::Other(m) => FetchError::Web(m),
        }))
    }

    async fn head(
        &mut self,
        url: &str,
    ) -> Result<Result<(u16, Option<String>), FetchError>, AppError> {
        // `head(uri.path)`: the query isn't asked for.
        let url = url.split(['?', '#']).next().unwrap_or(url);
        let Some(page) = self.page(url) else {
            return Ok(Err(FetchError::Web(format!("connection refused: {url}"))));
        };
        Ok(Ok((page.status.unwrap_or(200), page.location.clone())))
    }
}
