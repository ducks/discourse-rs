//! The plugin's jobs: BotInput (an input to the track selector),
//! NarrativeInit (a track's first step), NarrativeTimeout (the reminder)
//! and SendDefaultWelcomeMessage (the plain welcome message as discobot).

use serde_json::Value;

use super::bot::{Bot, User};
use super::narrative::{self, Track};
use super::track_selector;
use crate::posting::Ctx;
use crate::posting::create::NewPost;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Jobs::BotInput`
pub const BOT_INPUT: &str = "bot_input";
/// `Jobs::NarrativeInit`
pub const NARRATIVE_INIT: &str = "narrative_init";
/// `Jobs::NarrativeTimeout`
pub const NARRATIVE_TIMEOUT: &str = super::bot::NARRATIVE_TIMEOUT;
/// `Jobs::SendDefaultWelcomeMessage`
pub const SEND_DEFAULT_WELCOME_MESSAGE: &str = "send_default_welcome_message";

/// The plugin's jobs, for the job runner.
pub const JOBS: [&str; 4] = [
    BOT_INPUT,
    NARRATIVE_INIT,
    NARRATIVE_TIMEOUT,
    SEND_DEFAULT_WELCOME_MESSAGE,
];

fn int_arg(args: &Value, key: &str) -> Option<i64> {
    match &args[key] {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// `run`, boxed in a frame of its own, for the job runner's dispatch
/// (whose frame would otherwise hold the whole job's future).
pub fn run_boxed<'a>(state: &'a AppState, name: &'a str, args: &'a Value) -> super::Event<'a> {
    Box::pin(run(state, name, args))
}

/// Runs one of the plugin's jobs by name.
pub async fn run(state: &AppState, name: &str, args: &Value) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let Some(user_id) = int_arg(args, "user_id").and_then(|id| i32::try_from(id).ok()) else {
        return Ok(());
    };
    // User.find_by(id:): nothing to do for a user who is gone.
    let Some(user) = User::find(&mut conn, user_id).await? else {
        return Ok(());
    };
    drop(conn);
    if name == SEND_DEFAULT_WELCOME_MESSAGE {
        return Box::pin(send_default_welcome_message(state, &ctx, &user)).await;
    }
    // I18n.with_locale(user.effective_locale)
    let locale = user.effective_locale(&settings, &state.i18n)?;
    let bot = Bot::load(&state.pool, &ctx, locale).await?;
    match name {
        BOT_INPUT => {
            let input = args["input"].as_str().unwrap_or_default();
            let topic_id = int_arg(args, "topic_id").and_then(|id| i32::try_from(id).ok());
            Box::pin(track_selector::select(
                &bot,
                input,
                &user,
                int_arg(args, "post_id"),
                topic_id,
            ))
            .await
        }
        NARRATIVE_INIT | NARRATIVE_TIMEOUT => {
            let klass = args["klass"].as_str().unwrap_or_default();
            let track = Track::from_class_name(klass).ok_or(Unsupported(
                "a narrative job for a class that is not a track",
            ))?;
            if name == NARRATIVE_INIT {
                Box::pin(narrative::input(
                    &bot, track, "init", &user, None, None, false,
                ))
                .await?;
                Ok(())
            } else {
                narrative::notify_timeout(&bot, track, &user).await
            }
        }
        _ => Err(Unsupported("a narrative bot job that does not exist").into()),
    }
}

/// `Jobs::SendDefaultWelcomeMessage`: the welcome (or invited welcome)
/// system message, sent by discobot and closed.
async fn send_default_welcome_message(
    state: &AppState,
    ctx: &Ctx<'_>,
    user: &User,
) -> Result<(), AppError> {
    let pool = &state.pool;
    // user.invited_by: the inviter of the invite the user redeemed.
    let invited_by: Option<i32> = sqlx::query_scalar(
        "SELECT i.invited_by_id FROM invites i JOIN invited_users iu ON iu.invite_id = i.id \
         JOIN users u ON u.id = i.invited_by_id WHERE iu.user_id = $1 ORDER BY i.id LIMIT 1",
    )
    .bind(user.id)
    .fetch_optional(pool)
    .await?;
    let kind = if invited_by.is_some() {
        "welcome_invite"
    } else {
        "welcome_user"
    };
    let params = crate::system_message::defaults(pool, ctx, user.id, kind).await?;
    let args: Vec<(&str, &str)> = params
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let t = |part: &str| {
        ctx.i18n
            .t_with(&format!("system_messages.{kind}.{part}"), &args)
            .ok_or(Unsupported("system messages without a translation"))
    };
    let title = t("subject_template")?;
    let raw = t("text_body_template")?;
    let bot = Bot::load(pool, ctx, ctx.settings.get("default_locale")?.to_s()).await?;
    let post_id = bot
        .create_as(
            &bot.guardian,
            NewPost {
                raw,
                title: Some(title),
                pm_recipients: Some(vec![user.username.to_lowercase()]),
                skip_validations: true,
                ..Default::default()
            },
        )
        .await?;
    let topic_id: i32 = sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
        .bind(post_id)
        .fetch_one(pool)
        .await?;
    let mut conn = pool.acquire().await?;
    crate::topic_status::apply(
        &mut conn,
        ctx,
        topic_id,
        bot.discobot.id,
        crate::topic_status::Status::Closed,
        true,
        &crate::topic_status::StatusOpts::default(),
    )
    .await?;
    Ok(())
}
