//! Port of TopicsController#update: PUT /t/:topic_id and
//! PUT /t/:slug/:topic_id. The tracked topic fields go through
//! PostRevisor on the first post (crate::posting::revise_topic); the
//! answer is BasicTopicSerializer, with the visible tags when tags were
//! sent.
//!
//! Tags go through PostRevisor's tags field (DiscourseTagging.tag_topic
//! by the names resolved here); PUT /t/:id/tags changes them alone.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::session::{bad_csrf, csrf_ok};
use crate::guardian::Guardian;
use crate::params;
use crate::posting::Ctx;
use crate::posting::revise_topic::{self, Opts, Outcome, TopicFields};
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_guardian::TopicCtx;
use crate::{AppError, AppState, Unsupported};

fn json_error(status: StatusCode, errors: Vec<String>) -> Response {
    (status, Json(json!({"errors": errors}))).into_response()
}

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// PUT /t/:topic_id(.json)
pub async fn update_by_id(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    update(state, guardian, topic_id, headers, uri, body).await
}

/// PUT /t/:slug/:topic_id(.json), and PUT /t/:topic_id/tags(.json)
/// (topics#update_tags), which shares its shape.
pub async fn update_with_slug(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path((slug, topic_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if topic_id == "tags" || topic_id == "tags.json" {
        return update_tags(state, guardian, slug, headers, uri, body).await;
    }
    update(state, guardian, topic_id, headers, uri, body).await
}

/// `topics#update_tags`: the topic's tags alone, for those who may edit
/// them (a wiki's editors too), through PostRevisor unless they stay the
/// same; answered with the basic topic and its visible tags.
async fn update_tags(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    topic_id: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    // `topic_id: /\d+/`
    if topic_id.is_empty() || !topic_id.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(super::topics::not_found_response(&state));
    }
    let topic_id = crate::ruby::to_i(&topic_id) as i32;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let s = &settings;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    // ensure_can_edit_tags!(topic)
    let topic = if guardian.is_authenticated() {
        TopicCtx::load(&mut conn, s, &guardian, topic_id).await?
    } else {
        None
    };
    let can_edit_tags = match &topic {
        Some(topic) => {
            let secure = guardian.secure_category_ids(&mut conn, s).await?;
            let can_see = guardian.can_see_topic(s, topic, true, &secure)?;
            let anywhere = guardian.can_create_post_anywhere(&mut conn, s).await?;
            let can_edit_topic = guardian.can_edit_topic(s, topic, can_see, anywhere)?;
            let can_create_post = guardian.can_create_post(s, topic, anywhere)?;
            guardian.can_edit_tags(s, topic, can_edit_topic, can_create_post)?
        }
        None => false,
    };
    if !can_edit_tags {
        if guardian.is_anonymous() {
            return Ok(super::login_required::not_logged_in(&state));
        }
        return Ok(super::search::invalid_access(&state));
    }
    // reject_too_many_tags!(:tags)
    let max_tags = s.get("max_tags_per_topic")?.to_i();
    if let Some(v) = p.get("tags")
        && tag_param_count(v)? > max_tags
    {
        return Ok(json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            vec![ctx.t_count("tags.too_many_tags_for_topic", max_tags)],
        ));
    }
    // A hash of tags is its values.
    let incoming = Value::Array(revise_topic::tag_items(
        p.get("tags").unwrap_or(&Value::Null),
    )?);
    if !revise_topic::tag_change_noop(&mut conn, topic_id, &incoming).await? {
        // DiscourseTagging.tag_topic's names: a new tag's own, then those
        // of the ids.
        let names = match crate::tagging::parse_param(Some(&incoming))? {
            Some(param) => crate::tagging::names_for_tagging(&mut conn, &param).await?,
            None => Vec::new(),
        };
        drop(conn);
        let fields = TopicFields {
            tags: Some(names),
            ..Default::default()
        };
        match revise_topic::revise(
            &state.pool,
            &ctx,
            &guardian,
            topic_id,
            &fields,
            Opts::default(),
        )
        .await?
        {
            Outcome::Revised => {}
            Outcome::RateLimited(refused) => return Ok(refused.into_response(&state.i18n)),
            // render_json_error(topic): its errors; none renders the topic.
            Outcome::Invalid(errors) if !errors.is_empty() => {
                return Ok(json_error(StatusCode::UNPROCESSABLE_ENTITY, errors));
            }
            Outcome::Invalid(_) => {}
        }
        conn = state.pool.acquire().await?;
    }
    let mut payload = Map::new();
    payload.insert(
        "basic_topic".into(),
        Value::Object(basic_topic(&mut conn, s, topic_id).await?),
    );
    let tags = visible_tags(&mut conn, s, &guardian, topic_id).await?;
    payload.insert("tags".into(), json!(tags));
    Ok((StatusCode::OK, Json(Value::Object(payload))).into_response())
}

/// A scalar param as Ruby reads it; arrays and hashes where a string goes
/// are refused.
fn scalar(p: &Map<String, Value>, key: &str) -> Result<Option<String>, AppError> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => match params::scalar(v) {
            Some(s) => Ok(Some(s)),
            None if v.is_null() => Ok(Some(String::new())),
            None => Err(Unsupported("topics#update params of another shape").into()),
        },
    }
}

async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    topic_id: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let topic_id = topic_id
        .strip_suffix(".json")
        .unwrap_or(&topic_id)
        .to_string();
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    // `topic_id: /\d+/`
    if topic_id.is_empty() || !topic_id.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(super::topics::not_found_response(&state));
    }
    let topic_id = crate::ruby::to_i(&topic_id) as i32;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let s = &settings;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };

    // guardian.ensure_can_edit!(topic): a missing or deleted topic can't
    // be edited either.
    let title: Option<(String, Option<i32>)> = sqlx::query_as(
        "SELECT title, category_id FROM topics WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let can_edit = match &title {
        Some(_) if guardian.is_authenticated() => {
            can_edit_topic(&mut conn, s, &guardian, topic_id).await?
        }
        _ => false,
    };
    if !can_edit {
        if guardian.is_anonymous() {
            return Ok(super::login_required::not_logged_in(&state));
        }
        return Ok(super::search::invalid_access(&state));
    }
    let Some((current_title, current_category)) = title else {
        return Ok(super::search::invalid_access(&state));
    };

    // reject_too_many_tags!(:tags, :original_tags)
    let max_tags = s.get("max_tags_per_topic")?.to_i();
    for key in ["tags", "original_tags"] {
        if let Some(v) = p.get(key)
            && tag_param_count(v)? > max_tags
        {
            return Ok(json_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                vec![ctx.t_count("tags.too_many_tags_for_topic", max_tags)],
            ));
        }
    }

    if let Some(original) = scalar(&p, "original_title")?
        && !original.is_empty()
        && original != current_title
    {
        return Ok(json_error(
            StatusCode::CONFLICT,
            vec![ctx.t("edit_conflict")],
        ));
    }
    if let Some(original_tags) = p.get("original_tags") {
        let mut ids = original_tag_ids(original_tags)?;
        if !ids.is_empty() {
            ids.sort_unstable();
            let current = topic_tag_ids(&mut conn, topic_id).await?;
            if ids != current {
                return Ok(json_error(
                    StatusCode::CONFLICT,
                    vec![ctx.t("edit_conflict")],
                ));
            }
        }
    }

    // changes: the tracked topic fields present in the params, less a
    // title or category that stays the same.
    let mut fields = TopicFields {
        title: scalar(&p, "title")?,
        archetype: scalar(&p, "archetype")?,
        category_id: None,
        featured_link: scalar(&p, "featured_link")?,
        tags: None,
    };
    if fields.title.as_deref() == Some(current_title.as_str()) {
        fields.title = None;
    }
    if let Some(category) = scalar(&p, "category_id")? {
        let category_id = crate::ruby::to_i(&category) as i32;
        if category_id != current_category.unwrap_or(0) {
            match category_change_error(&mut conn, &ctx, &guardian, topic_id, category_id, &p)
                .await?
            {
                CategoryCheck::Ok => fields.category_id = Some(category_id),
                CategoryCheck::NotFound => return Ok(super::topics::not_found_response(&state)),
                CategoryCheck::Error(status, message) => {
                    return Ok(json_error(status, vec![message]));
                }
            }
        }
    }
    // tags: a change unless the same tags come back; with tagging off they
    // still reach the revisor (which can't tag) as a topic change.
    let tags_param = p.contains_key("tags");
    let tags_submitted = tags_param && s.get("tagging_enabled")?.truthy();
    if tags_param {
        let incoming = p.get("tags").cloned().unwrap_or(Value::Null);
        let present = !is_blank(&incoming);
        let keep = if !tags_submitted {
            true
        } else if present {
            !revise_topic::tag_change_noop(&mut conn, topic_id, &incoming).await?
        } else {
            !topic_tag_ids(&mut conn, topic_id).await?.is_empty()
        };
        if keep {
            fields.tags = Some(if present {
                resolve_tag_names(&mut conn, s, &guardian, &incoming).await?
            } else {
                Vec::new()
            });
        }
    }
    drop(conn);

    if !fields.is_empty() {
        let opts = Opts {
            keep_existing_draft: scalar(&p, "keep_existing_draft")?.as_deref() == Some("true"),
            ..Default::default()
        };
        match revise_topic::revise(&state.pool, &ctx, &guardian, topic_id, &fields, opts).await? {
            Outcome::Revised => {}
            Outcome::RateLimited(refused) => return Ok(refused.into_response(&state.i18n)),
            Outcome::Invalid(errors) => {
                let mut errors = if errors.is_empty() {
                    vec![ctx.t("activerecord.errors.models.topic.attributes.base.unable_to_update")]
                } else {
                    errors
                };
                // safe_revision_errors: a topic with tags this user can't
                // see says no more than that when its category changed.
                if fields.category_id.is_some() {
                    let mut conn = state.pool.acquire().await?;
                    let all = topic_tag_ids(&mut conn, topic_id).await?;
                    let visible = visible_tags(&mut conn, s, &guardian, topic_id).await?;
                    if visible.len() != all.len() {
                        errors = vec![ctx.t("category.errors.disallowed_tags_generic")];
                    }
                }
                return Ok(json_error(StatusCode::UNPROCESSABLE_ENTITY, errors));
            }
        }
    }

    let mut conn = state.pool.acquire().await?;
    let mut payload = Map::new();
    payload.insert(
        "basic_topic".into(),
        Value::Object(basic_topic(&mut conn, s, topic_id).await?),
    );
    if tags_submitted {
        let tags = visible_tags(&mut conn, s, &guardian, topic_id).await?;
        payload.insert("tags".into(), json!(tags));
    }
    Ok((StatusCode::OK, Json(Value::Object(payload))).into_response())
}

enum CategoryCheck {
    Ok,
    /// `Category.find` raising RecordNotFound.
    NotFound,
    Error(StatusCode, String),
}

/// `TopicCategoryChangeValidator.call`: the category exists, the user may
/// move the topic there, and its tags are allowed there (only checked
/// where tag groups or category tags exist, which wait for 189).
async fn category_change_error(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    topic_id: i32,
    category_id: i32,
    p: &Map<String, Value>,
) -> Result<CategoryCheck, AppError> {
    let s = ctx.settings;
    let shared_draft: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM shared_drafts WHERE topic_id = $1)")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
    if shared_draft {
        return Err(Unsupported("moving a shared draft").into());
    }
    let exists = |id: i32| {
        sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM categories WHERE id = $1)")
            .bind(id)
    };
    let category = if category_id == 0 {
        None
    } else {
        Some(category_id)
    };
    if let Some(id) = category
        && !exists(id).fetch_one(&mut *conn).await?
    {
        return Ok(CategoryCheck::Error(
            StatusCode::UNPROCESSABLE_ENTITY,
            ctx.t("category.errors.not_found"),
        ));
    }
    // ensure_can_move_topic_to_category!: no category means
    // `Category.find(uncategorized_category_id)`.
    let target = match category {
        Some(id) => id,
        None => {
            let uncategorized = s.get("uncategorized_category_id")?.to_i() as i32;
            if !exists(uncategorized).fetch_one(&mut *conn).await? {
                return Ok(CategoryCheck::NotFound);
            }
            uncategorized
        }
    };
    let can_move = guardian.is_staff()
        || (revise_topic::can_create_topic_on_category(&mut *conn, s, guardian, target).await?
            && !crate::posting::validate::posting_review_required(
                &mut *conn,
                guardian,
                target,
                crate::posting::validate::REVIEW_TOPIC,
            )
            .await?);
    if !can_move {
        return Ok(CategoryCheck::Error(
            StatusCode::FORBIDDEN,
            ctx.t("category.errors.move_topic_to_category_disallowed"),
        ));
    }
    if category.is_none() {
        return Ok(CategoryCheck::Ok);
    }
    // resolve_tag_names: the tags sent, else the topic's.
    let tags_changed = p.contains_key("tags");
    let tag_names: Vec<String> = match p.get("tags") {
        Some(v) if !is_blank(v) => resolve_tag_names(&mut *conn, s, guardian, v).await?,
        Some(_) => Vec::new(),
        None => {
            sqlx::query_scalar(
                "SELECT tags.name FROM topic_tags JOIN tags ON tags.id = topic_tags.tag_id \
                 WHERE topic_tags.topic_id = $1 ORDER BY topic_tags.id",
            )
            .bind(topic_id)
            .fetch_all(&mut *conn)
            .await?
        }
    };
    if tag_names.is_empty() {
        return Ok(CategoryCheck::Ok);
    }
    let Some(target) = crate::tagging::TagCategory::load(&mut *conn, target).await? else {
        return Ok(CategoryCheck::Ok);
    };
    let (allowed, _) = crate::tagging::filter_allowed_tags(
        &mut *conn,
        s,
        &crate::tagging::FilterOpts {
            guardian: Some(guardian),
            category: Some(&target),
            ..Default::default()
        },
    )
    .await?;
    let allowed: Vec<String> = allowed.into_iter().map(|t| t.name).collect();
    let hidden = crate::tagging::hidden_tag_names(&mut *conn, guardian).await?;
    let invalid: Vec<String> = tag_names
        .iter()
        .filter(|n| !allowed.contains(n) && (tags_changed || !hidden.contains(n)))
        .map(|n| n.to_lowercase())
        .collect();
    // Tag.where_name: the tags that exist, by their own names.
    let invalid: Vec<String> = if invalid.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar("SELECT name FROM tags WHERE lower(name) = ANY($1) ORDER BY ctid")
            .bind(&invalid)
            .fetch_all(&mut *conn)
            .await?
    };
    if invalid.is_empty() {
        return Ok(CategoryCheck::Ok);
    }
    let message = if invalid.iter().any(|n| hidden.contains(n)) {
        ctx.t("category.errors.disallowed_tags_generic")
    } else {
        ctx.i18n
            .t_with(
                "category.errors.disallowed_topic_tags",
                &[("tags", &invalid.join(", "))],
            )
            .ok_or(Unsupported("category.errors.disallowed_topic_tags"))?
            .to_string()
    };
    Ok(CategoryCheck::Error(
        StatusCode::UNPROCESSABLE_ENTITY,
        message,
    ))
}

/// `guardian.can_edit?(topic)` for a topic that exists.
pub(crate) async fn can_edit_topic(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    topic_id: i32,
) -> Result<bool, AppError> {
    let Some(topic) = TopicCtx::load(&mut *conn, s, guardian, topic_id).await? else {
        return Ok(false);
    };
    let secure = guardian.secure_category_ids(&mut *conn, s).await?;
    let can_see = guardian.can_see_topic(s, &topic, true, &secure)?;
    let can_post_anywhere = guardian.can_create_post_anywhere(&mut *conn, s).await?;
    Ok(guardian.can_edit_topic(s, &topic, can_see, can_post_anywhere)?)
}

/// `tag_param_count`: an array's length, a hash's size, otherwise one.
fn tag_param_count(v: &Value) -> Result<i64, AppError> {
    Ok(match v {
        Value::Array(a) => a.len() as i64,
        Value::Object(o) => o.len() as i64,
        Value::Null => 0,
        Value::String(s) if s.is_empty() => 0,
        _ => 1,
    })
}

/// `present?` on a param.
fn is_blank(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// `resolve_tag_names`: names as sent (blank ones dropped), or tag
/// objects as their names: a new tag's own, then the visible tags of the
/// ids.
pub(crate) async fn resolve_tag_names(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    incoming: &Value,
) -> Result<Vec<String>, AppError> {
    let items = revise_topic::tag_items(incoming)?;
    if items.first().is_some_and(Value::is_string) {
        return Ok(items
            .iter()
            .filter_map(Value::as_str)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .collect());
    }
    let mut names = Vec::new();
    let mut ids = Vec::new();
    for item in &items {
        match item
            .get("id")
            .and_then(params::scalar)
            .filter(|i| !i.trim().is_empty())
        {
            Some(id) => ids.push(crate::ruby::to_i(&id) as i32),
            None => {
                if let Some(name) = item
                    .get("name")
                    .and_then(params::scalar)
                    .filter(|n| !n.trim().is_empty())
                {
                    names.push(name);
                }
            }
        }
    }
    if !ids.is_empty() {
        // Tag.visible(guardian): not one of the tags hidden from them.
        let hidden = crate::tagging::hidden_tag_names(&mut *conn, guardian).await?;
        let found: Vec<String> =
            sqlx::query_scalar("SELECT name FROM tags WHERE id = ANY($1) ORDER BY ctid")
                .bind(&ids)
                .fetch_all(&mut *conn)
                .await?;
        names.extend(found.into_iter().filter(|n| !hidden.contains(n)));
    }
    let _ = s;
    Ok(names)
}

/// `params[:original_tags]&.map { |t| t["id"] }`: the ids of tag objects.
fn original_tag_ids(v: &Value) -> Result<Vec<i32>, AppError> {
    let Value::Array(items) = v else {
        return Err(Unsupported("original_tags of another shape").into());
    };
    items
        .iter()
        .map(|t| match t.get("id").and_then(params::scalar) {
            Some(id) => Ok(crate::ruby::to_i(&id) as i32),
            None => Err(Unsupported("original_tags without ids").into()),
        })
        .collect()
}

pub(crate) async fn topic_tag_ids(
    conn: &mut PgConnection,
    topic_id: i32,
) -> Result<Vec<i32>, sqlx::Error> {
    sqlx::query_scalar("SELECT tag_id FROM topic_tags WHERE topic_id = $1 ORDER BY tag_id")
        .bind(topic_id)
        .fetch_all(conn)
        .await
}

/// BasicTopicSerializer: the fancy title computed and stored when the
/// save left it blank.
async fn basic_topic(
    conn: &mut PgConnection,
    s: &SiteSettings,
    topic_id: i32,
) -> Result<Map<String, Value>, AppError> {
    let (title, fancy, slug, posts_count): (String, Option<String>, Option<String>, i32) =
        sqlx::query_as("SELECT title, fancy_title, slug, posts_count FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
    let fancy =
        crate::topic_query::fancy_title(&mut *conn, s, topic_id, &title, fancy.as_deref()).await?;
    let slug = slug.ok_or(Unsupported("a topic without its slug"))?;
    let mut out = Map::new();
    out.insert("fancy_title".into(), json!(fancy));
    out.insert("id".into(), json!(topic_id));
    out.insert("title".into(), json!(title));
    out.insert("slug".into(), json!(slug));
    out.insert("posts_count".into(), json!(posts_count));
    Ok(out)
}

/// `topic.visible_tags(guardian)` as render_topic_with_tags lists them.
pub(crate) async fn visible_tags(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    topic_id: i32,
) -> Result<Vec<Value>, AppError> {
    let tags: Vec<crate::tags::Tag> = sqlx::query_as(
        "SELECT tags.id, tags.name, tags.slug, tags.description, tags.description_cooked, \
                tags.public_topic_count, tags.staff_topic_count, tags.pm_topic_count, tags.target_tag_id \
         FROM tags JOIN topic_tags ON topic_tags.tag_id = tags.id \
         WHERE topic_tags.topic_id = $1 ORDER BY tags.id",
    )
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?;
    let _ = s;
    let mut out = Vec::new();
    for tag in tags {
        if tag.visible_to(&mut *conn, guardian).await? {
            out.push(json!({"id": tag.id, "name": tag.name, "slug": tag.slug_for_url()}));
        }
    }
    Ok(out)
}
