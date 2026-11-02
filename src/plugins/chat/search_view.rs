//! Chat search, server-rendered (ChatSearch): the filter input and the
//! sort menu the drawer's search route draws (view.rs
//! drawer_search_route), and the results GET /live/chat/search draws for
//! static/js/chat-search.js as the member types (searchMessages over
//! Chat::SearchMessage, LIMIT at a time).

use serde_json::Value;

use super::channels::Context;
use super::page::{View, js, message_html, s};
use crate::post_view::d_icon;
use crate::topic_list_view::escape;
use crate::{AppError, Unsupported};

/// ChatSearch's LIMIT.
pub const LIMIT: i64 = 20;

/// ChatSearch with an empty query: the filter (DFilterInput, its
/// magnifying glass on the left) and the sort menu (DMenu on
/// relevance, its items in a template chat-search.js floats under it),
/// what chat-search.js draws the results into, and the result list's
/// and empty alert's templates.
pub fn chat_search(i18n: &crate::i18n::I18n, base_path: &str) -> String {
    chat_search_with(i18n, base_path, None)
}

/// ChatSearch, `full_page` the full page's query (its `q` param) when
/// it keeps the query in the address (enableQueryParams), as the filter's
/// value.
pub fn chat_search_with(
    i18n: &crate::i18n::I18n,
    base_path: &str,
    full_page: Option<&str>,
) -> String {
    let t = |key: &str| js(i18n, key);
    let sort = |value: &str| {
        format!(
            "<li class=\"dropdown-menu__item\"><button class=\"btn btn-text btn-transparent\" type=\"button\" data-sort=\"{value}\"><span class=\"d-button-label\">{}</span></button></li>",
            escape(&t(&format!("chat.search.sort.{value}")))
        )
    };
    format!(
        "<div class=\"c-search\" data-url=\"{}/live/chat/search\" data-limit=\"{LIMIT}\"{}><div class=\"chat-search__filters\"><div class=\"filter-input-container\">{}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input no-blur\" placeholder=\"{}\" type=\"text\" value=\"{}\"></div><button aria-expanded=\"false\" class=\"btn btn-icon-text fk-d-menu__trigger search-sort-options-trigger btn-default\" data-identifier=\"search-sort-options\" data-trigger=\"\" type=\"button\" data-sort=\"relevance\">{}<span class=\"d-button-label\">{}</span></button><template class=\"search-sort-options-template\"><ul class=\"dropdown-menu\">{}{}</ul></template></div><template class=\"chat-search__results-template\"><div class=\"sr-only\" id=\"chat-search-instructions\">{}</div><ul aria-label=\"{}\" class=\"chat-message-search-entries\" role=\"listbox\"></ul><div class=\"chat-search-loading\"><div class=\"loading-container\"></div></div><br></template><template class=\"chat-search__empty-template\"><div class=\"alert alert-info\">{}</div></template></div>",
        escape(base_path),
        if full_page.is_some() {
            " data-query-params=\"true\""
        } else {
            ""
        },
        d_icon("magnifying-glass", Some("-left")),
        escape(&t("chat.search_view.filter_placeholder")),
        escape(full_page.unwrap_or_default()),
        d_icon("sort", None),
        escape(&t("chat.search.sort.relevance")),
        sort("relevance"),
        sort("latest"),
        escape(&t("chat.search_view.sr_instructions")),
        escape(&t("chat.search_view.results_list_label")),
        escape(&t("chat.search.no_results")),
    )
}

/// What a page of results is: the entries and has_more.
pub struct Results {
    pub html: String,
    pub has_more: bool,
}

impl Context<'_> {
    /// A page of results as ChatSearch lists them, from GET
    /// /chat/api/search's answer: each message an entry
    /// (chat-message-search-entry) with its channel's title (unless the
    /// search is scoped to a channel), its thread's heading, and the
    /// message (ChatMessage with dateMode long, no separator, not
    /// interactive).
    pub async fn search_results(
        &mut self,
        found: &Value,
        scoped: bool,
    ) -> Result<Results, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let viewer_id = i64::from(self.guardian.user_id().unwrap_or(0));
        let online = self.online_ids().await?;
        let emojis = self.settings.emojis().clone();
        let max_image = (
            self.settings.get("max_image_width")?.to_f(),
            self.settings.get("max_image_height")?.to_f(),
        );
        let base_url = crate::url::Urls {
            config: self.config,
            settings: self.settings,
        }
        .base_url()?;
        let prioritize_name = super::page::prioritize_name(self.settings)?;
        let duration = self.settings.get("relative_date_duration")?.to_i();
        let mut out = String::new();
        let messages = found["messages"].as_array().cloned().unwrap_or_default();
        for m in &messages {
            let channel = &m["channel"];
            if !channel.is_object() {
                return Err(Unsupported("a chat search result without its channel").into());
            }
            let view = View {
                base_path: self.base_path,
                channel,
                viewer_id,
                online: online.clone(),
                emojis: emojis.clone(),
                max_image,
                base_url: base_url.clone(),
                // @interactive={{false}}
                can_interact: false,
                prioritize_name,
                admin: self.guardian.is_admin(),
                t: &t,
                i18n: self.i18n,
                relative_date_duration: duration,
                thread_context: false,
            };
            let html = message_html(&view, std::slice::from_ref(m), 0, None, None, false)?;
            let html = long_date(&super::page::not_interactive(&html, ""), m, &view);
            // ChatMessage#url
            let url = match m["thread_id"].as_i64() {
                Some(thread) => format!(
                    "{}/chat/c/-/{}/t/{thread}/{}",
                    self.base_path, channel["id"], m["id"]
                ),
                None => format!("{}/chat/c/-/{}/{}", self.base_path, channel["id"], m["id"]),
            };
            let thread_title = m["thread_title"].as_str().filter(|t| !t.is_empty());
            out.push_str(&format!(
                "<li aria-describedby=\"chat-search-instructions\" aria-label=\"{}\" class=\"chat-message-search-entry\" role=\"option\" tabindex=\"0\" data-url=\"{}\"><div class=\"chat-message-search-entry__info\">",
                escape(&accessible_label(&t, m, thread_title)),
                escape(&url)
            ));
            if !scoped {
                out.push_str(&super::page::channel_title(
                    channel,
                    &emojis,
                    self.base_path,
                    &online,
                ));
            }
            // ChatThreadHeading
            if let Some(title) = thread_title {
                out.push_str(&format!(
                    "<div class=\"chat-thread__heading\"><div class=\"chat-thread__heading-icon\">{}</div><h2 class=\"chat-thread__heading-title\">{}</h2></div>",
                    d_icon("discourse-threads", None),
                    crate::topic_list_view::emoji_unescape(&escape(title), &emojis, self.base_path)
                ));
            }
            out.push_str("</div>");
            out.push_str(&html);
            out.push_str("</li>");
        }
        Ok(Results {
            html: out,
            has_more: found["meta"]["has_more"] == true,
        })
    }
}

/// accessibleMessageLabel: the author, the excerpt (else the first 100
/// characters of the message), and the thread's title.
fn accessible_label(t: &dyn Fn(&str) -> String, m: &Value, thread_title: Option<&str>) -> String {
    let username = match m["user"]["username"].as_str() {
        Some(u) => u.to_string(),
        None => t("chat.deleted_user"),
    };
    let preview = match m["excerpt"].as_str().filter(|e| !e.is_empty()) {
        Some(excerpt) => excerpt.to_string(),
        None => match m["message"].as_str().filter(|e| !e.is_empty()) {
            Some(message) => format!("{}...", message.chars().take(100).collect::<String>()),
            None => String::new(),
        },
    };
    let thread = thread_title
        .map(|title| t("chat.search_view.in_thread").replace("%{title}", title))
        .unwrap_or_default();
    t("chat.search_view.message_result_label")
        .replace("%{username}", &username)
        .replace("%{preview}", &preview)
        .replace("%{threadInfo}", &thread)
        .trim()
        .to_string()
}

/// formatChatDate in the long mode (dates.long_no_year) in place of the
/// message's time, for dates.js to draw in the viewer's zone.
fn long_date(html: &str, m: &Value, v: &View) -> String {
    static TIME: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"<a title="[^"]*" tabindex="-1" class="chat-time" href="[^"]*" data-mode="[a-z]+"[^>]*>[^<]*</a>"#)
            .expect("static pattern")
    });
    let Ok(at) = chrono::DateTime::parse_from_rfc3339(s(&m["created_at"])) else {
        return html.to_string();
    };
    let at = at.with_timezone(&chrono::Utc);
    let format = |key: &str| {
        crate::pretty_text::render::local_dates::format_utc(at, &(v.t)(key)).unwrap_or_default()
    };
    let anchor = format!(
        "<a title=\"{}\" tabindex=\"-1\" class=\"chat-time\" href=\"{}/chat/c/-/{}/{}\"{}{}>{}</a>",
        escape(&format("dates.long_with_year")),
        v.base_path,
        v.channel["id"],
        m["id"],
        crate::viewer_dates::tz_attr(
            "title",
            &crate::viewer_dates::marker(at, &(v.t)("dates.long_with_year"))
        ),
        crate::viewer_dates::tz_attr(
            "text",
            &crate::viewer_dates::marker(at, &(v.t)("dates.long_no_year"))
        ),
        escape(&format("dates.long_no_year"))
    );
    TIME.replace(html, regex::NoExpand(&anchor)).into_owned()
}
