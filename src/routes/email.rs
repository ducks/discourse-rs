//! Port of email_controller.rb: the pages an email's unsubscribe link
//! opens (`GET /email/unsubscribe/:key`), the form's post
//! (`POST /email/unsubscribe/:key`) and the confirmation
//! (`GET /email/unsubscribed`), in the no_ember layout. The controller
//! skips check_xhr and the login and profile redirects.
//!
//! Its `skip_before_action :verify_authenticity_token, only:
//! [:unsubscribe], if: :one_click_unsubscribe?` never applies (the action
//! is a GET, the condition wants a POST), so a One-Click post is checked
//! like any other.

use askama::Template;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

use super::session::{bad_csrf, csrf_ok};
use crate::email::unsubscribe::{self, Page};
use crate::guardian::Guardian;
use crate::html::{Chrome, Crawler};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// A server-rendered page in Rails' no_ember layout.
#[derive(Template)]
#[template(path = "email_page.html")]
struct EmailPage {
    site_title: String,
    lang: String,
    base_path: String,
    crawler: Crawler,
    viewer: Option<crate::html::Viewer>,
    bus_position: String,
    chrome: Chrome,
    html: String,
}

/// The views' texts in English, as the site has them.
async fn refuse_untranslated(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
) -> Result<(), AppError> {
    let locale = settings.get("default_locale")?.to_s();
    if locale != "en" {
        return Err(Unsupported("the unsubscribe pages in locales other than English").into());
    }
    let overridden: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM translation_overrides WHERE locale = $1 \
         AND (translation_key LIKE 'unsubscribe.%' OR translation_key LIKE 'unsubscribed.%'))",
    )
    .bind(&locale)
    .fetch_one(&mut *conn)
    .await?;
    if overridden {
        return Err(Unsupported("translation overrides of the unsubscribe pages").into());
    }
    Ok(())
}

fn host(headers: &HeaderMap) -> &str {
    headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost")
}

/// The page in the no_ember layout, with the viewer's headers.
async fn render_page(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    uri: &Uri,
    guardian: &Guardian,
    html: String,
    vs: crate::html::ViewerState,
) -> Result<Response, AppError> {
    let base_path = state.config.globals.relative_url_root().to_string();
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let mut site = crate::html::Site::from_settings(settings, &base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        conn,
        state,
        settings,
        guardian,
        crate::sidebar::Active::None,
    )
    .await?;
    let mut crawler = Crawler::for_request(&urls, uri, None)?;
    crawler.description = String::new();
    let body = EmailPage {
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler,
        viewer: site.viewer,
        bus_position: String::new(),
        chrome: site.chrome,
        html,
    }
    .render()
    .map_err(crate::html::HtmlError::from)?;
    let response = (StatusCode::OK, axum::response::Html(body)).into_response();
    Ok(crate::html::with_viewer_headers(response, &vs))
}

/// GET /email/unsubscribe/:key (EmailController#unsubscribe)
pub async fn unsubscribe(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    Path(key): Path<String>,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    refuse_untranslated(&mut conn, &settings).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let base_path = state.config.globals.relative_url_root().to_string();

    // The form's token: the viewer's, else an anonymous session's.
    let mut vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let csrf = match &vs.viewer {
        Some(viewer) => viewer.csrf_token.clone(),
        None => {
            let (csrf, set_cookie) = super::session::anonymous_csrf(&state, &headers, &settings)?;
            vs.set_cookie = set_cookie;
            csrf
        }
    };

    let found = unsubscribe::find_key(&mut conn, &key).await?;
    let owner = match &found {
        Some(k) => unsubscribe::owner(&mut conn, k.user_id).await?,
        None => None,
    };
    let html = match (&found, &owner) {
        (None, _) => unsubscribe::render_unsubscribe(
            &settings,
            &state.i18n,
            &urls,
            &key,
            &Page::NotFound,
            Some(&csrf),
            "",
        )?,
        (Some(_), None) => unsubscribe::render_unsubscribe(
            &settings,
            &state.i18n,
            &urls,
            &key,
            &Page::UserNotFound,
            Some(&csrf),
            "",
        )?,
        (Some(k), Some(owner)) => {
            let options = unsubscribe::prepare(&mut conn, &settings, &state.i18n, k, owner).await?;
            // @different_user = @user.name: only a user with a name counts.
            let different = guardian
                .user()
                .filter(|u| u.id != owner.id && owner.name.is_some());
            match different {
                Some(current) => {
                    let return_url = format!(
                        "http://{}{}",
                        host(&headers),
                        uri.path_and_query().map(|p| p.as_str()).unwrap_or("/")
                    );
                    unsubscribe::render_unsubscribe(
                        &settings,
                        &state.i18n,
                        &urls,
                        &key,
                        &Page::DifferentUser {
                            username_lower: &current.username.to_lowercase(),
                            return_url: &return_url,
                        },
                        Some(&csrf),
                        "",
                    )?
                }
                None => {
                    let badge = match &options.topic {
                        Some(topic) if options.watched_category => match topic.category_id {
                            Some(id) => {
                                crate::category_badge::html_for_with(
                                    &mut conn, &settings, &base_path, id, true,
                                )
                                .await?
                            }
                            None => String::new(),
                        },
                        _ => String::new(),
                    };
                    unsubscribe::render_unsubscribe(
                        &settings,
                        &state.i18n,
                        &urls,
                        &key,
                        &Page::Form(&options),
                        Some(&csrf),
                        &badge,
                    )?
                }
            }
        }
    };
    render_page(&state, &mut conn, &settings, &uri, &guardian, html, vs).await
}

/// `redirect_to` a path of the site: the absolute URL.
fn redirect(headers: &HeaderMap, location: &str) -> Response {
    (
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("http://{}{location}", host(headers)),
        )],
    )
        .into_response()
}

/// `redirect_back fallback_location: path("/")`: to the referrer when it
/// is this host's (raise_on_open_redirects), else the fallback.
fn redirect_back(headers: &HeaderMap, base_path: &str) -> Response {
    let referer = headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .filter(|r| !r.is_empty());
    if let Some(referer) = referer {
        let request_host = host(headers).split(':').next().unwrap_or("");
        let same_host = match referer.split_once("://") {
            Some((_, rest)) => {
                let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
                let host = authority.rsplit('@').next().unwrap_or("");
                host.split(':').next().unwrap_or("") == request_host
            }
            // A relative URL has no host to object to.
            None => referer.starts_with('/') && !referer.starts_with("//"),
        };
        if same_host {
            return (StatusCode::FOUND, [(header::LOCATION, referer.to_string())]).into_response();
        }
    }
    redirect(headers, &format!("{base_path}/"))
}

/// POST /email/unsubscribe/:key (EmailController#perform_unsubscribe)
pub async fn perform_unsubscribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
    uri: Uri,
    Path(key): Path<String>,
    body: Bytes,
) -> Result<Response, AppError> {
    let mut params = crate::params::parse(uri.query(), &headers, &body);
    let form: Vec<(String, String)> = params
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !csrf_ok(&state, &headers, &form, uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let ip = crate::remote_ip::remote_ip(&headers, peer)?;
    let limiter = crate::rate_limiter::RateLimiter::new(None, &format!("unsubscribe_{ip}"), 10, 60);
    if let Err(refused) = limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    let mut conn = state.pool.acquire().await?;
    let Some(found) = unsubscribe::find_key(&mut conn, &key).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let Some(owner) = unsubscribe::owner(&mut conn, found.user_id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if params.get("List-Unsubscribe").and_then(|v| v.as_str()) == Some("One-Click") {
        params.insert("digest_after_minutes".into(), serde_json::json!(0));
        params.insert("unwatch_topic".into(), serde_json::json!(true));
        params.insert("unwatch_category".into(), serde_json::json!(true));
        params.insert("unsubscribe_all".into(), serde_json::json!(true));
    }
    let mut tx = state.pool.begin().await?;
    let updated = unsubscribe::unsubscribe(&mut tx, &found, owner.id, &params).await?;
    tx.commit().await?;
    let base_path = state.config.globals.relative_url_root();
    if !updated {
        return Ok(redirect_back(&headers, base_path));
    }
    // The address, kept an hour for the page that confirms it.
    let cache_key = format!("unsub_{}", crate::accounts::random_hex());
    crate::owned_schema::cached_setex(
        &mut conn,
        &format!("_CACHE:{cache_key}"),
        3600,
        owner.email.as_deref().unwrap_or(""),
    )
    .await?;
    let mut location = format!("{base_path}/email/unsubscribed?key={cache_key}");
    if let Some(topic) = unsubscribe::associated_topic(&mut conn, &found).await? {
        location.push_str(&format!("&topic_id={}", topic.id));
    }
    Ok(redirect(&headers, &location))
}

/// GET /email/unsubscribed (EmailController#unsubscribed)
pub async fn unsubscribed(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let params = crate::params::parse_query(uri.query().unwrap_or(""));
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    refuse_untranslated(&mut conn, &settings).await?;
    let email = match crate::params::string(&params, "key") {
        Some(key) => crate::owned_schema::cached_get(&mut conn, &format!("_CACHE:{key}")).await?,
        None => None,
    };
    // User.find_by_email: any of a user's addresses, case-insensitively.
    let known = match &email {
        Some(email) => {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM user_emails WHERE lower(email) = lower($1))",
            )
            .bind(email)
            .fetch_one(&mut *conn)
            .await?
        }
        None => false,
    };
    if !known {
        return Ok(super::topics::not_found_response(&state));
    }
    // Any topic_id given, "" too, says one was named.
    let topic_id = params.get("topic_id").filter(|v| !v.is_null());
    let mut topic = None;
    if let Some(id) = topic_id
        .and_then(crate::params::scalar)
        .and_then(|s| s.parse::<i32>().ok())
    {
        let row: Option<unsubscribe::KeyTopic> = sqlx::query_as(
            "SELECT id, title, slug, category_id FROM topics WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(row) = row {
            let anonymous = Guardian::anonymous();
            let visible = match crate::topic_guardian::TopicCtx::load(
                &mut conn, &settings, &anonymous, row.id,
            )
            .await?
            {
                Some(ctx) => anonymous.can_see_topic(&settings, &ctx, true, &[])?,
                None => false,
            };
            if visible {
                topic = Some(row);
            }
        }
    }
    let html = unsubscribe::render_unsubscribed(
        &state.i18n,
        state.config.globals.relative_url_root(),
        email.as_deref().unwrap_or(""),
        topic.as_ref(),
        topic_id.is_some(),
    )?;
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    render_page(&state, &mut conn, &settings, &uri, &guardian, html, vs).await
}
