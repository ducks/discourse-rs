//! discourse-solved's ComposerMessagesFinder check
//! (`add_to_class(:composer_messages_finder, :check_topic_is_solved)`):
//! replying to a solved topic, the member is asked to start a new topic
//! for an unrelated issue, unless disable_solved_education_message.

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::AppError;
use crate::composer::messages::Finder;

/// `check_topic_is_solved`
pub async fn check_topic_is_solved(
    conn: &mut PgConnection,
    finder: &Finder<'_>,
) -> Result<Option<Value>, AppError> {
    let settings = finder.settings;
    if !super::enabled(settings)? || settings.get("disable_solved_education_message")?.truthy() {
        return Ok(None);
    }
    let Some(topic) = finder.topic.as_ref().filter(|t| !t.private_message()) else {
        return Ok(None);
    };
    if !finder.replying() {
        return Ok(None);
    }
    // @topic.solved.nil?
    let solved: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM discourse_solved_solved_topics WHERE topic_id = $1)",
    )
    .bind(topic.id)
    .fetch_one(&mut *conn)
    .await?;
    if !solved {
        return Ok(None);
    }
    let base_url = crate::url::Urls {
        config: &finder.state.config,
        settings,
    }
    .base_url()?;
    let text = finder.t("education.topic_is_solved", &[("base_url", &base_url)]);
    Ok(Some(json!({
        "id": "solved_topic",
        "templateName": "education",
        "wait_for_typing": true,
        "extraClass": "education-message",
        "hide_if_whisper": true,
        "body": finder.cook(&text).await?,
    })))
}
