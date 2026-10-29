//! discourse-narrative-bot, ported from the plugin's specs: the welcome
//! post on signup (user_spec, discobot_welcome_post_spec, the
//! send_default_welcome_message job spec), the new user tutorial
//! (new_user_narrative_spec), the advanced one
//! (advanced_user_narrative_spec), the track selector's commands, resets
//! and generic replies (track_selector_spec), the bot_input job
//! (bot_input_spec), the certificate (discobot_certificate_spec,
//! certificate_generator_spec), and the events core hands the plugin.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use discourse_rs::guardian::Guardian;
use discourse_rs::posting::Ctx;
use discourse_rs::posting::create::{self, NewPost, Outcome};
use discourse_rs::pretty_text::Host;
use discourse_rs::session::current::SessionUser;
use discourse_rs::site_settings::SiteSettings;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const DISCOBOT: i32 = -2;
const NEW_USER: &str = "DiscourseNarrativeBot::NewUserNarrative";
const ADVANCED: &str = "DiscourseNarrativeBot::AdvancedUserNarrative";
/// The plugin's jobs that run as the tests go (the timeout reminder
/// only when a test asks for it).
const BOT_JOBS: [&str; 3] = [
    "bot_input",
    "narrative_init",
    "send_default_welcome_message",
];

async fn setup() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    common::set_setting(&db.pool, "discourse_narrative_bot_enabled", 5, "t").await;
    // The specs' short replies, some of them alike.
    for name in [
        "min_post_length",
        "min_first_post_length",
        "min_personal_message_post_length",
        "unique_posts_mins",
    ] {
        common::set_setting(
            &db.pool,
            name,
            3,
            if name == "unique_posts_mins" {
                "0"
            } else {
                "1"
            },
        )
        .await;
    }
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, st)
}

async fn settings(st: &AppState) -> SiteSettings {
    let mut conn = st.pool.acquire().await.unwrap();
    SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap()
}

async fn guardian(st: &AppState, user_id: i32) -> Guardian {
    let mut conn = st.pool.acquire().await.unwrap();
    let user = SessionUser::load(&mut conn, user_id)
        .await
        .unwrap()
        .unwrap();
    Guardian::for_user(&mut conn, &user).await.unwrap()
}

/// `I18n.t("discourse_narrative_bot.<key>", args)` with base_uri.
fn t(st: &AppState, key: &str, extra: &[(&str, &str)]) -> String {
    let mut args = vec![("base_uri", "")];
    args.extend_from_slice(extra);
    st.i18n
        .t_with(&format!("discourse_narrative_bot.{key}"), &args)
        .unwrap_or_else(|| panic!("no translation for {key}"))
}

/// A message without arguments, its placeholders as they are.
fn t0(st: &AppState, key: &str) -> String {
    st.i18n
        .t(&format!("discourse_narrative_bot.{key}"))
        .unwrap()
        .to_string()
}

/// "#{reply}\n\n#{instructions}", as PostCreator saves the heredoc.
fn two(a: &str, b: &str) -> String {
    format!("{a}\n\n{b}")
}

/// A new user (`Fabricate(:user)` through `user.save!`), its id.
async fn new_user(st: &AppState, username: &str, staged: bool) -> i32 {
    let settings = settings(st).await;
    let mut conn = st.pool.acquire().await.unwrap();
    let payloads = discourse_rs::web_hooks::Payloads {
        settings: &settings,
        i18n: &st.i18n,
        config: &st.config,
    };
    let email = format!("{username}@example.com");
    discourse_rs::signup::insert_user(
        &mut conn,
        &settings,
        &payloads,
        &discourse_rs::signup::NewUser {
            username,
            name: None,
            email: &email,
            password: Some("supersecretpassword"),
            trust_level: 1,
            active: true,
            approved: true,
            moderator: false,
            staged,
            locale: None,
            ip: None,
            custom_fields: &[],
        },
    )
    .await
    .unwrap()
}

/// PostCreator.create(user, opts): the new post's id.
async fn create_post(st: &AppState, user_id: i32, args: NewPost) -> i32 {
    let settings = settings(st).await;
    let host = Host::from_state(st);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &st.config,
        i18n: &st.i18n,
        bus: &st.bus,
    };
    let guardian = guardian(st, user_id).await;
    match Box::pin(create::create(&st.pool, &ctx, &guardian, args))
        .await
        .unwrap()
    {
        Outcome::Created { post_id } => post_id,
        other => panic!("not created: {other:?}"),
    }
}

/// A reply by the user in the topic.
async fn reply(st: &AppState, user_id: i32, topic_id: i32, raw: &str) -> i32 {
    create_post(
        st,
        user_id,
        NewPost {
            raw: raw.to_string(),
            topic_id: Some(topic_id),
            ..Default::default()
        },
    )
    .await
}

static TITLES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

/// A public topic by the user: its first post's id.
async fn public_topic(st: &AppState, user_id: i32, raw: &str) -> i32 {
    create_post(
        st,
        user_id,
        NewPost {
            raw: raw.to_string(),
            title: Some(format!(
                "A topic about capybaras and their friends number {}",
                TITLES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )),
            category: Some("4".to_string()),
            ..Default::default()
        },
    )
    .await
}

async fn like(st: &AppState, user_id: i32, post_id: i32) {
    let settings = settings(st).await;
    let host = Host::from_state(st);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &st.config,
        i18n: &st.i18n,
        bus: &st.bus,
    };
    let guardian = guardian(st, user_id).await;
    let outcome = discourse_rs::likes::like(&st.pool, &ctx, &guardian, post_id)
        .await
        .unwrap();
    assert!(matches!(outcome, discourse_rs::likes::Outcome::Done));
}

async fn bookmark(st: &AppState, user_id: i32, id: i32, kind: &str) {
    let settings = settings(st).await;
    let host = Host::from_state(st);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &st.config,
        i18n: &st.i18n,
        bus: &st.bus,
    };
    let guardian = guardian(st, user_id).await;
    let mut conn = st.pool.acquire().await.unwrap();
    let outcome = discourse_rs::bookmark_manager::create(
        &mut conn,
        &ctx,
        &guardian,
        &id.to_string(),
        kind,
        &discourse_rs::bookmark_manager::Fields {
            name: None,
            reminder_at: None,
            auto_delete_preference: None,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        discourse_rs::bookmark_manager::Outcome::Created(_)
    ));
}

async fn revise(st: &AppState, user_id: i32, post_id: i32, raw: &str) {
    let settings = settings(st).await;
    let host = Host::from_state(st);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &st.config,
        i18n: &st.i18n,
        bus: &st.bus,
    };
    let guardian = guardian(st, user_id).await;
    discourse_rs::posting::revise::revise(
        &st.pool,
        &ctx,
        &guardian,
        post_id,
        discourse_rs::posting::revise::Changes {
            raw: Some(raw.to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

/// The plugin's queued jobs, run until none are left; a failure fails
/// the test.
async fn run_bot_jobs(st: &AppState) {
    loop {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM discourse_rs.jobs WHERE name = ANY($1) AND failed_at IS NULL \
               AND last_error IS NULL ORDER BY id",
        )
        .bind(BOT_JOBS.to_vec())
        .fetch_all(&st.pool)
        .await
        .unwrap();
        if ids.is_empty() {
            return;
        }
        for id in ids {
            Box::pin(discourse_rs::jobs::perform_now(st, id))
                .await
                .unwrap();
            let error: Option<Option<String>> =
                sqlx::query_scalar("SELECT last_error FROM discourse_rs.jobs WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&st.pool)
                    .await
                    .unwrap();
            if let Some(Some(error)) = error {
                panic!("job {id} failed: {error}");
            }
        }
    }
}

/// Runs `bot_input` directly, as `Jobs::BotInput.new.execute(args)`.
async fn bot_input(st: &AppState, args: Value) {
    Box::pin(discourse_rs::plugins::narrative_bot::jobs::run(
        st,
        "bot_input",
        &args,
    ))
    .await
    .unwrap();
}

async fn queued(st: &AppState, name: &str) -> Vec<Value> {
    sqlx::query_scalar("SELECT args FROM discourse_rs.jobs WHERE name = $1 ORDER BY id")
        .bind(name)
        .fetch_all(&st.pool)
        .await
        .unwrap()
}

async fn clear_jobs(st: &AppState) {
    sqlx::query("DELETE FROM discourse_rs.jobs")
        .execute(&st.pool)
        .await
        .unwrap();
}

/// `Store.get(user_id)`
async fn data(st: &AppState, user_id: i32) -> Value {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT value FROM plugin_store_rows WHERE plugin_name = 'discourse-narrative-bot' AND key = $1",
    )
    .bind(user_id.to_string())
    .fetch_optional(&st.pool)
    .await
    .unwrap();
    value.map_or(Value::Null, |v| serde_json::from_str(&v).unwrap())
}

/// `narrative.set_data(user, hash)`
async fn set_data(st: &AppState, user_id: i32, value: Value) {
    sqlx::query(
        "DELETE FROM plugin_store_rows WHERE plugin_name = 'discourse-narrative-bot' AND key = $1",
    )
    .bind(user_id.to_string())
    .execute(&st.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO plugin_store_rows (plugin_name, key, type_name, value) \
         VALUES ('discourse-narrative-bot', $1, 'JSON', $2)",
    )
    .bind(user_id.to_string())
    .bind(value.to_string())
    .execute(&st.pool)
    .await
    .unwrap();
}

/// The newest post: (id, user_id, topic_id, raw).
async fn last_post(st: &AppState) -> (i32, i32, i32, String) {
    sqlx::query_as("SELECT id, user_id, topic_id, raw FROM posts ORDER BY id DESC LIMIT 1")
        .fetch_one(&st.pool)
        .await
        .unwrap()
}

async fn post_count(st: &AppState) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM posts")
        .fetch_one(&st.pool)
        .await
        .unwrap()
}

/// Starts the new user tutorial for the user (`narrative_init`): the
/// message's topic id and discobot's first post's id.
async fn start_new_user_track(st: &AppState, user_id: i32) -> (i32, i32) {
    discourse_rs::plugins::narrative_bot::jobs::run(
        st,
        "narrative_init",
        &json!({ "user_id": user_id, "klass": NEW_USER }),
    )
    .await
    .unwrap();
    let (post_id, author, topic_id, _) = last_post(st).await;
    assert_eq!(author, DISCOBOT);
    (topic_id, post_id)
}

async fn state_of(st: &AppState, user_id: i32) -> String {
    data(st, user_id).await["state"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn signing_up_starts_the_new_user_tutorial() {
    let (_db, st) = setup().await;
    common::set_setting(
        &st.pool,
        "disable_discourse_narrative_bot_welcome_post",
        5,
        "f",
    )
    .await;
    let user_id = new_user(&st, "newbie", false).await;
    assert_eq!(
        queued(&st, "narrative_init").await,
        vec![json!({ "user_id": user_id, "klass": NEW_USER })]
    );
    run_bot_jobs(&st).await;

    let (post_id, author, topic_id, raw) = last_post(&st).await;
    assert_eq!(author, DISCOBOT);
    let title = st
        .i18n
        .t_with(
            "discourse_narrative_bot.new_user_narrative.hello.title",
            &[("title", "Discourse")],
        )
        .unwrap();
    let (topic_title, archetype, subtype): (String, String, Option<String>) =
        sqlx::query_as("SELECT title, archetype, subtype FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&st.pool)
            .await
            .unwrap();
    assert_eq!(topic_title, title);
    assert_eq!(archetype, "private_message");
    assert_eq!(subtype.as_deref(), Some("system_message"));
    let site_title = settings(&st).await.get("title").unwrap().to_s();
    let hello = t(
        &st,
        "new_user_narrative.hello.message",
        &[("username", "newbie"), ("title", &site_title)],
    );
    let instructions = t(&st, "new_user_narrative.bookmark.instructions", &[]);
    assert_eq!(raw, two(&hello, &instructions));
    let allowed: Vec<i32> = sqlx::query_scalar(
        "SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 ORDER BY user_id",
    )
    .bind(topic_id)
    .fetch_all(&st.pool)
    .await
    .unwrap();
    assert_eq!(allowed, vec![DISCOBOT, user_id]);
    assert_eq!(
        data(&st, user_id).await,
        json!({ "topic_id": topic_id, "track": NEW_USER, "state": "tutorial_bookmark", "last_post_id": post_id })
    );
}

#[tokio::test]
async fn the_welcome_post_follows_its_settings() {
    let (_db, st) = setup().await;
    // Off by default.
    new_user(&st, "quiet", false).await;
    assert!(queued(&st, "narrative_init").await.is_empty());

    common::set_setting(
        &st.pool,
        "disable_discourse_narrative_bot_welcome_post",
        5,
        "f",
    )
    .await;
    // Staged users, ignored usernames and users who skip the tips get none.
    new_user(&st, "staged_one", true).await;
    common::set_setting(
        &st.pool,
        "discourse_narrative_bot_ignored_usernames",
        8,
        "discourse|test",
    )
    .await;
    new_user(&st, "test", false).await;
    common::set_setting(&st.pool, "default_other_skip_new_user_tips", 5, "t").await;
    new_user(&st, "skipper", false).await;
    assert!(queued(&st, "narrative_init").await.is_empty());
    common::set_setting(&st.pool, "default_other_skip_new_user_tips", 5, "f").await;

    // The plain welcome message, closed.
    common::set_setting(
        &st.pool,
        "discourse_narrative_bot_welcome_post_type",
        7,
        "welcome_message",
    )
    .await;
    let user_id = new_user(&st, "welcomed", false).await;
    assert_eq!(
        queued(&st, "send_default_welcome_message").await,
        vec![json!({ "user_id": user_id })]
    );
    run_bot_jobs(&st).await;
    let (_, _, topic_id, _) = last_post(&st).await;
    let (author, raw): (i32, String) =
        sqlx::query_as("SELECT user_id, raw FROM posts WHERE topic_id = $1 AND post_number = 1")
            .bind(topic_id)
            .fetch_one(&st.pool)
            .await
            .unwrap();
    assert_eq!(author, DISCOBOT);
    let (title, closed): (String, bool) =
        sqlx::query_as("SELECT title, closed FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&st.pool)
            .await
            .unwrap();
    let site_title = settings(&st).await.get("title").unwrap().to_s();
    assert_eq!(
        title,
        st.i18n
            .t_with(
                "system_messages.welcome_user.subject_template",
                &[("site_name", &site_title)]
            )
            .unwrap()
    );
    assert!(closed);
    assert!(
        raw.starts_with(
            st.i18n
                .t("system_messages.welcome_user.text_body_template")
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .split("%{")
                .next()
                .unwrap()
        ),
        "{raw}"
    );
}

#[tokio::test]
async fn a_delayed_welcome_post_waits_for_the_first_login() {
    let (_db, st) = setup().await;
    common::set_setting(
        &st.pool,
        "disable_discourse_narrative_bot_welcome_post",
        5,
        "f",
    )
    .await;
    common::set_setting(
        &st.pool,
        "discourse_narrative_bot_welcome_post_delay",
        3,
        "5",
    )
    .await;
    let user_id = new_user(&st, "latecomer", false).await;
    assert!(queued(&st, "narrative_init").await.is_empty());
    let settings = settings(&st).await;
    let mut conn = st.pool.acquire().await.unwrap();
    discourse_rs::plugins::narrative_bot::user_logged_in(&mut conn, &settings, user_id)
        .await
        .unwrap();
    assert_eq!(
        queued(&st, "narrative_init").await,
        vec![json!({ "user_id": user_id, "klass": NEW_USER })]
    );
    let delay: f64 = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM run_at - created_at)::float8 FROM discourse_rs.jobs \
         WHERE name = 'narrative_init'",
    )
    .fetch_one(&st.pool)
    .await
    .unwrap();
    assert_eq!(delay.round(), 5.0);
    // A user seen before is not new to it.
    sqlx::query("UPDATE users SET last_seen_at = now() WHERE id = $1")
        .bind(user_id)
        .execute(&st.pool)
        .await
        .unwrap();
    discourse_rs::plugins::narrative_bot::user_logged_in(&mut conn, &settings, user_id)
        .await
        .unwrap();
    assert_eq!(queued(&st, "narrative_init").await.len(), 1);
}

#[tokio::test]
async fn the_bookmark_tutorial_waits_for_a_bookmark_on_discobots_post() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "booker", false).await;
    let (topic_id, hello_id) = start_new_user_track(&st, user_id).await;

    // A reply instead: the reminder, and the timeout.
    reply(&st, user_id, topic_id, "Where is the bookmark?").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        t(&st, "new_user_narrative.bookmark.not_found", &[])
    );
    assert_eq!(state_of(&st, user_id).await, "tutorial_bookmark");
    assert_eq!(
        queued(&st, "narrative_timeout").await,
        vec![json!({ "user_id": user_id, "klass": NEW_USER })]
    );

    // Bookmarking someone else's post does nothing.
    let other = public_topic(&st, 1, "A post to bookmark somewhere else entirely.").await;
    let before = post_count(&st).await;
    bookmark(&st, user_id, other, "Post").await;
    run_bot_jobs(&st).await;
    assert_eq!(post_count(&st).await, before);

    bookmark(&st, user_id, hello_id, "Post").await;
    run_bot_jobs(&st).await;
    let bookmark_url = "http://test.localhost/u/booker/activity/bookmarks";
    assert_eq!(
        last_post(&st).await.3,
        two(
            &t(
                &st,
                "new_user_narrative.bookmark.reply",
                &[("bookmark_url", bookmark_url)]
            ),
            &t(&st, "new_user_narrative.onebox.instructions", &[])
        )
    );
    assert_eq!(state_of(&st, user_id).await, "tutorial_onebox");
    // One timeout pending, the latest.
    assert_eq!(queued(&st, "narrative_timeout").await.len(), 1);
}

#[tokio::test]
async fn the_new_user_tutorial_runs_to_its_certificate() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "learner", false).await;
    let (topic_id, hello_id) = start_new_user_track(&st, user_id).await;
    bookmark(&st, user_id, hello_id, "Post").await;
    run_bot_jobs(&st).await;

    // onebox
    reply(&st, user_id, topic_id, "Nothing to see").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        t(&st, "new_user_narrative.onebox.not_found", &[])
    );
    reply(
        &st,
        user_id,
        topic_id,
        "https://en.wikipedia.org/wiki/Death_by_coconut",
    )
    .await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        two(
            &t(&st, "new_user_narrative.onebox.reply", &[]),
            &t(&st, "new_user_narrative.emoji.instructions", &[])
        )
    );

    // emoji
    reply(&st, user_id, topic_id, "Happy :smile: here").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        two(
            &t(&st, "new_user_narrative.emoji.reply", &[]),
            &t(
                &st,
                "new_user_narrative.mention.instructions",
                &[("discobot_username", "discobot")]
            )
        )
    );

    // mention, once missed
    reply(&st, user_id, topic_id, "Hello there, bot").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        t(
            &st,
            "new_user_narrative.mention.not_found",
            &[("username", "learner"), ("discobot_username", "discobot")]
        )
    );
    reply(&st, user_id, topic_id, "Hello there, @discobot").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        two(
            &t(&st, "new_user_narrative.mention.reply", &[]),
            &t(&st, "new_user_narrative.formatting.instructions", &[])
        )
    );
    assert_eq!(state_of(&st, user_id).await, "tutorial_formatting");

    // formatting
    reply(&st, user_id, topic_id, "This is **bold** text").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        two(
            &t(&st, "new_user_narrative.formatting.reply", &[]),
            &t(&st, "new_user_narrative.quoting.instructions", &[])
        )
    );

    // quoting
    reply(
        &st,
        user_id,
        topic_id,
        &format!("[quote=\"discobot, post:1, topic:{topic_id}\"]\nHello\n[/quote]\nQuoted!"),
    )
    .await;
    run_bot_jobs(&st).await;
    let (images_instructions_id, _, _, raw) = last_post(&st).await;
    assert_eq!(
        raw,
        two(
            &t(&st, "new_user_narrative.quoting.reply", &[]),
            &t(&st, "new_user_narrative.images.instructions", &[])
        )
    );
    assert_eq!(state_of(&st, user_id).await, "tutorial_images");

    // images: an image without the like, then the like.
    reply(&st, user_id, topic_id, "No picture yet").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        t(
            &st,
            "new_user_narrative.images.not_found",
            &[(
                "image_url",
                "http://test.localhost/plugins/discourse-narrative-bot/images/dog-walk.gif"
            )]
        )
    );
    let image_post = reply(
        &st,
        user_id,
        topic_id,
        "<img src='https://example.com/capybara.jpg'>",
    )
    .await;
    run_bot_jobs(&st).await;
    let slug: String = sqlx::query_scalar("SELECT slug FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&st.pool)
        .await
        .unwrap();
    let number: i32 = sqlx::query_scalar("SELECT post_number FROM posts WHERE id = $1")
        .bind(images_instructions_id)
        .fetch_one(&st.pool)
        .await
        .unwrap();
    assert_eq!(
        last_post(&st).await.3,
        t(
            &st,
            "new_user_narrative.images.like_not_found",
            &[("url", &format!("/t/{slug}/{topic_id}/{number}"))]
        )
    );
    assert_eq!(
        data(&st, user_id).await["tutorial_images"]["post_id"],
        json!(image_post)
    );
    like(&st, user_id, images_instructions_id).await;
    run_bot_jobs(&st).await;
    let flag_instructions = t(
        &st,
        "new_user_narrative.flag.instructions",
        &[
            ("guidelines_url", "http://test.localhost/guidelines"),
            ("about_url", "http://test.localhost/about"),
        ],
    );
    assert_eq!(
        last_post(&st).await.3,
        two(
            &t(&st, "new_user_narrative.images.reply", &[]),
            &flag_instructions
        )
    );
    let liked_by_discobot: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM post_actions WHERE post_id = $1 AND user_id = -2 \
           AND post_action_type_id = 2)",
    )
    .bind(image_post)
    .fetch_one(&st.pool)
    .await
    .unwrap();
    assert!(liked_by_discobot);
    assert_eq!(state_of(&st, user_id).await, "tutorial_flag");

    // flag: the images step's last miss left `attempted` set (the like
    // that moved the user on is no reply), so a first reply gets no
    // reminder, as in Rails; skipping moves on to the search.
    let quiet = reply(&st, user_id, topic_id, "I would rather not").await;
    run_bot_jobs(&st).await;
    assert_eq!(last_post(&st).await.0, quiet);
    reply(&st, user_id, topic_id, "skip").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        t(&st, "new_user_narrative.search.instructions", &[])
    );
    assert_eq!(state_of(&st, user_id).await, "tutorial_search");
    // The answer hidden in the first post.
    let (first_raw, version): (String, i32) =
        sqlx::query_as("SELECT raw, version FROM posts WHERE topic_id = $1 AND post_number = 1")
            .bind(topic_id)
            .fetch_one(&st.pool)
            .await
            .unwrap();
    let hidden = t(
        &st,
        "new_user_narrative.search.hidden_message",
        &[("search_answer", ":herb:")],
    );
    assert!(first_raw.ends_with(&hidden), "{first_raw}");
    assert_eq!(
        data(&st, user_id).await["tutorial_search"]["post_version"],
        json!(version)
    );

    // search
    reply(&st, user_id, topic_id, "Is it a capybara?").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        t(&st, "new_user_narrative.search.not_found", &[])
    );
    reply(&st, user_id, topic_id, ":herb: this is a capybara").await;
    run_bot_jobs(&st).await;

    let posts: Vec<(i32, String)> = sqlx::query_as(
        "SELECT user_id, raw FROM posts WHERE topic_id = $1 ORDER BY post_number DESC LIMIT 2",
    )
    .bind(topic_id)
    .fetch_all(&st.pool)
    .await
    .unwrap();
    let date = chrono::Utc::now()
        .format("%b %d %Y")
        .to_string()
        .replace(' ', "+");
    let certificate = format!(
        "<iframe class='discobot-certificate' src='http://test.localhost/discobot/certificate.svg?date={date}&user_id={user_id}' width='650' height='464' alt='{}'></iframe>",
        t0(&st, "new_user_narrative.certificate.alt")
    );
    assert_eq!(
        posts[0].1,
        t(
            &st,
            "new_user_narrative.end.message",
            &[
                ("username", "learner"),
                ("base_url", "http://test.localhost"),
                ("certificate", &certificate),
                ("discobot_username", "discobot"),
                ("advanced_trigger", "advanced tutorial"),
            ]
        )
    );
    assert_eq!(
        posts[1].1,
        t(
            &st,
            "new_user_narrative.search.reply",
            &[("search_url", "http://test.localhost/search")]
        )
    );
    let end_post: i32 = sqlx::query_scalar(
        "SELECT id FROM posts WHERE topic_id = $1 ORDER BY post_number DESC LIMIT 1 OFFSET 1",
    )
    .bind(topic_id)
    .fetch_one(&st.pool)
    .await
    .unwrap();
    assert_eq!(
        data(&st, user_id).await,
        json!({ "topic_id": topic_id, "state": "end", "track": NEW_USER, "attempted": false })
    );
    let _ = end_post;
    // The certificate is cooked as an iframe.
    let cooked: String = sqlx::query_scalar(
        "SELECT cooked FROM posts WHERE topic_id = $1 ORDER BY post_number DESC LIMIT 1",
    )
    .bind(topic_id)
    .fetch_one(&st.pool)
    .await
    .unwrap();
    assert!(
        cooked.contains("<iframe src=\"http://test.localhost/discobot/certificate.svg?date="),
        "{cooked}"
    );
    // The badge, and the first post as it was.
    let badged: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_badges ub JOIN badges b ON b.id = ub.badge_id \
         WHERE ub.user_id = $1 AND b.name = 'Certified')",
    )
    .bind(user_id)
    .fetch_one(&st.pool)
    .await
    .unwrap();
    assert!(badged);
    let first_raw: String =
        sqlx::query_scalar("SELECT raw FROM posts WHERE topic_id = $1 AND post_number = 1")
            .bind(topic_id)
            .fetch_one(&st.pool)
            .await
            .unwrap();
    assert!(!first_raw.contains(&hidden));
    assert!(queued(&st, "narrative_timeout").await.is_empty());
}

#[tokio::test]
async fn skipped_tutorials_and_prerequisites_are_stepped_over() {
    let (_db, st) = setup().await;
    common::set_setting(
        &st.pool,
        "discourse_narrative_bot_skip_tutorials",
        8,
        "tutorial_onebox",
    )
    .await;
    common::set_setting(&st.pool, "enable_emoji", 5, "f").await;
    let user_id = new_user(&st, "hopper", false).await;
    let (_, hello_id) = start_new_user_track(&st, user_id).await;
    bookmark(&st, user_id, hello_id, "Post").await;
    run_bot_jobs(&st).await;
    // Onebox skipped by the setting, emoji by its prerequisite.
    assert_eq!(state_of(&st, user_id).await, "tutorial_mention");
    let bookmark_url = "http://test.localhost/u/hopper/activity/bookmarks";
    assert_eq!(
        last_post(&st).await.3,
        two(
            &t(
                &st,
                "new_user_narrative.bookmark.reply",
                &[("bookmark_url", bookmark_url)]
            ),
            &t(
                &st,
                "new_user_narrative.mention.instructions",
                &[("discobot_username", "discobot")]
            )
        )
    );
}

#[tokio::test]
async fn the_images_tutorial_gives_way_to_likes_without_embedded_media() {
    let (_db, st) = setup().await;
    common::set_setting(&st.pool, "embedded_media_post_allowed_groups", 20, "1").await;
    let user_id = new_user(&st, "liker", false).await;
    let (topic_id, _) = start_new_user_track(&st, user_id).await;
    set_data(
        &st,
        user_id,
        json!({ "state": "tutorial_quote", "topic_id": topic_id, "track": NEW_USER }),
    )
    .await;
    reply(&st, user_id, topic_id, "skip").await;
    run_bot_jobs(&st).await;
    assert_eq!(state_of(&st, user_id).await, "tutorial_likes");
    let (likes_post, _, _, raw) = last_post(&st).await;
    assert_eq!(raw, t(&st, "new_user_narrative.likes.instructions", &[]));
    reply(&st, user_id, topic_id, "liking is hard").await;
    run_bot_jobs(&st).await;
    let slug: String = sqlx::query_scalar("SELECT slug FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&st.pool)
        .await
        .unwrap();
    let number: i32 = sqlx::query_scalar("SELECT post_number FROM posts WHERE id = $1")
        .bind(likes_post)
        .fetch_one(&st.pool)
        .await
        .unwrap();
    assert_eq!(
        last_post(&st).await.3,
        t(
            &st,
            "new_user_narrative.likes.not_found",
            &[("url", &format!("/t/{slug}/{topic_id}/{number}"))]
        )
    );
    like(&st, user_id, likes_post).await;
    run_bot_jobs(&st).await;
    assert_eq!(state_of(&st, user_id).await, "tutorial_flag");
}

#[tokio::test]
async fn generic_replies_answer_twice_then_stay_quiet() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "chatty", false).await;
    let (topic_id, _) = start_new_user_track(&st, user_id).await;
    set_data(
        &st,
        user_id,
        json!({ "state": "end", "topic_id": topic_id, "track": NEW_USER }),
    )
    .await;
    reply(&st, user_id, topic_id, "what now").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        t0(&st, "track_selector.do_not_understand.first_response")
    );
    reply(&st, user_id, topic_id, "and now").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        st.i18n
            .t_with(
                "discourse_narrative_bot.track_selector.do_not_understand.second_response",
                &[("base_path", ""), ("reset_trigger", "start")]
            )
            .unwrap()
    );
    let before = post_count(&st).await;
    reply(&st, user_id, topic_id, "hello?").await;
    run_bot_jobs(&st).await;
    assert_eq!(post_count(&st).await, before + 1);
}

#[tokio::test]
async fn a_missed_step_twice_gets_the_generic_reply() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "stuck", false).await;
    let (topic_id, _) = start_new_user_track(&st, user_id).await;
    set_data(
        &st,
        user_id,
        json!({ "state": "tutorial_formatting", "topic_id": topic_id, "track": NEW_USER }),
    )
    .await;
    reply(&st, user_id, topic_id, "show me what you can do").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        t(&st, "new_user_narrative.formatting.not_found", &[])
    );
    reply(&st, user_id, topic_id, "show me what you can do").await;
    run_bot_jobs(&st).await;
    let track = st
        .i18n
        .t_with(
            "discourse_narrative_bot.track_selector.do_not_understand.track_response",
            &[
                ("reset_trigger", "start tutorial"),
                ("skip_trigger", "skip"),
            ],
        )
        .unwrap();
    assert_eq!(
        last_post(&st).await.3,
        two(
            &t0(&st, "track_selector.do_not_understand.first_response"),
            &track
        )
    );
}

#[tokio::test]
async fn thanking_discobot_ends_the_track_with_a_like() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "grateful", false).await;
    let (topic_id, _) = start_new_user_track(&st, user_id).await;
    let thanks = reply(&st, user_id, topic_id, "thanks!").await;
    run_bot_jobs(&st).await;
    let (last, ..) = last_post(&st).await;
    assert_eq!(last, thanks);
    let liked: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM post_actions WHERE post_id = $1 AND user_id = -2 AND post_action_type_id = 2)",
    )
    .bind(thanks)
    .fetch_one(&st.pool)
    .await
    .unwrap();
    assert!(liked);
    assert_eq!(data(&st, user_id).await["state"], Value::Null);
    assert_eq!(data(&st, user_id).await["topic_id"], Value::Null);
}

#[tokio::test]
async fn discobot_answers_commands_in_public() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "commander", false).await;
    let random_mention = st
        .i18n
        .t_with(
            "discourse_narrative_bot.track_selector.random_mention.reply",
            &[
                ("discobot_username", "discobot"),
                ("help_trigger", "display help"),
            ],
        )
        .unwrap();

    let first = public_topic(&st, user_id, "Show me what you can do @discobot").await;
    run_bot_jobs(&st).await;
    let (_, author, topic_id, raw) = last_post(&st).await;
    assert_eq!((author, raw.as_str()), (DISCOBOT, random_mention.as_str()));
    let _ = first;

    reply(&st, user_id, topic_id, "@discobot roll 2d1").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        t0(&st, "dice.results").replace("%{results}", "1, 1")
    );
    reply(&st, user_id, topic_id, "@discobot   roll 2d1").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        t0(&st, "dice.results").replace("%{results}", "1, 1")
    );

    reply(&st, user_id, topic_id, "@discobot quote").await;
    run_bot_jobs(&st).await;
    let quotes: Vec<String> = (1..=13)
        .map(|n| {
            let q = t0(&st, &format!("quote.{n}.quote"));
            let a = t0(&st, &format!("quote.{n}.author"));
            st.i18n
                .t_with(
                    "discourse_narrative_bot.quote.results",
                    &[("quote", &q), ("author", &a)],
                )
                .unwrap()
        })
        .collect();
    let (_, author, _, raw) = last_post(&st).await;
    assert_eq!(author, DISCOBOT);
    assert!(quotes.contains(&raw), "{raw}");

    reply(&st, user_id, topic_id, "@discobot fortune").await;
    run_bot_jobs(&st).await;
    let answers: Vec<String> = (1..=20)
        .map(|n| {
            let a = t0(&st, &format!("magic_8_ball.answers.{n}"));
            t0(&st, "magic_8_ball.result").replace("%{result}", &a)
        })
        .collect();
    assert!(answers.contains(&last_post(&st).await.3));

    reply(&st, user_id, topic_id, "@discobot display help").await;
    run_bot_jobs(&st).await;
    let help = last_post(&st).await.3;
    let tracks = st
        .i18n
        .t_with(
            "discourse_narrative_bot.track_selector.random_mention.tracks",
            &[
                ("discobot_username", "discobot"),
                ("reset_trigger", "start"),
                ("tracks", "tutorial, advanced tutorial"),
            ],
        )
        .unwrap();
    assert!(help.starts_with(&tracks), "{help}");

    // A command inside a quote is not one.
    let before = post_count(&st).await;
    reply(
        &st,
        user_id,
        topic_id,
        "[quote=\"Donkey, post:6, topic:1\"]\n@discobot roll 2d1\n[/quote]",
    )
    .await;
    run_bot_jobs(&st).await;
    assert_eq!(post_count(&st).await, before + 1);

    // Nor a trigger in code.
    reply(&st, user_id, topic_id, "`@discobot start tutorial`").await;
    run_bot_jobs(&st).await;
    assert_eq!(post_count(&st).await, before + 2);
    assert!(queued(&st, "narrative_init").await.is_empty());
}

#[tokio::test]
async fn public_replies_can_be_turned_off_and_tips_skipped() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "quietone", false).await;
    common::set_setting(
        &st.pool,
        "discourse_narrative_bot_disable_public_replies",
        5,
        "t",
    )
    .await;
    let before = post_count(&st).await;
    public_topic(&st, user_id, "Show me what you can do @discobot").await;
    run_bot_jobs(&st).await;
    assert_eq!(post_count(&st).await, before + 1);

    common::set_setting(
        &st.pool,
        "discourse_narrative_bot_disable_public_replies",
        5,
        "f",
    )
    .await;
    sqlx::query("UPDATE user_options SET skip_new_user_tips = TRUE WHERE user_id = $1")
        .bind(user_id)
        .execute(&st.pool)
        .await
        .unwrap();
    public_topic(&st, user_id, "Show me what you can do, @discobot").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        t0(&st, "track_selector.random_mention.discobot_disabled")
    );
}

#[tokio::test]
async fn dice_out_of_bounds_are_explained() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "gambler", false).await;
    let (topic_id, _) = start_new_user_track(&st, user_id).await;
    set_data(
        &st,
        user_id,
        json!({ "state": "end", "topic_id": topic_id }),
    )
    .await;
    reply(&st, user_id, topic_id, "roll 0d1").await;
    run_bot_jobs(&st).await;
    assert_eq!(last_post(&st).await.3, t0(&st, "dice.invalid"));
    reply(&st, user_id, topic_id, "roll 21d1").await;
    run_bot_jobs(&st).await;
    let not_enough = st
        .i18n
        .t_count("discourse_narrative_bot.dice.not_enough_dice", 20, &[])
        .unwrap();
    let ones = vec!["1"; 20].join(", ");
    assert_eq!(
        last_post(&st).await.3,
        two(
            &not_enough,
            &t0(&st, "dice.results").replace("%{results}", &ones)
        )
    );
}

#[tokio::test]
async fn a_reset_trigger_restarts_the_tutorial() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "restarter", false).await;
    let (topic_id, _) = start_new_user_track(&st, user_id).await;
    set_data(
        &st,
        user_id,
        json!({ "state": "tutorial_formatting", "topic_id": topic_id, "track": NEW_USER }),
    )
    .await;
    let post_id = reply(&st, user_id, topic_id, "'start tutorial'").await;
    // The reset keeps the message and starts over in two seconds.
    bot_input(
        &st,
        json!({ "user_id": user_id, "post_id": post_id, "input": "reply" }),
    )
    .await;
    assert_eq!(data(&st, user_id).await, json!({ "topic_id": topic_id }));
    let init = queued(&st, "narrative_init").await;
    assert_eq!(init, vec![json!({ "user_id": user_id, "klass": NEW_USER })]);
    sqlx::query("DELETE FROM discourse_rs.jobs WHERE name = 'bot_input'")
        .execute(&st.pool)
        .await
        .unwrap();
    run_bot_jobs(&st).await;
    // The hello goes into the same message.
    let state = data(&st, user_id).await;
    assert_eq!(state["state"], json!("tutorial_bookmark"));
    assert_eq!(state["topic_id"], json!(topic_id));
    assert_eq!(last_post(&st).await.2, topic_id);
}

#[tokio::test]
async fn a_long_post_resets_only_through_its_cooked_trigger() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "verbose", false).await;
    let (topic_id, _) = start_new_user_track(&st, user_id).await;
    set_data(
        &st,
        user_id,
        json!({ "state": "tutorial_formatting", "topic_id": topic_id, "track": NEW_USER }),
    )
    .await;
    let long = format!("I would like to start tutorial now {}", "a".repeat(200));
    reply(&st, user_id, topic_id, &long).await;
    run_bot_jobs(&st).await;
    assert_eq!(state_of(&st, user_id).await, "tutorial_formatting");
    assert_eq!(
        last_post(&st).await.3,
        t(&st, "new_user_narrative.formatting.not_found", &[])
    );
}

#[tokio::test]
async fn the_advanced_tutorial_runs_to_its_certificate() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "advanced", false).await;
    // `@discobot start advanced tutorial` in public: the track in a new
    // message.
    public_topic(&st, user_id, "@discobot start advanced tutorial").await;
    run_bot_jobs(&st).await;
    let (topic_id, title): (i32, String) = sqlx::query_as(
        "SELECT id, title FROM topics WHERE archetype = 'private_message' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&st.pool)
    .await
    .unwrap();
    assert_eq!(title, t0(&st, "advanced_user_narrative.title"));
    let posts: Vec<(i32, i32, String)> = sqlx::query_as(
        "SELECT id, user_id, raw FROM posts WHERE topic_id = $1 ORDER BY post_number",
    )
    .bind(topic_id)
    .fetch_all(&st.pool)
    .await
    .unwrap();
    assert_eq!(
        posts[0].2,
        two(
            &t(
                &st,
                "advanced_user_narrative.start_message",
                &[("username", "advanced")]
            ),
            &t(&st, "advanced_user_narrative.edit.instructions", &[])
        )
    );
    // init_tutorial_edit: a post of the user's, with no bot input of its own.
    assert_eq!(posts[1].1, user_id);
    assert_eq!(
        posts[1].2,
        t(
            &st,
            "advanced_user_narrative.edit.bot_created_post_raw",
            &[("discobot_username", "discobot")]
        )
    );
    let to_edit = posts[1].0;
    assert_eq!(state_of(&st, user_id).await, "tutorial_edit");
    assert_eq!(
        data(&st, user_id).await["tutorial_edit"]["post_id"],
        json!(to_edit)
    );

    // A reply instead of an edit: the reminder.
    reply(&st, user_id, topic_id, "Which post?").await;
    run_bot_jobs(&st).await;
    let slug: String = sqlx::query_scalar("SELECT slug FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&st.pool)
        .await
        .unwrap();
    assert_eq!(
        last_post(&st).await.3,
        t(
            &st,
            "advanced_user_narrative.edit.not_found",
            &[("url", &format!("/t/{slug}/{topic_id}/2"))]
        )
    );

    // edit
    revise(&st, user_id, to_edit, "I edited this post, just as asked.").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        two(
            &t(&st, "advanced_user_narrative.edit.reply", &[]),
            &t(&st, "advanced_user_narrative.delete.instructions", &[])
        )
    );
    assert_eq!(state_of(&st, user_id).await, "tutorial_delete");

    // delete: the user removes their own post.
    let doomed = reply(&st, user_id, topic_id, "This post will not last long.").await;
    run_bot_jobs(&st).await;
    let settings = settings(&st).await;
    let host = Host::from_state(&st);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &st.config,
        i18n: &st.i18n,
        bus: &st.bus,
    };
    discourse_rs::post_destroyer::destroy_as(&st.pool, &ctx, user_id, doomed, "")
        .await
        .unwrap();
    run_bot_jobs(&st).await;
    // The delete's reply, then the post to recover (made and deleted by
    // the user, with no delete input of its own).
    let recent: Vec<(i32, String, bool)> = sqlx::query_as(
        "SELECT user_id, raw, user_deleted FROM posts WHERE topic_id = $1 ORDER BY post_number DESC LIMIT 2",
    )
    .bind(topic_id)
    .fetch_all(&st.pool)
    .await
    .unwrap();
    assert_eq!(
        recent[1].1,
        two(
            &t(&st, "advanced_user_narrative.delete.reply", &[]),
            &t(&st, "advanced_user_narrative.recover.instructions", &[])
        )
    );
    assert_eq!(recent[0].0, user_id);
    assert!(recent[0].2, "the post to recover is deleted by its author");
    assert_eq!(state_of(&st, user_id).await, "tutorial_recover");

    // Recovering is the author's (not ported in core): skip it.
    reply(&st, user_id, topic_id, "skip").await;
    run_bot_jobs(&st).await;
    let category: String = sqlx::query_scalar(
        "SELECT CASE WHEN p.slug IS NULL THEN c.slug ELSE p.slug || ':' || c.slug END \
         FROM categories c LEFT JOIN categories p ON p.id = c.parent_category_id \
         WHERE NOT c.read_restricted ORDER BY c.id DESC LIMIT 1",
    )
    .fetch_one(&st.pool)
    .await
    .unwrap();
    assert_eq!(
        last_post(&st).await.3,
        t(
            &st,
            "advanced_user_narrative.category_hashtag.instructions",
            &[("category", &format!("#{category}"))]
        )
    );

    // category hashtag
    reply(&st, user_id, topic_id, "No hashtag here").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        t(
            &st,
            "advanced_user_narrative.category_hashtag.not_found",
            &[]
        )
    );
    reply(&st, user_id, topic_id, "Have a look at #uncategorized").await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        two(
            &t(&st, "advanced_user_narrative.category_hashtag.reply", &[]),
            &t(
                &st,
                "advanced_user_narrative.change_topic_notification_level.instructions",
                &[]
            )
        )
    );

    // notification level
    let mut conn = st.pool.acquire().await.unwrap();
    discourse_rs::posting::change_topic_user(
        &st.bus,
        &mut conn,
        user_id,
        topic_id,
        &[discourse_rs::posting::TopicUserAttr::NotificationLevel(
            1, 3,
        )],
    )
    .await
    .unwrap();
    drop(conn);
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        two(
            &t(
                &st,
                "advanced_user_narrative.change_topic_notification_level.reply",
                &[]
            ),
            &t(&st, "advanced_user_narrative.poll.instructions", &[])
        )
    );

    // poll
    reply(
        &st,
        user_id,
        topic_id,
        "[poll]\n* Capybara\n* Unicorn\n[/poll]",
    )
    .await;
    run_bot_jobs(&st).await;
    assert_eq!(
        last_post(&st).await.3,
        two(
            &t(&st, "advanced_user_narrative.poll.reply", &[]),
            &t(&st, "advanced_user_narrative.details.instructions", &[])
        )
    );

    // details
    reply(
        &st,
        user_id,
        topic_id,
        "[details=\"More\"]\nHidden\n[/details]",
    )
    .await;
    run_bot_jobs(&st).await;
    let date = chrono::Utc::now()
        .format("%b %d %Y")
        .to_string()
        .replace(' ', "+");
    let certificate = format!(
        "<iframe class='discobot-certificate' src='http://test.localhost/discobot/certificate.svg?date={date}&type=advanced&user_id={user_id}' width='650' height='464' alt='{}'></iframe>",
        t0(&st, "advanced_user_narrative.certificate.alt")
    );
    let last_two: Vec<String> = sqlx::query_scalar(
        "SELECT raw FROM posts WHERE topic_id = $1 ORDER BY post_number DESC LIMIT 2",
    )
    .bind(topic_id)
    .fetch_all(&st.pool)
    .await
    .unwrap();
    assert_eq!(
        last_two[0],
        t(
            &st,
            "advanced_user_narrative.end.message",
            &[("certificate", &certificate)]
        )
    );
    assert_eq!(
        last_two[1],
        t(&st, "advanced_user_narrative.details.reply", &[])
    );
    assert_eq!(
        data(&st, user_id).await,
        json!({ "topic_id": topic_id, "state": "end", "track": ADVANCED, "attempted": false })
    );
    let badged: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_badges ub JOIN badges b ON b.id = ub.badge_id \
         WHERE ub.user_id = $1 AND b.name = 'Licensed')",
    )
    .bind(user_id)
    .fetch_one(&st.pool)
    .await
    .unwrap();
    assert!(badged);
}

#[tokio::test]
async fn the_timeout_nudges_under_discobots_last_post() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "sleepy", false).await;
    let (topic_id, hello_id) = start_new_user_track(&st, user_id).await;
    discourse_rs::plugins::narrative_bot::jobs::run(
        &st,
        "narrative_timeout",
        &json!({ "user_id": user_id, "klass": NEW_USER }),
    )
    .await
    .unwrap();
    let (id, author, topic, raw) = last_post(&st).await;
    assert_eq!((author, topic), (DISCOBOT, topic_id));
    assert_eq!(
        raw,
        t(
            &st,
            "timeout.message",
            &[
                ("username", "sleepy"),
                ("skip_trigger", "skip"),
                ("reset_trigger", "start tutorial"),
            ]
        )
    );
    let reply_to: Option<i32> =
        sqlx::query_scalar("SELECT reply_to_post_number FROM posts WHERE id = $1")
            .bind(id)
            .fetch_one(&st.pool)
            .await
            .unwrap();
    let hello_number: i32 = sqlx::query_scalar("SELECT post_number FROM posts WHERE id = $1")
        .bind(hello_id)
        .fetch_one(&st.pool)
        .await
        .unwrap();
    assert_eq!(reply_to, Some(hello_number));
    // Emailed, unlike the other replies.
    let options: Vec<Value> = queued(&st, "post_alert")
        .await
        .into_iter()
        .filter(|a| a["post_id"] == json!(id))
        .map(|a| a["options"].clone())
        .collect();
    assert_eq!(options, vec![json!({ "skip_send_email": false })]);
}

#[tokio::test]
async fn core_events_enqueue_the_bot_input() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "eventful", false).await;
    clear_jobs(&st).await;
    let post_id = public_topic(&st, user_id, "A topic to see the events go by.").await;
    let topic_id: i32 = sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
        .bind(post_id)
        .fetch_one(&st.pool)
        .await
        .unwrap();
    let inputs = queued(&st, "bot_input").await;
    // The creator watching the topic, then the post.
    assert_eq!(
        inputs,
        vec![
            json!({ "user_id": user_id, "topic_id": topic_id, "input": "topic_notification_level_changed" }),
            json!({ "user_id": user_id, "post_id": post_id, "input": "reply" }),
        ]
    );
    clear_jobs(&st).await;
    revise(
        &st,
        user_id,
        post_id,
        "A topic to see the events go by, edited.",
    )
    .await;
    assert_eq!(
        queued(&st, "bot_input").await,
        vec![json!({ "user_id": user_id, "post_id": post_id, "input": "edit" })]
    );
    clear_jobs(&st).await;
    let admin_post = reply(&st, 1, topic_id, "An admin's reply, to like.").await;
    // Admins are human too.
    clear_jobs(&st).await;
    like(&st, user_id, admin_post).await;
    bookmark(&st, user_id, topic_id, "Topic").await;
    assert_eq!(
        queued(&st, "bot_input").await,
        vec![
            json!({ "user_id": user_id, "post_id": admin_post, "input": "like" }),
            json!({ "user_id": user_id, "post_id": post_id, "input": "bookmark" }),
        ]
    );
    // Not for ignored users, nor while the plugin is off.
    clear_jobs(&st).await;
    common::set_setting(
        &st.pool,
        "discourse_narrative_bot_ignored_usernames",
        8,
        "eventful",
    )
    .await;
    reply(&st, user_id, topic_id, "Ignored by the bot, this reply is.").await;
    common::set_setting(&st.pool, "discourse_narrative_bot_ignored_usernames", 8, "").await;
    common::set_setting(&st.pool, "discourse_narrative_bot_enabled", 5, "f").await;
    reply(&st, user_id, topic_id, "The plugin is off for this one.").await;
    assert!(queued(&st, "bot_input").await.is_empty());
}

#[tokio::test]
async fn bot_input_for_a_missing_user_does_nothing() {
    let (_db, st) = setup().await;
    let post_id = public_topic(&st, 2, "@discobot quote").await;
    let before = post_count(&st).await;
    bot_input(
        &st,
        json!({ "user_id": 99999, "post_id": post_id, "input": "reply" }),
    )
    .await;
    assert_eq!(post_count(&st).await, before);
    bot_input(
        &st,
        json!({ "user_id": 2, "post_id": post_id, "input": "reply" }),
    )
    .await;
    assert_eq!(post_count(&st).await, before + 1);
    assert_eq!(last_post(&st).await.1, DISCOBOT);
}

#[tokio::test]
async fn deleting_a_user_removes_their_tutorial_state() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "leaving", false).await;
    set_data(&st, user_id, json!({ "state": "end" })).await;
    let settings = settings(&st).await;
    let mut conn = st.pool.acquire().await.unwrap();
    discourse_rs::plugins::narrative_bot::user_destroyed(&mut conn, &settings, user_id)
        .await
        .unwrap();
    assert_eq!(data(&st, user_id).await, Value::Null);
}

#[tokio::test]
async fn replies_to_discobots_system_messages_are_allowed() {
    let (_db, st) = setup().await;
    common::set_setting(&st.pool, "enable_system_message_replies", 5, "f").await;
    let user_id = new_user(&st, "replier", false).await;
    let (topic_id, _) = start_new_user_track(&st, user_id).await;
    reply(&st, user_id, topic_id, "Replying to discobot anyway").await;
    assert_eq!(last_post(&st).await.1, user_id);
}

#[tokio::test]
async fn the_promotion_to_tl2_invites_to_the_advanced_tutorial() {
    let (_db, st) = setup().await;
    let user_id = new_user(&st, "promoted", false).await;
    let settings = settings(&st).await;
    let host = Host::from_state(&st);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &st.config,
        i18n: &st.i18n,
        bus: &st.bus,
    };
    discourse_rs::system_message::create(
        &st.pool,
        &ctx,
        user_id,
        "tl2_promotion_message",
        &[],
        None,
    )
    .await
    .unwrap();
    let (_, author, topic_id, raw) = last_post(&st).await;
    assert_eq!(author, DISCOBOT);
    assert_eq!(
        raw,
        st.i18n
            .t_with(
                "discourse_narrative_bot.tl2_promotion_message.text_body_template",
                &[
                    ("discobot_username", "discobot"),
                    ("reset_trigger", "start advanced tutorial")
                ]
            )
            .unwrap()
            .trim_end()
    );
    let title: String = sqlx::query_scalar("SELECT title FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&st.pool)
        .await
        .unwrap();
    assert_eq!(
        title,
        st.i18n
            .t("discourse_narrative_bot.tl2_promotion_message.subject_template")
            .unwrap()
    );
}

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(&mut self, method: Method, path: &str, body: &str) -> (u16, String, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest");
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        if let Some(token) = &self.csrf {
            request = request.header("x-csrf-token", token.as_str());
        }
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        let kind = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap();
            let pair = v.split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, kind, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn anonymous(state: AppState) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, _, csrf) = client.send(Method::GET, "/session/csrf.json", "").await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client::anonymous(state).await;
        let (status, _, body) = client
            .send(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        client
    }
}

#[tokio::test]
async fn the_certificate_is_drawn_for_logged_in_users() {
    let (_db, st) = setup().await;
    let date = "Oct+28+2026";
    let path = format!("/discobot/certificate.svg?date={date}&user_id=3");
    let mut anon = Client::anonymous(st.clone()).await;
    assert_eq!(anon.send(Method::GET, &path, "").await.0, 404);

    let mut client = Client::logged_in(st.clone(), "user1").await;
    let (status, kind, svg) = client.send(Method::GET, &path, "").await;
    assert_eq!(status, 200, "{svg}");
    assert_eq!(kind, "image/svg+xml; charset=utf-8");
    assert!(svg.starts_with("<svg"), "{}", &svg[..100]);
    assert!(svg.contains(">\n      October 28, 2026\n"), "the date");
    assert!(svg.contains("User1"), "the user's name, titleized");
    assert!(svg.contains("discobot"));
    assert!(
        svg.contains(
            st.i18n
                .t("discourse_narrative_bot.new_user_narrative.cert_title")
                .unwrap()
        )
    );
    assert!(svg.contains(
        "http://test.localhost/plugins/discourse-narrative-bot/fonts/GreatVibes-Regular.ttf"
    ));
    let (_, _, advanced) = client
        .send(Method::GET, &format!("{path}&type=advanced"), "")
        .await;
    assert!(
        advanced.contains(
            st.i18n
                .t("discourse_narrative_bot.advanced_user_narrative.cert_title")
                .unwrap()
        )
    );
    assert!(!advanced.contains("<%"));

    // Each param is required.
    for missing in [
        "/discobot/certificate.svg?user_id=3",
        "/discobot/certificate.svg?date=Oct+28+2026",
    ] {
        assert_eq!(client.send(Method::GET, missing, "").await.0, 400);
    }
    // A user that does not exist.
    assert_eq!(
        client
            .send(
                Method::GET,
                &format!("/discobot/certificate.svg?date={date}&user_id=99999"),
                ""
            )
            .await
            .0,
        404
    );
    // The images the messages show.
    let (status, kind, _) = client
        .send(
            Method::GET,
            "/plugins/discourse-narrative-bot/images/dog-walk.gif",
            "",
        )
        .await;
    assert_eq!((status, kind.as_str()), (200, "image/gif"));
}
