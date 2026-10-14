//! `DiscoursePoll::PollsController`'s actions: voting and removing a vote
//! (`change_vote`), opening and closing a poll (`toggle_status`, also the
//! ClosePoll job), a page of voters and results grouped by a user field.

use chrono::NaiveDateTime;
use serde_json::{Value, json};
use sqlx::PgConnection;

use super::serializer::PostsData;
use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::topic_guardian::{PostCtx, TopicCtx};
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// What an action answers.
pub enum Outcome {
    Done(Value),
    /// `DiscoursePoll::Error`, rendered with `render_json_error`.
    Error(String),
    /// `Discourse::InvalidParameters`
    InvalidParameters(&'static str),
    /// `Discourse::InvalidAccess`
    Forbidden,
    /// `{error}` with a 404 (grouped results' missing poll).
    NotFound(String),
    /// `{error}` with a 422 (grouped results of a ranked choice poll).
    Unprocessable(String),
}

/// A post with what the guardian answers about it.
struct Loaded {
    post: PostCtx,
    topic: Option<TopicCtx>,
    can_see_topic: bool,
    can_see_post: bool,
}

/// The post, deleted ones only `with_deleted` (Post's default scope),
/// and its topic unless deleted.
async fn load_post(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i64,
    with_deleted: bool,
) -> Result<Option<Loaded>, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i32,
        user_id: Option<i32>,
        topic_id: i32,
        post_number: i32,
        post_type: i32,
        hidden: bool,
        hidden_at: Option<NaiveDateTime>,
        locked_by_id: Option<i32>,
        deleted_at: Option<NaiveDateTime>,
        user_deleted: bool,
        wiki: bool,
        created_at: NaiveDateTime,
        author_staff: Option<bool>,
    }
    let Ok(post_id) = i32::try_from(post_id) else {
        return Ok(None);
    };
    let row: Option<Row> = sqlx::query_as(
        "SELECT p.id, p.user_id, p.topic_id, p.post_number, p.post_type, p.hidden, p.hidden_at, \
                p.locked_by_id, p.deleted_at, p.user_deleted, p.wiki, p.created_at, \
                (u.admin OR u.moderator) AS author_staff \
         FROM posts p LEFT JOIN users u ON u.id = p.user_id WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row.filter(|r| with_deleted || r.deleted_at.is_none()) else {
        return Ok(None);
    };
    let s = ctx.settings;
    let topic = TopicCtx::load(&mut *conn, s, guardian, row.topic_id)
        .await?
        .filter(|t| !t.trashed());
    let can_see_topic = match &topic {
        Some(topic) => {
            let secure = guardian.secure_category_ids(&mut *conn, s).await?;
            guardian.can_see_topic(s, topic, true, &secure)?
        }
        None => false,
    };
    let post = PostCtx {
        id: row.id,
        user_id: row.user_id,
        post_number: row.post_number,
        post_type: row.post_type,
        hidden: row.hidden,
        hidden_at: row.hidden_at,
        locked_by_id: row.locked_by_id,
        deleted_at: row.deleted_at,
        user_deleted: row.user_deleted,
        wiki: row.wiki,
        created_at: row.created_at,
        author_staff: row.author_staff.unwrap_or(false),
    };
    let can_see_post = guardian.can_see_post(s, &post, can_see_topic)?;
    Ok(Some(Loaded {
        post,
        topic,
        can_see_topic,
        can_see_post,
    }))
}

/// The `ensure_can_see_post` before action: refused only for a post
/// whose topic the viewer sees.
pub async fn ensure_can_see_post(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i64,
) -> Result<bool, AppError> {
    Ok(
        match load_post(conn, ctx, guardian, post_id, false).await? {
            Some(l) => !l.can_see_topic || l.can_see_post,
            None => true,
        },
    )
}

fn t(ctx: &Ctx<'_>, key: &str, args: &[(&str, &str)]) -> String {
    ctx.i18n
        .t_with(key, args)
        .unwrap_or_else(|| key.to_string())
}

async fn find_poll(
    conn: &mut PgConnection,
    post_id: i64,
    name: &str,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM polls WHERE post_id = $1 AND name = $2 ORDER BY id LIMIT 1")
        .bind(post_id)
        .bind(name)
        .fetch_optional(conn)
        .await
}

/// `change_vote`'s checks: the poll the user may vote in, or the error.
async fn change_vote_poll(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    urls: &Urls<'_>,
    guardian: &Guardian,
    post_id: i64,
    poll_name: &str,
) -> Result<Result<(i32, i32, PostsData), String>, AppError> {
    let Some(loaded) = load_post(conn, ctx, guardian, post_id, false).await? else {
        return Ok(Err(t(ctx, "poll.post_is_deleted", &[])));
    };
    if loaded.topic.as_ref().is_some_and(|t| t.archived) {
        return Ok(Err(t(ctx, "poll.topic_must_be_open_to_vote", &[])));
    }
    let can_create_post = match &loaded.topic {
        Some(topic) => {
            let anywhere = guardian
                .can_create_post_anywhere(&mut *conn, ctx.settings)
                .await?;
            guardian.can_create_post(ctx.settings, topic, anywhere)?
        }
        None => false,
    };
    if !loaded.can_see_post || !can_create_post {
        return Ok(Err(t(ctx, "poll.user_cant_post_in_topic", &[])));
    }
    let Some(poll_id) = find_poll(conn, post_id, poll_name).await? else {
        return Ok(Err(t(
            ctx,
            "poll.no_poll_with_this_name",
            &[("name", poll_name)],
        )));
    };
    let data = PostsData::load_poll(conn, urls, poll_id, guardian.user_id()).await?;
    let poll = data.poll().expect("the poll was found");
    if poll.is_closed() {
        return Ok(Err(t(ctx, "poll.poll_must_be_open_to_vote", &[])));
    }
    if let Some(groups) = &poll.groups {
        let names: Vec<String> = sqlx::query_scalar(
            "SELECT lower(g.name) FROM groups g JOIN group_users gu ON gu.group_id = g.id \
             WHERE gu.user_id = $1",
        )
        .bind(guardian.user_id())
        .fetch_all(&mut *conn)
        .await?;
        if !groups.split(',').any(|g| names.contains(&g.to_lowercase())) {
            return Ok(Err(t(
                ctx,
                "js.poll.results.groups.title",
                &[("groups", groups)],
            )));
        }
    }
    Ok(Ok((loaded.post.id, loaded.topic.map_or(0, |t| t.id), data)))
}

/// After the change: the poll serialized for the user, and for anyone
/// on `/polls/<topic_id>`.
async fn changed(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    urls: &Urls<'_>,
    guardian: &Guardian,
    post_id: i32,
    poll_id: i64,
) -> Result<Value, AppError> {
    let data = PostsData::load_poll(conn, urls, poll_id, guardian.user_id()).await?;
    let poll = data.poll().expect("the poll exists");
    let serialized = data.serialize_poll(guardian, poll);
    let anonymous = Guardian::anonymous();
    let public = PostsData::load_poll(conn, urls, poll_id, None).await?;
    let message = json!({
        "post_id": post_id,
        "polls": [public.serialize_poll(&anonymous, public.poll().expect("the poll exists"))],
    });
    let channel = format!("/polls/{}", post_topic(conn, post_id).await?);
    crate::bus::publish_post_message(ctx.bus, conn, post_id, &channel, &message).await?;
    Ok(serialized)
}

async fn post_topic(conn: &mut PgConnection, post_id: i32) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
        .bind(post_id)
        .fetch_one(conn)
        .await
}

/// A ranked choice vote's rank, cast to the integer column.
fn rank_value(rank: &Value) -> Result<i32, Unsupported> {
    match rank {
        Value::Number(n) => n.as_i64().and_then(|n| i32::try_from(n).ok()),
        Value::String(s) => crate::ruby::integer(s.trim()).and_then(|n| i32::try_from(n).ok()),
        _ => None,
    }
    .ok_or(Unsupported(
        "a ranked choice vote with a rank that is not an integer",
    ))
}

/// `DiscoursePoll::Poll.vote`: the poll and the votes it kept.
pub async fn vote(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i64,
    poll_name: &str,
    options: &Value,
) -> Result<Outcome, AppError> {
    let urls = Urls {
        config: ctx.config,
        settings: ctx.settings,
    };
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("voting anonymously"))?;
    let (post_id, _, data) =
        match change_vote_poll(conn, ctx, &urls, guardian, post_id, poll_name).await? {
            Ok(found) => found,
            Err(error) => return Ok(Outcome::Error(error)),
        };
    let poll = data.poll().expect("the poll was found");
    let ranked = poll.kind() == "ranked_choice";
    let available: Vec<&str> = data
        .options_of(poll.id)
        .map(|o| o.digest.as_str())
        .collect();
    // The options that are the poll's: digests, or for ranked choice
    // {digest, rank}.
    let kept: Vec<Value> = if ranked {
        let Value::Object(map) = options else {
            return Err(Unsupported("ranked choice votes not given by index").into());
        };
        let mut kept = Vec::new();
        for value in map.values() {
            if !value.is_object() {
                return Err(Unsupported("ranked choice votes not given as objects").into());
            }
            if value["digest"]
                .as_str()
                .is_some_and(|d| available.contains(&d))
            {
                kept.push(value.clone());
            }
        }
        if kept.iter().all(|o| o["rank"] == json!("0")) {
            return Ok(Outcome::Error(t(
                ctx,
                "poll.requires_that_at_least_one_option_is_ranked",
                &[],
            )));
        }
        kept
    } else {
        let Value::Array(list) = options else {
            return Err(Unsupported("poll votes not given as a list").into());
        };
        list.iter()
            .filter(|o| o.as_str().is_some_and(|d| available.contains(&d)))
            .cloned()
            .collect()
    };
    if kept.is_empty() {
        return Ok(Outcome::Error(t(
            ctx,
            "poll.requires_at_least_1_valid_option",
            &[],
        )));
    }
    let chosen = |digest: &str| {
        if ranked {
            kept.iter().find(|o| o["digest"] == json!(digest))
        } else {
            kept.iter().find(|o| o.as_str() == Some(digest))
        }
    };
    let new_options: Vec<(i64, Option<&Value>)> = data
        .options_of(poll.id)
        .filter_map(|o| chosen(&o.digest).map(|c| (o.id, Some(c))))
        .collect();
    // validate_votes!
    let count = new_options.len() as i64;
    match poll.kind() {
        "multiple" => {
            if let Some(min) = poll.min.filter(|m| count < i64::from(*m)) {
                return Ok(Outcome::Error(
                    ctx.i18n
                        .t_count(
                            "poll.min_vote_per_user",
                            i64::from(min),
                            &[("count", &min.to_string())],
                        )
                        .unwrap_or_default(),
                ));
            }
            if let Some(max) = poll.max.filter(|m| count > i64::from(*m)) {
                return Ok(Outcome::Error(
                    ctx.i18n
                        .t_count(
                            "poll.max_vote_per_user",
                            i64::from(max),
                            &[("count", &max.to_string())],
                        )
                        .unwrap_or_default(),
                ));
            }
        }
        "ranked_choice" => {
            if data.options_of(poll.id).count() as i64 != count {
                // Its message reads poll.options, which a Poll lacks: a 500.
                return Err(Unsupported("a ranked choice vote leaving options out").into());
            }
        }
        _ => {
            if count > 1 {
                return Ok(Outcome::Error(t(ctx, "poll.one_vote_per_user", &[])));
            }
        }
    }
    let new_ids: Vec<i64> = new_options.iter().map(|(id, _)| *id).collect();
    let old_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT o.id FROM poll_options o WHERE o.poll_id = $1 \
           AND EXISTS (SELECT 1 FROM poll_votes v WHERE v.poll_option_id = o.id AND v.user_id = $2) \
         ORDER BY o.id",
    )
    .bind(poll.id)
    .bind(i64::from(user_id))
    .fetch_all(&mut *conn)
    .await?;
    let creation: Vec<(i64, i32)> = if ranked {
        sqlx::query("DELETE FROM poll_votes WHERE poll_id = $1 AND user_id = $2")
            .bind(poll.id)
            .bind(i64::from(user_id))
            .execute(&mut *conn)
            .await?;
        let mut creation = Vec::new();
        for (id, choice) in &new_options {
            let rank = rank_value(&choice.expect("chosen")["rank"])?;
            creation.push((*id, rank));
        }
        creation
    } else {
        sqlx::query(
            "DELETE FROM poll_votes WHERE poll_id = $1 AND user_id = $2 \
             AND NOT (poll_option_id = ANY($3))",
        )
        .bind(poll.id)
        .bind(i64::from(user_id))
        .bind(&new_ids)
        .execute(&mut *conn)
        .await?;
        new_ids
            .iter()
            .filter(|id| !old_ids.contains(id))
            .map(|id| (*id, 0))
            .collect()
    };
    for (option_id, rank) in creation {
        sqlx::query(
            "INSERT INTO poll_votes (poll_id, poll_option_id, user_id, rank, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, clock_timestamp(), clock_timestamp())",
        )
        .bind(poll.id)
        .bind(option_id)
        .bind(i64::from(user_id))
        .bind(rank)
        .execute(&mut *conn)
        .await?;
    }
    let poll_id = poll.id;
    let kind = poll.kind();
    let mut serialized = changed(conn, ctx, &urls, guardian, post_id, poll_id).await?;
    if !ranked {
        // Votes past the poll's allowance go, all of the user's with them
        // (the DELETE joins on poll and user only).
        let offset = if kind == "multiple" {
            serialized["max"].as_i64().unwrap_or_else(|| {
                serialized["options"]
                    .as_array()
                    .map_or(0, |o| o.len() as i64)
            })
        } else {
            1
        };
        sqlx::query(
            "DELETE FROM poll_votes USING (\
               SELECT poll_id, user_id FROM poll_votes WHERE poll_id = $1 AND user_id = $3 \
               ORDER BY created_at DESC OFFSET $2) to_delete_poll_votes \
             WHERE poll_votes.poll_id = to_delete_poll_votes.poll_id \
               AND poll_votes.user_id = to_delete_poll_votes.user_id",
        )
        .bind(poll_id)
        .bind(offset)
        .bind(i64::from(user_id))
        .execute(&mut *conn)
        .await?;
    }
    if (kind == "ranked_choice" || kind == "multiple")
        && let Some(options) = serialized["options"].as_array_mut()
    {
        for option in options {
            let digest = option["id"].as_str().unwrap_or_default().to_string();
            let ranks: Vec<i32> = sqlx::query_scalar(
                "SELECT v.rank FROM poll_votes v JOIN poll_options o ON o.id = v.poll_option_id \
                 WHERE o.digest = $1 AND v.user_id = $2 AND v.poll_id = $3 LIMIT 1",
            )
            .bind(&digest)
            .bind(i64::from(user_id))
            .bind(poll_id)
            .fetch_all(&mut *conn)
            .await?;
            if let Value::Object(o) = option {
                if kind == "ranked_choice" {
                    o.insert("rank".into(), json!(ranks));
                } else {
                    o.insert("chosen".into(), json!(!ranks.is_empty()));
                }
            }
        }
    }
    Ok(Outcome::Done(json!({"poll": serialized, "vote": kept})))
}

/// `DiscoursePoll::Poll.remove_vote`
pub async fn remove_vote(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i64,
    poll_name: &str,
) -> Result<Outcome, AppError> {
    let urls = Urls {
        config: ctx.config,
        settings: ctx.settings,
    };
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("voting anonymously"))?;
    let (post_id, _, data) =
        match change_vote_poll(conn, ctx, &urls, guardian, post_id, poll_name).await? {
            Ok(found) => found,
            Err(error) => return Ok(Outcome::Error(error)),
        };
    let poll_id = data.poll().expect("the poll was found").id;
    sqlx::query("DELETE FROM poll_votes WHERE poll_id = $1 AND user_id = $2")
        .bind(poll_id)
        .bind(i64::from(user_id))
        .execute(&mut *conn)
        .await?;
    let serialized = changed(conn, ctx, &urls, guardian, post_id, poll_id).await?;
    Ok(Outcome::Done(json!({ "poll": serialized })))
}

/// `DiscoursePoll::Poll.toggle_status`; without `raise_errors` (the
/// ClosePoll job) a refusal is quiet.
pub async fn toggle_status(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i64,
    poll_name: &str,
    status: &str,
    raise_errors: bool,
) -> Result<Outcome, AppError> {
    let urls = Urls {
        config: ctx.config,
        settings: ctx.settings,
    };
    let error = |key: &str, args: &[(&str, &str)]| {
        Ok(if raise_errors {
            Outcome::Error(t(ctx, key, args))
        } else {
            Outcome::Done(Value::Null)
        })
    };
    let Some(loaded) = load_post(conn, ctx, guardian, post_id, false).await? else {
        return error("poll.post_is_deleted", &[]);
    };
    if loaded.topic.as_ref().is_some_and(|t| t.archived) {
        return error("poll.topic_must_be_open_to_toggle_status", &[]);
    }
    if !loaded.can_see_post {
        return error("poll.user_cant_post_in_topic", &[]);
    }
    let user_id = guardian.user_id();
    if !(user_id.is_some() && loaded.post.user_id == user_id || guardian.is_staff()) {
        return error("poll.only_staff_or_op_can_toggle_status", &[]);
    }
    if !loaded.can_see_topic {
        return error("poll.user_cant_post_in_topic", &[]);
    }
    let Some(poll_id) = find_poll(conn, post_id, poll_name).await? else {
        return error("poll.no_poll_with_this_name", &[("name", poll_name)]);
    };
    let status_id = match status {
        "open" => 0,
        "closed" => 1,
        _ => return Err(Unsupported("a poll status other than open or closed").into()),
    };
    let current: i32 = sqlx::query_scalar("SELECT status FROM polls WHERE id = $1")
        .bind(poll_id)
        .fetch_one(&mut *conn)
        .await?;
    if current != status_id {
        let closing = status_id == 1;
        sqlx::query(
            "UPDATE polls SET status = $2, closed_by_id = $3, \
               closed_at = CASE WHEN $4 THEN clock_timestamp() END, updated_at = clock_timestamp() \
             WHERE id = $1",
        )
        .bind(poll_id)
        .bind(status_id)
        .bind(if closing { user_id } else { None })
        .bind(closing)
        .execute(&mut *conn)
        .await?;
        // StaffActionLogger#log_custom: post_id and subject are columns,
        // leaving no details.
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, custom_type, details, post_id, subject, \
                                         admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, '', $4, $5, TRUE, clock_timestamp(), clock_timestamp())",
        )
        .bind(crate::staff_action_logs::action_id("custom_staff"))
        .bind(user_id)
        .bind(if closing { "poll_closed" } else { "poll_opened" })
        .bind(loaded.post.id)
        .bind(poll_name)
        .execute(&mut *conn)
        .await?;
    }
    let serialized = changed(conn, ctx, &urls, guardian, loaded.post.id, poll_id).await?;
    Ok(Outcome::Done(serialized))
}

/// The voters' page params: `page`, `limit` and `option_id`.
pub struct VoterPage<'a> {
    pub page: Option<&'a str>,
    pub limit: Option<&'a str>,
    pub option_id: Option<&'a str>,
}

/// GET /polls/voters
pub async fn voters(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i64,
    poll_name: &str,
    page: &VoterPage<'_>,
) -> Result<Outcome, AppError> {
    let urls = Urls {
        config: ctx.config,
        settings: ctx.settings,
    };
    let Some(poll_id) = find_poll(conn, post_id, poll_name).await? else {
        let exists = load_post(conn, ctx, guardian, post_id, false)
            .await?
            .is_some();
        return Ok(Outcome::InvalidParameters(if exists {
            "poll_name"
        } else {
            "post_id"
        }));
    };
    // ensure_can_see!(poll.post), the post found with deleted ones.
    let can_see = load_post(conn, ctx, guardian, post_id, true)
        .await?
        .is_some_and(|l| l.can_see_post);
    if !can_see {
        return Ok(Outcome::Forbidden);
    }
    let data = PostsData::load_poll(conn, &urls, poll_id, guardian.user_id()).await?;
    let poll = data.poll().expect("the poll was found");
    if !poll.can_see_voters(guardian) {
        return Ok(Outcome::InvalidParameters("poll_name"));
    }
    let VoterPage {
        page,
        limit,
        option_id,
    } = page;
    let page = page.map_or(1, crate::ruby::to_i).max(1);
    let limit = limit.map_or(25, crate::ruby::to_i).clamp(1, 50);
    let option = option_id.filter(|o| !crate::ruby::is_blank(o));
    let voters =
        super::serializer::serialized_voters(conn, &urls, poll, page, limit, option).await?;
    Ok(Outcome::Done(json!({ "voters": voters })))
}

/// GET /polls/grouped_poll_results
pub async fn grouped_poll_results(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i64,
    poll_name: &str,
    user_field_name: &str,
) -> Result<Outcome, AppError> {
    let urls = Urls {
        config: ctx.config,
        settings: ctx.settings,
    };
    let Some(poll_id) = find_poll(conn, post_id, poll_name).await? else {
        return Ok(Outcome::NotFound(t(ctx, "poll.errors.poll_not_found", &[])));
    };
    let data = PostsData::load_poll(conn, &urls, poll_id, guardian.user_id()).await?;
    let poll = data.poll().expect("the poll was found");
    if poll.kind() == "ranked_choice" {
        return Ok(Outcome::Unprocessable(t(
            ctx,
            "poll.ranked_choice.no_group_results_support",
            &[],
        )));
    }
    let Some(loaded) = load_post(conn, ctx, guardian, post_id, false).await? else {
        return Ok(Outcome::InvalidParameters("post_id"));
    };
    if !loaded.can_see_post {
        return Ok(Outcome::Error(t(ctx, "poll.user_cant_post_in_topic", &[])));
    }
    let can_create_post = match &loaded.topic {
        Some(topic) => {
            let anywhere = guardian
                .can_create_post_anywhere(&mut *conn, ctx.settings)
                .await?;
            guardian.can_create_post(ctx.settings, topic, anywhere)?
        }
        None => false,
    };
    if !can_create_post {
        return Ok(Outcome::Error(t(ctx, "poll.user_cant_post_in_topic", &[])));
    }
    if !poll.can_see_results(guardian) {
        return Ok(Outcome::InvalidParameters("poll_name"));
    }
    let groupable = ctx.settings.get("poll_groupable_user_fields")?.to_s();
    if !groupable.split('|').any(|f| f == user_field_name) {
        return Ok(Outcome::InvalidParameters("user_field_name"));
    }
    // transform_for_user_field_override
    let field: Option<i32> =
        sqlx::query_scalar("SELECT id FROM user_fields WHERE name = $1 LIMIT 1")
            .bind(user_field_name)
            .fetch_optional(&mut *conn)
            .await?;
    let field_name = match field {
        Some(id) => format!("user_field_{id}"),
        None => user_field_name.to_string(),
    };
    let votes: Vec<(i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT v.poll_option_id, v.user_id, \
                (SELECT f.value FROM user_custom_fields f WHERE f.user_id = v.user_id AND f.name = $2 \
                 ORDER BY f.id DESC LIMIT 1) \
         FROM poll_votes v WHERE v.poll_id = $1",
    )
    .bind(poll_id)
    .bind(&field_name)
    .fetch_all(&mut *conn)
    .await?;
    let options: Vec<(i64, String, String)> = data
        .options_of(poll_id)
        .map(|o| (o.id, o.digest.clone(), o.html.clone()))
        .collect();
    // group_by the field's value, in order seen.
    let mut groups: Vec<(Option<String>, Vec<i64>)> = Vec::new();
    for (option_id, _, value) in votes {
        match groups.iter_mut().find(|(v, _)| *v == value) {
            Some((_, ids)) => ids.push(option_id),
            None => groups.push((value, vec![option_id])),
        }
    }
    let mut chart = Vec::new();
    for (value, option_ids) in groups {
        let label = match value {
            Some(v) => titleize(&v)?,
            None => t(ctx, "poll.user_field.no_data", &[]),
        };
        let options: Vec<Value> = options
            .iter()
            .map(|(id, digest, html)| {
                let votes = option_ids.iter().filter(|o| *o == id).count();
                json!({"digest": digest, "html": html, "votes": votes})
            })
            .collect();
        chart.push(json!({"group": label, "options": options}));
    }
    Ok(Outcome::Done(json!({ "grouped_results": chart })))
}

/// ActiveSupport's `String#titleize` (no acronyms or human inflections):
/// underscore, humanize, then each word's first letter up.
pub(crate) fn titleize(word: &str) -> Result<String, Unsupported> {
    if !word.is_ascii() {
        return Err(Unsupported("titleizing a non-ASCII user field value"));
    }
    let word_char = |c: char| c.is_ascii_alphanumeric() || c == '_';
    // underscore
    let chars: Vec<char> = word.replace("::", "/").chars().collect();
    let mut underscored = String::new();
    if word.contains(|c: char| c.is_ascii_uppercase() || c == '-') || word.contains("::") {
        for (i, c) in chars.iter().enumerate() {
            if i > 0 && c.is_ascii_uppercase() {
                let prev = chars[i - 1];
                let next_lower = chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
                if prev.is_ascii_lowercase()
                    || prev.is_ascii_digit()
                    || (prev.is_ascii_uppercase() && next_lower)
                {
                    underscored.push('_');
                }
            }
            underscored.push(if *c == '-' { '_' } else { *c });
        }
        underscored = underscored.to_lowercase();
    } else {
        underscored = word.to_string();
    }
    // humanize
    let mut human = underscored.trim_start_matches('_').to_string();
    if let Some(stripped) = human.strip_suffix("_id") {
        human = stripped.to_string();
    }
    let human: String = human.replace('_', " ");
    let mut lowered = String::new();
    let mut run = String::new();
    for c in human.chars() {
        if c.is_ascii_alphanumeric() {
            run.push(c);
        } else {
            lowered.push_str(&run.to_lowercase());
            run.clear();
            lowered.push(c);
        }
    }
    lowered.push_str(&run.to_lowercase());
    let mut human: Vec<char> = lowered.chars().collect();
    if let Some(first) = human.first_mut()
        && word_char(*first)
    {
        *first = first.to_ascii_uppercase();
    }
    // titleize: /\b(?<!\w['’`()])[a-z]/
    for i in 0..human.len() {
        let c = human[i];
        if !c.is_ascii_lowercase() || (i > 0 && word_char(human[i - 1])) {
            continue;
        }
        let guarded = i >= 2 && word_char(human[i - 2]) && "'`()".contains(human[i - 1]);
        if !guarded {
            human[i] = c.to_ascii_uppercase();
        }
    }
    Ok(human.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::titleize;

    #[test]
    fn titleizes_as_active_support() {
        assert_eq!(titleize("united states").unwrap(), "United States");
        assert_eq!(titleize("fooBar").unwrap(), "Foo Bar");
        assert_eq!(
            titleize("x-men: the last stand").unwrap(),
            "X Men: The Last Stand"
        );
        assert_eq!(titleize("employee_id").unwrap(), "Employee");
        assert_eq!(titleize("don't stop").unwrap(), "Don't Stop");
    }
}
