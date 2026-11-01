//! Archiving a category channel: POST /chat/api/channels/:id/archives
//! (Chat::Api::ChannelsArchivesController: a new archive, or the retry of
//! a failed one), `Chat::ChannelArchiveService` (the channel made read
//! only, the chat_channel_archives row that tracks the destination and
//! the progress, the messages quoted into posts by the system user in
//! batches through the TranscriptService and trashed as each post is
//! made, the members unfollowed, the channel archived, the archiver told
//! by a system message and the channel's audience over
//! /chat/channel-archive-status) and `Jobs::Chat::ChannelArchive`.
//!
//! Each batch's post and the trashing of its messages are two
//! transactions here (PostCreator commits its own), where Rails has one.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::channels::{CHANNEL_COLUMNS, ChannelRow, Context, Found};
use super::transcript::{TRANSCRIPT_COLUMNS, TranscriptMessage, TranscriptOpts};
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Jobs::Chat::ChannelArchive`
pub const ARCHIVE_JOB: &str = "Jobs::Chat::ChannelArchive";

/// `ChannelArchiveService::ARCHIVED_MESSAGES_PER_POST`
pub const ARCHIVED_MESSAGES_PER_POST: usize = 100;

const READ_ONLY: i32 = 1;
const ARCHIVED: i32 = 3;

/// A `Chat::ChannelArchive`.
#[derive(sqlx::FromRow, Clone, Debug)]
pub struct ArchiveRow {
    pub id: i64,
    pub chat_channel_id: i64,
    pub archived_by_id: i32,
    pub destination_topic_id: Option<i32>,
    pub destination_topic_title: Option<String>,
    pub destination_category_id: Option<i32>,
    pub destination_tags: Option<Vec<String>>,
    pub total_messages: i32,
    pub archived_messages: i32,
    pub archive_error: Option<String>,
}

const ARCHIVE_COLUMNS: &str = "id, chat_channel_id, archived_by_id, destination_topic_id, \
     destination_topic_title, destination_category_id, destination_tags, total_messages, \
     archived_messages, archive_error";

/// An exception the service raises (its message is what the archiver is
/// told and archive_error keeps).
#[derive(Debug)]
pub struct Raised(pub String);

impl std::fmt::Display for Raised {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Raised {}

/// How the controller answers.
#[derive(Debug)]
pub enum Archived {
    Done,
    NotFound,
    /// `Discourse::InvalidAccess`
    Forbidden,
    /// `ActionController::ParameterMissing`
    ParamMissing(&'static str),
    /// `Discourse::InvalidParameters`, bare or naming the parameter.
    InvalidParameters(Option<&'static str>),
    /// `ArchiveValidationError`: 400 with the new topic's errors.
    Invalid(Vec<String>),
}

/// The topic params `create_archive_process` takes.
#[derive(Default, Debug, Clone)]
pub struct TopicParams {
    pub topic_id: Option<String>,
    pub topic_title: Option<String>,
    pub category_id: Option<String>,
    pub tags: Option<Vec<String>>,
}

fn present(s: &Option<String>) -> bool {
    s.as_deref().is_some_and(|s| !s.trim().is_empty())
}

/// An integer column's cast of a param.
fn cast_integer(s: &Option<String>) -> Option<i32> {
    s.as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| crate::ruby::to_i(s) as i32)
}

/// Why TopicCreator#valid? stopped short of its errors.
enum TopicCheck {
    Valid,
    Invalid(Vec<String>),
    /// `ensure_can_create!(Topic, category)`
    Forbidden,
    /// `InvalidParameters.new(:category)`
    NoCategory,
}

/// `ChannelArchiveService.validate_topic_params`: TopicCreator#valid? for
/// a topic by the system user, with the archiver's guardian.
async fn validate_topic_params(
    conn: &mut sqlx::PgConnection,
    ctx: &crate::posting::Ctx<'_>,
    guardian: &Guardian,
    title: &str,
    category: Option<&str>,
    tags: &[String],
) -> Result<TopicCheck, AppError> {
    let s = ctx.settings;
    // find_category: digits only.
    let category_id = match category.filter(|c| !c.is_empty()) {
        Some(c) if c.chars().all(|ch| ch.is_ascii_digit()) => {
            sqlx::query_scalar::<_, i32>("SELECT id FROM categories WHERE id = $1")
                .bind(c.parse::<i32>().ok())
                .fetch_optional(&mut *conn)
                .await?
        }
        _ => None,
    };
    let can_create = guardian.can_create_topic(&mut *conn, s).await?
        && match category_id {
            Some(id) => guardian
                .topic_create_allowed_category_ids(&mut *conn, s)
                .await?
                .contains(&id),
            None => true,
        };
    if !can_create {
        return Ok(TopicCheck::Forbidden);
    }
    if category.is_some_and(|c| !c.trim().is_empty()) && category_id.is_none() {
        return Ok(TopicCheck::NoCategory);
    }
    let system = crate::web_hooks::system_guardian(&mut *conn).await?;
    let mut errors = crate::posting::validate::validate_topic(
        &mut *conn,
        ctx,
        &system,
        &crate::posting::validate::TopicInput {
            title,
            category_id,
            private_message: false,
        },
    )
    .await?;
    errors.extend(
        crate::posting::create::validate_topic_tags(
            &mut *conn,
            ctx,
            guardian,
            category_id,
            false,
            tags,
        )
        .await?,
    );
    Ok(if errors.is_empty() {
        TopicCheck::Valid
    } else {
        TopicCheck::Invalid(errors)
    })
}

async fn load_guardian(conn: &mut sqlx::PgConnection, user_id: i32) -> Result<Guardian, AppError> {
    let user = crate::session::current::SessionUser::load(&mut *conn, user_id)
        .await?
        .ok_or(Raised(
            "undefined method for nil (the archiving user is gone)".into(),
        ))?;
    Ok(Guardian::for_user(&mut *conn, &user).await?)
}

async fn find_archive(
    conn: &mut sqlx::PgConnection,
    channel_id: i64,
) -> Result<Option<ArchiveRow>, AppError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {ARCHIVE_COLUMNS} FROM chat_channel_archives WHERE chat_channel_id = $1 ORDER BY id LIMIT 1"
    ))
    .bind(channel_id)
    .fetch_optional(&mut *conn)
    .await?)
}

/// `Chat::ChannelArchive#complete?`: every message archived and none left.
async fn complete(conn: &mut sqlx::PgConnection, archive: &ArchiveRow) -> Result<bool, AppError> {
    if archive.archived_messages < archive.total_messages {
        return Ok(false);
    }
    let live: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM chat_messages WHERE chat_channel_id = $1 AND deleted_at IS NULL)",
    )
    .bind(archive.chat_channel_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(!live)
}

/// `Chat::ChannelArchive#failed?`
async fn failed(conn: &mut sqlx::PgConnection, archive: &ArchiveRow) -> Result<bool, AppError> {
    Ok(present(&archive.archive_error) && !complete(conn, archive).await?)
}

impl Context<'_> {
    /// POST /chat/api/channels/:channel_id/archives:
    /// `ChannelsArchivesController#create` with `params[:archive]`.
    pub async fn archive_channel(
        &mut self,
        state: &AppState,
        channel_id: &str,
        archive: Option<&Value>,
    ) -> Result<Archived, AppError> {
        let channel = match self.find_joinable(channel_id).await? {
            Found::Channel(c) => c,
            Found::NotFound => return Ok(Archived::NotFound),
            Found::Forbidden => return Ok(Archived::Forbidden),
        };
        if let Some(existing) = find_archive(&mut *self.conn, channel.id).await? {
            // ensure_can_change_channel_status!(channel, :archived), then
            // retry_archive_process for a failed archive alone.
            if !super::channel_settings::can_change_status(self.guardian, &channel, ARCHIVED)
                || !failed(&mut *self.conn, &existing).await?
            {
                return Ok(Archived::Forbidden);
            }
            crate::jobs::enqueue(
                &mut *self.conn,
                ARCHIVE_JOB,
                json!({ "chat_channel_archive_id": existing.id }),
            )
            .await?;
            return Ok(Archived::Done);
        }
        if !self.settings.get("chat_allow_archiving_channels")?.truthy() {
            return Ok(Archived::Forbidden);
        }
        // params.require(:archive).tap { |ca| ca.require(:type) }
        let archive = match archive {
            Some(Value::Object(a)) if !a.is_empty() => a,
            Some(Value::String(s)) if !s.is_empty() => {
                return Err(Unsupported("an archive param that isn't a hash").into());
            }
            _ => return Ok(Archived::ParamMissing("archive")),
        };
        let scalar = |key: &str| archive.get(key).and_then(crate::params::scalar);
        let Some(kind) = scalar("type").filter(|t| !t.is_empty()) else {
            return Ok(Archived::ParamMissing("type"));
        };
        let params = TopicParams {
            topic_id: scalar("topic_id"),
            topic_title: scalar("title"),
            category_id: scalar("category_id"),
            tags: match archive.get("tags") {
                Some(Value::Array(tags)) => {
                    Some(tags.iter().filter_map(crate::params::scalar).collect())
                }
                _ => None,
            },
        };
        let new_topic = kind == "new_topic";
        if new_topic && !present(&params.topic_title) {
            return Ok(Archived::InvalidParameters(None));
        }
        if !new_topic && !present(&params.topic_id) {
            return Ok(Archived::InvalidParameters(None));
        }
        if !new_topic {
            let id = cast_integer(&params.topic_id).unwrap_or(0);
            let s = self.settings;
            let topic =
                crate::topic_guardian::TopicCtx::load(&mut *self.conn, s, self.guardian, id)
                    .await?;
            let allowed = match &topic {
                Some(t) => {
                    let secure = self
                        .guardian
                        .secure_category_ids(&mut *self.conn, s)
                        .await?;
                    let anywhere = self
                        .guardian
                        .can_create_post_anywhere(&mut *self.conn, s)
                        .await?;
                    self.guardian.can_see_topic(s, t, true, &secure)?
                        && self.guardian.can_create_post_on_topic(s, t, anywhere)?
                }
                None => false,
            };
            if !allowed {
                return Ok(Archived::Forbidden);
            }
        }
        if !super::channel_settings::can_change_status(self.guardian, &channel, READ_ONLY) {
            return Ok(Archived::Forbidden);
        }
        self.create_archive_process(state, &channel, &params).await
    }

    /// `ChannelArchiveService.create_archive_process`: nothing for a
    /// channel archived already; the new topic's params validated; the
    /// channel made read only, the archive saved and its job enqueued.
    async fn create_archive_process(
        &mut self,
        state: &AppState,
        channel: &ChannelRow,
        params: &TopicParams,
    ) -> Result<Archived, AppError> {
        if find_archive(&mut *self.conn, channel.id).await?.is_some() {
            return Ok(Archived::Done);
        }
        if !present(&params.topic_id) {
            let host = crate::pretty_text::Host::from_state(state);
            let ctx = crate::posting::Ctx {
                host: &host,
                settings: self.settings,
                config: &state.config,
                i18n: &state.i18n,
                bus: &state.bus,
            };
            match validate_topic_params(
                &mut *self.conn,
                &ctx,
                self.guardian,
                params.topic_title.as_deref().unwrap_or(""),
                params.category_id.as_deref(),
                params.tags.as_deref().unwrap_or(&[]),
            )
            .await?
            {
                TopicCheck::Valid => {}
                TopicCheck::Invalid(errors) => return Ok(Archived::Invalid(errors)),
                TopicCheck::Forbidden => return Ok(Archived::Forbidden),
                TopicCheck::NoCategory => return Ok(Archived::InvalidParameters(Some("category"))),
            }
        }
        if params
            .topic_title
            .as_deref()
            .is_some_and(|t| t.chars().count() > 1000)
        {
            return Err(Unsupported(
                "a chat archive's destination title over 1000 characters (RecordInvalid)",
            )
            .into());
        }
        let user_id = self.guardian.user_id().unwrap_or(0);
        // chat_channel.read_only!(acting_user)
        if super::channel_settings::can_change_status(self.guardian, channel, READ_ONLY) {
            self.change_status(&state.bus, channel, READ_ONLY, user_id)
                .await?;
        }
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM chat_messages WHERE chat_channel_id = $1 AND deleted_at IS NULL",
        )
        .bind(channel.id)
        .fetch_one(&mut *self.conn)
        .await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO chat_channel_archives (chat_channel_id, archived_by_id, total_messages, \
               destination_topic_id, destination_topic_title, destination_category_id, \
               destination_tags, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, clock_timestamp(), clock_timestamp()) RETURNING id",
        )
        .bind(channel.id)
        .bind(user_id)
        .bind(total as i32)
        .bind(cast_integer(&params.topic_id))
        .bind(&params.topic_title)
        .bind(cast_integer(&params.category_id))
        .bind(&params.tags)
        .fetch_one(&mut *self.conn)
        .await?;
        crate::jobs::enqueue(
            &mut *self.conn,
            ARCHIVE_JOB,
            json!({ "chat_channel_archive_id": id }),
        )
        .await?;
        Ok(Archived::Done)
    }
}

/// `Jobs::Chat::ChannelArchive`: a complete archive's channel marked
/// archived and the status published; otherwise the service, under the
/// channel's mutex. `sidekiq_options retry: false`: a failure (which the
/// service has recorded and told the archiver about) is logged, not
/// retried.
pub async fn channel_archive_job(state: &AppState, args: &Value) -> Result<(), AppError> {
    let id = args["chat_channel_archive_id"].as_i64().or_else(|| {
        args["chat_channel_archive_id"]
            .as_str()
            .and_then(|s| s.parse().ok())
    });
    let mut conn = state.pool.acquire().await?;
    let archive: Option<ArchiveRow> = sqlx::query_as(&format!(
        "SELECT {ARCHIVE_COLUMNS} FROM chat_channel_archives WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(archive) = archive else {
        tracing::warn!(
            "Chat channel archive {} could not be found, aborting archive job.",
            id.map(|i| i.to_string()).unwrap_or_default()
        );
        return Ok(());
    };
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if complete(&mut conn, &archive).await? {
        let channel = channel_row(&mut conn, archive.chat_channel_id).await?;
        sqlx::query(
            "UPDATE chat_channels SET status = $2, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(channel.id)
        .bind(ARCHIVED)
        .execute(&mut *conn)
        .await?;
        super::publisher::publish_archive_status(
            &mut conn,
            &state.bus,
            &settings,
            &channel,
            &super::publisher::ArchiveStatus {
                failed: false,
                archived_messages: archive.archived_messages,
                archive_topic_id: archive.destination_topic_id,
                total_messages: archive.total_messages,
            },
        )
        .await?;
        return Ok(());
    }
    drop(conn);
    // DistributedMutex "archive_chat_channel_<id>", held by a transaction
    // of its own while the service works.
    let mut mutex = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("archive_chat_channel_{}", archive.chat_channel_id))
        .execute(&mut *mutex)
        .await?;
    let result = execute(state, &settings, archive.id).await;
    mutex.commit().await?;
    if let Err(e) = result {
        tracing::error!(archive = archive.id, error = %e, "chat channel archive failed");
    }
    Ok(())
}

async fn channel_row(conn: &mut sqlx::PgConnection, id: i64) -> Result<ChannelRow, AppError> {
    sqlx::query_as(&format!(
        "SELECT {CHANNEL_COLUMNS} FROM chat_channels WHERE id = $1 AND deleted_at IS NULL"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| Raised("undefined method for nil (the archived channel is gone)".into()).into())
}

/// How `notify_archiver` was asked to report.
#[derive(Clone, Copy, PartialEq)]
enum Outcome {
    Success,
    Failed,
    FailedNoTopic,
}

/// `Chat::ChannelArchiveService.new(archive).execute`: idempotent, so a
/// failed archive runs again from where it stopped.
pub async fn execute(
    state: &AppState,
    settings: &SiteSettings,
    archive_id: i64,
) -> Result<(), AppError> {
    execute_with(state, settings, archive_id, ARCHIVED_MESSAGES_PER_POST).await
}

/// [`execute`] with that many messages a post (which chat's specs stub).
pub async fn execute_with(
    state: &AppState,
    settings: &SiteSettings,
    archive_id: i64,
    per_post: usize,
) -> Result<(), AppError> {
    let mut service = Service::new(state, settings, archive_id, per_post.max(2)).await?;
    sqlx::query("UPDATE chat_channel_archives SET archive_error = NULL, updated_at = clock_timestamp() WHERE id = $1")
        .bind(archive_id)
        .execute(&state.pool)
        .await?;
    service.archive.archive_error = None;
    match service.run().await {
        Ok(()) => Ok(()),
        Err(e) => {
            service
                .notify_archiver(Outcome::Failed, Some(&e.to_string()))
                .await?;
            Err(e)
        }
    }
}

/// The service's state: the archive, its channel and the archiver.
struct Service<'a> {
    state: &'a AppState,
    settings: &'a SiteSettings,
    host: crate::pretty_text::Host,
    archive: ArchiveRow,
    channel: ChannelRow,
    guardian: Guardian,
    title: String,
    archived_message_ids: Vec<i64>,
    batch_size: usize,
}

impl<'a> Service<'a> {
    async fn new(
        state: &'a AppState,
        settings: &'a SiteSettings,
        archive_id: i64,
        batch_size: usize,
    ) -> Result<Service<'a>, AppError> {
        let mut conn = state.pool.acquire().await?;
        let archive: ArchiveRow = sqlx::query_as(&format!(
            "SELECT {ARCHIVE_COLUMNS} FROM chat_channel_archives WHERE id = $1"
        ))
        .bind(archive_id)
        .fetch_one(&mut *conn)
        .await?;
        let channel = channel_row(&mut conn, archive.chat_channel_id).await?;
        let guardian = load_guardian(&mut conn, archive.archived_by_id).await?;
        let mut service = Service {
            state,
            settings,
            host: crate::pretty_text::Host::from_state(state),
            archive,
            channel,
            guardian,
            title: String::new(),
            archived_message_ids: Vec::new(),
            batch_size,
        };
        let channel = service.channel.clone();
        service.title = service
            .context(&mut conn)
            .channel(&channel, None, None, None)
            .await?["title"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Ok(service)
    }

    fn context<'c>(&'c self, conn: &'c mut sqlx::PgConnection) -> Context<'c> {
        Context {
            conn,
            settings: self.settings,
            i18n: &self.state.i18n,
            guardian: &self.guardian,
            base_path: self.state.config.globals.relative_url_root(),
            config: &self.state.config,
        }
    }

    fn ctx(&self) -> crate::posting::Ctx<'_> {
        crate::posting::Ctx {
            host: &self.host,
            settings: self.settings,
            config: &self.state.config,
            i18n: &self.state.i18n,
            bus: &self.state.bus,
        }
    }

    fn base_url(&self) -> Result<String, AppError> {
        Ok(crate::url::Urls {
            config: &self.state.config,
            settings: self.settings,
        }
        .base_url()?)
    }

    /// `Chat::Channel#url`
    fn channel_url(&self) -> Result<String, AppError> {
        Ok(format!(
            "{}/chat/c/{}/{}",
            self.base_url()?,
            self.channel.slug.as_deref().unwrap_or("-"),
            self.channel.id
        ))
    }

    /// The archive's destination topic (not trashed): its title and slug.
    async fn destination_topic(&self) -> Result<Option<(i32, String, Option<String>)>, AppError> {
        let Some(id) = self.archive.destination_topic_id else {
            return Ok(None);
        };
        Ok(sqlx::query_as(
            "SELECT id, title, slug FROM topics WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.state.pool)
        .await?)
    }

    /// The batches, each thread kept whole where it fits and split with
    /// its original message repeated and its range where it doesn't, the
    /// members unfollowed and the archive completed.
    async fn run(&mut self) -> Result<(), AppError> {
        if !self.ensure_destination_topic_exists().await? {
            return Ok(());
        }
        tracing::info!(
            "Creating posts from message batches for {} archive, {} messages to archive ({} posts).",
            self.title,
            self.archive.total_messages,
            self.archive.total_messages as usize / self.batch_size
        );
        let n = self.batch_size;
        let mut buffer: Vec<TranscriptMessage> = Vec::new();
        let mut batch_thread_ranges: HashMap<i64, Option<String>> = HashMap::new();
        let mut cursor: Option<(chrono::NaiveDateTime, i64)> = None;
        loop {
            // find_in_batches(cursor: %i[created_at id], order: :asc)
            let message_batch: Vec<TranscriptMessage> = sqlx::query_as(&format!(
                "SELECT {TRANSCRIPT_COLUMNS} FROM chat_messages \
                 LEFT JOIN users ON users.id = chat_messages.user_id \
                 WHERE chat_messages.chat_channel_id = $1 AND chat_messages.deleted_at IS NULL \
                   AND ($2::timestamp IS NULL OR (chat_messages.created_at, chat_messages.id) > ($2, $3)) \
                 ORDER BY chat_messages.created_at, chat_messages.id LIMIT $4"
            ))
            .bind(self.channel.id)
            .bind(cursor.map(|c| c.0))
            .bind(cursor.map_or(0, |c| c.1))
            .bind(n as i64)
            .fetch_all(&self.state.pool)
            .await?;
            let Some(last) = message_batch.last() else {
                break;
            };
            cursor = Some((last.created_at, last.id));
            let fetched = message_batch.len();

            let mut thread_ids: Vec<i64> =
                message_batch.iter().filter_map(|m| m.thread_id).collect();
            thread_ids.sort_unstable();
            thread_ids.dedup();
            let threads: Vec<TranscriptMessage> = sqlx::query_as(&format!(
                "SELECT {TRANSCRIPT_COLUMNS} FROM chat_messages \
                 LEFT JOIN users ON users.id = chat_messages.user_id \
                 WHERE chat_messages.chat_channel_id = $1 AND chat_messages.deleted_at IS NULL \
                   AND chat_messages.thread_id IN (SELECT thread_id FROM chat_messages \
                     WHERE thread_id = ANY($2) AND deleted_at IS NULL \
                     GROUP BY thread_id HAVING COUNT(*) > 1) \
                 ORDER BY chat_messages.created_at, chat_messages.id"
            ))
            .bind(self.channel.id)
            .bind(&thread_ids)
            .fetch_all(&self.state.pool)
            .await?;

            // (buffer + message_batch + threads).uniq, grouped by thread.
            let mut full_batch: Vec<TranscriptMessage> = Vec::new();
            for m in buffer
                .drain(..)
                .chain(message_batch)
                .chain(threads.iter().cloned())
            {
                if !full_batch.iter().any(|f| f.id == m.id) {
                    full_batch.push(m);
                }
            }
            let message_chunk = group_by_thread(full_batch);

            let mut thread_om: Option<TranscriptMessage> = None;
            let mut thread_ranges: HashMap<i64, Vec<String>> = HashMap::new();
            if message_chunk.len() > n {
                let post_last_message = &message_chunk[n - 1];
                let thread: Vec<&TranscriptMessage> = threads
                    .iter()
                    .filter(|m| m.thread_id == post_last_message.thread_id)
                    .collect();
                if let Some(om) = thread.first() {
                    thread_om = Some((*om).clone());
                    thread_ranges = self.calculate_thread_ranges(
                        &message_chunk,
                        &thread,
                        om,
                        post_last_message,
                    );
                }
            }

            let mut batch: Vec<TranscriptMessage> = Vec::new();
            let mut batch_thread_added = false;
            for message in &message_chunk {
                // A thread split across posts: its original message again
                // ahead of the rest, for context, with each part's range.
                if let Some(om) = &thread_om {
                    let om_thread = om.thread_id.unwrap_or_default();
                    if batch.is_empty()
                        && message_chunk.len() > n
                        && message.thread_id == om.thread_id
                        && message.id != om.id
                    {
                        batch.push(om.clone());
                        let ranges = thread_ranges.entry(om_thread).or_default();
                        let first = ranges.first().cloned();
                        batch_thread_ranges.insert(om.id, first);
                        if !ranges.is_empty() {
                            ranges.remove(0);
                        }
                    } else if let Some(t) = message.thread_id
                        && thread_ranges.get(&t).is_some_and(|r| !r.is_empty())
                        && !batch_thread_added
                    {
                        let ranges = thread_ranges.entry(t).or_default();
                        batch_thread_ranges.insert(om.id, ranges.first().cloned());
                        ranges.remove(0);
                        batch_thread_added = true;
                    }
                }
                let batch_size = if thread_om.as_ref().is_some_and(|om| om.id == message.id)
                    && batch.len() + 1 >= n
                {
                    batch.len() + 1
                } else {
                    batch.push(message.clone());
                    batch.len()
                };
                if batch_size >= n {
                    self.create_post_from_batch(&batch, &batch_thread_ranges)
                        .await?;
                    batch.clear();
                }
            }
            buffer.extend(batch);
            if fetched < n {
                break;
            }
        }
        if !buffer.is_empty() {
            self.create_post_from_batch(&buffer, &batch_thread_ranges)
                .await?;
        }
        self.kick_all_users().await?;
        self.complete_archive().await
    }

    /// `calculate_thread_ranges`: the parts a thread is split into across
    /// the posts of a chunk larger than one post.
    fn calculate_thread_ranges(
        &self,
        message_chunk: &[TranscriptMessage],
        thread: &[&TranscriptMessage],
        thread_om: &TranscriptMessage,
        post_last_message: &TranscriptMessage,
    ) -> HashMap<i64, Vec<String>> {
        let n = self.batch_size;
        let mut ranges: HashMap<i64, Vec<String>> = HashMap::new();
        let thread_size = thread.len() - 1;
        let mut last_thread_index = 0usize;
        let iterations = message_chunk.len().div_ceil(n - 1);
        for index in 0..iterations {
            if last_thread_index == thread_size {
                continue;
            }
            let thread_index = if index == 0 {
                thread
                    .iter()
                    .position(|m| m.id == post_last_message.id)
                    .unwrap_or(0)
            } else {
                let next = message_chunk.get(n * (index + 1) - index);
                if next.is_some_and(|m| m.thread_id == post_last_message.thread_id) {
                    last_thread_index + n - 1
                } else {
                    thread_size
                }
            };
            let range = self
                .state
                .i18n
                .t_with(
                    "chat.transcript.split_thread_range",
                    &[
                        ("start", &(last_thread_index + 1).to_string()),
                        ("end", &thread_index.to_string()),
                        ("total", &thread_size.to_string()),
                    ],
                )
                .unwrap_or_default();
            ranges
                .entry(thread_om.thread_id.unwrap_or_default())
                .or_default()
                .push(range);
            last_thread_index = thread_index;
        }
        ranges
    }

    /// A post of the batch's transcript (noLink, reactions), then the
    /// batch trashed and counted.
    async fn create_post_from_batch(
        &mut self,
        messages: &[TranscriptMessage],
        thread_ranges: &HashMap<i64, Option<String>>,
    ) -> Result<(), AppError> {
        let mut conn = self.state.pool.acquire().await?;
        let channel = self.channel.clone();
        let raw = self
            .context(&mut conn)
            .generate_transcript(
                &channel,
                messages.to_vec(),
                thread_ranges,
                TranscriptOpts {
                    no_link: true,
                    include_reactions: true,
                },
            )
            .await?;
        drop(conn);
        self.create_post(raw, self.archive.destination_topic_id)
            .await?;
        self.delete_message_batch(messages.iter().map(|m| m.id).collect())
            .await
    }

    /// `create_post`: PostCreator as the system user, in import mode,
    /// without validations, to the destination topic (or a new topic with
    /// `new_topic`'s title, category and tags). The post's id.
    async fn create_post_with(
        &self,
        raw: String,
        topic_id: Option<i32>,
        new_topic: Option<(String, Option<String>, Option<Vec<String>>)>,
    ) -> Result<i32, AppError> {
        let mut conn = self.state.pool.acquire().await?;
        let system = crate::web_hooks::system_guardian(&mut conn).await?;
        drop(conn);
        let ctx = self.ctx();
        let (title, category, tags) = match new_topic {
            Some((title, category, tags)) => (Some(title), category, tags),
            None => (None, None, None),
        };
        let outcome = Box::pin(crate::posting::create::create(
            &self.state.pool,
            &ctx,
            &system,
            crate::posting::create::NewPost {
                raw,
                topic_id,
                title,
                category,
                tags: tags
                    .filter(|t| !t.is_empty())
                    .map(crate::tagging::TagsParam::Names),
                creator_guardian: Some(self.guardian.clone()),
                skip_validations: true,
                import_mode: true,
                ..Default::default()
            },
        ))
        .await?;
        match outcome {
            crate::posting::create::Outcome::Created { post_id } => Ok(post_id),
            crate::posting::create::Outcome::Invalid(errors) => {
                Err(Raised(errors.join(", ")).into())
            }
            other => Err(Raised(format!("the archive post was not created: {other:?}")).into()),
        }
    }

    async fn create_post(&self, raw: String, topic_id: Option<i32>) -> Result<i32, AppError> {
        self.create_post_with(raw, topic_id, None).await
    }

    /// `ensure_destination_topic_exists!`: a new topic made (as the system
    /// user, with the archiver's guardian) with its first post, or the
    /// archiver told why it couldn't be; then its status.
    async fn ensure_destination_topic_exists(&mut self) -> Result<bool, AppError> {
        if self.destination_topic().await?.is_none() {
            tracing::info!("Creating topic for {} archive.", self.title);
            let title = self
                .archive
                .destination_topic_title
                .clone()
                .unwrap_or_default();
            let category = self.archive.destination_category_id.map(|c| c.to_string());
            let tags = self.archive.destination_tags.clone().unwrap_or_default();
            let mut conn = self.state.pool.acquire().await?;
            let check = validate_topic_params(
                &mut conn,
                &self.ctx(),
                &self.guardian,
                &title,
                category.as_deref(),
                &tags,
            )
            .await?;
            drop(conn);
            match check {
                TopicCheck::Valid => {
                    let raw = self
                        .state
                        .i18n
                        .t_with(
                            "chat.channel.archive.first_post_raw",
                            &[
                                ("channel_name", &self.title),
                                ("channel_url", &self.channel_url()?),
                            ],
                        )
                        .unwrap_or_default();
                    tracing::info!("Creating first post for {} archive.", self.title);
                    let post_id = self
                        .create_post_with(raw, None, Some((title, category, Some(tags))))
                        .await?;
                    // TopicCreator made the topic and PostCreator its first
                    // post as a reply, which leaves the excerpt unset.
                    let topic_id: i32 = sqlx::query_scalar(
                        "UPDATE topics SET excerpt = NULL \
                         WHERE id = (SELECT topic_id FROM posts WHERE id = $1) RETURNING id",
                    )
                    .bind(post_id)
                    .fetch_one(&self.state.pool)
                    .await?;
                    sqlx::query(
                        "UPDATE chat_channel_archives SET destination_topic_id = $2, \
                           updated_at = clock_timestamp() WHERE id = $1",
                    )
                    .bind(self.archive.id)
                    .bind(topic_id)
                    .execute(&self.state.pool)
                    .await?;
                    self.archive.destination_topic_id = Some(topic_id);
                }
                TopicCheck::Invalid(errors) => {
                    tracing::info!(
                        "Destination topic for {} archive was not valid.",
                        self.title
                    );
                    self.notify_archiver(Outcome::FailedNoTopic, Some(&errors.join("\n")))
                        .await?;
                }
                TopicCheck::Forbidden => {
                    return Err(Raised("Discourse::InvalidAccess".into()).into());
                }
                TopicCheck::NoCategory => return Err(Raised("category".into()).into()),
            }
        } else {
            tracing::info!("Topic already exists for {} archive.", self.title);
        }
        if self.destination_topic().await?.is_some() {
            self.update_destination_topic_status().await?;
            return Ok(true);
        }
        Ok(false)
    }

    /// `update_destination_topic_status`: a new topic (one with a title
    /// saved) archived or closed as chat_archive_destination_topic_status
    /// says; an existing topic left as it is.
    async fn update_destination_topic_status(&self) -> Result<(), AppError> {
        if !present(&self.archive.destination_topic_title) {
            return Ok(());
        }
        let column = match self
            .settings
            .get("chat_archive_destination_topic_status")?
            .to_s()
            .as_str()
        {
            "archived" => "archived",
            "closed" => "closed",
            _ => return Ok(()),
        };
        sqlx::query(&format!(
            "UPDATE topics SET {column} = TRUE, updated_at = clock_timestamp() WHERE id = $1"
        ))
        .bind(self.archive.destination_topic_id)
        .execute(&self.state.pool)
        .await?;
        Ok(())
    }

    /// `delete_message_batch`: the live messages among the batch trashed
    /// by the archiver, and the archive's count of this run's messages.
    async fn delete_message_batch(&mut self, ids: Vec<i64>) -> Result<(), AppError> {
        let mut tx = self.state.pool.begin().await?;
        sqlx::query(
            "UPDATE chat_messages SET deleted_at = clock_timestamp(), deleted_by_id = $2 \
             WHERE id = ANY($1) AND deleted_at IS NULL",
        )
        .bind(&ids)
        .bind(self.archive.archived_by_id)
        .execute(&mut *tx)
        .await?;
        for id in ids {
            if !self.archived_message_ids.contains(&id) {
                self.archived_message_ids.push(id);
            }
        }
        self.archive.archived_messages = self.archived_message_ids.len() as i32;
        sqlx::query(
            "UPDATE chat_channel_archives SET archived_messages = $2, updated_at = clock_timestamp() \
             WHERE id = $1",
        )
        .bind(self.archive.id)
        .bind(self.archive.archived_messages)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        tracing::info!(
            "Archived {} messages for {} archive.",
            self.archive.archived_messages,
            self.title
        );
        Ok(())
    }

    /// `kick_all_users`: ChannelMembershipManager#unfollow_all_users, the
    /// last read message the channel's last live one (none now).
    async fn kick_all_users(&self) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE user_chat_channel_memberships SET following = FALSE, \
               last_read_message_id = (SELECT MAX(id) FROM chat_messages \
                 WHERE chat_channel_id = $1 AND deleted_at IS NULL) \
             WHERE chat_channel_id = $1",
        )
        .bind(self.channel.id)
        .execute(&self.state.pool)
        .await?;
        Ok(())
    }

    /// `complete_archive`: `chat_channel.archived!(archived_by)`, then the
    /// archiver told.
    async fn complete_archive(&mut self) -> Result<(), AppError> {
        tracing::info!("Creating posts completed for {} archive.", self.title);
        let mut conn = self.state.pool.acquire().await?;
        let channel = channel_row(&mut conn, self.channel.id).await?;
        if super::channel_settings::can_change_status(&self.guardian, &channel, ARCHIVED) {
            let user_id = self.archive.archived_by_id;
            self.context(&mut conn)
                .change_status(&self.state.bus, &channel, ARCHIVED, user_id)
                .await?;
        }
        drop(conn);
        self.notify_archiver(Outcome::Success, None).await
    }

    /// `notify_archiver`: a failure logged and kept as archive_error, a
    /// system message to the archiver, the status published.
    async fn notify_archiver(
        &mut self,
        outcome: Outcome,
        error_message: Option<&str>,
    ) -> Result<(), AppError> {
        let topic = self.destination_topic().await?;
        let base_url = self.base_url()?;
        let hashtag_or_name = match self.channel.slug.as_deref().filter(|s| !s.is_empty()) {
            Some(slug) => format!("#{slug}::channel"),
            None => self.title.clone(),
        };
        let topic_url = topic.as_ref().map(|(id, _, slug)| match slug.as_deref() {
            Some(slug) if !slug.is_empty() => format!("{base_url}/t/{slug}/{id}"),
            _ => format!("{base_url}/t/{id}"),
        });
        let mut params: Vec<(String, String)> = vec![
            ("channel_hashtag_or_name".into(), hashtag_or_name),
            (
                "topic_title".into(),
                topic.as_ref().map(|t| t.1.clone()).unwrap_or_default(),
            ),
            ("topic_url".into(), topic_url.unwrap_or_default()),
            (
                "topic_validation_errors".into(),
                if outcome == Outcome::FailedNoTopic {
                    error_message.unwrap_or_default().to_string()
                } else {
                    String::new()
                },
            ),
        ];
        let message_type = if outcome == Outcome::Success {
            "chat_channel_archive_complete"
        } else {
            let error = error_message.unwrap_or_default();
            tracing::warn!(
                chat_channel_id = self.channel.id,
                chat_channel_name = %self.title,
                "Error when archiving chat channel {}. {error}",
                self.title
            );
            params.push(("channel_url".into(), self.channel_url()?));
            params.push((
                "messages_archived".into(),
                self.archive.archived_messages.to_string(),
            ));
            // archive.update(archive_error:), which the length validation
            // may refuse.
            if error.chars().count() <= 1000 {
                sqlx::query(
                    "UPDATE chat_channel_archives SET archive_error = $2, updated_at = clock_timestamp() \
                     WHERE id = $1",
                )
                .bind(self.archive.id)
                .bind(error)
                .execute(&self.state.pool)
                .await?;
                self.archive.archive_error = Some(error.to_string());
            }
            if outcome == Outcome::Failed {
                "chat_channel_archive_failed"
            } else {
                "chat_channel_archive_failed_no_topic"
            }
        };
        crate::system_message::create_from_system_user(
            &self.state.pool,
            &self.ctx(),
            self.archive.archived_by_id,
            message_type,
            &params,
        )
        .await?;
        let mut conn = self.state.pool.acquire().await?;
        super::publisher::publish_archive_status(
            &mut conn,
            &self.state.bus,
            self.settings,
            &self.channel,
            &super::publisher::ArchiveStatus {
                failed: outcome != Outcome::Success,
                archived_messages: self.archive.archived_messages,
                archive_topic_id: self.archive.destination_topic_id,
                total_messages: self.archive.total_messages,
            },
        )
        .await?;
        Ok(())
    }
}

/// `group_by { thread or message }.values.flatten`: each thread's
/// messages together, where its first one stood.
fn group_by_thread(messages: Vec<TranscriptMessage>) -> Vec<TranscriptMessage> {
    let mut keys: Vec<(bool, i64)> = Vec::new();
    let mut groups: HashMap<(bool, i64), Vec<TranscriptMessage>> = HashMap::new();
    for m in messages {
        let key = match m.thread_id {
            Some(t) => (true, t),
            None => (false, m.id),
        };
        if !groups.contains_key(&key) {
            keys.push(key);
        }
        groups.entry(key).or_default().push(m);
    }
    keys.iter()
        .flat_map(|k| groups.remove(k).unwrap_or_default())
        .collect()
}
