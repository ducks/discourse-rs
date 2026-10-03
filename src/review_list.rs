//! The review queue: ReviewablesController#index as `Reviewable.list_for`
//! pages it, ReviewableFlaggedPostSerializer (with ReviewableSerializer)
//! and the records it side-loads (users, topics, scores, score types,
//! bundled actions and their actions, notes, histories, claims), and the
//! meta the controller adds.
//!
//! Refused: viewers other than admins (moderators and category group
//! moderators go through the post visibility scopes), reviewable types
//! other than flagged posts on a page, the `ids`, date and additional
//! filters, claimed topics, notes, score reasons and conversations,
//! authors under a penalty, posts the user deleted, potential spam and
//! illegal content (the delete user actions), system users, and content
//! localization.

use std::collections::HashSet;

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::params;
use crate::posting::Ctx;
use crate::topic_list::{Mode, TopicListSerializer, time_json};
use crate::topic_query::{TOPIC_COLUMNS, TopicRow};
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `ReviewablesController::PER_PAGE`
const PER_PAGE: i64 = 10;

/// `Reviewable.statuses`
const STATUSES: [(&str, i32); 5] = [
    ("pending", 0),
    ("approved", 1),
    ("rejected", 2),
    ("ignored", 3),
    ("deleted", 4),
];

/// `ReviewableHistory.types[:transitioned]`
const HISTORY_TRANSITIONED: i32 = 1;

/// `UserHistory.actions`
const SUSPEND_USER: i32 = 10;
const SILENCE_USER: i32 = 30;

/// `ReviewableQueuedPost.statuses[:rejected]`
const REJECTED: i32 = 2;

/// `User::MAX_STAFF_DELETE_POST_COUNT`
const MAX_STAFF_DELETE_POST_COUNT: i64 = 5;

/// Staff user custom fields plugins on the reference allow
/// (`allow_staff_user_custom_field`: discourse-events, discourse-user-notes).
const PLUGIN_STAFF_USER_CUSTOM_FIELDS: [&str; 2] = ["on_holiday", "user_notes_count"];

/// The filters the controller echoes in `meta`, in its order.
const ECHOED: [&str; 10] = [
    "priority",
    "username",
    "reviewed_by",
    "claimed_by",
    "from_date",
    "to_date",
    "type",
    "sort_order",
    "flagged_by",
    "score_type",
];

/// How the request ends when it is not a page.
pub enum Listed {
    Page(Value),
    /// `Discourse::InvalidParameters` for the named param.
    InvalidParameter(&'static str),
}

/// A bound value for the list query.
enum Bind {
    Int(i64),
    Float(f64),
    Text(String),
}

/// `Reviewable.list_for`'s WHERE clause and joins, with its binds.
#[derive(Default)]
struct Query {
    joins: String,
    wheres: Vec<String>,
    binds: Vec<Bind>,
    /// A filter named a user that does not exist: `none`.
    none: bool,
}

impl Query {
    fn bind(&mut self, value: Bind) -> String {
        self.binds.push(value);
        format!("${}", self.binds.len())
    }

    fn sql(&self, select: &str, tail: &str) -> String {
        let wheres = if self.wheres.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", self.wheres.join(" AND "))
        };
        format!(
            "SELECT {select} FROM reviewables {} {wheres} {tail}",
            self.joins
        )
    }

    fn query<'q>(
        &'q self,
        sql: &'q str,
    ) -> sqlx::query::QueryScalar<'q, sqlx::Postgres, i64, sqlx::postgres::PgArguments> {
        let mut q = sqlx::query_scalar(sql);
        for b in &self.binds {
            q = match b {
                Bind::Int(v) => q.bind(*v),
                Bind::Float(v) => q.bind(*v),
                Bind::Text(v) => q.bind(v.clone()),
            };
        }
        q
    }
}

async fn user_id_by_username(
    conn: &mut PgConnection,
    username: &str,
) -> Result<Option<i32>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1")
        .bind(username.to_lowercase())
        .fetch_optional(conn)
        .await
}

/// `Reviewable.min_score_for_priority(priority)`
async fn min_score(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    priority: Option<&str>,
) -> Result<f64, AppError> {
    let priority = match priority.filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => ctx.settings.get("reviewable_default_visibility")?.to_s(),
    };
    let id = match priority.as_str() {
        "low" => 0,
        "medium" => 5,
        "high" => 10,
        _ => return Ok(0.0),
    };
    let value: Option<Option<String>> = sqlx::query_scalar(
        "SELECT value FROM plugin_store_rows WHERE plugin_name = 'reviewables' AND key = $1",
    )
    .bind(format!("priority_{id}"))
    .fetch_optional(conn)
    .await?;
    Ok(value
        .flatten()
        .map(|v| crate::ruby::to_f(&v))
        .unwrap_or(0.0))
}

/// ReviewablesController#index for an admin.
pub async fn index(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    p: &Map<String, Value>,
) -> Result<Listed, AppError> {
    let s = ctx.settings;
    let Some(user) = guardian.user() else {
        return Err(Unsupported("the review queue for anonymous users").into());
    };
    if !user.admin {
        return Err(Unsupported("the review queue for moderators").into());
    }
    if s.get("content_localization_enabled")?.truthy() {
        return Err(Unsupported("content localization in the review queue").into());
    }
    let text = |k: &str| p.get(k).and_then(params::scalar).filter(|v| !v.is_empty());
    let offset = text("offset").map(|v| crate::ruby::to_i(&v)).unwrap_or(0);

    if let Some(t) = text("type")
        && !crate::reviewables::STI_NAMES.contains(&t.as_str())
    {
        return Ok(Listed::InvalidParameter("type"));
    }
    let status = p
        .get("status")
        .and_then(params::scalar)
        .unwrap_or_else(|| "pending".into());
    let status_ids: Option<Vec<i32>> = match status.as_str() {
        "all" => None,
        "reviewed" => Some(STATUSES[1..].iter().map(|(_, i)| *i).collect()),
        other => match STATUSES.iter().find(|(n, _)| *n == other) {
            Some((_, i)) => Some(vec![*i]),
            None => return Ok(Listed::InvalidParameter("status")),
        },
    };
    if p.contains_key("ids") {
        return Err(Unsupported("the review queue's ids filter").into());
    }
    if text("from_date").is_some() || text("to_date").is_some() {
        return Err(Unsupported("the review queue's date filters").into());
    }
    if let Some(filters) = text("additional_filters")
        && serde_json::from_str::<Value>(&filters)
            .ok()
            .and_then(|v| v.as_object().map(|m| !m.is_empty()))
            .unwrap_or(true)
    {
        return Err(Unsupported("the review queue's additional filters").into());
    }
    let topic_id = p
        .get("topic_id")
        .and_then(params::scalar)
        .map(|v| crate::ruby::to_i(&v));
    let category_id = p
        .get("category_id")
        .and_then(params::scalar)
        .map(|v| crate::ruby::to_i(&v));

    let mut q = Query::default();
    let order = match text("sort_order").as_deref() {
        Some("score_asc") => "reviewables.score ASC, reviewables.created_at DESC",
        Some("created_at") => "reviewables.created_at DESC, reviewables.score DESC",
        Some("created_at_asc") => "reviewables.created_at ASC, reviewables.score DESC",
        _ => "reviewables.score DESC, reviewables.created_at DESC",
    };
    let username_id = match text("username") {
        Some(u) => match user_id_by_username(&mut *conn, &u).await? {
            Some(id) => Some(id),
            None => {
                q.none = true;
                None
            }
        },
        None => None,
    };
    if let Some(ids) = &status_ids {
        let list = ids
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        q.wheres.push(format!("reviewables.status IN ({list})"));
    }
    if let Some(t) = text("type") {
        let b = q.bind(Bind::Text(t));
        q.wheres.push(format!("reviewables.type = {b}"));
    }
    if let Some(id) = category_id {
        let b = q.bind(Bind::Int(id));
        q.wheres.push(format!("reviewables.category_id = {b}"));
    }
    if let Some(id) = topic_id {
        let b = q.bind(Bind::Int(id));
        q.wheres.push(format!("reviewables.topic_id = {b}"));
    }
    if let Some(name) = text("flagged_by") {
        match user_id_by_username(&mut *conn, &name).await? {
            Some(id) => {
                let b = q.bind(Bind::Int(id.into()));
                q.wheres.push(format!(
                    "EXISTS (SELECT 1 FROM reviewable_scores WHERE reviewable_scores.reviewable_id = reviewables.id \
                     AND reviewable_scores.user_id = {b})"
                ));
            }
            None => q.none = true,
        }
    }
    if let Some(score_type) = text("score_type") {
        let b = q.bind(Bind::Int(crate::ruby::to_i(&score_type)));
        q.wheres.push(format!(
            "EXISTS (SELECT 1 FROM reviewable_scores WHERE reviewable_scores.reviewable_id = reviewables.id \
             AND reviewable_scores.reviewable_score_type = {b})"
        ));
    }
    if let Some(name) = text("reviewed_by") {
        match user_id_by_username(&mut *conn, &name).await? {
            Some(id) => q.joins.push_str(&format!(
                " INNER JOIN (SELECT reviewable_id FROM reviewable_histories \
                   WHERE reviewable_history_type = {HISTORY_TRANSITIONED} AND status <> 0 \
                   AND created_by_id = {id}) AS rh ON rh.reviewable_id = reviewables.id"
            )),
            None => q.none = true,
        }
    }
    if let Some(name) = text("claimed_by") {
        match user_id_by_username(&mut *conn, &name).await? {
            Some(id) => {
                q.joins.push_str(
                    " INNER JOIN reviewable_claimed_topics rct_filter \
                      ON rct_filter.topic_id = reviewables.topic_id",
                );
                q.wheres.push(format!("rct_filter.user_id = {id}"));
            }
            None => q.none = true,
        }
    }
    let min = min_score(&mut *conn, ctx, text("priority").as_deref()).await?;
    if min > 0.0 {
        let b = q.bind(Bind::Float(min));
        if status == "pending" {
            q.wheres.push(format!(
                "(reviewables.score >= {b} OR reviewables.force_review)"
            ));
        } else {
            q.wheres.push(format!("reviewables.score >= {b}"));
        }
    }
    if let Some(id) = username_id {
        q.wheres.push(format!(
            "((reviewables.target_id IS NULL AND reviewables.created_by_id = {id}) \
             OR (reviewables.target_created_by_id = {id}))"
        ));
    }
    let types = crate::reviewables::STI_NAMES
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(", ");
    q.wheres.push(format!("reviewables.type IN ({types})"));

    let (total, ids): (i64, Vec<i64>) = if q.none {
        (0, Vec::new())
    } else {
        let count_sql = q.sql("COUNT(*)", "");
        let total = q.query(&count_sql).fetch_one(&mut *conn).await?;
        let page_sql = q.sql(
            "reviewables.id",
            &format!("ORDER BY {order} LIMIT {PER_PAGE} OFFSET {}", offset.max(0)),
        );
        let ids = q.query(&page_sql).fetch_all(&mut *conn).await?;
        (total, ids)
    };

    let mut out = Out::default();
    let mut reviewables = Vec::new();
    for id in ids {
        reviewables.push(serialize(&mut *conn, ctx, guardian, id, &mut out).await?);
    }

    // meta: the filters as the controller holds them, then the rest.
    let mut meta = Map::new();
    meta.insert("ids".into(), Value::Null);
    meta.insert("status".into(), json!(status));
    meta.insert("category_id".into(), json!(category_id));
    meta.insert("topic_id".into(), json!(topic_id));
    meta.insert("additional_filters".into(), json!({}));
    for key in ECHOED {
        meta.insert(key.into(), p.get(key).cloned().unwrap_or(Value::Null));
    }
    meta.insert("total_rows_reviewables".into(), json!(total));
    meta.insert(
        "types".into(),
        json!({
            "created_by": "user",
            "target_created_by": "user",
            "target_deleted_by": "user",
            "reviewed_by": "user",
            "claimed_by": "claimed_by",
        }),
    );
    meta.insert(
        "reviewable_types".into(),
        json!(crate::reviewables::STI_NAMES),
    );
    meta.insert("unknown_reviewable_types_and_sources".into(), json!([]));
    meta.insert(
        "score_types".into(),
        meta_score_types(&mut *conn, ctx).await?,
    );
    let (count, unseen) =
        crate::reviewables::staff_counts(&mut *conn, s, user.id, user.admin, user.moderator)
            .await?;
    meta.insert("reviewable_count".into(), json!(count));
    meta.insert("unseen_reviewable_count".into(), json!(unseen));
    if offset + PER_PAGE < total {
        meta.insert(
            "load_more_reviewables".into(),
            json!(load_more_path(ctx, &meta, offset + PER_PAGE)),
        );
    }

    // A root appears once a serializer adds to it: the reviewable's own
    // associations with any reviewable, score types with a score, actions
    // with a bundle.
    let any = !reviewables.is_empty();
    let mut doc = Map::new();
    doc.insert("reviewables".into(), Value::Array(reviewables));
    doc.insert("meta".into(), Value::Object(meta));
    if any {
        doc.insert("users".into(), Value::Array(out.users));
        doc.insert("topics".into(), Value::Array(out.topics));
        doc.insert("reviewable_scores".into(), Value::Array(out.scores));
        if !out.score_types.is_empty() {
            doc.insert("score_types".into(), Value::Array(out.score_types));
        }
        let bundled = !out.bundles.is_empty();
        doc.insert("bundled_actions".into(), Value::Array(out.bundles));
        if bundled {
            doc.insert("actions".into(), Value::Array(out.actions));
        }
        doc.insert("reviewable_notes".into(), json!([]));
        doc.insert("reviewable_histories".into(), Value::Array(out.histories));
        doc.insert("claimed_bies".into(), json!([]));
    }
    doc.insert("__rest_serializer".into(), json!("1"));
    Ok(Listed::Page(Value::Object(doc)))
}

/// `review_path(filters.merge(offset:))`: the filters that hold a value,
/// sorted as `to_query` sorts them.
fn load_more_path(ctx: &Ctx<'_>, meta: &Map<String, Value>, offset: i64) -> String {
    let mut pairs: Vec<(String, String)> = vec![("offset".into(), offset.to_string())];
    for key in ["ids", "status", "category_id", "topic_id"]
        .into_iter()
        .chain(ECHOED)
    {
        if let Some(v) = meta.get(key).and_then(params::scalar) {
            pairs.push((key.to_string(), v));
        }
    }
    pairs.sort();
    let query = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    format!("{}/review?{query}", ctx.config.globals.relative_url_root())
}

/// meta `score_types`: every score type but notify_user, with its title.
async fn meta_score_types(conn: &mut PgConnection, ctx: &Ctx<'_>) -> Result<Value, AppError> {
    let types = score_types(conn).await?;
    Ok(Value::Array(
        types
            .iter()
            .filter(|t| t.key != "notify_user")
            .map(|t| json!({ "id": t.id, "name": type_title(ctx, t) }))
            .collect(),
    ))
}

struct ScoreType {
    id: i64,
    key: String,
    name: String,
}

/// `ReviewableScore.types`: the flag types by position, then the score
/// types.
async fn score_types(conn: &mut PgConnection) -> Result<Vec<ScoreType>, sqlx::Error> {
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT id::bigint, name_key, name FROM flags WHERE id <> 2 ORDER BY score_type, position",
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, key, name)| ScoreType { id, key, name })
        .collect())
}

/// `ReviewableScore.type_title(type)`
fn type_title(ctx: &Ctx<'_>, t: &ScoreType) -> String {
    ctx.i18n
        .t(&format!("post_action_types.{}.title", t.key))
        .or_else(|| {
            ctx.i18n
                .t(&format!("reviewable_score_types.{}.title", t.key))
        })
        .map(str::to_string)
        .unwrap_or_else(|| t.name.clone())
}

/// What a page side-loads, in the order the serializers add it.
#[derive(Default)]
struct Out {
    users: Vec<Value>,
    topics: Vec<Value>,
    scores: Vec<Value>,
    score_types: Vec<Value>,
    bundles: Vec<Value>,
    actions: Vec<Value>,
    histories: Vec<Value>,
}

/// One reviewable's side-loads: each user and topic once (AMS's
/// `unique_values` hold per serializer, so across reviewables they repeat).
#[derive(Default)]
struct Seen {
    users: HashSet<i32>,
    topics: HashSet<i32>,
}

#[derive(sqlx::FromRow)]
struct Reviewable {
    id: i64,
    #[sqlx(rename = "type")]
    kind: String,
    type_source: Option<String>,
    status: i32,
    created_by_id: i32,
    category_id: Option<i32>,
    topic_id: Option<i32>,
    score: f64,
    potential_spam: bool,
    potentially_illegal: Option<bool>,
    target_id: Option<i32>,
    target_type: Option<String>,
    target_created_by_id: Option<i32>,
    version: i32,
    created_at: NaiveDateTime,
}

#[derive(sqlx::FromRow)]
struct Post {
    id: i32,
    topic_id: i32,
    post_number: i32,
    cooked: String,
    raw: String,
    reply_count: i32,
    reply_to_post_number: Option<i32>,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
    deleted_at: Option<NaiveDateTime>,
    deleted_by_id: Option<i32>,
    user_deleted: bool,
    hidden: bool,
    version: i32,
}

/// ReviewableFlaggedPostSerializer for one reviewable, its side-loads
/// added to `out`.
async fn serialize(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    id: i64,
    out: &mut Out,
) -> Result<Value, AppError> {
    let s = ctx.settings;
    let urls = Urls {
        config: ctx.config,
        settings: s,
    };
    let r: Reviewable = sqlx::query_as(
        "SELECT id, type, type_source, status, created_by_id, category_id, topic_id, score, potential_spam, \
                potentially_illegal, target_id, target_type, target_created_by_id, version, created_at \
         FROM reviewables WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    if r.kind != "ReviewableFlaggedPost" {
        return Err(Unsupported("reviewables other than flagged posts in the queue").into());
    }
    if r.potential_spam || r.potentially_illegal == Some(true) {
        return Err(Unsupported("potential spam and illegal content in the queue").into());
    }
    let claimed: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM reviewable_claimed_topics WHERE topic_id = $1)",
    )
    .bind(r.topic_id)
    .fetch_one(&mut *conn)
    .await?;
    let notes: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM reviewable_notes WHERE reviewable_id = $1)",
    )
    .bind(r.id)
    .fetch_one(&mut *conn)
    .await?;
    if claimed || notes {
        return Err(Unsupported("claimed topics and notes in the queue").into());
    }
    let post: Option<Post> = match (r.target_type.as_deref(), r.target_id) {
        (Some("Post"), Some(target)) => {
            sqlx::query_as(
                "SELECT id, topic_id, post_number, cooked, raw, reply_count, reply_to_post_number, created_at, \
                        updated_at, deleted_at, deleted_by_id, user_deleted, hidden, version \
                 FROM posts WHERE id = $1",
            )
            .bind(target)
            .fetch_optional(&mut *conn)
            .await?
        }
        _ => None,
    };
    if post.as_ref().is_some_and(|p| p.user_deleted) {
        return Err(Unsupported("posts the user deleted in the queue").into());
    }
    let topic: Option<TopicRow> = match r.topic_id {
        Some(tid) => {
            sqlx::query_as(&format!("SELECT {TOPIC_COLUMNS} FROM topics WHERE id = $1"))
                .bind(tid)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    let author = match r.target_created_by_id {
        Some(uid) => Some(load_user(&mut *conn, uid).await?),
        None => None,
    };
    if author
        .as_ref()
        .is_some_and(|a| a.silenced() || a.suspended())
    {
        return Err(Unsupported("authors under a penalty (author_penalties)").into());
    }

    let mut seen = Seen::default();
    let mut d = Map::new();
    d.insert("id".into(), json!(r.id));
    d.insert("type".into(), json!(r.kind));
    d.insert("type_source".into(), json!(r.type_source));
    if let Some(tid) = r.topic_id {
        d.insert("topic_id".into(), json!(tid));
    }
    let topic_url = match &topic {
        Some(t) => {
            let Some(slug) = &t.slug else {
                return Err(Unsupported("topics without a stored slug (Slug.for)").into());
            };
            Some(format!("{}/t/{slug}/{}", urls.base_url()?, t.id))
        }
        None => None,
    };
    if let Some(u) = &topic_url {
        d.insert("topic_url".into(), json!(u));
    }
    d.insert("target_type".into(), json!(r.target_type));
    d.insert("target_id".into(), json!(r.target_id));
    // target_url: `Discourse.base_url + post.url`, else the topic's URL.
    // Deleted content is loaded (`with_deleted_content`).
    let target_url = match (&post, &topic) {
        (Some(p), Some(t)) => {
            let slug = t.slug.as_deref().unwrap_or_default();
            Some(format!(
                "{}/t/{slug}/{}/{}",
                urls.base_url()?,
                p.topic_id,
                p.post_number
            ))
        }
        _ => topic_url.clone(),
    };
    if let Some(u) = &target_url {
        d.insert("target_url".into(), json!(u));
    }
    if r.target_type.as_deref() == Some("Post") {
        d.insert(
            "target_created_at".into(),
            json!(post.as_ref().map(|p| time_json(p.created_at))),
        );
    }
    // target_deleted_by / target_deleted_at (posts the user deleted are
    // refused above).
    let deleted_by = post.as_ref().and_then(|p| p.deleted_by_id);
    if let (Some(p), Some(_)) = (&post, deleted_by)
        && let Some(at) = p.deleted_at
    {
        d.insert("target_deleted_at".into(), json!(time_json(at)));
    }
    if topic.is_some() && s.get("tagging_enabled")?.truthy() {
        let tags: Vec<(i32, String, Option<String>)> = sqlx::query_as(
            "SELECT tags.id, tags.name, tags.slug FROM topic_tags JOIN tags ON tags.id = topic_tags.tag_id \
             WHERE topic_tags.topic_id = $1 ORDER BY topic_tags.id",
        )
        .bind(r.topic_id)
        .fetch_all(&mut *conn)
        .await?;
        d.insert(
            "topic_tags".into(),
            Value::Array(
                tags.into_iter()
                    .map(|(id, name, slug)| json!({ "id": id, "name": name, "slug": slug }))
                    .collect(),
            ),
        );
    }
    if let Some(cid) = r.category_id {
        d.insert("category_id".into(), json!(cid));
    }
    d.insert("created_at".into(), json!(time_json(r.created_at)));
    // editable_fields: a flagged post has none.
    d.insert("can_edit".into(), json!(false));
    d.insert("score".into(), json!(r.score));
    d.insert("version".into(), json!(r.version));
    d.insert(
        "target_created_by_trust_level".into(),
        json!(author.as_ref().map(|a| a.trust_level)),
    );
    d.insert("created_from_flag".into(), json!(true));
    d.insert("status".into(), json!(r.status));
    match &post {
        Some(p) => {
            for (key, value) in [
                ("cooked", json!(p.cooked)),
                ("raw", json!(p.raw)),
                ("reply_count", json!(p.reply_count)),
            ] {
                if !value.as_str().is_some_and(|v| v.trim().is_empty()) {
                    d.insert(key.into(), value);
                }
            }
            if let Some(n) = p.reply_to_post_number {
                d.insert("reply_to_post_number".into(), json!(n));
            }
            if let Some(at) = p.deleted_at {
                d.insert("deleted_at".into(), json!(time_json(at)));
            }
            d.insert("post_updated_at".into(), json!(time_json(p.updated_at)));
            d.insert("post_version".into(), json!(p.version));
            d.insert("post_id".into(), json!(p.id));
        }
        None => {
            d.insert("post_updated_at".into(), Value::Null);
            d.insert("post_version".into(), Value::Null);
            d.insert("blank_post".into(), json!(true));
        }
    }
    if topic.is_none() {
        d.insert("removed_topic_id".into(), json!(r.topic_id));
    }

    // has_one :target_created_by (FlaggedUserSerializer)
    d.insert("target_created_by_id".into(), json!(r.target_created_by_id));
    if let Some(a) = &author
        && seen.users.insert(a.id)
    {
        out.users
            .push(flagged_user(&mut *conn, ctx, guardian, &urls, a).await?);
    }
    // has_one :created_by (UserWithCustomFieldsSerializer)
    d.insert("created_by_id".into(), json!(r.created_by_id));
    if seen.users.insert(r.created_by_id) {
        let u = load_user(&mut *conn, r.created_by_id).await?;
        let mut v = basic_user(ctx, &urls, &u)?;
        v["custom_fields"] = custom_fields(&mut *conn, ctx, guardian, u.id).await?;
        out.users.push(v);
    }
    // has_one :target_deleted_by (BasicUserSerializer)
    if let Some(uid) = deleted_by {
        d.insert("target_deleted_by_id".into(), json!(uid));
        if seen.users.insert(uid) {
            let u = load_user(&mut *conn, uid).await?;
            out.users.push(basic_user(ctx, &urls, &u)?);
        }
    }
    // has_one :topic (ListableTopicSerializer)
    if let Some(t) = &topic
        && seen.topics.insert(t.id)
    {
        let mut ser = TopicListSerializer {
            conn: &mut *conn,
            settings: s,
            i18n: ctx.i18n,
            guardian,
            urls: &urls,
            more_topics_url: None,
            category_id: None,
            group_id: None,
            prefetched: Default::default(),
        };
        out.topics
            .push(ser.serialize_topic(t, &[], false, Mode::Reviewable).await?);
    }
    d.insert("editable_fields".into(), json!([]));

    // has_many :reviewable_scores, newest first
    let score_ids = scores(&mut *conn, ctx, &urls, &r, &mut seen, out).await?;
    d.insert("reviewable_score_ids".into(), json!(score_ids));

    // has_many :bundled_actions
    let bundle_ids = match &post {
        Some(p) if r.status == crate::reviewables::PENDING => {
            actions(&mut *conn, ctx, guardian, &r, p, author.as_ref(), out).await?
        }
        _ => Vec::new(),
    };
    d.insert("bundled_action_ids".into(), json!(bundle_ids));
    d.insert("reviewable_note_ids".into(), json!([]));

    // has_many :reviewable_histories
    let histories: Vec<(i64, NaiveDateTime, i32, i32, i32)> = sqlx::query_as(
        "SELECT id, created_at, reviewable_history_type, status, created_by_id \
         FROM reviewable_histories WHERE reviewable_id = $1 ORDER BY id",
    )
    .bind(r.id)
    .fetch_all(&mut *conn)
    .await?;
    let mut history_ids = Vec::new();
    for (hid, at, kind, status, by) in histories {
        if seen.users.insert(by) {
            let u = load_user(&mut *conn, by).await?;
            out.users.push(basic_user(ctx, &urls, &u)?);
        }
        out.histories.push(json!({
            "id": hid,
            "created_at": time_json(at),
            "reviewable_history_type": kind,
            "status": status,
            "created_by_id": by,
        }));
        history_ids.push(hid);
    }
    d.insert("reviewable_history_ids".into(), json!(history_ids));
    d.insert("author_penalties".into(), json!([]));
    d.insert("claimed_by_id".into(), Value::Null);
    Ok(Value::Object(d))
}

/// The reviewable's scores (ReviewableScoreSerializer), newest first; their
/// ids.
async fn scores(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    urls: &Urls<'_>,
    r: &Reviewable,
    seen: &mut Seen,
    out: &mut Out,
) -> Result<Vec<i64>, AppError> {
    #[derive(sqlx::FromRow)]
    struct Score {
        id: i64,
        user_id: i32,
        reviewable_score_type: i32,
        status: i32,
        score: f64,
        reviewed_by_id: Option<i32>,
        reviewed_at: Option<NaiveDateTime>,
        meta_topic_id: Option<i32>,
        created_at: NaiveDateTime,
        reason: Option<String>,
    }
    let rows: Vec<Score> = sqlx::query_as(
        "SELECT id, user_id, reviewable_score_type, status, score, reviewed_by_id, reviewed_at, meta_topic_id, \
                created_at, reason \
         FROM reviewable_scores WHERE reviewable_id = $1 ORDER BY created_at DESC",
    )
    .bind(r.id)
    .fetch_all(&mut *conn)
    .await?;
    let types = score_types(&mut *conn).await?;
    let mut ids = Vec::new();
    for sc in rows {
        if sc.reason.is_some() || sc.meta_topic_id.is_some() {
            return Err(Unsupported("score reasons and conversations in the queue").into());
        }
        let stats: (i32, i32, i32) = sqlx::query_as(
            "SELECT flags_agreed, flags_disagreed, flags_ignored FROM user_stats WHERE user_id = $1",
        )
        .bind(sc.user_id)
        .fetch_one(&mut *conn)
        .await?;
        out.scores.push(json!({
            "id": sc.id,
            "score": sc.score,
            "agree_stats": { "agreed": stats.0, "disagreed": stats.1, "ignored": stats.2 },
            "reason_type": null,
            "reason_data": null,
            "created_at": time_json(sc.created_at),
            "reviewed_at": sc.reviewed_at.map(time_json),
            "status": sc.status,
            "user_id": sc.user_id,
            "score_type_id": sc.reviewable_score_type,
            "reviewed_by_id": sc.reviewed_by_id,
        }));
        // has_one :user, :score_type, :reviewed_by in that order.
        if seen.users.insert(sc.user_id) {
            let u = load_user(&mut *conn, sc.user_id).await?;
            out.users.push(basic_user(ctx, urls, &u)?);
        }
        let Some(t) = types
            .iter()
            .find(|t| t.id == i64::from(sc.reviewable_score_type))
        else {
            return Err(Unsupported("score types not in the flags table").into());
        };
        out.score_types.push(json!({
            "id": t.id,
            "title": type_title(ctx, t),
            "icon": "flag",
            "type": t.key,
        }));
        if let Some(by) = sc.reviewed_by_id
            && seen.users.insert(by)
        {
            let u = load_user(&mut *conn, by).await?;
            out.users.push(basic_user(ctx, urls, &u)?);
        }
        ids.push(sc.id);
    }
    Ok(ids)
}

/// A bundle being built: its id, icon, label key, secondary flag and the
/// actions' ids.
struct Bundle {
    id: String,
    icon: Option<&'static str>,
    label: Option<&'static str>,
    secondary: bool,
    actions: Vec<String>,
}

/// `Reviewable::Actions` for a flagged post.
struct Actions<'a> {
    ctx: &'a Ctx<'a>,
    reviewable_id: i64,
    bundles: Vec<Bundle>,
    actions: Vec<Value>,
}

impl Actions<'_> {
    fn add_bundle(&mut self, suffix: &str, icon: &'static str, label: &'static str) -> usize {
        self.bundles.push(Bundle {
            id: format!("{}-{suffix}", self.reviewable_id),
            icon: Some(icon),
            label: Some(label),
            secondary: false,
            actions: Vec::new(),
        });
        self.bundles.len() - 1
    }

    /// `build_action` for a core reviewable on a post.
    fn add(
        &mut self,
        id: &str,
        icon: &str,
        bundle: Option<usize>,
        client_action: Option<&str>,
        confirm: bool,
        secondary: bool,
    ) {
        let i18n = self.ctx.i18n;
        let action_name = format!("post-{id}");
        let scoped = format!("{}-{action_name}", self.reviewable_id);
        let prefix = format!("reviewables.actions.{id}");
        let mut a = Map::new();
        a.insert("id".into(), json!(scoped));
        a.insert("action_name".into(), json!(action_name));
        a.insert("icon".into(), json!(icon));
        a.insert("button_class".into(), Value::Null);
        a.insert(
            "label".into(),
            json!(i18n.t(&format!("{prefix}.title")).unwrap_or_default()),
        );
        if confirm {
            a.insert(
                "confirm_message".into(),
                json!(i18n.t(&format!("{prefix}.confirm")).unwrap_or_default()),
            );
        }
        if let Some(desc) = i18n.t(&format!("{prefix}.description")) {
            a.insert("description".into(), json!(desc));
        }
        a.insert("server_action".into(), json!(id));
        if let Some(c) = client_action {
            a.insert("client_action".into(), json!(c));
        }
        a.insert(
            "completed_message".into(),
            json!(i18n.t(&format!("{prefix}.complete"))),
        );
        self.actions.push(Value::Object(a));
        let bundle = match bundle {
            Some(b) => b,
            None => {
                self.bundles.push(Bundle {
                    id: scoped.clone(),
                    icon: None,
                    label: None,
                    secondary,
                    actions: Vec::new(),
                });
                self.bundles.len() - 1
            }
        };
        self.bundles[bundle].actions.push(scoped);
    }
}

/// `ReviewableFlaggedPost#build_combined_actions`; the bundles' ids.
async fn actions(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    r: &Reviewable,
    post: &Post,
    author: Option<&UserRow>,
    out: &mut Out,
) -> Result<Vec<String>, AppError> {
    let s = ctx.settings;
    let mut b = Actions {
        ctx,
        reviewable_id: r.id,
        bundles: Vec::new(),
        actions: Vec::new(),
    };
    let trashed = post.deleted_at.is_some();
    let agree = b.add_bundle("agree", "thumbs-up", "reviewables.actions.agree.title");
    if trashed {
        b.add(
            "agree_and_keep_deleted",
            "far-eye-slash",
            Some(agree),
            None,
            false,
            false,
        );
    } else if post.hidden {
        b.add(
            "agree_and_keep_hidden",
            "far-eye-slash",
            Some(agree),
            None,
            false,
            false,
        );
    } else {
        b.add(
            "agree_and_hide",
            "far-eye-slash",
            Some(agree),
            None,
            false,
            false,
        );
        b.add("agree_and_keep", "far-eye", Some(agree), None, false, false);
        b.add(
            "agree_and_edit",
            "pencil",
            Some(agree),
            Some("edit"),
            false,
            false,
        );
    }

    // can_delete_post_or_topic?
    let access =
        crate::posting::revisions::find_post_with_deleted(&mut *conn, ctx, guardian, post.id)
            .await?;
    let can_delete = match &access {
        Some(a) if post.post_number == 1 => guardian.can_delete_topic(s, &a.topic)?,
        Some(a) => guardian.can_delete_post(s, &a.topic, &a.post, a.can_see_post)?,
        None => false,
    };
    let can_delete_existing = can_delete && !trashed;
    if can_delete_existing {
        b.add(
            "delete_and_agree",
            "trash-can",
            Some(agree),
            None,
            false,
            false,
        );
        if post.reply_count > 0 {
            b.add(
                "delete_and_agree_replies",
                "trash-can",
                Some(agree),
                None,
                true,
                false,
            );
        }
    }
    // build_penalty_actions: can_suspend?, and the author is neither
    // silenced nor suspended (refused above).
    if let Some(a) = author
        && !a.admin
        && !a.moderator
    {
        b.add(
            "agree_and_silence",
            "microphone-slash",
            Some(agree),
            Some("silence"),
            false,
            false,
        );
        b.add(
            "agree_and_suspend",
            "ban",
            Some(agree),
            Some("suspend"),
            false,
            false,
        );
    }

    let can_ignore = trashed || !post.hidden || guardian.user_id() == Some(-1);
    if can_delete || can_ignore {
        let disagree = b.add_bundle(
            "disagree",
            "far-eye",
            "reviewables.actions.disagree_bundle.title",
        );
        if trashed {
            b.add(
                "disagree_and_keep_deleted",
                "far-eye-slash",
                Some(disagree),
                None,
                false,
                false,
            );
        } else if post.hidden {
            b.add(
                "disagree_and_restore",
                "far-eye",
                Some(disagree),
                None,
                false,
                false,
            );
        } else {
            b.add("disagree", "far-eye", Some(disagree), None, false, false);
        }
        if can_ignore {
            b.add(
                "ignore_and_do_nothing",
                "xmark",
                Some(disagree),
                None,
                false,
                false,
            );
        }
        if can_delete_existing {
            b.add(
                "delete_and_ignore",
                "trash-can",
                Some(disagree),
                None,
                false,
                false,
            );
            if post.reply_count > 0 {
                b.add(
                    "delete_and_ignore_replies",
                    "trash-can",
                    Some(disagree),
                    None,
                    true,
                    false,
                );
            }
        }
    }

    // Empty bundles are dropped.
    let i18n = ctx.i18n;
    let mut ids = Vec::new();
    for bundle in b.bundles.into_iter().filter(|b| !b.actions.is_empty()) {
        let mut v = Map::new();
        v.insert("id".into(), json!(bundle.id));
        if let Some(icon) = bundle.icon {
            v.insert("icon".into(), json!(icon));
        }
        if let Some(label) = bundle.label.and_then(|l| i18n.t(l)) {
            v.insert("label".into(), json!(label));
        }
        if bundle.secondary {
            v.insert("secondary".into(), json!(true));
        }
        v.insert("action_ids".into(), json!(bundle.actions));
        ids.push(bundle.id);
        out.bundles.push(Value::Object(v));
    }
    out.actions.extend(b.actions);
    Ok(ids)
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: i32,
    username: String,
    name: Option<String>,
    uploaded_avatar_id: Option<i32>,
    admin: bool,
    moderator: bool,
    trust_level: i32,
    created_at: NaiveDateTime,
    ip_address: Option<String>,
    silenced_till: Option<NaiveDateTime>,
    suspended_till: Option<NaiveDateTime>,
}

impl UserRow {
    fn silenced(&self) -> bool {
        self.silenced_till
            .is_some_and(|t| t > crate::clock::now_naive())
    }

    fn suspended(&self) -> bool {
        self.suspended_till
            .is_some_and(|t| t > crate::clock::now_naive())
    }
}

async fn load_user(conn: &mut PgConnection, id: i32) -> Result<UserRow, AppError> {
    if id <= 0 {
        return Err(Unsupported("system users in the review queue").into());
    }
    Ok(sqlx::query_as(
        "SELECT id, username, name, uploaded_avatar_id, admin, moderator, trust_level, created_at, \
                host(ip_address) AS ip_address, silenced_till, suspended_till \
         FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_one(conn)
    .await?)
}

/// `BasicUserSerializer`
fn basic_user(ctx: &Ctx<'_>, urls: &Urls<'_>, u: &UserRow) -> Result<Value, AppError> {
    let mut v = Map::new();
    v.insert("id".into(), json!(u.id));
    v.insert("username".into(), json!(u.username));
    if ctx.settings.get("enable_names")?.truthy() {
        v.insert("name".into(), json!(u.name));
    }
    v.insert(
        "avatar_template".into(),
        json!(crate::avatar::avatar_template(
            urls,
            u.id,
            &u.username,
            u.uploaded_avatar_id,
            None
        )?),
    );
    Ok(Value::Object(v))
}

/// `User.allowed_user_custom_fields(guardian)` with the user's values
/// that are present.
async fn custom_fields(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    user_id: i32,
) -> Result<Value, AppError> {
    let s = ctx.settings;
    let mut fields: Vec<String> = Vec::new();
    let split = |v: String| -> Vec<String> {
        v.split('|')
            .filter(|f| !f.is_empty())
            .map(str::to_string)
            .collect()
    };
    fields.extend(split(s.get("public_user_custom_fields")?.to_s()));
    if guardian.is_staff() {
        fields.extend(split(s.get("staff_user_custom_fields")?.to_s()));
        fields.extend(
            PLUGIN_STAFF_USER_CUSTOM_FIELDS
                .iter()
                .map(|f| f.to_string()),
        );
    }
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT name, value FROM user_custom_fields WHERE user_id = $1 AND name = ANY($2) ORDER BY id",
    )
    .bind(user_id)
    .bind(&fields)
    .fetch_all(conn)
    .await?;
    let mut out = Map::new();
    for (name, value) in rows {
        if out.contains_key(&name) {
            return Err(Unsupported("multi-valued user custom fields").into());
        }
        if value.as_deref().is_some_and(|v| !v.trim().is_empty()) {
            out.insert(name, json!(value));
        }
    }
    Ok(Value::Object(out))
}

/// `FlaggedUserSerializer` for an admin.
async fn flagged_user(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    urls: &Urls<'_>,
    u: &UserRow,
) -> Result<Value, AppError> {
    let mut v = basic_user(ctx, urls, u)?;
    let stats: (i32, i32, i32, i32, i32, Option<NaiveDateTime>) = sqlx::query_as(
        "SELECT post_count, topic_count, flags_agreed, flags_disagreed, flags_ignored, first_post_created_at \
         FROM user_stats WHERE user_id = $1",
    )
    .bind(u.id)
    .fetch_one(&mut *conn)
    .await?;
    let (post_count, topic_count, agreed, disagreed, ignored, _) = stats;
    // can_delete_all_posts? for an admin: anyone but admins.
    v["can_delete_all_posts"] = json!(!u.admin);
    v["can_be_deleted"] = json!(can_delete_user(&mut *conn, ctx, guardian, u, &stats).await?);
    v["post_count"] = json!(post_count);
    v["topic_count"] = json!(topic_count);
    // can_see_ip? and can_check_emails?: admins.
    v["ip_address"] = json!(u.ip_address);
    let email: Option<String> =
        sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
            .bind(u.id)
            .fetch_optional(&mut *conn)
            .await?;
    v["email"] = json!(email);
    v["custom_fields"] = custom_fields(&mut *conn, ctx, guardian, u.id).await?;
    v["flags_agreed"] = json!(agreed);
    v["flags_disagreed"] = json!(disagreed);
    v["flags_ignored"] = json!(ignored);
    v["created_at"] = json!(time_json(u.created_at));
    v["trust_level"] = json!(u.trust_level);
    let count = |action: i32| {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM user_histories WHERE target_user_id = $1 AND action = $2",
        )
        .bind(u.id)
        .bind(action)
    };
    v["silenced_count"] = json!(count(SILENCE_USER).fetch_one(&mut *conn).await?);
    v["suspended_count"] = json!(count(SUSPEND_USER).fetch_one(&mut *conn).await?);
    let rejected: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM reviewables WHERE type = 'ReviewableQueuedPost' AND status = $2 \
         AND target_created_by_id = $1",
    )
    .bind(u.id)
    .bind(REJECTED)
    .fetch_one(&mut *conn)
    .await?;
    v["rejected_posts_count"] = json!(rejected);
    Ok(v)
}

/// `can_delete_user?(user)` for an admin.
async fn can_delete_user(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    u: &UserRow,
    stats: &(i32, i32, i32, i32, i32, Option<NaiveDateTime>),
) -> Result<bool, AppError> {
    if u.admin {
        return Ok(false);
    }
    if guardian.is_me(u.id) {
        return Err(Unsupported("reviewing one's own posts").into());
    }
    let (post_count, topic_count, .., first_post) = *stats;
    let Some(first_post) = first_post else {
        return Ok(true);
    };
    // has_more_posts_than?(MAX_STAFF_DELETE_POST_COUNT)
    let more = if i64::from(topic_count + post_count) > MAX_STAFF_DELETE_POST_COUNT {
        true
    } else {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(1) FROM ( \
               SELECT 1 FROM posts p JOIN topics t ON (p.topic_id = t.id) \
               WHERE p.user_id = $1 AND p.deleted_at IS NULL AND t.deleted_at IS NULL AND \
                 (t.archetype <> 'private_message' OR \
                  EXISTS (SELECT 1 FROM topic_allowed_users a \
                          WHERE a.topic_id = t.id AND a.user_id > 0 AND a.user_id <> $1) OR \
                  EXISTS (SELECT 1 FROM topic_allowed_groups g WHERE g.topic_id = p.topic_id)) \
               LIMIT $2) x",
        )
        .bind(u.id)
        .bind(MAX_STAFF_DELETE_POST_COUNT + 1)
        .fetch_one(conn)
        .await?;
        n > MAX_STAFF_DELETE_POST_COUNT
    };
    if !more {
        return Ok(true);
    }
    let days = ctx.settings.get("delete_user_max_post_age")?.to_i();
    Ok(first_post > crate::clock::now_naive() - chrono::Duration::days(days))
}
