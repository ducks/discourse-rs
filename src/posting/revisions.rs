//! posts#revisions and posts#latest_revision: finding the post and the
//! revision as PostsController does, and PostRevisionSerializer.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::Ctx;
use crate::discourse_diff::{body_changes, html_diff};
use crate::guardian::Guardian;
use crate::topic_guardian::{PostCtx, TopicCtx};
use crate::topic_list::time_json;
use crate::url::Urls;
use crate::{AppError, Unsupported, avatar, modifications};

/// A post with what the guardian answers about it for this viewer.
pub struct PostAccess {
    pub topic: TopicCtx,
    pub post: PostCtx,
    pub topic_id: i32,
    pub can_see_topic: bool,
    pub can_see_post: bool,
    pub can_create_post: bool,
}

/// `find_post_from_params`: the post when the viewer can see it.
pub async fn find_post(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i32,
) -> Result<Option<PostAccess>, AppError> {
    find(conn, ctx, guardian, post_id, false).await
}

/// `find_post_from_params` as `find_post_using` does it in full: a deleted
/// post, or one in a deleted topic, for those who can moderate the topic.
pub async fn find_post_with_deleted(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i32,
) -> Result<Option<PostAccess>, AppError> {
    find(conn, ctx, guardian, post_id, true).await
}

async fn find(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i32,
    with_deleted: bool,
) -> Result<Option<PostAccess>, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i32,
        user_id: Option<i32>,
        topic_id: i32,
        post_number: i32,
        post_type: i32,
        hidden: bool,
        hidden_at: Option<NaiveDateTime>,
        locked_by_id: Option<i32>,
        deleted_at: Option<NaiveDateTime>,
        user_deleted: bool,
        wiki: bool,
        created_at: NaiveDateTime,
        author_staff: Option<bool>,
    }
    let row: Option<Row> = sqlx::query_as(
        "SELECT p.id, p.user_id, p.topic_id, p.post_number, p.post_type, p.hidden, p.hidden_at, p.locked_by_id, \
                p.deleted_at, p.user_deleted, p.wiki, p.created_at, (u.admin OR u.moderator) AS author_staff \
         FROM posts p LEFT JOIN users u ON u.id = p.user_id WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let s = ctx.settings;
    let Some(topic) = TopicCtx::load(&mut *conn, s, guardian, row.topic_id).await? else {
        return Ok(None);
    };
    let secure = guardian.secure_category_ids(&mut *conn, s).await?;
    let can_see_topic = guardian.can_see_topic(s, &topic, true, &secure)?;
    if row.deleted_at.is_some() || topic.trashed() {
        if !with_deleted {
            if guardian.is_staff() {
                return Err(Unsupported("deleted posts for staff").into());
            }
            return Ok(None);
        }
        // can_moderate_topic?
        let can_moderate_topic = guardian.is_staff()
            || guardian.can_perform_action_available_to_group_moderators(s, can_see_topic)?;
        if !can_moderate_topic {
            return Ok(None);
        }
    }
    let post = PostCtx {
        id: row.id,
        user_id: row.user_id,
        post_number: row.post_number,
        post_type: row.post_type,
        hidden: row.hidden,
        hidden_at: row.hidden_at,
        locked_by_id: row.locked_by_id,
        deleted_at: row.deleted_at,
        user_deleted: row.user_deleted,
        wiki: row.wiki,
        created_at: row.created_at,
        author_staff: row.author_staff.unwrap_or(false),
    };
    let can_see_post = guardian.can_see_post(s, &post, can_see_topic)?;
    if !can_see_post {
        return Ok(None);
    }
    let can_post_anywhere = guardian.can_create_post_anywhere(&mut *conn, s).await?;
    let can_create_post = guardian.can_create_post(s, &topic, can_post_anywhere)?;
    Ok(Some(PostAccess {
        topic_id: row.topic_id,
        topic,
        post,
        can_see_topic,
        can_see_post,
        can_create_post,
    }))
}

impl PostAccess {
    pub fn can_edit(&self, ctx: &Ctx<'_>, guardian: &Guardian) -> Result<bool, AppError> {
        Ok(guardian.is_authenticated()
            && guardian.can_edit_post(
                ctx.settings,
                &self.topic,
                &self.post,
                self.can_see_topic,
                self.can_create_post,
            )?)
    }
}

/// Which revision was asked for.
pub enum Which {
    Number(String),
    Latest,
}

pub enum RevisionResult {
    Found(Value),
    NotFound,
    InvalidRevision,
    Forbidden,
}

#[derive(sqlx::FromRow)]
struct RevisionRow {
    number: i32,
    user_id: Option<i32>,
    hidden: bool,
    modifications: Option<String>,
    created_at: NaiveDateTime,
}

/// A revision as the serializer flattens it: field values by name.
struct Flat {
    revision: i32,
    hidden: bool,
    fields: Map<String, Value>,
}

/// posts#revisions (`Which::Number`) and posts#latest_revision.
pub async fn show(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i32,
    which: Which,
) -> Result<RevisionResult, AppError> {
    let Some(access) = find_post(conn, ctx, guardian, post_id).await? else {
        return Ok(RevisionResult::NotFound);
    };
    let staff = guardian.is_staff();
    if access.post.hidden && !staff {
        return Ok(RevisionResult::NotFound);
    }
    let revision: Option<RevisionRow> = match which {
        Which::Number(n) => {
            let n = crate::ruby::to_i(&n);
            if n < 2 {
                return Ok(RevisionResult::InvalidRevision);
            }
            sqlx::query_as(
                "SELECT number, user_id, hidden, modifications, created_at FROM post_revisions \
                 WHERE post_id = $1 AND number = $2",
            )
            .bind(post_id)
            .bind(n as i32)
            .fetch_optional(&mut *conn)
            .await?
        }
        Which::Latest => {
            sqlx::query_as(
                "SELECT number, user_id, hidden, modifications, created_at FROM post_revisions \
                 WHERE post_id = $1 AND (NOT hidden OR $2) ORDER BY number DESC LIMIT 1",
            )
            .bind(post_id)
            .bind(staff)
            .fetch_optional(&mut *conn)
            .await?
        }
    };
    let Some(revision) = revision else {
        return Ok(RevisionResult::NotFound);
    };
    // ensure_can_see!(post_revision)
    let can_see = (staff || !revision.hidden)
        && guardian.can_view_edit_history(ctx.settings, &access.post, access.can_see_post)?;
    if !can_see {
        return Ok(RevisionResult::Forbidden);
    }
    serialize(conn, ctx, guardian, &access, &revision).await
}

async fn serialize(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    access: &PostAccess,
    object: &RevisionRow,
) -> Result<RevisionResult, AppError> {
    let s = ctx.settings;
    let staff = guardian.is_staff();
    let rows: Vec<RevisionRow> = sqlx::query_as(
        "SELECT number, user_id, hidden, modifications, created_at FROM post_revisions \
         WHERE post_id = $1 ORDER BY number DESC LIMIT 99",
    )
    .bind(access.post.id)
    .fetch_all(&mut *conn)
    .await?;
    if rows.iter().any(|r| r.hidden) {
        return Err(Unsupported("hidden post revisions").into());
    }
    #[derive(sqlx::FromRow)]
    struct Latest {
        raw: String,
        cooked: String,
        edit_reason: Option<String>,
        wiki: bool,
        post_type: i32,
        user_id: Option<i32>,
        locale: Option<String>,
        reply_to_post_number: Option<i32>,
        version: i32,
        hidden: bool,
        title: String,
        archetype: String,
        category_id: Option<i32>,
        featured_link: Option<String>,
        tags: Vec<String>,
    }
    let latest: Latest = sqlx::query_as(
        "SELECT p.raw, p.cooked, p.edit_reason, p.wiki, p.post_type, p.user_id, p.locale, p.reply_to_post_number, \
                p.version, p.hidden, t.title, t.archetype, t.category_id, t.featured_link, \
                COALESCE(ARRAY(SELECT tg.name FROM topic_tags tt JOIN tags tg ON tg.id = tt.tag_id \
                  WHERE tt.topic_id = t.id ORDER BY tg.name), '{}') AS tags \
         FROM posts p JOIN topics t ON t.id = p.topic_id WHERE p.id = $1",
    )
    .bind(access.post.id)
    .fetch_one(&mut *conn)
    .await?;

    // all_revisions: each stored revision's "before" values, then the
    // post as it is now, gaps filled from the next one.
    let mut all: Vec<Flat> = Vec::new();
    for r in rows.iter().rev() {
        let mut fields = Map::new();
        for (field, [before, _]) in
            modifications::load(r.modifications.as_deref().unwrap_or("--- {}\n"))?
        {
            fields.insert(field, before);
        }
        all.push(Flat {
            revision: r.number,
            hidden: r.hidden,
            fields,
        });
    }
    let mut now = Map::new();
    now.insert("raw".into(), json!(latest.raw));
    now.insert("cooked".into(), json!(latest.cooked));
    now.insert("edit_reason".into(), json!(latest.edit_reason));
    now.insert("wiki".into(), json!(latest.wiki));
    now.insert("post_type".into(), json!(latest.post_type));
    now.insert("user_id".into(), json!(latest.user_id));
    now.insert("locale".into(), json!(latest.locale));
    now.insert(
        "reply_to_post_number".into(),
        json!(latest.reply_to_post_number),
    );
    now.insert("title".into(), json!(latest.title));
    now.insert("archetype".into(), json!(latest.archetype));
    now.insert("category_id".into(), json!(latest.category_id));
    now.insert("featured_link".into(), json!(latest.featured_link));
    now.insert("tags".into(), json!(latest.tags));
    let last_number = all.last().map(|r| r.revision).unwrap_or(1);
    all.push(Flat {
        revision: last_number + 1,
        hidden: latest.hidden,
        fields: now,
    });
    for r in (1..all.len()).rev() {
        let later = all[r].fields.clone();
        for (k, v) in later {
            all[r - 1].fields.entry(k).or_insert(v);
        }
    }
    let revisions: Vec<&Flat> = all.iter().filter(|r| staff || !r.hidden).collect();
    let current_revision = object.number;
    let previous = revisions
        .iter()
        .rfind(|r| r.revision <= current_revision)
        .ok_or(Unsupported("a revision with nothing before it"))?;
    let current = revisions
        .iter()
        .find(|r| r.revision > current_revision)
        .ok_or(Unsupported("a revision with nothing after it"))?;
    let first_revision = revisions[0].revision;
    let last_revision = revisions
        .iter()
        .rfind(|r| r.revision <= latest.version)
        .map(|r| r.revision)
        .ok_or(Unsupported("a revision past the post's version"))?;
    let previous_revision = revisions
        .iter()
        .rfind(|r| r.revision >= first_revision && r.revision < current_revision)
        .map(|r| r.revision);
    let next_revision = revisions
        .iter()
        .find(|r| r.revision <= last_revision && r.revision > current_revision)
        .map(|r| r.revision);
    let current_version = revisions
        .iter()
        .filter(|r| r.revision <= current_revision)
        .count()
        + 1;

    let field = |r: &Flat, k: &str| r.fields.get(k).cloned().unwrap_or(Value::Null);
    for k in [
        "user_id",
        "reply_to_post_number",
        "tags",
        "category_id",
        "wiki",
        "post_type",
        "locale",
    ] {
        if field(previous, k) != field(current, k) {
            return Err(Unsupported(
                "revisions changing more than the body, title and edit reason",
            )
            .into());
        }
    }

    let user_id = object
        .user_id
        .ok_or(Unsupported("revisions without a user"))?;
    if user_id <= 0 {
        return Err(Unsupported("revisions by the system user").into());
    }
    let (username, name, uploaded_avatar_id): (String, Option<String>, Option<i32>) =
        sqlx::query_as("SELECT username, name, uploaded_avatar_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    let urls = Urls {
        config: ctx.config,
        settings: s,
    };
    let avatar_template =
        avatar::avatar_template(&urls, user_id, &username, uploaded_avatar_id, None)?;

    let mut out = Map::new();
    out.insert("created_at".into(), json!(time_json(object.created_at)));
    out.insert("post_id".into(), json!(access.post.id));
    out.insert("previous_hidden".into(), json!(previous.hidden));
    out.insert("current_hidden".into(), json!(current.hidden));
    out.insert("first_revision".into(), json!(first_revision));
    out.insert("previous_revision".into(), json!(previous_revision));
    out.insert("current_revision".into(), json!(current_revision));
    out.insert("next_revision".into(), json!(next_revision));
    out.insert("last_revision".into(), json!(last_revision));
    out.insert("current_version".into(), json!(current_version));
    out.insert("version_count".into(), json!(revisions.len()));
    out.insert("username".into(), json!(username.to_lowercase()));
    out.insert("display_username".into(), json!(username));
    if s.get("enable_names")?.truthy() {
        out.insert("acting_user_name".into(), json!(name));
    }
    out.insert("avatar_template".into(), json!(avatar_template));
    if staff || current.revision == previous.revision + 1 {
        out.insert("edit_reason".into(), field(current, "edit_reason"));
    }
    let mut diff_error = false;
    let text = |v: Value| v.as_str().unwrap_or("").to_string();
    match body_changes(
        &text(field(previous, "cooked")),
        &text(field(current, "cooked")),
        &text(field(previous, "raw")),
        &text(field(current, "raw")),
    ) {
        Ok(b) => {
            out.insert(
                "body_changes".into(),
                json!({"inline": b.inline, "side_by_side": b.side_by_side, "side_by_side_markdown": b.side_by_side_markdown}),
            );
        }
        Err(_) => {
            out.insert("body_changes".into(), Value::Null);
            diff_error = true;
        }
    }
    if access.post.post_number == 1 {
        let escape = |v: Value| {
            v.as_str()
                .map(crate::discourse_diff::escape_html)
                .unwrap_or_default()
        };
        let prev = format!("<div>{}</div>", escape(field(previous, "title")));
        let cur = format!("<div>{}</div>", escape(field(current, "title")));
        match html_diff(&prev, &cur) {
            Ok((inline, side_by_side)) => {
                out.insert(
                    "title_changes".into(),
                    json!({"inline": inline, "side_by_side": side_by_side}),
                );
            }
            Err(_) => {
                out.insert("title_changes".into(), Value::Null);
                diff_error = true;
            }
        }
    }
    out.insert("can_edit".into(), json!(access.can_edit(ctx, guardian)?));
    if diff_error {
        out.insert("diff_error".into(), json!(true));
    }
    Ok(RevisionResult::Found(Value::Object(out)))
}
