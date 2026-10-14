//! Reports over users and links: users by type and trust level, the
//! trust level pipeline and growth, associated accounts, the incoming
//! links reports (IncomingLinksReport), trending searches and the web
//! hook event aggregates.

use std::collections::HashMap;

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::counts::{in_list, lit};
use super::{Cx, Report, colors, date_json, real_users, round_to};
use crate::{AppError, Unsupported};

/// `Float#to_s`: integral values keep their `.0`.
pub fn ruby_float(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 1e16 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

/// `report_users_by_type`
pub async fn users_by_type(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.dates_filtering = false;
    r.labels = json!([
        {"property": "x", "title": cx.t("reports.users_by_type.labels.type")?},
        {"property": "y", "type": "number", "title": cx.t("reports.default.labels.count")?},
    ]);
    let real = real_users("users");
    let rows = [
        ("admins", "shield-halved", "admin", "admin"),
        ("moderators", "shield-halved", "moderator", "moderator"),
        (
            "suspended",
            "ban",
            "suspended",
            "suspended_till IS NOT NULL AND suspended_till > now()",
        ),
        (
            "silenced",
            "ban",
            "silenced",
            "silenced_till IS NOT NULL AND silenced_till > now()",
        ),
    ];
    let mut data = Vec::new();
    for (key, icon, label, cond) in rows {
        let n: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM users WHERE {real} AND ({cond})"
        ))
        .fetch_one(&mut *conn)
        .await?;
        if n > 0 {
            data.push(json!({
                "url": format!("/admin/users/list/{key}"),
                "icon": icon,
                "key": key,
                "x": cx.t(&format!("reports.users_by_type.xaxis_labels.{label}"))?,
                "y": n,
            }));
        }
    }
    r.data = Value::Array(data);
    Ok(())
}

/// `TrustLevel.levels`
const TRUST_LEVELS: [&str; 5] = ["newuser", "basic", "member", "regular", "leader"];

/// `report_users_by_trust_level`
pub async fn users_by_trust_level(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.dates_filtering = false;
    r.labels = json!([
        {"property": "key", "title": cx.t("reports.users_by_trust_level.labels.level")?},
        {"property": "y", "type": "number", "title": cx.t("reports.default.labels.count")?},
    ]);
    let rows: Vec<(i32, i64)> = sqlx::query_as(&format!(
        "SELECT trust_level, count(*) FROM users WHERE {} GROUP BY trust_level ORDER BY trust_level",
        real_users("users")
    ))
    .fetch_all(&mut *conn)
    .await?;
    r.data = Value::Array(
        rows.iter()
            .map(|(level, n)| {
                let key = TRUST_LEVELS.get(*level as usize).copied();
                json!({
                    "url": format!("/admin/users/list/{}", key.unwrap_or("")),
                    "key": key,
                    "x": level,
                    "y": n,
                })
            })
            .collect(),
    );
    Ok(())
}

/// `UserHistory.actions[:change_trust_level]`, `[:auto_trust_level_change]`
const CHANGE_TRUST_LEVEL: i32 = 2;
const AUTO_TRUST_LEVEL_CHANGE: i32 = 15;

/// `report_trust_level_pipeline`
pub async fn trust_level_pipeline(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"property": "name", "title": cx.t("reports.trust_level_pipeline.labels.level")?},
        {"property": "count", "type": "number", "title": cx.t("reports.trust_level_pipeline.labels.count")?},
        {"property": "share_formatted", "title": cx.t("reports.trust_level_pipeline.labels.share")?},
        {"property": "promoted_in", "type": "number", "title": cx.t("reports.trust_level_pipeline.labels.promoted_in")?},
        {"property": "demoted_in", "type": "number", "title": cx.t("reports.trust_level_pipeline.labels.demoted_in")?},
        {"property": "signups", "type": "number", "title": cx.t("reports.trust_level_pipeline.labels.signups")?},
    ]);
    let real = real_users("users");
    let snapshot: HashMap<i32, i64> = sqlx::query_as::<_, (i32, i64)>(&format!(
        "SELECT trust_level, count(*) FROM users WHERE {real} GROUP BY trust_level"
    ))
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();
    let total_members: i64 = snapshot.values().sum();
    let new_signups: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM users WHERE {real} AND users.created_at BETWEEN {} AND {}",
        lit(r.start_date),
        lit(r.end_date)
    ))
    .fetch_one(&mut *conn)
    .await?;
    let entry_level = cx.settings.get("default_trust_level")?.to_i();
    let moves: Vec<(i32, i32, i64)> = sqlx::query_as(&format!(
        "WITH trust_changes AS MATERIALIZED ( \
           SELECT target_user_id, new_value, previous_value, created_at \
           FROM user_histories WHERE action IN ({CHANGE_TRUST_LEVEL}, {AUTO_TRUST_LEVEL_CHANGE}) \
         ) \
         SELECT new_value::integer AS new_tl, previous_value::integer AS prev_tl, COUNT(*) AS move_count \
         FROM trust_changes \
         WHERE created_at >= {} AND created_at <= {} \
           AND previous_value ~ '^\\d+$' AND new_value ~ '^\\d+$' \
           AND target_user_id IN (SELECT id FROM users WHERE id > 0) \
         GROUP BY new_value::integer, previous_value::integer",
        lit(r.start_date),
        lit(r.end_date)
    ))
    .fetch_all(&mut *conn)
    .await?;
    let mut promoted: HashMap<i32, i64> = HashMap::new();
    let mut demoted: HashMap<i32, i64> = HashMap::new();
    let (mut up, mut down) = (0i64, 0i64);
    for (new_tl, prev_tl, n) in moves {
        if new_tl == prev_tl {
            continue;
        }
        if new_tl > prev_tl {
            *promoted.entry(new_tl).or_default() += n;
            up += n;
        } else {
            *demoted.entry(new_tl).or_default() += n;
            down += n;
        }
    }
    let mut data = Vec::new();
    for tl in (0..=4).rev() {
        let n = snapshot.get(&tl).copied().unwrap_or(0);
        let share = if total_members == 0 {
            0.0
        } else {
            round_to(n as f64 / total_members as f64 * 100.0, 2)
        };
        data.push(json!({
            "trust_level": tl,
            "name": cx.t(&format!("reports.trust_level_pipeline.levels.{tl}"))?,
            "count": n,
            "share": super::float_json(share),
            "share_formatted": format!("{}%", ruby_float(share)),
            "promoted_in": promoted.get(&tl).copied().unwrap_or(0),
            "demoted_in": demoted.get(&tl).copied().unwrap_or(0),
            "signups": if i64::from(tl) == entry_level { new_signups } else { 0 },
        }));
    }
    r.data = Value::Array(data);
    let net = up - down;
    let direction = match net {
        n if n > 0 => "climbing",
        n if n < 0 => "dropping",
        _ => "stable",
    };
    r.total = Some(json!(total_members));
    r.prev_period = Some(json!({"direction": direction, "net": net.abs()}));
    Ok(())
}

/// `report_trust_level_growth`
pub async fn trust_level_growth(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["stacked_chart".into()];
    let sql = format!(
        "SELECT date(created_at), \
           (count(*) filter (WHERE previous_value::integer < 1 AND new_value = '1')) as tl1_reached, \
           (count(*) filter (WHERE previous_value::integer < 2 AND new_value = '2')) as tl2_reached, \
           (count(*) filter (WHERE previous_value::integer < 3 AND new_value = '3')) as tl3_reached, \
           (count(*) filter (WHERE previous_value::integer < 4 AND new_value = '4')) as tl4_reached \
         FROM user_histories \
         WHERE (created_at >= '{}' AND created_at <= '{}') \
           AND (action = {CHANGE_TRUST_LEVEL} OR action = {AUTO_TRUST_LEVEL_CHANGE}) \
         GROUP BY date(created_at) ORDER BY date(created_at)",
        super::interp_time(r.start_date),
        super::interp_time(r.end_date)
    );
    let rows: Vec<(NaiveDate, i64, i64, i64, i64)> =
        sqlx::query_as(&sql).fetch_all(&mut *conn).await?;
    let series = [
        ("tl1_reached", colors::LIME),
        ("tl2_reached", colors::MAGENTA),
        ("tl3_reached", colors::YELLOW),
        ("tl4_reached", colors::PURPLE),
    ];
    let mut data = Vec::new();
    for (i, (req, color)) in series.iter().enumerate() {
        let points: Vec<Value> = rows
            .iter()
            .map(|row| {
                let y = match i {
                    0 => row.1,
                    1 => row.2,
                    2 => row.3,
                    _ => row.4,
                };
                json!({"x": row.0.format("%Y-%m-%d").to_string(), "y": y})
            })
            .collect();
        data.push(json!({
            "req": req,
            "label": cx.t(&format!("reports.trust_level_growth.xaxis.{req}"))?,
            "color": color,
            "data": points,
        }));
    }
    r.data = Value::Array(data);
    Ok(())
}

/// Settings that turn on an external login (Discourse.enabled_authenticators).
const LOGIN_SETTINGS: [&str; 14] = [
    "enable_google_oauth2_logins",
    "enable_facebook_logins",
    "enable_twitter_logins",
    "enable_github_logins",
    "enable_discord_logins",
    "enable_linkedin_oidc_logins",
    "sign_in_with_apple_enabled",
    "microsoft_auth_enabled",
    "enable_login_with_amazon",
    "oauth2_enabled",
    "openid_connect_enabled",
    "lti_enabled",
    "patreon_login_enabled",
    "enable_discourse_connect",
];

/// `report_associated_accounts_by_provider`, for a site without
/// external logins (the providers' own counts are not ported).
pub async fn associated_accounts(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.dates_filtering = false;
    r.labels = json!([
        {"property": "provider", "title": cx.t("reports.associated_accounts_by_provider.labels.provider")?},
        {"property": "count", "type": "number", "title": cx.t("reports.default.labels.count")?},
    ]);
    for setting in LOGIN_SETTINGS {
        if cx.settings.get(setting).is_ok_and(|v| v.truthy()) {
            return Err(
                Unsupported("associated accounts by provider with an external login on").into(),
            );
        }
    }
    // Statistics.users[:count]: real, active, neither suspended nor silenced.
    let mut cond = format!(
        "{} AND active AND (suspended_till IS NULL OR suspended_till <= now()) \
         AND (silenced_till IS NULL OR silenced_till <= now())",
        real_users("users")
    );
    if cx.settings.get("must_approve_users")?.truthy() {
        cond.push_str(" AND approved");
    }
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM users WHERE {cond}"))
        .fetch_one(&mut *conn)
        .await?;
    let mut data = vec![
        (
            "total_users",
            cx.t("reports.associated_accounts_by_provider.labels.total_users")?,
            total,
        ),
        (
            "no_accounts",
            cx.t("reports.associated_accounts_by_provider.labels.no_accounts")?,
            total,
        ),
    ];
    // sort_by! { -count }
    data.sort_by_key(|(_, _, n)| -n);
    r.data = Value::Array(
        data.into_iter()
            .map(|(key, provider, n)| json!({"key": key, "provider": provider, "count": n}))
            .collect(),
    );
    Ok(())
}

/// IncomingLinksReport.public_incoming_links with its time window.
async fn public_links(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    filter: Option<(i64, bool)>,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<(String, String), AppError> {
    let from = "incoming_links \
                INNER JOIN posts ON posts.id = incoming_links.post_id AND posts.deleted_at IS NULL \
                INNER JOIN topics ON topics.id = posts.topic_id AND topics.deleted_at IS NULL"
        .to_string();
    let mut cond = "topics.archetype = 'regular'".to_string();
    if let Some((id, include)) = filter {
        cond.push_str(
            &super::counts::in_category(conn, cx, "topics.category_id", id, include).await?,
        );
    }
    cond.push_str(&format!(
        " AND incoming_links.created_at > {} AND incoming_links.created_at < {}",
        lit(start),
        lit(end)
    ));
    Ok((from, cond))
}

/// `sort_by { num_clicks }.reverse[0, n]`: ties keep no particular
/// order in Ruby; here the later one first, as a stable sort reversed.
fn top_by_clicks(mut rows: Vec<(Value, i64)>, n: usize) -> Vec<Value> {
    rows.sort_by_key(|(_, clicks)| *clicks);
    rows.into_iter().rev().take(n).map(|(v, _)| v).collect()
}

/// `report_top_referred_topics`
pub async fn top_referred_topics(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    let filter = r.add_category_filter();
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"type": "topic", "properties": {"title": "topic_title", "id": "topic_id"}, "title": cx.t("reports.top_referred_topics.labels.topic")?},
        {"property": "num_clicks", "type": "number", "title": cx.t("reports.top_referred_topics.labels.num_clicks")?},
    ]);
    let limit = r.limit.unwrap_or(8);
    let (from, cond) = public_links(conn, cx, filter, r.start_date, r.end_date).await?;
    let mut clicks: Vec<(i32, i64)> = sqlx::query_as(&format!(
        "SELECT posts.topic_id, count(*) FROM {from} WHERE {cond} AND posts.topic_id IS NOT NULL \
         GROUP BY posts.topic_id"
    ))
    .fetch_all(&mut *conn)
    .await?;
    // to_a.sort_by { count }.last(limit).reverse
    clicks.sort_by_key(|(_, n)| *n);
    let keep = clicks.len().saturating_sub(limit.max(0) as usize);
    let clicks: Vec<(i32, i64)> = clicks.into_iter().skip(keep).rev().collect();
    let ids: Vec<i64> = clicks.iter().map(|(id, _)| i64::from(*id)).collect();
    let mut topic_cond = format!("id IN {}", in_list(&ids));
    if let Some((id, include)) = filter {
        topic_cond
            .push_str(&super::counts::in_category(conn, cx, "category_id", id, include).await?);
    }
    let topics: Vec<(i32, String, Option<String>)> = sqlx::query_as(&format!(
        "SELECT id, title, slug FROM topics WHERE deleted_at IS NULL AND {topic_cond}"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let mut data = Vec::new();
    for (topic_id, n) in clicks {
        if let Some((_, title, slug)) = topics.iter().find(|t| t.0 == topic_id) {
            let slug = slug
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or(Unsupported("a topic without a slug"))?;
            data.push(json!({
                "topic_id": topic_id,
                "topic_title": title,
                "topic_url": format!("{}/t/{slug}/{topic_id}", cx.base_path()),
                "num_clicks": n,
            }));
        }
    }
    r.data = Value::Array(data);
    Ok(())
}

/// `report_top_traffic_sources`
pub async fn top_traffic_sources(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    let filter = r.add_category_filter();
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"property": "domain", "title": cx.t("reports.top_traffic_sources.labels.domain")?},
        {"property": "num_clicks", "type": "number", "title": cx.t("reports.top_traffic_sources.labels.num_clicks")?},
        {"property": "num_topics", "type": "number", "title": cx.t("reports.top_traffic_sources.labels.num_topics")?},
    ]);
    let (from, cond) = public_links(conn, cx, filter, r.start_date, r.end_date).await?;
    let from = format!(
        "{from} INNER JOIN incoming_referers ON incoming_referers.id = incoming_links.incoming_referer_id \
         INNER JOIN incoming_domains ON incoming_domains.id = incoming_referers.incoming_domain_id"
    );
    let clicks: Vec<(String, i64)> = sqlx::query_as(&format!(
        "SELECT incoming_domains.name, count(*) AS count_all FROM {from} WHERE {cond} \
         GROUP BY incoming_domains.name ORDER BY count_all DESC LIMIT 10"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let names: Vec<String> = clicks.iter().map(|(n, _)| n.clone()).collect();
    let topics: HashMap<String, i64> = sqlx::query_as::<_, (String, i64)>(&format!(
        "SELECT incoming_domains.name, count(DISTINCT posts.topic_id) FROM {from} WHERE {cond} \
           AND incoming_domains.name = ANY($1) GROUP BY incoming_domains.name"
    ))
    .bind(&names)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();
    let rows = clicks
        .into_iter()
        .map(|(domain, n)| {
            let t = topics.get(&domain).copied();
            (
                json!({"domain": domain, "num_clicks": n, "num_topics": t}),
                n,
            )
        })
        .collect();
    r.data = Value::Array(top_by_clicks(rows, 10));
    Ok(())
}

/// `report_top_referrers`
pub async fn top_referrers(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"type": "user", "properties": {"username": "username", "id": "user_id", "avatar": "user_avatar_template"}, "title": cx.t("reports.top_referrers.labels.user")?},
        {"property": "num_clicks", "type": "number", "title": cx.t("reports.top_referrers.labels.num_clicks")?},
        {"property": "num_topics", "type": "number", "title": cx.t("reports.top_referrers.labels.num_topics")?},
    ]);
    let (from, cond) = public_links(conn, cx, None, r.start_date, r.end_date).await?;
    let from = format!("{from} INNER JOIN users ON users.id = incoming_links.user_id");
    let cond = format!("{cond} AND incoming_links.user_id IS NOT NULL");
    let clicks: Vec<(String, i64)> = sqlx::query_as(&format!(
        "SELECT users.username, count(*) FROM {from} WHERE {cond} GROUP BY users.username"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let topics: HashMap<String, i64> = sqlx::query_as::<_, (String, i64)>(&format!(
        "SELECT users.username, count(DISTINCT posts.topic_id) FROM {from} WHERE {cond} GROUP BY users.username"
    ))
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();
    let mut rows = Vec::new();
    for (username, n) in clicks {
        let user: (i32, Option<i32>) =
            sqlx::query_as("SELECT id, uploaded_avatar_id FROM users WHERE username = $1")
                .bind(&username)
                .fetch_one(&mut *conn)
                .await?;
        rows.push((
            json!({
                "username": username,
                "user_id": user.0,
                "user_avatar_template": cx.avatar_template(user.0, &username, user.1)?,
                "num_clicks": n,
                "num_topics": topics.get(&username).copied(),
            }),
            n,
        ));
    }
    r.data = Value::Array(top_by_clicks(rows, 10));
    Ok(())
}

/// `report_trending_search`
pub async fn trending_search(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.labels = json!([
        {"property": "term", "type": "text", "title": cx.t("reports.trending_search.labels.term")?},
        {"property": "searches", "type": "number", "title": cx.t("reports.trending_search.labels.searches")?},
        {"type": "percent", "property": "ctr", "title": cx.t("reports.trending_search.labels.click_through")?},
    ]);
    r.modes = vec!["table".into()];
    let limit = r.limit.unwrap_or(100);
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(&format!(
        "SELECT lower(term) term, COUNT(*) AS searches, \
                SUM(CASE WHEN search_result_id IS NOT NULL THEN 1 ELSE 0 END) AS click_through \
         FROM search_logs WHERE search_logs.created_at > {} AND search_logs.created_at < {} \
         GROUP BY lower(term) ORDER BY searches DESC, click_through DESC, term ASC LIMIT {limit}",
        lit(r.start_date),
        lit(r.end_date)
    ))
    .fetch_all(&mut *conn)
    .await?;
    r.data = Value::Array(
        rows.into_iter()
            .map(|(term, searches, clicks)| {
                let ctr = if clicks == 0 || searches == 0 {
                    json!(0)
                } else {
                    super::float_json(super::ceil_to(clicks as f64 / searches as f64 * 100.0, 1))
                };
                json!({"term": term, "searches": searches, "ctr": ctr})
            })
            .collect(),
    );
    Ok(())
}

/// `WebHookEventsDailyAggregate.by_day(start, end)` as records (default
/// scope: newest first).
async fn aggregates_by_day(
    conn: &mut PgConnection,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<Vec<Map<String, Value>>, AppError> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        i64,
        i64,
        Option<NaiveDate>,
        Option<i32>,
        Option<i32>,
        Option<i32>,
        NaiveDateTime,
        NaiveDateTime,
    )> = sqlx::query_as(
        "SELECT id::bigint, web_hook_id::bigint, date, successful_event_count, failed_event_count, \
                mean_duration, created_at, updated_at \
         FROM web_hook_events_daily_aggregates WHERE date >= $1 AND date <= $2 ORDER BY created_at DESC",
    )
    .bind(start.date())
    .bind(end.date())
    .fetch_all(conn)
    .await?;
    let ms = |t: NaiveDateTime| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    Ok(rows
        .into_iter()
        .map(|(id, hook, date, ok, failed, mean, created, updated)| {
            let mut m = Map::new();
            m.insert("id".into(), json!(id));
            m.insert("web_hook_id".into(), json!(hook));
            m.insert("date".into(), date.map(date_json).unwrap_or(Value::Null));
            m.insert("successful_event_count".into(), json!(ok));
            m.insert("failed_event_count".into(), json!(failed));
            m.insert("mean_duration".into(), json!(mean));
            m.insert("created_at".into(), json!(ms(created)));
            m.insert("updated_at".into(), json!(ms(updated)));
            m
        })
        .collect())
}

/// `report_web_hook_events_daily_aggregate`
pub async fn web_hook_events(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.labels = json!([
        {"type": "date", "property": "x", "title": cx.t("reports.default.labels.day")?},
        {"type": "percent", "property": "y", "title": cx.t("reports.default.labels.percent")?},
    ]);
    let kind = r.filter_str("type_of_web_hook_event");
    let choices: Vec<Value> = ["all", "failed events", "successful events", "mean duration"]
        .iter()
        .map(|t| json!({"id": t, "name": t}))
        .collect();
    r.add_filter(
        "type_of_web_hook_event",
        json!({
            "type": "list",
            "default": kind.clone().unwrap_or_else(|| "all".into()),
            "choices": choices,
            "allow_any": false,
            "auto_insert_none_item": false,
        }),
    );
    r.average = true;
    r.percent = true;
    let points = aggregates_by_day(conn, r.start_date, r.end_date).await?;
    let mut data = Vec::new();
    for p in &points {
        let y = match kind.as_deref() {
            Some("failed events") => p["failed_event_count"].clone(),
            Some("successful events") => p["successful_event_count"].clone(),
            Some("mean duration") => p["mean_duration"].clone(),
            _ => match (
                p["successful_event_count"].as_i64(),
                p["failed_event_count"].as_i64(),
            ) {
                (Some(a), Some(b)) => json!(a + b),
                _ => return Err(Unsupported("web hook aggregates without counts").into()),
            },
        };
        data.push(json!({"x": p["date"], "y": y}));
    }
    r.data = Value::Array(data);
    if r.has_facet("prev_period") {
        let prev = aggregates_by_day(conn, r.prev_start_date(), r.prev_end_date()).await?;
        r.prev_period = Some(Value::Array(prev.into_iter().map(Value::Object).collect()));
    }
    if r.has_facet("prev30Days") {
        let prev = aggregates_by_day(
            conn,
            r.start_date - chrono::Duration::days(30),
            r.start_date,
        )
        .await?;
        r.prev30_days = Some(Value::Array(prev.into_iter().map(Value::Object).collect()));
    }
    Ok(())
}
