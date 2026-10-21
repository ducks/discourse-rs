//! Jobs::CrawlTopicLink: an external link's title, from the topic
//! embedding its url, else the file name of a linked image, else the
//! linked page's title (RetrieveTitle.crawl). The link is marked crawled
//! however it went.

use serde_json::Value;

use crate::final_destination::Fetcher;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// The job's Sidekiq name.
pub const CRAWL_TOPIC_LINK: &str = "crawl_topic_link";

/// `Jobs::CrawlTopicLink#execute`
pub async fn crawl_topic_link(state: &AppState, args: &Value) -> Result<(), AppError> {
    let id = match args.get("topic_link_id") {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(v)) if !crate::ruby::is_blank(v) => Some(crate::ruby::to_i(v)),
        _ => None,
    };
    let Some(id) = id else {
        return Err(
            Unsupported("crawl_topic_link without a topic_link_id (InvalidParameters)").into(),
        );
    };
    let mut conn = state.pool.acquire().await?;
    // TopicLink.find_by(id:, internal: false, crawled_at: nil)
    let link: Option<(i32, String)> = sqlx::query_as(
        "SELECT id, url FROM topic_links WHERE id = $1 AND NOT internal AND crawled_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((id, url)) = link else {
        return Ok(());
    };
    // A topic embedding the url gives its title: the first embed, whose
    // topic may be gone.
    let embedded: Option<Option<String>> = sqlx::query_scalar(
        "SELECT t.title FROM topic_embeds e \
         LEFT JOIN topics t ON t.id = e.topic_id AND t.deleted_at IS NULL \
         WHERE e.embed_url = $1 AND e.deleted_at IS NULL ORDER BY e.id LIMIT 1",
    )
    .bind(&url)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(Some(title)) = embedded {
        set_title(&mut conn, id, Some(&truncate(&title, 256))).await?;
        return Ok(());
    }
    let mut crawled = false;
    // A link to an image: its file name.
    if crate::uploads::is_supported_image(&url) {
        // URI(url): a url Ruby can't parse raises, which the job swallows.
        if let Some(uri) = crate::onebox::uri::RubyUri::parse(&url) {
            let filename = ruby_basename(&uri.path);
            crawled = set_title(&mut conn, id, Some(&filename)).await? == 1;
        } else {
            set_title(&mut conn, id, None).await?;
            return Ok(());
        }
    }
    if !crawled {
        let s =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        let fetcher = Fetcher {
            settings: &s,
            urls: Urls {
                config: &state.config,
                settings: &s,
            },
        };
        let title =
            crate::retrieve_title::crawl(&mut conn, &fetcher, &url, 5, false, Vec::new()).await?;
        if let Some(title) = title.filter(|t| !crate::ruby::is_blank(t)) {
            crawled = set_title(&mut conn, id, Some(&truncate(&title, 255))).await? == 1;
        }
    }
    if !crawled {
        set_title(&mut conn, id, None).await?;
    }
    Ok(())
}

/// `TopicLink.where(id:).update_all(title:, crawled_at: CURRENT_TIMESTAMP)`
/// (just crawled_at without a title): the rows updated.
async fn set_title(
    conn: &mut sqlx::PgConnection,
    id: i32,
    title: Option<&str>,
) -> Result<u64, AppError> {
    let done = match title {
        Some(title) => {
            sqlx::query("UPDATE topic_links SET title = $2, crawled_at = now() WHERE id = $1")
                .bind(id)
                .bind(title)
                .execute(&mut *conn)
                .await?
        }
        None => {
            sqlx::query("UPDATE topic_links SET crawled_at = now() WHERE id = $1")
                .bind(id)
                .execute(&mut *conn)
                .await?
        }
    };
    Ok(done.rows_affected())
}

/// `s[0..n-1]`: the first `n` characters.
fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `File.basename(path)`: the last segment, trailing slashes aside.
fn ruby_basename(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.is_empty() {
            String::new()
        } else {
            "/".into()
        };
    }
    trimmed.rsplit('/').next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basenames_as_ruby() {
        assert_eq!(ruby_basename("/images/cat.png"), "cat.png");
        assert_eq!(ruby_basename("/a/b/"), "b");
        assert_eq!(ruby_basename("/"), "/");
        assert_eq!(ruby_basename(""), "");
    }
}
