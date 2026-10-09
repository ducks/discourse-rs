//! The jobs: `Jobs::<Name>#execute(args)` ports, by name.

use serde_json::{Value, json};
use sqlx::PgConnection;

use super::{Job, JobError};
use crate::posting::links::{self, LinkPost};
use crate::pretty_text::cooked_post_processor;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// Runs a job by its Sidekiq name.
pub async fn run(state: &AppState, job: &Job) -> Result<(), JobError> {
    let result = match job.name.as_str() {
        "post_update_topic_tracking_state" => {
            post_update_topic_tracking_state(state, &job.args).await
        }
        // Publishes reviewable counts to staff on MessageBus, likewise.
        "notify_reviewable" => Ok(()),
        "feature_topic_users" => feature_topic_users(state, &job.args).await,
        "process_post" => process_post(state, &job.args).await,
        "post_alert" => post_alert(state, &job.args).await,
        "user_email" => user_email(state, &job.args, false).await,
        "critical_user_email" => user_email(state, &job.args, true).await,
        "send_system_message" => send_system_message(state, &job.args).await,
        "send_email_login_code" => send_email_login_code(state, &job.args).await,
        // Fires user_added_to_group / user_removed_from_group, which core
        // only hands to web hooks (none are ported).
        "publish_group_membership_updates" => {
            let hooks: bool =
                match sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM web_hooks WHERE active)")
                    .fetch_one(&state.pool)
                    .await
                {
                    Ok(hooks) => hooks,
                    Err(e) => return Err(AppError::from(e).into()),
                };
            if hooks {
                Err(Unsupported("web hooks for group membership").into())
            } else {
                Ok(())
            }
        }
        // UserAuthToken.is_suspicious compares the login's location with the
        // user's earlier ones, through MaxMind's databases; without them
        // (no license key to download them) no login is suspicious.
        "suspicious_login" => {
            if std::env::var("DISCOURSE_MAXMIND_LICENSE_KEY").is_ok_and(|k| !k.is_empty()) {
                Err(Unsupported("suspicious logins by location (MaxMind)").into())
            } else {
                Ok(())
            }
        }
        "process_email" => process_email(state, &job.args).await,
        "Jobs::DiscourseTopicVoting::BackfillBadges" => {
            topic_voting_backfill_badges(state, &job.args).await
        }
        other => {
            return Err(JobError::Unported(format!(
                "not ported yet: the {other} job"
            )));
        }
    };
    Ok(result?)
}

/// Jobs::DiscourseTopicVoting::BackfillBadges: the voting badges
/// backfilled for the topic's first post.
async fn topic_voting_backfill_badges(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(topic_id) = int_arg(args, "topic_id")? else {
        return Ok(());
    };
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    if !s.get("enable_badges")?.truthy() {
        return Ok(());
    }
    let first_post: Option<i32> =
        sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1 AND post_number = 1")
            .bind(topic_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(first_post) = first_post else {
        return Ok(());
    };
    let badges = crate::badge_granter::Badge::enabled_named(
        &mut tx,
        &crate::plugins::topic_voting::BADGE_NAMES,
    )
    .await?;
    for badge in &badges {
        crate::badge_granter::backfill(
            &mut tx,
            &state.bus,
            &s,
            &state.i18n,
            badge,
            Some(crate::badge_granter::Scope::Posts(&[first_post])),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

fn int_arg(args: &Value, key: &str) -> Result<Option<i32>, AppError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v
            .as_i64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        {
            Some(n) => Ok(Some(n as i32)),
            None => Err(Unsupported("job arguments that are not ids").into()),
        },
    }
}

fn bool_arg(args: &Value, key: &str) -> bool {
    matches!(args.get(key), Some(Value::Bool(true)))
}

async fn settings(state: &AppState, conn: &mut PgConnection) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `Jobs::PostUpdateTopicTrackingState`: a regular topic's lists and
/// counts told about a post, in one transaction. Messages
/// (PrivateMessageTopicTrackingState, TopicGroup.new_message_update) are
/// not ported and left alone, as before.
async fn post_update_topic_tracking_state(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(post_id) = args.get("post_id").and_then(Value::as_i64) else {
        return Ok(());
    };
    let mut tx = state.pool.begin().await?;
    let post: Option<(i32, i32, i32, String)> = sqlx::query_as(
        "SELECT p.topic_id, p.post_number, p.post_type, t.archetype FROM posts p \
         JOIN topics t ON t.id = p.topic_id WHERE p.id = $1",
    )
    .bind(post_id as i32)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((topic_id, post_number, post_type, archetype)) = post else {
        return Ok(());
    };
    if post_type == crate::posting::post_types::SMALL_ACTION || archetype == "private_message" {
        return Ok(());
    }
    let s = settings(state, &mut tx).await?;
    use crate::topic_tracking_state as tracking;
    tracking::publish_unmuted(&state.bus, &mut tx, topic_id).await?;
    if post_number > 1 {
        tracking::publish_muted(&state.bus, &mut tx, topic_id).await?;
        tracking::publish_unread(&state.bus, &s, &mut tx, post_id as i32).await?;
    }
    if post_type != crate::posting::post_types::WHISPER {
        tracking::publish_latest(&state.bus, &s, &mut tx, topic_id).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// `Jobs::FeatureTopicUsers` -> `TopicFeaturedUsers#choose`: the four
/// featured posters and the participant count.
async fn feature_topic_users(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(topic_id) = int_arg(args, "topic_id")? else {
        return Err(
            Unsupported("feature_topic_users without a topic_id (InvalidParameters)").into(),
        );
    };
    let mut conn = state.pool.acquire().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1)")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Ok(());
    }
    choose_featured_users(&mut conn, topic_id).await
}

/// `TopicFeaturedUsers#choose` (what Topic#feature_topic_users runs).
pub(crate) async fn choose_featured_users(
    conn: &mut sqlx::PgConnection,
    topic_id: i32,
) -> Result<(), AppError> {
    // TopicFeaturedUsers.ensure_consistency!(topic_id): two frequent and
    // two recent posters other than the topic's creator and last poster.
    sqlx::query(
        "WITH poster_stats AS ( \
           SELECT t.id, t.user_id AS topic_user_id, t.last_post_user_id, p.user_id, COUNT(*) post_count, \
                  MAX(p.created_at) last_post_date, MAX(p.id) last_post_id, \
                  ROW_NUMBER() OVER (PARTITION BY t.id ORDER BY MAX(p.created_at) DESC, MAX(p.id) DESC) AS recent_rank \
           FROM topics t JOIN posts p ON p.topic_id = t.id \
           WHERE p.deleted_at IS NULL AND NOT p.hidden AND p.post_type IN (1, 2, 3) \
             AND p.user_id <> t.user_id AND p.user_id <> t.last_post_user_id AND t.id = $1 \
           GROUP BY t.id, t.user_id, t.last_post_user_id, p.user_id), \
         selected_recent_posters AS ( \
           SELECT id, user_id, recent_rank + 2 AS rank FROM poster_stats \
           WHERE topic_user_id = last_post_user_id AND recent_rank <= 2 \
           UNION ALL \
           SELECT id, user_id, recent_rank + 2 AS rank FROM poster_stats \
           WHERE topic_user_id <> last_post_user_id AND recent_rank <= 1), \
         selected_frequent_posters AS ( \
           SELECT id, user_id, ROW_NUMBER() OVER (PARTITION BY id ORDER BY post_count DESC, last_post_date DESC, last_post_id DESC) AS rank \
           FROM poster_stats WHERE topic_user_id = last_post_user_id AND recent_rank > 2 \
           UNION ALL \
           SELECT id, user_id, ROW_NUMBER() OVER (PARTITION BY id ORDER BY post_count DESC, last_post_date DESC, last_post_id DESC) AS rank \
           FROM poster_stats WHERE topic_user_id <> last_post_user_id AND recent_rank > 1), \
         selected_topic_posters AS ( \
           SELECT id, user_id, rank FROM selected_frequent_posters WHERE rank <= 2 \
           UNION ALL \
           SELECT id, user_id, rank FROM selected_recent_posters) \
         UPDATE topics tt SET \
           featured_user1_id = x.featured_user1, featured_user2_id = x.featured_user2, \
           featured_user3_id = x.featured_user3, featured_user4_id = x.featured_user4 \
         FROM topics AS tt2 \
         LEFT OUTER JOIN ( \
           SELECT id, \
             MAX(CASE WHEN rank = 1 THEN user_id END) featured_user1, \
             MAX(CASE WHEN rank = 2 THEN user_id END) featured_user2, \
             MAX(CASE WHEN rank = 3 THEN user_id END) featured_user3, \
             MAX(CASE WHEN rank = 4 THEN user_id END) featured_user4 \
           FROM selected_topic_posters GROUP BY id) x ON x.id = tt2.id \
         WHERE tt.id = tt2.id AND ( \
           COALESCE(tt.featured_user1_id, -99) <> COALESCE(x.featured_user1, -99) OR \
           COALESCE(tt.featured_user2_id, -99) <> COALESCE(x.featured_user2, -99) OR \
           COALESCE(tt.featured_user3_id, -99) <> COALESCE(x.featured_user3, -99) OR \
           COALESCE(tt.featured_user4_id, -99) <> COALESCE(x.featured_user4, -99)) \
         AND tt.id = $1",
    )
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    // update_participant_count
    sqlx::query(
        "UPDATE topics SET participant_count = (SELECT COUNT(DISTINCT user_id) FROM posts \
           WHERE topic_id = $1 AND NOT hidden AND post_type IN (1, 2, 3) AND deleted_at IS NULL) \
         WHERE id = $1",
    )
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Badge::FirstEmoji`
const FIRST_EMOJI_BADGE: i32 = 41;

/// `Jobs::ProcessPost`: CookedPostProcessor over the post's cooked HTML,
/// the cooked column updated when it changes, then pull_hotlinked_images.
async fn process_post(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(post_id) = int_arg(args, "post_id")? else {
        return Ok(());
    };
    if args
        .get("cook")
        .is_some_and(|c| !c.is_null() && c != &json!(false))
    {
        return Err(Unsupported("process_post with cook (recooking)").into());
    }
    if args.get("image_sizes").is_some() {
        return Err(Unsupported("process_post with image sizes").into());
    }
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    #[derive(sqlx::FromRow)]
    struct Row {
        raw: String,
        cooked: String,
        topic_id: i32,
        post_number: i32,
        user_id: Option<i32>,
        image_upload_id: Option<i64>,
        topic_image_upload_id: Option<i64>,
        omit_nofollow: bool,
        author_staff_or_staged: bool,
        has_first_emoji: bool,
        first_emoji_enabled: bool,
    }
    let tl3_no_follow = s.get("tl3_links_no_follow")?.truthy();
    let post: Option<Row> = sqlx::query_as(
        "SELECT p.raw, p.cooked, p.topic_id, p.post_number, p.user_id, p.image_upload_id, \
                t.image_upload_id AS topic_image_upload_id, \
                COALESCE(u.admin OR u.moderator OR (NOT $2 AND (u.staged OR u.trust_level >= 3)), FALSE) AS omit_nofollow, \
                COALESCE(u.admin OR u.moderator OR u.staged, FALSE) AS author_staff_or_staged, \
                EXISTS (SELECT 1 FROM user_badges WHERE user_id = p.user_id AND badge_id = $3) AS has_first_emoji, \
                EXISTS (SELECT 1 FROM badges WHERE id = $3 AND enabled) AS first_emoji_enabled \
         FROM posts p JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
         LEFT JOIN users u ON u.id = p.user_id \
         WHERE p.id = $1",
    )
    .bind(post_id)
    .bind(tl3_no_follow)
    .bind(FIRST_EMOJI_BADGE)
    .fetch_optional(&mut *conn)
    .await?;
    // Nothing to do for a missing post or one whose topic is gone.
    let Some(post) = post else {
        return Ok(());
    };
    if bool_arg(args, "new_post")
        && s.get("remove_full_quote")?.truthy()
        && post.post_number > 1
        && post.raw.contains("[quote")
    {
        return Err(Unsupported("remove_full_quote_on_direct_reply").into());
    }
    let html = cooked_post_processor::post_process(
        &mut conn,
        &s,
        &state.config,
        &state.i18n,
        &post.cooked,
        post.omit_nofollow,
        Some(post_id),
    )
    .await?;
    // update_post_image: the post's (and a first post's topic's) image.
    let hostname = Urls {
        config: &state.config,
        settings: &s,
    }
    .current_hostname()?;
    let store = crate::file_store::FileStore::for_site(&state.config, &s)?;
    let image =
        crate::upload_references::post_image_upload(&mut conn, &hostname, &store, &html).await?;
    match image {
        Some(upload_id) => {
            sqlx::query("UPDATE posts SET image_upload_id = $2 WHERE id = $1")
                .bind(post_id)
                .bind(i64::from(upload_id))
                .execute(&mut *conn)
                .await?;
            if post.post_number == 1 {
                sqlx::query("UPDATE topics SET image_upload_id = $2 WHERE id = $1")
                    .bind(post.topic_id)
                    .bind(i64::from(upload_id))
                    .execute(&mut *conn)
                    .await?;
                // clear_generated_og_image!
                let og: Option<i64> =
                    sqlx::query_scalar("SELECT og_image_upload_id FROM topics WHERE id = $1")
                        .bind(post.topic_id)
                        .fetch_one(&mut *conn)
                        .await?;
                if let Some(og) = og {
                    sqlx::query("UPDATE topics SET og_image_upload_id = NULL WHERE id = $1")
                        .bind(post.topic_id)
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query(
                        "DELETE FROM upload_references WHERE target_type = 'Topic' AND target_id = $1 \
                         AND upload_id = $2",
                    )
                    .bind(post.topic_id)
                    .bind(og)
                    .execute(&mut *conn)
                    .await?;
                }
                if let Some(upload) =
                    crate::optimized_images::Upload::find(&mut conn, upload_id).await?
                {
                    crate::optimized_images::generate_topic_thumbnails(
                        &mut conn, &s, &store, &upload,
                    )
                    .await?;
                }
            }
        }
        None => {
            if post.image_upload_id.is_some() {
                sqlx::query("UPDATE posts SET image_upload_id = NULL WHERE id = $1")
                    .bind(post_id)
                    .execute(&mut *conn)
                    .await?;
            }
            if post.post_number == 1 {
                if post.topic_image_upload_id.is_some() {
                    sqlx::query("UPDATE topics SET image_upload_id = NULL WHERE id = $1")
                        .bind(post.topic_id)
                        .execute(&mut *conn)
                        .await?;
                }
                if s.get("generate_topic_og_image")?.truthy() {
                    return Err(Unsupported("generate_topic_og_image").into());
                }
            }
        }
    }
    // grant_badges: the first emoji badge is a write.
    let has_emoji = html.contains("class=\"emoji");
    if has_emoji && post.first_emoji_enabled && !post.has_first_emoji && post.user_id.is_some() {
        return Err(Unsupported("granting the first emoji badge").into());
    }
    // @post.link_post_uploads(fragments: @doc)
    crate::upload_references::link_post_uploads(&mut conn, &s, &hostname, &store, post_id, &html)
        .await?;
    if html != post.cooked {
        sqlx::query("UPDATE posts SET cooked = $2 WHERE id = $1")
            .bind(post_id)
            .bind(&html)
            .execute(&mut *conn)
            .await?;
        // sync_first_post_caches: the topic's excerpt from the new html; a
        // category's description is not synced from its definition here.
        if post.post_number == 1 {
            let definition: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE topic_id = $1)")
                    .bind(post.topic_id)
                    .fetch_one(&mut *conn)
                    .await?;
            if definition {
                return Err(
                    Unsupported("syncing a category description after post processing").into(),
                );
            }
            let excerpt = crate::excerpt::excerpt(
                &html,
                s.get("topic_excerpt_maxlength")?.to_i().max(0) as usize,
                &crate::excerpt::Options {
                    strip_links: true,
                    image_mode: crate::excerpt::ImageMode::Strip,
                    ..Default::default()
                },
            );
            sqlx::query("UPDATE topics SET excerpt = $2 WHERE id = $1")
                .bind(post.topic_id)
                .bind(excerpt)
                .execute(&mut *conn)
                .await?;
        }
        if let Some(user_id) = post.user_id {
            let urls = Urls {
                config: &state.config,
                settings: &s,
            };
            let hostname = urls.current_hostname()?;
            links::extract_from(
                &mut conn,
                &links::Site {
                    hostname: &hostname,
                    base_path: state.config.globals.relative_url_root(),
                    base_url_no_prefix: &urls.base_url_no_prefix()?,
                    settings: &s,
                },
                &LinkPost {
                    id: post_id,
                    user_id,
                    topic_id: post.topic_id,
                    cooked: &html,
                },
            )
            .await?;
        }
        let host = crate::pretty_text::Host::from_state(state);
        let ctx = crate::posting::Ctx {
            host: &host,
            settings: &s,
            config: &state.config,
            i18n: &state.i18n,
            bus: &state.bus,
        };
        crate::bus::publish_post_change(
            &ctx,
            &mut conn,
            post_id,
            "revised",
            Default::default(),
            false,
        )
        .await?;
    }
    if !bool_arg(args, "skip_pull_hotlinked_images") {
        // Jobs.cancel_scheduled_job, then enqueue.
        sqlx::query(
            "DELETE FROM discourse_rs.jobs WHERE name = 'pull_hotlinked_images' AND failed_at IS NULL \
             AND locked_until IS NULL AND args = $1",
        )
        .bind(json!({"post_id": post_id}))
        .execute(&mut *conn)
        .await?;
        super::enqueue(
            &mut conn,
            "pull_hotlinked_images",
            json!({"post_id": post_id}),
        )
        .await?;
    }
    // WordWatcher flags (watched words) for non-staff authors.
    if !post.author_staff_or_staged && !bool_arg(args, "bypass_bump") {
        let watched: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM watched_words)")
            .fetch_one(&mut *conn)
            .await?;
        if watched {
            return Err(Unsupported("watched words in process_post").into());
        }
    }
    Ok(())
}

/// `Jobs::PostAlert`, in one transaction so a refused case writes nothing.
async fn post_alert(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    drop(conn);
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let mut tx = state.pool.begin().await?;
    super::post_alert::run(&ctx, &mut tx, args).await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::UserEmail`, in one transaction: a refused case writes no
/// unsubscribe key or log, and nothing is delivered before it commits.
async fn user_email(state: &AppState, args: &Value, critical: bool) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    drop(conn);
    let host = crate::pretty_text::Host::from_state(state);
    let posting = crate::posting::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let ctx = crate::email::sender::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        mailer: &state.mailer,
    };
    let mut tx = state.pool.begin().await?;
    super::user_email::run(&mut tx, &ctx, &posting, args, critical).await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::SendEmailLoginCode`: EmailLoginCodeMailer's code email, sent
/// with no user (`Email::Sender.new(message, :email_login_code)`).
async fn send_email_login_code(state: &AppState, args: &Value) -> Result<(), AppError> {
    let (Some(to), Some(code)) = (
        args.get("to_address").and_then(Value::as_str),
        args.get("code").and_then(Value::as_str),
    ) else {
        return Err(Unsupported(
            "send_email_login_code without an address or code (InvalidParameters)",
        )
        .into());
    };
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    if !s.get("enable_local_logins_via_code")?.truthy() {
        return Ok(());
    }
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::email::sender::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        mailer: &state.mailer,
    };
    let template = if bool_arg(args, "password_reset") {
        "password_reset_code_mailer"
    } else {
        "email_login_code_mailer"
    };
    let mut tx = state.pool.begin().await?;
    let built = crate::email::account::build_template(
        &mut tx,
        &ctx,
        template,
        to,
        &[("code", code), ("minutes", "10")],
        None,
    )
    .await?;
    crate::email::sender::send(&mut tx, &ctx, built, "email_login_code", None).await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::ProcessEmail`: Email::Processor.process!(mail, source:).
async fn process_email(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mail = args.get("mail").and_then(Value::as_str).unwrap_or("");
    let source = args.get("source").and_then(Value::as_str);
    crate::email::receiver::process(state, mail, source).await
}

/// `Jobs::SendSystemMessage`: SystemMessage.create for the user, the
/// message options interpolated into the templates.
async fn send_system_message(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(user_id) = int_arg(args, "user_id")? else {
        return Err(
            Unsupported("send_system_message without a user_id (InvalidParameters)").into(),
        );
    };
    let Some(message_type) = args
        .get("message_type")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    else {
        return Err(
            Unsupported("send_system_message without a message_type (InvalidParameters)").into(),
        );
    };
    let mut conn = state.pool.acquire().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Ok(());
    }
    let s = settings(state, &mut conn).await?;
    drop(conn);
    let mut params = Vec::new();
    let mut post_alert_options = None;
    if let Some(options) = args.get("message_options").and_then(Value::as_object) {
        for (key, value) in options {
            if key == "post_alert_options" {
                post_alert_options = Some(value.clone());
                continue;
            }
            // Ruby's string interpolation of the value.
            let text = match value {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => {
                    return Err(
                        Unsupported("system message options that are lists or hashes").into(),
                    );
                }
            };
            params.push((key.clone(), text));
        }
    }
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    crate::system_message::create(
        &state.pool,
        &ctx,
        user_id,
        message_type,
        &params,
        post_alert_options,
    )
    .await?;
    Ok(())
}
