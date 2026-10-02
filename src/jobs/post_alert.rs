//! `Jobs::PostAlert` -> `PostAlerter#after_save_post`: who hears about a
//! post (mentioned users, the user replied to, the topic's author and
//! watchers, first-post watchers), each through `create_notification`
//! with its gates, collapsing, user action and notification email job.
//!
//! Refused rather than approximated: group and @here mentions, quotes,
//! links to topics, messages, nested replies, push notifications, do not
//! disturb and the notification consolidation plans for likes and links
//! (a first like is notified; one that would consolidate is refused).

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::posting::revisions::find_post;
use crate::posting::validate::analyze;
use crate::session::current::SessionUser;
use crate::{AppError, Unsupported};

/// `Notification.types` this job writes.
mod types {
    pub const MENTIONED: i32 = 1;
    pub const REPLIED: i32 = 2;
    pub const QUOTED: i32 = 3;
    pub const EDITED: i32 = 4;
    pub const LIKED: i32 = 5;
    pub const LIKED_CONSOLIDATED: i32 = 25;
    pub const POSTED: i32 = 9;
    pub const PRIVATE_MESSAGE: i32 = 6;
    pub const LINKED: i32 = 11;
    pub const WATCHING_FIRST_POST: i32 = 17;
    pub const WATCHING_CATEGORY_OR_TAG: i32 = 36;
}

const COLLAPSED: [i32; 4] = [
    types::REPLIED,
    types::POSTED,
    types::PRIVATE_MESSAGE,
    types::WATCHING_CATEGORY_OR_TAG,
];

/// The post being alerted about.
#[derive(sqlx::FromRow)]
struct Post {
    id: i32,
    user_id: Option<i32>,
    last_editor_id: Option<i32>,
    topic_id: i32,
    post_number: i32,
    post_type: i32,
    raw: String,
    cooked: String,
    reply_to_post_number: Option<i32>,
    action_code: Option<String>,
    username: Option<String>,
    name: Option<String>,
    editor_username: Option<String>,
    topic_title: String,
    topic_user_id: Option<i32>,
    category_id: Option<i32>,
    archetype: String,
}

/// Options `create_notification` takes from its callers.
#[derive(Default, Clone)]
struct Opts {
    /// The editor who added a mention (user_id, original/display username).
    user_id: Option<i32>,
    display_username: Option<String>,
    original_username: Option<String>,
    /// The like a liked notification is for.
    post_action_id: Option<i32>,
}

/// The post and what create_notification reads about it; None when it or
/// its topic is gone.
async fn load_post(conn: &mut PgConnection, post_id: i32) -> Result<Option<Post>, AppError> {
    Ok(sqlx::query_as(
        "SELECT p.id, p.user_id, p.last_editor_id, p.topic_id, p.post_number, p.post_type, p.raw, p.cooked, \
                p.reply_to_post_number, p.action_code, u.username, u.name, e.username AS editor_username, \
                t.title AS topic_title, t.user_id AS topic_user_id, t.category_id, t.archetype \
         FROM posts p JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
         LEFT JOIN users u ON u.id = p.user_id \
         LEFT JOIN users e ON e.id = COALESCE(p.last_editor_id, p.user_id) \
         WHERE p.id = $1 AND p.deleted_at IS NULL",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?)
}

/// `PostActionNotifier.post_action_created` for a like: the post's author
/// hears who liked it.
pub async fn notify_liked(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    post_id: i32,
    liker_id: i32,
    liker_username: &str,
    post_action_id: i32,
) -> Result<(), AppError> {
    let Some(post) = load_post(conn, post_id).await? else {
        return Ok(());
    };
    let Some(author) = post.user_id else {
        return Ok(());
    };
    let mut alerter = Alerter {
        ctx,
        post: &post,
        notified: Vec::new(),
    };
    let opts = Opts {
        user_id: Some(liker_id),
        display_username: Some(liker_username.to_string()),
        original_username: None,
        post_action_id: Some(post_action_id),
    };
    alerter
        .create_notification(conn, author, types::LIKED, &opts)
        .await?;
    Ok(())
}

pub async fn run(ctx: &Ctx<'_>, conn: &mut PgConnection, args: &Value) -> Result<(), AppError> {
    let Some(post_id) = args.get("post_id").and_then(Value::as_i64) else {
        return Ok(());
    };
    if args
        .get("options")
        .is_some_and(|o| !o.is_null() && o != &json!({}))
    {
        return Err(Unsupported("post_alert options").into());
    }
    let new_record = args.get("new_record") == Some(&json!(true));
    let post = load_post(conn, post_id as i32).await?;
    let Some(post) = post else {
        return Ok(());
    };
    if post.raw.trim().is_empty() {
        return Ok(());
    }
    let s = ctx.settings;
    if post.archetype == "private_message" {
        return Err(Unsupported("alerts for messages").into());
    }
    if s.get("nested_replies_enabled")?.truthy() {
        return Err(Unsupported("alerts with nested replies").into());
    }
    let mut alerter = Alerter {
        ctx,
        post: &post,
        notified: Vec::new(),
    };
    alerter.notified.extend(post.user_id);
    if let Some(editor) = post.last_editor_id
        && !alerter.notified.contains(&editor)
    {
        alerter.notified.push(editor);
    }

    // mentions
    let analysis = analyze(&post.cooked, ctx.config.globals.relative_url_root())?;
    let mentions = analysis.mention_names;
    if !mentions.is_empty() {
        let groups: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM groups WHERE LOWER(name) = ANY($1)")
                .bind(&mentions)
                .fetch_one(&mut *conn)
                .await?;
        if groups > 0 {
            return Err(Unsupported("group mentions").into());
        }
        if mentions.contains(&s.get("here_mention")?.to_s()) {
            return Err(Unsupported("@here mentions").into());
        }
        let mut opts = Opts::default();
        if post.last_editor_id != post.user_id {
            opts.user_id = post.last_editor_id;
            opts.original_username = post.editor_username.clone();
            opts.display_username = post.editor_username.clone();
        }
        let users: Vec<i32> = sqlx::query_scalar(
            "SELECT id FROM users WHERE username_lower = ANY($1) AND id IS DISTINCT FROM $2 ORDER BY id",
        )
        .bind(&mentions)
        .bind(post.user_id)
        .fetch_all(&mut *conn)
        .await?;
        let targets: Vec<i32> = users
            .into_iter()
            .filter(|u| !alerter.notified.contains(u))
            .collect();
        for user_id in targets {
            if alerter
                .create_notification(conn, user_id, types::MENTIONED, &opts)
                .await?
            {
                alerter.notified.push(user_id);
            }
        }
    }

    // replies
    let notify_about_reply = post.post_type == crate::posting::post_types::REGULAR
        || (post.post_type == crate::posting::post_types::WHISPER && post.action_code.is_none());
    if new_record && notify_about_reply {
        // reply_notification_target
        if let Some(n) = post.reply_to_post_number {
            let target: Option<Option<i32>> = sqlx::query_scalar(
                "SELECT user_id FROM posts WHERE topic_id = $1 AND post_number = $2 \
                 AND user_id IS DISTINCT FROM $3 AND deleted_at IS NULL LIMIT 1",
            )
            .bind(post.topic_id)
            .bind(n)
            .bind(post.user_id)
            .fetch_optional(&mut *conn)
            .await?;
            if let Some(Some(target)) = target
                && !alerter.notified.contains(&target)
                && alerter
                    .create_notification(conn, target, types::REPLIED, &Opts::default())
                    .await?
            {
                alerter.notified.push(target);
            }
        }
        if let Some(author) = post.topic_user_id {
            let watching: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM topic_users WHERE user_id = $1 AND topic_id = $2 \
                 AND notification_level = 3)",
            )
            .bind(author)
            .bind(post.topic_id)
            .fetch_one(&mut *conn)
            .await?;
            if !alerter.notified.contains(&author)
                && watching
                && alerter
                    .create_notification(conn, author, types::REPLIED, &Opts::default())
                    .await?
            {
                alerter.notified.push(author);
            }
        }
    }

    // quotes and links
    if post.raw.contains("[quote=") {
        return Err(Unsupported("alerts for quotes").into());
    }
    let topic_links: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_links WHERE post_id = $1 AND NOT reflection \
           AND (link_post_id IS NOT NULL OR link_topic_id IS NOT NULL))",
    )
    .bind(post.id)
    .fetch_one(&mut *conn)
    .await?;
    if topic_links {
        return Err(Unsupported("alerts for links to topics").into());
    }

    // category_or_tag_muters join the notified.
    let muters: Vec<i32> = sqlx::query_scalar(
        "SELECT uo.user_id FROM user_options uo \
         LEFT JOIN topic_users tus ON tus.user_id = uo.user_id AND tus.topic_id = $1 \
         LEFT JOIN category_users cu ON cu.user_id = uo.user_id AND cu.category_id = $2 \
         LEFT JOIN tag_users tu ON tu.user_id = uo.user_id \
         JOIN topic_tags tt ON tt.tag_id = tu.tag_id AND tt.topic_id = $1 \
         WHERE (tus.id IS NULL OR tus.notification_level != 3) \
           AND (cu.notification_level = 0 OR tu.notification_level = 0) \
           AND uo.watched_precedence_over_muted IS false",
    )
    .bind(post.topic_id)
    .bind(post.category_id.unwrap_or(0))
    .fetch_all(&mut *conn)
    .await?;
    alerter.notified.extend(muters);

    if new_record && notify_about_reply {
        // Topic watchers, then category and tag watchers.
        alerter.notify_post_users(conn, true, false).await?;
        alerter.notify_post_users(conn, false, true).await?;
    }

    // sync_group_mentions: no group mentions, so the post has none.
    sqlx::query("DELETE FROM group_mentions WHERE post_id = $1")
        .bind(post.id)
        .execute(&mut *conn)
        .await?;

    if new_record && post.post_number == 1 {
        let tags: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM topic_tags WHERE topic_id = $1)")
                .bind(post.topic_id)
                .fetch_one(&mut *conn)
                .await?;
        if tags {
            return Err(Unsupported("first post alerts for tagged topics").into());
        }
        let watchers: Vec<i32> = sqlx::query_scalar(
            "SELECT DISTINCT user_id FROM category_users WHERE category_id = $1 AND notification_level = 4 \
             AND user_id IS DISTINCT FROM $2 AND user_id IS DISTINCT FROM $3 ORDER BY user_id",
        )
        .bind(post.category_id)
        .bind(post.user_id)
        .bind(post.last_editor_id)
        .fetch_all(&mut *conn)
        .await?;
        for user_id in watchers {
            if alerter.notified.contains(&user_id) {
                continue;
            }
            if alerter
                .create_notification(conn, user_id, types::WATCHING_FIRST_POST, &Opts::default())
                .await?
            {
                alerter.notified.push(user_id);
            }
        }
    }
    Ok(())
}

struct Alerter<'a> {
    ctx: &'a Ctx<'a>,
    post: &'a Post,
    notified: Vec<i32>,
}

impl Alerter<'_> {
    /// `notify_post_users`: topic watchers (`topic`), or category watchers
    /// as watching_category_or_tag (`category`).
    async fn notify_post_users(
        &mut self,
        conn: &mut PgConnection,
        topic: bool,
        category: bool,
    ) -> Result<(), AppError> {
        let post = self.post;
        let mut sql = String::from("SELECT id FROM users WHERE false");
        if topic {
            sql.push_str(
                " UNION SELECT user_id FROM topic_users WHERE notification_level = 3 AND topic_id = $1",
            );
        }
        if category {
            sql.push_str(
                " UNION SELECT cu.user_id FROM category_users cu \
                  LEFT JOIN topic_users tu ON tu.user_id = cu.user_id AND tu.topic_id = $1 \
                  WHERE cu.notification_level = 3 AND cu.category_id = $2 \
                    AND (tu.user_id IS NULL OR tu.notification_level = 3)",
            );
            // Tag watchers, through the tag group permissions (everyone is
            // group 0, staff group 3).
            sql.push_str(
                " UNION SELECT tag_users.user_id FROM tag_users \
                  LEFT JOIN topic_users tu ON tu.user_id = tag_users.user_id AND tu.topic_id = $1 \
                  LEFT JOIN tag_group_memberships tgm ON tag_users.tag_id = tgm.tag_id \
                  LEFT JOIN tag_group_permissions tgp ON tgm.tag_group_id = tgp.tag_group_id \
                  LEFT JOIN group_users gu ON gu.user_id = tag_users.user_id \
                  WHERE (tgp.group_id IS NULL OR tgp.group_id = gu.group_id OR tgp.group_id = 0 OR gu.group_id = 3) \
                    AND tag_users.notification_level = 3 \
                    AND tag_users.tag_id IN (SELECT tag_id FROM topic_tags WHERE topic_id = $1) \
                    AND (tu.user_id IS NULL OR tu.notification_level = 3)",
            );
        }
        let users: Vec<i32> = sqlx::query_scalar(&format!(
            "SELECT id FROM users WHERE id IN ({sql}) AND NOT (id = ANY($3)) ORDER BY id"
        ))
        .bind(post.topic_id)
        .bind(post.category_id.unwrap_or(0))
        .bind(&self.notified)
        .fetch_all(&mut *conn)
        .await?;
        let notification_type = if category {
            types::WATCHING_CATEGORY_OR_TAG
        } else {
            types::POSTED
        };
        for user_id in users {
            if self
                .create_notification(conn, user_id, notification_type, &Opts::default())
                .await?
            {
                self.notified.push(user_id);
            }
        }
        Ok(())
    }

    /// `create_notification(user, type, post, opts)`; whether one was
    /// created.
    async fn create_notification(
        &mut self,
        conn: &mut PgConnection,
        user_id: i32,
        notification_type: i32,
        opts: &Opts,
    ) -> Result<bool, AppError> {
        let post = self.post;
        let ctx = self.ctx;
        if user_id <= 0 {
            return Ok(false);
        }
        let Some(user) = SessionUser::load(&mut *conn, user_id).await? else {
            return Ok(false);
        };
        // like_notification_frequency: always 0, first_time_and_daily 1,
        // first_time 2, never 3.
        let like_frequency: i32 = if notification_type == types::LIKED {
            sqlx::query_scalar(
                "SELECT like_notification_frequency FROM user_options WHERE user_id = $1",
            )
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or(1)
        } else {
            1
        };
        if notification_type == types::LIKED && like_frequency == 3 {
            return Ok(false);
        }
        if notification_type == types::LINKED {
            return Err(Unsupported("linked notifications").into());
        }
        // can_receive_post_notifications?
        let guardian = Guardian::for_user(&mut *conn, &user).await?;
        if guardian.is_admin()
            && ctx
                .settings
                .get("suppress_secured_categories_from_admin")?
                .truthy()
        {
            return Err(Unsupported("suppress_secured_categories_from_admin").into());
        }
        if find_post(&mut *conn, ctx, &guardian, post.id)
            .await?
            .is_none()
        {
            return Ok(false);
        }
        if user.staged {
            return Err(Unsupported("notifying staged users").into());
        }
        // The notifier muted or ignored by the user (staff can't be).
        let notifier = opts.user_id.or(post.user_id);
        if let Some(notifier) = notifier {
            let screened: bool = sqlx::query_scalar(
                "SELECT NOT COALESCE((SELECT admin OR moderator FROM users WHERE id = $2), FALSE) AND ( \
                   EXISTS (SELECT 1 FROM muted_users WHERE user_id = $1 AND muted_user_id = $2) \
                   OR EXISTS (SELECT 1 FROM ignored_users WHERE user_id = $1 AND ignored_user_id = $2))",
            )
            .bind(user_id)
            .bind(notifier)
            .fetch_one(&mut *conn)
            .await?;
            if screened {
                return Ok(false);
            }
        }
        let muted: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topic_users WHERE topic_id = $1 AND user_id = $2 AND notification_level = 0)",
        )
        .bind(post.topic_id)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if muted {
            return Ok(false);
        }
        let existing: Vec<i32> = sqlx::query_scalar(
            "SELECT notification_type FROM notifications WHERE user_id = $1 AND topic_id = $2 \
             AND post_number = $3 ORDER BY id DESC LIMIT 10",
        )
        .bind(user_id)
        .bind(post.topic_id)
        .bind(post.post_number)
        .fetch_all(&mut *conn)
        .await?;
        if existing.contains(&notification_type) {
            if notification_type == types::EDITED {
                return Err(Unsupported("repeated edit notifications").into());
            }
            // should_notify_like?: always, or first_time_and_daily once the
            // last one is a day old.
            let renotify = notification_type == types::LIKED && match like_frequency {
                0 => return Err(Unsupported("likes notified every time (liked_by_two_users)").into()),
                1 => sqlx::query_scalar::<_, bool>(
                    "SELECT created_at < now() - interval '1 day' FROM notifications WHERE user_id = $1 \
                     AND topic_id = $2 AND post_number = $3 AND notification_type = $4 ORDER BY id DESC LIMIT 1",
                )
                .bind(user_id)
                .bind(post.topic_id)
                .bind(post.post_number)
                .bind(types::LIKED)
                .fetch_one(&mut *conn)
                .await?,
                _ => false,
            };
            if !renotify {
                return Ok(false);
            }
        }
        if [types::QUOTED, types::LINKED, types::MENTIONED].contains(&notification_type)
            && existing.contains(&types::REPLIED)
        {
            return Ok(false);
        }

        let mut target_post_number = post.post_number;
        let mut display_username = opts.display_username.clone();
        let original_username = opts
            .display_username
            .clone()
            .filter(|u| !u.is_empty())
            .or_else(|| post.username.clone());
        if COLLAPSED.contains(&notification_type) {
            // destroy_notifications(user, COLLAPSED, topic) when the user can
            // see the topic (checked above), then the first unread post.
            sqlx::query(
                "DELETE FROM notifications WHERE user_id = $1 AND topic_id = $2 AND notification_type = ANY($3)",
            )
            .bind(user_id)
            .bind(post.topic_id)
            .bind(&COLLAPSED[..])
            .execute(&mut *conn)
            .await?;
            let visible = guardian.visible_post_types(ctx.settings)?;
            let (first, count): (Option<i32>, i64) = sqlx::query_as(
                "SELECT MIN(post_number), COUNT(*) FROM posts WHERE topic_id = $2 AND deleted_at IS NULL \
                   AND post_type = ANY($6) \
                   AND post_number > COALESCE((SELECT last_read_post_number FROM topic_users tu \
                                               WHERE tu.user_id = $1 AND tu.topic_id = $2), 0) \
                   AND (reply_to_user_id = $1 \
                        OR EXISTS (SELECT 1 FROM topic_users tu WHERE tu.user_id = $1 AND tu.topic_id = $2 \
                                   AND notification_level = $4) \
                        OR EXISTS (SELECT 1 FROM category_users cu WHERE cu.user_id = $1 AND cu.category_id = $3 \
                                   AND notification_level = $4) \
                        OR EXISTS (SELECT 1 FROM tag_users tu WHERE tu.user_id = $1 \
                                   AND tu.tag_id IN (SELECT tag_id FROM topic_tags WHERE topic_id = $2) \
                                   AND notification_level = $5))",
            )
            .bind(user_id)
            .bind(post.topic_id)
            .bind(post.category_id)
            .bind(3)
            .bind(3)
            .bind(&visible)
            .fetch_one(&mut *conn)
            .await?;
            if let Some(first) = first {
                target_post_number = first;
            }
            if count > 1 {
                let n = count.to_string();
                display_username = Some(
                    ctx.i18n
                        .t_with("embed.replies.other", &[("count", &n)])
                        .unwrap_or_else(|| format!("{count} replies")),
                );
            }
        }

        // UserActionManager.notification_created
        let action = match notification_type {
            types::QUOTED => Some(9),
            types::REPLIED => Some(6),
            types::MENTIONED => Some(7),
            types::EDITED => Some(11),
            types::LINKED => Some(17),
            _ => None,
        };
        if let Some(action) = action {
            sqlx::query(
                "INSERT INTO user_actions (action_type, user_id, acting_user_id, target_topic_id, target_post_id, \
                                           created_at, updated_at) \
                 SELECT $1, $2, $3, $4, $5, clock_timestamp(), clock_timestamp() \
                 WHERE NOT EXISTS (SELECT 1 FROM user_actions WHERE action_type = $1 AND user_id = $2 \
                   AND acting_user_id = $3 AND target_topic_id = $4 AND target_post_id = $5)",
            )
            .bind(action)
            .bind(user_id)
            .bind(post.user_id)
            .bind(post.topic_id)
            .bind(post.id)
            .execute(&mut *conn)
            .await?;
        }

        // The data, in Rails' key order.
        let displayed = display_username.clone().or_else(|| post.username.clone());
        let mut data = Map::new();
        data.insert("topic_title".into(), json!(post.topic_title));
        data.insert("original_post_id".into(), json!(post.id));
        data.insert("original_post_type".into(), json!(post.post_type));
        data.insert("original_username".into(), json!(original_username));
        data.insert("revision_number".into(), Value::Null);
        data.insert("display_username".into(), json!(displayed));
        if let Some(name) = &post.name {
            data.insert("display_name".into(), json!(name));
        }
        let data = Value::Object(data);
        if target_post_number != post.post_number {
            return Err(Unsupported("collapsed notifications pointing at an earlier post").into());
        }
        // consolidate_or_create!: the liked plan rolls one liker's likes into a
        // liked_consolidated notification within the window; not ported.
        if notification_type == types::LIKED {
            let window = ctx
                .settings
                .get("likes_notification_consolidation_window_mins")?
                .to_i();
            let (consolidated, unconsolidated): (bool, i64) = sqlx::query_as(
                "SELECT EXISTS (SELECT 1 FROM notifications WHERE user_id = $1 AND notification_type = $4 \
                   AND data::json ->> 'display_username' = $2 \
                   AND created_at > now() - make_interval(mins => $3)), \
                        (SELECT COUNT(*) FROM notifications WHERE user_id = $1 AND notification_type = $5 \
                   AND data::json ->> 'username2' IS NULL AND data::json ->> 'display_username' = $2 \
                   AND created_at > now() - make_interval(mins => $3))",
            )
            .bind(user_id)
            .bind(&displayed)
            .bind(window as i32)
            .bind(types::LIKED_CONSOLIDATED)
            .bind(types::LIKED)
            .fetch_one(&mut *conn)
            .await?;
            let threshold = ctx
                .settings
                .get("notification_consolidation_threshold")?
                .to_i();
            if consolidated || unconsolidated + 1 >= threshold {
                return Err(Unsupported("consolidated like notifications").into());
            }
        }
        let notification_id: i64 = sqlx::query_scalar(
            "INSERT INTO notifications (notification_type, user_id, topic_id, post_number, data, read, \
                                        high_priority, post_action_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, FALSE, FALSE, $6, clock_timestamp(), clock_timestamp()) RETURNING id",
        )
        .bind(notification_type)
        .bind(user_id)
        .bind(post.topic_id)
        .bind(target_post_number)
        .bind(data.to_string())
        .bind(opts.post_action_id)
        .fetch_one(&mut *conn)
        .await?;

        // after_commit send_email: NotificationEmailer.process_notification.
        let (dnd, email_level, active, approved): (bool, Option<i32>, bool, bool) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM do_not_disturb_timings WHERE user_id = $1 \
                              AND starts_at <= now() AND ends_at > now()), \
                    (SELECT email_level FROM user_options WHERE user_id = $1), u.active, u.approved \
             FROM users u WHERE u.id = $1",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if dnd {
            return Err(Unsupported("do not disturb (shelved notifications)").into());
        }
        let email_type = match notification_type {
            types::MENTIONED => Some("user_mentioned"),
            types::POSTED | types::WATCHING_CATEGORY_OR_TAG => Some("user_posted"),
            types::QUOTED => Some("user_quoted"),
            types::REPLIED => Some("user_replied"),
            types::LINKED => Some("user_linked"),
            types::WATCHING_FIRST_POST => Some("user_watching_first_post"),
            _ => None,
        };
        // email_level never (2), inactive users and unapproved ones under
        // must_approve_users get no email.
        let emailable = email_level != Some(2)
            && active
            && (approved || !ctx.settings.get("must_approve_users")?.truthy())
            && [1, 2, 4].contains(&post.post_type);
        if let (Some(email_type), true) = (email_type, emailable) {
            let type_name = match notification_type {
                types::MENTIONED => "mentioned",
                types::REPLIED => "replied",
                types::QUOTED => "quoted",
                types::POSTED => "posted",
                types::LINKED => "linked",
                types::WATCHING_FIRST_POST => "watching_first_post",
                types::WATCHING_CATEGORY_OR_TAG => "watching_category_or_tag",
                _ => "",
            };
            super::enqueue_in(
                &mut *conn,
                ctx.settings.get("email_time_window_mins")?.to_i() * 60,
                "user_email",
                json!({
                    "type": email_type,
                    "user_id": user_id,
                    "notification_id": notification_id,
                    "notification_data_hash": data,
                    "notification_type": type_name,
                    "post_id": post.id,
                }),
            )
            .await?;
        }
        // create_notification_alert: MessageBus (not ported) and push
        // notifications, refused for users who have push set up.
        if existing.is_empty() && !user.suspended() {
            let push: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM push_subscriptions WHERE user_id = $1) \
                 OR EXISTS (SELECT 1 FROM user_api_keys k \
                            WHERE k.user_id = $1 AND k.revoked_at IS NULL AND k.push_url IS NOT NULL)",
            )
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
            if push {
                return Err(Unsupported("push notifications").into());
            }
        }
        Ok(true)
    }
}
