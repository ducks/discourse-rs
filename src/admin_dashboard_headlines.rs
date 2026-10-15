//! The dashboard's headline tables: which title and summary (and call to
//! action) the engagement and search sections lead with for the
//! directions their metrics moved in (admin/lib/engagement-headline.js and
//! search-headline.js).

/// engagementHeadlineKeys: stickiness (DAU/MAU), daily engagement and new
/// sign-ups, each improved, declined, flat or unavailable.
#[rustfmt::skip]
pub fn engagement(
    stickiness: &str,
    daily_engagement: &str,
    new_signups: &str,
) -> (&'static str, &'static str) {
    match (stickiness, daily_engagement, new_signups) {
        ("improved", "improved", "improved") => ("engagement_up", "all_metrics_improved"),
        ("improved", "improved", "declined") => ("stickiness_and_daily_engagement_increased", "stickiness_and_daily_engagement_up_new_signups_down"),
        ("improved", "improved", "flat") => ("stickiness_and_daily_engagement_increased", "stickiness_and_daily_engagement_up_new_signups_flat"),
        ("improved", "improved", "unavailable") => ("stickiness_and_daily_engagement_increased", "stickiness_and_daily_engagement_up"),
        ("improved", "declined", "improved") => ("stickiness_and_new_signups_increased", "stickiness_and_new_signups_up_daily_engagement_down"),
        ("improved", "declined", "declined") => ("stickiness_increased", "stickiness_up_daily_engagement_and_new_signups_down"),
        ("improved", "declined", "flat") => ("stickiness_increased", "stickiness_up_daily_engagement_down"),
        ("improved", "declined", "unavailable") => ("stickiness_increased", "stickiness_up_daily_engagement_down"),
        ("improved", "flat", "improved") => ("stickiness_and_new_signups_increased", "stickiness_and_new_signups_up_daily_engagement_flat"),
        ("improved", "flat", "declined") => ("stickiness_increased", "stickiness_up_new_signups_down"),
        ("improved", "flat", "flat") => ("stickiness_increased", "stickiness_up_daily_engagement_and_new_signups_flat"),
        ("improved", "flat", "unavailable") => ("stickiness_increased", "stickiness_up_daily_engagement_flat"),
        ("improved", "unavailable", "improved") => ("stickiness_and_new_signups_increased", "stickiness_and_new_signups_up"),
        ("improved", "unavailable", "declined") => ("stickiness_increased", "stickiness_up_new_signups_down"),
        ("improved", "unavailable", "flat") => ("stickiness_increased", "stickiness_up_new_signups_flat"),
        ("improved", "unavailable", "unavailable") => ("stickiness_increased", "stickiness_up"),
        ("declined", "improved", "improved") => ("daily_engagement_and_new_signups_increased", "daily_engagement_and_new_signups_up_stickiness_down"),
        ("declined", "improved", "declined") => ("daily_engagement_increased", "daily_engagement_up_stickiness_and_new_signups_down"),
        ("declined", "improved", "flat") => ("daily_engagement_increased", "daily_engagement_up_stickiness_down"),
        ("declined", "improved", "unavailable") => ("daily_engagement_increased", "daily_engagement_up_stickiness_down"),
        ("declined", "declined", "improved") => ("new_signups_increased", "new_signups_up_stickiness_and_daily_engagement_down"),
        ("declined", "declined", "declined") => ("engagement_declined", "all_metrics_down"),
        ("declined", "declined", "flat") => ("some_engagement_declines", "stickiness_and_daily_engagement_down_new_signups_steady"),
        ("declined", "declined", "unavailable") => ("engagement_declined", "stickiness_and_daily_engagement_down"),
        ("declined", "flat", "improved") => ("new_signups_increased", "new_signups_up_stickiness_down"),
        ("declined", "flat", "declined") => ("some_engagement_declines", "stickiness_and_new_signups_down_daily_engagement_steady"),
        ("declined", "flat", "flat") => ("some_engagement_declines", "stickiness_down_daily_engagement_and_new_signups_steady"),
        ("declined", "flat", "unavailable") => ("some_engagement_declines", "stickiness_down_daily_engagement_steady"),
        ("declined", "unavailable", "improved") => ("new_signups_increased", "new_signups_up_stickiness_down"),
        ("declined", "unavailable", "declined") => ("engagement_declined", "stickiness_and_new_signups_down"),
        ("declined", "unavailable", "flat") => ("some_engagement_declines", "stickiness_down_new_signups_steady"),
        ("declined", "unavailable", "unavailable") => ("engagement_declined", "stickiness_down"),
        ("flat", "improved", "improved") => ("daily_engagement_and_new_signups_increased", "daily_engagement_and_new_signups_up_stickiness_flat"),
        ("flat", "improved", "declined") => ("daily_engagement_increased", "daily_engagement_up_new_signups_down"),
        ("flat", "improved", "flat") => ("daily_engagement_increased", "daily_engagement_up_stickiness_and_new_signups_flat"),
        ("flat", "improved", "unavailable") => ("daily_engagement_increased", "daily_engagement_up_stickiness_flat"),
        ("flat", "declined", "improved") => ("new_signups_increased", "new_signups_up_daily_engagement_down"),
        ("flat", "declined", "declined") => ("some_engagement_declines", "daily_engagement_and_new_signups_down_stickiness_steady"),
        ("flat", "declined", "flat") => ("some_engagement_declines", "daily_engagement_down_stickiness_and_new_signups_steady"),
        ("flat", "declined", "unavailable") => ("some_engagement_declines", "daily_engagement_down_stickiness_steady"),
        ("flat", "flat", "improved") => ("new_signups_increased", "new_signups_up_stickiness_and_daily_engagement_flat"),
        ("flat", "flat", "declined") => ("some_engagement_declines", "new_signups_down_stickiness_and_daily_engagement_steady"),
        ("flat", "flat", "flat") => ("engagement_steady", "all_metrics_steady"),
        ("flat", "flat", "unavailable") => ("engagement_steady", "stickiness_and_daily_engagement_steady"),
        ("flat", "unavailable", "improved") => ("new_signups_increased", "new_signups_up_stickiness_flat"),
        ("flat", "unavailable", "declined") => ("some_engagement_declines", "new_signups_down_stickiness_steady"),
        ("flat", "unavailable", "flat") => ("engagement_steady", "stickiness_and_new_signups_steady"),
        ("flat", "unavailable", "unavailable") => ("engagement_steady", "stickiness_steady"),
        ("unavailable", "improved", "improved") => ("daily_engagement_and_new_signups_increased", "daily_engagement_and_new_signups_up"),
        ("unavailable", "improved", "declined") => ("daily_engagement_increased", "daily_engagement_up_new_signups_down"),
        ("unavailable", "improved", "flat") => ("daily_engagement_increased", "daily_engagement_up_new_signups_flat"),
        ("unavailable", "improved", "unavailable") => ("daily_engagement_increased", "daily_engagement_up"),
        ("unavailable", "declined", "improved") => ("new_signups_increased", "new_signups_up_daily_engagement_down"),
        ("unavailable", "declined", "declined") => ("engagement_declined", "daily_engagement_and_new_signups_down"),
        ("unavailable", "declined", "flat") => ("some_engagement_declines", "daily_engagement_down_new_signups_steady"),
        ("unavailable", "declined", "unavailable") => ("engagement_declined", "daily_engagement_down"),
        ("unavailable", "flat", "improved") => ("new_signups_increased", "new_signups_up_daily_engagement_flat"),
        ("unavailable", "flat", "declined") => ("some_engagement_declines", "new_signups_down_daily_engagement_steady"),
        ("unavailable", "flat", "flat") => ("engagement_steady", "daily_engagement_and_new_signups_steady"),
        ("unavailable", "flat", "unavailable") => ("engagement_steady", "daily_engagement_steady"),
        ("unavailable", "unavailable", "improved") => ("new_signups_increased", "new_signups_up"),
        ("unavailable", "unavailable", "declined") => ("engagement_declined", "new_signups_down"),
        ("unavailable", "unavailable", "flat") => ("engagement_steady", "new_signups_steady"),
        ("unavailable", "unavailable", "unavailable") => ("insufficient_activity", "insufficient_activity"),
        _ => ("insufficient_activity", "insufficient_activity"),
    }
}

/// searchHeadlineKeys: total searches up, down, flat or unavailable, and
/// the no-result rate likewise; with no searches at all the section shows
/// the no-data headline before asking.
pub fn search(
    searches: &str,
    no_result_rate: &str,
) -> (&'static str, &'static str, Option<&'static str>) {
    match (searches, no_result_rate) {
        ("up", "down") => (
            "searches_increased_no_result_rate_declined",
            "searches_up_results_improved",
            None,
        ),
        ("up", "up") => (
            "searches_increased",
            "searches_up_no_result_rate_up",
            Some("content_gaps"),
        ),
        ("up", "flat") => (
            "searches_increased",
            "searches_up_no_result_rate_flat",
            Some("content_gaps"),
        ),
        ("down", "down") => (
            "no_result_rate_decreased",
            "results_improved_searches_down",
            None,
        ),
        ("flat", "down") => (
            "no_result_rate_decreased",
            "results_improved_searches_steady",
            None,
        ),
        ("down", "up") => (
            "searches_decreased_no_result_rate_increased",
            "searches_down_results_worsened",
            Some("content_gaps"),
        ),
        ("down", "flat") => (
            "searches_decreased",
            "searches_down_no_result_rate_flat",
            None,
        ),
        ("flat", "up") => (
            "no_result_rate_increased",
            "results_worsened_searches_steady",
            Some("content_gaps"),
        ),
        ("flat", "flat") => ("search_steady", "searches_and_no_result_rate_steady", None),
        ("up", "unavailable") => ("searches_increased", "searches_up", None),
        ("down", "unavailable") => ("searches_decreased", "searches_down", None),
        ("flat", "unavailable") => ("search_steady", "searches_steady", None),
        ("unavailable", "up") => (
            "no_result_rate_increased",
            "results_worsened",
            Some("content_gaps"),
        ),
        ("unavailable", "down") => ("no_result_rate_decreased", "results_improved", None),
        ("unavailable", "flat") => ("search_steady", "no_result_rate_steady", None),
        _ => ("insufficient_activity", "insufficient_activity", None),
    }
}
