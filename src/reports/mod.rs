//! Report (app/models/report.rb) and its concerns
//! (app/models/concerns/reports/), Reports::ListQuery, and what the admin
//! reports controller reads from them.
//!
//! A report is filled by its `report_<type>` method (here `fill`), or by
//! `req_report` for the `*_reqs` types, inside a read-only transaction
//! with a statement timeout (wrap_slow_query). Times are UTC
//! NaiveDateTimes held to the nanosecond as Ruby's are; what goes to
//! Postgres is truncated to the microsecond, as Rails quotes them.

mod counts;
mod lists;
mod pageviews;
pub use pageviews::browser_pageviews_where as pageviews_browser_where;
mod tables;

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::{Definitions, SiteSettings};
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `Report::SCHEMA_VERSION`
const SCHEMA_VERSION: i32 = 5;
/// `Report::RELATED_ITEMS_LIMIT`
pub const RELATED_ITEMS_LIMIT: i64 = 50;

/// `Report::COLORS`
pub mod colors {
    pub const TURQUOISE: &str = "#1EB8D1";
    pub const LIME: &str = "#9BC53D";
    pub const PURPLE: &str = "#721D8D";
    pub const MAGENTA: &str = "#E84A5F";
    pub const BROWN: &str = "#8A6916";
    pub const YELLOW: &str = "#FFCD56";
}

/// `Report::HIDDEN_PAGEVIEW_REPORTS`
const HIDDEN_PAGEVIEW_REPORTS: [&str; 3] = [
    "site_traffic",
    "page_view_legacy_total_reqs",
    "top_entry_urls",
];
/// `Report::HIDDEN_LEGACY_PAGEVIEW_REPORTS`
const HIDDEN_LEGACY_PAGEVIEW_REPORTS: [&str; 3] = [
    "consolidated_page_views_browser_detection",
    "page_view_anon_reqs",
    "page_view_logged_in_reqs",
];
/// `Report::ADMIN_ONLY_REPORTS`
const ADMIN_ONLY_REPORTS: [&str; 6] = [
    "admin_logins",
    "top_uploads",
    "topic_view_stats",
    "top_entry_urls",
    "top_referrers_by_browser_pageviews",
    "top_countries_by_browser_pageviews",
];
/// `Report.admin_only_related_items_report_types`, with discourse-solved's.
const ADMIN_ONLY_RELATED_ITEMS_REPORTS: [&str; 3] =
    ["new_contributors", "signups", "accepted_solutions"];

/// `Report::LEGACY_REPORTS`
pub const LEGACY_REPORTS: [&str; 22] = [
    "associated_accounts_by_provider",
    "bookmarks",
    "consolidated_api_requests",
    "flags",
    "flags_status",
    "likes",
    "moderator_warning_private_messages",
    "mobile_visits",
    "notify_moderators_private_messages",
    "notify_user_private_messages",
    "post_edits",
    "profile_views",
    "reactions",
    "suspicious_logins",
    "system_private_messages",
    "top_referrers",
    "top_users_by_likes_received_from_inferior_trust_level",
    "top_users_by_likes_received_from_a_variety_of_people",
    "trust_level_growth",
    "user_flagging_ratio",
    "user_to_user_private_messages",
    "web_hook_events_daily_aggregate",
];

/// `Report::INTERNAL_REPORT_TYPES`
const INTERNAL_REPORT_TYPES: [&str; 1] = ["posters_by_member_type_members"];

/// Core's `report_*` methods (Report.singleton_methods), `about` and
/// `storage_stats` among them, in the order the concerns are included.
pub const CORE_REPORTS: [&str; 56] = [
    "about",
    "associated_accounts_by_provider",
    "bookmarks",
    "consolidated_api_requests",
    "consolidated_page_views",
    "consolidated_page_views_browser_detection",
    "site_traffic",
    "daily_engaged_users",
    "dau_by_mau",
    "emails",
    "flags",
    "flags_status",
    "likes",
    "mobile_visits",
    "moderator_warning_private_messages",
    "moderators_activity",
    "new_contributors",
    "notify_moderators_private_messages",
    "notify_user_private_messages",
    "post_edits",
    "posts",
    "profile_views",
    "signups",
    "admin_logins",
    "storage_stats",
    "suspicious_logins",
    "system_private_messages",
    "time_to_first_response",
    "top_countries_by_browser_pageviews",
    "top_entry_urls",
    "top_ignored_users",
    "top_referred_topics",
    "top_referrers",
    "top_referrers_by_browser_pageviews",
    "top_traffic_sources",
    "top_uploads",
    "top_users_by_likes_received",
    "top_users_by_likes_received_from_a_variety_of_people",
    "top_users_by_likes_received_from_inferior_trust_level",
    "topics",
    "topics_with_no_response",
    "topic_view_stats",
    "trending_search",
    "trust_level_growth",
    "trust_level_pipeline",
    "posters_by_member_type",
    "posters_by_member_type_members",
    "activity_by_category",
    "user_flagging_ratio",
    "user_to_user_private_messages",
    "user_to_user_private_messages_with_replies",
    "users_by_trust_level",
    "users_by_type",
    "visits",
    "web_crawlers",
    "web_hook_events_daily_aggregate",
];

/// The bundled plugins' reports: (plugin, its enabled setting, its
/// humanized name, the report types), those the port draws marked.
pub struct PluginReports {
    pub plugin: &'static str,
    pub enabled_setting: &'static str,
    pub display_name: &'static str,
    pub types: &'static [&'static str],
    pub ported: bool,
}

pub const PLUGIN_REPORTS: [PluginReports; 7] = [
    PluginReports {
        plugin: "discourse-solved",
        enabled_setting: "solved_enabled",
        display_name: "Solved",
        types: &["accepted_solutions"],
        ported: true,
    },
    PluginReports {
        plugin: "discourse-reactions",
        enabled_setting: "discourse_reactions_enabled",
        display_name: "Reactions",
        types: &["reactions"],
        ported: true,
    },
    PluginReports {
        plugin: "discourse-adplugin",
        enabled_setting: "discourse_adplugin_enabled",
        display_name: "Adplugin",
        types: &[
            "ad_plugin_ad_impressions",
            "ad_plugin_click_through_rate",
            "ad_plugin_click_through_rate_by_ad_type",
            "ad_plugin_click_through_rate_by_placement",
            "ad_plugin_house_ads_performance",
            "ad_plugin_impressions_by_user",
        ],
        ported: false,
    },
    PluginReports {
        plugin: "discourse-events",
        enabled_setting: "discourse_events_enabled",
        display_name: "Events",
        types: &["currently_away"],
        ported: false,
    },
    PluginReports {
        plugin: "discourse-user-notes",
        enabled_setting: "user_notes_enabled",
        display_name: "User Notes",
        types: &["user_notes"],
        ported: false,
    },
    PluginReports {
        plugin: "discourse-policy",
        enabled_setting: "policy_enabled",
        display_name: "Policy",
        types: &["unaccepted-policies"],
        ported: false,
    },
    PluginReports {
        plugin: "discourse-ai",
        enabled_setting: "discourse_ai_enabled",
        display_name: "AI",
        types: &[
            "emotion_admiration",
            "emotion_amusement",
            "emotion_anger",
            "emotion_annoyance",
            "emotion_approval",
            "emotion_caring",
            "emotion_confusion",
            "emotion_curiosity",
            "emotion_desire",
            "emotion_disappointment",
            "emotion_disapproval",
            "emotion_disgust",
            "emotion_embarrassment",
            "emotion_excitement",
            "emotion_fear",
            "emotion_gratitude",
            "emotion_grief",
            "emotion_joy",
            "emotion_love",
            "emotion_nervousness",
            "emotion_neutral",
            "emotion_optimism",
            "emotion_pride",
            "emotion_realization",
            "emotion_relief",
            "emotion_remorse",
            "emotion_sadness",
            "emotion_surprise",
            "overall_sentiment",
            "sentiment_analysis",
        ],
        ported: false,
    },
];

fn plugin_of(kind: &str) -> Option<&'static PluginReports> {
    PLUGIN_REPORTS.iter().find(|p| p.types.contains(&kind))
}

/// `ApplicationRequest.req_types`
pub const REQ_TYPES: [(&str, i32); 22] = [
    ("http_total", 0),
    ("http_2xx", 1),
    ("http_background", 2),
    ("http_3xx", 3),
    ("http_4xx", 4),
    ("http_5xx", 5),
    ("page_view_crawler", 6),
    ("page_view_logged_in", 7),
    ("page_view_anon", 8),
    ("page_view_logged_in_mobile", 9),
    ("page_view_anon_mobile", 10),
    ("api", 11),
    ("user_api", 12),
    ("page_view_anon_browser", 13),
    ("page_view_anon_browser_mobile", 14),
    ("page_view_logged_in_browser", 15),
    ("page_view_logged_in_browser_mobile", 16),
    ("page_view_anon_browser_beacon", 17),
    ("page_view_anon_browser_mobile_beacon", 18),
    ("page_view_logged_in_browser_beacon", 19),
    ("page_view_logged_in_browser_mobile_beacon", 20),
    ("page_view_embed", 21),
];

pub fn req_type(name: &str) -> Option<i32> {
    REQ_TYPES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, id)| *id)
}

/// What a report reads besides the database.
pub struct Cx<'a> {
    pub settings: &'a SiteSettings,
    pub defs: &'a Definitions,
    pub i18n: &'a I18n,
    pub urls: &'a Urls<'a>,
    /// `ColorScheme.hex_for_name("tertiary")`
    pub tertiary: Option<String>,
}

impl Cx<'_> {
    pub fn t(&self, key: &str) -> Result<String, AppError> {
        Ok(self
            .i18n
            .t(key)
            .ok_or(Unsupported("a report label without its translation"))?
            .to_string())
    }

    /// `I18n.t(key, default: nil)` as JSON: a string, a hash of them, or nil.
    fn t_value(&self, key: &str) -> Value {
        match self.i18n.t(key) {
            Some(s) => json!(s),
            None => self.i18n.subtree(key).unwrap_or(Value::Null),
        }
    }

    /// `I18n.t(key, default: nil)`
    fn t_opt(&self, key: &str) -> Option<String> {
        self.i18n.t(key).map(str::to_string)
    }

    pub fn base_path(&self) -> &str {
        self.urls.config.globals.relative_url_root()
    }

    /// `User.avatar_template(username, uploaded_avatar_id)`
    pub fn avatar_template(
        &self,
        user_id: i32,
        username: &str,
        uploaded_avatar_id: Option<i32>,
    ) -> Result<String, AppError> {
        Ok(crate::avatar::avatar_template(
            self.urls,
            user_id,
            username,
            uploaded_avatar_id,
            None,
        )?)
    }
}

/// Who asks: `report.guardian`, nil for the reports Rails finds without
/// one (the crawler report's related web crawlers).
#[derive(Clone)]
pub struct Viewer<'a> {
    pub guardian: Option<&'a Guardian>,
}

impl Viewer<'_> {
    pub fn user_id(&self) -> Option<i32> {
        self.guardian.and_then(Guardian::user_id)
    }

    pub fn is_admin(&self) -> bool {
        self.guardian.is_some_and(Guardian::is_admin)
    }

    pub fn is_staff(&self) -> bool {
        self.guardian.is_some_and(Guardian::is_staff)
    }

    /// `Guardian#can_see_ip?`
    fn can_see_ip(&self, settings: &SiteSettings) -> Result<Option<bool>, AppError> {
        let Some(g) = self.guardian else {
            return Ok(None);
        };
        Ok(Some(
            g.is_admin() || (g.is_moderator() && settings.get("moderators_view_ips")?.truthy()),
        ))
    }
}

/// What the controller (or a caller) asks for: Report._get's opts.
#[derive(Default, Clone)]
pub struct Opts {
    pub start_date: Option<NaiveDateTime>,
    pub end_date: Option<NaiveDateTime>,
    pub facets: Option<Vec<String>>,
    pub limit: Option<i64>,
    pub filters: Option<Map<String, Value>>,
    pub include_related_items: bool,
}

/// A report being filled, then its JSON.
pub struct Report {
    pub kind: String,
    pub data: Value,
    pub total: Option<Value>,
    pub prev30_days: Option<Value>,
    pub start_date: NaiveDateTime,
    pub end_date: NaiveDateTime,
    pub labels: Value,
    pub prev_period: Option<Value>,
    pub facets: Vec<String>,
    pub limit: Option<i64>,
    pub average: bool,
    pub percent: bool,
    pub higher_is_better: bool,
    pub icon: Option<String>,
    pub modes: Vec<String>,
    pub prev_data: Value,
    pub dates_filtering: bool,
    pub error: Option<&'static str>,
    pub primary_color: String,
    pub secondary_color: String,
    pub filters: Map<String, Value>,
    /// Whether `filters` came from the request (an empty hash still dumps).
    filters_given: bool,
    pub available_filters: Vec<(String, Map<String, Value>)>,
    pub legacy: bool,
    pub default_group_by: Option<String>,
    pub y_axis_title: Option<String>,
    pub include_related_items: bool,
    pub related_items: Option<Value>,
    pub related_items_totals: Option<Value>,
    pub related_report: Option<Value>,
    report_key: String,
}

impl Report {
    /// `Report.cache_key(report)`
    pub fn report_key(&self) -> &str {
        &self.report_key
    }

    pub fn has_facet(&self, facet: &str) -> bool {
        self.facets.iter().any(|f| f == facet)
    }

    /// `prev_start_date`: as far before the start as the end is after.
    pub fn prev_start_date(&self) -> NaiveDateTime {
        self.start_date - (self.end_date - self.start_date)
    }

    /// `prev_end_date`
    pub fn prev_end_date(&self) -> NaiveDateTime {
        self.start_date
    }

    /// `filters.dig(key)` / `filters[key]`
    pub fn filter(&self, key: &str) -> Option<&Value> {
        self.filters.get(key).filter(|v| !v.is_null())
    }

    /// A filter's value as a string, as it was given.
    pub fn filter_str(&self, key: &str) -> Option<String> {
        self.filter(key).and_then(crate::params::scalar)
    }

    /// `add_filter(name, options)`
    pub fn add_filter(&mut self, name: &str, options: Value) {
        let options = match options {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        if let Some(entry) = self.available_filters.iter_mut().find(|(n, _)| n == name) {
            entry.1 = options;
        } else {
            self.available_filters.push((name.to_string(), options));
        }
    }

    /// `add_category_filter`: the category and whether its subcategories
    /// count, when one is asked for.
    pub fn add_category_filter(&mut self) -> Option<(i64, bool)> {
        let category_id = self
            .filter("category")
            .filter(|v| !is_blank(v))
            .map(|v| crate::ruby::to_i(&crate::params::scalar(v).unwrap_or_default()));
        self.add_filter(
            "category",
            json!({"type": "category", "default": category_id}),
        );
        let category_id = category_id?;
        let include = crate::ruby::cast_boolean(
            self.filter("include_subcategories")
                .and_then(crate::params::scalar)
                .as_deref(),
        )
        .unwrap_or(false);
        self.add_filter(
            "include_subcategories",
            json!({"type": "bool", "default": include}),
        );
        Some((category_id, include))
    }

    /// `Report.default_labels`
    fn default_labels(i18n: &I18n) -> Value {
        json!([
            {"type": "date", "property": "x", "title": i18n.t("reports.default.labels.day")},
            {"type": "number", "property": "y", "title": i18n.t("reports.default.labels.count")},
        ])
    }

    /// `Report#as_json`
    pub fn as_json(&self, cx: &Cx<'_>) -> Value {
        let kind = &self.kind;
        let blank_none = |key: String| cx.t_opt(&key).filter(|s| !s.is_empty());
        let mut out = Map::new();
        out.insert("type".into(), json!(kind));
        out.insert("title".into(), cx.t_value(&format!("reports.{kind}.title")));
        out.insert("xaxis".into(), cx.t_value(&format!("reports.{kind}.xaxis")));
        out.insert("yaxis".into(), cx.t_value(&format!("reports.{kind}.yaxis")));
        out.insert(
            "description".into(),
            json!(blank_none(format!("reports.{kind}.description"))),
        );
        out.insert(
            "description_link".into(),
            json!(blank_none(format!("reports.{kind}.description_link"))),
        );
        out.insert("data".into(), self.data.clone());
        out.insert("start_date".into(), json!(iso8601(self.start_date)));
        out.insert("end_date".into(), json!(iso8601(self.end_date)));
        out.insert("prev_data".into(), self.prev_data.clone());
        out.insert(
            "prev_start_date".into(),
            json!(iso8601(self.prev_start_date())),
        );
        out.insert("prev_end_date".into(), json!(iso8601(self.prev_end_date())));
        out.insert(
            "prev30Days".into(),
            self.prev30_days.clone().unwrap_or(Value::Null),
        );
        out.insert("dates_filtering".into(), json!(self.dates_filtering));
        out.insert("report_key".into(), json!(self.report_key));
        out.insert("primary_color".into(), json!(self.primary_color));
        out.insert("secondary_color".into(), json!(self.secondary_color));
        out.insert(
            "available_filters".into(),
            Value::Array(
                self.available_filters
                    .iter()
                    .map(|(id, options)| {
                        let mut m = Map::new();
                        m.insert("id".into(), json!(id));
                        for (k, v) in options {
                            m.insert(k.clone(), v.clone());
                        }
                        Value::Object(m)
                    })
                    .collect(),
            ),
        );
        out.insert("labels".into(), self.labels.clone());
        out.insert("average".into(), json!(self.average));
        out.insert("percent".into(), json!(self.percent));
        out.insert("higher_is_better".into(), json!(self.higher_is_better));
        out.insert("modes".into(), json!(self.modes));
        if self.legacy {
            out.insert("legacy".into(), json!(true));
        }
        if let Some(icon) = &self.icon {
            out.insert("icon".into(), json!(icon));
        }
        if let Some(error) = self.error {
            out.insert("error".into(), json!(error));
        }
        if let Some(total) = self.total.clone().filter(truthy) {
            out.insert("total".into(), total);
        }
        if let Some(prev) = self.prev_period.clone().filter(truthy) {
            out.insert("prev_period".into(), prev);
        }
        if let Some(limit) = self.limit {
            out.insert("limit".into(), json!(limit));
        }
        if let Some(g) = &self.default_group_by {
            out.insert("default_group_by".into(), json!(g));
        }
        if let Some(t) = &self.y_axis_title {
            out.insert("y_axis_title".into(), json!(t));
        }
        if let Some(r) = self.related_items.clone().filter(truthy) {
            out.insert("related_items".into(), r);
        }
        if let Some(r) = self.related_items_totals.clone().filter(truthy) {
            out.insert("related_items_totals".into(), r);
        }
        if kind == "page_view_crawler_reqs" {
            out.insert(
                "related_report".into(),
                self.related_report.clone().unwrap_or(Value::Null),
            );
        }
        Value::Object(out)
    }
}

/// Ruby truthiness of a JSON value (only nil and false are falsy).
fn truthy(v: &Value) -> bool {
    !matches!(v, Value::Null | Value::Bool(false))
}

/// `Object#blank?` of a parameter value.
fn is_blank(v: &Value) -> bool {
    match v {
        Value::Null | Value::Bool(false) => true,
        Value::String(s) => crate::ruby::is_blank(s),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// `Time#iso8601` of a UTC time: whole seconds, `Z`.
pub fn iso8601(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// A time as Rails binds it: to the microsecond.
pub fn bind_time(t: NaiveDateTime) -> NaiveDateTime {
    use chrono::Timelike;
    t.with_nanosecond(t.nanosecond() / 1000 * 1000).unwrap_or(t)
}

/// `"#{time}"` interpolated into SQL: whole seconds, " UTC".
pub fn interp_time(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

/// `date.beginning_of_day`
pub fn beginning_of_day(d: NaiveDate) -> NaiveDateTime {
    d.and_time(NaiveTime::MIN)
}

/// `date.end_of_day`: 23:59:59.999999999.
pub fn end_of_day(d: NaiveDate) -> NaiveDateTime {
    d.and_hms_nano_opt(23, 59, 59, 999_999_999)
        .expect("a valid time")
}

/// A Ruby Date as JSON.
pub fn date_json(d: NaiveDate) -> Value {
    json!(d.format("%Y-%m-%d").to_string())
}

/// `Array#inspect` of the facets (symbols).
fn inspect_facets(facets: &[String]) -> String {
    let symbol = |s: &str| {
        let plain = s
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if plain {
            format!(":{s}")
        } else {
            format!(":{}", crate::ruby::inspect_string(s))
        }
    };
    format!(
        "[{}]",
        facets
            .iter()
            .map(|f| symbol(f))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// `Report.cache_key`
fn cache_key(r: &Report, viewer: &Viewer<'_>, settings: &SiteSettings) -> Result<String, AppError> {
    let mut parts: Vec<String> = vec![
        "reports".into(),
        r.kind.clone(),
        r.start_date.format("%Y%m%d").to_string(),
        r.end_date.format("%Y%m%d").to_string(),
        inspect_facets(&r.facets),
    ];
    if let Some(limit) = r.limit {
        parts.push(limit.to_string());
    }
    if r.filters_given && !r.filters.is_empty() {
        parts.push(Value::Object(r.filters.clone()).to_string());
    }
    parts.push(SCHEMA_VERSION.to_string());
    if let Some(id) = viewer.user_id() {
        parts.push(id.to_string());
    }
    if let Some(ip) = viewer.can_see_ip(settings)? {
        parts.push(ip.to_string());
    }
    // CrawlerScorer.enabled?
    parts.push(crawler_scorer_enabled(settings)?.to_string());
    Ok(parts.join(":"))
}

/// `CrawlerScorer.enabled?`: the improved crawler detection change.
pub fn crawler_scorer_enabled(settings: &SiteSettings) -> Result<bool, AppError> {
    Ok(settings.get("improved_crawler_detection")?.truthy())
}

/// `Report.hidden?(type, guardian:)`
pub fn hidden(kind: &str, viewer: &Viewer<'_>, settings: &SiteSettings) -> Result<bool, AppError> {
    if !viewer.is_admin() && ADMIN_ONLY_REPORTS.contains(&kind) {
        return Ok(true);
    }
    Ok(if settings.get("use_legacy_pageviews")?.truthy() {
        HIDDEN_PAGEVIEW_REPORTS.contains(&kind)
    } else {
        HIDDEN_LEGACY_PAGEVIEW_REPORTS.contains(&kind)
    })
}

/// `rgba_color(hex, opacity)`
fn rgba_color(hex: &str, opacity: &str) -> String {
    let mut hex = hex.replace('#', "");
    if hex.chars().count() == 3 {
        hex = hex.chars().flat_map(|c| [c, c]).collect();
    }
    if hex.len() < 3 {
        let last = hex.chars().last().unwrap_or('0');
        while hex.len() < 6 {
            hex.push(last);
        }
    }
    let rgbs: Vec<String> = hex
        .as_bytes()
        .chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| {
            let s = std::str::from_utf8(c).unwrap_or("00");
            // String#hex: the leading hex digits, 0 for none.
            let digits: String = s.chars().take_while(char::is_ascii_hexdigit).collect();
            i64::from_str_radix(&digits, 16).unwrap_or(0).to_string()
        })
        .collect();
    format!("rgba({},{opacity})", rgbs.join(","))
}

/// `Report._get(type, opts)`
pub fn get(cx: &Cx<'_>, viewer: &Viewer<'_>, kind: &str, opts: &Opts) -> Result<Report, AppError> {
    let now = crate::clock::now_naive();
    let tertiary = cx.tertiary.clone().unwrap_or_else(|| "0088cc".to_string());
    let start_date = opts
        .start_date
        .unwrap_or_else(|| beginning_of_day((now - Duration::days(30)).date()));
    let end_date = opts.end_date.unwrap_or_else(|| end_of_day(now.date()));
    let facets = opts
        .facets
        .clone()
        .unwrap_or_else(|| vec!["total".into(), "prev30Days".into()]);
    let include_related_items = opts.include_related_items
        && (viewer.is_admin() || !ADMIN_ONLY_RELATED_ITEMS_REPORTS.contains(&kind));
    let mut report = Report {
        kind: kind.to_string(),
        data: Value::Null,
        total: None,
        prev30_days: None,
        start_date,
        end_date,
        labels: Report::default_labels(cx.i18n),
        prev_period: None,
        facets,
        limit: opts.limit,
        average: false,
        percent: false,
        higher_is_better: true,
        icon: None,
        modes: vec!["chart".into(), "table".into()],
        prev_data: Value::Null,
        dates_filtering: true,
        error: None,
        primary_color: rgba_color(&tertiary, "1"),
        secondary_color: rgba_color(&tertiary, "0.1"),
        filters: opts.filters.clone().unwrap_or_default(),
        filters_given: opts.filters.is_some(),
        available_filters: Vec::new(),
        legacy: LEGACY_REPORTS.contains(&kind),
        default_group_by: None,
        y_axis_title: None,
        include_related_items,
        related_items: None,
        related_items_totals: None,
        related_report: None,
        report_key: String::new(),
    };
    report.report_key = cache_key(&report, viewer, cx.settings)?;
    Ok(report)
}

/// The context reports are drawn in, with the tertiary colour looked up.
pub async fn context<'a>(
    conn: &mut PgConnection,
    settings: &'a SiteSettings,
    defs: &'a Definitions,
    i18n: &'a I18n,
    urls: &'a Urls<'a>,
) -> Result<Cx<'a>, AppError> {
    let tertiary = crate::color_scheme::hex_for_name(conn, settings, "tertiary").await?;
    Ok(Cx {
        settings,
        defs,
        i18n,
        urls,
        tertiary,
    })
}

/// `Report.find_cached(type, opts)`: the cached JSON, if any (never for
/// reports with related items).
pub async fn find_cached(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    kind: &str,
    opts: &Opts,
) -> Result<Option<Value>, AppError> {
    let report = get(cx, viewer, kind, opts)?;
    if report.include_related_items {
        return Ok(None);
    }
    let hit = crate::owned_schema::cached_get(conn, report.report_key()).await?;
    Ok(hit.and_then(|s| serde_json::from_str(&s).ok()))
}

/// `Report.cache(report)`: an hour, a minute after an exception.
pub async fn cache(conn: &mut PgConnection, cx: &Cx<'_>, report: &Report) -> Result<(), AppError> {
    if report.include_related_items {
        return Ok(());
    }
    let seconds = if report.error == Some("exception") {
        60
    } else {
        3600
    };
    let json = report.as_json(cx);
    crate::owned_schema::cached_setex(conn, report.report_key(), seconds, &json.to_string())
        .await?;
    Ok(())
}

/// Whether a report type has a `report_<type>` method the port runs.
fn known(kind: &str) -> bool {
    CORE_REPORTS.contains(&kind) || plugin_of(kind).is_some()
}

/// `Report.find(type, opts)`: None when there is no such report.
pub async fn find(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    kind: &str,
    opts: &Opts,
) -> Result<Option<Report>, AppError> {
    let mut report = get(cx, viewer, kind, opts)?;
    if !known(kind) && !kind.ends_with("_reqs") {
        return Ok(None);
    }
    if let Some(p) = plugin_of(kind)
        && !p.ported
    {
        return Err(Unsupported("a report of a plugin the port doesn't run").into());
    }
    // wrap_slow_query: read only, with a statement timeout.
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout = 20000")
        .execute(&mut *tx)
        .await?;
    let result = fill(&mut tx, cx, viewer, &mut report).await;
    tx.rollback().await?;
    match result {
        Ok(()) => {}
        Err(e) if e.is_unsupported() => return Err(e),
        Err(e) if is_query_canceled(&e) => report.error = Some("timeout"),
        Err(e) => {
            tracing::error!(report = kind, error = %e, "error while computing report");
            report.error = Some("exception");
        }
    }
    if kind == "page_view_crawler_reqs" {
        // as_json embeds the web crawlers report (found without a viewer).
        let crawlers = Box::pin(find(
            conn,
            cx,
            &Viewer { guardian: None },
            "web_crawlers",
            &Opts {
                start_date: Some(report.start_date),
                end_date: Some(report.end_date),
                ..Opts::default()
            },
        ))
        .await?;
        report.related_report = crawlers.map(|r| r.as_json(cx));
    }
    Ok(Some(report))
}

/// ActiveRecord::QueryCanceled (SQLSTATE 57014).
fn is_query_canceled(e: &AppError) -> bool {
    e.sql_state().as_deref() == Some("57014")
}

/// `public_send("report_#{type}", report)` or `req_report`.
async fn fill(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    let kind = r.kind.clone();
    match kind.as_str() {
        // report_about(report) without its subject class: ArgumentError.
        "about" => {
            Err(std::io::Error::other("wrong number of arguments (given 1, expected 2..3)").into())
        }
        "signups" => counts::signups(conn, cx, r).await,
        "topics" => counts::topics(conn, cx, r).await,
        "posts" => counts::posts(conn, cx, r).await,
        "likes" => counts::post_action_report(conn, cx, r, 2, "heart").await,
        "flags" => counts::flags(conn, cx, r).await,
        "emails" => counts::emails(conn, r).await,
        "bookmarks" => counts::bookmarks(conn, cx, r).await,
        "visits" => counts::visits(conn, cx, r).await,
        "mobile_visits" => counts::mobile_visits(conn, r).await,
        "profile_views" => counts::profile_views(conn, r).await,
        "dau_by_mau" => counts::dau_by_mau(conn, cx, r).await,
        "daily_engaged_users" => counts::daily_engaged_users(conn, r).await,
        "new_contributors" => counts::new_contributors(conn, cx, r).await,
        "time_to_first_response" => counts::time_to_first_response(conn, cx, r).await,
        "topics_with_no_response" => counts::topics_with_no_response(conn, cx, r).await,
        "user_to_user_private_messages" => counts::private_messages(conn, r, "user_to_user").await,
        "system_private_messages" => counts::private_messages(conn, r, "system_message").await,
        "moderator_warning_private_messages" => {
            counts::private_messages(conn, r, "moderator_warning").await
        }
        "notify_moderators_private_messages" => {
            counts::private_messages(conn, r, "notify_moderators").await
        }
        "notify_user_private_messages" => counts::private_messages(conn, r, "notify_user").await,
        "user_to_user_private_messages_with_replies" => {
            counts::private_messages_with_replies(conn, r).await
        }
        "accepted_solutions" => counts::accepted_solutions(conn, cx, viewer, r).await,
        "consolidated_page_views" => pageviews::consolidated_page_views(conn, cx, r).await,
        "consolidated_api_requests" => pageviews::consolidated_api_requests(conn, cx, r).await,
        "site_traffic" | "consolidated_page_views_browser_detection" => {
            pageviews::site_traffic(conn, cx, r).await
        }
        "web_crawlers" => pageviews::web_crawlers(conn, cx, r).await,
        "top_entry_urls" => pageviews::top_entry_urls(conn, cx, r).await,
        "top_countries_by_browser_pageviews" => pageviews::top_countries(conn, cx, r).await,
        "top_referrers_by_browser_pageviews" => pageviews::top_referrers(conn, cx, r).await,
        "storage_stats" => pageviews::storage_stats(conn, cx, r).await,
        "users_by_type" => lists::users_by_type(conn, cx, r).await,
        "users_by_trust_level" => lists::users_by_trust_level(conn, cx, r).await,
        "trust_level_pipeline" => lists::trust_level_pipeline(conn, cx, r).await,
        "trust_level_growth" => lists::trust_level_growth(conn, cx, r).await,
        "associated_accounts_by_provider" => lists::associated_accounts(conn, cx, r).await,
        "top_referred_topics" => lists::top_referred_topics(conn, cx, r).await,
        "top_traffic_sources" => lists::top_traffic_sources(conn, cx, r).await,
        "top_referrers" => lists::top_referrers(conn, cx, r).await,
        "trending_search" => lists::trending_search(conn, cx, r).await,
        "web_hook_events_daily_aggregate" => lists::web_hook_events(conn, cx, r).await,
        "moderators_activity" => tables::moderators_activity(conn, cx, r).await,
        "flags_status" => tables::flags_status(conn, cx, r).await,
        "post_edits" => tables::post_edits(conn, cx, viewer, r).await,
        "admin_logins" => tables::admin_logins(conn, cx, r).await,
        "suspicious_logins" => tables::suspicious_logins(conn, cx, viewer, r).await,
        "top_uploads" => tables::top_uploads(conn, cx, r).await,
        "top_ignored_users" => tables::top_ignored_users(conn, cx, r).await,
        "top_users_by_likes_received" => tables::top_users_by_likes_received(conn, cx, r).await,
        "top_users_by_likes_received_from_a_variety_of_people" => {
            tables::likes_from_a_variety(conn, cx, r).await
        }
        "top_users_by_likes_received_from_inferior_trust_level" => {
            tables::likes_from_inferior_trust_level(conn, cx, r).await
        }
        "user_flagging_ratio" => tables::user_flagging_ratio(conn, cx, r).await,
        "topic_view_stats" => tables::topic_view_stats(conn, cx, r).await,
        "posters_by_member_type" => tables::posters_by_member_type(conn, cx, viewer, r).await,
        "posters_by_member_type_members" => {
            tables::posters_by_member_type_members(conn, cx, viewer, r).await
        }
        "activity_by_category" => tables::activity_by_category(conn, cx, viewer, r).await,
        "reactions" => tables::reactions(conn, cx, r).await,
        _ if kind.ends_with("_reqs") => {
            let filter = kind.trim_end_matches("_reqs").to_string();
            pageviews::req_report(conn, cx, r, &filter).await
        }
        _ => Err(Unsupported("a report the port doesn't draw").into()),
    }
}

/// `Reports::ListQuery.call(guardian:)`
pub fn list(cx: &Cx<'_>, viewer: &Viewer<'_>) -> Result<Value, AppError> {
    let mut names: Vec<String> = vec!["page_view_total_reqs".into()];
    for (name, _) in REQ_TYPES {
        if name.starts_with("page_view_") && !name.contains("mobile") && !name.contains("beacon") {
            names.push(format!("{name}_reqs"));
        }
    }
    let legacy_pageviews = cx.settings.get("use_legacy_pageviews")?.truthy();
    if !legacy_pageviews {
        names.push("page_view_legacy_total_reqs".into());
    }
    for kind in CORE_REPORTS {
        if kind != "about" && kind != "storage_stats" {
            names.push(kind.to_string());
        }
    }
    for p in &PLUGIN_REPORTS {
        for kind in p.types {
            names.push(kind.to_string());
        }
    }
    let mut out: Vec<(String, Value)> = Vec::new();
    for kind in names {
        if hidden(&kind, viewer, cx.settings)? {
            continue;
        }
        let plugin = plugin_of(&kind);
        if let Some(p) = plugin
            && !cx.settings.get(p.enabled_setting)?.truthy()
        {
            continue;
        }
        if LEGACY_REPORTS.contains(&kind.as_str()) || INTERNAL_REPORT_TYPES.contains(&kind.as_str())
        {
            continue;
        }
        if let Some(p) = plugin
            && !p.ported
        {
            return Err(Unsupported("the reports of a plugin the port doesn't run").into());
        }
        let legacy = legacy_pageviews
            && matches!(
                kind.as_str(),
                "consolidated_page_views" | "consolidated_page_views_browser_detection"
            );
        let title = if legacy {
            cx.t_opt(&format!("reports.{kind}.title_legacy"))
        } else {
            cx.t_opt(&format!("reports.{kind}.title"))
        };
        let description = if legacy {
            cx.t_opt(&format!("reports.{kind}.description_legacy"))
        } else {
            cx.t_opt(&format!("reports.{kind}.description"))
                .filter(|s| !s.is_empty())
        };
        let mut entry = Map::new();
        entry.insert("type".into(), json!(kind));
        entry.insert("title".into(), json!(title));
        entry.insert("description".into(), json!(description));
        entry.insert(
            "description_link".into(),
            json!(
                cx.t_opt(&format!("reports.{kind}.description_link"))
                    .filter(|s| !s.is_empty())
            ),
        );
        if let Some(p) = plugin {
            entry.insert("plugin".into(), json!(p.plugin));
            entry.insert("plugin_display_name".into(), json!(p.display_name));
        }
        let title = title.unwrap_or_default();
        out.push((title, Value::Object(entry)));
    }
    // sort_by title: a stable merge sort stands in for Ruby's, whose
    // order between equal titles is its own.
    out.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    Ok(Value::Array(out.into_iter().map(|(_, v)| v).collect()))
}

/// `User.real`'s condition on `users` aliased `alias`.
pub fn real_users(alias: &str) -> String {
    format!(
        "{alias}.id > 0 AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = {alias}.id)"
    )
}

/// `Category.subcategory_ids(id)`: the category and its descendants
/// down to max_category_nesting.
pub async fn subcategory_ids(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    category_id: i64,
) -> Result<Vec<i64>, AppError> {
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE subcategories AS ( \
           SELECT $1::bigint id, 1 depth \
           UNION \
           SELECT categories.id, (subcategories.depth + 1) depth \
           FROM categories JOIN subcategories ON subcategories.id = categories.parent_category_id \
           WHERE subcategories.depth < $2 \
         ) SELECT id FROM subcategories",
    )
    .bind(category_id)
    .bind(settings.get("max_category_nesting")?.to_i() as i32)
    .fetch_all(conn)
    .await?)
}

/// The categories a category filter covers: the one, or it with its
/// subcategories.
pub async fn category_ids(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    filter: Option<(i64, bool)>,
) -> Result<Option<Vec<i64>>, AppError> {
    Ok(match filter {
        None => None,
        Some((id, true)) => Some(subcategory_ids(conn, cx.settings, id).await?),
        Some((id, false)) => Some(vec![id]),
    })
}

/// A `{date => count}` hash as report data points.
pub fn points(rows: &[(NaiveDate, i64)]) -> Value {
    Value::Array(
        rows.iter()
            .map(|(d, n)| json!({"x": date_json(*d), "y": n}))
            .collect(),
    )
}

/// A `{date => count}` hash, in date order.
pub type DayCounts = Vec<(NaiveDate, i64)>;

/// `split_date_counts(counts, current_start)`
pub fn split_date_counts(
    rows: Vec<(NaiveDate, i64)>,
    start: NaiveDateTime,
) -> (DayCounts, DayCounts) {
    let start = start.date();
    rows.into_iter().partition(|(d, _)| *d >= start)
}

/// A Postgres numeric as Ruby's BigDecimal writes it in JSON
/// (`to_s`): the digits with at least one decimal place.
pub fn decimal_json(text: &str) -> Value {
    let (neg, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (int, frac) = digits.split_once('.').unwrap_or((digits, ""));
    let int = int.trim_start_matches('0');
    let int = if int.is_empty() { "0" } else { int };
    let frac = frac.trim_end_matches('0');
    let frac = if frac.is_empty() { "0" } else { frac };
    let sign = if neg && !(int == "0" && frac == "0") {
        "-"
    } else {
        ""
    };
    json!(format!("{sign}{int}.{frac}"))
}

/// Ruby's `Float#round(digits)` (rb_flo_round_by_double with
/// round_half_up, float.c).
pub fn round_to(x: f64, digits: i32) -> f64 {
    let s = 10f64.powi(digits);
    let mut f = (x * s).round();
    if x > 0.0 && (f + 0.5) / s <= x {
        f += 1.0;
    } else if x < 0.0 && (f - 0.5) / s >= x {
        f -= 1.0;
    }
    f / s
}

/// `Float#ceil(digits)`
pub fn ceil_to(x: f64, digits: i32) -> f64 {
    let s = 10f64.powi(digits);
    (x * s).ceil() / s
}

/// A Ruby Float as JSON: always with a fraction when integral.
pub fn float_json(x: f64) -> Value {
    serde_json::Number::from_f64(x)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_as_rails_writes_them() {
        assert_eq!(rgba_color("0088cc", "1"), "rgba(0,136,204,1)");
        assert_eq!(rgba_color("#08c", "0.1"), "rgba(0,136,204,0.1)");
    }

    #[test]
    fn decimals_as_bigdecimal_writes_them() {
        assert_eq!(decimal_json("30"), json!("30.0"));
        assert_eq!(decimal_json("1.50"), json!("1.5"));
        assert_eq!(decimal_json("0.0001"), json!("0.0001"));
        assert_eq!(decimal_json("-2"), json!("-2.0"));
    }

    #[test]
    fn floats_round_and_ceil_as_ruby() {
        // Probed on the reference.
        assert_eq!(round_to(2.675, 2), 2.68);
        assert_eq!(round_to(1.005, 2), 1.01);
        assert_eq!(round_to(200.0 / 3.0, 2), 66.67);
        assert_eq!(round_to(0.125, 2), 0.13);
        assert_eq!(ceil_to(1.1, 2), 1.11);
        assert_eq!(ceil_to(100.0 / 3.0, 2), 33.34);
        assert_eq!(ceil_to(0.29, 2), 0.29);
    }

    #[test]
    fn facets_inspect_as_symbols() {
        assert_eq!(
            inspect_facets(&["total".into(), "prev30Days".into()]),
            "[:total, :prev30Days]"
        );
    }
}
