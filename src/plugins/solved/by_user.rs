//! discourse-solved's SolvedTopicsController#by_user: the posts of a user
//! that are accepted answers, newest acceptance first, as the viewer may
//! see them (DiscourseSolved::SolvedPostSerializer).

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::topic_list::time_json;
use crate::url::Urls;
use crate::{AppError, Unsupported};

#[derive(sqlx::FromRow)]
struct Row {
    id: i32,
    created_at: NaiveDateTime,
    post_number: i32,
    post_type: i32,
    user_id: Option<i32>,
    cooked: String,
    raw: String,
    topic_id: i32,
    title: String,
    fancy_title: Option<String>,
    archived: bool,
    closed: bool,
    category_id: Option<i32>,
    username: Option<String>,
    name: Option<String>,
    uploaded_avatar_id: Option<i32>,
}

/// The page of answers, `offset` and `limit` as given.
pub async fn by_user(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    urls: &Urls<'_>,
    guardian: &Guardian,
    user_id: i32,
    offset: i64,
    limit: i64,
) -> Result<Value, AppError> {
    let settings = ctx.settings;
    if settings.get("content_localization_enabled")?.truthy() {
        return Err(Unsupported("solved posts with content localization").into());
    }
    if settings.get("slug_generation_method")?.to_s() != "ascii" {
        return Err(Unsupported("solved posts' slugs other than ascii").into());
    }
    let locale = settings.get("default_locale")?.to_s();
    let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
    let include_unlisted = guardian.is_me(user_id) || guardian.can_see_unlisted_topics();
    let all_hidden = guardian.can_see_all_hidden_posts(settings)?;
    let visible_types = guardian.visible_post_types(settings)?;
    let mut sql = String::from(
        "SELECT posts.id, posts.created_at, posts.post_number, posts.post_type, posts.user_id, posts.cooked, \
                posts.raw, posts.topic_id, topics.title, topics.fancy_title, topics.archived, \
                topics.closed, topics.category_id, u.username, u.name, u.uploaded_avatar_id \
         FROM posts \
         INNER JOIN discourse_solved_topic_answers ON discourse_solved_topic_answers.answer_post_id = posts.id \
         INNER JOIN discourse_solved_solved_topics \
           ON discourse_solved_solved_topics.id = discourse_solved_topic_answers.solved_topic_id \
         INNER JOIN topics ON topics.id = posts.topic_id AND topics.deleted_at IS NULL \
         LEFT JOIN categories ON categories.id = topics.category_id \
         LEFT JOIN users u ON u.id = posts.user_id \
         WHERE posts.user_id = $1 AND posts.deleted_at IS NULL AND topics.archetype = 'regular' \
           AND (topics.category_id IS NULL OR NOT categories.read_restricted \
                OR topics.category_id = ANY($2))",
    );
    if !include_unlisted {
        sql.push_str(" AND topics.visible");
    }
    // filter_hidden_posts
    if !all_hidden {
        sql.push_str(" AND (posts.hidden = FALSE OR posts.user_id = $3)");
    }
    if !guardian.is_admin() {
        sql.push_str(" AND (posts.user_id = $4 OR posts.post_type = ANY($5))");
    }
    sql.push_str(" ORDER BY discourse_solved_topic_answers.created_at DESC OFFSET $6 LIMIT $7");
    let rows: Vec<Row> = sqlx::query_as(&sql)
        .bind(user_id)
        .bind(&secure)
        .bind(guardian.user_id())
        // current_user&.id || Discourse.system_user.id
        .bind(guardian.user_id().unwrap_or(crate::avatar::SYSTEM_USER_ID))
        .bind(&visible_types)
        .bind(offset)
        .bind(limit)
        .fetch_all(&mut *conn)
        .await?;

    let enable_names = settings.get("enable_names")?.truthy();
    let base_url = urls.base_url()?;
    let mut out = Vec::new();
    for r in rows {
        // posts.select { guardian.can_see_post?(post) }
        if crate::posting::revisions::find_post(&mut *conn, ctx, guardian, r.id)
            .await?
            .is_none()
        {
            continue;
        }
        let fancy = crate::topic_query::fancy_title(
            &mut *conn,
            settings,
            r.topic_id,
            &r.title,
            r.fancy_title.as_deref(),
        )
        .await?;
        // Slug.for(topic.title)
        let slug = crate::posting::text::slug_for(&r.title, &locale)?;
        let mut p = Map::new();
        // PostItemExcerpt: the excerpt, and truncated past 300 characters.
        p.insert(
            "excerpt".into(),
            json!(crate::excerpt::excerpt(
                &r.cooked,
                300,
                &crate::excerpt::Options {
                    keep_emoji_images: true,
                    ..Default::default()
                },
            )),
        );
        if r.cooked.chars().count() > 300 {
            p.insert("truncated".into(), json!(true));
        }
        p.insert("created_at".into(), json!(time_json(r.created_at)));
        p.insert("archived".into(), json!(r.archived));
        let avatar = match (r.user_id, r.username.as_deref()) {
            (Some(id), Some(username)) => Some(crate::avatar::avatar_template(
                urls,
                id,
                username,
                r.uploaded_avatar_id,
                None,
            )?),
            _ => None,
        };
        p.insert("avatar_template".into(), json!(avatar));
        p.insert("category_id".into(), json!(r.category_id));
        p.insert("closed".into(), json!(r.closed));
        p.insert("cooked".into(), json!(r.cooked));
        if enable_names {
            p.insert("name".into(), json!(r.name));
        }
        p.insert("post_id".into(), json!(r.id));
        p.insert("post_number".into(), json!(r.post_number));
        p.insert("post_type".into(), json!(r.post_type));
        p.insert("raw".into(), json!(r.raw));
        if !r.title.is_empty() {
            p.insert("slug".into(), json!(slug));
        }
        p.insert("topic_id".into(), json!(r.topic_id));
        p.insert("topic_title".into(), json!(fancy));
        p.insert(
            "url".into(),
            json!(format!(
                "{base_url}/t/{slug}/{}/{}",
                r.topic_id, r.post_number
            )),
        );
        p.insert("user_id".into(), json!(r.user_id));
        p.insert("username".into(), json!(r.username));
        out.push(Value::Object(p));
    }
    Ok(json!({ "user_solved_posts": out }))
}
