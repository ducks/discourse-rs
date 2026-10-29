//! `DiscourseNarrativeBot::TrackSelector`: what an input means. In the
//! user's tutorial message it goes to their track (or resets, skips or
//! ends it); anywhere discobot is mentioned or replied to it is a command
//! (`@discobot roll 2d6`, `quote`, `fortune`, `display help`, `start
//! tutorial`); otherwise nothing. Also the commands' answers (`Dice`,
//! `QuoteGenerator`, `Magic8Ball`).

use rand::Rng;
use serde_json::{Value, json};

use super::bot::{self, Bot, BotError, PostRow, ReplyOpts, User};
use super::narrative::{self, Track};
use super::store;
use crate::AppError;

/// `GENERIC_REPLIES_COUNT_PREFIX`
pub const GENERIC_REPLIES_COUNT_PREFIX: &str = "discourse-narrative-bot:track-selector-count:";
/// `PUBLIC_DISPLAY_BOT_HELP_KEY`
pub const PUBLIC_DISPLAY_BOT_HELP_KEY: &str =
    "discourse-narrative-bot:track-selector:display-bot-help";
/// `RESET_TRIGGER_EXACT_MATCH_LENGTH`
pub const RESET_TRIGGER_EXACT_MATCH_LENGTH: usize = 200;
/// `TRACKS`, in the order a reset tries them.
const TRACKS: [Track; 2] = [Track::Advanced, Track::NewUser];
/// `Dice::MAXIMUM_NUM_OF_DICE` and `MAXIMUM_RANGE_OF_DICE`.
pub const MAXIMUM_NUM_OF_DICE: i64 = 20;
pub const MAXIMUM_RANGE_OF_DICE: i64 = 120;
/// `Post.types[:regular]`
const REGULAR: i32 = 1;

/// `TrackSelector.new(input, user, post_id:, topic_id:).select`
pub async fn select(
    bot: &Bot<'_>,
    input: &str,
    user: &User,
    post_id: Option<i64>,
    topic_id: Option<i32>,
) -> Result<(), AppError> {
    let post = match post_id {
        Some(id) => bot::find_post(&mut *bot.pool.acquire().await?, id).await?,
        None => None,
    };
    let mut selector = Selector {
        bot,
        input,
        user,
        post,
        topic_id,
        is_pm_to_bot: false,
        bot_mentioned: None,
    };
    Box::pin(selector.select()).await
}

struct Selector<'s, 'a> {
    bot: &'s Bot<'a>,
    input: &'s str,
    user: &'s User,
    post: Option<PostRow>,
    topic_id: Option<i32>,
    is_pm_to_bot: bool,
    bot_mentioned: Option<bool>,
}

fn track_of(data: &store::Data) -> Result<Track, AppError> {
    let name = store::str_of(data, "track").unwrap_or(Track::NewUser.class_name());
    Track::from_class_name(name).ok_or_else(|| BotError(format!("an unknown track {name}")).into())
}

impl Selector<'_, '_> {
    fn is_topic_action(&self) -> bool {
        matches!(self.input, "delete" | "topic_notification_level_changed")
    }

    fn post(&self) -> &PostRow {
        self.post.as_ref().expect("a post to select on")
    }

    async fn select(&mut self) -> Result<(), AppError> {
        let bot = self.bot;
        let data = store::get(&mut *bot.pool.acquire().await?, self.user.id).await?;
        let topic_action = self.is_topic_action();
        let regular_post = self.post.as_ref().is_some_and(|p| p.post_type == REGULAR);
        if regular_post && !topic_action {
            let post = self.post().clone();
            let is_reply = self.input == "reply";
            self.is_pm_to_bot = bot.pm_to_bot(&post).await?;
            if is_reply && self.reset_track().await? {
                return Ok(());
            }
            let in_tutorial = data
                .as_ref()
                .is_some_and(|d| store::int_of(d, "topic_id") == Some(i64::from(post.topic_id)))
                && self.is_pm_to_bot;
            if let Some(data) = data.filter(|_| in_tutorial) {
                let state = store::str_of(&data, "state").map(str::to_string);
                let track = track_of(&data)?;
                if is_reply && self.like_user_post().await? {
                    self.terminate_track(data).await?;
                } else if state.as_deref() == Some("end") && is_reply {
                    let mentioned = self.bot_mentioned().await?;
                    if self.bot_commands(mentioned).await?.is_none() {
                        self.generic_replies(&track.reset_trigger(bot)?, None)
                            .await?;
                    }
                } else if is_reply {
                    let previous_status = data.get("attempted").cloned();
                    let skip = self.skip_track()?;
                    let current =
                        narrative::input(bot, track, self.input, self.user, Some(post), None, skip)
                            .await?;
                    let mut conn = bot.pool.acquire().await?;
                    let mut data = store::get(&mut conn, self.user.id)
                        .await?
                        .ok_or_else(|| BotError("the tutorial state is gone".into()))?;
                    drop(conn);
                    let attempted = current.is_none();
                    data.insert("attempted".into(), json!(attempted));
                    let previously = previous_status
                        .as_ref()
                        .is_some_and(|v| !matches!(v, Value::Null | Value::Bool(false)));
                    if previously
                        && previous_status == Some(json!(attempted))
                        && !store::truthy(&data, "skip_attempted")
                    {
                        self.generic_replies(&track.reset_trigger(bot)?, state.as_deref())
                            .await?;
                    } else {
                        crate::owned_schema::cached_del(
                            &mut *bot.pool.acquire().await?,
                            &generic_replies_key(self.user.id),
                        )
                        .await?;
                    }
                    store::set(&mut *bot.pool.acquire().await?, self.user.id, &data).await?;
                } else {
                    let skip = self.skip_track()?;
                    narrative::input(bot, track, self.input, self.user, Some(post), None, skip)
                        .await?;
                }
            } else if is_reply && (self.is_pm_to_bot || self.public_reply().await?) {
                if self.is_pm_to_bot {
                    self.like_user_post().await?;
                }
                self.bot_commands(true).await?;
            }
        } else if let Some(data) = data
            && store::str_of(&data, "state") != Some("end")
            && topic_action
        {
            let track = track_of(&data)?;
            narrative::input(
                bot,
                track,
                self.input,
                self.user,
                self.post.clone(),
                self.topic_id,
                false,
            )
            .await?;
        }
        Ok(())
    }

    /// `reset_track`: the reply asks to start a track over.
    async fn reset_track(&mut self) -> Result<bool, AppError> {
        for track in TRACKS {
            if self.selected_track(track).await? {
                narrative::reset_bot(self.bot, track, self.user, self.post()).await?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn selected_track(&mut self, track: Track) -> Result<bool, AppError> {
        let trigger = format!(
            "{} {}",
            self.bot.t("track_selector.reset_trigger")?,
            track.reset_trigger(self.bot)?
        );
        let raw = &self.post().raw;
        if raw.chars().count() < RESET_TRIGGER_EXACT_MATCH_LENGTH && self.is_pm_to_bot {
            Ok(regex(&format!(r"\b\W?{trigger}\W?\b"))?.is_match(raw))
        } else {
            Ok(self.match_trigger(&trigger).await?.is_some())
        }
    }

    /// `bot_commands(hint)`: the answer to a command, else (with `hint`)
    /// how to ask for help, rationed in public topics.
    async fn bot_commands(&mut self, hint: bool) -> Result<Option<PostRow>, AppError> {
        let bot = self.bot;
        let dice = format!("{} (\\d+)d(\\d+)", bot.t("dice.trigger")?);
        let raw = if self.user.manually_disabled_discobot() {
            Some(bot.t("track_selector.random_mention.discobot_disabled")?)
        } else if let Some(caps) = self.match_trigger(&dice).await? {
            let count = caps.first().map_or(0, |c| crate::ruby::to_i(c));
            let range = caps.get(1).map_or(0, |c| crate::ruby::to_i(c));
            Some(roll(bot, count, range)?)
        } else if self
            .match_trigger(&bot.t("quote.trigger")?)
            .await?
            .is_some()
        {
            Some(generate_quote(bot)?)
        } else if self
            .match_trigger(&bot.t("magic_8_ball.trigger")?)
            .await?
            .is_some()
        {
            Some(magic_8_ball(bot)?)
        } else if self
            .match_trigger(&bot.t("track_selector.help_trigger")?)
            .await?
            .is_some()
        {
            Some(self.help_message()?)
        } else if hint {
            let message = bot.t_with(
                "track_selector.random_mention.reply",
                &[
                    ("discobot_username", &bot.discobot_username()),
                    ("help_trigger", &bot.t("track_selector.help_trigger")?),
                ],
            )?;
            if self.public_reply().await? {
                let post = self.post();
                let key = format!("{PUBLIC_DISPLAY_BOT_HELP_KEY}:{}", post.topic_id);
                let mut conn = bot.pool.acquire().await?;
                let last = crate::owned_schema::cached_get(&mut conn, &key).await?;
                let show = match last {
                    None => true,
                    Some(last) => {
                        i64::from(post.post_number) - 10 > crate::ruby::to_i(&last)
                            && (86_400 - bot::ttl(&mut conn, &key).await?) > 6 * 3600
                    }
                };
                if show {
                    crate::owned_schema::cached_setex(
                        &mut conn,
                        &key,
                        86_400,
                        &post.post_number.to_string(),
                    )
                    .await?;
                    Some(message)
                } else {
                    None
                }
            } else {
                Some(message)
            }
        } else {
            None
        };
        match raw {
            Some(raw) => {
                bot.fake_delay().await;
                Ok(Some(
                    bot.reply_to(self.post.as_ref(), raw, ReplyOpts::default(), true)
                        .await?,
                ))
            }
            None => Ok(None),
        }
    }

    fn help_message(&self) -> Result<String, AppError> {
        let bot = self.bot;
        let username = bot.discobot_username();
        let tracks = format!(
            "{}, {}",
            Track::NewUser.reset_trigger(bot)?,
            Track::Advanced.reset_trigger(bot)?
        );
        let mut message = bot.t_with(
            "track_selector.random_mention.tracks",
            &[
                ("discobot_username", &username),
                ("reset_trigger", &bot.t("track_selector.reset_trigger")?),
                ("tracks", &tracks),
            ],
        )?;
        let actions = bot.t_with(
            "track_selector.random_mention.bot_actions",
            &[
                ("discobot_username", &username),
                ("dice_trigger", &bot.t("dice.trigger")?),
                ("quote_trigger", &bot.t("quote.trigger")?),
                ("quote_sample", &generate_quote(bot)?),
                ("magic_8_ball_trigger", &bot.t("magic_8_ball.trigger")?),
            ],
        )?;
        message.push_str("\n\n");
        message.push_str(&actions);
        Ok(message)
    }

    /// `generic_replies(track_reset_trigger, state)`: two answers that it
    /// did not understand, then silence (for 15 minutes).
    async fn generic_replies(
        &mut self,
        track_reset_trigger: &str,
        state: Option<&str>,
    ) -> Result<(), AppError> {
        let bot = self.bot;
        let reset_trigger = format!(
            "{} {track_reset_trigger}",
            bot.t("track_selector.reset_trigger")?
        );
        let key = generic_replies_key(self.user.id);
        let mut conn = bot.pool.acquire().await?;
        let count = match crate::owned_schema::cached_get(&mut conn, &key).await? {
            Some(count) => crate::ruby::to_i(&count),
            None => {
                crate::owned_schema::cached_setex(&mut conn, &key, 900, "0").await?;
                0
            }
        };
        drop(conn);
        match count {
            0 => {
                let mut raw = bot.t("track_selector.do_not_understand.first_response")?;
                if let Some(state) = state
                    && state != "end"
                {
                    let track = bot.t_with(
                        "track_selector.do_not_understand.track_response",
                        &[
                            ("reset_trigger", &reset_trigger),
                            ("skip_trigger", &bot.t("track_selector.skip_trigger")?),
                        ],
                    )?;
                    raw = format!("{raw}\n\n{track}");
                }
                bot.reply_to(self.post.as_ref(), raw, ReplyOpts::default(), true)
                    .await?;
            }
            1 => {
                let raw = bot.t_with(
                    "track_selector.do_not_understand.second_response",
                    &[
                        ("base_path", bot.base_path()),
                        ("reset_trigger", &bot.t("track_selector.reset_trigger")?),
                    ],
                )?;
                bot.reply_to(self.post.as_ref(), raw, ReplyOpts::default(), true)
                    .await?;
            }
            // Stay out of the user's way.
            _ => {}
        }
        bot::incr(&mut *bot.pool.acquire().await?, &key).await?;
        Ok(())
    }

    /// `skip_track?`: in the tutorial message, the skip trigger alone or
    /// after discobot's name at the start of a line.
    fn skip_track(&self) -> Result<bool, AppError> {
        if !self.is_pm_to_bot {
            return Ok(false);
        }
        let skip = self.bot.t("track_selector.skip_trigger")?;
        let pattern = format!("((^@{} {skip})|(^{skip}$))", self.bot.discobot_username());
        Ok(regex(&format!("(?m){pattern}"))?.is_match(&self.post().raw))
    }

    /// `match_trigger?(trigger)`: the trigger cooked after discobot's
    /// mention, matched against the cooked post (its leading `<p>` cut, so
    /// the trigger may end a paragraph); in the tutorial message the raw
    /// being just the trigger counts too. The captures, when matched.
    async fn match_trigger(&self, trigger: &str) -> Result<Option<Vec<String>>, AppError> {
        let bot = self.bot;
        let cooked = crate::pretty_text::cook(
            bot.ctx.host,
            &format!("@{}\\s+{trigger}", bot.discobot_username()),
            &crate::pretty_text::MarkdownOptions::default(),
        )
        .await?;
        let pattern = cooked.get(3..).unwrap_or_default();
        let post = self.post();
        let captures = |re: &regex::Regex, text: &str| {
            re.captures(text).map(|c| {
                c.iter()
                    .skip(1)
                    .map(|m| m.map(|m| m.as_str().to_string()).unwrap_or_default())
                    .collect::<Vec<_>>()
            })
        };
        let found = captures(&regex(pattern)?, &post.cooked);
        if found.is_some() || !self.is_pm_to_bot {
            return Ok(found);
        }
        Ok(captures(
            &regex(&format!("(?m)^{trigger}$"))?,
            post.raw.trim(),
        ))
    }

    /// `like_user_post`: discobot likes a post thanking it. True whenever
    /// the post says thanks, whatever the like did.
    async fn like_user_post(&self) -> Result<bool, AppError> {
        let post = self.post();
        if regex("thank")?.is_match(&post.raw) {
            self.bot.like(post.id).await?;
            return Ok(true);
        }
        Ok(false)
    }

    /// `bot_mentioned?`: discobot among the raw's mentions outside quotes and
    /// code (PostAnalyzer after PostStripper, on the raw cooked again).
    async fn bot_mentioned(&mut self) -> Result<bool, AppError> {
        if let Some(mentioned) = self.bot_mentioned {
            return Ok(mentioned);
        }
        let post = self.post();
        let mentioned = if post.raw.trim().is_empty() {
            false
        } else {
            let cooked = crate::pretty_text::cook(
                self.bot.ctx.host,
                &post.raw,
                &crate::pretty_text::MarkdownOptions {
                    topic_id: Some(i64::from(post.topic_id)),
                    ..Default::default()
                },
            )
            .await?;
            crate::posting::validate::analyze(&cooked, self.bot.base_path())?
                .mention_names
                .contains(&self.bot.discobot_username())
        };
        self.bot_mentioned = Some(mentioned);
        Ok(mentioned)
    }

    /// `public_reply?`
    async fn public_reply(&mut self) -> Result<bool, AppError> {
        if self
            .bot
            .ctx
            .settings
            .get("discourse_narrative_bot_disable_public_replies")?
            .truthy()
        {
            return Ok(false);
        }
        let post = self.post().clone();
        Ok(self.bot.reply_to_bot_post(&post).await? || self.bot_mentioned().await?)
    }

    /// `terminate_track(data)`: out of the track, no more reminders.
    async fn terminate_track(&self, mut data: store::Data) -> Result<(), AppError> {
        for key in ["track", "state", "topic_id"] {
            data.insert(key.into(), Value::Null);
        }
        let mut conn = self.bot.pool.acquire().await?;
        store::set(&mut conn, self.user.id, &data).await?;
        // The selector's own cancel_timeout_job: `self.class` is the
        // selector, so the klass it names matches no narrative's timeout.
        bot::cancel_timeout_job(
            &mut conn,
            self.user.id,
            "DiscourseNarrativeBot::TrackSelector",
        )
        .await
    }
}

fn generic_replies_key(user_id: i32) -> String {
    format!("{GENERIC_REPLIES_COUNT_PREFIX}{user_id}")
}

/// `Regexp.new(pattern, "i")`
fn regex(pattern: &str) -> Result<regex::Regex, AppError> {
    regex::RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .map_err(|e| BotError(format!("a trigger that is not a regexp: {e}")).into())
}

/// `Dice.roll(num_of_dice, range_of_dice)`
pub fn roll(bot: &Bot<'_>, count: i64, range: i64) -> Result<String, AppError> {
    let mut rng = rand::thread_rng();
    roll_with(bot, count, range, |r| rng.gen_range(1..=r))
}

pub fn roll_with(
    bot: &Bot<'_>,
    mut count: i64,
    mut range: i64,
    mut die: impl FnMut(i64) -> i64,
) -> Result<String, AppError> {
    if count == 0 || range == 0 {
        return bot.t("dice.invalid");
    }
    let mut output = String::new();
    if count > MAXIMUM_NUM_OF_DICE {
        output.push_str(
            &bot.tr()
                .t_count(
                    "discourse_narrative_bot.dice.not_enough_dice",
                    MAXIMUM_NUM_OF_DICE,
                    &[],
                )
                .ok_or(crate::Unsupported(
                    "a discobot message without a translation",
                ))?,
        );
        output.push_str("\n\n");
        count = MAXIMUM_NUM_OF_DICE;
    }
    if range > MAXIMUM_RANGE_OF_DICE {
        output.push_str(&bot.t("dice.out_of_range")?);
        output.push_str("\n\n");
        range = MAXIMUM_RANGE_OF_DICE;
    }
    let results: Vec<String> = (0..count).map(|_| die(range).to_string()).collect();
    output.push_str(&bot.t_with("dice.results", &[("results", &results.join(", "))])?);
    Ok(output)
}

/// `Magic8Ball.generate_answer`
pub fn magic_8_ball(bot: &Bot<'_>) -> Result<String, AppError> {
    let n = rand::thread_rng().gen_range(1..=20);
    let answer = bot.t(&format!("magic_8_ball.answers.{n}"))?;
    bot.t_with("magic_8_ball.result", &[("result", &answer)])
}

/// `QuoteGenerator.generate(user)`: one of the quotes, in the user's
/// locale.
pub fn generate_quote(bot: &Bot<'_>) -> Result<String, AppError> {
    let mut quotes = Vec::new();
    for n in 1.. {
        let base = format!("discourse_narrative_bot.quote.{n}");
        let tr = bot.tr();
        match (
            tr.t(&format!("{base}.quote")),
            tr.t(&format!("{base}.author")),
        ) {
            (Some(quote), Some(author)) => quotes.push((quote.to_string(), author.to_string())),
            _ => break,
        }
    }
    if quotes.is_empty() {
        return Err(BotError("no quotes".into()).into());
    }
    let (quote, author) = &quotes[rand::thread_rng().gen_range(0..quotes.len())];
    format_quote(bot, quote, author)
}

/// `QuoteGenerator.format_quote(quote, author)`
pub fn format_quote(bot: &Bot<'_>, quote: &str, author: &str) -> Result<String, AppError> {
    bot.t_with("quote.results", &[("quote", quote), ("author", author)])
}
