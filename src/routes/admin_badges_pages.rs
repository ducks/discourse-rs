//! The admin badges pages as HTML (crate::admin_badges_view): the
//! adminBadges service's document (/admin/badges.json), then the tab's
//! outlet. The JSON stays with crate::routes::admin_badges.

use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::Response;
use serde_json::{Value, json};

use crate::admin_badges_view::{self as view, Page};
use crate::{AppError, AppState};

/// The badges page for `which`, titled Badges (the settings tab
/// Settings first).
pub(super) async fn page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    which: Page,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let shell =
        super::admin_pages::Shell::load(&mut conn, state, guardian, headers, "admin_badges")
            .await?;
    let cx = shell.cx(state);
    let doc = super::admin_badges::document(state, &mut conn, &shell.settings).await?;
    let filter: String = form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
        .find(|(k, _)| k == "filter")
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default();
    let find = |id: &str| -> Option<&Value> {
        let id: i64 = id.parse().ok()?;
        doc["badges"]
            .as_array()?
            .iter()
            .find(|b| b["id"].as_i64() == Some(id))
    };
    let mut script = "admin-badges.js";
    // The badge the show form edits, for its data.
    let mut form_badge = Value::Null;
    let (classes, content) = match &which {
        Page::Index => (
            "admin-badges",
            format!(
                "{}{}",
                view::list(&cx, &doc, "/admin/badges/", None, &filter),
                view::intro(&cx)
            ),
        ),
        Page::Settings => {
            script = "site-settings.js";
            (
                "admin-badges settings",
                super::admin_advanced::area_settings(
                    &mut conn,
                    state,
                    &shell,
                    uri,
                    json!({ "filter_area": "badges" }),
                )
                .await?,
            )
        }
        Page::Show(id) => {
            let new_badge;
            let badge = if id == "new" {
                // The route's Badge.create.
                new_badge = json!({
                    "name": crate::admin_view::t(&state.i18n, "admin.badges.new_badge"),
                    "enabled": true,
                    "badge_type_id": doc["badge_types"][0]["id"],
                    "badge_grouping_id": doc["badge_groupings"][0]["id"],
                    "trigger": 0,
                });
                Some(&new_badge)
            } else {
                find(id)
            };
            let mut content = view::list(&cx, &doc, "/admin/badges/", Some(id), &filter);
            if let Some(badge) = badge {
                form_badge = badge.clone();
                content.push_str(&view::show(
                    &cx,
                    &view::ShowData {
                        badge,
                        doc: &doc,
                        badge_sql: shell.settings.get("enable_badge_sql")?.truthy(),
                    },
                ));
            }
            ("admin-badges", content)
        }
        Page::Award(id) => (
            "admin-badges award",
            format!(
                "{}{}",
                view::list(&cx, &doc, "/admin/badges/award/", Some(id), &filter),
                view::award(&cx, find(id))
            ),
        ),
    };
    // The groupings for Group settings' modal and the badge for the form.
    let data = json!({
        "badge": form_badge,
        "badge_groupings": doc["badge_groupings"],
        "grouping_names": doc["badge_groupings"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|g| {
                let name = g["name"].as_str().unwrap_or_default();
                // badgeGroupingDisplayName
                let key = format!(
                    "js.badges.badge_grouping.{}.name",
                    name.to_lowercase().replace(char::is_whitespace, "_")
                );
                state.i18n.t(&key).unwrap_or(name).to_string()
            })
            .collect::<Vec<_>>(),
    });
    let mut main = format!(
        "{}<script type=\"application/json\" id=\"admin-badges-data\">{}</script>{}",
        view::strings(&cx),
        data.to_string().replace('<', "\\u003c"),
        view::render(&cx, &which, &content)
    );
    if matches!(which, Page::Settings) {
        main.push_str(&format!(
            "<script src=\"{}\"></script>",
            crate::assets::url(&shell.site.base_path, "admin-badges.js")
        ));
    }
    let main = crate::admin_ui::contents(classes, "", &main);
    let mut title = crate::admin_view::t(&state.i18n, "admin.config.badges.title");
    if matches!(which, Page::Settings) {
        title = format!(
            "{} - {title}",
            crate::admin_view::t(&state.i18n, "settings")
        );
    }
    shell.render(state, uri, &title, main, script, StatusCode::OK)
}
