//! ChatChannelListPreferences' bypassed filters: a section's "show all
//! channels" (the filtered empty state's button, the filter toggle) sets
//! the member's saved filter aside without changing it, until the toggle
//! applies it again or a filter is chosen from the menu.
//!
//! Ember keeps the bypassed sections in the service, so they last until
//! the app is loaded again. The lists are drawn here, so
//! static/js/chat-channel-menus.js keeps them in a session cookie (the
//! section names joined by dots) that [`layer`] reads for the request's
//! task: they last for the browser's session, across reloads and tabs.

use axum::extract::Request;
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;

pub const COOKIE: &str = "chat_list_filter_bypass";

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Bypassed {
    channels: bool,
    starred: bool,
    dms: bool,
}

impl Bypassed {
    pub fn from_headers(headers: &HeaderMap) -> Bypassed {
        let mut out = Bypassed::default();
        for section in crate::session::current::cookie(headers, COOKIE)
            .unwrap_or_default()
            .split('.')
        {
            match section {
                "channels" => out.channels = true,
                "starred" => out.starred = true,
                "dms" => out.dms = true,
                _ => {}
            }
        }
        out
    }
}

tokio::task_local! {
    static BYPASSED: Bypassed;
}

/// `isFilterBypassedFor(section)` for the request being rendered; never
/// outside a request.
pub fn bypassed(section: &str) -> bool {
    let b = BYPASSED.try_with(|b| *b).unwrap_or_default();
    match section {
        "channels" => b.channels,
        "starred" => b.starred,
        "dms" => b.dms,
        _ => false,
    }
}

/// The request's bypassed sections, to carry into work that outlives the
/// request's task (the live stream's body, rendered as it is polled).
pub fn current() -> Bypassed {
    BYPASSED.try_with(|b| *b).unwrap_or_default()
}

/// Runs `f` with `bypassed` as the request's sections.
pub async fn scope<F: std::future::Future>(bypassed: Bypassed, f: F) -> F::Output {
    BYPASSED.scope(bypassed, f).await
}

/// Reads the cookie once per request for the lists it renders.
pub async fn layer(request: Request, next: Next) -> Response {
    let bypassed = Bypassed::from_headers(request.headers());
    BYPASSED.scope(bypassed, next.run(request)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, header};

    fn read(cookie: &str) -> Bypassed {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
        Bypassed::from_headers(&h)
    }

    #[test]
    fn the_cookie_names_the_bypassed_sections() {
        assert_eq!(read("other=1"), Bypassed::default());
        assert_eq!(
            read("chat_list_filter_bypass=dms.starred; other=1"),
            Bypassed {
                channels: false,
                starred: true,
                dms: true
            }
        );
        assert_eq!(
            read("chat_list_filter_bypass=channels.nothing."),
            Bypassed {
                channels: true,
                starred: false,
                dms: false
            }
        );
    }
}
