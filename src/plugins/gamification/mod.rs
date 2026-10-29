//! discourse-gamification: points users earn for what they do (likes,
//! posts, topics, solutions, reactions, visits, reading, invites, flags,
//! chat), rolled up per leaderboard and day (`scorables`), ranked per
//! period in materialized views (`cached_view`), kept current by its jobs
//! (`jobs`); its leaderboards with their admin, the score events API, the
//! user card's `gamification_score` and the pages Ember draws from them
//! (`view`, `admin_view`).

pub mod admin_view;
pub mod cached_view;
pub mod jobs;
pub mod scorables;
pub mod view;

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::PluginError;
use crate::guardian::Guardian;
use crate::site_settings::{SettingError, SiteSettings};
use crate::url::Urls;

/// `enabled_site_setting :discourse_gamification_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("discourse_gamification_enabled")?.truthy())
}

/// `GamificationLeaderboard.periods`, in enum order (all_time 0 ...
/// daily 5).
pub const PERIODS: [&str; 6] = [
    "all_time",
    "yearly",
    "quarterly",
    "monthly",
    "weekly",
    "daily",
];

/// `GamificationLeaderboard::PAGE_SIZE`
pub const PAGE_SIZE: i64 = 100;

/// A gamification_leaderboards row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Leaderboard {
    pub id: i64,
    pub name: String,
    pub from_date: Option<NaiveDate>,
    pub to_date: Option<NaiveDate>,
    pub for_category_id: Option<i32>,
    pub created_by_id: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub visible_to_groups_ids: Vec<i32>,
    pub included_groups_ids: Vec<i32>,
    pub excluded_groups_ids: Vec<i32>,
    pub default_period: Option<i32>,
    pub period_filter_disabled: bool,
    pub score_overrides: Option<Value>,
    pub scorable_category_ids: Option<Vec<i32>>,
}

impl Leaderboard {
    /// `find(id)`
    pub async fn find(conn: &mut PgConnection, id: i64) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as("SELECT * FROM gamification_leaderboards WHERE id = $1")
            .bind(id)
            .fetch_optional(conn)
            .await
    }

    /// `GamificationLeaderboard.first`: by id.
    pub async fn first(conn: &mut PgConnection) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as("SELECT * FROM gamification_leaderboards ORDER BY id LIMIT 1")
            .fetch_optional(conn)
            .await
    }

    /// `order(updated_at: :desc)`
    pub async fn all_by_updated(conn: &mut PgConnection) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as("SELECT * FROM gamification_leaderboards ORDER BY updated_at DESC")
            .fetch_all(conn)
            .await
    }

    /// `score_override_for(key)`: an integer override; a value of any
    /// other JSON type reads as none.
    pub fn score_override_for(&self, key: &str) -> Option<i64> {
        self.score_overrides.as_ref()?.get(key)?.as_i64()
    }

    /// `resolve_period(given)`: the given period when it is one, else the
    /// default period's, else all_time.
    pub fn resolve_period(&self, given: Option<&str>) -> &'static str {
        if let Some(p) = given.and_then(|g| PERIODS.iter().find(|p| **p == g)) {
            return p;
        }
        self.default_period
            .and_then(|d| usize::try_from(d).ok())
            .and_then(|d| PERIODS.get(d))
            .copied()
            .unwrap_or("all_time")
    }
}

/// `Guardian#can_see_leaderboard?`
pub fn can_see(guardian: &Guardian, lb: &Leaderboard) -> bool {
    if lb.visible_to_groups_ids.is_empty() || guardian.is_admin() {
        return true;
    }
    guardian
        .group_ids()
        .iter()
        .any(|g| lb.visible_to_groups_ids.iter().any(|v| i64::from(*v) == *g))
}

/// `GamificationLeaderboard.scores_for(id, page:, for_user_id:, period:,
/// user_limit:)`
pub async fn scores_for(
    conn: &mut PgConnection,
    leaderboard_id: i64,
    page: i64,
    for_user_id: Option<i32>,
    period: Option<&str>,
    user_limit: Option<i64>,
) -> Result<Result<Vec<cached_view::Position>, cached_view::NotReady>, PluginError> {
    let offset = PAGE_SIZE * page;
    let limit = user_limit.unwrap_or(PAGE_SIZE);
    cached_view::scores(
        conn,
        leaderboard_id,
        period.unwrap_or("all_time"),
        for_user_id,
        Some(limit),
        offset,
    )
    .await
}

/// The generate positions job for a leaderboard whose views aren't there
/// (what a NotReadyError's rescue enqueues).
pub async fn enqueue_generate(
    conn: &mut PgConnection,
    leaderboard_id: i64,
) -> Result<(), sqlx::Error> {
    crate::jobs::enqueue(
        conn,
        jobs::GENERATE_LEADERBOARD_POSITIONS,
        json!({ "leaderboard_id": leaderboard_id }),
    )
    .await
}

/// `User#gamification_score`: the user's all time total on the default
/// leaderboard (the first), 0 without one; while its positions aren't
/// generated yet, 0 and the job to generate them.
pub async fn user_score(conn: &mut PgConnection, user_id: i32) -> Result<i64, PluginError> {
    let default: Option<i64> =
        sqlx::query_scalar("SELECT id FROM gamification_leaderboards ORDER BY id LIMIT 1")
            .fetch_optional(&mut *conn)
            .await?;
    let Some(id) = default else {
        return Ok(0);
    };
    match scores_for(conn, id, 0, Some(user_id), Some("all_time"), None).await? {
        Ok(rows) => Ok(rows.first().map_or(0, |p| p.total_score)),
        Err(cached_view::NotReady) => {
            enqueue_generate(conn, id).await?;
            Ok(0)
        }
    }
}

/// The user card serializer's key (UserSerializer's too, which extends
/// it), while the plugin is on.
pub async fn user_keys(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    out: &mut Map<String, Value>,
) -> Result<(), PluginError> {
    if enabled(settings)? {
        out.insert(
            "gamification_score".into(),
            json!(user_score(conn, user_id).await?),
        );
    }
    Ok(())
}

/// The site serializer's `default_gamification_leaderboard_id`, while the
/// plugin is on: the first leaderboard's id, null without one.
pub async fn site_keys(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    out: &mut Map<String, Value>,
) -> Result<(), PluginError> {
    if enabled(settings)? {
        let id: Option<i64> =
            sqlx::query_scalar("SELECT id FROM gamification_leaderboards ORDER BY id LIMIT 1")
                .fetch_optional(conn)
                .await?;
        out.insert("default_gamification_leaderboard_id".into(), json!(id));
    }
    Ok(())
}

fn date_json(d: Option<NaiveDate>) -> Value {
    json!(d.map(|d| d.format("%Y-%m-%d").to_string()))
}

/// LeaderboardSerializer, with AdminLeaderboardSerializer's scoring
/// configuration when `admin`.
pub fn leaderboard_json(lb: &Leaderboard, admin: bool) -> Value {
    let mut out = Map::new();
    out.insert("id".into(), json!(lb.id));
    out.insert("name".into(), json!(lb.name));
    out.insert("created_by_id".into(), json!(lb.created_by_id));
    out.insert("from_date".into(), date_json(lb.from_date));
    out.insert("to_date".into(), date_json(lb.to_date));
    out.insert(
        "visible_to_groups_ids".into(),
        json!(lb.visible_to_groups_ids),
    );
    out.insert("included_groups_ids".into(), json!(lb.included_groups_ids));
    out.insert("excluded_groups_ids".into(), json!(lb.excluded_groups_ids));
    out.insert("default_period".into(), json!(lb.default_period));
    out.insert(
        "updated_at".into(),
        json!(crate::topic_list::time_json(lb.updated_at)),
    );
    out.insert(
        "period_filter_disabled".into(),
        json!(lb.period_filter_disabled),
    );
    if admin {
        out.insert("score_overrides".into(), json!(lb.score_overrides));
        out.insert(
            "scorable_category_ids".into(),
            json!(lb.scorable_category_ids),
        );
    }
    Value::Object(out)
}

/// UserScoreSerializer (BasicUserSerializer's keys, then total_score and
/// position) for each position.
pub async fn user_scores_json(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    positions: &[cached_view::Position],
) -> Result<Vec<Value>, PluginError> {
    let settings = urls.settings;
    if positions.is_empty() {
        return Ok(Vec::new());
    }
    if settings.get("enable_user_status")?.truthy() {
        let ids: Vec<i32> = positions.iter().map(|p| p.id).collect();
        let with_status: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_statuses WHERE user_id = ANY($1) \
             AND (ends_at IS NULL OR ends_at > $2))",
        )
        .bind(&ids)
        .bind(crate::clock::now_naive())
        .fetch_one(&mut *conn)
        .await?;
        if with_status {
            return Err(PluginError::Unsupported(crate::Unsupported(
                "user status on BasicUserSerializer",
            )));
        }
    }
    let names = settings.get("enable_names")?.truthy();
    let mut out = Vec::with_capacity(positions.len());
    for p in positions {
        let mut u = Map::new();
        u.insert("id".into(), json!(p.id));
        u.insert("username".into(), json!(p.username));
        if names {
            u.insert("name".into(), json!(p.name));
        }
        u.insert(
            "avatar_template".into(),
            json!(crate::avatar::avatar_template(
                urls,
                p.id,
                &p.username,
                p.uploaded_avatar_id,
                // The system user (whose avatar may be the logo) is never ranked.
                None
            )?),
        );
        u.insert("total_score".into(), json!(p.total_score));
        u.insert("position".into(), json!(p.position));
        out.push(Value::Object(u));
    }
    Ok(out)
}

/// What LeaderboardViewSerializer is given.
pub struct ViewParams<'a> {
    pub leaderboard: &'a Leaderboard,
    pub page: i64,
    pub for_user_id: Option<i32>,
    pub period: &'static str,
    pub user_limit: Option<i64>,
}

/// LeaderboardViewSerializer: `personal` (the viewer's position, `{}`
/// without a viewer), the leaderboard and the page of users; NotReady
/// while the period's view isn't there.
pub async fn view_json(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    p: &ViewParams<'_>,
) -> Result<Result<Value, cached_view::NotReady>, PluginError> {
    let users = match scores_for(
        conn,
        p.leaderboard.id,
        p.page,
        None,
        Some(p.period),
        p.user_limit,
    )
    .await?
    {
        Ok(rows) => rows,
        Err(e) => return Ok(Err(e)),
    };
    let personal = match p.for_user_id {
        None => json!({}),
        Some(user_id) => {
            let mine = match scores_for(
                conn,
                p.leaderboard.id,
                0,
                Some(user_id),
                Some(p.period),
                None,
            )
            .await?
            {
                Ok(rows) => rows,
                Err(e) => return Ok(Err(e)),
            };
            match mine.first() {
                Some(pos) => {
                    let user = user_scores_json(conn, urls, std::slice::from_ref(pos))
                        .await?
                        .remove(0);
                    json!({ "user": user, "position": pos.position })
                }
                None => json!({ "user": null, "position": null }),
            }
        }
    };
    Ok(Ok(json!({
        "personal": personal,
        "leaderboard": leaderboard_json(p.leaderboard, false),
        "users": user_scores_json(conn, urls, &users).await?,
    })))
}

/// The model's validations: the reserved name, and the score overrides'
/// keys and values (`validate_score_overrides`). Full messages.
pub fn validate(name: &str, score_overrides: Option<&Value>) -> Vec<String> {
    let mut errors = Vec::new();
    if name == "new" {
        errors.push(format!("Name {name} is reserved."));
    }
    if let Some(Value::Object(map)) = score_overrides {
        for (key, value) in map {
            if !scorables::VALID_KEYS.contains(&key.as_str()) {
                errors.push(format!(
                    "Score overrides contains invalid scorable key: {key}"
                ));
            }
            if !value.as_i64().is_some_and(|v| v >= 0) {
                errors.push("Score overrides values must be non-negative integers".into());
            }
        }
    }
    errors
}

/// A gamification_score_events row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ScoreEvent {
    pub id: i64,
    pub user_id: i32,
    pub date: NaiveDate,
    pub points: i32,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// AdminGamificationScoreEventSerializer
pub fn score_event_json(e: &ScoreEvent) -> Value {
    json!({
        "id": e.id,
        "user_id": e.user_id,
        "date": e.date.format("%Y-%m-%d").to_string(),
        "points": e.points,
        "description": e.description,
        "created_at": crate::topic_list::time_json(e.created_at),
        "updated_at": crate::topic_list::time_json(e.updated_at),
    })
}

/// UserDestroyer: the user's scores go with them (`has_many
/// :gamification_leaderboard_scores, dependent: :destroy`, declared
/// whether or not the plugin is enabled).
pub async fn user_destroying(conn: &mut PgConnection, user_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM gamification_leaderboard_scores WHERE user_id = $1")
        .bind(i64::from(user_id))
        .execute(conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lb(default_period: Option<i32>) -> Leaderboard {
        let at = chrono::DateTime::from_timestamp(0, 0).unwrap().naive_utc();
        Leaderboard {
            id: 1,
            name: "lb".into(),
            from_date: None,
            to_date: None,
            for_category_id: None,
            created_by_id: 1,
            created_at: at,
            updated_at: at,
            visible_to_groups_ids: vec![],
            included_groups_ids: vec![],
            excluded_groups_ids: vec![],
            default_period,
            period_filter_disabled: false,
            score_overrides: None,
            scorable_category_ids: None,
        }
    }

    // spec/models/gamification_leaderboard_spec.rb
    #[test]
    fn resolve_period_takes_the_given_then_the_default() {
        assert_eq!(lb(Some(0)).resolve_period(Some("")), "all_time");
        assert_eq!(lb(Some(0)).resolve_period(None), "all_time");
        assert_eq!(lb(Some(5)).resolve_period(None), "daily");
        for p in PERIODS {
            assert_eq!(lb(Some(0)).resolve_period(Some(p)), p);
        }
        assert_eq!(lb(Some(0)).resolve_period(Some("year")), "all_time");
        assert_eq!(lb(Some(2)).resolve_period(Some("quart")), "quarterly");
        assert_eq!(lb(Some(-1)).resolve_period(Some("invalid")), "all_time");
    }

    #[test]
    fn validations() {
        assert_eq!(validate("new", None), vec!["Name new is reserved."]);
        assert!(validate("x", Some(&json!({"like_given": 3}))).is_empty());
        assert_eq!(
            validate("x", Some(&json!({"bogus": -1}))),
            vec![
                "Score overrides contains invalid scorable key: bogus",
                "Score overrides values must be non-negative integers"
            ]
        );
    }
}
