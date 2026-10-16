//! Port of GroupManager (lib/group_manager.rb): adding and removing many
//! users at once with their side effects (titles, primary group and
//! flair, granted trust levels, the group's category and tag
//! notification defaults, the member count), the /categories update or
//! refresh to the members, and the `user_added_to_group` /
//! `user_removed_from_group` events (their web hooks, chat's auto join
//! and auto leave).
//! `add` and `remove` are `Group#add` and `Group#remove` for one user with
//! GroupActionLogger's membership logs, as the admin user routes use them.
//!
//! Refused: a /categories update with categories the member may now see (CategorySerializer is
//! not ported).

use serde_json::json;
use sqlx::PgConnection;

use crate::{AppError, AppState, Unsupported};

/// `GroupHistory.actions`
pub const ADD_USER_TO_GROUP: i32 = 2;
pub const REMOVE_USER_FROM_GROUP: i32 = 3;
/// Group::AUTO_GROUPS ids: automatic groups hide bots from user_count.
const MAX_AUTO_GROUP_ID: i32 = 14;
/// `Group::PUBLISH_CATEGORIES_LIMIT`
const PUBLISH_CATEGORIES_LIMIT: i64 = 10;
/// `NotificationLevels.all[:watching]`, which ranks above watching first
/// post when defaults meet existing levels.
const WATCHING: i32 = 3;

#[derive(sqlx::FromRow)]
pub struct Group {
    pub id: i32,
    pub automatic: bool,
    pub title: Option<String>,
    pub primary_group: bool,
    pub grant_trust_level: Option<i32>,
    pub default_notification_level: i32,
}

pub async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<Group>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, automatic, title, primary_group, grant_trust_level, default_notification_level \
         FROM groups WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(conn)
    .await
}

impl Group {
    fn title(&self) -> Option<&str> {
        self.title.as_deref().filter(|t| !t.is_empty())
    }

    fn granted_level(&self) -> Option<i32> {
        self.grant_trust_level.filter(|l| *l != 0)
    }

    /// `counted_user_ids`: bots left out where the group hides them.
    fn counted(&self, user_ids: &[i32]) -> i32 {
        let hides = !self.automatic || self.id <= MAX_AUTO_GROUP_ID;
        user_ids.iter().filter(|id| !hides || **id > 0).count() as i32
    }
}

/// What the side effects need: the request's state (the bus, chat) and
/// promotion's context (settings, i18n, host).
pub struct Cx<'a> {
    pub state: &'a AppState,
    pub promotion: &'a crate::promotion::Ctx<'a>,
}

impl Cx<'_> {
    fn payloads(&self) -> crate::web_hooks::Payloads<'_> {
        crate::web_hooks::Payloads {
            settings: self.promotion.settings,
            i18n: self.promotion.i18n,
            config: self.promotion.config,
        }
    }
}

/// GroupActionLogger's membership logs (one `GroupHistory.insert_all` for
/// the bulk ones): one row a user.
pub async fn log_many(
    conn: &mut PgConnection,
    group_id: i32,
    acting_user_id: i32,
    user_ids: &[i32],
    action: i32,
) -> Result<(), sqlx::Error> {
    if user_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO group_histories (group_id, acting_user_id, target_user_id, action, subject, \
                                      created_at, updated_at) \
         SELECT $1, $2, u, $4, NULL, now(), now() FROM unnest($3::int[]) WITH ORDINALITY AS t(u, n) \
         ORDER BY n",
    )
    .bind(group_id)
    .bind(acting_user_id)
    .bind(user_ids)
    .bind(action)
    .execute(conn)
    .await?;
    Ok(())
}

async fn log(
    conn: &mut PgConnection,
    group_id: i32,
    acting_user_id: i32,
    user_id: i32,
    action: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO group_histories (group_id, acting_user_id, target_user_id, action, subject, \
                                      created_at, updated_at) \
         VALUES ($1, $2, $3, $4, NULL, clock_timestamp(), clock_timestamp())",
    )
    .bind(group_id)
    .bind(acting_user_id)
    .bind(user_id)
    .bind(action)
    .execute(conn)
    .await?;
    Ok(())
}

/// `group.add(user)` then `log_add_user_to_group`. Already a member: no
/// change, still logged, as the controller logs regardless.
pub async fn add(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    group: &Group,
    user_id: i32,
    acting_user_id: i32,
) -> Result<(), AppError> {
    add_many(conn, cx, group, &[user_id], false).await?;
    log(conn, group.id, acting_user_id, user_id, ADD_USER_TO_GROUP).await?;
    Ok(())
}

/// `group.remove(user)` and, when it removed them,
/// `log_remove_user_from_group`.
pub async fn remove(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    group: &Group,
    user_id: i32,
    acting_user_id: i32,
) -> Result<(), AppError> {
    let removed = remove_many(conn, cx, group, &[user_id]).await?;
    if !removed.is_empty() {
        log(
            conn,
            group.id,
            acting_user_id,
            user_id,
            REMOVE_USER_FROM_GROUP,
        )
        .await?;
    }
    Ok(())
}

/// `GroupManager#add(user_ids, automatic:)`: the users not yet members
/// added (in id order), their side effects, the categories update, and
/// `user_added_to_group` for each.
pub async fn add_many(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    group: &Group,
    user_ids: &[i32],
    _automatic: bool,
) -> Result<Vec<i32>, AppError> {
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    // bulk_add_transaction
    let added: Vec<i32> = sqlx::query_scalar(
        "INSERT INTO group_users (group_id, user_id, notification_level, created_at, updated_at) \
         SELECT $1, u.id, $3, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP FROM users u \
         WHERE u.id = ANY($2) AND NOT EXISTS (SELECT 1 FROM group_users gu \
           WHERE gu.user_id = u.id AND gu.group_id = $1) \
         ORDER BY u.id \
         ON CONFLICT (group_id, user_id) DO NOTHING RETURNING user_id",
    )
    .bind(group.id)
    .bind(user_ids)
    .bind(group.default_notification_level)
    .fetch_all(&mut *conn)
    .await?;
    if added.is_empty() {
        return Ok(added);
    }
    sync_add_side_effects(conn, cx, group, &added).await?;
    bulk_publish_category_updates(conn, cx, group, &added).await?;
    let mut ordered = added.clone();
    ordered.sort_unstable();
    for user_id in ordered {
        // trigger_user_added_event: the web hooks' listener, then the
        // plugins'.
        let group_user_id: i64 = sqlx::query_scalar(
            "SELECT id::int8 FROM group_users WHERE group_id = $1 AND user_id = $2",
        )
        .bind(group.id)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        crate::web_hooks::enqueue_group_user_hooks(
            &mut *conn,
            &cx.payloads(),
            "user_added_to_group",
            group_user_id,
            group.id,
            None,
        )
        .await?;
        crate::plugins::user_added_to_group(conn, cx.state, cx.promotion.settings, user_id).await?;
    }
    Ok(added)
}

/// `GroupManager#sync_add_side_effects`, which GroupUser's create also
/// runs for each member a new group starts with.
pub async fn sync_add_side_effects(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    group: &Group,
    added: &[i32],
) -> Result<(), AppError> {
    // update_title
    if let Some(title) = group.title() {
        sqlx::query(
            "UPDATE users SET title = $2 WHERE id = ANY($1) AND (title IS NULL OR title = '')",
        )
        .bind(added)
        .bind(title)
        .execute(&mut *conn)
        .await?;
    }
    // set_primary_group_and_update_flair
    if group.primary_group {
        sqlx::query(
            "UPDATE users SET flair_group_id = $2 WHERE id = ANY($1) \
             AND flair_group_id IS NOT DISTINCT FROM primary_group_id",
        )
        .bind(added)
        .bind(group.id)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "UPDATE users u SET title = $2 WHERE u.id = ANY($1) AND u.primary_group_id IS NOT NULL \
             AND EXISTS (SELECT 1 FROM groups g WHERE g.id = u.primary_group_id AND g.title = u.title)",
        )
        .bind(added)
        .bind(&group.title)
        .execute(&mut *conn)
        .await?;
        sqlx::query("UPDATE users SET primary_group_id = $2 WHERE id = ANY($1)")
            .bind(added)
            .bind(group.id)
            .execute(&mut *conn)
            .await?;
    }
    // grant_trust_level: TrustLevelGranter for one user, the job for more.
    if let Some(level) = group.granted_level() {
        if let [user_id] = added {
            grant_trust_level(conn, cx.promotion, *user_id, level).await?;
        } else {
            crate::jobs::enqueue(
                &mut *conn,
                "bulk_grant_trust_level",
                json!({ "user_ids": added, "trust_level": level }),
            )
            .await?;
        }
    }
    bulk_set_category_notifications(conn, group.id, added).await?;
    bulk_set_tag_notifications(conn, group.id, added).await?;
    // increase_group_user_count
    sqlx::query("UPDATE groups SET user_count = user_count + $2 WHERE id = $1")
        .bind(group.id)
        .bind(group.counted(added))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `TrustLevelGranter.grant(level, user)`
pub async fn grant_trust_level(
    conn: &mut PgConnection,
    cx: &crate::promotion::Ctx<'_>,
    user_id: i32,
    level: i32,
) -> Result<(), AppError> {
    let current: Option<i32> = sqlx::query_scalar("SELECT trust_level FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&mut *conn)
        .await?;
    if current.is_some_and(|c| c < level)
        && let Err(message) =
            crate::promotion::change_trust_level(conn, cx, user_id, level, None).await?
    {
        return Err(std::io::Error::other(message).into());
    }
    Ok(())
}

/// `semantically_higher_notification_level_sql`: watching ranks above
/// watching first post.
fn higher_level_sql(new_col: &str, existing_col: &str) -> String {
    format!(
        "CASE WHEN (CASE {new_col} WHEN {WATCHING} THEN 5 ELSE {new_col} END) >= \
                   (CASE {existing_col} WHEN {WATCHING} THEN 5 ELSE {existing_col} END) \
              THEN {new_col} ELSE {existing_col} END"
    )
}

/// `GroupUser.bulk_set_category_notifications`
async fn bulk_set_category_notifications(
    conn: &mut PgConnection,
    group_id: i32,
    user_ids: &[i32],
) -> Result<(), sqlx::Error> {
    let defaults: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT category_id, notification_level FROM group_category_notification_defaults \
         WHERE group_id = $1 ORDER BY id",
    )
    .bind(group_id)
    .fetch_all(&mut *conn)
    .await?;
    if defaults.is_empty() {
        return Ok(());
    }
    for (category_id, level) in defaults {
        sqlx::query(&format!(
            "INSERT INTO category_users (user_id, category_id, notification_level) \
             SELECT unnest($1::int[]), $2, $3 \
             ON CONFLICT (user_id, category_id) DO UPDATE SET notification_level = {}",
            higher_level_sql(
                "EXCLUDED.notification_level",
                "category_users.notification_level"
            )
        ))
        .bind(user_ids)
        .bind(category_id)
        .bind(level)
        .execute(&mut *conn)
        .await?;
    }
    crate::user_updater::auto_watch_many(conn, user_ids).await?;
    crate::user_updater::auto_track_many(conn, user_ids).await?;
    Ok(())
}

/// `GroupUser.bulk_set_tag_notifications`
async fn bulk_set_tag_notifications(
    conn: &mut PgConnection,
    group_id: i32,
    user_ids: &[i32],
) -> Result<(), sqlx::Error> {
    let defaults: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT tag_id, notification_level FROM group_tag_notification_defaults \
         WHERE group_id = $1 ORDER BY id",
    )
    .bind(group_id)
    .fetch_all(&mut *conn)
    .await?;
    if defaults.is_empty() {
        return Ok(());
    }
    for (tag_id, level) in defaults {
        sqlx::query(&format!(
            "INSERT INTO tag_users (user_id, tag_id, notification_level, created_at, updated_at) \
             SELECT unnest($1::int[]), $2, $3, NOW(), NOW() \
             ON CONFLICT (user_id, tag_id) DO UPDATE SET notification_level = {}, updated_at = NOW()",
            higher_level_sql("EXCLUDED.notification_level", "tag_users.notification_level")
        ))
        .bind(user_ids)
        .bind(tag_id)
        .bind(level)
        .execute(&mut *conn)
        .await?;
    }
    tag_auto_watch(conn, user_ids).await?;
    tag_auto_track(conn, user_ids).await?;
    Ok(())
}

/// `TopicUser.notification_reasons`
const AUTO_WATCH_TAG: i32 = 10;
const AUTO_TRACK_TAG: i32 = 11;
const USER_CHANGED: i32 = 3;

/// `TagUser.auto_watch(user_ids:)`
pub async fn tag_auto_watch(conn: &mut PgConnection, user_ids: &[i32]) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE topic_users \
            SET notification_level = CASE WHEN should_watch THEN 3 ELSE 2 END, \
                notifications_reason_id = CASE WHEN should_watch THEN $2 ELSE NULL END \
           FROM ( \
             SELECT tu.topic_id, tu.user_id, \
                    CASE WHEN MAX(tag_users.notification_level) = 3 THEN true ELSE false END should_watch, \
                    CASE WHEN MAX(tag_users.notification_level) IS NULL AND tu.notification_level = 3 \
                              AND tu.notifications_reason_id = $2 THEN true ELSE false END should_track \
               FROM topic_users tu \
          LEFT JOIN topic_tags ON tu.topic_id = topic_tags.topic_id \
          LEFT JOIN tag_users ON tag_users.user_id = tu.user_id AND topic_tags.tag_id = tag_users.tag_id \
                AND tag_users.notification_level = 3 \
              WHERE tu.notification_level IN (2, 1, 3) \
                AND COALESCE(tu.notifications_reason_id, 0) != $3 AND tu.user_id = ANY($1) \
           GROUP BY tu.topic_id, tu.user_id, tu.notification_level, tu.notifications_reason_id \
           ) AS X \
          WHERE X.topic_id = topic_users.topic_id AND X.user_id = topic_users.user_id \
            AND (should_track OR should_watch)",
    )
    .bind(user_ids)
    .bind(AUTO_WATCH_TAG)
    .bind(USER_CHANGED)
    .execute(conn)
    .await?;
    Ok(())
}

/// `TagUser.auto_track(user_ids:)`
pub async fn tag_auto_track(conn: &mut PgConnection, user_ids: &[i32]) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE topic_users SET notification_level = 2, notifications_reason_id = $2 \
           FROM ( \
             SELECT DISTINCT tu.topic_id, tu.user_id FROM topic_users tu \
               JOIN topic_tags ON tu.topic_id = topic_tags.topic_id \
               JOIN tag_users ON tag_users.user_id = tu.user_id AND topic_tags.tag_id = tag_users.tag_id \
                AND tag_users.notification_level = 2 \
              WHERE tu.user_id = ANY($1) \
           ) AS X \
          WHERE topic_users.notification_level = 1 \
            AND COALESCE(topic_users.notifications_reason_id, 0) != $3 \
            AND topic_users.topic_id = X.topic_id AND topic_users.user_id = X.user_id",
    )
    .bind(user_ids)
    .bind(AUTO_TRACK_TAG)
    .bind(USER_CHANGED)
    .execute(conn)
    .await?;
    Ok(())
}

/// `bulk_publish_category_updates`: for a group with categories, one
/// member is sent the categories they may now see and those they no
/// longer may (fewer than ten categories; else a refresh), several a
/// refresh.
async fn bulk_publish_category_updates(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    group: &Group,
    user_ids: &[i32],
) -> Result<(), AppError> {
    let category_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT c.id FROM categories c JOIN category_groups cg ON cg.category_id = c.id \
         WHERE cg.group_id = $1 ORDER BY cg.id",
    )
    .bind(group.id)
    .fetch_all(&mut *conn)
    .await?;
    if category_ids.is_empty() || user_ids.is_empty() {
        return Ok(());
    }
    let bus = &cx.state.bus;
    let refresh =
        |ids: &[i32]| -> Vec<String> { ids.iter().map(|id| crate::bus::user_tag(*id)).collect() };
    if let [user_id] = user_ids
        && (category_ids.len() as i64) < PUBLISH_CATEGORIES_LIMIT
    {
        // Category.set_permission! for the member: the categories with a
        // permission go out through CategorySerializer (no scope), the
        // others as deleted.
        let Some(user) = crate::session::current::SessionUser::load(&mut *conn, *user_id).await?
        else {
            return Ok(());
        };
        let guardian = crate::guardian::Guardian::for_user(&mut *conn, &user).await?;
        let settings = cx.promotion.settings;
        let anonymous = crate::guardian::Guardian::anonymous();
        let mut updated = Vec::new();
        let mut removed = Vec::new();
        for id in &category_ids {
            let Some(c) =
                crate::category_serializer::GuardedCategory::load(&mut *conn, *id).await?
            else {
                continue;
            };
            match crate::category_serializer::set_permission(&mut *conn, settings, &guardian, &c)
                .await?
            {
                Some(permission) => {
                    let pre = crate::category_serializer::Preloaded {
                        permission: Some(permission),
                        ..Default::default()
                    };
                    updated.push(
                        crate::category_serializer::serialize(
                            &mut *conn,
                            &crate::category_serializer::Cx::of(cx.state),
                            settings,
                            &anonymous,
                            *id,
                            &pre,
                        )
                        .await?,
                    );
                }
                None => removed.push(*id),
            }
        }
        bus.publish(
            &mut *conn,
            "/categories",
            &json!({ "categories": updated, "deleted_categories": removed }),
            Some(&refresh(user_ids)),
        )
        .await?;
        return Ok(());
    }
    // Discourse.request_refresh!(user_ids:)
    bus.publish(
        &mut *conn,
        "/refresh_client",
        &json!("clobber"),
        Some(&refresh(user_ids)),
    )
    .await?;
    Ok(())
}

/// `GroupManager#remove(user_ids)`: the members among them removed (in
/// their membership order), their side effects, the trust level job,
/// the categories update, the inaccessible notification cleanups, and
/// `user_removed_from_group` for each.
pub async fn remove_many(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    group: &Group,
    user_ids: &[i32],
) -> Result<Vec<i32>, AppError> {
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    let removed: Vec<i32> = sqlx::query_scalar(
        "SELECT user_id FROM group_users WHERE group_id = $1 AND user_id = ANY($2) ORDER BY id",
    )
    .bind(group.id)
    .bind(user_ids)
    .fetch_all(&mut *conn)
    .await?;
    if removed.is_empty() {
        return Ok(removed);
    }
    // build_user_removed_webhook_payloads: before the rows go, by id.
    let mut hook_payloads: Vec<(i64, serde_json::Value)> = Vec::new();
    if crate::web_hooks::active(&mut *conn, "user_removed_from_group").await? {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id::int8 FROM group_users WHERE group_id = $1 AND user_id = ANY($2) ORDER BY id",
        )
        .bind(group.id)
        .bind(&removed)
        .fetch_all(&mut *conn)
        .await?;
        for id in ids {
            hook_payloads.push((id, cx.payloads().group_user(&mut *conn, id).await?));
        }
    }
    // bulk_remove_transaction
    sqlx::query("DELETE FROM group_users WHERE group_id = $1 AND user_id = ANY($2)")
        .bind(group.id)
        .bind(&removed)
        .execute(&mut *conn)
        .await?;
    remove_primary_and_flair_group(conn, group.id, &removed).await?;
    grant_other_available_title(conn, cx.promotion.i18n, group, &removed).await?;
    decrease_group_user_count(conn, group, &removed).await?;
    // recalculate_trust_level
    if group.granted_level().is_some() {
        crate::jobs::enqueue(
            &mut *conn,
            "bulk_grant_trust_level",
            json!({ "user_ids": removed, "recalculate": true }),
        )
        .await?;
    }
    bulk_publish_category_updates(conn, cx, group, &removed).await?;
    // enqueue_pm_notification_cleanup: per topic, its users.
    let pairs: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT DISTINCT n.topic_id, n.user_id FROM notifications n \
         WHERE n.user_id = ANY($1) AND n.topic_id IN (SELECT topic_id FROM topic_allowed_groups WHERE group_id = $2) \
         ORDER BY n.topic_id, n.user_id",
    )
    .bind(&removed)
    .bind(group.id)
    .fetch_all(&mut *conn)
    .await?;
    let mut topics: Vec<i32> = pairs.iter().map(|(t, _)| *t).collect();
    topics.dedup();
    for topic_id in topics {
        let users: Vec<i32> = pairs
            .iter()
            .filter(|(t, _)| *t == topic_id)
            .map(|(_, u)| *u)
            .collect();
        crate::jobs::enqueue(
            &mut *conn,
            "delete_inaccessible_notifications",
            json!({ "topic_id": topic_id, "user_ids": users }),
        )
        .await?;
    }
    let mut ordered = removed.clone();
    ordered.sort_unstable();
    for user_id in ordered {
        crate::plugins::user_removed_from_group(conn, cx.promotion.settings, user_id).await?;
    }
    // enqueue_user_removed_webhook_events
    for (group_user_id, payload) in hook_payloads {
        crate::web_hooks::enqueue_group_user_hooks(
            &mut *conn,
            &cx.payloads(),
            "user_removed_from_group",
            group_user_id,
            group.id,
            Some(payload),
        )
        .await?;
    }
    Ok(removed)
}

/// `remove_primary_and_flair_group`
pub async fn remove_primary_and_flair_group(
    conn: &mut PgConnection,
    group_id: i32,
    user_ids: &[i32],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET primary_group_id = NULL WHERE id = ANY($1) AND primary_group_id = $2",
    )
    .bind(user_ids)
    .bind(group_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE users SET flair_group_id = NULL WHERE id = ANY($1) AND flair_group_id = $2",
    )
    .bind(user_ids)
    .bind(group_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `decrease_group_user_count`
pub async fn decrease_group_user_count(
    conn: &mut PgConnection,
    group: &Group,
    user_ids: &[i32],
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE groups SET user_count = user_count - $2 WHERE id = $1")
        .bind(group.id)
        .bind(group.counted(user_ids))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `grant_other_available_title`: the group's title taken from those who
/// have no other group or badge title, and the next best title given to
/// those who do (`update_column`, no timestamps).
pub async fn grant_other_available_title(
    conn: &mut PgConnection,
    i18n: &crate::i18n::I18n,
    group: &Group,
    user_ids: &[i32],
) -> Result<(), sqlx::Error> {
    let Some(title) = group.title() else {
        return Ok(());
    };
    sqlx::query(
        "UPDATE users u SET title = NULL WHERE u.id = ANY($1) AND u.title = $2 \
         AND NOT EXISTS (SELECT 1 FROM group_users gu JOIN groups g ON g.id = gu.group_id \
           WHERE gu.user_id = u.id AND g.title IS NOT NULL AND g.title <> '') \
         AND NOT EXISTS (SELECT 1 FROM user_badges ub JOIN badges b ON b.id = ub.badge_id \
           WHERE ub.user_id = u.id AND b.allow_title = true)",
    )
    .bind(user_ids)
    .bind(title)
    .execute(&mut *conn)
    .await?;
    let keeping: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM users WHERE id = ANY($1) AND title = $2 ORDER BY id")
            .bind(user_ids)
            .bind(title)
            .fetch_all(&mut *conn)
            .await?;
    for user_id in keeping {
        let next = next_best_title(conn, i18n, user_id).await?;
        sqlx::query("UPDATE users SET title = $2 WHERE id = $1")
            .bind(user_id)
            .bind(next)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// `User#next_best_title`: a titled group's title (the primary group's
/// first, then primary groups), else a title badge's display name.
/// Rails orders no further; ties go by id here.
pub async fn next_best_title(
    conn: &mut PgConnection,
    i18n: &crate::i18n::I18n,
    user_id: i32,
) -> Result<Option<String>, sqlx::Error> {
    let group_title: Option<String> = sqlx::query_scalar(
        "SELECT g.title FROM groups g JOIN group_users gu ON gu.group_id = g.id \
         JOIN users u ON u.id = gu.user_id \
         WHERE gu.user_id = $1 AND g.title <> '' \
         ORDER BY (u.primary_group_id IS NOT NULL AND g.id = u.primary_group_id) DESC, \
                  g.primary_group DESC, g.id LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    if group_title.is_some() {
        return Ok(group_title);
    }
    let badge: Option<String> = sqlx::query_scalar(
        "SELECT b.name FROM badges b JOIN user_badges ub ON ub.badge_id = b.id \
         WHERE ub.user_id = $1 AND b.allow_title ORDER BY ub.id LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(badge.map(|name| {
        let key = format!("badges.{}.name", name.to_lowercase().replace(' ', "_"));
        i18n.t(&key).map(str::to_string).unwrap_or(name)
    }))
}

/// `Discourse.system_user`
const SYSTEM_USER_ID: i32 = -1;

/// `User#email_confirmed?`: a confirmed token for the primary email, no
/// tokens at all, or the email the DiscourseConnect record has.
pub async fn email_confirmed(conn: &mut PgConnection, user_id: i32) -> Result<bool, AppError> {
    let (confirmed, none, sso): (bool, bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM email_tokens t JOIN user_emails e ON e.user_id = t.user_id \
                          AND e.primary AND t.email = e.email \
                        WHERE t.user_id = $1 AND t.confirmed), \
                NOT EXISTS (SELECT 1 FROM email_tokens WHERE user_id = $1), \
                EXISTS (SELECT 1 FROM single_sign_on_records r JOIN user_emails e \
                          ON e.user_id = r.user_id AND e.primary \
                        WHERE r.user_id = $1 AND lower(r.external_email) = e.email)",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(confirmed || none || sso)
}

/// `User#set_automatic_groups`: an active, unstaged, confirmed user added
/// (and logged by the system user) to each group whose email domains
/// match their primary email and that they are not in.
pub async fn set_automatic_groups(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    user_id: i32,
) -> Result<(), AppError> {
    let user: Option<(bool, bool, Option<String>)> = sqlx::query_as(
        "SELECT active, staged, (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\" LIMIT 1) \
         FROM users u WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((true, false, Some(email))) = user else {
        return Ok(());
    };
    if !email_confirmed(conn, user_id).await? {
        return Ok(());
    }
    let groups: Vec<(i32, String)> = sqlx::query_as(
        "SELECT id, automatic_membership_email_domains FROM groups WHERE NOT automatic \
         AND LENGTH(COALESCE(automatic_membership_email_domains, '')) > 0 ORDER BY id",
    )
    .fetch_all(&mut *conn)
    .await?;
    for (group_id, domains) in groups {
        let pattern = format!("(?i)@({})$", domains.replace('.', "\\."));
        let Ok(re) = regex::Regex::new(&pattern) else {
            return Err(Unsupported("email domains that are not a regular expression").into());
        };
        if !re.is_match(&email) {
            continue;
        }
        let member: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM group_users WHERE group_id = $1 AND user_id = $2)",
        )
        .bind(group_id)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if member {
            continue;
        }
        let Some(group) = find(conn, group_id).await? else {
            continue;
        };
        add_many(conn, cx, &group, &[user_id], false).await?;
        log_many(
            conn,
            group_id,
            SYSTEM_USER_ID,
            &[user_id],
            ADD_USER_TO_GROUP,
        )
        .await?;
    }
    Ok(())
}

/// `Jobs::AutomaticGroupMembership`: every activated, unstaged user with
/// an email in the group's domains (once an email, by id) who confirmed
/// it, added automatically and logged by the system user (logged also
/// when already a member).
pub async fn automatic_group_membership(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    group_id: i32,
) -> Result<(), AppError> {
    let Some(group) = find(conn, group_id).await? else {
        return Err(Unsupported(
            "automatic_group_membership of a missing group (InvalidParameters)",
        )
        .into());
    };
    let domains: Option<String> =
        sqlx::query_scalar("SELECT automatic_membership_email_domains FROM groups WHERE id = $1")
            .bind(group_id)
            .fetch_one(&mut *conn)
            .await?;
    let Some(domains) = domains.filter(|d| !d.trim().is_empty()) else {
        return Ok(());
    };
    let pattern = format!("@({})$", domains.replace('.', "\\."));
    let users: Vec<i32> = sqlx::query_scalar(
        "SELECT u.id FROM users u JOIN user_emails e ON e.user_id = u.id \
         WHERE e.email ~* $1 AND u.active AND NOT u.staged ORDER BY u.id",
    )
    .bind(&pattern)
    .fetch_all(&mut *conn)
    .await?;
    for user_id in users {
        if !email_confirmed(conn, user_id).await? {
            continue;
        }
        add_many(conn, cx, &group, &[user_id], true).await?;
        log_many(
            conn,
            group_id,
            SYSTEM_USER_ID,
            &[user_id],
            ADD_USER_TO_GROUP,
        )
        .await?;
    }
    Ok(())
}
