//! ScoreCalculator: each post's score from its replies, likes, links,
//! bookmarks and reads, its rank within the topic, and each topic's
//! average score and whether it has a summary. Jobs::CalculateScores runs
//! it over everything weekly; periodical updates over recent topics.

use chrono::NaiveDateTime;
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `Jobs::CalculateScores`, weekly.
pub const CALCULATE_SCORES: &str = "calculate_scores";

/// `ScoreCalculator.default_score_weights`, in their order. Inlined into
/// the SQL as literals, as mini_sql inlines them: `0.2` is a numeric, so
/// the score is computed exactly before it is stored as a float.
const WEIGHTS: [(&str, &str); 5] = [
    ("reply_count", "5"),
    ("like_score", "15"),
    ("incoming_link_count", "5"),
    ("bookmark_count", "2"),
    ("reads", "0.2"),
];

/// The rows each batched update takes.
const BATCH: u64 = 20_000;

/// `calculate`'s options: topics bumped since, and shorter than.
#[derive(Default, Clone, Copy)]
pub struct Filters {
    pub bumped_since: Option<NaiveDateTime>,
    pub max_topic_length: Option<i64>,
}

impl Filters {
    /// `filter_topics`, as SQL with the values inlined.
    fn sql(&self) -> String {
        let mut out = String::new();
        if let Some(at) = self.bumped_since {
            out.push_str(&format!(
                " AND (topics.bumped_at > '{}' )",
                at.format("%Y-%m-%d %H:%M:%S%.6f")
            ));
        }
        if let Some(max) = self.max_topic_length {
            out.push_str(&format!(" AND (topics.posts_count < {max})"));
        }
        out
    }
}

/// `ScoreCalculator#calculate`
pub async fn calculate(
    conn: &mut PgConnection,
    s: &SiteSettings,
    filters: Filters,
) -> Result<(), AppError> {
    let filter = filters.sql();
    let expression = WEIGHTS
        .iter()
        .map(|(column, weight)| format!("COALESCE(posts.{column}, 0) * {weight}"))
        .collect::<Vec<_>>()
        .join(" + ");
    // update_posts_score
    let scores = format!(
        "UPDATE posts p SET score = x.score FROM ( \
           SELECT posts.id, {expression} as score FROM posts \
           JOIN topics ON posts.topic_id = topics.id \
           WHERE (posts.score IS NULL OR posts.score <> {expression}){filter} \
           LIMIT {BATCH}) AS x \
         WHERE x.id = p.id"
    );
    while sqlx::query(&scores)
        .execute(&mut *conn)
        .await?
        .rows_affected()
        == BATCH
    {}
    // update_posts_rank
    let ranks = format!(
        "UPDATE posts SET percent_rank = X.percent_rank FROM ( \
           SELECT posts.id, Y.percent_rank FROM posts \
           JOIN (SELECT id, percent_rank() OVER (PARTITION BY topic_id ORDER BY SCORE DESC) as percent_rank \
                 FROM posts) Y ON Y.id = posts.id \
           JOIN topics ON posts.topic_id = topics.id \
           WHERE (posts.percent_rank IS NULL OR Y.percent_rank <> posts.percent_rank){filter} \
           LIMIT {BATCH}) AS X \
         WHERE posts.id = X.id"
    );
    while sqlx::query(&ranks)
        .execute(&mut *conn)
        .await?
        .rows_affected()
        == BATCH
    {}
    // update_topics_rank
    let likes = s.get("summary_likes_required")?.to_i();
    let posts = s.get("summary_posts_required")?.to_i();
    let score = s.get("summary_score_threshold")?.to_i();
    let summary = format!(
        "topics.like_count >= {likes} AND topics.posts_count >= {posts} AND x.max_score >= {score}"
    );
    sqlx::query(&format!(
        "UPDATE topics AS topics SET has_summary = ({summary}), score = x.avg_score \
         FROM (SELECT p.topic_id, MAX(p.score) AS max_score, AVG(p.score) AS avg_score \
               FROM posts AS p GROUP BY p.topic_id) AS x \
         WHERE (x.topic_id = topics.id AND ( \
           (topics.score <> x.avg_score OR topics.score IS NULL) OR \
           (topics.has_summary IS NULL OR topics.has_summary <> ({summary})))){filter}"
    ))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Jobs::CalculateScores`: every post and topic, without filters.
pub async fn calculate_scores(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    calculate(&mut conn, &s, Filters::default()).await
}
