//! The two tutorials (`NewUserNarrative`, `AdvancedUserNarrative`) and the
//! state machine they share (`Base#input`): each state names the next one,
//! the instructions that introduce it and what each input does; an
//! action's post moves the user on, its absence keeps them where they
//! are. Prerequisites and `discourse_narrative_bot_skip_tutorials` step
//! over states, `skip` jumps a state, and the last one ends the track
//! with a certificate and a badge.

use chrono::Utc;
use serde_json::{Value, json};

use super::bot::{self, Bot, BotError, PostRow, ReplyOpts, User};
use super::store::{self, Data};
use super::{BOT_USER_ID, jobs};
use crate::guardian::Guardian;
use crate::posting::create::NewPost;
use crate::session::current::SessionUser;
use crate::{AppError, Unsupported};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Track {
    NewUser,
    Advanced,
}

impl Track {
    /// The Ruby class name, as the store and the jobs carry it.
    pub fn class_name(self) -> &'static str {
        match self {
            Track::NewUser => "DiscourseNarrativeBot::NewUserNarrative",
            Track::Advanced => "DiscourseNarrativeBot::AdvancedUserNarrative",
        }
    }

    /// `klass.constantize`
    pub fn from_class_name(name: &str) -> Option<Track> {
        match name {
            "DiscourseNarrativeBot::NewUserNarrative" => Some(Track::NewUser),
            "DiscourseNarrativeBot::AdvancedUserNarrative" => Some(Track::Advanced),
            _ => None,
        }
    }

    /// `I18N_KEY` without its `discourse_narrative_bot.` prefix.
    fn key(self) -> &'static str {
        match self {
            Track::NewUser => "new_user_narrative",
            Track::Advanced => "advanced_user_narrative",
        }
    }

    /// `BADGE_NAME`
    pub fn badge_name(self) -> &'static str {
        match self {
            Track::NewUser => "Certified",
            Track::Advanced => "Licensed",
        }
    }

    /// `reset_trigger`
    pub fn reset_trigger(self, bot: &Bot<'_>) -> Result<String, AppError> {
        bot.t(&format!("{}.reset_trigger", self.key()))
    }

    /// The DistributedMutex `synchronize` takes.
    fn mutex(self, user_id: i32) -> String {
        match self {
            Track::NewUser => format!("new_user_narrative_{user_id}"),
            Track::Advanced => format!("advanced_user_narrative_{user_id}"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Action {
    SayHello,
    ReplyToBookmark,
    MissingBookmark,
    ReplyToOnebox,
    ReplyToEmoji,
    ReplyToMention,
    ReplyToFormatting,
    ReplyToQuote,
    ReplyToImage,
    TrackImagesLike,
    ReplyToLikes,
    MissingLikesLike,
    ReplyToFlag,
    MissingFlag,
    ReplyToSearch,
    StartAdvancedTrack,
    ReplyToEdit,
    MissingEdit,
    ReplyToDelete,
    MissingDelete,
    ReplyToRecover,
    MissingRecover,
    ReplyToCategoryHashtag,
    ReplyToTopicNotificationLevelChanged,
    MissingTopicNotificationLevelChange,
    ReplyToPoll,
    ReplyToDetails,
}

#[derive(Debug, Clone, Copy)]
enum Prereq {
    EnableEmoji,
    EnableMentions,
    EmbeddedMedia,
    NoEmbeddedMedia,
    AllowFlaggingStaff,
    Poll,
}

/// An input's entry in a state (`reply: { next_state:, action: }`).
struct On {
    input: &'static str,
    next_state: Option<&'static str>,
    next_instructions: Option<&'static str>,
    action: Action,
}

/// A state of `TRANSITION_TABLE`. Instructions are named by their
/// translation section (`<section>.instructions`).
struct StateDef {
    next_state: Option<&'static str>,
    next_instructions: Option<&'static str>,
    prerequisite: Option<Prereq>,
    inputs: Vec<On>,
}

const fn on(input: &'static str, action: Action) -> On {
    On {
        input,
        next_state: None,
        next_instructions: None,
        action,
    }
}

const fn on_to(input: &'static str, next_state: &'static str, action: Action) -> On {
    On {
        input,
        next_state: Some(next_state),
        next_instructions: None,
        action,
    }
}

fn state(
    next_state: &'static str,
    next_instructions: Option<&'static str>,
    prerequisite: Option<Prereq>,
    inputs: Vec<On>,
) -> StateDef {
    StateDef {
        next_state: Some(next_state),
        next_instructions,
        prerequisite,
        inputs,
    }
}

fn table(track: Track, name: &str) -> Option<StateDef> {
    use Action::*;
    Some(match (track, name) {
        (Track::NewUser, "begin") => StateDef {
            next_state: None,
            next_instructions: None,
            prerequisite: None,
            inputs: vec![On {
                input: "init",
                next_state: Some("tutorial_bookmark"),
                next_instructions: Some("bookmark"),
                action: SayHello,
            }],
        },
        (Track::NewUser, "tutorial_bookmark") => state(
            "tutorial_onebox",
            Some("onebox"),
            None,
            vec![
                on("bookmark", ReplyToBookmark),
                on_to("reply", "tutorial_bookmark", MissingBookmark),
            ],
        ),
        (Track::NewUser, "tutorial_onebox") => state(
            "tutorial_emoji",
            Some("emoji"),
            None,
            vec![on("reply", ReplyToOnebox)],
        ),
        (Track::NewUser, "tutorial_emoji") => state(
            "tutorial_mention",
            Some("mention"),
            Some(Prereq::EnableEmoji),
            vec![on("reply", ReplyToEmoji)],
        ),
        (Track::NewUser, "tutorial_mention") => state(
            "tutorial_formatting",
            Some("formatting"),
            Some(Prereq::EnableMentions),
            vec![on("reply", ReplyToMention)],
        ),
        (Track::NewUser, "tutorial_formatting") => state(
            "tutorial_quote",
            Some("quoting"),
            None,
            vec![on("reply", ReplyToFormatting)],
        ),
        (Track::NewUser, "tutorial_quote") => state(
            "tutorial_images",
            Some("images"),
            None,
            vec![on("reply", ReplyToQuote)],
        ),
        // tutorial_images and tutorial_likes are mutually exclusive: their
        // prerequisites see that only one of them runs.
        (Track::NewUser, "tutorial_images") => state(
            "tutorial_likes",
            Some("likes"),
            Some(Prereq::EmbeddedMedia),
            vec![on("reply", ReplyToImage), on("like", TrackImagesLike)],
        ),
        (Track::NewUser, "tutorial_likes") => state(
            "tutorial_flag",
            Some("flag"),
            Some(Prereq::NoEmbeddedMedia),
            vec![
                on("like", ReplyToLikes),
                on_to("reply", "tutorial_likes", MissingLikesLike),
            ],
        ),
        (Track::NewUser, "tutorial_flag") => state(
            "tutorial_search",
            Some("search"),
            Some(Prereq::AllowFlaggingStaff),
            vec![
                on("flag", ReplyToFlag),
                on_to("reply", "tutorial_flag", MissingFlag),
            ],
        ),
        (Track::NewUser, "tutorial_search") => {
            state("end", None, None, vec![on("reply", ReplyToSearch)])
        }
        (Track::Advanced, "begin") => state(
            "tutorial_edit",
            Some("edit"),
            None,
            vec![on("init", StartAdvancedTrack)],
        ),
        (Track::Advanced, "tutorial_edit") => state(
            "tutorial_delete",
            Some("delete"),
            None,
            vec![
                on("edit", ReplyToEdit),
                on_to("reply", "tutorial_edit", MissingEdit),
            ],
        ),
        (Track::Advanced, "tutorial_delete") => state(
            "tutorial_recover",
            Some("recover"),
            None,
            vec![
                on("delete", ReplyToDelete),
                on_to("reply", "tutorial_delete", MissingDelete),
            ],
        ),
        (Track::Advanced, "tutorial_recover") => state(
            "tutorial_category_hashtag",
            Some("category_hashtag"),
            None,
            vec![
                on("recover", ReplyToRecover),
                on_to("reply", "tutorial_recover", MissingRecover),
            ],
        ),
        (Track::Advanced, "tutorial_category_hashtag") => state(
            "tutorial_change_topic_notification_level",
            Some("change_topic_notification_level"),
            None,
            vec![on("reply", ReplyToCategoryHashtag)],
        ),
        (Track::Advanced, "tutorial_change_topic_notification_level") => state(
            "tutorial_poll",
            Some("poll"),
            None,
            vec![
                on(
                    "topic_notification_level_changed",
                    ReplyToTopicNotificationLevelChanged,
                ),
                on_to(
                    "reply",
                    "tutorial_change_topic_notification_level",
                    MissingTopicNotificationLevelChange,
                ),
            ],
        ),
        (Track::Advanced, "tutorial_poll") => state(
            "tutorial_details",
            Some("details"),
            Some(Prereq::Poll),
            vec![on("reply", ReplyToPoll)],
        ),
        (Track::Advanced, "tutorial_details") => {
            state("end", None, None, vec![on("reply", ReplyToDetails)])
        }
        _ => return None,
    })
}

/// `NewUserNarrative.search_answer` and its emoji.
pub const SEARCH_ANSWER: &str = ":herb:";
const SEARCH_ANSWER_EMOJI: &str = "\u{1F33F}";

/// `klass.new.input(input, user, post:, topic_id:, skip:)`: the new post
/// that moved the user on, or None.
pub async fn input(
    bot: &Bot<'_>,
    track: Track,
    input: &str,
    user: &User,
    post: Option<PostRow>,
    topic_id: Option<i32>,
    skip: bool,
) -> Result<Option<PostRow>, AppError> {
    // synchronize(user): a DistributedMutex per user and track, held by a
    // transaction of its own while the input runs.
    let mut mutex = bot.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(track.mutex(user.id))
        .execute(&mut *mutex)
        .await?;
    let data = store::get(&mut *bot.pool.acquire().await?, user.id)
        .await?
        .unwrap_or_default();
    let state = store::str_of(&data, "state").unwrap_or("begin").to_string();
    let mut run = Run {
        bot,
        track,
        user,
        user_guardian: None,
        post,
        topic_id,
        skip,
        data,
        state,
        input: input.to_string(),
        next_instructions: None,
    };
    let result = Box::pin(run.run()).await;
    mutex.commit().await?;
    result
}

/// `reset_bot(user, post)`: the state cleared (keeping the message when
/// the reset came in it), and the track started again in two seconds.
pub async fn reset_bot(
    bot: &Bot<'_>,
    track: Track,
    user: &User,
    post: &PostRow,
) -> Result<(), AppError> {
    let mut data = Data::new();
    if bot.pm_to_bot(post).await? {
        data.insert("topic_id".into(), json!(post.topic_id));
    }
    let mut conn = bot.pool.acquire().await?;
    store::set(&mut conn, user.id, &data).await?;
    crate::jobs::enqueue_in(
        &mut conn,
        2,
        jobs::NARRATIVE_INIT,
        json!({ "user_id": user.id, "klass": track.class_name() }),
    )
    .await?;
    Ok(())
}

/// `notify_timeout(user)`: a nudge under discobot's last post, emailed.
pub async fn notify_timeout(bot: &Bot<'_>, track: Track, user: &User) -> Result<(), AppError> {
    let mut conn = bot.pool.acquire().await?;
    let data = store::get(&mut conn, user.id).await?.unwrap_or_default();
    let Some(last_post_id) = store::int_of(&data, "last_post_id") else {
        return Ok(());
    };
    let Some(post) = bot::find_post(&mut conn, last_post_id).await? else {
        return Ok(());
    };
    drop(conn);
    let reset_trigger = format!(
        "{} {}",
        bot.t("track_selector.reset_trigger")?,
        track.reset_trigger(bot)?
    );
    let raw = bot.t_post(
        "timeout.message",
        &[
            ("username", &user.username),
            ("skip_trigger", &bot.t("track_selector.skip_trigger")?),
            ("reset_trigger", &reset_trigger),
        ],
    )?;
    bot.reply_to(Some(&post), raw, ReplyOpts::default(), false)
        .await?;
    Ok(())
}

/// What a transition decided (`opts`).
struct Opts {
    next_state: Option<&'static str>,
    next_instructions: Option<&'static str>,
    action: Option<Action>,
}

/// One `input` call: Base's instance variables.
struct Run<'r, 'a> {
    bot: &'r Bot<'a>,
    track: Track,
    user: &'r User,
    user_guardian: Option<Guardian>,
    post: Option<PostRow>,
    topic_id: Option<i32>,
    skip: bool,
    data: Data,
    state: String,
    input: String,
    next_instructions: Option<&'static str>,
}

impl Run<'_, '_> {
    async fn run(&mut self) -> Result<Option<PostRow>, AppError> {
        // transition, then past states whose prerequisite fails or that
        // the skip setting names; an input the state has no transition
        // for is ignored (InvalidTransitionError).
        let Some(mut opts) = self.transition() else {
            return Ok(None);
        };
        let skipped = super::list(
            self.bot.ctx.settings,
            "discourse_narrative_bot_skip_tutorials",
        )?;
        loop {
            let next_state = opts
                .next_state
                .ok_or_else(|| BotError("a transition without a next state".into()))?;
            if next_state == "end" {
                break;
            }
            let next = table(self.track, next_state)
                .ok_or_else(|| BotError(format!("no state {next_state}")))?;
            let holds = match next.prerequisite {
                None => true,
                Some(p) => self.prerequisite(p).await?,
            };
            if holds && !skipped.iter().any(|s| s == next_state) {
                break;
            }
            opts.next_state = next.next_state;
            opts.next_instructions = next.next_instructions;
        }
        let next_state = opts.next_state.unwrap_or("end");
        if let Some(instructions) = opts.next_instructions {
            self.next_instructions = Some(instructions);
        }
        let old_data = self.data.clone();
        match Box::pin(self.apply(next_state, opts.action, &old_data)).await {
            Ok(post) => Ok(post),
            Err(e) => {
                self.data = old_data;
                self.save().await?;
                Err(e)
            }
        }
    }

    fn transition(&self) -> Option<Opts> {
        let def = table(self.track, &self.state)?;
        let on = def.inputs.iter().find(|i| i.input == self.input)?;
        let mut opts = Opts {
            next_state: def.next_state,
            next_instructions: def.next_instructions,
            action: None,
        };
        if !self.skip {
            if on.next_state.is_some() {
                opts.next_state = on.next_state;
            }
            if on.next_instructions.is_some() {
                opts.next_instructions = on.next_instructions;
            }
            opts.action = Some(on.action);
        }
        Some(opts)
    }

    async fn apply(
        &mut self,
        next_state: &'static str,
        action: Option<Action>,
        old_data: &Data,
    ) -> Result<Option<PostRow>, AppError> {
        let new_post = if self.skip && self.state != "end" {
            self.skip_tutorial(next_state).await?
        } else {
            let action = action.ok_or_else(|| BotError("a transition without an action".into()))?;
            Box::pin(self.perform(action)).await?
        };
        let Some(new_post) = new_post else {
            return Ok(None);
        };
        let old_state = store::str_of(old_data, "state").map(str::to_string);
        let state_changed = old_state.as_deref().unwrap_or("") != next_state;
        if state_changed && let Some(old) = &old_state {
            self.clean_up_state(old).await?;
        }
        self.state = next_state.to_string();
        self.data.insert("state".into(), json!(next_state));
        self.data.insert("last_post_id".into(), json!(new_post.id));
        self.save().await?;
        if state_changed {
            self.init_state(next_state).await?;
        }
        if next_state == "end" {
            self.end_reply().await?;
            let mut conn = self.bot.pool.acquire().await?;
            bot::cancel_timeout_job(&mut conn, self.user.id, self.track.class_name()).await?;
            self.grant_badge(&mut conn).await?;
            let mut done = Data::new();
            done.insert("topic_id".into(), json!(new_post.topic_id));
            done.insert("state".into(), json!("end"));
            done.insert("track".into(), json!(self.track.class_name()));
            store::set(&mut conn, self.user.id, &done).await?;
        }
        Ok(Some(new_post))
    }

    /// `BadgeGranter.grant(Badge.find_by(name: badge_name), user)`
    async fn grant_badge(&self, conn: &mut sqlx::PgConnection) -> Result<(), AppError> {
        let id: Option<i32> = sqlx::query_scalar("SELECT id FROM badges WHERE name = $1")
            .bind(self.track.badge_name())
            .fetch_optional(&mut *conn)
            .await?;
        let Some(id) = id else {
            return Ok(());
        };
        let Some(badge) = crate::badge_granter::Badge::find(&mut *conn, id).await? else {
            return Ok(());
        };
        let ctx = self.bot.ctx;
        crate::badge_granter::grant(
            conn,
            ctx.bus,
            ctx.settings,
            ctx.i18n,
            &badge,
            self.user.id,
            &crate::badge_granter::GrantOpts::system(None),
        )
        .await?;
        Ok(())
    }

    async fn save(&self) -> Result<(), AppError> {
        store::set(
            &mut *self.bot.pool.acquire().await?,
            self.user.id,
            &self.data,
        )
        .await
    }

    /// `set_state_data(key, value)`: under the current state's hash.
    async fn set_state_data(&mut self, key: &str, value: Value) -> Result<(), AppError> {
        let entry = self
            .data
            .entry(self.state.clone())
            .or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({});
        }
        entry
            .as_object_mut()
            .expect("an object")
            .insert(key.to_string(), value);
        self.save().await
    }

    /// `get_state_data(key)`
    fn get_state_data(&self, key: &str) -> Option<&Value> {
        self.data
            .get(&self.state)
            .and_then(|s| s.get(key))
            .filter(|v| !v.is_null())
    }

    /// `valid_topic?(topic_id)`: the tutorial's own message.
    fn valid_topic(&self, topic_id: Option<i32>) -> bool {
        match (topic_id, self.data.get("topic_id")) {
            (Some(id), Some(Value::Number(n))) => n.as_i64() == Some(i64::from(id)),
            (None, None | Some(Value::Null)) => true,
            _ => false,
        }
    }

    fn post(&self) -> Result<&PostRow, AppError> {
        self.post
            .as_ref()
            .ok_or_else(|| BotError("an input without its post".into()).into())
    }

    fn post_topic_id(&self) -> Option<i32> {
        self.post.as_ref().map(|p| p.topic_id)
    }

    fn attempted(&self) -> bool {
        store::truthy(&self.data, "attempted")
    }

    async fn enqueue_timeout(&self) -> Result<(), AppError> {
        bot::enqueue_timeout_job(
            &mut *self.bot.pool.acquire().await?,
            self.user.id,
            self.track.class_name(),
        )
        .await
    }

    async fn user_guardian(&mut self) -> Result<&Guardian, AppError> {
        if self.user_guardian.is_none() {
            let mut conn = self.bot.pool.acquire().await?;
            let user = SessionUser::load(&mut conn, self.user.id)
                .await?
                .ok_or_else(|| BotError("the user is gone".into()))?;
            self.user_guardian = Some(Guardian::for_user(&mut conn, &user).await?);
        }
        Ok(self.user_guardian.as_ref().expect("loaded"))
    }

    async fn prerequisite(&mut self, p: Prereq) -> Result<bool, AppError> {
        let s = self.bot.ctx.settings;
        Ok(match p {
            Prereq::EnableEmoji => s.get("enable_emoji")?.truthy(),
            Prereq::EnableMentions => s.get("enable_mentions")?.truthy(),
            Prereq::AllowFlaggingStaff => s.get("allow_flagging_staff")?.truthy(),
            Prereq::EmbeddedMedia => self.embedded_media().await?,
            Prereq::NoEmbeddedMedia => !self.embedded_media().await?,
            Prereq::Poll => {
                s.get("poll_enabled")?.truthy()
                    && self
                        .user_guardian()
                        .await?
                        .in_setting_groups(s, "poll_create_allowed_groups")?
            }
        })
    }

    /// `@user.in_any_groups?(SiteSetting.embedded_media_post_allowed_groups_map)`
    async fn embedded_media(&mut self) -> Result<bool, AppError> {
        let s = self.bot.ctx.settings;
        Ok(self
            .user_guardian()
            .await?
            .in_setting_groups(s, "embedded_media_post_allowed_groups")?)
    }

    /// `instance_eval(&@next_instructions)`
    async fn instructions(&mut self) -> Result<String, AppError> {
        let section = self
            .next_instructions
            .ok_or_else(|| BotError("no instructions to give".into()))?;
        let key = format!("{}.{section}.instructions", self.track.key());
        let bot = self.bot;
        match (self.track, section) {
            (Track::NewUser, "mention") => bot.t_with(
                &key,
                &[
                    ("discobot_username", &bot.discobot_username()),
                    ("base_uri", bot.base_path()),
                ],
            ),
            (Track::NewUser, "flag") => {
                let base = bot.urls().base_url()?;
                bot.t_with(
                    &key,
                    &[
                        ("guidelines_url", &format!("{base}/guidelines")),
                        ("about_url", &format!("{base}/about")),
                        ("base_uri", bot.base_path()),
                    ],
                )
            }
            (Track::Advanced, "category_hashtag") => {
                let category = self.last_secured_category().await?;
                bot.t_post(&key, &[("category", &format!("#{category}"))])
            }
            _ => bot.t_post(&key, &[]),
        }
    }

    /// `Category.secured(@user.guardian).last.slug_ref`
    async fn last_secured_category(&mut self) -> Result<String, AppError> {
        let s = self.bot.ctx.settings;
        let pool = self.bot.pool;
        let secure = {
            let guardian = self.user_guardian().await?.clone();
            let mut conn = pool.acquire().await?;
            guardian.secure_category_ids(&mut conn, s).await?
        };
        let row: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT c.slug, p.slug FROM categories c \
             LEFT JOIN categories p ON p.id = c.parent_category_id \
             WHERE NOT c.read_restricted OR c.id = ANY($1) ORDER BY c.id DESC LIMIT 1",
        )
        .bind(&secure)
        .fetch_optional(&mut *pool.acquire().await?)
        .await?;
        match row {
            Some((slug, Some(parent))) => Ok(format!("{parent}:{slug}")),
            Some((slug, None)) => Ok(slug),
            None => Err(BotError("no category the user can see".into()).into()),
        }
    }

    /// "#{reply}\n\n#{instructions}\n", a heredoc's two paragraphs.
    async fn with_instructions(&mut self, text: String) -> Result<String, AppError> {
        Ok(format!("{text}\n\n{}\n", self.instructions().await?))
    }

    async fn reply(&self, raw: String) -> Result<PostRow, AppError> {
        self.bot
            .reply_to(self.post.as_ref(), raw, ReplyOpts::default(), true)
            .await
    }

    /// `skip_tutorial(next_state)`
    async fn skip_tutorial(&mut self, next_state: &str) -> Result<Option<PostRow>, AppError> {
        if !self.valid_topic(self.post_topic_id()) {
            return Ok(None);
        }
        self.bot.fake_delay().await;
        if next_state != "end" {
            let raw = self.instructions().await?;
            let reply = self.reply(raw).await?;
            self.enqueue_timeout().await?;
            Ok(Some(reply))
        } else {
            Ok(self.post.clone())
        }
    }

    async fn clean_up_state(&mut self, state: &str) -> Result<(), AppError> {
        if self.track == Track::NewUser && state == "tutorial_search" {
            self.clean_up_tutorial_search().await?;
        }
        Ok(())
    }

    async fn init_state(&mut self, state: &str) -> Result<(), AppError> {
        match (self.track, state) {
            (Track::NewUser, "tutorial_search") => self.init_tutorial_search().await,
            (Track::Advanced, "tutorial_edit") => self.init_tutorial_edit().await,
            (Track::Advanced, "tutorial_recover") => self.init_tutorial_recover().await,
            _ => Ok(()),
        }
    }

    async fn perform(&mut self, action: Action) -> Result<Option<PostRow>, AppError> {
        use Action::*;
        match action {
            SayHello => self.say_hello().await.map(Some),
            ReplyToBookmark => self.reply_to_bookmark().await,
            MissingBookmark => self.missing_bookmark().await,
            ReplyToOnebox => self.reply_to_onebox().await,
            ReplyToEmoji => self.reply_if_found("emoji", ".emoji").await,
            ReplyToMention => self.reply_to_mention().await,
            ReplyToFormatting => {
                self.reply_if_found("formatting", "b, strong, em, i, .bbcode-i, .bbcode-b")
                    .await
            }
            ReplyToQuote => self.reply_if_found("quoting", ".quote").await,
            ReplyToImage => self.reply_to_image().await,
            TrackImagesLike => self.track_images_like().await,
            ReplyToLikes => self.reply_to_likes().await,
            MissingLikesLike => self.missing_likes_like().await,
            ReplyToFlag => self.reply_to_flag().await,
            MissingFlag => self.missing_flag().await,
            ReplyToSearch => self.reply_to_search().await,
            StartAdvancedTrack => self.start_advanced_track().await.map(Some),
            ReplyToEdit => self.reply_to_edit().await,
            MissingEdit => self.missing_edit().await,
            ReplyToDelete => self.reply_to_delete().await,
            MissingDelete => self.missing_reply("delete").await,
            ReplyToRecover => self.reply_to_recover().await,
            MissingRecover => self.missing_recover().await,
            ReplyToCategoryHashtag => {
                self.reply_if_found("category_hashtag", ".hashtag-cooked")
                    .await
            }
            ReplyToTopicNotificationLevelChanged => {
                self.reply_to_topic_notification_level_changed().await
            }
            MissingTopicNotificationLevelChange => {
                self.missing_reply("change_topic_notification_level").await
            }
            ReplyToPoll => self.reply_if_found("poll", ".poll").await,
            ReplyToDetails => self.reply_to_details().await,
        }
    }

    /// `say_hello`: the welcome message with the first instructions, in a
    /// new system message to the user (or the message they are in).
    async fn say_hello(&mut self) -> Result<PostRow, AppError> {
        let bot = self.bot;
        let s = bot.ctx.settings;
        let site_title = s.get("title")?.to_s();
        let hello = bot.t_post(
            "new_user_narrative.hello.message",
            &[("username", &self.user.username), ("title", &site_title)],
        )?;
        let raw = self.with_instructions(hello).await?;
        let mut title = bot.t_with("new_user_narrative.hello.title", &[("title", &site_title)])?;
        if s.get("max_emojis_in_title")?.to_i() == 0 {
            title = strip_emoji_codes(&title).trim().to_string();
        }
        let mut opts = ReplyOpts {
            topic_id: None,
            title: Some(title),
            target_username: Some(self.user.username.clone()),
            subtype: Some("system_message".to_string()),
        };
        if let Some(post) = &self.post
            && self.in_message(post.topic_id).await?
        {
            opts.topic_id = Some(post.topic_id);
        }
        if let Some(topic_id) = store::int_of(&self.data, "topic_id") {
            opts.topic_id = Some(topic_id as i32);
            opts.title = None;
            opts.target_username = None;
        }
        let post = bot.reply_to(self.post.as_ref(), raw, opts, true).await?;
        self.data.insert("topic_id".into(), json!(post.topic_id));
        self.data
            .insert("track".into(), json!(self.track.class_name()));
        Ok(post)
    }

    /// `@post.topic.private_message? && topic_allowed_users include the
    /// user`
    async fn in_message(&self, topic_id: i32) -> Result<bool, AppError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topics t JOIN topic_allowed_users a ON a.topic_id = t.id \
             WHERE t.id = $1 AND t.archetype = 'private_message' AND a.user_id = $2)",
        )
        .bind(topic_id)
        .bind(self.user.id)
        .fetch_one(&mut *self.bot.pool.acquire().await?)
        .await?)
    }

    async fn missing_bookmark(&mut self) -> Result<Option<PostRow>, AppError> {
        let post = self.post()?;
        if !self.valid_topic(Some(post.topic_id)) || post.user_id == Some(BOT_USER_ID) {
            return Ok(None);
        }
        self.bot.fake_delay().await;
        self.enqueue_timeout().await?;
        if !self.attempted() {
            let raw = self
                .bot
                .t_post("new_user_narrative.bookmark.not_found", &[])?;
            self.reply(raw).await?;
        }
        Ok(None)
    }

    async fn reply_to_bookmark(&mut self) -> Result<Option<PostRow>, AppError> {
        let post = self.post()?;
        if !self.valid_topic(Some(post.topic_id)) || post.user_id != Some(BOT_USER_ID) {
            return Ok(None);
        }
        // url_helpers(:user_url, username:)
        let bookmark_url = format!(
            "{}/u/{}/activity/bookmarks",
            self.bot.urls().base_url()?,
            self.user.username
        );
        let text = self.bot.t_post(
            "new_user_narrative.bookmark.reply",
            &[("bookmark_url", &bookmark_url)],
        )?;
        let raw = self.with_instructions(text).await?;
        self.bot.fake_delay().await;
        let reply = self.reply(raw).await?;
        self.enqueue_timeout().await?;
        Ok(Some(reply))
    }

    async fn reply_to_onebox(&mut self) -> Result<Option<PostRow>, AppError> {
        let post = self.post()?;
        if !self.valid_topic(Some(post.topic_id)) {
            return Ok(None);
        }
        // Brittle if the instructions' site text changes, or the user
        // drops some other link; the reference accepts that.
        static LINK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(
                r"https://en\.wikipedia\.org/wiki/(Inherently_funny_word|Death_by_coconut|Calculator_spelling|Exotic_Shorthair)",
            )
            .expect("a valid regex")
        });
        if LINK.is_match(&post.raw) {
            let text = self.bot.t_post("new_user_narrative.onebox.reply", &[])?;
            let raw = self.with_instructions(text).await?;
            self.bot.fake_delay().await;
            let reply = self.reply(raw).await?;
            self.enqueue_timeout().await?;
            Ok(Some(reply))
        } else {
            self.bot.fake_delay().await;
            if !self.attempted() {
                let raw = self
                    .bot
                    .t_post("new_user_narrative.onebox.not_found", &[])?;
                self.reply(raw).await?;
            }
            self.enqueue_timeout().await?;
            Ok(None)
        }
    }

    /// The tutorials that look for something in the cooked reply
    /// (`reply_to_emoji`, `_formatting`, `_quote`, `_category_hashtag`,
    /// `_poll`): its reply and the next instructions when found, else the
    /// reminder (once).
    async fn reply_if_found(
        &mut self,
        section: &str,
        selector: &str,
    ) -> Result<Option<PostRow>, AppError> {
        let post = self.post()?;
        if !self.valid_topic(Some(post.topic_id)) {
            return Ok(None);
        }
        let key = self.track.key();
        if css_count(&post.cooked, selector) > 0 {
            let text = self.bot.t_post(&format!("{key}.{section}.reply"), &[])?;
            let raw = self.with_instructions(text).await?;
            self.bot.fake_delay().await;
            let reply = self.reply(raw).await?;
            // The new user tutorial's replies set the timeout; the
            // advanced one's (hashtag, poll) do not.
            if self.track == Track::NewUser {
                self.enqueue_timeout().await?;
            }
            Ok(Some(reply))
        } else {
            self.bot.fake_delay().await;
            if !self.attempted() {
                let raw = self
                    .bot
                    .t_post(&format!("{key}.{section}.not_found"), &[])?;
                self.reply(raw).await?;
            }
            self.enqueue_timeout().await?;
            Ok(None)
        }
    }

    async fn reply_to_mention(&mut self) -> Result<Option<PostRow>, AppError> {
        let post = self.post()?;
        if !self.valid_topic(Some(post.topic_id)) {
            return Ok(None);
        }
        if self.bot.bot_mentioned_in_cooked(&post.cooked)? {
            let text = self.bot.t_post("new_user_narrative.mention.reply", &[])?;
            let raw = self.with_instructions(text).await?;
            self.bot.fake_delay().await;
            let reply = self.reply(raw).await?;
            self.enqueue_timeout().await?;
            Ok(Some(reply))
        } else {
            self.bot.fake_delay().await;
            if !self.attempted() {
                let raw = self.bot.t_post(
                    "new_user_narrative.mention.not_found",
                    &[
                        ("username", &self.user.username),
                        ("discobot_username", &self.bot.discobot_username()),
                    ],
                )?;
                self.reply(raw).await?;
            }
            self.enqueue_timeout().await?;
            Ok(None)
        }
    }

    /// `PostAction.exists?(like on data[:last_post_id] by the user)`
    async fn last_post_liked(&self) -> Result<bool, AppError> {
        let Some(last_post_id) = store::int_of(&self.data, "last_post_id") else {
            return Ok(false);
        };
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM post_actions WHERE post_action_type_id = 2 \
               AND post_id = $1 AND user_id = $2 AND deleted_at IS NULL)",
        )
        .bind(last_post_id as i32)
        .bind(self.user.id)
        .fetch_one(&mut *self.bot.pool.acquire().await?)
        .await?)
    }

    async fn track_images_like(&mut self) -> Result<Option<PostRow>, AppError> {
        if !self.valid_topic(self.post_topic_id()) {
            return Ok(None);
        }
        if !self.last_post_liked().await? {
            return Ok(None);
        }
        self.set_state_data("liked", json!(true)).await?;
        let Some(post_id) = self.get_state_data("post_id").and_then(Value::as_i64) else {
            return Ok(None);
        };
        let Some(image_post) =
            bot::find_post(&mut *self.bot.pool.acquire().await?, post_id).await?
        else {
            return Ok(None);
        };
        self.bot.fake_delay().await;
        self.bot.like(image_post.id).await?;
        let text = self.bot.t_post("new_user_narrative.images.reply", &[])?;
        let raw = self.with_instructions(text).await?;
        let reply = self.reply(raw).await?;
        self.enqueue_timeout().await?;
        Ok(Some(reply))
    }

    async fn reply_to_image(&mut self) -> Result<Option<PostRow>, AppError> {
        let post = self.post()?.clone();
        if !self.valid_topic(Some(post.topic_id)) {
            return Ok(None);
        }
        let mut transition = true;
        let attempted_count = self
            .get_state_data("attempted")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if attempted_count < 2 {
            self.data.insert("skip_attempted".into(), json!(true));
            self.data.insert("attempted".into(), json!(false));
        } else {
            self.data.insert("skip_attempted".into(), json!(false));
        }
        // post_analyzer.cook(raw): the raw cooked again.
        let cooked = crate::pretty_text::cook(
            self.bot.ctx.host,
            &post.raw,
            &crate::pretty_text::MarkdownOptions::default(),
        )
        .await?;
        let raw = if css_count(&cooked, "img") > 0 {
            self.set_state_data("post_id", json!(post.id)).await?;
            if self
                .get_state_data("liked")
                .is_some_and(|v| v != &json!(false))
            {
                self.bot.like(post.id).await?;
                let text = self.bot.t_post("new_user_narrative.images.reply", &[])?;
                self.with_instructions(text).await?
            } else {
                let last_post_id = store::int_of(&self.data, "last_post_id").unwrap_or_default();
                let url = bot::post_url(&mut *self.bot.pool.acquire().await?, last_post_id).await?;
                transition = false;
                self.bot
                    .t_post("new_user_narrative.images.like_not_found", &[("url", &url)])?
            }
        } else {
            let image_url = format!(
                "{}/plugins/discourse-narrative-bot/images/dog-walk.gif",
                self.bot.urls().base_url()?
            );
            transition = false;
            self.bot.t_post(
                "new_user_narrative.images.not_found",
                &[("image_url", &image_url)],
            )?
        };
        self.bot.fake_delay().await;
        if !transition {
            self.set_state_data("attempted", json!(attempted_count + 1))
                .await?;
        }
        let reply = if self.attempted() && !transition {
            None
        } else {
            Some(self.reply(raw).await?)
        };
        self.enqueue_timeout().await?;
        Ok(if transition { reply } else { None })
    }

    async fn missing_likes_like(&mut self) -> Result<Option<PostRow>, AppError> {
        let post = self.post()?;
        if !self.valid_topic(Some(post.topic_id)) || post.user_id == Some(BOT_USER_ID) {
            return Ok(None);
        }
        self.bot.fake_delay().await;
        self.enqueue_timeout().await?;
        let last_post_id = store::int_of(&self.data, "last_post_id").unwrap_or_default();
        let url = bot::post_url(&mut *self.bot.pool.acquire().await?, last_post_id).await?;
        let raw = self
            .bot
            .t_post("new_user_narrative.likes.not_found", &[("url", &url)])?;
        self.reply(raw).await?;
        Ok(None)
    }

    async fn reply_to_likes(&mut self) -> Result<Option<PostRow>, AppError> {
        if !self.valid_topic(self.post_topic_id()) {
            return Ok(None);
        }
        if !self.last_post_liked().await? {
            return Ok(None);
        }
        let text = self.bot.t_post("new_user_narrative.likes.reply", &[])?;
        let raw = self.with_instructions(text).await?;
        self.bot.fake_delay().await;
        let reply = self.reply(raw).await?;
        self.enqueue_timeout().await?;
        Ok(Some(reply))
    }

    async fn missing_flag(&mut self) -> Result<Option<PostRow>, AppError> {
        let post = self.post()?.clone();
        if !self.valid_topic(Some(post.topic_id)) {
            return Ok(None);
        }
        // Wrong flags on discobot's post go, so the user can try again.
        if post.user_id == Some(BOT_USER_ID) {
            sqlx::query(
                "DELETE FROM post_actions WHERE post_id = $1 AND user_id = $2 \
                   AND post_action_type_id IN (SELECT id FROM flags WHERE NOT score_type AND id NOT IN ($3, $4))",
            )
            .bind(post.id)
            .bind(self.user.id)
            .bind(super::LIKE)
            .bind(super::INAPPROPRIATE)
            .execute(&mut *self.bot.pool.acquire().await?)
            .await?;
        }
        self.bot.fake_delay().await;
        if !self.attempted() {
            let raw = self.bot.t_post("new_user_narrative.flag.not_found", &[])?;
            self.reply(raw).await?;
        }
        Ok(None)
    }

    async fn reply_to_flag(&mut self) -> Result<Option<PostRow>, AppError> {
        let post = self.post()?.clone();
        if !self.valid_topic(Some(post.topic_id)) || post.user_id != Some(BOT_USER_ID) {
            return Ok(None);
        }
        // Group.find(AUTO_GROUPS[:staff]).full_url
        let staff: String = sqlx::query_scalar("SELECT name FROM groups WHERE id = 3")
            .fetch_one(&mut *self.bot.pool.acquire().await?)
            .await?;
        let group_url = format!(
            "{}/g/{}",
            self.bot.urls().base_url()?,
            crate::pretty_text::cleanup::encode_component(&staff)
        );
        let text = self.bot.t_post(
            "new_user_narrative.flag.reply",
            &[("group_url", &group_url)],
        )?;
        let raw = self.with_instructions(text).await?;
        self.bot.fake_delay().await;
        let reply = self.reply(raw).await?;
        // @post.post_actions.where(user_id:).destroy_all
        sqlx::query("DELETE FROM post_actions WHERE post_id = $1 AND user_id = $2")
            .bind(post.id)
            .bind(self.user.id)
            .execute(&mut *self.bot.pool.acquire().await?)
            .await?;
        self.enqueue_timeout().await?;
        Ok(Some(reply))
    }

    async fn reply_to_search(&mut self) -> Result<Option<PostRow>, AppError> {
        let post = self.post()?;
        if !self.valid_topic(Some(post.topic_id)) {
            return Ok(None);
        }
        if post.raw.contains(SEARCH_ANSWER) || post.raw.contains(SEARCH_ANSWER_EMOJI) {
            self.bot.fake_delay().await;
            let search_url = format!("{}/search", self.bot.urls().base_url()?);
            let raw = self.bot.t_post(
                "new_user_narrative.search.reply",
                &[("search_url", &search_url)],
            )?;
            Ok(Some(self.reply(raw).await?))
        } else {
            self.bot.fake_delay().await;
            if !self.attempted() {
                let raw = self
                    .bot
                    .t_post("new_user_narrative.search.not_found", &[])?;
                self.reply(raw).await?;
            }
            self.enqueue_timeout().await?;
            Ok(None)
        }
    }

    /// `init_tutorial_search`: the search answer hidden in the message's
    /// first post, through a revision discobot makes.
    async fn init_tutorial_search(&mut self) -> Result<(), AppError> {
        let topic_id = self.post()?.topic_id;
        let first = bot::first_post(&mut *self.bot.pool.acquire().await?, topic_id).await?;
        let hidden = self.bot.t_post(
            "new_user_narrative.search.hidden_message",
            &[("search_answer", SEARCH_ANSWER)],
        )?;
        let raw = format!("{}\n\n{hidden}\n", first.raw);
        Box::pin(crate::posting::revise::revise(
            self.bot.pool,
            self.bot.ctx,
            &self.bot.guardian,
            first.id,
            crate::posting::revise::Changes {
                raw: Some(raw),
                force_new_version: true,
                skip_validations: true,
                ..Default::default()
            },
        ))
        .await?;
        let version: Option<i32> = sqlx::query_scalar("SELECT version FROM posts WHERE id = $1")
            .bind(first.id)
            .fetch_optional(&mut *self.bot.pool.acquire().await?)
            .await?;
        self.set_state_data("post_version", json!(version.unwrap_or(0)))
            .await
    }

    /// `clean_up_tutorial_search`: the first post reverted to before the
    /// hidden answer (`revert_to(post_version - 1)`, `save!`, then
    /// `publish_change_to_clients!(:revised)`).
    async fn clean_up_tutorial_search(&mut self) -> Result<(), AppError> {
        let topic_id = self.post()?.topic_id;
        let pool = self.bot.pool;
        let first = bot::first_post(&mut *pool.acquire().await?, topic_id).await?;
        let version = self
            .get_state_data("post_version")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let number = version - 1;
        let current: i32 = sqlx::query_scalar("SELECT version FROM posts WHERE id = $1")
            .bind(first.id)
            .fetch_one(&mut *pool.acquire().await?)
            .await?;
        if number < i64::from(current) {
            let modifications: Option<Option<String>> = sqlx::query_scalar(
                "SELECT modifications FROM post_revisions WHERE post_id = $1 AND number = $2",
            )
            .bind(first.id)
            .bind((number + 1) as i32)
            .fetch_optional(&mut *pool.acquire().await?)
            .await?;
            let modifications = modifications
                .flatten()
                .ok_or_else(|| BotError("the revision to revert is gone".into()))?;
            let mut raw = first.raw.clone();
            for (attribute, change) in crate::modifications::load(&modifications)? {
                match attribute.as_str() {
                    "raw" => {
                        raw = change[0]
                            .as_str()
                            .ok_or(Unsupported("reverting to a raw that is not a string"))?
                            .to_string();
                    }
                    // Post's before_save cooks the raw again.
                    "cooked" => {}
                    _ => {
                        return Err(Unsupported(
                            "reverting a revision of more than the raw and cooked",
                        )
                        .into());
                    }
                }
            }
            if raw != first.raw {
                let cooked = crate::pretty_text::cook(
                    self.bot.ctx.host,
                    &raw,
                    &crate::pretty_text::MarkdownOptions {
                        topic_id: Some(i64::from(first.topic_id)),
                        ..Default::default()
                    },
                )
                .await?;
                sqlx::query(
                    "UPDATE posts SET raw = $2, cooked = $3, baked_at = clock_timestamp(), \
                            baked_version = $4, updated_at = clock_timestamp() WHERE id = $1",
                )
                .bind(first.id)
                .bind(&raw)
                .bind(&cooked)
                .bind(crate::posting::BAKED_VERSION)
                .execute(&mut *pool.acquire().await?)
                .await?;
            }
        }
        let mut conn = pool.acquire().await?;
        crate::bus::publish_post_change(
            self.bot.ctx,
            &mut conn,
            first.id,
            "revised",
            Default::default(),
            false,
        )
        .await?;
        Ok(())
    }

    async fn end_reply(&mut self) -> Result<(), AppError> {
        self.bot.fake_delay().await;
        let bot = self.bot;
        match self.track {
            Track::NewUser => {
                let certificate = self.certificate(None)?;
                let raw = bot.t_post(
                    "new_user_narrative.end.message",
                    &[
                        ("username", &self.user.username),
                        ("base_url", &bot.urls().base_url()?),
                        ("certificate", &certificate),
                        ("discobot_username", &bot.discobot_username()),
                        ("advanced_trigger", &Track::Advanced.reset_trigger(bot)?),
                    ],
                )?;
                let opts = ReplyOpts {
                    topic_id: store::int_of(&self.data, "topic_id").map(|t| t as i32),
                    ..Default::default()
                };
                bot.reply_to(self.post.as_ref(), raw, opts, true).await?;
            }
            Track::Advanced => {
                let certificate = self.certificate(Some("advanced"))?;
                let raw = bot.t_post(
                    "advanced_user_narrative.end.message",
                    &[("certificate", &certificate)],
                )?;
                self.reply(raw).await?;
            }
        }
        Ok(())
    }

    /// `certificate(type)`: the iframe drawing the user's certificate.
    fn certificate(&self, kind: Option<&str>) -> Result<String, AppError> {
        let date = Utc::now().format("%b %d %Y").to_string();
        // certificate_path(options): the query's keys sorted (to_query).
        let mut query = format!("date={}", form_escape(&date));
        if let Some(kind) = kind {
            query.push_str(&format!("&type={}", form_escape(kind)));
        }
        query.push_str(&format!("&user_id={}", self.user.id));
        let src = format!(
            "{}/discobot/certificate.svg?{query}",
            self.bot.urls().base_url()?
        );
        let alt = escape_html(
            &self
                .bot
                .t(&format!("{}.certificate.alt", self.track.key()))?,
        );
        Ok(format!(
            "<iframe class='discobot-certificate' src='{src}' width='650' height='464' alt='{alt}'></iframe>"
        ))
    }

    /// `start_advanced_track`
    async fn start_advanced_track(&mut self) -> Result<PostRow, AppError> {
        let bot = self.bot;
        let start = bot.t_post(
            "advanced_user_narrative.start_message",
            &[("username", &self.user.username)],
        )?;
        let raw = self.with_instructions(start).await?;
        let mut opts = ReplyOpts {
            topic_id: None,
            title: Some(bot.t("advanced_user_narrative.title")?),
            target_username: Some(self.user.username.clone()),
            subtype: None,
        };
        if let Some(topic_id) = store::int_of(&self.data, "topic_id") {
            opts.topic_id = Some(topic_id as i32);
            opts.title = None;
            opts.target_username = None;
        }
        let post = bot.reply_to(self.post.as_ref(), raw, opts, true).await?;
        self.data.insert("topic_id".into(), json!(post.topic_id));
        self.data
            .insert("track".into(), json!(self.track.class_name()));
        Ok(post)
    }

    /// A post the user makes in the tutorial (`PostCreator.create!(@user,
    /// raw:, topic_id:, skip_bot: true, skip_validations: true)`).
    async fn post_as_user(&mut self, raw: String) -> Result<i32, AppError> {
        let mut conn = self.bot.pool.acquire().await?;
        let data = store::get(&mut conn, self.user.id)
            .await?
            .unwrap_or_default();
        drop(conn);
        let guardian = self.user_guardian().await?.clone();
        self.bot
            .create_as(
                &guardian,
                NewPost {
                    raw,
                    topic_id: store::int_of(&data, "topic_id").map(|t| t as i32),
                    skip_validations: true,
                    skip_bot: true,
                    ..Default::default()
                },
            )
            .await
    }

    /// `init_tutorial_edit`: a post of the user's to edit.
    async fn init_tutorial_edit(&mut self) -> Result<(), AppError> {
        self.bot.fake_delay().await;
        let raw = self.bot.t_post(
            "advanced_user_narrative.edit.bot_created_post_raw",
            &[("discobot_username", &self.bot.discobot_username())],
        )?;
        let post_id = self.post_as_user(raw).await?;
        self.set_state_data("post_id", json!(post_id)).await
    }

    /// `init_tutorial_recover`: a post of the user's, deleted by them for
    /// them to recover.
    async fn init_tutorial_recover(&mut self) -> Result<(), AppError> {
        let raw = self.bot.t_post(
            "advanced_user_narrative.recover.deleted_post_raw",
            &[("discobot_username", &self.bot.discobot_username())],
        )?;
        let post_id = self.post_as_user(raw).await?;
        self.set_state_data("post_id", json!(post_id)).await?;
        if self
            .bot
            .ctx
            .settings
            .get("delete_removed_posts_after")?
            .to_i()
            < 1
        {
            // discobot flags the post to moderators and ignores the
            // review, so the author's delete only marks it.
            return Err(Unsupported(
                "the recover tutorial with delete_removed_posts_after below 1 (flags in messages)",
            )
            .into());
        }
        Box::pin(crate::post_destroyer::destroy_as(
            self.bot.pool,
            self.bot.ctx,
            self.user.id,
            post_id,
            "",
        ))
        .await?;
        // skip_bot: the destroyer's post_destroyed is not the tutorial's.
        sqlx::query(
            "DELETE FROM discourse_rs.jobs WHERE name = $1 AND locked_until IS NULL \
             AND args = $2",
        )
        .bind(jobs::BOT_INPUT)
        .bind(json!({
            "user_id": self.user.id,
            "post_id": post_id,
            "topic_id": store::int_of(&self.data, "topic_id"),
            "input": "delete",
        }))
        .execute(&mut *self.bot.pool.acquire().await?)
        .await?;
        Ok(())
    }

    async fn reply_to_edit(&mut self) -> Result<Option<PostRow>, AppError> {
        if !self.valid_topic(self.post_topic_id()) {
            return Ok(None);
        }
        self.bot.fake_delay().await;
        let text = self.bot.t_post("advanced_user_narrative.edit.reply", &[])?;
        let raw = self.with_instructions(text).await?;
        Ok(Some(self.reply(raw).await?))
    }

    async fn missing_edit(&mut self) -> Result<Option<PostRow>, AppError> {
        let post = self.post()?;
        let post_id = self.get_state_data("post_id").and_then(Value::as_i64);
        if !self.valid_topic(Some(post.topic_id)) || post_id == Some(i64::from(post.id)) {
            return Ok(None);
        }
        self.bot.fake_delay().await;
        if !self.attempted() {
            let url = bot::post_url(
                &mut *self.bot.pool.acquire().await?,
                post_id.unwrap_or_default(),
            )
            .await?;
            let raw = self
                .bot
                .t_post("advanced_user_narrative.edit.not_found", &[("url", &url)])?;
            self.reply(raw).await?;
        }
        self.enqueue_timeout().await?;
        Ok(None)
    }

    /// A discobot post in the tutorial's message
    /// (`PostCreator.create!(discobot_user, raw:, topic_id:)`).
    async fn post_in_topic(&self, raw: String, topic_id: Option<i32>) -> Result<PostRow, AppError> {
        let post_id = self
            .bot
            .create_as(
                &self.bot.guardian,
                NewPost {
                    raw,
                    topic_id,
                    ..Default::default()
                },
            )
            .await?;
        bot::find_post(&mut *self.bot.pool.acquire().await?, i64::from(post_id))
            .await?
            .ok_or_else(|| BotError("discobot's post vanished".into()).into())
    }

    async fn reply_to_delete(&mut self) -> Result<Option<PostRow>, AppError> {
        if !self.valid_topic(self.topic_id) {
            return Ok(None);
        }
        self.bot.fake_delay().await;
        let text = self
            .bot
            .t_post("advanced_user_narrative.delete.reply", &[])?;
        let raw = self.with_instructions(text).await?;
        Ok(Some(self.post_in_topic(raw, self.topic_id).await?))
    }

    /// `missing_delete`, `missing_topic_notification_level_change`: the
    /// reminder (once) and the timeout.
    async fn missing_reply(&mut self, section: &str) -> Result<Option<PostRow>, AppError> {
        if !self.valid_topic(self.post_topic_id()) {
            return Ok(None);
        }
        self.bot.fake_delay().await;
        if !self.attempted() {
            let raw = self
                .bot
                .t_post(&format!("advanced_user_narrative.{section}.not_found"), &[])?;
            self.reply(raw).await?;
        }
        self.enqueue_timeout().await?;
        Ok(None)
    }

    async fn reply_to_recover(&mut self) -> Result<Option<PostRow>, AppError> {
        if !self.valid_topic(self.post_topic_id()) {
            return Ok(None);
        }
        self.bot.fake_delay().await;
        let after = self
            .bot
            .ctx
            .settings
            .get("delete_removed_posts_after")?
            .to_s();
        let text = self.bot.t_post(
            "advanced_user_narrative.recover.reply",
            &[("deletion_after", &after)],
        )?;
        let raw = self.with_instructions(text).await?;
        Ok(Some(self.post_in_topic(raw, self.post_topic_id()).await?))
    }

    async fn missing_recover(&mut self) -> Result<Option<PostRow>, AppError> {
        // `post_id = get_state_data(:post_id) && @post.id != post_id`
        // assigns the whole && (the right post_id still nil): only whether
        // there is a post to recover counts.
        if !self.valid_topic(self.post_topic_id()) || self.get_state_data("post_id").is_none() {
            return Ok(None);
        }
        self.missing_reply("recover").await
    }

    async fn reply_to_topic_notification_level_changed(
        &mut self,
    ) -> Result<Option<PostRow>, AppError> {
        if !self.valid_topic(self.topic_id) {
            return Ok(None);
        }
        self.bot.fake_delay().await;
        let text = self.bot.t_post(
            "advanced_user_narrative.change_topic_notification_level.reply",
            &[],
        )?;
        let raw = self.with_instructions(text).await?;
        self.bot.fake_delay().await;
        let post = self.post_in_topic(raw, self.topic_id).await?;
        self.enqueue_timeout().await?;
        Ok(Some(post))
    }

    async fn reply_to_details(&mut self) -> Result<Option<PostRow>, AppError> {
        let post = self.post()?;
        if !self.valid_topic(Some(post.topic_id)) {
            return Ok(None);
        }
        self.bot.fake_delay().await;
        if css_count(&post.cooked, "details") > 0 {
            let raw = self
                .bot
                .t_post("advanced_user_narrative.details.reply", &[])?;
            Ok(Some(self.reply(raw).await?))
        } else {
            if !self.attempted() {
                let raw = self
                    .bot
                    .t_post("advanced_user_narrative.details.not_found", &[])?;
                self.reply(raw).await?;
            }
            self.enqueue_timeout().await?;
            Ok(None)
        }
    }
}

/// How many elements of the cooked HTML match the selector.
pub fn css_count(cooked: &str, selector: &str) -> usize {
    let dom = crate::pretty_text::cleanup::parse(cooked);
    let root = crate::pretty_text::cleanup::fragment_root(&dom);
    crate::pretty_text::select::select(&root, selector).len()
}

/// `title.gsub(Emoji::EMOJI_CODE_REGEXP, "")`
fn strip_emoji_codes(title: &str) -> String {
    static CODE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r":([\w\-+]+(?::t\d)?):").expect("a valid regex")
    });
    CODE.replace_all(title, "").into_owned()
}

/// `CGI.escapeHTML`
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// `CGI.escape`, as `to_query` writes values.
fn form_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
