//! Port of app/models/topic_converter.rb: a regular topic becomes a
//! message or a message a regular topic, through PostRevisor (the
//! archetype and category in a hidden revision) and what the converter
//! adds around it: the category's count, the posters' counts, the
//! allowed users, the tag counters, the jobs, watching, and the small
//! action once it commits.

use serde_json::json;
use sqlx::{PgConnection, PgPool};

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::posting::revise_topic::{self, Applied, Deferred, Opts, TopicFields};
use crate::posting::small_action::{self, SmallAction};
use crate::posting::{TopicUserAttr, change_topic_user, notification_levels, notification_reasons};
use crate::{AppError, Unsupported};

/// How a conversion ends.
pub enum Converted {
    /// The topic's relative URL (render_topic_changes).
    Done(String),
    /// The topic's errors (render_json_error).
    Invalid(Vec<String>),
}

/// `revise_opts`: a silent conversion (the bulk action's) doesn't bump or
/// notify; the revision is hidden either way.
fn revise_opts(silent: bool) -> Opts {
    Opts {
        bypass_bump: silent,
        silent,
        hidden: true,
        ..Default::default()
    }
}

/// `TopicConverter#convert_to_public_topic(category_id)`; a silent one
/// leaves no small action and changes no one's watching.
pub async fn to_public(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    user: &Guardian,
    topic_id: i32,
    category_id: Option<i32>,
    silent: bool,
) -> Result<Converted, AppError> {
    let s = ctx.settings;
    let uncategorized = s.get("uncategorized_category_id")?.to_i() as i32;
    let mut tx = pool.begin().await?;
    let category_id = match category_id {
        Some(id) => Some(id),
        None if s.get("allow_uncategorized_topics")?.truthy() => Some(uncategorized),
        None => None,
    };
    let mut category: Option<(i32, bool)> = match category_id {
        Some(id) => {
            sqlx::query_as("SELECT id, read_restricted FROM categories WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
        }
        None => None,
    };
    if category.is_none() {
        category = sqlx::query_as(
            "SELECT id, read_restricted FROM categories WHERE NOT read_restricted AND id <> $1 \
             ORDER BY id LIMIT 1",
        )
        .bind(uncategorized)
        .fetch_optional(&mut *tx)
        .await?;
    }
    let (category_id, read_restricted) = category.ok_or(Unsupported(
        "converting to a public topic without a category",
    ))?;
    let fields = TopicFields {
        category_id: Some(category_id),
        archetype: Some("regular".into()),
        ..Default::default()
    };
    let mut deferred = Deferred::default();
    match revise(
        &mut tx,
        pool,
        ctx,
        user,
        topic_id,
        &fields,
        revise_opts(silent),
        &mut deferred,
    )
    .await?
    {
        Some(Ok(())) => {}
        Some(Err(errors)) => {
            tx.rollback().await?;
            return Ok(Converted::Invalid(errors));
        }
        None => {
            tx.rollback().await?;
            return Ok(Converted::Done(relative_url(pool, ctx, topic_id).await?));
        }
    }
    // update_user_stats
    users_post_count(&mut tx, topic_id, 1).await?;
    sqlx::query(
        "UPDATE user_stats SET topic_count = topic_count + 1 \
         WHERE user_id = (SELECT user_id FROM posts WHERE topic_id = $1 AND post_number = 1 \
                            AND deleted_at IS NULL)",
    )
    .bind(topic_id)
    .execute(&mut *tx)
    .await?;
    deferred.push(
        "update_topic_upload_security",
        json!({"topic_id": topic_id}),
    );
    tag_counters(&mut tx, topic_id, 1, !read_restricted).await?;
    crate::jobs::enqueue(
        &mut tx,
        "topic_action_converter",
        json!({"topic_id": topic_id}),
    )
    .await?;
    crate::jobs::enqueue(
        &mut tx,
        "delete_inaccessible_notifications",
        json!({"topic_id": topic_id}),
    )
    .await?;
    if !silent {
        watch_topic(&mut tx, ctx, topic_id).await?;
    }
    deferred.enqueue(&mut tx, ctx).await?;
    tx.commit().await?;
    if !silent {
        add_small_action(pool, ctx, user, topic_id, "public_topic").await?;
    }
    Ok(Converted::Done(relative_url(pool, ctx, topic_id).await?))
}

/// `TopicConverter#convert_to_private_message`
pub async fn to_private(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    user: &Guardian,
    topic_id: i32,
    silent: bool,
) -> Result<Converted, AppError> {
    let s = ctx.settings;
    let user_id = user
        .user_id()
        .ok_or(Unsupported("converting a topic anonymously"))?;
    let mut tx = pool.begin().await?;
    // exceeds_recipient_cap?
    let posters = posters(&mut tx, topic_id).await?;
    let allowed: Vec<i32> =
        sqlx::query_scalar("SELECT user_id FROM topic_allowed_users WHERE topic_id = $1")
            .bind(topic_id)
            .fetch_all(&mut *tx)
            .await?;
    let mut recipients: Vec<i32> = posters.clone();
    for id in allowed.iter().chain(std::iter::once(&user_id)) {
        if !recipients.contains(id) {
            recipients.push(*id);
        }
    }
    let max = s.get("max_allowed_message_recipients")?.to_i();
    if recipients.len() as i64 > max {
        let message = ctx
            .i18n
            .t_with(
                "topic_converter.too_many_recipients",
                &[("max", &max.to_string())],
            )
            .unwrap_or_default();
        return Ok(Converted::Invalid(vec![message]));
    }
    let (category_id, visible): (Option<i32>, bool) =
        sqlx::query_as("SELECT category_id, visible FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&mut *tx)
            .await?;
    let category_id = category_id.ok_or(Unsupported("converting a topic without a category"))?;
    let was_public: bool =
        sqlx::query_scalar("SELECT NOT read_restricted FROM categories WHERE id = $1")
            .bind(category_id)
            .fetch_one(&mut *tx)
            .await?;
    if visible {
        // update_category_topic_count_by(-1)
        sqlx::query(
            "UPDATE categories SET topic_count = topic_count - 1 \
             WHERE id = $1 AND (topic_id <> $2 OR topic_id IS NULL)",
        )
        .bind(category_id)
        .bind(topic_id)
        .execute(&mut *tx)
        .await?;
    }
    let fields = TopicFields {
        category_id: Some(0),
        archetype: Some("private_message".into()),
        ..Default::default()
    };
    let mut deferred = Deferred::default();
    match revise(
        &mut tx,
        pool,
        ctx,
        user,
        topic_id,
        &fields,
        revise_opts(silent),
        &mut deferred,
    )
    .await?
    {
        Some(Ok(())) => {}
        Some(Err(errors)) => {
            tx.rollback().await?;
            return Ok(Converted::Invalid(errors));
        }
        None => {
            tx.rollback().await?;
            return Ok(Converted::Done(relative_url(pool, ctx, topic_id).await?));
        }
    }
    let published: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM published_pages WHERE topic_id = $1)")
            .bind(topic_id)
            .fetch_one(&mut *tx)
            .await?;
    if published {
        return Err(
            Unsupported("converting a published page's topic (PublishedPage.unpublish!)").into(),
        );
    }
    // add_allowed_users
    users_post_count(&mut tx, topic_id, -1).await?;
    sqlx::query(
        "UPDATE user_stats SET topic_count = topic_count - 1 \
         WHERE topic_count >= 1 AND user_id = (SELECT user_id FROM posts WHERE topic_id = $1 \
                                               AND post_number = 1 AND deleted_at IS NULL)",
    )
    .bind(topic_id)
    .execute(&mut *tx)
    .await?;
    let mut to_allow: Vec<i32> = Vec::new();
    for id in posters.iter().chain(std::iter::once(&user_id)) {
        if !allowed.contains(id) && !to_allow.contains(id) {
            to_allow.push(*id);
        }
    }
    for id in &to_allow {
        sqlx::query(
            "INSERT INTO topic_allowed_users (topic_id, user_id, created_at, updated_at) \
             VALUES ($1, $2, clock_timestamp(), clock_timestamp())",
        )
        .bind(topic_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    revise_topic::save_as_is(&mut tx, ctx, user, topic_id).await?;
    deferred.push(
        "update_topic_upload_security",
        json!({"topic_id": topic_id}),
    );
    tag_counters(&mut tx, topic_id, -1, was_public).await?;
    sqlx::query("UPDATE user_profiles SET featured_topic_id = NULL WHERE featured_topic_id = $1")
        .bind(topic_id)
        .execute(&mut *tx)
        .await?;
    crate::jobs::enqueue(
        &mut tx,
        "topic_action_converter",
        json!({"topic_id": topic_id}),
    )
    .await?;
    crate::jobs::enqueue(
        &mut tx,
        "delete_inaccessible_notifications",
        json!({"topic_id": topic_id}),
    )
    .await?;
    if !silent {
        watch_topic(&mut tx, ctx, topic_id).await?;
    }
    deferred.enqueue(&mut tx, ctx).await?;
    tx.commit().await?;
    if !silent {
        add_small_action(pool, ctx, user, topic_id, "private_topic").await?;
    }
    Ok(Converted::Done(relative_url(pool, ctx, topic_id).await?))
}

/// PostRevisor inside the converter's transaction. None when revise!
/// returned false without errors on the topic (slow mode).
#[allow(clippy::too_many_arguments)]
async fn revise(
    conn: &mut PgConnection,
    pool: &PgPool,
    ctx: &Ctx<'_>,
    user: &Guardian,
    topic_id: i32,
    fields: &TopicFields,
    opts: Opts,
    deferred: &mut Deferred,
) -> Result<Option<Result<(), Vec<String>>>, AppError> {
    let Some(prepared) = revise_topic::prepare(&mut *conn, ctx, user, topic_id, fields).await?
    else {
        return Ok(None);
    };
    match revise_topic::apply(
        &mut *conn, pool, ctx, user, &prepared, fields, opts, deferred,
    )
    .await?
    {
        Applied::Revised(revised) => {
            revise_topic::finish(&mut *conn, ctx, user, &prepared, &revised, opts).await?;
            Ok(Some(Ok(())))
        }
        Applied::Invalid(errors) if errors.is_empty() => Ok(None),
        Applied::Invalid(errors) => Ok(Some(Err(errors))),
        Applied::RateLimited(_) => Err(Unsupported("the edit rate limit on a conversion").into()),
    }
}

/// `posters`: who wrote the topic's regular posts, as DISTINCT plucks them.
async fn posters(conn: &mut PgConnection, topic_id: i32) -> Result<Vec<i32>, sqlx::Error> {
    // The order the allowed users are added in: Postgres hashes the
    // DISTINCT on a table with its statistics, while a fresh copy without
    // them sorts; the plan is pinned to the hash aggregate for this query.
    for setting in ["enable_sort", "enable_indexscan", "enable_bitmapscan"] {
        sqlx::query(&format!("SET LOCAL {setting} = off"))
            .execute(&mut *conn)
            .await?;
    }
    let ids: Vec<Option<i32>> = sqlx::query_scalar(
        "SELECT DISTINCT posts.user_id FROM posts \
         WHERE posts.deleted_at IS NULL AND posts.topic_id = $1 AND posts.post_type NOT IN ($2, $3)",
    )
    .bind(topic_id)
    .bind(crate::posting::post_types::SMALL_ACTION)
    .bind(crate::posting::post_types::WHISPER)
    .fetch_all(&mut *conn)
    .await?;
    for setting in ["enable_sort", "enable_indexscan", "enable_bitmapscan"] {
        sqlx::query(&format!("SET LOCAL {setting} TO DEFAULT"))
            .execute(&mut *conn)
            .await?;
    }
    Ok(ids.into_iter().flatten().collect())
}

/// `update_users_post_count`: each poster's count of the topic's regular
/// replies (not hidden) added or taken away.
async fn users_post_count(
    conn: &mut PgConnection,
    topic_id: i32,
    sign: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE user_stats SET post_count = post_count + $2 * x.count \
         FROM (SELECT us.user_id, COUNT(*) AS count FROM user_stats us \
               INNER JOIN posts ON posts.topic_id = $1 AND posts.user_id = us.user_id \
               WHERE posts.post_number > 1 AND NOT posts.hidden AND posts.post_type = $3 \
               GROUP BY us.user_id) x \
         WHERE x.user_id = user_stats.user_id",
    )
    .bind(topic_id)
    .bind(sign)
    .bind(crate::posting::post_types::REGULAR)
    .execute(conn)
    .await?;
    Ok(())
}

/// `update_tag_counters(delta, include_public:)`
async fn tag_counters(
    conn: &mut PgConnection,
    topic_id: i32,
    delta: i32,
    include_public: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE tags SET staff_topic_count = COALESCE(staff_topic_count, 0) + $2, \
                pm_topic_count = COALESCE(pm_topic_count, 0) - $2, \
                public_topic_count = COALESCE(public_topic_count, 0) + CASE WHEN $3 THEN $2 ELSE 0 END \
         WHERE id IN (SELECT tag_id FROM topic_tags WHERE topic_id = $1)",
    )
    .bind(topic_id)
    .bind(delta)
    .bind(include_public)
    .execute(conn)
    .await?;
    Ok(())
}

/// `watch_topic`: the owner watches as its creator, the other allowed
/// users (humans) as if they chose to.
async fn watch_topic(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
) -> Result<(), AppError> {
    let owner: Option<i32> = sqlx::query_scalar("SELECT user_id FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    let watch = |reason| TopicUserAttr::NotificationLevel(notification_levels::WATCHING, reason);
    if let Some(owner) = owner {
        change_topic_user(
            ctx.bus,
            &mut *conn,
            owner,
            topic_id,
            &[watch(notification_reasons::CREATED_TOPIC)],
        )
        .await?;
    }
    let allowed: Vec<i32> = sqlx::query_scalar(
        "SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 ORDER BY id",
    )
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?;
    for user_id in allowed {
        if user_id < 0 || Some(user_id) == owner {
            continue;
        }
        change_topic_user(
            ctx.bus,
            &mut *conn,
            user_id,
            topic_id,
            &[watch(notification_reasons::USER_CHANGED)],
        )
        .await?;
    }
    Ok(())
}

/// `add_small_action` from `DB.after_commit`: the topic's small action by
/// the converting user, in a transaction of its own.
async fn add_small_action(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    user: &Guardian,
    topic_id: i32,
    action_code: &str,
) -> Result<(), AppError> {
    let user_id = user
        .user_id()
        .ok_or(Unsupported("converting a topic anonymously"))?;
    let mut tx = pool.begin().await?;
    let action = SmallAction {
        user_id,
        topic_id,
        action_code,
        bump: false,
        who: None,
    };
    if action_code == "private_topic" {
        small_action::add_to_new_message(&mut tx, ctx, &action).await?;
    } else {
        small_action::add(&mut tx, ctx, &action).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// `topic.relative_url`
async fn relative_url(pool: &PgPool, ctx: &Ctx<'_>, topic_id: i32) -> Result<String, AppError> {
    let slug: Option<String> = sqlx::query_scalar("SELECT slug FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(pool)
        .await?;
    let base = ctx.config.globals.relative_url_root();
    Ok(match slug.filter(|s| !s.is_empty()) {
        Some(slug) => format!("{base}/t/{slug}/{topic_id}"),
        None => format!("{base}/t/{topic_id}"),
    })
}
