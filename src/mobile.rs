//! Mobile or desktop: which layout a page is rendered in.
//!
//! Discourse decides in the browser, from the viewport: `Site#mobileView` is
//! `!capabilities.viewport.sm`, the `(min-width: 40rem)` media query, and
//! the client re-renders when the window crosses it. The stylesheets follow
//! the same breakpoint (`(width < 40rem)` media queries, the plugins'
//! `_mobile` and `_desktop` bundles). The server only guesses, from the user
//! agent (`MobileDetection.mobile_device?`), for what it renders before the
//! client runs: the logo it preloads, the anonymous cache key, the crawler
//! content of a modern phone.
//!
//! The port renders the client's markup on the server, so it needs the
//! viewport before it renders. The browser tells it: the layout's head
//! script (templates/layout.html) writes the `mobile_view` cookie (`1` below
//! 40rem, `0` above) and reloads the page when the server guessed wrong or
//! the window crosses the breakpoint. Without the cookie (a first visit, no
//! script) the server guesses from the user agent as Rails does.
//!
//! [`layer`] decides once per request and scopes the answer to the
//! request's task, where [`mobile_view`] reads it.

use axum::extract::Request;
use axum::http::{HeaderMap, header};
use axum::middleware::Next;
use axum::response::Response;

/// The cookie the layout's head script keeps in step with the viewport.
pub const COOKIE: &str = "mobile_view";

tokio::task_local! {
    static MOBILE_VIEW: bool;
}

/// `MobileDetection.mobile_device?`: a phone's user agent, not an iPad's.
pub fn mobile_device(user_agent: &str) -> bool {
    user_agent.contains("Mobile") && !user_agent.contains("iPad")
}

/// The request's layout: the `mobile_view` cookie the client wrote from its
/// viewport, else the user agent's guess.
pub fn decide(headers: &HeaderMap) -> bool {
    match crate::session::current::cookie(headers, COOKIE) {
        Some("1") => true,
        Some("0") => false,
        _ => mobile_device(
            headers
                .get(header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
        ),
    }
}

/// `site.mobileView` for the request being rendered; desktop outside a
/// request (tests rendering a view directly).
pub fn mobile_view() -> bool {
    MOBILE_VIEW.try_with(|m| *m).unwrap_or(false)
}

/// Runs `f` as a request in the given layout would.
pub async fn scope<F: std::future::Future>(mobile: bool, f: F) -> F::Output {
    MOBILE_VIEW.scope(mobile, f).await
}

/// Decides the request's layout and renders it in that scope.
pub async fn layer(request: Request, next: Next) -> Response {
    let mobile = decide(request.headers());
    MOBILE_VIEW.scope(mobile, next.run(request)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(ua: &str, cookie: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::USER_AGENT, HeaderValue::from_str(ua).unwrap());
        if let Some(c) = cookie {
            h.insert(header::COOKIE, HeaderValue::from_str(c).unwrap());
        }
        h
    }

    const IPHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";
    const IPAD: &str = "Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";
    const DESKTOP: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

    #[test]
    fn the_user_agent_guesses_as_mobile_detection_does() {
        assert!(mobile_device(IPHONE));
        assert!(!mobile_device(IPAD));
        assert!(!mobile_device(DESKTOP));
        assert!(decide(&headers(IPHONE, None)));
        assert!(!decide(&headers(DESKTOP, None)));
    }

    #[test]
    fn the_viewport_cookie_wins_over_the_user_agent() {
        assert!(decide(&headers(DESKTOP, Some("a=b; mobile_view=1"))));
        assert!(!decide(&headers(IPHONE, Some("mobile_view=0"))));
        assert!(decide(&headers(IPHONE, Some("mobile_view=x"))));
    }

    #[tokio::test]
    async fn the_decision_is_scoped_to_the_request() {
        assert!(!mobile_view());
        assert!(scope(true, async { mobile_view() }).await);
        assert!(!scope(false, async { mobile_view() }).await);
    }
}
