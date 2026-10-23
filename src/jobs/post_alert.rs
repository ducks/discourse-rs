//! `Jobs::PostAlert` -> `PostAlerter#after_save_post`: who hears about a
//! post (mentioned users, the user replied to, quoted users, the topic's
//! author and watchers, first-post watchers), each through
//! `create_notification` with its gates, collapsing, user action and
//! notification email job.
//!
//! Likes and discourse-reactions' reactions are notified through it too,
//! with their consolidation plans.
//!
//! Messages notify their directly targeted users, and their groups'
//! members by their level on it, with the groups' inbox summaries.
//!
//! Refused rather than approximated: group SMTP and push notifications.
//! The notifications themselves go through `notification_create`.

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
    pub const POSTED: i32 = 9;
    pub const PRIVATE_MESSAGE: i32 = 6;
    pub const LINKED: i32 = 11;
    pub const GROUP_MENTIONED: i32 = 15;
    pub const WATCHING_FIRST_POST: i32 = 17;
    pub const WATCHING_CATEGORY_OR_TAG: i32 = 36;
    /// discourse-reactions' type.
    pub const REACTION: i32 = 25;
}

/// `TagUser.notification_levels[:watching_first_post]`
const WATCHING_FIRST_POST_LEVEL: i32 = 4;

const COLLAPSED: [i32; 4] = [
    types::REPLIED,
    types::POSTED,
    types::PRIVATE_MESSAGE,
    types::WATCHING_CATEGORY_OR_TAG,
];

/// `PostAlerter::NOTIFIABLE_TYPES`: the types that alert the user live
/// and by push. Event reminders and invitations come from a plugin, so
/// they are not here.
const NOTIFIABLE: [i32; 9] = [
    types::MENTIONED,
    types::REPLIED,
    types::QUOTED,
    types::POSTED,
    types::LINKED,
    types::PRIVATE_MESSAGE,
    types::GROUP_MENTIONED,
    types::WATCHING_FIRST_POST,
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
    via_email: bool,
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
    /// `display_name`; the post author's name when None.
    display_name: Option<String>,
    /// `custom_data`, merged into the data last.
    custom_data: Map<String, Value>,
    /// `acting_user_id`: who the user action names (the author otherwise).
    acting_user_id: Option<i32>,
    /// `revision_number`, an edit's.
    revision_number: Option<i32>,
    /// `group`: the group mentioned (id, name).
    group: Option<(i32, String)>,
}

/// The post and what create_notification reads about it; None when it or
/// its topic is gone.
async fn load_post(conn: &mut PgConnection, post_id: i32) -> Result<Option<Post>, AppError> {
    Ok(sqlx::query_as(
        "SELECT p.id, p.user_id, p.last_editor_id, p.topic_id, p.post_number, p.post_type, p.raw, p.cooked, \
                p.reply_to_post_number, p.action_code, u.username, u.name, e.username AS editor_username, \
                t.title AS topic_title, t.user_id AS topic_user_id, t.category_id, t.archetype, p.via_email \
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
    let mut alerter = Alerter::new(ctx, &post);
    let opts = Opts {
        user_id: Some(liker_id),
        display_username: Some(liker_username.to_string()),
        original_username: None,
        post_action_id: Some(post_action_id),
        ..Default::default()
    };
    alerter
        .create_notification(conn, author, types::LIKED, &opts)
        .await?;
    Ok(())
}

/// `ReactionNotification#create`: the post's author hears who reacted
/// (`custom_data` carries the heart's icon), unless their like
/// notification frequency is never.
pub async fn notify_reaction(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    post_id: i32,
    reactor_id: i32,
    reactor_username: &str,
    reactor_name: Option<&str>,
    custom_data: Map<String, Value>,
) -> Result<(), AppError> {
    let Some(post) = load_post(conn, post_id).await? else {
        return Ok(());
    };
    let Some(author) = post.user_id else {
        return Ok(());
    };
    let never: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_options WHERE user_id = $1 AND like_notification_frequency = 3)",
    )
    .bind(author)
    .fetch_one(&mut *conn)
    .await?;
    if never {
        return Ok(());
    }
    let mut alerter = Alerter::new(ctx, &post);
    let opts = Opts {
        user_id: Some(reactor_id),
        display_username: Some(reactor_username.to_string()),
        display_name: reactor_name.map(str::to_string),
        custom_data,
        ..Default::default()
    };
    alerter
        .create_notification(conn, author, types::REACTION, &opts)
        .await?;
    Ok(())
}

pub async fn run(ctx: &Ctx<'_>, conn: &mut PgConnection, args: &Value) -> Result<(), AppError> {
    let Some(post_id) = args.get("post_id").and_then(Value::as_i64) else {
        return Ok(());
    };
    // `args[:options] || {}`, PostAlerter's default opts, merged under
    // every create_notification's own. Callers pass only skip_send_email
    // (the silenced user's system message, discourse-narrative-bot).
    let mut skip_send_email = false;
    match args.get("options") {
        None | Some(Value::Null) => {}
        Some(Value::Object(options)) => {
            for (key, value) in options {
                if key != "skip_send_email" {
                    return Err(Unsupported("post_alert options other than skip_send_email").into());
                }
                skip_send_email = !matches!(value, Value::Null | Value::Bool(false));
            }
        }
        Some(_) => return Err(Unsupported("post_alert options that are not a hash").into()),
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
    let private_message = post.archetype == "private_message";
    // allowed_users and allowed_group_users, neither the author nor a bot
    // (not_allowed?); the group members once per membership, as the join
    // gives them.
    let not_allowed = |u: i32| u <= 0 || Some(u) == post.user_id;
    let (allowed_users, allowed_group_users): (Vec<(i32, bool)>, Vec<i32>) = if private_message {
        let users: Vec<(i32, bool)> = sqlx::query_as(
            "SELECT u.id, u.staged FROM topic_allowed_users tau JOIN users u ON u.id = tau.user_id \
             WHERE tau.topic_id = $1 ORDER BY tau.id",
        )
        .bind(post.topic_id)
        .fetch_all(&mut *conn)
        .await?;
        let group_users: Vec<i32> = sqlx::query_scalar(
            "SELECT gu.user_id FROM topic_allowed_groups tag JOIN group_users gu ON gu.group_id = tag.group_id \
             WHERE tag.topic_id = $1 ORDER BY tag.id, gu.id",
        )
        .bind(post.topic_id)
        .fetch_all(&mut *conn)
        .await?;
        (
            users
                .into_iter()
                .filter(|(u, _)| !not_allowed(*u))
                .collect(),
            group_users
                .into_iter()
                .filter(|u| !not_allowed(*u))
                .collect(),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    // directly_targeted_users: allowed users outside the allowed groups.
    let direct: Vec<(i32, bool)> = allowed_users
        .iter()
        .filter(|(u, _)| !allowed_group_users.contains(u))
        .copied()
        .collect();
    // pm_watching_users: the directly targeted watching the message.
    let pm_watching: Vec<i32> = if private_message {
        let ids: Vec<i32> = direct.iter().map(|(u, _)| *u).collect();
        sqlx::query_scalar(
            "SELECT user_id FROM topic_users WHERE topic_id = $1 AND notification_level = 3 \
             AND user_id = ANY($2)",
        )
        .bind(post.topic_id)
        .bind(&ids)
        .fetch_all(&mut *conn)
        .await?
    } else {
        Vec::new()
    };
    // all_allowed_users: moderators join them in a flagged message
    // (has_flags?: a pending ReviewableFlaggedPost scored at least
    // min_score_for_priority) or an official warning.
    let mut all_allowed: Vec<i32> = allowed_users
        .iter()
        .map(|(u, _)| *u)
        .chain(allowed_group_users.iter().copied())
        .collect();
    if private_message {
        let min_score = crate::reviewables::min_score_for_priority(&mut *conn, s).await?;
        let with_moderators: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND subtype = 'moderator_warning') \
             OR EXISTS (SELECT 1 FROM reviewables WHERE topic_id = $1 \
                        AND type = 'ReviewableFlaggedPost' AND status = 0 AND score >= $2)",
        )
        .bind(post.topic_id)
        .bind(min_score)
        .fetch_one(&mut *conn)
        .await?;
        if with_moderators {
            let moderators: Vec<i32> =
                sqlx::query_scalar("SELECT id FROM users WHERE moderator ORDER BY id")
                    .fetch_all(&mut *conn)
                    .await?;
            all_allowed.extend(moderators.into_iter().filter(|u| !not_allowed(*u)));
        }
    }
    let mut alerter = Alerter::new(ctx, &post);
    alerter.skip_send_email = skip_send_email;
    alerter.notified.extend(post.user_id);
    if let Some(editor) = post.last_editor_id
        && !alerter.notified.contains(&editor)
    {
        alerter.notified.push(editor);
    }

    // mentions
    let analysis = analyze(&post.cooked, ctx.config.globals.relative_url_root())?;
    let mentions = analysis.mention_names;
    // extract_mentions: the groups named, the rest as users.
    let mentioned_groups: Vec<(i32, String)> = if mentions.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as("SELECT id, name FROM groups WHERE LOWER(name) = ANY($1) ORDER BY ctid")
            .bind(&mentions)
            .fetch_all(&mut *conn)
            .await?
    };
    let group_names: Vec<String> = mentioned_groups
        .iter()
        .map(|(_, name)| name.to_lowercase())
        .collect();
    let mentions: Vec<String> = mentions
        .into_iter()
        .filter(|m| !group_names.contains(m))
        .collect();
    if !mentions.is_empty() || !mentioned_groups.is_empty() {
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
        // only_allowed_users and, in a message, not its watchers.
        let targets: Vec<i32> = users
            .into_iter()
            .filter(|u| !private_message || all_allowed.contains(u))
            .filter(|u| !pm_watching.contains(u))
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
        // expand_group_mentions: the groups the author may mention (public
        // ones not included), each under max_users_notified_per_group_mention
        // members; its members allowed in a message and not notified yet
        // hear of it as group_mentioned.
        for (group_id, group_name, members) in
            expand_group_mentions(ctx, conn, &post, &mentioned_groups).await?
        {
            let targets: Vec<i32> = members
                .into_iter()
                .filter(|u| !private_message || all_allowed.contains(u))
                .filter(|u| !alerter.notified.contains(u))
                .collect();
            let group_opts = Opts {
                group: Some((group_id, group_name)),
                ..opts.clone()
            };
            let mut reached = Vec::new();
            for user_id in targets {
                if alerter
                    .create_notification(conn, user_id, types::GROUP_MENTIONED, &group_opts)
                    .await?
                {
                    reached.push(user_id);
                }
            }
            alerter.notified.extend(reached);
        }
        // @here, when the author can_mention_here?: expand_here_mention's
        // posters not notified yet, only those allowed in a message.
        if mentions.contains(&s.get("here_mention")?.to_s())
            && can_mention_here(ctx, conn, &post).await?
        {
            let users = expand_here_mention(ctx, conn, &post, &alerter.notified).await?;
            let targets: Vec<i32> = users
                .into_iter()
                .filter(|u| !private_message || all_allowed.contains(u))
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
    }

    // replies (notify_non_pm_users: none in a message)
    let notify_about_reply = post.post_type == crate::posting::post_types::REGULAR
        || (post.post_type == crate::posting::post_types::WHISPER && post.action_code.is_none());
    // reply_notification_target
    // (the topic's first post for a root reply in a nested topic)
    let reply_to = match post.reply_to_post_number {
        None if nested_view(ctx, conn, &post).await? => Some(1),
        n => n,
    };
    let reply_target: Option<i32> = match reply_to {
        Some(n) => sqlx::query_scalar::<_, Option<i32>>(
            "SELECT user_id FROM posts WHERE topic_id = $1 AND post_number = $2 \
             AND user_id IS DISTINCT FROM $3 AND deleted_at IS NULL LIMIT 1",
        )
        .bind(post.topic_id)
        .bind(n)
        .bind(post.user_id)
        .fetch_optional(&mut *conn)
        .await?
        .flatten(),
        None => None,
    };
    if new_record && notify_about_reply && !private_message {
        if let Some(target) = reply_target
            && !alerter.notified.contains(&target)
            && alerter
                .create_notification(conn, target, types::REPLIED, &Opts::default())
                .await?
        {
            alerter.notified.push(target);
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

    // quotes (notify_non_pm_users: none in a message)
    let quoted_users = extract_quoted_users(ctx, conn, &post).await?;
    if !private_message {
        let targets: Vec<i32> = quoted_users
            .iter()
            .copied()
            .filter(|u| !alerter.notified.contains(u))
            .collect();
        let mut reached = Vec::new();
        for user_id in targets {
            if alerter
                .create_notification(conn, user_id, types::QUOTED, &Opts::default())
                .await?
            {
                reached.push(user_id);
            }
        }
        alerter.notified.extend(reached);
    }

    // linked (none in a message either): extract_linked_users, the
    // authors of the posts (or first posts of the topics) linked to, once
    // per link; those not notified yet hear of it.
    let linked: Vec<i32> = sqlx::query_scalar(
        "SELECT COALESCE(lp.user_id, fp.user_id) FROM topic_links tl \
         LEFT JOIN posts lp ON lp.id = tl.link_post_id AND lp.deleted_at IS NULL \
         LEFT JOIN topics t ON t.id = tl.link_topic_id AND t.deleted_at IS NULL \
         LEFT JOIN posts fp ON lp.id IS NULL AND fp.topic_id = t.id AND fp.post_number = 1 \
           AND fp.deleted_at IS NULL \
         WHERE tl.post_id = $1 AND NOT tl.reflection \
           AND COALESCE(lp.user_id, fp.user_id) IS DISTINCT FROM $2 \
           AND EXISTS (SELECT 1 FROM users u WHERE u.id = COALESCE(lp.user_id, fp.user_id)) \
         ORDER BY tl.id",
    )
    .bind(post.id)
    .bind(post.user_id)
    .fetch_all(&mut *conn)
    .await?;
    if !private_message {
        let targets: Vec<i32> = linked
            .into_iter()
            .filter(|u| !alerter.notified.contains(u))
            .collect();
        let mut reached = Vec::new();
        for user_id in targets {
            if alerter
                .create_notification(conn, user_id, types::LINKED, &Opts::default())
                .await?
            {
                reached.push(user_id);
            }
        }
        alerter.notified.extend(reached);
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

    // group_watchers: the message's groups' members watching first posts.
    let group_watchers: Vec<i32> = if private_message {
        sqlx::query_scalar(
            "SELECT user_id FROM group_users WHERE notification_level = 4 \
             AND group_id IN (SELECT group_id FROM topic_allowed_groups WHERE topic_id = $1)",
        )
        .bind(post.topic_id)
        .fetch_all(&mut *conn)
        .await?
    } else {
        Vec::new()
    };
    if new_record && private_message {
        // notify_pm_users
        if group_notifying_via_smtp(ctx, conn, &post).await? {
            return Err(Unsupported("group SMTP").into());
        }
        // Each directly targeted user not yet notified hears of it when
        // replied to, watching the message, or staged.
        for (user_id, staged) in &direct {
            if alerter.notified.contains(user_id) {
                continue;
            }
            if reply_target == Some(*user_id) || pm_watching.contains(user_id) || *staged {
                alerter
                    .create_notification(conn, *user_id, types::PRIVATE_MESSAGE, &Opts::default())
                    .await?;
            }
        }
        // The groups' members by their level on the message: watching
        // hear of it, tracking when replied to (or on a new message their
        // group watches the first post of) or else get their group's
        // summary, regular when replied to. Those it reached are notified.
        let indirect: Vec<i32> = allowed_group_users
            .iter()
            .copied()
            .filter(|u| !alerter.notified.contains(u))
            .collect();
        let mut stats = None;
        let mut reached = Vec::new();
        for user_id in indirect {
            let level: Option<i32> = sqlx::query_scalar(
                "SELECT notification_level FROM topic_users WHERE topic_id = $1 AND user_id = $2",
            )
            .bind(post.topic_id)
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
            // is_replying?: replied to, or quoted.
            let replying = reply_target == Some(user_id) || quoted_users.contains(&user_id);
            let got = match level {
                Some(3) => {
                    alerter
                        .create_notification(
                            conn,
                            user_id,
                            types::PRIVATE_MESSAGE,
                            &Opts::default(),
                        )
                        .await?
                }
                Some(2) => {
                    if replying || (post.post_number == 1 && group_watchers.contains(&user_id)) {
                        alerter
                            .create_notification(
                                conn,
                                user_id,
                                types::PRIVATE_MESSAGE,
                                &Opts::default(),
                            )
                            .await?
                    } else {
                        if stats.is_none() {
                            stats = Some(
                                crate::posting::group_messages::group_stats(conn, post.topic_id)
                                    .await?,
                            );
                        }
                        crate::posting::group_messages::notify_group_summary(
                            ctx,
                            conn,
                            stats.as_deref().unwrap_or_default(),
                            user_id,
                            post.topic_id,
                            None,
                        )
                        .await?
                    }
                }
                Some(1) if replying => {
                    alerter
                        .create_notification(
                            conn,
                            user_id,
                            types::PRIVATE_MESSAGE,
                            &Opts::default(),
                        )
                        .await?
                }
                _ => false,
            };
            if got {
                reached.push(user_id);
            }
        }
        alerter.notified.extend(reached);
    } else if new_record && notify_about_reply {
        // Topic watchers, then category and tag watchers.
        alerter.notify_post_users(conn, true, false).await?;
        alerter.notify_post_users(conn, false, true).await?;
    }

    // sync_group_mentions: the post's group mentions replaced by the
    // groups it names.
    sqlx::query("DELETE FROM group_mentions WHERE post_id = $1")
        .bind(post.id)
        .execute(&mut *conn)
        .await?;
    for (group_id, _) in &mentioned_groups {
        sqlx::query(
            "INSERT INTO group_mentions (post_id, group_id, created_at, updated_at) \
             VALUES ($1, $2, now(), now()) ON CONFLICT DO NOTHING",
        )
        .bind(post.id)
        .bind(group_id)
        .execute(&mut *conn)
        .await?;
    }

    if new_record && post.post_number == 1 {
        let mut watchers: Vec<i32> = sqlx::query_scalar(
            "SELECT DISTINCT user_id FROM category_users WHERE category_id = $1 AND notification_level = 4 \
             ORDER BY user_id",
        )
        .bind(post.category_id)
        .fetch_all(&mut *conn)
        .await?;
        // tag_watchers: watching the first post of one of its tags.
        watchers.extend(
            crate::tagging::tag_watchers(&mut *conn, post.topic_id, WATCHING_FIRST_POST_LEVEL)
                .await?,
        );
        watchers.extend(group_watchers.iter().copied());
        // In a message, only those who can see it.
        if private_message {
            watchers.retain(|u| all_allowed.contains(u));
        }
        watchers.retain(|u| Some(*u) != post.user_id && Some(*u) != post.last_editor_id);
        watchers.sort_unstable();
        watchers.dedup();
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

/// `Topic#nested_view?`: nested replies on, not a message, and the topic
/// converted (a nested_topics row) or nested by default.
async fn nested_view(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    post: &Post,
) -> Result<bool, AppError> {
    let s = ctx.settings;
    if !s.get("nested_replies_enabled")?.truthy() || post.archetype == "private_message" {
        return Ok(false);
    }
    let converted: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM nested_topics WHERE topic_id = $1)")
            .bind(post.topic_id)
            .fetch_one(&mut *conn)
            .await?;
    Ok(converted || s.get("nested_replies_default")?.truthy())
}

/// `expand_group_mentions(groups, post)`: of the groups named, those the
/// author may mention (`Group.mentionable(user, include_public: false)`),
/// each with fewer members than max_users_notified_per_group_mention, with
/// its members (`group.users`); none without an author.
async fn expand_group_mentions(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    post: &Post,
    groups: &[(i32, String)],
) -> Result<Vec<(i32, String, Vec<i32>)>, AppError> {
    let Some(author) = post.user_id.filter(|_| !groups.is_empty()) else {
        return Ok(Vec::new());
    };
    let Some(user) = SessionUser::load(&mut *conn, author).await? else {
        return Ok(Vec::new());
    };
    let guardian = Guardian::for_user(&mut *conn, &user).await?;
    let ids: Vec<i32> = groups.iter().map(|(id, _)| *id).collect();
    let mentionable: Vec<(i32, String, i32)> = sqlx::query_as(&format!(
        "SELECT id, name, user_count FROM groups WHERE ({}) AND id = ANY($1) ORDER BY ctid",
        crate::user_search::mentionable_where_with(&guardian, author, false)
    ))
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    let max = ctx
        .settings
        .get("max_users_notified_per_group_mention")?
        .to_i();
    let mut out = Vec::new();
    for (id, name, user_count) in mentionable {
        if i64::from(user_count) >= max {
            continue;
        }
        let members: Vec<i32> = sqlx::query_scalar(
            "SELECT users.id FROM users JOIN group_users ON users.id = group_users.user_id \
             WHERE group_users.group_id = $1 ORDER BY group_users.ctid",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
        out.push((id, name, members));
    }
    Ok(out)
}

/// `Guardian#can_mention_here?` for the post's author: a here_mention set,
/// max_here_mentioned at least one, no user named it, and the author in
/// here_mention_allowed_groups.
async fn can_mention_here(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    post: &Post,
) -> Result<bool, AppError> {
    let s = ctx.settings;
    let here = s.get("here_mention")?.to_s();
    if here.trim().is_empty() || s.get("max_here_mentioned")?.to_i() < 1 {
        return Ok(false);
    }
    let Some(author) = post.user_id else {
        return Ok(false);
    };
    let Some(user) = SessionUser::load(&mut *conn, author).await? else {
        return Ok(false);
    };
    let taken: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE username_lower = $1)")
            .bind(&here)
            .fetch_one(&mut *conn)
            .await?;
    if taken {
        return Ok(false);
    }
    let guardian = Guardian::for_user(&mut *conn, &user).await?;
    Ok(guardian.in_setting_groups(s, "here_mention_allowed_groups")?)
}

/// `expand_here_mention(post, exclude_ids:)`: real users with a regular or
/// moderator action post in the topic (a whisper too when the author is
/// staff), less those excluded, at most max_here_mentioned, in the
/// table's order.
async fn expand_here_mention(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    post: &Post,
    exclude: &[i32],
) -> Result<Vec<i32>, AppError> {
    use crate::posting::post_types;
    let staff: bool = sqlx::query_scalar(
        "SELECT COALESCE((SELECT admin OR moderator FROM users WHERE id = $1), FALSE)",
    )
    .bind(post.user_id)
    .fetch_one(&mut *conn)
    .await?;
    let mut types = vec![post_types::REGULAR, post_types::MODERATOR_ACTION];
    if staff {
        types.push(post_types::WHISPER);
    }
    Ok(sqlx::query_scalar(
        "SELECT id FROM users WHERE id > 0 \
           AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id) \
           AND id IN (SELECT user_id FROM posts WHERE topic_id = $1 AND post_type = ANY($2) \
                      AND deleted_at IS NULL AND NOT (user_id = ANY($3))) \
         ORDER BY ctid LIMIT $4",
    )
    .bind(post.topic_id)
    .bind(&types)
    .bind(exclude)
    .bind(ctx.settings.get("max_here_mentioned")?.to_i())
    .fetch_all(&mut *conn)
    .await?)
}

/// `group_notifying_via_smtp(post)`: with SMTP on, a regular post in a
/// message to one group that sends by SMTP, or to several when the
/// message came in by email (the group it was sent to is then looked up,
/// so any SMTP group there counts).
async fn group_notifying_via_smtp(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    post: &Post,
) -> Result<bool, AppError> {
    if !ctx.settings.get("enable_smtp")?.truthy()
        || post.post_type != crate::posting::post_types::REGULAR
    {
        return Ok(false);
    }
    let (groups, smtp, incoming): (i64, bool, bool) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(bool_or(g.smtp_enabled), FALSE), \
                EXISTS (SELECT 1 FROM incoming_emails WHERE topic_id = $1) \
         FROM topic_allowed_groups tag JOIN groups g ON g.id = tag.group_id WHERE tag.topic_id = $1",
    )
    .bind(post.topic_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(smtp && (groups == 1 || incoming))
}

/// `extract_quoted_users(post)`: the users the raw's quotes name (by the
/// quote's `username:` when display names lead, else its first part),
/// the author left out.
async fn extract_quoted_users(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    post: &Post,
) -> Result<Vec<i32>, AppError> {
    let s = ctx.settings;
    let by_username =
        s.get("display_name_on_posts")?.truthy() && !s.get("prioritize_username_in_ux")?.truthy();
    let names = quoted_usernames(&post.raw, by_username);
    if names.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_scalar(
        "SELECT id FROM users WHERE username_lower = ANY($1) AND id IS DISTINCT FROM $2 ORDER BY id",
    )
    .bind(&names)
    .bind(post.user_id)
    .fetch_all(&mut *conn)
    .await?)
}

/// The usernames `extract_quoted_users` scans the raw for: each
/// `username:<alnum>*"]` (`by_username`), else each `[quote="<name>,...`
/// with `"]` later on the line; stripped, downcased, each once.
fn quoted_usernames(raw: &str, by_username: bool) -> Vec<String> {
    use std::sync::LazyLock;
    // Onigmo's [[:alnum:]] on a UTF-8 string: letters, marks and digits.
    static USERNAME: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"username:([\p{L}\p{M}\p{Nd}]*)"\]"#).expect("valid regex")
    });
    static FIRST: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r#"\[quote="([^,]+),.+"\]"#).expect("valid regex"));
    let re = if by_username { &*USERNAME } else { &*FIRST };
    let mut out: Vec<String> = Vec::new();
    for caps in re.captures_iter(raw) {
        let name = crate::ruby::strip(&caps[1]).to_lowercase();
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

struct Alerter<'a> {
    ctx: &'a Ctx<'a>,
    post: &'a Post,
    notified: Vec<i32>,
    /// The job's options (`@default_opts`): only `skip_send_email`.
    skip_send_email: bool,
}

impl<'a> Alerter<'a> {
    fn new(ctx: &'a Ctx<'a>, post: &'a Post) -> Self {
        Alerter {
            ctx,
            post,
            notified: Vec::new(),
            skip_send_email: false,
        }
    }
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
        let like_frequency: i32 = if [types::LIKED, types::REACTION].contains(&notification_type) {
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
            let notify: bool = sqlx::query_scalar(
                "SELECT notify_on_linked_posts FROM user_options WHERE user_id = $1",
            )
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or(true);
            if !notify {
                return Ok(false);
            }
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
            // notify_users leaves staged users out of a message's
            // notifications; notify_pm_users gives them its own.
            if post.archetype == "private_message" && notification_type != types::PRIVATE_MESSAGE {
                return Ok(false);
            }
            let mirror: bool = sqlx::query_scalar(
                "SELECT COALESCE((SELECT mailinglist_mirror FROM categories WHERE id = $1), FALSE)",
            )
            .bind(post.category_id)
            .fetch_one(&mut *conn)
            .await?;
            if mirror {
                return Ok(false);
            }
        }
        // The notifier muted or ignored by the user (staff can't be).
        let notifier = opts.user_id.or(post.user_id);
        if let Some(notifier) = notifier {
            let staff: bool = sqlx::query_scalar(
                "SELECT COALESCE((SELECT admin OR moderator FROM users WHERE id = $1), FALSE)",
            )
            .bind(notifier)
            .fetch_one(&mut *conn)
            .await?;
            let screener = crate::user_comm_screener::UserCommScreener::new(
                &mut *conn,
                crate::user_comm_screener::Actor {
                    id: notifier,
                    staff,
                },
                &[user_id],
            )
            .await?;
            if screener.ignoring_or_muting_actor(user_id)? {
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
        // Muted on the mentioned group: Rails looks the GroupUser up by
        // opts[:group_id], which no caller sets, so it never applies.
        let existing: Vec<i32> = sqlx::query_scalar(
            "SELECT notification_type FROM notifications WHERE user_id = $1 AND topic_id = $2 \
             AND post_number = $3 ORDER BY id DESC LIMIT 10",
        )
        .bind(user_id)
        .bind(post.topic_id)
        .bind(post.post_number)
        .fetch_all(&mut *conn)
        .await?;
        if existing.contains(&notification_type) && notification_type == types::EDITED {
            // should_notify_edit?: a day later, or by someone else.
            let (old, shown): (bool, Option<String>) = sqlx::query_as(
                "SELECT created_at < now() - interval '1 day', data::json->>'display_username' \
                 FROM notifications WHERE user_id = $1 AND topic_id = $2 AND post_number = $3 \
                   AND notification_type = $4 ORDER BY id DESC LIMIT 1",
            )
            .bind(user_id)
            .bind(post.topic_id)
            .bind(post.post_number)
            .bind(notification_type)
            .fetch_one(&mut *conn)
            .await?;
            let display = opts
                .display_username
                .clone()
                .filter(|u| !u.is_empty())
                .or_else(|| post.username.clone());
            if !old && shown == display {
                return Ok(false);
            }
        } else if existing.contains(&notification_type) {
            // should_notify_like?, for reactions too (PostAlerterExtension):
            // always, or first_time_and_daily once the last one is a day old.
            let renotify = [types::LIKED, types::REACTION].contains(&notification_type)
                && match like_frequency {
                0 => true,
                1 => sqlx::query_scalar::<_, bool>(
                    "SELECT created_at < now() - interval '1 day' FROM notifications WHERE user_id = $1 \
                     AND topic_id = $2 AND post_number = $3 AND notification_type = $4 ORDER BY id DESC LIMIT 1",
                )
                .bind(user_id)
                .bind(post.topic_id)
                .bind(post.post_number)
                .bind(notification_type)
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
        // In a nested topic a reply collapses only with those in its bucket
        // (the post it answers, or the root: Notification::TOPIC_ROOT_BUCKET).
        let mut bucket: Option<i32> = None;
        let mut consolidated_count: Option<i64> = None;
        let nested = COLLAPSED.contains(&notification_type) && nested_view(ctx, conn, post).await?;
        if notification_type == types::REPLIED && nested {
            let b = post.reply_to_post_number.unwrap_or(1);
            bucket = Some(b);
            // destroy_notifications(user, [replied], topic, bucket_post_number:)
            sqlx::query(
                "DELETE FROM notifications WHERE user_id = $1 AND topic_id = $2 AND notification_type = $3 \
                 AND data::jsonb ->> 'reply_to_post_number' = $4",
            )
            .bind(user_id)
            .bind(post.topic_id)
            .bind(types::REPLIED)
            .bind(b.to_string())
            .execute(&mut *conn)
            .await?;
            // unread_posts_in_bucket: secured, past the last read, replies
            // to the bucket's post (the root: no reply_to or post 1, not
            // post 1 itself).
            let visible = guardian.visible_post_types(ctx.settings)?;
            let (first, count): (Option<i32>, i64) = sqlx::query_as(
                "SELECT MIN(post_number), COUNT(*) FROM posts WHERE topic_id = $2 AND deleted_at IS NULL \
                   AND post_type = ANY($3) \
                   AND post_number > COALESCE((SELECT last_read_post_number FROM topic_users tu \
                                               WHERE tu.user_id = $1 AND tu.topic_id = $2), 0) \
                   AND CASE WHEN $4 = 1 THEN (reply_to_post_number IS NULL OR reply_to_post_number = 1) \
                                              AND post_number > 1 \
                            ELSE reply_to_post_number = $4 END",
            )
            .bind(user_id)
            .bind(post.topic_id)
            .bind(&visible)
            .bind(b)
            .fetch_one(&mut *conn)
            .await?;
            if let Some(first) = first {
                target_post_number = first;
            }
            if count > 1 {
                consolidated_count = Some(count);
                let n = count.to_string();
                display_username = Some(
                    ctx.i18n
                        .t_with("embed.replies.other", &[("count", &n)])
                        .unwrap_or_else(|| format!("{count} replies")),
                );
            }
        } else if COLLAPSED.contains(&notification_type) {
            // destroy_notifications(user, COLLAPSED, topic) when the user can
            // see the topic (checked above), less replied in a nested topic,
            // then the first unread post.
            let collapsed: Vec<i32> = COLLAPSED
                .iter()
                .copied()
                .filter(|t| !(nested && *t == types::REPLIED))
                .collect();
            sqlx::query(
                "DELETE FROM notifications WHERE user_id = $1 AND topic_id = $2 AND notification_type = ANY($3)",
            )
            .bind(user_id)
            .bind(post.topic_id)
            .bind(&collapsed)
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
            .bind(opts.acting_user_id.or(post.user_id))
            .bind(post.topic_id)
            .bind(post.id)
            .execute(&mut *conn)
            .await?;
        }

        // A collapsed notification points at the first unread post, and
        // takes its author for the display fields.
        let (target_username, target_name) = if target_post_number != post.post_number {
            sqlx::query_as::<_, (Option<String>, Option<String>)>(
                "SELECT u.username, u.name FROM posts p LEFT JOIN users u ON u.id = p.user_id \
                 WHERE p.topic_id = $1 AND p.post_number = $2",
            )
            .bind(post.topic_id)
            .bind(target_post_number)
            .fetch_one(&mut *conn)
            .await?
        } else {
            (post.username.clone(), post.name.clone())
        };
        // The data, in Rails' key order.
        let displayed = display_username.clone().or_else(|| target_username.clone());
        let mut data = Map::new();
        data.insert("topic_title".into(), json!(post.topic_title));
        data.insert("original_post_id".into(), json!(post.id));
        data.insert("original_post_type".into(), json!(post.post_type));
        data.insert("original_username".into(), json!(original_username));
        data.insert("revision_number".into(), json!(opts.revision_number));
        data.insert("display_username".into(), json!(displayed));
        if let Some(b) = bucket {
            data.insert("reply_to_post_number".into(), json!(b));
            if let Some(count) = consolidated_count {
                data.insert("consolidated_count".into(), json!(count));
            }
        }
        if let Some(name) = opts.display_name.as_ref().or(target_name.as_ref()) {
            data.insert("display_name".into(), json!(name));
        }
        for (key, value) in &opts.custom_data {
            data.insert(key.clone(), value.clone());
        }
        if let Some((group_id, group_name)) = &opts.group {
            data.insert("group_id".into(), json!(group_id));
            data.insert("group_name".into(), json!(group_name));
        }
        // consolidate_or_create!, with its after_commit work (the count,
        // and the email or the shelf).
        let mut notification = crate::notification_create::NewNotification::new(
            notification_type,
            user_id,
            Value::Object(data),
        )
        .topic(post.topic_id, Some(target_post_number));
        notification.post_action_id = opts.post_action_id;
        // skip_send_email: (skip_send_email_to comes only with group
        // SMTP, refused) a post by email skips those it was sent or copied
        // to, else the options say.
        let incoming: Option<(Option<String>, Option<String>)> = if post.via_email {
            sqlx::query_as(
                "SELECT to_addresses, cc_addresses FROM incoming_emails WHERE post_id = $1 \
                 ORDER BY id LIMIT 1",
            )
            .bind(post.id)
            .fetch_optional(&mut *conn)
            .await?
        } else {
            None
        };
        notification.skip_send_email = match incoming {
            Some((to, cc)) => {
                let email: Option<String> = sqlx::query_scalar(
                    "SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"",
                )
                .bind(user_id)
                .fetch_optional(&mut *conn)
                .await?;
                // to_addresses_split / cc_addresses_split
                let listed = |addresses: &Option<String>| {
                    addresses
                        .as_deref()
                        .is_some_and(|a| a.split(';').any(|a| Some(a) == email.as_deref()))
                };
                listed(&to) || listed(&cc)
            }
            None => self.skip_send_email,
        };
        crate::notification_create::consolidate_or_create(
            &mut *conn,
            ctx.bus,
            ctx.settings,
            &notification,
        )
        .await?;
        // create_notification_alert for a first notification of a notifiable
        // type: the live alert (to users seen in the last 30 days), then
        // push notifications, refused for users who have push set up.
        if existing.is_empty() && NOTIFIABLE.contains(&notification_type) && !user.suspended() {
            let slug: Option<String> = sqlx::query_scalar("SELECT slug FROM topics WHERE id = $1")
                .bind(post.topic_id)
                .fetch_one(&mut *conn)
                .await?;
            let excerpt = crate::excerpt::excerpt(
                &post.cooked,
                400,
                &crate::excerpt::Options {
                    text_entities: true,
                    strip_links: true,
                    remap_emoji: true,
                    denied_emojis: ctx.settings.emojis().denied.clone(),
                    plain_hashtags: true,
                    ..Default::default()
                },
                crate::excerpt::Plugins::from_settings(ctx.settings)?,
            );
            let mut payload = json!({
                "notification_type": notification_type,
                "post_number": post.post_number,
                "topic_title": post.topic_title,
                "topic_id": post.topic_id,
                "post_id": post.id,
                "excerpt": excerpt,
                "username": original_username,
                // Post.url: no base path.
                "post_url": format!(
                    "/t/{}/{}/{}",
                    slug.unwrap_or_default(),
                    post.topic_id,
                    post.post_number
                ),
            });
            if let Some((_, group_name)) = opts.group.as_ref().filter(|(_, n)| !n.trim().is_empty())
            {
                payload["group_name"] = json!(group_name);
            }
            crate::bus::publish_notification_alert(ctx.bus, &mut *conn, user_id, &payload).await?;
            push_notification(conn, ctx.settings, user_id).await?;
        }
        Ok(true)
    }
}

/// `PostAlerter.push_notification(user, payload)`: nothing for a user in
/// do not disturb; push delivery is not ported, so otherwise a user with
/// a push subscription or a push client (`UserApiKey.push_clients_for`)
/// is refused.
pub async fn push_notification(
    conn: &mut PgConnection,
    settings: &crate::site_settings::SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    if crate::notification_create::do_not_disturb(conn, user_id).await? {
        return Ok(());
    }
    // push_clients_for: none unless the push scope is allowed and some
    // push urls are; then active keys with the push or notifications
    // scope and a client, whose push url is inside the allowed urls.
    let scopes = settings.get("allow_user_api_key_scopes")?.to_s();
    let push_urls = settings.get("allowed_user_api_push_urls")?.to_s();
    let clients_possible = scopes.split('|').any(|s| s == "push") && !push_urls.trim().is_empty();
    let push: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM push_subscriptions WHERE user_id = $1) \
         OR ($2 AND EXISTS (SELECT 1 FROM user_api_keys k \
                    JOIN user_api_key_scopes s ON s.user_api_key_id = k.id \
                    JOIN user_api_key_clients c ON c.id = k.user_api_key_client_id \
                    WHERE k.user_id = $1 AND k.revoked_at IS NULL \
                      AND (k.expires_at IS NULL OR k.expires_at > now()) \
                      AND s.name IN ('push', 'notifications') \
                      AND k.push_url IS NOT NULL AND k.push_url <> '' \
                      AND position(k.push_url IN $3) > 0))",
    )
    .bind(user_id)
    .bind(clients_possible)
    .bind(&push_urls)
    .fetch_one(&mut *conn)
    .await?;
    if push {
        return Err(Unsupported("push notifications").into());
    }
    Ok(())
}

/// The post a notify job is about, and whether its topic is visible
/// (`post&.topic&.visible?`).
async fn load_visible_post(
    conn: &mut PgConnection,
    args: &Value,
) -> Result<Option<Post>, AppError> {
    let Some(post_id) = args.get("post_id").and_then(Value::as_i64) else {
        return Ok(None);
    };
    let Some(post) = load_post(conn, post_id as i32).await? else {
        return Ok(None);
    };
    let visible: bool = sqlx::query_scalar("SELECT visible FROM topics WHERE id = $1")
        .bind(post.topic_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(visible.then_some(post))
}

fn id_list(args: &Value, key: &str) -> Vec<i32> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_i64)
                .map(|id| id as i32)
                .collect()
        })
        .unwrap_or_default()
}

impl Alerter<'_> {
    /// `notify_post_users(post, notified, group_ids:, include_*_watchers:)`
    /// for an edit (not a new record): the watchers asked for, less the
    /// users in `notified`, each told the post was posted, or edited when
    /// they have read it already.
    async fn notify_watchers(
        &mut self,
        conn: &mut PgConnection,
        notified: &[i32],
        group_ids: Option<&[i32]>,
        topic: bool,
        category: bool,
        tags: bool,
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
        }
        let tagged: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM topic_tags WHERE topic_id = $1)")
                .bind(post.topic_id)
                .fetch_one(&mut *conn)
                .await?;
        if tags && tagged {
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
        let mut query = format!("SELECT id FROM users WHERE id IN ({sql}) AND NOT (id = ANY($3))");
        if group_ids.is_some() {
            query.push_str(" AND id IN (SELECT user_id FROM group_users WHERE group_id = ANY($4))");
        }
        if post.archetype == "private_message" {
            query.push_str(" AND NOT staged AND (admin OR moderator)");
        }
        // each_user_in_batches: User.where(id:), in the table's order.
        query.push_str(" ORDER BY ctid");
        let users: Vec<i32> = sqlx::query_scalar(&query)
            .bind(post.topic_id)
            .bind(post.category_id.unwrap_or(0))
            .bind(notified)
            .bind(group_ids.unwrap_or_default())
            .fetch_all(&mut *conn)
            .await?;
        let seen: Vec<i32> = sqlx::query_scalar(
            "SELECT user_id FROM topic_users WHERE topic_id = $1 AND last_read_post_number >= $2",
        )
        .bind(post.topic_id)
        .bind(post.post_number)
        .fetch_all(&mut *conn)
        .await?;
        for user_id in users {
            let (notification_type, opts) = if seen.contains(&user_id) {
                (
                    types::EDITED,
                    Opts {
                        display_username: post.editor_username.clone(),
                        ..Default::default()
                    },
                )
            } else {
                (types::POSTED, Opts::default())
            };
            self.create_notification(conn, user_id, notification_type, &opts)
                .await?;
        }
        Ok(())
    }

    /// `notify_first_post_watchers(post, user_ids)`: once each, never the
    /// author or last editor.
    async fn notify_first_post_watchers(
        &mut self,
        conn: &mut PgConnection,
        mut user_ids: Vec<i32>,
    ) -> Result<(), AppError> {
        let post = self.post;
        user_ids.retain(|u| Some(*u) != post.user_id && Some(*u) != post.last_editor_id);
        // User.where(id:), in the table's order.
        let users: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM users WHERE id = ANY($1) ORDER BY ctid")
                .bind(&user_ids)
                .fetch_all(&mut *conn)
                .await?;
        for user_id in users {
            self.create_notification(conn, user_id, types::WATCHING_FIRST_POST, &Opts::default())
                .await?;
        }
        Ok(())
    }
}

/// `Jobs::NotifyTagChange`: the tags' first-post watchers, then (unless
/// tag edit notifications are off) the topic's watchers other than those
/// named, limited to the tags' groups when every changed tag is in a
/// group.
pub async fn notify_tag_change(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    args: &Value,
) -> Result<(), AppError> {
    let Some(post) = load_visible_post(conn, args).await? else {
        return Ok(());
    };
    let mut alerter = Alerter::new(ctx, &post);
    let watchers =
        crate::tagging::tag_watchers(&mut *conn, post.topic_id, WATCHING_FIRST_POST_LEVEL).await?;
    alerter.notify_first_post_watchers(conn, watchers).await?;
    if ctx
        .settings
        .get("disable_tags_edit_notifications")?
        .truthy()
        && args.get("force") != Some(&json!(true))
    {
        return Ok(());
    }
    let diff_tags: Vec<String> = args
        .get("diff_tags")
        .and_then(Value::as_array)
        .map(|t| {
            t.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    // all_tags_in_hidden_groups?, then tag_group_ids.
    let group_ids: Option<Vec<i32>> = if diff_tags.is_empty() {
        None
    } else {
        let in_groups: i64 = sqlx::query_scalar(
            "SELECT count(DISTINCT tags.id) FROM tags \
             JOIN tag_group_memberships tgm ON tgm.tag_id = tags.id \
             JOIN tag_group_permissions tgp ON tgp.tag_group_id = tgm.tag_group_id \
             WHERE tags.name = ANY($1) AND tgp.group_id <> 0",
        )
        .bind(&diff_tags)
        .fetch_one(&mut *conn)
        .await?;
        if in_groups == diff_tags.len() as i64 {
            Some(
                sqlx::query_scalar(
                    "SELECT tgp.group_id FROM tags \
                     JOIN tag_group_memberships tgm ON tgm.tag_id = tags.id \
                     JOIN tag_group_permissions tgp ON tgp.tag_group_id = tgm.tag_group_id \
                     WHERE tags.name = ANY($1)",
                )
                .bind(&diff_tags)
                .fetch_all(&mut *conn)
                .await?,
            )
        } else {
            None
        }
    };
    let notified = id_list(args, "notified_user_ids");
    let private_message = post.archetype == "private_message";
    alerter
        .notify_watchers(
            conn,
            &notified,
            group_ids.as_deref(),
            !private_message,
            false,
            true,
        )
        .await
}

/// `Jobs::NotifyCategoryChange`: the topic's and category's watchers other
/// than those named, the category's first-post watchers, then the mailing
/// list job.
pub async fn notify_category_change(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    args: &Value,
) -> Result<(), AppError> {
    let Some(post) = load_visible_post(conn, args).await? else {
        return Ok(());
    };
    let mut alerter = Alerter::new(ctx, &post);
    let notified = id_list(args, "notified_user_ids");
    alerter
        .notify_watchers(conn, &notified, None, true, true, false)
        .await?;
    let watchers: Vec<i32> = sqlx::query_scalar(
        "SELECT user_id FROM category_users WHERE category_id = $1 AND notification_level = 4 \
         ORDER BY id",
    )
    .bind(post.category_id)
    .fetch_all(&mut *conn)
    .await?;
    alerter.notify_first_post_watchers(conn, watchers).await?;
    crate::jobs::enqueue(
        &mut *conn,
        "notify_mailing_list_subscribers",
        json!({"post_id": post.id}),
    )
    .await?;
    Ok(())
}

/// `Jobs::NotifyPostRevision`: each user named told of the edit (staff
/// only for a hidden revision), by the revision's author.
pub async fn notify_post_revision(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    args: &Value,
) -> Result<(), AppError> {
    if args.get("user_ids").is_none_or(Value::is_null) {
        return Err(
            Unsupported("notify_post_revision without user_ids (InvalidParameters)").into(),
        );
    }
    let Some(revision_id) = args.get("post_revision_id").and_then(Value::as_i64) else {
        return Ok(());
    };
    type Revision = (i32, i32, Option<i32>, bool, Option<String>);
    let revision: Option<Revision> = sqlx::query_as(
        "SELECT r.post_id, r.number, r.user_id, r.hidden, u.username FROM post_revisions r \
         LEFT JOIN users u ON u.id = r.user_id WHERE r.id = $1",
    )
    .bind(revision_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((post_id, number, editor_id, hidden, editor_username)) = revision else {
        return Ok(());
    };
    let Some(post) = load_post(conn, post_id).await? else {
        return Ok(());
    };
    let mut alerter = Alerter::new(ctx, &post);
    let users: Vec<(i32, bool)> =
        sqlx::query_as("SELECT id, admin OR moderator FROM users WHERE id = ANY($1) ORDER BY id")
            .bind(id_list(args, "user_ids"))
            .fetch_all(&mut *conn)
            .await?;
    for (user_id, staff) in users {
        if hidden && !staff {
            continue;
        }
        alerter
            .create_notification(
                conn,
                user_id,
                types::EDITED,
                &Opts {
                    display_username: editor_username.clone(),
                    acting_user_id: editor_id,
                    revision_number: Some(number),
                    ..Default::default()
                },
            )
            .await?;
    }
    Ok(())
}
