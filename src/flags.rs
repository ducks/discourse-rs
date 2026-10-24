//! Flags: PostActionCreator for the flag types: the post action and the
//! post's count of it (PostAction#update_counters), the post's
//! ReviewableFlaggedPost and the flag's score for the types that go to
//! review, and the rules a flag is held to (auto close, auto hide, auto
//! silence). Staff taking action hide the post and agree with its flags.
//! A flag with a message (notify user, notify moderators, illegal, spam)
//! first creates the message (create_message_creator): to the author for
//! notify user, else to the moderators, its post the action's related post
//! and its topic the score's meta topic. A topic's flag (flag_topic) is its
//! first post's, targeting the topic when it has replies. A post already
//! in review takes the flag's score on its reviewable, which is reopened
//! when it had been reviewed, unless the same type was reviewed lately
//! and the post not edited since (cannot_flag_again?).
//!
//! Refused: official warnings (is_warning), queueing for review as staff,
//! flags in messages, and the rules when they would close the topic, hide
//! on a trusted spam flag or silence the author. The badge queue is not
//! ported; flags write no user actions or notifications.

use sqlx::{PgConnection, PgPool};

use crate::guardian::Guardian;
use crate::likes::Outcome;
use crate::post_actions::{self, ActOpts, ActionTypes};
use crate::posting::Ctx;
use crate::posting::revisions::find_post;
use crate::reviewables::{self, Flagger, NewFlaggedPost, NewScore};
use crate::{AppError, Unsupported};

/// What a flag request asked for beyond the type.
pub struct FlagRequest {
    pub type_id: i64,
    pub take_action: bool,
    pub queue_for_review: bool,
    pub message: Option<String>,
    /// `is_warning`: the message as an official warning.
    pub is_warning: bool,
    /// `flag_topic`: the topic's flag, made on its first post.
    pub flag_topic: bool,
    /// `reason:` and `context:`, which the reviewable score keeps (a
    /// watched word flag's).
    pub reason: Option<&'static str>,
    pub context: Option<String>,
}

/// The post's author as the rules read them.
struct Author {
    staff: bool,
    staged: bool,
    trust_level: i32,
}

async fn author(
    conn: &mut PgConnection,
    user_id: Option<i32>,
) -> Result<Option<Author>, sqlx::Error> {
    let Some(id) = user_id else {
        return Ok(None);
    };
    let row: Option<(bool, bool, bool, i32)> =
        sqlx::query_as("SELECT admin, moderator, staged, trust_level FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(conn)
            .await?;
    Ok(row.map(|(admin, moderator, staged, trust_level)| Author {
        staff: admin || moderator,
        staged,
        trust_level,
    }))
}

/// `PostAction#update_counters` for a flag: the post's `<type>_count`.
pub async fn update_counters(
    conn: &mut PgConnection,
    types: &ActionTypes,
    post_id: i32,
    type_id: i64,
) -> Result<(), AppError> {
    let name = types
        .name(type_id)
        .ok_or(Unsupported("counting an unknown post action type"))?;
    // Only a type with a posts column of its own (a custom flag has none).
    let column = format!("{name}_count");
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_attribute \
         WHERE attrelid = 'posts'::regclass AND attname = $1 AND NOT attisdropped)",
    )
    .bind(&column)
    .fetch_one(&mut *conn)
    .await?;
    if !exists {
        return Ok(());
    }
    sqlx::query(&format!(
        "UPDATE posts SET {name}_count = (SELECT COUNT(*) FROM post_actions WHERE post_id = $1 \
                                          AND post_action_type_id = $2 AND deleted_at IS NULL) \
         WHERE id = $1"
    ))
    .bind(post_id)
    .bind(type_id as i32)
    .execute(conn)
    .await?;
    Ok(())
}

/// `PostActionCreator.new(user, post, type, ...).perform` for a flag.
pub async fn flag(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i32,
    req: &FlagRequest,
) -> Result<Outcome, AppError> {
    let user = guardian
        .user()
        .ok_or(Unsupported("flagging anonymously"))?
        .clone();
    let staff = guardian.is_staff();
    let mut tx = pool.begin().await?;
    // @created_at = Time.zone.now, when the creator is made.
    let created_at: chrono::NaiveDateTime =
        sqlx::query_scalar("SELECT clock_timestamp()::timestamp")
            .fetch_one(&mut *tx)
            .await?;
    let Some(access) = find_post(&mut tx, ctx, guardian, post_id).await? else {
        return Ok(Outcome::NotFound);
    };
    if access.topic.private_message() {
        return Err(Unsupported("flags in messages").into());
    }
    let post = &access.post;
    let types = ActionTypes::load(&mut tx).await?;
    let Some(name) = types.name(req.type_id).map(str::to_string) else {
        return Err(Unsupported("post action types that are neither likes nor flags").into());
    };
    // @take_action = take_action && guardian.is_staff?
    let take_action = req.take_action && staff;

    let taken = post_actions::taken_actions(&mut tx, &[post.id], Some(user.id)).await?;
    let taken = taken.get(&post.id);
    let post_author = author(&mut tx, post.user_id).await?;
    let can_act = guardian.post_can_act(
        ctx.settings,
        &types,
        (&name, req.type_id),
        &ActOpts {
            topic: &access.topic,
            post,
            taken,
            can_see_post: access.can_see_post,
            author_missing: post_author.is_none(),
        },
    )?;
    if !can_act || (req.queue_for_review && !staff) {
        return Ok(Outcome::Forbidden(
            if taken.is_some_and(|t| t.contains_key(&req.type_id)) {
                "action_already_performed"
            } else {
                "invalid_access"
            },
        ));
    }
    if let Err(refused) = crate::likes::perform(
        pool,
        ctx,
        post_actions::action_limiter(
            crate::likes::limit_user(guardian, &user),
            post.id,
            req.type_id,
        ),
    )
    .await?
    {
        return Ok(Outcome::RateLimited(refused));
    }
    if req.queue_for_review {
        return Err(Unsupported("queueing a post for review as staff").into());
    }
    // Reviewable.find_by(target: @post), of any type.
    let reviewable: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM reviewables WHERE target_type = 'Post' AND target_id = $1 ORDER BY id LIMIT 1",
    )
    .bind(post.id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(reviewable_id) = reviewable
        && types.is_notify_flag(req.type_id)
        && cannot_flag_again(&mut tx, ctx, &types, reviewable_id, post.id, req.type_id).await?
    {
        let message = ctx
            .i18n
            .t("reviewables.already_handled")
            .ok_or(Unsupported("reviewables.already_handled's text"))?;
        return Ok(Outcome::Invalid(vec![message.to_string()]));
    }

    // create meta topic / post if needed: a message present for a type
    // that sends one, spam and illegal included.
    let mut meta_post: Option<MetaPost> = None;
    if let Some(message) = req.message.as_deref().filter(|m| !m.trim().is_empty())
        && (types.requires_message(req.type_id) || name == "spam" || name == "illegal")
    {
        if req.is_warning {
            return Err(Unsupported("official warnings (is_warning)").into());
        }
        match create_message(pool, ctx, guardian, &access, &name, message).await? {
            Ok(meta) => meta_post = Some(meta),
            Err(errors) => return Ok(Outcome::Invalid(errors)),
        }
    }
    // @targets_topic: a topic's flag targets the topic once it has replies.
    let targets_topic = if req.flag_topic {
        let posts_count: i32 = sqlx::query_scalar("SELECT posts_count FROM topics WHERE id = $1")
            .bind(access.topic_id)
            .fetch_one(&mut *tx)
            .await?;
        posts_count != 1
    } else {
        false
    };
    let related_post_id = meta_post.as_ref().map(|m| m.post_id);

    // create_post_action: a trashed flag that was never reviewed comes
    // back, else a new one.
    let revived = sqlx::query(
        "UPDATE post_actions SET deleted_at = NULL, deleted_by_id = NULL, staff_took_action = $5, \
                related_post_id = $6, targets_topic = $7, created_at = $4, updated_at = clock_timestamp() \
         WHERE id = (SELECT id FROM post_actions WHERE post_id = $1 AND user_id = $2 AND post_action_type_id = $3 \
                       AND deleted_at IS NOT NULL AND agreed_at IS NULL AND disagreed_at IS NULL \
                       AND deferred_at IS NULL ORDER BY id LIMIT 1)",
    )
    .bind(post.id)
    .bind(user.id)
    .bind(req.type_id as i32)
    .bind(created_at)
    .bind(take_action)
    .bind(related_post_id)
    .bind(targets_topic)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if revived == 0 {
        sqlx::query(
            "INSERT INTO post_actions (post_id, user_id, post_action_type_id, staff_took_action, targets_topic, \
                                       related_post_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $5, $7, $6, $4, clock_timestamp())",
        )
        .bind(post.id)
        .bind(user.id)
        .bind(req.type_id as i32)
        .bind(created_at)
        .bind(take_action)
        .bind(related_post_id)
        .bind(targets_topic)
        .execute(&mut *tx)
        .await?;
        // PostAction.create's after_create: the day's flags.
        if let Err(refused) = crate::likes::perform(
            pool,
            ctx,
            post_actions::daily_limiter(
                ctx.settings,
                crate::likes::limit_user(guardian, &user),
                user.trust_level,
                "flag",
            )?,
        )
        .await?
        {
            return Ok(Outcome::RateLimited(refused));
        }
        // A new post action's badges, queued (related_post_id is NULL).
        crate::badge_granter::queue(
            &mut tx,
            ctx.settings,
            crate::badge_granter::Trigger::PostAction,
            &[post.id],
        )
        .await?;
    }
    // after_save
    update_counters(&mut tx, &types, post.id, req.type_id).await?;

    // create_reviewable, for flags that go to review; none for bots' posts.
    let mut reviewable_score = None;
    if types.is_notify_flag(req.type_id) && post.user_id.is_some_and(|id| id >= 0) {
        let reviewable = reviewables::flagged_post_needs_review(
            &mut tx,
            &crate::web_hooks::Payloads {
                settings: ctx.settings,
                i18n: ctx.i18n,
                config: ctx.config,
            },
            &NewFlaggedPost {
                created_by_id: user.id,
                post_id: post.id,
                topic_id: access.topic_id,
                category_id: access.topic.category_id,
                post_user_id: post.user_id,
                potential_spam: name == "spam",
                potentially_illegal: name == "illegal",
                targets_topic,
            },
        )
        .await?;
        let flagger = Flagger {
            id: user.id,
            staff,
            trust_level: user.trust_level,
        };
        reviewables::add_score(
            &mut tx,
            &crate::web_hooks::Payloads {
                settings: ctx.settings,
                i18n: ctx.i18n,
                config: ctx.config,
            },
            &reviewable,
            &flagger,
            &NewScore {
                score_type: req.type_id,
                created_at,
                take_action,
                reason: req.reason,
                context: req.context.clone(),
                meta_topic_id: meta_post.as_ref().map(|m| m.topic_id),
            },
        )
        .await?;
        reviewable_score =
            sqlx::query_scalar::<_, f64>("SELECT score FROM reviewables WHERE id = $1")
                .bind(reviewable.id)
                .fetch_optional(&mut *tx)
                .await?;
    }

    enforce_rules(
        &mut tx,
        ctx,
        guardian,
        &access,
        post_author.as_ref(),
        &name,
        req.type_id,
        &types,
        reviewable_score,
        take_action,
    )
    .await?;
    // notify_subscribers: a flag that goes to review redraws the post.
    if types.is_notify_flag(req.type_id) {
        crate::bus::publish_post_change(ctx, &mut tx, post.id, "acted", Default::default(), false)
            .await?;
    }
    // Taking action agrees with the post's flags, this one included.
    if take_action {
        crate::review::agree_with_flags(&mut tx, ctx, user.id, &user.username, post.id).await?;
        update_counters(&mut tx, &types, post.id, req.type_id).await?;
    }
    tx.commit().await?;
    Ok(Outcome::Done)
}

/// `cannot_flag_again?(reviewable)`: not for notify moderators; a type
/// already reviewed on the post's reviewable, the post not edited since
/// and the review within cooldown_hours_until_reflag.
async fn cannot_flag_again(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    types: &ActionTypes,
    reviewable_id: i64,
    post_id: i32,
    type_id: i64,
) -> Result<bool, AppError> {
    if types.name(type_id) == Some("notify_moderators") {
        return Ok(false);
    }
    let hours = ctx.settings.get("cooldown_hours_until_reflag")?.to_i();
    let refused: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM reviewable_scores rs \
                        WHERE rs.reviewable_id = r.id AND rs.reviewable_score_type = $2 \
                          AND rs.status <> $4) \
            AND (p.last_version_at IS NULL OR r.updated_at > p.last_version_at) \
            AND r.updated_at > clock_timestamp()::timestamp - make_interval(hours => $5) \
         FROM reviewables r, posts p WHERE r.id = $1 AND p.id = $3",
    )
    .bind(reviewable_id)
    .bind(type_id as i32)
    .bind(post_id)
    .bind(reviewables::PENDING)
    .bind(hours as i32)
    .fetch_one(conn)
    .await?;
    Ok(refused)
}

/// The message a flag sends: its post and topic.
struct MetaPost {
    post_id: i32,
    topic_id: i32,
}

/// ActiveSupport's `String#truncate(length, separator: /\s/)`.
fn truncate_at_space(s: &str, length: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= length {
        return s.to_string();
    }
    let stop = length.saturating_sub(3);
    // rindex(separator, stop): the last whitespace at or before `stop`.
    let cut = chars[..=stop.min(chars.len() - 1)]
        .iter()
        .rposition(|c| c.is_whitespace())
        .unwrap_or(stop);
    format!("{}...", chars[..cut].iter().collect::<String>())
}

/// `create_message_creator` then `creator.create`: a message from the
/// flagger, to the post's author for notify user, else to the moderators
/// group (and the category's moderation groups). The creator's errors when
/// it refuses.
async fn create_message(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    access: &crate::posting::revisions::PostAccess,
    name: &str,
    message: &str,
) -> Result<Result<MetaPost, Vec<String>>, AppError> {
    let s = ctx.settings;
    let mut conn = pool.acquire().await?;
    let (topic_title, slug, category_id): (String, Option<String>, Option<i32>) =
        sqlx::query_as("SELECT title, slug, category_id FROM topics WHERE id = $1")
            .bind(access.topic_id)
            .fetch_one(&mut *conn)
            .await?;
    // I18n.t("post_action_types.#{name}.email_title", default: illegal's).
    let key = |part: &str| {
        let own = format!("post_action_types.{name}.{part}");
        if ctx.i18n.t(&own).is_some() {
            own
        } else {
            format!("post_action_types.illegal.{part}")
        }
    };
    let title = ctx
        .i18n
        .t_with(&key("email_title"), &[("title", &topic_title)])
        .ok_or(Unsupported("a flag message's title"))?;
    let base_url = crate::url::Urls {
        config: ctx.config,
        settings: s,
    }
    .base_url()?;
    // Post#url
    let link = format!(
        "{base_url}/t/{}/{}/{}",
        slug.unwrap_or_default(),
        access.topic_id,
        access.post.post_number
    );
    let raw = ctx
        .i18n
        .t_with(&key("email_body"), &[("message", message), ("link", &link)])
        .ok_or(Unsupported("a flag message's body"))?;
    let max_title = s.get("max_topic_title_length")?.to_i().max(0) as usize;
    let mut post = crate::posting::create::NewPost {
        raw,
        title: Some(truncate_at_space(&title, max_title)),
        ..Default::default()
    };
    if name == "notify_user" {
        post.subtype = Some("notify_user".to_string());
        let username: Option<String> =
            sqlx::query_scalar("SELECT username_lower FROM users WHERE id = $1")
                .bind(access.post.user_id)
                .fetch_optional(&mut *conn)
                .await?;
        post.pm_recipients = Some(username.into_iter().collect());
    } else {
        post.subtype = Some("notify_moderators".to_string());
        let mut groups: Vec<String> = sqlx::query_scalar("SELECT name FROM groups WHERE id = 2")
            .fetch_all(&mut *conn)
            .await?;
        if s.get("enable_category_group_moderation")?.truthy()
            && let Some(category_id) = category_id
        {
            let more: Vec<String> = sqlx::query_scalar(
                "SELECT g.name FROM groups g JOIN category_moderation_groups cmg ON cmg.group_id = g.id \
                 WHERE cmg.category_id = $1",
            )
            .bind(category_id)
            .fetch_all(&mut *conn)
            .await?;
            for group in more {
                if !groups.contains(&group) {
                    groups.push(group);
                }
            }
        }
        post.pm_recipients = Some(Vec::new());
        post.pm_groups = groups.iter().map(|g| g.to_lowercase()).collect();
    }
    drop(conn);
    match crate::posting::create::create(pool, ctx, guardian, post).await? {
        crate::posting::create::Outcome::Created { post_id } => {
            let topic_id: i32 = sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
                .bind(post_id)
                .fetch_one(pool)
                .await?;
            Ok(Ok(MetaPost { post_id, topic_id }))
        }
        crate::posting::create::Outcome::Invalid(errors) => Ok(Err(errors)),
        _ => Err(Unsupported("a flag message PostCreator answered otherwise").into()),
    }
}

/// `enforce_rules`: auto close, auto hide, auto silence. Hiding is ported;
/// closing the topic, hiding by a trusted spam flagger and silencing are
/// refused when they would act.
#[allow(clippy::too_many_arguments)]
async fn enforce_rules(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    access: &crate::posting::revisions::PostAccess,
    post_author: Option<&Author>,
    name: &str,
    type_id: i64,
    types: &ActionTypes,
    reviewable_score: Option<f64>,
    take_action: bool,
) -> Result<(), AppError> {
    let s = ctx.settings;
    // auto_close_if_threshold_reached: closed by system, with an open
    // timer num_hours_to_close_topic away.
    if !access.topic.closed
        && crate::topic_timer::auto_close_threshold_reached(&mut *conn, s, access.topic_id).await?
    {
        let hours = s.get("num_hours_to_close_topic")?.to_i();
        let message = ctx.t_count("temporarily_closed_due_to_flags", hours);
        crate::topic_status::apply(
            &mut *conn,
            ctx,
            access.topic_id,
            crate::topic_timer::SYSTEM_USER_ID,
            crate::topic_status::Status::Closed,
            true,
            &crate::topic_status::StatusOpts {
                message: Some(&message),
                ..Default::default()
            },
        )
        .await?;
        crate::topic_timer::set_or_create_timer(
            &mut *conn,
            ctx,
            access.topic_id,
            crate::topic_timer::types::OPEN,
            crate::topic_timer::When::Hours(hours as f64),
            &crate::topic_timer::SetOptions {
                by_user: Some(crate::topic_timer::SYSTEM_USER_ID),
                ..Default::default()
            },
        )
        .await?;
    }

    // auto_hide_if_needed
    let flagger_staff = guardian.is_staff();
    let author_staff = post_author.is_some_and(|a| a.staff);
    if !access.post.hidden && (flagger_staff || !author_staff) && types.is_auto_action(type_id) {
        let trusted_spam_flagger = s.get("high_trust_flaggers_auto_hide_posts")?.truthy()
            && name == "spam"
            && guardian.has_trust_level(3)
            && post_author.is_some_and(|a| a.trust_level == 0);
        if trusted_spam_flagger {
            return Err(Unsupported("hiding a post flagged as spam by a trusted user").into());
        }
        let to_hide =
            reviewables::sensitivity_score(&mut *conn, s, "hide_post_sensitivity", 1.0).await?;
        if reviewable_score.unwrap_or(0.0) >= to_hide || take_action {
            crate::review::hide_post(&mut *conn, ctx, types, access.post.id, type_id).await?;
        }
    }

    // SpamRule::AutoSilence for the post's author
    if let (Some(author), Some(author_id)) = (post_author, access.post.user_id)
        && !author.staged
        && !(author.staff || author.trust_level >= 1)
        && s.get("num_users_to_silence_new_user")?.to_i() > 0
    {
        let (total, users): (f64, i64) = sqlx::query_as(
            "SELECT COALESCE(SUM(rs.score), 0)::float8, COUNT(DISTINCT rs.user_id) \
             FROM reviewables r JOIN reviewable_scores rs ON rs.reviewable_id = r.id \
             WHERE r.target_created_by_id = $1 AND rs.reviewable_score_type = $2 \
               AND rs.status IN ($3, $4)",
        )
        .bind(author_id)
        .bind(
            types
                .types
                .iter()
                .find(|(k, _)| k == "spam")
                .map_or(8, |(_, id)| *id) as i32,
        )
        .bind(reviewables::PENDING)
        .bind(reviewables::AGREED)
        .fetch_one(&mut *conn)
        .await?;
        let to_silence =
            reviewables::sensitivity_score(&mut *conn, s, "silence_new_user_sensitivity", 0.6)
                .await?;
        if total >= to_silence && users >= s.get("num_users_to_silence_new_user")?.to_i() {
            return Err(Unsupported("silencing a new user on spam flags").into());
        }
    }
    Ok(())
}
