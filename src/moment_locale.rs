//! The moment locale the client configures for the page's locale
//! (JsLocaleHelper's `configureMoment`): moment's own locale file for it,
//! the fix that keeps its digits ASCII, the timezone names translated
//! (`@discourse/moment-timezone-names-translations`, which supplants
//! `moment.tz.names()` with `moment.tz.localizedNames()`), and the
//! `shortDateNoYear`, `shortDate` and `longDate` formats. Served at
//! /assets/moment-locale/<locale>.js for static/js/local-dates.js to load
//! after moment and moment-timezone.

use axum::extract::Path;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

use crate::i18n::I18n;

include!(concat!(env!("OUT_DIR"), "/moment_locales.rs"));

/// `JsLocaleHelper::MOMENT_LOCALE_MAPPING`
const MAPPING: &[(&str, &str)] = &[("hy", "hy-am"), ("ug", "ug-cn")];

/// moment's file naming: the mapped locale, dashes for underscores,
/// lowercased.
fn file_name(locale: &str) -> String {
    let mapped = MAPPING
        .iter()
        .find(|(from, _)| *from == locale)
        .map(|(_, to)| *to)
        .unwrap_or(locale);
    mapped.replace('_', "-").to_lowercase()
}

/// `find_moment_locale([locale])` over a table: the locale's file, else
/// its language's, else none (no English fallback).
fn find<'a>(table: &'a [(&'a str, &'a str)], locale: &str) -> Option<&'a str> {
    let lookup = |name: &str| table.iter().find(|(n, _)| *n == name).map(|(_, s)| *s);
    let name = file_name(locale);
    if let Some(found) = lookup(&name) {
        return Some(found);
    }
    // find_locale tries the language alone with the mapped name split.
    let language = name.split(['-', '_']).next().unwrap_or(&name);
    if language != name {
        return lookup(language);
    }
    None
}

/// Whether the locale has anything for moment beyond the formats: a
/// locale or timezone names file.
pub fn has_files(locale: &str) -> bool {
    find(MOMENT_LOCALES, locale).is_some() || find(MOMENT_TIMEZONE_NAMES, locale).is_some()
}

fn wrapped(source: &str) -> String {
    format!("(function(){{\n{source}\n}}).apply(globalThis);\n")
}

/// `configureMoment`'s body for the locale, the server's date formats from
/// `i18n`.
pub fn script(locale: &str, i18n: &I18n) -> String {
    let mut out = String::new();
    if let Some(source) = find(MOMENT_LOCALES, locale) {
        out.push_str(&wrapped(source));
    }
    out.push_str(
        "if (moment.localeData().postformat(\"0\") !== \"0\") {\n  \
         moment.updateLocale(moment.locale(), { postformat: (string) => string });\n}\n",
    );
    if let Some(source) = find(MOMENT_TIMEZONE_NAMES, locale) {
        out.push_str(&wrapped(source));
    }
    for (name, key) in [
        ("shortDateNoYear", "dates.short_date_no_year"),
        ("shortDate", "dates.short_date"),
        ("longDate", "dates.long_date"),
    ] {
        let format = i18n.t(key).unwrap_or_default().replace('\'', "\\'");
        out.push_str(&format!(
            "moment.fn.{name} = function(){{ return this.format('{format}'); }};\n"
        ));
    }
    out
}

fn digest(body: &str) -> String {
    let hash = Sha256::digest(body.as_bytes());
    hash.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// The script's url for the locale, with its digest.
pub fn url(base_path: &str, locale: &str, i18n: &I18n) -> String {
    format!(
        "{base_path}/assets/moment-locale/{locale}.js?v={}",
        digest(&script(locale, i18n))
    )
}

/// GET /assets/moment-locale/<locale>.js: cached for good when asked for
/// with the current digest.
pub async fn serve(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
    Path(name): Path<String>,
    uri: axum::http::Uri,
    headers: HeaderMap,
) -> Response {
    let Some(locale) = name.strip_suffix(".js") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if locale.is_empty()
        || !locale
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let body = script(locale, &state.i18n);
    let digest = digest(&body);
    let etag = format!("\"{digest}\"");
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == etag)
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    let current = uri
        .query()
        .is_some_and(|q| q.split('&').any(|p| p == format!("v={digest}")));
    let cache = if current {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mut response = (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        body,
    )
        .into_response();
    let h = response.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_are_found_as_find_moment_locale_does() {
        assert!(find(MOMENT_LOCALES, "fr").is_some());
        assert!(find(MOMENT_LOCALES, "pt_BR").is_some());
        assert_eq!(file_name("pt_BR"), "pt-br");
        assert_eq!(file_name("hy"), "hy-am");
        // English has no file; nor has a made-up language.
        assert!(find(MOMENT_LOCALES, "en").is_none());
        assert!(find(MOMENT_LOCALES, "xx").is_none());
        // A region moment lacks falls back to the language.
        assert!(find(MOMENT_LOCALES, "de_XX").is_some());
        assert!(find(MOMENT_TIMEZONE_NAMES, "zh_CN").is_some());
        assert!(!has_files("en"));
        assert!(has_files("fr"));
    }
}
