//! discourse-narrative-bot: discobot (user -2) and its two tutorials, the
//! new user and the advanced user narratives (`narrative`), state machines
//! driven by what the user does in their message with discobot; the
//! `@discobot` commands (`track_selector`); the welcome post on signup;
//! the certificates the tutorials end with (`certificate`); and the jobs
//! carrying it all (`jobs`). Each user's state is a PluginStore hash
//! (`store`).
//!
//! Core reaches the plugin through the event functions here, each a no-op
//! while the plugin is off (`plugin.on` and `add_model_callback` check
//! `enabled?`): they enqueue the `bot_input` job, as plugin.rb's handlers
//! do, and the job does the rest.

pub mod bot;
pub mod certificate;
pub mod jobs;
pub mod narrative;
pub mod store;
pub mod track_selector;

use serde_json::json;
use sqlx::PgConnection;

use crate::AppError;
use crate::site_settings::{SettingError, SiteSettings};

/// `DiscourseNarrativeBot::PLUGIN_NAME`, also its PluginStore name.
pub const PLUGIN_NAME: &str = "discourse-narrative-bot";
/// `DiscourseNarrativeBot::BOT_USER_ID`
pub const BOT_USER_ID: i32 = -2;
/// `PostActionType.types[:like]` and `[:inappropriate]`.
const LIKE: i32 = 2;
const INAPPROPRIATE: i32 = 4;

/// `enabled_site_setting :discourse_narrative_bot_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("discourse_narrative_bot_enabled")?.truthy())
}

/// A `type: list` setting's entries.
fn list(settings: &SiteSettings, name: &str) -> Result<Vec<String>, SettingError> {
    Ok(settings
        .get(name)?
        .to_s()
        .split('|')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// What `enqueue_narrative_bot_job?` reads besides the settings.
struct Candidate {
    id: i32,
    username: String,
    staged: bool,
    trust_level: i32,
    anonymous_shadow: bool,
}

async fn candidate(conn: &mut PgConnection, user_id: i32) -> Result<Option<Candidate>, AppError> {
    let row: Option<(i32, String, bool, i32, bool)> = sqlx::query_as(
        "SELECT u.id, u.username, u.staged, u.trust_level, \
                EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = u.id) \
         FROM users u WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(
        |(id, username, staged, trust_level, anonymous_shadow)| Candidate {
            id,
            username,
            staged,
            trust_level,
            anonymous_shadow,
        },
    ))
}

/// `User#enqueue_narrative_bot_job?`: the plugin on, and a human who is
/// neither anonymous, staged nor among the ignored usernames.
fn allowed(
    enabled: bool,
    allow_anonymous_mode: bool,
    ignored: &[String],
    user: &Candidate,
) -> bool {
    let anonymous = allow_anonymous_mode && user.trust_level >= 1 && user.anonymous_shadow;
    enabled && user.id > 0 && !anonymous && !user.staged && !ignored.contains(&user.username)
}

/// `enqueue_narrative_bot_job?` for a user id: false when there is no
/// such user (`post.user&.`).
pub async fn enqueue_job_allowed(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: Option<i32>,
) -> Result<bool, AppError> {
    let Some(user_id) = user_id else {
        return Ok(false);
    };
    if !enabled(settings)? {
        return Ok(false);
    }
    let Some(user) = candidate(conn, user_id).await? else {
        return Ok(false);
    };
    Ok(allowed(
        true,
        settings.get("allow_anonymous_mode")?.truthy(),
        &list(settings, "discourse_narrative_bot_ignored_usernames")?,
        &user,
    ))
}

/// `Jobs.enqueue(:bot_input, args)`
async fn enqueue_bot_input(
    conn: &mut PgConnection,
    args: serde_json::Value,
) -> Result<(), AppError> {
    crate::jobs::enqueue(conn, jobs::BOT_INPUT, args).await?;
    Ok(())
}

/// `on(:post_created)`: the author's reply, unless the creator was told
/// to `skip_bot` (the advanced tutorial's own posts).
pub async fn post_created(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    post_id: i32,
    skip_bot: bool,
) -> Result<(), AppError> {
    if skip_bot || !enabled(settings)? {
        return Ok(());
    }
    let author = post_author(&mut *conn, post_id).await?;
    if enqueue_job_allowed(&mut *conn, settings, author).await? {
        enqueue_bot_input(
            conn,
            json!({ "user_id": author, "post_id": post_id, "input": "reply" }),
        )
        .await?;
    }
    Ok(())
}

/// `on(:post_edited)`: the author's edit.
pub async fn post_edited(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    post_id: i32,
) -> Result<(), AppError> {
    if !enabled(settings)? {
        return Ok(());
    }
    let author = post_author(&mut *conn, post_id).await?;
    if enqueue_job_allowed(&mut *conn, settings, author).await? {
        enqueue_bot_input(
            conn,
            json!({ "user_id": author, "post_id": post_id, "input": "edit" }),
        )
        .await?;
    }
    Ok(())
}

/// `on(:post_destroyed)`: a deletion by `user_id`, the destroyer.
pub async fn post_destroyed(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    post_id: i32,
    topic_id: i32,
) -> Result<(), AppError> {
    if enqueue_job_allowed(&mut *conn, settings, Some(user_id)).await? {
        enqueue_bot_input(
            conn,
            json!({ "user_id": user_id, "post_id": post_id, "topic_id": topic_id, "input": "delete" }),
        )
        .await?;
    }
    Ok(())
}

/// `on(:post_recovered)`: a recovery by `user_id`.
pub async fn post_recovered(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    post_id: i32,
) -> Result<(), AppError> {
    if enqueue_job_allowed(&mut *conn, settings, Some(user_id)).await? {
        enqueue_bot_input(
            conn,
            json!({ "user_id": user_id, "post_id": post_id, "input": "recover" }),
        )
        .await?;
    }
    Ok(())
}

/// PostAction's after_commit on create: a like, or a flag (the
/// inappropriate flag the tutorial asks for, any other flag as a reply).
pub async fn post_action_created(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    post_id: i32,
    post_action_type_id: i32,
    is_flag: bool,
) -> Result<(), AppError> {
    let input = if post_action_type_id == LIKE {
        "like"
    } else if is_flag && post_action_type_id == INAPPROPRIATE {
        "flag"
    } else if is_flag {
        "reply"
    } else {
        return Ok(());
    };
    if enqueue_job_allowed(&mut *conn, settings, Some(user_id)).await? {
        enqueue_bot_input(
            conn,
            json!({ "user_id": user_id, "post_id": post_id, "input": input }),
        )
        .await?;
    }
    Ok(())
}

/// Bookmark's after_commit on create: a post's bookmark, or a topic's as
/// its first post's.
pub async fn bookmark_created(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    bookmarkable_type: &str,
    bookmarkable_id: i32,
) -> Result<(), AppError> {
    if !enqueue_job_allowed(&mut *conn, settings, Some(user_id)).await? {
        return Ok(());
    }
    let post_id = match bookmarkable_type {
        "Post" => Some(bookmarkable_id),
        "Topic" => {
            sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1 AND post_number = 1")
                .bind(bookmarkable_id)
                .fetch_optional(&mut *conn)
                .await?
        }
        _ => return Ok(()),
    };
    enqueue_bot_input(
        conn,
        json!({ "user_id": user_id, "post_id": post_id, "input": "bookmark" }),
    )
    .await
}

/// `on(:topic_notification_level_changed)`, from TopicUser.change, which
/// has no site settings at hand: the two this reads come from the table,
/// with the defaults plugin settings.yml gives them.
pub async fn topic_notification_level_changed(
    conn: &mut PgConnection,
    user_id: i32,
    topic_id: i32,
) -> Result<(), AppError> {
    let rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT name, value FROM site_settings WHERE name = ANY($1)")
            .bind(vec![
                "discourse_narrative_bot_enabled",
                "discourse_narrative_bot_ignored_usernames",
                "allow_anonymous_mode",
            ])
            .fetch_all(&mut *conn)
            .await?;
    let stored = |name: &str| {
        rows.iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone().unwrap_or_default())
    };
    let truthy = |v: String| v == "t" || v == "true";
    let enabled = stored("discourse_narrative_bot_enabled").is_none_or(truthy);
    if !enabled {
        return Ok(());
    }
    let anonymous_mode = stored("allow_anonymous_mode").is_some_and(truthy);
    let ignored: Vec<String> = stored("discourse_narrative_bot_ignored_usernames")
        .unwrap_or_else(|| "discourse".to_string())
        .split('|')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let Some(user) = candidate(&mut *conn, user_id).await? else {
        return Ok(());
    };
    if allowed(true, anonymous_mode, &ignored, &user) {
        enqueue_bot_input(
            conn,
            json!({ "user_id": user_id, "topic_id": topic_id, "input": "topic_notification_level_changed" }),
        )
        .await?;
    }
    Ok(())
}

async fn post_author(conn: &mut PgConnection, post_id: i32) -> Result<Option<i32>, AppError> {
    Ok(
        sqlx::query_scalar::<_, Option<i32>>("SELECT user_id FROM posts WHERE id = $1")
            .bind(post_id)
            .fetch_optional(conn)
            .await?
            .flatten(),
    )
}

/// `User#enqueue_bot_welcome_post`: unless the welcome post is off, the
/// new user tutorial (for a user the bot talks to who has not turned the
/// tips off) or the plain welcome message, after the configured delay.
pub async fn enqueue_bot_welcome_post(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    if settings
        .get("disable_discourse_narrative_bot_welcome_post")?
        .truthy()
    {
        return Ok(());
    }
    let delay = settings
        .get("discourse_narrative_bot_welcome_post_delay")?
        .to_i();
    match settings
        .get("discourse_narrative_bot_welcome_post_type")?
        .to_s()
        .as_ref()
    {
        "new_user_track" => {
            let skip_tips: Option<bool> = sqlx::query_scalar(
                "SELECT skip_new_user_tips FROM user_options WHERE user_id = $1",
            )
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
            if enqueue_job_allowed(&mut *conn, settings, Some(user_id)).await?
                && skip_tips != Some(true)
            {
                crate::jobs::enqueue_in(
                    conn,
                    delay,
                    jobs::NARRATIVE_INIT,
                    json!({ "user_id": user_id, "klass": narrative::Track::NewUser.class_name() }),
                )
                .await?;
            }
        }
        "welcome_message" => {
            crate::jobs::enqueue_in(
                conn,
                delay,
                jobs::SEND_DEFAULT_WELCOME_MESSAGE,
                json!({ "user_id": user_id }),
            )
            .await?;
        }
        _ => {}
    }
    Ok(())
}

/// `on(:user_created)`: the welcome post right away when there is no
/// delay, for users who are not staged.
pub async fn user_created(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    staged: bool,
) -> Result<(), AppError> {
    if !enabled(settings)? || staged {
        return Ok(());
    }
    if settings
        .get("discourse_narrative_bot_welcome_post_delay")?
        .to_i()
        == 0
    {
        enqueue_bot_welcome_post(conn, settings, user_id).await?;
    }
    Ok(())
}

/// `on(:user_first_logged_in)` (User#logged_in for a user never seen
/// before): the delayed welcome post.
pub async fn user_logged_in(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    if !enabled(settings)? {
        return Ok(());
    }
    let seen_before: bool =
        sqlx::query_scalar("SELECT last_seen_at IS NOT NULL FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or(true);
    if !seen_before
        && settings
            .get("discourse_narrative_bot_welcome_post_delay")?
            .to_i()
            > 0
    {
        enqueue_bot_welcome_post(conn, settings, user_id).await?;
    }
    Ok(())
}

/// `on(:user_unstaged)`: the welcome post.
pub async fn user_unstaged(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    if enabled(settings)? {
        enqueue_bot_welcome_post(conn, settings, user_id).await?;
    }
    Ok(())
}

/// User's after_destroy: the user's tutorial state goes with them.
pub async fn user_destroyed(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    if enabled(settings)? {
        store::remove(conn, user_id).await?;
    }
    Ok(())
}

/// `on(:site_setting_changed)` for default_locale: discobot's bio in the
/// new locale (English is the only one the port carries it in).
pub async fn default_locale_changed(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    new_locale: &str,
) -> Result<(), AppError> {
    if !enabled(settings)? {
        return Ok(());
    }
    let profile: Option<i32> =
        sqlx::query_scalar("SELECT user_id FROM user_profiles WHERE user_id = $1")
            .bind(BOT_USER_ID)
            .fetch_optional(&mut *conn)
            .await?;
    if profile.is_none() {
        return Ok(());
    }
    let Some(bio) = i18n.locale(new_locale).t("discourse_narrative_bot.bio") else {
        return Ok(());
    };
    // profile.update!(bio_raw:): UserProfile's before_save cooks it.
    if bio.contains(['<', '>', '&', '*', '_', '`', '[', '\n', ':', '@', '#']) {
        return Err(crate::Unsupported("cooking a discobot bio with markup").into());
    }
    let cooked = format!("<p>{bio}</p>");
    sqlx::query(
        "UPDATE user_profiles SET bio_raw = $2, bio_cooked = $3, bio_cooked_version = 1 \
         WHERE user_id = $1",
    )
    .bind(BOT_USER_ID)
    .bind(bio)
    .bind(cooked)
    .execute(conn)
    .await?;
    Ok(())
}

/// `on(:system_message_sent)`: after the trust level 2 promotion message,
/// discobot invites the user to the advanced tutorial in a message of its
/// own, in the user's locale.
pub async fn system_message_sent(
    pool: &sqlx::PgPool,
    ctx: &crate::posting::Ctx<'_>,
    message_type: &str,
    recipient_id: i32,
) -> Result<(), AppError> {
    if !enabled(ctx.settings)? || message_type != "tl2_promotion_message" {
        return Ok(());
    }
    let Some(recipient) = bot::User::find(&mut *pool.acquire().await?, recipient_id).await? else {
        return Ok(());
    };
    let locale = recipient.effective_locale(ctx.settings, ctx.i18n)?;
    let bot = bot::Bot::load(pool, ctx, locale).await?;
    let reset_trigger = format!(
        "{} {}",
        bot.t("track_selector.reset_trigger")?,
        narrative::Track::Advanced.reset_trigger(&bot)?
    );
    let raw = bot.t_with(
        "tl2_promotion_message.text_body_template",
        &[
            ("discobot_username", &bot.discobot_username()),
            ("reset_trigger", &reset_trigger),
        ],
    )?;
    bot.create_as(
        &bot.guardian,
        crate::posting::create::NewPost {
            raw,
            title: Some(bot.t("tl2_promotion_message.subject_template")?),
            pm_recipients: Some(vec![recipient.username.to_lowercase()]),
            skip_validations: true,
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

/// The plugin's `pretty_text_allowed_iframes` modifier: the certificate
/// among the iframes posts may embed.
pub fn allowed_iframes(
    settings: &SiteSettings,
    urls: &crate::url::Urls<'_>,
    list: &mut Vec<String>,
) -> Result<(), AppError> {
    if !enabled(settings)? {
        return Ok(());
    }
    let certificate = format!("{}/discobot/certificate.svg", urls.base_url()?);
    if !list.contains(&certificate) {
        list.push(certificate);
    }
    Ok(())
}

/// PostGuardianExtension's `can_create_post?`: while the plugin is on, a
/// reply to a system message discobot started is always allowed.
pub fn can_reply_to_system_message(
    settings: &SiteSettings,
    subtype: Option<&str>,
    topic_user_id: Option<i32>,
) -> Result<bool, SettingError> {
    Ok(enabled(settings)?
        && subtype == Some("system_message")
        && topic_user_id == Some(BOT_USER_ID))
}
