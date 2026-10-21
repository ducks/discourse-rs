//! Port of ReviewablesController#perform: PUT
//! /review/:reviewable_id/perform/:action_id, answering with
//! ReviewablePerformResultSerializer.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::review::{self, Outcome};
use crate::review_list::Listed;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn json_error(state: &AppState, status: StatusCode, key: &str) -> Response {
    let text = state.i18n.t(key).unwrap_or(key);
    (status, Json(json!({ "errors": [text] }))).into_response()
}

/// PUT /review/:reviewable_id/perform/:action_id
pub async fn perform(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((reviewable_id, action_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    // requires_login
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    // :reviewable_id is constrained to digits.
    let Some(reviewable_id) = reviewable_id
        .parse::<i64>()
        .ok()
        .filter(|_| reviewable_id.bytes().all(|b| b.is_ascii_digit()))
    else {
        return Ok(super::topics::not_found_response(&state));
    };
    // version_required
    let Some(version) = p
        .get("version")
        .and_then(params::scalar)
        .filter(|v| !v.trim().is_empty())
    else {
        return Ok(json_error(
            &state,
            StatusCode::UNPROCESSABLE_ENTITY,
            "reviewables.missing_version",
        ));
    };
    let version = crate::ruby::to_i(&version);

    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    // ReviewableUser.additional_args
    let param = |name: &str| p.get(name).and_then(params::scalar);
    // ReviewableQueuedPost.additional_args: nothing without a revise_reason.
    let posts = match param("revise_reason").filter(|r| !r.trim().is_empty()) {
        Some(reason) => crate::review_posts::Args {
            revise_reason: Some(reason),
            revise_feedback: param("revise_feedback"),
            revise_custom_reason: param("revise_custom_reason"),
        },
        None => crate::review_posts::Args::default(),
    };
    let extra = review::ExtraArgs {
        reject_reason: param("reject_reason"),
        send_email: param("send_email").as_deref() != Some("false"),
        posts,
        mailer: &state.mailer,
    };
    let outcome = review::perform(
        &mut tx,
        &ctx,
        &guardian,
        reviewable_id,
        &action_id,
        version,
        &extra,
    )
    .await?;
    Ok(match outcome {
        Outcome::Done(result) => {
            tx.commit().await?;
            (StatusCode::OK, Json(result)).into_response()
        }
        Outcome::NotFound => super::topics::not_found_response(&state),
        Outcome::Forbidden => super::search::invalid_access(&state),
        Outcome::Conflict => json_error(&state, StatusCode::CONFLICT, "reviewables.conflict"),
        Outcome::NotFoundMessage(key) => {
            let text = state.i18n.t(key).unwrap_or(key);
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "errors": [text], "error_type": "not_found" })),
            )
                .into_response()
        }
    })
}

/// GET /review.json: ReviewablesController#index.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    // requires_login
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // ensure_can_see: can_see_review_queue?
    if !guardian.is_staff() {
        if settings.get("enable_category_group_moderation")?.truthy() {
            return Err(Unsupported("the review queue for category group moderators").into());
        }
        return Ok(super::search::invalid_access(&state));
    }
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let p = params::parse(uri.query(), &headers, &body);
    Ok(
        match crate::review_list::index(
            &mut tx,
            &crate::web_hooks::Payloads {
                settings: ctx.settings,
                i18n: ctx.i18n,
                config: ctx.config,
            },
            &guardian,
            &p,
        )
        .await?
        {
            Listed::Page(doc) => (StatusCode::OK, Json(doc)).into_response(),
            Listed::InvalidParameter(name) => super::search::invalid_parameters(&state, name),
        },
    )
}

/// The HTML of /review (layout plus the review.index route).
#[derive(askama::Template)]
#[template(path = "review.html")]
pub struct ReviewHtml {
    pub site_title: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: crate::html::Crawler,
    pub viewer: Option<crate::html::Viewer>,
    pub bus_position: String,
    pub chrome: crate::html::Chrome,
    /// The route's titleToken (`Review`), empty on the exception page.
    pub page_title: String,
    pub main: String,
}

/// The review.index route's query params in the order the controller
/// declares them, as its model asks /review.json for them: the defaults
/// filled in (status pending, priority the reviewable_default_visibility
/// setting, sort_order by status), the empty ones left out.
const QUERY_PARAMS: [&str; 14] = [
    "priority",
    "type",
    "status",
    "category_id",
    "topic_id",
    "username",
    "reviewed_by",
    "claimed_by",
    "from_date",
    "to_date",
    "sort_order",
    "additional_filters",
    "flagged_by",
    "score_type",
];

fn encode(pairs: &[(String, String)]) -> String {
    form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// GET /review: the Ember app's page, behind requires_login. Its model is
/// /review.json with the route's params; staff see the queue, anyone else
/// the exception page that request's 403 leaves.
pub async fn page(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    use askama::Template;

    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let bus_position = crate::bus::page_position(&state.bus).await?;
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let default_priority = settings
        .get("reviewable_default_visibility")?
        .to_s()
        .to_string();
    let q = params::parse(uri.query(), &headers, &Bytes::new());
    let given = |k: &str| q.get(k).and_then(params::scalar).filter(|v| !v.is_empty());
    let status = given("status").unwrap_or_else(|| "pending".into());
    let mut pairs: Vec<(String, String)> = Vec::new();
    for key in QUERY_PARAMS {
        let value = match key {
            "priority" => Some(given(key).unwrap_or_else(|| default_priority.clone())),
            "status" => Some(status.clone()),
            "sort_order" => Some(given(key).unwrap_or_else(|| {
                if status == "reviewed" || status == "all" {
                    "created_at".into()
                } else {
                    "score".into()
                }
            })),
            _ => given(key),
        };
        if let Some(v) = value {
            pairs.push((key.to_string(), v));
        }
    }
    let base_path = state.config.globals.relative_url_root().to_string();
    let request_url = format!("{base_path}/review?{}", encode(&pairs));
    // The View All link (and the sidebar's Review): the params that
    // differ from the controller's defaults, which leave sort_order unset.
    let view_all: Vec<(String, String)> = pairs
        .iter()
        .filter(|(k, v)| match k.as_str() {
            "priority" => *v != default_priority,
            "status" => v != "pending",
            _ => true,
        })
        .cloned()
        .collect();
    let view_all_query = if view_all.is_empty() {
        String::new()
    } else {
        format!("?{}", encode(&view_all))
    };

    if !guardian.is_staff() && settings.get("enable_category_group_moderation")?.truthy() {
        return Err(Unsupported("the review queue for category group moderators").into());
    }
    let listed = if guardian.is_staff() {
        let p: Map<String, Value> = pairs
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        Some(
            crate::review_list::index(
                &mut tx,
                &crate::web_hooks::Payloads {
                    settings: &settings,
                    i18n: &state.i18n,
                    config: &state.config,
                },
                &guardian,
                &p,
            )
            .await?,
        )
    } else {
        None
    };

    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, &base_path)?;
    site.viewer = vs.viewer.clone();
    let active = if matches!(listed, Some(Listed::Page(_))) {
        crate::sidebar::Active::Review(view_all_query.clone())
    } else {
        crate::sidebar::Active::None
    };
    site.load_chrome(&mut tx, &state, &settings, &guardian, active)
        .await?;
    site.bus_position = bus_position;
    let categories = crate::topic_list_view::categories(&mut tx).await?;
    let cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    let (main, page_title) = match &listed {
        Some(Listed::Page(doc)) => {
            let viewer = crate::admin_view::AdminViewer::from_current_user(&json!({
                "admin": guardian.is_admin(),
                "staff": guardian.is_staff(),
                "id": guardian.user_id(),
            }));
            let acx = crate::admin_view::AdminCx {
                i18n: &state.i18n,
                settings: &settings,
                base_path: &base_path,
                viewer: &viewer,
            };
            let claiming = settings.get("reviewable_claiming")?.to_s().to_string();
            let main = crate::review_view::render(&crate::review_view::Page {
                cx: &cx,
                acx: &acx,
                doc,
                viewer_id: guardian.user_id().map(i64::from).unwrap_or(0),
                admin: guardian.is_admin(),
                claiming: &claiming,
                blur_tl0: settings.get("blur_tl0_flagged_posts_media")?.truthy(),
                view_all_query: &view_all_query,
                default_priority: &default_priority,
            });
            (main, crate::topic_list_view::t(&cx, "review.title"))
        }
        // Discourse::InvalidParameters: the 400 the model's request gets.
        Some(Listed::InvalidParameter(_)) => (
            crate::messages_view::exception(&cx, 400, &request_url),
            String::new(),
        ),
        // ensure_can_see: a 403.
        None => (
            crate::messages_view::exception(&cx, 403, &request_url),
            String::new(),
        ),
    };
    // Listing writes nothing that should stay.
    tx.rollback().await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let crawler = crate::html::Crawler::for_request(&urls, &uri, None)?;
    let page = ReviewHtml {
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        page_title,
        main,
    };
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        &settings,
        &vs,
    )?)
}
