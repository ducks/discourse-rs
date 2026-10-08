//! `ApplicationController#build_not_found_page`: exceptions/not_found.html.erb
//! with the _not_found_topics partial, as topics#show sends it in a 404's
//! (or 403's) `extras.html`.
//!
//! The `onpopstate-handler` script carries no CSP nonce: the port sets no
//! content security policy. Not ported: custom message params, the group
//! membership buttons (detailed_404), plugin outlets, and
//! bootstrap_error_pages.

use sqlx::PgConnection;

use crate::category_badge::{codes_to_img, html_escape};
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::topic_query::{Filter, Options, TopicQuery, TopicRow};
use crate::url::Urls;
use crate::{AppError, Unsupported};

const ILLUSTRATION: &str = include_str!("../templates/exceptions/not_found_illustration.svg");

/// What the page says and for whom.
pub struct NotFound<'a> {
    /// status 403: page_forbidden.title instead of page_not_found.title.
    pub forbidden: bool,
    /// `opts[:custom_message]`: an i18n key that becomes the title.
    pub custom_message: Option<&'a str>,
    /// `params[:slug] || params[:id]`, as the route gave it.
    pub slug: &'a str,
    /// The current user, if any (`current_user rescue nil`).
    pub guardian: &'a Guardian,
}

/// The `/t/` paths topics#show serves: `/t/:id`, `/t/:slug/:id`,
/// `/t/:slug/:id/:post_number` (and `/t/:id/:post_number`), with an
/// optional `.json`. The slug or id the not-found page searches for comes
/// back.
pub fn topic_show_slug(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/t/")?;
    let rest = rest.strip_suffix(".json").unwrap_or(rest);
    let parts: Vec<&str> = rest.split('/').collect();
    let numeric = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match parts.as_slice() {
        [id] if !id.is_empty() => Some(id.to_string()),
        [slug, id] if numeric(id) => Some(slug.to_string()),
        [slug, id, n] if numeric(id) && numeric(n) => Some(slug.to_string()),
        _ => None,
    }
}

pub async fn build(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    urls: &Urls<'_>,
    page: &NotFound<'_>,
) -> Result<String, AppError> {
    let t = |key: &str| i18n.t(key).unwrap_or(key).to_string();
    let base_path = urls.config.globals.relative_url_root().to_string();
    let login_required = settings.get("login_required")?.truthy();
    let signed_in = page.guardian.is_authenticated();

    let topics = if !login_required || signed_in {
        topics_partial(conn, settings, i18n, &base_path).await?
    } else {
        String::new()
    };
    let title = match page.custom_message {
        Some(key) => t(key),
        None if page.forbidden => t("page_forbidden.title"),
        None => t("page_not_found.title"),
    };
    let hide_search = login_required || !page.guardian.can_search(settings)?;
    let slug = page.slug.replace('-', " ");

    let mut out = String::from(
        "\n\n\n\n<div class=\"page-not-found\">\n\n<div class=\"heading\">\n<div class=\"illustration-not-found\">\n",
    );
    out.push_str(ILLUSTRATION);
    out.push_str("</div>\n\n<div class=\"title_wrapper\">\n");
    out.push_str(&format!(
        "  <h1 class=\"title\">{}</h1>\n",
        html_escape(&title)
    ));
    out.push_str(&format!(
        "  <a class=\"btn btn-primary btn-large --home\" href=\"{}\"> {} {}\n  </a>\n  </div>\n</div>\n",
        urls.base_url()?,
        crate::svg_sprite::raw_svg("house")?,
        t("page_not_found.home")
    ));
    if !hide_search {
        out.push_str(&format!(
            "  <div class=\"row\">\n    <div class=\"page-not-found-search\">\n      <form action='{base_path}/search' id='discourse-search'>\n        <label for=\"search-input\">{}</label>\n        <div class=\"search-input__wrapper\">\n        <input type=\"text\" id=\"search-input\" class=\"search-input\" placeholder=\"{}\" name=\"q\" value=\"{}\">\n        <div class=\"search-input__icon\" aria-hidden=\"true\">{}</div>\n        </div>\n        <button class=\"btn btn-primary\">{}</button>\n      </form>\n    </div>\n  </div>\n\n",
            html_escape(&t("page_not_found.search_title")),
            html_escape(&t("page_not_found.placeholder")),
            html_escape(&slug),
            crate::svg_sprite::raw_svg("magnifying-glass")?,
            html_escape(&t("page_not_found.search_button")),
        ));
        // preload_script("js/onpopstate-handler")
        out.push_str(&format!(
            "  <script defer src=\"{base_path}/assets/js/onpopstate-handler.js\" data-discourse-entrypoint=\"js/onpopstate-handler\"></script>\n\n"
        ));
    }
    out.push_str("\n</div>\n\n\n\n");
    out.push_str(&topics);
    out.push('\n');
    Ok(out)
}

/// exceptions/_not_found_topics: the month's top topics and the most
/// recent ones, ten each, as anonymous sees them, category definitions
/// left out. (Rails caches it for ten minutes per locale.)
async fn topics_partial(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    base_path: &str,
) -> Result<String, AppError> {
    let custom_emoji: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM custom_emojis)")
        .fetch_one(&mut *conn)
        .await?;
    if custom_emoji {
        return Err(Unsupported("custom emojis in the not-found page's titles").into());
    }
    let definitions: Vec<i32> =
        sqlx::query_scalar("SELECT topic_id FROM categories WHERE topic_id IS NOT NULL")
            .fetch_all(&mut *conn)
            .await?;
    let anonymous = Guardian::anonymous();
    // TopicQuery.new(nil, except_topic_ids:).list_top_for("monthly").first(10)
    let top = TopicQuery {
        conn: &mut *conn,
        settings,
        guardian: &anonymous,
        options: Options {
            per_page: Some(100),
            ..Default::default()
        },
        filter: Default::default(),
        category: Default::default(),
        tags: Default::default(),
        user: Default::default(),
    }
    .list(Filter::Top("monthly".into()))
    .await?;
    let top: Vec<TopicRow> = top
        .topics
        .into_iter()
        .filter(|t| !definitions.contains(&t.id))
        .take(10)
        .collect();
    // Topic.recent(10): listable, visible, secured for anonymous.
    let recent: Vec<TopicRow> = sqlx::query_as(&format!(
        "SELECT {} FROM topics WHERE topics.deleted_at IS NULL AND topics.archetype <> 'private_message' \
           AND topics.visible AND NOT (topics.id = ANY($1)) \
           AND (topics.category_id IS NULL OR topics.category_id IN \
                (SELECT id FROM categories WHERE NOT read_restricted)) \
         ORDER BY topics.created_at DESC LIMIT 10",
        crate::topic_query::TOPIC_COLUMNS
    ))
    .bind(&definitions)
    .fetch_all(&mut *conn)
    .await?;

    let t = |key: &str| i18n.t(key).unwrap_or(key).to_string();
    let mut out = String::from("<div class=\"row page-not-found-topics\">\n");
    for (rows, class, title_key, more) in [
        (&top, "popular", "page_not_found.popular_topics", "/top"),
        (&recent, "recent", "page_not_found.recent_topics", "/latest"),
    ] {
        if rows.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "    <div class=\"{class}-topics\">\n      <h2 class=\"{class}-topics-title\">{}</h2>\n",
            t(title_key)
        ));
        for row in rows {
            let fancy = crate::topic_query::fancy_title(
                &mut *conn,
                settings,
                row.id,
                &row.title,
                row.fancy_title.as_deref(),
            )
            .await?;
            let badge = match row.category_id {
                Some(id) => {
                    crate::category_badge::html_for(&mut *conn, settings, base_path, id).await?
                }
                None => String::new(),
            };
            out.push_str(&format!(
                "        <div class='not-found-topic'>\n          <a href=\"{base_path}/t/{}/{}\">{}</a>{badge}\n        </div>\n",
                row.slug.as_deref().ok_or(Unsupported("topics without a stored slug (Slug.for)"))?,
                row.id,
                codes_to_img(settings, base_path, &fancy)?,
            ));
        }
        out.push_str(&format!(
            "      <a href=\"{base_path}{more}\" class=\"btn btn-default\">{}&hellip;</a>\n    </div>\n",
            t("page_not_found.see_more")
        ));
    }
    out.push_str("</div>\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::topic_show_slug;

    #[test]
    fn topic_show_paths() {
        assert_eq!(
            topic_show_slug("/t/nope-nope.json").as_deref(),
            Some("nope-nope")
        );
        assert_eq!(
            topic_show_slug("/t/made-up/123.json").as_deref(),
            Some("made-up")
        );
        assert_eq!(topic_show_slug("/t/x/123/4").as_deref(), Some("x"));
        assert_eq!(topic_show_slug("/t/123/summary"), None);
        assert_eq!(topic_show_slug("/t/123/status.json"), None);
        assert_eq!(topic_show_slug("/latest.json"), None);
    }
}
