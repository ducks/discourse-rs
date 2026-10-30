//! A chat message's mentions: Chat::ParsedMentions (who and what its
//! cooked text mentions, outside quotes and transcripts), Message#
//! upsert_mentions (its chat_mentions rows) and Chat::Notifier's
//! notify_new and notify_edit (who of them to notify, what the sender is
//! told of the others, and the jobs that notify).

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::pretty_text::cleanup;
use crate::session::current::SessionUser;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `Chat::ParsedMentions`
pub struct Parsed {
    /// `@name` mentions, normalized (lower case, no `@`), less @here/@all.
    pub direct: Vec<String>,
    /// `.mention-group` mentions, normalized.
    pub groups: Vec<String>,
    pub here: bool,
    pub all: bool,
}

impl Parsed {
    /// `#count`
    fn count(&self) -> usize {
        self.direct.len() + self.groups.len() + usize::from(self.here) + usize::from(self.all)
    }
}

fn parent(node: &markup5ever_rcdom::Handle) -> Option<markup5ever_rcdom::Handle> {
    let weak = node.parent.take();
    node.parent.set(weak.clone());
    weak.and_then(|w| w.upgrade())
}

/// Inside a quote (`aside.quote`) or a chat transcript, where mentions
/// don't count (cooked_stripped).
fn quoted(node: &markup5ever_rcdom::Handle) -> bool {
    let mut current = parent(node);
    while let Some(p) = current {
        if cleanup::has_class(&p, "chat-transcript")
            || (cleanup::element_name(&p) == Some("aside") && cleanup::has_class(&p, "quote"))
        {
            return true;
        }
        current = parent(&p);
    }
    false
}

/// `Chat::ParsedMentions.new(message)`, from its cooked text.
pub fn parse(cooked: &str) -> Parsed {
    let dom = cleanup::parse(cooked);
    let mut mentions: Vec<String> = Vec::new();
    let mut group_mentions: Vec<String> = Vec::new();
    for e in cleanup::all_elements(&dom) {
        if quoted(&e) {
            continue;
        }
        if cleanup::has_class(&e, "mention") {
            mentions.push(cleanup::text(&e));
        }
        if cleanup::has_class(&e, "mention-group") {
            group_mentions.push(cleanup::text(&e));
        }
    }
    // normalize: `mention[1..-1].downcase`, @here and @all left out.
    let normalize = |list: &[String]| -> Vec<String> {
        list.iter()
            .filter(|m| {
                let lower = m.to_lowercase();
                lower != "@here" && lower != "@all"
            })
            .map(|m| m.chars().skip(1).collect::<String>().to_lowercase())
            .collect()
    };
    Parsed {
        here: mentions.iter().any(|m| m == "@here"),
        all: mentions.iter().any(|m| m == "@all"),
        direct: normalize(&mentions),
        groups: normalize(&group_mentions),
    }
}

/// The message's author, as the notifier needs them.
pub struct Author {
    pub user: SessionUser,
    pub guardian: Guardian,
}

impl Author {
    pub async fn load(conn: &mut PgConnection, user_id: i32) -> Result<Option<Author>, AppError> {
        let Some(user) = SessionUser::load(&mut *conn, user_id).await? else {
            return Ok(None);
        };
        let guardian = Guardian::for_user(&mut *conn, &user).await?;
        Ok(Some(Author { user, guardian }))
    }

    fn username_lower(&self) -> String {
        self.user.username.to_lowercase()
    }
}

/// A group a message mentions.
#[derive(sqlx::FromRow, Clone)]
struct GroupRow {
    id: i32,
    name: String,
    user_count: i32,
    mentionable: bool,
}

/// What the message's mentions name: its direct mentions' users and the
/// groups it mentions as the author may see and mention them.
struct Resolved {
    /// `#direct_mentions`: users with chat on.
    direct: Vec<i32>,
    /// `#visible_groups`, by name, each with whether the author may
    /// mention it (`#mentionable_groups`).
    visible_groups: Vec<GroupRow>,
    max_per_group: i64,
}

impl Resolved {
    async fn load(
        conn: &mut PgConnection,
        settings: &SiteSettings,
        parsed: &Parsed,
        author: &Author,
    ) -> Result<Resolved, AppError> {
        let direct: Vec<i32> = sqlx::query_scalar(
            "SELECT DISTINCT users.id FROM users \
             JOIN user_options ON user_options.user_id = users.id AND user_options.chat_enabled \
             WHERE users.username_lower = ANY($1) ORDER BY users.id",
        )
        .bind(&parsed.direct)
        .fetch_all(&mut *conn)
        .await?;
        let visible_groups = if parsed.groups.is_empty() {
            Vec::new()
        } else {
            // Group.visible_groups(user) and Group.mentionable(user,
            // include_public: false) (alias_levels).
            let levels: Vec<i32> = if author.user.admin {
                vec![99, 1, 2, 3, 4]
            } else if author.user.moderator {
                vec![99, 2, 3, 4]
            } else {
                vec![99]
            };
            let sql = format!(
                "SELECT groups.id, groups.name, groups.user_count, \
                   (groups.mentionable_level = ANY($2) \
                    OR (groups.mentionable_level = 3 AND groups.id IN (SELECT group_id FROM group_users WHERE user_id = $3)) \
                    OR (groups.mentionable_level = 4 AND groups.id IN (SELECT group_id FROM group_users WHERE user_id = $3 AND owner IS TRUE))) AS mentionable \
                 FROM groups WHERE LOWER(groups.name) = ANY($1) AND groups.id > 0 AND groups.id NOT IN (4, 5) AND {} \
                 ORDER BY groups.name ASC",
                crate::groups::visible_groups_where(&author.guardian, "groups")
            );
            sqlx::query_as(&sql)
                .bind(&parsed.groups)
                .bind(&levels)
                .bind(author.user.id)
                .fetch_all(&mut *conn)
                .await?
        };
        Ok(Resolved {
            direct,
            visible_groups,
            max_per_group: settings.get("max_users_notified_per_group_mention")?.to_i(),
        })
    }

    /// `#groups_to_mention`: mentionable, and not too many members.
    fn groups_to_mention(&self) -> Vec<&GroupRow> {
        self.visible_groups
            .iter()
            .filter(|g| g.mentionable && i64::from(g.user_count) <= self.max_per_group)
            .collect()
    }

    /// `#groups_with_disabled_mentions`
    fn groups_with_disabled_mentions(&self) -> Vec<&GroupRow> {
        self.visible_groups
            .iter()
            .filter(|g| !g.mentionable)
            .collect()
    }

    /// `#groups_with_too_many_members`
    fn groups_with_too_many_members(&self) -> Vec<&GroupRow> {
        self.visible_groups
            .iter()
            .filter(|g| g.mentionable && i64::from(g.user_count) > self.max_per_group)
            .collect()
    }
}

/// `Chat::Message#upsert_mentions`: its user, group, @all and @here
/// mentions made to match what it mentions now.
pub async fn upsert(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    message_id: i64,
    parsed: &Parsed,
    author: &Author,
    now: NaiveDateTime,
) -> Result<(), AppError> {
    let resolved = Resolved::load(conn, settings, parsed, author).await?;
    let groups: Vec<i32> = resolved.groups_to_mention().iter().map(|g| g.id).collect();
    for (kind, targets) in [
        ("Chat::UserMention", resolved.direct.clone()),
        ("Chat::GroupMention", groups),
    ] {
        let old: Vec<i32> = sqlx::query_scalar(
            "SELECT target_id FROM chat_mentions WHERE chat_message_id = $1 AND type = $2",
        )
        .bind(message_id)
        .bind(kind)
        .fetch_all(&mut *conn)
        .await?;
        let removed: Vec<i32> = old
            .iter()
            .copied()
            .filter(|id| !targets.contains(id))
            .collect();
        if !removed.is_empty() {
            // destroy_all: each mention's notifications go with it.
            destroy_mentions(
                conn,
                bus,
                settings,
                "chat_message_id = $1 AND type = $2 AND target_id = ANY($3)",
                message_id,
                kind,
                Some(&removed),
            )
            .await?;
        }
        for id in targets.iter().filter(|id| !old.contains(id)) {
            sqlx::query(
                "INSERT INTO chat_mentions (chat_message_id, target_id, type, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $4)",
            )
            .bind(message_id)
            .bind(id)
            .bind(kind)
            .bind(now)
            .execute(&mut *conn)
            .await?;
        }
        // upsert_user_mentions: the mentioned join a thread that shows,
        // to track what they read.
        if kind == "Chat::UserMention" && !targets.is_empty() {
            let thread: Option<i64> = sqlx::query_scalar(
                "SELECT chat_threads.id FROM chat_messages \
                 JOIN chat_threads ON chat_threads.id = chat_messages.thread_id \
                 JOIN chat_channels ON chat_channels.id = chat_messages.chat_channel_id \
                 WHERE chat_messages.id = $1 AND (chat_channels.threading_enabled OR chat_threads.force)",
            )
            .bind(message_id)
            .fetch_optional(&mut *conn)
            .await?;
            if let Some(thread_id) = thread {
                let mut users: Vec<i32> =
                    sqlx::query_scalar("SELECT id FROM users WHERE id = ANY($1) ORDER BY id")
                        .bind(&targets)
                        .fetch_all(&mut *conn)
                        .await?;
                users.dedup();
                for user_id in users {
                    super::threads::add_member(
                        conn,
                        bus,
                        thread_id,
                        i64::from(user_id),
                        super::threads::NORMAL,
                        now,
                    )
                    .await?;
                }
            }
        }
    }
    // create_or_delete_all_mention, create_or_delete_here_mention
    for (kind, wanted) in [
        ("Chat::AllMention", parsed.all),
        ("Chat::HereMention", parsed.here),
    ] {
        let present: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM chat_mentions WHERE chat_message_id = $1 AND type = $2)",
        )
        .bind(message_id)
        .bind(kind)
        .fetch_one(&mut *conn)
        .await?;
        if present && !wanted {
            destroy_mentions(
                conn,
                bus,
                settings,
                "chat_message_id = $1 AND type = $2",
                message_id,
                kind,
                None,
            )
            .await?;
        } else if wanted && !present {
            sqlx::query(
                "INSERT INTO chat_mentions (chat_message_id, target_id, type, created_at, updated_at) \
                 VALUES ($1, NULL, $2, $3, $3)",
            )
            .bind(message_id)
            .bind(kind)
            .bind(now)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// Chat::Mention#destroy: its notifications first (before_destroy
/// destroy_notifications), then the mention and its links.
async fn destroy_mentions(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    clause: &str,
    message_id: i64,
    kind: &str,
    targets: Option<&[i32]>,
) -> Result<(), AppError> {
    let sql = format!("SELECT id FROM chat_mentions WHERE {clause}");
    let mut query = sqlx::query_scalar(&sql).bind(message_id).bind(kind);
    if let Some(targets) = targets {
        query = query.bind(targets);
    }
    let ids: Vec<i64> = query.fetch_all(&mut *conn).await?;
    if ids.is_empty() {
        return Ok(());
    }
    let notified: Vec<i32> = sqlx::query_scalar(
        "DELETE FROM notifications WHERE id IN (SELECT notification_id FROM chat_mention_notifications \
           WHERE chat_mention_id = ANY($1)) RETURNING user_id",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    // after_commit refresh_notification_count
    let mut refreshed: Vec<i32> = Vec::new();
    for user_id in notified {
        if !refreshed.contains(&user_id) {
            crate::bus::publish_notifications_state(bus, &mut *conn, settings, user_id).await?;
            refreshed.push(user_id);
        }
    }
    sqlx::query("DELETE FROM chat_mention_notifications WHERE chat_mention_id = ANY($1)")
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM chat_mentions WHERE id = ANY($1)")
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// A mentioned user the notifier sorts.
#[derive(sqlx::FromRow)]
struct Mentioned {
    id: i32,
    username: String,
}

/// The mentioned the sender is told of: those who can join but haven't
/// (`welcome_to_join`), and those who can't (`unreachable`).
#[derive(Default)]
struct Inaccessible {
    welcome_to_join: Vec<Mentioned>,
    unreachable: Vec<Mentioned>,
}

/// What the notifier works with.
pub struct Notifier<'a> {
    pub state: &'a AppState,
    pub settings: &'a SiteSettings,
    pub channel: &'a super::channels::ChannelRow,
    pub message_id: i64,
    pub author: &'a Author,
    pub parsed: &'a Parsed,
}

impl Notifier<'_> {
    /// `#group_users_to_notify`: who can join the channel, of them who
    /// follows it, and the humans of the rest.
    async fn group_users(
        &self,
        conn: &mut PgConnection,
        users: Vec<Mentioned>,
    ) -> Result<(Vec<Mentioned>, Vec<Mentioned>, Vec<Mentioned>), AppError> {
        let mut members = Vec::new();
        let mut welcome = Vec::new();
        let mut unreachable = Vec::new();
        for user in users {
            let Some(session) = SessionUser::load(&mut *conn, user.id).await? else {
                continue;
            };
            let guardian = Guardian::for_user(&mut *conn, &session).await?;
            let can_join = super::channels::Context {
                conn: &mut *conn,
                settings: self.settings,
                i18n: &self.state.i18n,
                guardian: &guardian,
                base_path: self.state.config.globals.relative_url_root(),
                config: &self.state.config,
            }
            .can_join(self.channel, None)
            .await?;
            if !can_join {
                if user.id > 0 {
                    unreachable.push(user);
                }
                continue;
            }
            // joined_by?: a public channel's following member.
            let joined: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM user_chat_channel_memberships \
                 WHERE user_id = $1 AND chat_channel_id = $2 AND following)",
            )
            .bind(user.id)
            .bind(self.channel.id)
            .fetch_one(&mut *conn)
            .await?;
            if joined {
                members.push(user);
            } else if user.id > 0 {
                welcome.push(user);
            }
        }
        Ok((members, welcome, unreachable))
    }

    /// `#list_users_to_notify`: by mention type, who to notify; who the
    /// sender is told of; everyone mentioned.
    async fn list_users_to_notify(
        &self,
        conn: &mut PgConnection,
    ) -> Result<(Map<String, Value>, Inaccessible, Vec<i32>), AppError> {
        let resolved = Resolved::load(conn, self.settings, self.parsed, self.author).await?;
        let max = self.settings.get("max_mentions_per_chat_message")?.to_i();
        let skip = self.parsed.count() as i64 > max;
        let author_lower = self.author.username_lower();
        let mut to_notify = Map::new();
        let mut inaccessible = Inaccessible::default();
        let mut covered: Vec<i32> = Vec::new();

        // expand_direct_mentions
        let direct: Vec<Mentioned> = if skip {
            Vec::new()
        } else {
            sqlx::query_as(
                "SELECT id, username FROM users WHERE id = ANY($1) \
                   AND (suspended_till IS NULL OR suspended_till <= now()) \
                   AND username_lower <> $2 ORDER BY id",
            )
            .bind(&resolved.direct)
            .bind(&author_lower)
            .fetch_all(&mut *conn)
            .await?
        };
        let (members, welcome, unreachable) = self.group_users(conn, direct).await?;
        let ids: Vec<i32> = members.iter().map(|u| u.id).collect();
        to_notify.insert("direct_mentions".into(), json!(ids));
        covered.extend(&ids);
        inaccessible.welcome_to_join = welcome;
        inaccessible.unreachable = unreachable;

        if !skip {
            // expand_group_mentions
            if !resolved.visible_groups.is_empty() {
                let groups = resolved.groups_to_mention();
                let group_ids: Vec<i32> = groups.iter().map(|g| g.id).collect();
                let reached: Vec<Mentioned> = sqlx::query_as(
                    "SELECT DISTINCT users.id, users.username FROM users \
                     JOIN user_options ON user_options.user_id = users.id AND user_options.chat_enabled \
                     WHERE users.id IN (SELECT user_id FROM group_users WHERE group_id = ANY($1)) \
                       AND (users.suspended_till IS NULL OR users.suspended_till <= now()) \
                       AND users.username_lower <> $2 AND NOT (users.id = ANY($3)) ORDER BY users.id",
                )
                .bind(&group_ids)
                .bind(&author_lower)
                .bind(&covered)
                .fetch_all(&mut *conn)
                .await?;
                for g in &groups {
                    to_notify.insert(g.name.to_lowercase(), json!([]));
                }
                let (members, welcome, unreachable) = self.group_users(conn, reached).await?;
                // The first group the message names that they belong to.
                let mentioned_names: Vec<String> = self
                    .parsed
                    .groups
                    .iter()
                    .filter(|name| groups.iter().any(|g| g.name.to_lowercase() == **name))
                    .cloned()
                    .collect();
                for user in members {
                    let theirs: Vec<String> = sqlx::query_scalar(
                        "SELECT LOWER(groups.name) FROM groups JOIN group_users ON group_users.group_id = groups.id \
                         WHERE group_users.user_id = $1",
                    )
                    .bind(user.id)
                    .fetch_all(&mut *conn)
                    .await?;
                    if let Some(name) = mentioned_names.iter().find(|n| theirs.contains(n))
                        && let Some(Value::Array(list)) = to_notify.get_mut(name)
                    {
                        list.push(json!(user.id));
                    }
                    covered.push(user.id);
                }
                inaccessible.welcome_to_join.extend(welcome);
                inaccessible.unreachable.extend(unreachable);
            }
            // expand_here_mention, expand_global_mention
            for (key, wanted, recent) in [
                ("here_mentions", self.parsed.here, true),
                ("global_mentions", self.parsed.all, false),
            ] {
                let ids: Vec<i32> = if wanted && self.channel.allow_channel_wide_mentions {
                    sqlx::query_scalar(&format!(
                        "SELECT DISTINCT users.id FROM users \
                         JOIN user_options ON user_options.user_id = users.id AND user_options.chat_enabled \
                         JOIN user_chat_channel_memberships m ON m.user_id = users.id AND m.following AND m.chat_channel_id = $1 \
                         WHERE NOT (users.username_lower = ANY($2)) {} \
                           AND (users.suspended_till IS NULL OR users.suspended_till <= now()) \
                           AND (user_options.ignore_channel_wide_mention IS NULL OR NOT user_options.ignore_channel_wide_mention) \
                           AND users.username_lower <> $3 AND NOT (users.id = ANY($4)) ORDER BY users.id",
                        if recent {
                            "AND users.last_seen_at > now() - interval '5 minutes'"
                        } else {
                            ""
                        }
                    ))
                    .bind(self.channel.id)
                    .bind(&self.parsed.direct)
                    .bind(&author_lower)
                    .bind(&covered)
                    .fetch_all(&mut *conn)
                    .await?
                } else {
                    Vec::new()
                };
                covered.extend(&ids);
                to_notify.insert(key.into(), json!(ids));
            }
        }

        // filter_users_ignoring_or_muting_creator: the screening adds the
        // welcome_to_join to everyone mentioned.
        covered.extend(inaccessible.welcome_to_join.iter().map(|u| u.id));
        if !covered.is_empty() && !self.author.guardian.is_staff() {
            let screening: Vec<i32> = sqlx::query_scalar(
                "SELECT user_id FROM muted_users WHERE muted_user_id = $1 AND user_id = ANY($2) \
                 UNION SELECT user_id FROM ignored_users WHERE ignored_user_id = $1 AND user_id = ANY($2)",
            )
            .bind(self.author.user.id)
            .bind(&covered)
            .fetch_all(&mut *conn)
            .await?;
            for list in to_notify.values_mut() {
                if let Value::Array(ids) = list {
                    ids.retain(|id| {
                        !id.as_i64()
                            .is_some_and(|id| screening.contains(&(id as i32)))
                    });
                }
            }
            inaccessible
                .welcome_to_join
                .retain(|u| !screening.contains(&u.id));
            covered.retain(|id| !screening.contains(id));
        }
        Ok((to_notify, inaccessible, covered))
    }

    /// `#notify_new`: the mentioned told of their new mention, the sender
    /// of the others, then NotifyMentioned and, 5 seconds on,
    /// NotifyWatching (all but the mentioned and the sender).
    pub async fn notify_new(
        &self,
        conn: &mut PgConnection,
        created_at: NaiveDateTime,
    ) -> Result<(), AppError> {
        let (to_notify, inaccessible, mentioned) = self.list_users_to_notify(conn).await?;
        for id in &mentioned {
            // Chat::Publisher.publish_new_mention
            self.state
                .bus
                .publish(
                    &mut *conn,
                    &format!("/chat/{}/new-mentions", self.channel.id),
                    &json!({ "message_id": self.message_id, "channel_id": self.channel.id }),
                    Some(&[crate::bus::user_tag(*id)]),
                )
                .await?;
        }
        let resolved = Resolved::load(conn, self.settings, self.parsed, self.author).await?;
        self.notify_creator(conn, &inaccessible, &resolved).await?;
        let timestamp = format!("{} UTC", created_at.format("%Y-%m-%d %H:%M:%S"));
        crate::jobs::enqueue(
            &mut *conn,
            super::notify::NOTIFY_MENTIONED_JOB,
            json!({
                "chat_message_id": self.message_id,
                "to_notify_ids_map": to_notify,
                "already_notified_user_ids": [],
                "timestamp": timestamp,
            }),
        )
        .await?;
        let mut except = mentioned;
        except.push(self.author.user.id);
        crate::jobs::enqueue_in(
            &mut *conn,
            5,
            super::notify::NOTIFY_WATCHING_JOB,
            json!({
                "chat_message_id": self.message_id,
                "except_user_ids": except,
                "timestamp": timestamp,
            }),
        )
        .await?;
        Ok(())
    }

    /// `#notify_edit`: those newly mentioned by the edit.
    pub async fn notify_edit(
        &self,
        conn: &mut PgConnection,
        timestamp: &str,
    ) -> Result<(), AppError> {
        let already: Vec<i32> = sqlx::query_scalar(
            "SELECT notifications.user_id FROM notifications \
             INNER JOIN chat_mention_notifications ON chat_mention_notifications.notification_id = notifications.id \
             INNER JOIN chat_mentions ON chat_mentions.id = chat_mention_notifications.chat_mention_id \
             WHERE notifications.notification_type = 29 AND chat_mentions.chat_message_id = $1",
        )
        .bind(self.message_id)
        .fetch_all(&mut *conn)
        .await?;
        let (to_notify, inaccessible, mentioned) = self.list_users_to_notify(conn).await?;
        if mentioned.iter().all(|id| already.contains(id)) {
            return Ok(());
        }
        let resolved = Resolved::load(conn, self.settings, self.parsed, self.author).await?;
        self.notify_creator(conn, &inaccessible, &resolved).await?;
        crate::jobs::enqueue(
            &mut *conn,
            super::notify::NOTIFY_MENTIONED_JOB,
            json!({
                "chat_message_id": self.message_id,
                "to_notify_ids_map": to_notify,
                "already_notified_user_ids": already,
                "timestamp": timestamp,
            }),
        )
        .await?;
        Ok(())
    }

    /// `#notify_creator_of_inaccessible_mentions`: notices on the channel
    /// for the sender alone.
    async fn notify_creator(
        &self,
        conn: &mut PgConnection,
        inaccessible: &Inaccessible,
        resolved: &Resolved,
    ) -> Result<(), AppError> {
        if let Some(first) = inaccessible.welcome_to_join.first() {
            let text = self.warning(
                "chat.mention_warning.without_membership",
                "chat.mention_warning.without_membership_multiple",
                &first.username,
                inaccessible.welcome_to_join.len(),
            );
            let ids: Vec<i32> = inaccessible.welcome_to_join.iter().map(|u| u.id).collect();
            self.notice(
                conn,
                json!({
                    "type": "notice",
                    "channel_id": self.channel.id,
                    "notice_type": "mention_without_membership",
                    "data": { "user_ids": ids, "text": text, "message_id": self.message_id },
                }),
            )
            .await?;
        }
        // hidden_member_groups: the groups mentioned whose members the
        // sender can't see.
        let groups = resolved.groups_to_mention();
        let mut hidden: Vec<&GroupRow> = Vec::new();
        for g in &groups {
            if !self.members_visible(conn, g.id).await? {
                hidden.push(g);
            }
        }
        let unreachable_ids: Vec<i32> = inaccessible.unreachable.iter().map(|u| u.id).collect();
        let hidden_ids: Vec<i32> = hidden.iter().map(|g| g.id).collect();
        let group_warning: bool = !unreachable_ids.is_empty()
            && !hidden_ids.is_empty()
            && sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM group_users WHERE group_id = ANY($1) AND user_id = ANY($2))",
            )
            .bind(&hidden_ids)
            .bind(&unreachable_ids)
            .fetch_one(&mut *conn)
            .await?;
        if group_warning {
            let text = self
                .state
                .i18n
                .t_with(
                    "chat.mention_warning.cannot_see_group",
                    &[("group_name", hidden[0].name.as_str())],
                )
                .unwrap_or_default();
            self.text_notice(conn, &text).await?;
        } else if let Some(first) = inaccessible.unreachable.first() {
            let text = self.warning(
                "chat.mention_warning.cannot_see",
                "chat.mention_warning.cannot_see_multiple",
                &first.username,
                inaccessible.unreachable.len(),
            );
            self.text_notice(conn, &text).await?;
        }
        if (self.parsed.all || self.parsed.here) && !self.channel.allow_channel_wide_mentions {
            let text = self
                .state
                .i18n
                .t("chat.mention_warning.global_mentions_disallowed")
                .unwrap_or_default()
                .to_string();
            self.text_notice(conn, &text).await?;
        }
        let disabled = resolved.groups_with_disabled_mentions();
        if let Some(first) = disabled.first() {
            let text = self.warning(
                "chat.mention_warning.group_mentions_disabled",
                "chat.mention_warning.group_mentions_disabled_multiple",
                &first.name,
                disabled.len(),
            );
            self.text_notice(conn, &text).await?;
        }
        let too_many = resolved.groups_with_too_many_members();
        if let Some(first) = too_many.first() {
            let text = self.warning(
                "chat.mention_warning.too_many_members",
                "chat.mention_warning.too_many_members_multiple",
                &first.name,
                too_many.len(),
            );
            self.text_notice(conn, &text).await?;
        }
        Ok(())
    }

    /// `Group.members_visible_groups(user)` holds the group.
    async fn members_visible(
        &self,
        conn: &mut PgConnection,
        group_id: i32,
    ) -> Result<bool, AppError> {
        let sql = format!(
            "SELECT EXISTS (SELECT 1 FROM groups WHERE groups.id = $1 AND {})",
            crate::groups::members_visible_groups_where(&self.author.guardian, "groups")
        );
        Ok(sqlx::query_scalar(&sql)
            .bind(group_id)
            .fetch_one(&mut *conn)
            .await?)
    }

    /// `#mention_warning_text`: the single form, else the multiple one
    /// counting the others.
    fn warning(&self, single: &str, multiple: &str, first: &str, count: usize) -> String {
        let i18n = &self.state.i18n;
        if count == 1 {
            i18n.t_with(single, &[("first_identifier", first)])
                .unwrap_or_default()
        } else {
            i18n.t_count(multiple, count as i64 - 1, &[("first_identifier", first)])
                .unwrap_or_default()
        }
    }

    async fn text_notice(&self, conn: &mut PgConnection, text: &str) -> Result<(), AppError> {
        self.notice(
            conn,
            json!({ "type": "notice", "channel_id": self.channel.id, "text_content": text }),
        )
        .await
    }

    /// `Chat::Publisher.publish_notice`: on the channel, to the sender.
    async fn notice(&self, conn: &mut PgConnection, payload: Value) -> Result<(), AppError> {
        self.state
            .bus
            .publish(
                &mut *conn,
                &format!("/chat/{}", self.channel.id),
                &payload,
                Some(&[crate::bus::user_tag(self.author.user.id)]),
            )
            .await?;
        Ok(())
    }
}
