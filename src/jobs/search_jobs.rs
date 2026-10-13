//! The search index's scheduled jobs: Jobs::ReindexSearch (rows out of
//! date or missing reindexed, rows of long-deleted posts and topics
//! cleaned) and Jobs::CreateRecentPostSearchIndexes (the recent posts'
//! partial index and the offset search reads).

use sqlx::PgConnection;

use crate::posting::search_index::{self, BaseUrls};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `Jobs::ReindexSearch`, every 2 hours.
pub const REINDEX_SEARCH: &str = "reindex_search";
/// `Jobs::CreateRecentPostSearchIndexes`, daily.
pub const CREATE_RECENT_POST_SEARCH_INDEXES: &str = "create_recent_post_search_indexes";

/// `SearchIndexer::*_INDEX_VERSION`
const POST_INDEX_VERSION: i32 = 5;
const TOPIC_INDEX_VERSION: i32 = 4;
const CATEGORY_INDEX_VERSION: i32 = 3;
const USER_INDEX_VERSION: i32 = 3;
const TAG_INDEX_VERSION: i32 = 3;
const CHAT_MESSAGE_INDEX_VERSION: i32 = 1;

async fn settings(state: &AppState, conn: &mut PgConnection) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `Jobs::ReindexSearch`: categories (their posts queued for a reindex),
/// tags, topics, posts, users and chat messages whose rows are missing,
/// of another locale or version; then the rows of topics and posts
/// deleted over a day ago.
pub async fn reindex_search(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let locale = s.get("default_locale")?.to_s().to_string();
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &s,
    };
    let base_path = state.config.globals.relative_url_root().to_string();
    let base = BaseUrls {
        base_path: &base_path,
        base_url_no_prefix: urls.base_url_no_prefix()?,
        store: crate::file_store::FileStore::for_site(urls.config, &s)?,
    };

    // rebuild_categories(limit: 500)
    let categories: Vec<(i32, String)> = sqlx::query_as(
        "SELECT categories.id, categories.name FROM categories \
         LEFT JOIN category_search_data ON category_id = categories.id \
         WHERE category_search_data.locale IS NULL OR category_search_data.locale != $1 \
            OR category_search_data.version != $2 \
         ORDER BY categories.id ASC LIMIT 500",
    )
    .bind(&locale)
    .bind(CATEGORY_INDEX_VERSION)
    .fetch_all(&mut *conn)
    .await?;
    for (id, name) in categories {
        // queue_category_posts_reindex
        sqlx::query(
            "UPDATE post_search_data SET version = 0 FROM posts \
             INNER JOIN topics ON posts.topic_id = topics.id \
             INNER JOIN categories ON topics.category_id = categories.id \
             WHERE post_search_data.post_id = posts.id AND categories.id = $1",
        )
        .bind(id)
        .execute(&mut *conn)
        .await?;
        search_index::index_category(&mut conn, &s, id, &name).await?;
    }

    // rebuild_tags(limit: 1000)
    let tags: Vec<(i32, String)> = sqlx::query_as(
        "SELECT tags.id, tags.name FROM tags \
         LEFT JOIN tag_search_data ON tag_id = tags.id \
         WHERE tag_search_data.locale IS NULL OR tag_search_data.locale != $1 \
            OR tag_search_data.version != $2 \
         ORDER BY tags.id ASC LIMIT 1000",
    )
    .bind(&locale)
    .bind(TAG_INDEX_VERSION)
    .fetch_all(&mut *conn)
    .await?;
    for (id, name) in tags {
        search_index::index_tag(&mut conn, &s, id, &name.to_lowercase()).await?;
    }

    // rebuild_topics(limit: 10000): through each topic's first post.
    let topics: Vec<i32> = sqlx::query_scalar(
        "SELECT topics.id FROM topics \
         LEFT JOIN topic_search_data ON topic_id = topics.id \
         WHERE topics.deleted_at IS NULL \
           AND (topic_search_data.locale IS NULL OR topic_search_data.locale != $1 \
                OR topic_search_data.version != $2) \
         ORDER BY topics.id DESC LIMIT 10000",
    )
    .bind(&locale)
    .bind(TOPIC_INDEX_VERSION)
    .fetch_all(&mut *conn)
    .await?;
    for topic_id in topics {
        let first: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL",
        )
        .bind(topic_id)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(post_id) = first {
            search_index::reindex_post(&mut conn, &s, &base, post_id).await?;
        }
    }

    // rebuild_posts(limit: 20000): those with raw text.
    let posts: Vec<(i32, String)> = sqlx::query_as(
        "SELECT posts.id, posts.raw FROM posts \
         JOIN topics ON topics.id = posts.topic_id \
         LEFT JOIN post_search_data ON post_search_data.post_id = posts.id \
         WHERE posts.deleted_at IS NULL AND posts.raw != '' AND topics.deleted_at IS NULL \
           AND (post_search_data.locale IS NULL OR post_search_data.locale != $1 \
                OR post_search_data.version != $2) \
         ORDER BY posts.id DESC LIMIT 20000",
    )
    .bind(&locale)
    .bind(POST_INDEX_VERSION)
    .fetch_all(&mut *conn)
    .await?;
    for (post_id, raw) in posts {
        // index(post, force: true) only when raw.present?
        if crate::ruby::is_blank(&raw) {
            continue;
        }
        search_index::reindex_post(&mut conn, &s, &base, post_id).await?;
    }

    // rebuild_users(limit: 5000)
    let users: Vec<(i32, String, Option<String>)> = sqlx::query_as(
        "SELECT users.id, users.username_lower, users.name FROM users \
         LEFT JOIN user_search_data ON user_id = users.id \
         WHERE user_search_data.locale IS NULL OR user_search_data.locale != $1 \
            OR user_search_data.version != $2 \
         ORDER BY users.id ASC LIMIT 5000",
    )
    .bind(&locale)
    .bind(USER_INDEX_VERSION)
    .fetch_all(&mut *conn)
    .await?;
    for (id, username_lower, name) in users {
        search_index::index_user(&mut conn, &s, id, &username_lower, name.as_deref()).await?;
    }

    // rebuild_registered_search_handlers: chat's messages.
    if crate::plugins::chat::enabled(&s)? && s.get("chat_search_enabled")?.truthy() {
        let messages: Vec<(i64, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT chat_messages.id, chat_messages.message, chat_messages.cooked FROM chat_messages \
             LEFT JOIN chat_message_search_data ON chat_message_id = chat_messages.id \
             WHERE chat_messages.deleted_at IS NULL \
               AND (chat_message_search_data.locale IS NULL OR chat_message_search_data.locale != $1 \
                    OR chat_message_search_data.version != $2) \
             ORDER BY chat_messages.id ASC LIMIT 10000",
        )
        .bind(&locale)
        .bind(CHAT_MESSAGE_INDEX_VERSION)
        .fetch_all(&mut *conn)
        .await?;
        for (id, message, cooked) in messages {
            search_index::index_chat_message(
                &mut conn,
                &s,
                &base,
                id,
                message.as_deref().unwrap_or_default(),
                cooked.as_deref().unwrap_or_default(),
            )
            .await?;
        }
    }

    // clean_topics, clean_posts: deleted a day ago or more. The inner
    // join on topics leaves rows whose post is gone, as in Rails.
    sqlx::query(
        "DELETE FROM topic_search_data WHERE topic_id IN ( \
           SELECT topic_id FROM topic_search_data LEFT JOIN topics ON topic_id = topics.id \
           WHERE topics.id IS NULL \
              OR (deleted_at IS NOT NULL AND deleted_at <= now() - interval '1 day'))",
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "DELETE FROM post_search_data WHERE post_id IN ( \
           SELECT post_id FROM post_search_data LEFT JOIN posts ON post_id = posts.id \
           JOIN topics ON posts.topic_id = topics.id \
           WHERE posts.id IS NULL OR posts.raw = '' \
              OR (posts.deleted_at IS NOT NULL AND posts.deleted_at <= now() - interval '1 day') \
              OR (topics.deleted_at IS NOT NULL AND topics.deleted_at <= now() - interval '1 day'))",
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Jobs::CreateRecentPostSearchIndexes`: on a site with more regular
/// posts indexed than search_enable_recent_regular_posts_offset_size, the
/// offset at the search_recent_posts_size'th newest one set, and the
/// partial index of the posts past it rebuilt (concurrently, then swapped
/// in).
pub async fn create_recent_post_search_indexes(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let enable_size = s
        .get("search_enable_recent_regular_posts_offset_size")?
        .to_i();
    let enough: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM post_search_data WHERE private_message = false \
           LIMIT 1 OFFSET $1)",
    )
    .bind((enable_size - 1).max(0))
    .fetch_one(&mut *conn)
    .await?;
    if !enough {
        return Ok(());
    }
    let offset_post_id: Option<i32> = sqlx::query_scalar(
        "SELECT post_id FROM post_search_data WHERE private_message = false \
         ORDER BY post_id DESC LIMIT 1 OFFSET $1",
    )
    .bind((s.get("search_recent_posts_size")?.to_i() - 1).max(0))
    .fetch_optional(&mut *conn)
    .await?;
    let post_id = i64::from(offset_post_id.unwrap_or(0));
    // SiteSetting.search_recent_regular_posts_offset_post_id = ...: the
    // plain setter (add_override!), hidden, so unlogged.
    let def = state
        .site_setting_defs
        .get("search_recent_regular_posts_offset_post_id")
        .ok_or(crate::Unsupported("the search offset setting's definition"))?;
    let current = s.get("search_recent_regular_posts_offset_post_id")?.clone();
    let target = crate::site_settings::Value::Int(post_id);
    crate::site_setting_update::set_and_log(&mut conn, &state.bus, def, &current, &target, -1)
        .await?;
    // The index swap, outside any transaction (CONCURRENTLY).
    for sql in [
        "DROP INDEX IF EXISTS temp_idx_recent_regular_post_search_data".to_string(),
        format!(
            "CREATE INDEX CONCURRENTLY temp_idx_recent_regular_post_search_data \
             ON post_search_data USING GIN(search_data) \
             WHERE NOT private_message AND post_id >= {post_id}"
        ),
    ] {
        sqlx::query(&sql).execute(&mut *conn).await?;
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("DROP INDEX IF EXISTS idx_recent_regular_post_search_data")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "ALTER INDEX temp_idx_recent_regular_post_search_data RENAME TO idx_recent_regular_post_search_data",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}
