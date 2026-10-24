//! Penalties staff put on users: suspending and silencing one user and
//! the similar users picked with it (`User::Suspend`, `User::Silence`,
//! `User::BulkSuspend` -> UserSuspender, UserSilencer), with the action
//! on the post that prompted it (`User::Action::TriggerPostAction`), and
//! lifting them (`UserSuspender.unsuspend`, `UserSilencer#unsilence`).

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::topic_list::time_json;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `UserHistory.actions`
const SUSPEND_USER: i32 = 10;
const UNSUSPEND_USER: i32 = 11;

/// `User::MAX_SIMILAR_USERS`
pub const MAX_SIMILAR_USERS: usize = 10;

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
    /// Already suspended or silenced: 409 with the policy's reason.
    Conflict(String),
}

/// What a penalty request carries, cast as the service's params contract
/// casts it.
#[derive(Default)]
pub struct Penalty<'a> {
    pub reason: Option<&'a str>,
    pub message: Option<&'a str>,
    /// `suspend_until` / `silenced_till`, as sent.
    pub until: Option<&'a str>,
    /// `other_user_ids` (`:array`): the items, each an integer when it
    /// reads as one.
    pub other_user_ids: Vec<ArrayItem>,
    pub post_id: Option<i64>,
    pub post_action: Option<String>,
    pub post_edit: Option<String>,
    pub reviewable_id: Option<i64>,
}

/// An item of an `:array` param: `Integer(item, exception: false) || item`.
#[derive(Clone, Debug, PartialEq)]
pub enum ArrayItem {
    Int(i64),
    Text(String),
}

impl ArrayItem {
    /// The id `User.where(id:)` matches it with (ActiveModel's integer
    /// cast), if any.
    pub fn id(&self) -> Option<i32> {
        match self {
            ArrayItem::Int(n) => i32::try_from(*n).ok(),
            ArrayItem::Text(s) => cast_integer(s).and_then(|n| i32::try_from(n).ok()),
        }
    }

    pub fn is_blank(&self) -> bool {
        matches!(self, ArrayItem::Text(s) if crate::ruby::is_blank(s))
    }
}

/// `ActiveSupportTypeExtensions::Array#cast_value`: a string split on
/// commas (or pipes), an array's items as integers where they are ones.
pub fn cast_array(value: Option<&Value>) -> Result<Vec<ArrayItem>, Unsupported> {
    let item = |s: String| match crate::ruby::integer(&s) {
        Some(n) => ArrayItem::Int(n),
        None => ArrayItem::Text(s),
    };
    Ok(match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                Value::Number(n) if n.is_i64() => {
                    Ok(ArrayItem::Int(n.as_i64().unwrap_or_default()))
                }
                v => crate::params::scalar(v)
                    .map(item)
                    .ok_or(Unsupported("nested values in an array param")),
            })
            .collect::<Result<_, _>>()?,
        Some(Value::String(s)) => {
            if s.contains(',') && s.contains('|') {
                return Err(Unsupported("array params split on both commas and pipes"));
            }
            let separator = if s.contains('|') { '|' } else { ',' };
            // String#split drops trailing empty fields.
            let mut parts: Vec<&str> = s.split(separator).collect();
            while parts.last().is_some_and(|p| p.is_empty()) {
                parts.pop();
            }
            parts.into_iter().map(|p| item(p.to_string())).collect()
        }
        Some(Value::Object(_)) => return Err(Unsupported("an array param sent as a hash")),
        Some(v) => crate::params::scalar(v).map(item).into_iter().collect(),
    })
}

/// ActiveModel's integer cast of a string: its leading number, nil when
/// it does not start with one.
pub fn cast_integer(v: &str) -> Option<i64> {
    v.trim_start()
        .trim_start_matches(['+', '-'])
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| crate::ruby::to_i(v))
}

/// ActiveModel's `:datetime` cast for the formats the admin UI sends;
/// None when blank. Other formats are not ported.
pub(crate) fn cast_time(value: Option<&str>) -> Result<Option<NaiveDateTime>, Unsupported> {
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
    silenced_till: Option<NaiveDateTime>,
}

const TARGET_COLUMNS: &str = "id, admin, moderator, suspended_till, silenced_till";

async fn target(conn: &mut PgConnection, user_id: i32) -> Result<Option<Target>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {TARGET_COLUMNS} FROM users WHERE id = $1"))
        .bind(user_id)
        .fetch_optional(conn)
        .await
}

/// `fetch_users`: the user, then `User.where(id: other_user_ids.uniq)`,
/// which has no ORDER BY (heap order, as Rails reads it).
async fn with_others(
    conn: &mut PgConnection,
    user: Target,
    others: &[ArrayItem],
) -> Result<Vec<Target>, sqlx::Error> {
    let mut ids: Vec<i32> = Vec::new();
    for item in others {
        if let Some(id) = item.id()
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    let mut users = vec![user];
    if !ids.is_empty() {
        let rest: Vec<Target> = sqlx::query_as(&format!(
            "SELECT {TARGET_COLUMNS} FROM users WHERE id = ANY($1)"
        ))
        .bind(&ids)
        .fetch_all(conn)
        .await?;
        users.extend(rest);
    }
    Ok(users)
}

/// `can_unsuspend?(user)`
fn can_unsuspend(guardian: &Guardian, user: &Target) -> bool {
    guardian.is_staff() && (!(user.admin || user.moderator) || guardian.is_admin())
}

/// `can_suspend?(user)`: can_unsuspend? on a regular user.
fn can_suspend(guardian: &Guardian, user: &Target) -> bool {
    can_unsuspend(guardian, user) && !(user.admin || user.moderator)
}

/// `can_silence_user?(user)`
fn can_silence(guardian: &Guardian, user: &Target) -> bool {
    guardian.is_staff() && !(user.admin || user.moderator)
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
pub(crate) fn contract_errors(
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

/// The rest of User::Suspend's and User::Silence's contract:
/// other_user_ids' length and post_action's inclusion.
fn penalty_contract_errors(p: &Penalty<'_>, post_actions: &[&str]) -> Vec<String> {
    let mut errors = Vec::new();
    if p.other_user_ids.len() > MAX_SIMILAR_USERS {
        errors.push(format!(
            "Other user ids is too long (maximum is {MAX_SIMILAR_USERS} characters)"
        ));
    }
    if let Some(action) = p
        .post_action
        .as_deref()
        .filter(|a| !crate::ruby::is_blank(a))
        && !post_actions.contains(&action)
    {
        errors.push("Post action is not included in the list".into());
    }
    errors
}

/// The policy's 409 message: who put the penalty on record and how long
/// ago (the latest history row of `action` for the user).
async fn already_message(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
    action: i32,
    key: &str,
) -> Result<String, AppError> {
    let record: Option<(Option<String>, NaiveDateTime)> = sqlx::query_as(
        "SELECT u.username, h.created_at FROM user_histories h \
         LEFT JOIN users u ON u.id = h.acting_user_id \
         WHERE h.target_user_id = $1 AND h.action = $2 ORDER BY h.id DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(action)
    .fetch_optional(&mut *conn)
    .await?;
    // Rails raises on a penalty without its record (nil.acting_user).
    let Some((Some(staff), created_at)) = record else {
        return Err(Unsupported("a penalty without its staff log record").into());
    };
    let time_ago =
        crate::age_words::time_ago_verbose(ctx.i18n, created_at, crate::clock::now_naive());
    Ok(ctx
        .i18n
        .t_with(key, &[("staff", &staff), ("time_ago", &time_ago)])
        .unwrap_or_default())
}

/// The suspension a request puts on each user.
pub(crate) struct Suspension<'a> {
    pub actor_id: i32,
    pub until: NaiveDateTime,
    pub reason: &'a str,
    pub message: Option<&'a str>,
    pub post_id: Option<i64>,
    pub reviewable_id: Option<i64>,
}

/// `UserSuspender#suspend`: the user's history details and suspended_at.
pub(crate) async fn suspend_user(
    pool: &PgPool,
    payloads: &crate::web_hooks::Payloads<'_>,
    user_id: i32,
    s: &Suspension<'_>,
) -> Result<(String, NaiveDateTime), AppError> {
    let mut tx = pool.begin().await?;
    let suspended_at: NaiveDateTime = sqlx::query_scalar(
        "UPDATE users SET suspended_till = $2, suspended_at = clock_timestamp(), updated_at = clock_timestamp() \
         WHERE id = $1 RETURNING suspended_at",
    )
    .bind(user_id)
    .bind(s.until)
    .fetch_one(&mut *tx)
    .await?;
    let details = staff_message(s.reason, s.message);
    let history_id: i32 = sqlx::query_scalar(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, details, post_id, reviewable_id, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, FALSE, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(SUSPEND_USER)
    .bind(s.actor_id)
    .bind(user_id)
    .bind(&details)
    .bind(s.post_id)
    .bind(s.reviewable_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut conn = pool.acquire().await?;
    // log_out!: push subscriptions cleared (the rest is MessageBus).
    sqlx::query("DELETE FROM push_subscriptions WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    // logged_out: the user_logged_out event's web hooks.
    crate::web_hooks::enqueue_user_hooks(&mut conn, payloads, "user_logged_out", user_id).await?;
    let iso6 = |t: NaiveDateTime| t.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    crate::jobs::enqueue_at(
        &mut conn,
        s.until,
        "user_suspension_expired",
        json!({
            "user_id": user_id,
            "suspended_at": iso6(suspended_at),
            "suspended_till": iso6(s.until),
        }),
    )
    .await?;
    if s.message.is_some_and(|m| !m.trim().is_empty()) {
        // Enqueued by class, as UserSuspender does.
        crate::jobs::enqueue(
            &mut conn,
            "Jobs::CriticalUserEmail",
            json!({ "type": "account_suspended", "user_id": user_id, "user_history_id": history_id }),
        )
        .await?;
    }
    // DiscourseEvent :user_suspended: its web hooks.
    crate::web_hooks::enqueue_user_hooks(&mut conn, payloads, "user_suspended", user_id).await?;
    Ok((details, suspended_at))
}

/// `User::Suspend.call`
pub async fn suspend(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    user_id: i32,
    p: &Penalty<'_>,
) -> Result<Outcome, AppError> {
    let until = cast_time(p.until)?;
    let mut errors = contract_errors(ctx, p.reason, "Suspend until", until);
    errors.extend(penalty_contract_errors(
        p,
        &["delete", "delete_replies", "delete_all", "edit", "none"],
    ));
    if !errors.is_empty() {
        return Ok(Outcome::Invalid(errors));
    }
    let (reason, until) = (p.reason.unwrap_or_default(), until.unwrap_or_default());
    let mut conn = pool.acquire().await?;
    let Some(user) = target(&mut conn, user_id).await? else {
        return Ok(Outcome::NotFound);
    };
    // not_suspended_already
    if user
        .suspended_till
        .is_some_and(|t| t > crate::clock::now_naive())
    {
        let message = already_message(
            &mut conn,
            ctx,
            user.id,
            SUSPEND_USER,
            "user.already_suspended",
        )
        .await?;
        return Ok(Outcome::Conflict(message));
    }
    let users = with_others(&mut conn, user, &p.other_user_ids).await?;
    // can_suspend_all_users
    if !users.iter().all(|u| can_suspend(guardian, u)) {
        return Ok(Outcome::Forbidden);
    }
    let actor = guardian
        .user()
        .ok_or(Unsupported("suspending anonymously"))?;
    drop(conn);

    // User::Action::SuspendAll
    let suspension = Suspension {
        actor_id: actor.id,
        until,
        reason,
        message: p.message,
        post_id: p.post_id,
        reviewable_id: p.reviewable_id,
    };
    let mut first = None;
    for u in &users {
        let done = suspend_user(
            pool,
            &crate::web_hooks::Payloads {
                settings: ctx.settings,
                i18n: ctx.i18n,
                config: ctx.config,
            },
            u.id,
            &suspension,
        )
        .await?;
        first.get_or_insert(done);
    }
    let (full_reason, suspended_at) = first.unwrap_or_default();
    trigger_post_action(pool, ctx, guardian, p).await?;
    let mut conn = pool.acquire().await?;
    Ok(Outcome::Done(json!({
        "suspension": {
            "suspend_reason": reason,
            "full_suspend_reason": full_reason,
            "suspended_till": time_json(until),
            "suspended_at": time_json(suspended_at),
            "suspended_by": basic_user(&mut conn, ctx, actor.id).await?,
        }
    })))
}

/// `User::Action::TriggerPostAction`: the post that prompted the
/// penalty deleted (with its replies) or edited, as the moderator chose.
async fn trigger_post_action(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    p: &Penalty<'_>,
) -> Result<(), AppError> {
    // model :post, optional: Post.find_by(id:), deleted posts left out.
    let Some(post_id) = p.post_id.and_then(|id| i32::try_from(id).ok()) else {
        return Ok(());
    };
    let action = match p.post_action.as_deref() {
        Some(a) if !crate::ruby::is_blank(a) && a != "none" => a,
        _ => return Ok(()),
    };
    let mut conn = pool.acquire().await?;
    let post: Option<(i32, i32, bool)> = sqlx::query_as(
        "SELECT p.topic_id, p.post_number, t.id IS NULL OR t.deleted_at IS NOT NULL \
         FROM posts p LEFT JOIN topics t ON t.id = p.topic_id \
         WHERE p.id = $1 AND p.deleted_at IS NULL",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((topic_id, post_number, topic_gone)) = post else {
        return Ok(());
    };
    let reviewable_id = p.reviewable_id;
    match action {
        "delete" | "delete_replies" => {
            if reviewable_id.is_some() {
                let reviewable: bool =
                    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM reviewables WHERE id = $1)")
                        .bind(reviewable_id)
                        .fetch_one(&mut *conn)
                        .await?;
                if reviewable {
                    return Err(Unsupported(
                        "deleting a post for a review with a penalty (notify_deletion)",
                    )
                    .into());
                }
            }
            // reply_ids, read before the post goes.
            let replies = if action == "delete_replies" {
                reply_ids(&mut conn, post_id, topic_id).await?
            } else {
                Vec::new()
            };
            drop(conn);
            if !delete_post(
                pool,
                ctx,
                guardian,
                post_id,
                topic_id,
                post_number,
                topic_gone,
                reviewable_id,
            )
            .await?
            {
                return Ok(());
            }
            if !replies.is_empty()
                && ctx
                    .settings
                    .get("notify_users_after_responses_deleted_on_flagged_post")?
                    .truthy()
                && reviewable_id.is_some()
            {
                return Err(Unsupported("notifying responders of deleted replies").into());
            }
            for reply in replies {
                let req = crate::post_destroyer::Request {
                    context: None,
                    reviewable_id: None,
                    deferred: None,
                };
                crate::post_destroyer::destroy_by_staff(pool, ctx, guardian, reply, &req).await?;
            }
            Ok(())
        }
        "edit" => {
            drop(conn);
            let mut conn = pool.acquire().await?;
            let Some(access) =
                crate::posting::revisions::find_post(&mut conn, ctx, guardian, post_id).await?
            else {
                return Ok(());
            };
            drop(conn);
            if !guardian.can_edit_post(
                ctx.settings,
                &access.topic,
                &access.post,
                access.can_see_topic,
                access.can_create_post,
            )? {
                return Ok(());
            }
            let outcome = crate::posting::revise::revise(
                pool,
                ctx,
                guardian,
                post_id,
                crate::posting::revise::Changes {
                    raw: Some(p.post_edit.clone().unwrap_or_default()),
                    edit_reason: None,
                    force_new_version: false,
                    skip_validations: true,
                    skip_revision: true,
                    ..Default::default()
                },
            )
            .await?;
            match outcome {
                crate::posting::revise::Outcome::Revised => Ok(()),
                _ => Err(Unsupported("a penalty's post edit the revisor refused").into()),
            }
        }
        // `send("delete_all")`: no such method, NoMethodError rescued.
        _ => Ok(()),
    }
}

/// `guardian.can_delete_post_or_topic?(post)`, then
/// `PostDestroyer.new(actor, post, reviewable_id:).destroy`; false when
/// the guardian refused.
#[allow(clippy::too_many_arguments)]
async fn delete_post(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i32,
    topic_id: i32,
    post_number: i32,
    topic_gone: bool,
    reviewable_id: Option<i64>,
) -> Result<bool, AppError> {
    use crate::post_destroyer::{self, Outcome, Request};
    let req = Request {
        context: None,
        reviewable_id,
        deferred: None,
    };
    let outcome = if post_number == 1 {
        // post.topic && can_delete_topic?(post.topic)
        if topic_gone {
            return Ok(false);
        }
        post_destroyer::destroy_topic(pool, ctx, guardian, topic_id, &req).await?
    } else {
        let mut conn = pool.acquire().await?;
        let access =
            crate::posting::revisions::find_post(&mut conn, ctx, guardian, post_id).await?;
        drop(conn);
        let Some(access) = access else {
            return Ok(false);
        };
        post_destroyer::destroy_post(pool, ctx, guardian, &access, &req).await?
    };
    match outcome {
        Outcome::Done => Ok(true),
        Outcome::Forbidden | Outcome::NotFound => Ok(false),
        Outcome::RateLimited(_) => Err(Unsupported("a penalty's post delete rate limited").into()),
    }
}

/// `Post#reply_ids(guardian, only_replies_to_single_post: false)` for
/// staff (who see every post): the replies, and theirs, up to
/// MAX_REPLY_LEVEL, by id.
async fn reply_ids(
    conn: &mut PgConnection,
    post_id: i32,
    topic_id: i32,
) -> Result<Vec<i32>, sqlx::Error> {
    /// `Post::MAX_REPLY_LEVEL`
    const MAX_REPLY_LEVEL: i32 = 1000;
    sqlx::query_scalar(
        "WITH RECURSIVE breadcrumb(id, level) AS ( \
           SELECT $1::int, 0 \
           UNION \
           SELECT reply_post_id, level + 1 FROM post_replies AS r \
             JOIN posts AS p ON p.id = reply_post_id \
             JOIN breadcrumb AS b ON (r.post_id = b.id) \
           WHERE r.post_id <> r.reply_post_id AND b.level < $2 AND p.topic_id = $3 \
         ), breadcrumb_with_count AS ( \
           SELECT id, level, COUNT(*) AS count FROM post_replies AS r \
             JOIN breadcrumb AS b ON (r.reply_post_id = b.id) \
           WHERE r.reply_post_id <> r.post_id GROUP BY id, level \
         ) \
         SELECT b.id FROM breadcrumb_with_count b JOIN posts p ON p.id = b.id \
         WHERE b.level > 0 AND p.deleted_at IS NULL GROUP BY b.id ORDER BY b.id",
    )
    .bind(post_id)
    .bind(MAX_REPLY_LEVEL)
    .bind(topic_id)
    .fetch_all(conn)
    .await
}

/// `Admin::UsersController#unsuspend` -> `UserSuspender.unsuspend`
pub async fn unsuspend(
    conn: &mut PgConnection,
    payloads: &crate::web_hooks::Payloads<'_>,
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
        // DiscourseEvent :user_unsuspended: its web hooks.
        crate::web_hooks::enqueue_user_hooks(&mut *conn, payloads, "user_unsuspended", user.id)
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

/// What UserSilencer#silence did for one user: the history row's id,
/// details and creation time.
struct Silenced {
    history_id: i32,
    details: String,
    at: NaiveDateTime,
}

/// `UserSilencer.new(user, actor, message_body:, keep_posts: true,
/// silenced_till:, reason:, post_id:, reviewable_id:).silence` for a user
/// who is not staff: nothing for a silence still on record.
async fn silence_user(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    user: &Target,
    s: &Suspension<'_>,
) -> Result<Option<Silenced>, AppError> {
    if user.silenced_till.is_some() {
        return Ok(None);
    }
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE users SET silenced_till = $2, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(user.id)
    .bind(s.until)
    .execute(&mut *tx)
    .await?;
    let details = staff_message(s.reason, s.message);
    // log_params[:post_id] = post_id.to_i
    let (history_id, at): (i32, NaiveDateTime) = sqlx::query_as(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, context, details, post_id, \
                                     reviewable_id, admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, FALSE, clock_timestamp(), clock_timestamp()) \
         RETURNING id, created_at",
    )
    .bind(SILENCE_USER)
    .bind(s.actor_id)
    .bind(user.id)
    .bind(format!("silenced_by_staff: {}", s.reason))
    .bind(&details)
    .bind(s.post_id)
    .bind(s.reviewable_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    crate::system_message::create(
        pool,
        ctx,
        user.id,
        "silenced_by_staff",
        &[],
        Some(json!({ "skip_send_email": true })),
    )
    .await?;
    Ok(Some(Silenced {
        history_id,
        details,
        at,
    }))
}

/// `User::Silence.call`: UserSilencer#silence keeping the users' posts,
/// the account_silenced email for each one silenced, then the post action.
pub async fn silence(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    user_id: i32,
    p: &Penalty<'_>,
) -> Result<Outcome, AppError> {
    let until = cast_time(p.until)?;
    let mut errors = contract_errors(ctx, p.reason, "Silenced till", until);
    errors.extend(penalty_contract_errors(
        p,
        &["delete", "delete_replies", "edit", "none"],
    ));
    if !errors.is_empty() {
        return Ok(Outcome::Invalid(errors));
    }
    let (reason, until) = (p.reason.unwrap_or_default(), until.unwrap_or_default());
    let mut conn = pool.acquire().await?;
    let Some(user) = target(&mut conn, user_id).await? else {
        return Ok(Outcome::NotFound);
    };
    // not_silenced_already
    if user
        .silenced_till
        .is_some_and(|t| t > crate::clock::now_naive())
    {
        let message = already_message(
            &mut conn,
            ctx,
            user.id,
            SILENCE_USER,
            "user.already_silenced",
        )
        .await?;
        return Ok(Outcome::Conflict(message));
    }
    let users = with_others(&mut conn, user, &p.other_user_ids).await?;
    // can_silence_all_users
    if !users.iter().all(|u| can_silence(guardian, u)) {
        return Ok(Outcome::Forbidden);
    }
    let actor = guardian
        .user()
        .ok_or(Unsupported("silencing anonymously"))?;
    drop(conn);

    // User::Action::SilenceAll
    let silence = Suspension {
        actor_id: actor.id,
        until,
        reason,
        message: p.message,
        post_id: p.post_id,
        reviewable_id: p.reviewable_id,
    };
    let mut first: Option<Option<Silenced>> = None;
    for u in &users {
        let done = silence_user(pool, ctx, u, &silence).await?;
        if let Some(done) = &done {
            let mut conn = pool.acquire().await?;
            crate::jobs::enqueue(
                &mut conn,
                "critical_user_email",
                json!({ "type": "account_silenced", "user_id": u.id, "user_history_id": done.history_id }),
            )
            .await?;
        }
        if first.is_none() {
            first = Some(done);
        }
    }
    let primary = first.flatten();
    let primary_till = users.first().and_then(|u| u.silenced_till);
    trigger_post_action(pool, ctx, guardian, p).await?;
    let mut conn = pool.acquire().await?;
    let silenced_by = basic_user(&mut conn, ctx, actor.id).await?;
    let body = match primary {
        Some(done) => {
            let full_reason = staff_reason_html(&done.details)?;
            let silence_reason = full_reason
                .split("<br>")
                .next()
                .unwrap_or_default()
                .to_string();
            json!({
                "silenced": true,
                "silence_reason": silence_reason,
                "full_silence_reason": done.details,
                "silenced_till": time_json(until),
                "silenced_at": time_json(done.at),
                "silenced_by": silenced_by,
            })
        }
        // A past silence still on record: UserSilencer did nothing, and
        // the user is not silenced?.
        None => json!({
            "silenced": true,
            "silence_reason": null,
            "full_silence_reason": null,
            "silenced_till": primary_till.map(time_json),
            "silenced_at": null,
            "silenced_by": silenced_by,
        }),
    };
    Ok(Outcome::Done(json!({ "silence": body })))
}

/// `UserSilencer.silence(user, by_user, silenced_till:, reason:)`, the
/// bare silencer (no service, no email): nothing for staff or a user with
/// a silence on record; else the silence, the staff action logged for
/// `by_user_id` and the silenced_by_staff system message. Without
/// `keep_posts`, a trust level 0 user's recent posts would be hidden too
/// (hide_posts), refused here.
#[allow(clippy::too_many_arguments)]
pub async fn silencer_silence(
    pool: &sqlx::PgPool,
    ctx: &Ctx<'_>,
    by_user_id: i32,
    user_id: i32,
    until: NaiveDateTime,
    reason: &str,
    reviewable_id: Option<i64>,
    keep_posts: bool,
) -> Result<bool, AppError> {
    let (staff, trust_level, silenced_till): (bool, i32, Option<NaiveDateTime>) = sqlx::query_as(
        "SELECT admin OR moderator, trust_level, silenced_till FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    if staff {
        return Ok(false);
    }
    if trust_level == 0 && !keep_posts {
        return Err(Unsupported("silencing a trust level 0 user (hide_posts)").into());
    }
    if silenced_till.is_some() {
        return Ok(false);
    }
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE users SET silenced_till = $2, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(user_id)
    .bind(until)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, context, details, reviewable_id, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(SILENCE_USER)
    .bind(by_user_id)
    .bind(user_id)
    .bind(format!("silenced_by_staff: {reason}"))
    .bind(staff_message(reason, None))
    .bind(reviewable_id)
    .execute(&mut *tx)
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
    Ok(true)
}

/// `UserSilencer#unsilence` for a silenced user: the silence lifted, the
/// unsilenced system message, and the staff action logged for whoever
/// lifted it (the system user when it ran out, Jobs::UnsilenceUsers).
pub async fn lift_silence(
    pool: &sqlx::PgPool,
    ctx: &Ctx<'_>,
    by_user_id: i32,
    user_id: i32,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE users SET silenced_till = NULL, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    crate::system_message::create(pool, ctx, user_id, "unsilenced", &[], None).await?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(UNSILENCE_USER)
    .bind(by_user_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// `UserSilencer.unsilence(user, by_user, reviewable_id:)` from a review
/// perform: the silence lifted and the unsilenced system message as
/// lift_silence does (their own transactions, committed before the
/// perform's, which holds nothing they write), the staff action logged
/// with the reviewable inside the perform's transaction.
pub async fn lift_silence_for_reviewable(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    by_user_id: i32,
    user_id: i32,
    reviewable_id: i64,
) -> Result<(), AppError> {
    let pool = &ctx.host.pool;
    sqlx::query(
        "UPDATE users SET silenced_till = NULL, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    crate::system_message::create(pool, ctx, user_id, "unsilenced", &[], None).await?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, reviewable_id, admin_only, \
                                     created_at, updated_at) \
         VALUES ($1, $2, $3, $4, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(UNSILENCE_USER)
    .bind(by_user_id)
    .bind(user_id)
    .bind(reviewable_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
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
    drop(conn);
    if silenced_till.is_some() {
        lift_silence(pool, ctx, actor.id, user_id).await?;
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
