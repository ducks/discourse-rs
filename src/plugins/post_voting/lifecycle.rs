//! discourse-post-voting's hooks into core's write paths: the subtype a
//! new topic gets (the NewPostManager handler), the post validation, the
//! reply an email makes to the question, and the user destroyer's
//! content deletion callback.

use sqlx::PgConnection;

use super::{SUBTYPE, enabled};
use crate::plugins::PluginError;
use crate::site_settings::SiteSettings;

/// The NewPostManager handler: a new regular topic created with
/// `create_as_post_voting` "true" is a post voting one.
pub fn new_topic_subtype(
    topic_id: Option<i32>,
    create_as_post_voting: Option<&str>,
    archetype: Option<&str>,
) -> Option<&'static str> {
    let regular = archetype.is_none_or(|a| a.is_empty() || a == "regular");
    (topic_id.is_none() && create_as_post_voting == Some("true") && regular).then_some(SUBTYPE)
}

/// PostExtension's `ensure_only_replies`: in a post voting topic a post
/// replies to the question or to nothing. The error key, if not.
pub async fn reply_error(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
    reply_to_post_number: Option<i32>,
) -> Result<Option<&'static str>, PluginError> {
    if reply_to_post_number.is_none_or(|n| n == 1) {
        return Ok(None);
    }
    if !enabled(settings)? || !topic_is_post_voting(conn, settings, topic_id).await? {
        return Ok(None);
    }
    Ok(Some(
        "post.post_voting.errors.replying_to_post_not_permited",
    ))
}

/// The post `before_create` callback: an emailed reply to the question
/// of a post voting topic is an answer.
pub async fn email_reply_to(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
    reply_to_post_number: Option<i32>,
) -> Result<Option<i32>, PluginError> {
    if reply_to_post_number == Some(1)
        && enabled(settings)?
        && topic_is_post_voting(conn, settings, topic_id).await?
    {
        return Ok(None);
    }
    Ok(reply_to_post_number)
}

/// `topic.is_post_voting?` for a topic not trashed (`post.topic`).
async fn topic_is_post_voting(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
) -> Result<bool, PluginError> {
    let topic: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT archetype, subtype FROM topics WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    match topic {
        Some((archetype, subtype)) => {
            super::is_post_voting(conn, settings, topic_id, &archetype, subtype.as_deref()).await
        }
        None => Ok(false),
    }
}

/// The user destroyer's content deletion callback: the user's comments
/// and the reviewables of them deleted outright, and their votes taken
/// back.
pub async fn user_content_deleting(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<(), sqlx::Error> {
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM post_voting_comments WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(&mut *conn)
            .await?;
    sqlx::query("DELETE FROM post_voting_comments WHERE id = ANY($1)")
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "DELETE FROM reviewables WHERE type = 'ReviewablePostVotingComment' \
         AND target_type = 'PostVotingComment' AND target_id = ANY($1)",
    )
    .bind(ids.iter().map(|&id| id as i32).collect::<Vec<i32>>())
    .execute(&mut *conn)
    .await?;
    super::votes::bulk_remove_votes_by(conn, user_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_new_regular_topics_asked_for_become_post_voting() {
        assert_eq!(new_topic_subtype(None, Some("true"), None), Some(SUBTYPE));
        assert_eq!(
            new_topic_subtype(None, Some("true"), Some("regular")),
            Some(SUBTYPE)
        );
        assert_eq!(
            new_topic_subtype(None, Some("true"), Some("")),
            Some(SUBTYPE)
        );
        assert_eq!(new_topic_subtype(Some(1), Some("true"), None), None);
        assert_eq!(new_topic_subtype(None, Some("1"), None), None);
        assert_eq!(
            new_topic_subtype(None, Some("true"), Some("private_message")),
            None
        );
    }
}
