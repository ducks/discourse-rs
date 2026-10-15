//! Port of TopicsController#bulk (PUT /topics/bulk), #reset_new (PUT
//! /topics/reset-new) and #private_message_reset_new (PUT
//! /topics/pm-reset-new), over crate::topics_bulk.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::session::{bad_csrf, csrf_ok};
use crate::guardian::Guardian;
use crate::params;
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_query::TopicQuery;
use crate::topics_bulk::{BulkAction, Performed};
use crate::{AppError, AppState, Unsupported};

/// `TopicsController::MAX_BULK_TOPIC_IDS`
const MAX_BULK_TOPIC_IDS: usize = 1_000;

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn present(p: &Map<String, Value>, key: &str) -> Option<String> {
    p.get(key)
        .and_then(params::scalar)
        .filter(|v| !v.trim().is_empty())
}

/// `params[key]` as Ruby's truthiness sees a param: given and not nil.
fn given(p: &Map<String, Value>, key: &str) -> bool {
    p.get(key).is_some_and(|v| !v.is_null())
}

/// `params[:topic_ids]`: None when blank, Err when it isn't a list.
fn topic_ids(p: &Map<String, Value>) -> Result<Option<Vec<i32>>, ()> {
    match p.get("topic_ids") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) if items.is_empty() => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::Array(items)) => Ok(Some(
            items
                .iter()
                .map(|v| params::scalar(v).map_or(0, |s| crate::ruby::to_i(&s) as i32))
                .collect(),
        )),
        Some(_) => Err(()),
    }
}

fn not_a_list(state: &AppState) -> Response {
    super::search::invalid_parameters(state, "Expecting topic_ids to contain a list of topic ids")
}

struct Request {
    p: Map<String, Value>,
    settings: SiteSettings,
}

async fn begin(
    state: &AppState,
    guardian: &Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
) -> Result<Result<Request, Response>, AppError> {
    let p = params::parse(uri.query(), headers, body);
    if !csrf_ok(state, headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(Err(bad_csrf()));
    }
    if guardian.is_anonymous() {
        return Ok(Err(super::login_required::not_logged_in(state)));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(Ok(Request { p, settings }))
}

/// Runs an operation in its own transaction.
async fn run(
    state: &AppState,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    topic_ids: Vec<i32>,
    operation: Map<String, Value>,
) -> Result<Performed, AppError> {
    let mut tx = state.pool.begin().await?;
    let performed = BulkAction::new(&mut tx, ctx, guardian, topic_ids, operation)
        .perform()
        .await?;
    tx.commit().await?;
    Ok(performed)
}

fn operation(kind: &str) -> Map<String, Value> {
    let mut op = Map::new();
    op.insert("type".into(), json!(kind));
    op
}

/// PUT /topics/bulk(.json)
pub async fn bulk(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &guardian, &headers, &uri, &body).await? {
        Ok(req) => req,
        Err(response) => return Ok(response),
    };
    let (p, settings) = (req.p, req.settings);
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let ids = match topic_ids(&p) {
        Err(()) => return Ok(not_a_list(&state)),
        Ok(Some(ids)) => {
            let mut unique: Vec<i32> = Vec::new();
            for id in ids {
                if !unique.contains(&id) {
                    unique.push(id);
                }
            }
            if unique.len() > MAX_BULK_TOPIC_IDS {
                let message = ctx
                    .i18n
                    .t_with(
                        "topics_bulk_action.too_many_topic_ids",
                        &[("limit", &MAX_BULK_TOPIC_IDS.to_string())],
                    )
                    .unwrap_or_default();
                return Ok(super::search::invalid_parameters(&state, &message));
            }
            unique
        }
        Ok(None) if present(&p, "filter").as_deref() == Some("unread") => {
            let mut conn = state.pool.acquire().await?;
            bulk_unread_topic_ids(&mut conn, &settings, &guardian, &p).await?
        }
        Ok(None) => return Ok(super::accounts::param_missing("topic_ids")),
    };
    let operation = match p.get("operation") {
        Some(Value::Object(op)) if !op.is_empty() => op.clone(),
        _ => return Ok(super::accounts::param_missing("operation")),
    };
    if operation
        .get("type")
        .and_then(params::scalar)
        .is_none_or(|t| t.trim().is_empty())
    {
        return Ok(super::accounts::param_missing("operation_type"));
    }
    for key in [
        "tag_ids",
        "tags",
        "add_tag_ids",
        "remove_tag_ids",
        "replace_tags",
    ] {
        if operation.contains_key(key) {
            return Err(Unsupported("tags in topic bulk actions (189)").into());
        }
    }
    if operation.contains_key("group") {
        return Err(Unsupported("bulk actions on a group's messages").into());
    }
    match run(&state, &ctx, &guardian, ids, operation).await? {
        Performed::InvalidParameters(message) => {
            Ok((StatusCode::BAD_REQUEST, Json(json!({"errors": [message]}))).into_response())
        }
        Performed::Done { topic_ids, errors } => {
            let mut result = Map::new();
            result.insert("topic_ids".into(), json!(topic_ids));
            if !errors.is_empty() {
                let errors: Map<String, Value> =
                    errors.into_iter().map(|(m, c)| (m, json!(c))).collect();
                result.insert("errors".into(), Value::Object(errors));
            }
            Ok((StatusCode::OK, Json(Value::Object(result))).into_response())
        }
    }
}

/// `bulk_unread_topic_ids`: the user's unread regular topics, narrowed
/// to tracked ones, a category (and its subcategories) or a tag.
async fn bulk_unread_topic_ids(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    p: &Map<String, Value>,
) -> Result<Vec<i32>, AppError> {
    if given(p, "private_message_inbox") {
        return Err(Unsupported("bulk actions over a message inbox's unread topics").into());
    }
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("bulk actions anonymously"))?;
    let column = if guardian.is_whisperer(s)? {
        "highest_staff_post_number"
    } else {
        "highest_post_number"
    };
    let mut clauses = vec![
        "topics.deleted_at IS NULL".to_string(),
        format!(
            "(tu.last_read_post_number < topics.{column}) AND (COALESCE(tu.notification_level, 1) >= 2)"
        ),
        "topics.archetype <> 'private_message'".to_string(),
    ];
    if present(p, "tracked").as_deref() == Some("true") {
        clauses.push(tracked_clause(s, user_id)?);
    }
    if let Some(category) = p.get("category_id").and_then(params::scalar) {
        let ids = if given(p, "include_subcategories") {
            subcategory_ids(conn, s, crate::ruby::to_i(&category) as i32).await?
        } else {
            vec![crate::ruby::to_i(&category) as i32]
        };
        clauses.push(format!("topics.category_id IN ({})", join_ids(&ids)));
    }
    if let Some(tag) = present(p, "tag_name") {
        clauses.push(format!(
            "topics.id IN (SELECT tt.topic_id FROM topic_tags tt JOIN tags ON tags.id = tt.tag_id \
             WHERE tags.name = '{}')",
            tag.replace('\'', "''")
        ));
    }
    Ok(sqlx::query_scalar(&format!(
        "SELECT topics.id FROM topics \
         LEFT OUTER JOIN topic_users AS tu ON (topics.id = tu.topic_id AND tu.user_id = {user_id}) \
         WHERE {}",
        clauses.join(" AND ")
    ))
    .fetch_all(&mut *conn)
    .await?)
}

fn join_ids(ids: &[i32]) -> String {
    if ids.is_empty() {
        return "NULL".into();
    }
    ids.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// `TopicQuery.tracked_filter`: topics in categories (or under ones) the
/// user tracks or watches, or with tags they do.
fn tracked_clause(s: &SiteSettings, user_id: i32) -> Result<String, AppError> {
    if s.get("max_category_nesting")?.to_i() == 3 {
        return Err(Unsupported("the tracked filter with three category levels").into());
    }
    let tracked = format!(
        "SELECT cd.category_id FROM category_users cd WHERE cd.user_id = {user_id} AND cd.notification_level >= 2"
    );
    let mut sql = format!(
        "(topics.category_id IN (SELECT c.id FROM categories c WHERE (c.id IN ({tracked})) \
           OR c.parent_category_id IN ({tracked}))"
    );
    if s.get("tagging_enabled")?.truthy() {
        sql.push_str(&format!(
            " OR topics.id IN (SELECT tt.topic_id FROM topic_tags tt WHERE tt.tag_id IN \
               (SELECT tu.tag_id FROM tag_users tu WHERE tu.user_id = {user_id} AND tu.notification_level >= 2))"
        ));
    }
    sql.push(')');
    Ok(sql)
}

/// `Category.subcategory_ids(id)`
async fn subcategory_ids(
    conn: &mut PgConnection,
    s: &SiteSettings,
    category_id: i32,
) -> Result<Vec<i32>, AppError> {
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE subcategories AS ( \
             SELECT $1::int AS id, 1 AS depth \
             UNION \
             SELECT categories.id, subcategories.depth + 1 \
             FROM categories JOIN subcategories ON subcategories.id = categories.parent_category_id \
             WHERE subcategories.depth < $2) \
         SELECT id FROM subcategories",
    )
    .bind(category_id)
    .bind(s.get("max_category_nesting")?.to_i() as i32)
    .fetch_all(&mut *conn)
    .await?)
}

fn changed_ids(performed: Performed) -> Result<Vec<i32>, AppError> {
    match performed {
        Performed::Done { topic_ids, .. } => Ok(topic_ids),
        Performed::InvalidParameters(_) => {
            Err(Unsupported("a dismissal refused its own parameters").into())
        }
    }
}

/// PUT /topics/reset-new(.json)
pub async fn reset_new(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &guardian, &headers, &uri, &body).await? {
        Ok(req) => req,
        Err(response) => return Ok(response),
    };
    let (p, settings) = (req.p, req.settings);
    let s = &settings;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("dismissing anonymously"))?;
    let mut conn = state.pool.acquire().await?;
    let unified = guardian
        .upcoming_change_enabled(&mut conn, s, "enable_unified_new")
        .await?;
    let dismiss_topics = given(&p, "dismiss_topics");
    let dismiss_posts = given(&p, "dismiss_posts");
    let (new, unread) = if unified {
        (dismiss_topics, dismiss_posts)
    } else {
        (true, false)
    };
    // The tag, when the user can see it.
    let tag_name = match p.get("tag_name").and_then(params::scalar) {
        Some(name) => {
            let visible = crate::tags::visible_tags_where(&guardian, s)?;
            sqlx::query_scalar::<_, String>(&format!(
                "SELECT tags.name FROM tags WHERE {visible} AND tags.name = $1 LIMIT 1"
            ))
            .bind(&name)
            .fetch_optional(&mut *conn)
            .await?
        }
        None => None,
    };
    let tag_clause = |name: &str| {
        format!(
            "topics.id IN (SELECT tt.topic_id FROM topic_tags tt JOIN tags ON tags.id = tt.tag_id \
             WHERE tags.name = '{}')",
            name.replace('\'', "''")
        )
    };
    let mut clauses: Vec<String> = Vec::new();
    if let Some(category) = present(&p, "category_id") {
        let category_id = crate::ruby::to_i(&category) as i32;
        let include = p
            .get("include_subcategories")
            .and_then(params::scalar)
            .is_some_and(|v| cast_bool(&v));
        let mut ids = if include {
            subcategory_ids(&mut conn, s, category_id).await?
        } else {
            vec![category_id]
        };
        let allowed = guardian.allowed_category_ids(&mut conn, s).await?;
        ids.retain(|id| allowed.contains(id));
        if ids.is_empty() {
            clauses.push("FALSE".into());
        } else {
            clauses.push(format!("topics.category_id IN ({})", join_ids(&ids)));
            if let Some(tag) = &tag_name {
                clauses.push(tag_clause(tag));
            }
        }
    } else if let Some(tag) = &tag_name {
        clauses.push(tag_clause(tag));
    } else if present(&p, "tracked").as_deref() == Some("true") {
        clauses.push(tracked_clause(s, user_id)?);
    } else {
        sqlx::query("UPDATE user_stats SET new_since = clock_timestamp() WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *conn)
            .await?;
    }
    match topic_ids(&p) {
        Err(()) => return Ok(not_a_list(&state)),
        Ok(Some(ids)) => clauses.push(format!("topics.id IN ({})", join_ids(&ids))),
        Ok(None) => {}
    }
    let extra = if clauses.is_empty() {
        "TRUE".to_string()
    } else {
        clauses.join(" AND ")
    };
    let scope = {
        let mut query = TopicQuery {
            conn: &mut conn,
            settings: s,
            guardian: &guardian,
            options: Default::default(),
            category: Default::default(),
            tags: Default::default(),
            filter: Default::default(),
            user: Default::default(),
        };
        query.new_and_unread_ids(new, unread, &extra).await?
    };
    drop(conn);
    let mut dismissed: Vec<i32> = Vec::new();
    if !unified || dismiss_topics {
        let performed = run(
            &state,
            &ctx,
            &guardian,
            scope.clone(),
            operation("dismiss_topics"),
        )
        .await?;
        dismissed = changed_ids(performed)?;
    }
    if dismiss_posts {
        let op = if given(&p, "untrack") {
            let mut op = operation("change_notification_level");
            op.insert(
                "notification_level_id".into(),
                json!(crate::posting::notification_levels::REGULAR),
            );
            op
        } else {
            operation("dismiss_posts")
        };
        let performed = run(&state, &ctx, &guardian, scope, op).await?;
        for id in changed_ids(performed)? {
            if !dismissed.contains(&id) {
                dismissed.push(id);
            }
        }
    }
    Ok((StatusCode::OK, Json(json!({"topic_ids": dismissed}))).into_response())
}

/// PUT /topics/pm-reset-new(.json)
pub async fn pm_reset_new(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &guardian, &headers, &uri, &body).await? {
        Ok(req) => req,
        Err(response) => return Ok(response),
    };
    let (p, settings) = (req.p, req.settings);
    let s = &settings;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("dismissing anonymously"))?;
    // private_messages_for(user, :all | :user)
    let personal = format!(
        "topics.id IN (SELECT topic_id FROM topic_allowed_users WHERE user_id = {user_id})"
    );
    let groups = format!(
        "topics.id IN (SELECT tg.topic_id FROM topic_allowed_groups tg \
           JOIN group_users gu ON gu.user_id = {user_id} AND gu.group_id = tg.group_id)"
    );
    let all = format!("({personal} OR {groups})");
    let mut conn = state.pool.acquire().await?;
    let scope: Vec<i32> = match topic_ids(&p) {
        Err(()) => return Ok(not_a_list(&state)),
        Ok(Some(ids)) => {
            sqlx::query_scalar(&format!(
                "SELECT topics.id FROM topics WHERE topics.deleted_at IS NULL \
               AND topics.archetype = 'private_message' AND {all} AND topics.id IN ({})",
                join_ids(&ids)
            ))
            .fetch_all(&mut *conn)
            .await?
        }
        Ok(None) => {
            let Some(inbox) = present(&p, "inbox") else {
                return Ok(super::accounts::param_missing("inbox"));
            };
            let messages = match inbox.as_str() {
                "group" => {
                    return Err(Unsupported("dismissing a group inbox's new messages").into());
                }
                "user" => personal,
                _ => all,
            };
            // filter_private_message_new
            let start = guardian.treat_as_new_topic_start_date(&mut conn, s).await?;
            let start = match start {
                Some(t) => format!(
                    "topics.created_at >= '{}'",
                    t.format("%Y-%m-%d %H:%M:%S%.6f")
                ),
                None => "FALSE".to_string(),
            };
            sqlx::query_scalar(&format!(
                "SELECT topics.id FROM topics \
                 LEFT OUTER JOIN topic_users AS tu ON (topics.id = tu.topic_id AND tu.user_id = {user_id}) \
                 WHERE topics.deleted_at IS NULL AND topics.archetype = 'private_message' AND {messages} \
                   AND ({start}) AND (tu.last_read_post_number IS NULL) \
                   AND (COALESCE(tu.notification_level, 2) >= 2)"
            ))
            .fetch_all(&mut *conn)
            .await?
        }
    };
    drop(conn);
    let performed = run(&state, &ctx, &guardian, scope, operation("dismiss_topics")).await?;
    let topic_ids = changed_ids(performed)?;
    Ok((
        StatusCode::OK,
        Json(json!({"success": "OK", "topic_ids": topic_ids})),
    )
        .into_response())
}

/// `ActiveModel::Type::Boolean.new.cast`: everything but the false values
/// is true.
fn cast_bool(value: &str) -> bool {
    !matches!(
        value,
        "" | "0" | "f" | "F" | "false" | "FALSE" | "off" | "OFF"
    )
}
