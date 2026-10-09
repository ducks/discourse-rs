//! discourse-topic-voting's VotesController services: Votes::Cast (POST
//! /voting/vote), Votes::Remove (POST /voting/unvote) and the voters a
//! topic shows (GET /voting/who). The caller holds the transaction.
//!
//! Not ported: the topic_upvote and topic_unvote web hooks, refused when
//! one is active, and the topic_voting_vote_created event, which only
//! discourse-workflows listens to, refused when a workflow could.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::{Categories, UserVotes};
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::topic_guardian::TopicCtx;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `DiscourseTopicVoting::VOTER_PREVIEW_LIMIT`
pub const VOTER_PREVIEW_LIMIT: i64 = 104;

pub enum Outcome {
    /// The voting response, with `alert` for a vote.
    Done(Value),
    /// `Discourse::NotFound`
    NotFound,
    /// `Discourse::InvalidAccess`
    Forbidden,
    /// The voter is out of votes: the voting response, with 403.
    OutOfVotes(Value),
}

/// `Topic.find_by(id:)` (not deleted) the guardian can see: None when it
/// doesn't exist, Some(false) when it can't be seen.
async fn visible_topic(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    topic_id: i32,
) -> Result<Option<(TopicCtx, bool)>, AppError> {
    let Some(topic) = TopicCtx::load(&mut *conn, settings, guardian, topic_id).await? else {
        return Ok(None);
    };
    if topic.deleted_at.is_some() {
        return Ok(None);
    }
    let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
    let can_see = guardian.can_see_topic(settings, &topic, true, &secure)?;
    Ok(Some((topic, can_see)))
}

/// `Topic#update_vote_count`: every vote, archived ones too.
pub async fn update_vote_count(conn: &mut PgConnection, topic_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO topic_voting_topic_vote_count (topic_id, votes_count, created_at, updated_at) \
         SELECT $1, count(*), CURRENT_TIMESTAMP, CURRENT_TIMESTAMP FROM topic_voting_votes WHERE topic_id = $1 \
         ON CONFLICT (topic_id) DO UPDATE SET votes_count = EXCLUDED.votes_count, updated_at = CURRENT_TIMESTAMP \
         WHERE topic_voting_topic_vote_count.topic_id = $1",
    )
    .bind(topic_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// `Topic#vote_count`
async fn vote_count(conn: &mut PgConnection, topic_id: i32) -> Result<i32, sqlx::Error> {
    let count: Option<Option<i32>> = sqlx::query_scalar(
        "SELECT votes_count FROM topic_voting_topic_vote_count WHERE topic_id = $1",
    )
    .bind(topic_id)
    .fetch_optional(conn)
    .await?;
    Ok(count.flatten().unwrap_or(0))
}

/// `Topic#who_voted`: the latest voters as BasicUsers, nil unless
/// topic_voting_show_who_voted.
pub async fn who_voted(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    topic_id: i32,
    limit: i64,
) -> Result<Value, AppError> {
    if !settings.get("topic_voting_show_who_voted")?.truthy() {
        return Ok(Value::Null);
    }
    let users: Vec<(i32, String, Option<String>, Option<i32>)> = sqlx::query_as(
        "SELECT u.id, u.username, u.name, u.uploaded_avatar_id FROM topic_voting_votes v \
         JOIN users u ON u.id = v.user_id WHERE v.topic_id = $1 \
         ORDER BY v.created_at DESC LIMIT $2",
    )
    .bind(topic_id)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    let names = settings.get("enable_names")?.truthy();
    let mut out = Vec::with_capacity(users.len());
    for (id, username, name, avatar) in users {
        let mut u = Map::new();
        u.insert("id".into(), json!(id));
        u.insert("username".into(), json!(username));
        if names {
            u.insert("name".into(), json!(name));
        }
        u.insert(
            "avatar_template".into(),
            json!(crate::avatar::avatar_template(
                urls, id, &username, avatar, None
            )?),
        );
        out.push(Value::Object(u));
    }
    Ok(Value::Array(out))
}

/// VotesController#voting_response
async fn voting_response(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    guardian: &Guardian,
    topic_id: i32,
) -> Result<(Map<String, Value>, UserVotes), AppError> {
    let user = guardian.user().ok_or(Unsupported("voting anonymously"))?;
    let votes = UserVotes::load(conn, settings, user.id, user.trust_level).await?;
    let mut out = Map::new();
    out.insert("can_vote".into(), json!(!votes.reached()));
    out.insert("vote_limit".into(), json!(votes.limit));
    out.insert(
        "vote_count".into(),
        json!(vote_count(conn, topic_id).await?),
    );
    out.insert(
        "who_voted".into(),
        who_voted(conn, settings, urls, topic_id, VOTER_PREVIEW_LIMIT).await?,
    );
    out.insert("votes_left".into(), json!(votes.left()));
    Ok((out, votes))
}

/// Votes::Cast
pub async fn cast(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    guardian: &Guardian,
    topic_id: i32,
) -> Result<Outcome, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Err(Unsupported("voting anonymously").into());
    };
    let Some((topic, can_see)) = visible_topic(conn, settings, guardian, topic_id).await? else {
        return Ok(Outcome::NotFound);
    };
    if !can_see {
        return Ok(Outcome::Forbidden);
    }
    let categories = Categories::load(conn).await?;
    if !categories.can_vote(topic.id, &topic.archetype, topic.category_id) {
        return Ok(Outcome::Forbidden);
    }
    // topic_not_already_voted: any vote, archived too.
    let voted: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_voting_votes WHERE topic_id = $1 AND user_id = $2)",
    )
    .bind(topic.id)
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if voted {
        return Ok(Outcome::Forbidden);
    }
    let user = guardian.user().ok_or(Unsupported("voting anonymously"))?;
    if UserVotes::load(conn, settings, user_id, user.trust_level)
        .await?
        .reached()
    {
        let (mut out, votes) = voting_response(conn, settings, urls, guardian, topic.id).await?;
        out.insert("alert".into(), json!(votes.alert_low(settings)?));
        return Ok(Outcome::OutOfVotes(Value::Object(out)));
    }
    sqlx::query(
        "INSERT INTO topic_voting_votes (topic_id, user_id, archive, created_at, updated_at) \
         VALUES ($1, $2, false, now(), now())",
    )
    .bind(topic.id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    update_vote_count(conn, topic.id).await?;
    crate::jobs::enqueue(
        conn,
        "Jobs::DiscourseTopicVoting::BackfillBadges",
        json!({ "topic_id": topic.id }),
    )
    .await?;
    if crate::plugins::web_hooks_active(conn, "topic_upvote").await? {
        return Err(Unsupported("topic_upvote web hooks").into());
    }
    workflows_unsupported(conn).await?;
    let (mut out, votes) = voting_response(conn, settings, urls, guardian, topic.id).await?;
    out.insert("alert".into(), json!(votes.alert_low(settings)?));
    Ok(Outcome::Done(Value::Object(out)))
}

/// Votes::Remove: the viewer's active vote, if any, gone.
pub async fn remove(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    guardian: &Guardian,
    topic_id: i32,
) -> Result<Outcome, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Err(Unsupported("voting anonymously").into());
    };
    let Some((topic, can_see)) = visible_topic(conn, settings, guardian, topic_id).await? else {
        return Ok(Outcome::NotFound);
    };
    if !can_see {
        return Ok(Outcome::Forbidden);
    }
    let removed = sqlx::query(
        "DELETE FROM topic_voting_votes WHERE topic_id = $1 AND user_id = $2 AND NOT archive",
    )
    .bind(topic.id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if removed > 0 {
        update_vote_count(conn, topic.id).await?;
        if crate::plugins::web_hooks_active(conn, "topic_unvote").await? {
            return Err(Unsupported("topic_unvote web hooks").into());
        }
    }
    let (out, _) = voting_response(conn, settings, urls, guardian, topic.id).await?;
    Ok(Outcome::Done(Value::Object(out)))
}

/// VotesController#who: the topic's voters (up to `limit`, at most
/// VOTER_PREVIEW_LIMIT), for a topic the viewer can see.
pub async fn who(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    guardian: &Guardian,
    topic_id: i32,
    limit: i64,
) -> Result<Outcome, AppError> {
    let Some((topic, can_see)) = visible_topic(conn, settings, guardian, topic_id).await? else {
        return Ok(Outcome::NotFound);
    };
    if !can_see {
        return Ok(Outcome::Forbidden);
    }
    Ok(Outcome::Done(
        who_voted(conn, settings, urls, topic.id, limit).await?,
    ))
}

/// discourse-workflows' "topic received vote" trigger runs on
/// topic_voting_vote_created; workflows aren't ported, so an active one
/// with that trigger is refused.
async fn workflows_unsupported(conn: &mut PgConnection) -> Result<(), AppError> {
    let workflows: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM discourse_workflows_workflows \
         WHERE active_version_id IS NOT NULL AND nodes::text LIKE '%trigger:topic_received_vote%')",
    )
    .fetch_one(conn)
    .await?;
    if workflows {
        return Err(Unsupported("discourse-workflows triggers on votes").into());
    }
    Ok(())
}
