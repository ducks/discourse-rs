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
    include_str!("../static/css/menu-panel.css"),
    include_str!("../static/css/navs.css"),
    include_str!("../static/css/welcome-banner.css"),
    include_str!("../static/css/topic-list.css"),
    include_str!("../static/css/categories.css"),
    include_str!("../static/css/sidebar.css"),
    include_str!("../static/css/cooked.css"),
    include_str!("../static/css/lightbox.css"),
    include_str!("../static/css/onebox.css"),
    include_str!("../static/css/topic.css"),
    include_str!("../static/css/user.css"),
    include_str!("../static/css/preferences.css"),
    include_str!("../static/css/user-notifications.css"),
    include_str!("../static/css/user-activity.css"),
    include_str!("../static/css/badges.css"),
    include_str!("../static/css/user-stream.css"),
    include_str!("../static/css/user-card.css"),
    include_str!("../static/css/composer.css"),
    include_str!("../static/css/kit.css"),
    include_str!("../static/css/modal.css"),
    include_str!("../static/css/date-picker.css"),
    include_str!("../static/css/form-kit.css"),
    include_str!("../static/css/d-multi-select.css"),
    include_str!("../static/css/menus.css"),
    include_str!("../static/css/directory.css"),
    include_str!("../static/css/groups.css"),
    include_str!("../static/css/search.css"),
    include_str!("../static/css/search-menu.css"),
    include_str!("../static/css/tooltip.css"),
    include_str!("../static/css/filter-controls.css"),
    include_str!("../static/css/theme-card.css"),
    include_str!("../static/css/not-found.css"),
    include_str!("../static/css/about.css"),
    include_str!("../static/css/d-page-header.css"),
    include_str!("../static/css/admin-kit.css"),
    include_str!("../static/css/preferences-more.css"),
    include_str!("../static/css/preferences-security.css"),
    include_str!("../static/css/file-uploader.css"),
    include_str!("../static/css/date-time-input.css"),
    include_str!("../static/css/segmented-control.css"),
    include_str!("../static/css/powered-by.css"),
    include_str!("../static/css/review.css"),
    // The bundled plugins' stylesheets, after core's.
    include_str!("../static/css/topic-voting.css"),
    include_str!("../static/css/discourse-reactions.css"),
    include_str!("../static/css/discourse-solved.css"),
    include_str!("../static/css/discourse-presence.css"),
    include_str!("../static/css/chat.css"),
    include_str!("../static/css/checklist.css"),
    include_str!("../static/css/discourse-details.css"),
    include_str!("../static/css/discourse-local-dates.css"),
    include_str!("../static/css/footnote.css"),
    include_str!("../static/css/spoiler-alert.css"),
    include_str!("../static/css/invites.css"),
    // The mobile view's rules, last as each follows its base rule.
    include_str!("../static/css/mobile.css"),
    // common/base/_mcp.scss compiles after table-layout.scss, whose mobile
    // rules its own (the access table's controls) override.
    include_str!("../static/css/admin-mcp.css"),
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
    // Ace, as Discourse's ace-editor-bundle imports it (static/vendor/README):
    // the editor, the modes and themes the admin's code editors use, search.
    (
        "ace-editor-bundle.js",
        JS,
        concat!(
            include_str!("../static/vendor/ace/ace.js"),
            "\n",
            include_str!("../static/vendor/ace/mode-scss.js"),
            "\n",
            include_str!("../static/vendor/ace/mode-html.js"),
            "\n",
            include_str!("../static/vendor/ace/mode-yaml.js"),
            "\n",
            include_str!("../static/vendor/ace/mode-sql.js"),
            "\n",
            include_str!("../static/vendor/ace/mode-javascript.js"),
            "\n",
            include_str!("../static/vendor/ace/mode-text.js"),
            "\n",
            include_str!("../static/vendor/ace/theme-chaos.js"),
            "\n",
            include_str!("../static/vendor/ace/theme-chrome.js"),
            "\n",
            include_str!("../static/vendor/ace/ext-searchbox.js"),
        ),
    ),
    // @floating-ui/dom, which float-kit positions with (static/vendor/README).
    (
        "floating-ui.core.umd.min.js",
        JS,
        include_str!("../static/vendor/floating-ui.core.umd.min.js"),
    ),
    (
        "floating-ui.dom.umd.min.js",
        JS,
        include_str!("../static/vendor/floating-ui.dom.umd.min.js"),
    ),
    ("page.js", JS, include_str!("../static/js/page.js")),
    ("dates.js", JS, include_str!("../static/js/dates.js")),
    ("float.js", JS, include_str!("../static/js/float.js")),
    (
        "auth-complete.js",
        JS,
        include_str!("../static/js/auth-complete.js"),
    ),
    (
        "user-card.js",
        JS,
        include_str!("../static/js/user-card.js"),
    ),
    (
        "inline-footnotes.js",
        JS,
        include_str!("../static/js/inline-footnotes.js"),
    ),
    (
        "local-dates.js",
        JS,
        include_str!("../static/js/local-dates.js"),
    ),
    (
        "local-dates-create.js",
        JS,
        include_str!("../static/js/local-dates-create.js"),
    ),
    // Pikaday as Discourse patches it (static/vendor/README), which
    // local-dates-create.js loads for its calendar.
    (
        "pikaday.js",
        JS,
        include_str!("../static/vendor/pikaday/pikaday.js"),
    ),
    // moment and moment-timezone as Discourse builds them in
    // (static/vendor/README), which local-dates.js loads when a page has a
    // date.
    (
        "moment.min.js",
        JS,
        include_str!("../static/vendor/moment/moment.min.js"),
    ),
    (
        "moment-timezone-with-data-10-year-range.min.js",
        JS,
        include_str!("../static/vendor/moment/moment-timezone-with-data-10-year-range.min.js"),
    ),
    ("hashtags.js", JS, include_str!("../static/js/hashtags.js")),
    ("tag-info.js", JS, include_str!("../static/js/tag-info.js")),
    ("oneboxes.js", JS, include_str!("../static/js/oneboxes.js")),
    ("cooked.js", JS, include_str!("../static/js/cooked.js")),
    ("lightbox.js", JS, include_str!("../static/js/lightbox.js")),
    // PhotoSwipe's ES modules, vendored from npm (static/vendor/README),
    // which lightbox.js imports when a post has lightboxes.
    (
        "photoswipe-lightbox.esm.min.js",
        JS,
        include_str!("../static/vendor/photoswipe-lightbox.esm.min.js"),
    ),
    (
        "photoswipe.esm.min.js",
        JS,
        include_str!("../static/vendor/photoswipe.esm.min.js"),
    ),
    (
        "highlight-syntax.js",
        JS,
        include_str!("../static/js/highlight-syntax.js"),
    ),
    // highlight.js's core as an ES module, which highlight-syntax.js
    // imports (static/vendor/README).
    (
        "highlightjs-core.min.js",
        JS,
        include_str!("../static/vendor/highlightjs/core.min.js"),
    ),
    ("modal.js", JS, include_str!("../static/js/modal.js")),
    (
        "spoiler-alert.js",
        JS,
        include_str!("../static/js/spoiler-alert.js"),
    ),
    (
        "post-decorations.js",
        JS,
        include_str!("../static/js/post-decorations.js"),
    ),
    (
        "codeblock-buttons.js",
        JS,
        include_str!("../static/js/codeblock-buttons.js"),
    ),
    (
        "quote-controls.js",
        JS,
        include_str!("../static/js/quote-controls.js"),
    ),
    (
        "quote-button.js",
        JS,
        include_str!("../static/js/quote-button.js"),
    ),
    ("sidebar.js", JS, include_str!("../static/js/sidebar.js")),
    ("invite.js", JS, include_str!("../static/js/invite.js")),
    (
        "user-invited.js",
        JS,
        include_str!("../static/js/user-invited.js"),
    ),
    // The admin pages: Rails' compiled admin stylesheet, which it serves to
    // staff (static/vendor/README), the admin shell's and the dashboard's
    // scripts, and Chart.js with its moment adapter, which Discourse's
    // chart-js-bundle loads for the dashboard's charts.
    ("admin.css", CSS, include_str!("../static/vendor/admin.css")),
    ("admin.js", JS, include_str!("../static/js/admin.js")),
    (
        "admin-dashboard.js",
        JS,
        include_str!("../static/js/admin-dashboard.js"),
    ),
    (
        "admin-users.js",
        JS,
        include_str!("../static/js/admin-users.js"),
    ),
    (
        "admin-user.js",
        JS,
        include_str!("../static/js/admin-user.js"),
    ),
    (
        "site-settings.js",
        JS,
        include_str!("../static/js/site-settings.js"),
    ),
    (
        "admin-reports.js",
        JS,
        include_str!("../static/js/admin-reports.js"),
    ),
    (
        "admin-logs.js",
        JS,
        include_str!("../static/js/admin-logs.js"),
    ),
    ("admin-ui.js", JS, include_str!("../static/js/admin-ui.js")),
    (
        "admin-backups.js",
        JS,
        include_str!("../static/js/admin-backups.js"),
    ),
    (
        "admin-api-keys.js",
        JS,
        include_str!("../static/js/admin-api-keys.js"),
    ),
    (
        "admin-report-charts.js",
        JS,
        include_str!("../static/js/admin-report-charts.js"),
    ),
    (
        "admin-report.js",
        JS,
        include_str!("../static/js/admin-report.js"),
    ),
    (
        "admin-config.js",
        JS,
        include_str!("../static/js/admin-config.js"),
    ),
    (
        "admin-emoji.js",
        JS,
        include_str!("../static/js/admin-emoji.js"),
    ),
    (
        "admin-branding.js",
        JS,
        include_str!("../static/js/admin-branding.js"),
    ),
    (
        "popular-themes.js",
        JS,
        include_str!("../static/js/popular-themes.js"),
    ),
    (
        "admin-customize.js",
        JS,
        include_str!("../static/js/admin-customize.js"),
    ),
    (
        "admin-category-management.js",
        JS,
        include_str!("../static/js/admin-category-management.js"),
    ),
    (
        "date-range-picker.js",
        JS,
        include_str!("../static/js/date-range-picker.js"),
    ),
    (
        "admin-mcp.js",
        JS,
        include_str!("../static/js/admin-mcp.js"),
    ),
    (
        "admin-search.js",
        JS,
        include_str!("../static/js/admin-search.js"),
    ),
    (
        "admin-upcoming-changes.js",
        JS,
        include_str!("../static/js/admin-upcoming-changes.js"),
    ),
    (
        "admin-whats-new.js",
        JS,
        include_str!("../static/js/admin-whats-new.js"),
    ),
    (
        "admin-web-hooks.js",
        JS,
        include_str!("../static/js/admin-web-hooks.js"),
    ),
    (
        "admin-email.js",
        JS,
        include_str!("../static/js/admin-email.js"),
    ),
    (
        "admin-problem-checks.js",
        JS,
        include_str!("../static/js/admin-problem-checks.js"),
    ),
    (
        "admin-badges.js",
        JS,
        include_str!("../static/js/admin-badges.js"),
    ),
    (
        "admin-plugins.js",
        JS,
        include_str!("../static/js/admin-plugins.js"),
    ),
    ("dialog.js", JS, include_str!("../static/js/dialog.js")),
    (
        "chart.umd.min.js",
        JS,
        include_str!("../static/vendor/chart.umd.min.js"),
    ),
    (
        "chartjs-adapter-moment.min.js",
        JS,
        include_str!("../static/vendor/chartjs-adapter-moment.min.js"),
    ),
    (
        "navigation-bar.js",
        JS,
        include_str!("../static/js/navigation-bar.js"),
    ),
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
    ("chat.js", JS, include_str!("../static/js/chat.js")),
    (
        "emoji-picker.js",
        JS,
        include_str!("../static/js/emoji-picker.js"),
    ),
    (
        "autocomplete.js",
        JS,
        include_str!("../static/js/autocomplete.js"),
    ),
    ("presence.js", JS, include_str!("../static/js/presence.js")),
    (
        "topic-presence.js",
        JS,
        include_str!("../static/js/topic-presence.js"),
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
    (
        "messages-dropdown.js",
        JS,
        include_str!("../static/js/messages-dropdown.js"),
    ),
    (
        "users-directory.js",
        JS,
        include_str!("../static/js/users-directory.js"),
    ),
    (
        "user-menu.js",
        JS,
        include_str!("../static/js/user-menu.js"),
    ),
    (
        "user-notifications.js",
        JS,
        include_str!("../static/js/user-notifications.js"),
    ),
    (
        "preferences.js",
        JS,
        include_str!("../static/js/preferences.js"),
    ),
    (
        "prefs-choosers.js",
        JS,
        include_str!("../static/js/prefs-choosers.js"),
    ),
    (
        "prefs-security.js",
        JS,
        include_str!("../static/js/prefs-security.js"),
    ),
    (
        "notifications-page.js",
        JS,
        include_str!("../static/js/notifications-page.js"),
    ),
    ("review.js", JS, include_str!("../static/js/review.js")),
    (
        "user-activity.js",
        JS,
        include_str!("../static/js/user-activity.js"),
    ),
    (
        "overflow-controls.js",
        JS,
        include_str!("../static/js/overflow-controls.js"),
    ),
    ("groups.js", JS, include_str!("../static/js/groups.js")),
    (
        "category-edit.js",
        JS,
        include_str!("../static/js/category-edit.js"),
    ),
    (
        "search-page.js",
        JS,
        include_str!("../static/js/search-page.js"),
    ),
    (
        "search-advanced.js",
        JS,
        include_str!("../static/js/search-advanced.js"),
    ),
    (
        "search-menu.js",
        JS,
        include_str!("../static/js/search-menu.js"),
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

/// Every asset's name; each needs its /assets/<name> route.
pub fn names() -> impl Iterator<Item = &'static str> {
    ASSETS.iter().map(|(name, _, _)| *name)
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
