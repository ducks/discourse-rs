//! Admin::Config::AboutController: the about page (90g6), its update
//! (SiteSetting::Update over the sections' settings) and the About
//! page's localizations (SiteSettingLocalizations::AboutConfig::Show and
//! Update). Admins only (AdminConstraint).

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::admin_pages::{Shell, wants_json};
use super::session::{bad_csrf, csrf_ok};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, params};

/// The hidden settings the update may change (`allow_changing_hidden:`).
const ALLOW_HIDDEN: [&str; 4] = [
    "extended_site_description",
    "extended_site_description_cooked",
    "about_banner_image",
    "community_owner",
];

/// `SiteSettingLocalization::ABOUT_CONFIG_PARAM_MAP`
const LOCALIZED: [(&str, &str); 3] = [
    ("name", "title"),
    ("summary", "site_description"),
    ("extended_description", "extended_site_description"),
];

pub(super) fn params_of(
    state: &AppState,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
    method: &str,
) -> Option<Map<String, Value>> {
    let p = params::parse(uri.query(), headers, body);
    let form: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    csrf_ok(state, headers, &form, uri.path(), method).then_some(p)
}

/// `section[key].to_s`
fn value(section: &Map<String, Value>, key: &str) -> String {
    section
        .get(key)
        .and_then(params::scalar)
        .unwrap_or_default()
}

fn section<'a>(p: &'a Map<String, Value>, name: &str) -> Option<&'a Map<String, Value>> {
    p.get(name).and_then(Value::as_object)
}

/// PUT /admin/config/about(.json)
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(p) = params_of(&state, &headers, &uri, &body, "PUT") else {
        return Ok(bad_csrf());
    };
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut settings: Vec<(String, String, bool)> = Vec::new();
    let mut add = |name: &str, v: String| settings.push((name.to_string(), v, false));
    if let Some(g) = section(&p, "general_settings") {
        add("title", value(g, "name"));
        add("site_description", value(g, "summary"));
        add("about_banner_image", value(g, "about_banner_image"));
        let extended = value(g, "extended_description");
        add("extended_site_description", extended.clone());
        add("short_site_description", value(g, "community_title"));
        // PrettyText.markdown of the description, or "".
        let cooked = if crate::ruby::is_blank(&extended) {
            String::new()
        } else {
            let host = crate::pretty_text::Host::from_state(&state);
            crate::pretty_text::markdown(
                &host,
                &extended,
                &crate::pretty_text::MarkdownOptions::default(),
            )
            .await?
        };
        add("extended_site_description_cooked", cooked);
    }
    if let Some(c) = section(&p, "contact_information") {
        add("community_owner", value(c, "community_owner"));
        add("contact_email", value(c, "contact_email"));
        add("contact_url", value(c, "contact_url"));
        add("site_contact_username", value(c, "contact_username"));
        add("site_contact_group_name", value(c, "contact_group_name"));
    }
    if let Some(o) = section(&p, "your_organization") {
        add("company_name", value(o, "company_name"));
        add("company_url", value(o, "company_url"));
        add("governing_law", value(o, "governing_law"));
        add("city_for_disputes", value(o, "city_for_disputes"));
    }
    if let Some(e) = section(&p, "extra_groups") {
        add("about_page_extra_groups", value(e, "groups"));
        add(
            "about_page_extra_groups_initial_members",
            value(e, "initial_members"),
        );
        add("about_page_extra_groups_order", value(e, "order"));
        add(
            "about_page_extra_groups_show_description",
            value(e, "show_description"),
        );
    }
    if settings.is_empty() {
        // `validates :settings, presence: true`: a failed contract the
        // controller has no handler for, rendering nothing.
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let outcome =
        super::admin_config_branding::update_settings(&state, &guardian, settings, &ALLOW_HIDDEN)
            .await?;
    Ok(match outcome {
        Ok(()) => Json(json!({ "success": "OK" })).into_response(),
        // An invalid value raises in the service's try block, which the
        // controller has no handler for: nothing is rendered.
        Err(_) => StatusCode::NO_CONTENT.into_response(),
    })
}

/// `SiteSettingLocalization.normalize_locale`
fn normalize_locale(locale: &str) -> String {
    locale.replace('-', "_")
}

/// `supported_content_locale?`: a supported content locale other than
/// the default one.
fn supported(settings: &SiteSettings, locale: &str) -> Result<bool, AppError> {
    let default = normalize_locale(&settings.get("default_locale")?.to_s());
    if locale == default {
        return Ok(false);
    }
    Ok(settings
        .get("content_localization_supported_locales")?
        .to_s()
        .split('|')
        .filter(|l| !l.is_empty())
        .any(|l| normalize_locale(l) == locale))
}

/// `SiteSettingLocalization.about_config_payload`
async fn payload(conn: &mut sqlx::PgConnection, locale: &str) -> Result<Value, AppError> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT setting_name, value, cooked FROM site_setting_localizations \
         WHERE locale = $1 AND setting_name = ANY($2) AND value <> '' ORDER BY id",
    )
    .bind(locale)
    .bind(LOCALIZED.iter().map(|(_, s)| *s).collect::<Vec<_>>())
    .fetch_all(&mut *conn)
    .await?;
    let localizations: Map<String, Value> = rows
        .into_iter()
        .map(|(name, value, cooked)| (name, json!({ "value": value, "cooked": cooked })))
        .collect();
    Ok(json!({ "locale": locale, "localizations": localizations }))
}

/// GET and PUT /admin/config/about/localizations(.json)
pub async fn localizations(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: axum::http::Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let write = method == axum::http::Method::PUT;
    let p = if write {
        match params_of(&state, &headers, &uri, &body, "PUT") {
            Some(p) => p,
            None => return Ok(bad_csrf()),
        }
    } else {
        params::parse(uri.query(), &headers, &body)
    };
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // ensure_can_localize_site_settings
    if !settings.get("content_localization_enabled")?.truthy() {
        return Ok(super::search::invalid_access(&state));
    }
    let locale = normalize_locale(&value(&p, "locale"));
    if locale.is_empty() || !supported(&settings, &locale)? {
        return Ok(super::search::invalid_parameters(&state, "locale"));
    }
    if write {
        let acting = guardian
            .user_id()
            .ok_or(crate::Unsupported("localizing settings anonymously"))?;
        // about_config_settings_from_params: the given keys of each section.
        let submitted: Vec<(&str, String)> = match section(&p, "general_settings") {
            Some(g) => LOCALIZED
                .iter()
                .filter(|(param, _)| g.contains_key(*param))
                .map(|(param, setting)| (*setting, value(g, param)))
                .collect(),
            None => Vec::new(),
        };
        let mut tx = state.pool.begin().await?;
        for (setting, v) in &submitted {
            if crate::ruby::is_blank(v) {
                sqlx::query(
                    "DELETE FROM site_setting_localizations WHERE setting_name = $1 AND locale = $2",
                )
                .bind(setting)
                .bind(&locale)
                .execute(&mut *tx)
                .await?;
                continue;
            }
            // localizable max_length: title 255, site_description 1000,
            // extended_site_description 10,000 (with its cooked).
            let max = match *setting {
                "title" => 255,
                "site_description" => 1000,
                _ => 10_000,
            };
            if v.chars().count() > max {
                return Err(crate::Unsupported("a localization over its maximum length").into());
            }
            let cooked = if *setting == "extended_site_description" {
                let host = crate::pretty_text::Host::from_state(&state);
                Some(
                    crate::pretty_text::markdown(
                        &host,
                        v,
                        &crate::pretty_text::MarkdownOptions::default(),
                    )
                    .await?,
                )
            } else {
                None
            };
            sqlx::query(
                "INSERT INTO site_setting_localizations \
                   (setting_name, locale, value, cooked, localizer_user_id, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, clock_timestamp(), clock_timestamp()) \
                 ON CONFLICT (setting_name, locale) DO UPDATE SET value = EXCLUDED.value, \
                   cooked = EXCLUDED.cooked, localizer_user_id = EXCLUDED.localizer_user_id, \
                   updated_at = EXCLUDED.updated_at \
                 WHERE site_setting_localizations.value IS DISTINCT FROM EXCLUDED.value \
                    OR site_setting_localizations.cooked IS DISTINCT FROM EXCLUDED.cooked \
                    OR site_setting_localizations.localizer_user_id IS DISTINCT FROM EXCLUDED.localizer_user_id",
            )
            .bind(setting)
            .bind(&locale)
            .bind(v)
            .bind(&cooked)
            .bind(acting)
            .execute(&mut *tx)
            .await?;
        }
        if !submitted.is_empty() {
            let mut names: Vec<&str> = submitted.iter().map(|(s, _)| *s).collect();
            names.sort();
            names.dedup();
            crate::staff_action_logs::log_custom(
                &mut tx,
                acting,
                "update_site_setting_localizations",
                &[
                    ("locale", locale.clone()),
                    ("setting_names", names.join("|")),
                ],
                None,
                None,
            )
            .await?;
        }
        tx.commit().await?;
    }
    Ok(Json(payload(&mut conn, &locale).await?).into_response())
}

/// GET /admin/config/about(.json): the page; for JSON the controller's
/// empty index (204).
pub async fn page(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    if wants_json(&uri, &headers) {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(
        &mut conn,
        &state,
        &guardian,
        &headers,
        "admin_about_your_site",
    )
    .await?;
    // The route's model: /admin/config/site_settings.json for the area.
    let filters = super::admin_site_settings::config_filters(
        &state,
        json!({ "filter_area": "about" })
            .as_object()
            .ok_or_else(|| std::io::Error::other("the about filter"))?,
    )
    .ok_or_else(|| std::io::Error::other("the about filter"))?;
    let (settings, _) =
        super::admin_site_settings::document(&mut conn, &state, &shell.settings, &filters).await?;
    let groups: Vec<(i64, String)> =
        sqlx::query_as("SELECT id::bigint, name FROM groups ORDER BY name")
            .fetch_all(&mut *conn)
            .await?;
    // The site contact chosen: UserChooser shows the username.
    let contact = shell.settings.get("site_contact_username")?.to_s();
    let contact_name = (!contact.is_empty()).then(|| (contact.clone(), contact.clone()));
    // DEditor's toolbar, as the composer draws it.
    let base_path = shell.site.base_path.clone();
    let categories = std::collections::HashMap::new();
    let list = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        hide_category: false,
        member_trust_level: None,
        settings: crate::topic_list_view::ListSettings::load(&shell.settings)?,
    };
    let toolbar = crate::composer_view::toolbar(&list, None, None);
    let default_locale = shell.settings.get("default_locale")?.to_s();
    // The language selector: the default locale and the supported ones,
    // named as available_locales names them.
    let locales = if shell.settings.get("content_localization_enabled")?.truthy() {
        let mut wanted = vec![default_locale.clone()];
        wanted.extend(
            shell
                .settings
                .get("content_localization_supported_locales")?
                .to_s()
                .split('|')
                .filter(|l| !l.is_empty())
                .map(str::to_string),
        );
        let names = crate::setting_enums::recorded("default_locale")
            .and_then(|r| r.get("valid_values"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Some(
            names
                .iter()
                .filter_map(|v| {
                    let value = v["value"].as_str()?;
                    wanted.iter().any(|w| w == value).then(|| {
                        let key = v["name"].as_str().unwrap_or_default();
                        (
                            value.to_string(),
                            crate::admin_view::t_opt(&state.i18n, key)
                                .unwrap_or_else(|| key.to_string()),
                        )
                    })
                })
                .collect(),
        )
    } else {
        None
    };
    let cx = shell.cx(&state);
    let content = crate::admin_about_view::page(
        &cx,
        &crate::admin_about_view::About {
            settings: &settings,
            groups: &groups,
            contact_name,
            toolbar: &toolbar,
            locales,
            default_locale: &default_locale,
        },
    );
    let strings =
        crate::admin_ui::strings(&cx, "admin-about-strings", crate::admin_about_view::STRINGS);
    let main = crate::admin_view::contents("admin-config about", &format!("{content}{strings}"));
    let title = crate::admin_view::t(cx.i18n, "admin.config.about.title");
    shell.render(
        &state,
        &uri,
        &title,
        main,
        "admin-branding.js",
        StatusCode::OK,
    )
}

pub(super) fn routes(mut router: axum::Router<AppState>) -> axum::Router<AppState> {
    use axum::routing::get;
    for suffix in ["", ".json"] {
        router = router
            .route(
                &format!("/admin/config/about{suffix}"),
                get(page).put(update),
            )
            .route(
                &format!("/admin/config/about/localizations{suffix}"),
                get(localizations).put(localizations),
            );
    }
    router
}
