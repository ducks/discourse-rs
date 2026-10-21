//! discourse-solved's support section as its Ember component draws it
//! (admin/components/dashboard/support.gjs with SupportTopicOutcomes,
//! SupportResponseTime, SupportWhosAnswering and supportHeadlineKeys),
//! from the section's data in the dashboard's document. Its category
//! filter is the closed MultipleCategoriesSelector, whose picker
//! admin-dashboard.js refuses as it does the engagement section's.

use serde_json::Value;

use crate::admin_dashboard_view::{
    Dashboard, category_slug_for, enc, js_number, js_round, section as dashboard_section, tooltip,
};
use crate::admin_view::{t, t_with};
use crate::topic_list_view::escape;
use crate::{AppError, user_profile_view};

const PREFIX: &str = "admin.dashboard.sections.support";

/// `supportHeadlineKeys`: the title, summary and call to action for the
/// resolution rate's and the first reply time's directions.
fn headline_keys(
    resolution: &str,
    reply: &str,
) -> (&'static str, &'static str, Option<&'static str>) {
    match (resolution, reply) {
        ("unavailable", "unavailable") => ("insufficient_activity", "insufficient_activity", None),
        ("up", "down") => (
            "resolution_rate_and_first_reply_time_improved",
            "more_answers_and_faster_replies",
            None,
        ),
        ("up", "up") => (
            "resolution_rate_improved",
            "more_answers_but_slower_replies",
            Some("unanswered"),
        ),
        ("up", "flat") => (
            "resolution_rate_improved",
            "more_answers_first_reply_time_flat",
            Some("unanswered"),
        ),
        ("down", "down") => (
            "first_reply_time_decreased",
            "faster_replies_but_lower_resolution_rate",
            Some("in_progress"),
        ),
        ("flat", "down") => (
            "first_reply_time_decreased",
            "faster_replies_resolution_rate_steady",
            Some("in_progress"),
        ),
        ("down", "up") => (
            "resolution_rate_and_first_reply_time_declined",
            "fewer_answers_and_slower_replies",
            Some("in_progress_and_unanswered"),
        ),
        ("down", "flat") => (
            "resolution_rate_decreased",
            "fewer_answers_first_reply_time_steady",
            Some("in_progress"),
        ),
        ("flat", "up") => (
            "first_reply_time_increased",
            "slower_replies_resolution_rate_steady",
            Some("unanswered"),
        ),
        ("flat", "flat") => (
            "support_steady",
            "resolution_rate_and_first_reply_time_steady",
            None,
        ),
        ("up", _) => ("resolution_rate_improved", "more_answers", None),
        ("down", _) => (
            "resolution_rate_decreased",
            "fewer_answers",
            Some("in_progress"),
        ),
        ("flat", _) => ("support_steady", "resolution_rate_steady", None),
        (_, "down") => ("first_reply_time_decreased", "faster_replies", None),
        (_, "up") => (
            "first_reply_time_increased",
            "slower_replies",
            Some("unanswered"),
        ),
        _ => ("support_steady", "first_reply_time_steady", None),
    }
}

/// `#direction(kpi, { noPriorDirection })`
fn direction(kpi: &Value, no_prior: Option<&'static str>) -> &'static str {
    let Some(value) = kpi["value"].as_f64() else {
        return "unavailable";
    };
    if kpi["previous_value"].is_null()
        && let Some(d) = no_prior
    {
        return d;
    }
    let previous = kpi["previous_value"].as_f64().unwrap_or(0.0);
    let change = js_round(value - previous);
    if change > 0.0 {
        "up"
    } else if change < 0.0 {
        "down"
    } else {
        "flat"
    }
}

/// A metric's change (`hasDelta`, `deltaText`, `deltaClass`).
struct Delta {
    text: Option<String>,
    class: &'static str,
}

/// DeltaPill: "stable" when unchanged, else the change.
fn delta_pill(d: &Dashboard, delta: Option<Delta>) -> String {
    match delta {
        None => String::new(),
        Some(Delta {
            class: "--neutral", ..
        }) => format!(
            "<span class=\"db-pill\">{}</span>",
            escape(&d.t("admin.dashboard.stable"))
        ),
        Some(Delta { text, class }) => format!(
            "<div class=\"db-delta {class}\">{}</div>",
            escape(&text.unwrap_or_default())
        ),
    }
}

/// `resolutionRate` and `staffInvolvement`: a whole percent and its
/// change, `lower_is_better` for staff involvement.
fn percent_metric(kpi: &Value, lower_is_better: bool) -> (String, Option<Delta>) {
    let value = kpi["value"].as_f64().unwrap_or(0.0);
    let shown = format!("{}%", js_round(value) as i64);
    let delta = kpi["previous_value"].as_f64().map(|previous| {
        let diff = js_round(value - previous) as i64;
        let better = if lower_is_better { diff < 0 } else { diff > 0 };
        Delta {
            text: Some(format!("{}{diff}%", if diff > 0 { "+" } else { "" })),
            class: if diff == 0 {
                "--neutral"
            } else if better {
                "--pos"
            } else {
                "--neg"
            },
        }
    });
    (shown, delta)
}

fn duration_tiny(d: &Dashboard, seconds: f64) -> String {
    user_profile_view::duration(d.list, seconds as i64, "tiny")
}

/// `avgFirstReply`: durationTiny, an em dash for none; faster is better.
fn avg_first_reply(d: &Dashboard, kpi: &Value) -> (String, Option<Delta>) {
    let value = kpi["value"].as_f64();
    let previous = kpi["previous_value"].as_f64();
    let shown = match value {
        Some(v) => duration_tiny(d, v),
        // Ember's em dash for no value.
        None => "\u{2014}".into(),
    };
    let delta = match (value, previous) {
        (Some(v), Some(p)) => {
            let diff = v - p;
            let slower = v > p;
            Some(Delta {
                text: (diff != 0.0).then(|| {
                    format!(
                        "{}{}",
                        if slower { "+" } else { "-" },
                        duration_tiny(d, diff.abs())
                    )
                }),
                class: if diff == 0.0 {
                    "--neutral"
                } else if slower {
                    "--neg"
                } else {
                    "--pos"
                },
            })
        }
        _ => None,
    };
    (shown, delta)
}

/// SupportSection
pub fn section(d: &Dashboard, data: &Value, error: bool) -> Result<String, AppError> {
    let mut body = String::new();
    if error {
        body.push_str(&d.error(&format!("{PREFIX}.fetch_error")));
    } else {
        body.push_str(&headline(d, &data["kpis"]));
        // showFilter: more than one support category.
        let options = data["category_options"].as_array().map_or(0, Vec::len);
        if options > 1 {
            let ids: Vec<i64> = data["category_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_i64)
                .collect();
            body.push_str(&format!(
                "<div class=\"db-support__filter\">{}</div>",
                d.categories_selector(&ids, Some("category.all"))
            ));
        }
        body.push_str(
            "<div class=\"db-section__row-group\"><div class=\"db-section__row\"><div class=\"db-section__row-block db-support-outcomes\">",
        );
        body.push_str(&outcomes(d, data)?);
        body.push_str("</div><div class=\"db-section__row-block db-support-response\">");
        body.push_str(&response_time(d, &data["response_time_distribution"]));
        body.push_str("</div></div><div class=\"db-section__row\"><div class=\"db-section__row-block db-support-answerers\">");
        body.push_str(&whos_answering(d, &data["whos_answering"]));
        body.push_str("</div><div class=\"db-section__row-block\"></div></div></div>");
    }
    Ok(dashboard_section(
        "support",
        &d.t(&format!("{PREFIX}.title")),
        "column",
        true,
        "",
        None,
        &body,
    ))
}

/// The headline and the three metrics.
fn headline(d: &Dashboard, kpis: &Value) -> String {
    let resolution = &kpis["resolution_rate"];
    let reply = &kpis["avg_first_reply"];
    let (title, summary, cta) =
        headline_keys(direction(resolution, None), direction(reply, Some("up")));
    let mut summary = d.t(&format!("{PREFIX}.headline.summaries.{summary}"));
    if let Some(cta) = cta {
        summary = format!("{summary} {}", d.t(&format!("{PREFIX}.headline.cta.{cta}")));
    }
    let mut out = format!(
        "<div class=\"db-section__subheader\"><div class=\"db-section__subintro\"><h3>{}</h3><p>{}</p></div><div class=\"db-section__metrics\">",
        escape(&d.t(&format!("{PREFIX}.headline.titles.{title}"))),
        escape(&summary)
    );
    let kpi_tooltip = |name: &str| {
        tooltip(
            "db-section__info",
            None,
            &d.t(&format!("{PREFIX}.kpi.{name}.tooltip")),
        )
    };

    // The resolution rate links to its report for the period.
    let (value, delta) = percent_metric(resolution, false);
    let mut query = Vec::new();
    for key in ["end_date", "filters", "start_date"] {
        match &resolution["report_query"][key] {
            Value::Null => {}
            Value::String(s) => query.push(format!("{key}={}", enc(s))),
            other => query.push(format!("{key}={}", enc(&other.to_string()))),
        }
    }
    let report = resolution["report_type"].as_str().unwrap_or_default();
    let href = format!(
        "{}/admin/reports/{}{}",
        d.admin.base_path,
        escape(report),
        if query.is_empty() {
            String::new()
        } else {
            format!("?{}", escape(&query.join("&")))
        }
    );
    out.push_str(&format!(
        "<div class=\"db-section__metric\"><div class=\"db-section__metric-number\">{}</div><div class=\"db-section__metric-label\"><a href=\"{href}\">{}</a>{}</div>{}</div>",
        escape(&value),
        escape(&d.t(&format!("{PREFIX}.kpi.resolution_rate.label"))),
        kpi_tooltip("resolution_rate"),
        delta_pill(d, delta)
    ));

    for (name, (value, delta)) in [
        (
            "staff_involvement",
            percent_metric(&kpis["staff_involvement"], true),
        ),
        ("avg_first_reply", avg_first_reply(d, reply)),
    ] {
        out.push_str(&format!(
            "<div class=\"db-section__metric\"><div class=\"db-section__metric-number\">{}</div><div class=\"db-section__metric-label\">{}{}</div>{}</div>",
            escape(&value),
            escape(&d.t(&format!("{PREFIX}.kpi.{name}.label"))),
            kpi_tooltip(name),
            delta_pill(d, delta)
        ));
    }
    out.push_str("</div></div>");
    out
}

/// `#categoryTerm`: exactly these categories, by their slugs.
fn category_term(d: &Dashboard, ids: &[i64]) -> Option<String> {
    let slugs: Vec<String> = ids
        .iter()
        .filter_map(|id| d.list.categories.get(id))
        .map(|c| category_slug_for(d.list, c))
        .collect();
    (!slugs.is_empty()).then(|| format!("=category:{}", slugs.join(",")))
}

/// SupportTopicOutcomes, each count linking to its topics on /filter.
fn outcomes(d: &Dashboard, data: &Value) -> Result<String, AppError> {
    let i18n = d.admin.i18n;
    let ids = |v: &Value| -> Vec<i64> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_i64().or_else(|| x["id"].as_i64()))
            .collect()
    };
    // categoryFilterTerm: the applied categories, else (unless every
    // topic takes answers) all the support categories.
    let applied = ids(&data["category_ids"]);
    let category = match category_term(d, &applied) {
        Some(term) => Some(term),
        None if d.admin.settings.get("allow_solved_on_all_topics")?.truthy() => None,
        None => category_term(d, &ids(&data["category_options"])),
    };
    // dateRangeTerms: created-before the day after the end.
    let dates = [
        format!("created-after:{}", d.range.start.format("%Y-%m-%d")),
        format!(
            "created-before:{}",
            (d.range.end + chrono::Duration::days(1)).format("%Y-%m-%d")
        ),
    ];
    let outcomes = &data["topic_outcomes"];
    let count = |key: &str| outcomes[key].as_f64().unwrap_or(0.0);
    let keys = ["resolved", "in_progress", "unanswered"];
    let max = keys.iter().map(|k| count(k)).fold(1.0f64, f64::max);
    let mut out = format!(
        "<div class=\"db-section__row-block-header\"><h3 class=\"db-section__row-block-title\">{}</h3></div><div class=\"db-support-outcomes__bars\">",
        escape(&t(i18n, &format!("{PREFIX}.outcomes.title")))
    );
    for key in keys {
        let status: &[&str] = match key {
            "resolved" => &["status:solved"],
            "in_progress" => &["status:unsolved", "posts-min:2"],
            _ => &["status:unsolved", "status:noreplies"],
        };
        let mut terms: Vec<String> = status.iter().map(|s| s.to_string()).collect();
        terms.extend(dates.iter().cloned());
        terms.extend(category.clone());
        let n = count(key);
        out.push_str(&format!(
            "<div class=\"db-support-outcomes__row\"><span aria-expanded=\"false\" class=\"fk-d-tooltip__trigger\" data-trigger=\"\" role=\"button\" data-tooltip-content=\"{}\"><span class=\"fk-d-tooltip__trigger-container\"><a class=\"db-support-outcomes__label\" href=\"{}/filter?q={}\">{}</a></span></span><span aria-hidden=\"true\" class=\"db-support-outcomes__track\"><span class=\"db-support-outcomes__fill --{}\" style=\"width: {}%\"></span></span><span class=\"db-support-outcomes__share\">{}</span></div>",
            escape(&t(i18n, &format!("{PREFIX}.outcomes.{key}.tooltip"))),
            d.admin.base_path,
            enc(&terms.join(" ")),
            escape(&t(i18n, &format!("{PREFIX}.outcomes.{key}.label"))),
            key.replacen('_', "-", 1),
            js_number(&serde_json::json!(n / max * 100.0)),
            if outcomes[key].is_null() {
                "0".to_string()
            } else {
                js_number(&outcomes[key])
            }
        ));
    }
    out.push_str("</div>");
    Ok(out)
}

/// SupportResponseTime: the four buckets' shares and the trend.
fn response_time(d: &Dashboard, data: &Value) -> String {
    let i18n = d.admin.i18n;
    let mut out = format!(
        "<div class=\"db-section__row-block-header\"><h3 class=\"db-section__row-block-title\">{}</h3>",
        escape(&t(i18n, &format!("{PREFIX}.response_time.title")))
    );
    let trend = &data["trend"];
    let seconds = trend["seconds"].as_f64().unwrap_or(0.0);
    if let Some(direction) = trend["direction"].as_str()
        && direction != "flat"
        && seconds != 0.0
    {
        out.push_str(&format!(
            "<span class=\"db-pill {}\">{}</span>",
            if direction == "faster" {
                "--pos"
            } else {
                "--neg"
            },
            escape(&t_with(
                i18n,
                &format!("{PREFIX}.response_time.trend.{direction}"),
                &[("duration", &duration_tiny(d, seconds))]
            ))
        ));
    }
    out.push_str("</div>");
    let empty = Vec::new();
    let buckets = data["buckets"].as_array().unwrap_or(&empty);
    let rows: Vec<(&str, String, String)> = ["lt_1h", "1_4h", "4_24h", "gt_24h"]
        .into_iter()
        .map(|key| {
            let share = buckets
                .iter()
                .find(|b| b["key"] == key)
                .map(|b| b["share"].clone())
                .filter(|s| !s.is_null())
                .unwrap_or(serde_json::json!(0));
            let formatted = format!("{}%", js_round(share.as_f64().unwrap_or(0.0)) as i64);
            (key, formatted, js_number(&share))
        })
        .collect();
    let label = |key: &str| t(i18n, &format!("{PREFIX}.response_time.buckets.{key}"));
    let aria: Vec<String> = rows
        .iter()
        .map(|(key, formatted, _)| format!("{} {formatted}", label(key)))
        .collect();
    out.push_str(&format!(
        "<div aria-label=\"{}\" class=\"db-support-response__bars\" role=\"img\">",
        escape(&aria.join(", "))
    ));
    for (key, formatted, width) in &rows {
        out.push_str(&format!(
            "<div class=\"db-support-response__row\"><span class=\"db-support-response__label\">{}</span><span class=\"db-support-response__track\"><span class=\"db-support-response__fill --bucket-{key}\" style=\"width: {width}%\"></span></span><span class=\"db-support-response__share\">{formatted}</span></div>",
            escape(&label(key))
        ));
    }
    out.push_str("</div>");
    out
}

/// SupportWhosAnswering: the groups with replies, largest share first.
fn whos_answering(d: &Dashboard, data: &Value) -> String {
    let i18n = d.admin.i18n;
    let mut out = format!(
        "<div class=\"db-section__row-block-header\"><h3 class=\"db-section__row-block-title\">{}</h3></div>",
        escape(&t(i18n, &format!("{PREFIX}.answerers.title")))
    );
    if data["total"].as_f64().unwrap_or(0.0) <= 0.0 {
        out.push_str(&format!(
            "<p class=\"db-whos-posting__empty\">{}</p>",
            escape(&t(i18n, &format!("{PREFIX}.answerers.empty")))
        ));
        return out;
    }
    let mut rows: Vec<&Value> = data["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["count"].as_f64().unwrap_or(0.0) > 0.0)
        .collect();
    // A stable sort, as Array#sort.
    rows.sort_by(|a, b| {
        let share = |r: &Value| r["share"].as_f64().unwrap_or(0.0);
        share(b)
            .partial_cmp(&share(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let label = |r: &Value| {
        t(
            i18n,
            &format!(
                "{PREFIX}.answerers.{}",
                r["type"].as_str().unwrap_or_default()
            ),
        )
    };
    let formatted = |r: &Value| format!("{}%", js_round(r["share"].as_f64().unwrap_or(0.0)) as i64);
    let aria: Vec<String> = rows
        .iter()
        .map(|r| format!("{} {}", label(r), formatted(r)))
        .collect();
    out.push_str(&format!(
        "<div aria-label=\"{}\" class=\"db-whos-posting__bars\" role=\"img\">",
        escape(&aria.join(", "))
    ));
    for r in rows {
        out.push_str(&format!(
            "<div class=\"db-whos-posting__bar-row\"><span class=\"db-whos-posting__bar-label\">{}</span><span class=\"db-whos-posting__bar-track\"><span class=\"db-whos-posting__bar-fill --{}\" style=\"width: {}%\"></span></span><span class=\"db-whos-posting__bar-share\">{}</span></div>",
            escape(&label(r)),
            escape(r["type"].as_str().unwrap_or_default()),
            js_number(&r["share"]),
            formatted(r)
        ));
    }
    out.push_str("</div>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn directions_round_the_change() {
        assert_eq!(
            direction(&json!({"value": 33.3, "previous_value": 0.0}), None),
            "up"
        );
        assert_eq!(
            direction(&json!({"value": 33.3, "previous_value": 33.0}), None),
            "flat"
        );
        assert_eq!(direction(&json!({"value": null}), None), "unavailable");
        assert_eq!(
            direction(&json!({"value": 900, "previous_value": null}), Some("up")),
            "up"
        );
        assert_eq!(
            direction(&json!({"value": 20.0, "previous_value": null}), None),
            "up"
        );
        assert_eq!(
            direction(&json!({"value": 900, "previous_value": 10800}), Some("up")),
            "down"
        );
    }

    #[test]
    fn every_scenario_has_a_headline() {
        let dirs = ["up", "down", "flat", "unavailable"];
        for r in dirs {
            for f in dirs {
                let (title, summary, _) = headline_keys(r, f);
                assert!(!title.is_empty() && !summary.is_empty());
            }
        }
        assert_eq!(
            headline_keys("unavailable", "flat"),
            ("support_steady", "first_reply_time_steady", None)
        );
        assert_eq!(
            headline_keys("down", "up").2,
            Some("in_progress_and_unanswered")
        );
    }

    #[test]
    fn percent_metrics_compare_whole_percents() {
        let (shown, delta) = percent_metric(&json!({"value": 33.3, "previous_value": 0.0}), false);
        assert_eq!(shown, "33%");
        let delta = delta.unwrap();
        assert_eq!(delta.text.as_deref(), Some("+33%"));
        assert_eq!(delta.class, "--pos");
        let (_, delta) = percent_metric(&json!({"value": 10.0, "previous_value": 20.0}), true);
        assert_eq!(delta.unwrap().class, "--pos");
        let (shown, delta) = percent_metric(&json!({"value": null, "previous_value": null}), false);
        assert_eq!(shown, "0%");
        assert!(delta.is_none());
    }
}
