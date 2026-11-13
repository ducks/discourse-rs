//! Every integration test file under tests/ is a module of this one test
//! binary, so the library links once for all of them rather than once per
//! file. writes.rs keeps its own binary for its harness. nextest still runs
//! each test in its own process. Run one file's tests with
//! `cargo nextest run -E 'test(/^categories::/)'`.

mod common;

mod about_pages;
mod admin_advanced_pages;
mod admin_config_pages;
mod admin_customize_pages;
mod admin_pages;
mod anonymous_cache;
mod assign;
mod backup_restore;
mod backups;
mod categories;
mod category_pages;
mod chat_admin;
mod chat_archive;
mod chat_channel_create;
mod chat_channel_settings;
mod chat_client;
mod chat_cook;
mod chat_drawer_routes;
mod chat_message_actions;
mod chat_moderation;
mod chat_notify;
mod chat_pages_rest;
mod chat_pane;
mod chat_pane_rest;
mod chat_pins;
mod chat_processor;
mod chat_rest;
mod chat_reviewables;
mod chat_search;
mod chat_site;
mod chat_threads;
mod chat_threads_rest;
mod chat_uploads;
mod color_modes;
mod composer_actions;
mod composer_mentions;
mod composer_messages;
mod crawler;
mod demo_smoke;
mod direct_uploads;
mod discourse_diff;
mod external_auth;
mod fancy_title;
mod finish_installation;
mod groups_pages;
mod html_pages;
mod html_to_markdown;
mod i18n_locales;
mod incoming_mail;
mod invite_pages;
mod jobs;
mod latest_json;
mod live_alert;
mod live_page;
mod live_pm_tracking;
mod live_topic;
mod live_tracking;
mod login_pages;
mod login_required;
mod messages_pages;
mod moderator_pages;
mod multisite;
mod narrative_bot;
mod new_member_smoke;
mod new_topic;
mod not_found_page;
mod object_storage;
mod onebox_engines;
mod parity;
mod plugin_api;
mod plugin_cakeday;
mod plugin_captcha;
mod plugin_gamification;
mod plugin_graphviz;
mod plugin_lazy_videos;
mod plugin_login_with_amazon;
mod plugin_math;
mod plugin_microsoft_auth;
mod plugin_pages;
mod plugin_styleguide;
mod plugin_templates;
mod plugin_wasm;
mod plugin_wasm_api;
mod plugin_wasm_htmx;
mod policy;
mod post_reply_to;
mod post_voting;
mod posts_bulk;
mod preferences_pages;
mod presence;
mod pretty_text;
mod rate_limiter;
mod readonly;
mod reply_trimmer;
mod request_cancellation;
mod request_tracker;
mod request_tracking;
mod review_pages;
mod schema;
mod search;
mod sessions;
mod sidebar;
mod similar_topics;
mod site_basic_info;
mod site_icons;
mod site_json;
mod slugs;
mod static_pages;
mod svg_sprite;
mod tag_pages;
mod theme_html_pages;
mod theme_screenshots;
mod top_hot_lists;
mod topic_json;
mod tracking_report;
mod user_activity_pages;
mod user_avatars;
mod user_comm_screener;
mod user_notes;
mod user_pages;
mod users_pages;
mod web_hooks;
mod webauthn;

/// A file under tests/ that is not listed above would never be compiled or
/// run.
#[test]
fn every_test_file_is_a_module() {
    let listed = include_str!("integration.rs");
    let missing: Vec<String> = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests"))
        .unwrap()
        .filter_map(|entry| entry.unwrap().file_name().into_string().ok())
        .filter_map(|name| name.strip_suffix(".rs").map(str::to_owned))
        .filter(|name| !["integration", "writes"].contains(&name.as_str()))
        .filter(|name| !listed.contains(&format!("\nmod {name};\n")))
        .collect();
    assert!(
        missing.is_empty(),
        "add to tests/integration.rs: {}",
        missing
            .iter()
            .map(|name| format!("mod {name};"))
            .collect::<Vec<_>>()
            .join(" ")
    );
}
