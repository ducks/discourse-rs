//! UserDestroyer (app/services/user_destroyer.rb) and what `user.destroy`
//! takes with it: the associations User declares `dependent:` (row
//! callbacks that move counters: group user counts, badge grant counts,
//! invite redemptions), its `before_destroy` block, then the user row.
//! Used by the admin's delete (DELETE /admin/users/:id), deleting one's
//! own account (DELETE /u/:username), the review queue and the cleanup
//! jobs.
//!
//! With `delete_posts` the user's posts go through PostDestroyer on the
//! same transaction (its after-commit jobs held for the commit), with
//! `delete_as_spammer` their pending flags agreed first, their reviewed
//! posts rejected and deleted and their queued posts rejected.
//!
//! Refused until ported: `block_urls` on a user with external
//! links (Oneboxer's engine check), a category definition post of the
//! user's, posts PostDestroyer refuses (a pending flag, embeds), associated
//! groups that put the user in groups, MCP authorizations,
//! and the unported plugins' rows.

use chrono::NaiveDateTime;
use serde_json::json;
use sqlx::PgConnection;

use crate::guardian::{DeletableUser, Guardian};
use crate::posting::Ctx;
use crate::{AppError, Unsupported};

/// `Discourse::SYSTEM_USER_ID`
const SYSTEM_USER_ID: i32 = -1;
/// `UserHistory.actions[:delete_user]`
const DELETE_USER: i32 = 1;
/// `Reviewable.statuses`
const PENDING: i32 = 0;
const IGNORED: i32 = 3;
/// `ReviewableHistory.types`
const HISTORY_TRANSITIONED: i32 = 1;
const HISTORY_UNCLAIMED: i32 = 4;
/// `ReviewableScore.statuses`
const SCORE_PENDING: i32 = 0;
const SCORE_DISAGREED: i32 = 2;
const SCORE_IGNORED: i32 = 3;
/// `Group::AUTO_GROUPS[:staff]`
const STAFF_GROUP: i64 = 3;
/// `PostActionType.types[:like]`
const LIKE: i32 = 2;
/// `Post.types[:whisper]`
const WHISPER: i32 = 4;

/// `UserDestroyer#destroy`'s options.
#[derive(Debug, Default, Clone)]
pub struct Options {
    pub delete_posts: bool,
    /// The raw parameter's truthiness (the string "false" counts).
    pub delete_as_spammer: bool,
    pub block_email: bool,
    pub block_urls: bool,
    pub block_ip: bool,
    pub prepare_for_destroy: bool,
    pub quiet: bool,
    pub context: Option<String>,
    pub reviewable_id: Option<i64>,
}

/// How a destroy ends.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Destroyed,
    /// `PostsExistError`: live posts in live topics, without delete_posts.
    PostsExist,
    /// `ensure_can_delete_user!` refused (Discourse::InvalidAccess).
    Forbidden,
}

/// The user as the destroyer reads it before it goes.
#[derive(sqlx::FromRow)]
struct Target {
    id: i32,
    username: String,
    name: Option<String>,
    created_at: NaiveDateTime,
    trust_level: i32,
    last_seen_at: Option<NaiveDateTime>,
    last_emailed_at: Option<NaiveDateTime>,
    /// `ip_address.to_s`: the address without its mask.
    ip_address: Option<String>,
    registration_ip_address: Option<String>,
}

/// Ruby's `Time#to_s` for a UTC time.
fn ruby_time(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

/// `user.posts.joins(:topic).count`: the live posts (messages too) in
/// live topics, which `PostsExistError` and the admin's refusal count.
pub async fn live_post_count(conn: &mut PgConnection, user_id: i32) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM posts p JOIN topics t ON t.id = p.topic_id \
         WHERE p.user_id = $1 AND p.deleted_at IS NULL AND t.deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(conn)
    .await
}

/// `UserDestroyer.new(actor).destroy(user, opts)` on the caller's
/// connection (its transaction: Rails' own, or the caller's with
/// `transaction: false`), the ReviewableUser of the user closed after.
pub async fn destroy(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    actor: &Guardian,
    user_id: i32,
    opts: &Options,
) -> Result<Outcome, AppError> {
    let actor_user = actor
        .user()
        .ok_or(Unsupported("destroying a user without an acting user"))?
        .clone();
    let Some(deletable) = DeletableUser::load(&mut *conn, user_id).await? else {
        return Ok(Outcome::Forbidden);
    };
    if !opts.delete_posts && live_post_count(&mut *conn, user_id).await? != 0 {
        return Ok(Outcome::PostsExist);
    }
    if !actor
        .can_delete_user(&mut *conn, ctx.settings, &deletable)
        .await?
    {
        return Ok(Outcome::Forbidden);
    }
    let user: Target = sqlx::query_as(
        "SELECT id, username, name, created_at, trust_level, last_seen_at, last_emailed_at, \
                host(ip_address) AS ip_address, host(registration_ip_address) AS registration_ip_address \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    refuse_unported(conn, user_id, opts).await?;

    if opts.prepare_for_destroy {
        prepare_for_destroy(conn, user_id).await?;
    }

    // Jobs Rails enqueues from after_commit callbacks, in their order.
    let mut after_commit: Vec<(&str, serde_json::Value)> = Vec::new();

    for sql in [
        "DELETE FROM user_security_keys WHERE user_id = $1",
        "DELETE FROM bookmarks WHERE user_id = $1",
        "DELETE FROM drafts WHERE user_id = $1",
        "DELETE FROM reviewable_notes WHERE reviewable_id IN \
           (SELECT id FROM reviewables WHERE created_by_id = $1)",
        "DELETE FROM reviewables WHERE created_by_id = $1",
        "DELETE FROM reviewable_claimed_topics WHERE user_id = $1",
    ] {
        sqlx::query(sql).bind(user_id).execute(&mut *conn).await?;
    }

    if opts.delete_posts {
        crate::plugins::user_content_deleting(conn, user_id).await?;
        if opts.delete_as_spammer {
            agree_with_flags(conn, ctx, &actor_user, user_id).await?;
            let deferred = crate::post_destroyer::Deferred::default();
            crate::review_posts::reject_for_spammer(conn, ctx, actor, user_id, &deferred).await?;
            after_commit.extend(
                deferred
                    .into_inner()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
        }
        // block_external_urls: refused in refuse_unported when it would act.
        delete_posts(conn, ctx, actor, user_id, opts, &mut after_commit).await?;
    }

    resolve_reviewables_targeting(conn, ctx, user_id).await?;
    remove_post_actions(conn, ctx, user_id).await?;

    // Staff actions the user took, marked with who they were.
    sqlx::query(
        "UPDATE user_histories SET details = CONCAT(details, $2::text) \
         WHERE acting_user_id = (SELECT id FROM users WHERE username_lower = lower($3) LIMIT 1) \
           AND action = ANY($4)",
    )
    .bind(user_id)
    .bind(format!(
        "\nuser_id: {}\nusername: {}",
        user.id, user.username
    ))
    .bind(&user.username)
    .bind(crate::staff_action_logs::staff_action_ids())
    .execute(&mut *conn)
    .await?;

    // emails = user_emails.email | associated accounts' info->>'email'
    let mut emails: Vec<String> =
        sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 ORDER BY id")
            .bind(user_id)
            .fetch_all(&mut *conn)
            .await?;
    let associated: Vec<String> = sqlx::query_scalar(
        "SELECT info->>'email' FROM user_associated_accounts \
         WHERE user_id = $1 AND info->>'email' IS NOT NULL ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    for email in associated {
        if !emails.contains(&email) {
            emails.push(email);
        }
    }

    destroy_user_row(conn, ctx, user_id, &mut after_commit).await?;

    if opts.block_email {
        for email in &emails {
            block_email(conn, email, user.ip_address.as_deref()).await?;
        }
    }
    if opts.block_ip
        && let Some(ip) = &user.ip_address
    {
        watch_ip(conn, ip).await?;
        if let Some(registration) = &user.registration_ip_address
            && registration != ip
        {
            watch_ip(conn, registration).await?;
        }
    }

    sqlx::query("UPDATE posts SET user_id = NULL WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;

    adopt_categories(conn, ctx, user_id, &mut after_commit).await?;

    // Invites to the user's addresses: their groups and topics, then the
    // invite itself (invited_users went with the user).
    let invites: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM invites WHERE email = ANY($1) AND deleted_at IS NULL ORDER BY id",
    )
    .bind(&emails)
    .fetch_all(&mut *conn)
    .await?;
    for invite in invites {
        for sql in [
            "DELETE FROM invited_groups WHERE invite_id = $1",
            "DELETE FROM topic_invites WHERE invite_id = $1",
            "DELETE FROM invites WHERE id = $1",
        ] {
            sqlx::query(sql).bind(invite).execute(&mut *conn).await?;
        }
    }

    if !opts.quiet {
        let (acting, context) = if actor_user.id == user_id {
            let url = opts.context.clone().unwrap_or_default();
            let message = ctx
                .i18n
                .t_with("staff_action_logs.user_delete_self", &[("url", &url)])
                .ok_or(Unsupported("staff_action_logs.user_delete_self"))?;
            (SYSTEM_USER_ID, Some(message))
        } else {
            (actor_user.id, opts.context.clone())
        };
        // StaffActionLogger#log_user_deletion
        let opt_time = |t: Option<NaiveDateTime>| t.map(ruby_time).unwrap_or_default();
        let details = format!(
            "id: {}\nusername: {}\nname: {}\ncreated_at: {}\ntrust_level: {}\nlast_seen_at: {}\nlast_emailed_at: {}",
            user.id,
            user.username,
            user.name.as_deref().unwrap_or(""),
            ruby_time(user.created_at),
            user.trust_level,
            opt_time(user.last_seen_at),
            opt_time(user.last_emailed_at),
        );
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, context, reviewable_id, ip_address, \
                                         details, admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, FALSE, clock_timestamp(), clock_timestamp())",
        )
        .bind(DELETE_USER)
        .bind(acting)
        .bind(context)
        .bind(opts.reviewable_id)
        .bind(user.ip_address.clone().unwrap_or_default())
        .bind(details)
        .execute(&mut *conn)
        .await?;
    }
    ctx.bus
        .publish(
            &mut *conn,
            &format!("/logout/{user_id}"),
            &json!(user_id),
            Some(&[crate::bus::user_tag(user_id)]),
        )
        .await?;

    // A record saved more than once in the transaction (a category) runs
    // its commit callbacks once.
    let mut seen: Vec<(&str, serde_json::Value)> = Vec::new();
    for (job, args) in after_commit {
        if seen.iter().any(|(n, a)| *n == job && *a == args) {
            continue;
        }
        crate::jobs::enqueue(&mut *conn, job, args.clone()).await?;
        seen.push((job, args));
    }

    // The account's own reviewable, unless it is the one deleting it.
    let pending: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM reviewables WHERE type = 'ReviewableUser' AND status = $1 \
           AND target_type = 'User' AND target_id = $2 ORDER BY id LIMIT 1",
    )
    .bind(PENDING)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(reviewable_id) = pending
        && Some(reviewable_id) != opts.reviewable_id
    {
        let performer = crate::reviewable_user::Performer {
            id: actor_user.id,
            username: &actor_user.username,
        };
        crate::reviewable_user::reject_for_deleted_user(
            conn,
            &crate::web_hooks::Payloads {
                settings: ctx.settings,
                i18n: ctx.i18n,
                config: ctx.config,
            },
            reviewable_id,
            &performer,
        )
        .await?;
    }
    Ok(Outcome::Destroyed)
}

/// `agree_with_flags` (delete_as_spammer): every pending flag review of
/// the user's posts agreed and kept by the actor. Their other pending
/// post reviews (rejected and deleted, queued posts rejected) are refused
/// in refuse_unported.
async fn agree_with_flags(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    actor: &crate::session::current::SessionUser,
    user_id: i32,
) -> Result<(), AppError> {
    let posts: Vec<Option<i32>> = sqlx::query_scalar(
        "SELECT target_id FROM reviewables WHERE type = 'ReviewableFlaggedPost' \
           AND target_created_by_id = $1 AND status = $2 ORDER BY id",
    )
    .bind(user_id)
    .bind(PENDING)
    .fetch_all(&mut *conn)
    .await?;
    for post_id in posts {
        let post_id = post_id.ok_or(Unsupported("a flag review without its post"))?;
        crate::review::agree_with_flags(&mut *conn, ctx, actor.id, &actor.username, post_id)
            .await?;
    }
    Ok(())
}

/// `delete_posts`: each live post of the user, in id order, destroyed by
/// the actor when staff, else by system ("User was destroyed"); a topic
/// whose first post went loses its owner.
async fn delete_posts(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    actor: &Guardian,
    user_id: i32,
    opts: &Options,
    after_commit: &mut Vec<(&'static str, serde_json::Value)>,
) -> Result<(), AppError> {
    let posts: Vec<(i32, i32, i32, bool)> = sqlx::query_as(
        "SELECT p.id, p.topic_id, p.post_number, \
                EXISTS (SELECT 1 FROM topics t WHERE t.id = p.topic_id AND t.deleted_at IS NULL) \
         FROM posts p WHERE p.user_id = $1 AND p.deleted_at IS NULL ORDER BY p.id",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    if posts.is_empty() {
        return Ok(());
    }
    let category_topic: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM posts p JOIN categories c ON c.topic_id = p.topic_id \
                        WHERE p.user_id = $1 AND p.deleted_at IS NULL AND p.post_number = 1)",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if category_topic {
        return Err(
            Unsupported("giving a deleted user's category definition post to system").into(),
        );
    }
    let system_guardian;
    let destroyer = if actor.is_staff() {
        actor
    } else {
        let system = crate::session::current::SessionUser::load(&mut *conn, SYSTEM_USER_ID)
            .await?
            .ok_or(Unsupported("a site without its system user"))?;
        system_guardian = Guardian::for_user(&mut *conn, &system).await?;
        &system_guardian
    };
    let context = ctx.t("staff_action_logs.user_associated_posts_deleted");
    let deferred = crate::post_destroyer::Deferred::default();
    let req = crate::post_destroyer::Request {
        context: Some(&context),
        reviewable_id: opts.reviewable_id,
        deferred: Some(&deferred),
    };
    for (post_id, topic_id, post_number, live_topic) in posts {
        crate::post_destroyer::destroy_by_staff_in(&mut *conn, ctx, destroyer, post_id, &req)
            .await?;
        // `post.topic` (read live, before the destroy) of a first post.
        if live_topic && post_number == 1 {
            sqlx::query("UPDATE topics SET user_id = NULL WHERE id = $1")
                .bind(topic_id)
                .execute(&mut *conn)
                .await?;
        }
    }
    after_commit.extend(
        deferred
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    Ok(())
}

/// What the destroy would do here that is not ported.
async fn refuse_unported(
    conn: &mut PgConnection,
    user_id: i32,
    opts: &Options,
) -> Result<(), AppError> {
    if opts.delete_posts {
        let links: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topic_links WHERE user_id = $1 AND NOT internal)",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if opts.block_urls && links {
            return Err(Unsupported("blocking a deleted user's links (block_urls)").into());
        }
    }
    let (associated_groups, mcp): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM user_associated_groups uag \
                        JOIN group_associated_groups gag ON gag.associated_group_id = uag.associated_group_id \
                        WHERE uag.user_id = $1), \
                EXISTS (SELECT 1 FROM mcp_oauth_authorizations WHERE user_id = $1) \
                  OR EXISTS (SELECT 1 FROM mcp_oauth_access_tokens WHERE user_id = $1)",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if associated_groups {
        return Err(Unsupported("deleting a user that associated groups put in groups").into());
    }
    if mcp {
        return Err(Unsupported("deleting a user with MCP authorizations").into());
    }
    Ok(())
}

/// `prepare_for_destroy` (admin and bulk deletes): the user's live
/// actions, user actions on any side, timings, views, topic users,
/// message access and notifications, deleted without recounting.
async fn prepare_for_destroy(conn: &mut PgConnection, user_id: i32) -> Result<(), sqlx::Error> {
    for sql in [
        "DELETE FROM post_actions WHERE user_id = $1 AND deleted_at IS NULL",
        "DELETE FROM user_actions WHERE user_id = $1 OR target_user_id = $1 OR acting_user_id = $1",
        "DELETE FROM post_timings WHERE user_id = $1",
        "DELETE FROM topic_views WHERE user_id = $1",
        "DELETE FROM topic_users WHERE user_id = $1",
        "DELETE FROM topic_allowed_users WHERE user_id = $1",
        "DELETE FROM notifications WHERE user_id = $1",
    ] {
        sqlx::query(sql).bind(user_id).execute(&mut *conn).await?;
    }
    Ok(())
}

/// `resolve_reviewables_targeting`: each pending reviewable of the user's
/// content gets a note by system and is ignored by system.
async fn resolve_reviewables_targeting(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
) -> Result<(), AppError> {
    let reviewables: Vec<(i64, Option<i32>)> = sqlx::query_as(
        "SELECT id, topic_id FROM reviewables WHERE status = $1 AND target_created_by_id = $2 ORDER BY id",
    )
    .bind(PENDING)
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let note = ctx.t("reviewables.target_user_deleted");
    for (id, topic_id) in reviewables {
        sqlx::query(
            "INSERT INTO reviewable_notes (reviewable_id, user_id, content, created_at, updated_at) \
             VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
        )
        .bind(id)
        .bind(SYSTEM_USER_ID)
        .bind(&note)
        .execute(&mut *conn)
        .await?;
        // transition_to(:ignored, system)
        if let Some(topic_id) = topic_id {
            release_manual_claim(conn, ctx, topic_id).await?;
        }
        sqlx::query(
            "UPDATE reviewables SET status = $2, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(id)
        .bind(IGNORED)
        .execute(&mut *conn)
        .await?;
        log_history(conn, id, HISTORY_TRANSITIONED, IGNORED, SYSTEM_USER_ID).await?;
        // DiscourseEvent :reviewable_transitioned_to: its web hooks, before
        // the scores move.
        crate::web_hooks::enqueue_reviewable_hooks(
            &mut *conn,
            &crate::web_hooks::Payloads {
                settings: ctx.settings,
                i18n: ctx.i18n,
                config: ctx.config,
            },
            "reviewable_transitioned_to",
            id,
        )
        .await?;
        sqlx::query(
            "UPDATE reviewable_scores SET status = $2, reviewed_by_id = $3, reviewed_at = clock_timestamp() \
             WHERE reviewable_id = $1 AND status IN ($4, $5)",
        )
        .bind(id)
        .bind(SCORE_IGNORED)
        .bind(SYSTEM_USER_ID)
        .bind(SCORE_PENDING)
        .bind(SCORE_DISAGREED)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
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
    .execute(conn)
    .await?;
    Ok(())
}

/// `ReviewableClaimedTopic.release_manual_claim(topic_id, system)`: the
/// topic's manual claim deleted, logged as unclaimed on its pending
/// reviewables when claiming is on, and the staff's counts refreshed.
async fn release_manual_claim(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
) -> Result<(), AppError> {
    let claim: Option<i64> = sqlx::query_scalar(
        "DELETE FROM reviewable_claimed_topics WHERE id = \
           (SELECT id FROM reviewable_claimed_topics WHERE topic_id = $1 AND NOT automatic LIMIT 1) \
         RETURNING id",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    if claim.is_none() {
        return Ok(());
    }
    if ctx.settings.get("reviewable_claiming")?.to_s() != "disabled" {
        let pending: Vec<(i64, i32)> = sqlx::query_as(
            "SELECT id, status FROM reviewables WHERE status = $1 AND topic_id = $2 ORDER BY id",
        )
        .bind(PENDING)
        .bind(topic_id)
        .fetch_all(&mut *conn)
        .await?;
        for (id, status) in pending {
            log_history(conn, id, HISTORY_UNCLAIMED, status, SYSTEM_USER_ID).await?;
        }
    }
    if ctx
        .settings
        .get("enable_category_group_moderation")?
        .truthy()
    {
        return Err(Unsupported("releasing claims with category group moderation").into());
    }
    let group_ids = [STAFF_GROUP];
    let system = crate::session::current::SessionUser::load(&mut *conn, SYSTEM_USER_ID)
        .await?
        .ok_or(Unsupported("a site without its system user"))?;
    // BasicUserSerializer of the performer.
    let urls = crate::url::Urls {
        config: ctx.config,
        settings: ctx.settings,
    };
    let avatar_template = crate::avatar::avatar_template(
        &urls,
        system.id,
        &system.username,
        system.uploaded_avatar_id,
        None,
    )?;
    ctx.bus
        .publish(
            &mut *conn,
            "/reviewable_claimed",
            &json!({
                "topic_id": topic_id,
                "user": {
                    "id": system.id,
                    "username": system.username,
                    "name": system.name,
                    "avatar_template": avatar_template,
                },
                "automatic": false,
                "claimed": false,
            }),
            Some(&[crate::bus::group_tag(STAFF_GROUP as i32)]),
        )
        .await?;
    crate::jobs::enqueue(
        &mut *conn,
        "refresh_users_reviewable_counts",
        json!({ "group_ids": group_ids }),
    )
    .await?;
    Ok(())
}

/// Each of the user's live post actions `remove_act!`ed by system:
/// trashed, then the post's count of that type recounted (a like also
/// its score, the liker's topic user and the topic's like count). The
/// rows themselves go with the user.
async fn remove_post_actions(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
) -> Result<(), AppError> {
    let actions: Vec<(i32, i32, i32)> = sqlx::query_as(
        "SELECT id, post_id, post_action_type_id FROM post_actions \
         WHERE user_id = $1 AND deleted_at IS NULL ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    if actions.is_empty() {
        return Ok(());
    }
    let types = crate::post_actions::ActionTypes::load(&mut *conn).await?;
    let staff_weight = ctx.settings.get("staff_like_weight")?.to_i();
    for (id, post_id, type_id) in actions {
        sqlx::query(
            "UPDATE post_actions SET deleted_at = clock_timestamp(), deleted_by_id = $2, \
                                     updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(id)
        .bind(SYSTEM_USER_ID)
        .execute(&mut *conn)
        .await?;
        // update_counters: Post.where (live posts) and Topic.find_by (a
        // live topic).
        if type_id == LIKE {
            sqlx::query(
                "UPDATE posts SET \
                   like_count = (SELECT COUNT(*) FROM post_actions WHERE post_id = $1 \
                                 AND post_action_type_id = $2 AND deleted_at IS NULL), \
                   like_score = (SELECT COALESCE(SUM(CASE WHEN u.moderator OR u.admin THEN $3 ELSE 1 END), 0) \
                                 FROM post_actions pa JOIN users u ON u.id = pa.user_id \
                                 WHERE pa.post_id = $1 AND pa.deleted_at IS NULL) \
                 WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(post_id)
            .bind(LIKE)
            .bind(staff_weight)
            .execute(&mut *conn)
            .await?;
        } else {
            let name = types
                .types
                .iter()
                .find(|(_, t)| *t == i64::from(type_id))
                .map(|(name, _)| name.clone());
            let column = name.map(|n| format!("{n}_count"));
            let exists = match &column {
                Some(column) => sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
                     WHERE table_schema = current_schema() AND table_name = 'posts' AND column_name = $1)",
                )
                .bind(column)
                .fetch_one(&mut *conn)
                .await?,
                None => false,
            };
            if let (Some(column), true) = (column, exists) {
                sqlx::query(&format!(
                    "UPDATE posts SET {column} = (SELECT COUNT(*) FROM post_actions WHERE post_id = $1 \
                                                  AND post_action_type_id = $2 AND deleted_at IS NULL) \
                     WHERE id = $1 AND deleted_at IS NULL"
                ))
                .bind(post_id)
                .bind(type_id)
                .execute(&mut *conn)
                .await?;
            }
        }
        if type_id != LIKE {
            continue;
        }
        let topic_id: Option<i32> = sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
            .bind(post_id)
            .fetch_optional(&mut *conn)
            .await?;
        let Some(topic_id) = topic_id else {
            continue;
        };
        // TopicUser.update_post_action_cache(user_id:, topic_id:)
        sqlx::query(
            "UPDATE topic_users tu SET liked = x.state FROM ( \
               SELECT EXISTS ( \
                 SELECT 1 FROM post_actions pa JOIN posts p ON p.id = pa.post_id JOIN topics t ON t.id = p.topic_id \
                 WHERE pa.deleted_at IS NULL AND p.deleted_at IS NULL AND t.deleted_at IS NULL \
                   AND pa.post_action_type_id = $3 AND tu2.topic_id = t.id AND tu2.user_id = pa.user_id) AS state, \
                 tu2.topic_id, tu2.user_id \
               FROM topic_users tu2 WHERE tu2.user_id = $1 AND tu2.topic_id = $2) x \
             WHERE x.topic_id = tu.topic_id AND x.user_id = tu.user_id AND x.state != tu.liked",
        )
        .bind(user_id)
        .bind(topic_id)
        .bind(LIKE)
        .execute(&mut *conn)
        .await?;
        // Topic#update_action_counts
        sqlx::query(
            "UPDATE topics SET like_count = (SELECT COALESCE(SUM(like_count), 0) FROM posts \
                                             WHERE topic_id = $1 AND post_type <> $2 AND deleted_at IS NULL) \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(topic_id)
        .bind(WHISPER)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `user.destroy`: the associations in declaration order with their row
/// callbacks, the before_destroy block, the row, `after_destroy
/// :clear_acls`.
async fn destroy_user_row(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
    after_commit: &mut Vec<(&'static str, serde_json::Value)>,
) -> Result<(), AppError> {
    // HasDestroyedWebHook: the payload before the destroy, the hooks after.
    let payloads = crate::web_hooks::Payloads {
        settings: ctx.settings,
        i18n: ctx.i18n,
        config: ctx.config,
    };
    let destroyed_payload = if crate::web_hooks::active(&mut *conn, "user_destroyed").await? {
        Some(payloads.user(&mut *conn, user_id).await?)
    } else {
        None
    };
    let exec = |sql: &'static str| sqlx::query(sql).bind(user_id);

    exec("DELETE FROM category_users WHERE user_id = $1")
        .execute(&mut *conn)
        .await?;
    exec("DELETE FROM tag_users WHERE user_id = $1")
        .execute(&mut *conn)
        .await?;
    exec(
        "DELETE FROM user_api_key_scopes WHERE user_api_key_id IN \
           (SELECT id FROM user_api_keys WHERE user_id = $1)",
    )
    .execute(&mut *conn)
    .await?;
    exec("DELETE FROM user_api_keys WHERE user_id = $1")
        .execute(&mut *conn)
        .await?;
    exec("DELETE FROM topic_allowed_users WHERE user_id = $1")
        .execute(&mut *conn)
        .await?;
    exec("DELETE FROM user_archived_messages WHERE user_id = $1")
        .execute(&mut *conn)
        .await?;
    // EmailChangeRequest belongs_to its tokens, dependent: :destroy.
    exec(
        "WITH gone AS (DELETE FROM email_change_requests WHERE user_id = $1 \
                       RETURNING old_email_token_id, new_email_token_id) \
         DELETE FROM email_tokens WHERE id IN (SELECT old_email_token_id FROM gone \
                                               UNION SELECT new_email_token_id FROM gone)",
    )
    .execute(&mut *conn)
    .await?;
    exec("DELETE FROM email_tokens WHERE user_id = $1")
        .execute(&mut *conn)
        .await?;
    // TopicLink: its clicks, and after commit a crawl of the gone link.
    let links: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM topic_links WHERE user_id = $1 ORDER BY id")
            .bind(user_id)
            .fetch_all(&mut *conn)
            .await?;
    for id in links {
        sqlx::query("DELETE FROM topic_link_clicks WHERE topic_link_id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM topic_links WHERE id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
        after_commit.push(("crawl_topic_link", json!({ "topic_link_id": id })));
    }
    exec("DELETE FROM user_uploads WHERE user_id = $1")
        .execute(&mut *conn)
        .await?;
    exec("DELETE FROM upload_references WHERE target_type = 'User' AND target_id = $1")
        .execute(&mut *conn)
        .await?;
    // UserEmail after_destroy: every token for the address, anyone's.
    exec(
        "WITH gone AS (DELETE FROM user_emails WHERE user_id = $1 RETURNING email) \
         DELETE FROM email_tokens WHERE email IN (SELECT email FROM gone)",
    )
    .execute(&mut *conn)
    .await?;
    for sql in [
        "DELETE FROM user_associated_accounts WHERE user_id = $1",
        "DELETE FROM oauth2_user_infos WHERE user_id = $1",
        "DELETE FROM user_second_factors WHERE user_id = $1",
    ] {
        sqlx::query(sql).bind(user_id).execute(&mut *conn).await?;
    }
    // user_badges (for_enabled_badges): each one's notification, and the
    // badge's grant count down. Each one's after_destroy triggers
    // user_badge_revoked: its web hooks get the grant as Rails held it.
    let mut revoked: Vec<(i64, serde_json::Value)> = Vec::new();
    if crate::web_hooks::active(&mut *conn, "user_badge_revoked").await? {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT ub.id::int8 FROM user_badges ub JOIN badges b ON b.id = ub.badge_id \
             WHERE b.enabled AND ub.user_id = $1 ORDER BY ub.ctid",
        )
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?;
        for id in ids {
            revoked.push((
                id,
                crate::web_hooks::payloads::user_badge_json(&mut *conn, id).await?,
            ));
        }
    }
    let badges: Vec<(i32, Option<i64>)> = sqlx::query_as(
        "DELETE FROM user_badges ub USING badges b \
         WHERE ub.badge_id = b.id AND b.enabled AND ub.user_id = $1 \
         RETURNING ub.badge_id, ub.notification_id",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    for (badge_id, notification_id) in badges {
        if let Some(notification_id) = notification_id {
            sqlx::query("DELETE FROM notifications WHERE id = $1")
                .bind(notification_id)
                .execute(&mut *conn)
                .await?;
        }
        sqlx::query("UPDATE badges SET grant_count = COALESCE(grant_count, 0) - 1 WHERE id = $1")
            .bind(badge_id)
            .execute(&mut *conn)
            .await?;
    }
    for (id, payload) in revoked {
        let mut opts = serde_json::Map::new();
        opts.insert("id".into(), json!(id));
        opts.insert("payload".into(), json!(crate::web_hooks::to_json(&payload)));
        crate::web_hooks::enqueue_hooks(&mut *conn, "user_badge", "user_badge_revoked", opts)
            .await?;
    }
    // UserAuthToken's before_destroy logs only with verbose logging, and
    // the user's logs go below.
    exec("DELETE FROM user_auth_tokens WHERE user_id = $1")
        .execute(&mut *conn)
        .await?;
    // GroupUser after commit: GroupManager#sync_removal_side_effects
    // counts the user out (a human: bots hidden or not), then stops, the
    // user being gone.
    let groups: Vec<i32> =
        sqlx::query_scalar("DELETE FROM group_users WHERE user_id = $1 RETURNING group_id")
            .bind(user_id)
            .fetch_all(&mut *conn)
            .await?;
    for group_id in groups {
        sqlx::query("UPDATE groups SET user_count = user_count - 1 WHERE id = $1")
            .bind(group_id)
            .execute(&mut *conn)
            .await?;
    }
    for sql in [
        "DELETE FROM user_warnings WHERE user_id = $1",
        "DELETE FROM api_keys WHERE user_id = $1",
        "DELETE FROM push_subscriptions WHERE user_id = $1",
        "DELETE FROM group_histories WHERE acting_user_id = $1",
        "DELETE FROM group_histories WHERE target_user_id = $1",
        "DELETE FROM reviewable_scores WHERE user_id = $1",
        "DELETE FROM invites WHERE invited_by_id = $1 AND deleted_at IS NULL",
        "DELETE FROM user_custom_fields WHERE user_id = $1",
        "DELETE FROM user_associated_groups WHERE user_id = $1",
        "DELETE FROM user_options WHERE user_id = $1",
        "DELETE FROM upload_references WHERE target_type = 'UserAvatar' AND target_id IN \
           (SELECT id FROM user_avatars WHERE user_id = $1)",
        "DELETE FROM user_avatars WHERE user_id = $1",
        "DELETE FROM user_stats WHERE user_id = $1",
        "DELETE FROM upload_references WHERE target_type = 'UserProfile' AND target_id = $1",
        "DELETE FROM user_profile_views WHERE user_profile_id = $1",
        "DELETE FROM user_profiles WHERE user_id = $1",
        "DELETE FROM single_sign_on_records WHERE user_id = $1",
        "DELETE FROM anonymous_users WHERE user_id = $1",
        "DELETE FROM anonymous_users WHERE master_user_id = $1 AND active",
    ] {
        sqlx::query(sql).bind(user_id).execute(&mut *conn).await?;
    }
    // InvitedUser after_destroy: the invite's redemption count down.
    exec(
        "WITH gone AS (DELETE FROM invited_users WHERE user_id = $1 RETURNING invite_id) \
         UPDATE invites SET redemption_count = redemption_count - 1 \
         WHERE id IN (SELECT invite_id FROM gone) AND redemption_count > 0",
    )
    .execute(&mut *conn)
    .await?;
    // SidebarSection: its links, a custom link's SidebarUrl with it.
    exec(
        "WITH gone AS (DELETE FROM sidebar_section_links WHERE sidebar_section_id IN \
                         (SELECT id FROM sidebar_sections WHERE user_id = $1) \
                       RETURNING linkable_id, linkable_type) \
         DELETE FROM sidebar_urls WHERE id IN \
           (SELECT linkable_id FROM gone WHERE linkable_type = 'SidebarUrl')",
    )
    .execute(&mut *conn)
    .await?;
    for sql in [
        "DELETE FROM user_notification_schedules WHERE user_id = $1",
        "DELETE FROM user_passwords WHERE user_id = $1",
        "DELETE FROM sidebar_sections WHERE user_id = $1",
        "DELETE FROM user_statuses WHERE user_id = $1",
        // dependent: :delete_all
        "DELETE FROM bookmarks WHERE user_id = $1",
        "DELETE FROM notifications WHERE user_id = $1",
        "DELETE FROM topic_users WHERE user_id = $1",
        "DELETE FROM incoming_emails WHERE user_id = $1",
        "DELETE FROM user_visits WHERE user_id = $1",
        "DELETE FROM user_auth_token_logs WHERE user_id = $1",
        "DELETE FROM group_requests WHERE user_id = $1",
        "DELETE FROM muted_users WHERE user_id = $1",
        "DELETE FROM ignored_users WHERE user_id = $1",
        "DELETE FROM do_not_disturb_timings WHERE user_id = $1",
        "DELETE FROM reviewable_histories WHERE created_by_id = $1",
        "DELETE FROM sidebar_section_links WHERE user_id = $1",
        // Searchable
        "DELETE FROM user_search_data WHERE user_id = $1",
    ] {
        sqlx::query(sql).bind(user_id).execute(&mut *conn).await?;
    }
    crate::plugins::user_destroying(conn, user_id).await?;
    // The before_destroy block.
    for sql in [
        "DELETE FROM post_timings WHERE user_id = $1",
        "DELETE FROM topic_views WHERE user_id = $1",
        "DELETE FROM user_actions WHERE user_id = $1 OR target_user_id = $1 OR acting_user_id = $1",
        "DELETE FROM post_actions WHERE user_id = $1",
        "DELETE FROM directory_items WHERE user_id = $1",
        "DELETE FROM user_security_keys WHERE user_id = $1",
        "DELETE FROM developers WHERE user_id = $1",
        "DELETE FROM draft_sequences WHERE user_id = $1",
        "DELETE FROM given_daily_likes WHERE user_id = $1",
        "DELETE FROM muted_users WHERE user_id = $1 OR muted_user_id = $1",
        "DELETE FROM ignored_users WHERE user_id = $1 OR ignored_user_id = $1",
        "DELETE FROM user_avatars WHERE user_id = $1",
        "DELETE FROM users WHERE id = $1",
    ] {
        sqlx::query(sql).bind(user_id).execute(&mut *conn).await?;
    }
    if let Some(payload) = destroyed_payload {
        let mut opts = serde_json::Map::new();
        opts.insert("id".into(), json!(user_id));
        opts.insert("payload".into(), json!(crate::web_hooks::to_json(&payload)));
        crate::web_hooks::enqueue_hooks(&mut *conn, "user", "user_destroyed", opts).await?;
    }
    // after_destroy :clear_acls
    crate::jobs::enqueue(
        &mut *conn,
        "cleanup_acls_for_deleted",
        json!({ "user_id": user_id }),
    )
    .await?;
    Ok(())
}

/// `ScreenedEmail.block(email, ip_address:)&.record_match!`: the
/// canonical address found or created, its match counted.
async fn block_email(
    conn: &mut PgConnection,
    email: &str,
    ip_address: Option<&str>,
) -> Result<(), sqlx::Error> {
    let canonical = canonical_email(email);
    let found: Option<i32> = sqlx::query_scalar("SELECT id FROM screened_emails WHERE email = $1")
        .bind(&canonical)
        .fetch_optional(&mut *conn)
        .await?;
    match found {
        Some(id) => {
            record_match(conn, "screened_emails", id).await?;
        }
        None => {
            // create! (action_type block), then record_match!.
            sqlx::query(
                "INSERT INTO screened_emails (email, action_type, match_count, last_match_at, \
                                              ip_address, created_at, updated_at) \
                 VALUES ($1, 1, 1, clock_timestamp(), $2::inet, clock_timestamp(), clock_timestamp())",
            )
            .bind(&canonical)
            .bind(ip_address)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// `ScreenedEmail.canonical`: the local part's `+tag` dropped (and its
/// dots on Gmail), downcased.
fn canonical_email(email: &str) -> String {
    let (name, domain) = email.split_once('@').unwrap_or((email, ""));
    let name = name.split('+').next().unwrap_or(name);
    let name = if ["gmail.com", "googlemail.com"].contains(&domain.to_lowercase().as_str()) {
        name.replace('.', "")
    } else {
        name.to_string()
    };
    if email.contains('@') {
        format!("{name}@{domain}").to_lowercase()
    } else {
        name.to_lowercase()
    }
}

/// `ScreenedIpAddress.watch(ip)&.record_match!`: the most specific
/// screened range holding the address, or a new block of it.
async fn watch_ip(conn: &mut PgConnection, ip: &str) -> Result<(), sqlx::Error> {
    let found: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM screened_ip_addresses WHERE $1::inet <<= ip_address \
         ORDER BY masklen(ip_address) DESC LIMIT 1",
    )
    .bind(ip)
    .fetch_optional(&mut *conn)
    .await?;
    match found {
        Some(id) => record_match(conn, "screened_ip_addresses", id).await,
        None => {
            sqlx::query(
                "INSERT INTO screened_ip_addresses (ip_address, action_type, match_count, last_match_at, \
                                                     created_at, updated_at) \
                 VALUES ($1::inet, 1, 1, clock_timestamp(), clock_timestamp(), clock_timestamp())",
            )
            .bind(ip)
            .execute(&mut *conn)
            .await?;
            Ok(())
        }
    }
}

/// `ScreeningModel#record_match!`
async fn record_match(conn: &mut PgConnection, table: &str, id: i32) -> Result<(), sqlx::Error> {
    sqlx::query(&format!(
        "UPDATE {table} SET match_count = match_count + 1, last_match_at = clock_timestamp(), \
                            updated_at = clock_timestamp() WHERE id = $1"
    ))
    .bind(id)
    .execute(conn)
    .await?;
    Ok(())
}

/// The categories the user created pass to system (`save!`), and each
/// one's definition topic too (recovered, `save!`).
async fn adopt_categories(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
    after_commit: &mut Vec<(&'static str, serde_json::Value)>,
) -> Result<(), AppError> {
    let categories: Vec<(i32, Option<i32>)> =
        sqlx::query_as("SELECT id, topic_id FROM categories WHERE user_id = $1 ORDER BY id")
            .bind(user_id)
            .fetch_all(&mut *conn)
            .await?;
    let cx = crate::category_writes::Cx::from_posting(ctx);
    for (category_id, topic_id) in categories {
        // c.user_id = system; c.save! (Category#save with its callbacks).
        let mut record = crate::category_writes::Record::load(&mut *conn, category_id)
            .await?
            .ok_or(Unsupported(
                "a category that vanished during a user's deletion",
            ))?;
        record.set("user_id", json!(SYSTEM_USER_ID));
        match crate::category_writes::save(&mut *conn, &cx, &mut record).await? {
            crate::category_writes::Saved::Invalid(_) => {
                return Err(Unsupported(
                    "a category failing its validations as its owner is deleted (save!)",
                )
                .into());
            }
            crate::category_writes::Saved::Done(_) => {}
        }
        // Its commit callbacks: category_updated (web hooks refused, the
        // plugins' listeners), then the index job.
        if crate::web_hooks::active(&mut *conn, "category_updated").await? {
            return Err(Unsupported("category_updated web hooks from a user's deletion").into());
        }
        crate::category_writes::category_updated_plugins(&mut *conn, &cx, category_id, false)
            .await?;
        after_commit.push((
            "index_category_for_search",
            json!({ "category_id": category_id, "force": false }),
        ));
        let Some(topic_id) = topic_id else {
            continue;
        };
        let deleted: Option<Option<NaiveDateTime>> =
            sqlx::query_scalar("SELECT deleted_at FROM topics WHERE id = $1")
                .bind(topic_id)
                .fetch_optional(&mut *conn)
                .await?;
        let Some(deleted) = deleted else {
            continue;
        };
        if deleted.is_some() {
            recover_definition(conn, ctx, topic_id).await?;
        }
        // user_id = -1 and save!.
        let slug =
            crate::posting::topic_save::reassigned_slug(&mut *conn, ctx.settings, topic_id).await?;
        sqlx::query(
            "UPDATE topics SET user_id = $2, slug = $3, fancy_title = NULL, \
                               updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(topic_id)
        .bind(SYSTEM_USER_ID)
        .bind(&slug)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `Topic#recover!` for a deleted category definition: the category's
/// own count leaves its definition out, so only the row is recovered;
/// tags, an embed and the plugins' topic_recovered listeners (topic
/// voting's reclaimed votes, boards) are refused.
async fn recover_definition(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
) -> Result<(), AppError> {
    let (visible, category_id, own_category, closed, archived): (bool, Option<i32>, bool, bool, bool) =
        sqlx::query_as(
        "SELECT t.visible, t.category_id, \
                EXISTS (SELECT 1 FROM categories c WHERE c.id = t.category_id AND c.topic_id = t.id), \
                t.closed, t.archived \
         FROM topics t WHERE t.id = $1",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    if visible && category_id.is_some() && !own_category {
        return Err(Unsupported("recovering a definition topic outside its category").into());
    }
    let (tags, embed): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM topic_tags WHERE topic_id = $1), \
                EXISTS (SELECT 1 FROM topic_embeds WHERE topic_id = $1)",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    if tags || embed {
        return Err(Unsupported("recovering a tagged or embedded category definition").into());
    }
    if ctx.settings.get("boards_enabled").is_ok_and(|v| v.truthy()) {
        return Err(Unsupported("the boards plugin's topic_recovered sync").into());
    }
    sqlx::query("UPDATE topics SET deleted_at = NULL, deleted_by_id = NULL WHERE id = $1")
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    // DiscourseEvent :topic_recovered
    if crate::plugins::topic_voting::enabled(ctx.settings)? {
        crate::plugins::topic_voting::lifecycle::topic_recovered(
            &mut *conn, topic_id, closed, archived,
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_emails() {
        assert_eq!(canonical_email("User3@Example.com"), "user3@example.com");
        assert_eq!(canonical_email("a.b+tag@example.com"), "a.b@example.com");
        assert_eq!(canonical_email("a.b+tag@GMail.com"), "ab@gmail.com");
    }

    #[test]
    fn ruby_times() {
        let t = NaiveDateTime::parse_from_str("2026-07-25 05:59:17.123", "%Y-%m-%d %H:%M:%S%.f")
            .unwrap();
        assert_eq!(ruby_time(t), "2026-07-25 05:59:17 UTC");
    }
}
