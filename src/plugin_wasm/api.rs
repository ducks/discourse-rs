//! A WebAssembly plugin as plugin_api sees it: every hook of
//! [`crate::plugin_api::Plugin`] answered by the module's exports or its
//! directory's files. A hook that fails costs the page that hook's keys or
//! HTML (logged), not the page; a route that fails answers 500 and a job
//! that fails is retried, as core's own.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::extract::{RawPathParams, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodFilter, MethodRouter};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Ctx, KeyType, Phase, Plugin, coerce};
use crate::guardian::Guardian;
use crate::plugin_api::{self, Homepage, Outlet};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// A route's request body, at most.
const MAX_BODY: usize = 1 << 20;

/// What the guest's code said went wrong.
#[derive(Debug)]
struct GuestError(String);

impl std::fmt::Display for GuestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GuestError {}

/// Strings live as long as the process, as a compiled-in plugin's do: a
/// plugin loads once, at boot.
fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

pub(super) fn method_filter(method: &str) -> Option<MethodFilter> {
    Some(match method {
        "get" => MethodFilter::GET,
        "post" => MethodFilter::POST,
        "put" => MethodFilter::PUT,
        "patch" => MethodFilter::PATCH,
        "delete" => MethodFilter::DELETE,
        _ => return None,
    })
}

pub struct WasmPlugin {
    plugin: Arc<Plugin>,
    name: &'static str,
    enabled_setting: Option<&'static str>,
    settings_yml: Option<&'static str>,
    locales: Vec<&'static str>,
    svg_icons: Vec<&'static str>,
    jobs: Vec<&'static str>,
    scheduled: Vec<(&'static str, Duration)>,
    homepage: Option<Homepage>,
}

impl WasmPlugin {
    pub fn new(plugin: Arc<Plugin>) -> WasmPlugin {
        let all = |v: &[String]| v.iter().map(|s| leak(s)).collect();
        WasmPlugin {
            name: leak(&plugin.name),
            enabled_setting: plugin.enabled_setting.as_deref().map(leak),
            settings_yml: plugin.files.settings_yml.as_deref().map(leak),
            locales: all(&plugin.files.locales),
            svg_icons: all(&plugin.svg_icons),
            jobs: all(&plugin.jobs),
            scheduled: plugin
                .scheduled
                .iter()
                .map(|s| (leak(&s.name), Duration::from_secs(s.every_secs)))
                .collect(),
            homepage: plugin.homepage.as_ref().map(|h| Homepage {
                id: leak(&h.id),
                name: leak(&h.name),
                path: leak(&h.path),
                anonymous: h.anonymous,
                server_side: h.server_side,
            }),
            plugin,
        }
    }

    /// A serializer hook's keys, declared and coerced; none when it fails.
    async fn keys(
        &self,
        ctx: &mut Ctx<'_>,
        phase: Phase,
        payload: Value,
        keys: &[(String, KeyType)],
    ) -> Map<String, Value> {
        let result = self
            .plugin
            .run(ctx, "serializer", phase, payload)
            .await
            .and_then(|v| coerce(&json!({ "keys": v }), keys));
        match result {
            Ok(mut merged) => merged
                .remove("keys")
                .unwrap_or_default()
                .into_iter()
                .collect(),
            Err(e) => {
                self.plugin.log_skip(phase, &e);
                Map::new()
            }
        }
    }
}

#[async_trait]
impl plugin_api::Plugin for WasmPlugin {
    fn name(&self) -> &'static str {
        self.name
    }

    fn enabled_setting(&self) -> Option<&'static str> {
        self.enabled_setting
    }

    fn settings_yml(&self) -> Option<&'static str> {
        self.settings_yml
    }

    fn locale_ymls(&self) -> &[&'static str] {
        &self.locales
    }

    fn stylesheet_url(&self, base_path: &str) -> Option<String> {
        let file = self.plugin.stylesheet.as_ref()?;
        Some(format!("{base_path}/plugins/{}/{file}", self.name))
    }

    fn svg_icons(&self) -> &[&'static str] {
        &self.svg_icons
    }

    fn routes(&self) -> Router<AppState> {
        let mut paths: Vec<(String, MethodRouter<AppState>)> = Vec::new();
        for decl in &self.plugin.routes {
            let Some(filter) = method_filter(&decl.method) else {
                continue;
            };
            let plugin = self.plugin.clone();
            let route = decl.path.clone();
            let form = decl.form;
            let handler = move |State(state): State<AppState>,
                                AuthGuardian(guardian): AuthGuardian,
                                params: RawPathParams,
                                request: axum::extract::Request| async move {
                let call = Route {
                    route: &route,
                    form,
                    state: &state,
                    guardian: &guardian,
                    params: &params,
                };
                match call.handle(&plugin, request).await {
                    Ok(response) => response,
                    Err(e) => {
                        plugin.log_skip(Phase::Route, &e);
                        StatusCode::INTERNAL_SERVER_ERROR.into_response()
                    }
                }
            };
            match paths.iter_mut().find(|(p, _)| *p == decl.path) {
                Some((_, methods)) => *methods = std::mem::take(methods).on(filter, handler),
                None => paths.push((decl.path.clone(), axum::routing::on(filter, handler))),
            }
        }
        let mut router = Router::new();
        for (path, methods) in paths {
            router = router.route(&path, methods);
        }
        if let Some(public) = &self.plugin.files.public {
            router = router.nest_service(
                &format!("/plugins/{}", self.name),
                tower_http::services::ServeDir::new(public),
            );
        }
        router
    }

    fn jobs(&self) -> &[&'static str] {
        &self.jobs
    }

    fn scheduled(&self) -> Vec<(&'static str, Duration)> {
        self.scheduled.clone()
    }

    async fn run_job(&self, state: &AppState, name: &str, args: &Value) -> Result<(), AppError> {
        let mut conn = state.pool.acquire().await?;
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        let guardian = Guardian::anonymous();
        let mut ctx = Ctx {
            conn: &mut conn,
            settings: &settings,
            guardian: &guardian,
            i18n: Some(&state.i18n),
            pool: Some(&state.pool),
            config: Some(&state.config),
            bus: Some(&state.bus),
        };
        self.plugin
            .run(
                &mut ctx,
                "job",
                Phase::Job,
                json!({ "name": name, "args": args }),
            )
            .await
            .map(|_| ())
            .map_err(|e| AppError::from(GuestError(e)))
    }

    async fn current_user_fields(
        &self,
        ctx: &mut plugin_api::Ctx<'_>,
        guardian: &Guardian,
    ) -> Result<Map<String, Value>, AppError> {
        let Some(decl) = &self.plugin.current_user else {
            return Ok(Map::new());
        };
        let mut ctx = Ctx {
            conn: &mut *ctx.conn,
            settings: ctx.settings,
            guardian,
            i18n: Some(ctx.i18n),
            pool: None,
            config: None,
            bus: None,
        };
        Ok(self
            .keys(&mut ctx, Phase::CurrentUser, Value::Null, &decl.keys)
            .await)
    }

    async fn topic_view_fields(
        &self,
        ctx: &mut plugin_api::Ctx<'_>,
        guardian: &Guardian,
        topic_id: i32,
    ) -> Result<Map<String, Value>, AppError> {
        let Some(decl) = &self.plugin.topic_view else {
            return Ok(Map::new());
        };
        let mut ctx = Ctx {
            conn: &mut *ctx.conn,
            settings: ctx.settings,
            guardian,
            i18n: Some(ctx.i18n),
            pool: None,
            config: None,
            bus: None,
        };
        Ok(self
            .keys(
                &mut ctx,
                Phase::TopicView,
                json!({ "topic_id": topic_id }),
                &decl.keys,
            )
            .await)
    }

    async fn outlet_html(
        &self,
        ctx: &mut plugin_api::Ctx<'_>,
        outlet: &Outlet<'_>,
    ) -> Result<Option<String>, AppError> {
        let Outlet::TopicAbovePosts { view } = outlet;
        if !self.plugin.outlets.topic_above_posts {
            return Ok(None);
        }
        // plugin_api's outlets carry no guardian; the guest gets an
        // anonymous one and the view, which is already the reader's.
        let guardian = Guardian::anonymous();
        let mut ctx = Ctx {
            conn: &mut *ctx.conn,
            settings: ctx.settings,
            guardian: &guardian,
            i18n: Some(ctx.i18n),
            pool: None,
            config: None,
            bus: None,
        };
        let result = self
            .plugin
            .run(
                &mut ctx,
                "outlet",
                Phase::TopicAbovePosts,
                json!({ "view": view }),
            )
            .await
            .and_then(|v| match v {
                Value::Null => Ok(None),
                Value::String(html) => Ok(Some(html)),
                _ => Err("an outlet returns HTML or null".into()),
            });
        Ok(result.unwrap_or_else(|e| {
            self.plugin.log_skip(Phase::TopicAbovePosts, &e);
            None
        }))
    }

    async fn post_menu_buttons(
        &self,
        ctx: &mut plugin_api::Ctx<'_>,
        guardian: &Guardian,
        topic_id: i64,
        posts: &[Value],
    ) -> Result<HashMap<i64, String>, AppError> {
        if !self.plugin.outlets.post_menu {
            return Ok(HashMap::new());
        }
        // What a button decides on, not the whole serialized post.
        let posts: Vec<Value> = posts
            .iter()
            .map(|p| {
                json!({
                    "id": p["id"],
                    "post_number": p["post_number"],
                    "user_id": p["user_id"],
                    "username": p["username"],
                    "yours": p["yours"],
                    "deleted": p["deleted_at"].is_string(),
                })
            })
            .collect();
        let ids: Vec<i64> = posts.iter().filter_map(|p| p["id"].as_i64()).collect();
        let base_path = ctx.base_path;
        let mut ctx = Ctx {
            conn: &mut *ctx.conn,
            settings: ctx.settings,
            guardian,
            i18n: Some(ctx.i18n),
            pool: None,
            config: None,
            bus: None,
        };
        let result = self
            .plugin
            .run(
                &mut ctx,
                "outlet",
                Phase::PostMenu,
                json!({ "topic_id": topic_id, "posts": posts, "base_path": base_path }),
            )
            .await
            .and_then(|v| {
                let Value::Object(by_id) = v else {
                    return Err("post_menu returns {post_id: html}".into());
                };
                let mut out = HashMap::new();
                for (id, html) in by_id {
                    let id: i64 = id.parse().map_err(|_| format!("post id {id}"))?;
                    let Value::String(html) = html else {
                        return Err(format!("post {id}: HTML is a string"));
                    };
                    // Only the posts it was asked about.
                    if ids.contains(&id) {
                        out.insert(id, html);
                    }
                }
                Ok(out)
            });
        Ok(result.unwrap_or_else(|e| {
            self.plugin.log_skip(Phase::PostMenu, &e);
            HashMap::new()
        }))
    }

    fn homepage(&self) -> Option<Homepage> {
        self.homepage
    }

    async fn homepage_available(
        &self,
        conn: &mut sqlx::PgConnection,
        settings: &SiteSettings,
        guardian: &Guardian,
    ) -> Result<bool, AppError> {
        if !self.plugin.homepage.as_ref().is_some_and(|h| h.available) {
            return Ok(true);
        }
        let mut ctx = Ctx {
            conn,
            settings,
            guardian,
            i18n: None,
            pool: None,
            config: None,
            bus: None,
        };
        let result = self
            .plugin
            .run(&mut ctx, "homepage", Phase::HomepageAvailable, Value::Null)
            .await
            .and_then(|v| v.as_bool().ok_or_else(|| "available returns a bool".into()));
        Ok(result.unwrap_or_else(|e| {
            self.plugin.log_skip(Phase::HomepageAvailable, &e);
            false
        }))
    }
}

/// One request to one of the plugin's routes.
struct Route<'a> {
    route: &'a str,
    form: bool,
    state: &'a AppState,
    guardian: &'a Guardian,
    params: &'a RawPathParams,
}

/// `route.handle`'s answer. Only these reach the response: a plugin sets
/// no cookies and no other headers.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    #[serde(default = "ok")]
    status: u16,
    content_type: Option<String>,
    #[serde(default)]
    body: String,
    /// For a redirect.
    location: Option<String>,
}

fn ok() -> u16 {
    200
}

impl Route<'_> {
    async fn handle(
        &self,
        plugin: &Plugin,
        request: axum::extract::Request,
    ) -> Result<Response, String> {
        let s = |e: &dyn std::fmt::Display| e.to_string();
        let (parts, body) = request.into_parts();
        let body = axum::body::to_bytes(body, MAX_BODY)
            .await
            .map_err(|e| s(&e))?;
        let body = String::from_utf8(body.to_vec()).map_err(|_| "a body that is not UTF-8")?;
        let content_type = parts
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let state = self.state;
        let mut conn = state.pool.acquire().await.map_err(|e| s(&e))?;
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals)
                .await
                .map_err(|e| s(&e))?;
        // verify_authenticity_token, before the plugin runs: it can't forget.
        if !matches!(parts.method, Method::GET | Method::HEAD) {
            let form: Vec<(String, String)> =
                if content_type.starts_with("application/x-www-form-urlencoded") {
                    form_urlencoded::parse(body.as_bytes())
                        .into_owned()
                        .collect()
                } else {
                    Vec::new()
                };
            if !plugin_api::verify_authenticity_token(
                state,
                &parts.headers,
                &form,
                parts.uri.path(),
                parts.method.as_str(),
            ) {
                return Ok(crate::routes::bad_csrf());
            }
        }
        let (csrf_token, cookie) = if self.form {
            let (token, cookie) =
                plugin_api::form_authenticity_token(state, &parts.headers, &settings)
                    .map_err(|e| s(&e))?;
            (Some(token), cookie)
        } else {
            (None, None)
        };
        let params: HashMap<&str, &str> = self.params.iter().collect();
        let payload = json!({
            "method": parts.method.as_str(),
            "path": parts.uri.path(),
            "route": self.route,
            "base_path": state.config.globals.relative_url_root(),
            "params": params,
            "query": parts.uri.query(),
            "content_type": content_type,
            "body": body,
            "csrf_token": csrf_token,
        });
        let mut ctx = Ctx {
            conn: &mut conn,
            settings: &settings,
            guardian: self.guardian,
            i18n: Some(&state.i18n),
            pool: Some(&state.pool),
            config: Some(&state.config),
            bus: Some(&state.bus),
        };
        let reply: Reply =
            serde_json::from_value(plugin.run(&mut ctx, "route", Phase::Route, payload).await?)
                .map_err(|e| format!("route.handle's answer: {e}"))?;
        let status = StatusCode::from_u16(reply.status).map_err(|e| s(&e))?;
        let mut response = (status, reply.body).into_response();
        let headers = response.headers_mut();
        let content_type = reply
            .content_type
            .as_deref()
            .unwrap_or("text/plain; charset=utf-8");
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(content_type).map_err(|e| s(&e))?,
        );
        if let Some(location) = reply.location.filter(|_| status.is_redirection()) {
            headers.insert(
                header::LOCATION,
                HeaderValue::from_str(&location).map_err(|e| s(&e))?,
            );
        }
        if let Some(cookie) = cookie {
            headers.append(
                header::SET_COOKIE,
                HeaderValue::from_str(&cookie).map_err(|e| s(&e))?,
            );
        }
        Ok(response)
    }
}
