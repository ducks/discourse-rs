//! Penalties staff put on users: suspending and unsuspending
//! (`User::Suspend` -> UserSuspender, `UserSuspender.unsuspend`).
//!
//! Refused: penalizing several users at once (other_user_ids), acting on a
//! post with the penalty, reviewables, and suspending an already suspended
//! user (its 409 message says how long ago, from the suspension's log).

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::topic_list::time_json;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `UserHistory.actions`
const SUSPEND_USER: i32 = 10;
const UNSUSPEND_USER: i32 = 11;

/// How a penalty request ends when it isn't a server error.
pub enum Outcome {
    /// The rendered JSON.
    Done(Value),
    /// `Discourse::NotFound`
    NotFound,
    /// `Discourse::InvalidAccess`
    Forbidden,
    /// A failed contract: 400 with the full messages.
    Invalid(Vec<String>),
}

/// What a penalty request carries.
pub struct Penalty<'a> {
    pub reason: Option<&'a str>,
    pub message: Option<&'a str>,
    /// `suspend_until` / `silenced_till`, as sent.
    pub until: Option<&'a str>,
    /// Params this port does not take: other_user_ids, post_id,
    /// post_action, reviewable_id.
    pub unported: Option<&'static str>,
}

/// ActiveModel's `:datetime` cast for the formats the admin UI sends;
/// None when blank. Other formats are not ported.
fn cast_time(value: Option<&str>) -> Result<Option<NaiveDateTime>, Unsupported> {
    let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(v) {
        return Ok(Some(t.naive_utc()));
    }
    if let Ok(t) = NaiveDateTime::parse_from_str(v, "%Y-%m-%d %H:%M:%S") {
        return Ok(Some(t));
    }
    if let Ok(d) = NaiveDate::parse_from_str(v, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(0, 0, 0));
    }
    Err(Unsupported("penalty times in other formats"))
}

/// The target as the guardian and the penalty read it.
#[derive(sqlx::FromRow)]
struct Target {
    id: i32,
    admin: bool,
    moderator: bool,
    suspended_till: Option<NaiveDateTime>,
}

async fn target(conn: &mut PgConnection, user_id: i32) -> Result<Option<Target>, sqlx::Error> {
    sqlx::query_as("SELECT id, admin, moderator, suspended_till FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(conn)
        .await
}

/// `can_unsuspend?(user)`
fn can_unsuspend(guardian: &Guardian, user: &Target) -> bool {
    guardian.is_staff() && (!(user.admin || user.moderator) || guardian.is_admin())
}

/// `StaffMessageFormat#format`: the reason, then the message.
fn staff_message(reason: &str, message: Option<&str>) -> String {
    let mut out = reason.to_string();
    if let Some(m) = message.filter(|m| !m.trim().is_empty()) {
        out.push_str("\n\n");
        out.push_str(m);
    }
    out
}

/// `BasicUserSerializer`
pub(crate) async fn basic_user(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
) -> Result<Value, AppError> {
    let (username, name, uploaded_avatar_id): (String, Option<String>, Option<i32>) =
        sqlx::query_as("SELECT username, name, uploaded_avatar_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(conn)
            .await?;
    let urls = Urls {
        config: ctx.config,
        settings: ctx.settings,
    };
    let avatar =
        crate::avatar::avatar_template(&urls, user_id, &username, uploaded_avatar_id, None)?;
    Ok(json!({ "id": user_id, "username": username, "name": name, "avatar_template": avatar }))
}

/// The contract's errors for a reason and an until time, in declaration
/// order, as full messages.
fn contract_errors(
    ctx: &Ctx<'_>,
    reason: Option<&str>,
    until_attribute: &str,
    until: Option<NaiveDateTime>,
) -> Vec<String> {
    let mut errors = Vec::new();
    let blank = |attribute: &str| {
        ctx.i18n
            .t_with("errors.messages.blank", &[])
            .map(|m| format!("{attribute} {m}"))
            .unwrap_or_else(|| format!("{attribute} can't be blank"))
    };
    match reason.map(str::trim).filter(|r| !r.is_empty()) {
        None => errors.push(blank("Reason")),
        Some(r) if r.chars().count() > 300 => {
            errors.push("Reason is too long (maximum is 300 characters)".into())
        }
        Some(_) => {}
    }
    if until.is_none() {
        errors.push(blank(until_attribute));
    }
    errors
}

/// `User::Suspend.call`
pub async fn suspend(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    user_id: i32,
    p: &Penalty<'_>,
) -> Result<Outcome, AppError> {
    let until = cast_time(p.until)?;
    let errors = contract_errors(ctx, p.reason, "Suspend until", until);
    if !errors.is_empty() {
        return Ok(Outcome::Invalid(errors));
    }
    if let Some(what) = p.unported {
        return Err(Unsupported(what).into());
    }
    let (reason, until) = (p.reason.unwrap_or_default(), until.unwrap_or_default());
    let Some(user) = target(&mut *conn, user_id).await? else {
        return Ok(Outcome::NotFound);
    };
    // not_suspended_already
    if user
        .suspended_till
        .is_some_and(|t| t > crate::clock::now_naive())
    {
        return Err(Unsupported("suspending an already suspended user").into());
    }
    // can_suspend_all_users: can_unsuspend? && user.regular?
    if !(can_unsuspend(guardian, &user) && !(user.admin || user.moderator)) {
        return Ok(Outcome::Forbidden);
    }
    let actor = guardian
        .user()
        .ok_or(Unsupported("suspending anonymously"))?;

    // UserSuspender#suspend
    let suspended_at: NaiveDateTime = sqlx::query_scalar(
        "UPDATE users SET suspended_till = $2, suspended_at = clock_timestamp(), updated_at = clock_timestamp() \
         WHERE id = $1 RETURNING suspended_at",
    )
    .bind(user.id)
    .bind(until)
    .fetch_one(&mut *conn)
    .await?;
    let details = staff_message(reason, p.message);
    let history_id: i32 = sqlx::query_scalar(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, details, admin_only, \
                                     created_at, updated_at) \
         VALUES ($1, $2, $3, $4, FALSE, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(SUSPEND_USER)
    .bind(actor.id)
    .bind(user.id)
    .bind(&details)
    .fetch_one(&mut *conn)
    .await?;
    // log_out!: push subscriptions cleared (the rest is MessageBus).
    sqlx::query("DELETE FROM push_subscriptions WHERE user_id = $1")
        .bind(user.id)
        .execute(&mut *conn)
        .await?;
    let iso6 = |t: NaiveDateTime| t.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    crate::jobs::enqueue_at(
        &mut *conn,
        until,
        "user_suspension_expired",
        json!({
            "user_id": user.id,
            "suspended_at": iso6(suspended_at),
            "suspended_till": iso6(until),
        }),
    )
    .await?;
    if p.message.is_some_and(|m| !m.trim().is_empty()) {
        // Enqueued by class, as UserSuspender does.
        crate::jobs::enqueue(
            &mut *conn,
            "Jobs::CriticalUserEmail",
            json!({ "type": "account_suspended", "user_id": user.id, "user_history_id": history_id }),
        )
        .await?;
    }
    Ok(Outcome::Done(json!({
        "suspension": {
            "suspend_reason": reason,
            "full_suspend_reason": details,
            "suspended_till": time_json(until),
            "suspended_at": time_json(suspended_at),
            "suspended_by": basic_user(&mut *conn, ctx, actor.id).await?,
        }
    })))
}

/// `Admin::UsersController#unsuspend` -> `UserSuspender.unsuspend`
pub async fn unsuspend(
    conn: &mut PgConnection,
    guardian: &Guardian,
    user_id: i32,
) -> Result<Outcome, AppError> {
    let Some(user) = target(&mut *conn, user_id).await? else {
        return Ok(Outcome::NotFound);
    };
    if !can_unsuspend(guardian, &user) {
        return Ok(Outcome::Forbidden);
    }
    let actor = guardian
        .user()
        .ok_or(Unsupported("unsuspending anonymously"))?;
    if user.suspended_till.is_some() {
        sqlx::query(
            "UPDATE users SET suspended_at = NULL, suspended_till = NULL, updated_at = clock_timestamp() \
             WHERE id = $1",
        )
        .bind(user.id)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, target_user_id, admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, FALSE, clock_timestamp(), clock_timestamp())",
        )
        .bind(UNSUSPEND_USER)
        .bind(actor.id)
        .bind(user.id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(Outcome::Done(
        json!({ "suspension": { "suspended_till": null, "suspended_at": null } }),
    ))
}

/// `UserHistory.actions`
const SILENCE_USER: i32 = 30;
const UNSILENCE_USER: i32 = 31;

/// `User.sanitize_staff_reason` for plain text: line breaks as `<br>`.
/// Reasons with markup would need the sanitizer, not ported.
fn staff_reason_html(text: &str) -> Result<String, Unsupported> {
    if text.contains(['<', '>', '&']) {
        return Err(Unsupported(
            "staff reasons with markup (the reason sanitizer)",
        ));
    }
    Ok(text.replace('\n', "<br>"))
}

/// `User::Silence.call`: UserSilencer#silence keeping the user's posts,
/// then the account_silenced email.
pub async fn silence(
    pool: &sqlx::PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    user_id: i32,
    p: &Penalty<'_>,
) -> Result<Outcome, AppError> {
    let until = cast_time(p.until)?;
    let errors = contract_errors(ctx, p.reason, "Silenced till", until);
    if !errors.is_empty() {
        return Ok(Outcome::Invalid(errors));
    }
    if let Some(what) = p.unported {
        return Err(Unsupported(what).into());
    }
    let (reason, until) = (p.reason.unwrap_or_default(), until.unwrap_or_default());
    let mut tx = pool.begin().await?;
    let row: Option<(i32, bool, bool, Option<NaiveDateTime>)> =
        sqlx::query_as("SELECT id, admin, moderator, silenced_till FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((user_id, admin, moderator, silenced_till)) = row else {
        return Ok(Outcome::NotFound);
    };
    // not_silenced_already (its 409 says how long ago), and UserSilencer
    // doing nothing for a past silence still on record.
    if silenced_till.is_some() {
        return Err(Unsupported("silencing a user with a silence on record").into());
    }
    // can_silence_user?
    if !(guardian.is_staff() && !(admin || moderator)) {
        return Ok(Outcome::Forbidden);
    }
    let actor = guardian
        .user()
        .ok_or(Unsupported("silencing anonymously"))?;

    sqlx::query(
        "UPDATE users SET silenced_till = $2, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(user_id)
    .bind(until)
    .execute(&mut *tx)
    .await?;
    let details = staff_message(reason, p.message);
    let (history_id, silenced_at): (i32, NaiveDateTime) = sqlx::query_as(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, context, details, admin_only, \
                                     created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, FALSE, clock_timestamp(), clock_timestamp()) RETURNING id, created_at",
    )
    .bind(SILENCE_USER)
    .bind(actor.id)
    .bind(user_id)
    .bind(format!("silenced_by_staff: {reason}"))
    .bind(&details)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    crate::system_message::create(
        pool,
        ctx,
        user_id,
        "silenced_by_staff",
        &[],
        Some(json!({ "skip_send_email": true })),
    )
    .await?;
    let mut conn = pool.acquire().await?;
    crate::jobs::enqueue(
        &mut conn,
        "critical_user_email",
        json!({ "type": "account_silenced", "user_id": user_id, "user_history_id": history_id }),
    )
    .await?;
    let full_reason = staff_reason_html(&details)?;
    let silence_reason = full_reason
        .split("<br>")
        .next()
        .unwrap_or_default()
        .to_string();
    Ok(Outcome::Done(json!({
        "silence": {
            "silenced": true,
            "silence_reason": silence_reason,
            "full_silence_reason": details,
            "silenced_till": time_json(until),
            "silenced_at": time_json(silenced_at),
            "silenced_by": basic_user(&mut conn, ctx, actor.id).await?,
        }
    })))
}

/// `Admin::UsersController#unsilence` -> `UserSilencer.unsilence`
pub async fn unsilence(
    pool: &sqlx::PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    user_id: i32,
) -> Result<Outcome, AppError> {
    let mut conn = pool.acquire().await?;
    let row: Option<(i32, bool, bool, Option<NaiveDateTime>)> =
        sqlx::query_as("SELECT id, admin, moderator, silenced_till FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((user_id, admin, moderator, silenced_till)) = row else {
        return Ok(Outcome::NotFound);
    };
    // can_unsilence_user?
    if !(guardian.is_staff() && (!(admin || moderator) || guardian.is_admin())) {
        return Ok(Outcome::Forbidden);
    }
    let actor = guardian
        .user()
        .ok_or(Unsupported("unsilencing anonymously"))?;
    if silenced_till.is_some() {
        sqlx::query(
            "UPDATE users SET silenced_till = NULL, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
        drop(conn);
        crate::system_message::create(pool, ctx, user_id, "unsilenced", &[], None).await?;
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, target_user_id, admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, FALSE, clock_timestamp(), clock_timestamp())",
        )
        .bind(UNSILENCE_USER)
        .bind(actor.id)
        .bind(user_id)
        .execute(pool)
        .await?;
    }
    Ok(Outcome::Done(json!({
        "unsilence": {
            "silenced": false,
            "silence_reason": null,
            "full_silence_reason": null,
            "silenced_till": null,
            "silenced_at": null,
        }
    })))
}
