//! The customize admin pages (90g7) as HTML: the routes' JSON
//! controllers hand a page request here.

use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

use super::admin_pages::Shell;
use crate::{AppError, AppState};

/// GET /admin/customize/colors(/:id): Rails' color_schemes#index for the
/// JSON; a page request goes on to the color palettes' page, as the
/// adminCustomize.colors and colors-show routes' replaceWith do.
pub async fn customize_colors(
    state: axum::extract::State<AppState>,
    guardian: crate::session::current::AuthGuardian,
    id: Option<axum::extract::Path<String>>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.0.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    if super::admin_pages::wants_json(&uri, &headers) {
        return super::color_schemes::index(state, guardian, headers, uri).await;
    }
    let mut to = format!(
        "{}/admin/config/colors",
        state.config.globals.relative_url_root()
    );
    if let Some(axum::extract::Path(id)) = id {
        to.push('/');
        to.push_str(&id);
    }
    Ok(axum::response::Redirect::to(&to).into_response())
}

/// GET /admin/config/colors as the page (adminConfig.colorPalettes.index)
/// over its document; `filter` and `type` are DFilterControls' query
/// params.
pub(super) async fn colors_page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    doc: &serde_json::Value,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, state, guardian, headers, "admin_color_palettes").await?;
    drop(conn);
    let query = crate::params::parse_query(uri.query().unwrap_or(""));
    let filters = crate::admin_colors_view::ListFilters {
        filter: crate::params::string(&query, "filter").unwrap_or_default(),
        kind: crate::params::string(&query, "type").unwrap_or_default(),
    };
    let theme = default_theme(&doc["extras"]["default_theme"]);
    let cx = shell.cx(state);
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-customize-strings",
        &[
            "admin.customize.colors.select_base.title",
            "admin.customize.colors.select_base.description",
            "admin.customize.colors.new_name",
            "admin.customize.colors.delete_confirm",
            "admin.customize.colors.set_default_success",
            "admin.customize.colors.default_error",
            "admin.customize.colors.filters.no_results",
            "admin.customize.new",
            "admin.customize.delete",
            "cancel",
            "generic_error",
        ],
    );
    let data = format!(
        "<script type=\"application/json\" id=\"color-palettes-data\">{}</script>",
        doc.to_string().replace('<', "\\u003c")
    );
    let content = format!(
        "{}{data}{strings}",
        crate::admin_colors_view::list(&cx, doc, theme.as_ref(), &filters)
    );
    let main = crate::admin_view::contents("admin-config color-palettes", &content);
    shell.render(state, uri, "", main, "admin-customize.js", StatusCode::OK)
}

/// The default theme's ids and name from its serialization.
fn default_theme(theme: &serde_json::Value) -> Option<crate::admin_colors_view::DefaultTheme> {
    Some(crate::admin_colors_view::DefaultTheme {
        id: theme["id"].as_i64()?,
        name: theme["name"].as_str().unwrap_or_default().to_string(),
        color_scheme_id: theme["color_scheme_id"].as_i64(),
        dark_color_scheme_id: theme["dark_color_scheme_id"].as_i64(),
    })
}

/// GET /admin/config/colors/:id as the page (adminConfig.colorPalettes
/// .show): the palette's editor, the default theme the site's
/// user_themes name.
pub(super) async fn color_palette_page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    palette: &serde_json::Value,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, state, guardian, headers, "admin_color_palettes").await?;
    let default_id = shell.settings.get("default_theme_id")?.to_i();
    let row: Option<(i32, String, Option<i32>, Option<i32>)> = sqlx::query_as(
        "SELECT id, name, color_scheme_id, dark_color_scheme_id FROM themes WHERE id = $1",
    )
    .bind(default_id as i32)
    .fetch_optional(&mut *conn)
    .await?;
    drop(conn);
    let theme = row.map(
        |(id, name, light, dark)| crate::admin_colors_view::DefaultTheme {
            id: id.into(),
            name,
            color_scheme_id: light.map(Into::into),
            dark_color_scheme_id: dark.map(Into::into),
        },
    );
    let cx = shell.cx(state);
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-customize-strings",
        &[
            "admin.config_areas.color_palettes.palette_name",
            "admin.config_areas.color_palettes.unsaved_changes",
            "admin.config_areas.color_palettes.copied_to_clipboard",
            "admin.config_areas.color_palettes.copy_to_clipboard_error",
            "admin.config_areas.color_palettes.copy_of",
            "admin.config_areas.color_palettes.copy_created",
            "admin.config_areas.color_palettes.delete_confirm",
            "admin.config_areas.color_palettes.illegal_character_in_color",
            "admin.config_areas.color_palettes.invalid_color_length",
            "admin.customize.delete",
            "form_kit.errors_summary_title.one",
            "form_kit.errors_summary_title.other",
            "form_kit.errors.required",
            "saved",
            "cancel",
            "generic_error",
        ],
    );
    let data = format!(
        "<script type=\"application/json\" id=\"color-palette-data\">{}</script>",
        palette.to_string().replace('<', "\\u003c")
    );
    let content = format!(
        "{}{data}{strings}",
        crate::admin_colors_view::editor(&cx, palette, theme.as_ref())
    );
    let main = crate::admin_view::contents("admin-config color-palettes", &content);
    shell.render(state, uri, "", main, "admin-customize.js", StatusCode::OK)
}

/// GET /admin/customize/email_style/:field as the page
/// (adminCustomizeEmailStyle.edit) over EmailStyleSerializer's document.
pub(super) async fn email_style_page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    doc: &serde_json::Value,
    field: &str,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, state, guardian, headers, "admin_appearance").await?;
    drop(conn);
    let cx = shell.cx(state);
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-customize-strings",
        &[
            "admin.customize.email_style.reset_confirm",
            "admin.customize.email_style.html",
            "admin.customize.email_style.css",
            "admin.customize.email_style.save_error_with_reason",
            "admin.customize.theme.unsaved_changes_alert",
            "admin.customize.theme.discard",
            "admin.customize.theme.stay",
            "admin.customize.save",
            "saving",
            "generic_error",
        ],
    );
    let data = format!(
        "<script type=\"application/json\" id=\"email-style-data\">{}</script>",
        serde_json::json!({ "style": doc["email_style"], "field": field })
            .to_string()
            .replace('<', "\\u003c")
    );
    let content = format!(
        "{}{data}{strings}",
        crate::admin_customize_view::customize_page(&crate::admin_code_view::email_style(
            &cx, doc, field
        ))
    );
    let main =
        crate::admin_view::contents("admin-customize admin-customize-email-style edit", &content);
    shell.render(state, uri, "", main, "admin-customize.js", StatusCode::OK)
}

/// The form templates' pages (adminCustomizeFormTemplates): the list
/// (`form` None), or the form for a new template (Some(None)) or the
/// template given.
pub(super) async fn form_templates_page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    list: &[serde_json::Value],
    form: Option<Option<&serde_json::Value>>,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(
        &mut conn,
        state,
        guardian,
        headers,
        "admin_customize_form_templates",
    )
    .await?;
    // Each template's active categories: the site's categories that
    // use it.
    let mut templates = Vec::new();
    for t in list {
        let ids: Vec<i32> = sqlx::query_scalar(
            "SELECT category_id FROM category_form_templates WHERE form_template_id = $1 ORDER BY category_id",
        )
        .bind(t["id"].as_i64().unwrap_or_default())
        .fetch_all(&mut *conn)
        .await?;
        let mut t = t.clone();
        t["category_ids"] = serde_json::json!(ids);
        templates.push(t);
    }
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    drop(conn);
    let list_cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &shell.site.base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: None,
        settings: crate::topic_list_view::ListSettings::load(&shell.settings)?,
    };
    let cx = shell.cx(state);
    let (class, inner) = match form {
        None => (
            "admin-customize admin-customize-form-templates",
            crate::admin_code_view::form_templates(&cx, &templates, &list_cx),
        ),
        Some(template) => (
            if template.is_some() {
                "admin-customize admin-customize-form-templates edit"
            } else {
                "admin-customize admin-customize-form-templates new"
            },
            crate::admin_code_view::form_template_page(&cx, template),
        ),
    };
    let mut keys: Vec<String> = [
        "admin.form_templates.delete_confirm",
        "admin.form_templates.view_template.close",
        "admin.form_templates.view_template.edit",
        "admin.form_templates.view_template.delete",
        "admin.form_templates.view_template.toggle_preview",
        "admin.form_templates.validations_modal.modal_title",
        "admin.form_templates.field_placeholders.id",
        "admin.form_templates.field_placeholders.label",
        "admin.form_templates.field_placeholders.placeholder",
        "admin.form_templates.field_placeholders.validations",
        "admin.form_templates.field_placeholders.none_label",
        "admin.form_templates.field_placeholders.choices.first",
        "admin.form_templates.field_placeholders.choices.second",
        "admin.form_templates.field_placeholders.choices.third",
        "generic_error",
    ]
    .iter()
    .map(|k| k.to_string())
    .collect();
    for header in ["key", "type", "description"] {
        keys.push(format!(
            "admin.form_templates.validations_modal.table_headers.{header}"
        ));
    }
    for v in ["required", "minimum", "maximum", "pattern", "type"] {
        for part in ["key", "type", "description"] {
            keys.push(format!(
                "admin.form_templates.validations_modal.validations.{v}.{part}"
            ));
        }
    }
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    let strings = crate::admin_ui::strings(&cx, "admin-customize-strings", &keys);
    let data = format!(
        "<script type=\"application/json\" id=\"form-templates-data\">{}</script>",
        serde_json::json!({ "templates": templates, "template": form.flatten() })
            .to_string()
            .replace('<', "\\u003c")
    );
    let content = format!(
        "{}{data}{strings}",
        crate::admin_customize_view::customize_page(&inner)
    );
    let main = crate::admin_view::contents(class, &content);
    shell.render(state, uri, "", main, "admin-customize.js", StatusCode::OK)
}

/// GET /admin/customize/robots as the page (adminCustomizeRobotsTxt) from
/// the controller's document. The route has no title token.
pub(super) async fn robots_page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    doc: &serde_json::Value,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, state, guardian, headers, "admin_robots").await?;
    let cx = shell.cx(state);
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-customize-strings",
        &[
            "generic_error",
            "save",
            "saving",
            "saved",
            "admin.customize.robots.overridden",
        ],
    );
    let content = format!(
        "{}{strings}",
        crate::admin_customize_view::customize_page(&crate::admin_customize_view::robots(&cx, doc))
    );
    let main = crate::admin_view::contents("admin-customize admin-customize-robots-txt", &content);
    shell.render(state, uri, "", main, "admin-customize.js", StatusCode::OK)
}

/// What a JSON route's document answers the client's request
/// (`path_and_query`): Ok with the document, or Err with the response as
/// it is (a 404, an invalid parameter) for the page to answer with.
async fn client_json<F, Fut>(
    path_and_query: &str,
    call: F,
) -> Result<Result<serde_json::Value, Response>, AppError>
where
    F: FnOnce(Uri) -> Fut,
    Fut: std::future::Future<Output = Result<Response, AppError>>,
{
    let uri: Uri = path_and_query
        .parse()
        .map_err(|_| std::io::Error::other("a client request's path"))?;
    let response = call(uri).await?;
    if response.status() != StatusCode::OK {
        return Ok(Err(response));
    }
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(Ok(serde_json::from_slice(&bytes)?))
}

/// The locales' (value, name) as the available_locales setting gives
/// them.
fn available_locales(i18n: &crate::i18n::I18n) -> Vec<(String, String)> {
    crate::setting_enums::recorded("default_locale")
        .and_then(|r| r.get("valid_values"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            // The site settings service's display name: the translated
            // name with the native one, or the native one alone.
            let native = v.get("native_name")?.as_str()?;
            let name = v.get("name")?.as_str()?;
            let display = match crate::admin_view::t_opt(i18n, name) {
                Some(local) if local != native => format!("{local} ({native})"),
                _ => native.to_string(),
            };
            Some((v.get("value")?.as_str()?.to_string(), display))
        })
        .collect()
}

/// GET /admin/customize/site_texts as the page (adminSiteText.index): the
/// route's query params resolved as the controller does, the results the
/// store's find gets for them. `page` (not a route param) draws a later
/// page, which admin-customize.js asks for as the list scrolls.
pub(super) async fn site_texts_page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<Response, AppError> {
    let query = crate::params::parse_query(uri.query().unwrap_or(""));
    let param = |k: &str| crate::params::string(&query, k).filter(|v| !v.is_empty());
    let flag = |k: &str| param(k).is_some_and(|v| v == "true");
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(
        &mut conn,
        state,
        guardian,
        headers,
        "admin_customize_site_texts",
    )
    .await?;
    drop(conn);
    let filters = crate::admin_site_texts_view::Filters {
        q: param("q").unwrap_or_default(),
        locale: param("locale").unwrap_or(shell.settings.get("default_locale")?.to_s()),
        theme_id: param("theme_id").and_then(|v| v.parse().ok()),
        overridden: flag("overridden"),
        outdated: flag("outdated"),
        untranslated: flag("untranslated"),
        only_selected_locale: flag("onlySelectedLocale"),
    };
    let page = param("page").unwrap_or_else(|| "0".into());
    let path = {
        let mut json_query = url::form_urlencoded::Serializer::new(String::new());
        if !filters.q.is_empty() {
            json_query.append_pair("q", &filters.q);
        }
        if let Some(id) = filters.theme_id {
            json_query.append_pair("theme_id", &id.to_string());
        }
        json_query
            .append_pair("overridden", &filters.overridden.to_string())
            .append_pair("outdated", &filters.outdated.to_string())
            .append_pair("locale", &filters.locale)
            .append_pair("untranslated", &filters.untranslated.to_string())
            .append_pair(
                "only_selected_locale",
                &filters.only_selected_locale.to_string(),
            )
            .append_pair("page", &page);
        format!("/admin/customize/site_texts.json?{}", json_query.finish())
    };
    let doc = match client_json(&path, |u| super::site_texts::index_json(state.clone(), u)).await? {
        Ok(doc) => doc,
        Err(response) => return Ok(response),
    };
    let cx = shell.cx(state);
    if page != "0" {
        // The next page's results, for the script to append.
        return Ok(axum::response::Html(crate::admin_site_texts_view::results(
            &cx, &doc, &filters,
        ))
        .into_response());
    }
    let locales = available_locales(&state.i18n);
    let show_untranslated = shell
        .settings
        .get("admin_allow_filter_untranslated_text")?
        .truthy()
        && filters.locale != "en";
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-customize-strings",
        &[
            "admin.reseed.modal.title",
            "admin.reseed.modal.subtitle",
            "admin.reseed.modal.categories",
            "admin.reseed.modal.topics",
            "admin.reseed.modal.replace",
            "cancel",
            "admin.site_text.filters",
            "admin.site_text.filters_active.one",
            "admin.site_text.filters_active.other",
            "select_kit.filter_placeholder",
            "generic_error",
        ],
    );
    let data = format!(
        "<script type=\"application/json\" id=\"site-text-locales\">{}</script>",
        serde_json::Value::Array(
            locales
                .iter()
                .map(|(v, n)| serde_json::json!({ "value": v, "name": n }))
                .collect()
        )
        .to_string()
        .replace('<', "\\u003c")
    );
    let content = format!(
        "{}{data}{strings}",
        crate::admin_customize_view::customize_page(&crate::admin_site_texts_view::index(
            &cx,
            &doc,
            &filters,
            &locales,
            show_untranslated
        ))
    );
    let main = crate::admin_view::contents("admin-customize admin-site-text", &content);
    let title = crate::admin_view::t(cx.i18n, "admin.config.site_texts.title");
    shell.render(
        state,
        uri,
        &title,
        main,
        "admin-customize.js",
        StatusCode::OK,
    )
}

/// GET /admin/customize/site_texts/:id as the page (adminSiteText.edit):
/// the site text the route's ajax gets for the `locale` param.
pub(super) async fn site_text_page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    id: &str,
) -> Result<Response, AppError> {
    let query = crate::params::parse_query(uri.query().unwrap_or(""));
    let locale = crate::params::string(&query, "locale").unwrap_or_default();
    let theme_id = crate::params::string(&query, "theme_id").filter(|v| !v.is_empty());
    let path = format!(
        "/admin/customize/site_texts/{}.json?locale={}",
        url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>(),
        url::form_urlencoded::byte_serialize(locale.as_bytes()).collect::<String>()
    );
    let doc = match client_json(&path, |u| {
        super::site_texts::show_json(state.clone(), id.to_string(), u)
    })
    .await?
    {
        Ok(doc) => doc,
        Err(response) => return Ok(response),
    };
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(
        &mut conn,
        state,
        guardian,
        headers,
        "admin_customize_site_texts",
    )
    .await?;
    drop(conn);
    let locale_name = available_locales(&state.i18n)
        .into_iter()
        .find(|(v, _)| *v == locale)
        .map(|(_, n)| n)
        .unwrap_or_default();
    let cx = shell.cx(state);
    let text = &doc["site_text"];
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-customize-strings",
        &[
            "admin.site_text.revert_confirm",
            "admin.site_text.interpolation_key_invalid",
            "admin.site_text.interpolation_key_used",
            "admin.site_text.interpolation_key_insert",
            "save",
            "saving",
            "saved",
            "generic_error",
        ],
    );
    let data = format!(
        "<script type=\"application/json\" id=\"site-text-data\">{}</script>",
        serde_json::json!({ "site_text": text, "locale": locale })
            .to_string()
            .replace('<', "\\u003c")
    );
    let content = format!(
        "{}{data}{strings}",
        crate::admin_customize_view::customize_page(&crate::admin_site_texts_view::edit(
            &cx,
            text,
            &locale,
            &locale_name,
            theme_id.as_deref()
        ))
    );
    let main = crate::admin_view::contents("admin-customize admin-site-text edit", &content);
    let title = crate::admin_view::t(cx.i18n, "admin.config.site_texts.title");
    shell.render(
        state,
        uri,
        &title,
        main,
        "admin-customize.js",
        StatusCode::OK,
    )
}

/// GET /admin/email/templates/:id as the page (adminEmailTemplates.edit):
/// the email pages' header over the template's editor, the template the
/// parent route's list holds (none: the outlet left empty).
pub(super) async fn email_template_page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    id: &str,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, state, guardian, headers, "admin_server_setup").await?;
    let doc = super::admin_email_templates::document(
        &mut conn,
        state,
        &shell.settings,
        guardian.user_id().unwrap_or_default(),
    )
    .await?;
    let template = doc["email_templates"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["id"].as_str() == Some(id));
    let cx = shell.cx(state);
    let editor = template
        .map(|t| crate::admin_site_texts_view::email_template(&cx, t))
        .unwrap_or_default();
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-customize-strings",
        &[
            "admin.customize.email_templates.revert",
            "admin.customize.email_templates.revert_confirm",
            "admin.site_text.interpolation_key_invalid",
            "admin.site_text.interpolation_key_used",
            "admin.site_text.interpolation_key_insert",
            "composer.heading_level_n",
            "composer.heading_level_paragraph",
            "composer.ulist_title",
            "composer.olist_title",
            "composer.bold_text",
            "composer.italic_text",
            "composer.blockquote_text",
            "composer.code_text",
            "composer.list_item",
            "composer.emoji",
            "save",
            "saving",
            "saved",
            "generic_error",
        ],
    );
    let data = format!(
        "<script type=\"application/json\" id=\"email-template-data\">{}</script>",
        serde_json::json!({
            "template": template,
            "preview_wasm": format!(
                "{}/assets/markdown.wasm?v={}",
                shell.site.base_path,
                super::composer::MARKDOWN_WASM_VERSION
            ),
            "preview_settings": format!("{}/assets/markdown-settings.json", shell.site.base_path),
        })
        .to_string()
        .replace('<', "\\u003c")
    );
    let content = format!(
        "{}{data}{strings}",
        crate::admin_email_view::render(&cx, crate::admin_email_view::Tab::Templates, &editor)
    );
    let main = crate::admin_view::contents("admin-email admin-email-templates edit", &content);
    let title = crate::admin_view::t(cx.i18n, "admin.config.email.title");
    shell.render(
        state,
        uri,
        &title,
        main,
        "admin-customize.js",
        StatusCode::OK,
    )
}

/// GET /admin/customize/embedding and its tabs and forms as the page
/// (adminEmbedding): `sub` is what follows the path (empty for the hosts,
/// settings, posts_and_topics, crawlers, new, or a host's id).
pub(super) async fn embedding_page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    sub: &str,
) -> Result<Response, AppError> {
    use crate::admin_embedding_view::{self as view, Tab};
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, state, guardian, headers, "admin_embedding").await?;
    let doc = super::admin_embedding::document(state, &mut conn, &shell.settings, &[]).await?;
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    let list = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &shell.site.base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: None,
        settings: crate::topic_list_view::ListSettings::load(&shell.settings)?,
    };
    let cx = shell.cx(state);
    let t = |k: &str| crate::admin_view::t(cx.i18n, k);
    let embedding = &doc["embedding"];
    let (tab, class, title, outlet, script) = match sub {
        "" => (
            Some(Tab::Hosts),
            "admin-customize admin-embedding",
            String::new(),
            view::hosts(
                &cx,
                &list,
                &doc,
                shell.settings.get("embed_full_app")?.truthy(),
                shell.settings.get("uncategorized_category_id")?.to_i(),
            ),
            "admin-customize.js",
        ),
        "settings" => (
            Some(Tab::Settings),
            "admin-customize admin-embedding settings",
            String::new(),
            super::admin_advanced::area_settings(
                &mut conn,
                state,
                &shell,
                uri,
                serde_json::json!({ "filter_area": "embedding" }),
            )
            .await?,
            "site-settings.js",
        ),
        "posts_and_topics" => (
            Some(Tab::PostsAndTopics),
            "admin-customize admin-embedding posts-and-topics",
            String::new(),
            view::posts_and_topics(&cx, embedding),
            "admin-customize.js",
        ),
        "crawlers" => (
            Some(Tab::Crawlers),
            "admin-customize admin-embedding crawlers",
            String::new(),
            view::crawlers(&cx, embedding),
            "admin-customize.js",
        ),
        "new" => (
            None,
            "admin-customize admin-embedding new",
            t("admin.embedding.host_form.add_header"),
            view::host_form(&cx, &list, None),
            "admin-customize.js",
        ),
        id => {
            // The edit route's model: the host by parseInt(id), if any.
            let wanted = id.trim_end_matches(".json").parse::<i64>().ok();
            let host = doc["embeddable_hosts"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|h| wanted.is_some() && h["id"].as_i64() == wanted);
            (
                None,
                "admin-customize admin-embedding edit",
                t("admin.embedding.host_form.edit_header"),
                view::host_form(&cx, &list, host),
                "admin-customize.js",
            )
        }
    };
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-customize-strings",
        &[
            "admin.embedding.confirm_delete",
            "admin.embedding.crawler_settings_saved",
            "admin.embedding.posts_and_topics_settings_saved",
            "admin.embedding.sample",
            "form_kit.errors_summary_title.one",
            "form_kit.errors_summary_title.other",
            "form_kit.errors.required",
            "select_kit.filter_placeholder",
            "select_kit.no_content",
            "select_kit.delete_item",
            "select_kit.create",
            "generic_error",
        ],
    );
    let data = format!(
        "<script type=\"application/json\" id=\"embedding-data\">{}</script>{}",
        serde_json::json!({
            "base_url": embedding["base_url"],
            "full_app": shell.settings.get("embed_full_app")?.truthy(),
        })
        .to_string()
        .replace('<', "\\u003c"),
        crate::admin_customize_kit::categories_json(&list)
    );
    let content = format!(
        "{}{data}{strings}",
        crate::admin_customize_view::customize_page(&view::page(&cx, tab, &outlet))
    );
    let main = crate::admin_view::contents(class, &content);
    shell.render(state, uri, &title, main, script, StatusCode::OK)
}

/// GET /admin/customize/watched_words and .../action/:id as the page
/// (adminWatchedWords). The index goes on to the first action, as its
/// route's replaceWith does; `filter` is the route's query param, and
/// `show_words` stands for the controller's checkbox when admin-customize.js
/// asks for the list again.
pub(super) async fn watched_words_page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    action: Option<&str>,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, state, guardian, headers, "admin_watched_words").await?;
    let tagging = shell.settings.get("tagging_enabled")?.truthy();
    let words = crate::watched_words::all(&mut conn, tagging).await?;
    let list = crate::watched_words::list_json(&mut conn, &shell.settings, &words).await?;
    let query = crate::params::parse_query(uri.query().unwrap_or(""));
    let Some(action) = action else {
        let first = list["actions"][0].as_str().unwrap_or("block");
        let mut to = format!(
            "{}/admin/customize/watched_words/action/{first}",
            shell.site.base_path
        );
        if let Some(q) = uri.query() {
            to.push('?');
            to.push_str(q);
        }
        return Ok(axum::response::Redirect::to(&to).into_response());
    };
    let filter = crate::params::string(&query, "filter").unwrap_or_default();
    let show_words = crate::params::string(&query, "show_words").is_some_and(|v| v == "1");
    let regex = shell
        .settings
        .get("watched_words_regular_expressions")?
        .truthy();
    let cx = shell.cx(state);
    let page = crate::admin_customize_view::WatchedWords {
        list: &list,
        action,
        filter: &filter,
        show_words,
        regex,
    };
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-customize-strings",
        &[
            "admin.watched_words.clear_all_confirm",
            "admin.watched_words.form.success",
            "admin.watched_words.form.exists",
            "admin.watched_words.form.upload_successful",
            "admin.watched_words.test.modal_title",
            "admin.watched_words.test.description",
            "admin.watched_words.test.found_matches",
            "admin.watched_words.test.no_matches",
            "admin.watched_words.actions.block",
            "admin.watched_words.actions.censor",
            "admin.watched_words.actions.require_approval",
            "admin.watched_words.actions.flag",
            "admin.watched_words.actions.replace",
            "admin.watched_words.actions.tag",
            "admin.watched_words.actions.silence",
            "admin.watched_words.actions.link",
            "admin.watched_words.invalid_regex_multiple",
            "select_kit.create",
            "select_kit.delete_item",
            "generic_error",
            "generic_error_with_reason",
        ],
    );
    let data = format!(
        "<script type=\"application/json\" id=\"watched-words-data\">{}</script>",
        list.to_string().replace('<', "\\u003c")
    );
    let content = format!(
        "{}{data}{strings}",
        crate::admin_customize_view::customize_page(&crate::admin_customize_view::watched_words(
            &cx, &page
        ))
    );
    let attrs = format!(
        " data-action=\"{}\"",
        crate::topic_list_view::escape(action)
    );
    let main = crate::admin_ui::contents(
        "admin-customize admin-watched-words action",
        &attrs,
        &content,
    );
    let title = crate::admin_view::t(cx.i18n, "admin.config.watched_words.title");
    shell.render(
        state,
        uri,
        &title,
        main,
        "admin-customize.js",
        StatusCode::OK,
    )
}
