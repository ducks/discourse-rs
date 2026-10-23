//! The review queue: ReviewablesController#index as `Reviewable.list_for`
//! pages it, ReviewableFlaggedPostSerializer (with ReviewableSerializer)
//! and the records it side-loads (users, topics, scores, score types,
//! bundled actions and their actions, notes, histories, claims), and the
//! meta the controller adds.
//!
//! Viewers see `Reviewable.viewable_by` (reviewables.rs). Refused: category
//! group moderators (before the list), reviewable types other than
//! flagged posts on a page, the `ids`, date and additional filters, score
//! reasons other than watched words and conversations, authors under a
//! penalty, posts the user deleted, potential spam and illegal content
//! (the delete user actions), bot users, and content localization.

use std::collections::HashSet;

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::params;
use crate::topic_list::{Mode, TopicListSerializer, time_json};
use crate::topic_query::{TOPIC_COLUMNS, TopicRow};
use crate::url::Urls;
use crate::web_hooks::Payloads as Ctx;
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

/// ReviewablesController#index.
pub async fn index(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    p: &Map<String, Value>,
) -> Result<Listed, AppError> {
    let s = ctx.settings;
    if guardian.is_anonymous() {
        return Err(Unsupported("the review queue for anonymous users").into());
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
    // Reviewable.viewable_by(user)
    if let Some(v) = crate::reviewables::viewable_by(&mut *conn, s, guardian).await? {
        q.joins.push_str(v.joins);
        q.wheres.push(v.clause);
    }
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
        reviewables.push(serialize(&mut *conn, ctx, guardian, id, &mut out, false).await?);
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
    let (count, unseen) = crate::reviewables::staff_counts(&mut *conn, s, guardian).await?;
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
        doc.insert("reviewable_notes".into(), Value::Array(out.notes));
        doc.insert("reviewable_histories".into(), Value::Array(out.histories));
        doc.insert("claimed_bies".into(), Value::Array(out.claims));
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
    notes: Vec<Value>,
    histories: Vec<Value>,
    claims: Vec<Value>,
}

/// One reviewable's side-loads: each user and topic once (AMS's
/// `unique_values` hold per serializer, so across reviewables they repeat).
#[derive(Default)]
struct Seen {
    users: HashSet<i32>,
    topics: HashSet<i32>,
    /// A web hook's payload: the root object alone, no side-loaded users.
    web_hook: bool,
}

impl Seen {
    /// Whether to side-load this user: the first time it is seen.
    fn user(&mut self, id: i32) -> bool {
        !self.web_hook && self.users.insert(id)
    }
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
    web_hook: bool,
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
    // target_deleted_by / target_deleted_at read target_post_with_deleted.
    let deleted_by = post.as_ref().and_then(|p| p.deleted_by_id);
    let target_deleted_at = post
        .as_ref()
        .and_then(|p| p.deleted_by_id.and(p.deleted_at));
    // A web hook's serializer runs outside `with_deleted_content`: a
    // trashed post or topic is no target.
    let post = post.filter(|p| !web_hook || p.deleted_at.is_none());
    let topic: Option<TopicRow> = match r.topic_id {
        Some(tid) => sqlx::query_as(&format!(
            "SELECT {TOPIC_COLUMNS} FROM topics WHERE id = $1 AND (NOT $2 OR deleted_at IS NULL)"
        ))
        .bind(tid)
        .bind(web_hook)
        .fetch_optional(&mut *conn)
        .await?,
        None => None,
    };
    let author = match r.target_created_by_id {
        Some(uid) => Some(load_user(&mut *conn, ctx, uid).await?),
        None => None,
    };
    if author
        .as_ref()
        .is_some_and(|a| a.silenced() || a.suspended())
    {
        return Err(Unsupported("authors under a penalty (author_penalties)").into());
    }

    let mut seen = Seen {
        web_hook,
        ..Seen::default()
    };
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
    if let Some(at) = target_deleted_at {
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
        && seen.user(a.id)
    {
        out.users
            .push(flagged_user(&mut *conn, ctx, guardian, &urls, a).await?);
    }
    // has_one :created_by (UserWithCustomFieldsSerializer)
    d.insert("created_by_id".into(), json!(r.created_by_id));
    if seen.user(r.created_by_id) {
        let u = load_user(&mut *conn, ctx, r.created_by_id).await?;
        let mut v = basic_user(ctx, &urls, &u)?;
        v["custom_fields"] = custom_fields(&mut *conn, ctx, guardian, u.id).await?;
        out.users.push(v);
    }
    // has_one :target_deleted_by (BasicUserSerializer)
    if let Some(uid) = deleted_by {
        d.insert("target_deleted_by_id".into(), json!(uid));
        if seen.user(uid) {
            let u = load_user(&mut *conn, ctx, uid).await?;
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
    // has_many :reviewable_notes, oldest first (ReviewableNoteSerializer,
    // its author embedded).
    let notes: Vec<(i64, String, NaiveDateTime, NaiveDateTime, i32)> = sqlx::query_as(
        "SELECT id, content, created_at, updated_at, user_id::integer FROM reviewable_notes \
         WHERE reviewable_id = $1 ORDER BY created_at, id",
    )
    .bind(r.id)
    .fetch_all(&mut *conn)
    .await?;
    let mut note_ids = Vec::new();
    for (nid, content, created_at, updated_at, user_id) in notes {
        if !web_hook {
            let u = load_user(&mut *conn, ctx, user_id).await?;
            out.notes.push(json!({
                "id": nid,
                "content": content,
                "created_at": time_json(created_at),
                "updated_at": time_json(updated_at),
                "user": basic_user(ctx, &urls, &u)?,
            }));
        }
        note_ids.push(nid);
    }
    d.insert("reviewable_note_ids".into(), json!(note_ids));

    // has_many :reviewable_histories
    let histories: Vec<(i64, NaiveDateTime, i32, i32, i32)> = sqlx::query_as(
        "SELECT id, created_at, reviewable_history_type, status, created_by_id \
         FROM reviewable_histories WHERE reviewable_id = $1 ORDER BY ctid",
    )
    .bind(r.id)
    .fetch_all(&mut *conn)
    .await?;
    let mut history_ids = Vec::new();
    for (hid, at, kind, status, by) in histories {
        if seen.user(by) {
            let u = load_user(&mut *conn, ctx, by).await?;
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
    // has_one :claimed_by: ReviewableClaimedTopic.claimed_hash, automatic
    // claims alone while claiming is off (ReviewableClaimedTopicSerializer,
    // its user side-loaded with custom fields).
    let claim: Option<(i64, bool, i32)> = match r.topic_id {
        Some(topic_id) => {
            sqlx::query_as(
                "SELECT id, automatic, user_id FROM reviewable_claimed_topics \
                 WHERE topic_id = $1 AND (automatic OR $2) ORDER BY id LIMIT 1",
            )
            .bind(topic_id)
            .bind(s.get("reviewable_claiming")?.to_s() != "disabled")
            .fetch_optional(&mut *conn)
            .await?
        }
        None => None,
    };
    match claim {
        Some((cid, automatic, user_id)) if !web_hook => {
            d.insert("claimed_by_id".into(), json!(cid));
            out.claims
                .push(json!({ "id": cid, "automatic": automatic, "user_id": user_id }));
            if seen.user(user_id) {
                let u = load_user(&mut *conn, ctx, user_id).await?;
                let mut v = basic_user(ctx, &urls, &u)?;
                v["custom_fields"] = custom_fields(&mut *conn, ctx, guardian, u.id).await?;
                out.users.push(v);
            }
        }
        _ => {
            d.insert("claimed_by_id".into(), Value::Null);
        }
    }
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
        context: Option<String>,
    }
    let rows: Vec<Score> = sqlx::query_as(
        "SELECT id, user_id, reviewable_score_type, status, score, reviewed_by_id, reviewed_at, meta_topic_id, \
                created_at, reason, context \
         FROM reviewable_scores WHERE reviewable_id = $1 ORDER BY created_at DESC",
    )
    .bind(r.id)
    .fetch_all(&mut *conn)
    .await?;
    let types = score_types(&mut *conn).await?;
    let mut ids = Vec::new();
    for sc in rows {
        if sc.meta_topic_id.is_some() {
            return Err(Unsupported("score conversations in the queue").into());
        }
        let (reason, reason_type, reason_data) = match sc.reason.as_deref() {
            None => (None, Value::Null, Value::Null),
            Some("watched_word") => {
                let words = watched_words_found(sc.context.as_deref())?;
                (
                    Some(watched_word_reason(ctx, urls, &words)?),
                    json!("watched_word"),
                    json!(words),
                )
            }
            Some(_) => return Err(Unsupported("score reasons other than watched words").into()),
        };
        let stats: (i32, i32, i32) = sqlx::query_as(
            "SELECT flags_agreed, flags_disagreed, flags_ignored FROM user_stats WHERE user_id = $1",
        )
        .bind(sc.user_id)
        .fetch_one(&mut *conn)
        .await?;
        let mut score = Map::new();
        score.insert("id".into(), json!(sc.id));
        score.insert("score".into(), json!(sc.score));
        score.insert(
            "agree_stats".into(),
            json!({ "agreed": stats.0, "disagreed": stats.1, "ignored": stats.2 }),
        );
        // include_reason?: only when present.
        if let Some(reason) = reason {
            score.insert("reason".into(), json!(reason));
        }
        score.insert("reason_type".into(), reason_type);
        score.insert("reason_data".into(), reason_data);
        score.insert("created_at".into(), json!(time_json(sc.created_at)));
        score.insert("reviewed_at".into(), json!(sc.reviewed_at.map(time_json)));
        score.insert("status".into(), json!(sc.status));
        score.insert("user_id".into(), json!(sc.user_id));
        score.insert("score_type_id".into(), json!(sc.reviewable_score_type));
        score.insert("reviewed_by_id".into(), json!(sc.reviewed_by_id));
        out.scores.push(Value::Object(score));
        // has_one :user, :score_type, :reviewed_by in that order.
        if seen.user(sc.user_id) {
            let u = load_user(&mut *conn, ctx, sc.user_id).await?;
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
            && seen.user(by)
        {
            let u = load_user(&mut *conn, ctx, by).await?;
            out.users.push(basic_user(ctx, urls, &u)?);
        }
        ids.push(sc.id);
    }
    Ok(ids)
}

/// `watched_words_found`: the score's context split on commas, downcased
/// and unique. Without a context Rails guesses from the post's current
/// raw, which isn't ported.
fn watched_words_found(context: Option<&str>) -> Result<Vec<String>, AppError> {
    let context = context.ok_or(Unsupported("watched word scores without their words"))?;
    let mut words: Vec<String> = Vec::new();
    // Ruby's split drops trailing empty strings.
    let mut parts: Vec<&str> = context.split(',').collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    for w in parts {
        let w = w.to_lowercase();
        if !words.contains(&w) {
            words.push(w);
        }
    }
    Ok(words)
}

/// ReviewableScoreSerializer#reason for "watched_word", as staff see it:
/// `PrettyText.sanitize("<p>#{watched_word_reason(link)}</p>")`.
fn watched_word_reason(
    ctx: &Ctx<'_>,
    urls: &Urls<'_>,
    words: &[String],
) -> Result<String, AppError> {
    let link_text = ctx
        .t("reviewables.reasons.links.watched_word")
        .replace('_', " ");
    let link = format!(
        "<a href=\"{}/admin/customize/watched_words\">{link_text}</a>",
        urls.base_url()?
    );
    let text = if words.is_empty() {
        ctx.i18n
            .t_with(
                "reviewables.reasons.no_context.watched_word",
                &[("link", &link)],
            )
            .unwrap_or_default()
    } else {
        let escaped: Vec<String> = words
            .iter()
            .map(|w| crate::category_badge::html_escape(w))
            .collect();
        let form = if words.len() == 1 { "one" } else { "other" };
        ctx.i18n
            .t_with(
                &format!("reviewables.reasons.watched_word.{form}"),
                &[
                    ("link", &link),
                    ("words", &escaped.join(", ")),
                    ("count", &words.len().to_string()),
                ],
            )
            .unwrap_or_default()
    };
    Ok(discourse_markdown::sanitizer::sanitize(
        &format!("<p>{text}</p>"),
        &discourse_markdown::sanitizer::AllowList::new(),
    ))
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
    let access = crate::posting::revisions::find_post_with_deleted_in(
        &mut *conn,
        ctx.settings,
        guardian,
        post.id,
    )
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
    /// The small logo's url, for the system user (its avatar when the site
    /// uses it).
    logo_small_url: Option<String>,
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

async fn load_user(conn: &mut PgConnection, ctx: &Ctx<'_>, id: i32) -> Result<UserRow, AppError> {
    // The system user (watched word flags) is ported; bots are not.
    if id < crate::avatar::SYSTEM_USER_ID || id == 0 {
        return Err(Unsupported("bot users in the review queue").into());
    }
    Ok(sqlx::query_as(
        "SELECT id, username, name, uploaded_avatar_id, admin, moderator, trust_level, created_at, \
                host(ip_address) AS ip_address, silenced_till, suspended_till, \
                CASE WHEN id = -1 THEN (SELECT url FROM uploads WHERE id = $2) END AS logo_small_url \
         FROM users WHERE id = $1",
    )
    .bind(id)
    .bind(ctx.settings.get("logo_small")?.to_i() as i32)
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
            u.logo_small_url.as_deref()
        )?),
    );
    Ok(Value::Object(v))
}

/// `User.allowed_user_custom_fields(guardian)`
pub(crate) fn allowed_user_custom_fields(
    s: &crate::site_settings::SiteSettings,
    guardian: &Guardian,
) -> Result<Vec<String>, crate::site_settings::SettingError> {
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
    Ok(fields)
}

/// `User.allowed_user_custom_fields(guardian)` with the user's values
/// that are present.
async fn custom_fields(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    user_id: i32,
) -> Result<Value, AppError> {
    let fields = allowed_user_custom_fields(ctx.settings, guardian)?;
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

/// `FlaggedUserSerializer`
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
    let (post_count, topic_count, agreed, disagreed, ignored, first_post) = stats;
    let s = ctx.settings;
    // can_delete_all_posts?
    let max_age = chrono::Duration::days(s.get("delete_user_max_post_age")?.to_i());
    let can_delete_all = guardian.is_staff()
        && !u.admin
        && (!u.moderator || guardian.is_admin())
        && (guardian.is_admin()
            || (first_post.is_none_or(|t| t >= crate::clock::now_naive() - max_age)
                && i64::from(post_count) <= s.get("delete_all_posts_max")?.to_i()));
    v["can_delete_all_posts"] = json!(can_delete_all);
    v["can_be_deleted"] = json!(can_delete_user(&mut *conn, ctx, guardian, u, &stats).await?);
    v["post_count"] = json!(post_count);
    v["topic_count"] = json!(topic_count);
    if crate::admin_users::can_see_ip(s, guardian)? {
        v["ip_address"] = json!(u.ip_address);
    }
    // can_check_emails?(scope.user)
    if guardian.is_admin() || (guardian.is_staff() && s.get("moderators_view_emails")?.truthy()) {
        let email: Option<String> =
            sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
                .bind(u.id)
                .fetch_optional(&mut *conn)
                .await?;
        v["email"] = json!(email);
    }
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

/// `can_delete_user?(user)`
async fn can_delete_user(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    u: &UserRow,
    stats: &(i32, i32, i32, i32, i32, Option<NaiveDateTime>),
) -> Result<bool, AppError> {
    let (post_count, topic_count, .., first_post) = *stats;
    let user = crate::guardian::DeletableUser {
        id: u.id,
        admin: u.admin,
        moderator: u.moderator,
        topic_count: Some(topic_count),
        post_count: Some(post_count),
        first_post_created_at: first_post,
    };
    Ok(guardian.can_delete_user(conn, ctx.settings, &user).await?)
}

/// `reviewable.serializer` (root: false) for a web hook's payload, seen by
/// the system user: flagged posts (the queue's serializer, without what it
/// side-loads) and users.
pub async fn web_hook_json(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    id: i64,
) -> Result<Value, AppError> {
    let kind: String = sqlx::query_scalar("SELECT type FROM reviewables WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    match kind.as_str() {
        "ReviewableFlaggedPost" => {
            let mut out = Out::default();
            let mut v = serialize(conn, ctx, guardian, id, &mut out, true).await?;
            if let Some(m) = v.as_object_mut() {
                m.shift_remove("claimed_by_id");
            }
            Ok(v)
        }
        "ReviewableUser" => user_json(conn, ctx, guardian, id).await,
        _ => Err(Unsupported(
            "web hook payloads of reviewables other than flagged posts and users",
        )
        .into()),
    }
}

/// ReviewableUserSerializer, root: false.
async fn user_json(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    id: i64,
) -> Result<Value, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i64,
        type_source: Option<String>,
        target_id: Option<i32>,
        target_type: Option<String>,
        created_at: NaiveDateTime,
        score: f64,
        version: i32,
        status: i32,
        reject_reason: Option<String>,
        payload: Option<Value>,
        created_by_id: i32,
        topic_id: Option<i32>,
    }
    let r: Row = sqlx::query_as(
        "SELECT id, type_source, target_id, target_type, created_at, score, version, status, reject_reason, \
                payload::json AS payload, created_by_id, topic_id \
         FROM reviewables WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    if r.topic_id.is_some() {
        return Err(Unsupported("user reviewables with a topic in web hook payloads").into());
    }
    let urls = Urls {
        config: ctx.config,
        settings: ctx.settings,
    };
    let target = match r.target_id {
        Some(uid) => {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
                    .bind(uid)
                    .fetch_one(&mut *conn)
                    .await?;
            if exists {
                let u = load_user(&mut *conn, ctx, uid).await?;
                Some(flagged_user(&mut *conn, ctx, guardian, &urls, &u).await?)
            } else {
                None
            }
        }
        None => None,
    };
    let score_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM reviewable_scores WHERE reviewable_id = $1 ORDER BY created_at DESC",
    )
    .bind(r.id)
    .fetch_all(&mut *conn)
    .await?;
    let history_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM reviewable_histories WHERE reviewable_id = $1 ORDER BY ctid",
    )
    .bind(r.id)
    .fetch_all(&mut *conn)
    .await?;
    // build_actions (ReviewableUser#build_combined_actions) for the system
    // user, an admin: nothing once approved; scrub for a rejected review
    // whose user is gone or renamed; removing a pending user's avatar,
    // approving them, rejecting them.
    let suspect: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM reviewable_scores WHERE reviewable_id = $1 AND reason = 'suspect_user')",
    )
    .bind(r.id)
    .fetch_one(&mut *conn)
    .await?;
    if suspect && r.status == crate::reviewables::PENDING {
        return Err(Unsupported("suspect user reviewables in web hook payloads").into());
    }
    let target_row: Option<(String, bool, Option<i32>)> = match r.target_id {
        Some(uid) => {
            sqlx::query_as("SELECT username, approved, uploaded_avatar_id FROM users WHERE id = $1")
                .bind(uid)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    let mut bundles: Vec<String> = Vec::new();
    const APPROVED: i32 = 1;
    const REJECTED_STATUS: i32 = 2;
    if r.status != APPROVED {
        let payload_username = r
            .payload
            .as_ref()
            .and_then(|p| p.get("username"))
            .and_then(Value::as_str)
            .filter(|u| !u.trim().is_empty());
        if guardian.is_admin()
            && r.status == REJECTED_STATUS
            && let Some(username) = payload_username
            && target_row
                .as_ref()
                .is_none_or(|(name, _, _)| name != username)
        {
            bundles.push(format!("{}-user-scrub", r.id));
        }
        let pending = r.status == crate::reviewables::PENDING;
        if pending && target_row.as_ref().is_some_and(|t| t.2.is_some()) {
            bundles.push(format!("{}-user-remove_avatar", r.id));
        }
        if pending && target_row.as_ref().is_some_and(|t| !t.1) && guardian.is_staff() {
            bundles.push(format!("{}-user-approve_user", r.id));
        }
        if pending {
            bundles.push(format!("{}-reject_user", r.id));
        }
    }
    let bundles = json!(bundles);
    let mut d = Map::new();
    d.insert("id".into(), json!(r.id));
    d.insert("type".into(), json!("ReviewableUser"));
    d.insert("type_source".into(), json!(r.type_source));
    d.insert("target_type".into(), json!(r.target_type));
    d.insert("target_id".into(), json!(r.target_id));
    d.insert("created_at".into(), json!(time_json(r.created_at)));
    d.insert("can_edit".into(), json!(false));
    d.insert("score".into(), json!(r.score));
    d.insert("version".into(), json!(r.version));
    d.insert("target_created_by_trust_level".into(), Value::Null);
    d.insert("created_from_flag".into(), json!(false));
    d.insert("status".into(), json!(r.status));
    d.insert(
        "link_admin".into(),
        json!(guardian.is_staff() && target.is_some()),
    );
    d.insert("reject_reason".into(), json!(r.reject_reason));
    let present = target.is_some();
    if let Some(target) = target {
        d.insert("target_user".into(), target);
    }
    d.insert("removed_topic_id".into(), Value::Null);
    if present {
        d.insert("user_id".into(), json!(r.target_id));
    }
    // payload_attributes, sliced in their order (the system user may
    // check emails).
    let stored = r.payload.unwrap_or(Value::Null);
    let mut payload = Map::new();
    for key in [
        "username",
        "email",
        "name",
        "bio",
        "website",
        "avatar_url",
        "scrubbed_by",
        "scrubbed_reason",
        "scrubbed_at",
    ] {
        if let Some(v) = stored.get(key) {
            payload.insert(key.into(), v.clone());
        }
    }
    d.insert("payload".into(), Value::Object(payload));
    // has_one :target_created_by: a user reviewable's is its target.
    d.insert(
        "target_created_by_id".into(),
        json!(target_row.as_ref().and(r.target_id)),
    );
    d.insert("created_by_id".into(), json!(r.created_by_id));
    d.insert("topic_id".into(), Value::Null);
    d.insert("editable_fields".into(), json!([]));
    d.insert("reviewable_score_ids".into(), json!(score_ids));
    d.insert("bundled_action_ids".into(), bundles);
    let note_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM reviewable_notes WHERE reviewable_id = $1 ORDER BY ctid",
    )
    .bind(r.id)
    .fetch_all(&mut *conn)
    .await?;
    d.insert("reviewable_note_ids".into(), json!(note_ids));
    d.insert("reviewable_history_ids".into(), json!(history_ids));
    d.insert("author_penalties".into(), json!([]));
    Ok(Value::Object(d))
}
