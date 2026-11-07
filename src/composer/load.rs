//! Load the member's composer with its categories, permissions and settings.

use std::collections::HashMap;

use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::topic_list_view::ListCategory;
use crate::{AppError, AppState};

pub(crate) struct Options {
    pub composition_mode: i32,
    pub hide_presence: bool,
}

pub(crate) struct Loaded {
    pub html: String,
    pub categories: HashMap<i64, ListCategory>,
}

pub(crate) async fn load(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    guardian: &Guardian,
    base_path: &str,
    options: Option<&Options>,
    can_create_topic: bool,
) -> Result<Loaded, AppError> {
    let mut allowed = guardian
        .topic_create_allowed_category_ids(&mut *conn, settings)
        .await?;
    // The chooser leaves out uncategorized unless topics may go there.
    if !settings.get("allow_uncategorized_topics")?.truthy() {
        let uncategorized = settings.get("uncategorized_category_id")?.to_i() as i32;
        allowed.retain(|id| *id != uncategorized);
    }
    // Category#minimumRequiredTags: with required tag groups, the
    // larger of their counts' sum and the minimum; else the minimum.
    let rows: Vec<(i32, i32, Option<String>, i64)> = sqlx::query_as(
        "SELECT c.id, c.topic_count, c.description, \
           CASE WHEN EXISTS (SELECT 1 FROM category_required_tag_groups r WHERE r.category_id = c.id) \
             THEN GREATEST((SELECT COALESCE(SUM(r.min_count), 0) FROM category_required_tag_groups r \
                            WHERE r.category_id = c.id), COALESCE(c.minimum_required_tags, 0)) \
             ELSE GREATEST(COALESCE(c.minimum_required_tags, 0), 0) END::bigint \
         FROM categories c WHERE c.id = ANY($1)",
    )
    .bind(&allowed)
    .fetch_all(&mut *conn)
    .await?;
    let mut chooser = Vec::with_capacity(rows.len());
    for (id, topic_count, description, minimum_required_tags) in rows {
        chooser.push(super::view::ChooserCategory {
            id: i64::from(id),
            topic_count: i64::from(topic_count),
            // Category#description_text
            description_text: crate::categories::description_plain_text(description.as_deref())?,
            minimum_required_tags,
        });
    }
    // composer.canEditTags for a new topic: site.can_tag_topics.
    let tag_chooser = if guardian.can_tag_topics(settings)? {
        Some(super::view::TagChooser {
            can_create: guardian.can_create_tag(settings)?,
            staff: guardian.is_staff(),
            max_tags_per_topic: settings.get("max_tags_per_topic")?.to_i(),
            max_tag_search_results: settings.get("max_tag_search_results")?.to_i(),
            max_tag_length: settings.get("max_tag_length")?.to_i(),
            force_lowercase: settings.get("force_lowercase_tags")?.truthy(),
            sort_alphabetically: settings.get("tags_sort_alphabetically")?.truthy(),
            prioritize_recent: settings.get("prioritize_recently_used_tags")?.truthy(),
            style: settings.get("tag_style")?.to_s(),
        })
    } else {
        None
    };
    let categories = crate::topic_list_view::categories(&mut *conn).await?;
    let list = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path,
        now: chrono::Utc::now(),
        categories: &categories,
        expand_all_pinned: false,
        hide_category: false,
        member_trust_level: guardian.user().map(|u| u.trust_level),
        settings: crate::topic_list_view::ListSettings::load(settings)?,
    };
    let default_category =
        Some(settings.get("default_composer_category")?.to_i()).filter(|id| *id > 0);
    let wasm = format!(
        "{}/assets/markdown.wasm?v={}",
        base_path,
        super::preview::MARKDOWN_WASM_VERSION
    );
    let render_settings = format!("{}/assets/markdown-settings.json", base_path);
    // allowPreview: not for the rich editor (composition_mode 1).
    let (allow_preview, hide_presence) = match guardian.user_id() {
        Some(_) => (
            options.map(|o| o.composition_mode) != Some(1),
            options.is_some_and(|o| o.hide_presence),
        ),
        None => (false, false),
    };
    // discourse-presence: what the composer and the topic's footer show
    // of who else is replying or editing.
    let presence = if crate::plugins::presence::enabled(settings)? {
        let js = |key: &str| {
            state
                .i18n
                .t(&format!("js.presence.{key}"))
                .unwrap_or(key)
                .to_string()
        };
        Some(serde_json::json!({
            "replying_one": js("replying.one"),
            "replying_other": js("replying.other"),
            "editing_one": js("editing.one"),
            "editing_other": js("editing.other"),
            "replying_to_topic_one": js("replying_to_topic.one"),
            "replying_to_topic_other": js("replying_to_topic.other"),
            "max_users_shown": settings.get("presence_max_users_shown")?.to_i(),
            "whisperer": guardian.is_whisperer(settings)?,
            "hide_presence": hide_presence,
            "user_id": guardian.user_id(),
            "profile_possessive": state
                .i18n
                .t("js.user.profile_possessive")
                .unwrap_or("%{username}'s profile"),
        }))
    } else {
        None
    };
    // The toolbar's options menu: add-poll-ui-builder's currentUser
    // half of its condition (staff or can_create_poll); a PM with a
    // non-human user shows the item to anyone (composer.js).
    let can_create_poll = guardian.is_staff()
        || guardian.in_setting_groups(settings, "poll_create_allowed_groups")?;
    let can_create_policy = guardian.in_setting_groups(settings, "create_policy_allowed_groups")?;
    let mut popup_options =
        super::view::popup_options(settings, can_create_poll, can_create_policy)?;
    // discourse-templates' item, for a member who can_use_templates.
    if crate::plugins::templates::enabled(settings)?
        && crate::plugins::templates::can_use_templates(&mut *conn, settings, guardian).await?
    {
        crate::plugins::templates::view::add_popup_option(&mut popup_options);
    }
    let unported_popup = super::view::unported_popup_plugins(settings)?;
    // discourse-post-voting's category defaults, for the categories the
    // chooser lists.
    let post_voting = if crate::plugins::post_voting::enabled(settings)? {
        let fields = crate::plugins::post_voting::category_fields(&mut *conn, &allowed).await?;
        let on = |id: i32, name: &str| {
            fields
                .get(&(id, name.to_string()))
                .and_then(|v| crate::ruby::cast_boolean(v.as_deref()))
                == Some(true)
        };
        let pick = |name: &str| -> Vec<i64> {
            allowed
                .iter()
                .filter(|id| on(**id, name))
                .map(|id| i64::from(*id))
                .collect()
        };
        Some((
            pick(crate::plugins::post_voting::CREATE_AS_POST_VOTING_DEFAULT),
            pick(crate::plugins::post_voting::ONLY_POST_VOTING_IN_THIS_CATEGORY),
        ))
    } else {
        None
    };
    let actions = super::view::ActionsUi {
        can_create_topic,
        can_send_private_messages: guardian.can_send_private_messages(settings)?,
        staff: guardian.is_staff(),
        can_skip_bump: guardian.can_skip_bump(),
        whisperer: guardian.is_whisperer(settings)?,
        // SiteSerializer#shared_drafts_category_id
        shared_drafts: guardian.can_see_shared_draft(settings)?
            && settings.get("shared_drafts_category")?.presence().is_some(),
        prioritize_username: settings.get("prioritize_username_in_ux")?.truthy(),
        post_voting,
    };
    // discourse-poll's builder, behind its options menu item.
    let poll_builder = if popup_options.iter().any(|o| o.name == "poll") {
        let visible = crate::groups::visible_groups_where(guardian, "groups");
        let groups: Vec<String> = sqlx::query_scalar(&format!(
            "SELECT name FROM groups WHERE id > 0 AND {visible} ORDER BY name ASC"
        ))
        .fetch_all(&mut *conn)
        .await?;
        Some(super::view::PollBuilderUi {
            src: crate::assets::url(base_path, "poll-builder.js"),
            max_options: settings.get("poll_maximum_options")?.to_i(),
            default_public: settings.get("poll_default_public")?.truthy(),
            staff: guardian.is_staff(),
            groups,
            local_dates: crate::plugins::local_dates::enabled(settings)?,
            moment_src: crate::assets::url(base_path, "moment.min.js"),
            pikaday_src: crate::assets::url(base_path, "pikaday.js"),
        })
    } else {
        None
    };
    let html = super::view::render(
        &list,
        &chooser,
        default_category,
        Some((
            &wasm,
            &render_settings,
            settings.get("max_oneboxes_per_post")?.to_i(),
        )),
        allow_preview,
        options.map(|o| o.composition_mode) == Some(1),
        presence.as_ref(),
        super::view::UploadUi::for_user(
            &settings.get("authorized_extensions")?.to_s(),
            &settings.get("authorized_extensions_for_staff")?.to_s(),
            guardian.is_staff(),
        )
        .as_ref(),
        Some((&popup_options, &unported_popup)),
        tag_chooser.as_ref(),
        &super::view::RouteData {
            max_title_length: settings.get("max_topic_title_length")?.to_i(),
            dialog_src: crate::assets::url(base_path, "dialog.js"),
            dialog_strings: crate::html::dialog_strings(&state.i18n),
            pm: super::view::PmUi {
                username: guardian
                    .user()
                    .map(|u| u.username.clone())
                    .unwrap_or_default(),
                staff: guardian.is_staff(),
                // can_send_private_email_messages
                allow_emails: settings.get("enable_staged_users")?.truthy()
                    && guardian.in_setting_groups(settings, "personal_message_enabled_groups")?
                    && guardian
                        .in_setting_groups(settings, "send_email_messages_allowed_groups")?,
                // site.can_tag_pms
                can_tag_pms: guardian.can_tag_pms(settings)?,
            },
            actions,
            poll_builder,
        },
    );
    Ok(Loaded { html, categories })
}
