//! What's new: DiscourseUpdates' new features. The feed meta.discourse.org
//! publishes (new-features.json), cached as Redis keeps it (the
//! `new_features` key, in discourse_rs.cached_values), less the entries
//! for a newer Discourse or a plugin the site lacks; merged with the
//! permanent upcoming changes; the newest entry's date each admin has
//! seen, and the one the daily CheckNewFeatures job last notified them
//! of.
//!
//! GitUtils.has_commit? (`git merge-base --is-ancestor <sha> HEAD`) is
//! answered from the vendored commit's history of the year before it
//! (vendor/discourse/commits.txt, scripts/record-discourse-commits): the
//! port runs as that commit, with no checkout of its own.

use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::Duration;

use chrono::{DateTime, Datelike, Months, NaiveDate};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::owned_schema::{cached_del, cached_get, cached_setex};
use crate::{AppError, AppState};

/// `new_features_key`
pub const FEED_KEY: &str = "new_features";
/// `latest_new_feature_created_at_key`
const LATEST_KEY: &str = "latest_new_feature_created_at";
/// `refresh_latest_new_feature_lock_key`
const LOCK_KEY: &str = "refresh_latest_new_feature_lock";
/// Jobs::RefreshLatestNewFeature
pub const REFRESH_JOB: &str = "refresh_latest_new_feature";
/// Jobs::CheckNewFeatures, daily.
pub const CHECK_JOB: &str = "check_new_features";
/// The meta feed (`new_features_endpoint` in production).
pub const META_ENDPOINT: &str = "https://meta.discourse.org/new-features.json";

/// A Redis value without a TTL.
const FOREVER: i64 = 100 * 365 * 24 * 3600;

fn last_seen_key(user_id: i32) -> String {
    format!("new_features_last_seen_user_{user_id}")
}

/// `last_viewed_feature_dates_for_users_hash`'s field for the user.
fn last_viewed_key(user_id: i32) -> String {
    format!("last_viewed_feature_dates_for_users_hash:{user_id}")
}

static COMMITS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    include_str!("../vendor/discourse/commits.txt")
        .lines()
        .filter(|l| !l.is_empty())
        .collect()
});

/// `GitUtils.has_commit?`: a full sha in the vendored commit's history.
pub fn has_commit(sha: &str) -> bool {
    sha.len() == 40
        && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        && COMMITS.contains(sha)
}

/// `Time.zone.parse(value).to_i`, None when it isn't a time.
fn seconds(value: &Value) -> Option<i64> {
    let s = value.as_str()?;
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.timestamp())
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S UTC")
                .ok()
                .map(|t| t.and_utc().timestamp())
        })
}

/// `sort_by { created_at.to_i }.reverse`: ties end up in reverse order.
fn newest_first(entries: &mut [Map<String, Value>]) {
    entries.sort_by_key(|e| e.get("created_at").and_then(seconds).unwrap_or(0));
    entries.reverse();
}

/// `DiscourseUpdates.current_version`
async fn current_version(conn: &mut PgConnection) -> Result<String, AppError> {
    Ok(cached_get(conn, "last_installed_version")
        .await?
        .unwrap_or_else(|| crate::schema::discourse_version().to_string()))
}

/// `DiscourseUpdates.new_features`: the cached feed, less the entries
/// for a newer Discourse (neither a version this one reaches nor a commit
/// it has) or a plugin it lacks, newest first.
pub async fn feed(conn: &mut PgConnection) -> Result<Vec<Map<String, Value>>, AppError> {
    let Some(raw) = cached_get(conn, FEED_KEY).await? else {
        return Ok(Vec::new());
    };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&raw) else {
        return Ok(Vec::new());
    };
    let version = current_version(conn).await?;
    let mut entries: Vec<Map<String, Value>> = items
        .into_iter()
        .filter_map(|item| match item {
            Value::Object(map) => Some(map),
            // A non-hash entry raises in the select: rescued as false.
            _ => None,
        })
        .filter(|item| {
            let valid_version = match item.get("discourse_version") {
                None | Some(Value::Null) => true,
                Some(Value::String(v)) => {
                    crate::themes::has_needed_version(&version, v) || has_commit(v)
                }
                // Gem::Version of a number, or a raise.
                Some(Value::Number(n)) => {
                    crate::themes::has_needed_version(&version, &n.to_string())
                }
                Some(_) => false,
            };
            let valid_plugin = match item.get("plugin_name") {
                None | Some(Value::Null) => true,
                Some(Value::String(name)) if name.trim().is_empty() => true,
                Some(Value::String(name)) => crate::admin_plugins::exists(name),
                Some(_) => false,
            };
            valid_version && valid_plugin
        })
        .collect();
    // The sort's Time.zone.parse raises on an entry without a date.
    if entries
        .iter()
        .any(|e| e.get("created_at").and_then(seconds).is_none())
    {
        return Err(std::io::Error::other("a new feature without a created_at time").into());
    }
    newest_first(&mut entries);
    Ok(entries)
}

/// `UpcomingChanges.permanent_upcoming_changes` (as the system user) and
/// `current_statuses`' changed_at of each.
async fn permanent_changes(
    conn: &mut PgConnection,
    state: &AppState,
) -> Result<Vec<(Value, Option<chrono::NaiveDateTime>)>, AppError> {
    let settings = crate::site_settings::SiteSettings::load(
        &mut *conn,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let cx = crate::admin_site_settings::Context {
        defs: &state.site_setting_defs,
        settings: &settings,
        i18n: &state.i18n,
        globals: &state.config.globals,
        base_path: state.config.globals.relative_url_root(),
        urls: &urls,
    };
    let changes =
        crate::upcoming_changes::list(&mut *conn, &cx, &["permanent".to_string()]).await?;
    let mut out = Vec::new();
    for change in changes {
        // The latest status change's time, with the status it set.
        let changed_at: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
            "SELECT created_at FROM upcoming_change_events WHERE event_type = $1 \
             AND upcoming_change_name = $2 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(crate::upcoming_changes::event_types::STATUS_CHANGED)
        .bind(change["setting"].as_str().unwrap_or_default())
        .fetch_optional(&mut *conn)
        .await?;
        out.push((change, changed_at));
    }
    Ok(out)
}

/// `DiscourseUpdates.merge_new_features_with_upcoming_changes`: the feed's
/// entries for upcoming changes kept only for permanent ones, and each
/// permanent change the feed doesn't cover added as an entry dated when
/// it became permanent; newest first.
pub async fn merged(
    conn: &mut PgConnection,
    state: &AppState,
) -> Result<Vec<Map<String, Value>>, AppError> {
    let mut entries = feed(&mut *conn).await?;
    let permanent = permanent_changes(&mut *conn, state).await?;
    let names: Vec<String> = permanent
        .iter()
        .map(|(c, _)| c["setting"].as_str().unwrap_or_default().to_string())
        .collect();
    let change_name = |e: &Map<String, Value>| -> Option<String> {
        e.get("upcoming_change_setting_name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    };
    entries.retain(|e| change_name(e).is_none_or(|n| names.contains(&n)));
    if names.is_empty() {
        return Ok(entries);
    }
    for (change, changed_at) in permanent {
        let setting = change["setting"].as_str().unwrap_or_default();
        if entries
            .iter()
            .any(|e| change_name(e).as_deref() == Some(setting))
        {
            continue;
        }
        let at = crate::topic_list::time_json(
            changed_at.unwrap_or_else(|| crate::clock::now().naive_utc()),
        );
        let mut entry = Map::new();
        entry.insert("title".into(), change["humanized_name"].clone());
        entry.insert("description".into(), change["description"].clone());
        entry.insert(
            "link".into(),
            change["upcoming_change"]["learn_more_url"].clone(),
        );
        entry.insert("created_at".into(), json!(at));
        entry.insert("updated_at".into(), json!(at));
        entry.insert("released_at".into(), json!(at));
        entry.insert(
            "screenshot_url".into(),
            change["upcoming_change"]["image"]["url"].clone(),
        );
        entry.insert("upcoming_change_setting_name".into(), json!(setting));
        entries.push(entry);
    }
    newest_first(&mut entries);
    Ok(entries)
}

/// The newest entry's created_at, as given.
fn newest(entries: &[Map<String, Value>]) -> Option<Value> {
    entries
        .iter()
        .max_by_key(|e| e.get("created_at").and_then(seconds).unwrap_or(0))
        .and_then(|e| e.get("created_at").cloned())
}

/// `refresh_latest_new_feature_created_at!` (the background job's).
pub async fn refresh_latest(conn: &mut PgConnection, state: &AppState) -> Result<(), AppError> {
    let entries = merged(&mut *conn, state).await?;
    match newest(&entries).as_ref().and_then(seconds) {
        Some(at) => {
            let iso = DateTime::from_timestamp(at, 0)
                .unwrap_or_default()
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string();
            cached_setex(&mut *conn, LATEST_KEY, FOREVER, &iso).await?;
        }
        None => cached_del(&mut *conn, LATEST_KEY).await?,
    }
    Ok(())
}

/// `has_unseen_features?`: false (with a refresh queued, once a minute)
/// until the newest date is known; then whether it is newer than what the
/// user last saw.
pub async fn has_unseen(conn: &mut PgConnection, user_id: i32) -> Result<bool, AppError> {
    let latest = cached_get(&mut *conn, LATEST_KEY)
        .await?
        .and_then(|v| seconds(&json!(v)));
    let Some(latest) = latest else {
        if crate::owned_schema::set_once(&mut *conn, LOCK_KEY, 60).await? {
            crate::jobs::enqueue(&mut *conn, REFRESH_JOB, json!({})).await?;
        }
        return Ok(false);
    };
    let last_seen = cached_get(&mut *conn, &last_seen_key(user_id))
        .await?
        .and_then(|v| seconds(&json!(v)));
    Ok(last_seen.is_none_or(|seen| latest > seen))
}

/// `mark_new_features_as_seen`: the newest entry's date.
pub async fn mark_seen(
    conn: &mut PgConnection,
    state: &AppState,
    user_id: i32,
) -> Result<(), AppError> {
    let entries = merged(&mut *conn, state).await?;
    if let Some(at) = newest(&entries).and_then(|v| v.as_str().map(str::to_string)) {
        cached_setex(&mut *conn, &last_seen_key(user_id), FOREVER, &at).await?;
    }
    Ok(())
}

/// `get_last_viewed_feature_date`, as seconds.
async fn last_viewed(conn: &mut PgConnection, user_id: i32) -> Result<Option<i64>, AppError> {
    Ok(cached_get(conn, &last_viewed_key(user_id))
        .await?
        .and_then(|v| seconds(&json!(v))))
}

/// `bump_last_viewed_feature_date`
pub async fn bump_last_viewed(
    conn: &mut PgConnection,
    user_id: i32,
    date: &Value,
) -> Result<(), AppError> {
    let value = date
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| date.to_string());
    cached_setex(conn, &last_viewed_key(user_id), FOREVER, &value).await?;
    Ok(())
}

/// `new_features_endpoint` with its query: the meta feed in production,
/// else DISCOURSE_NEW_FEATURES_ENDPOINT or the local one; features
/// released after the end of the month five months ago.
pub fn endpoint_url(state: &AppState) -> String {
    let base = state.config.new_features_endpoint.clone();
    let five_ago = crate::clock::now().date_naive() - Months::new(5);
    let first_next = NaiveDate::from_ymd_opt(five_ago.year(), five_ago.month(), 1)
        .and_then(|d| d.checked_add_months(Months::new(1)))
        .unwrap_or(five_ago);
    let end_of_month = first_next.pred_opt().unwrap_or(five_ago);
    format!("{base}?released_after={}", end_of_month.format("%Y-%m-%d"))
}

/// `new_features_response_json`: Excon's GET (200 expected, five seconds
/// each way), through FinalDestination's checked connection.
pub async fn fetch(
    state: &AppState,
    settings: &crate::site_settings::SiteSettings,
) -> Result<String, AppError> {
    let url = endpoint_url(state);
    let fetcher = crate::final_destination::Fetcher {
        settings,
        urls: crate::url::Urls {
            config: &state.config,
            settings,
        },
    };
    let user_agent = crate::schema::user_agent();
    let posted = fetcher
        .send(
            reqwest::Method::GET,
            &crate::final_destination::Post {
                url: &url,
                headers: &[],
                body: &[],
                verify_certificate: true,
                timeout: Duration::from_secs(5),
                user_agent: &user_agent,
            },
        )
        .await?;
    match posted {
        crate::final_destination::Posted::Response {
            status: 200, body, ..
        } => Ok(body),
        crate::final_destination::Posted::Response { status, .. } => Err(std::io::Error::other(
            format!("Excon::Error::HTTPStatus: Expected(200) <=> Actual({status}) ({url})"),
        )
        .into()),
        crate::final_destination::Posted::Blocked => Err(std::io::Error::other(format!(
            "FinalDestination::SSRFDetector::DisallowedIpError ({url})"
        ))
        .into()),
        crate::final_destination::Posted::Failed(message) => {
            Err(std::io::Error::other(format!("Excon::Error::Socket: {message} ({url})")).into())
        }
    }
}

/// `update_new_features`: the feed cached, the newest date derived.
pub async fn update(
    conn: &mut PgConnection,
    state: &AppState,
    json_text: &str,
) -> Result<(), AppError> {
    cached_setex(&mut *conn, FEED_KEY, FOREVER, json_text).await?;
    refresh_latest(conn, state).await
}

/// Jobs::CheckNewFeatures: the admins who never looked set to the newest
/// date known, the feed fetched, then each admin who hasn't seen the
/// newest entry notified (one new_features notification, the previous
/// ones deleted); on a site less than an hour old, every admin set to it
/// instead.
pub async fn check(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = crate::site_settings::SiteSettings::load(
        &mut conn,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    let admin_ids: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM users WHERE id > 0 AND admin ORDER BY id")
            .fetch_all(&mut *conn)
            .await?;
    let from_feed = feed(&mut conn).await?;
    if !from_feed.is_empty() {
        let entries = merged(&mut conn, state).await?;
        if let Some(first) = entries.first().and_then(|e| e.get("created_at")).cloned() {
            for &id in &admin_ids {
                if last_viewed(&mut conn, id).await?.is_none() {
                    bump_last_viewed(&mut conn, id, &first).await?;
                }
            }
        }
    }
    let body = fetch(state, &settings).await?;
    update(&mut conn, state, &body).await?;
    let entries = merged(&mut conn, state).await?;
    let Some(newest) = entries.first().and_then(|e| e.get("created_at")).cloned() else {
        return Ok(());
    };
    let newest_at = seconds(&newest).unwrap_or(0);
    // Migration::Helpers.new_site? (Discourse.site_creation_date within
    // the hour), not in development.
    let created: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
        "SELECT created_at FROM schema_migration_details ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(&mut *conn)
    .await?;
    let now = crate::clock::now().naive_utc();
    let new_site = created.is_none_or(|at| at >= now - chrono::Duration::hours(1));
    if new_site && state.config.rails_env != crate::config::RailsEnv::Development {
        for &id in &admin_ids {
            bump_last_viewed(&mut conn, id, &newest).await?;
        }
        return Ok(());
    }
    for &id in &admin_ids {
        if last_viewed(&mut conn, id)
            .await?
            .is_none_or(|seen| seen < newest_at)
        {
            let n = crate::notification_create::NewNotification::new(
                crate::notification_create::types::NEW_FEATURES,
                id,
                json!({}),
            );
            crate::notification_create::consolidate_or_create(&mut conn, &state.bus, &settings, &n)
                .await?;
            bump_last_viewed(&mut conn, id, &newest).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_are_the_vendored_history() {
        assert!(has_commit("b809efdd1cc434ad9274d71b74bf7d27c2fbc0d6"));
        assert!(has_commit(crate::schema::discourse_commit()));
        assert!(!has_commit("b809efdd"));
        assert!(!has_commit("0000000000000000000000000000000000000000"));
    }

    #[test]
    fn ties_keep_the_reverse_order() {
        let mut entries: Vec<Map<String, Value>> = ["a", "b", "c"]
            .iter()
            .zip([
                "2026-09-30T06:20:37.100Z",
                "2026-09-30T06:20:37.300Z",
                "2026-09-01T00:00:00.000Z",
            ])
            .map(|(name, at)| {
                let mut m = Map::new();
                m.insert("title".into(), json!(name));
                m.insert("created_at".into(), json!(at));
                m
            })
            .collect();
        newest_first(&mut entries);
        let titles: Vec<&str> = entries
            .iter()
            .map(|e| e["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, ["b", "a", "c"]);
    }
}
