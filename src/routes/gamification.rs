//! discourse-gamification's routes: its engine at /leaderboard
//! (GamificationLeaderboardController#respond, JSON, and the
//! gamificationLeaderboard pages Ember draws from it), the admin
//! leaderboards (AdminGamificationLeaderboardController: index, show,
//! create, update, destroy, recalculate_scores) with their pages, and the
//! score events API (AdminGamificationScoreEventController).

use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use chrono::NaiveDate;
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::guardian::Guardian;
use crate::params;
use crate::plugins::gamification::{
    self as plugin, Leaderboard, ScoreEvent, ViewParams, admin_view, cached_view, jobs,
};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// ActionController::ParameterMissing
fn param_missing(key: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "errors": [format!("param is missing or the value is empty or invalid: {key}")]
        })),
    )
        .into_response()
}

/// `render_json_error(model)`: the model's full messages.
fn record_invalid(errors: Vec<String>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": errors })),
    )
        .into_response()
}

fn success() -> Response {
    Json(json!({ "success": "OK" })).into_response()
}

/// The path segment without its `.json` format.
fn strip_format(segment: &str) -> (&str, bool) {
    match segment.strip_suffix(".json") {
        Some(s) => (s, true),
        None => (segment, false),
    }
}

/// `find(params[:id])`: an integer id (Rails casts "12abc" to 12).
fn id_param(raw: &str) -> Option<i64> {
    let t = raw.trim_start();
    t.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+')
        .then(|| crate::ruby::to_i(t))
}

async fn load_settings(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// GET /leaderboard(.json)
pub async fn default_leaderboard(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let json = uri.path().ends_with(".json") || super::wants_json(uri.query(), &headers);
    respond(&state, &guardian, None, json, &headers, &uri).await
}

/// GET /leaderboard/:id(.json)
pub async fn leaderboard(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let (id, suffix) = strip_format(&id);
    let json = suffix || super::wants_json(uri.query(), &headers);
    respond(
        &state,
        &guardian,
        Some(id.to_string()),
        json,
        &headers,
        &uri,
    )
    .await
}

/// `respond`: the leaderboard (the first without an id) for the period
/// asked (`all` for all_time, else the leaderboard's default), its users'
/// page and the viewer's position; 202 with the reason while its
/// positions aren't generated, which the job then does.
async fn respond(
    state: &AppState,
    guardian: &Guardian,
    id: Option<String>,
    json: bool,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), headers, b"");
    let param = |key: &str| p.get(key).and_then(params::scalar);
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(state, &mut conn).await?;
    // requires_plugin (and, for the page, an unknown route).
    if !plugin::enabled(&settings)? {
        return Ok(super::topics::not_found_response(state));
    }
    let lb = match id {
        Some(raw) => match id_param(&raw) {
            Some(id) => Leaderboard::find(&mut conn, id).await?,
            None => None,
        },
        None => match Leaderboard::first(&mut conn).await? {
            Some(lb) => Some(lb),
            // `GamificationLeaderboard.first.id` on nil.
            None => return Err(Unsupported("a leaderboard request without any leaderboard").into()),
        },
    };
    let Some(lb) = lb else {
        return Ok(super::topics::not_found_response(state));
    };
    if !plugin::can_see(guardian, &lb) {
        return Ok(super::topics::not_found_response(state));
    }
    let period_param = param("period").map(|p| if p == "all" { "all_time".into() } else { p });
    let period = lb.resolve_period(period_param.as_deref());
    let page = param("page").map_or(0, |v| crate::ruby::to_i(&v));
    if page < 0 {
        return Err(
            Unsupported("a negative leaderboard page (OFFSET must not be negative)").into(),
        );
    }
    let user_limit = param("user_limit").map(|v| crate::ruby::to_i(&v));
    if user_limit.is_some_and(|l| l < 0) {
        return Err(Unsupported("a negative leaderboard user_limit").into());
    }
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let view = plugin::view_json(
        &mut conn,
        &urls,
        &ViewParams {
            leaderboard: &lb,
            page,
            for_user_id: guardian.user_id(),
            period,
            user_limit,
        },
    )
    .await?;
    let (status, doc) = match view {
        Ok(doc) => (StatusCode::OK, doc),
        Err(cached_view::NotReady) => {
            plugin::enqueue_generate(&mut conn, lb.id).await?;
            let mut doc = plugin::leaderboard_json(&lb, false);
            doc["users"] = json!([]);
            doc["reason"] = json!(
                state
                    .i18n
                    .t("errors.leaderboard_positions_not_ready")
                    .unwrap_or_default()
            );
            (StatusCode::ACCEPTED, doc)
        }
    };
    if json {
        let mut response = (status, Json(doc)).into_response();
        response
            .extensions_mut()
            .insert(crate::anonymous_cache::CacheFor(Duration::from_secs(60)));
        return Ok(response);
    }

    // The page: the not-ready document is LeaderboardSerializer's at the
    // top level, as the component reads `model.leaderboard` from it.
    let doc = if status == StatusCode::ACCEPTED {
        json!({ "leaderboard": doc, "users": [], "reason": doc["reason"] })
    } else {
        doc
    };
    let frame = super::groups::frame(
        state,
        &mut conn,
        &settings,
        guardian,
        headers,
        uri,
        crate::sidebar::Active::None,
    )
    .await?;
    let base = state.config.globals.relative_url_root();
    let cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: base,
        now: crate::clock::now(),
        categories: &frame.categories,
        expand_all_pinned: false,
        hide_category: false,
        member_trust_level: frame.site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    let main = plugin::view::page(
        &cx,
        &plugin::view::Page {
            doc: &doc,
            period: plugin::view::chooser_period(period),
            page,
            viewer_id: guardian.user_id().map(i64::from),
            staff: guardian.is_staff(),
            prioritize_username: settings.get("prioritize_username_in_ux")?.truthy(),
        },
    );
    let main = format!(
        "{main}<script src=\"{}\"></script>",
        crate::assets::url(base, "gamification.js")
    );
    drop(cx);
    super::groups::finish(frame, &settings, String::new(), main)
}

/// What the admin endpoints check first: the route's StaffConstraint
/// (not found), Admin::AdminController's ensure_admin (invalid access),
/// then requires_plugin (not found).
async fn admin_gate(
    state: &AppState,
    guardian: &Guardian,
    settings: &SiteSettings,
) -> Result<Option<Response>, AppError> {
    if !guardian.is_staff() {
        return Ok(Some(super::topics::not_found_response(state)));
    }
    if !guardian.is_admin() {
        return Ok(Some(super::search::invalid_access(state)));
    }
    if !plugin::enabled(settings)? {
        return Ok(Some(super::topics::not_found_response(state)));
    }
    Ok(None)
}

/// A write's params, its CSRF token checked.
fn write_params(
    state: &AppState,
    method: &Method,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
) -> Result<Map<String, Value>, Box<Response>> {
    let p = params::parse(uri.query(), headers, body);
    let pairs: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !csrf_ok(state, headers, &pairs, uri.path(), method.as_str()) {
        return Err(Box::new(bad_csrf()));
    }
    Ok(p)
}

/// `params.require(key)`: present, not blank.
fn require(p: &Map<String, Value>, key: &str) -> Option<String> {
    match p.get(key)? {
        Value::String(s) if s.trim().is_empty() => None,
        Value::Array(a) if a.is_empty() => None,
        Value::Object(o) if o.is_empty() => None,
        v => Some(params::scalar(v).unwrap_or_default()),
    }
}

/// The groups as AdminGamificationIndexSerializer lists them
/// (`Group.all` through BasicGroupSerializer).
async fn groups_json(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    guardian: &Guardian,
    settings: &SiteSettings,
) -> Result<Vec<Value>, AppError> {
    let groups: Vec<crate::groups::BasicGroup> = sqlx::query_as(&format!(
        "SELECT {} FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id",
        crate::groups::BASIC_GROUP_COLUMNS
    ))
    .fetch_all(&mut *conn)
    .await?;
    let memberships: Vec<(i32, bool)> =
        sqlx::query_as("SELECT group_id, owner FROM group_users WHERE user_id = $1")
            .bind(guardian.user_id().unwrap_or(0))
            .fetch_all(&mut *conn)
            .await?;
    let mut out = Vec::with_capacity(groups.len());
    for g in &groups {
        let membership = memberships
            .iter()
            .find(|(id, _)| *id == g.id)
            .map(|(_, owner)| (true, *owner));
        out.push(g.json(&state.i18n, guardian, settings, membership)?);
    }
    Ok(out)
}

/// The index document (AdminGamificationIndexSerializer).
async fn index_json(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    guardian: &Guardian,
    settings: &SiteSettings,
) -> Result<Value, AppError> {
    let remaining = jobs::recalculate_limiter().remaining(&state.pool).await?;
    let leaderboards: Vec<Value> = Leaderboard::all_by_updated(&mut *conn)
        .await?
        .iter()
        .map(|lb| plugin::leaderboard_json(lb, true))
        .collect();
    Ok(json!({
        "gamification_recalculate_scores_remaining": remaining,
        "gamification_leaderboards": leaderboards,
        "gamification_groups": groups_json(state, conn, guardian, settings).await?,
    }))
}

/// GET /admin/plugins/gamification(.json): the index document; the page
/// is the plugin's admin page.
pub async fn admin_index(
    State(state): State<AppState>,
    auth: AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if super::admin_email::html_request(&uri, &headers) {
        return super::admin_plugins::show(
            State(state),
            auth,
            Path(vec![("plugin_id".into(), "gamification".into())]),
            headers,
            uri,
        )
        .await;
    }
    let guardian = auth.0;
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(&state, &mut conn).await?;
    if let Some(r) = admin_gate(&state, &guardian, &settings).await? {
        return Ok(r);
    }
    Ok(Json(index_json(&state, &mut conn, &guardian, &settings).await?).into_response())
}

/// GET /admin/plugins/discourse-gamification/leaderboards(.json): the index
/// document, or the leaderboards page.
pub async fn admin_leaderboards(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(&state, &mut conn).await?;
    if !super::admin_email::html_request(&uri, &headers) {
        if let Some(r) = admin_gate(&state, &guardian, &settings).await? {
            return Ok(r);
        }
        return Ok(
            Json(index_json(&state, &mut conn, &guardian, &settings).await?).into_response(),
        );
    }
    if !guardian.is_staff() || !plugin::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    let leaderboards = if guardian.is_admin() {
        Leaderboard::all_by_updated(&mut conn).await?
    } else {
        // The route's model is empty for a moderator.
        Vec::new()
    };
    let remaining = jobs::recalculate_limiter().remaining(&state.pool).await?;
    let content = |cx: &crate::admin_view::AdminCx| {
        admin_view::leaderboards(
            cx,
            &leaderboards,
            remaining,
            crate::clock::now().date_naive(),
        )
    };
    admin_page(&state, &mut conn, &guardian, &headers, &uri, true, content).await
}

/// GET /admin/plugins/discourse-gamification/leaderboards/:id(.json)
pub async fn admin_leaderboard(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let (id, _) = strip_format(&id);
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(&state, &mut conn).await?;
    let html = super::admin_email::html_request(&uri, &headers);
    if html {
        if !guardian.is_staff() || !plugin::enabled(&settings)? {
            return Ok(super::topics::not_found_response(&state));
        }
    } else if let Some(r) = admin_gate(&state, &guardian, &settings).await? {
        return Ok(r);
    }
    let lb = match id_param(id) {
        Some(id) => Leaderboard::find(&mut conn, id).await?,
        None => None,
    };
    let Some(lb) = lb else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !html {
        return Ok(
            Json(json!({ "leaderboard": plugin::leaderboard_json(&lb, true) })).into_response(),
        );
    }
    let groups: Vec<(i64, String)> =
        sqlx::query_as("SELECT id::bigint, name FROM groups WHERE id <> 0 ORDER BY name")
            .fetch_all(&mut *conn)
            .await?;
    let categories: Vec<(i64, String)> =
        sqlx::query_as("SELECT id::bigint, name FROM categories ORDER BY position, id")
            .fetch_all(&mut *conn)
            .await?;
    let content = |cx: &crate::admin_view::AdminCx| admin_view::edit(cx, &lb, &groups, &categories);
    admin_page(&state, &mut conn, &guardian, &headers, &uri, true, content).await
}

/// One of the plugin's admin pages in AdminPluginConfigPage.
async fn admin_page(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    guardian: &Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    leaderboards_tab: bool,
    content: impl FnOnce(&crate::admin_view::AdminCx) -> String,
) -> Result<Response, AppError> {
    let Some(plugin) = crate::admin_plugins::find(
        &SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?,
        "discourse-gamification",
    )?
    else {
        return Ok(super::topics::not_found_response(state));
    };
    let shell = super::admin_pages::Shell::load(
        conn,
        state,
        guardian,
        headers,
        "admin_plugin_discourse-gamification",
    )
    .await?;
    let cx = shell.cx(state);
    let crumb = admin_view::crumb(&cx);
    let page = crate::admin_plugins_view::config_page(
        &cx,
        &plugin,
        (crumb.clone(), admin_view::LEADERBOARDS_PATH),
        Some(admin_view::config_nav(&cx, leaderboards_tab)),
        &content(&cx),
    );
    let main = format!(
        "{}{}",
        admin_view::strings(&cx),
        crate::admin_plugins_view::render(&page)
    );
    let main = crate::admin_ui::contents("admin-plugins discourse-gamification", "", &main);
    let title = format!(
        "{crumb} - {} - {}",
        crate::admin_plugins_view::name_titleized(&cx, &plugin),
        crate::admin_view::t(&state.i18n, "admin.plugins.title"),
    );
    shell.render(state, uri, &title, main, "gamification.js", StatusCode::OK)
}

/// POST /admin/plugins/gamification/leaderboard(.json): a leaderboard
/// with its name and creator, its scores then calculated.
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match write_params(&state, &method, &headers, &uri, &body) {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    let mut tx = state.pool.begin().await?;
    let settings = load_settings(&state, &mut tx).await?;
    if let Some(r) = admin_gate(&state, &guardian, &settings).await? {
        return Ok(r);
    }
    let Some(name) = require(&p, "name") else {
        return Ok(param_missing("name"));
    };
    let Some(created_by) = require(&p, "created_by_id") else {
        return Ok(param_missing("created_by_id"));
    };
    let errors = plugin::validate(&name, None);
    if !errors.is_empty() {
        return Ok(record_invalid(errors));
    }
    let created_by = created_by
        .trim()
        .parse::<i32>()
        .map_err(|_| Unsupported("a created_by_id that is not an integer"))?;
    let now = crate::clock::now_naive();
    let lb: Leaderboard = sqlx::query_as(
        "INSERT INTO gamification_leaderboards (name, created_by_id, created_at, updated_at) \
         VALUES ($1, $2, $3, $3) RETURNING *",
    )
    .bind(&name)
    .bind(created_by)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    crate::jobs::enqueue(
        &mut tx,
        jobs::RECALCULATE_LEADERBOARD_SCORES,
        json!({ "leaderboard_id": lb.id }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(plugin::leaderboard_json(&lb, true)).into_response())
}

/// An integer array param as ActiveRecord casts it: the elements' to_i,
/// blank ones dropped; anything but an array as empty.
fn int_array(v: Option<&Value>) -> Vec<i32> {
    match v {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(params::scalar)
            .filter(|s| !s.trim().is_empty())
            .map(|s| crate::ruby::to_i(&s) as i32)
            .collect(),
        _ => Vec::new(),
    }
}

/// A date param as the date column casts it: none for blank or invalid.
fn date_param(p: &Map<String, Value>, key: &str) -> Option<NaiveDate> {
    let s = p.get(key).and_then(params::scalar)?;
    let h = crate::ruby_date::parse(s.trim());
    match (h.year, h.mon, h.mday) {
        (Some(y), Some(m), Some(d)) => NaiveDate::from_ymd_opt(
            i32::try_from(y).ok()?,
            u32::try_from(m).ok()?,
            u32::try_from(d).ok()?,
        ),
        _ => None,
    }
}

/// PUT /admin/plugins/gamification/leaderboard/:id(.json): the
/// leaderboard's settings, its scores recalculated when its scoring
/// changed, else its positions refreshed.
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match write_params(&state, &method, &headers, &uri, &body) {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    let (id, _) = strip_format(&id);
    let mut tx = state.pool.begin().await?;
    let settings = load_settings(&state, &mut tx).await?;
    if let Some(r) = admin_gate(&state, &guardian, &settings).await? {
        return Ok(r);
    }
    let Some(name) = require(&p, "name") else {
        return Ok(param_missing("name"));
    };
    let lb = match id_param(id) {
        Some(id) => Leaderboard::find(&mut tx, id).await?,
        None => None,
    };
    let Some(lb) = lb else {
        return Ok(super::topics::not_found_response(&state));
    };
    let default_period = match p.get("default_period").and_then(params::scalar) {
        Some(s) if !s.trim().is_empty() => Some(crate::ruby::to_i(&s) as i32),
        _ => None,
    };
    let period_filter_disabled = match p.get("period_filter_disabled") {
        None | Some(Value::Null) => false,
        Some(v) => !matches!(
            params::scalar(v)
                .unwrap_or_default()
                .to_lowercase()
                .as_str(),
            "" | "0" | "f" | "false" | "off"
        ),
    };
    let score_overrides = match p.get("score_overrides") {
        Some(Value::Object(map)) if !map.is_empty() => Some(Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let n = params::scalar(v).map_or(0, |s| crate::ruby::to_i(&s));
                    (k.clone(), json!(n))
                })
                .collect(),
        )),
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(_) => return Err(Unsupported("score_overrides that are not a hash").into()),
    };
    let scorable_category_ids = match p.get("scorable_category_ids") {
        Some(Value::Array(a)) if !a.is_empty() => Some(int_array(p.get("scorable_category_ids"))),
        _ => None,
    };
    let errors = plugin::validate(&name, score_overrides.as_ref());
    if !errors.is_empty() {
        return Ok(record_invalid(errors));
    }
    let new = Leaderboard {
        name,
        to_date: date_param(&p, "to_date"),
        from_date: date_param(&p, "from_date"),
        included_groups_ids: int_array(p.get("included_groups_ids")),
        excluded_groups_ids: int_array(p.get("excluded_groups_ids")),
        visible_to_groups_ids: int_array(p.get("visible_to_groups_ids")),
        default_period,
        period_filter_disabled,
        score_overrides,
        scorable_category_ids,
        ..lb.clone()
    };
    let changed = new.name != lb.name
        || new.to_date != lb.to_date
        || new.from_date != lb.from_date
        || new.included_groups_ids != lb.included_groups_ids
        || new.excluded_groups_ids != lb.excluded_groups_ids
        || new.visible_to_groups_ids != lb.visible_to_groups_ids
        || new.default_period != lb.default_period
        || new.period_filter_disabled != lb.period_filter_disabled
        || new.score_overrides != lb.score_overrides
        || new.scorable_category_ids != lb.scorable_category_ids;
    // ActiveRecord saves (and touches updated_at) only what changed.
    if changed {
        sqlx::query(
            "UPDATE gamification_leaderboards SET name = $2, to_date = $3, from_date = $4, \
             included_groups_ids = $5, excluded_groups_ids = $6, visible_to_groups_ids = $7, \
             default_period = $8, period_filter_disabled = $9, score_overrides = $10, \
             scorable_category_ids = $11, updated_at = $12 WHERE id = $1",
        )
        .bind(lb.id)
        .bind(&new.name)
        .bind(new.to_date)
        .bind(new.from_date)
        .bind(&new.included_groups_ids)
        .bind(&new.excluded_groups_ids)
        .bind(&new.visible_to_groups_ids)
        .bind(new.default_period)
        .bind(new.period_filter_disabled)
        .bind(&new.score_overrides)
        .bind(&new.scorable_category_ids)
        .bind(crate::clock::now_naive())
        .execute(&mut *tx)
        .await?;
    }
    let scoring_changed = new.score_overrides != lb.score_overrides
        || new.scorable_category_ids != lb.scorable_category_ids;
    let job = if scoring_changed {
        jobs::RECALCULATE_LEADERBOARD_SCORES
    } else {
        jobs::REFRESH_LEADERBOARD_POSITIONS
    };
    crate::jobs::enqueue(&mut tx, job, json!({ "leaderboard_id": lb.id })).await?;
    tx.commit().await?;
    Ok(success())
}

/// DELETE /admin/plugins/gamification/leaderboard/:id(.json): the
/// leaderboard and its scores, its positions dropped by the job.
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = write_params(&state, &method, &headers, &uri, &body) {
        return Ok(*r);
    }
    let (id, _) = strip_format(&id);
    let mut tx = state.pool.begin().await?;
    let settings = load_settings(&state, &mut tx).await?;
    if let Some(r) = admin_gate(&state, &guardian, &settings).await? {
        return Ok(r);
    }
    let lb = match id_param(id) {
        Some(id) => Leaderboard::find(&mut tx, id).await?,
        None => None,
    };
    let Some(lb) = lb else {
        return Ok(super::topics::not_found_response(&state));
    };
    sqlx::query("DELETE FROM gamification_leaderboard_scores WHERE leaderboard_id = $1")
        .bind(lb.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM gamification_leaderboards WHERE id = $1")
        .bind(lb.id)
        .execute(&mut *tx)
        .await?;
    crate::jobs::enqueue(
        &mut tx,
        jobs::DELETE_LEADERBOARD_POSITIONS,
        json!({ "leaderboard_id": lb.id }),
    )
    .await?;
    tx.commit().await?;
    Ok(success())
}

/// `Date.parse(value)`: a full date, or a month and day of this year;
/// None where Ruby raises.
fn ruby_date_parse(value: &str, today: NaiveDate) -> Option<NaiveDate> {
    let h = crate::ruby_date::parse(value);
    let year = match h.year {
        Some(y) => i32::try_from(y).ok()?,
        None => chrono::Datelike::year(&today),
    };
    NaiveDate::from_ymd_opt(
        year,
        u32::try_from(h.mon?).ok()?,
        u32::try_from(h.mday?).ok()?,
    )
}

/// PUT /admin/plugins/gamification/recalculate-scores(.json): every
/// leaderboard's scores recalculated from the date, five times a day at
/// most.
pub async fn recalculate_scores(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match write_params(&state, &method, &headers, &uri, &body) {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(&state, &mut conn).await?;
    if let Some(r) = admin_gate(&state, &guardian, &settings).await? {
        return Ok(r);
    }
    if let Err(refused) = jobs::recalculate_limiter()
        .performed(&state.pool, state.config.rate_limits)
        .await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    let now = crate::clock::now_naive();
    let since = p
        .get("from_date")
        .and_then(params::scalar)
        .and_then(|s| ruby_date_parse(&s, now.date()))
        .map(|d| d.and_hms_opt(0, 0, 0).expect("midnight"));
    let Some(since) = since.filter(|s| *s <= now) else {
        return Ok(super::search::invalid_parameters(&state, "from_date"));
    };
    crate::jobs::enqueue(
        &mut conn,
        jobs::RECALCULATE_SCORES,
        json!({
            "since": crate::topic_list::time_json(since),
            "user_id": guardian.user_id(),
        }),
    )
    .await?;
    Ok(success())
}

/// GET /admin/plugins/gamification/score_events(.json): up to 100 events,
/// by id, user and date when given.
pub async fn score_events(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, b"");
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(&state, &mut conn).await?;
    if let Some(r) = admin_gate(&state, &guardian, &settings).await? {
        return Ok(r);
    }
    let mut filters = Vec::new();
    let int = |key: &str| -> Option<Option<i64>> {
        let s = p.get(key).and_then(params::scalar)?;
        Some(s.trim().parse::<i64>().ok())
    };
    // A value the column can't take matches nothing.
    for key in ["id", "user_id"] {
        match int(key) {
            None => {}
            Some(Some(v)) => filters.push(format!("{key} = {v}")),
            Some(None) => filters.push("1=0".into()),
        }
    }
    if p.get("date").and_then(params::scalar).is_some() {
        // Blank or not a date: cast to nil.
        match date_param(&p, "date") {
            Some(d) => filters.push(format!("date = '{}'", d.format("%Y-%m-%d"))),
            None => filters.push("date IS NULL".into()),
        }
    }
    let where_sql = if filters.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", filters.join(" AND "))
    };
    let events: Vec<ScoreEvent> = sqlx::query_as(&format!(
        "SELECT * FROM gamification_score_events{where_sql} LIMIT 100"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let events: Vec<Value> = events.iter().map(plugin::score_event_json).collect();
    Ok(Json(json!({ "events": events })).into_response())
}

/// POST /admin/plugins/gamification/score_events(.json): an event of
/// points for a user on a date.
pub async fn create_score_event(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match write_params(&state, &method, &headers, &uri, &body) {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(&state, &mut conn).await?;
    if let Some(r) = admin_gate(&state, &guardian, &settings).await? {
        return Ok(r);
    }
    for key in ["user_id", "date", "points"] {
        if require(&p, key).is_none() {
            return Ok(param_missing(key));
        }
    }
    let user_id = require(&p, "user_id")
        .and_then(|s| s.trim().parse::<i32>().ok())
        .ok_or(Unsupported("a score event user_id that is not an integer"))?;
    // belongs_to :user, required.
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Ok(record_invalid(vec!["User must exist".into()]));
    }
    let date =
        date_param(&p, "date").ok_or(Unsupported("a score event date that is not a date"))?;
    let points = require(&p, "points")
        .and_then(|s| s.trim().parse::<i32>().ok())
        .ok_or(Unsupported("score event points that are not an integer"))?;
    let description = p.get("description").and_then(params::scalar);
    let now = crate::clock::now_naive();
    let event: ScoreEvent = sqlx::query_as(
        "INSERT INTO gamification_score_events (user_id, date, points, description, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $5) RETURNING *",
    )
    .bind(user_id)
    .bind(date)
    .bind(points)
    .bind(&description)
    .bind(now)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Json(plugin::score_event_json(&event)).into_response())
}

/// PUT /admin/plugins/gamification/score_events(.json): an event's points
/// and description.
pub async fn update_score_event(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match write_params(&state, &method, &headers, &uri, &body) {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(&state, &mut conn).await?;
    if let Some(r) = admin_gate(&state, &guardian, &settings).await? {
        return Ok(r);
    }
    let Some(id) = require(&p, "id") else {
        return Ok(param_missing("id"));
    };
    let Some(points) = require(&p, "points") else {
        return Ok(param_missing("points"));
    };
    let event: Option<ScoreEvent> =
        sqlx::query_as("SELECT * FROM gamification_score_events WHERE id = $1")
            .bind(id_param(&id).unwrap_or(0))
            .fetch_optional(&mut *conn)
            .await?;
    let Some(event) = event else {
        return Ok(super::topics::not_found_response(&state));
    };
    let points = points
        .trim()
        .parse::<i32>()
        .map_err(|_| Unsupported("score event points that are not an integer"))?;
    let description = p
        .get("description")
        .and_then(params::scalar)
        .or(event.description.clone());
    if points != event.points || description != event.description {
        sqlx::query(
            "UPDATE gamification_score_events SET points = $2, description = $3, updated_at = $4 \
             WHERE id = $1",
        )
        .bind(event.id)
        .bind(points)
        .bind(&description)
        .bind(crate::clock::now_naive())
        .execute(&mut *conn)
        .await?;
    }
    Ok(success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_cast_as_rails_does() {
        assert_eq!(id_param("12"), Some(12));
        assert_eq!(id_param("12abc"), Some(12));
        assert_eq!(id_param("abc"), None);
    }

    #[test]
    fn date_parse_fills_the_year() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        assert_eq!(
            ruby_date_parse("2014-8-26", today),
            NaiveDate::from_ymd_opt(2014, 8, 26)
        );
        assert_eq!(
            ruby_date_parse("Oct 18, 2026", today),
            NaiveDate::from_ymd_opt(2026, 10, 18)
        );
        assert_eq!(ruby_date_parse("nonsense", today), None);
    }
}
