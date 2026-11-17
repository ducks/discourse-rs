//! `CrawlerDetection.crawler?`: whether a user agent is a crawler, by the
//! site's agent lists (each a `|`-separated list matched as an escaped,
//! case-insensitive regexp union).

use crate::site_settings::{SettingError, SiteSettings};

/// `CrawlerDetection::WAYBACK_MACHINE_URL`
const WAYBACK_MACHINE_URL: &str = "archive.org";

/// `to_matcher(list)` applied to `text`: any alternative found in it. An
/// empty alternative (an empty list, or `a||b`) matches everything, as the
/// empty regexp does.
fn matches(list: &str, text_lower: &str) -> bool {
    // Ruby's split("|") drops trailing empty strings.
    let mut parts: Vec<&str> = list.split('|').collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    parts.is_empty()
        || parts
            .iter()
            .any(|p| p.is_empty() || text_lower.contains(&p.to_lowercase()))
}

/// `CrawlerDetection.crawler?(user_agent, via_header)`: a missing agent,
/// or one (or a Via header) naming the Wayback Machine, is a crawler; one
/// not on non_crawler_user_agents is a crawler; one that is, only when it
/// names a known bot not on crawler_check_bypass_agents.
pub fn crawler(
    settings: &SiteSettings,
    user_agent: Option<&str>,
    via: Option<&str>,
) -> Result<bool, SettingError> {
    let Some(ua) = user_agent else {
        return Ok(true);
    };
    if ua.contains(WAYBACK_MACHINE_URL) || via.is_some_and(|v| v.contains(WAYBACK_MACHINE_URL)) {
        return Ok(true);
    }
    let lower = ua.to_lowercase();
    let list = |name: &str| settings.get(name).map(|v| v.to_s());
    if !matches(&list("non_crawler_user_agents")?, &lower) {
        return Ok(true);
    }
    if !matches(&list("crawler_user_agents")?, &lower) {
        return Ok(false);
    }
    Ok(!matches(&list("crawler_check_bypass_agents")?, &lower))
}

/// `CrawlerDetection.crawler_layout_request?`: a request with a user agent,
/// for HTML (no other media type, format neither json nor rss), that asks
/// to print, escapes its fragment (enable_escaped_fragments), comes from a
/// browser browser_update_user_agents names, or from a crawler.
pub fn layout_request(
    settings: &SiteSettings,
    headers: &axum::http::HeaderMap,
    query: Option<&str>,
) -> Result<bool, SettingError> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let Some(ua) = header("user-agent").filter(|ua| !crate::ruby::is_blank(ua)) else {
        return Ok(false);
    };
    if let Some(media_type) = header("content-type")
        .and_then(|t| t.split(';').next())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        && !media_type.contains("html")
    {
        return Ok(false);
    }
    let params: Vec<(String, String)> = query
        .map(|q| form_urlencoded::parse(q.as_bytes()).into_owned().collect())
        .unwrap_or_default();
    let param = |name: &str| {
        params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    if matches!(param("format"), Some("json" | "rss")) {
        return Ok(false);
    }
    let escaped_fragment =
        settings.get("enable_escaped_fragments")?.truthy() && param("_escaped_fragment_").is_some();
    let browser_update = settings.get("browser_update_user_agents")?.to_s();
    let browser_update =
        !crate::ruby::is_blank(&browser_update) && matches(&browser_update, &ua.to_lowercase());
    Ok(escaped_fragment
        || param("print").is_some()
        || browser_update
        || crawler(settings, Some(ua), header("via"))?)
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn lists_match_as_rubys_regexp_union() {
        assert!(matches("Googlebot|bingbot", "mozilla googlebot/2.1"));
        assert!(!matches("Googlebot|bingbot", "mozilla firefox"));
        assert!(matches("", "anything"));
        assert!(matches("a||b", "anything"));
        assert!(!matches("a|b|", "zzz"));
    }
}
