pub mod about;
pub mod about_view;
pub mod accounts;
pub mod admin_about_view;
pub mod admin_api_keys_view;
pub mod admin_backups_view;
pub mod admin_badges;
pub mod admin_badges_view;
pub mod admin_branding_view;
pub mod admin_category_management_view;
pub mod admin_code_view;
pub mod admin_colors_view;
pub mod admin_config_view;
pub mod admin_confirmation;
pub mod admin_create;
pub mod admin_customize_kit;
pub mod admin_customize_view;
pub mod admin_dashboard;
pub mod admin_dashboard_headlines;
pub mod admin_dashboard_traffic;
pub mod admin_dashboard_view;
pub mod admin_email_view;
pub mod admin_embedding_view;
pub mod admin_emoji_view;
pub mod admin_flags;
pub mod admin_form_kit;
pub mod admin_groups_view;
pub mod admin_logs_view;
pub mod admin_mcp_view;
pub mod admin_plugins;
pub mod admin_plugins_view;
pub mod admin_problem_checks_view;
pub mod admin_report_view;
pub mod admin_reports_view;
pub mod admin_search;
pub mod admin_search_view;
pub mod admin_setting_links;
pub mod admin_site_settings;
pub mod admin_site_settings_view;
pub mod admin_site_texts_view;
pub mod admin_theme_view;
pub mod admin_themes_view;
pub mod admin_traffic_explorer;
pub mod admin_ui;
pub mod admin_upcoming_changes_view;
pub mod admin_user_show;
pub mod admin_user_view;
pub mod admin_users;
pub mod admin_users_view;
pub mod admin_view;
pub mod admin_web_hooks_view;
pub mod admin_whats_new_view;
pub mod age_words;
pub mod anonymous_cache;
pub mod archived_messages;
pub mod assets;
pub mod avatar;
pub mod backup_restore;
pub mod badge_granter;
pub mod badge_preview;
pub mod badges_page;
pub mod badges_view;
pub mod bookmark_manager;
pub mod bookmarks;
pub mod browser_pageviews;
pub mod bulk_invite;
pub mod bus;
pub mod categories;
pub mod categories_view;
pub mod category;
pub mod category_badge;
pub mod category_edit_view;
pub mod category_list;
pub mod category_serializer;
pub mod category_type_config;
pub mod category_types;
pub mod category_writes;
pub mod clock;
pub mod color_mode;
pub mod color_scheme;
pub mod composer_view;
pub mod config;
pub mod cose;
pub mod current_user;
pub mod custom_emoji;
pub mod custom_emoji_import;
pub mod design_wizard;
pub mod directory;
pub mod directory_view;
pub mod discourse_connect;
pub mod discourse_diff;
pub mod do_not_disturb;
pub mod drafts;
pub mod email;
pub mod email_logs;
pub mod email_updater;
pub mod embedding;
pub mod emojis;
pub use discourse_markdown::emoji;
pub mod api_key_scopes;
pub mod auth;
pub mod crass;
pub mod crawler_detection;
pub mod excerpt;
pub mod fast_image;
pub mod file_store;
pub mod final_destination;
pub mod flag_modal_view;
pub mod flags;
pub mod form_templates;
pub mod group_manager;
pub mod group_message;
pub mod group_writes;
pub mod groups;
pub mod groups_view;
pub mod guardian;
pub mod hashtag_decorator;
pub mod hashtags;
pub mod header_badge;
pub mod header_topic_view;
pub mod highlight_js;
pub mod homepage;
pub mod hotlinked_media;
pub mod html;
pub mod html_prettify;
pub mod i18n;
pub mod images;
pub mod impersonation;
pub mod incoming_link;
pub mod invite_mailer;
pub mod invite_page_view;
pub mod invites;
pub mod jobs;
pub mod letter_avatar;
pub mod likes;
pub mod login_page_view;
pub mod mcp;
pub mod mcp_clients;
pub mod message_format;
pub mod messages_view;
pub mod mobile;
pub mod modifications;
pub mod moment_locale;
pub mod multisite;
pub mod mustache;
pub mod new_features;
pub mod not_found_page;
pub mod notification_create;
pub mod notification_emailer;
pub mod notification_schedule;
pub mod notifications;
pub mod object_storage;
pub mod onebox;
pub mod optimized_images;
pub mod owned_schema;
pub mod params;
pub mod parity;
pub mod penalize_view;
pub mod permalinks;
pub mod plugins;
pub mod pm_invite;
pub mod pm_lists;
pub mod pm_tracking_state;
pub mod post_actions;
pub mod post_destroyer;
pub mod post_mover;
pub mod post_owner_changer;
pub mod post_view;
pub mod posting;
pub mod preferences_view;
pub mod presence;
pub mod pretty_text;
pub mod problem_checks;
pub mod promotion;
pub mod rate_limiter;
pub mod read_tracking;
pub mod readonly;
pub mod remote_ip;
pub mod reports;
pub mod request_tracker;
pub mod request_tracking;
pub mod retrieve_title;
pub mod review;
pub mod review_list;
pub mod review_posts;
pub mod review_view;
pub mod reviewable_claims;
pub mod reviewable_user;
pub mod reviewables;
pub mod roles;
pub mod routes;
pub mod ruby;
pub mod ruby_date;
pub mod sanitize;
pub mod sanitize_field;
pub mod schema;
pub mod schema_objects;
pub mod screened;
pub mod search;
pub mod search_advanced_view;
pub mod search_logs;
pub mod search_menu_view;
pub mod search_view;
pub mod second_factor;
pub mod second_factor_auth;
pub mod seed_topics;
pub mod session;
pub mod setting_enums;
pub mod setup;
pub mod sidebar;
pub mod sidebar_backfill;
pub mod signup;
pub mod site;
pub mod site_icons;
pub mod site_setting_lists;
pub mod site_setting_update;
pub mod site_settings;
pub mod site_state;
pub mod site_texts;
pub mod staff_action_logs;
pub mod static_page_view;
pub mod stylesheet;
pub mod svg_sprite;
pub mod svg_sprite_xml;
pub mod system_message;
pub mod system_themes;
pub mod tag_groups;
pub mod tag_writes;
pub mod tagging;
pub mod tags;
pub mod theme_cache;
pub mod theme_html;
pub mod theme_import;
pub mod theme_writes;
pub mod themes;
pub mod topic_converter;
pub mod topic_guardian;
pub mod topic_list;
pub mod topic_list_view;
pub mod topic_query;
pub mod topic_status;
pub mod topic_timer;
pub mod topic_tracking_report;
pub mod topic_tracking_state;
pub mod topic_view;
pub mod topics_bulk;
pub mod upcoming_changes;
pub mod upload_references;
pub mod uploads;
pub mod url;
pub mod user_activity_view;
pub mod user_anonymizer;
pub mod user_api_key_crypto;
pub mod user_avatar;
pub mod user_card_view;
pub mod user_comm_screener;
pub mod user_destroyer;
pub mod user_fields;
pub mod user_info_view;
pub mod user_menu_view;
pub mod user_merger;
pub mod user_name_suggester;
pub mod user_notifications_view;
pub mod user_option_defaults;
pub mod user_penalties;
pub mod user_private;
pub mod user_profile_view;
pub mod user_search;
pub mod user_updater;
pub mod username_changer;
pub mod users;
pub mod viewer_dates;
pub mod watched_words;
pub mod web_hooks;
pub mod webauthn;
pub mod wildcard_url;
pub mod wizard;
pub mod word_watcher;

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sqlx::PgPool;
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::i18n::I18n;
use crate::site_settings::Definitions;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub site_setting_defs: Arc<Definitions>,
    pub i18n: Arc<I18n>,
    /// `SearchLog.log`'s per-IP dedupe window.
    pub search_log_cache: Arc<search::SearchLogCache>,
    /// secret_key_base and the cookie codec derived from it.
    pub keys: Arc<session::current::Keys>,
    /// Where outgoing mail goes (SMTP, sendmail, or memory in tests).
    pub mailer: email::Mailer,
    /// Live updates to browsers (MessageBus's role).
    pub bus: pg_bus::Bus,
    /// Anonymous responses kept for the actions that allow it.
    pub anonymous_cache: Arc<anonymous_cache::Cache>,
    /// The request tracker's rate limit lists of this site (per user,
    /// per crawler), kept in the process.
    pub rate_limits: rate_limiter::MemoryStore,
    /// The tracker's `GLOBAL::` lists (per address), one store for every
    /// site of the process.
    pub global_rate_limits: rate_limiter::MemoryStore,
    /// What the site keeps in the process between requests and jobs.
    pub site: Arc<site_state::SiteState>,
}

pub fn app(state: AppState) -> Router {
    // The method override and the request format must run before routing
    // (a POST becomes a DELETE, an Accept-led JSON request its `.json`
    // route), and the session is resolved before anything else, so they
    // wrap the routed app from the outside. The mobile layer scopes the
    // request's layout (mobile::mobile_view) around everything it renders.
    let routed = routes::router(&state).with_state(state.clone());
    Router::new()
        .fallback_service(routed)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            translation_overrides,
        ))
        .layer(axum::middleware::from_fn(mobile::layer))
        .layer(axum::middleware::from_fn(color_mode::layer))
        .layer(axum::middleware::from_fn(routes::method_override))
        .layer(axum::middleware::from_fn(routes::request_format))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            session::current::layer,
        ))
        .layer(compression())
        // Outside compression: a hit runs nothing else and is sent as it
        // was stored, compressed for the encoding in its key.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            anonymous_cache::layer,
        ))
        // Middleware::RequestTracker, ahead of the app and the cache.
        .layer(axum::middleware::from_fn_with_state(
            state,
            request_tracker::layer,
        ))
        .layer(TraceLayer::new_for_http())
}

/// The site texts as of this request: the translation overrides read again
/// when the table changed (Rails reloads them on TranslationOverride's
/// writes).
async fn translation_overrides(
    axum::extract::State(state): axum::extract::State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let refreshed = match state.pool.acquire().await {
        Ok(mut conn) => state.i18n.refresh_overrides(&mut conn).await,
        Err(e) => Err(e),
    };
    // A database that cannot be reached is the handler's to answer (the
    // failover pages); the overrides read last stay meanwhile.
    if let Err(e) = refreshed {
        tracing::warn!(error = %e, "translation overrides not read");
    }
    next.run(request).await
}

/// Responses compressed for browsers that accept it (nginx's role in front
/// of Discourse): gzip or brotli at level 5, a fast setting for responses
/// compressed on every request. tower-http's default predicate leaves out
/// small bodies, images and the live stream (text/event-stream, which must
/// flush as it goes); fonts arrive compressed already.
fn compression() -> tower_http::compression::CompressionLayer<
    tower_http::compression::predicate::And<
        tower_http::compression::DefaultPredicate,
        tower_http::compression::predicate::NotForContentType,
    >,
> {
    use tower_http::compression::predicate::{NotForContentType, Predicate};
    tower_http::compression::CompressionLayer::new()
        .quality(tower_http::CompressionLevel::Precise(5))
        .compress_when(
            tower_http::compression::DefaultPredicate::new()
                .and(NotForContentType::const_new("font/")),
        )
}

pub use discourse_markdown::Unsupported;

/// Any failure a handler can't turn into a proper response: logged in full,
/// returned as a bare 500.
#[derive(Debug)]
pub struct AppError(Box<dyn std::error::Error + Send + Sync>);

impl<E> From<E> for AppError
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn from(e: E) -> Self {
        AppError(Box::new(e))
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl AppError {
    /// Whether the failure is something not ported (`Unsupported`), as
    /// opposed to an error.
    pub fn is_unsupported(&self) -> bool {
        self.0.downcast_ref::<Unsupported>().is_some()
            || self.0.to_string().starts_with("not ported yet:")
    }

    /// The SQLSTATE of a database error, if that is what this is.
    pub fn sql_state(&self) -> Option<String> {
        self.0
            .downcast_ref::<sqlx::Error>()
            .and_then(|e| e.as_database_error())
            .and_then(|e| e.code())
            .map(|c| c.into_owned())
    }

    /// The rate limit refusal raised, if that is what this is.
    pub fn rate_limited(&self) -> Option<&rate_limiter::LimitExceeded> {
        self.0.downcast_ref::<rate_limiter::LimitExceeded>()
    }

    /// The error raised, if it is a `T` (what Rails would rescue by
    /// class).
    pub fn downcast_ref<T: std::error::Error + 'static>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }
}

impl AppError {
    /// Discourse::ReadOnly raised, or Postgres refusing a write in a read
    /// only transaction (the `PG::ReadOnlySqlTransaction` rescue).
    fn read_only(&self) -> bool {
        self.0.downcast_ref::<readonly::ReadOnly>().is_some()
            || self
                .0
                .downcast_ref::<sqlx::Error>()
                .and_then(|e| e.as_database_error())
                .and_then(|e| e.code())
                .is_some_and(|code| code == "25006")
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // The read-only gate renders the 503 for the request's format.
        if self.read_only() {
            let mut response = StatusCode::SERVICE_UNAVAILABLE.into_response();
            response.extensions_mut().insert(readonly::Raised);
            return response;
        }
        tracing::error!(error = %self.0, "request failed");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}
