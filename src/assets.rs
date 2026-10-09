//! The stylesheets and scripts embedded in the binary, served under
//! /assets/<name> with the browser cache in mind, as Discourse serves its
//! digested assets: each carries a digest of its content, pages link it as
//! `/assets/<name>?v=<digest>` (`url`), and a request for the current
//! digest is cached for good (`immutable`). Any other request (an old page,
//! no `v`) gets `no-cache`, and every response an ETag, so a browser holding
//! the file revalidates for a 304 instead of fetching it again.

use std::collections::HashMap;
use std::sync::OnceLock;

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

/// The ported stylesheets (static/css), in the order Discourse's common
/// stylesheet imports their sources, after normalize.css.
const DISCOURSE_CSS: &str = concat!(
    include_str!("../static/vendor/normalize.css"),
    include_str!("../static/css/foundation.css"),
    include_str!("../static/css/buttons.css"),
    include_str!("../static/css/d-icon.css"),
    include_str!("../static/css/header.css"),
    include_str!("../static/css/navs.css"),
    include_str!("../static/css/welcome-banner.css"),
    include_str!("../static/css/topic-list.css"),
    include_str!("../static/css/categories.css"),
    include_str!("../static/css/sidebar.css"),
    include_str!("../static/css/topic.css"),
    include_str!("../static/css/user.css"),
    include_str!("../static/css/user-stream.css"),
    include_str!("../static/css/composer.css"),
    include_str!("../static/css/menus.css"),
    include_str!("../static/css/not-found.css"),
    include_str!("../static/css/powered-by.css"),
    // The bundled plugins' stylesheets, after core's.
    include_str!("../static/css/topic-voting.css"),
    include_str!("../static/css/discourse-reactions.css"),
    include_str!("../static/css/discourse-solved.css"),
);

const CSS: &str = "text/css; charset=utf-8";
const JS: &str = "text/javascript; charset=utf-8";

/// Every embedded asset: its name under /assets, type and content.
const ASSETS: &[(&str, &str, &str)] = &[
    ("discourse.css", CSS, DISCOURSE_CSS),
    ("site.css", CSS, include_str!("../static/site.css")),
    // htmx and its SSE extension, vendored from npm (static/vendor/README).
    (
        "htmx.min.js",
        JS,
        include_str!("../static/vendor/htmx.min.js"),
    ),
    (
        "htmx-ext-sse.js",
        JS,
        include_str!("../static/vendor/htmx-ext-sse.js"),
    ),
    ("page.js", JS, include_str!("../static/js/page.js")),
    ("sidebar.js", JS, include_str!("../static/js/sidebar.js")),
    ("topic.js", JS, include_str!("../static/js/topic.js")),
    (
        "discourse-reactions.js",
        JS,
        include_str!("../static/js/discourse-reactions.js"),
    ),
    (
        "discourse-solved.js",
        JS,
        include_str!("../static/js/discourse-solved.js"),
    ),
    (
        "topic-voting.js",
        JS,
        include_str!("../static/js/topic-voting.js"),
    ),
    (
        "tracking-menu.js",
        JS,
        include_str!("../static/js/tracking-menu.js"),
    ),
    ("composer.js", JS, include_str!("../static/js/composer.js")),
    (
        "screen-track.js",
        JS,
        include_str!("../static/js/screen-track.js"),
    ),
    // Discourse's frontend/discourse/scripts/js/onpopstate-handler.js
    // (GPL-2.0), which the not-found page loads under Rails' path.
    (
        "js/onpopstate-handler.js",
        JS,
        include_str!("../static/js/onpopstate-handler.js"),
    ),
];

/// The first 16 hex digits of a body's SHA-256.
pub fn digest_of(body: &[u8]) -> String {
    Sha256::digest(body)
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn digests() -> &'static HashMap<&'static str, String> {
    static DIGESTS: OnceLock<HashMap<&'static str, String>> = OnceLock::new();
    DIGESTS.get_or_init(|| {
        ASSETS
            .iter()
            .map(|(name, _, body)| (*name, digest_of(body.as_bytes())))
            .collect()
    })
}

/// An asset's url with its digest, for a page to link.
pub fn url(base_path: &str, name: &str) -> String {
    match digests().get(name) {
        Some(digest) => format!("{base_path}/assets/{name}?v={digest}"),
        None => format!("{base_path}/assets/{name}"),
    }
}

/// The request names this digest in `If-None-Match`.
fn not_modified(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|t| t.trim().trim_start_matches("W/") == etag)
        })
}

/// A response for `body` under `etag`: a 304 when the browser holds it,
/// else the body, cached for good when `immutable`.
pub fn cached(
    headers: &HeaderMap,
    content_type: &'static str,
    body: impl IntoResponse,
    digest: &str,
    immutable: bool,
) -> Response {
    let etag = format!("\"{digest}\"");
    let cache_control = if immutable {
        "max-age=31556952, public, immutable"
    } else {
        "no-cache"
    };
    let mut response = if not_modified(headers, &etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(header::CONTENT_TYPE, content_type)], body).into_response()
    };
    let h = response.headers_mut();
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    if let Ok(etag) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, etag);
    }
    response
}

/// GET /assets/<name>
pub fn serve(name: &str, query: Option<&str>, headers: &HeaderMap) -> Response {
    let Some((_, content_type, body)) = ASSETS.iter().find(|(n, _, _)| *n == name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let digest = &digests()[name];
    let current = query
        .into_iter()
        .flat_map(|q| q.split('&'))
        .any(|pair| pair == format!("v={digest}"));
    cached(headers, content_type, *body, digest, current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_digest_is_cached_for_good_and_others_revalidate() {
        let url = url("", "topic.js");
        let query = url.split_once('?').map(|(_, q)| q);
        let fresh = serve("topic.js", query, &HeaderMap::new());
        assert_eq!(fresh.status(), StatusCode::OK);
        assert!(
            fresh.headers()[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("immutable")
        );

        let stale = serve("topic.js", Some("v=0"), &HeaderMap::new());
        assert_eq!(stale.headers()[header::CACHE_CONTROL], "no-cache");

        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, stale.headers()[header::ETAG].clone());
        assert_eq!(
            serve("topic.js", None, &headers).status(),
            StatusCode::NOT_MODIFIED
        );
        assert_eq!(
            serve("nope.js", None, &headers).status(),
            StatusCode::NOT_FOUND
        );
    }
}
