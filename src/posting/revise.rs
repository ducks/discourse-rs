//! `PostRevisor#revise!` for the fields posts#update sends: the raw and
//! the edit reason, and on a first post the title and category, which go
//! through revise_topic's `update_topic` in the same revision (or on their
//! own when the post itself stays the same). Owner and wiki changes are
//! not ported.
//!
//! Within the editing grace period an edit by the last editor goes into
//! the current version: Rails keeps the post's original raw and cooked in
//! Redis (here discourse_rs.cached_values, owned_schema's setex) for the
//! grace period's diff and the version made later, and folds the change
//! into the current revision (update_revision).

use chrono::NaiveDateTime;
use serde_json::{Value, json};
use sqlx::PgPool;

use super::create::omit_nofollow;
use super::links::{self, LinkPost};
use super::search_index::{self, BaseUrls, PostIndex};
use super::text::{normalize_whitespaces, word_count};
use super::validate::{self, PostInput};
use super::{BAKED_VERSION, Ctx, next_draft_sequence};
use crate::discourse_diff::diff_size;
use crate::guardian::Guardian;
use crate::modifications;
use crate::pretty_text::{self, MarkdownOptions};
use crate::url::Urls;
use crate::{AppError, Unsupported};

use super::revise_topic::{self, Deferred, TopicFields};

/// `Post::MAX_EDIT_REASON_LENGTH`
const MAX_EDIT_REASON_LENGTH: usize = 1000;

/// What posts#update hands the revisor.
#[derive(Default)]
pub struct Changes {
    pub raw: Option<String>,
    pub edit_reason: Option<String>,
    /// `force_new_version:` a new version even within the grace period.
    pub force_new_version: bool,
    /// `skip_validations:` the post is saved without PostValidator.
    pub skip_validations: bool,
    /// `skip_revision:` the edit goes into the current version: no new
    /// version and no PostRevision (the original kept in Redis).
    pub skip_revision: bool,
    /// A first post's topic fields (title, category).
    pub topic: TopicFields,
    /// `bypass_bump:` no wiki bump.
    pub bypass_bump: bool,
    /// `expected_raw:` the revision is refused (`EditConflict`) when the
    /// post's raw, locked in the revision's transaction, is no longer it.
    pub expected_raw: Option<String>,
    /// `preserve_cooked_token:` sent with the revised message, so the
    /// client that already drew the change keeps its drawing.
    pub preserve_cooked_token: Option<String>,
}

#[derive(Debug)]
pub enum Outcome {
    Revised,
    /// EditRateLimiter refused it (raised in the transaction: the edit
    /// rolled back).
    RateLimited(crate::rate_limiter::LimitExceeded),
    /// The post's validation errors: a 422 `render_json_error(post)`.
    Invalid(Vec<String>),
    /// `expected_raw` is no longer the post's raw: the `:edit_conflict`
    /// error, nothing saved.
    EditConflict,
}

#[derive(sqlx::FromRow)]
struct PostRow {
    id: i32,
    user_id: Option<i32>,
    topic_id: i32,
    post_number: i32,
    raw: String,
    cooked: String,
    edit_reason: Option<String>,
    last_editor_id: Option<i32>,
    version: i32,
    public_version: i32,
    last_version_at: NaiveDateTime,
    hidden: bool,
    wiki: bool,
    post_type: i32,
    self_edits: i32,
    reply_to_post_number: Option<i32>,
    locale: Option<String>,
    reply_quoted: bool,
}

/// `PostRevisor#update_revision`: the current version's revision takes
/// the change (a field back to where the revision started leaves it); a
/// revision left empty goes, and the post back to the version before it
/// (true: that second save is the last, its changes without the cooked).
async fn update_revision(
    conn: &mut sqlx::PgConnection,
    post: &PostRow,
    editor_id: i32,
    changes: &[(&str, [Value; 2])],
) -> Result<bool, AppError> {
    let revision: Option<(i32, Option<String>, bool)> = sqlx::query_as(
        "SELECT id, modifications, hidden FROM post_revisions WHERE post_id = $1 AND number = $2",
    )
    .bind(post.id)
    .bind(post.version)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((revision_id, yaml, hidden)) = revision else {
        return Ok(false);
    };
    let mut fields = modifications::load(yaml.as_deref().unwrap_or("--- {}\n"))?;
    for (field, change) in changes {
        let new = &change[1];
        match fields.iter().position(|(f, _)| f == field) {
            Some(i) => {
                let old = fields[i].1[0].clone();
                if ruby_to_s(&old) != ruby_to_s(new) {
                    fields[i].1 = [old, new.clone()];
                } else {
                    fields.remove(i);
                }
            }
            None => fields.push((field.to_string(), change.clone())),
        }
    }
    if fields.is_empty() {
        sqlx::query("DELETE FROM post_revisions WHERE id = $1")
            .bind(revision_id)
            .execute(&mut *conn)
            .await?;
        let last_editor: Option<i32> = sqlx::query_scalar(
            "SELECT COALESCE((SELECT user_id FROM post_revisions WHERE post_id = $1 \
                              ORDER BY number DESC LIMIT 1), $2)",
        )
        .bind(post.id)
        .bind(post.user_id)
        .fetch_one(&mut *conn)
        .await?;
        sqlx::query(
            "UPDATE posts SET last_editor_id = $2, version = version - 1, \
                    public_version = public_version - $3, updated_at = clock_timestamp() \
             WHERE id = $1",
        )
        .bind(post.id)
        .bind(last_editor)
        .bind(i32::from(!hidden))
        .execute(&mut *conn)
        .await?;
        return Ok(true);
    }
    let borrowed: Vec<(&str, [Value; 2])> = fields
        .iter()
        .map(|(f, change)| (f.as_str(), change.clone()))
        .collect();
    sqlx::query(
        "UPDATE post_revisions SET user_id = $2, modifications = $3, updated_at = clock_timestamp() \
         WHERE id = $1",
    )
    .bind(revision_id)
    .bind(editor_id)
    .bind(modifications::dump(&borrowed)?)
    .execute(&mut *conn)
    .await?;
    Ok(false)
}

/// Ruby's `to_s` on a revision value: nil is empty.
fn ruby_to_s(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// `PostRevisor.new(post, topic).revise!(editor, changes)`
pub async fn revise(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    editor: &Guardian,
    post_id: i32,
    changes: Changes,
) -> Result<Outcome, AppError> {
    let s = ctx.settings;
    let editor_user = editor
        .user()
        .ok_or(Unsupported("editing anonymously"))?
        .clone();
    let mut conn = pool.acquire().await?;
    let post: PostRow = sqlx::query_as(
        "SELECT id, user_id, topic_id, post_number, raw, cooked, edit_reason, last_editor_id, version, \
                public_version, last_version_at, hidden, wiki, post_type, self_edits, reply_to_post_number, locale, \
                reply_quoted \
         FROM posts WHERE id = $1",
    )
    .bind(post_id)
    .fetch_one(&mut *conn)
    .await?;
    let (topic_title, category_id, archetype, slow_mode_seconds): (
        String,
        Option<i32>,
        String,
        i32,
    ) = sqlx::query_as(
        "SELECT title, category_id, archetype, slow_mode_seconds FROM topics WHERE id = $1",
    )
    .bind(post.topic_id)
    .fetch_one(&mut *conn)
    .await?;
    // revise!: the fields, cleaned.
    let raw = changes
        .raw
        .map(|r| normalize_whitespaces(&r).trim_end().to_string());
    let edit_reason = changes.edit_reason.filter(|r| !r.trim().is_empty());

    let post_changed = raw.as_ref().is_some_and(|r| *r != post.raw)
        || edit_reason
            .as_ref()
            .is_some_and(|r| Some(r) != post.edit_reason.as_ref());
    let topic_changed = !changes.topic.is_empty();
    if topic_changed && post.post_number != 1 {
        return Err(Unsupported("topic fields revised on a reply").into());
    }
    if topic_changed && !post_changed {
        // should_create_new_version?: topic-only changes always make one.
        drop(conn);
        let opts = revise_topic::Opts {
            bypass_bump: changes.bypass_bump,
            keep_edit_reason: edit_reason.is_some(),
            ..Default::default()
        };
        return Ok(
            match revise_topic::revise(pool, ctx, editor, post.topic_id, &changes.topic, opts)
                .await?
            {
                revise_topic::Outcome::Revised => Outcome::Revised,
                revise_topic::Outcome::Invalid(errors) => Outcome::Invalid(errors),
                revise_topic::Outcome::RateLimited(refused) => Outcome::RateLimited(refused),
            },
        );
    }
    if post.hidden {
        return Err(Unsupported("editing hidden posts").into());
    }
    if post.post_type != super::post_types::REGULAR
        && post.post_type != super::post_types::SMALL_ACTION
    {
        return Err(Unsupported("editing posts other than regular and small action ones").into());
    }
    let category_definition: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE topic_id = $1)")
            .bind(post.topic_id)
            .fetch_one(&mut *conn)
            .await?;
    if category_definition && post.post_number == 1 {
        return Err(Unsupported("editing category descriptions").into());
    }
    if !post_changed {
        // Nothing to revise: only the draft sequence moves on.
        if let Some(editor_id) = post.last_editor_id {
            next_draft_sequence(&mut conn, editor_id, &format!("topic_{}", post.topic_id)).await?;
        }
        return Ok(Outcome::Revised);
    }

    let (revised_at,): (NaiveDateTime,) = sqlx::query_as("SELECT clock_timestamp()::timestamp")
        .fetch_one(&mut *conn)
        .await?;
    let last_version_at = post.last_version_at;
    let grace = s.get("editing_grace_period")?.to_i();
    if slow_mode_seconds > 0 && !editor.is_staff() {
        return Err(Unsupported("editing in slow mode").into());
    }
    // expected_raw is checked first in Rails' transaction, before
    // revise_post validates: here before the validations too, and again
    // under the row lock below.
    if changes
        .expected_raw
        .as_ref()
        .is_some_and(|e| *e != post.raw)
    {
        return Ok(Outcome::EditConflict);
    }
    // The original raw and cooked kept for the grace period, keyed by the
    // version's time (original_raw_key, original_cooked_key).
    let version_ms = last_version_at
        .and_utc()
        .timestamp_micros()
        .div_euclid(1000);
    let raw_key = format!("original_raw_{version_ms}{}", post.id);
    let cooked_key = format!("original_cooked_{version_ms}{}", post.id);
    let cached_raw = crate::owned_schema::cached_get(&mut conn, &raw_key).await?;
    let cached_cooked = crate::owned_schema::cached_get(&mut conn, &cooked_key).await?;
    let original_raw = cached_raw.clone().unwrap_or_else(|| post.raw.clone());
    // grace_period_edit?
    let elapsed = (revised_at - last_version_at).num_milliseconds() as f64 / 1000.0;
    let mut grace_period_edit = elapsed <= grace as f64;
    if grace_period_edit && let Some(new_raw) = &raw {
        let max_diff = if editor.is_staff() || editor_user.trust_level > 1 {
            s.get("editing_grace_period_max_diff_high_trust")?.to_i()
        } else {
            s.get("editing_grace_period_max_diff")?.to_i()
        };
        let size_change =
            (original_raw.chars().count() as i64 - new_raw.chars().count() as i64).abs();
        let diff = diff_size(&original_raw, new_raw)
            .map(|d| d as i64)
            .unwrap_or(i64::MAX);
        if size_change > max_diff || diff > max_diff {
            grace_period_edit = false;
        }
    }
    let flagged: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM post_actions WHERE post_id = $1 AND deleted_at IS NULL \
           AND post_action_type_id NOT IN (1, 2))",
    )
    .bind(post.id)
    .fetch_one(&mut *conn)
    .await?;
    let edited_by_another = post.last_editor_id != Some(editor_user.id);
    let edit_reason_specified =
        edit_reason.is_some() && edit_reason.as_ref() != post.edit_reason.as_ref();
    // should_create_new_version?
    let new_version = !changes.skip_revision
        && (edited_by_another
            || flagged
            || !grace_period_edit
            || changes.force_new_version
            || edit_reason_specified);
    // use_cached_original_for_created_revision?: the last editor making a
    // new version with no revision before the one it replaces.
    let use_cached_original = new_version
        && !edited_by_another
        && !sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM post_revisions WHERE post_id = $1 AND number = $2)",
        )
        .bind(post.id)
        .bind(post.version)
        .fetch_one(&mut *conn)
        .await?;

    // update_post
    let new_raw = raw.clone().unwrap_or_else(|| post.raw.clone());
    let new_edit_reason = if new_version {
        edit_reason.clone()
    } else {
        post.edit_reason.clone()
    };
    let self_edit = post.user_id == Some(editor_user.id);
    let words = word_count(&new_raw);
    // extract_quoted_post_numbers: the quote_count saved with the post.
    let quoted = super::quotes::quoted_post_numbers(&new_raw, post.topic_id, post.post_number);
    // before_save: cooked for a changed raw, cooked as the author's post
    // with the editor as last editor.
    let raw_changed = new_raw != post.raw;
    let cooked = if raw_changed {
        let omit = match post.user_id {
            Some(uid) => omit_nofollow(&mut conn, ctx, uid).await?,
            None => false,
        };
        drop(conn);
        let cooked = pretty_text::cook(
            ctx.host,
            &new_raw,
            &MarkdownOptions {
                topic_id: Some(i64::from(post.topic_id)),
                post_id: Some(i64::from(post.id)),
                user_id: Some(i64::from(editor_user.id)),
                force_quote_link: false,
                omit_nofollow: omit,
                chat: false,
            },
        )
        .await?;
        conn = pool.acquire().await?;
        cooked
    } else {
        post.cooked.clone()
    };
    let base_path = ctx.config.globals.relative_url_root().to_string();
    let analysis = validate::analyze_stripped(
        &cooked,
        &base_path,
        crate::plugins::chat::enabled(ctx.settings)?,
    )?;
    crate::onebox::refuse_unported(
        &analysis.onebox_links,
        &crate::url::Urls {
            config: ctx.config,
            settings: ctx.settings,
        },
        &base_path,
    )?;
    // discourse-poll's validate_polls on a changed raw (none without
    // validations): PollsValidator, PostValidator for the editor, then
    // PollsUpdater's plan, written with the revision.
    let mut poll_errors = Vec::new();
    let mut poll_plan = None;
    if raw_changed && !changes.skip_validations && crate::plugins::poll::enabled(ctx.settings)? {
        let extracted = if new_raw.contains("[/poll]") {
            crate::plugins::poll::extract(&cooked)
        } else {
            Vec::new()
        };
        match crate::plugins::poll::validate_polls(ctx.settings, ctx.i18n, extracted)? {
            Err(errors) => poll_errors = errors,
            Ok(polls) => {
                let rights = if polls.is_empty() {
                    None
                } else {
                    crate::plugins::poll::validate_post(
                        &mut conn,
                        ctx.settings,
                        ctx.i18n,
                        editor,
                        Some(post.topic_id),
                    )
                    .await?
                };
                if let Some(error) = rights {
                    poll_errors.push(error);
                } else {
                    let had_polls: bool = sqlx::query_scalar(
                        "SELECT EXISTS (SELECT 1 FROM polls WHERE post_id = $1)",
                    )
                    .bind(i64::from(post.id))
                    .fetch_one(&mut *conn)
                    .await?;
                    if !polls.is_empty() || had_polls {
                        match crate::plugins::poll::updater::plan(&mut conn, ctx, post.id, &polls)
                            .await?
                        {
                            Ok(plan) => poll_plan = Some(plan),
                            Err(error) => poll_errors.push(error),
                        }
                    }
                }
            }
        }
    }
    let private_message = archetype == "private_message";
    let mut errors = if changes.skip_validations {
        Vec::new()
    } else {
        validate::validate_post(
            &mut conn,
            ctx,
            editor,
            &PostInput {
                raw: &new_raw,
                topic_id: Some(post.topic_id),
                first_post: post.post_number == 1,
                private_message,
                new_record: false,
                post_id: Some(post.id),
                user_id: post.user_id.unwrap_or(editor_user.id),
            },
            &analysis,
        )
        .await?
    };
    // discourse-policy's validate_policy on a changed raw, before poll's.
    if raw_changed
        && !changes.skip_validations
        && let Some(error) = crate::plugins::policy::lifecycle::validate_post(
            &mut conn,
            ctx.host,
            ctx.settings,
            ctx.i18n,
            &crate::plugins::policy::lifecycle::Saving {
                old_cooked: Some(&post.cooked),
                new_raw: &new_raw,
                wiki: post.wiki,
                acting_user_id: Some(editor_user.id),
                user_id: post.user_id,
            },
        )
        .await?
    {
        errors.push(error);
    }
    errors.extend(poll_errors);
    // validates :edit_reason, length: { maximum: MAX_EDIT_REASON_LENGTH },
    // after PostValidator; with validations skipped, before_save's
    // ensure_edit_reason_length aborts the save the same way.
    if new_edit_reason
        .as_ref()
        .is_some_and(|r| r.chars().count() > MAX_EDIT_REASON_LENGTH)
    {
        let message = ctx.t_count("errors.messages.too_long", MAX_EDIT_REASON_LENGTH as i64);
        errors.push(format!("Edit reason {message}"));
    }
    // The topic's side, read before the transaction as revise_topic does.
    let prepared = if topic_changed {
        Some(
            revise_topic::prepare(&mut conn, ctx, editor, post.topic_id, &changes.topic)
                .await?
                .ok_or(Unsupported("a topic edit slow mode keeps out"))?,
        )
    } else {
        None
    };
    if !errors.is_empty() {
        // The failed save is rolled back after advance_draft_sequence,
        // whose upsert has taken a draft_sequences id all the same.
        if editor_user.id > 0 {
            sqlx::query("SELECT nextval('draft_sequences_id_seq')")
                .execute(&mut *conn)
                .await?;
        }
        return Ok(Outcome::Invalid(errors));
    }
    drop(conn);

    let mut tx = pool.begin().await?;
    if let Some(expected) = &changes.expected_raw {
        let locked: String = sqlx::query_scalar("SELECT raw FROM posts WHERE id = $1 FOR UPDATE")
            .bind(post.id)
            .fetch_one(&mut *tx)
            .await?;
        if locked != *expected {
            tx.rollback().await?;
            return Ok(Outcome::EditConflict);
        }
    }
    // revise_post without a new version: the original kept for the grace
    // period, unless it already is.
    if !new_version && cached_raw.is_none() {
        let ttl = grace + 1;
        crate::owned_schema::cached_setex(&mut tx, &raw_key, ttl, &post.raw).await?;
        crate::owned_schema::cached_setex(&mut tx, &cooked_key, ttl, &post.cooked).await?;
    }
    // The post's validation, where PollsUpdater writes.
    if let Some(plan) = poll_plan {
        crate::plugins::poll::updater::apply(&mut tx, ctx, post.id, plan).await?;
    }
    let version = post.version + i32::from(new_version);
    let public_version = post.public_version + i32::from(new_version);
    let (updated_at,): (NaiveDateTime,) = sqlx::query_as(
        "UPDATE posts SET raw = $2, cooked = $3, edit_reason = $4, last_editor_id = $5, word_count = $6, \
                self_edits = self_edits + $7, version = $8, public_version = $9, last_version_at = $10, \
                updated_at = clock_timestamp(), \
                baked_at = CASE WHEN $11 THEN clock_timestamp() ELSE baked_at END, \
                baked_version = CASE WHEN $11 THEN $12 ELSE baked_version END, quote_count = $13 \
         WHERE id = $1 RETURNING updated_at",
    )
    .bind(post.id)
    .bind(&new_raw)
    .bind(&cooked)
    .bind(&new_edit_reason)
    .bind(editor_user.id)
    .bind(words)
    .bind(i32::from(self_edit))
    .bind(version)
    .bind(public_version)
    .bind(if new_version { revised_at } else { last_version_at })
    .bind(raw_changed)
    .bind(BAKED_VERSION)
    .bind(quoted.len() as i32)
    .fetch_one(&mut *tx)
    .await?;
    let _ = updated_at;
    // @post.link_post_uploads
    let hostname = Urls {
        config: ctx.config,
        settings: s,
    }
    .current_hostname()?;
    let store = crate::file_store::FileStore::for_site(ctx.config, s)?;
    crate::upload_references::link_post_uploads(&mut tx, s, &hostname, &store, post.id, &cooked)
        .await?;
    // save_reply_relationships: the posts quoted and the one replied to
    // (deleted posts aren't edited here).
    super::quotes::save_reply_relationships(
        &mut tx,
        super::quotes::ReplyingPost {
            id: post.id,
            topic_id: post.topic_id,
            post_type: post.post_type,
            reply_to_post_number: post.reply_to_post_number,
            deleted: false,
        },
        &quoted,
    )
    .await?;
    if editor_user.id != -1 {
        sqlx::query(
            "UPDATE user_stats SET post_edits_count = COALESCE(post_edits_count, 0) + 1 WHERE user_id = $1",
        )
        .bind(editor_user.id)
        .execute(&mut *tx)
        .await?;
    }
    // update_topic, when topic fields came along.
    let mut deferred = Deferred::default();
    let mut topic_diff = Vec::new();
    let mut new_archetype = archetype.clone();
    if let Some(prepared) = &prepared {
        match revise_topic::update_topic(
            &mut tx,
            ctx,
            editor,
            prepared,
            &changes.topic,
            false,
            &mut deferred,
        )
        .await?
        {
            Ok((diff, archetype)) => {
                topic_diff = diff;
                new_archetype = archetype;
            }
            Err(errors) => {
                tx.rollback().await?;
                // advance_draft_sequence ran in the rolled back
                // transaction: its upsert has taken a draft_sequences id
                // all the same.
                if editor_user.id > 0 {
                    sqlx::query("SELECT nextval('draft_sequences_id_seq')")
                        .execute(pool)
                        .await?;
                }
                return Ok(Outcome::Invalid(errors));
            }
        }
    }
    let category_changed = topic_diff.iter().any(|(k, _)| *k == "category_id");
    // create_revision: what changed, in POST_TRACKED_FIELDS order, then
    // the topic's diff.
    let mut fields: Vec<(&str, [Value; 2])> = Vec::new();
    if raw_changed {
        fields.push(("raw", [json!(post.raw), json!(new_raw)]));
        if cooked != post.cooked {
            fields.push(("cooked", [json!(post.cooked), json!(cooked)]));
        }
    }
    if new_edit_reason != post.edit_reason {
        fields.push((
            "edit_reason",
            [json!(post.edit_reason), json!(new_edit_reason)],
        ));
    }
    fields.extend(topic_diff.iter().cloned());
    if use_cached_original {
        for (field, change) in fields.iter_mut() {
            match *field {
                "raw" => {
                    if let Some(raw) = &cached_raw {
                        change[0] = json!(raw);
                    }
                }
                "cooked" => {
                    if let Some(cooked) = &cached_cooked {
                        change[0] = json!(cooked);
                    }
                }
                _ => {}
            }
        }
    }
    let yaml = modifications::dump_column(&fields)?;
    // create_or_update_revision: none with skip_revision; within the
    // grace period the current revision takes the change.
    let mut revision_dropped = false;
    if !new_version && !changes.skip_revision {
        revision_dropped = update_revision(&mut tx, &post, editor_user.id, &fields).await?;
    }
    let revision_id: i32 = if !new_version {
        0
    } else {
        sqlx::query_scalar(
        "INSERT INTO post_revisions (user_id, post_id, number, modifications, hidden, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, FALSE, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(editor_user.id)
    .bind(post.id)
    .bind(version)
    .bind(&yaml)
    .fetch_one(&mut *tx)
    .await?
    };
    // PostActionNotifier.after_create_post_revision, told once the edit
    // commits.
    if new_version
        && let Some(user_ids) = revise_topic::revision_recipients(
            &mut tx,
            ctx,
            &revise_topic::NotifyRevision {
                topic_id: post.topic_id,
                author_id: post.user_id,
                editor_id: editor_user.id,
                first_post: post.post_number == 1,
                wiki: post.wiki,
                private_message: new_archetype == "private_message",
                silent: false,
                category_changed,
                tags_changed: topic_diff.iter().any(|(k, _)| *k == "tags"),
            },
        )
        .await?
    {
        deferred.push(
            "notify_post_revision",
            json!({ "user_ids": user_ids, "post_revision_id": revision_id }),
        );
    }
    // perform_edit: once the new version is saved (create_or_update_revision
    // and its notification are before it).
    if ctx.config.rate_limits && new_version {
        let limiter = super::limits::edit_limiter(
            s,
            crate::rate_limiter::LimitUser {
                id: editor_user.id,
                staff: editor.is_staff(),
            },
            editor_user.trust_level,
        )?;
        if let Err(refused) = limiter.performed(pool, true).await? {
            tx.rollback().await?;
            return Ok(Outcome::RateLimited(refused));
        }
    }
    // revise_topic: a first post's excerpt.
    if post.post_number == 1 {
        let excerpt = crate::excerpt::excerpt(
            &cooked,
            s.get("topic_excerpt_maxlength")?.to_i().max(0) as usize,
            &crate::excerpt::Options {
                strip_links: true,
                image_mode: crate::excerpt::ImageMode::Strip,
                ..Default::default()
            },
            crate::excerpt::Plugins::from_settings(s)?,
        );
        sqlx::query("UPDATE topics SET excerpt = $2 WHERE id = $1")
            .bind(post.topic_id)
            .bind(excerpt)
            .execute(&mut *tx)
            .await?;
    }
    // advance_draft_sequence for the (new) last editor.
    next_draft_sequence(&mut tx, editor_user.id, &format!("topic_{}", post.topic_id)).await?;
    // What Rails sends from after_commit goes out before what revise!
    // does after its transaction.
    deferred.enqueue(&mut tx, ctx).await?;
    // post_process_post and alert_users, committed with the edit.
    crate::jobs::enqueue(
        &mut tx,
        "process_post",
        json!({"bypass_bump": false, "cooking_options": null, "new_post": false, "post_id": post.id, "skip_pull_hotlinked_images": false, "invalidate_oneboxes": true}),
    )
    .await?;
    // DiscourseEvent :post_edited: post_edited's web hooks (`post.topic&.
    // trashed?` never holds: a trashed topic is no topic to the post), or
    // topic_edited's for a first post with topic fields.
    let payloads = crate::web_hooks::Payloads {
        settings: s,
        i18n: ctx.i18n,
        config: ctx.config,
    };
    if topic_changed {
        crate::web_hooks::enqueue_topic_hooks(
            &mut tx,
            &payloads,
            "topic_edited",
            post.topic_id,
            None,
        )
        .await?;
    } else {
        crate::web_hooks::enqueue_post_hooks(
            &mut tx,
            &payloads,
            "post_edited",
            post.id,
            crate::topic_view::Held::default(),
        )
        .await?;
    }
    if category_changed {
        crate::plugins::topic_category_revised(&mut tx, s, post.topic_id).await?;
    }
    // ... and to discourse-assign: auto assigning by mention.
    crate::plugins::assign::lifecycle::post_saved(&mut tx, ctx, post.id).await?;
    // ... and to discourse-narrative-bot: the author's edit.
    crate::plugins::narrative_bot::post_edited(&mut tx, s, post.id).await?;
    if editor_user.id != -1 {
        crate::jobs::enqueue(&mut tx, "post_alert", json!({"post_id": post.id})).await?;
    }
    // publish_changes: reload_topic when the topic changed.
    let mut publish = serde_json::Map::new();
    if !topic_diff.is_empty() {
        publish.insert("reload_topic".into(), json!(true));
    }
    if let Some(token) = &changes.preserve_cooked_token {
        publish.insert("preserve_cooked_token".into(), json!(token));
    }
    crate::bus::publish_post_change(ctx, &mut tx, post.id, "revised", publish, false).await?;
    // grant_badge
    crate::badge_granter::queue(
        &mut tx,
        s,
        crate::badge_granter::Trigger::PostRevision,
        &[post.id],
    )
    .await?;
    tx.commit().await?;

    // bump_topic, after the transaction as Rails does. should_bump?: a new
    // version that changed the post (every edit that gets this far) of a
    // wiki first post, unless bypass_bump.
    if new_version && post.post_number == 1 && post.wiki && !changes.bypass_bump {
        let mut tx = pool.begin().await?;
        sqlx::query("UPDATE topics SET bumped_at = clock_timestamp() WHERE id = $1")
            .bind(post.topic_id)
            .execute(&mut *tx)
            .await?;
        use crate::topic_tracking_state as tracking;
        tracking::publish_muted(ctx.bus, &mut tx, post.topic_id).await?;
        tracking::publish_unmuted(ctx.bus, &mut tx, post.topic_id).await?;
        tracking::publish_latest(ctx.bus, s, &mut tx, post.topic_id).await?;
        tx.commit().await?;
    }

    let mut conn = pool.acquire().await?;
    if s.get("staff_edit_locks_post")?.truthy() && !post.wiki && editor.is_staff() {
        return Err(Unsupported("staff_edit_locks_post").into());
    }
    // StaffActionLogger#log_post_edit for staff editing someone else's raw.
    if editor.is_staff() && post.user_id != Some(editor_user.id) {
        let truncate = |s: &str| {
            if s.chars().count() > 50_000 {
                format!("{}...", s.chars().take(50_001).collect::<String>())
            } else {
                s.to_string()
            }
        };
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, post_id, details, admin_only, created_at, updated_at) \
             VALUES (53, $1, $2, $3, FALSE, clock_timestamp(), clock_timestamp())",
        )
        .bind(editor_user.id)
        .bind(post.id)
        .bind(format!("{}\n\n---\n\n{}", truncate(&post.raw), truncate(&new_raw)))
        .execute(&mut *conn)
        .await?;
    }
    let urls = Urls {
        config: ctx.config,
        settings: s,
    };
    let hostname = urls.current_hostname()?;
    // QuotedPost.extract_from, then TopicLink.extract_from.
    super::rebake::extract_quoted_posts(
        &mut conn,
        post.id,
        post.topic_id,
        post.post_number,
        post.reply_to_post_number,
        post.reply_quoted,
        &cooked,
    )
    .await?;
    if let Some(user_id) = post.user_id {
        links::extract_from(
            &mut conn,
            &links::Site {
                hostname: &hostname,
                base_path: &base_path,
                base_url_no_prefix: &urls.base_url_no_prefix()?,
                settings: s,
            },
            &LinkPost {
                id: post.id,
                user_id,
                topic_id: post.topic_id,
                cooked: &cooked,
            },
        )
        .await?;
    }
    // Topic.reset_highest
    let highest: i32 = sqlx::query_scalar(
        "UPDATE topics SET \
           highest_staff_post_number = (SELECT COALESCE(MAX(post_number), 0) FROM posts \
             WHERE topic_id = $1 AND deleted_at IS NULL AND post_type <> 3), \
           highest_post_number = (SELECT COALESCE(MAX(post_number), 0) FROM posts \
             WHERE topic_id = $1 AND deleted_at IS NULL AND post_type NOT IN (3, 4)), \
           posts_count = (SELECT count(*) FROM posts WHERE deleted_at IS NULL AND topic_id = $1 \
             AND post_type NOT IN (3, 4)), \
           word_count = (SELECT SUM(COALESCE(posts.word_count, 0)) FROM posts WHERE topic_id = $1 \
             AND deleted_at IS NULL AND post_type NOT IN (3, 4)), \
           last_posted_at = (SELECT MAX(created_at) FROM posts WHERE topic_id = $1 \
             AND deleted_at IS NULL AND post_type NOT IN (3, 4)), \
           last_post_user_id = COALESCE((SELECT user_id FROM posts WHERE topic_id = $1 \
             AND deleted_at IS NULL AND post_type NOT IN (3, 4) ORDER BY created_at DESC LIMIT 1), last_post_user_id) \
         WHERE id = $1 RETURNING highest_post_number",
    )
    .bind(post.topic_id)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE topic_users SET last_read_post_number = $2 WHERE topic_id = $1 AND last_read_post_number > $2",
    )
    .bind(post.topic_id)
    .bind(highest)
    .execute(&mut *conn)
    .await?;
    // The post's after_commit search index, when its last save changed the
    // cooked and it has a raw (`obj.raw.present?`: a staff edit may blank a
    // small action's).
    if cooked != post.cooked && !revision_dropped && !new_raw.trim().is_empty() {
        // The topic as the edit left it.
        let (topic_title, category_name, tags): (String, Option<String>, Option<String>) =
            sqlx::query_as(&format!(
                "SELECT t.title, (SELECT name FROM categories WHERE id = t.category_id), {} \
                 FROM topics t WHERE t.id = $1",
                super::search_index::topic_tag_names_sql("$1")
            ))
            .bind(post.topic_id)
            .fetch_one(&mut *conn)
            .await?;
        search_index::index_post(
            &mut conn,
            s,
            &BaseUrls {
                base_path: &base_path,
                base_url_no_prefix: urls.base_url_no_prefix()?,
                store: crate::file_store::FileStore::for_site(urls.config, s)?,
            },
            &PostIndex {
                post_id: post.id,
                topic_id: post.topic_id,
                is_first_post: post.post_number == 1,
                topic_title: &topic_title,
                category_name: category_name.as_deref(),
                tag_names: tags.as_deref(),
                cooked: &cooked,
                private_message: new_archetype == "private_message",
            },
        )
        .await?;
    }
    let _ = (post.self_edits, post.locale, topic_title, category_id);
    Ok(Outcome::Revised)
}
