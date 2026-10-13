//! `Jobs::PeriodicalUpdates` (every 15 minutes): featured topics, the
//! scores of recent topics, the old posts, bios and tag descriptions
//! cooked again, the cut-off for new topics, and a category's auto bump.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use crate::site_settings::{SiteSettings, Value};
use crate::{AppError, AppState, Unsupported};

/// `Jobs::PeriodicalUpdates`, every 15 minutes.
pub const PERIODICAL_UPDATES: &str = "periodical_updates";

/// `should_update_long_topics?`'s count of runs in this process.
static CALLS: AtomicU64 = AtomicU64::new(0);

/// `category-featured-topic:next-category-id`: where the next batch of
/// categories starts. Rails keeps it in Redis for a day; the port's one
/// worker keeps it in memory.
static NEXT_CATEGORY_ID: AtomicI64 = AtomicI64::new(0);

/// `CategoryFeaturedTopic.feature_topics(batched: true)`'s batch.
const FEATURE_BATCH: i64 = 100;

/// `Jobs::PeriodicalUpdates#execute`, step by step.
pub async fn periodical_updates(state: &AppState) -> Result<(), AppError> {
    // should_update_long_topics?: every 24th run, the first included.
    let calls = CALLS.fetch_add(1, Ordering::SeqCst) + 1;
    let long_topics = calls % 24 == 1;
    let bumped_since = crate::clock::now_naive() - chrono::Duration::days(1);

    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;

    // CategoryFeaturedTopic.feature_topics(batched: true): each category's
    // list rewritten unless it came out the same.
    let rows: Vec<(i32, i32)> =
        sqlx::query_as("SELECT topic_id, category_id FROM category_featured_topics ORDER BY rank")
            .fetch_all(&mut *conn)
            .await?;
    let mut current: HashMap<i32, Vec<i32>> = HashMap::new();
    for (topic_id, category_id) in rows {
        current.entry(category_id).or_default().push(topic_id);
    }
    let next = NEXT_CATEGORY_ID.load(Ordering::SeqCst);
    let batch: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM categories WHERE id >= $1 ORDER BY id ASC LIMIT $2")
            .bind(next as i32)
            .bind(FEATURE_BATCH)
            .fetch_all(&mut *conn)
            .await?;
    let following = match batch.last() {
        Some(last) if batch.len() as i64 == FEATURE_BATCH => {
            sqlx::query_scalar::<_, i32>(
                "SELECT id FROM categories WHERE id > $1 ORDER BY id LIMIT 1",
            )
            .bind(last)
            .fetch_optional(&mut *conn)
            .await?
        }
        _ => None,
    };
    NEXT_CATEGORY_ID.store(following.map_or(0, i64::from), Ordering::SeqCst);
    for category_id in batch {
        let existing = current.remove(&category_id).unwrap_or_default();
        crate::posting::create::feature_topics_for(&mut conn, &s, category_id, Some(&existing))
            .await?;
    }

    // ScoreCalculator over topics bumped in the last day (short ones but
    // every 24th run).
    super::scores::calculate(
        &mut conn,
        &s,
        super::scores::Filters {
            bumped_since: Some(bumped_since),
            max_topic_length: (!long_topics).then_some(500),
        },
    )
    .await?;

    // Post.rebake_old, unless gravatars are still to be downloaded.
    let gravatars_pending = s.get("automatically_download_gravatars")?.truthy()
        && sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM user_avatars JOIN users ON users.id = user_avatars.user_id \
             JOIN user_emails ON user_emails.user_id = users.id \
             WHERE user_emails.primary = TRUE AND user_avatars.last_gravatar_download_attempt IS NULL)",
        )
        .fetch_one(&mut *conn)
        .await?;
    if !gravatars_pending {
        rebake_old_posts(state, &s, &mut conn).await?;
    }

    // UserProfile.rebake_old(250)
    let host = crate::pretty_text::Host::from_state(state);
    #[derive(sqlx::FromRow)]
    struct Profile {
        user_id: i32,
        bio_raw: Option<String>,
        trust_level: i32,
        staff_or_staged: bool,
    }
    let profiles: Vec<Profile> = sqlx::query_as(
        "SELECT p.user_id, p.bio_raw, u.trust_level, (u.admin OR u.moderator OR u.staged) AS staff_or_staged \
         FROM user_profiles p JOIN users u ON u.id = p.user_id \
         WHERE p.bio_cooked_version IS NULL OR p.bio_cooked_version < 1 LIMIT 250",
    )
    .fetch_all(&mut *conn)
    .await?;
    for p in profiles {
        // UserProfile#cooked
        let cooked = match p.bio_raw.as_deref().filter(|b| !crate::ruby::is_blank(b)) {
            Some(raw) => {
                if raw.contains("upload://") || raw.contains("/uploads/") {
                    return Err(Unsupported("uploads in the bio (UploadReference)").into());
                }
                let tl3 = p.staff_or_staged || p.trust_level >= 3;
                let opts = crate::pretty_text::MarkdownOptions {
                    omit_nofollow: tl3 && !s.get("tl3_links_no_follow")?.truthy(),
                    ..Default::default()
                };
                Some(crate::pretty_text::cook(&host, raw, &opts).await?)
            }
            None => None,
        };
        sqlx::query(
            "UPDATE user_profiles SET bio_cooked = $2, bio_cooked_version = 1 WHERE user_id = $1",
        )
        .bind(p.user_id)
        .bind(cooked)
        .execute(&mut *conn)
        .await?;
    }

    // Tag.rebake_old(250) and TagLocalization.rebake_old(250): a
    // description is cooked without oneboxes, which is not ported; none
    // is NULL.
    for table in ["tags", "tag_localizations"] {
        let with_text: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS (SELECT 1 FROM {table} WHERE (description_cooked_version IS NULL \
               OR description_cooked_version < 1) AND COALESCE(TRIM(description), '') <> '')"
        ))
        .fetch_one(&mut *conn)
        .await?;
        if with_text {
            return Err(Unsupported("cooking tag descriptions (features: onebox off)").into());
        }
        sqlx::query(&format!(
            "UPDATE {table} SET description_cooked = NULL, description_cooked_version = 1 \
             WHERE id IN (SELECT id FROM {table} WHERE description_cooked_version IS NULL \
               OR description_cooked_version < 1 LIMIT 250)"
        ))
        .execute(&mut *conn)
        .await?;
    }

    // min_new_topics_time: the creation of the max_new_topics'th newest
    // topic, as unix seconds (the plain setter: hidden, unlogged).
    let cutoff: Option<i64> = sqlx::query_scalar(
        "SELECT floor(extract(epoch FROM created_at))::bigint FROM topics WHERE deleted_at IS NULL \
         ORDER BY created_at DESC LIMIT 1 OFFSET $1",
    )
    .bind(s.get("max_new_topics")?.to_i())
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(cutoff) = cutoff {
        let def = state
            .site_setting_defs
            .get("min_new_topics_time")
            .ok_or(Unsupported("min_new_topics_time's definition"))?;
        let current = s.get("min_new_topics_time")?.clone();
        crate::site_setting_update::set_and_log(
            &mut conn,
            &state.bus,
            def,
            &current,
            &Value::Int(cutoff),
            -1,
        )
        .await?;
    }

    // Category.auto_bump_topic!: limited per category per day by a rate
    // limiter, which is not ported; without it every run would bump.
    let auto_bump: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM category_settings WHERE num_auto_bump_daily > 0)",
    )
    .fetch_one(&mut *conn)
    .await?;
    if auto_bump {
        return Err(Unsupported("auto bumping topics (the rate limiter)").into());
    }
    Ok(())
}

/// `Post.rebake_old(rebake_old_posts_count)`: the newest posts baked
/// before the current version, each rebaked. A post that fails is retried
/// on later runs, then marked baked after three attempts (its
/// rebake_attempts custom field); what the port refuses aborts the job.
async fn rebake_old_posts(
    state: &AppState,
    s: &SiteSettings,
    conn: &mut sqlx::PgConnection,
) -> Result<(), AppError> {
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE (baked_version IS NULL OR baked_version < 2) \
           AND posts.deleted_at IS NULL ORDER BY id DESC LIMIT $1",
    )
    .bind(s.get("rebake_old_posts_count")?.to_i())
    .fetch_all(&mut *conn)
    .await?;
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    for id in ids {
        let Err(e) = crate::posting::rebake::rebake(&state.pool, &ctx, id).await else {
            continue;
        };
        if e.is_unsupported() {
            return Err(e);
        }
        tracing::warn!(post_id = id, error = %e, "rebaking a post failed");
        let attempts: Option<String> = sqlx::query_scalar(
            "SELECT value FROM post_custom_fields WHERE post_id = $1 AND name = 'rebake_attempts' ORDER BY id LIMIT 1",
        )
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
        let attempts = attempts.map_or(0, |a| crate::ruby::to_i(&a));
        if attempts > 3 {
            sqlx::query("UPDATE posts SET baked_version = 2 WHERE id = $1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
        } else {
            let updated = sqlx::query(
                "UPDATE post_custom_fields SET value = $2, updated_at = clock_timestamp() \
                 WHERE post_id = $1 AND name = 'rebake_attempts'",
            )
            .bind(id)
            .bind((attempts + 1).to_string())
            .execute(&mut *conn)
            .await?;
            if updated.rows_affected() == 0 {
                sqlx::query(
                    "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
                     VALUES ($1, 'rebake_attempts', $2, clock_timestamp(), clock_timestamp())",
                )
                .bind(id)
                .bind((attempts + 1).to_string())
                .execute(&mut *conn)
                .await?;
            }
        }
    }
    Ok(())
}
