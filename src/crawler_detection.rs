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
