mod about;
mod accounts;
mod admin_advanced;
mod admin_api_keys;
mod admin_badges;
mod admin_dashboard;
mod admin_dashboard_sections;
mod admin_email;
mod admin_email_pages;
mod admin_email_style;
mod admin_email_templates;
mod admin_embedding;
mod admin_emoji;
mod admin_flags;
mod admin_form_templates;
mod admin_groups;
mod admin_logs;
mod admin_pages;
mod admin_reports;
mod admin_site_settings;
mod admin_user_fields;
mod admin_users;
mod admin_watched_words;
mod admin_web_hooks;
mod archive_message;
mod backups;
mod badges;
mod bookmark_writes;
mod bookmarks;
mod bus;
mod change_owner;
pub(crate) mod composer;
mod deleted_posts;
mod directory;
mod directory_columns;
mod discourse_connect;
mod do_not_disturb;
mod drafts;
mod email;
mod emojis;
mod group_membership;
mod groups;
mod hashtags;
mod highlight_js;
mod list;
mod live;
mod login_required;
mod messages;
mod not_found;
mod notifications;
mod omniauth;
mod permalinks;
mod pm_invite;
mod post_actions;
mod post_destroy;
mod posts;
mod presence;
mod read_tracking;
mod recent_searches;
mod review;
mod robots;
mod search;
mod search_menu;
mod uploads;
pub(crate) use search::{invalid_access, invalid_access_at};
pub(crate) use session::{csrf_ok, viewer_state};
pub(crate) use sitemap::regenerate_sitemaps;
pub(crate) use topics::not_found_response;
mod categories;
mod category_notifications;
mod chat;
mod chat_pages;
mod color_schemes;
mod impersonate;
mod invite_redeem;
pub(crate) use invite_redeem::create_staged_user;
mod invites;
mod onebox;
mod polls;
mod reactions;
mod required_fields;
mod session;
mod session_2fa;
mod sidebar_sections;
mod site;
mod site_texts;
mod sitemap;
mod solved;
mod srv;
mod stylesheets;
mod svg_sprite;
mod tag_admin;
mod tag_groups;
mod tag_search;
mod tags;
mod themes;
mod topic_convert;
mod topic_excerpts;
mod topic_move;
mod topic_notifications;
mod topic_status;
mod topic_timer;
mod topic_update;
mod topic_voting;
mod topics;
mod topics_bulk;
mod user_activity;
mod user_api_key_auth;
mod user_api_key_device;
mod user_api_keys;
mod user_avatar_writes;
mod user_avatars;
mod user_badges;
pub(crate) mod user_card;
pub(crate) mod user_menu;
mod user_notifications;
mod user_preferences;
mod user_search;
mod user_security;
mod user_update;
mod users;
mod users_email;
mod wizard;

use axum::Router;
use axum::routing::{delete, get, post, put};
use tower_http::services::ServeDir;

use crate::AppState;

/// The Font Awesome icons the pages use, as a sprite of <symbol>s
/// (scripts/build-icon-sprite): served as a script (svg_sprite::chrome_script)
/// and inlined where a page has no sprite script.
pub const ICONS_SVG: &str = include_str!("../../static/vendor/icons.svg");

/// GET /assets/<name>: the embedded stylesheets and scripts (crate::assets).
async fn asset(uri: axum::http::Uri, headers: axum::http::HeaderMap) -> axum::response::Response {
    let name = uri
        .path()
        .trim_start_matches('/')
        .trim_start_matches("assets/");
    crate::assets::serve(name, uri.query(), &headers)
}

/// A route Rails caches for anonymous visitors (`discourse_expires_in
/// 1.minute`): the response carries the duration for crate::anonymous_cache.
macro_rules! cached {
    ($route:expr) => {
        $route.layer(axum::middleware::from_fn(
            crate::anonymous_cache::expires_in_one_minute,
        ))
    };
}

pub fn router(state: &AppState) -> Router<AppState> {
    let config = &state.config;
    let public = &config.public_dir;
    // Site images first (a backup's or an install's public/), then the
    // stock ones from a Discourse checkout.
    let images = match &config.discourse_src {
        Some(src) => {
            ServeDir::new(public.join("images")).fallback(ServeDir::new(src.join("public/images")))
        }
        None => ServeDir::new(public.join("images")).fallback(ServeDir::new(public.join("images"))),
    };
    let mut router = Router::new()
        .route("/srv/status", get(srv::status))
        .route("/presence/get", get(presence::get))
        .route("/presence/update", post(presence::update))
        .route("/bus/events", get(bus::events))
        .route("/bus/poll", get(bus::poll))
        .route("/live", get(live::page))
        .route("/user-menu", get(user_menu::show))
        .route("/search-menu", get(search_menu::show))
        .route("/user-card/{username}", get(user_card::show))
        .route("/live/post/{id}", get(live::post))
        .route(
            "/live/chat/{channel_id}/{message_id}",
            get(chat_pages::live_message),
        )
        .route("/", cached!(get(list::latest)))
        .route("/latest", cached!(get(list::latest)))
        .route("/latest.json", cached!(get(list::latest_json)))
        .route("/top", cached!(get(list::top)))
        .route("/top.json", cached!(get(list::top_json)))
        .route("/top/{period}", get(list::top_period_redirect))
        .route("/hot", cached!(get(list::hot)))
        .route("/hot.json", cached!(get(list::hot_json)))
        // discourse-topic-voting's filter.
        .route("/votes", cached!(get(list::user_list)))
        .route("/votes.json", cached!(get(list::user_list)))
        .route("/unread", cached!(get(list::user_list)))
        .route("/unread.json", cached!(get(list::user_list)))
        .route("/new", cached!(get(list::user_list)))
        .route("/new.json", cached!(get(list::user_list)))
        .route("/unseen", cached!(get(list::user_list)))
        .route("/unseen.json", cached!(get(list::user_list)))
        .route("/read", cached!(get(list::user_list)))
        .route("/read.json", cached!(get(list::user_list)))
        .route("/posted", cached!(get(list::user_list)))
        .route("/posted.json", cached!(get(list::user_list)))
        .route(
            "/bookmarks",
            cached!(get(list::user_list)).post(bookmark_writes::create),
        )
        .route(
            "/bookmarks.json",
            cached!(get(list::user_list)).post(bookmark_writes::create),
        )
        .route(
            "/bookmarks/bulk",
            put(user_activity::bulk_bookmarks).delete(user_activity::destroy_bookmark_bulk),
        )
        .route(
            "/bookmarks/{id}",
            put(bookmark_writes::update).delete(bookmark_writes::destroy),
        )
        .route(
            "/bookmarks/{id}/toggle_pin",
            put(bookmark_writes::toggle_pin),
        )
        .route(
            "/bookmarks/{id}/toggle_pin.json",
            put(bookmark_writes::toggle_pin),
        )
        .route(
            "/do-not-disturb",
            post(do_not_disturb::create).delete(do_not_disturb::destroy),
        )
        .route(
            "/do-not-disturb.json",
            post(do_not_disturb::create).delete(do_not_disturb::destroy),
        )
        .route("/notifications", get(notifications::index))
        .route("/notifications.json", get(notifications::index))
        .route("/onebox", get(onebox::show))
        .route("/inline-onebox", get(onebox::inline))
        .route("/hashtags", get(hashtags::lookup))
        .route("/hashtags.json", get(hashtags::lookup))
        .route("/hashtags/by-ids", get(hashtags::by_ids))
        .route("/hashtags/by-ids.json", get(hashtags::by_ids))
        .route("/hashtags/search", get(hashtags::search))
        .route("/hashtags/search.json", get(hashtags::search))
        .route("/drafts", get(drafts::index).post(drafts::create))
        .route("/drafts.json", get(drafts::index).post(drafts::create))
        .route(
            "/drafts/bulk_destroy",
            get(drafts::show_bulk_destroy).delete(drafts::bulk_destroy),
        )
        .route(
            "/drafts/bulk_destroy.json",
            get(drafts::show_bulk_destroy).delete(drafts::bulk_destroy),
        )
        .route("/drafts/{id}", get(drafts::show).delete(drafts::destroy))
        .route("/notifications/mark-read", put(read_tracking::mark_read))
        .route(
            "/notifications/mark-read.json",
            put(read_tracking::mark_read),
        )
        .route("/notifications/read", put(read_tracking::mark_read))
        .route("/notifications/read.json", put(read_tracking::mark_read))
        .route("/topics/timings", post(read_tracking::timings))
        .route("/topics/bulk", put(topics_bulk::bulk))
        .route("/topics/bulk.json", put(topics_bulk::bulk))
        .route("/topics/reset-new", put(topics_bulk::reset_new))
        .route("/topics/reset-new.json", put(topics_bulk::reset_new))
        .route("/topics/pm-reset-new", put(topics_bulk::pm_reset_new))
        .route("/topics/pm-reset-new.json", put(topics_bulk::pm_reset_new))
        .route("/topics/timings.json", post(read_tracking::timings))
        .route(
            "/t/{slug}/timings",
            post(read_tracking::topic_timings).delete(read_tracking::destroy_timings),
        )
        .route(
            "/t/{slug}/timings.json",
            delete(read_tracking::destroy_timings),
        )
        // chat's Chat::Api controllers.
        // Chat::Api::ChannelMessagesController#create.
        .route("/chat/{id}", post(chat::create_message))
        // Full-page chat (Chat::ChatController#respond).
        .route("/chat", get(chat_pages::index))
        .route("/chat/channels", get(chat_pages::index))
        .route("/chat/disabled", get(chat_pages::disabled))
        .route("/chat/browse", get(chat_pages::browse_index))
        .route("/chat/browse/{tab}", get(chat_pages::browse))
        .route("/chat/browse/{tab}/page", get(chat_pages::browse_more))
        .route("/chat/c/{slug}/{id}", get(chat_pages::channel))
        .route("/chat/c/{slug}/{id}/{message_id}", get(chat_pages::channel))
        .route("/chat/direct_messages", get(chat::direct_messages))
        .route("/chat/direct_messages.json", get(chat::direct_messages))
        .route(
            "/chat/dismiss-retention-reminder",
            post(chat::dismiss_retention_reminder),
        )
        .route(
            "/chat/dismiss-retention-reminder.json",
            post(chat::dismiss_retention_reminder),
        )
        .route("/chat/api/me/channels", get(chat::me_channels))
        .route("/chat/api/me/channels.json", get(chat::me_channels))
        .route("/chat/api/channels", get(chat::index))
        .route("/chat/api/channels.json", get(chat::index))
        .route("/chat/api/chatables", get(chat::chatables))
        .route("/chat/api/chatables.json", get(chat::chatables))
        .route(
            "/chat/api/direct-message-channels",
            post(chat::create_direct_message_channel),
        )
        .route(
            "/chat/api/direct-message-channels.json",
            post(chat::create_direct_message_channel),
        )
        .route(
            "/chat/api/channels/{id}",
            get(chat::show).put(chat::update_channel),
        )
        .route(
            "/chat/api/channels/{id}/memberships",
            get(chat::memberships).post(chat::add_members),
        )
        .route(
            "/chat/api/channels/{id}/memberships.json",
            get(chat::memberships).post(chat::add_members),
        )
        .route(
            "/chat/api/channels/{id}/memberships/{user_id}",
            delete(chat::remove_member),
        )
        .route("/chat/api/channels/read", put(chat::mark_all_read))
        .route("/chat/api/channels/read.json", put(chat::mark_all_read))
        .route("/chat/api/channels/{id}/drafts", post(chat::draft))
        .route("/chat/api/channels/{id}/drafts.json", post(chat::draft))
        .route("/chat/api/channels/{id}/read", put(chat::mark_read))
        .route("/chat/api/channels/{id}/read.json", put(chat::mark_read))
        .route(
            "/chat/api/channels/{id}/memberships/me",
            post(chat::own_membership)
                .put(chat::own_membership)
                .delete(chat::own_membership),
        )
        .route(
            "/chat/api/channels/{id}/memberships/me.json",
            post(chat::own_membership)
                .put(chat::own_membership)
                .delete(chat::own_membership),
        )
        .route(
            "/chat/api/channels/{id}/memberships/me/follows",
            delete(chat::unfollow),
        )
        .route(
            "/chat/api/channels/{id}/memberships/me/follows.json",
            delete(chat::unfollow),
        )
        .route("/chat/api/channels/{id}/messages", get(chat::messages))
        .route("/chat/api/channels/{id}/messages.json", get(chat::messages))
        .route(
            "/chat/api/channels/{id}/messages/{message_id}",
            put(chat::update_message).delete(chat::trash_message),
        )
        .route(
            "/chat/api/channels/{id}/messages/{message_id}/restore",
            put(chat::restore_message),
        )
        .route(
            "/chat/api/channels/{id}/messages/{message_id}/restore.json",
            put(chat::restore_message),
        )
        .route("/chat/{id}/react/{message_id}", put(chat::react))
        .route("/chat/{id}/{message_id}/rebake", put(chat::rebake_message))
        .route(
            "/chat/{id}/{message_id}/rebake.json",
            put(chat::rebake_message),
        )
        // discourse-solved's AnswerController.
        .route("/solution/accept", post(solved::accept))
        .route("/solution/accept.json", post(solved::accept))
        .route("/solution/unaccept", post(solved::unaccept))
        .route("/solution/unaccept.json", post(solved::unaccept))
        .route("/solution/by_user", get(solved::by_user))
        .route("/solution/shared_issue", post(solved::shared_issue))
        .route("/solution/shared_issue.json", post(solved::shared_issue))
        .route("/solution/by_user.json", get(solved::by_user))
        .route(
            "/category/{category_id}/notifications",
            post(category_notifications::set_notifications).get(permalinks::category_redirect),
        )
        .route(
            "/category/{category_id}/notifications.json",
            post(category_notifications::set_notifications).get(permalinks::category_redirect),
        )
        .route(
            "/category/{category_id}/slug",
            put(categories::update_slug).get(permalinks::category_redirect),
        )
        .route(
            "/category/{category_id}/slug.json",
            put(categories::update_slug).get(permalinks::category_redirect),
        )
        .route("/new-category", get(categories::new_category_page))
        .route("/new-category/{tab}", get(categories::new_category_page))
        .route(
            "/categories/{id}",
            put(categories::update).delete(categories::destroy),
        )
        // discourse-reactions' CustomReactionsController.
        .route(
            "/discourse-reactions/posts/{post_id}/custom-reactions/{reaction}/toggle",
            put(reactions::toggle),
        )
        .route(
            "/discourse-reactions/posts/{post_id}/custom-reactions/{reaction}/toggle.json",
            put(reactions::toggle),
        )
        .route(
            "/discourse-reactions/posts/reactions",
            get(reactions::reactions_given),
        )
        .route(
            "/discourse-reactions/posts/reactions.json",
            get(reactions::reactions_given),
        )
        .route(
            "/discourse-reactions/posts/reactions-received",
            get(reactions::reactions_received),
        )
        .route(
            "/discourse-reactions/posts/reactions-received.json",
            get(reactions::reactions_received),
        )
        .route(
            "/discourse-reactions/posts/{id}/reactions-users",
            get(reactions::post_reactions_users),
        )
        .route(
            "/discourse-reactions/posts/{id}/reactions-users.json",
            get(reactions::post_reactions_users),
        )
        .route(
            "/discourse-reactions/posts/{id}/reactions-users-list",
            get(reactions::reactions_users_list),
        )
        .route(
            "/discourse-reactions/posts/{id}/reactions-users-list.json",
            get(reactions::reactions_users_list),
        )
        // discourse-topic-voting's VotesController.
        .route("/polls/vote", put(polls::vote).delete(polls::remove_vote))
        .route(
            "/polls/vote.json",
            put(polls::vote).delete(polls::remove_vote),
        )
        .route("/polls/toggle_status", put(polls::toggle_status))
        .route("/polls/toggle_status.json", put(polls::toggle_status))
        .route("/polls/voters", get(polls::voters))
        .route("/polls/voters.json", get(polls::voters))
        .route(
            "/polls/grouped_poll_results",
            get(polls::grouped_poll_results),
        )
        .route(
            "/polls/grouped_poll_results.json",
            get(polls::grouped_poll_results),
        )
        .route("/voting/vote", post(topic_voting::vote))
        .route("/voting/vote.json", post(topic_voting::vote))
        .route("/voting/unvote", post(topic_voting::unvote))
        .route("/voting/unvote.json", post(topic_voting::unvote))
        .route("/voting/who", get(topic_voting::who))
        .route("/voting/who.json", get(topic_voting::who))
        .route(
            "/topics/voted-by/{username}",
            cached!(get(topic_voting::voted_by)),
        )
        .route(
            "/topics/created-by/{username}",
            cached!(get(list::topics_by)),
        )
        // TopicsController#set_notifications; {slug} holds the topic id.
        .route(
            "/t/{slug}/notifications",
            post(topic_notifications::set_notifications),
        )
        .route("/topics/{kind}/{username}", get(messages::personal))
        .route(
            "/topics/private-messages-group/{username}/{*rest}",
            get(messages::group),
        )
        .route(
            "/t/{id}",
            cached!(get(topics::show_by_id))
                .delete(post_destroy::destroy_topic)
                .put(topic_update::update_by_id),
        )
        .route(
            "/t/{slug}/{id}",
            cached!(get(topics::show_with_slug)).put(topic_update::update_with_slug),
        )
        .route(
            "/t/{slug}/{id}/{post_number}",
            cached!(get(topics::show_post)),
        )
        // TopicsController#status; {slug} holds the topic id here.
        .route("/t/{slug}/status", put(topic_status::status))
        // TopicsController#move_posts; {slug} holds the topic id.
        .route("/t/{slug}/move-posts", post(topic_move::move_posts))
        .route("/t/{slug}/move-posts.json", post(topic_move::move_posts))
        // TopicsController#merge_topic; {slug} holds the topic id.
        .route("/t/{slug}/merge-topic", post(topic_move::merge_topic))
        .route("/t/{slug}/merge-topic.json", post(topic_move::merge_topic))
        // TopicsController#convert_topic; {slug} holds the topic id.
        .route(
            "/t/{slug}/convert-topic/{kind}",
            put(topic_convert::convert),
        )
        // TopicsController#timer; {slug} holds the topic id.
        .route("/t/{slug}/timer", post(topic_timer::timer))
        .route("/t/{slug}/excerpts", get(topic_excerpts::excerpts))
        .route("/t/{slug}/excerpts.json", get(topic_excerpts::excerpts))
        .route("/t/{slug}/slow_mode", put(topic_timer::slow_mode))
        // {slug} holds the topic id.
        .route("/t/{slug}/archive-message", put(archive_message::archive))
        .route("/t/{slug}/change-owner", post(change_owner::change_owner))
        .route(
            "/t/{slug}/change-owner.json",
            post(change_owner::change_owner),
        )
        // InvitesController
        .route("/invites", post(invites::create).delete(invites::destroy))
        .route(
            "/invites.json",
            post(invites::create).delete(invites::destroy),
        )
        .route(
            "/invites/{id}",
            get(invite_redeem::show)
                .put(invites::update)
                .delete(invites::destroy),
        )
        .route(
            "/invites/show/{id}",
            put(invite_redeem::perform_accept_invitation),
        )
        .route(
            "/session/login-code",
            post(invite_redeem::create_login_code),
        )
        .route(
            "/session/login-code.json",
            post(invite_redeem::create_login_code),
        )
        .route(
            "/session/login-code/verify",
            post(invite_redeem::verify_login_code),
        )
        .route(
            "/session/login-code/verify.json",
            post(invite_redeem::verify_login_code),
        )
        .route("/invites/upload_csv", post(invites::upload_csv))
        .route("/invites/upload_csv.json", post(invites::upload_csv))
        .route("/invites/retrieve", get(invites::retrieve))
        .route("/invites/retrieve.json", get(invites::retrieve))
        .route("/invites/create-multiple", post(invites::create_multiple))
        .route(
            "/invites/create-multiple.json",
            post(invites::create_multiple),
        )
        .route(
            "/invites/destroy-all-expired",
            post(invites::destroy_all_expired),
        )
        .route(
            "/invites/destroy-all-expired.json",
            post(invites::destroy_all_expired),
        )
        .route("/invites/reinvite", post(invites::resend_invite))
        .route("/invites/reinvite.json", post(invites::resend_invite))
        .route("/invites/reinvite-all", post(invites::resend_all_invites))
        .route(
            "/invites/reinvite-all.json",
            post(invites::resend_all_invites),
        )
        .route("/t/{slug}/invite", post(pm_invite::invite))
        .route("/t/{slug}/invite.json", post(pm_invite::invite))
        .route("/t/{slug}/invite-group", post(pm_invite::invite_group))
        .route("/t/{slug}/invite-group.json", post(pm_invite::invite_group))
        .route(
            "/t/{slug}/remove-allowed-group",
            put(pm_invite::remove_allowed_group),
        )
        .route(
            "/t/{slug}/remove-allowed-group.json",
            put(pm_invite::remove_allowed_group),
        )
        .route(
            "/t/{slug}/remove-allowed-user",
            put(pm_invite::remove_allowed_user),
        )
        .route(
            "/t/{slug}/remove-allowed-user.json",
            put(pm_invite::remove_allowed_user),
        )
        .route(
            "/t/{slug}/move-to-inbox",
            put(archive_message::move_to_inbox),
        )
        .route("/t/{slug}/{id}/status", put(topic_status::status_with_slug))
        .route(
            "/u/recent-searches",
            get(recent_searches::show).delete(recent_searches::reset),
        )
        .route(
            "/u/recent-searches.json",
            get(recent_searches::show).delete(recent_searches::reset),
        )
        .route("/u/check_username", get(user_preferences::check_username))
        .route(
            "/u/check_username.json",
            get(user_preferences::check_username),
        )
        .route(
            "/users/check_username",
            get(user_preferences::check_username),
        )
        .route(
            "/users/check_username.json",
            get(user_preferences::check_username),
        )
        .route("/my/{*path}", get(user_preferences::my_redirect))
        .route("/login-preferences", get(login_required::show_login))
        .route(
            "/.well-known/change-password",
            get(user_preferences::change_password),
        )
        .route("/u/search/users", get(user_search::search_users))
        .route("/u/search/users.json", get(user_search::search_users))
        .route(
            "/u/{username}",
            get(users::show)
                .put(user_update::update)
                .post(user_badges::post_user)
                .delete(user_update::destroy),
        )
        .route("/u/{username}/bookmarks", get(bookmarks::index))
        .route("/u/{username}/bookmarks.json", get(bookmarks::index))
        .route(
            "/u/{username}/user-menu-bookmarks",
            get(bookmarks::user_menu),
        )
        .route(
            "/u/{username}/user-menu-bookmarks.json",
            get(bookmarks::user_menu),
        )
        .route(
            "/u/{username}/user-menu-private-messages",
            get(user_menu::messages),
        )
        .route(
            "/u/{username}/user-menu-private-messages.json",
            get(user_menu::messages),
        )
        .route(
            "/u/confirm-old-email/{token}",
            get(users_email::show_confirm_old).put(users_email::confirm_old),
        )
        .route(
            "/u/confirm-new-email/{token}",
            get(users_email::show_confirm_new).put(users_email::confirm_new),
        )
        .route(
            "/users/confirm-old-email/{token}",
            get(users_email::show_confirm_old).put(users_email::confirm_old),
        )
        .route(
            "/users/confirm-new-email/{token}",
            get(users_email::show_confirm_new).put(users_email::confirm_new),
        )
        .route(
            "/u/{username}/{*rest}",
            get(users::show_with_tail)
                .put(user_badges::put_user_tail)
                .post(user_preferences::post_tail)
                .delete(user_preferences::delete_tail),
        )
        .route(
            "/users/{username}",
            get(users::show).post(user_badges::post_user),
        )
        .route("/users/{username}/bookmarks", get(bookmarks::index))
        .route("/users/{username}/bookmarks.json", get(bookmarks::index))
        .route(
            "/users/{username}/user-menu-bookmarks",
            get(bookmarks::user_menu),
        )
        .route(
            "/users/{username}/user-menu-bookmarks.json",
            get(bookmarks::user_menu),
        )
        .route(
            "/users/{username}/user-menu-private-messages",
            get(user_menu::messages),
        )
        .route(
            "/users/{username}/user-menu-private-messages.json",
            get(user_menu::messages),
        )
        .route(
            "/users/{username}/{*rest}",
            get(users::show_with_tail)
                .put(user_badges::put_user_tail)
                .post(user_preferences::post_tail)
                .delete(user_preferences::delete_tail),
        )
        .route("/user_actions.json", get(users::actions))
        // UserApiKeysController and UserApiKeyClientsController.
        .route("/user-api-key/new", get(user_api_key_auth::new))
        .route("/user-api-key/new.json", get(user_api_key_auth::new))
        .route("/user-api-key", post(user_api_key_auth::create))
        .route("/user-api-key.json", post(user_api_key_auth::create))
        .route(
            "/user-api-key/otp",
            get(user_api_key_auth::otp).post(user_api_key_auth::create_otp),
        )
        .route(
            "/user-api-key/otp.json",
            get(user_api_key_auth::otp).post(user_api_key_auth::create_otp),
        )
        .route(
            "/user-api-key/device",
            post(user_api_key_device::create_device_request),
        )
        .route(
            "/user-api-key/device.json",
            post(user_api_key_device::create_device_request),
        )
        .route(
            "/user-api-key/device/poll",
            post(user_api_key_device::poll_device_request),
        )
        .route(
            "/user-api-key/device/poll.json",
            post(user_api_key_device::poll_device_request),
        )
        .route(
            "/user-api-key/activate",
            get(user_api_key_device::activate).post(user_api_key_device::activate),
        )
        .route(
            "/user-api-key/activate.json",
            get(user_api_key_device::activate).post(user_api_key_device::activate),
        )
        .route(
            "/user-api-key/device/authorize",
            post(user_api_key_device::authorize_device_request),
        )
        .route(
            "/user-api-key/device/authorize.json",
            post(user_api_key_device::authorize_device_request),
        )
        .route(
            "/user-api-key/device/deny",
            post(user_api_key_device::deny_device_request),
        )
        .route(
            "/user-api-key/device/deny.json",
            post(user_api_key_device::deny_device_request),
        )
        .route(
            "/session/otp/{token}",
            get(user_api_key_auth::session_otp).post(user_api_key_auth::session_otp),
        )
        .route("/user-api-key/revoke", post(user_api_keys::revoke))
        .route("/user-api-key/revoke.json", post(user_api_keys::revoke))
        .route(
            "/user-api-key/undo-revoke",
            post(user_api_keys::undo_revoke),
        )
        .route(
            "/user-api-key/undo-revoke.json",
            post(user_api_keys::undo_revoke),
        )
        .route(
            "/user-api-key-client",
            get(user_api_keys::client_show).post(user_api_keys::client_create),
        )
        .route(
            "/user-api-key-client.json",
            get(user_api_keys::client_show).post(user_api_keys::client_create),
        )
        .route("/c/{*path}", cached!(get(list::category)))
        .route("/robots.txt", get(robots::index))
        .route("/robots-builder.json", get(robots::builder))
        .route("/sitemap.xml", get(sitemap::index))
        .route("/sitemap_{page}", get(sitemap::page))
        .route("/news.xml", get(sitemap::news))
        .route(
            "/admin/config/emoji",
            get(admin_emoji::index).post(admin_emoji::create),
        )
        .route(
            "/admin/config/emoji.json",
            get(admin_emoji::index).post(admin_emoji::create),
        )
        .route("/admin/config/emoji/new", get(admin_emoji::page))
        .route("/admin/config/emoji/settings", get(admin_emoji::page))
        .route("/admin/config/emoji/export", post(admin_emoji::export))
        .route("/admin/config/emoji/export.json", post(admin_emoji::export))
        .route(
            "/admin/config/emoji/import_preview",
            post(admin_emoji::import_preview),
        )
        .route(
            "/admin/config/emoji/import_preview.json",
            post(admin_emoji::import_preview),
        )
        .route(
            "/admin/config/emoji/import_confirm",
            post(admin_emoji::import_confirm),
        )
        .route(
            "/admin/config/emoji/import_confirm.json",
            post(admin_emoji::import_confirm),
        )
        .route("/admin/config/emoji/import", get(admin_emoji::page))
        .route("/admin/config/emoji/{id}", delete(admin_emoji::destroy))
        .route(
            "/admin/config/user_fields",
            get(admin_user_fields::index).post(admin_user_fields::create),
        )
        .route(
            "/admin/config/user_fields.json",
            get(admin_user_fields::index).post(admin_user_fields::create),
        )
        .route(
            "/admin/config/user_fields/{id}",
            get(admin_user_fields::show)
                .put(admin_user_fields::update)
                .delete(admin_user_fields::destroy),
        )
        .route(
            "/admin/config/user_fields/{id}/edit",
            get(admin_user_fields::edit),
        )
        .route(
            "/admin/config/user_fields/{id}/edit.json",
            get(admin_user_fields::edit),
        )
        .route(
            "/admin/config/user-fields",
            get(admin_user_fields::index_alias),
        )
        .route(
            "/admin/config/user-fields.json",
            get(admin_user_fields::index_alias),
        )
        .route(
            "/admin/config/user-fields/{id}",
            get(admin_user_fields::show),
        )
        .route(
            "/admin/config/user-fields/{id}/edit",
            get(admin_user_fields::edit),
        )
        .route(
            "/admin/config/user-fields/{id}/edit.json",
            get(admin_user_fields::edit),
        )
        .route(
            "/admin/config/flags",
            get(admin_flags::empty).post(admin_flags::create),
        )
        .route(
            "/admin/config/flags.json",
            get(admin_flags::empty).post(admin_flags::create),
        )
        .route(
            "/admin/config/flags/{id}",
            get(admin_flags::empty_member)
                .put(admin_flags::update)
                .delete(admin_flags::destroy),
        )
        .route(
            "/admin/config/flags/{flag_id}/toggle",
            put(admin_flags::toggle),
        )
        .route(
            "/admin/config/flags/{flag_id}/toggle.json",
            put(admin_flags::toggle),
        )
        .route(
            "/admin/config/flags/{flag_id}/reorder/{direction}",
            put(admin_flags::reorder),
        )
        .route(
            "/admin/customize/embedding",
            get(admin_embedding::show).put(admin_embedding::update),
        )
        .route(
            "/admin/customize/embedding.json",
            get(admin_embedding::show).put(admin_embedding::update),
        )
        .route(
            "/admin/customize/embedding/{id}",
            get(admin_embedding::edit),
        )
        .route("/admin/embeddable_hosts", post(admin_embedding::create))
        .route(
            "/admin/embeddable_hosts.json",
            post(admin_embedding::create),
        )
        .route(
            "/admin/embeddable_hosts/{id}",
            put(admin_embedding::update_host).delete(admin_embedding::destroy_host),
        )
        .route(
            "/admin/customize/form-templates",
            get(admin_form_templates::index).post(admin_form_templates::create),
        )
        .route(
            "/admin/customize/form-templates.json",
            get(admin_form_templates::index).post(admin_form_templates::create),
        )
        .route(
            "/admin/customize/form-templates/{id}",
            get(admin_form_templates::show)
                .put(admin_form_templates::update)
                .delete(admin_form_templates::destroy),
        )
        .route(
            "/admin/customize/form-templates/{id}/edit",
            get(admin_form_templates::edit),
        )
        .route(
            "/admin/customize/form-templates/{id}/edit.json",
            get(admin_form_templates::edit),
        )
        .route("/form-templates", get(admin_form_templates::public_index))
        .route(
            "/form-templates.json",
            get(admin_form_templates::public_index),
        )
        .route(
            "/form-templates/{id}",
            get(admin_form_templates::public_show),
        )
        .route("/site/emoji", get(admin_emoji::site_emoji))
        .route("/site/emoji.json", get(admin_emoji::site_emoji))
        .route(
            "/admin/badges",
            get(admin_badges::index).post(admin_badges::create),
        )
        .route(
            "/admin/badges.json",
            get(admin_badges::index).post(admin_badges::create),
        )
        .route("/admin/badges/types", get(admin_badges::badge_types))
        .route("/admin/badges/types.json", get(admin_badges::badge_types))
        .route(
            "/admin/badges/badge_groupings",
            post(admin_badges::save_badge_groupings),
        )
        .route(
            "/admin/badges/badge_groupings.json",
            post(admin_badges::save_badge_groupings),
        )
        .route("/admin/badges/preview", post(admin_badges::preview))
        .route("/admin/badges/preview.json", post(admin_badges::preview))
        .route(
            "/admin/badges/award/{badge_id}",
            get(admin_badges::page).post(admin_badges::mass_award),
        )
        .route(
            "/admin/badges/{id}",
            get(admin_badges::page)
                .put(admin_badges::update)
                .delete(admin_badges::destroy),
        )
        .route("/admin/site_settings", get(admin_pages::site_settings_root))
        .route(
            "/admin/config/site_settings",
            get(admin_site_settings::config_index),
        )
        .route(
            "/admin/config/site_settings.json",
            get(admin_site_settings::config_index),
        )
        .route(
            "/admin/site_settings/category/{category}",
            get(admin_pages::site_settings_category),
        )
        .route("/admin/site_settings.json", get(admin_site_settings::index))
        .route(
            "/admin/site_settings/{id}",
            put(admin_site_settings::update),
        )
        .route(
            "/admin/site_settings/{id}/user_count",
            put(admin_site_settings::user_count),
        )
        .route(
            "/admin/site_settings/{id}/user_count.json",
            put(admin_site_settings::user_count),
        )
        .route(
            "/admin/logs/screened_emails",
            get(admin_logs::screened_emails),
        )
        .route(
            "/admin/logs/screened_emails.json",
            get(admin_logs::screened_emails),
        )
        .route(
            "/admin/logs/screened_emails/{id}",
            delete(admin_logs::destroy_screened_email),
        )
        .route("/admin/email-logs", get(admin_logs::email_logs_index))
        .route("/admin/email-logs/{kind}", get(admin_logs::email_logs))
        .route(
            "/admin/email-logs/incoming/{id}",
            get(admin_logs::incoming_email),
        )
        .route(
            "/admin/email-logs/incoming_from_bounced/{id}",
            get(admin_logs::incoming_from_bounced),
        )
        .route("/admin/logs/search_logs", get(admin_logs::search_logs))
        .route("/admin/logs/search_logs.json", get(admin_logs::search_logs))
        .route(
            "/admin/logs/search_logs/term",
            get(admin_logs::search_log_term),
        )
        .route(
            "/admin/logs/search_logs/term.json",
            get(admin_logs::search_log_term),
        )
        .route("/admin/logs/screened_urls", get(admin_logs::screened_urls))
        .route(
            "/admin/logs/screened_urls.json",
            get(admin_logs::screened_urls),
        )
        .route(
            "/admin/logs/screened_ip_addresses",
            get(admin_logs::screened_ip_addresses).post(admin_logs::create_screened_ip),
        )
        .route(
            "/admin/logs/screened_ip_addresses.json",
            get(admin_logs::screened_ip_addresses).post(admin_logs::create_screened_ip),
        )
        .route(
            "/admin/logs/screened_ip_addresses/{id}",
            put(admin_logs::update_screened_ip).delete(admin_logs::destroy_screened_ip),
        )
        .route(
            "/admin/customize/watched_words",
            get(admin_watched_words::index).post(admin_watched_words::create),
        )
        .route(
            "/admin/customize/watched_words.json",
            get(admin_watched_words::index).post(admin_watched_words::create),
        )
        .route(
            "/admin/customize/watched_words/upload",
            post(admin_watched_words::upload),
        )
        .route(
            "/admin/customize/watched_words/{id}",
            delete(admin_watched_words::destroy),
        )
        .route(
            "/admin/customize/watched_words/action/{id}",
            get(admin_watched_words::index_action).delete(admin_watched_words::clear_all),
        )
        .route(
            "/admin/customize/watched_words/action/{id}/download",
            get(admin_watched_words::download),
        )
        .route("/admin/reports", get(admin_pages::reports_index))
        .route("/admin/reports.json", get(admin_reports::index))
        .route("/admin/reports/bulk", get(admin_reports::bulk))
        .route("/admin/reports/bulk.json", get(admin_reports::bulk))
        .route("/admin/reports/{kind}", get(admin_reports::show))
        .route("/admin", get(admin_pages::dashboard))
        .route("/admin/dashboard", get(admin_dashboard_sections::index))
        .route(
            "/admin/dashboard.json",
            get(admin_dashboard_sections::index),
        )
        .route(
            "/admin/dashboard/general",
            get(admin_dashboard_sections::general),
        )
        .route(
            "/admin/dashboard/general.json",
            get(admin_dashboard_sections::general),
        )
        .route(
            "/admin/dashboard/moderation",
            get(admin_dashboard_sections::empty),
        )
        .route(
            "/admin/dashboard/moderation.json",
            get(admin_dashboard_sections::empty),
        )
        .route(
            "/admin/dashboard/security",
            get(admin_dashboard_sections::empty),
        )
        .route(
            "/admin/dashboard/security.json",
            get(admin_dashboard_sections::empty),
        )
        .route(
            "/admin/dashboard/reports",
            get(admin_dashboard_sections::empty),
        )
        .route(
            "/admin/dashboard/reports.json",
            get(admin_dashboard_sections::empty),
        )
        .route(
            "/admin/dashboard/configuration",
            put(admin_dashboard_sections::update_configuration),
        )
        .route(
            "/admin/dashboard/configuration.json",
            put(admin_dashboard_sections::update_configuration),
        )
        .route(
            "/admin/dashboard/sections/{section_id}/settings/{setting_key}",
            put(admin_dashboard_sections::update_section_settings),
        )
        .route(
            "/admin/dashboard/reports/available",
            get(admin_dashboard_sections::available_reports),
        )
        .route(
            "/admin/dashboard/reports/available.json",
            get(admin_dashboard_sections::available_reports),
        )
        .route(
            "/admin/dashboard/reports/layout",
            put(admin_dashboard_sections::update_reports_layout),
        )
        .route(
            "/admin/dashboard/reports/layout.json",
            put(admin_dashboard_sections::update_reports_layout),
        )
        .route(
            "/admin/dashboard/reports/bulk",
            post(admin_dashboard_sections::bulk_reports),
        )
        .route(
            "/admin/dashboard/reports/bulk.json",
            post(admin_dashboard_sections::bulk_reports),
        )
        .route(
            "/admin/dashboard/site-traffic-explorer",
            get(admin_dashboard_sections::traffic),
        )
        .route(
            "/admin/dashboard/site-traffic-explorer.json",
            get(admin_dashboard_sections::traffic),
        )
        .route(
            "/admin/dashboard/whats-new",
            get(admin_dashboard_sections::upcoming_changes),
        )
        .route(
            "/admin/dashboard/whats-new.json",
            get(admin_dashboard_sections::upcoming_changes),
        )
        .route(
            "/admin/whats-new",
            get(admin_dashboard_sections::upcoming_changes),
        )
        .route(
            "/admin/whats-new.json",
            get(admin_dashboard_sections::upcoming_changes),
        )
        .route(
            "/admin/toggle-feature",
            post(admin_dashboard_sections::upcoming_changes),
        )
        .route(
            "/admin/toggle-feature.json",
            post(admin_dashboard_sections::upcoming_changes),
        )
        .route("/admin/dashboard/problems", post(admin_dashboard::problems))
        .route(
            "/admin/dashboard/problems.json",
            post(admin_dashboard::problems),
        )
        .route("/admin/problem-checks", get(admin_dashboard::index))
        .route("/admin/problem-checks.json", get(admin_dashboard::index))
        .route("/admin/problem_checks", get(admin_dashboard::index))
        .route("/admin/problem_checks.json", get(admin_dashboard::index))
        .route(
            "/admin/problem_checks/{id}/ignore",
            put(admin_dashboard::ignore),
        )
        .route(
            "/admin/problem_checks/{id}/ignore.json",
            put(admin_dashboard::ignore),
        )
        .route(
            "/admin/problem_checks/{id}/watch",
            put(admin_dashboard::watch),
        )
        .route(
            "/admin/problem_checks/{id}/watch.json",
            put(admin_dashboard::watch),
        )
        .route(
            "/admin/admin_notices/{id}",
            delete(admin_dashboard::dismiss),
        )
        .route("/admin/logs", get(admin_logs::staff_action_logs))
        .route("/admin/logs.json", get(admin_logs::staff_action_logs))
        .route(
            "/admin/logs/staff_action_logs",
            get(admin_logs::staff_action_logs),
        )
        .route(
            "/admin/logs/staff_action_logs.json",
            get(admin_logs::staff_action_logs),
        )
        .route("/admin/users", get(admin_pages::users_root))
        .route("/admin/users.json", get(admin_pages::users_root))
        .route("/admin/users/list", get(admin_pages::users_root))
        .route("/admin/users/list.json", get(admin_users::index))
        .route("/admin/users/list/{query}", get(admin_pages::users_list))
        .route("/admin/users/settings", get(admin_pages::users_settings))
        .route(
            "/admin/users/{user_id}/{username}",
            get(admin_pages::user_show),
        )
        .route(
            "/admin/users/{user_id}/{username}/badges",
            get(admin_pages::user_subpage),
        )
        .route(
            "/admin/users/{user_id}/{username}/tl3_requirements",
            get(admin_pages::user_subpage),
        )
        .route(
            "/admin/users/{user_id}",
            get(admin_users::show).delete(admin_users::destroy),
        )
        .route(
            "/admin/users/destroy-bulk",
            delete(admin_users::destroy_bulk),
        )
        .route(
            "/admin/users/delete-others-with-same-ip",
            delete(admin_users::delete_others_with_same_ip),
        )
        .route(
            "/admin/users/delete-others-with-same-ip.json",
            delete(admin_users::delete_others_with_same_ip),
        )
        .route(
            "/admin/users/destroy-bulk.json",
            delete(admin_users::destroy_bulk),
        )
        .route(
            "/admin/users/{user_id}/grant_moderation",
            put(admin_users::grant_moderation),
        )
        .route(
            "/admin/users/{user_id}/revoke_moderation",
            put(admin_users::revoke_moderation),
        )
        .route(
            "/admin/users/{user_id}/revoke_admin",
            put(admin_users::revoke_admin),
        )
        .route(
            "/admin/users/{user_id}/grant_moderation.json",
            put(admin_users::grant_moderation),
        )
        .route(
            "/admin/users/{user_id}/revoke_moderation.json",
            put(admin_users::revoke_moderation),
        )
        .route(
            "/admin/users/{user_id}/revoke_admin.json",
            put(admin_users::revoke_admin),
        )
        .route(
            "/admin/users/{user_id}/trust_level",
            put(admin_users::trust_level),
        )
        .route(
            "/admin/users/{user_id}/trust_level.json",
            put(admin_users::trust_level),
        )
        .route(
            "/admin/users/{user_id}/trust_level_lock",
            put(admin_users::trust_level_lock),
        )
        .route(
            "/admin/users/{user_id}/trust_level_lock.json",
            put(admin_users::trust_level_lock),
        )
        .route(
            "/admin/users/{user_id}/sso_record",
            delete(discourse_connect::destroy_sso_record),
        )
        .route(
            "/admin/users/{user_id}/sso_record.json",
            delete(discourse_connect::destroy_sso_record),
        )
        .route(
            "/admin/users/{user_id}/delete_associated_accounts",
            put(discourse_connect::delete_associated_accounts),
        )
        .route(
            "/admin/users/{user_id}/delete_associated_accounts.json",
            put(discourse_connect::delete_associated_accounts),
        )
        .route("/admin/users/sync_sso", post(discourse_connect::sync_sso))
        .route(
            "/admin/users/sync_sso.json",
            post(discourse_connect::sync_sso),
        )
        .route("/admin/users/{user_id}/log_out", post(admin_users::log_out))
        .route(
            "/admin/users/{user_id}/log_out.json",
            post(admin_users::log_out),
        )
        .route(
            "/admin/users/{user_id}/activate",
            put(admin_users::activate),
        )
        .route(
            "/admin/users/{user_id}/activate.json",
            put(admin_users::activate),
        )
        .route(
            "/admin/users/{user_id}/deactivate",
            put(admin_users::deactivate),
        )
        .route(
            "/admin/users/{user_id}/deactivate.json",
            put(admin_users::deactivate),
        )
        .route(
            "/admin/users/{user_id}/groups",
            post(admin_users::add_group),
        )
        .route(
            "/admin/users/{user_id}/groups/{group_id}",
            delete(admin_users::remove_group),
        )
        .route(
            "/admin/users/{user_id}/primary_group",
            put(admin_users::primary_group),
        )
        .route(
            "/admin/users/{user_id}/groups.json",
            post(admin_users::add_group),
        )
        .route(
            "/admin/users/{user_id}/primary_group.json",
            put(admin_users::primary_group),
        )
        .route("/admin/users/{user_id}/approve", put(admin_users::approve))
        .route(
            "/admin/users/{user_id}/approve.json",
            put(admin_users::approve),
        )
        .route("/admin/users/{user_id}/suspend", put(admin_users::suspend))
        .route(
            "/admin/users/{user_id}/suspend.json",
            put(admin_users::suspend),
        )
        .route(
            "/admin/users/{user_id}/unsuspend",
            put(admin_users::unsuspend),
        )
        .route(
            "/admin/users/{user_id}/unsuspend.json",
            put(admin_users::unsuspend),
        )
        .route("/admin/users/{user_id}/silence", put(admin_users::silence))
        .route(
            "/admin/users/{user_id}/silence.json",
            put(admin_users::silence),
        )
        .route(
            "/admin/users/{user_id}/unsilence",
            put(admin_users::unsilence),
        )
        .route(
            "/admin/users/{user_id}/unsilence.json",
            put(admin_users::unsilence),
        )
        .route(
            "/admin/impersonate",
            get(impersonate::index)
                .post(impersonate::create)
                .delete(impersonate::destroy),
        )
        .route(
            "/admin/impersonate.json",
            get(impersonate::index)
                .post(impersonate::create)
                .delete(impersonate::destroy),
        )
        .route(
            "/admin/users/{user_id}/grant_admin",
            put(admin_users::grant_admin),
        )
        .route(
            "/admin/users/{user_id}/grant_admin.json",
            put(admin_users::grant_admin),
        )
        .route("/admin/users/suspend-bulk", put(admin_users::suspend_bulk))
        .route(
            "/admin/users/suspend-bulk.json",
            put(admin_users::suspend_bulk),
        )
        .route(
            "/admin/users/{user_id}/penalty_history",
            delete(admin_users::penalty_history),
        )
        .route(
            "/admin/users/{user_id}/penalty_history.json",
            delete(admin_users::penalty_history),
        )
        .route(
            "/admin/users/{user_id}/similar-users.json",
            get(admin_users::similar_users),
        )
        .route(
            "/admin/users/{user_id}/anonymize",
            put(admin_users::anonymize),
        )
        .route(
            "/admin/users/{user_id}/anonymize.json",
            put(admin_users::anonymize),
        )
        .route("/admin/users/{user_id}/merge", post(admin_users::merge))
        .route(
            "/admin/users/{user_id}/merge.json",
            post(admin_users::merge),
        )
        .route("/admin/themes", get(themes::index).post(themes::create))
        .route(
            "/admin/themes.json",
            get(themes::index).post(themes::create),
        )
        .route(
            "/admin/themes/import",
            post(themes::import).layer(axum::extract::DefaultBodyLimit::max(
                themes::BUNDLE_BODY_LIMIT,
            )),
        )
        .route(
            "/admin/themes/import.json",
            post(themes::import).layer(axum::extract::DefaultBodyLimit::max(
                themes::BUNDLE_BODY_LIMIT,
            )),
        )
        .route(
            "/admin/themes/upload_asset",
            post(themes::upload_asset).layer(axum::extract::DefaultBodyLimit::max(
                themes::BUNDLE_BODY_LIMIT,
            )),
        )
        .route(
            "/admin/themes/upload_asset.json",
            post(themes::upload_asset).layer(axum::extract::DefaultBodyLimit::max(
                themes::BUNDLE_BODY_LIMIT,
            )),
        )
        .route(
            "/admin/themes/generate_key_pair",
            post(themes::generate_key_pair),
        )
        .route(
            "/admin/themes/generate_key_pair.json",
            post(themes::generate_key_pair),
        )
        .route(
            "/admin/themes/{id}/objects_setting_metadata/{setting_name}",
            get(themes::objects_setting_metadata),
        )
        .route("/admin/themes/{id}/source", put(themes::update_source))
        .route("/admin/themes/{id}/source.json", put(themes::update_source))
        .route("/admin/customize/themes/{id}/export", get(themes::export))
        .route(
            "/admin/customize/themes/{id}/export.json",
            get(themes::export),
        )
        .route(
            "/admin/themes/bulk_destroy",
            axum::routing::delete(themes::bulk_destroy),
        )
        .route(
            "/admin/themes/bulk_destroy.json",
            axum::routing::delete(themes::bulk_destroy),
        )
        .route(
            "/admin/themes/{id}",
            get(themes::show)
                .put(themes::update)
                .delete(themes::destroy),
        )
        .route(
            "/admin/themes/{id}/translations/{locale}",
            get(themes::translations),
        )
        .route("/admin/themes/{id}/preview", get(themes::preview))
        .route("/admin/themes/{id}/setting", put(themes::update_setting))
        .route(
            "/admin/themes/{id}/setting.json",
            put(themes::update_setting),
        )
        .route(
            "/admin/themes/{id}/site-setting",
            put(themes::update_site_setting),
        )
        .route(
            "/admin/themes/{id}/site-setting.json",
            put(themes::update_site_setting),
        )
        .route("/admin/config/customize/themes", get(themes::config_themes))
        .route(
            "/admin/config/customize/themes.json",
            get(themes::config_themes),
        )
        .route(
            "/admin/config/customize/components",
            get(themes::config_components),
        )
        .route(
            "/admin/config/customize/components.json",
            get(themes::config_components),
        )
        .route(
            "/admin/config/customize/theme-site-settings",
            get(themes::config_theme_site_settings),
        )
        .route(
            "/admin/config/customize/theme-site-settings.json",
            get(themes::config_theme_site_settings),
        )
        .route("/admin/config/colors", get(themes::config_colors))
        .route("/admin/config/colors.json", get(themes::config_colors))
        .route(
            "/admin/config/welcome-banner/themes-with-setting",
            get(themes::welcome_banner_themes),
        )
        .route(
            "/admin/config/welcome-banner/themes-with-setting.json",
            get(themes::welcome_banner_themes),
        )
        .route(
            "/admin/config/design-wizard",
            get(wizard::design_wizard).put(wizard::update_design_wizard),
        )
        .route(
            "/admin/config/design-wizard.json",
            get(wizard::design_wizard).put(wizard::update_design_wizard),
        )
        .route("/wizard", get(wizard::index))
        .route("/wizard.json", get(wizard::index))
        .route(
            "/wizard/steps/{id}",
            get(wizard::step_page).put(wizard::update_step),
        )
        .route(
            "/sidebar_sections",
            get(sidebar_sections::index).post(sidebar_sections::create),
        )
        .route(
            "/sidebar_sections.json",
            get(sidebar_sections::index).post(sidebar_sections::create),
        )
        .route(
            "/sidebar_sections/{id}",
            get(sidebar_sections::show)
                .put(sidebar_sections::update)
                .delete(sidebar_sections::destroy),
        )
        .route("/sidebar_sections/reset/{id}", put(sidebar_sections::reset))
        .route(
            "/sidebar_sections/{id}/reorder",
            put(sidebar_sections::reorder),
        )
        .route(
            "/sidebar_sections/{id}/reorder.json",
            put(sidebar_sections::reorder),
        )
        .route(
            "/sidebar_sections/{id}/move_link",
            put(sidebar_sections::move_link),
        )
        .route(
            "/sidebar_sections/{id}/move_link.json",
            put(sidebar_sections::move_link),
        )
        .route(
            "/admin/color_schemes",
            get(color_schemes::index).post(color_schemes::create),
        )
        .route(
            "/admin/color_schemes.json",
            get(color_schemes::index).post(color_schemes::create),
        )
        .route(
            "/admin/color_schemes/{id}",
            put(color_schemes::update).delete(color_schemes::destroy),
        )
        .route("/admin/config/colors/{id}", get(color_schemes::show))
        .route("/admin/customize/site_texts", get(site_texts::index))
        .route("/admin/customize/site_texts.json", get(site_texts::index))
        .route(
            "/admin/customize/site_texts/{id}",
            get(site_texts::show)
                .put(site_texts::update)
                .delete(site_texts::revert),
        )
        .route(
            "/admin/customize/site_texts/{id}/dismiss_outdated",
            put(site_texts::dismiss_outdated),
        )
        .route(
            "/admin/customize/site_texts/{id}/dismiss_outdated.json",
            put(site_texts::dismiss_outdated),
        )
        .route(
            "/admin/customize/reseed",
            get(site_texts::reseed_options).post(site_texts::reseed),
        )
        .route(
            "/admin/customize/reseed.json",
            get(site_texts::reseed_options).post(site_texts::reseed),
        )
        .route(
            "/admin/api/web_hooks",
            get(admin_web_hooks::index).post(admin_web_hooks::create),
        )
        .route(
            "/admin/api/web_hooks.json",
            get(admin_web_hooks::index).post(admin_web_hooks::create),
        )
        .route(
            "/admin/api/web_hooks/{id}",
            get(admin_web_hooks::show)
                .put(admin_web_hooks::update)
                .delete(admin_web_hooks::destroy),
        )
        .route("/admin/api/web_hooks/{id}/edit", get(admin_web_hooks::edit))
        .route(
            "/admin/api/web_hooks/{id}/edit.json",
            get(admin_web_hooks::edit),
        )
        .route(
            "/admin/api/web_hook_events/{id}",
            get(admin_web_hooks::list_events),
        )
        .route(
            "/admin/api/web_hooks/{id}/events/bulk",
            get(admin_web_hooks::bulk_events),
        )
        .route(
            "/admin/api/web_hooks/{id}/events/bulk.json",
            get(admin_web_hooks::bulk_events),
        )
        .route(
            "/admin/api/web_hooks/{id}/events/{event_id}/redeliver",
            post(admin_web_hooks::redeliver_event),
        )
        .route(
            "/admin/api/web_hooks/{id}/events/{event_id}/redeliver.json",
            post(admin_web_hooks::redeliver_event),
        )
        .route(
            "/admin/api/web_hooks/{id}/events/failed_redeliver",
            post(admin_web_hooks::redeliver_failed_events),
        )
        .route(
            "/admin/api/web_hooks/{id}/events/failed_redeliver.json",
            post(admin_web_hooks::redeliver_failed_events),
        )
        .route(
            "/admin/api/web_hooks/{id}/ping",
            post(admin_web_hooks::ping),
        )
        .route(
            "/admin/api/web_hooks/{id}/ping.json",
            post(admin_web_hooks::ping),
        )
        .route(
            "/admin/permalinks",
            get(permalinks::index).post(permalinks::create),
        )
        .route(
            "/admin/permalinks.json",
            get(permalinks::index).post(permalinks::create),
        )
        .route(
            "/admin/permalinks/{id}",
            get(permalinks::show_admin)
                .put(permalinks::update)
                .delete(permalinks::destroy),
        )
        .route(
            "/admin/config/permalinks",
            get(permalinks::index).post(permalinks::create),
        )
        .route(
            "/admin/config/permalinks.json",
            get(permalinks::index).post(permalinks::create),
        )
        .route(
            "/admin/config/permalinks/{id}",
            get(permalinks::show_admin).delete(permalinks::destroy),
        )
        .route("/permalink-check", get(permalinks::check))
        .route("/permalink-check.json", get(permalinks::check))
        // The last route, under PermalinkConstraint.
        .route(
            permalinks::CATCH_ALL,
            get(permalinks::show).fallback(permalinks::no_route),
        )
        // Admin::BackupsController
        .route("/admin/backups", get(backups::index).post(backups::create))
        .route(
            "/admin/backups.json",
            get(backups::index).post(backups::create),
        )
        .route("/admin/backups/cancel", delete(backups::cancel))
        .route("/admin/backups/cancel.json", delete(backups::cancel))
        .route("/admin/backups/rollback", post(backups::rollback))
        .route("/admin/backups/rollback.json", post(backups::rollback))
        .route("/admin/backups/{id}/restore", post(backups::restore))
        .route("/admin/backups/{id}/restore.json", post(backups::restore))
        .route("/admin/backups/settings", get(backups::index))
        .route("/admin/backups/settings.json", get(backups::index))
        .route("/admin/backups/status", get(backups::status))
        .route("/admin/backups/status.json", get(backups::status))
        .route("/admin/backups/logs", get(backups::logs))
        .route("/admin/backups/logs.json", get(backups::logs))
        .route("/admin/backups/readonly", put(backups::readonly))
        .route("/admin/backups/readonly.json", put(backups::readonly))
        .route(
            "/admin/backups/upload",
            get(backups::check_backup_chunk)
                .post(backups::upload_backup_chunk)
                .layer(axum::extract::DefaultBodyLimit::max(
                    backups::CHUNK_BODY_LIMIT,
                )),
        )
        .route(
            "/admin/backups/upload.json",
            get(backups::check_backup_chunk)
                .post(backups::upload_backup_chunk)
                .layer(axum::extract::DefaultBodyLimit::max(
                    backups::CHUNK_BODY_LIMIT,
                )),
        )
        .route("/admin/backups/upload_url", get(backups::create_upload_url))
        .route(
            "/admin/backups/upload_url.json",
            get(backups::create_upload_url),
        )
        .route("/admin/backups/create-multipart", post(backups::multipart))
        .route(
            "/admin/backups/create-multipart.json",
            post(backups::multipart),
        )
        .route(
            "/admin/backups/complete-multipart",
            post(backups::multipart),
        )
        .route(
            "/admin/backups/complete-multipart.json",
            post(backups::multipart),
        )
        .route("/admin/backups/abort-multipart", post(backups::multipart))
        .route(
            "/admin/backups/abort-multipart.json",
            post(backups::multipart),
        )
        .route(
            "/admin/backups/batch-presign-multipart-parts",
            post(backups::multipart),
        )
        .route(
            "/admin/backups/batch-presign-multipart-parts.json",
            post(backups::multipart),
        )
        .route(
            "/admin/backups/{id}",
            get(backups::show)
                .put(backups::email)
                .delete(backups::destroy),
        )
        // Admin::ApiController
        .route("/admin/api", get(admin_api_keys::index))
        .route("/admin/api.json", get(admin_api_keys::index))
        .route(
            "/admin/api/keys",
            get(admin_api_keys::index).post(admin_api_keys::create),
        )
        .route(
            "/admin/api/keys.json",
            get(admin_api_keys::index).post(admin_api_keys::create),
        )
        .route("/admin/api/keys/scopes", get(admin_api_keys::scopes))
        .route("/admin/api/keys/scopes.json", get(admin_api_keys::scopes))
        .route(
            "/admin/api/keys/{id}",
            get(admin_api_keys::show)
                .put(admin_api_keys::update)
                .delete(admin_api_keys::destroy),
        )
        .route("/admin/api/keys/{id}/revoke", post(admin_api_keys::revoke))
        .route(
            "/admin/api/keys/{id}/revoke.json",
            post(admin_api_keys::revoke),
        )
        .route(
            "/admin/api/keys/{id}/undo-revoke",
            post(admin_api_keys::undo_revoke),
        )
        .route(
            "/admin/api/keys/{id}/undo-revoke.json",
            post(admin_api_keys::undo_revoke),
        )
        .route("/admin/email/handle_mail", post(admin_email::handle_mail))
        .route(
            "/admin/email/preview-digest",
            get(admin_email::preview_digest),
        )
        .route(
            "/admin/email/preview-digest.json",
            get(admin_email::preview_digest),
        )
        .route("/admin/email/send-digest", post(admin_email::send_digest))
        .route(
            "/admin/email/send-digest.json",
            post(admin_email::send_digest),
        )
        .route(
            "/admin/email/handle_mail.json",
            post(admin_email::handle_mail),
        )
        .route("/admin/email", get(admin_email::index))
        .route("/admin/email.json", get(admin_email::index))
        .route(
            "/admin/email/server-settings",
            get(admin_email::server_settings),
        )
        .route(
            "/admin/email/server-settings.json",
            get(admin_email::server_settings),
        )
        .route(
            "/admin/email/smtp_should_reject",
            get(admin_email::smtp_should_reject),
        )
        .route(
            "/admin/email/smtp_should_reject.json",
            get(admin_email::smtp_should_reject),
        )
        .route("/admin/email/test", post(admin_email::test))
        .route("/admin/email/test.json", post(admin_email::test))
        .route(
            "/admin/email/advanced-test",
            get(admin_email::advanced_test_get).post(admin_email::advanced_test),
        )
        .route(
            "/admin/email/advanced-test.json",
            get(admin_email::advanced_test_get).post(admin_email::advanced_test),
        )
        .route("/admin/email/templates", get(admin_email_templates::index))
        .route(
            "/admin/email/templates.json",
            get(admin_email_templates::index),
        )
        .route(
            "/admin/email/templates/{id}",
            get(admin_email_templates::show)
                .put(admin_email_templates::update)
                .delete(admin_email_templates::revert),
        )
        .route(
            "/admin/customize/email_style",
            get(admin_email_style::show).put(admin_email_style::update),
        )
        .route(
            "/admin/customize/email_style.json",
            get(admin_email_style::show).put(admin_email_style::update),
        )
        .route(
            "/admin/customize/email_style/{field}",
            get(admin_email_style::show_field),
        )
        .route("/session/hp", get(accounts::honeypot))
        .route("/session/hp.json", get(accounts::honeypot))
        .route("/u", get(directory::page).post(accounts::create_user))
        .route("/u.json", post(accounts::create_user))
        .route("/users", post(accounts::create_user))
        .route("/users.json", post(accounts::create_user))
        .route(
            "/u/activate-account/{token}",
            put(accounts::perform_account_activation),
        )
        .route(
            "/users/activate-account/{token}",
            put(accounts::perform_account_activation),
        )
        .route("/u/email-login", post(accounts::email_login))
        .route("/session/forgot_password", post(accounts::forgot_password))
        .route(
            "/session/forgot_password.json",
            post(accounts::forgot_password),
        )
        .route(
            "/session/password-reset-code/verify",
            post(accounts::redeem_password_reset_code),
        )
        .route(
            "/session/password-reset-code/verify.json",
            post(accounts::redeem_password_reset_code),
        )
        .route(
            "/u/password-reset/{token}",
            put(accounts::password_reset_update),
        )
        .route(
            "/users/password-reset/{token}",
            put(accounts::password_reset_update),
        )
        .route("/u/email-login.json", post(accounts::email_login))
        .route("/users/email-login", post(accounts::email_login))
        .route("/users/email-login.json", post(accounts::email_login))
        .route("/uploads.json", post(uploads::create))
        .route("/uploads/lookup-urls", post(uploads::lookup_urls))
        .route("/review", get(review::page))
        .route("/review.json", get(review::index))
        .route(
            "/review/{reviewable_id}",
            get(permalinks::show).delete(user_activity::destroy_reviewable),
        )
        .route(
            "/review/{reviewable_id}/perform/{action_id}",
            put(review::perform),
        )
        .route("/post_actions", post(post_actions::create))
        .route("/post_actions.json", post(post_actions::create))
        .route("/post_actions/{id}", delete(post_actions::destroy))
        .route(
            "/user_badges",
            get(badges::user_badges_index).post(user_badges::create),
        )
        .route(
            "/user_badges.json",
            get(badges::user_badges_index).post(user_badges::create),
        )
        .route("/badges", get(badges::index))
        .route("/badges.json", get(badges::index))
        .route("/badges/{id}", get(badges::show))
        .route("/badges/{id}/{slug}", get(badges::show_with_slug))
        .route("/user-badges/{username}", get(badges::username))
        .route("/user_badges/{id}", delete(user_badges::destroy))
        .route(
            "/user_badges/{id}/toggle_favorite",
            put(user_badges::toggle_favorite),
        )
        .route(
            "/user_badges/{id}/toggle_favorite.json",
            put(user_badges::toggle_favorite),
        )
        .route("/posts", post(posts::create))
        .route("/posts.json", post(posts::create))
        .route(
            "/posts/{id}",
            put(posts::update).delete(post_destroy::destroy_post),
        )
        .route(
            "/posts/by_number/{topic_id}/{post_number}",
            get(posts::by_number),
        )
        .route("/raw/{topic_id}/{post_number}", get(posts::raw))
        .route("/posts/{id}/deleted", get(deleted_posts::index))
        .route("/posts/{id}/deleted.json", get(deleted_posts::index))
        .route("/posts/{id}/pending", get(user_activity::pending))
        .route("/posts/{id}/pending.json", get(user_activity::pending))
        .route("/posts/{id}/recover", put(post_destroy::recover_post))
        .route("/posts/{id}/recover.json", put(post_destroy::recover_post))
        .route("/posts/{id}/revisions/{revision}", get(posts::revision))
        .route("/session", post(session::create))
        .route("/session.json", post(session::create))
        .route(
            "/email/unsubscribe/{key}",
            get(email::unsubscribe).post(email::perform_unsubscribe),
        )
        .route("/email/unsubscribed", get(email::unsubscribed))
        .route(
            "/session/2fa",
            get(session_2fa::show).post(session_2fa::perform),
        )
        .route(
            "/session/2fa.json",
            get(session_2fa::show).post(session_2fa::perform),
        )
        .route(
            "/u/confirm-admin/{token}",
            get(admin_users::confirm_admin).post(admin_users::confirm_admin),
        )
        .route(
            "/users/confirm-admin/{token}",
            get(admin_users::confirm_admin).post(admin_users::confirm_admin),
        )
        .route(
            "/auth/{provider}",
            get(omniauth::request).post(omniauth::request),
        )
        .route(
            "/auth/{provider}/callback",
            get(omniauth::callback).post(omniauth::callback),
        )
        .route(
            "/auth/discourse_id/revoke",
            post(omniauth::discourse_id_revoke),
        )
        .route(
            "/auth/{provider}/initiate",
            get(omniauth::initiate).post(omniauth::initiate),
        )
        .route(
            crate::plugins::lti::SUBMIT_ON_LOAD_PATH,
            get(omniauth::lti_submit_on_load_js),
        )
        .route(
            "/associate/{token}",
            get(omniauth::connect_info).post(omniauth::connect),
        )
        .route("/session/sso", get(discourse_connect::sso))
        .route("/session/sso_login", get(discourse_connect::sso_login))
        .route(
            "/session/sso_provider",
            get(discourse_connect::sso_provider),
        )
        .route("/session/csrf", get(session::csrf))
        .route("/session/csrf.json", get(session::csrf))
        .route("/session/current", get(session::current))
        .route("/session/current.json", get(session::current))
        .route("/session/{username}", delete(session::destroy))
        .route(
            "/login",
            get(login_required::show_login).post(session::enter),
        )
        .route("/signup", get(login_required::show_signup))
        .route("/search", cached!(get(search::show)))
        .route("/search.json", cached!(get(search::show_json)))
        .route("/search/query", cached!(get(search::query)))
        .route("/search/query.json", cached!(get(search::query)))
        .route("/search/click", post(search::click))
        .route(
            "/tag/{*path}",
            cached!(get(tags::show))
                .put(tag_admin::put)
                .delete(tag_admin::delete)
                .post(tag_admin::post),
        )
        .route("/tags/list", get(tag_admin::list))
        .route("/tags/list.json", get(tag_admin::list))
        .route(
            "/tags/personal_messages/{username}",
            get(tag_admin::personal_messages),
        )
        .route("/tags/upload", post(tag_admin::upload))
        .route("/tags/upload.json", post(tag_admin::upload))
        .route("/tags/bulk_create", post(tag_admin::bulk_create))
        .route("/tags/bulk_create.json", post(tag_admin::bulk_create))
        .route(
            "/tags/unused",
            get(tag_admin::unused).delete(tag_admin::destroy_unused),
        )
        .route(
            "/tags/unused.json",
            get(tag_admin::unused).delete(tag_admin::destroy_unused),
        )
        .route("/tags", get(tags::index))
        .route("/tags.json", get(tags::index_json))
        .route("/tags/c/{*path}", cached!(get(tags::show_in_category)))
        .route(
            "/tag_groups",
            get(tag_groups::index).post(tag_groups::create),
        )
        .route(
            "/tag_groups.json",
            get(tag_groups::index).post(tag_groups::create),
        )
        .route("/tag_groups/filter/search", get(tag_groups::search))
        .route("/tag_groups/filter/search.json", get(tag_groups::search))
        .route(
            "/tag_groups/{id}",
            get(tag_groups::show)
                .put(tag_groups::update)
                .delete(tag_groups::destroy),
        )
        .route("/tags/filter/search", get(tag_search::search))
        .route("/tags/filter/search.json", get(tag_search::search))
        .route(
            "/categories",
            cached!(get(list::categories)).post(categories::create),
        )
        .route(
            "/categories.json",
            cached!(get(list::categories_json)).post(categories::create),
        )
        .route("/emojis.json", get(emojis::index))
        .route("/assets/emoji-data.json", get(emojis::client_data))
        .route("/emojis/search-aliases", get(emojis::search_aliases))
        .route("/emojis/search-aliases.json", get(emojis::search_aliases))
        .route("/site", get(site::site))
        .route("/site.json", get(site::site))
        .route("/site/basic-info", get(site::basic_info))
        .route("/site/basic-info.json", get(site::basic_info))
        .route("/about", get(about::page))
        .route("/about.json", get(about::show))
        .route("/groups", get(groups::index_page))
        .route("/groups.json", get(groups::index))
        .route("/g", get(groups::index_page))
        .route("/g.json", get(groups::index))
        .route("/groups/search", get(groups::search))
        .route("/groups/search.json", get(groups::search))
        .route("/groups/check-name", get(groups::check_name))
        .route("/groups/check-name.json", get(groups::check_name))
        .route("/groups/by-id/{id}", get(groups::show_by_id))
        .route("/g/by-id/{id}", get(groups::show_by_id))
        .route(
            "/groups/{name}",
            get(groups::show).put(admin_groups::update),
        )
        .route("/g/{name}", get(groups::show).put(admin_groups::update))
        .route("/groups/{name}/owners", put(admin_groups::add_owners))
        .route("/groups/{name}/owners.json", put(admin_groups::add_owners))
        .route("/g/{name}/owners", put(admin_groups::add_owners))
        .route("/g/{name}/owners.json", put(admin_groups::add_owners))
        .route("/groups/{name}/join", put(group_membership::join))
        .route("/groups/{name}/leave", delete(group_membership::leave))
        .route(
            "/groups/{name}/handle_membership_request",
            put(group_membership::handle_membership_request),
        )
        .route(
            "/groups/{name}/request_membership",
            post(group_membership::request_membership),
        )
        .route(
            "/groups/{name}/notifications",
            post(group_membership::set_notifications),
        )
        .route("/groups/{name}/join.json", put(group_membership::join))
        .route("/groups/{name}/leave.json", delete(group_membership::leave))
        .route(
            "/groups/{name}/handle_membership_request.json",
            put(group_membership::handle_membership_request),
        )
        .route(
            "/groups/{name}/request_membership.json",
            post(group_membership::request_membership),
        )
        .route(
            "/groups/{name}/notifications.json",
            post(group_membership::set_notifications),
        )
        .route("/g/{name}/join", put(group_membership::join))
        .route("/g/{name}/leave", delete(group_membership::leave))
        .route(
            "/g/{name}/handle_membership_request",
            put(group_membership::handle_membership_request),
        )
        .route(
            "/g/{name}/request_membership",
            post(group_membership::request_membership),
        )
        .route(
            "/g/{name}/notifications",
            post(group_membership::set_notifications),
        )
        .route("/g/{name}/join.json", put(group_membership::join))
        .route("/g/{name}/leave.json", delete(group_membership::leave))
        .route(
            "/g/{name}/handle_membership_request.json",
            put(group_membership::handle_membership_request),
        )
        .route(
            "/g/{name}/request_membership.json",
            post(group_membership::request_membership),
        )
        .route(
            "/g/{name}/notifications.json",
            post(group_membership::set_notifications),
        )
        .route(
            "/admin/groups",
            get(admin_pages::groups_index).post(admin_groups::create),
        )
        .route(
            "/admin/groups.json",
            get(admin_pages::groups_index).post(admin_groups::create),
        )
        .route("/admin/groups/settings", get(admin_pages::groups_settings))
        .route(
            "/admin/groups/settings.json",
            get(admin_pages::groups_settings),
        )
        .route(
            "/admin/groups/automatic_membership_count",
            put(admin_groups::automatic_membership_count),
        )
        .route(
            "/admin/groups/automatic_membership_count.json",
            put(admin_groups::automatic_membership_count),
        )
        .route("/admin/groups/{id}", delete(admin_groups::destroy))
        .route(
            "/admin/groups/{id}/owners",
            delete(admin_groups::remove_owner),
        )
        .route(
            "/admin/groups/{id}/owners.json",
            delete(admin_groups::remove_owner),
        )
        .route("/admin/groups/{id}/primary", put(admin_groups::set_primary))
        .route(
            "/admin/groups/{id}/primary.json",
            put(admin_groups::set_primary),
        )
        .route(
            "/admin/groups/{id}/members",
            put(admin_groups::add_members).delete(admin_groups::remove_member),
        )
        .route(
            "/admin/groups/{id}/members.json",
            put(admin_groups::add_members).delete(admin_groups::remove_member),
        )
        .route("/groups/{name}/{*tab}", get(groups::show_tab))
        .route("/g/{name}/{*tab}", get(groups::show_tab))
        .route(
            "/groups/{name}/members",
            get(groups::members)
                .put(admin_groups::add_members)
                .delete(admin_groups::remove_member),
        )
        .route(
            "/groups/{name}/members.json",
            get(groups::members)
                .put(admin_groups::add_members)
                .delete(admin_groups::remove_member),
        )
        .route(
            "/g/{name}/members",
            get(groups::members)
                .put(admin_groups::add_members)
                .delete(admin_groups::remove_member),
        )
        .route(
            "/g/{name}/members.json",
            get(groups::members)
                .put(admin_groups::add_members)
                .delete(admin_groups::remove_member),
        )
        .route("/groups/{name}/logs", get(groups::histories))
        .route("/groups/{name}/logs.json", get(groups::histories))
        .route("/g/{name}/logs", get(groups::histories))
        .route("/g/{name}/logs.json", get(groups::histories))
        .route("/groups/{name}/mentionable", get(groups::mentionable))
        .route("/groups/{name}/mentionable.json", get(groups::mentionable))
        .route("/g/{name}/mentionable", get(groups::mentionable))
        .route("/g/{name}/mentionable.json", get(groups::mentionable))
        .route("/groups/{name}/messageable", get(groups::messageable_route))
        .route(
            "/groups/{name}/messageable.json",
            get(groups::messageable_route),
        )
        .route("/g/{name}/messageable", get(groups::messageable_route))
        .route("/g/{name}/messageable.json", get(groups::messageable_route))
        .route("/groups/{name}/permissions", get(groups::permissions))
        .route("/groups/{name}/permissions.json", get(groups::permissions))
        .route("/g/{name}/permissions", get(groups::permissions))
        .route("/g/{name}/permissions.json", get(groups::permissions))
        .route("/directory_items", cached!(get(directory::index)))
        .route("/directory_items.json", cached!(get(directory::index)))
        .route("/directory-columns", get(directory_columns::index))
        .route("/directory-columns.json", get(directory_columns::index))
        .route(
            "/edit-directory-columns",
            get(directory_columns::edit_index).put(directory_columns::edit_update),
        )
        .route(
            "/edit-directory-columns.json",
            get(directory_columns::edit_index).put(directory_columns::edit_update),
        )
        .route("/about/live_post_counts", get(about::live_post_counts))
        .route("/about/live_post_counts.json", get(about::live_post_counts))
        .route("/assets/site.css", get(asset))
        .route("/assets/discourse.css", get(asset))
        .route(
            "/assets/color_definitions_light.css",
            get(stylesheets::light),
        )
        .route("/assets/color_definitions_dark.css", get(stylesheets::dark))
        .route("/fonts/{file}", get(stylesheets::font))
        .route(
            "/letter_avatar_proxy/{version}/letter/{letter}/{color}/{size}",
            get(user_avatars::show_proxy_letter),
        )
        .route(
            "/letter_avatar/{username}/{size}/{version}",
            get(user_avatars::show_letter),
        )
        .route(
            "/user_avatar/{hostname}/{username}/{size}/{version}",
            get(user_avatars::show),
        )
        .route(
            "/user_avatar/{username}/refresh_gravatar",
            post(user_avatar_writes::refresh_gravatar),
        )
        .route(
            "/user_avatar/{username}/refresh_gravatar.json",
            post(user_avatar_writes::refresh_gravatar),
        )
        .route(
            "/assets/moment-locale/{name}",
            get(crate::moment_locale::serve),
        )
        .route("/assets/htmx.min.js", get(asset))
        .route("/assets/htmx-ext-sse.js", get(asset))
        .route("/assets/page.js", get(asset))
        .route("/assets/auth-complete.js", get(asset))
        .route("/assets/category-edit.js", get(asset))
        .route("/assets/hashtags.js", get(asset))
        .route("/assets/tag-info.js", get(asset))
        .route("/assets/oneboxes.js", get(asset))
        .route("/assets/cooked.js", get(asset))
        .route("/assets/lightbox.js", get(asset))
        .route("/assets/photoswipe-lightbox.esm.min.js", get(asset))
        .route("/assets/photoswipe.esm.min.js", get(asset))
        .route("/assets/highlight-syntax.js", get(asset))
        .route("/assets/highlightjs-core.min.js", get(asset))
        .route("/highlight-js/{hostname}/{file}", get(highlight_js::show))
        .route("/svg-sprite/{hostname}/{file}", get(svg_sprite::show))
        .route("/assets/modal.js", get(asset))
        .route("/assets/spoiler-alert.js", get(asset))
        .route("/assets/post-decorations.js", get(asset))
        .route("/assets/codeblock-buttons.js", get(asset))
        .route("/assets/quote-controls.js", get(asset))
        .route("/assets/floating-ui.core.umd.min.js", get(asset))
        .route("/assets/floating-ui.dom.umd.min.js", get(asset))
        .route("/assets/float.js", get(asset))
        .route("/assets/user-card.js", get(asset))
        .route("/assets/inline-footnotes.js", get(asset))
        .route("/assets/local-dates.js", get(asset))
        .route("/assets/local-dates-create.js", get(asset))
        .route("/assets/pikaday.js", get(asset))
        .route("/assets/moment.min.js", get(asset))
        .route(
            "/assets/moment-timezone-with-data-10-year-range.min.js",
            get(asset),
        )
        .route("/assets/sidebar.js", get(asset))
        .route("/assets/invite.js", get(asset))
        .route("/assets/user-invited.js", get(asset))
        .route("/assets/admin.css", get(asset))
        .route("/assets/admin.js", get(asset))
        .route("/assets/admin-dashboard.js", get(asset))
        .route("/assets/admin-users.js", get(asset))
        .route("/assets/admin-reports.js", get(asset))
        .route("/assets/admin-logs.js", get(asset))
        .route("/assets/admin-report-charts.js", get(asset))
        .route("/assets/admin-report.js", get(asset))
        .route("/assets/admin-user.js", get(asset))
        .route("/assets/admin-ui.js", get(asset))
        .route("/assets/admin-backups.js", get(asset))
        .route("/assets/admin-api-keys.js", get(asset))
        .route("/assets/admin-web-hooks.js", get(asset))
        .route("/assets/admin-email.js", get(asset))
        .route("/assets/site-settings.js", get(asset))
        .route("/assets/dialog.js", get(asset))
        .route("/assets/chart.umd.min.js", get(asset))
        .route("/assets/chartjs-adapter-moment.min.js", get(asset))
        .route("/assets/navigation-bar.js", get(asset))
        .route("/assets/topic.js", get(asset))
        .route("/assets/topic-voting.js", get(asset))
        .route("/assets/discourse-reactions.js", get(asset))
        .route("/assets/discourse-solved.js", get(asset))
        .route("/assets/chat.js", get(asset))
        .route("/assets/emoji-picker.js", get(asset))
        .route("/assets/autocomplete.js", get(asset))
        .route("/assets/presence.js", get(asset))
        .route("/assets/topic-presence.js", get(asset))
        .route("/assets/tracking-menu.js", get(asset))
        .route("/assets/messages-dropdown.js", get(asset))
        .route("/assets/users-directory.js", get(asset))
        .route("/assets/user-menu.js", get(asset))
        .route("/assets/user-notifications.js", get(asset))
        .route("/assets/preferences.js", get(asset))
        .route("/assets/prefs-choosers.js", get(asset))
        .route("/assets/prefs-security.js", get(asset))
        .route("/assets/notifications-page.js", get(asset))
        .route("/assets/user-activity.js", get(asset))
        .route("/assets/overflow-controls.js", get(asset))
        .route("/assets/groups.js", get(asset))
        .route("/assets/search-page.js", get(asset))
        .route("/assets/search-advanced.js", get(asset))
        .route("/assets/search-menu.js", get(asset))
        .route("/assets/composer.js", get(asset))
        .route("/assets/screen-track.js", get(asset))
        .route("/assets/js/onpopstate-handler.js", get(asset))
        .route("/assets/markdown.wasm", get(composer::markdown_wasm))
        .route(
            "/assets/markdown-settings.json",
            get(composer::markdown_settings),
        )
        // The local store's files (FileStore::Local): what nginx serves in a
        // Rails deployment. A remote store's urls point elsewhere.
        .nest_service("/uploads", ServeDir::new(public.join("uploads")));
    if let Some(emoji) = config.emoji_dir() {
        router = router.nest_service("/images/emoji", ServeDir::new(emoji));
    }
    // The session confirmation and second factor routes under both user
    // roots, each with its .json twin.
    for root in ["/u", "/users"] {
        // users#show by external id: `.json` stays on the id (the route's
        // `[^/]+` takes it) and asks for JSON.
        router = router
            .route(
                &format!("{root}/by-external/{{external_id}}"),
                get(users::show_by_external),
            )
            .route(
                &format!("{root}/by-external/{{external_provider}}/{{external_id}}"),
                get(users::show_by_external),
            );
        for ext in ["", ".json"] {
            router = router
                .route(
                    &format!("{root}/trusted-session{ext}"),
                    get(user_security::trusted_session),
                )
                .route(
                    &format!("{root}/confirm-session{ext}"),
                    post(user_security::confirm_session),
                )
                .route(
                    &format!("{root}/second_factors{ext}"),
                    post(user_security::list_second_factors),
                );
            for (path, put_route) in [
                ("second_factor", true),
                ("security_key", true),
                ("disable_second_factor", true),
                ("second_factors_backup", true),
                ("create_second_factor_security_key", false),
                ("register_second_factor_security_key", false),
                ("create_second_factor_totp", false),
                ("enable_second_factor_totp", false),
                ("create_passkey", false),
                ("register_passkey", false),
            ] {
                let path = format!("{root}/{path}{ext}");
                router = if put_route {
                    router.route(&path, put(user_security::second_factor_write))
                } else {
                    router.route(&path, post(user_security::second_factor_write))
                };
            }
        }
        router = router
            .route(
                &format!("{root}/rename_passkey/{{id}}"),
                put(user_security::second_factor_write),
            )
            .route(
                &format!("{root}/delete_passkey/{{id}}"),
                delete(user_security::second_factor_write),
            );
    }
    for path in ["/calendar-subscriptions", "/calendar-subscriptions.json"] {
        router = router.route(
            path,
            get(user_security::calendar_subscriptions)
                .post(user_security::calendar_subscriptions)
                .delete(user_security::calendar_subscriptions),
        );
    }
    // The push notifications' icons (app/assets/images/push-notifications),
    // undigested.
    if let Some(src) = &config.discourse_src {
        router = router.nest_service(
            "/assets/push-notifications",
            ServeDir::new(src.join("app/assets/images/push-notifications")),
        );
    }
    router
        .nest_service("/images", images)
        // redirect_to_profile_if_required, after the login checks.
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            required_fields::gate,
        ))
        // Only for routed requests, as Rails' before_action only runs once a
        // route matched: an unknown path (a browser's /favicon.ico) is a 404,
        // not a redirect to the login page that overwrites destination_url.
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            login_required::gate,
        ))
        // block_if_readonly_mode runs before the login checks.
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::readonly::gate,
        ))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            not_found::html_errors,
        ))
        // PermalinkConstraint: the `*url` route without a permalink is no
        // route at all, before any filter.
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            permalinks::constraint,
        ))
}

/// `Rack::MethodOverride`: a POST with a form `_method` (or the
/// `X-HTTP-Method-Override` header) of delete/put/patch is handled as
/// that method, which is how HTML forms log out.
pub async fn method_override(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::body::Body;
    use axum::http::{Method, header};
    if request.method() != Method::POST {
        return next.run(request).await;
    }
    let (mut parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, 1 << 20).await {
        Ok(b) => b,
        Err(_) => {
            return axum::response::IntoResponse::into_response(
                axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            );
        }
    };
    let form = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"));
    let overridden = parts
        .headers
        .get("x-http-method-override")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            if !form {
                return None;
            }
            form_urlencoded::parse(&bytes)
                .find(|(k, _)| k == "_method")
                .map(|(_, v)| v.into_owned())
        });
    if let Some(m) = overridden
        && let Ok(method) = Method::from_bytes(m.to_ascii_uppercase().as_bytes())
        && matches!(method, Method::DELETE | Method::PUT | Method::PATCH)
    {
        parts.method = method;
    }
    next.run(axum::extract::Request::from_parts(parts, Body::from(bytes)))
        .await
}

/// `request.format` for a path without an extension, as ActionDispatch
/// decides it: `?format=json`, or a valid Accept header (an XHR's, or one
/// that is not browser-like: no `*/*` beside other types) whose preferred
/// type is JSON. The port tells JSON by the `.json` suffix, so such a GET
/// goes to the path's `.json` twin, for the pages whose controllers then
/// answer JSON (/search, /login and the /my redirects stay HTML; so does
/// the front page, whose JSON is the homepage list's, not ported).
pub async fn request_format(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::Method;
    if (request.method() == Method::GET || request.method() == Method::HEAD)
        && json_twin_page(request.uri().path())
        && wants_json(request.uri().query(), request.headers())
    {
        let uri = request.uri().clone();
        request.extensions_mut().insert(RequestedUri(uri.clone()));
        let rewritten = match uri.query() {
            Some(q) => format!("{}.json?{q}", uri.path()),
            None => format!("{}.json", uri.path()),
        };
        if let Ok(uri) = rewritten.parse() {
            *request.uri_mut() = uri;
        }
    }
    next.run(request).await
}

/// The URI as requested, before request_format sent it to its `.json`
/// twin (Rails' `request.fullpath` has no format added).
#[derive(Clone)]
pub struct RequestedUri(pub axum::http::Uri);

/// `request.fullpath`: the path and query as requested.
pub(crate) fn fullpath_of(requested: Option<&RequestedUri>, uri: &axum::http::Uri) -> String {
    let uri = requested.map(|r| &r.0).unwrap_or(uri);
    uri.path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default()
}

/// The pages Rails answers as JSON when asked to: the topic lists, the
/// categories and tags, topics, user pages, drafts, notifications, the
/// review queue and the about page.
fn json_twin_page(path: &str) -> bool {
    let Some(rest) = path.strip_prefix('/') else {
        return false;
    };
    let first = rest.split('/').next().unwrap_or("");
    let last = rest.rsplit('/').next().unwrap_or("");
    !rest.is_empty()
        && !last.contains('.')
        && matches!(
            first,
            "latest"
                | "top"
                | "hot"
                | "new"
                | "unread"
                | "unseen"
                | "read"
                | "posted"
                | "bookmarks"
                | "categories"
                | "c"
                | "tags"
                | "tag"
                | "t"
                | "u"
                | "drafts"
                | "notifications"
                | "review"
                | "about"
        )
}

/// ActionDispatch's `formats`: the format param first, then the Accept
/// header when valid (`valid_accept_header`), its types by q, the first
/// kept on ties.
pub(super) fn wants_json(query: Option<&str>, headers: &axum::http::HeaderMap) -> bool {
    if let Some(format) = query
        .into_iter()
        .flat_map(|q| q.split('&'))
        .find_map(|p| p.strip_prefix("format="))
    {
        return format == "json";
    }
    let Some(accept) = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .filter(|a| !a.trim().is_empty())
    else {
        return false;
    };
    let xhr = headers
        .get("x-requested-with")
        .is_some_and(|v| v == "XMLHttpRequest");
    // BROWSER_LIKE_ACCEPTS: `*/*` alongside other types.
    static BROWSER_LIKE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let browser_like = BROWSER_LIKE
        .get_or_init(|| regex::Regex::new(r",\s*\*/\*|\*/\*\s*,").unwrap())
        .is_match(accept);
    if !xhr && browser_like {
        return false;
    }
    let mut types: Vec<(f32, &str)> = accept
        .split(',')
        .map(|entry| {
            let mut parts = entry.split(';');
            let mime = parts.next().unwrap_or("").trim();
            let q = parts
                .filter_map(|p| p.trim().strip_prefix("q="))
                .find_map(|q| q.parse::<f32>().ok())
                .unwrap_or(1.0);
            (q, mime)
        })
        .collect();
    types.sort_by(|a, b| b.0.total_cmp(&a.0));
    types
        .first()
        .is_some_and(|(_, mime)| *mime == "application/json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, header};

    fn headers(accept: &str, xhr: bool) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::ACCEPT, HeaderValue::from_str(accept).unwrap());
        if xhr {
            h.insert(
                "x-requested-with",
                HeaderValue::from_static("XMLHttpRequest"),
            );
        }
        h
    }

    #[test]
    fn json_from_accept_as_actiondispatch_decides() {
        // As the reference answered /latest for each.
        let ember = "application/json, text/javascript, */*; q=0.01";
        assert!(wants_json(None, &headers(ember, true)));
        assert!(wants_json(None, &headers("application/json", false)));
        // Browser-like (*/* beside other types) and not an XHR: HTML.
        assert!(!wants_json(None, &headers("application/json, */*", false)));
        assert!(!wants_json(
            None,
            &headers("text/html,application/xhtml+xml,*/*;q=0.8", false)
        ));
        // q orders the types.
        assert!(!wants_json(
            None,
            &headers("application/json;q=0.5, text/html", false)
        ));
        // The format param wins over the header.
        assert!(wants_json(
            Some("page=2&format=json"),
            &headers("text/html", false)
        ));
        assert!(!wants_json(
            Some("format=html"),
            &headers("application/json", false)
        ));
        assert!(!wants_json(None, &HeaderMap::new()));
    }

    #[test]
    fn json_twins_are_the_pages_rails_answers_as_json() {
        for path in [
            "/latest",
            "/c/general/4/l/top",
            "/t/welcome/5",
            "/u/user1/summary",
            "/tag/guide",
        ] {
            assert!(json_twin_page(path), "{path}");
        }
        for path in [
            "/",
            "/search",
            "/login",
            "/my/messages",
            "/latest.json",
            "/uploads/x.png",
        ] {
            assert!(!json_twin_page(path), "{path}");
        }
    }
}
