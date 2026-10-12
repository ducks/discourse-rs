//! The scheduled jobs of moderation and trust: TL3 promotions and
//! demotions, the review queue's priorities and expired flags, and the
//! deletion of posts their authors deleted and of old hidden posts.

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Jobs::Tl3Promotions`, daily (Rails: at 4am).
pub const TL3_PROMOTIONS: &str = "tl3_promotions";
/// `Jobs::ReviewablePriorities`, daily.
pub const REVIEWABLE_PRIORITIES: &str = "reviewable_priorities";
/// `Jobs::AutoQueueHandler`, daily.
pub const AUTO_QUEUE_HANDLER: &str = "auto_queue_handler";
/// `Jobs::DestroyOldDeletionStubs`, every 30 minutes.
pub const DESTROY_OLD_DELETION_STUBS: &str = "destroy_old_deletion_stubs";
/// `Jobs::DestroyOldHiddenPosts`, daily.
pub const DESTROY_OLD_HIDDEN_POSTS: &str = "destroy_old_hidden_posts";

/// `UserHistory.actions[:auto_trust_level_change]`
const AUTO_TRUST_LEVEL_CHANGE: i32 = 15;
/// `Reviewable.statuses`
const PENDING: i32 = 0;
const APPROVED: i32 = 1;

async fn settings(state: &AppState) -> Result<SiteSettings, AppError> {
    let mut conn = state.pool.acquire().await?;
    Ok(SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?)
}

fn host_ctx<'a>(
    state: &'a AppState,
    settings: &'a SiteSettings,
    host: &'a crate::pretty_text::Host,
) -> crate::posting::Ctx<'a> {
    crate::posting::Ctx {
        host,
        settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    }
}

/// `Jobs::Tl3Promotions`: (unless everyone starts at TL3) TL3 members
/// whose level isn't locked or granted by a group, past their grace
/// period, demoted to TL2 when they lost TL3's requirements; then TL2
/// members (not just demoted) meeting the totals reviewed for TL3.
pub async fn tl3_promotions(state: &AppState) -> Result<(), AppError> {
    let s = settings(state).await?;
    let mut conn = state.pool.acquire().await?;
    let promotion_cx = crate::promotion::Ctx {
        settings: &s,
        i18n: &state.i18n,
        host: &crate::pretty_text::Host::from_state(state),
    };
    // Discourse::InvalidAccess from change_trust_level!
    let invalid = |m: String| AppError::from(std::io::Error::other(m));
    let mut demoted: Vec<i32> = Vec::new();
    if s.get("default_trust_level")?.to_i() < 3 {
        let candidates: Vec<i32> = sqlx::query_scalar(
            "SELECT users.id FROM users \
             LEFT JOIN (SELECT gu.user_id, MAX(g.grant_trust_level) AS group_granted_trust_level \
                        FROM groups g, group_users gu WHERE g.id = gu.group_id GROUP BY gu.user_id) tl \
               ON users.id = tl.user_id \
             WHERE users.id > 0 AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id) \
               AND users.trust_level = 3 AND users.manual_locked_trust_level IS NULL \
               AND (group_granted_trust_level IS NULL OR group_granted_trust_level < 3) \
             ORDER BY users.id",
        )
        .fetch_all(&mut *conn)
        .await?;
        let grace_days = s.get("tl3_promotion_min_duration")?.to_i();
        for user_id in candidates {
            // on_tl3_grace_period?: promoted from TL2 within the duration
            // (always, for a duration reaching back before 2013).
            let grace: bool = sqlx::query_scalar(
                "SELECT (now() - make_interval(days => $2))::date < '2013-01-01' OR EXISTS ( \
                   SELECT 1 FROM user_histories WHERE target_user_id = $1 AND action = $3 \
                     AND created_at >= now() - make_interval(days => $2) \
                     AND previous_value = '2' AND new_value = '3')",
            )
            .bind(user_id)
            .bind(grace_days as i32)
            .bind(AUTO_TRUST_LEVEL_CHANGE)
            .fetch_one(&mut *conn)
            .await?;
            if grace {
                continue;
            }
            if crate::promotion::tl3_lost(&mut conn, &s, user_id).await? {
                demoted.push(user_id);
                crate::promotion::change_trust_level(&mut conn, &promotion_cx, user_id, 2, None)
                    .await?
                    .map_err(invalid)?;
            }
        }
    }
    let promotable: Vec<i32> = sqlx::query_scalar(
        "SELECT users.id FROM users JOIN user_stats ON user_stats.user_id = users.id \
         WHERE users.id > 0 AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id) \
           AND (users.suspended_till IS NULL OR users.suspended_till <= now()) \
           AND users.trust_level = 2 AND users.manual_locked_trust_level IS NULL \
           AND users.id <> ALL($1) \
           AND user_stats.days_visited >= $2 AND user_stats.topics_entered >= $3 \
           AND user_stats.posts_read_count >= $4 AND user_stats.likes_given >= $5 \
           AND user_stats.likes_received >= $6 \
         ORDER BY users.id",
    )
    .bind(&demoted)
    .bind(s.get("tl3_requires_days_visited")?.to_i() as i32)
    .bind(s.get("tl3_requires_topics_viewed_all_time")?.to_i() as i32)
    .bind(s.get("tl3_requires_posts_read_all_time")?.to_i() as i32)
    .bind(s.get("tl3_requires_likes_given")?.to_i() as i32)
    .bind(s.get("tl3_requires_likes_received")?.to_i() as i32)
    .fetch_all(&mut *conn)
    .await?;
    for user_id in promotable {
        // Promotion#review_tl2
        if crate::promotion::tl_met(&mut conn, &s, user_id, 3).await? == Some(true) {
            crate::promotion::change_trust_level(&mut conn, &promotion_cx, user_id, 3, None)
                .await?
                .map_err(invalid)?;
        }
    }
    Ok(())
}

/// Ruby's `Float#to_s`
fn ruby_float(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e16 {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

/// `PluginStore.set("reviewables", "priority_#{id}", value)`
async fn set_priority(
    conn: &mut sqlx::PgConnection,
    id: i32,
    type_name: &str,
    value: &str,
) -> Result<(), sqlx::Error> {
    let key = format!("priority_{id}");
    let updated = sqlx::query(
        "UPDATE plugin_store_rows SET type_name = $2, value = $3 WHERE plugin_name = 'reviewables' AND key = $1",
    )
    .bind(&key)
    .bind(type_name)
    .bind(value)
    .execute(&mut *conn)
    .await?;
    if updated.rows_affected() == 0 {
        sqlx::query(
            "INSERT INTO plugin_store_rows (plugin_name, key, type_name, value) VALUES ('reviewables', $1, $2, $3)",
        )
        .bind(&key)
        .bind(type_name)
        .bind(value)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `Jobs::ReviewablePriorities`: with at least 15 approved reviewables
/// over the low threshold, the medium and high priorities set to the
/// median and 85th percentile score of those flagged at least twice.
pub async fn reviewable_priorities(state: &AppState) -> Result<(), AppError> {
    let s = settings(state).await?;
    let mut conn = state.pool.acquire().await?;
    let low = s.get("reviewable_low_priority_threshold")?.to_i();
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM reviewables WHERE status = $1 AND score > $2")
            .bind(APPROVED)
            .bind(low as f64)
            .fetch_one(&mut *conn)
            .await?;
    if count < 15 {
        return Ok(());
    }
    let (medium, high): (f64, f64) = sqlx::query_as(
        "SELECT COALESCE(PERCENTILE_DISC(0.5) WITHIN GROUP (ORDER BY score), 0.0) AS medium, \
                COALESCE(PERCENTILE_DISC(0.85) WITHIN GROUP (ORDER BY score), 0.0) AS high \
         FROM (SELECT r.score FROM reviewables AS r \
               INNER JOIN reviewable_scores AS rs ON rs.reviewable_id = r.id \
               WHERE r.score > $1 AND r.status = 1 GROUP BY r.id HAVING COUNT(*) >= 2) AS x",
    )
    .bind(low as f64)
    .fetch_one(&mut *conn)
    .await?;
    // Reviewable.priorities: low 0 (the threshold, an Integer), medium 5,
    // high 10 (Floats).
    set_priority(&mut conn, 0, "Integer", &low.to_string()).await?;
    set_priority(&mut conn, 5, "Float", &ruby_float(medium)).await?;
    set_priority(&mut conn, 10, "Float", &ruby_float(high)).await?;
    Ok(())
}

/// `Jobs::AutoQueueHandler`: pending reviewables older than
/// auto_handle_queued_age days: flags ignored (expired). Queued posts
/// rejected and users deleted are not ported.
pub async fn auto_queue_handler(state: &AppState) -> Result<(), AppError> {
    let s = settings(state).await?;
    let days = s.get("auto_handle_queued_age")?.to_i();
    if days == 0 {
        return Ok(());
    }
    let pending: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, type FROM reviewables WHERE status = $1 \
           AND created_at < now() - make_interval(days => $2) ORDER BY id",
    )
    .bind(PENDING)
    .bind(days as i32)
    .fetch_all(&state.pool)
    .await?;
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = host_ctx(state, &s, &host);
    for (id, kind) in pending {
        match kind.as_str() {
            "ReviewableFlaggedPost" => {
                let mut tx = state.pool.begin().await?;
                crate::review::expire_flags(&mut tx, &ctx, id).await?;
                tx.commit().await?;
            }
            "ReviewableQueuedPost" => {
                return Err(Unsupported("rejecting queued posts").into());
            }
            "ReviewableUser" => return Err(Unsupported("deleting users").into()),
            _ => {}
        }
    }
    Ok(())
}

/// `PostDestroyer.destroy_stubs`: posts their authors deleted, untouched
/// for delete_removed_posts_after hours, outside deleted topics and
/// without an active flag, destroyed by the system user.
pub async fn destroy_old_deletion_stubs(state: &AppState) -> Result<(), AppError> {
    let s = settings(state).await?;
    let mut conn = state.pool.acquire().await?;
    let types = crate::post_actions::ActionTypes::load(&mut conn).await?;
    let notify_flags: Vec<i32> = types
        .types
        .iter()
        .filter(|(_, id)| types.is_notify_flag(*id))
        .map(|(_, id)| *id as i32)
        .collect();
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE deleted_at IS NULL AND user_deleted = true \
           AND NOT EXISTS (SELECT 1 FROM topics t WHERE t.deleted_at IS NOT NULL AND t.id = posts.topic_id) \
           AND updated_at < now() - make_interval(hours => $1) \
           AND NOT EXISTS (SELECT 1 FROM post_actions pa WHERE pa.post_id = posts.id \
                             AND pa.deleted_at IS NULL AND pa.deferred_at IS NULL \
                             AND pa.post_action_type_id = ANY($2)) \
         ORDER BY id",
    )
    .bind(s.get("delete_removed_posts_after")?.to_i() as i32)
    .bind(&notify_flags)
    .fetch_all(&mut *conn)
    .await?;
    drop(conn);
    let context = state
        .i18n
        .t("remove_posts_deleted_by_author")
        .unwrap_or("Deleted by author")
        .to_string();
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = host_ctx(state, &s, &host);
    for id in ids {
        crate::post_destroyer::destroy_as_system(&state.pool, &ctx, id, &context).await?;
    }
    Ok(())
}

/// `PostDestroyer.destroy_old_hidden_posts` (when delete_old_hidden_posts):
/// posts hidden over 30 days, destroyed by the system user.
pub async fn destroy_old_hidden_posts(state: &AppState) -> Result<(), AppError> {
    let s = settings(state).await?;
    if !s.get("delete_old_hidden_posts")?.truthy() {
        return Ok(());
    }
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE deleted_at IS NULL AND hidden = true \
           AND hidden_at < now() - interval '30 days' ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = host_ctx(state, &s, &host);
    for id in ids {
        crate::post_destroyer::destroy_as_system(
            &state.pool,
            &ctx,
            id,
            "Automatically destroyed hidden posts",
        )
        .await?;
    }
    Ok(())
}
