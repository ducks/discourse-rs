//! discourse-gamification's client on the server-rendered pages: the
//! leaderboard page (GamificationLeaderboard: the podium, the viewer's
//! own row, the ranking paged by htmx as DLoadMore scrolls, the period
//! chooser and the "How does this work?" modal, with
//! static/js/gamification.js), and the score on the user card
//! (user-card-metadata) and profile (user-profile-secondary), linked to
//! the default leaderboard.

use serde_json::Value;

use crate::admin_dashboard_view::to_number;
use crate::topic_list_view::{ListContext, escape, icon, t};

/// What the score connectors read: the site's
/// `default_gamification_leaderboard_id`.
#[derive(Debug, Clone)]
pub struct GamificationUi {
    pub default_leaderboard_id: Option<i64>,
}

impl GamificationUi {
    /// None while the plugin is off (its client isn't loaded).
    pub async fn load(
        conn: &mut sqlx::PgConnection,
        settings: &crate::site_settings::SiteSettings,
    ) -> Result<Option<Self>, crate::plugins::PluginError> {
        if !super::enabled(settings)? {
            return Ok(None);
        }
        let id: Option<i64> =
            sqlx::query_scalar("SELECT id FROM gamification_leaderboards ORDER BY id LIMIT 1")
                .fetch_optional(conn)
                .await?;
        Ok(Some(GamificationUi {
            default_leaderboard_id: id,
        }))
    }
}

/// The `fullnumber` helper: `I18n.toNumber(n, { precision: 0 })`.
pub fn fullnumber(n: i64) -> String {
    to_number(n as f64, 0)
}

/// GamificationScore: the score, linked to the default leaderboard when
/// there is one.
fn score(cx: &ListContext, ui: &GamificationUi, u: &Value) -> String {
    let n = fullnumber(u["gamification_score"].as_i64().unwrap_or(0));
    let inner = match ui.default_leaderboard_id {
        Some(id) => format!(
            "<a href=\"{}/leaderboard/{id}\" class=\"gamification-score__link\">{n}</a>",
            cx.base_path
        ),
        None => n,
    };
    format!("<span class=\"gamification-score\">{inner}</span>")
}

/// The user-card-metadata connector: the label and score, for a score
/// that isn't zero.
pub fn user_card(cx: &ListContext, ui: &GamificationUi, u: &Value) -> String {
    let mut out = String::from("<div class=\"user-card-metadata-outlet gamification-score\">");
    if u["gamification_score"].as_i64().unwrap_or(0) != 0 {
        out.push_str(&format!(
            "<span class=\"desc\">{} </span><span>{}</span>",
            escape(&t(cx, "gamification.score")),
            score(cx, ui, u)
        ));
    }
    out.push_str("</div>");
    out
}

/// The user-profile-secondary connector, an item of the secondary panel's
/// list.
pub fn profile(cx: &ListContext, ui: &GamificationUi, u: &Value) -> String {
    if u["gamification_score"].as_i64().unwrap_or(0) == 0 {
        return String::new();
    }
    format!(
        "<div><dt>{}</dt><dd>{}</dd></div>",
        escape(&t(cx, "gamification.score")),
        score(cx, ui, u)
    )
}

/// What the leaderboard page shows.
pub struct Page<'a> {
    /// The leaderboard's document: LeaderboardViewSerializer's, or while
    /// its positions aren't ready LeaderboardSerializer's with `users`
    /// empty and the `reason`.
    pub doc: &'a Value,
    /// The period on show, as the chooser names it (`all` for all_time).
    pub period: &'a str,
    /// The page of users the document holds (0 for the first).
    pub page: i64,
    pub viewer_id: Option<i64>,
    pub staff: bool,
    /// `prioritize_username_in_ux`
    pub prioritize_username: bool,
}

/// `dAvatar user imageSize=...`, titled with the username.
fn avatar(cx: &ListContext, u: &Value, size: u32) -> String {
    let template = u["avatar_template"].as_str().unwrap_or_default();
    let img = crate::user_profile_view::avatar(cx, template, size);
    match img.strip_suffix('>') {
        Some(head) => format!(
            "{head} title=\"{}\">",
            escape(u["username"].as_str().unwrap_or_default())
        ),
        None => img,
    }
}

fn display_name(p: &Page, u: &Value) -> String {
    let username = u["username"].as_str().unwrap_or_default();
    if p.prioritize_username {
        return username.to_string();
    }
    u["name"]
        .as_str()
        .filter(|n| !n.is_empty())
        .unwrap_or(username)
        .to_string()
}

/// The score as the rows show it: `dNumber` in the mobile view, else
/// `fullnumber`.
fn row_score(cx: &ListContext, n: i64) -> String {
    if crate::mobile::mobile_view() {
        crate::topic_list_view::number(cx, n)
    } else {
        fullnumber(n)
    }
}

/// `periodString(default_period)`: the chooser's name of a period.
pub fn chooser_period(period: &str) -> &str {
    if period == "all_time" { "all" } else { period }
}

/// GamificationLeaderboardRow
fn row(cx: &ListContext, p: &Page, u: &Value) -> String {
    format!(
        // `rank.currentUser` is never set (the component marks
        // isCurrentUser), so the highlight class never shows.
        "<div class=\"user \" id=\"leaderboard-user-{}\"><div class=\"user__rank\">{}</div><div class=\"user__avatar clickable\" data-user-card=\"{}\" role=\"button\">{}<span class=\"user__name\">{}</span></div><div class=\"user__score\">{}</div></div>",
        u["id"],
        u["position"],
        escape(u["username"].as_str().unwrap_or_default()),
        avatar(cx, u, 45),
        escape(&display_name(p, u)),
        row_score(cx, u["total_score"].as_i64().unwrap_or(0)),
    )
}

/// The ranking's rows of a later page, which the sentinel appends.
pub fn rows(cx: &ListContext, p: &Page) -> String {
    p.doc["users"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|u| row(cx, p, u))
        .collect()
}

/// The page's own path for a later page.
fn page_path(cx: &ListContext, p: &Page, page: i64) -> String {
    format!(
        "{}/leaderboard/{}?page={page}&period={}",
        cx.base_path, p.doc["leaderboard"]["id"], p.period
    )
}

/// DLoadMore's sentinel: while the last page loaded had users, revealing
/// it loads the next, whose rows join the ranking and whose sentinel
/// takes its place.
fn sentinel(cx: &ListContext, p: &Page) -> String {
    let more = p.doc["users"].as_array().is_some_and(|u| !u.is_empty());
    if !more {
        return "<div aria-hidden=\"true\" class=\"load-more-sentinel\" id=\"leaderboard-load-more\"></div>".into();
    }
    format!(
        "<div aria-hidden=\"true\" class=\"load-more-sentinel\" id=\"leaderboard-load-more\" hx-get=\"{}\" hx-trigger=\"revealed\" hx-select=\".leaderboard-page-rows > .user\" hx-target=\".leaderboard .ranking .load-more\" hx-swap=\"beforeend\" hx-select-oob=\"#leaderboard-load-more\" hx-push-url=\"false\"></div>",
        escape(&page_path(cx, p, p.page + 1))
    )
}

/// The period chooser (PeriodChooser with `leaderboard__period-chooser`),
/// disabled with the leaderboard's period filter.
fn period_chooser(cx: &ListContext, p: &Page) -> String {
    let chooser = crate::directory_view::period_chooser(cx, p.period);
    let disabled = p.doc["leaderboard"]["period_filter_disabled"] == true;
    chooser.replacen(
        "class=\"select-kit single-select dropdown-select-box period-chooser has-selection\"",
        &format!(
            "class=\"select-kit single-select dropdown-select-box period-chooser has-selection leaderboard__period-chooser{}\"",
            if disabled { " is-disabled" } else { "" }
        ),
        1,
    )
}

/// The "How does this work?" modal's content (LeaderboardInfo), for
/// gamification.js to show.
fn info_modal(cx: &ListContext) -> String {
    format!(
        "<template class=\"leaderboard-info-template\" data-title=\"{}\">{}{}</template>",
        escape(&t(cx, "gamification.leaderboard.modal.title")),
        icon("award", None),
        // trustHTML: the translation is HTML.
        t(cx, "gamification.leaderboard.modal.text")
    )
}

/// The leaderboard page (templates/gamification-leaderboard/*.gjs), or
/// for a later page only its rows.
pub fn page(cx: &ListContext, p: &Page) -> String {
    if p.page > 0 {
        return format!(
            "<div class=\"leaderboard-page-rows\">{}</div>{}",
            rows(cx, p),
            sentinel(cx, p)
        );
    }
    let mobile = crate::mobile::mobile_view();
    let lb = &p.doc["leaderboard"];
    let mut out = format!(
        "<div class=\"leaderboard\"><div class=\"page__header\"><h1 class=\"page__title\">{}</h1>",
        escape(lb["name"].as_str().unwrap_or_default())
    );
    let info = crate::admin_ui::Button {
        class: "-ghost leaderboard-info",
        icon: Some("circle-info"),
        label: (!mobile).then(|| t(cx, "gamification.leaderboard.info")),
        ..Default::default()
    };
    out.push_str(&info.render());
    out.push_str("</div><div class=\"leaderboard__controls\">");
    out.push_str(&period_chooser(cx, p));
    if p.staff {
        out.push_str(&format!(
            "<a class=\"leaderboard__settings\" href=\"{}/admin/plugins/gamification\">{}{}</a>",
            cx.base_path,
            icon("gear", None),
            if mobile {
                String::new()
            } else {
                escape(&t(cx, "gamification.leaderboard.link_to_settings"))
            }
        ));
    }
    out.push_str("</div>");
    if let Some(reason) = p.doc["reason"].as_str() {
        let refresh = crate::admin_ui::Button {
            class: "btn-primary refresh",
            icon: Some("arrows-rotate"),
            label: Some(t(cx, "gamification.leaderboard.refresh")),
            ..Default::default()
        };
        out.push_str(&format!(
            "<div class=\"leaderboard__not-ready\"><p>{}</p>{}</div>",
            escape(reason),
            refresh.render()
        ));
    } else {
        let users: &[Value] = p.doc["users"].as_array().map_or(&[], Vec::as_slice);
        out.push_str("<div class=\"podium__wrapper\"><div class=\"podium\">");
        for w in users.iter().take(3) {
            out.push_str(&format!(
                "<div class=\"winner -position{pos}\"><div class=\"winner__crown\">{}</div><div class=\"winner__avatar clickable\" data-user-card=\"{}\" role=\"button\">{}<div class=\"winner__rank\"><span>{pos}</span></div></div><div class=\"winner__name\">{}</div><div class=\"winner__score\">{}</div></div>",
                icon("crown", None),
                escape(w["username"].as_str().unwrap_or_default()),
                avatar(cx, w, 144),
                escape(&display_name(p, w)),
                fullnumber(w["total_score"].as_i64().unwrap_or(0)),
                pos = w["position"],
            ));
        }
        out.push_str(&format!(
            "</div></div><div class=\"ranking\"><div class=\"ranking-col-names\"><span>{}</span><span>{}{}</span></div><div class=\"ranking-col-names__sticky-border\"></div>",
            escape(&t(cx, "gamification.leaderboard.rank")),
            icon("award", None),
            escape(&t(cx, "gamification.score"))
        ));
        let personal = &p.doc["personal"];
        if personal["user"].is_object() {
            out.push_str(&format!(
                "<div class=\"user -self\"><div class=\"user__rank\">{}</div><div class=\"user__name\">{}</div><div class=\"user__score\">{}</div></div>",
                personal["position"],
                escape(&t(cx, "gamification.you")),
                row_score(cx, personal["user"]["total_score"].as_i64().unwrap_or(0))
            ));
        }
        out.push_str("<div class=\"load-more\">");
        for u in users.iter().skip(3) {
            out.push_str(&row(cx, p, u));
        }
        out.push_str("</div>");
        out.push_str(&sentinel(cx, p));
        out.push_str("</div>");
    }
    out.push_str(&info_modal(cx));
    out.push_str("</div>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullnumbers_are_grouped() {
        assert_eq!(fullnumber(1234567), "1,234,567");
        assert_eq!(fullnumber(12), "12");
    }

    #[test]
    fn all_time_is_all_to_the_chooser() {
        assert_eq!(chooser_period("all_time"), "all");
        assert_eq!(chooser_period("weekly"), "weekly");
    }
}
