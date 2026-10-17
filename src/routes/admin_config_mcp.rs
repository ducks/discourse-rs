//! The MCP server pages (90g6): /admin/config/mcp and its tabs, each
//! section's model read as its route reads it from /admin/mcp. Their
//! JSON is the config site settings controller's (Rails routes `mcp` and
//! `mcp/*path` to it). With `fragment`, the clients', authorizations' and
//! activity's rows for admin-mcp.js's filters and next pages.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use super::admin_pages::{Shell, wants_json};
use crate::admin_mcp_view::{self as view, Section};
use crate::session::current::AuthGuardian;
use crate::{AppError, AppState, params};

async fn page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    body: Bytes,
    path: &str,
) -> Result<Response, AppError> {
    if wants_json(uri, headers) || !guardian.is_admin() {
        return super::admin_site_settings::config_index(
            State(state.clone()),
            AuthGuardian(guardian.clone()),
            headers.clone(),
            uri.clone(),
            body,
        )
        .await;
    }
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let query = params::parse_query(uri.query().unwrap_or(""));
    let fragment = query.contains_key("fragment");
    let mut conn = state.pool.acquire().await?;
    // The sidebar link is adminConfig.mcp.index's: the other tabs mark none.
    let link = if parts.is_empty() { "admin_mcp" } else { "" };
    let shell = Shell::load(&mut conn, state, guardian, headers, link).await?;
    let cx = shell.cx(state);
    let settings = &shell.settings;
    let filter = params::string(&query, "filter")
        .unwrap_or_default()
        .trim()
        .to_string();
    let cursor = params::string(&query, "cursor")
        .and_then(|c| crate::ruby::integer(&c))
        .filter(|c| *c >= 1);
    let today = crate::clock::now().date_naive();
    let range = crate::admin_dashboard_view::Range::from_params(
        params::string(&query, "range")
            .as_deref()
            .or(Some("last_7_days")),
        params::string(&query, "start_date").as_deref(),
        params::string(&query, "end_date").as_deref(),
        today,
    );
    let (section, classes) = match parts.as_slice() {
        [] => {
            let urls = crate::url::Urls {
                config: &state.config,
                settings,
            };
            let base_url = urls.base_url()?;
            (
                Section::Overview(
                    crate::mcp::overview(&mut conn, settings, &state.i18n, &base_url).await?,
                ),
                "admin-config mcp",
            )
        }
        ["capabilities"] => (
            Section::Capabilities(crate::mcp::primitives_payload(&mut conn, settings).await?),
            "admin-config mcp capabilities",
        ),
        ["access", rest @ ..] => {
            let mut model = crate::mcp::primitives_payload(&mut conn, settings).await?;
            let rules = crate::mcp::access_rules(&mut conn).await?;
            if let Value::Object(map) = &mut model {
                map.insert("access_rules".into(), json!(rules));
            }
            match rest {
                [] => (Section::Access(model), "admin-config mcp access"),
                ["new"] => {
                    let used: Vec<i64> = rules
                        .iter()
                        .filter_map(|r| r["group_id"].as_i64())
                        .collect();
                    let groups: Vec<(i64, String)> = sqlx::query_as::<_, (i64, String)>(
                        "SELECT id::bigint, name FROM groups WHERE id > 0 ORDER BY name",
                    )
                    .fetch_all(&mut *conn)
                    .await?
                    .into_iter()
                    .filter(|(id, _)| !used.contains(id))
                    .collect();
                    (
                        Section::AccessForm(model, None, groups),
                        "admin-config mcp access new",
                    )
                }
                [id, "edit"] => {
                    let Some(rule) = rules
                        .iter()
                        .find(|r| {
                            r["group_id"]
                                .as_i64()
                                .is_some_and(|g| Some(g) == crate::ruby::integer(id))
                        })
                        .cloned()
                    else {
                        return Ok(super::topics::not_found_response(state));
                    };
                    (
                        Section::AccessForm(model, Some(rule), Vec::new()),
                        "admin-config mcp access edit",
                    )
                }
                _ => return Ok(super::topics::not_found_response(state)),
            }
        }
        ["clients"] => {
            let model = crate::mcp_clients::page(&mut conn, 50, cursor, &filter).await?;
            if fragment {
                let list = model["clients"].as_array().cloned().unwrap_or_default();
                return Ok(Json(json!({
                    "html": view::client_rows(&cx, &list),
                    "count": list.len(),
                    "next_cursor": model["meta"]["next_cursor"],
                }))
                .into_response());
            }
            (Section::Clients(model), "admin-config mcp clients")
        }
        ["clients", "new"] => {
            if let Some(preset) = params::string(&query, "preset").filter(|_| fragment) {
                let (name, client_id, uris) = view::preset_values(&preset);
                return Ok(axum::response::Html(view::client_form(
                    &cx, &preset, name, client_id, uris,
                ))
                .into_response());
            }
            (Section::ClientNew, "admin-config mcp clients new")
        }
        ["clients", id] => {
            let Some(client) = (match crate::ruby::integer(id) {
                Some(id) => crate::mcp_clients::find(&mut conn, id).await?,
                None => None,
            }) else {
                return Ok(super::topics::not_found_response(state));
            };
            (
                Section::ClientDetail(json!({ "client": crate::mcp_clients::serialize(&client) })),
                "admin-config mcp clients",
            )
        }
        ["authorizations"] => {
            let model =
                crate::mcp::authorizations(&mut conn, settings, 50, cursor, &filter).await?;
            if fragment {
                let list = model["authorizations"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                return Ok(Json(json!({
                    "html": view::authorization_rows(&cx, &list),
                    "ids": list.iter().map(|a| a["id"].clone()).collect::<Vec<_>>(),
                    "count": list.len(),
                    "next_cursor": model["meta"]["next_cursor"],
                }))
                .into_response());
            }
            (
                Section::Authorizations(model),
                "admin-config mcp authorizations",
            )
        }
        ["activity"] => {
            let start = range.start.format("%Y-%m-%d").to_string();
            let end = range.end.format("%Y-%m-%d").to_string();
            let outcome = params::string(&query, "outcome").filter(|o| o != "all" && !o.is_empty());
            let q = crate::mcp::ActivityQuery {
                limit: 50,
                cursor,
                start_date: Some(&json!(start)),
                end_date: Some(&json!(end)),
                filter: filter.clone(),
                outcome,
            };
            let model = match crate::mcp::activity(&mut conn, &q).await? {
                Ok(m) => m,
                Err(name) => return Ok(super::search::invalid_parameters(state, name)),
            };
            if fragment {
                let list = model["activity"].as_array().cloned().unwrap_or_default();
                return Ok(Json(json!({
                    "html": view::activity_rows(&cx, &list),
                    "count": list.len(),
                    "metrics": if list.is_empty() || cursor.is_some() { String::new() } else { view::activity_metrics(&cx, &model["metrics"]) },
                    "next_cursor": model["next_cursor"],
                }))
                .into_response());
            }
            (Section::Activity(model), "admin-config mcp activity")
        }
        ["settings"] => (
            Section::Settings(
                super::admin_advanced::area_settings(
                    &mut conn,
                    state,
                    &shell,
                    uri,
                    json!({ "filter_area": "mcp" }),
                )
                .await?,
            ),
            "admin-config mcp settings",
        ),
        _ => return Ok(super::topics::not_found_response(state)),
    };
    let date_range = crate::admin_dashboard_view::date_range_trigger(&cx, &range);
    let content = view::page(&cx, &section, &date_range);
    let strings = crate::admin_ui::strings(&cx, "admin-mcp-strings", view::STRINGS);
    // The activity's date range picker reads moment.
    let picker = if matches!(section, Section::Activity(_)) {
        format!(
            "<script src=\"{}\"></script><script src=\"{}\"></script>",
            crate::assets::url(cx.base_path, "moment.min.js"),
            crate::assets::url(cx.base_path, "date-range-picker.js")
        )
    } else {
        String::new()
    };
    let data = format!(
        "<script type=\"application/json\" id=\"admin-mcp-data\">{}</script>{picker}",
        view::data(&section).to_string().replace('<', "\\u003c"),
    );
    let main = crate::admin_view::contents(classes, &format!("{content}{strings}{data}"));
    let title = crate::admin_view::t(cx.i18n, "admin.config.mcp.title");
    let script = if matches!(section, Section::Settings(_)) {
        "site-settings.js"
    } else {
        "admin-mcp.js"
    };
    shell.render(state, uri, &title, main, script, StatusCode::OK)
}

pub(super) fn routes(mut router: axum::Router<AppState>) -> axum::Router<AppState> {
    use axum::routing::get;
    for suffix in ["", ".json"] {
        router = router.route(
            &format!("/admin/config/mcp{suffix}"),
            get(
                |State(state): State<AppState>,
                 AuthGuardian(guardian): AuthGuardian,
                 headers: HeaderMap,
                 uri: Uri,
                 body: Bytes| async move {
                    page(&state, &guardian, &headers, &uri, body, "").await
                },
            ),
        );
    }
    router.route(
        "/admin/config/mcp/{*path}",
        get(
            |State(state): State<AppState>,
             AuthGuardian(guardian): AuthGuardian,
             Path(path): Path<String>,
             headers: HeaderMap,
             uri: Uri,
             body: Bytes| async move {
                let path = path.strip_suffix(".json").unwrap_or(&path).to_string();
                page(&state, &guardian, &headers, &uri, body, &path).await
            },
        ),
    )
}
