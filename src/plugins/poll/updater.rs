//! `DiscoursePoll::PollsUpdater`: an edited post's polls against the ones
//! it has. Planned before the revision writes anything, since a poll
//! with votes past its edit window fails the post's validation (and
//! PostRevisor rolls everything back); applied in the revision's
//! transaction.

use chrono::NaiveDateTime;
use serde_json::json;
use sqlx::PgConnection;

use super::serializer::PostsData;
use super::{DEFAULT_POLL_NAME, Extracted, NewPoll};
use crate::AppError;
use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::url::Urls;

#[derive(sqlx::FromRow)]
struct OldPoll {
    id: i64,
    name: String,
    #[sqlx(rename = "type")]
    kind: i32,
    results: i32,
    visibility: i32,
    min: Option<i32>,
    max: Option<i32>,
    step: Option<i32>,
    close_at: Option<NaiveDateTime>,
    title: Option<String>,
    groups: Option<String>,
    dynamic: bool,
    votes: i64,
    /// `created_at < edit_window.minutes.ago`
    past_window: bool,
}

enum Options {
    /// A dynamic poll's: the options gone, and the new ones.
    Dynamic {
        gone: Vec<String>,
        added: Vec<(String, String)>,
    },
    /// Every option replaced, keeping the anonymous votes by digest.
    Replace(Vec<(String, String)>),
}

struct Update {
    poll_id: i64,
    /// POLL_ATTRIBUTES' new values, when any differs.
    attributes: Option<NewPoll>,
    options: Options,
}

/// What the edit does to the post's polls.
pub struct Plan {
    deleted: Vec<i64>,
    created: Vec<NewPoll>,
    updated: Vec<Update>,
}

impl Plan {
    fn changed(&self) -> bool {
        !self.created.is_empty() || !self.updated.is_empty()
    }
}

/// `PollsUpdater.update` up to its writes: the plan, or the edit window's
/// error.
pub async fn plan(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    post_id: i32,
    polls: &[Extracted],
) -> Result<Result<Plan, String>, AppError> {
    let window = ctx.settings.get("poll_edit_window_mins")?.to_i();
    let old: Vec<OldPoll> = sqlx::query_as(
        "SELECT p.id, p.name, p.type, p.results, p.visibility, p.min, p.max, p.step, \
                p.close_at, p.title, p.groups, p.dynamic, \
                (SELECT COUNT(*) FROM poll_votes v WHERE v.poll_id = p.id) AS votes, \
                p.created_at < now() - make_interval(mins => $2) AS past_window \
         FROM polls p WHERE p.post_id = $1 ORDER BY p.id",
    )
    .bind(i64::from(post_id))
    .bind(window as i32)
    .fetch_all(&mut *conn)
    .await?;
    let new_names: Vec<&str> = polls.iter().map(Extracted::name).collect();
    let deleted: Vec<i64> = old
        .iter()
        .filter(|p| !new_names.contains(&p.name.as_str()))
        .map(|p| p.id)
        .collect();
    let mut created = Vec::new();
    for poll in polls
        .iter()
        .filter(|p| !old.iter().any(|o| o.name == p.name()))
    {
        created.push(NewPoll::from_extracted(poll)?);
    }
    let mut updated = Vec::new();
    for old_poll in old.iter().filter(|p| !deleted.contains(&p.id)) {
        let new_poll = polls
            .iter()
            .find(|p| p.name() == old_poll.name)
            .expect("kept polls are in the edit");
        let candidate = NewPoll::from_extracted(new_poll)?;
        // compute_dynamic_flag: a poll turns dynamic only at creation.
        let dynamic = new_poll.get("dynamic") == Some("true") && old_poll.dynamic;
        let attributes_differ = old_poll.kind != candidate.kind
            || old_poll.close_at != candidate.close_at
            || old_poll.max != candidate.max
            || old_poll.min != candidate.min
            || old_poll.results != candidate.results
            || old_poll.step != candidate.step
            || old_poll.visibility != candidate.visibility
            || old_poll.title != candidate.title
            || old_poll.groups != candidate.groups;
        let old_digests: Vec<String> = sqlx::query_scalar(
            "SELECT digest FROM poll_options WHERE poll_id = $1 ORDER BY digest",
        )
        .bind(old_poll.id)
        .fetch_all(&mut *conn)
        .await?;
        let mut new_digests: Vec<String> =
            candidate.options.iter().map(|(d, _)| d.clone()).collect();
        new_digests.sort();
        // is_different?
        if !attributes_differ && old_digests == new_digests {
            continue;
        }
        if old_poll.votes > 0 && !dynamic && window > 0 && old_poll.past_window {
            let minutes = window.to_string();
            let error = if old_poll.name == DEFAULT_POLL_NAME {
                ctx.i18n.t_with(
                    "poll.edit_window_expired.cannot_edit_default_poll_with_votes",
                    &[("minutes", &minutes)],
                )
            } else {
                ctx.i18n.t_with(
                    "poll.edit_window_expired.cannot_edit_named_poll_with_votes",
                    &[("minutes", &minutes), ("name", &old_poll.name)],
                )
            };
            return Ok(Err(error.unwrap_or_default()));
        }
        let options = if dynamic {
            Options::Dynamic {
                gone: old_digests
                    .iter()
                    .filter(|d| !candidate.options.iter().any(|(n, _)| n == *d))
                    .cloned()
                    .collect(),
                added: candidate
                    .options
                    .iter()
                    .filter(|(d, _)| !old_digests.contains(d))
                    .cloned()
                    .collect(),
            }
        } else {
            Options::Replace(candidate.options.clone())
        };
        updated.push(Update {
            poll_id: old_poll.id,
            attributes: attributes_differ.then_some(candidate),
            options,
        });
    }
    Ok(Ok(Plan {
        deleted,
        created,
        updated,
    }))
}

/// `PollOption#destroy`: its votes, then the option.
async fn destroy_options(
    conn: &mut PgConnection,
    poll_id: i64,
    digests: Option<&[String]>,
) -> Result<(), sqlx::Error> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM poll_options WHERE poll_id = $1 AND ($2::text[] IS NULL OR digest = ANY($2))",
    )
    .bind(poll_id)
    .bind(digests)
    .fetch_all(&mut *conn)
    .await?;
    for id in ids {
        sqlx::query("DELETE FROM poll_votes WHERE poll_option_id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM poll_options WHERE id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

async fn create_option(
    conn: &mut PgConnection,
    poll_id: i64,
    digest: &str,
    html: &str,
    anonymous_votes: Option<i32>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO poll_options (poll_id, digest, html, anonymous_votes, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, clock_timestamp(), clock_timestamp())",
    )
    .bind(poll_id)
    .bind(digest)
    .bind(html.trim())
    .bind(anonymous_votes)
    .execute(conn)
    .await?;
    Ok(())
}

/// `PollsUpdater.update`'s writes, then the post's `has_polls` and, when
/// anything changed, the polls on `/polls/<topic_id>` as anyone sees them.
pub async fn apply(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    post_id: i32,
    plan: Plan,
) -> Result<(), AppError> {
    for poll_id in &plan.deleted {
        destroy_options(conn, *poll_id, None).await?;
        sqlx::query("DELETE FROM polls WHERE id = $1")
            .bind(poll_id)
            .execute(&mut *conn)
            .await?;
    }
    let changed = plan.changed();
    super::insert_polls(conn, post_id, &plan.created).await?;
    for update in plan.updated {
        if let Some(a) = &update.attributes {
            sqlx::query(
                "UPDATE polls SET close_at = $2, max = $3, min = $4, results = $5, step = $6, type = $7, \
                                  visibility = $8, title = $9, groups = $10, updated_at = clock_timestamp() \
                 WHERE id = $1",
            )
            .bind(update.poll_id)
            .bind(a.close_at)
            .bind(a.max)
            .bind(a.min)
            .bind(a.results)
            .bind(a.step)
            .bind(a.kind)
            .bind(a.visibility)
            .bind(&a.title)
            .bind(&a.groups)
            .execute(&mut *conn)
            .await?;
        }
        match update.options {
            Options::Dynamic { gone, added } => {
                if !gone.is_empty() {
                    destroy_options(conn, update.poll_id, Some(&gone)).await?;
                }
                for (digest, html) in added {
                    create_option(conn, update.poll_id, &digest, &html, None).await?;
                }
            }
            Options::Replace(options) => {
                let anonymous: Vec<(String, Option<i32>)> = sqlx::query_as(
                    "SELECT digest, anonymous_votes FROM poll_options WHERE poll_id = $1",
                )
                .bind(update.poll_id)
                .fetch_all(&mut *conn)
                .await?;
                destroy_options(conn, update.poll_id, None).await?;
                for (digest, html) in options {
                    let votes = anonymous
                        .iter()
                        .find(|(d, _)| *d == digest)
                        .and_then(|(_, v)| *v);
                    create_option(conn, update.poll_id, &digest, &html, votes).await?;
                }
            }
        }
    }
    // update_post_custom_fields
    let any: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM polls WHERE post_id = $1)")
        .bind(i64::from(post_id))
        .fetch_one(&mut *conn)
        .await?;
    if any {
        // save_custom_fields: a changed value's row is replaced.
        let value: Option<Option<String>> = sqlx::query_scalar(
            "SELECT value FROM post_custom_fields WHERE post_id = $1 AND name = 'has_polls' \
             ORDER BY id LIMIT 1",
        )
        .bind(post_id)
        .fetch_optional(&mut *conn)
        .await?;
        if value.as_ref().is_none_or(|v| v.as_deref() != Some("t")) {
            sqlx::query("DELETE FROM post_custom_fields WHERE post_id = $1 AND name = 'has_polls'")
                .bind(post_id)
                .execute(&mut *conn)
                .await?;
            sqlx::query(
                "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
                 VALUES ($1, 'has_polls', 't', clock_timestamp(), clock_timestamp())",
            )
            .bind(post_id)
            .execute(&mut *conn)
            .await?;
        }
    } else {
        sqlx::query("DELETE FROM post_custom_fields WHERE post_id = $1 AND name = 'has_polls'")
            .bind(post_id)
            .execute(&mut *conn)
            .await?;
    }
    if changed {
        let urls = Urls {
            config: ctx.config,
            settings: ctx.settings,
        };
        let data = PostsData::load(conn, &urls, &[post_id], None).await?;
        let polls = data.serialize(&Guardian::anonymous(), post_id);
        let topic_id: i32 = sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
            .bind(post_id)
            .fetch_one(&mut *conn)
            .await?;
        crate::bus::publish_post_message(
            ctx.bus,
            conn,
            post_id,
            &format!("/polls/{topic_id}"),
            &json!({"post_id": post_id, "polls": polls}),
        )
        .await?;
    }
    Ok(())
}
