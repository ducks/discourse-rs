//! What the narratives and the track selector share
//! (`DiscourseNarrativeBot::Actions` and the parts of `Base` that are not
//! the state machine): discobot, its replies, the rate limits a reply
//! gives back, the timeout job, and the lookups on posts and topics.

use serde_json::json;
use sqlx::{PgConnection, PgPool};

use super::{BOT_USER_ID, PLUGIN_NAME, store};
use crate::guardian::Guardian;
use crate::i18n::Localized;
use crate::posting::Ctx;
use crate::posting::create::{self, NewPost, Outcome};
use crate::session::current::SessionUser;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `Actions::TIMEOUT_DURATION`: 15 minutes.
pub const TIMEOUT_DURATION: i64 = 900;
/// `Jobs::NarrativeTimeout`
pub const NARRATIVE_TIMEOUT: &str = "narrative_timeout";

/// What Rails raises where the port has no error of its own: a
/// PostCreator.create! refused, a record gone that the plugin assumed.
#[derive(Debug)]
pub struct BotError(pub String);

impl std::fmt::Display for BotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "discourse-narrative-bot: {}", self.0)
    }
}

impl std::error::Error for BotError {}

fn fail(message: impl Into<String>) -> AppError {
    BotError(message.into()).into()
}

/// A post as the plugin reads one (`Post.find_by(id:)`, deleted posts
/// left out by Trashable's default scope).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PostRow {
    pub id: i32,
    pub user_id: Option<i32>,
    pub topic_id: i32,
    pub post_number: i32,
    pub raw: String,
    pub cooked: String,
    pub post_type: i32,
    pub reply_to_post_number: Option<i32>,
    pub wiki: bool,
}

const POST_COLUMNS: &str =
    "id, user_id, topic_id, post_number, raw, cooked, post_type, reply_to_post_number, wiki";

pub async fn find_post(conn: &mut PgConnection, id: i64) -> Result<Option<PostRow>, AppError> {
    let Ok(id) = i32::try_from(id) else {
        return Ok(None);
    };
    Ok(sqlx::query_as(&format!(
        "SELECT {POST_COLUMNS} FROM posts WHERE id = $1 AND deleted_at IS NULL"
    ))
    .bind(id)
    .fetch_optional(conn)
    .await?)
}

/// The topic's first post (`topic.first_post`).
pub async fn first_post(conn: &mut PgConnection, topic_id: i32) -> Result<PostRow, AppError> {
    let post: Option<PostRow> = sqlx::query_as(&format!(
        "SELECT {POST_COLUMNS} FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL"
    ))
    .bind(topic_id)
    .fetch_optional(conn)
    .await?;
    post.ok_or_else(|| fail("a topic without a first post"))
}

/// `Post#url`: `/t/<slug>/<topic id>/<number>`, without the base path.
pub async fn post_url(conn: &mut PgConnection, post_id: i64) -> Result<String, AppError> {
    let row: Option<(i32, i32, Option<String>)> = sqlx::query_as(
        "SELECT t.id, p.post_number, t.slug FROM posts p JOIN topics t ON t.id = p.topic_id \
         WHERE p.id = $1 AND p.deleted_at IS NULL AND t.deleted_at IS NULL",
    )
    .bind(post_id as i32)
    .fetch_optional(conn)
    .await?;
    match row {
        Some((topic_id, number, Some(slug))) => Ok(format!("/t/{slug}/{topic_id}/{number}")),
        Some((_, _, None)) => Err(Unsupported("Post#url for a topic without a stored slug").into()),
        // Post.find_by(id:).url on nil raises.
        None => Err(fail("the post to link to is gone")),
    }
}

/// The human the bot talks to, as the plugin reads them.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: i32,
    pub username: String,
    pub trust_level: i32,
    pub admin: bool,
    pub moderator: bool,
    pub staged: bool,
    pub locale: Option<String>,
    pub created_at: chrono::NaiveDateTime,
    /// `user_option.skip_new_user_tips` (`manually_disabled_discobot?`).
    pub skip_new_user_tips: Option<bool>,
}

impl User {
    pub async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<User>, AppError> {
        Ok(sqlx::query_as(
            "SELECT u.id, u.username, u.trust_level, u.admin, u.moderator, u.staged, u.locale, \
                    u.created_at, o.skip_new_user_tips \
             FROM users u LEFT JOIN user_options o ON o.user_id = u.id WHERE u.id = $1",
        )
        .bind(id)
        .fetch_optional(conn)
        .await?)
    }

    /// `manually_disabled_discobot?`
    pub fn manually_disabled_discobot(&self) -> bool {
        self.skip_new_user_tips == Some(true)
    }

    /// `User#effective_locale`
    pub fn effective_locale(
        &self,
        settings: &crate::site_settings::SiteSettings,
        i18n: &crate::i18n::I18n,
    ) -> Result<String, AppError> {
        let default = settings.get("default_locale")?.to_s().to_string();
        Ok(match self.locale.as_deref() {
            Some(locale)
                if !locale.is_empty()
                    && settings.get("allow_user_locale")?.truthy()
                    && i18n.available_locales().contains(&locale) =>
            {
                locale.to_string()
            }
            _ => default,
        })
    }
}

/// discobot and what it needs to act: the posting context, the locale it
/// speaks in (`I18n.with_locale(user.effective_locale)`), its user and
/// guardian.
pub struct Bot<'a> {
    pub pool: &'a PgPool,
    pub ctx: &'a Ctx<'a>,
    pub locale: String,
    pub discobot: SessionUser,
    pub guardian: Guardian,
}

impl<'a> Bot<'a> {
    /// `User.find(-2)`, which raises when discobot is gone.
    pub async fn load(
        pool: &'a PgPool,
        ctx: &'a Ctx<'a>,
        locale: String,
    ) -> Result<Self, AppError> {
        let mut conn = pool.acquire().await?;
        let discobot = SessionUser::load(&mut conn, BOT_USER_ID)
            .await?
            .ok_or_else(|| fail("the discobot user is gone"))?;
        let guardian = Guardian::for_user(&mut conn, &discobot).await?;
        Ok(Bot {
            pool,
            ctx,
            locale,
            discobot,
            guardian,
        })
    }

    pub fn tr(&self) -> Localized<'_> {
        self.ctx.i18n.locale(&self.locale)
    }

    pub fn urls(&self) -> Urls<'_> {
        Urls {
            config: self.ctx.config,
            settings: self.ctx.settings,
        }
    }

    /// `discobot_username`: its username_lower.
    pub fn discobot_username(&self) -> String {
        self.discobot.username.to_lowercase()
    }

    /// `Discourse.base_path`
    pub fn base_path(&self) -> &str {
        self.ctx.config.globals.relative_url_root()
    }

    /// `I18n.t("discourse_narrative_bot.<key>")` with no arguments: its
    /// placeholders stay as they are.
    pub fn t(&self, key: &str) -> Result<String, AppError> {
        let full = format!("discourse_narrative_bot.{key}");
        self.tr()
            .t(&full)
            .map(str::to_string)
            .ok_or_else(|| Unsupported("a discobot message without a translation").into())
    }

    /// `I18n.t("discourse_narrative_bot.<key>", args)`
    pub fn t_with(&self, key: &str, args: &[(&str, &str)]) -> Result<String, AppError> {
        let full = format!("discourse_narrative_bot.{key}");
        self.tr()
            .t_with(&full, args)
            .ok_or_else(|| Unsupported("a discobot message without a translation").into())
    }

    /// `I18n.t(key, i18n_post_args(extra))`: base_uri, then the extras.
    pub fn t_post(&self, key: &str, extra: &[(&str, &str)]) -> Result<String, AppError> {
        let mut args = vec![("base_uri", self.base_path())];
        args.extend_from_slice(extra);
        self.t_with(key, &args)
    }

    /// `fake_delay`: a second or two before replying, in production only.
    pub async fn fake_delay(&self) {
        if self.ctx.config.rails_env == crate::config::RailsEnv::Production {
            use rand::Rng;
            let secs: f64 = rand::thread_rng().gen_range(1.0..2.0);
            tokio::time::sleep(std::time::Duration::from_secs_f64(secs)).await;
        }
    }

    /// `reply_to(post, raw, opts, post_alert_options)`: PostCreator.create!
    /// as discobot without validations, a reply to the post when there is
    /// one (and then the poster's create limits handed back), emails
    /// skipped unless asked for.
    pub async fn reply_to(
        &self,
        post: Option<&PostRow>,
        raw: String,
        opts: ReplyOpts,
        skip_send_email: bool,
    ) -> Result<PostRow, AppError> {
        let topic_id = opts.topic_id.or(post.map(|p| p.topic_id));
        let reply_to_post_number = post.map(|p| p.post_number);
        let new_topic = topic_id.is_none();
        let args = NewPost {
            raw,
            topic_id,
            reply_to_post_number,
            title: if new_topic { opts.title } else { None },
            pm_recipients: match (&opts.target_username, new_topic) {
                (Some(username), true) => Some(vec![username.to_lowercase()]),
                _ => None,
            },
            subtype: if new_topic { opts.subtype } else { None },
            skip_validations: true,
            post_alert_options: Some(json!({ "skip_send_email": skip_send_email })),
            ..Default::default()
        };
        let post_id = self.create_as(&self.guardian, args).await?;
        let created = find_post(&mut *self.pool.acquire().await?, i64::from(post_id))
            .await?
            .ok_or_else(|| fail("discobot's reply vanished"))?;
        if let Some(post) = post {
            self.reset_rate_limits(post).await?;
        }
        Ok(created)
    }

    /// `PostCreator.create!(user, opts)`: the new post's id, or the
    /// creator's refusal raised.
    pub async fn create_as(&self, guardian: &Guardian, args: NewPost) -> Result<i32, AppError> {
        match Box::pin(create::create(self.pool, self.ctx, guardian, args)).await? {
            Outcome::Created { post_id } => Ok(post_id),
            Outcome::Invalid(errors) | Outcome::InvalidRaw(errors) => {
                Err(fail(format!("PostCreator refused: {}", errors.join(", "))))
            }
            _ => Err(fail("PostCreator refused the post")),
        }
    }

    /// `reset_rate_limits(post)`: twice per topic and state within the
    /// poster's create limit, the limiters the poster's post performed
    /// are given back so the tutorial does not hold them up.
    async fn reset_rate_limits(&self, post: &PostRow) -> Result<(), AppError> {
        let s = self.ctx.settings;
        let Some(user_id) = post.user_id else {
            return Ok(());
        };
        let mut conn = self.pool.acquire().await?;
        let Some(user) = User::find(&mut conn, user_id).await? else {
            return Ok(());
        };
        let staff = user.admin || user.moderator;
        let day_ago = crate::clock::now_naive() - chrono::Duration::hours(24);
        // User#new_user?
        let new_user =
            (user.created_at >= day_ago || user.trust_level == 0) && user.trust_level < 2 && !staff;
        let duration = if new_user {
            s.get("rate_limit_new_user_create_post")?.to_i()
        } else {
            s.get("rate_limit_create_post")?.to_i()
        };
        if duration <= 0 {
            return Ok(());
        }
        let Some(data) = store::get(&mut conn, user.id).await? else {
            return Ok(());
        };
        let state = store::str_of(&data, "state").unwrap_or_default();
        let key = format!("{PLUGIN_NAME}:reset-rate-limit:{}:{state}", post.topic_id);
        let count = match crate::owned_schema::cached_get(&mut conn, &key).await? {
            Some(count) => crate::ruby::to_i(&count),
            None => {
                crate::owned_schema::cached_setex(&mut conn, &key, duration, "0").await?;
                0
            }
        };
        if count < 2 {
            let poster = crate::posting::limits::Poster {
                id: user.id,
                trust_level: user.trust_level,
                admin: user.admin,
                staff,
                created_at: user.created_at,
            };
            drop(conn);
            let limiters = crate::posting::limits::post_limiters(s, &poster, post.post_number)?;
            for limiter in limiters {
                limiter
                    .rollback(self.pool, self.ctx.config.rate_limits)
                    .await?;
            }
            let mut conn = self.pool.acquire().await?;
            incr(&mut conn, &key).await?;
        }
        Ok(())
    }

    /// `bot_mentioned?(post)`: a `.mention` in the cooked post naming
    /// discobot.
    pub fn bot_mentioned_in_cooked(&self, cooked: &str) -> Result<bool, AppError> {
        let target = format!("@{}", self.discobot_username());
        let dom = crate::pretty_text::cleanup::parse(cooked);
        for e in crate::pretty_text::cleanup::all_elements(&dom) {
            if crate::pretty_text::cleanup::has_class(&e, "mention") {
                let text = crate::pretty_text::cleanup::text(&e);
                if crate::accounts::normalize_username(&text)? == target {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// `reply_to_bot_post?(post)`: a reply to one of discobot's posts.
    pub async fn reply_to_bot_post(&self, post: &PostRow) -> Result<bool, AppError> {
        let Some(number) = post.reply_to_post_number else {
            return Ok(false);
        };
        let author: Option<Option<i32>> = sqlx::query_scalar(
            "SELECT user_id FROM posts WHERE topic_id = $1 AND post_number = $2 AND deleted_at IS NULL",
        )
        .bind(post.topic_id)
        .bind(number)
        .fetch_optional(&mut *self.pool.acquire().await?)
        .await?;
        Ok(author.flatten() == Some(BOT_USER_ID))
    }

    /// `pm_to_bot?(post)`: a message between one human and discobot
    /// (`pm_with_non_human_user?`, no groups), discobot among its users.
    pub async fn pm_to_bot(&self, post: &PostRow) -> Result<bool, AppError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topics t \
               WHERE t.id = $1 AND t.deleted_at IS NULL AND t.archetype = 'private_message' \
                 AND NOT EXISTS (SELECT 1 FROM topic_allowed_groups g WHERE g.topic_id = t.id) \
                 AND (SELECT COUNT(*) FROM topic_allowed_users a \
                      WHERE a.topic_id = t.id AND a.user_id > 0) = 1 \
                 AND EXISTS (SELECT 1 FROM topic_allowed_users a \
                             WHERE a.topic_id = t.id AND a.user_id = $2))",
        )
        .bind(post.topic_id)
        .bind(BOT_USER_ID)
        .fetch_one(&mut *self.pool.acquire().await?)
        .await?)
    }

    /// `PostActionCreator.like(discobot_user, post)`, whatever its result.
    pub async fn like(&self, post_id: i32) -> Result<(), AppError> {
        Box::pin(crate::likes::like(
            self.pool,
            self.ctx,
            &self.guardian,
            post_id,
        ))
        .await?;
        Ok(())
    }
}

/// The options `reply_to` merges over its defaults.
#[derive(Default)]
pub struct ReplyOpts {
    pub topic_id: Option<i32>,
    pub title: Option<String>,
    /// `target_usernames` with `archetype: private_message`.
    pub target_username: Option<String>,
    pub subtype: Option<String>,
}

/// Redis's `INCR` on a value set with `setex`, its expiry kept.
pub async fn incr(conn: &mut PgConnection, key: &str) -> Result<i64, AppError> {
    let value: Option<String> = sqlx::query_scalar(
        "UPDATE discourse_rs.cached_values SET value = (COALESCE(NULLIF(value, ''), '0')::bigint + 1)::text \
         WHERE key = $1 AND expires_at > clock_timestamp() RETURNING value",
    )
    .bind(key)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(value) = value {
        return Ok(crate::ruby::to_i(&value));
    }
    // INCR of a missing key: 1, with no expiry.
    crate::owned_schema::cached_setex(conn, key, 100 * 365 * 24 * 3600, "1").await?;
    Ok(1)
}

/// Redis's `TTL key` in seconds, -2 for a missing key.
pub async fn ttl(conn: &mut PgConnection, key: &str) -> Result<i64, AppError> {
    let secs: Option<f64> = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM expires_at - clock_timestamp())::float8 \
         FROM discourse_rs.cached_values WHERE key = $1 AND expires_at > clock_timestamp()",
    )
    .bind(key)
    .fetch_optional(conn)
    .await?;
    Ok(secs.map_or(-2, |s| s.ceil() as i64))
}

/// `Jobs.cancel_scheduled_job(:narrative_timeout, user_id:, klass:)`
pub async fn cancel_timeout_job(
    conn: &mut PgConnection,
    user_id: i32,
    klass: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "DELETE FROM discourse_rs.jobs WHERE name = $1 AND failed_at IS NULL \
         AND locked_until IS NULL AND args = $2",
    )
    .bind(NARRATIVE_TIMEOUT)
    .bind(json!({ "user_id": user_id, "klass": klass }))
    .execute(conn)
    .await?;
    Ok(())
}

/// `enqueue_timeout_job(user)`: the timeout reminder in 15 minutes,
/// replacing the one pending.
pub async fn enqueue_timeout_job(
    conn: &mut PgConnection,
    user_id: i32,
    klass: &str,
) -> Result<(), AppError> {
    cancel_timeout_job(&mut *conn, user_id, klass).await?;
    crate::jobs::enqueue_in(
        conn,
        TIMEOUT_DURATION,
        NARRATIVE_TIMEOUT,
        json!({ "user_id": user_id, "klass": klass }),
    )
    .await?;
    Ok(())
}
