//! Sending a chat message (POST /chat/:chat_channel_id): Chat::CreateMessage,
//! and the job it enqueues, Jobs::Chat::ProcessMessage (the message cooked
//! again, its mentions upserted, its links extracted, Chat::Notifier's
//! mention and watching jobs enqueued, and the processed message published).
//!
//! A reply makes (or joins) the replied-to message's thread, as Rails does
//! in channels without threading too: the thread stays hidden there. Not
//! ported yet, and refused: uploads, blocks, threads proper (threading on,
//! forced threads, a thread_id), bots, group and channel-wide mentions,
//! oneboxes and images in a message, and the per-user rate limiter.

use chrono::NaiveDateTime;
use serde_json::{Value, json};

use super::channels::{ChannelRow, Context};
use crate::{AppError, Unsupported};

/// The job that processes a new or edited message.
pub const PROCESS_MESSAGE_JOB: &str = "Jobs::Chat::ProcessMessage";
/// The job that recounts a thread's replies.
pub const UPDATE_THREAD_REPLY_COUNT_JOB: &str = "Jobs::Chat::UpdateThreadReplyCount";
const NOTIFY_MENTIONED_JOB: &str = "Jobs::Chat::NotifyMentioned";
const NOTIFY_WATCHING_JOB: &str = "Jobs::Chat::NotifyWatching";

/// `Chat::NotificationLevels.all[:tracking]`
const THREAD_TRACKING: i32 = 2;

/// Chat::CreateMessage's params.
#[derive(Default)]
pub struct Params {
    /// The channel's id or slug.
    pub chat_channel_id: String,
    pub message: Option<String>,
    pub in_reply_to_id: Option<String>,
    pub staged_id: Option<String>,
    pub thread_id: Option<String>,
    pub upload_ids: Vec<String>,
    pub blocks: bool,
    pub client_created_at: Option<String>,
}

pub enum Outcome {
    Created(i64),
    NotFound,
    Forbidden,
    /// The contract's errors (400).
    Invalid(Vec<String>),
    /// A policy's reason or the model's errors (422).
    Unprocessable(String),
}

fn t(i18n: &crate::i18n::I18n, key: &str) -> String {
    i18n.t(key).unwrap_or(key).to_string()
}

impl Context<'_> {
    /// `Chat::Channel.find_by_id_or_slug`: a live channel by id, else by
    /// slug.
    async fn find_by_id_or_slug(
        &mut self,
        id_or_slug: &str,
    ) -> Result<Option<ChannelRow>, AppError> {
        let sql = format!(
            "SELECT {} FROM chat_channels WHERE deleted_at IS NULL \
               AND (id::text = $1 OR lower(slug) = lower($1)) ORDER BY (id::text = $1) DESC LIMIT 1",
            super::channels::CHANNEL_COLUMNS
        );
        Ok(sqlx::query_as(&sql)
            .bind(id_or_slug.trim())
            .fetch_optional(&mut *self.conn)
            .await?)
    }

    /// Chat::CreateMessage, the message created by the guardian's user.
    pub async fn create_message(
        &mut self,
        host: &crate::pretty_text::Host,
        bus: &pg_bus::Bus,
        params: Params,
    ) -> Result<Outcome, AppError> {
        let g = self.guardian;
        let settings = self.settings;
        let Some(user) = g.user().cloned() else {
            return Ok(Outcome::Forbidden);
        };
        // policy :no_silenced_user, policy :accept_blocks
        if g.is_silenced() {
            return Ok(Outcome::Forbidden);
        }
        if user.id <= 0 {
            return Err(Unsupported("chat messages sent by bots").into());
        }
        if params.blocks {
            return Ok(Outcome::Forbidden);
        }
        // The contract.
        if !params.upload_ids.is_empty() {
            return Err(Unsupported("chat message uploads").into());
        }
        let raw = params.message.clone().unwrap_or_default();
        let mut errors = Vec::new();
        if params.chat_channel_id.trim().is_empty() {
            errors.push("Chat channel can't be blank".to_string());
        }
        if raw.trim().is_empty() {
            errors.push("Message can't be blank".to_string());
        }
        let max = settings.get("chat_maximum_message_length")?.to_i();
        if raw.chars().count() as i64 > max {
            errors.push(format!("Message is too long (maximum is {max} characters)"));
        }
        if !errors.is_empty() {
            return Ok(Outcome::Invalid(errors));
        }
        let message = crate::posting::text::clean_message(&raw, true);

        // model :channel, policy :can_post_in_channel
        let Some(channel) = self.find_by_id_or_slug(&params.chat_channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        if channel.chatable_type != "Category" {
            return Err(Unsupported("chat direct messages").into());
        }
        let can_post = g.is_admin()
            || !sqlx::query_scalar::<_, i32>(&format!(
                "SELECT categories.id FROM categories WHERE categories.id = $1 AND {}",
                super::categories_scoped_to(g, "1, 2")
            ))
            .bind(channel.chatable_id as i32)
            .fetch_all(&mut *self.conn)
            .await?
            .is_empty();
        if !can_post {
            return Ok(Outcome::Forbidden);
        }
        // model :membership
        let has_membership: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_chat_channel_memberships WHERE user_id = $1 AND chat_channel_id = $2)",
        )
        .bind(user.id)
        .bind(channel.id)
        .fetch_one(&mut *self.conn)
        .await?;
        if !has_membership {
            return Ok(Outcome::NotFound);
        }
        // policy :channel_allows_message_creation (can_create_channel_message?)
        let status = super::channels::STATUSES
            .get(channel.status as usize)
            .copied()
            .unwrap_or("open");
        let allowed = status == "open" || (g.is_staff() && status == "closed");
        if !allowed {
            return Ok(Outcome::Unprocessable(t(
                self.i18n,
                &format!("chat.errors.channel_new_message_disallowed.{status}"),
            )));
        }
        // model :reply, policy :ensure_reply_consistency
        let reply: Option<(i64, i64, Option<i32>, Option<i64>)> = match params
            .in_reply_to_id
            .as_deref()
            .filter(|r| !r.trim().is_empty())
        {
            None => None,
            Some(id) => {
                let found: Option<(i64, i64, Option<i32>, Option<i64>)> = sqlx::query_as(
                    "SELECT id, chat_channel_id, user_id, thread_id FROM chat_messages \
                     WHERE id = $1 AND deleted_at IS NULL",
                )
                .bind(crate::ruby::to_i(id))
                .fetch_optional(&mut *self.conn)
                .await?;
                match found {
                    Some(r) if r.1 == channel.id => Some(r),
                    _ => return Ok(Outcome::NotFound),
                }
            }
        };
        // model :thread: a thread_id, or the reply's thread (made when it
        // has none).
        if params
            .thread_id
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
        {
            return Err(Unsupported("chat threads").into());
        }
        if channel.threading_enabled && reply.is_some() {
            return Err(Unsupported("chat threads").into());
        }
        let existing_thread = match reply.and_then(|r| r.3) {
            Some(thread_id) => {
                let force: bool =
                    sqlx::query_scalar("SELECT force FROM chat_threads WHERE id = $1")
                        .bind(thread_id)
                        .fetch_one(&mut *self.conn)
                        .await?;
                if force {
                    return Err(Unsupported("chat threads").into());
                }
                Some(thread_id)
            }
            None => None,
        };

        // model :message_instance: cooked, then the model's validations.
        let cooked = super::cook::cook(host, &message, user.id, &user.username).await?;
        if !sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM watched_words WHERE action = 1)",
        )
        .fetch_one(&mut *self.conn)
        .await?
        {
            // WatchedWordsValidator has nothing to block.
        } else {
            return Err(Unsupported("watched words that block chat messages").into());
        }
        let now = crate::clock::now_naive();
        let created_at = params
            .client_created_at
            .as_deref()
            .and_then(|c| chrono::DateTime::parse_from_rfc3339(c).ok())
            .map(|c| c.naive_utc())
            .filter(|c| (now - *c).num_seconds().abs() <= 60)
            .unwrap_or(now);
        let mut model_errors = Vec::new();
        // Chat::DuplicateMessageValidator
        let duplicate: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM chat_messages WHERE chat_channel_id = $1 AND user_id = $2 \
               AND deleted_at IS NULL AND created_at >= $3 - interval '10 seconds' AND LOWER(message) = $4)",
        )
        .bind(channel.id)
        .bind(user.id)
        .bind(now)
        .bind(message.trim().to_lowercase())
        .fetch_one(&mut *self.conn)
        .await?;
        if duplicate {
            model_errors.push(t(self.i18n, "chat.errors.duplicate_message"));
        }
        let min = settings.get("chat_minimum_message_length")?.to_i();
        if (message.chars().count() as i64) < min {
            model_errors.push(
                t(
                    self.i18n,
                    if min == 1 {
                        "chat.errors.minimum_length_not_met.one"
                    } else {
                        "chat.errors.minimum_length_not_met.other"
                    },
                )
                .replace("%{count}", &min.to_string()),
            );
        }
        if cooked.chars().count() > 20_000 {
            model_errors.push("Cooked is too long (maximum is 20000 characters)".into());
        }
        if !model_errors.is_empty() {
            return Ok(Outcome::Unprocessable(model_errors.join(", ")));
        }

        // The transaction.
        let excerpt = super::messages::build_excerpt_for(&message, &cooked)?;
        let new_thread = match (reply, existing_thread) {
            (Some(r), None) => {
                let original_user =
                    r.2.ok_or(Unsupported("replies to deleted users' messages"))?;
                let id: i64 = sqlx::query_scalar(
                    "INSERT INTO chat_threads (channel_id, original_message_id, original_message_user_id, status, \
                       replies_count, force, created_at, updated_at) \
                     VALUES ($1, $2, $3, 0, 0, FALSE, $4, $4) RETURNING id",
                )
                .bind(channel.id)
                .bind(r.0)
                .bind(i64::from(original_user))
                .bind(now)
                .fetch_one(&mut *self.conn)
                .await?;
                Some(id)
            }
            _ => None,
        };
        let thread_id = existing_thread.or(new_thread);
        let message_id: i64 = sqlx::query_scalar(
            "INSERT INTO chat_messages (chat_channel_id, user_id, created_at, updated_at, message, cooked, \
               cooked_version, last_editor_id, in_reply_to_id, thread_id, streaming, excerpt, created_by_sdk) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $2, $8, $9, FALSE, $10, FALSE) RETURNING id",
        )
        .bind(channel.id)
        .bind(user.id)
        .bind(created_at)
        .bind(now)
        .bind(&message)
        .bind(&cooked)
        .bind(super::cook::BAKED_VERSION)
        .bind(reply.map(|r| r.0))
        .bind(thread_id)
        .bind(&excerpt)
        .fetch_one(&mut *self.conn)
        .await?;
        // The reply's autosaved thread.
        if let (Some(r), Some(new)) = (reply, new_thread) {
            sqlx::query("UPDATE chat_messages SET thread_id = $2, updated_at = $3 WHERE id = $1")
                .bind(r.0)
                .bind(new)
                .bind(now)
                .execute(&mut *self.conn)
                .await?;
        }
        // delete_drafts
        sqlx::query("DELETE FROM chat_drafts WHERE user_id = $1 AND chat_channel_id = $2")
            .bind(user.id)
            .bind(channel.id)
            .execute(&mut *self.conn)
            .await?;
        // post_process_thread
        if let (Some(thread_id), Some(r)) = (thread_id, reply) {
            sqlx::query(
                "UPDATE chat_threads SET last_message_id = $2, updated_at = $3 WHERE id = $1",
            )
            .bind(thread_id)
            .bind(message_id)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
            // increment_replies_count_cache: the count in Redis (not
            // kept here), and the job that writes it down.
            crate::jobs::enqueue_in(
                &mut *self.conn,
                5,
                UPDATE_THREAD_REPLY_COUNT_JOB,
                json!({ "thread_id": thread_id }),
            )
            .await?;
            let original_user = r.2.map(i64::from).unwrap_or(0);
            self.add_to_thread(thread_id, original_user, now).await?;
            self.add_to_thread(thread_id, i64::from(user.id), now)
                .await?;
            sqlx::query(
                "UPDATE user_chat_thread_memberships SET last_read_message_id = $3, updated_at = $4 \
                 WHERE thread_id = $1 AND user_id = $2",
            )
            .bind(thread_id)
            .bind(i64::from(user.id))
            .bind(message_id)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
        }
        // update_channel_last_message, update_membership_last_read (a
        // hidden thread's messages are the channel's).
        sqlx::query("UPDATE chat_channels SET last_message_id = $2, updated_at = $3 WHERE id = $1")
            .bind(channel.id)
            .bind(message_id)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
        sqlx::query(
            "UPDATE user_chat_channel_memberships SET last_read_message_id = $3, updated_at = $4 \
             WHERE user_id = $1 AND chat_channel_id = $2",
        )
        .bind(user.id)
        .bind(channel.id)
        .bind(message_id)
        .bind(now)
        .execute(&mut *self.conn)
        .await?;

        // process: publish_new!, then the processing job.
        super::publisher::publish_new(self, bus, &channel, message_id, params.staged_id.as_deref())
            .await?;
        crate::jobs::enqueue(
            &mut *self.conn,
            PROCESS_MESSAGE_JOB,
            json!({ "chat_message_id": message_id, "staged_id": params.staged_id }),
        )
        .await?;
        super::publisher::publish_user_tracking_state(self, bus, channel.id, message_id, thread_id)
            .await?;
        // index_message (deferred in Rails, made here).
        let urls = crate::url::Urls {
            config: self.config,
            settings,
        };
        let base = crate::posting::search_index::BaseUrls {
            base_path: self.base_path,
            base_url_no_prefix: urls.base_url_no_prefix()?,
            store: crate::file_store::FileStore::for_site(self.config, settings)?,
        };
        crate::posting::search_index::index_chat_message(
            &mut *self.conn,
            settings,
            &base,
            message_id,
            &message,
            &cooked,
        )
        .await?;
        Ok(Outcome::Created(message_id))
    }

    /// `Chat::Thread#add(user)`: a tracking membership, unless there is one.
    async fn add_to_thread(
        &mut self,
        thread_id: i64,
        user_id: i64,
        now: NaiveDateTime,
    ) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO user_chat_thread_memberships (user_id, thread_id, notification_level, created_at, updated_at) \
             SELECT $1, $2, $3, $4, $4 WHERE NOT EXISTS \
               (SELECT 1 FROM user_chat_thread_memberships WHERE user_id = $1 AND thread_id = $2)",
        )
        .bind(user_id)
        .bind(thread_id)
        .bind(THREAD_TRACKING)
        .bind(now)
        .execute(&mut *self.conn)
        .await?;
        Ok(())
    }
}

/// Jobs::Chat::ProcessMessage
pub async fn process_message(
    state: &crate::AppState,
    conn: &mut sqlx::PgConnection,
    settings: &crate::site_settings::SiteSettings,
    args: &Value,
) -> Result<(), AppError> {
    let Some(message_id) = args["chat_message_id"].as_i64() else {
        return Ok(());
    };
    #[derive(sqlx::FromRow)]
    struct Row {
        chat_channel_id: i64,
        user_id: Option<i32>,
        last_editor_id: i32,
        message: Option<String>,
        cooked: Option<String>,
        created_at: NaiveDateTime,
        deleted_at: Option<NaiveDateTime>,
    }
    let row: Option<Row> = sqlx::query_as(
        "SELECT chat_channel_id, user_id, last_editor_id, message, cooked, created_at, deleted_at \
         FROM chat_messages WHERE id = $1",
    )
    .bind(message_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(Row {
        chat_channel_id: channel_id,
        user_id,
        last_editor_id: editor_id,
        message,
        cooked: stored_cooked,
        created_at,
        deleted_at,
    }) = row
    else {
        return Ok(());
    };
    let user_id = user_id.ok_or(Unsupported("chat messages of deleted users"))?;
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    // Chat::MessageProcessor: cooked again, then the post processors,
    // which only change what holds oneboxes, images or videos.
    let host = crate::pretty_text::Host::from_state(state);
    let cooked = super::cook::cook(
        &host,
        message.as_deref().unwrap_or_default(),
        editor_id,
        &username,
    )
    .await?;
    if cooked.contains("class=\"onebox\"")
        || cooked.contains("<img") && !only_emoji_images(&cooked)
        || cooked.contains("<video")
    {
        return Err(Unsupported("oneboxes, images and videos in chat messages").into());
    }
    if stored_cooked.as_deref() != Some(cooked.as_str()) {
        sqlx::query("UPDATE chat_messages SET cooked = $2, cooked_version = $3, updated_at = $4 WHERE id = $1")
            .bind(message_id)
            .bind(&cooked)
            .bind(super::cook::BAKED_VERSION)
            .bind(crate::clock::now_naive())
            .execute(&mut *conn)
            .await?;
    }

    // upsert_mentions: direct user mentions.
    let mentions = parsed_mentions(&cooked)?;
    let mentioned: Vec<(i32, String)> = sqlx::query_as(
        "SELECT DISTINCT users.id, users.username_lower FROM users \
         JOIN user_options ON user_options.user_id = users.id AND user_options.chat_enabled \
         WHERE users.username_lower = ANY($1) ORDER BY users.id",
    )
    .bind(&mentions)
    .fetch_all(&mut *conn)
    .await?;
    let old: Vec<i32> = sqlx::query_scalar(
        "SELECT target_id FROM chat_mentions WHERE chat_message_id = $1 AND type = 'Chat::UserMention'",
    )
    .bind(message_id)
    .fetch_all(&mut *conn)
    .await?;
    let new_ids: Vec<i32> = mentioned.iter().map(|(id, _)| *id).collect();
    let removed: Vec<i32> = old
        .iter()
        .copied()
        .filter(|id| !new_ids.contains(id))
        .collect();
    if !removed.is_empty() {
        sqlx::query(
            "DELETE FROM chat_mentions WHERE chat_message_id = $1 AND type = 'Chat::UserMention' AND target_id = ANY($2)",
        )
        .bind(message_id)
        .bind(&removed)
        .execute(&mut *conn)
        .await?;
    }
    let now = crate::clock::now_naive();
    for id in new_ids.iter().filter(|id| !old.contains(id)) {
        sqlx::query(
            "INSERT INTO chat_mentions (chat_message_id, target_id, type, created_at, updated_at) \
             VALUES ($1, $2, 'Chat::UserMention', $3, $3)",
        )
        .bind(message_id)
        .bind(id)
        .bind(now)
        .execute(&mut *conn)
        .await?;
    }

    // Chat::MessageLink.extract_from
    if deleted_at.is_none() {
        extract_links(conn, message_id, &cooked, now).await?;
    }

    // Chat::Notifier#notify_new, for direct mentions.
    if args.get("edit_timestamp").is_some() {
        return Err(Unsupported("notifications of edited chat messages").into());
    }
    if !args["skip_notifications"].as_bool().unwrap_or(false) {
        notify_new(
            conn, &state.bus, settings, channel_id, message_id, user_id, &username, created_at,
            &mentioned,
        )
        .await?;
    }

    // Chat::Publisher.publish_processed!
    let anonymous = crate::guardian::Guardian::anonymous();
    let sql = format!(
        "SELECT {} FROM chat_channels WHERE id = $1",
        super::channels::CHANNEL_COLUMNS
    );
    let channel: Option<ChannelRow> = sqlx::query_as(&sql)
        .bind(channel_id)
        .fetch_optional(&mut *conn)
        .await?;
    if let Some(channel) = channel {
        let mut cx = Context {
            conn: &mut *conn,
            settings,
            i18n: &state.i18n,
            guardian: &anonymous,
            base_path: state.config.globals.relative_url_root(),
            config: &state.config,
        };
        super::publisher::publish_message(&mut cx, &state.bus, &channel, message_id, "processed")
            .await?;
    }
    Ok(())
}

/// Whether every image in the cooked HTML is an emoji.
fn only_emoji_images(cooked: &str) -> bool {
    cooked.matches("<img").count() == cooked.matches("class=\"emoji").count()
}

/// Chat::ParsedMentions' direct mentions: the `.mention` texts outside
/// quotes and transcripts, normalized. Group and channel-wide mentions are
/// refused until mentions are ported.
fn parsed_mentions(cooked: &str) -> Result<Vec<String>, AppError> {
    if cooked.contains("mention-group") {
        return Err(Unsupported("group mentions in chat").into());
    }
    let mentions = crate::pretty_text::extract_mentions(&strip_quoted(cooked)?)?;
    let mut out = Vec::new();
    for m in mentions {
        let lower = m.to_lowercase();
        if lower == "here" || lower == "all" {
            return Err(Unsupported("channel-wide mentions in chat").into());
        }
        if !out.contains(&lower) {
            out.push(lower);
        }
    }
    Ok(out)
}

/// The cooked HTML less its quotes and transcripts, where mentions don't
/// count (cooked_stripped); one with mentions inside is refused.
fn strip_quoted(cooked: &str) -> Result<String, AppError> {
    if (cooked.contains("<aside class=\"quote") || cooked.contains("chat-transcript"))
        && cooked.contains("class=\"mention")
    {
        return Err(Unsupported("mentions alongside quotes in chat").into());
    }
    Ok(cooked.to_string())
}

/// `Chat::MessageLink.extract_from`
async fn extract_links(
    conn: &mut sqlx::PgConnection,
    message_id: i64,
    cooked: &str,
    now: NaiveDateTime,
) -> Result<(), AppError> {
    let mut urls: Vec<String> = Vec::new();
    for link in crate::posting::links::extract(cooked)? {
        // UrlHelper.relaxed_parse: the url as written, with its scheme and
        // host; relative urls (mentions' profiles) have no host.
        let (scheme, rest) = match link.split_once(':') {
            Some((s, r))
                if !s.is_empty()
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) =>
            {
                (s, r)
            }
            _ => ("", link.as_str()),
        };
        if scheme.eq_ignore_ascii_case("mailto") {
            continue;
        }
        let host = rest
            .strip_prefix("//")
            .and_then(|r| r.split(['/', '?', '#']).next())
            .and_then(|h| h.rsplit('@').next())
            .and_then(|h| h.split(':').next())
            .unwrap_or_default();
        if host.is_empty() || host.len() > 100 {
            continue;
        }
        let url: String = link.chars().take(500).collect();
        if urls.contains(&url) {
            continue;
        }
        sqlx::query(
            "INSERT INTO chat_message_links (chat_message_id, url, created_at, updated_at) \
             VALUES ($1, $2, $3, $3) ON CONFLICT (chat_message_id, url) DO NOTHING",
        )
        .bind(message_id)
        .bind(&url)
        .bind(now)
        .execute(&mut *conn)
        .await?;
        urls.push(url);
    }
    sqlx::query(
        "DELETE FROM chat_message_links WHERE chat_message_id = $1 AND NOT (url = ANY($2))",
    )
    .bind(message_id)
    .bind(&urls)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Chat::Notifier#notify_new` for direct mentions: who to notify (members
/// who can chat, not suspended), the new mention published to each, then
/// NotifyMentioned and, 5 seconds on, NotifyWatching enqueued.
#[allow(clippy::too_many_arguments)]
async fn notify_new(
    conn: &mut sqlx::PgConnection,
    bus: &pg_bus::Bus,
    settings: &crate::site_settings::SiteSettings,
    channel_id: i64,
    message_id: i64,
    user_id: i32,
    username: &str,
    created_at: NaiveDateTime,
    mentioned: &[(i32, String)],
) -> Result<(), AppError> {
    let max = settings.get("max_mentions_per_chat_message")?.to_i();
    let skip = mentioned.len() as i64 > max;
    let mut direct = Vec::new();
    if !skip {
        for (id, username_lower) in mentioned {
            if *username_lower == username.to_lowercase() {
                continue;
            }
            let row: Option<(bool, bool)> = sqlx::query_as(
                "SELECT (suspended_till IS NOT NULL AND suspended_till > now()), \
                        EXISTS (SELECT 1 FROM user_chat_channel_memberships m \
                                WHERE m.user_id = users.id AND m.chat_channel_id = $2 AND m.following) \
                 FROM users WHERE id = $1",
            )
            .bind(id)
            .bind(channel_id)
            .fetch_optional(&mut *conn)
            .await?;
            let Some((suspended, member)) = row else {
                continue;
            };
            if suspended {
                continue;
            }
            // group_users_to_notify: a mentioned user who can't join, or
            // can but isn't a member, gets a notice to the sender instead.
            if !member {
                return Err(Unsupported("chat mentions of users outside the channel").into());
            }
            direct.push(*id);
        }
    }
    // filter_users_ignoring_or_muting_creator
    let screened: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM ignored_users WHERE user_id = ANY($1) AND ignored_user_id = $2) \
           OR EXISTS (SELECT 1 FROM muted_users WHERE user_id = ANY($1) AND muted_user_id = $2)",
    )
    .bind(&direct)
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if screened {
        return Err(Unsupported("chat mentions by an ignored or muted user").into());
    }
    // Chat::Publisher.publish_new_mention, to each mentioned member.
    for id in &direct {
        bus.publish(
            &mut *conn,
            &format!("/chat/{channel_id}/new-mentions"),
            &json!({ "message_id": message_id, "channel_id": channel_id }),
            Some(&[crate::bus::user_tag(*id)]),
        )
        .await?;
    }
    let timestamp = format!("{} UTC", created_at.format("%Y-%m-%d %H:%M:%S"));
    crate::jobs::enqueue(
        &mut *conn,
        NOTIFY_MENTIONED_JOB,
        json!({
            "chat_message_id": message_id,
            "to_notify_ids_map": {
                "direct_mentions": direct,
                "here_mentions": [],
                "global_mentions": [],
            },
            "already_notified_user_ids": [],
            "timestamp": timestamp,
        }),
    )
    .await?;
    let mut except: Vec<i32> = direct.clone();
    except.push(user_id);
    crate::jobs::enqueue_in(
        &mut *conn,
        5,
        NOTIFY_WATCHING_JOB,
        json!({
            "chat_message_id": message_id,
            "except_user_ids": except,
            "timestamp": timestamp,
        }),
    )
    .await?;
    Ok(())
}
