//! `PostRevisor#revise!` for the fields posts#update sends: the raw and
//! the edit reason. Topic fields (title, category, tags), owner and wiki
//! changes are not ported.
//!
//! Within the editing grace period Rails keeps the post's original raw in
//! Redis so the version made later can diff against it. Edits that could
//! need that cache are refused; the rest match Rails without it.

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

/// What posts#update hands the revisor.
pub struct Changes {
    pub raw: Option<String>,
    pub edit_reason: Option<String>,
    /// `force_new_version:` a new version even within the grace period.
    pub force_new_version: bool,
    /// `skip_validations:` the post is saved without PostValidator.
    pub skip_validations: bool,
}

#[derive(Debug)]
pub enum Outcome {
    Revised,
    /// The post's validation errors: a 422 `render_json_error(post)`.
    Invalid(Vec<String>),
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
                public_version, last_version_at, hidden, wiki, post_type, self_edits, reply_to_post_number, locale \
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
    if archetype == "private_message" {
        return Err(Unsupported("editing messages").into());
    }
    if post.hidden {
        return Err(Unsupported("editing hidden posts").into());
    }
    if post.post_type != super::post_types::REGULAR {
        return Err(Unsupported("editing posts other than regular ones").into());
    }
    let watched: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM watched_words)")
        .fetch_one(&mut *conn)
        .await?;
    if watched {
        return Err(Unsupported("watched words").into());
    }
    let category_definition: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE topic_id = $1)")
            .bind(post.topic_id)
            .fetch_one(&mut *conn)
            .await?;
    if category_definition && post.post_number == 1 {
        return Err(Unsupported("editing category descriptions").into());
    }

    // revise!: the fields, cleaned.
    let raw = changes
        .raw
        .map(|r| normalize_whitespaces(&r).trim_end().to_string());
    let edit_reason = changes.edit_reason.filter(|r| !r.trim().is_empty());

    let post_changed = raw.as_ref().is_some_and(|r| *r != post.raw)
        || edit_reason
            .as_ref()
            .is_some_and(|r| Some(r) != post.edit_reason.as_ref());
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
    // grace_period_edit?
    let elapsed = (revised_at - last_version_at).num_milliseconds() as f64 / 1000.0;
    let mut grace_period_edit = elapsed <= grace as f64;
    if grace_period_edit && let Some(new_raw) = &raw {
        let max_diff = if editor.is_staff() || editor_user.trust_level > 1 {
            s.get("editing_grace_period_max_diff_high_trust")?.to_i()
        } else {
            s.get("editing_grace_period_max_diff")?.to_i()
        };
        let size_change = (post.raw.chars().count() as i64 - new_raw.chars().count() as i64).abs();
        let diff = diff_size(&post.raw, new_raw)
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
    let new_version = edited_by_another
        || flagged
        || !grace_period_edit
        || changes.force_new_version
        || edit_reason_specified;
    if !new_version {
        return Err(
            Unsupported("edits within the editing grace period (original kept in Redis)").into(),
        );
    }
    let previous_revision_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM post_revisions WHERE post_id = $1 AND number = $2)",
    )
    .bind(post.id)
    .bind(post.version - 1)
    .fetch_one(&mut *conn)
    .await?;
    if !edited_by_another && !previous_revision_exists && elapsed <= (grace + 1) as f64 {
        // use_cached_original_for_created_revision? with the cache
        // possibly still alive.
        return Err(Unsupported(
            "new versions within the editing grace period (original kept in Redis)",
        )
        .into());
    }

    // update_post
    let new_raw = raw.clone().unwrap_or_else(|| post.raw.clone());
    let new_edit_reason = edit_reason.clone();
    let self_edit = post.user_id == Some(editor_user.id);
    let words = word_count(&new_raw);
    if new_raw.contains("[quote=") {
        return Err(Unsupported("posts with quotes (QuotedPost)").into());
    }
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
            },
        )
        .await?;
        conn = pool.acquire().await?;
        cooked
    } else {
        post.cooked.clone()
    };
    let base_path = ctx.config.globals.relative_url_root().to_string();
    let analysis = validate::analyze(&cooked, &base_path)?;
    if analysis.has_upload_media {
        return Err(Unsupported("posts with uploaded video or audio").into());
    }
    let private_message = false;
    let errors = if changes.skip_validations {
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
    if !errors.is_empty() {
        return Ok(Outcome::Invalid(errors));
    }
    drop(conn);

    let mut tx = pool.begin().await?;
    let version = post.version + 1;
    let public_version = post.public_version + 1;
    let (updated_at,): (NaiveDateTime,) = sqlx::query_as(
        "UPDATE posts SET raw = $2, cooked = $3, edit_reason = $4, last_editor_id = $5, word_count = $6, \
                self_edits = self_edits + $7, version = $8, public_version = $9, last_version_at = $10, \
                updated_at = clock_timestamp(), \
                baked_at = CASE WHEN $11 THEN clock_timestamp() ELSE baked_at END, \
                baked_version = CASE WHEN $11 THEN $12 ELSE baked_version END \
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
    .bind(revised_at)
    .bind(raw_changed)
    .bind(BAKED_VERSION)
    .fetch_one(&mut *tx)
    .await?;
    let _ = updated_at;
    // @post.link_post_uploads
    let hostname = Urls {
        config: ctx.config,
        settings: s,
    }
    .current_hostname()?;
    crate::upload_references::link_post_uploads(&mut tx, s, &hostname, post.id, &cooked).await?;
    // save_reply_relationships: the reply_to_post_number link, kept.
    if let Some(n) = post.reply_to_post_number {
        let parent: Option<i32> =
            sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1 AND post_number = $2")
                .bind(post.topic_id)
                .bind(n)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(parent) = parent {
            let inserted = sqlx::query(
                "INSERT INTO post_replies (post_id, reply_post_id, created_at, updated_at) \
                 VALUES ($1, $2, clock_timestamp(), clock_timestamp()) ON CONFLICT DO NOTHING",
            )
            .bind(parent)
            .bind(post.id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if inserted > 0 {
                sqlx::query("UPDATE posts SET reply_count = reply_count + 1 WHERE id = $1")
                    .bind(parent)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }
    if editor_user.id != -1 {
        sqlx::query(
            "UPDATE user_stats SET post_edits_count = COALESCE(post_edits_count, 0) + 1 WHERE user_id = $1",
        )
        .bind(editor_user.id)
        .execute(&mut *tx)
        .await?;
    }
    // create_revision: what changed, in POST_TRACKED_FIELDS order.
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
    let yaml = modifications::dump(&fields)?;
    let revision_id: i32 = sqlx::query_scalar(
        "INSERT INTO post_revisions (user_id, post_id, number, modifications, hidden, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, FALSE, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(editor_user.id)
    .bind(post.id)
    .bind(version)
    .bind(&yaml)
    .fetch_one(&mut *tx)
    .await?;
    // PostActionNotifier.after_create_post_revision: the author when
    // someone else edits, and the topic's watchers when a wiki or freely
    // editable first post changes; told once the edit commits.
    let notify_disabled =
        s.get("disable_system_edit_notifications")?.truthy() && editor_user.id == -1;
    if !notify_disabled {
        let mut user_ids: Vec<i32> = Vec::new();
        if let Some(author) = post.user_id
            && author != editor_user.id
        {
            user_ids.push(author);
        }
        if post.post_number == 1 {
            let unlimited: bool = sqlx::query_scalar(
                "SELECT COALESCE(c.allow_unlimited_owner_edits_on_first_post, FALSE) \
                 FROM topics t LEFT JOIN categories c ON c.id = t.category_id WHERE t.id = $1",
            )
            .bind(post.topic_id)
            .fetch_one(&mut *tx)
            .await?;
            if post.wiki || unlimited {
                let watchers: Vec<i32> = sqlx::query_scalar(
                    "SELECT user_id FROM topic_users WHERE topic_id = $1 AND notification_level = 3 \
                       AND user_id <> $2 ORDER BY id",
                )
                .bind(post.topic_id)
                .bind(editor_user.id)
                .fetch_all(&mut *tx)
                .await?;
                user_ids.extend(watchers);
            }
        }
        if !user_ids.is_empty() {
            crate::jobs::enqueue(
                &mut tx,
                "notify_post_revision",
                json!({ "user_ids": user_ids, "post_revision_id": revision_id }),
            )
            .await?;
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
        );
        sqlx::query("UPDATE topics SET excerpt = $2 WHERE id = $1")
            .bind(post.topic_id)
            .bind(excerpt)
            .execute(&mut *tx)
            .await?;
    }
    // advance_draft_sequence for the (new) last editor.
    next_draft_sequence(&mut tx, editor_user.id, &format!("topic_{}", post.topic_id)).await?;
    // post_process_post and alert_users, committed with the edit.
    crate::jobs::enqueue(
        &mut tx,
        "process_post",
        json!({"bypass_bump": false, "cooking_options": null, "new_post": false, "post_id": post.id, "skip_pull_hotlinked_images": false, "invalidate_oneboxes": true}),
    )
    .await?;
    if editor_user.id != -1 {
        crate::jobs::enqueue(&mut tx, "post_alert", json!({"post_id": post.id})).await?;
    }
    // publish_changes; reload_topic only comes with topic edits, refused
    // above.
    crate::bus::publish_post_change(ctx, &mut tx, post.id, "revised", Default::default(), false)
        .await?;
    tx.commit().await?;

    // bump_topic, after the transaction as Rails does. should_bump?: a new
    // version that changed the post (every edit that gets this far) of a
    // wiki first post; bypass_bump is refused by the route.
    if post.post_number == 1 && post.wiki {
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
    // The post's after_commit search index, when its cooked changed.
    if cooked != post.cooked {
        let (category_name, tags): (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT (SELECT name FROM categories WHERE id = $2), \
               (SELECT string_agg(name, ' ') FROM (SELECT t.name FROM topic_tags tt JOIN tags t ON t.id = tt.tag_id \
                  WHERE tt.topic_id = $1 \
                UNION ALL SELECT s.name FROM tags s WHERE s.target_tag_id IN \
                  (SELECT tag_id FROM topic_tags WHERE topic_id = $1)) n)",
        )
        .bind(post.topic_id)
        .bind(category_id)
        .fetch_one(&mut *conn)
        .await?;
        search_index::index_post(
            &mut conn,
            s,
            &BaseUrls {
                base_path: &base_path,
                base_url_no_prefix: urls.base_url_no_prefix()?,
            },
            &PostIndex {
                post_id: post.id,
                topic_id: post.topic_id,
                is_first_post: post.post_number == 1,
                topic_title: &topic_title,
                category_name: category_name.as_deref(),
                tag_names: tags.as_deref(),
                cooked: &cooked,
                private_message,
            },
        )
        .await?;
    }
    let _ = (post.self_edits, post.locale);
    Ok(Outcome::Revised)
}
