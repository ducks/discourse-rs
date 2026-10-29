//! `NewPostManager#perform` -> `PostCreator#create` (with TopicCreator for
//! a new topic): validation, then one transaction writing the post, its
//! topic, stats, tracking and user actions, then the category's latest
//! post and the search index.

use chrono::NaiveDateTime;
use serde_json::{Map, json};
use sqlx::{PgConnection, PgPool};

use super::limits::{self, Performed, Poster};
use super::links::{self, LinkPost};
use super::search_index::{self, BaseUrls, PostIndex};
use super::text::{TitleOptions, clean_title, normalize_whitespaces, slug_for, word_count};
use super::validate::{self, PostInput, TopicInput};
use super::{
    BAKED_VERSION, Ctx, TopicUserAttr, change_topic_user, current_draft_sequence,
    next_draft_sequence, notification_levels, notification_reasons, post_types, record_timing,
    user_actions,
};
use crate::guardian::Guardian;
use crate::pretty_text::{self, MarkdownOptions};
use crate::topic_guardian::TopicCtx;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// The `create_params` this slice accepts.
#[derive(Debug, Default)]
pub struct NewPost {
    pub raw: String,
    pub topic_id: Option<i32>,
    pub title: Option<String>,
    /// `params[:category]`, as sent.
    pub category: Option<String>,
    pub reply_to_post_number: Option<i32>,
    pub typing_duration_msecs: Option<i64>,
    pub composer_open_duration_msecs: Option<i64>,
    pub composer_version: Option<i32>,
    pub user_agent: Option<String>,
    /// A new private message to these usernames, lowercased (`archetype`
    /// and `target_recipients`).
    pub pm_recipients: Option<Vec<String>>,
    /// `target_group_names`: the recipients that are groups the sender may
    /// message, lowercased.
    pub pm_groups: Vec<String>,
    /// Set for a post Email::Receiver creates.
    pub email: Option<EmailOrigin>,
    /// `advance_draft`: the composer's draft sequence moves on (not for API
    /// requests or email).
    pub advance_draft: bool,
    /// `draft_key`: the composer's draft, else the topic's or `new_topic`.
    pub draft_key: Option<String>,
    /// `first_post_checks`: a first post's fast-typing check (not for API
    /// requests or email).
    pub first_post_checks: bool,
    /// `skip_validations:` no post or topic validations, suspension check
    /// or recipient permissions (SystemMessage).
    pub skip_validations: bool,
    /// A new message's subtype (`user_to_user` unless given).
    pub subtype: Option<String>,
    /// `post_alert_options:`, handed to the post_alert job.
    pub post_alert_options: Option<serde_json::Value>,
    /// `topic_opts: { custom_fields: }`: a new topic's custom fields.
    pub topic_custom_fields: Vec<(String, String)>,
    /// Sent by PostsController, which passes the writing device; email and
    /// system messages have none.
    pub from_composer: bool,
    /// A queued post staff approved (`reviewed_queued_post`, with
    /// `skip_guardian`): the review queue isn't asked again, and the
    /// author may post where the reviewer's guardian lets them.
    pub reviewed_queued_post: bool,
    /// `tags`: a new topic's tags (TopicCreator), as names or tag objects.
    pub tags: Option<crate::tagging::TagsParam>,
    /// The `tags` param as sent, which a queued post keeps in its payload.
    pub tags_payload: Option<serde_json::Value>,
    /// PostCreator's guardian when it isn't the author's: a staff reviewer
    /// approving a queued post (`opts[:guardian]`), whom TopicCreator tags
    /// the topic as.
    pub creator_guardian: Option<Guardian>,
}

/// What Email::Receiver#create_post adds to a post: the mail's date as
/// created_at, the raw message, and the incoming_emails row to link.
#[derive(Debug)]
pub struct EmailOrigin {
    pub raw_email: String,
    pub created_at: NaiveDateTime,
    pub incoming_email_id: i32,
    pub message_id: String,
}

/// How a create ends when it isn't a server error.
#[derive(Debug)]
pub enum Outcome {
    Created {
        post_id: i32,
    },
    /// `NewPostResult` errors: a 422 with `{action: "create_post", errors}`.
    Invalid(Vec<String>),
    /// A post for the review queue whose raw PostValidator refused: a 422
    /// with `{action: "created_post", errors: [errors[:raw]]}` (the
    /// messages as one error, an array).
    InvalidRaw(Vec<String>),
    /// Sent to the review queue (`action: "enqueued"`): 200, or 422 when
    /// the creator refused it.
    Enqueued(super::enqueue::Enqueued),
    /// A NewPostManager handler's failed result: a 422 with its action.
    Refused {
        action: &'static str,
        errors: Vec<String>,
    },
    /// `Discourse::InvalidAccess` from the guardian.
    Forbidden,
    /// `Discourse::InvalidParameters`
    InvalidParameter(&'static str),
    /// A topic or post limit refused it (RateLimiter::LimitExceeded out of
    /// the transaction, everything rolled back).
    RateLimited(crate::rate_limiter::LimitExceeded),
}

/// The topic a reply goes to, as PostCreator reads it.
#[derive(sqlx::FromRow)]
struct ReplyTopic {
    id: i32,
    title: String,
    archetype: String,
    category_id: Option<i32>,
    slow_mode_seconds: i32,
    word_count: Option<i32>,
    closed: bool,
}

/// Why TopicCreator stopped a new topic, its transaction rolled back.
enum TopicRefused {
    RateLimited(crate::rate_limiter::LimitExceeded),
    /// `rollback_with!`: the topic's errors.
    Invalid(Vec<String>),
}

/// What a new topic is created with.
struct TopicPlan {
    title: String,
    /// None for a message.
    category_id: Option<i32>,
    /// A message's recipients, by id order, as add_users_from_scope adds them.
    pm_recipients: Option<Vec<i32>>,
    /// A message's allowed groups, in id order.
    pm_groups: Vec<super::group_messages::AllowedGroup>,
    /// Emails of no user among a message's recipients: a staged user is
    /// created for each (TopicCreator#add_emails), at its place in the list.
    pm_staged: Vec<(usize, String)>,
    slow_mode_seconds: Option<i32>,
    all_topics_wiki: bool,
    /// A message's subtype.
    subtype: Option<String>,
    /// The topic's custom fields (`topic_opts[:custom_fields]`).
    custom_fields: Vec<(String, String)>,
    /// TopicCreator#setup_tags: the tags asked for, the category the topic
    /// was given (none for uncategorized, filled in at save), and the text
    /// tag watched words are matched in (`"#{title} #{raw}"`).
    tags: Option<crate::tagging::TagsParam>,
    tag_category_id: Option<i32>,
    tag_text: String,
}

/// `BrowserDetection.device(user_agent)`
fn device(user_agent: Option<&str>) -> &'static str {
    let Some(ua) = user_agent else {
        return "unknown";
    };
    let ua = ua.to_lowercase();
    for (needle, device) in [
        ("android", "android"),
        ("cros", "chromebook"),
        ("ipad", "ipad"),
        ("iphone", "iphone"),
        ("ipod", "ipod"),
        ("mobile", "mobile"),
        ("macintosh", "mac"),
        ("linux", "linux"),
        ("windows", "windows"),
    ] {
        if ua.contains(needle) {
            return device;
        }
    }
    "unknown"
}

/// ActiveSupport's `String#truncate(400)`.
fn truncate_400(s: &str) -> String {
    if s.chars().count() <= 400 {
        return s.to_string();
    }
    format!("{}...", s.chars().take(397).collect::<String>())
}

/// A regular topic's plan: its category (Uncategorized when none) and
/// what the category gives new topics.
async fn regular_topic_plan(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    title: String,
    category_id: Option<i32>,
) -> Result<TopicPlan, AppError> {
    let uncategorized = s.get("uncategorized_category_id")?.to_i() as i32;
    let category_id = category_id.unwrap_or(uncategorized);
    let (slow_mode, all_topics_wiki): (Option<i32>, bool) = sqlx::query_as(
        "SELECT default_slow_mode_seconds, all_topics_wiki FROM categories WHERE id = $1",
    )
    .bind(category_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(TopicPlan {
        title,
        category_id: Some(category_id),
        pm_recipients: None,
        pm_groups: Vec::new(),
        pm_staged: Vec::new(),
        slow_mode_seconds: slow_mode,
        all_topics_wiki,
        subtype: None,
        custom_fields: Vec::new(),
        tags: None,
        tag_category_id: None,
        tag_text: String::new(),
    })
}

/// `NewPostManager.new(user, params).perform` for a reply or a regular topic.
pub async fn create(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    args: NewPost,
) -> Result<Outcome, AppError> {
    let s = ctx.settings;
    let user = guardian
        .user()
        .ok_or(Unsupported("creating posts anonymously"))?
        .clone();
    if s.get("site_archived")?.truthy() {
        return Err(Unsupported("site_archived").into());
    }
    // discourse-poll's NewPostManager handler: the polls must be valid.
    // Without validations (PostCreator's save(validate: false)) none are
    // extracted or written.
    let polls = if !args.skip_validations && crate::plugins::poll::enabled(s)? {
        let extracted = crate::plugins::poll::extract_raw(ctx, &args.raw).await?;
        match crate::plugins::poll::validate_polls(s, ctx.i18n, extracted)? {
            Ok(polls) => polls,
            Err(errors) => {
                return Ok(Outcome::Refused {
                    action: "poll",
                    errors,
                });
            }
        }
    } else {
        Vec::new()
    };
    let mut conn = pool.acquire().await?;
    let new_topic = args.topic_id.is_none();
    // A new private message: no category (find_category drops it), and no
    // can_create?(Topic) check (the recipients are checked instead).
    let new_pm = new_topic && args.pm_recipients.is_some();

    // TopicCreator#setup_topic_params: the category, and the guardian's
    // can_create?(Topic, category).
    let category_id = if new_topic && !new_pm {
        match args.category.as_deref().filter(|c| !c.is_empty()) {
            Some(c) if c.chars().all(|ch| ch.is_ascii_digit()) => {
                let id: Option<i32> = c.parse().ok();
                let found: Option<i32> =
                    sqlx::query_scalar("SELECT id FROM categories WHERE id = $1")
                        .bind(id)
                        .fetch_optional(&mut *conn)
                        .await?;
                match found {
                    Some(id) => Some(id),
                    None => {
                        // Category.find_by(id:) misses; InvalidParameters
                        // follows the guardian check below.
                        if !guardian.can_create_topic(&mut conn, s).await? {
                            return Ok(Outcome::Forbidden);
                        }
                        return Ok(Outcome::InvalidParameter("category"));
                    }
                }
            }
            Some(_) => return Ok(Outcome::InvalidParameter("category")),
            None => None,
        }
    } else {
        None
    };
    // TopicCreator's guardian check, after the review queue's (which
    // answers its own way for a post it would queue).
    let mut forbidden = false;
    if new_topic && !new_pm && !args.reviewed_queued_post {
        forbidden = !match category_id {
            None => guardian.can_create_topic(&mut conn, s).await?,
            Some(id) => {
                guardian.can_create_topic(&mut conn, s).await?
                    && guardian
                        .topic_create_allowed_category_ids(&mut conn, s)
                        .await?
                        .contains(&id)
            }
        };
    }

    // The reply's topic, before the review-queue check reads its category.
    let reply_topic: Option<ReplyTopic> =
        match args.topic_id {
            Some(id) => sqlx::query_as(
                "SELECT id, title, archetype, category_id, slow_mode_seconds, word_count, closed \
                 FROM topics WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?,
            None => None,
        };
    // A message: a new one, or a reply in one.
    let private_message = new_pm
        || reply_topic
            .as_ref()
            .is_some_and(|t| t.archetype == "private_message");
    if args.reviewed_queued_post {
        if user.staged {
            return Err(Unsupported("approving a staged user's queued post").into());
        }
    } else {
        // NewPostManager#perform: blocked words in "#{title} #{raw}", for
        // everyone but staff, before any handler.
        if !guardian.is_staff() {
            let text = format!("{} {}", args.title.as_deref().unwrap_or(""), args.raw);
            if let Some(message) =
                crate::word_watcher::block_error(&mut conn, s, ctx.i18n, &text).await?
            {
                return Ok(Outcome::Refused {
                    action: "created_post",
                    errors: vec![message],
                });
            }
        }
        validate::refuse_unported_author(guardian)?;
        // ReviewablePost.queue_for_review_if_possible after the create:
        // a regular post in a regular topic by a non-staff author below
        // trust level 4 goes to the review queue too.
        if !private_message
            && s.get("review_every_post")?.truthy()
            && guardian.user().is_some_and(|u| u.id > 0)
            && !guardian.is_staff()
            && !guardian.has_trust_level(4)
        {
            return Err(Unsupported("review_every_post on a new post").into());
        }
        // NewPostManager#perform: messages are never queued; anything else
        // post_needs_approval? finds a reason for goes to the queue.
        if !private_message {
            let approval = validate::post_needs_approval(
                &mut conn,
                ctx,
                guardian,
                &validate::ApprovalArgs {
                    raw: &args.raw,
                    title: args.title.as_deref(),
                    topic_id: args.topic_id,
                    category_id,
                    typing_duration_msecs: args.typing_duration_msecs.unwrap_or(0),
                    first_post_checks: args.first_post_checks,
                },
            )
            .await?;
            if let Some(reason) = approval.reason {
                if forbidden {
                    // can_create_topic_on_category? fails with
                    // js.errors.reasons.forbidden, a client string.
                    return Err(Unsupported("enqueueing a topic its author can't create").into());
                }
                drop(conn);
                let topic = reply_topic.as_ref().map(|t| super::enqueue::QueueTopic {
                    id: t.id,
                    category_id: t.category_id,
                    slow_mode_seconds: t.slow_mode_seconds,
                });
                return super::enqueue::enqueue(
                    pool,
                    ctx,
                    guardian,
                    &args,
                    &approval,
                    reason,
                    category_id,
                    topic.as_ref(),
                    !polls.is_empty(),
                )
                .await;
            }
        }
    }
    if forbidden {
        return Ok(Outcome::Forbidden);
    }

    // PostCreator#valid?
    if user.suspended() && !args.skip_validations {
        return Ok(Outcome::Invalid(vec![ctx.t("user_is_suspended")]));
    }
    // The recipients: their number, and whether they take messages from
    // the sender (UserCommScreener).
    // target_usernames: the recipients other than emails.
    let usernames: Option<Vec<String>> = args
        .pm_recipients
        .as_ref()
        .map(|names| names.iter().filter(|n| !n.contains('@')).cloned().collect());
    if let Some(names) = &usernames
        && !names.is_empty()
        && !args.skip_validations
        && !guardian.is_staff()
    {
        let max = s.get("max_allowed_message_recipients")?.to_i();
        if names.len() as i64 > max {
            return Ok(Outcome::Invalid(vec![
                ctx.i18n
                    .t_with(
                        "max_pm_recipients",
                        &[("recipients_limit", &max.to_string())],
                    )
                    .unwrap_or_default(),
            ]));
        }
        let target_users: Vec<(i32, String)> =
            sqlx::query_as("SELECT id, username FROM users WHERE username_lower = ANY($1)")
                .bind(names)
                .fetch_all(&mut *conn)
                .await?;
        let ids: Vec<i32> = target_users.iter().map(|(id, _)| *id).collect();
        let screener = crate::user_comm_screener::UserCommScreener::new(
            &mut conn,
            crate::user_comm_screener::Actor {
                id: user.id,
                staff: guardian.is_staff(),
            },
            &ids,
        )
        .await?;
        let errors: Vec<String> = screener
            .preventing_actor_communication()
            .into_iter()
            .map(|id| {
                let username = target_users
                    .iter()
                    .find(|(t, _)| *t == id)
                    .map(|(_, name)| name.as_str())
                    .unwrap_or_default();
                ctx.i18n
                    .t_with("not_accepting_pms", &[("username", username)])
                    .unwrap_or_default()
            })
            .collect();
        if !errors.is_empty() {
            return Ok(Outcome::Invalid(errors));
        }
    }
    let mut plan = None;
    if new_topic {
        let title_options = TitleOptions {
            prettify: s.get("title_prettify")?.truthy(),
            allow_uppercase_posts: s.get("allow_uppercase_posts")?.truthy(),
            remove_extraneous_space: s.get("title_remove_extraneous_space")?.truthy(),
        };
        let title = clean_title(args.title.as_deref().unwrap_or(""), &title_options);
        let mut errors = validate::validate_topic(
            &mut conn,
            ctx,
            guardian,
            &TopicInput {
                title: &title,
                category_id,
                private_message,
            },
        )
        .await?;
        if !args.skip_validations {
            let names = match &args.tags {
                Some(param) => crate::tagging::names_for_validation(&mut conn, param).await?,
                None => Vec::new(),
            };
            errors.extend(
                validate_topic_tags(
                    &mut conn,
                    ctx,
                    guardian,
                    category_id,
                    private_message,
                    &names,
                )
                .await?,
            );
        }
        if !errors.is_empty() && !args.skip_validations {
            return Ok(Outcome::Invalid(errors));
        }
        if new_pm {
            plan = Some(TopicPlan {
                title,
                category_id: None,
                pm_recipients: None,
                pm_groups: Vec::new(),
                pm_staged: Vec::new(),
                slow_mode_seconds: None,
                all_topics_wiki: false,
                subtype: args.subtype.clone(),
                custom_fields: args.topic_custom_fields.clone(),
                tags: None,
                tag_category_id: None,
                tag_text: String::new(),
            });
        } else {
            let mut regular = regular_topic_plan(&mut conn, s, title, category_id).await?;
            regular.custom_fields = args.topic_custom_fields.clone();
            // A plugin handler's (discourse-post-voting's question_answer).
            regular.subtype = args.subtype.clone();
            plan = Some(regular);
        }
        if let Some(plan) = plan.as_mut() {
            plan.tags = args.tags.clone().filter(|t| !t.is_empty());
            plan.tag_category_id = category_id;
            plan.tag_text = format!("{} {}", args.title.as_deref().unwrap_or(""), args.raw);
        }
    }
    if !new_topic {
        if args.pm_recipients.is_some() && reply_topic.is_some() {
            return Ok(Outcome::Invalid(vec![ctx.t("create_pm_on_existing_topic")]));
        }
        let Some(topic) = &reply_topic else {
            return Ok(Outcome::Invalid(vec![ctx.t("topic_not_found")]));
        };
        // A reviewed queued post: the reviewer's guardian, which slow mode
        // doesn't hold back, and no can_create?(Post, topic).
        if topic.slow_mode_seconds > 0 && !guardian.is_staff() && !args.reviewed_queued_post {
            return Err(Unsupported("slow mode").into());
        }
        let topic_ctx = TopicCtx::load(&mut conn, s, guardian, topic.id).await?;
        let can_post_anywhere = guardian.can_create_post_anywhere(&mut conn, s).await?;
        let can = match &topic_ctx {
            Some(t) => {
                let can_see = guardian.can_see_topic(
                    s,
                    t,
                    true,
                    &guardian.secure_category_ids(&mut conn, s).await?,
                )?;
                can_see && guardian.can_create_post_on_topic(s, t, can_post_anywhere)?
            }
            None => false,
        };
        if !can && !args.reviewed_queued_post {
            return Ok(Outcome::Invalid(vec![ctx.t("topic_not_found")]));
        }
        if topic.closed {
            return Err(Unsupported("replying to closed topics as staff").into());
        }
    }

    // setup_post
    let raw = normalize_whitespaces(&args.raw.replace('\0', ""))
        .trim_end()
        .to_string();
    let reply_to_post_number = if new_topic {
        None
    } else {
        args.reply_to_post_number
    };
    let topic_id_for_cook = reply_topic.as_ref().map(|t| t.id);
    let omit_nofollow = omit_nofollow(&mut conn, ctx, user.id).await?;
    drop(conn);
    let cooked = pretty_text::cook(
        ctx.host,
        &raw,
        &MarkdownOptions {
            topic_id: topic_id_for_cook.map(i64::from),
            post_id: None,
            user_id: Some(i64::from(user.id)),
            force_quote_link: false,
            omit_nofollow,
            chat: false,
        },
    )
    .await?;
    let base_path = ctx.config.globals.relative_url_root().to_string();
    let analysis = validate::analyze(&cooked, &base_path)?;
    crate::onebox::refuse_unported(
        &analysis.onebox_links,
        &crate::url::Urls {
            config: ctx.config,
            settings: ctx.settings,
        },
        &base_path,
    )?;
    let mut conn = pool.acquire().await?;
    // has_host_spam?, ahead of PostValidator: refused, and the moderators
    // told (handle_spam) unless the post was approved from the queue.
    if !args.skip_validations {
        let reply_in_message = reply_topic
            .as_ref()
            .is_some_and(|t| t.archetype == "private_message");
        let max_oneboxes = s.get("max_oneboxes_per_post")?.to_i();
        if let Some(hosts) = validate::host_spam(
            &mut conn,
            ctx,
            guardian,
            reply_in_message,
            &analysis,
            max_oneboxes,
        )
        .await?
        {
            // NewPostManager#perform_create_post then flags the user's
            // linked posts as spam (flag_linked_posts_as_spam): system
            // flags with a message, not ported.
            let flagged: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM topic_links WHERE user_id = $1 AND post_id NOT IN \
                   (SELECT post_id FROM post_actions WHERE post_action_type_id = 8 \
                    AND disagreed_at IS NOT NULL))",
            )
            .bind(user.id)
            .fetch_one(&mut *conn)
            .await?;
            if flagged {
                return Err(Unsupported(
                    "flagging a new user's linked posts as spam (flag_linked_posts_as_spam)",
                )
                .into());
            }
            drop(conn);
            if !args.reviewed_queued_post {
                // Boxed: the message is posted through create.
                Box::pin(spam_post_blocked(pool, ctx, &user.username, &hosts)).await?;
            }
            return Ok(Outcome::Invalid(vec![ctx.t("spamming_host")]));
        }
    }
    let mut errors = validate::validate_post(
        &mut conn,
        ctx,
        guardian,
        &PostInput {
            raw: &raw,
            topic_id: topic_id_for_cook,
            first_post: false,
            private_message,
            new_record: true,
            post_id: None,
            user_id: user.id,
        },
        &analysis,
    )
    .await?;
    // discourse-policy's validate_policy, before poll's (load order).
    if let Some(error) = crate::plugins::policy::lifecycle::validate_post(
        &mut conn,
        ctx.host,
        s,
        ctx.i18n,
        &crate::plugins::policy::lifecycle::Saving {
            old_cooked: None,
            new_raw: &args.raw,
            // create_topic makes a post in an all_topics_wiki category a wiki
            // before the save validates it.
            wiki: plan.as_ref().is_some_and(|p| p.all_topics_wiki),
            acting_user_id: Some(user.id),
            user_id: Some(user.id),
        },
    )
    .await?
    {
        errors.push(error);
    }
    // discourse-post-voting's ensure_only_replies.
    if let Some(topic) = &reply_topic
        && let Some(key) = crate::plugins::post_voting::lifecycle::reply_error(
            &mut conn,
            s,
            topic.id,
            reply_to_post_number,
        )
        .await?
    {
        errors.push(ctx.t(key));
    }
    // discourse-poll's validate_polls: PostValidator, who may create them.
    if !polls.is_empty()
        && let Some(error) =
            crate::plugins::poll::validate_post(&mut conn, s, ctx.i18n, guardian, topic_id_for_cook)
                .await?
    {
        errors.push(error);
    }
    if !errors.is_empty() && !args.skip_validations {
        return Ok(Outcome::Invalid(errors));
    }
    // handle_spam's SpamRule::FlagSockpuppets, after a reply is created.
    if let Some(topic) = &reply_topic
        && !args.skip_validations
        && !args.reviewed_queued_post
        && s.get("flag_sockpuppets")?.truthy()
        && reply_is_from_sockpuppet(&mut conn, user.id, topic.id).await?
    {
        return Err(Unsupported("flag_sockpuppets (flagging a sockpuppet's reply)").into());
    }
    let new_polls = polls
        .iter()
        .map(crate::plugins::poll::NewPoll::from_extracted)
        .collect::<Result<Vec<_>, _>>()?;
    if new_topic {
        // Saving the post validates again with the topic in place, which
        // makes it a first post.
        let errors = validate::validate_post(
            &mut conn,
            ctx,
            guardian,
            &PostInput {
                raw: &raw,
                topic_id: None,
                first_post: true,
                private_message,
                new_record: true,
                post_id: None,
                user_id: user.id,
            },
            &analysis,
        )
        .await?;
        if !errors.is_empty() && !args.skip_validations {
            return Ok(Outcome::Invalid(errors));
        }
    }
    // TopicCreator#process_private_message: the recipients, each one the
    // sender may message, in id order (find_each).
    if new_pm {
        let names = usernames.clone().unwrap_or_default();
        let emails: Vec<String> = args
            .pm_recipients
            .iter()
            .flatten()
            .filter(|n| n.contains('@'))
            .cloned()
            .collect();
        let base = "activerecord.errors.models.topic.attributes.base";
        if names.is_empty() && emails.is_empty() && args.pm_groups.is_empty() {
            return Ok(Outcome::Invalid(vec![
                ctx.t(&format!("{base}.no_user_selected")),
            ]));
        }
        // can_send_private_messages_to_email?
        if !emails.is_empty()
            && !(s.get("enable_staged_users")?.truthy()
                && guardian.is_authenticated()
                && guardian.in_setting_groups(s, "personal_message_enabled_groups")?
                && guardian.in_setting_groups(s, "send_email_messages_allowed_groups")?)
        {
            return Ok(Outcome::Invalid(vec![
                ctx.t(&format!("{base}.send_to_email_disabled")),
            ]));
        }
        #[derive(sqlx::FromRow)]
        struct Target {
            id: i32,
            allow_private_messages: Option<bool>,
            suspended: bool,
            staff: bool,
        }
        let targets: Vec<Target> = sqlx::query_as(
            "SELECT u.id, o.allow_private_messages, COALESCE(u.suspended_till > now(), FALSE) AS suspended, \
                    (u.admin OR u.moderator) AS staff \
             FROM users u LEFT JOIN user_options o ON o.user_id = u.id \
             WHERE u.username_lower = ANY($1) ORDER BY u.id",
        )
        .bind(&names)
        .fetch_all(&mut *conn)
        .await?;
        // can_send_private_messages?
        let sender_can =
            user.id == -1 || guardian.in_setting_groups(s, "personal_message_enabled_groups")?;
        for t in &targets {
            // can_send_private_message?(target)
            let ok = sender_can
                && (guardian.is_staff() || t.allow_private_messages.unwrap_or(true))
                && (guardian.is_staff() || !t.suspended)
                && (!guardian.is_silenced() || t.staff);
            if !ok && !args.skip_validations {
                return Ok(Outcome::Invalid(vec![
                    ctx.t(&format!("{base}.cant_send_pm")),
                ]));
            }
        }
        if targets.len() != names.len() {
            return Ok(Outcome::Invalid(vec![
                ctx.t(&format!("{base}.target_user_not_found")),
            ]));
        }
        // add_emails: the user with each email (any of theirs), else a
        // staged user to create; each one the sender may message.
        let mut recipient_ids: Vec<i32> = targets.iter().map(|t| t.id).collect();
        let mut staged: Vec<(usize, String)> = Vec::new();
        for email in &emails {
            let found: Option<Target> = sqlx::query_as(
                "SELECT u.id, o.allow_private_messages, COALESCE(u.suspended_till > now(), FALSE) AS suspended, \
                        (u.admin OR u.moderator) AS staff \
                 FROM users u LEFT JOIN user_options o ON o.user_id = u.id \
                 WHERE u.id IN (SELECT user_id FROM user_emails WHERE lower(email) = lower($1)) \
                 ORDER BY u.id LIMIT 1",
            )
            .bind(email)
            .fetch_optional(&mut *conn)
            .await?;
            match found {
                Some(t) => {
                    let ok = sender_can
                        && (guardian.is_staff() || t.allow_private_messages.unwrap_or(true))
                        && (guardian.is_staff() || !t.suspended)
                        && (!guardian.is_silenced() || t.staff);
                    if !ok && !args.skip_validations {
                        return Ok(Outcome::Invalid(vec![
                            ctx.t(&format!("{base}.cant_send_pm")),
                        ]));
                    }
                    if !recipient_ids.contains(&t.id) {
                        recipient_ids.push(t.id);
                    }
                }
                // A new staged user takes messages unless the sender is
                // silenced or may not send any.
                None => {
                    if !sender_can && !args.skip_validations {
                        return Ok(Outcome::Invalid(vec![
                            ctx.t(&format!("{base}.cant_send_pm")),
                        ]));
                    }
                    if guardian.is_silenced() && !args.skip_validations {
                        return Ok(Outcome::Invalid(vec![
                            ctx.t(&format!("{base}.cant_send_pm")),
                        ]));
                    }
                    if !staged.iter().any(|(_, e)| e == email) {
                        staged.push((recipient_ids.len() + staged.len(), email.clone()));
                    }
                }
            }
        }
        // add_groups: each group the sender may message (unless
        // skip_validations), all of them found.
        let groups = super::group_messages::find(&mut conn, &args.pm_groups).await?;
        let notify_moderators = args.subtype.as_deref() == Some("notify_moderators");
        for group in &groups {
            if !args.skip_validations
                && !super::group_messages::can_send_to(
                    &mut conn,
                    guardian,
                    sender_can,
                    notify_moderators,
                    group,
                )
                .await?
            {
                return Ok(Outcome::Invalid(vec![
                    ctx.t(&format!("{base}.cant_send_pm")),
                ]));
            }
        }
        if groups.len() != args.pm_groups.len() {
            return Ok(Outcome::Invalid(vec![
                ctx.t(&format!("{base}.target_group_not_found")),
            ]));
        }
        if let Some(plan) = plan.as_mut() {
            plan.pm_recipients = Some(recipient_ids);
            plan.pm_staged = staged;
            plan.pm_groups = groups;
        }
    }
    let poster = Poster::load(&mut conn, &user).await?;
    drop(conn);

    // The limits after_create performs (disable_rate_limits! with
    // skip_validations), rolled back when the create fails.
    let mut performed = Performed::new(pool, ctx.config.rate_limits && !args.skip_validations);
    // PostCreator#create's transaction.
    let mut tx = pool.begin().await?;
    let urls = Urls {
        config: ctx.config,
        settings: s,
    };
    let draft_key = match (&args.draft_key, &reply_topic) {
        (Some(key), _) => key.clone(),
        (None, Some(t)) => format!("topic_{}", t.id),
        (None, None) => "new_topic".to_string(),
    };
    // build_post_stats
    let sequence = current_draft_sequence(&mut tx, user.id, &draft_key).await?;
    let drafts_saved: i32 = sqlx::query_scalar(
        "SELECT revisions FROM drafts WHERE sequence = $1 AND user_id = $2 AND draft_key = $3 LIMIT 1",
    )
    .bind(sequence)
    .bind(user.id)
    .bind(&draft_key)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(0);

    // create_topic
    let mut tag_hooks: Vec<crate::tagging::CreatedTag> = Vec::new();
    let (topic_id, topic_title, topic_word_count, category_id, wiki) = match (&reply_topic, &plan) {
        (Some(t), _) => (
            t.id,
            t.title.clone(),
            t.word_count.unwrap_or(0),
            t.category_id,
            false,
        ),
        (None, Some(plan)) => {
            let id = match create_topic(
                &mut tx,
                ctx,
                guardian,
                args.creator_guardian.as_ref().unwrap_or(guardian),
                &poster,
                plan,
                args.advance_draft,
                &mut performed,
                args.skip_validations,
                &mut tag_hooks,
            )
            .await?
            {
                Ok(id) => id,
                Err(refused) => {
                    tx.rollback().await?;
                    performed.undo().await?;
                    return Ok(match refused {
                        TopicRefused::RateLimited(refused) => Outcome::RateLimited(refused),
                        TopicRefused::Invalid(errors) => Outcome::Invalid(errors),
                    });
                }
            };
            (
                id,
                plan.title.clone(),
                0,
                plan.category_id,
                plan.all_topics_wiki,
            )
        }
        (None, None) => unreachable!("a new topic always has a plan"),
    };
    // create_post_notice
    let notice = post_notice(&mut tx, ctx, user.id).await?;

    // save_post: before_create_tasks (reply info, word count, post number
    // through Topic.next_post_number, cook, sort order).
    let reply_to_user_id: Option<i32> = match reply_to_post_number {
        Some(n) => {
            let reply: Option<(Option<i32>, i32)> = sqlx::query_as(
                "SELECT user_id, post_type FROM posts WHERE topic_id = $1 AND post_number = $2 FOR UPDATE",
            )
            .bind(topic_id)
            .bind(n)
            .fetch_optional(&mut *tx)
            .await?;
            if reply.is_some_and(|(_, t)| t == post_types::WHISPER) {
                return Err(Unsupported("replies to whispers").into());
            }
            reply.and_then(|(u, _)| u)
        }
        None => None,
    };
    let words = word_count(&raw);
    let highest: i32 =
        sqlx::query_scalar("SELECT COALESCE(MAX(post_number), 0) FROM posts WHERE topic_id = $1")
            .bind(topic_id)
            .fetch_one(&mut *tx)
            .await?;
    let reply_sql = if reply_to_post_number.is_some() {
        ", reply_count = reply_count + 1"
    } else {
        ""
    };
    let post_number: i32 = sqlx::query_scalar(&format!(
        "UPDATE topics SET highest_staff_post_number = $1 + 1, highest_post_number = $1 + 1{reply_sql}, \
         posts_count = posts_count + 1 WHERE id = $2 RETURNING highest_post_number"
    ))
    .bind(highest)
    .bind(topic_id)
    .fetch_one(&mut *tx)
    .await?;
    if s.get("generate_topic_og_image")?.truthy() {
        return Err(Unsupported("generate_topic_og_image").into());
    }
    // discourse-post-voting's before_create, after core's: an emailed reply
    // to the question is an answer (its reply_to_user_id and the topic's
    // reply count stay).
    let reply_to_post_number = if args.email.is_some() {
        crate::plugins::post_voting::lifecycle::email_reply_to(
            &mut tx,
            s,
            topic_id,
            reply_to_post_number,
        )
        .await?
    } else {
        reply_to_post_number
    };
    // setup_post's extract_quoted_post_numbers: the quote_count.
    let quoted = super::quotes::quoted_post_numbers(&raw, topic_id, post_number);
    let (post_id, created_at): (i32, NaiveDateTime) = sqlx::query_as(
        "INSERT INTO posts (user_id, topic_id, post_number, raw, cooked, created_at, updated_at, \
                            reply_to_post_number, reply_to_user_id, last_editor_id, word_count, \
                            sort_order, last_version_at, baked_at, baked_version, wiki, quote_count, \
                            via_email, raw_email) \
         VALUES ($1, $2, $3, $4, $5, COALESCE($11, clock_timestamp()), clock_timestamp(), $6, $7, $1, $8, $3, \
                 clock_timestamp(), clock_timestamp(), $9, $10, $14, $12, $13) \
         RETURNING id, created_at",
    )
    .bind(user.id)
    .bind(topic_id)
    .bind(post_number)
    .bind(&raw)
    .bind(&cooked)
    .bind(reply_to_post_number)
    .bind(reply_to_user_id)
    .bind(words)
    .bind(BAKED_VERSION)
    .bind(wiki)
    .bind(args.email.as_ref().map(|e| e.created_at))
    .bind(args.email.is_some())
    .bind(args.email.as_ref().map(|e| e.raw_email.as_str()))
    .bind(quoted.len() as i32)
    .fetch_one(&mut *tx)
    .await?;
    // Post's after_create: its limits.
    if let Err(refused) = performed
        .perform(limits::post_limiters(s, &poster, post_number)?)
        .await?
    {
        tx.rollback().await?;
        performed.undo().await?;
        return Ok(Outcome::RateLimited(refused));
    }
    sqlx::query(
        "INSERT INTO post_stats (post_id, drafts_saved, typing_duration_msecs, composer_open_duration_msecs, \
                                 writing_device, writing_device_user_agent, composer_version, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, clock_timestamp(), clock_timestamp())",
    )
    .bind(post_id)
    .bind(drafts_saved)
    .bind(args.typing_duration_msecs.unwrap_or(0) as i32)
    .bind(args.composer_open_duration_msecs.unwrap_or(0) as i32)
    // PostsController sends the device; email and system messages have none.
    .bind(args.from_composer.then(|| device(args.user_agent.as_deref())))
    .bind(args.user_agent.as_deref().map(truncate_400))
    .bind(args.composer_version)
    .execute(&mut *tx)
    .await?;
    // discourse-poll's after_save. Its :post_created schedules the
    // ClosePoll jobs, pushed at once (Jobs.enqueue_at), ahead of the jobs
    // the transaction defers.
    crate::plugins::poll::create(&mut tx, post_id, &new_polls).await?;
    if !new_polls.is_empty() {
        crate::plugins::poll::schedule_jobs(&mut tx, post_id).await?;
    }
    if let Some(notice) = notice {
        sqlx::query(
            "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
             VALUES ($1, 'notice', $2, clock_timestamp(), clock_timestamp())",
        )
        .bind(post_id)
        .bind(notice.to_string())
        .execute(&mut *tx)
        .await?;
    }

    // UserActionManager.post_created: a reply, or in a message one row per
    // allowed user (the poster's NEW_PRIVATE_MESSAGE, the others'
    // GOT_PRIVATE_MESSAGE).
    if !new_topic {
        if private_message {
            let allowed: Vec<i32> = sqlx::query_scalar(
                "SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 ORDER BY id",
            )
            .bind(topic_id)
            .fetch_all(&mut *tx)
            .await?;
            for allowed_user in allowed {
                let action = if allowed_user == user.id {
                    user_actions::NEW_PRIVATE_MESSAGE
                } else {
                    user_actions::GOT_PRIVATE_MESSAGE
                };
                super::log_user_action_by(
                    &mut tx,
                    action,
                    allowed_user,
                    user.id,
                    topic_id,
                    post_id,
                    created_at,
                )
                .await?;
            }
        } else {
            super::log_user_action(
                &mut tx,
                user_actions::REPLY,
                user.id,
                topic_id,
                post_id,
                created_at,
            )
            .await?;
        }
    }
    // extract_links: TopicLink, then QuotedPost.
    let hostname = urls.current_hostname()?;
    links::extract_from(
        &mut tx,
        &links::Site {
            hostname: &hostname,
            base_path: &base_path,
            base_url_no_prefix: &urls.base_url_no_prefix()?,
            settings: s,
        },
        &LinkPost {
            id: post_id,
            user_id: user.id,
            topic_id,
            cooked: &cooked,
        },
    )
    .await?;
    super::rebake::extract_quoted_posts(
        &mut tx,
        post_id,
        topic_id,
        post_number,
        reply_to_post_number,
        false,
        &cooked,
    )
    .await?;
    // track_topic
    change_topic_user(
        ctx.bus,
        &mut tx,
        user.id,
        topic_id,
        &[
            TopicUserAttr::Posted(true),
            TopicUserAttr::LastReadPostNumber(post_number),
            TopicUserAttr::LastPostedAtNow,
        ],
    )
    .await?;
    record_timing(&mut tx, topic_id, user.id, post_number, 5000).await?;
    // A staged user watches (auto_notification_for_staging); in a message
    // the level is left as the topic user was made.
    if user.staged {
        change_topic_user(
            ctx.bus,
            &mut tx,
            user.id,
            topic_id,
            &[TopicUserAttr::NotificationLevel(
                notification_levels::WATCHING,
                notification_reasons::AUTO_WATCH,
            )],
        )
        .await?;
    } else if !private_message {
        let replying_level: Option<i32> = sqlx::query_scalar(
            "SELECT notification_level_when_replying FROM user_options WHERE user_id = $1",
        )
        .bind(user.id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
        super::auto_notification(
            ctx.bus,
            &mut tx,
            user.id,
            topic_id,
            notification_reasons::CREATED_POST,
            replying_level.unwrap_or(notification_levels::TRACKING),
        )
        .await?;
    }
    // update_topic_stats
    let excerpt = if new_topic {
        Some(crate::excerpt::excerpt(
            &cooked,
            s.get("topic_excerpt_maxlength")?.to_i().max(0) as usize,
            &crate::excerpt::Options {
                strip_links: true,
                image_mode: crate::excerpt::ImageMode::Strip,
                ..Default::default()
            },
            crate::excerpt::Plugins::from_settings(s)?,
        ))
    } else {
        None
    };
    sqlx::query(
        "UPDATE topics SET updated_at = clock_timestamp(), last_posted_at = $2, last_post_user_id = $3, \
                word_count = $4, bumped_at = $2, excerpt = COALESCE($5, excerpt) WHERE id = $1",
    )
    .bind(topic_id)
    .bind(created_at)
    .bind(user.id)
    .bind(topic_word_count + words)
    .bind(excerpt)
    .execute(&mut *tx)
    .await?;
    crate::topic_timer::update_topic_auto_close(&mut tx, ctx, topic_id).await?;
    // update_user_counts
    let first_post = sqlx::query(
        "UPDATE user_stats SET first_post_created_at = $2 \
         WHERE user_id = $1 AND first_post_created_at IS NULL",
    )
    .bind(user.id)
    .bind(created_at)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    // UserStat's and User's after_save: the user's badges, queued.
    if first_post || !private_message {
        crate::badge_granter::queue_user_change(&mut tx, s, user.id).await?;
    }
    let column = if new_topic {
        "topic_count"
    } else {
        "post_count"
    };
    // UserStatCountUpdater.increment! and last_posted_at skip messages.
    if !private_message {
        sqlx::query(&format!(
            "UPDATE user_stats SET {column} = {column} + 1 WHERE user_id = $1"
        ))
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE users SET last_posted_at = $2, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(user.id)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
    }
    // delete_owned_bookmarks (on_owner_reply), then topic_users.bookmarked
    sqlx::query(
        "DELETE FROM bookmarks WHERE id IN (SELECT bookmarks.id FROM bookmarks \
           LEFT JOIN posts ON posts.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Post' \
           LEFT JOIN topics ON (topics.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Topic') \
                            OR (topics.id = posts.topic_id) \
           WHERE bookmarks.user_id = $1 AND (topics.id = $2 OR posts.topic_id = $2) \
             AND posts.deleted_at IS NULL AND topics.deleted_at IS NULL \
             AND bookmarks.auto_delete_preference = 2)",
    )
    .bind(user.id)
    .bind(topic_id)
    .execute(&mut *tx)
    .await?;
    let bookmarked: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM bookmarks \
           LEFT JOIN posts ON posts.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Post' \
           LEFT JOIN topics ON (topics.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Topic') \
                            OR (topics.id = posts.topic_id) \
           WHERE bookmarks.user_id = $1 AND (topics.id = $2 OR posts.topic_id = $2) \
             AND posts.deleted_at IS NULL AND topics.deleted_at IS NULL)",
    )
    .bind(user.id)
    .bind(topic_id)
    .fetch_one(&mut *tx)
    .await?;
    change_topic_user(
        ctx.bus,
        &mut tx,
        user.id,
        topic_id,
        &[TopicUserAttr::Bookmarked(bookmarked)],
    )
    .await?;
    // @post.link_post_uploads
    crate::upload_references::link_post_uploads(
        &mut tx,
        s,
        &urls.current_hostname()?,
        &crate::file_store::FileStore::for_site(ctx.config, s)?,
        post_id,
        &cooked,
    )
    .await?;
    if private_message {
        message_posted(&mut tx, ctx, topic_id, user.id, guardian.is_staff()).await?;
    }
    // DraftSequence.next!(user, draft_key) when the composer asked
    // (advance_draft).
    if args.advance_draft {
        next_draft_sequence(&mut tx, user.id, &draft_key).await?;
    }
    super::quotes::save_reply_relationships(
        &mut tx,
        super::quotes::ReplyingPost {
            id: post_id,
            topic_id,
            post_type: post_types::REGULAR,
            reply_to_post_number,
            deleted: false,
        },
        &quoted,
    )
    .await?;
    // Email::Receiver links its incoming email to the post before the
    // jobs go out.
    if let Some(email) = &args.email {
        sqlx::query("UPDATE incoming_emails SET topic_id = $2, post_id = $3 WHERE id = $1")
            .bind(email.incoming_email_id)
            .bind(topic_id)
            .bind(post_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE posts SET outbound_message_id = $2, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(post_id)
        .bind(&email.message_id)
        .execute(&mut *tx)
        .await?;
    }
    // The new tags' after_commit: their tag_created web hooks.
    crate::tagging::enqueue_tag_created(&mut tx, &tag_hooks).await?;
    // PostCreator#publish: replies only (the first post's topic is news to
    // the topic list instead).
    if post_number != 1 {
        crate::bus::publish_post_change(ctx, &mut tx, post_id, "created", Map::new(), false)
            .await?;
    } else {
        // PostJobsEnqueuer#after_topic_create
        if private_message {
            crate::pm_tracking_state::publish_new(ctx.bus, &mut tx, topic_id).await?;
        } else {
            crate::topic_tracking_state::publish_new(ctx.bus, ctx.settings, &mut tx, topic_id)
                .await?;
        }
    }
    // trigger_after_events: topic_created, then post_created, to the web
    // hooks listening (their jobs come before PostJobsEnqueuer's).
    // Email::Receiver skips them and triggers both after the jobs.
    if args.email.is_none() {
        after_events(&mut tx, ctx, new_topic, topic_id, post_id).await?;
    }
    // PostJobsEnqueuer#enqueue_jobs, committed with the post.
    crate::jobs::enqueue(
        &mut tx,
        "post_alert",
        json!({"post_id": post_id, "new_record": true, "options": args.post_alert_options}),
    )
    .await?;
    crate::jobs::enqueue(
        &mut tx,
        "feature_topic_users",
        json!({"topic_id": topic_id}),
    )
    .await?;
    crate::jobs::enqueue(
        &mut tx,
        "process_post",
        json!({"bypass_bump": false, "cooking_options": null, "new_post": true, "post_id": post_id, "skip_pull_hotlinked_images": false}),
    )
    .await?;
    crate::jobs::enqueue(
        &mut tx,
        "post_update_topic_tracking_state",
        json!({"post_id": post_id}),
    )
    .await?;
    // after_post_create: mailing lists are not told about messages.
    if !private_message {
        crate::jobs::enqueue_in(
            &mut tx,
            s.get("email_time_window_mins")?.to_i() * 60,
            "notify_mailing_list_subscribers",
            json!({"post_id": post_id}),
        )
        .await?;
    }
    if args.email.is_some() {
        after_events(&mut tx, ctx, new_topic, topic_id, post_id).await?;
    }
    crate::badge_granter::queue(
        &mut tx,
        s,
        crate::badge_granter::Trigger::PostRevision,
        &[post_id],
    )
    .await?;
    tx.commit().await?;
    performed.commit();

    // After the transaction: store_unique_post_key, track_latest_on_category,
    // auto_close, then the deferred search index.
    let mut conn = pool.acquire().await?;
    super::validate::store_unique_post_key(&mut conn, s, user.id, private_message, &raw, post_id)
        .await?;
    if let Some(category_id) = category_id {
        if new_topic {
            sqlx::query(
                "UPDATE categories SET latest_topic_id = $2, latest_post_id = $3 WHERE id = $1",
            )
            .bind(category_id)
            .bind(topic_id)
            .bind(post_id)
            .execute(&mut *conn)
            .await?;
        } else {
            sqlx::query("UPDATE categories SET latest_post_id = $2 WHERE id = $1")
                .bind(category_id)
                .bind(post_id)
                .execute(&mut *conn)
                .await?;
        }
    }
    // DiscourseEvent :post_created, to discourse-poll: a reply's polls.
    if !new_polls.is_empty() {
        crate::plugins::poll::publish_created(ctx, &mut conn, guardian, post_id).await?;
    }
    // ... and to discourse-assign: auto assigning by mention.
    crate::plugins::assign::lifecycle::post_saved(&mut conn, ctx, post_id).await?;
    let posts_count: i32 = sqlx::query_scalar("SELECT posts_count FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    let auto_close = s.get("auto_close_topics_post_count")?.to_i();
    if auto_close > 0 && auto_close <= i64::from(posts_count) {
        return Err(Unsupported("auto_close_topics_post_count").into());
    }
    let (category_name, tags): (Option<String>, Option<String>) = sqlx::query_as(&format!(
        "SELECT (SELECT name FROM categories WHERE id = $2), {}",
        search_index::topic_tag_names_sql("$1")
    ))
    .bind(topic_id)
    .bind(category_id)
    .fetch_one(&mut *conn)
    .await?;
    search_index::index_post(
        &mut conn,
        s,
        &BaseUrls {
            base_path: &base_path,
            base_url_no_prefix: urls.base_url_no_prefix()?,
            store: crate::file_store::FileStore::for_site(urls.config, s)?,
        },
        &PostIndex {
            post_id,
            topic_id,
            is_first_post: new_topic,
            topic_title: &topic_title,
            category_name: category_name.as_deref(),
            tag_names: tags.as_deref(),
            cooked: &cooked,
            private_message,
        },
    )
    .await?;
    Ok(Outcome::Created { post_id })
}

/// DiscourseEvent :topic_created (a new topic) and :post_created, to the
/// web hooks: the payloads of PostCreator's objects.
async fn after_events(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    new_topic: bool,
    topic_id: i32,
    post_id: i32,
) -> Result<(), AppError> {
    let payloads = crate::web_hooks::Payloads {
        settings: ctx.settings,
        i18n: ctx.i18n,
        config: ctx.config,
    };
    if new_topic {
        crate::web_hooks::enqueue_topic_hooks(
            &mut *conn,
            &payloads,
            "topic_created",
            topic_id,
            None,
        )
        .await?;
    }
    crate::web_hooks::enqueue_post_hooks(
        conn,
        &payloads,
        "post_created",
        post_id,
        crate::topic_view::Held {
            topic_loaded: true,
            just_created: true,
            topic_posts_count: None,
        },
    )
    .await
}

/// `Post#omit_nofollow?` for the author.
pub async fn omit_nofollow(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
) -> Result<bool, AppError> {
    let tl3_no_follow = ctx.settings.get("tl3_links_no_follow")?.truthy();
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(admin OR moderator OR (NOT $2 AND (staged OR trust_level >= 3)), FALSE) \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .bind(tl3_no_follow)
    .fetch_optional(&mut *conn)
    .await?
    .unwrap_or(false))
}

/// PostCreator's message steps: `ensure_in_allowed_users` for a staff
/// author, then `unarchive_message`.
pub(crate) async fn message_posted(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    user_id: i32,
    staff: bool,
) -> Result<(), AppError> {
    // ensure_in_allowed_users: a staff author not allowed (nor in an
    // allowed group) joins the message.
    if staff {
        sqlx::query(
            "INSERT INTO topic_allowed_users (topic_id, user_id, created_at, updated_at) \
             SELECT $1, $2, clock_timestamp(), clock_timestamp() \
             WHERE NOT EXISTS (SELECT 1 FROM topic_allowed_users WHERE topic_id = $1 AND user_id = $2) \
               AND NOT EXISTS (SELECT 1 FROM topic_allowed_groups WHERE topic_id = $1 \
                 AND group_id IN (SELECT group_id FROM group_users WHERE user_id = $2))",
        )
        .bind(topic_id)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    }
    // unarchive_message: back in each archiver's inbox, unless they
    // muted it.
    let archivers: Vec<i32> =
        sqlx::query_scalar("SELECT user_id FROM user_archived_messages WHERE topic_id = $1")
            .bind(topic_id)
            .fetch_all(&mut *conn)
            .await?;
    for archiver in archivers {
        if crate::archived_messages::user_move_to_inbox(ctx.bus, &mut *conn, archiver, topic_id)
            .await?
        {
            crate::plugins::assign::lifecycle::move_to_inbox(&mut *conn, ctx, topic_id, false)
                .await?;
        }
    }
    let archiving_groups: Vec<i32> =
        sqlx::query_scalar("SELECT group_id FROM group_archived_messages WHERE topic_id = $1")
            .bind(topic_id)
            .fetch_all(&mut *conn)
            .await?;
    for group_id in archiving_groups {
        if crate::archived_messages::group_move_to_inbox(
            ctx.bus,
            &mut *conn,
            group_id,
            topic_id,
            Some(user_id),
        )
        .await?
        {
            crate::plugins::assign::lifecycle::move_to_inbox(&mut *conn, ctx, topic_id, true)
                .await?;
        }
    }
    Ok(())
}

/// `create_post_notice`: a first post gets the new-user notice, one after
/// a long absence the returning-user notice.
/// `handle_spam` for a post has_host_spam? refused: a message to the
/// moderators naming its hosts (GroupMessage, once a day per user).
async fn spam_post_blocked(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    username: &str,
    hosts: &[String],
) -> Result<(), AppError> {
    let mut conn = pool.acquire().await?;
    // Group[:moderators].name
    let moderators: String = sqlx::query_scalar("SELECT name FROM groups WHERE id = $1")
        .bind(crate::guardian::auto_groups::MODERATORS as i32)
        .fetch_one(&mut *conn)
        .await?;
    drop(conn);
    crate::group_message::GroupMessage {
        group_name: &moderators,
        message_type: "spam_post_blocked",
        limit_once_per: Some(86_400),
        message_params: vec![("domains".to_string(), hosts.join(", "))],
        user: Some(username),
    }
    .create(pool, ctx)
    .await?;
    Ok(())
}

/// `SpamRule::FlagSockpuppets#reply_is_from_sockpuppet?` short of
/// `ScreenedIpAddress.is_allowed?`, which only narrows it: the topic's
/// first post is by another user, neither of them staff or staged, with
/// the replying new user's IP address.
async fn reply_is_from_sockpuppet(
    conn: &mut PgConnection,
    user_id: i32,
    topic_id: i32,
) -> Result<bool, AppError> {
    let day_ago = crate::clock::now_naive() - chrono::Duration::hours(24);
    let found: Option<bool> = sqlx::query_scalar(
        "SELECT NOT (f.admin OR f.moderator) AND NOT (u.admin OR u.moderator) \
                AND NOT f.staged AND NOT u.staged AND u.id <> f.id \
                AND u.ip_address IS NOT DISTINCT FROM f.ip_address \
                AND (u.created_at >= $3 OR u.trust_level = 0) AND u.trust_level < 2 \
         FROM users u, \
              (SELECT user_id FROM posts WHERE topic_id = $2 AND deleted_at IS NULL \
               ORDER BY post_number LIMIT 1) p \
         JOIN users f ON f.id = p.user_id \
         WHERE u.id = $1",
    )
    .bind(user_id)
    .bind(topic_id)
    .bind(day_ago)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(found == Some(true))
}

pub(crate) async fn post_notice(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
) -> Result<Option<serde_json::Value>, AppError> {
    if user_id <= 0 {
        return Ok(None);
    }
    let last: Option<NaiveDateTime> = sqlx::query_scalar(
        "SELECT created_at FROM posts WHERE user_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(last) = last else {
        return Ok(Some(json!({"type": "new_user"})));
    };
    let days = ctx.settings.get("returning_users_days")?.to_i();
    if days > 0 {
        let returning: bool = sqlx::query_scalar("SELECT $1 < now() - make_interval(days => $2)")
            .bind(last)
            .bind(days as i32)
            .fetch_one(&mut *conn)
            .await?;
        if returning {
            return Err(Unsupported("returning-user post notices").into());
        }
    }
    Ok(None)
}

/// TopicCreator#valid?'s tags: with a category the topic was given and a
/// guardian that may tag it, `DiscourseTagging.validate_category_tags`.
pub(super) async fn validate_topic_tags(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    category_id: Option<i32>,
    private_message: bool,
    names: &[String],
) -> Result<Vec<String>, AppError> {
    let s = ctx.settings;
    let Some(category_id) = category_id else {
        return Ok(Vec::new());
    };
    if !crate::tagging::can_tag(guardian, s, private_message)? {
        return Ok(Vec::new());
    }
    let Some(category) = crate::tagging::TagCategory::load(conn, category_id).await? else {
        return Ok(Vec::new());
    };
    crate::tagging::validate_category_tags(conn, s, ctx.i18n, guardian, &category, names).await
}

/// `TopicCreator#setup_tags`: the tags asked for (`DiscourseTagging.tag_topic`,
/// the topic rolled back with its errors and unable_to_tag when they fail,
/// or with skip_validations `add_or_create_tags_by_name`), then those its
/// tag watched words add, tagged as the system user (whose failure leaves
/// the tags as they were). None when nothing tags the topic.
async fn setup_tags(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    plan: &TopicPlan,
    pm: bool,
    skip_validations: bool,
) -> Result<Result<Option<crate::tagging::Tagging>, Vec<String>>, AppError> {
    use crate::tagging::{self, TaggedTopic};
    let s = ctx.settings;
    let topic = TaggedTopic {
        id: None,
        private_message: pm,
        category_id: plan.tag_category_id,
    };
    let mut tagging: Option<tagging::Tagging> = None;
    if let Some(param) = &plan.tags {
        match tagging::tag_topic(conn, s, ctx.i18n, guardian, &topic, param).await? {
            Ok(t) => tagging = Some(t),
            Err(rejected) => {
                let mut errors = rejected.errors;
                if skip_validations {
                    let names = tagging::names_for_tagging(conn, param).await?;
                    let mut added = tagging::add_or_create_tags_by_name(conn, s, &names).await?;
                    let mut created = rejected.created;
                    created.append(&mut added.created);
                    added.created = created;
                    tagging = Some(added);
                } else {
                    errors.push(
                        ctx.t("activerecord.errors.models.topic.attributes.base.unable_to_tag"),
                    );
                    return Ok(Err(errors));
                }
            }
        }
    }
    let current: Vec<String> = tagging
        .as_ref()
        .map(|t| t.tags.iter().map(|t| t.name.clone()).collect())
        .unwrap_or_default();
    if let Some(names) = tagging::watched_word_tags(conn, s, &plan.tag_text, &current).await? {
        let system = crate::web_hooks::system_guardian(conn).await?;
        match tagging::tag_topic_by_names(conn, s, ctx.i18n, &system, &topic, &names, false).await?
        {
            Ok(mut by_words) => {
                // `topic.tags = tags` on the unsaved topic keeps the tags it
                // already holds, in their order, and appends the others.
                let before = tagging.unwrap_or_default();
                let mut ordered: Vec<tagging::HeldTag> = before
                    .tags
                    .iter()
                    .filter(|t| by_words.tags.contains(t))
                    .cloned()
                    .collect();
                for t in by_words.tags {
                    if !ordered.contains(&t) {
                        ordered.push(t);
                    }
                }
                by_words.tags = ordered;
                let mut created = before.created;
                created.extend(by_words.created.iter().cloned());
                tagging = Some(tagging::Tagging {
                    created,
                    ..by_words
                });
            }
            // The topic keeps the tags it had; tags created on the way stay.
            Err(rejected) => {
                let mut kept = tagging.unwrap_or_default();
                kept.created.extend(rejected.created);
                tagging = Some(kept);
            }
        }
    }
    Ok(Ok(tagging))
}

/// `TopicCreator#create` for a regular topic: the row, the category's
/// count and featured topics, the creator watching it, the NEW_TOPIC
/// action and the topic's draft sequence.
#[allow(clippy::too_many_arguments)]
async fn create_topic(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    tag_guardian: &Guardian,
    poster: &Poster,
    plan: &TopicPlan,
    advance_draft: bool,
    performed: &mut Performed,
    skip_validations: bool,
    tag_hooks: &mut Vec<crate::tagging::CreatedTag>,
) -> Result<Result<i32, TopicRefused>, AppError> {
    let s = ctx.settings;
    let user_id = poster.id;
    if s.get("slug_generation_method")?.to_s() != "ascii" {
        return Err(Unsupported("slug_generation_method other than ascii").into());
    }
    let pm = plan.pm_recipients.is_some();
    // setup_tags, before the topic is saved.
    let tagging = match setup_tags(conn, ctx, tag_guardian, plan, pm, skip_validations).await? {
        Ok(tagging) => tagging,
        Err(errors) => return Ok(Err(TopicRefused::Invalid(errors))),
    };
    if let Some(t) = &tagging {
        tag_hooks.extend(t.created.iter().cloned());
        // The unsaved tag fails the topic's autosaved association.
        if t.has_invalid_tag() && skip_validations {
            return Err(
                Unsupported("saving a tag Tag's validations refuse, without validations").into(),
            );
        }
        if t.has_invalid_tag() {
            return Ok(Err(TopicRefused::Invalid(vec![
                crate::tagging::invalid_tags_message(ctx.i18n),
            ])));
        }
    }
    // process_private_message's add_emails: the staged users, before the
    // topic is saved, at their place among the recipients.
    let mut recipients = plan.pm_recipients.clone();
    if let Some(list) = recipients.as_mut() {
        for (position, email) in &plan.pm_staged {
            let id = crate::routes::create_staged_user(&mut *conn, ctx.i18n, s, ctx.config, email)
                .await?;
            list.insert((*position).min(list.len()), id);
        }
    }
    let slug = slug_for(&plan.title, &s.get("default_locale")?.to_s())?;
    let fancy_title = crate::posting::text::fancy_title(
        &plan.title,
        crate::posting::text::EmojiEscape::from_settings(s)?,
    );
    let (topic_id, created_at): (i32, NaiveDateTime) = sqlx::query_as(
        "INSERT INTO topics (title, fancy_title, slug, user_id, last_post_user_id, visible, category_id, \
                             archetype, subtype, bumped_at, created_at, updated_at, slow_mode_seconds) \
         VALUES ($1, $2, $3, $4, $4, TRUE, $5, $7, $8, clock_timestamp(), clock_timestamp(), \
                 clock_timestamp(), COALESCE($6, 0)) \
         RETURNING id, created_at",
    )
    .bind(&plan.title)
    .bind(&fancy_title)
    .bind(&slug)
    .bind(user_id)
    .bind(plan.category_id)
    .bind(plan.slow_mode_seconds)
    .bind(if pm { "private_message" } else { "regular" })
    .bind(if pm {
        Some(plan.subtype.as_deref().unwrap_or("user_to_user"))
    } else {
        plan.subtype.as_deref()
    })
    .fetch_one(&mut *conn)
    .await?;
    // HasCustomFields: the topic's custom fields, saved with it.
    for (name, value) in &plan.custom_fields {
        sqlx::query(
            "INSERT INTO topic_custom_fields (topic_id, name, value, created_at, updated_at) \
             VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
        )
        .bind(topic_id)
        .bind(name)
        .bind(value)
        .execute(&mut *conn)
        .await?;
    }
    // The topic tags, autosaved with the topic, and its after_save.
    if let Some(t) = &tagging {
        let saved = crate::tagging::SavedTopic {
            id: topic_id,
            private_message: pm,
            category_id: plan.category_id,
        };
        crate::tagging::apply(conn, &saved, t).await?;
        crate::tagging::after_tags_saved(conn, topic_id).await?;
    }
    // before_save on create: the category's auto close, saved with the
    // topic (topic_timers <<).
    crate::topic_timer::inherit_auto_close_from_category(
        &mut *conn,
        ctx,
        topic_id,
        crate::topic_timer::types::CLOSE,
    )
    .await?;
    // Topic's after_create: its limits, before the other callbacks.
    if let Err(refused) = performed
        .perform(limits::topic_limiters(
            s,
            poster,
            pm,
            pm.then(|| plan.subtype.as_deref().unwrap_or("user_to_user")),
        )?)
        .await?
    {
        return Ok(Err(TopicRefused::RateLimited(refused)));
    }
    // process_private_message: the recipients, then the sender.
    if let Some(recipients) = &recipients {
        let mut allowed = recipients.clone();
        if !allowed.contains(&user_id) {
            allowed.push(user_id);
        }
        for allowed_user in allowed {
            sqlx::query(
                "INSERT INTO topic_allowed_users (topic_id, user_id, created_at, updated_at) \
                         VALUES ($1, $2, clock_timestamp(), clock_timestamp())",
            )
            .bind(topic_id)
            .bind(allowed_user)
            .execute(&mut *conn)
            .await?;
        }
    }
    // add_groups: the allowed groups, each marked as having messages
    // (update_columns: updated_at stays).
    for group in &plan.pm_groups {
        sqlx::query("INSERT INTO topic_allowed_groups (topic_id, group_id) VALUES ($1, $2)")
            .bind(topic_id)
            .bind(group.id)
            .execute(&mut *conn)
            .await?;
        if !group.has_messages {
            sqlx::query("UPDATE groups SET has_messages = TRUE WHERE id = $1")
                .bind(group.id)
                .execute(&mut *conn)
                .await?;
        }
    }
    // after_create: changed_to_category, then the draft sequence when
    // asked (advance_draft).
    if let Some(category_id) = plan.category_id {
        let is_definition: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE topic_id = $1)")
                .bind(topic_id)
                .fetch_one(&mut *conn)
                .await?;
        if !is_definition {
            sqlx::query("UPDATE categories SET topic_count = topic_count + 1 WHERE id = $1")
                .bind(category_id)
                .execute(&mut *conn)
                .await?;
            feature_topics_for(conn, s, category_id, None).await?;
        }
    }
    if advance_draft {
        next_draft_sequence(conn, user_id, &format!("topic_{topic_id}")).await?;
    }
    // set_author_notification_level: watch it.
    change_topic_user(
        ctx.bus,
        conn,
        user_id,
        topic_id,
        &[TopicUserAttr::NotificationLevel(
            notification_levels::WATCHING,
            notification_reasons::CREATED_TOPIC,
        )],
    )
    .await?;
    // apply_pm_recipient_notification_levels: the recipients watch too
    // (TopicNotifier#watch!, so the reason is user_changed).
    let allowed: Vec<i32> = if pm {
        sqlx::query_scalar(
            "SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 ORDER BY id",
        )
        .bind(topic_id)
        .fetch_all(&mut *conn)
        .await?
    } else {
        Vec::new()
    };
    for &recipient in allowed.iter().filter(|&&u| u != -1 && u != user_id) {
        change_topic_user(
            ctx.bus,
            conn,
            recipient,
            topic_id,
            &[TopicUserAttr::NotificationLevel(
                notification_levels::WATCHING,
                notification_reasons::USER_CHANGED,
            )],
        )
        .await?;
    }
    // Then each allowed group's members at their group level, unless the
    // group is over group_pm_user_limit (rollback_with! too_large_group).
    let limit = s.get("group_pm_user_limit")?.to_i();
    for group in &plan.pm_groups {
        if i64::from(group.user_count) > limit {
            return Ok(Err(TopicRefused::Invalid(vec![
                ctx.i18n
                    .t_with(
                        "activerecord.errors.models.topic.attributes.base.too_large_group",
                        &[("group_name", &group.name), ("limit", &limit.to_string())],
                    )
                    .unwrap_or_default(),
            ])));
        }
        super::group_messages::apply_default_levels(
            ctx.bus,
            conn,
            group.id,
            topic_id,
            Some(user_id),
            false,
        )
        .await?;
    }
    // UserActionManager.topic_created: the row of the other kind goes; the
    // topic's (or the message's, with GOT_PRIVATE_MESSAGE for each
    // recipient) is logged.
    let (own, other) = if pm {
        (user_actions::NEW_PRIVATE_MESSAGE, user_actions::NEW_TOPIC)
    } else {
        (user_actions::NEW_TOPIC, user_actions::NEW_PRIVATE_MESSAGE)
    };
    sqlx::query(
        "DELETE FROM user_actions WHERE action_type = $1 AND user_id = $2 AND acting_user_id = $2 \
         AND target_topic_id = $3 AND target_post_id = -1",
    )
    .bind(other)
    .bind(user_id)
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    super::log_user_action(conn, own, user_id, topic_id, -1, created_at).await?;
    for &recipient in allowed.iter().filter(|&&u| u != user_id) {
        super::log_user_action_by(
            conn,
            user_actions::GOT_PRIVATE_MESSAGE,
            recipient,
            user_id,
            topic_id,
            -1,
            created_at,
        )
        .await?;
    }
    let _ = guardian;
    Ok(Ok(topic_id))
}

/// `CategoryFeaturedTopic.feature_topics_for(category, existing)`: the
/// system user's list, then the anonymous one, rewritten in rank order
/// unless it equals `existing` (only the periodical updates pass it).
pub async fn feature_topics_for(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    category_id: i32,
    existing: Option<&[i32]>,
) -> Result<(), AppError> {
    let (num_featured, definition, sort_order): (i32, Option<i32>, Option<String>) =
        sqlx::query_as(
            "SELECT num_featured_topics, topic_id, sort_order FROM categories WHERE id = $1",
        )
        .bind(category_id)
        .fetch_one(&mut *conn)
        .await?;
    if sort_order.is_some_and(|o| !o.is_empty()) {
        return Err(Unsupported("featured topics for categories with a sort order").into());
    }
    if s.get("suppress_secured_categories_from_admin")?.truthy() {
        return Err(Unsupported("suppress_secured_categories_from_admin").into());
    }
    let nesting = s.get("max_category_nesting")?.to_i() as i32;
    let definitions_listed = s.get("show_category_definitions_in_topic_lists")?.truthy();
    let mut results: Vec<i32> = Vec::new();
    for anonymous in [false, true] {
        let secured = if anonymous {
            "categories.id IN (SELECT id FROM categories WHERE NOT read_restricted)"
        } else {
            "TRUE"
        };
        let mut except = results.clone();
        if let Some(d) = definition {
            except.push(d);
        }
        let base = format!(
            "FROM topics LEFT OUTER JOIN categories ON categories.id = topics.category_id \
             WHERE topics.deleted_at IS NULL AND topics.archetype <> 'private_message' \
               AND topics.category_id IN (WITH RECURSIVE sub AS (SELECT $1::int AS id, 1 AS depth \
                   UNION SELECT c.id, sub.depth + 1 FROM categories c JOIN sub ON sub.id = c.parent_category_id \
                   WHERE sub.depth < $2) SELECT id FROM sub) \
               {definitions} \
               AND {secured} \
               AND COALESCE(categories.topic_id, 0) <> topics.id \
               AND topics.visible \
               AND NOT (topics.id = ANY($3))",
            definitions = if definitions_listed {
                ""
            } else {
                "AND (categories.topic_id IS DISTINCT FROM topics.id OR topics.category_id = $1)"
            },
        );
        let pinned: Vec<i32> = sqlx::query_scalar(&format!(
            "SELECT topics.id {base} AND topics.pinned_at IS NOT NULL AND topics.category_id = $1 \
             ORDER BY topics.bumped_at DESC, topics.pinned_at DESC"
        ))
        .bind(category_id)
        .bind(nesting)
        .bind(&except)
        .fetch_all(&mut *conn)
        .await?;
        let rest: Vec<i32> = sqlx::query_scalar(&format!(
            "SELECT topics.id {base} AND (topics.pinned_at IS NULL OR topics.category_id <> $1) \
             ORDER BY topics.bumped_at DESC LIMIT $4"
        ))
        .bind(category_id)
        .bind(nesting)
        .bind(&except)
        .bind(i64::from(num_featured))
        .fetch_all(&mut *conn)
        .await?;
        for id in pinned.into_iter().chain(rest) {
            if !results.contains(&id) {
                results.push(id);
            }
        }
    }
    if existing.is_some_and(|e| e == results.as_slice()) {
        return Ok(());
    }
    sqlx::query("DELETE FROM category_featured_topics WHERE category_id = $1")
        .bind(category_id)
        .execute(&mut *conn)
        .await?;
    for (rank, topic_id) in results.iter().enumerate() {
        sqlx::query(
            "INSERT INTO category_featured_topics (category_id, topic_id, rank, created_at, updated_at) \
             VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
        )
        .bind(category_id)
        .bind(topic_id)
        .bind(rank as i32)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}
