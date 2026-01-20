use actix_web::{get, web, HttpResponse, Responder};
use diesel::prelude::*;
use diesel::sql_types::{Integer, Text, Timestamp};
use serde::{Deserialize, Serialize};

use crate::DbPool;

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    20
}

#[derive(Serialize, QueryableByName)]
pub struct TopicSearchResult {
    #[diesel(sql_type = Integer)]
    pub id: i32,
    #[diesel(sql_type = Text)]
    pub title: String,
    #[diesel(sql_type = Text)]
    pub slug: String,
    #[diesel(sql_type = Integer)]
    pub user_id: i32,
    #[diesel(sql_type = Text)]
    pub username: String,
    #[diesel(sql_type = Integer)]
    pub posts_count: i32,
    #[diesel(sql_type = Timestamp)]
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Serialize, QueryableByName)]
pub struct PostSearchResult {
    #[diesel(sql_type = Integer)]
    pub id: i32,
    #[diesel(sql_type = Integer)]
    pub topic_id: i32,
    #[diesel(sql_type = Text)]
    pub topic_title: String,
    #[diesel(sql_type = Integer)]
    pub post_number: i32,
    #[diesel(sql_type = Text)]
    pub raw: String,
    #[diesel(sql_type = Integer)]
    pub user_id: i32,
    #[diesel(sql_type = Text)]
    pub username: String,
    #[diesel(sql_type = Timestamp)]
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Serialize)]
pub struct SearchResults {
    pub topics: Vec<TopicSearchResult>,
    pub posts: Vec<PostSearchResult>,
    pub query: String,
}

#[get("/search")]
async fn search(pool: web::Data<DbPool>, query: web::Query<SearchQuery>) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let search_term = query.q.trim();
    if search_term.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Search query cannot be empty"
        }));
    }

    let limit = query.limit.min(100).max(1);

    // Search topics by title using PostgreSQL full-text search
    let topics: Vec<TopicSearchResult> = match diesel::sql_query(
        "SELECT t.id, t.title, t.slug, t.user_id, u.username, t.posts_count, t.created_at
         FROM topics t
         JOIN users u ON t.user_id = u.id
         WHERE to_tsvector('english', t.title) @@ plainto_tsquery('english', $1)
         ORDER BY ts_rank(to_tsvector('english', t.title), plainto_tsquery('english', $1)) DESC
         LIMIT $2"
    )
    .bind::<Text, _>(search_term)
    .bind::<diesel::sql_types::BigInt, _>(limit)
    .load(&mut conn)
    {
        Ok(results) => results,
        Err(e) => {
            log::error!("Topic search failed: {}", e);
            vec![]
        }
    };

    // Search posts by content using PostgreSQL full-text search
    let posts: Vec<PostSearchResult> = match diesel::sql_query(
        "SELECT p.id, p.topic_id, t.title as topic_title, p.post_number,
                LEFT(p.raw, 300) as raw, p.user_id, u.username, p.created_at
         FROM posts p
         JOIN topics t ON p.topic_id = t.id
         JOIN users u ON p.user_id = u.id
         WHERE p.deleted_at IS NULL
           AND p.hidden = false
           AND to_tsvector('english', p.raw) @@ plainto_tsquery('english', $1)
         ORDER BY ts_rank(to_tsvector('english', p.raw), plainto_tsquery('english', $1)) DESC
         LIMIT $2"
    )
    .bind::<Text, _>(search_term)
    .bind::<diesel::sql_types::BigInt, _>(limit)
    .load(&mut conn)
    {
        Ok(results) => results,
        Err(e) => {
            log::error!("Post search failed: {}", e);
            vec![]
        }
    };

    HttpResponse::Ok().json(SearchResults {
        topics,
        posts,
        query: search_term.to_string(),
    })
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(search);
}
