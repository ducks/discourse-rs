//! The review queue's bookkeeping that flagging goes through: Reviewable
//! (`needs_review!`, `add_score`, `recalculate_score`, `log_history`), the
//! score of a flag (ReviewableScore), and the thresholds flags are held to
//! (`sensitivity_score` and `min_score_for_priority`, from the priorities
//! Jobs::ReviewablePriorities keeps in the plugin store).

use serde_json::json;
use sqlx::PgConnection;

use crate::guardian::{Guardian, GuardianError};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `Reviewable.sti_names`: the core types, then the type chat registers
/// on the reference.
pub const STI_NAMES: [&str; 5] = [
    "ReviewableFlaggedPost",
    "ReviewableQueuedPost",
    "ReviewableUser",
    "ReviewablePost",
    "Chat::ReviewableMessage",
];

/// `Reviewable.statuses[:pending]`, also `ReviewableScore.statuses[:pending]`
pub const PENDING: i32 = 0;
/// `ReviewableScore.statuses[:agreed]`
pub const AGREED: i32 = 1;
/// `Reviewable.statuses[:approved]`
const APPROVED: i32 = 1;
/// `ReviewableHistory.types[:created]`
const HISTORY_CREATED: i32 = 0;
/// `ReviewableHistory.types[:transitioned]`
const HISTORY_TRANSITIONED: i32 = 1;

/// `Reviewable.typical_sensitivity`
const TYPICAL_SENSITIVITY: f64 = 12.5;
/// `Reviewable.sensitivities[:low]`
const LOW_SENSITIVITY: f64 = 9.0;

/// `PluginStore.get("reviewables", "priority_#{id}")` as `to_f` reads it.
async fn priority(conn: &mut PgConnection, id: i32) -> Result<Option<f64>, sqlx::Error> {
    let value: Option<Option<String>> = sqlx::query_scalar(
        "SELECT value FROM plugin_store_rows WHERE plugin_name = 'reviewables' AND key = $1",
    )
    .bind(format!("priority_{id}"))
    .fetch_optional(conn)
    .await?;
    Ok(value.flatten().map(|v| crate::ruby::to_f(&v)))
}

/// `Reviewable.min_score_for_priority` for reviewable_default_visibility.
pub async fn min_score_for_priority(
    conn: &mut PgConnection,
    s: &SiteSettings,
) -> Result<f64, AppError> {
    Ok(min_score(conn, s).await?)
}

/// `min_score_for_priority` with the guardian's error type.
pub async fn min_score(
    conn: &mut PgConnection,
    s: &SiteSettings,
) -> Result<f64, crate::guardian::GuardianError> {
    let id = match s.get("reviewable_default_visibility")?.to_s().as_str() {
        "low" => 0,
        "medium" => 5,
        "high" => 10,
        _ => return Ok(0.0),
    };
    Ok(priority(conn, id).await?.unwrap_or(0.0))
}

/// `Reviewable.sensitivity_score(SiteSetting.<setting>, scale:)`
pub async fn sensitivity_score(
    conn: &mut PgConnection,
    s: &SiteSettings,
    setting: &str,
    scale: f64,
) -> Result<f64, AppError> {
    let sensitivity = s.get(setting)?.to_i() as f64;
    let value = if sensitivity == 0.0 {
        f64::MAX
    } else {
        let high = priority(conn, 10).await?.unwrap_or(TYPICAL_SENSITIVITY);
        ((high * (sensitivity / LOW_SENSITIVITY)) * scale * 100.0).trunc() / 100.0
    };
    Ok(value.max(min_score_for_priority(conn, s).await?))
}

/// `Reviewable.viewable_by(user)` past its order and preloads: the joins
/// and the condition it adds for a user who isn't an admin.
pub struct Viewable {
    /// `left_joins(category: :category_moderation_groups)`
    pub joins: &'static str,
    pub clause: String,
}

const VIEWABLE_JOINS: &str = " LEFT OUTER JOIN categories ON categories.id = reviewables.category_id \
     LEFT OUTER JOIN category_moderation_groups ON category_moderation_groups.category_id = categories.id";

/// `Reviewable.viewable_by(user)`: None for admins, who see every
/// reviewable. Anyone else sees those moderators may review (when a
/// moderator) or that their groups moderate the category of (with
/// enable_category_group_moderation), outside categories they can't see,
/// private messages they can't see and posts they can't review
/// (`reviewable_post_scope`).
pub async fn viewable_by(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
) -> Result<Option<Viewable>, GuardianError> {
    if guardian.is_admin() {
        return Ok(None);
    }
    // `return none if user.blank?`
    if guardian.is_anonymous() {
        return Ok(Some(Viewable {
            joins: VIEWABLE_JOINS,
            clause: "FALSE".into(),
        }));
    }
    let group_ids = if s.get("enable_category_group_moderation")?.truthy() {
        guardian.group_ids().to_vec()
    } else {
        Vec::new()
    };
    let group_ids = if group_ids.is_empty() {
        "NULL".to_string()
    } else {
        group_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };
    let mut clauses = vec![
        format!(
            "((reviewables.reviewable_by_moderator AND {}) OR (category_moderation_groups.group_id IN ({group_ids})))",
            guardian.is_moderator()
        ),
        format!(
            "(reviewables.category_id IS NULL OR reviewables.category_id IN ({}))",
            guardian.allowed_category_ids_sql(s)?
        ),
    ];
    // exclude_private_messages_hidden_from: private_message_topic_scope
    // over every topic, read for the private messages alone.
    let min = min_score(&mut *conn, s).await?;
    if let Some(pm) = guardian.private_message_topic_clause(min) {
        clauses.push(format!(
            "NOT EXISTS (SELECT 1 FROM topics private_message \
               WHERE private_message.id = reviewables.topic_id \
                 AND private_message.archetype = 'private_message' \
                 AND private_message.id NOT IN \
                   (SELECT topics.id FROM topics WHERE topics.archetype = 'private_message' AND {pm}))"
        ));
    }
    let posts = reviewable_post_clause(&mut *conn, s, guardian).await?;
    clauses.push(format!(
        "NOT EXISTS (SELECT 1 FROM posts reviewable_target \
           WHERE reviewables.target_type = 'Post' AND reviewable_target.id = reviewables.target_id \
             AND NOT EXISTS (SELECT posts.id FROM posts WHERE {posts} AND posts.id = reviewable_target.id))"
    ));
    Ok(Some(Viewable {
        joins: VIEWABLE_JOINS,
        clause: clauses.join(" AND "),
    }))
}

/// `guardian.reviewable_post_scope` as a condition on `posts` (deleted
/// posts in, as `Post.with_deleted`): every post for admins; for staff
/// the posts of a type they see in topics they see, deleted ones too; for
/// category group moderators only in the categories they moderate.
pub async fn reviewable_post_clause(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
) -> Result<String, GuardianError> {
    if guardian.is_admin() {
        return Ok("TRUE".into());
    }
    let group_moderator = guardian.category_group_moderation_allowed(s)?;
    if !guardian.is_staff() && !group_moderator {
        return Ok("FALSE".into());
    }
    let mut topics = guardian
        .visible_topic_clause(&mut *conn, s, false)
        .await?
        .unwrap_or_else(|| "TRUE".into());
    if !guardian.is_staff()
        && let Some(moderated) = guardian.category_group_moderator_sql()
    {
        topics = format!("{topics} AND topics.category_id IN ({moderated})");
    }
    let types = guardian
        .visible_post_types(s)?
        .iter()
        .map(i32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!(
        "(posts.post_type IN ({types}) AND posts.topic_id IN \
           (SELECT topics.id FROM topics LEFT OUTER JOIN shared_drafts ON shared_drafts.topic_id = topics.id \
            WHERE {topics}))"
    ))
}

/// `Reviewable#can_review_target?(guardian)`: a reviewable whose target is
/// a post (deleted ones too) needs `can_review_post?`; any other, or one
/// whose post is gone, is reviewable.
pub async fn can_review_target(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    target_type: Option<&str>,
    target_id: Option<i32>,
) -> Result<bool, AppError> {
    let (Some("Post"), Some(post_id)) = (target_type, target_id) else {
        return Ok(true);
    };
    if guardian.is_admin() {
        return Ok(true);
    }
    let posts = reviewable_post_clause(&mut *conn, s, guardian).await?;
    Ok(sqlx::query_scalar(&format!(
        "SELECT NOT EXISTS (SELECT 1 FROM posts reviewable_target WHERE reviewable_target.id = $1) \
             OR EXISTS (SELECT 1 FROM posts WHERE {posts} AND posts.id = $1)"
    ))
    .bind(post_id)
    .fetch_one(&mut *conn)
    .await?)
}

/// ReviewablesController#find_reviewable's lookup:
/// `Reviewable.viewable_by(current_user).where(id:).first` exists.
pub async fn is_viewable(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    id: i64,
) -> Result<bool, AppError> {
    let (joins, clause) = match viewable_by(&mut *conn, s, guardian).await? {
        Some(v) => (v.joins, v.clause),
        None => ("", "TRUE".to_string()),
    };
    Ok(sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM reviewables{joins} WHERE reviewables.id = $1 AND {clause})"
    ))
    .bind(id)
    .fetch_one(&mut *conn)
    .await?)
}

/// `User#reviewable_count` and `Reviewable.unseen_reviewable_count(user)`
/// for staff: the user's pending list, and the part of it past the
/// reviewable they saw last.
pub async fn staff_counts(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
) -> Result<(i64, i64), AppError> {
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("reviewable counts for anonymous users"))?;
    let base = pending_list_for(&mut *conn, s, guardian).await?;
    let last_seen: Option<i32> =
        sqlx::query_scalar("SELECT last_seen_reviewable_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) {base}"))
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    let unseen: i64 = match last_seen {
        Some(id) => {
            sqlx::query_scalar(&format!("SELECT COUNT(*) {base} AND reviewables.id > $2"))
                .bind(user_id)
                .bind(id)
                .fetch_one(&mut *conn)
                .await?
        }
        None => count,
    };
    Ok((count, unseen))
}

/// The pending reviewables a user's own lists read (`Reviewable.list_for`
/// with the pending status, the default priority and
/// `include_claimed_by_others: false`): the FROM and WHERE, `$1` the user.
async fn pending_list_for(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
) -> Result<String, AppError> {
    let viewable = viewable_by(&mut *conn, s, guardian).await?;
    let types = STI_NAMES
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(",");
    let (joins, clause) = match &viewable {
        Some(v) => (v.joins, format!(" AND {}", v.clause)),
        None => ("", String::new()),
    };
    let mut base = format!(
        "FROM reviewables{joins} LEFT JOIN reviewable_claimed_topics rct ON reviewables.topic_id = rct.topic_id \
         WHERE reviewables.status = 0{clause} AND (rct.user_id IS NULL OR rct.user_id = $1) \
         AND reviewables.type IN ({types})"
    );
    let min = min_score_for_priority(&mut *conn, s).await?;
    if min > 0.0 {
        base.push_str(&format!(
            " AND (reviewables.score >= {min} OR reviewables.force_review)"
        ));
    }
    Ok(base)
}

/// `Reviewable.user_menu_list_for(user)` (30 at most, by score then
/// newest) through `Reviewable.basic_serializers_for_list`: the basic
/// serializer of each type (BasicReviewableFlaggedPostSerializer,
/// ...QueuedPostSerializer, ...UserSerializer, else
/// BasicReviewableSerializer).
pub async fn user_menu_list(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
) -> Result<Vec<serde_json::Value>, AppError> {
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("the review queue for anonymous users"))?;
    type Row = (
        i64,
        String,
        i32,
        chrono::NaiveDateTime,
        Option<String>,
        Option<String>,
        Option<i32>,
        Option<i32>,
        Option<serde_json::Value>,
    );
    let base = pending_list_for(&mut *conn, s, guardian).await?;
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT reviewables.id, reviewables.type, reviewables.status, reviewables.created_at, \
                (SELECT username FROM users WHERE users.id = reviewables.created_by_id), \
                reviewables.target_type, reviewables.target_id, reviewables.topic_id, reviewables.payload \
         {base} ORDER BY reviewables.score DESC, reviewables.created_at DESC LIMIT 30"
    ))
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for (id, kind, status, created_at, flagger, target_type, target_id, topic_id, payload) in rows {
        let mut r = serde_json::Map::new();
        r.insert("flagger_username".into(), json!(flagger));
        r.insert("id".into(), json!(id));
        r.insert("type".into(), json!(kind));
        r.insert("pending".into(), json!(status == PENDING));
        r.insert(
            "created_at".into(),
            json!(crate::topic_list::time_json(created_at)),
        );
        // The topic as `object.topic` loads it: Trashable leaves a
        // deleted one out.
        let topic: Option<(i32, String, Option<String>)> = match topic_id {
            Some(tid) => sqlx::query_as(
                "SELECT id, title, fancy_title FROM topics WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(tid)
            .fetch_optional(&mut *conn)
            .await?,
            None => None,
        };
        let fancy = match &topic {
            Some((tid, title, stored)) => Some(
                crate::topic_query::fancy_title(&mut *conn, s, *tid, title, stored.as_deref())
                    .await?,
            ),
            None => None,
        };
        let payload = payload.unwrap_or(serde_json::Value::Null);
        // `payload["title"].present?`
        let payload_title = payload["title"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .map(str::to_string);
        match kind.as_str() {
            "ReviewableFlaggedPost" => {
                let post_number: Option<i32> =
                    match (target_type.as_deref(), target_id) {
                        (Some("Post"), Some(pid)) => sqlx::query_scalar(
                            "SELECT post_number FROM posts WHERE id = $1 AND deleted_at IS NULL",
                        )
                        .bind(pid)
                        .fetch_optional(&mut *conn)
                        .await?,
                        _ => None,
                    };
                if let Some(n) = post_number {
                    r.insert("post_number".into(), json!(n));
                }
                if let Some(f) = fancy {
                    r.insert("topic_fancy_title".into(), json!(f));
                }
            }
            "ReviewableQueuedPost" => {
                if let Some(f) = fancy {
                    r.insert("topic_fancy_title".into(), json!(f));
                }
                if let Some(title) = &payload_title {
                    r.insert("payload_title".into(), json!(title));
                }
                r.insert("is_new_topic".into(), json!(payload_title.is_some()));
            }
            "ReviewableUser" => {
                r.insert("username".into(), payload["username"].clone());
            }
            _ => {}
        }
        out.push(serde_json::Value::Object(r));
    }
    Ok(out)
}

/// `User#bump_last_seen_reviewable!`: the newest of
/// `Reviewable.unseen_list_for(user)` past the one last seen becomes it,
/// and the user's reviewable counts go out on `/reviewable_counts/:id`.
pub async fn bump_last_seen(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    s: &SiteSettings,
    guardian: &Guardian,
) -> Result<(), AppError> {
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("the review queue for anonymous users"))?;
    let base = pending_list_for(&mut *conn, s, guardian).await?;
    let max: Option<i64> = sqlx::query_scalar(&format!(
        "SELECT MAX(reviewables.id) {base} AND reviewables.id > \
         COALESCE((SELECT last_seen_reviewable_id FROM users WHERE id = $1), 0)"
    ))
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    let Some(max) = max else {
        return Ok(());
    };
    sqlx::query("UPDATE users SET last_seen_reviewable_id = $2, updated_at = now() WHERE id = $1")
        .bind(user_id)
        .bind(i32::try_from(max).unwrap_or(i32::MAX))
        .execute(&mut *conn)
        .await?;
    publish_counts(&mut *conn, bus, s, guardian, None).await
}

/// `User#publish_reviewable_counts(extra_data)`: the user's pending and
/// unseen counts, with `updates` when given, on `/reviewable_counts/:id`
/// to the user alone.
pub async fn publish_counts(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    s: &SiteSettings,
    guardian: &Guardian,
    updates: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Result<(), AppError> {
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("reviewable counts for anonymous users"))?;
    let (count, unseen) = staff_counts(&mut *conn, s, guardian).await?;
    let mut data = json!({ "reviewable_count": count, "unseen_reviewable_count": unseen });
    // `extra_data.present?`
    if let Some(updates) = updates.filter(|u| !u.is_empty()) {
        data["updates"] = serde_json::Value::Object(updates.clone());
    }
    bus.publish(
        &mut *conn,
        &format!("/reviewable_counts/{user_id}"),
        &data,
        Some(&[crate::bus::user_tag(user_id)]),
    )
    .await?;
    Ok(())
}

/// `Jobs::NotifyReviewable#execute`: every admin's reviewable counts, then
/// every moderator's not yet told when moderators may review it, each with
/// the `updates` (the reviewables the action resolved, their status and
/// who performed it) that staff member may see.
pub async fn notify_reviewable(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    s: &SiteSettings,
    args: &serde_json::Value,
) -> Result<(), AppError> {
    let Some(id) = args["reviewable_id"].as_i64() else {
        return Ok(());
    };
    let reviewable: Option<(bool, Option<i32>)> = sqlx::query_as(
        "SELECT reviewable_by_moderator, category_id FROM reviewables WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((by_moderator, category_id)) = reviewable else {
        return Ok(());
    };
    let group_moderation = s.get("enable_category_group_moderation")?.truthy();
    if group_moderation && category_id.is_some() {
        return Err(Unsupported("notify_reviewable for category group moderators").into());
    }
    let mut admin_updates = serde_json::Map::new();
    let mut moderator_updates = serde_json::Map::new();
    let updated: Vec<i64> = args["updated_reviewable_ids"]
        .as_array()
        .map(|ids| ids.iter().filter_map(serde_json::Value::as_i64).collect())
        .unwrap_or_default();
    if !updated.is_empty() {
        let rows: Vec<(i64, i32, bool, Option<i32>)> = sqlx::query_as(
            "SELECT id, status, reviewable_by_moderator, category_id FROM reviewables \
             WHERE id = ANY($1)",
        )
        .bind(&updated)
        .fetch_all(&mut *conn)
        .await?;
        for (rid, status, rid_by_moderator, rid_category) in rows {
            if group_moderation && rid_category.is_some() {
                return Err(Unsupported(
                    "notify_reviewable's updates for category group moderators",
                )
                .into());
            }
            let payload = json!({
                "last_performing_username": args["performing_username"],
                "status": status,
            });
            admin_updates.insert(rid.to_string(), payload.clone());
            if rid_by_moderator {
                moderator_updates.insert(rid.to_string(), payload);
            }
        }
    }
    // User.real.admins, then User.real.moderators not already told, each
    // by id (find_each).
    let real = "id > 0 AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id)";
    let admins: Vec<i32> = sqlx::query_scalar(&format!(
        "SELECT id FROM users WHERE admin AND {real} ORDER BY id"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let moderators: Vec<i32> = if by_moderator {
        sqlx::query_scalar(&format!(
            "SELECT id FROM users WHERE moderator AND NOT admin AND {real} ORDER BY id"
        ))
        .fetch_all(&mut *conn)
        .await?
    } else {
        Vec::new()
    };
    for (ids, updates) in [(admins, &admin_updates), (moderators, &moderator_updates)] {
        for user_id in ids {
            publish_counts_of(&mut *conn, bus, s, user_id, Some(updates)).await?;
        }
    }
    Ok(())
}

/// `Jobs::RefreshUsersReviewableCounts#execute`: the reviewable counts of
/// every human member of the groups.
pub async fn refresh_users_reviewable_counts(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    s: &SiteSettings,
    args: &serde_json::Value,
) -> Result<(), AppError> {
    let group_ids: Vec<i64> = args["group_ids"]
        .as_array()
        .map(|ids| ids.iter().filter_map(serde_json::Value::as_i64).collect())
        .unwrap_or_default();
    if group_ids.is_empty() {
        return Ok(());
    }
    // `.each` without an order: by id here.
    let users: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM users WHERE id > 0 AND id IN \
           (SELECT user_id FROM group_users WHERE group_id = ANY($1)) ORDER BY id",
    )
    .bind(&group_ids)
    .fetch_all(&mut *conn)
    .await?;
    for user_id in users {
        publish_counts_of(&mut *conn, bus, s, user_id, None).await?;
    }
    Ok(())
}

/// `publish_reviewable_counts` for a user by id.
async fn publish_counts_of(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    s: &SiteSettings,
    user_id: i32,
    updates: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Result<(), AppError> {
    let Some(user) = crate::session::current::SessionUser::load(&mut *conn, user_id).await? else {
        return Ok(());
    };
    let guardian = Guardian::for_user(&mut *conn, &user).await?;
    publish_counts(&mut *conn, bus, s, &guardian, updates).await
}

/// `ReviewableScore.calc_user_accuracy_bonus(agreed, disagreed)`
fn accuracy_bonus(agreed: i32, disagreed: i32) -> f64 {
    let total = f64::from(agreed + disagreed);
    if total <= 5.0 {
        return 0.0;
    }
    let axis = 0.7;
    let correct = f64::from(agreed) / total;
    let positive = correct >= axis;
    let (bottom, top) = if positive { (axis, 1.0) } else { (0.0, axis) };
    let distance = if positive {
        correct - bottom
    } else {
        top - correct
    };
    let sign = if positive { 1.0 } else { -1.0 };
    let bonus = distance * (1.0 / (top - bottom)) * sign * (total.ln() / 4f64.ln() * 5.0);
    (bonus * 100.0).round() / 100.0
}

/// A flagger as ReviewableScore weighs them.
pub struct Flagger {
    pub id: i32,
    pub staff: bool,
    pub trust_level: i32,
}

/// `ReviewableScore.user_accuracy_bonus(user)`: none for bots or without
/// user stats.
async fn user_accuracy_bonus(conn: &mut PgConnection, user: &Flagger) -> Result<f64, sqlx::Error> {
    if user.id < 0 {
        return Ok(0.0);
    }
    let stats: Option<(i32, i32)> =
        sqlx::query_as("SELECT flags_agreed, flags_disagreed FROM user_stats WHERE user_id = $1")
            .bind(user.id)
            .fetch_optional(conn)
            .await?;
    Ok(stats.map_or(0.0, |(a, d)| accuracy_bonus(a, d)))
}

/// A new ReviewableFlaggedPost (`needs_review!` when the post has none).
pub struct NewFlaggedPost {
    pub created_by_id: i32,
    pub post_id: i32,
    pub topic_id: i32,
    pub category_id: Option<i32>,
    pub post_user_id: Option<i32>,
    pub potential_spam: bool,
    pub potentially_illegal: bool,
    pub targets_topic: bool,
}

/// `ReviewableFlaggedPost.needs_review!`: the post's reviewable set back
/// to pending (potential_spam and potentially_illegal only ever turned
/// on), and when it had been reviewed its score recalculated and a
/// `transitioned` history logged by the flagger; a new one when the post
/// has none.
pub async fn flagged_post_needs_review(
    conn: &mut PgConnection,
    p: &crate::web_hooks::Payloads<'_>,
    r: &NewFlaggedPost,
) -> Result<InReview, AppError> {
    let old: Option<(i64, i32, f64, Option<i32>)> = sqlx::query_as(
        "UPDATE reviewables SET status = $1, \
                potential_spam = COALESCE($3, reviewables.potential_spam), \
                potentially_illegal = COALESCE($4, reviewables.potentially_illegal) \
         FROM reviewables AS old_reviewables \
         WHERE reviewables.target_id = $2 AND reviewables.target_type = 'Post' \
           AND reviewables.type = 'ReviewableFlaggedPost' AND old_reviewables.id = reviewables.id \
         RETURNING reviewables.id, old_reviewables.status, old_reviewables.score, reviewables.topic_id",
    )
    .bind(PENDING)
    .bind(r.post_id)
    .bind(r.potential_spam.then_some(true))
    .bind(r.potentially_illegal.then_some(true))
    .fetch_optional(&mut *conn)
    .await?;
    let Some((id, old_status, loaded_score, topic_id)) = old else {
        let id = create_flagged_post(conn, p, r).await?;
        return Ok(InReview {
            id,
            loaded_score: 0.0,
            topic_id: Some(r.topic_id),
        });
    };
    if old_status != PENDING {
        recalculate_score(&mut *conn, p, id).await?;
        // log_history(:transitioned, created_by), the status now pending.
        sqlx::query(
            "INSERT INTO reviewable_histories (reviewable_id, reviewable_history_type, status, \
                                               created_by_id, edited, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, NULL, clock_timestamp(), clock_timestamp())",
        )
        .bind(id)
        .bind(HISTORY_TRANSITIONED)
        .bind(PENDING)
        .bind(r.created_by_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(InReview {
        id,
        loaded_score,
        topic_id,
    })
}

/// The reviewable a flag scores, as `needs_review!` hands it on.
pub struct InReview {
    pub id: i64,
    /// Its score as loaded (0 for a new one). recalculate_score assigns
    /// the recalculated score, so add_score's `update(score:)` is a no-op
    /// for score when the sum lands back on this value: an ignored or
    /// disagreed flag's reviewable reopens at 0 and the new score is lost
    /// on the reviewable (not the topic), as on Rails.
    pub loaded_score: f64,
    pub topic_id: Option<i32>,
}

/// `Reviewable#recalculate_score`: the pending and agreed scores' sum,
/// then the topic's from its pending and approved reviewables, then
/// reviewable_score_updated.
pub async fn recalculate_score(
    conn: &mut PgConnection,
    p: &crate::web_hooks::Payloads<'_>,
    reviewable_id: i64,
) -> Result<(), AppError> {
    let topic_id: Option<i32> = sqlx::query_scalar(
        "UPDATE reviewables SET score = COALESCE(( \
             SELECT sum(score) FROM reviewable_scores AS rs \
             WHERE rs.reviewable_id = $1 AND rs.status IN ($2, $3)), 0.0) \
         WHERE id = $1 RETURNING topic_id",
    )
    .bind(reviewable_id)
    .bind(PENDING)
    .bind(AGREED)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE topics SET reviewable_score = COALESCE(( \
             SELECT SUM(score) FROM reviewables AS r \
             WHERE r.topic_id = $1 AND r.status IN ($2, $3)), 0.0) \
         WHERE id = $1",
    )
    .bind(topic_id)
    .bind(PENDING)
    .bind(APPROVED)
    .execute(&mut *conn)
    .await?;
    crate::web_hooks::enqueue_reviewable_hooks(
        &mut *conn,
        p,
        "reviewable_score_updated",
        reviewable_id,
    )
    .await
}

/// `ReviewableFlaggedPost.needs_review!` for a post with no reviewable:
/// saved, its `created` history logged, notify_reviewable enqueued on
/// commit as it is pending.
async fn create_flagged_post(
    conn: &mut PgConnection,
    p: &crate::web_hooks::Payloads<'_>,
    r: &NewFlaggedPost,
) -> Result<i64, AppError> {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO reviewables (type, type_source, status, created_by_id, reviewable_by_moderator, \
                                  reviewable_by_group_id, category_id, topic_id, score, potential_spam, \
                                  target_id, target_type, target_created_by_id, payload, version, \
                                  latest_score, force_review, potentially_illegal, created_at, updated_at) \
         VALUES ('ReviewableFlaggedPost', 'core', $1, $2, TRUE, NULL, $3, $4, 0, $5, $6, 'Post', $7, $8, 0, \
                 NULL, FALSE, $9, clock_timestamp(), clock_timestamp()) \
         RETURNING id",
    )
    .bind(PENDING)
    .bind(r.created_by_id)
    .bind(r.category_id)
    .bind(r.topic_id)
    .bind(r.potential_spam)
    .bind(r.post_id)
    .bind(r.post_user_id)
    .bind(json!({ "targets_topic": r.targets_topic }))
    .bind(r.potentially_illegal)
    .fetch_one(&mut *conn)
    .await?;
    // after_create log_history(:created, created_by)
    sqlx::query(
        "INSERT INTO reviewable_histories (reviewable_id, reviewable_history_type, status, created_by_id, \
                                           edited, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, NULL, clock_timestamp(), clock_timestamp())",
    )
    .bind(id)
    .bind(HISTORY_CREATED)
    .bind(PENDING)
    .bind(r.created_by_id)
    .execute(&mut *conn)
    .await?;
    // after_commit: reviewable_created's web hooks.
    crate::web_hooks::enqueue_reviewable_hooks(&mut *conn, p, "reviewable_created", id).await?;
    crate::jobs::enqueue(
        &mut *conn,
        "notify_reviewable",
        json!({ "reviewable_id": id }),
    )
    .await?;
    Ok(id)
}

/// A flag's score: its type, when it was made, and whether staff took
/// action with it.
pub struct NewScore {
    pub score_type: i64,
    pub created_at: chrono::NaiveDateTime,
    pub take_action: bool,
    pub reason: Option<&'static str>,
    pub context: Option<String>,
    /// `meta_topic_id:` the message a flag sent.
    pub meta_topic_id: Option<i32>,
}

/// `Reviewable#add_score(user, type, created_at:, take_action:, reason:,
/// context:, meta_topic_id:)` without forcing review. Returns the score
/// added.
pub async fn add_score(
    conn: &mut PgConnection,
    p: &crate::web_hooks::Payloads<'_>,
    reviewable: &InReview,
    user: &Flagger,
    new: &NewScore,
) -> Result<f64, AppError> {
    let reviewable_id = reviewable.id;
    let s = p.settings;
    let (score_type, created_at) = (new.score_type, new.created_at);
    let take_action_bonus = if new.take_action { 5.0 } else { 0.0 };
    let type_bonus: Option<f64> =
        sqlx::query_scalar("SELECT score_bonus FROM post_action_types WHERE id = $1")
            .bind(score_type as i32)
            .fetch_optional(&mut *conn)
            .await?;
    let accuracy = user_accuracy_bonus(&mut *conn, user).await?;
    // ReviewableScore.calculate_score: user_flag_score + type bonus +
    // take action bonus
    let user_flag_score =
        1.0 + if user.staff {
            5.0
        } else {
            f64::from(user.trust_level)
        } + accuracy;
    let score = (user_flag_score + type_bonus.unwrap_or(0.0) + take_action_bonus).max(0.0);
    sqlx::query(
        "INSERT INTO reviewable_scores (reviewable_id, user_id, reviewable_score_type, status, score, \
                                        take_action_bonus, user_accuracy_bonus, meta_topic_id, reason, \
                                        context, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $8, $6, $11, $9, $10, $7, clock_timestamp())",
    )
    .bind(reviewable_id)
    .bind(user.id)
    .bind(score_type as i32)
    .bind(PENDING)
    .bind(score)
    .bind(accuracy)
    .bind(created_at)
    .bind(take_action_bonus)
    .bind(new.reason)
    .bind(&new.context)
    .bind(new.meta_topic_id)
    .execute(&mut *conn)
    .await?;
    // update(score:, latest_score:, force_review:); still pending, so
    // notify_reviewable again on commit. Dirty tracking leaves score
    // alone when the sum equals the score as loaded (see InReview).
    sqlx::query(
        "UPDATE reviewables SET score = CASE WHEN score + $2 = $4 THEN score ELSE score + $2 END, \
                latest_score = $3, updated_at = clock_timestamp() \
         WHERE id = $1",
    )
    .bind(reviewable_id)
    .bind(score)
    .bind(created_at)
    .bind(reviewable.loaded_score)
    .execute(&mut *conn)
    .await?;
    crate::jobs::enqueue(
        &mut *conn,
        "notify_reviewable",
        json!({ "reviewable_id": reviewable_id }),
    )
    .await?;
    // reviewable_score_updated's web hooks.
    crate::web_hooks::enqueue_reviewable_hooks(
        &mut *conn,
        p,
        "reviewable_score_updated",
        reviewable_id,
    )
    .await?;
    // topic.update(reviewable_score: topic.reviewable_score + rs.score) if topic
    if let Some(topic_id) = reviewable.topic_id {
        update_topic_reviewable_score(conn, s, topic_id, score).await?;
    }
    Ok(score)
}

/// `topic.update(reviewable_score: ...)`: a validated save (see
/// `topic_save::reassigned_slug`), written when anything changed.
pub(crate) async fn update_topic_reviewable_score(
    conn: &mut PgConnection,
    s: &SiteSettings,
    topic_id: i32,
    delta: f64,
) -> Result<(), AppError> {
    let new_slug = crate::posting::topic_save::reassigned_slug(&mut *conn, s, topic_id).await?;
    sqlx::query(
        "UPDATE topics SET reviewable_score = reviewable_score + $2, slug = $3, fancy_title = NULL, \
                           updated_at = clock_timestamp() \
         WHERE id = $1 AND ($2 <> 0 OR slug IS DISTINCT FROM $3 OR fancy_title IS NOT NULL)",
    )
    .bind(topic_id)
    .bind(delta)
    .bind(&new_slug)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::accuracy_bonus;

    #[test]
    fn accuracy_bonus_like_rails() {
        assert_eq!(accuracy_bonus(3, 2), 0.0);
        // 10 agreed of 10: log4(10) * 5
        assert_eq!(accuracy_bonus(10, 0), 8.3);
        // 3 of 10: (0.7 - 0.3) / 0.7 * -log4(10) * 5
        assert_eq!(accuracy_bonus(3, 7), -4.75);
    }
}
