//! What a validated Topic save (`topic.update(...)`, `topic.save`) writes
//! beyond the attributes it was given.

use sqlx::PgConnection;

use super::text::{TitleOptions, clean_title, slug_for};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// Topic's before_validation assigns the cleaned title back through
/// `title=`, which recomputes the slug and clears fancy_title (read back
/// lazily later). Returns the slug the save writes alongside
/// `fancy_title = NULL`; a save then happens whenever the slug or
/// fancy_title differ, even with nothing else changed.
pub async fn reassigned_slug(
    conn: &mut PgConnection,
    s: &SiteSettings,
    topic_id: i32,
) -> Result<String, AppError> {
    let title: String = sqlx::query_scalar("SELECT title FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    let options = TitleOptions {
        prettify: s.get("title_prettify")?.truthy(),
        allow_uppercase_posts: s.get("allow_uppercase_posts")?.truthy(),
        remove_extraneous_space: s.get("title_remove_extraneous_space")?.truthy(),
    };
    if clean_title(&title, &options) != title {
        return Err(Unsupported("saving a topic whose title the cleaner changes").into());
    }
    if s.get("slug_generation_method")?.to_s() != "ascii" {
        return Err(Unsupported("slug_generation_method other than ascii").into());
    }
    Ok(slug_for(&title, &s.get("default_locale")?.to_s())?)
}
