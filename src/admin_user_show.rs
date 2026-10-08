//! Admin::UsersController#show: AdminDetailedUserSerializer (over
//! AdminUserSerializer and AdminUserListSerializer) for a user, with the
//! trust level 3 requirements, the user's groups and how upcoming changes
//! apply to them.
//!
//! Refused: user fields, a DiscourseConnect record, associated accounts, a
//! recent user export, and penalty reasons with markup.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::admin_users::{self, EntryOptions, Row};
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::{Definitions, SiteSettings};
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// UserHistory.actions
const SUSPEND_USER: i32 = 10;
const UNSUSPEND_USER: i32 = 11;
const SILENCE_USER: i32 = 30;
const UNSILENCE_USER: i32 = 31;
const AUTO_TRUST_LEVEL_CHANGE: i32 = 15;
/// UserAction types
const LIKE: i32 = 1;
const WAS_LIKED: i32 = 2;
/// Post.types[:small_action]
const SMALL_ACTION: i32 = 3;
const SYSTEM_USER_ID: i32 = -1;

pub struct Context<'a> {
    pub settings: &'a SiteSettings,
    pub defs: &'a Definitions,
    pub i18n: &'a I18n,
    pub urls: &'a Urls<'a>,
    pub globals: &'a crate::config::GlobalSettings,
    /// Rails.env.development?, which makes every admin a developer.
    pub development: bool,
}

/// The user's detailed admin view, None for an unknown id.
/// The options #show renders with; the role actions render without them.
#[derive(Clone, Copy)]
pub struct ShowOptions {
    /// `include_silence_reason`, `similar_users_count` and `include_ip:
    /// guardian.can_see_ip?`
    pub show: bool,
    /// AdminDetailedUserSerializer, else AdminUserSerializer (trust_level).
    pub detailed: bool,
}

pub async fn show(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    guardian: &Guardian,
    user_id: i32,
    options: ShowOptions,
) -> Result<Option<Value>, AppError> {
    let s = cx.settings;
    let row: Option<Row> =
        sqlx::query_as(&format!("{} WHERE users.id = $1", admin_users::ROW_SELECT))
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(r) = row else {
        return Ok(None);
    };
    #[derive(sqlx::FromRow)]
    struct Extra {
        ip_address: Option<String>,
        registration_ip_address: Option<String>,
        primary_group_id: Option<i32>,
        approved_by_id: Option<i32>,
        email: Option<String>,
        post_edits_count: Option<i32>,
        distinct_badge_count: Option<i32>,
        bounce_score: Option<f64>,
        reset_bounce_score_after: Option<NaiveDateTime>,
    }
    let x: Extra = sqlx::query_as(
        "SELECT host(u.ip_address) AS ip_address, host(u.registration_ip_address) AS registration_ip_address, \
                u.primary_group_id, u.approved_by_id, \
                (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\") AS email, \
                us.post_edits_count, us.distinct_badge_count, us.bounce_score::float8 AS bounce_score, \
                us.reset_bounce_score_after \
         FROM users u LEFT JOIN user_stats us ON us.user_id = u.id WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;

    for (sql, what, detailed_only) in [
        (
            "SELECT EXISTS (SELECT 1 FROM user_custom_fields WHERE user_id = $1 AND name LIKE 'user_field_%')",
            "user fields on the admin view of a user",
            true,
        ),
        (
            "SELECT EXISTS (SELECT 1 FROM single_sign_on_records WHERE user_id = $1)",
            "DiscourseConnect records on the admin view of a user",
            false,
        ),
        (
            "SELECT EXISTS (SELECT 1 FROM user_exports WHERE user_id = $1 AND created_at > now() - interval '2 days')",
            "a recent user export on the admin view of a user",
            true,
        ),
    ] {
        if detailed_only && !options.detailed {
            continue;
        }
        let found: bool = sqlx::query_scalar(sql)
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
        if found {
            return Err(Unsupported(what).into());
        }
    }

    let now = crate::clock::now_naive();
    let suspended = r.suspended_till.is_some_and(|t| t > now);
    let silenced = r.silenced_till.is_some_and(|t| t > now);
    let suspend_record = latest_penalty(conn, user_id, SUSPEND_USER).await?;
    let silence_record = latest_penalty(conn, user_id, SILENCE_USER).await?;
    // full_penalty_reason: the record's sanitized details.
    let full_reason =
        |record: &Option<(Option<i32>, Option<String>)>, active: bool| -> Result<Value, AppError> {
            if !active {
                return Ok(Value::Null);
            }
            let details = record.as_ref().and_then(|(_, d)| d.clone());
            match details {
                None => Ok(Value::Null),
                Some(d) if d.contains(['<', '>', '&', '"', '\'']) => {
                    Err(Unsupported("penalty reasons with markup (sanitize_staff_reason)").into())
                }
                Some(d) => Ok(json!(d.replace('\n', "<br>"))),
            }
        };
    let full_suspend_reason = full_reason(&suspend_record, suspended)?;
    let full_silence_reason = full_reason(&silence_record, silenced)?;
    let first_line = |v: &Value| -> Value {
        v.as_str()
            .map(|s| json!(s.split("<br>").next().unwrap_or_default()))
            .unwrap_or(Value::Null)
    };

    let logo_small_url = admin_users::logo_small_url(conn, s).await?;
    let opts = EntryOptions {
        emails_desired: false,
        can_be_suspended: false,
        silence_reason: options.show.then(|| first_line(&full_silence_reason)),
        suspend_reason: None,
    };
    let mut u = admin_users::entry(
        conn,
        s,
        cx.urls,
        guardian,
        &r,
        logo_small_url.as_deref(),
        &opts,
    )
    .await?;
    // AdminDetailedUserSerializer#include_name?: for admins.
    if options.detailed && !guardian.is_admin() {
        u.remove("name");
    } else {
        u.insert("name".into(), json!(r.name));
    }
    let can_check_emails =
        guardian.is_admin() || (guardian.is_staff() && s.get("moderators_view_emails")?.truthy());
    if admin_users::include_email(guardian, &r, false, can_check_emails) {
        let accounts: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_associated_accounts WHERE user_id = $1)",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if accounts {
            return Err(Unsupported("associated accounts on the admin view of a user").into());
        }
        u.insert("associated_accounts".into(), json!([]));
    }

    let staff = r.admin || r.moderator;
    let can_suspend = guardian.is_staff() && (!staff || guardian.is_admin()) && !staff;
    // AdminUserSerializer
    u.insert(
        "can_send_activation_email".into(),
        json!(guardian.is_staff() && !s.get("must_approve_users")?.truthy()),
    );
    u.insert(
        "can_activate".into(),
        json!(guardian.is_staff() && !r.active),
    );
    u.insert("can_deactivate".into(), json!(can_suspend));
    if s.get("must_approve_users")?.truthy() {
        u.insert(
            "can_approve".into(),
            json!(guardian.is_staff() && r.active && !r.approved),
        );
    }
    u.insert(
        "can_change_trust_level".into(),
        json!(
            guardian.is_admin()
                || (guardian.is_moderator()
                    && s.get("moderators_change_trust_levels")?.truthy()
                    && !staff)
        ),
    );
    let can_see_ip =
        guardian.is_admin() || (guardian.is_moderator() && s.get("moderators_view_ips")?.truthy());
    if can_see_ip {
        u.insert("ip_address".into(), json!(x.ip_address));
        u.insert(
            "registration_ip_address".into(),
            json!(x.registration_ip_address),
        );
    }
    // include_ip: the option, null without it.
    u.insert(
        "include_ip".into(),
        if options.show {
            json!(can_see_ip)
        } else {
            Value::Null
        },
    );
    let can_check_sso = guardian.is_admin()
        || (guardian.is_moderator() && s.get("moderators_view_sso_details")?.truthy());
    if can_check_sso {
        u.insert("single_sign_on_record".into(), Value::Null);
    }

    if !options.detailed {
        return Ok(Some(Value::Object(u)));
    }

    // AdminDetailedUserSerializer
    let administer = guardian.is_admin() && user_id > 0;
    let me = guardian.user_id() == Some(user_id);
    u.insert(
        "can_grant_admin".into(),
        json!(administer && !me && !r.admin),
    );
    u.insert(
        "can_revoke_admin".into(),
        json!(administer && !me && r.admin),
    );
    u.insert(
        "can_grant_moderation".into(),
        json!(administer && !r.moderator),
    );
    u.insert(
        "can_revoke_moderation".into(),
        json!(administer && r.moderator),
    );
    u.insert(
        "can_impersonate".into(),
        json!(
            allow_impersonation(cx)
                && guardian.is_admin()
                && (!r.admin || is_developer(conn, cx, guardian).await?)
        ),
    );
    let count = |sql: &'static str| sqlx::query_scalar::<_, i64>(sql).bind(user_id);
    u.insert(
        "like_count".into(),
        json!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM user_actions WHERE user_id = $1 AND action_type = $2"
            )
            .bind(user_id)
            .bind(WAS_LIKED)
            .fetch_one(&mut *conn)
            .await?
        ),
    );
    u.insert(
        "like_given_count".into(),
        json!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM user_actions WHERE user_id = $1 AND action_type = $2"
            )
            .bind(user_id)
            .bind(LIKE)
            .fetch_one(&mut *conn)
            .await?
        ),
    );
    let post_count = count("SELECT COUNT(*) FROM posts WHERE user_id = $1 AND deleted_at IS NULL")
        .fetch_one(&mut *conn)
        .await?;
    u.insert("post_count".into(), json!(post_count));
    u.insert(
        "topic_count".into(),
        json!(
            count("SELECT COUNT(*) FROM topics WHERE user_id = $1 AND deleted_at IS NULL")
                .fetch_one(&mut *conn)
                .await?
        ),
    );
    u.insert("post_edits_count".into(), json!(x.post_edits_count));
    // flag_types_without_additional_message
    let flags = "SELECT id FROM flags WHERE NOT score_type AND id <> 2 AND NOT require_message";
    u.insert(
        "flags_given_count".into(),
        json!(sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(*) FROM post_actions WHERE user_id = $1 AND deleted_at IS NULL AND post_action_type_id IN ({flags})"
        ))
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?),
    );
    u.insert(
        "flags_received_count".into(),
        json!(sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(*) FROM posts p LEFT JOIN post_actions pa ON pa.post_id = p.id AND pa.deleted_at IS NULL \
             WHERE p.user_id = $1 AND p.deleted_at IS NULL AND pa.post_action_type_id IN ({flags})"
        ))
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?),
    );
    u.insert(
        "private_topics_count".into(),
        json!(
            count(
                "SELECT COUNT(*) FROM topics t JOIN topic_allowed_users tau ON tau.topic_id = t.id \
             WHERE tau.user_id = $1 AND t.deleted_at IS NULL AND t.archetype = 'private_message'"
            )
            .fetch_one(&mut *conn)
            .await?
        ),
    );
    // can_delete_all_posts?
    let delete_all = guardian.is_staff()
        && !r.admin
        && (!r.moderator || guardian.is_admin())
        && (guardian.is_admin()
            || ((r.first_post_created_at.is_none()
                || r.first_post_created_at.is_some_and(|t| {
                    t >= now
                        - chrono::Duration::days(
                            s.get("delete_user_max_post_age")
                                .map(|v| v.to_i())
                                .unwrap_or(0),
                        )
                }))
                && i64::from(r.post_count.unwrap_or(0)) <= s.get("delete_all_posts_max")?.to_i()));
    u.insert("can_delete_all_posts".into(), json!(delete_all));
    // can_anonymize_user?: not an anonymized address.
    let anonymized = x
        .email
        .as_deref()
        .is_some_and(|e| e.ends_with("@anonymized.invalid"));
    u.insert(
        "can_be_anonymized".into(),
        json!(guardian.is_staff() && !staff && !anonymized),
    );
    u.insert("can_be_merged".into(), json!(guardian.is_admin() && !staff));
    u.insert("full_suspend_reason".into(), full_suspend_reason);
    u.insert("full_silence_reason".into(), full_silence_reason);
    let penalties = penalty_counts(conn, user_id, silenced, suspended).await?;
    u.insert(
        "penalty_counts".into(),
        json!({ "silenced": penalties.0, "suspended": penalties.1 }),
    );
    u.insert(
        "next_penalty".into(),
        next_penalty(s, penalties.0 + penalties.1)?,
    );
    u.insert("primary_group_id".into(), json!(x.primary_group_id));
    u.insert("badge_count".into(), json!(x.distinct_badge_count));
    u.insert(
        "warnings_received_count".into(),
        json!(
            count("SELECT COUNT(*) FROM user_warnings WHERE user_id = $1")
                .fetch_one(&mut *conn)
                .await?
        ),
    );
    u.insert("bounce_score".into(), json!(x.bounce_score));
    u.insert(
        "reset_bounce_score_after".into(),
        json!(x.reset_bounce_score_after.map(crate::topic_list::time_json)),
    );
    u.insert("can_view_action_logs".into(), json!(guardian.is_staff()));
    u.insert(
        "can_disable_second_factor".into(),
        json!(guardian.is_admin() && !me),
    );
    u.insert(
        "can_delete_sso_record".into(),
        json!(s.get("enable_discourse_connect")?.truthy() && guardian.is_admin()),
    );
    u.insert(
        "api_key_count".into(),
        json!(
            count("SELECT COUNT(*) FROM api_keys WHERE user_id = $1 AND revoked_at IS NULL")
                .fetch_one(&mut *conn)
                .await?
        ),
    );
    if guardian.is_admin() {
        u.insert("external_ids".into(), json!({}));
    }
    // similar_users: other real, non-staff users at the same IP.
    let similar: i64 =
        match &x.ip_address {
            None => 0,
            Some(ip) => sqlx::query_scalar(
                "SELECT COUNT(*) FROM users WHERE id > 0 AND id <> $1 AND ip_address = $2::inet \
                 AND NOT admin AND NOT moderator \
                 AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id)",
            )
            .bind(user_id)
            .bind(ip)
            .fetch_one(&mut *conn)
            .await?,
        };
    if options.show {
        u.insert("similar_users_count".into(), json!(similar));
    }
    // include_latest_export?: can_export_entity?("user_archive", id).
    if guardian.is_admin() || guardian.is_moderator() {
        u.insert("latest_export".into(), Value::Null);
    }
    if guardian.is_staff() {
        u.insert(
            "upcoming_changes_stats".into(),
            upcoming_changes_stats(conn, cx, guardian, user_id).await?,
        );
    }
    u.insert(
        "approved_by".into(),
        basic_user(conn, cx, x.approved_by_id, logo_small_url.as_deref()).await?,
    );
    u.insert(
        "suspended_by".into(),
        basic_user(
            conn,
            cx,
            suspend_record.and_then(|(a, _)| a),
            logo_small_url.as_deref(),
        )
        .await?,
    );
    u.insert(
        "silenced_by".into(),
        basic_user(
            conn,
            cx,
            silence_record.and_then(|(a, _)| a),
            logo_small_url.as_deref(),
        )
        .await?,
    );
    // has_trust_level?(2): staff, or at least member.
    if staff || r.trust_level >= 2 {
        u.insert(
            "tl3_requirements".into(),
            tl3_requirements(conn, s, &r, silenced, suspended, penalties).await?,
        );
    }
    u.insert("groups".into(), groups(conn, cx, guardian, user_id).await?);
    Ok(Some(Value::Object(u)))
}

/// `TrustLevel3Requirements#requirements_met?` and `#requirements_lost?`
/// for Promotion.
pub(crate) async fn tl3_met_lost(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user_id: i32,
) -> Result<(bool, bool), AppError> {
    let r: Row = sqlx::query_as(&format!("{} WHERE users.id = $1", admin_users::ROW_SELECT))
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    let now = crate::clock::now_naive();
    let suspended = r.suspended_till.is_some_and(|t| t > now);
    let silenced = r.silenced_till.is_some_and(|t| t > now);
    let penalties = penalty_counts(conn, user_id, silenced, suspended).await?;
    let req = tl3_requirements(conn, s, &r, silenced, suspended, penalties).await?;
    Ok((
        req["requirements_met"] == true,
        req["requirements_lost"] == true,
    ))
}
/// `suspend_record` / `silenced_record`: the latest such log, its acting
/// user and details.
async fn latest_penalty(
    conn: &mut PgConnection,
    user_id: i32,
    action: i32,
) -> Result<Option<(Option<i32>, Option<String>)>, AppError> {
    Ok(sqlx::query_as(
        "SELECT acting_user_id, details FROM user_histories WHERE target_user_id = $1 AND action = $2 \
         ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(action)
    .fetch_optional(&mut *conn)
    .await?)
}

/// BasicUserSerializer embedded as an object, null for nobody.
pub(crate) async fn basic_user(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    user_id: Option<i32>,
    logo_small_url: Option<&str>,
) -> Result<Value, AppError> {
    let Some(id) = user_id else {
        return Ok(Value::Null);
    };
    let row: Option<(String, Option<String>, Option<i32>)> =
        sqlx::query_as("SELECT username, name, uploaded_avatar_id FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((username, name, avatar)) = row else {
        return Ok(Value::Null);
    };
    let mut out = Map::new();
    out.insert("id".into(), json!(id));
    out.insert("username".into(), json!(username));
    if cx.settings.get("enable_names")?.truthy() {
        out.insert("name".into(), json!(name));
    }
    out.insert(
        "avatar_template".into(),
        json!(crate::avatar::avatar_template(
            cx.urls,
            id,
            &username,
            avatar,
            logo_small_url
        )?),
    );
    Ok(Value::Object(out))
}

/// GlobalSetting.allow_impersonation, true by default.
fn allow_impersonation(cx: &Context<'_>) -> bool {
    cx.globals
        .get("allow_impersonation")
        .is_none_or(|v| !matches!(v, "false" | "0" | ""))
}

/// `is_developer?`: in development, a Developer row, or a developer email.
async fn is_developer(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    guardian: &Guardian,
) -> Result<bool, AppError> {
    if cx.development {
        return Ok(true);
    }
    let Some(uid) = guardian.user_id() else {
        return Ok(false);
    };
    if guardian.is_admin() {
        let developer: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM developers WHERE user_id = $1)")
                .bind(uid)
                .fetch_one(&mut *conn)
                .await?;
        if developer {
            return Ok(true);
        }
    }
    if let Some(emails) = cx.globals.get("developer_emails").filter(|e| !e.is_empty()) {
        let email: Option<String> =
            sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
                .bind(uid)
                .fetch_optional(&mut *conn)
                .await?;
        return Ok(email.is_some_and(|e| emails.split(',').any(|d| d.trim() == e)));
    }
    Ok(false)
}

/// TrustLevel3Requirements#penalty_counts: penalties of the last six
/// months, less those lifted by someone other than the system, a current
/// one counted if none is.
async fn penalty_counts(
    conn: &mut PgConnection,
    user_id: i32,
    silenced: bool,
    suspended: bool,
) -> Result<(i64, i64), AppError> {
    let (silence, suspend): (Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT SUM(CASE WHEN action = $2 THEN 1 WHEN action = $3 AND acting_user_id <> $6 THEN -1 ELSE 0 END)::bigint, \
                SUM(CASE WHEN action = $4 THEN 1 WHEN action = $5 AND acting_user_id <> $6 THEN -1 ELSE 0 END)::bigint \
         FROM user_histories uh WHERE uh.target_user_id = $1 AND uh.action IN ($2, $3, $4, $5) AND uh.created_at > $7",
    )
    .bind(user_id)
    .bind(SILENCE_USER)
    .bind(UNSILENCE_USER)
    .bind(SUSPEND_USER)
    .bind(UNSUSPEND_USER)
    .bind(SYSTEM_USER_ID)
    .bind(crate::clock::now_naive() - chrono::Months::new(6))
    .fetch_one(&mut *conn)
    .await?;
    let mut silence = silence.unwrap_or(0);
    let mut suspend = suspend.unwrap_or(0);
    if silence == 0 && silenced {
        silence += 1;
    }
    if suspend == 0 && suspended {
        suspend += 1;
    }
    Ok((silence, suspend))
}

/// `next_penalty`: the step of penalty_step_hours after the user's
/// penalties, from now.
fn next_penalty(s: &SiteSettings, total: i64) -> Result<Value, AppError> {
    let steps = s.get("penalty_step_hours")?.to_s();
    let steps: Vec<&str> = steps.split('|').collect();
    let step = (total as usize).min(steps.len());
    let Some(hours) = steps.get(step).and_then(|h| h.parse::<i64>().ok()) else {
        return Ok(Value::Null);
    };
    Ok(json!(crate::topic_list::time_json(
        crate::clock::now_naive() + chrono::Duration::hours(hours)
    )))
}

/// TrustLevel3RequirementsSerializer
async fn tl3_requirements(
    conn: &mut PgConnection,
    s: &SiteSettings,
    r: &Row,
    silenced: bool,
    suspended: bool,
    penalties: (i64, i64),
) -> Result<Value, AppError> {
    let id = r.id;
    let now = crate::clock::now_naive();
    let period = s.get("tl3_time_period")?.to_i();
    let since = now - chrono::Duration::days(period);
    let setting = |name: &str| s.get(name).map(|v| v.to_i());
    let one = |sql: &'static str| sqlx::query_scalar::<_, i64>(sql);

    let days_visited = one("SELECT COUNT(*) FROM user_visits WHERE user_id = $1 AND visited_at > $2 AND posts_read > 0")
        .bind(id)
        .bind(since.date())
        .fetch_one(&mut *conn)
        .await?;
    let num_topics_replied_to: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(DISTINCT posts.topic_id) FROM posts INNER JOIN topics ON topics.id = posts.topic_id \
         WHERE posts.user_id = $1 AND topics.user_id <> posts.user_id AND posts.deleted_at IS NULL \
           AND topics.deleted_at IS NULL AND topics.archetype <> 'private_message' \
           AND posts.post_type <> {SMALL_ACTION} AND posts.created_at > $2"
    ))
    .bind(id)
    .bind(since)
    .fetch_one(&mut *conn)
    .await?;
    let viewed = "SELECT COUNT(*) FROM topic_views tv JOIN topics t ON t.id = tv.topic_id \
                  WHERE tv.user_id = $1 AND t.archetype <> 'private_message'";
    let topics_viewed: i64 = sqlx::query_scalar(&format!("{viewed} AND tv.viewed_at > $2"))
        .bind(id)
        .bind(since.date())
        .fetch_one(&mut *conn)
        .await?;
    let topics_viewed_all_time: i64 = sqlx::query_scalar(viewed)
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    let posts_read: i64 = one("SELECT COALESCE(SUM(posts_read), 0)::bigint FROM user_visits WHERE user_id = $1 AND visited_at > $2")
        .bind(id)
        .bind(since.date())
        .fetch_one(&mut *conn)
        .await?;
    let posts_read_all_time: i64 =
        one("SELECT COALESCE(SUM(posts_read), 0)::bigint FROM user_visits WHERE user_id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    // num_topics_in_time_period / num_posts_in_time_period (Rails caches
    // them for a day).
    let num_topics: i64 = one(
        "SELECT COUNT(*) FROM topics WHERE archetype <> 'private_message' AND deleted_at IS NULL \
         AND visible AND created_at > $1",
    )
    .bind(since)
    .fetch_one(&mut *conn)
    .await?;
    let num_posts: i64 = one(
        "SELECT COUNT(*) FROM posts p JOIN topics t ON t.id = p.topic_id \
         WHERE p.deleted_at IS NULL AND t.deleted_at IS NULL AND t.archetype <> 'private_message' \
           AND t.visible AND NOT p.hidden AND p.created_at > $1",
    )
    .bind(since)
    .fetch_one(&mut *conn)
    .await?;
    let min_topics_viewed = ((num_topics as f64
        * (setting("tl3_requires_topics_viewed")? as f64 / 100.0))
        .round() as i64)
        .min(setting("tl3_requires_topics_viewed_cap")?);
    let min_posts_read = ((num_posts as f64 * (setting("tl3_requires_posts_read")? as f64 / 100.0))
        .round() as i64)
        .min(setting("tl3_requires_posts_read_cap")?);
    // flagged_post_ids: the user's posts of the period with spam or
    // inappropriate flags, deleted ones too; agreed flags by others.
    let flagged = "SELECT id FROM posts WHERE user_id = $1 AND created_at > $2 \
                   AND (spam_count > 0 OR inappropriate_count > 0)";
    let (num_flagged_posts, num_flagged_by_users): (i64, i64) = sqlx::query_as(&format!(
        "SELECT COUNT(DISTINCT post_id), COUNT(DISTINCT user_id) FROM post_actions \
         WHERE post_id IN ({flagged}) AND user_id <> $1 AND agreed_at IS NOT NULL"
    ))
    .bind(id)
    .bind(since)
    .fetch_one(&mut *conn)
    .await?;
    let likes = |action: i32| {
        format!(
            "FROM user_actions JOIN topics ON topics.id = user_actions.target_topic_id \
             WHERE user_actions.user_id = $1 AND user_actions.action_type = {action} \
               AND user_actions.created_at > $2 AND topics.archetype <> 'private_message'"
        )
    };
    let num_likes_given: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) {}", likes(LIKE)))
        .bind(id)
        .bind(since)
        .fetch_one(&mut *conn)
        .await?;
    let (num_likes_received, num_likes_received_days, num_likes_received_users): (i64, i64, i64) =
        sqlx::query_as(&format!(
            "SELECT COUNT(*), COUNT(DISTINCT date(user_actions.created_at)), COUNT(DISTINCT user_actions.acting_user_id) {}",
            likes(WAS_LIKED)
        ))
        .bind(id)
        .bind(since)
        .fetch_one(&mut *conn)
        .await?;

    let min_days_visited = setting("tl3_requires_days_visited")?;
    let min_topics_replied_to = setting("tl3_requires_topics_replied_to")?;
    let min_topics_viewed_all_time = setting("tl3_requires_topics_viewed_all_time")?;
    let min_posts_read_all_time = setting("tl3_requires_posts_read_all_time")?;
    let max_flagged = setting("tl3_requires_max_flagged")?;
    let min_likes_given = setting("tl3_requires_likes_given")?;
    let min_likes_received = setting("tl3_requires_likes_received")?;
    let min_likes_received_days =
        ((min_likes_received as f64 / 3.0).ceil() as i64).min((0.75 * period as f64).ceil() as i64);
    let min_likes_received_users = (min_likes_received as f64 / 4.0).ceil() as i64;
    let locked = r.manual_locked_trust_level.is_some();
    let total_penalties = penalties.0 + penalties.1;

    let met = !locked
        && !suspended
        && !silenced
        && total_penalties == 0
        && days_visited >= min_days_visited
        && num_topics_replied_to >= min_topics_replied_to
        && topics_viewed >= min_topics_viewed
        && posts_read >= min_posts_read
        && num_flagged_posts <= max_flagged
        && num_flagged_by_users <= max_flagged
        && topics_viewed_all_time >= min_topics_viewed_all_time
        && posts_read_all_time >= min_posts_read_all_time
        && num_likes_given >= min_likes_given
        && num_likes_received >= min_likes_received
        && num_likes_received_users >= min_likes_received_users
        && num_likes_received_days >= min_likes_received_days;
    const LOW_WATER_MARK: f64 = 0.9;
    let below = |have: i64, min: i64| (have as f64) < min as f64 * LOW_WATER_MARK;
    let lost = !locked
        && setting("default_trust_level")? <= 2
        && (suspended
            || silenced
            || total_penalties > 0
            || below(days_visited, min_days_visited)
            || below(num_topics_replied_to, min_topics_replied_to)
            || below(topics_viewed, min_topics_viewed)
            || below(posts_read, min_posts_read)
            || num_flagged_posts > max_flagged
            || num_flagged_by_users > max_flagged
            || topics_viewed_all_time < min_topics_viewed_all_time
            || posts_read_all_time < min_posts_read_all_time
            || below(num_likes_given, min_likes_given)
            || below(num_likes_received, min_likes_received)
            || below(num_likes_received_users, min_likes_received_users)
            || below(num_likes_received_days, min_likes_received_days));

    // on_tl3_grace_period?: promoted to leader within the minimum duration.
    let min_duration = setting("tl3_promotion_min_duration")?;
    let cutoff = now - chrono::Duration::days(min_duration);
    let on_grace_period = if chrono::Datelike::year(&cutoff) < 2013 {
        true
    } else {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_histories WHERE target_user_id = $1 AND action = $2 \
             AND created_at >= $3 AND previous_value = '2' AND new_value = '3')",
        )
        .bind(id)
        .bind(AUTO_TRUST_LEVEL_CHANGE)
        .bind(cutoff)
        .fetch_one(&mut *conn)
        .await?
    };

    Ok(json!({
        "time_period": period,
        "requirements_met": met,
        "requirements_lost": lost,
        "trust_level_locked": locked,
        "on_grace_period": on_grace_period,
        "days_visited": days_visited,
        "min_days_visited": min_days_visited,
        "num_topics_replied_to": num_topics_replied_to,
        "min_topics_replied_to": min_topics_replied_to,
        "topics_viewed": topics_viewed,
        "min_topics_viewed": min_topics_viewed,
        "posts_read": posts_read,
        "min_posts_read": min_posts_read,
        "topics_viewed_all_time": topics_viewed_all_time,
        "min_topics_viewed_all_time": min_topics_viewed_all_time,
        "posts_read_all_time": posts_read_all_time,
        "min_posts_read_all_time": min_posts_read_all_time,
        "num_flagged_posts": num_flagged_posts,
        "max_flagged_posts": max_flagged,
        "num_flagged_by_users": num_flagged_by_users,
        "max_flagged_by_users": max_flagged,
        "num_likes_given": num_likes_given,
        "min_likes_given": min_likes_given,
        "num_likes_received": num_likes_received,
        "min_likes_received": min_likes_received,
        "num_likes_received_days": num_likes_received_days,
        "min_likes_received_days": min_likes_received_days,
        "num_likes_received_users": num_likes_received_users,
        "min_likes_received_users": min_likes_received_users,
        "penalty_counts": {
            "silenced": penalties.0,
            "suspended": penalties.1,
            "total": penalties.0 + penalties.1,
        },
    }))
}

/// `object.groups.visible_groups(scope.user)` through BasicGroupSerializer,
/// by name.
async fn groups(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    guardian: &Guardian,
    user_id: i32,
) -> Result<Value, AppError> {
    let visible = crate::groups::visible_groups_where(guardian, "g");
    let groups: Vec<crate::groups::BasicGroup> = sqlx::query_as(&format!(
        "SELECT {} FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id \
         JOIN group_users gu ON gu.group_id = g.id \
         WHERE gu.user_id = $1 AND g.id > 0 AND ({visible}) ORDER BY g.name ASC",
        crate::groups::BASIC_GROUP_COLUMNS
    ))
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let memberships: Vec<(i32, bool)> = match guardian.user_id() {
        Some(uid) => {
            sqlx::query_as("SELECT group_id, owner FROM group_users WHERE user_id = $1")
                .bind(uid)
                .fetch_all(&mut *conn)
                .await?
        }
        None => Vec::new(),
    };
    let mut out = Vec::with_capacity(groups.len());
    for g in &groups {
        let membership = memberships
            .iter()
            .find(|(id, _)| *id == g.id)
            .map(|(_, owner)| (true, *owner));
        out.push(g.json(cx.i18n, guardian, cx.settings, membership)?);
    }
    Ok(Value::Array(out))
}

/// UpcomingChanges.stats_for_user: each upcoming change past conceptual,
/// whether it applies to the user and why.
async fn upcoming_changes_stats(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    guardian: &Guardian,
    user_id: i32,
) -> Result<Value, AppError> {
    use crate::site_settings::ChangeStatus;
    let belonging: Vec<i32> =
        sqlx::query_scalar("SELECT group_id FROM group_users WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(&mut *conn)
            .await?;
    let visible = crate::groups::visible_groups_where(guardian, "g");
    let visible_ids: Vec<i32> = sqlx::query_scalar(&format!(
        "SELECT g.id FROM groups g WHERE g.id > 0 AND ({visible})"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let labels = crate::admin_site_settings::Context {
        defs: cx.defs,
        settings: cx.settings,
        i18n: cx.i18n,
        globals: cx.globals,
        base_path: cx.urls.config.globals.relative_url_root(),
        urls: cx.urls,
    };
    let mut out = Vec::new();
    for def in cx.defs.iter() {
        let Some(status) = def.upcoming_change else {
            continue;
        };
        if status == ChangeStatus::Conceptual {
            continue;
        }
        let name = def.name.as_str();
        let group_ids: Vec<i32> = sqlx::query_scalar::<_, Option<String>>(
            "SELECT group_ids FROM site_setting_groups WHERE name = $1",
        )
        .bind(name)
        .fetch_optional(&mut *conn)
        .await?
        .flatten()
        .map(|list| {
            list.split('|')
                .filter(|s| !s.is_empty())
                .map(|s| crate::ruby::to_i(s) as i32)
                .collect()
        })
        .unwrap_or_default();
        let on = cx.settings.get(name)?.truthy();
        let enabled =
            on && (group_ids.is_empty() || group_ids.iter().any(|g| belonging.contains(g)));
        let (specific, reason) = if !group_ids.is_empty() {
            let ids: Vec<i32> = group_ids
                .iter()
                .copied()
                .filter(|g| visible_ids.contains(g) && belonging.contains(g))
                .collect();
            let names: Vec<String> =
                sqlx::query_scalar("SELECT name FROM groups WHERE id = ANY($1)")
                    .bind(&ids)
                    .fetch_all(&mut *conn)
                    .await?;
            (
                names,
                if enabled {
                    "in_specific_groups"
                } else {
                    "not_in_specific_groups"
                },
            )
        } else if enabled {
            (Vec::new(), "enabled_for_everyone")
        } else {
            (Vec::new(), "enabled_for_no_one")
        };
        out.push(json!({
            "name": name,
            "humanized_name": crate::admin_site_settings::humanized_name(name),
            "description": crate::admin_site_settings::description(&labels, name),
            "enabled": enabled,
            "specific_groups": specific,
            "reason": reason,
        }));
    }
    // The changes come back by name.
    out.sort_by_key(|c| c["name"].as_str().unwrap_or_default().to_string());
    Ok(Value::Array(out))
}
