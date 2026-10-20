//! SidebarSectionsController: custom and public sidebar sections with
//! their links (SidebarSection, SidebarUrl and SidebarSectionLink with
//! the nested attributes, SidebarSectionUpdater, the ReorderLinks and
//! MoveLink services, reset_community!), and their staff logs.
//!
//! Refused: content localization (the port refuses the setting).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

const MAX_TITLE_LENGTH: usize = 30;
const MAX_ICON_LENGTH: usize = 40;
const MAX_NAME_LENGTH: usize = 80;
const MAX_VALUE_LENGTH: usize = 1000;

/// SidebarUrl::COMMUNITY_SECTION_LINKS: name, path, icon, segment.
const COMMUNITY_SECTION_LINKS: [(&str, &str, &str, i32); 12] = [
    ("Topics", "/latest", "layer-group", 0),
    ("My posts", "/my/activity", "user", 0),
    ("My messages", "/my/messages", "inbox", 0),
    ("Review", "/review", "flag", 0),
    ("Admin", "/admin", "wrench", 0),
    ("Invite", "/new-invite", "paper-plane", 0),
    ("Users", "/u", "users", 1),
    ("About", "/about", "circle-info", 1),
    ("FAQ", "/faq", "circle-question", 1),
    ("Groups", "/g", "user-group", 1),
    ("Badges", "/badges", "certificate", 1),
    ("Filter", "/filter", "filter", 1),
];

fn pairs(p: &Map<String, Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn id_of(raw: &str) -> i64 {
    crate::ruby::to_i(raw.strip_suffix(".json").unwrap_or(raw))
}

fn failed(status: StatusCode) -> Response {
    (status, Json(json!({"failed": "FAILED"}))).into_response()
}

fn json_error(message: String) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"errors": [message]})),
    )
        .into_response()
}

/// `ActiveModel::Type::Boolean.new.cast`
fn cast_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(*b),
        other => {
            let s = crate::params::scalar(other).unwrap_or_default();
            if s.is_empty() {
                None
            } else {
                Some(!matches!(
                    s.as_str(),
                    "0" | "f" | "F" | "false" | "FALSE" | "off" | "OFF"
                ))
            }
        }
    }
}

#[derive(sqlx::FromRow, Clone)]
struct Section {
    id: i32,
    title: String,
    user_id: i32,
    public: bool,
    section_type: Option<i32>,
    locale: Option<String>,
}

async fn find_section(conn: &mut PgConnection, id: i64) -> Result<Option<Section>, sqlx::Error> {
    let Ok(id) = i32::try_from(id) else {
        return Ok(None);
    };
    sqlx::query_as(
        "SELECT id::int AS id, title, user_id, public, section_type, locale FROM sidebar_sections WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await
}

/// SidebarGuardian#can_edit_sidebar_section?
fn can_edit(guardian: &Guardian, s: &Section) -> bool {
    if s.public || s.section_type.is_some() {
        return guardian.is_admin();
    }
    guardian.user_id() == Some(s.user_id)
}

/// SidebarGuardian#can_delete_sidebar_section?
fn can_delete(guardian: &Guardian, s: &Section) -> bool {
    if s.section_type.is_some() {
        return false;
    }
    if s.public {
        return guardian.is_admin();
    }
    guardian.user_id() == Some(s.user_id)
}

#[derive(sqlx::FromRow, Clone)]
struct Url {
    id: i32,
    name: String,
    value: String,
    icon: String,
    external: bool,
    segment: i32,
    locale: Option<String>,
}

/// `section.sidebar_urls`: through the links, by position.
async fn urls_of(conn: &mut PgConnection, section_id: i32) -> Result<Vec<Url>, sqlx::Error> {
    sqlx::query_as(
        "SELECT u.id::int AS id, u.name, u.value, u.icon, u.external, u.segment, u.locale FROM sidebar_urls u \
         JOIN sidebar_section_links l ON l.linkable_id = u.id AND l.linkable_type = 'SidebarUrl' \
         WHERE l.sidebar_section_id = $1 ORDER BY l.position",
    )
    .bind(section_id)
    .fetch_all(&mut *conn)
    .await
}

/// SidebarSectionSerializer (and the edit one's `can_localize`).
async fn serialize(conn: &mut PgConnection, id: i32, edit: bool) -> Result<Value, AppError> {
    let s = find_section(conn, i64::from(id))
        .await?
        .ok_or(Unsupported("a sidebar section gone once saved"))?;
    let links: Vec<Value> = urls_of(conn, s.id)
        .await?
        .into_iter()
        .map(|u| {
            let mut link = json!({
                "id": u.id,
                "name": u.name,
                "value": u.value,
                "icon": u.icon,
                "external": u.external,
                "segment": if u.segment == 0 { "primary" } else { "secondary" },
                "locale": u.locale,
            });
            if edit {
                link["can_localize"] = Value::Bool(false);
            }
            link
        })
        .collect();
    Ok(json!({
        "id": s.id,
        "title": s.title,
        "links": links,
        "slug": crate::site::parameterize(&s.title),
        "public": s.public,
        "section_type": s.section_type.map(|_| "community"),
        "locale": s.locale,
    }))
}

async fn rendered(conn: &mut PgConnection, id: i32) -> Result<Response, AppError> {
    Ok(Json(json!({"sidebar_section": serialize(conn, id, false).await?})).into_response())
}

async fn log(
    conn: &mut PgConnection,
    action: &str,
    acting: i32,
    subject: &str,
    details: Option<&str>,
) -> Result<(), AppError> {
    let action = crate::staff_action_logs::action_id(action)
        .ok_or(Unsupported("a staff action missing from enums.json"))?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, subject, details, admin_only, \
         created_at, updated_at) VALUES ($1, $2, $3, $4, TRUE, now(), now())",
    )
    .bind(action)
    .bind(acting)
    .bind(subject)
    .bind(details)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `custom_section_details`
fn details(urls: &[(String, String)]) -> String {
    let parts: Vec<String> = urls.iter().map(|(n, v)| format!("{n} - {v}")).collect();
    format!("links: {}", parts.join(", "))
}

/// One `links` entry, as permitted.
#[derive(Clone, Default)]
struct LinkParams {
    id: Option<String>,
    destroy: bool,
    icon: Option<String>,
    name: Option<String>,
    value: Option<String>,
    segment: Option<String>,
}

fn links_params(p: &Map<String, Value>) -> Option<Vec<LinkParams>> {
    let items: Vec<&Value> = match p.get("links")? {
        Value::Array(a) => a.iter().collect(),
        Value::Object(m) => m.values().collect(),
        _ => return None,
    };
    Some(
        items
            .into_iter()
            .filter_map(Value::as_object)
            .map(|l| {
                let s = |k: &str| l.get(k).and_then(crate::params::scalar);
                LinkParams {
                    id: s("id").filter(|v| !v.is_empty()),
                    destroy: l.get("_destroy").and_then(cast_bool).unwrap_or(false),
                    icon: s("icon"),
                    name: s("name"),
                    value: s("value"),
                    segment: s("segment"),
                }
            })
            .collect(),
    )
}

/// A link as it will be saved: an existing url's id or a new one.
#[derive(Clone)]
struct Link {
    id: Option<i32>,
    icon: String,
    name: String,
    value: String,
    segment: i32,
    locale: String,
    destroy: bool,
    changed: bool,
}

impl Link {
    fn external(&self) -> bool {
        self.value.starts_with("http://") || self.value.starts_with("https://")
    }

    /// SidebarUrl's validations, after its before_validation hooks.
    fn errors(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut check = |attr: &str, v: &str, max: usize| {
            if crate::ruby::is_blank(v) {
                out.push(format!("Sidebar urls {attr} can't be blank"));
            }
            if v.chars().count() > max {
                out.push(format!(
                    "Sidebar urls {attr} is too long (maximum is {max} characters)"
                ));
            }
        };
        check("icon", &self.icon, MAX_ICON_LENGTH);
        check("name", &self.name, MAX_NAME_LENGTH);
        check("value", &self.value, MAX_VALUE_LENGTH);
        out
    }
}

fn segment_of(s: Option<&str>) -> Result<Option<i32>, Unsupported> {
    match s {
        None => Ok(None),
        Some("primary") | Some("0") => Ok(Some(0)),
        Some("secondary") | Some("1") => Ok(Some(1)),
        Some(_) => Err(Unsupported(
            "a sidebar link segment that is not an enum value",
        )),
    }
}

/// `remove_internal_hostname`: `\Ahttp(s)?://<hostname>` taken off (the
/// hostname as a pattern, unanchored at its end).
fn remove_internal_hostname(value: &str, hostname: &str) -> String {
    let re = regex::Regex::new(&format!("^https?://{hostname}"));
    match re {
        Ok(re) => re.replace(value, "").into_owned(),
        Err(_) => value.to_string(),
    }
}

struct Ctx {
    hostname: String,
    default_locale: String,
    max_links: i64,
}

async fn ctx(state: &AppState, conn: &mut PgConnection) -> Result<(SiteSettings, Ctx), AppError> {
    let settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    if settings.get("content_localization_enabled")?.truthy() {
        return Err(Unsupported("content_localization_enabled").into());
    }
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let hostname = urls.current_hostname()?;
    let c = Ctx {
        hostname,
        default_locale: settings.get("default_locale")?.to_s(),
        max_links: settings.get("max_sidebar_section_links")?.to_i(),
    };
    Ok((settings, c))
}

/// The section's validations (title presence, uniqueness per user and
/// length), after its links': the first message, as the controller shows.
async fn first_error(
    conn: &mut PgConnection,
    title: &str,
    validation_user: Option<i32>,
    own_id: Option<i32>,
    links: &[Link],
    limit_error: Option<String>,
) -> Result<Option<String>, sqlx::Error> {
    for link in links
        .iter()
        .filter(|l| !l.destroy && (l.id.is_none() || l.changed))
    {
        if let Some(e) = link.errors().into_iter().next() {
            return Ok(Some(e));
        }
        if link.external() && !uri_regexp_matches(&link.value) {
            return Ok(Some("Sidebar urls value is not valid".into()));
        }
    }
    if crate::ruby::is_blank(title) {
        return Ok(Some("Title can't be blank".into()));
    }
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sidebar_sections WHERE title = $1 \
         AND user_id IS NOT DISTINCT FROM $2 AND id IS DISTINCT FROM $3)",
    )
    .bind(title)
    .bind(validation_user)
    .bind(own_id)
    .fetch_one(&mut *conn)
    .await?;
    if taken {
        return Ok(Some("Title has already been taken".into()));
    }
    if title.chars().count() > MAX_TITLE_LENGTH {
        return Ok(Some(format!(
            "Title is too long (maximum is {MAX_TITLE_LENGTH} characters)"
        )));
    }
    Ok(limit_error)
}

/// `Discourse::Utils::URI_REGEXP` (URI.regexp for http and https),
/// unanchored: an http(s) scheme followed by an authority or path.
fn uri_regexp_matches(value: &str) -> bool {
    ["http://", "https://"].iter().any(|scheme| {
        value
            .match_indices(scheme)
            .any(|(i, _)| value[i + scheme.len()..].chars().next().is_some())
    })
}

async fn insert_url(conn: &mut PgConnection, l: &Link) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO sidebar_urls (name, value, icon, external, segment, locale, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, now(), now()) RETURNING id::int",
    )
    .bind(&l.name)
    .bind(&l.value)
    .bind(&l.icon)
    .bind(l.external())
    .bind(l.segment)
    .bind(&l.locale)
    .fetch_one(&mut *conn)
    .await
}

/// A SidebarSectionLink for a url: the section's user, after its last
/// position (before_create).
async fn insert_link(
    conn: &mut PgConnection,
    section_id: i32,
    user_id: i32,
    url_id: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sidebar_section_links (user_id, linkable_id, linkable_type, sidebar_section_id, \
         position, created_at, updated_at) \
         VALUES ($1, $2, 'SidebarUrl', $3, \
           COALESCE((SELECT max(position) FROM sidebar_section_links WHERE sidebar_section_id = $3), 0) + 1, \
           now(), now())",
    )
    .bind(user_id)
    .bind(url_id)
    .bind(section_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// A SidebarSectionLink destroyed, and its url with it (after_destroy).
async fn destroy_link(conn: &mut PgConnection, link_id: i32) -> Result<(), sqlx::Error> {
    let linkable: Option<(String, i32)> = sqlx::query_as(
        "DELETE FROM sidebar_section_links WHERE id = $1 RETURNING linkable_type, linkable_id",
    )
    .bind(link_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((kind, url_id)) = linkable
        && kind == "SidebarUrl"
    {
        sqlx::query("DELETE FROM sidebar_urls WHERE id = $1")
            .bind(url_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// `apply_links_order!`: the links renumbered into positions none holds,
/// in the given order (the rest after, as they were).
async fn apply_links_order(
    conn: &mut PgConnection,
    section_id: i32,
    ordered: &[i32],
) -> Result<(), sqlx::Error> {
    let links: Vec<(i32, i32, i32)> = sqlx::query_as(
        "SELECT id::int, linkable_id, position FROM sidebar_section_links WHERE sidebar_section_id = $1 \
         ORDER BY position",
    )
    .bind(section_id)
    .fetch_all(&mut *conn)
    .await?;
    if links.is_empty() {
        return Ok(());
    }
    let rank = |linkable: i32| {
        // `each_with_index.to_h`: a repeated id keeps its last index.
        ordered
            .iter()
            .rposition(|id| *id == linkable)
            .unwrap_or(ordered.len())
    };
    let mut reordered = links.clone();
    reordered.sort_by_key(|(_, linkable, _)| rank(*linkable));
    let used: Vec<i32> = links.iter().map(|(_, _, p)| *p).collect();
    let free: Vec<i32> = (0..=(links.len() as i32 * 2))
        .filter(|p| !used.contains(p))
        .collect();
    for ((id, _, current), position) in reordered.iter().zip(free) {
        if *current != position {
            sqlx::query(
                "UPDATE sidebar_section_links SET position = $2, updated_at = now() WHERE id = $1",
            )
            .bind(id)
            .bind(position)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// GET /sidebar_sections(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut conn = state.pool.acquire().await?;
    ctx(&state, &mut conn).await?;
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id::int FROM sidebar_sections WHERE public OR user_id = $1 \
         ORDER BY section_type IS NOT NULL DESC, public DESC, title ASC",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for id in ids {
        out.push(serialize(&mut conn, id, false).await?);
    }
    Ok(Json(json!({"sidebar_sections": out})).into_response())
}

/// GET /sidebar_sections/:id(.json): SidebarSectionEditSerializer.
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
) -> Result<Response, AppError> {
    if guardian.user_id().is_none() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut conn = state.pool.acquire().await?;
    ctx(&state, &mut conn).await?;
    let Some(section) = find_section(&mut conn, id_of(&raw)).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !can_edit(&guardian, &section) {
        return Ok(failed(StatusCode::FORBIDDEN));
    }
    let body = serialize(&mut conn, section.id, true).await?;
    Ok(Json(json!({"sidebar_section": body})).into_response())
}

/// The prelude of the writes: login, CSRF, the public check.
async fn prelude(
    state: &AppState,
    guardian: &Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
    method: &str,
) -> Result<Result<Map<String, Value>, Response>, AppError> {
    let p = crate::params::parse(uri.query(), headers, body);
    if !super::session::csrf_ok(state, headers, &pairs(&p), uri.path(), method) {
        return Ok(Err(super::session::bad_csrf()));
    }
    if guardian.user_id().is_none() {
        return Ok(Err(super::login_required::not_logged_in(state)));
    }
    // check_access_if_public
    let public = p.get("public").and_then(cast_bool);
    if public == Some(true) && !guardian.is_admin() {
        return Ok(Err(super::search::invalid_access(state)));
    }
    Ok(Ok(p))
}

/// Links from the params, normalized as SidebarUrl's hooks do.
fn build_link(c: &Ctx, l: &LinkParams, existing: Option<&Url>) -> Result<Link, AppError> {
    let value = match (&l.value, existing) {
        (Some(v), _) => v.clone(),
        (None, Some(u)) => u.value.clone(),
        (None, None) => return Err(Unsupported("a sidebar link without a value").into()),
    };
    let value = remove_internal_hostname(&value, &c.hostname);
    let segment = segment_of(l.segment.as_deref())?;
    Ok(match existing {
        Some(u) => {
            let icon = l.icon.clone().unwrap_or_else(|| u.icon.clone());
            let name = l.name.clone().unwrap_or_else(|| u.name.clone());
            let segment = segment.unwrap_or(u.segment);
            let locale = u
                .locale
                .clone()
                .filter(|l| !l.is_empty())
                .unwrap_or(c.default_locale.clone());
            let changed = icon != u.icon
                || name != u.name
                || value != u.value
                || segment != u.segment
                || Some(&locale) != u.locale.as_ref()
                || (value.starts_with("http://") || value.starts_with("https://")) != u.external;
            Link {
                id: Some(u.id),
                icon,
                name,
                value,
                segment,
                locale,
                destroy: l.destroy,
                changed,
            }
        }
        None => Link {
            id: None,
            icon: l.icon.clone().unwrap_or_default(),
            name: l.name.clone().unwrap_or_default(),
            value,
            segment: segment.unwrap_or(0),
            locale: c.default_locale.clone(),
            destroy: false,
            changed: true,
        },
    })
}

/// POST /sidebar_sections(.json)
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match prelude(&state, &guardian, &headers, &uri, &body, "POST").await? {
        Ok(p) => p,
        Err(r) => return Ok(r),
    };
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("a section without a user"))?;
    let mut tx = state.pool.begin().await?;
    let (_, c) = ctx(&state, &mut tx).await?;
    let public = guardian.is_admin() && p.get("public").and_then(cast_bool) == Some(true);
    let title = p
        .get("title")
        .and_then(crate::params::scalar)
        .unwrap_or_default();
    let params = links_params(&p).unwrap_or_default();
    if params.len() as i64 > c.max_links {
        return Ok(json_error(format!(
            "Maximum {} records are allowed. Got {} records instead.",
            c.max_links,
            params.len()
        )));
    }
    let mut links = Vec::new();
    for l in params.iter().filter(|l| !l.destroy) {
        if l.id.is_some() {
            return Err(Unsupported("a new section's link with an id (RecordNotFound)").into());
        }
        links.push(build_link(&c, l, None)?);
    }
    let validation_user = (!public).then_some(user_id);
    if let Some(e) = first_error(&mut tx, &title, validation_user, None, &links, None).await? {
        return Ok(json_error(e));
    }
    let owner = if public { -1 } else { user_id };
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO sidebar_sections (title, user_id, public, locale, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, now(), now()) RETURNING id::int",
    )
    .bind(&title)
    .bind(owner)
    .bind(public)
    .bind(&c.default_locale)
    .fetch_one(&mut *tx)
    .await?;
    for l in &links {
        let url_id = insert_url(&mut tx, l).await?;
        insert_link(&mut tx, id, owner, url_id).await?;
    }
    if public {
        let pairs: Vec<(String, String)> = links
            .iter()
            .map(|l| (l.name.clone(), l.value.clone()))
            .collect();
        log(
            &mut tx,
            "create_public_sidebar_section",
            user_id,
            &title,
            Some(&details(&pairs)),
        )
        .await?;
    }
    let response = rendered(&mut tx, id).await?;
    tx.commit().await?;
    Ok(response)
}

/// PUT /sidebar_sections/:id(.json): SidebarSectionUpdater.
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match prelude(&state, &guardian, &headers, &uri, &body, "PUT").await? {
        Ok(p) => p,
        Err(r) => return Ok(r),
    };
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("a section without a user"))?;
    let mut tx = state.pool.begin().await?;
    let (_, c) = ctx(&state, &mut tx).await?;
    let Some(section) = find_section(&mut tx, id_of(&raw)).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !can_edit(&guardian, &section) {
        return Ok(failed(StatusCode::FORBIDDEN));
    }
    let params = links_params(&p).unwrap_or_default();
    if params.len() as i64 > c.max_links {
        return Ok(json_error(format!(
            "Maximum {} records are allowed. Got {} records instead.",
            c.max_links,
            params.len()
        )));
    }
    // section_params
    let title = match p.get("title") {
        Some(v) => crate::params::scalar(v).unwrap_or_default(),
        None => section.title.clone(),
    };
    let public_param = if guardian.is_admin() {
        p.get("public").map(cast_bool)
    } else {
        None
    };
    let public = match public_param {
        Some(Some(b)) => b,
        Some(None) => return Err(Unsupported("a blank public flag (NOT NULL violation)").into()),
        None => section.public,
    };
    // `user: current_user` unless public was given true; public sections
    // then go to the system user before saving.
    let validation_user = if public_param.flatten() == Some(true) {
        section.user_id
    } else {
        user_id
    };
    let owner = if public { -1 } else { validation_user };

    // The nested attributes over the section's urls.
    let existing = urls_of(&mut tx, section.id).await?;
    let mut touched: Vec<Link> = Vec::new();
    for l in &params {
        match &l.id {
            Some(raw_id) => {
                let found = existing.iter().find(|u| u.id.to_string() == *raw_id);
                let Some(u) = found else {
                    return Ok(super::topics::not_found_response(&state));
                };
                touched.push(build_link(&c, l, Some(u))?);
            }
            None if l.destroy => {}
            None => touched.push(build_link(&c, l, None)?),
        }
    }
    // The target: the urls by position (the last assignment of each wins),
    // then the new ones.
    let mut target: Vec<Link> = Vec::new();
    for u in &existing {
        let link = touched
            .iter()
            .rev()
            .find(|t| t.id == Some(u.id))
            .cloned()
            .unwrap_or(Link {
                id: Some(u.id),
                icon: u.icon.clone(),
                name: u.name.clone(),
                value: u.value.clone(),
                segment: u.segment,
                locale: u.locale.clone().unwrap_or_default(),
                destroy: false,
                changed: false,
            });
        target.push(link);
    }
    target.extend(touched.iter().filter(|t| t.id.is_none()).cloned());

    // sidebar_urls_count_within_limit (the update context).
    let count = target.iter().filter(|t| !t.destroy).count() as i64;
    let persisted = existing.len() as i64;
    let limit_error = (count > c.max_links.max(persisted)).then(|| {
        format!(
            "Maximum {} records are allowed. Got {count} records instead.",
            c.max_links
        )
    });
    if let Some(e) = first_error(
        &mut tx,
        &title,
        Some(validation_user),
        Some(section.id),
        &target,
        limit_error,
    )
    .await?
    {
        return Ok(json_error(e));
    }

    // save!
    let locale = section
        .locale
        .clone()
        .filter(|l| !l.is_empty())
        .unwrap_or(c.default_locale.clone());
    if title != section.title
        || public != section.public
        || owner != section.user_id
        || Some(&locale) != section.locale.as_ref()
    {
        sqlx::query(
            "UPDATE sidebar_sections SET title = $2, public = $3, user_id = $4, locale = $5, \
             updated_at = now() WHERE id = $1",
        )
        .bind(section.id)
        .bind(&title)
        .bind(public)
        .bind(owner)
        .bind(&locale)
        .execute(&mut *tx)
        .await?;
    }
    for t in &target {
        match t.id {
            Some(url_id) if t.destroy => {
                let link_ids: Vec<i32> = sqlx::query_scalar(
                    "SELECT id::int FROM sidebar_section_links WHERE sidebar_section_id = $1 \
                     AND linkable_type = 'SidebarUrl' AND linkable_id = $2",
                )
                .bind(section.id)
                .bind(url_id)
                .fetch_all(&mut *tx)
                .await?;
                for link_id in link_ids {
                    destroy_link(&mut tx, link_id).await?;
                }
            }
            Some(url_id) if t.changed => {
                sqlx::query(
                    "UPDATE sidebar_urls SET name = $2, value = $3, icon = $4, external = $5, segment = $6, \
                     locale = $7, updated_at = now() WHERE id = $1",
                )
                .bind(url_id)
                .bind(&t.name)
                .bind(&t.value)
                .bind(&t.icon)
                .bind(t.external())
                .bind(t.segment)
                .bind(&t.locale)
                .execute(&mut *tx)
                .await?;
            }
            Some(_) => {}
            None => {
                let url_id = insert_url(&mut tx, t).await?;
                insert_link(&mut tx, section.id, owner, url_id).await?;
            }
        }
    }
    sqlx::query("UPDATE sidebar_section_links SET user_id = $2 WHERE sidebar_section_id = $1")
        .bind(section.id)
        .bind(owner)
        .execute(&mut *tx)
        .await?;

    // update_link_order: the urls in the order the params name them (by
    // name and value), the others first as they were.
    let saved = urls_of(&mut tx, section.id).await?;
    let mut kept: Vec<(i32, String, String)> = Vec::new();
    for t in target.iter().filter(|t| !t.destroy) {
        let id = match t.id {
            Some(id) => id,
            // A new url: the one saved with its name and value.
            None => saved
                .iter()
                .rev()
                .find(|u| {
                    u.name == t.name && u.value == t.value && !kept.iter().any(|k| k.0 == u.id)
                })
                .map(|u| u.id)
                .ok_or(Unsupported("a new sidebar link gone once saved"))?,
        };
        kept.push((id, t.name.clone(), t.value.clone()));
    }
    let mut sorted = kept.clone();
    sorted.sort_by_key(|(_, name, value)| {
        params
            .iter()
            .position(|l| l.name.as_deref() == Some(name) && l.value.as_deref() == Some(value))
            .map_or(-1, |i| i as i64)
    });
    let ordered: Vec<i32> = sorted.iter().map(|(id, _, _)| *id).collect();
    apply_links_order(&mut tx, section.id, &ordered).await?;

    if section.public || public {
        let pairs: Vec<(String, String)> = kept
            .iter()
            .map(|(_, n, v)| (n.clone(), v.clone()))
            .collect();
        log(
            &mut tx,
            "update_public_sidebar_section",
            user_id,
            &title,
            Some(&details(&pairs)),
        )
        .await?;
    }
    let response = rendered(&mut tx, section.id).await?;
    tx.commit().await?;
    Ok(response)
}

/// DELETE /sidebar_sections/:id(.json)
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = prelude(&state, &guardian, &headers, &uri, &body, "DELETE").await? {
        return Ok(r);
    }
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("a section without a user"))?;
    let mut tx = state.pool.begin().await?;
    ctx(&state, &mut tx).await?;
    let Some(section) = find_section(&mut tx, id_of(&raw)).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !can_delete(&guardian, &section) {
        return Ok(failed(StatusCode::FORBIDDEN));
    }
    let link_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id::int FROM sidebar_section_links WHERE sidebar_section_id = $1 ORDER BY position",
    )
    .bind(section.id)
    .fetch_all(&mut *tx)
    .await?;
    for id in link_ids {
        destroy_link(&mut tx, id).await?;
    }
    sqlx::query("DELETE FROM sidebar_sections WHERE id = $1")
        .bind(section.id)
        .execute(&mut *tx)
        .await?;
    if section.public {
        log(
            &mut tx,
            "destroy_public_sidebar_section",
            user_id,
            &section.title,
            None,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}

/// PUT /sidebar_sections/reset/:id(.json): the community section back to
/// its built-in links.
pub async fn reset(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = prelude(&state, &guardian, &headers, &uri, &body, "PUT").await? {
        return Ok(r);
    }
    let mut tx = state.pool.begin().await?;
    let (_, c) = ctx(&state, &mut tx).await?;
    let Some(section) = find_section(&mut tx, id_of(&raw)).await? else {
        return Ok(super::search::invalid_parameters(
            &state,
            "Discourse::InvalidParameters",
        ));
    };
    if !can_edit(&guardian, &section) {
        return Ok(super::search::invalid_access(&state));
    }
    if section.section_type == Some(0) {
        // update!(title: "Community"), its locale defaulted on validation.
        let locale = section
            .locale
            .clone()
            .filter(|l| !l.is_empty())
            .unwrap_or(c.default_locale.clone());
        if section.title != "Community" || section.locale.as_ref() != Some(&locale) {
            sqlx::query(
                "UPDATE sidebar_sections SET title = 'Community', locale = $2, updated_at = now() \
                 WHERE id = $1",
            )
            .bind(section.id)
            .bind(&locale)
            .execute(&mut *tx)
            .await?;
        }
        let link_ids: Vec<i32> = sqlx::query_scalar(
            "SELECT id::int FROM sidebar_section_links WHERE sidebar_section_id = $1 ORDER BY position",
        )
        .bind(section.id)
        .fetch_all(&mut *tx)
        .await?;
        for id in link_ids {
            destroy_link(&mut tx, id).await?;
        }
        for (position, (name, path, icon, segment)) in COMMUNITY_SECTION_LINKS.iter().enumerate() {
            let url_id: i32 = sqlx::query_scalar(
                "INSERT INTO sidebar_urls (name, value, icon, segment, external, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, false, now(), now()) RETURNING id::int",
            )
            .bind(name)
            .bind(path)
            .bind(icon)
            .bind(segment)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO sidebar_section_links (user_id, linkable_id, linkable_type, sidebar_section_id, \
                 position, created_at, updated_at) VALUES (-1, $1, 'SidebarUrl', $2, $3, now(), now())",
            )
            .bind(url_id)
            .bind(section.id)
            .bind(position as i32)
            .execute(&mut *tx)
            .await?;
        }
    }
    let response = rendered(&mut tx, section.id).await?;
    tx.commit().await?;
    Ok(response)
}

fn bad_contract(errors: Vec<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"failed": "FAILED", "errors": errors})),
    )
        .into_response()
}

/// PUT /sidebar_sections/:id/reorder(.json): SidebarSection::ReorderLinks.
pub async fn reorder(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match prelude(&state, &guardian, &headers, &uri, &body, "PUT").await? {
        Ok(p) => p,
        Err(r) => return Ok(r),
    };
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("a section without a user"))?;
    // `Integer(link_id, exception: false)` of each.
    let order: Vec<i32> = match p.get("links_order") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(crate::params::scalar)
            .filter_map(|s| crate::ruby::integer(&s))
            .filter_map(|i| i32::try_from(i).ok())
            .collect(),
        Some(Value::Object(m)) => m
            .values()
            .filter_map(crate::params::scalar)
            .filter_map(|s| crate::ruby::integer(&s))
            .filter_map(|i| i32::try_from(i).ok())
            .collect(),
        _ => Vec::new(),
    };
    if order.is_empty() {
        return Ok(bad_contract(vec!["Links order can't be blank".into()]));
    }
    let mut tx = state.pool.begin().await?;
    ctx(&state, &mut tx).await?;
    let Some(section) = find_section(&mut tx, id_of(&raw)).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !can_edit(&guardian, &section) {
        return Ok(failed(StatusCode::FORBIDDEN));
    }
    let mut current: Vec<i32> = sqlx::query_scalar(
        "SELECT linkable_id FROM sidebar_section_links WHERE sidebar_section_id = $1",
    )
    .bind(section.id)
    .fetch_all(&mut *tx)
    .await?;
    let mut wanted = order.clone();
    current.sort_unstable();
    wanted.sort_unstable();
    if current != wanted {
        return Ok(super::search::invalid_parameters(&state, "links_order"));
    }
    sqlx::query("SELECT id FROM sidebar_sections WHERE id = $1 FOR UPDATE")
        .bind(section.id)
        .execute(&mut *tx)
        .await?;
    apply_links_order(&mut tx, section.id, &order).await?;
    if section.public {
        let urls = urls_of(&mut tx, section.id).await?;
        let pairs: Vec<(String, String)> = urls.into_iter().map(|u| (u.name, u.value)).collect();
        log(
            &mut tx,
            "update_public_sidebar_section",
            user_id,
            &section.title,
            Some(&details(&pairs)),
        )
        .await?;
    }
    let response = rendered(&mut tx, section.id).await?;
    tx.commit().await?;
    Ok(response)
}

/// PUT /sidebar_sections/:id/move_link(.json): SidebarSection::MoveLink.
pub async fn move_link(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match prelude(&state, &guardian, &headers, &uri, &body, "PUT").await? {
        Ok(p) => p,
        Err(r) => return Ok(r),
    };
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("a section without a user"))?;
    let int = |k: &str| -> Result<Option<i64>, AppError> {
        match p.get(k).and_then(crate::params::scalar) {
            None => Ok(None),
            Some(s) if crate::ruby::is_blank(&s) => Ok(None),
            Some(s) => crate::ruby::integer(&s)
                .map(Some)
                .ok_or_else(|| Unsupported("a non-numeric sidebar move parameter").into()),
        }
    };
    let link_id = int("link_id")?;
    let target_id = int("target_section_id")?;
    let position = int("position")?;
    let mut errors = Vec::new();
    if link_id.is_none() {
        errors.push("Link can't be blank".to_string());
    }
    if target_id.is_none() {
        errors.push("Target section can't be blank".to_string());
    }
    if position.is_some_and(|p| p < 0) {
        errors.push("Position must be greater than or equal to 0".to_string());
    }
    if !errors.is_empty() {
        return Ok(bad_contract(errors));
    }
    let (link_id, target_id) = (link_id.unwrap_or_default(), target_id.unwrap_or_default());
    let mut tx = state.pool.begin().await?;
    let (_, c) = ctx(&state, &mut tx).await?;
    let Some(source) = find_section(&mut tx, id_of(&raw)).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let Some(target) = find_section(&mut tx, target_id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if source.id == target.id {
        return Ok(super::search::invalid_parameters(
            &state,
            "target_section_id",
        ));
    }
    if source.section_type.is_some() || target.section_type.is_some() {
        return Ok(super::search::invalid_access(&state));
    }
    if !can_edit(&guardian, &source) || !can_edit(&guardian, &target) {
        return Ok(failed(StatusCode::FORBIDDEN));
    }
    let link: Option<(i32, i32)> = sqlx::query_as(
        "SELECT id::int, linkable_id FROM sidebar_section_links WHERE sidebar_section_id = $1 \
         AND linkable_type = 'SidebarUrl' AND linkable_id = $2 LIMIT 1",
    )
    .bind(source.id)
    .bind(i32::try_from(link_id).unwrap_or(0))
    .fetch_optional(&mut *tx)
    .await?;
    let Some((link_row, linkable)) = link else {
        return Ok(super::topics::not_found_response(&state));
    };
    sqlx::query("SELECT id FROM sidebar_sections WHERE id IN ($1, $2) ORDER BY id FOR UPDATE")
        .bind(source.id)
        .bind(target.id)
        .execute(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sidebar_section_links WHERE sidebar_section_id = $1",
    )
    .bind(target.id)
    .fetch_one(&mut *tx)
    .await?;
    if count >= c.max_links {
        let message = state
            .i18n
            .t_with(
                "activerecord.errors.models.sidebar_section.attributes.base.too_many_sidebar_urls",
                &[
                    ("limit", &c.max_links.to_string()),
                    ("count", &(count + 1).to_string()),
                ],
            )
            .ok_or(Unsupported("a missing sidebar message"))?;
        return Ok(json_error(message));
    }
    sqlx::query(
        "UPDATE sidebar_section_links SET sidebar_section_id = $2, position = \
           COALESCE((SELECT max(position) FROM sidebar_section_links WHERE sidebar_section_id = $2), 0) + 1, \
         updated_at = now() WHERE id = $1",
    )
    .bind(link_row)
    .bind(target.id)
    .execute(&mut *tx)
    .await?;
    let mut ordered: Vec<i32> = sqlx::query_scalar(
        "SELECT linkable_id FROM sidebar_section_links WHERE sidebar_section_id = $1 ORDER BY position",
    )
    .bind(target.id)
    .fetch_all(&mut *tx)
    .await?;
    ordered.retain(|id| *id != linkable);
    let index = match position {
        Some(p) => (p as usize).min(ordered.len()),
        None => ordered.len(),
    };
    ordered.insert(index, linkable);
    apply_links_order(&mut tx, target.id, &ordered).await?;
    for s in [&source, &target] {
        if s.public {
            let urls = urls_of(&mut tx, s.id).await?;
            let pairs: Vec<(String, String)> =
                urls.into_iter().map(|u| (u.name, u.value)).collect();
            log(
                &mut tx,
                "update_public_sidebar_section",
                user_id,
                &s.title,
                Some(&details(&pairs)),
            )
            .await?;
        }
    }
    let sections = vec![
        serialize(&mut tx, source.id, false).await?,
        serialize(&mut tx, target.id, false).await?,
    ];
    tx.commit().await?;
    Ok(Json(json!({"sidebar_sections": sections})).into_response())
}
