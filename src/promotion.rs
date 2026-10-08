//! Port of `Promotion` (lib/promotion.rb), `TrustLevel.calculate` and
//! `Group.user_trust_level_change!`: a user's trust level changed by
//! staff or recalculated, with its log, the recooked bio and the trust
//! level groups.
//!
//! Not ported: `BadgeGranter.queue_badge_grant` (a Redis queue the
//! badge jobs drain; badges are milestone 6), and refused: web hooks for
//! `user_promoted`/`user_added_to_group`, trust level groups with
//! notification defaults, titles, flair or granted levels.

use chrono::NaiveDateTime;
use serde_json::json;
use sqlx::PgConnection;

use crate::i18n::I18n;
use crate::pretty_text::Host;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// UserHistory.actions
const CHANGE_TRUST_LEVEL: i32 = 2;
const AUTO_TRUST_LEVEL_CHANGE: i32 = 15;
const LOCK_TRUST_LEVEL: i32 = 41;
const UNLOCK_TRUST_LEVEL: i32 = 42;
/// Post.types[:small_action]
const SMALL_ACTION: i32 = 3;
/// Group::AUTO_GROUPS[:trust_level_0] .. [:trust_level_4]
const TRUST_GROUP_IDS: [i32; 5] = [10, 11, 12, 13, 14];
/// Badge::BasicUser
const BASIC_USER_BADGE: i32 = 1;
const SYSTEM_USER_ID: i32 = -1;

pub struct Ctx<'a> {
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    pub host: &'a Host,
}

#[derive(sqlx::FromRow)]
struct UserRow {
    name: Option<String>,
    username_lower: String,
    admin: bool,
    moderator: bool,
    active: bool,
    trust_level: i32,
    manual_locked_trust_level: Option<i32>,
}

async fn load(conn: &mut PgConnection, user_id: i32) -> Result<UserRow, sqlx::Error> {
    sqlx::query_as(
        "SELECT name, username_lower, admin, moderator, active, trust_level, manual_locked_trust_level \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await
}

/// `Promotion.tl1_met?`, `tl2_met?` and `tl3_met?`; None for other
/// levels (no such method).
pub async fn tl_met(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user_id: i32,
    level: i32,
) -> Result<Option<bool>, AppError> {
    let tier = match level {
        1 => "tl1",
        2 => "tl2",
        3 => {
            let (met, _) = crate::admin_user_show::tl3_met_lost(conn, s, user_id).await?;
            return Ok(Some(met));
        }
        _ => return Ok(None),
    };
    #[derive(sqlx::FromRow)]
    struct Stat {
        topics_entered: i32,
        posts_read_count: i32,
        time_read: i32,
        days_visited: i32,
        likes_received: i32,
        likes_given: i32,
        created_at: NaiveDateTime,
    }
    let st: Stat = sqlx::query_as(
        "SELECT s.topics_entered, s.posts_read_count, s.time_read, s.days_visited, s.likes_received, \
                s.likes_given, u.created_at \
         FROM user_stats s JOIN users u ON u.id = s.user_id WHERE s.user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    let req = |name: &str| s.get(&format!("{tier}_requires_{name}")).map(|v| v.to_i());
    let mins = req("time_spent_mins")?;
    let age_mins = (crate::clock::now_naive() - st.created_at).num_milliseconds() as f64 / 60_000.0;
    if i64::from(st.topics_entered) < req("topics_entered")?
        || i64::from(st.posts_read_count) < req("read_posts")?
        || i64::from(st.time_read / 60) < mins
        || age_mins < mins as f64
    {
        return Ok(Some(false));
    }
    if level == 1 {
        return Ok(Some(true));
    }
    if i64::from(st.days_visited) < req("days_visited")?
        || i64::from(st.likes_received) < req("likes_received")?
        || i64::from(st.likes_given) < req("likes_given")?
    {
        return Ok(Some(false));
    }
    // UserStat#calc_topic_reply_count!
    let replied: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT posts.topic_id) FROM posts INNER JOIN topics ON topics.id = posts.topic_id \
         WHERE posts.user_id = $1 AND topics.user_id <> posts.user_id \
         AND posts.deleted_at IS NULL AND topics.deleted_at IS NULL \
         AND topics.archetype <> 'private_message' AND posts.post_type <> $2",
    )
    .bind(user_id)
    .bind(SMALL_ACTION)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Some(replied >= req("topic_reply_count")?))
}

/// `Promotion.tl3_lost?`
pub async fn tl3_lost(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user_id: i32,
) -> Result<bool, AppError> {
    Ok(crate::admin_user_show::tl3_met_lost(conn, s, user_id)
        .await?
        .1)
}

/// `user.manual_locked_trust_level = lock; user.save`: the column and
/// updated_at when it changes, then User's after_save callbacks.
pub async fn save_lock(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user_id: i32,
    lock: Option<i32>,
) -> Result<(), AppError> {
    let u = load(conn, user_id).await?;
    if u.manual_locked_trust_level == lock {
        return Ok(());
    }
    sqlx::query(
        "UPDATE users SET manual_locked_trust_level = $2, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(user_id)
    .bind(lock)
    .execute(&mut *conn)
    .await?;
    let active_admin = u.admin && u.active;
    crate::user_updater::after_save(
        conn,
        s,
        user_id,
        active_admin,
        &u.username_lower,
        u.name.as_deref(),
    )
    .await
}

/// `Promotion#change_trust_level!(level, log_action_for:)`. The inner
/// error is Discourse::InvalidAccess's message: a demotion below levels
/// the user still meets, unless the level is locked.
pub async fn change_trust_level(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    user_id: i32,
    new_level: i32,
    log_action_for: Option<i32>,
) -> Result<Result<(), String>, AppError> {
    let s = cx.settings;
    if !(0..=4).contains(&new_level) {
        return Err(Unsupported("an invalid trust level (a RuntimeError)").into());
    }
    let u = load(conn, user_id).await?;
    let old_level = u.trust_level;
    if new_level < old_level
        && u.manual_locked_trust_level.is_none()
        && tl_met(conn, s, user_id, new_level + 1).await? == Some(true)
    {
        let (new, old) = (new_level.to_string(), old_level.to_string());
        let name = u.name.as_deref().unwrap_or_default();
        let message = cx
            .i18n
            .t_with(
                "trust_levels.change_failed_explanation",
                &[
                    ("user_name", name),
                    ("new_trust_level", &new),
                    ("current_trust_level", &old),
                ],
            )
            .unwrap_or_default();
        return Ok(Err(message));
    }
    let web_hooks: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM web_hooks WHERE active)")
            .fetch_one(&mut *conn)
            .await?;
    if web_hooks {
        return Err(Unsupported("web hooks for user_promoted").into());
    }

    let (action, acting) = match log_action_for {
        Some(admin) => (CHANGE_TRUST_LEVEL, Some(admin)),
        None => (AUTO_TRUST_LEVEL_CHANGE, None),
    };
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, previous_value, new_value, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(action)
    .bind(acting)
    .bind(user_id)
    .bind(old_level.to_string())
    .bind(new_level.to_string())
    .execute(&mut *conn)
    .await?;
    if new_level != old_level {
        sqlx::query(
            "UPDATE users SET trust_level = $2, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(user_id)
        .bind(new_level)
        .execute(&mut *conn)
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
    }
    recook_bio(conn, cx, user_id, new_level, u.admin || u.moderator).await?;
    user_trust_level_change(conn, user_id, new_level).await?;
    Ok(Ok(()))
}

/// `user_profile.recook_bio; user_profile.save!`: bio_raw marked changed,
/// so the bio is cooked again (links follow at TL3) and its after_save
/// hooks run.
async fn recook_bio(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    user_id: i32,
    trust_level: i32,
    staff: bool,
) -> Result<(), AppError> {
    let s = cx.settings;
    #[derive(sqlx::FromRow)]
    struct Profile {
        bio_raw: Option<String>,
        bio_cooked: Option<String>,
        location: Option<String>,
        profile_background_upload_id: Option<i32>,
        card_background_upload_id: Option<i32>,
    }
    let p: Profile = sqlx::query_as(
        "SELECT bio_raw, bio_cooked, location, profile_background_upload_id, card_background_upload_id \
         FROM user_profiles WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if p.location.as_deref().is_some_and(|l| !l.trim().is_empty()) {
        let watched: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM watched_words)")
            .fetch_one(&mut *conn)
            .await?;
        if watched {
            return Err(Unsupported("watched words in a profile location").into());
        }
    }
    let raw = p.bio_raw.as_deref().filter(|b| !b.trim().is_empty());
    match raw {
        Some(raw) => {
            if raw.contains("upload://") || raw.contains("/uploads/") {
                return Err(Unsupported("uploads in the bio (UploadReference)").into());
            }
            // has_trust_level?(TL3): staff count too.
            let tl3 = staff || trust_level >= 3;
            let opts = crate::pretty_text::MarkdownOptions {
                omit_nofollow: tl3 && !s.get("tl3_links_no_follow")?.truthy(),
                ..Default::default()
            };
            let cooked = crate::pretty_text::cook(cx.host, raw, &opts).await?;
            sqlx::query(
                "UPDATE user_profiles SET bio_cooked = $2, bio_cooked_version = 1 WHERE user_id = $1",
            )
            .bind(user_id)
            .bind(cooked)
            .execute(&mut *conn)
            .await?;
        }
        None if p.bio_cooked.is_some() => {
            sqlx::query("UPDATE user_profiles SET bio_cooked = NULL WHERE user_id = $1")
                .bind(user_id)
                .execute(&mut *conn)
                .await?;
        }
        None => {}
    }
    // pull_hotlinked_image
    crate::jobs::enqueue_in(
        &mut *conn,
        s.get("editing_grace_period")?.to_i(),
        "pull_user_profile_hotlinked_images",
        json!({ "user_id": user_id }),
    )
    .await?;
    // UploadReference.ensure_exist! for the backgrounds and the bio's
    // uploads (none).
    if p.profile_background_upload_id.is_some() || p.card_background_upload_id.is_some() {
        return Err(Unsupported("upload references of profile backgrounds").into());
    }
    sqlx::query(
        "DELETE FROM upload_references WHERE target_type = 'UserProfile' AND target_id = $1",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Group.user_trust_level_change!(user_id, trust_level)`: out of the
/// trust level groups above the level (a bare delete: user_count stays),
/// into those up to it (GroupUser create and its GroupManager side
/// effects).
pub async fn user_trust_level_change(
    conn: &mut PgConnection,
    user_id: i32,
    trust_level: i32,
) -> Result<(), AppError> {
    let (desired, undesired): (Vec<i32>, Vec<i32>) = TRUST_GROUP_IDS
        .iter()
        .partition(|&&id| id == TRUST_GROUP_IDS[0] || trust_level + 10 >= id);
    sqlx::query("DELETE FROM group_users WHERE group_id = ANY($1) AND user_id = $2")
        .bind(&undesired)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    for id in desired {
        #[derive(sqlx::FromRow)]
        struct Group {
            default_notification_level: Option<i32>,
            title: Option<String>,
            primary_group: bool,
            grant_trust_level: Option<i32>,
            defaults: bool,
        }
        let group: Option<Group> = sqlx::query_as(
            "SELECT default_notification_level, title, primary_group, grant_trust_level, \
                    EXISTS (SELECT 1 FROM group_category_notification_defaults WHERE group_id = g.id) \
                    OR EXISTS (SELECT 1 FROM group_tag_notification_defaults WHERE group_id = g.id) AS defaults \
             FROM groups g WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some(group) = group else {
            return Err(
                Unsupported("a missing trust level group (refresh_automatic_group!)").into(),
            );
        };
        let member: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM group_users WHERE group_id = $1 AND user_id = $2)",
        )
        .bind(id)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if member {
            continue;
        }
        if group.title.as_deref().is_some_and(|t| !t.is_empty())
            || group.primary_group
            || group.grant_trust_level.is_some_and(|l| l != 0)
            || group.defaults
        {
            return Err(Unsupported(
                "trust level groups with titles, flair, granted levels or notification defaults",
            )
            .into());
        }
        sqlx::query(
            "INSERT INTO group_users (group_id, user_id, notification_level, created_at, updated_at) \
             VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
        )
        .bind(id)
        .bind(user_id)
        .bind(group.default_notification_level.unwrap_or(3))
        .execute(&mut *conn)
        .await?;
        // increase_group_user_count: trust level groups hide bots.
        if user_id > 0 {
            sqlx::query("UPDATE groups SET user_count = user_count + 1 WHERE id = $1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

/// `TrustLevel.calculate(user)`
async fn calculate(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user_id: i32,
    lock: Option<i32>,
) -> Result<i32, AppError> {
    if let Some(lock) = lock {
        return Ok(lock);
    }
    let granted: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(g.grant_trust_level) FROM group_users gu JOIN groups g ON g.id = gu.group_id \
         WHERE gu.user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    let redeemed: Option<bool> = sqlx::query_scalar(
        "SELECT redeemed_at IS NOT NULL FROM invited_users WHERE user_id = $1 ORDER BY id LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let invitee = if redeemed == Some(true) {
        s.get("default_invitee_trust_level")?.to_i() as i32
    } else {
        0
    };
    let default = s.get("default_trust_level")?.to_i() as i32;
    Ok(granted.unwrap_or(0).max(invitee).max(default))
}

/// `Promotion.recalculate(user, performed_by)`
pub async fn recalculate(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    user_id: i32,
    performed_by: Option<i32>,
) -> Result<(), AppError> {
    let s = cx.settings;
    let u = load(conn, user_id).await?;
    let mut granted = calculate(conn, s, user_id, u.manual_locked_trust_level).await?;
    // can_downgrade_trust_level?
    if granted < u.trust_level && tl_met(conn, s, user_id, u.trust_level).await? == Some(true) {
        granted = u.trust_level;
    }
    // update_column: no callbacks, no updated_at.
    sqlx::query("UPDATE users SET trust_level = $2 WHERE id = $1")
        .bind(user_id)
        .bind(granted)
        .execute(&mut *conn)
        .await?;
    if u.manual_locked_trust_level.is_some() {
        return Ok(());
    }
    let invalid = |m: String| AppError::from(std::io::Error::other(m));
    // review_tl0, review_tl1, review_tl2
    if granted < 1 && tl_met(conn, s, user_id, 1).await? == Some(true) {
        change_trust_level(conn, cx, user_id, 1, None)
            .await?
            .map_err(invalid)?;
        let welcome: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM badges WHERE id = $1 AND enabled) \
             AND NOT EXISTS (SELECT 1 FROM user_badges WHERE badge_id = $1 AND user_id = $2)",
        )
        .bind(BASIC_USER_BADGE)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if welcome && s.get("send_tl1_welcome_message")?.truthy() {
            enqueue_message(conn, user_id, "welcome_tl1_user").await?;
        }
    }
    if granted < 2 && tl_met(conn, s, user_id, 2).await? == Some(true) {
        change_trust_level(conn, cx, user_id, 2, None)
            .await?
            .map_err(invalid)?;
        if s.get("send_tl2_promotion_message")?.truthy() {
            enqueue_message(conn, user_id, "tl2_promotion_message").await?;
        }
    }
    if granted < 3 && tl_met(conn, s, user_id, 3).await? == Some(true) {
        change_trust_level(conn, cx, user_id, 3, None)
            .await?
            .map_err(invalid)?;
    }
    let level = load(conn, user_id).await?.trust_level;
    user_trust_level_change(conn, user_id, level).await?;
    if level == 3 && tl3_lost(conn, s, user_id).await? {
        let by = performed_by.unwrap_or(SYSTEM_USER_ID);
        change_trust_level(conn, cx, user_id, 2, Some(by))
            .await?
            .map_err(invalid)?;
    }
    Ok(())
}

async fn enqueue_message(
    conn: &mut PgConnection,
    user_id: i32,
    message_type: &str,
) -> Result<(), sqlx::Error> {
    crate::jobs::enqueue(
        conn,
        "send_system_message",
        json!({ "user_id": user_id, "message_type": message_type }),
    )
    .await
}

/// `StaffActionLogger#log_lock_trust_level`
pub async fn log_lock(
    conn: &mut PgConnection,
    acting_user_id: i32,
    user_id: i32,
    locked: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(if locked { LOCK_TRUST_LEVEL } else { UNLOCK_TRUST_LEVEL })
    .bind(acting_user_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
