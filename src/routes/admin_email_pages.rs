//! The admin email pages as HTML (crate::admin_email_view): what each
//! tab's route loads, drawn in the admin shell. The JSON stays with
//! crate::routes::admin_email and crate::routes::admin_email_templates.

use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::Response;
use serde_json::json;

use crate::admin_email_view::{self as view, Tab};
use crate::{AppError, AppState};

/// The page for the tab: the route's model, then the header and the
/// outlet. Every tab is titled Email settings, the settings tab twice
/// (its route's titleToken and the parent's).
pub(super) async fn page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    tab: Tab,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let shell =
        super::admin_pages::Shell::load(&mut conn, state, guardian, headers, "admin_server_setup")
            .await?;
    let cx = shell.cx(state);
    let mut script = "admin-email.js";
    let content = match tab {
        Tab::Settings => {
            script = "site-settings.js";
            view::settings(
                &super::admin_advanced::area_settings(
                    &mut conn,
                    state,
                    &shell,
                    uri,
                    json!({ "filter_area": "email" }),
                )
                .await?,
            )
        }
        Tab::ServerSettings => {
            view::server_settings(&cx, &super::admin_email::server_settings_document(state))
        }
        Tab::AdvancedTest => view::advanced_test(&cx),
        Tab::PreviewDigest => {
            // EmailPreview.findDigest for the viewer, a week back.
            let username = guardian
                .user()
                .map(|u| u.username.clone())
                .unwrap_or_default();
            let last_seen = (crate::clock::now() - chrono::Duration::days(7))
                .format("%Y-%m-%d")
                .to_string();
            let preview = super::admin_email::digest_preview(state, &last_seen, &username)
                .await?
                .unwrap_or_default();
            view::preview_digest(
                &cx,
                &view::Preview {
                    username: &username,
                    last_seen: &last_seen,
                    html_content: preview["html_content"].as_str().unwrap_or_default(),
                    text_content: preview["text_content"].as_str().unwrap_or_default(),
                    mobile: crate::mobile::mobile_view(),
                },
            )
        }
        Tab::Templates => {
            let doc = super::admin_email_templates::document(
                &mut conn,
                state,
                &shell.settings,
                guardian.user_id().unwrap_or_default(),
            )
            .await?;
            let query: Vec<(String, String)> =
                form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
                    .into_owned()
                    .collect();
            let param = |k: &str| {
                query
                    .iter()
                    .find(|(name, _)| name == k)
                    .map(|(_, v)| v.as_str())
            };
            view::templates(
                &cx,
                &doc,
                param("filter").unwrap_or_default(),
                param("overridden") == Some("true"),
            )
        }
    };
    let mut main = format!("{}{}", view::strings(&cx), view::render(&cx, tab, &content));
    if tab == Tab::Settings {
        main.push_str(&format!(
            "<script src=\"{}\"></script>",
            crate::assets::url(&shell.site.base_path, "admin-email.js")
        ));
    }
    let main = crate::admin_ui::contents(tab.classes(), "", &main);
    let mut title = crate::admin_view::t(&state.i18n, "admin.config.email.title");
    if tab == Tab::Settings {
        title = format!("{title} - {title}");
    }
    shell.render(state, uri, &title, main, script, StatusCode::OK)
}
