//! ReviewableUser for approving a user from the admin
//! (Admin::UsersController#approve): the reviewable Jobs::CreateUserReviewable
//! makes when none exists, and `perform(:approve_user, allow_reviewed:)`.
//!
//! Refused: users with an uploaded avatar (the payload's URL and the
//! avatar snapshot), invite-only sites without must_approve_users, a
//! reviewable claimed or for a group.

use serde_json::json;
use sqlx::PgConnection;

use crate::reviewables::{AGREED, PENDING};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

const SYSTEM_USER_ID: i32 = -1;
/// `Reviewable.statuses[:approved]`
const APPROVED: i32 = 1;
/// `ReviewableHistory.types`
const HISTORY_CREATED: i32 = 0;
const HISTORY_TRANSITIONED: i32 = 1;
/// `ReviewableScore.statuses[:disagreed]`
const SCORE_DISAGREED: i32 = 2;
/// `ReviewableScore.types[:needs_approval]`
const NEEDS_APPROVAL: i32 = 9;
/// UserHistory.actions[:approve_user]
const APPROVE_USER: i32 = 69;

/// `ReviewableUser.find_by(target: user)`, else the one
/// Jobs::CreateUserReviewable creates. None when the job makes none
/// (approved user, or neither must_approve_users nor invite_only).
pub async fn find_or_create(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user_id: i32,
) -> Result<Option<i64>, AppError> {
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM reviewables WHERE type = 'ReviewableUser' AND target_type = 'User' \
         AND target_id = $1 ORDER BY id LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    if existing.is_some() {
        return Ok(existing);
    }
    let reason = if s.get("must_approve_users")?.truthy() {
        "must_approve_users"
    } else if s.get("invite_only")?.truthy() {
        return Err(Unsupported("user reviewables on invite-only sites").into());
    } else {
        return Ok(None);
    };
    #[derive(sqlx::FromRow)]
    struct Target {
        username: String,
        name: Option<String>,
        approved: bool,
        uploaded_avatar_id: Option<i32>,
        email: Option<String>,
        bio_raw: Option<String>,
        website: Option<String>,
    }
    let Some(t): Option<Target> = sqlx::query_as(
        "SELECT u.username, u.name, u.approved, u.uploaded_avatar_id, \
                (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\") AS email, \
                p.bio_raw, p.website \
         FROM users u LEFT JOIN user_profiles p ON p.user_id = u.id WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };
    if t.approved {
        return Ok(None);
    }
    if t.uploaded_avatar_id.is_some() {
        return Err(Unsupported("user reviewables for users with an uploaded avatar").into());
    }
    // ReviewableUser.payload_for(user)
    let payload = json!({
        "username": t.username,
        "name": t.name,
        "email": t.email,
        "bio": t.bio_raw,
        "website": t.website,
        "avatar_upload_id": null,
        "avatar_url": null,
    });
    // needs_review!(target:, created_by: system, reviewable_by_moderator:, payload:)
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO reviewables (type, type_source, status, created_by_id, reviewable_by_moderator, \
                                  reviewable_by_group_id, category_id, topic_id, score, potential_spam, \
                                  target_id, target_type, target_created_by_id, payload, version, \
                                  latest_score, force_review, potentially_illegal, created_at, updated_at) \
         VALUES ('ReviewableUser', 'core', $1, $2, TRUE, NULL, NULL, NULL, 0, TRUE, $3, 'User', NULL, $4, 0, \
                 NULL, FALSE, FALSE, clock_timestamp(), clock_timestamp()) \
         RETURNING id",
    )
    .bind(PENDING)
    .bind(SYSTEM_USER_ID)
    .bind(user_id)
    .bind(&payload)
    .fetch_one(&mut *conn)
    .await?;
    log_history(conn, id, HISTORY_CREATED, PENDING, SYSTEM_USER_ID).await?;
    crate::jobs::enqueue(
        &mut *conn,
        "notify_reviewable",
        json!({ "reviewable_id": id }),
    )
    .await?;

    // add_score(system, :needs_approval, reason:, force_review: true): the
    // system user scores as staff (1 + 5), no accuracy or type bonus.
    let score = 6.0;
    let created_at: chrono::NaiveDateTime = sqlx::query_scalar(
        "INSERT INTO reviewable_scores (reviewable_id, user_id, reviewable_score_type, status, score, \
                                        take_action_bonus, user_accuracy_bonus, meta_topic_id, reason, \
                                        context, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, 0, 0, NULL, $6, NULL, clock_timestamp(), clock_timestamp()) \
         RETURNING created_at",
    )
    .bind(id)
    .bind(SYSTEM_USER_ID)
    .bind(NEEDS_APPROVAL)
    .bind(PENDING)
    .bind(score)
    .bind(reason)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE reviewables SET score = score + $2, latest_score = $3, force_review = TRUE, \
                                updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(id)
    .bind(score)
    .bind(created_at)
    .execute(&mut *conn)
    .await?;
    crate::jobs::enqueue(
        &mut *conn,
        "notify_reviewable",
        json!({ "reviewable_id": id }),
    )
    .await?;
    Ok(Some(id))
}

async fn log_history(
    conn: &mut PgConnection,
    reviewable_id: i64,
    kind: i32,
    status: i32,
    created_by_id: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO reviewable_histories (reviewable_id, reviewable_history_type, status, created_by_id, \
                                           edited, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, NULL, clock_timestamp(), clock_timestamp())",
    )
    .bind(reviewable_id)
    .bind(kind)
    .bind(status)
    .bind(created_by_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The performer of a reviewable action.
pub struct Performer<'a> {
    pub id: i32,
    pub username: &'a str,
}

/// `reviewable.perform(performer, :approve_user, allow_reviewed: true)`
/// for a user the guardian may approve (active, not approved): Ok(false)
/// when the reviewable offers no approve action (already approved).
pub async fn approve(
    conn: &mut PgConnection,
    s: &SiteSettings,
    reviewable_id: i64,
    user_id: i32,
    performer: &Performer<'_>,
) -> Result<bool, AppError> {
    let (status, topic_id): (i32, Option<i32>) =
        sqlx::query_as("SELECT status, topic_id FROM reviewables WHERE id = $1")
            .bind(reviewable_id)
            .fetch_one(&mut *conn)
            .await?;
    // build_actions: none once approved.
    if status == APPROVED {
        return Ok(false);
    }
    if topic_id.is_some() {
        return Err(Unsupported("user reviewables with a topic (claims)").into());
    }
    // increment_version!
    sqlx::query("UPDATE reviewables SET version = version + 1 WHERE id = $1")
        .bind(reviewable_id)
        .execute(&mut *conn)
        .await?;

    // perform_approve_user: set_approved_fields!, target.save!
    #[derive(sqlx::FromRow)]
    struct Saved {
        username_lower: String,
        name: Option<String>,
        admin: bool,
        active: bool,
    }
    let u: Saved = sqlx::query_as(
        "UPDATE users SET approved = TRUE, approved_by_id = COALESCE(approved_by_id, $2), \
                          approved_at = COALESCE(approved_at, clock_timestamp()), \
                          updated_at = clock_timestamp() \
         WHERE id = $1 RETURNING username_lower, name, admin, active",
    )
    .bind(user_id)
    .bind(performer.id)
    .fetch_one(&mut *conn)
    .await?;
    crate::user_updater::after_save(
        conn,
        s,
        user_id,
        u.admin && u.active,
        &u.username_lower,
        u.name.as_deref(),
    )
    .await?;
    if s.get("must_approve_users")?.truthy() {
        crate::jobs::enqueue(
            &mut *conn,
            "critical_user_email",
            json!({ "type": "signup_after_approval", "user_id": user_id }),
        )
        .await?;
    }
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, reviewable_id, admin_only, \
                                     created_at, updated_at) \
         VALUES ($1, $2, $3, $4, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(APPROVE_USER)
    .bind(performer.id)
    .bind(user_id)
    .bind(reviewable_id)
    .execute(&mut *conn)
    .await?;

    // transition_to(:approved): the status, its history, the scores agreed.
    sqlx::query("UPDATE reviewables SET status = $2, updated_at = clock_timestamp() WHERE id = $1")
        .bind(reviewable_id)
        .bind(APPROVED)
        .execute(&mut *conn)
        .await?;
    log_history(
        conn,
        reviewable_id,
        HISTORY_TRANSITIONED,
        APPROVED,
        performer.id,
    )
    .await?;
    sqlx::query(
        "UPDATE reviewable_scores SET status = $2, reviewed_by_id = $3, reviewed_at = clock_timestamp() \
         WHERE reviewable_id = $1 AND status IN ($4, $5)",
    )
    .bind(reviewable_id)
    .bind(AGREED)
    .bind(performer.id)
    .bind(PENDING)
    .bind(SCORE_DISAGREED)
    .execute(&mut *conn)
    .await?;
    crate::jobs::enqueue(
        &mut *conn,
        "notify_reviewable",
        json!({
            "reviewable_id": reviewable_id,
            "performing_username": performer.username,
            "updated_reviewable_ids": [reviewable_id],
        }),
    )
    .await?;
    Ok(true)
}
