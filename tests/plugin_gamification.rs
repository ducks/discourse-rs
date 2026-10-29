//! discourse-gamification, ported from the plugin's specs: the score
//! sources (spec/lib/scorables), the daily rollup and overrides
//! (spec/models), the cached positions (spec/lib/leaderboard_cached_view_spec.rb),
//! the directory column (spec/lib/directory_integration_spec.rb), the
//! jobs (spec/jobs), the controllers (spec/requests), the serializer keys
//! (spec/plugin_spec.rb), and the pages Ember draws.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use discourse_rs::plugins::gamification::scorables::{self, Scorable, Since};
use discourse_rs::plugins::gamification::{self as plugin, Leaderboard, cached_view};
use discourse_rs::site_settings::SiteSettings;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    fn anonymous(state: AppState) -> Client {
        Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        }
    }

    async fn request(
        &mut self,
        method: Method,
        path: &str,
        body: &str,
        xhr: bool,
    ) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if xhr {
            request = request.header("x-requested-with", "XMLHttpRequest");
        }
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
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap();
            let pair = v.split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client::anonymous(state);
        let (_, csrf) = client
            .request(Method::GET, "/session/csrf.json", "", true)
            .await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .request(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
                true,
            )
            .await;
        client
    }

    async fn json(&mut self, method: Method, path: &str, body: &str) -> (StatusCode, Value) {
        let (status, body) = self.request(method, path, body, true).await;
        (status, serde_json::from_str(&body).unwrap_or(Value::Null))
    }

    async fn get(&mut self, path: &str) -> (StatusCode, Value) {
        self.json(Method::GET, path, "").await
    }

    async fn page(&mut self, path: &str) -> (StatusCode, String) {
        self.request(Method::GET, path, "", false).await
    }
}

async fn exec(db: &TestDb, sql: &str) {
    sqlx::query(sql)
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn scalar_i64(db: &TestDb, sql: &str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// The plugin on, without the seeded Global Leaderboard (the specs run
/// without fixtures).
async fn setup() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    set_setting(&db.pool, "discourse_gamification_enabled", 5, "t").await;
    exec(&db, "DELETE FROM gamification_leaderboards").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, st)
}

async fn settings(st: &AppState, conn: &mut sqlx::PgConnection) -> SiteSettings {
    SiteSettings::load(conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap()
}

/// `Fabricate(:user)`: a copy of user3 with its options and a primary
/// email of its own.
async fn fabricate_user(db: &TestDb, id: i32, username: &str, extra: &str) {
    exec(
        db,
        &format!(
            "INSERT INTO users SELECT (jsonb_populate_record(u, jsonb_build_object(\
               'id', {id}, 'username', '{username}', 'username_lower', lower('{username}'), \
               'name', '{username} name', 'created_at', now() - interval '2 years'{extra}))).* \
             FROM users u WHERE u.id = 5"
        ),
    )
    .await;
    exec(
        db,
        &format!(
            "INSERT INTO user_options SELECT (jsonb_populate_record(o, jsonb_build_object('user_id', {id}))).* \
             FROM user_options o WHERE o.user_id = 5"
        ),
    )
    .await;
    exec(
        db,
        &format!(
            "INSERT INTO user_emails (user_id, email, \"primary\", created_at, updated_at) \
             VALUES ({id}, '{username}@example.com', TRUE, now(), now())"
        ),
    )
    .await;
    exec(
        db,
        &format!(
            "INSERT INTO user_stats SELECT (jsonb_populate_record(s, jsonb_build_object('user_id', {id}))).* \
             FROM user_stats s WHERE s.user_id = 5"
        ),
    )
    .await;
}

/// `Fabricate(:gamification_leaderboard)`, with the columns given.
async fn fabricate_leaderboard(db: &TestDb, name: &str, extra: &[(&str, &str)]) -> i64 {
    let mut cols = vec!["name", "created_by_id", "created_at", "updated_at"];
    let quoted = format!("'{name}'");
    let mut vals = vec![quoted.as_str(), "1", "now()", "now()"];
    for (c, v) in extra {
        cols.push(c);
        vals.push(v);
    }
    scalar_i64(
        db,
        &format!(
            "INSERT INTO gamification_leaderboards ({}) VALUES ({}) RETURNING id",
            cols.join(", "),
            vals.join(", ")
        ),
    )
    .await
}

/// `Fabricate(:gamification_leaderboard_score)`; `date` SQL.
async fn score(db: &TestDb, lb: i64, user_id: i32, points: i32, date: &str) {
    exec(
        db,
        &format!(
            "INSERT INTO gamification_leaderboard_scores (leaderboard_id, user_id, date, score) \
             VALUES ({lb}, {user_id}, {date}, {points})"
        ),
    )
    .await;
}

/// `Fabricate(:topic, user:)` in a category; returns its id.
async fn fabricate_topic(db: &TestDb, user_id: i32, category: Option<i32>, extra: &str) -> i32 {
    let category = category.map_or("NULL".into(), |c| c.to_string());
    let id: i32 = sqlx::query_scalar(&format!(
        "INSERT INTO topics (title, slug, bumped_at, last_post_user_id, updated_at, created_at, user_id, category_id, archetype{}) \
         VALUES ('A topic about things', 'a-topic-about-things', now(), {user_id}, now(), now(), {user_id}, {category}, 'regular'{}) RETURNING id",
        if extra.is_empty() { "" } else { ", deleted_at" },
        if extra.is_empty() { String::new() } else { format!(", {extra}") },
    ))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    id
}

/// `Fabricate(:post, user:, topic:, post_number:)`; returns its id.
async fn fabricate_post(db: &TestDb, topic_id: i32, user_id: i32, number: i32, extra: &str) -> i32 {
    sqlx::query_scalar(&format!(
        "INSERT INTO posts (topic_id, user_id, post_number, raw, cooked, created_at, updated_at, last_version_at{}) \
         VALUES ({topic_id}, {user_id}, {number}, 'hello world', '<p>hello world</p>', now(), now(), now(){}) RETURNING id",
        if extra.is_empty() { "" } else { ", " }.to_string() + extra.split('=').next().unwrap_or(""),
        if extra.is_empty() { String::new() } else { format!(", {}", extra.split('=').nth(1).unwrap_or("")) },
    ))
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// A like of the post.
async fn like(db: &TestDb, post_id: i32, user_id: i32) {
    exec(
        db,
        &format!(
            "INSERT INTO post_actions (post_id, user_id, post_action_type_id, created_at, updated_at) \
             VALUES ({post_id}, {user_id}, 2, now(), now())"
        ),
    )
    .await;
}

/// The plugin's `user.gamification_score`, positions generated first.
async fn user_score(st: &AppState, user_id: i32) -> i64 {
    let mut conn = st.pool.acquire().await.unwrap();
    plugin::user_score(&mut conn, user_id).await.unwrap()
}

/// `calculate_scores(leaderboard, since_date:, only_subclass:)` then
/// `LeaderboardCachedView.create_all`.
async fn calculate(st: &AppState, lb: i64, since: Since, only: Option<Scorable>) {
    let mut conn = st.pool.acquire().await.unwrap();
    let s = settings(st, &mut conn).await;
    let leaderboard = Leaderboard::find(&mut conn, lb).await.unwrap().unwrap();
    scorables::calculate_scores(&mut conn, &s, &leaderboard, &since, only)
        .await
        .unwrap();
    cached_view::delete(&mut conn, lb).await.unwrap();
    cached_view::create(&mut conn, &s, lb).await.unwrap();
}

fn since_2022() -> Since {
    Since::Text("2022-1-1".into())
}

// spec/lib/scorables/shared_scorables_spec.rb

#[tokio::test]
async fn likes_received_and_given() {
    let (db, st) = setup().await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    fabricate_user(&db, 101, "current", "").await;
    fabricate_user(&db, 102, "other", "").await;
    fabricate_user(&db, 103, "third", "").await;
    let topic = fabricate_topic(&db, 102, Some(4), "").await;
    let mut posts = Vec::new();
    for n in 0..10 {
        posts.push(fabricate_post(&db, topic, 101, n + 2, "").await);
    }
    for p in &posts {
        like(&db, *p, 102).await;
    }
    // ten likes received
    calculate(&st, lb, since_2022(), Some(Scorable::LikeReceived)).await;
    assert_eq!(user_score(&st, 101).await, 10);
    // ten likes given, by the other user; five by the third
    for p in posts.iter().take(5) {
        like(&db, *p, 103).await;
    }
    calculate(&st, lb, since_2022(), Some(Scorable::LikeGiven)).await;
    assert_eq!(user_score(&st, 102).await, 10);
    assert_eq!(user_score(&st, 103).await, 5);
}

#[tokio::test]
async fn likes_in_deleted_posts_and_messages_score_nothing() {
    let (db, st) = setup().await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    fabricate_user(&db, 101, "current", "").await;
    let deleted = fabricate_topic(&db, 101, Some(4), "now()").await;
    let p1 = fabricate_post(&db, deleted, 101, 2, "deleted_at=now()").await;
    let pm: i32 = sqlx::query_scalar(
        "INSERT INTO topics (title, bumped_at, last_post_user_id, updated_at, created_at, user_id, archetype) \
         VALUES ('A private message here', now(), 1, now(), now(), 1, 'private_message') RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let p2 = fabricate_post(&db, pm, 101, 2, "").await;
    like(&db, p1, 1).await;
    like(&db, p2, 1).await;
    calculate(&st, lb, since_2022(), Some(Scorable::LikeReceived)).await;
    assert_eq!(user_score(&st, 101).await, 0);
}

#[tokio::test]
async fn posts_created_leave_out_first_posts_small_actions_hidden_and_deleted() {
    let (db, st) = setup().await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    fabricate_user(&db, 101, "current", "").await;
    let topic = fabricate_topic(&db, 1, Some(4), "").await;
    fabricate_post(&db, topic, 101, 2, "").await;
    fabricate_post(&db, topic, 101, 3, "").await;
    // OP is not counted
    let own = fabricate_topic(&db, 101, Some(4), "").await;
    fabricate_post(&db, own, 101, 1, "").await;
    // small actions are not counted
    fabricate_post(&db, topic, 101, 4, "post_type=3").await;
    // hidden posts are not counted
    fabricate_post(&db, topic, 101, 5, "hidden=TRUE").await;
    // deleted topics are not counted
    let deleted = fabricate_topic(&db, 1, Some(4), "now()").await;
    fabricate_post(&db, deleted, 101, 2, "").await;
    // wiki posts are not counted
    fabricate_post(&db, topic, 101, 6, "wiki=TRUE").await;
    calculate(&st, lb, since_2022(), Some(Scorable::PostCreated)).await;
    assert_eq!(user_score(&st, 101).await, 4);
}

#[tokio::test]
async fn visits_reading_and_time_read() {
    let (db, st) = setup().await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    fabricate_user(&db, 101, "current", "").await;
    exec(
        &db,
        "INSERT INTO user_visits (user_id, visited_at, posts_read, time_read) \
         SELECT 101, d::date, 100, 3600 FROM generate_series('2022-01-01'::date, '2022-01-30', '1 day') d",
    )
    .await;
    // thirty days visited
    calculate(&st, lb, since_2022(), Some(Scorable::DayVisited)).await;
    assert_eq!(user_score(&st, 101).await, 30);
    // thirty days of reading a hundred posts
    calculate(&st, lb, since_2022(), Some(Scorable::PostRead)).await;
    assert_eq!(user_score(&st, 101).await, 30);
    // thirty days of reading an hour
    calculate(&st, lb, since_2022(), Some(Scorable::TimeRead)).await;
    assert_eq!(user_score(&st, 101).await, 30);
}

#[tokio::test]
async fn flags_and_invites() {
    let (db, st) = setup().await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    fabricate_user(&db, 101, "current", "").await;
    exec(
        &db,
        "INSERT INTO reviewables (type, created_by_id, status, target_id, target_type, created_at, updated_at) \
         SELECT 'ReviewableFlaggedPost', 101, 1, 1000 + n, 'Post', now(), now() FROM generate_series(1, 10) n",
    )
    .await;
    // ten flags created
    calculate(&st, lb, since_2022(), Some(Scorable::FlagCreated)).await;
    assert_eq!(user_score(&st, 101).await, 100);
    exec(
        &db,
        "INSERT INTO invites (invited_by_id, invite_key, expires_at, created_at, updated_at, redemption_count) \
         SELECT 101, 'key' || n, now() + interval '1 day', now(), now(), 1 FROM generate_series(1, 10) n",
    )
    .await;
    // ten users invited
    calculate(&st, lb, since_2022(), Some(Scorable::UserInvited)).await;
    assert_eq!(user_score(&st, 101).await, 100);
}

#[tokio::test]
async fn topics_created_and_their_category_scope() {
    let (db, st) = setup().await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    fabricate_user(&db, 101, "current", "").await;
    for _ in 0..10 {
        fabricate_topic(&db, 101, Some(4), "").await;
    }
    // ten topics created; deleted ones and messages don't count
    fabricate_topic(&db, 101, Some(4), "now()").await;
    exec(
        &db,
        "INSERT INTO topics (title, bumped_at, last_post_user_id, updated_at, created_at, user_id, archetype) \
         VALUES ('A private message here', now(), 101, now(), now(), 101, 'private_message')",
    )
    .await;
    calculate(&st, lb, since_2022(), Some(Scorable::TopicCreated)).await;
    assert_eq!(user_score(&st, 101).await, 50);
    // Category Scoped Scorable Type
    set_setting(&db.pool, "scorable_categories", 11, "2").await;
    calculate(&st, lb, since_2022(), Some(Scorable::TopicCreated)).await;
    assert_eq!(user_score(&st, 101).await, 0);
    set_setting(&db.pool, "scorable_categories", 11, "4").await;
    calculate(&st, lb, since_2022(), Some(Scorable::TopicCreated)).await;
    assert_eq!(user_score(&st, 101).await, 50);
    // The leaderboard's own categories over the setting.
    exec(
        &db,
        &format!(
            "UPDATE gamification_leaderboards SET scorable_category_ids = '{{2}}' WHERE id = {lb}"
        ),
    )
    .await;
    calculate(&st, lb, since_2022(), Some(Scorable::TopicCreated)).await;
    assert_eq!(user_score(&st, 101).await, 0);
}

#[tokio::test]
async fn chat_messages_and_reactions() {
    let (db, st) = setup().await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    fabricate_user(&db, 101, "current", "").await;
    fabricate_user(&db, 102, "other", "").await;
    exec(
        &db,
        "INSERT INTO chat_messages (chat_channel_id, user_id, last_editor_id, message, created_at, updated_at) \
         SELECT 1, 101, 101, 'hello ' || n, now(), now() FROM generate_series(1, 10) n",
    )
    .await;
    // a deleted message counts for nothing
    exec(
        &db,
        "INSERT INTO chat_messages (chat_channel_id, user_id, last_editor_id, message, created_at, updated_at, deleted_at) \
         VALUES (1, 101, 101, 'gone', now(), now(), now())",
    )
    .await;
    calculate(&st, lb, since_2022(), Some(Scorable::ChatMessageCreated)).await;
    assert_eq!(user_score(&st, 101).await, 10);
    exec(
        &db,
        "INSERT INTO chat_message_reactions (chat_message_id, user_id, emoji, created_at, updated_at) \
         SELECT id, 102, 'heart', now(), now() FROM chat_messages WHERE user_id = 101 AND deleted_at IS NULL",
    )
    .await;
    // reactions on one's own message don't count
    exec(
        &db,
        "INSERT INTO chat_message_reactions (chat_message_id, user_id, emoji, created_at, updated_at) \
         SELECT id, 101, 'smile', now(), now() FROM chat_messages WHERE user_id = 101 AND deleted_at IS NULL LIMIT 1",
    )
    .await;
    calculate(&st, lb, since_2022(), Some(Scorable::ChatReactionReceived)).await;
    assert_eq!(user_score(&st, 101).await, 10);
    calculate(&st, lb, since_2022(), Some(Scorable::ChatReactionGiven)).await;
    assert_eq!(user_score(&st, 102).await, 10);
    assert_eq!(user_score(&st, 101).await, 0);
    // chat off: the sources are off
    set_setting(&db.pool, "chat_enabled", 5, "f").await;
    let mut conn = st.pool.acquire().await.unwrap();
    let s = settings(&st, &mut conn).await;
    assert!(!Scorable::ChatMessageCreated.enabled(&s, None).unwrap());
}

#[tokio::test]
async fn reactions_given_and_received() {
    let (db, st) = setup().await;
    set_setting(&db.pool, "discourse_reactions_enabled", 5, "t").await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    fabricate_user(&db, 101, "current", "").await;
    fabricate_user(&db, 102, "other", "").await;
    let topic = fabricate_topic(&db, 1, Some(4), "").await;
    for n in 0..3 {
        let p = fabricate_post(&db, topic, 101, n + 2, "").await;
        exec(
            &db,
            &format!(
                "INSERT INTO discourse_reactions_reaction_users (post_id, user_id, created_at, updated_at) \
                 VALUES ({p}, 102, now(), now())"
            ),
        )
        .await;
    }
    calculate(&st, lb, since_2022(), Some(Scorable::ReactionReceived)).await;
    assert_eq!(user_score(&st, 101).await, 3);
    calculate(&st, lb, since_2022(), Some(Scorable::ReactionGiven)).await;
    assert_eq!(user_score(&st, 102).await, 3);
}

// spec/lib/scorables/solutions_spec.rb

#[tokio::test]
async fn solutions_score_accepted_answers_not_self_accepted() {
    let (db, st) = setup().await;
    set_setting(&db.pool, "solved_enabled", 5, "t").await;
    set_setting(&db.pool, "solution_score_value", 3, "5").await;
    fabricate_user(&db, 101, "answerer", "").await;
    let topic = fabricate_topic(&db, 1, Some(4), "").await;
    let answer = fabricate_post(&db, topic, 101, 2, "").await;
    let answer2 = fabricate_post(&db, topic, 101, 3, "").await;
    let solved: i64 = sqlx::query_scalar(&format!(
        "INSERT INTO discourse_solved_solved_topics (topic_id, created_at, updated_at) \
         VALUES ({topic}, now(), now()) RETURNING id"
    ))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    exec(
        &db,
        &format!(
            "INSERT INTO discourse_solved_topic_answers (solved_topic_id, answer_post_id, accepter_user_id, created_at, updated_at) \
             VALUES ({solved}, {answer}, -1, now(), now()), ({solved}, {answer2}, -1, now(), now())"
        ),
    )
    .await;
    let mut conn = st.pool.acquire().await.unwrap();
    let s = settings(&st, &mut conn).await;
    assert!(Scorable::Solutions.enabled(&s, None).unwrap());
    let sql = scorables::bind_since(
        &Scorable::Solutions.query(&s, None).unwrap(),
        &Since::Text("2022-1-1".into()),
    );
    let rows: Vec<(i32, i64)> = sqlx::query_as(&format!(
        "SELECT user_id::int, points::bigint FROM ({sql}) q"
    ))
    .fetch_all(&mut *conn)
    .await
    .unwrap();
    // two accepted solutions by the same user
    assert_eq!(rows, vec![(101, 10)]);
    // self-accepted: the topic's own author answered
    exec(
        &db,
        &format!("UPDATE topics SET user_id = 101 WHERE id = {topic}"),
    )
    .await;
    let rows: Vec<(i32, i64)> = sqlx::query_as(&format!(
        "SELECT user_id::int, points::bigint FROM ({sql}) q"
    ))
    .fetch_all(&mut *conn)
    .await
    .unwrap();
    assert!(rows.is_empty());
    set_setting(&db.pool, "solution_score_value", 3, "0").await;
    let s = settings(&st, &mut conn).await;
    assert!(!Scorable::Solutions.enabled(&s, None).unwrap());
    set_setting(&db.pool, "solution_score_value", 3, "1").await;
    set_setting(&db.pool, "solved_enabled", 5, "f").await;
    let s = settings(&st, &mut conn).await;
    assert!(!Scorable::Solutions.enabled(&s, None).unwrap());
}

// spec/models/gamification_score_spec.rb

#[tokio::test]
async fn calculate_scores_follows_topics_and_overrides() {
    let (db, st) = setup().await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    fabricate_user(&db, 101, "current", "").await;
    let mut topics = Vec::new();
    for _ in 0..10 {
        topics.push(fabricate_topic(&db, 101, Some(4), "").await);
    }
    let today = || Since::Date(discourse_rs::clock::now().date_naive());
    calculate(&st, lb, today(), None).await;
    assert_eq!(user_score(&st, 101).await, 50);
    for t in topics.iter().take(5) {
        exec(
            &db,
            &format!("UPDATE topics SET deleted_at = now() WHERE id = {t}"),
        )
        .await;
    }
    calculate(&st, lb, today(), None).await;
    assert_eq!(user_score(&st, 101).await, 25);
    exec(
        &db,
        "UPDATE topics SET deleted_at = now() WHERE user_id = 101",
    )
    .await;
    calculate(&st, lb, today(), None).await;
    assert_eq!(user_score(&st, 101).await, 0);
    // uses leaderboard score overrides; 0 disables the source
    exec(
        &db,
        "UPDATE topics SET deleted_at = NULL WHERE user_id = 101",
    )
    .await;
    exec(
        &db,
        &format!(
            "UPDATE gamification_leaderboards SET score_overrides = '{{\"topic_created\": 10}}' WHERE id = {lb}"
        ),
    )
    .await;
    calculate(&st, lb, today(), None).await;
    assert_eq!(user_score(&st, 101).await, 100);
    exec(
        &db,
        &format!(
            "UPDATE gamification_leaderboards SET score_overrides = '{{\"topic_created\": 0}}' WHERE id = {lb}"
        ),
    )
    .await;
    calculate(&st, lb, today(), None).await;
    assert_eq!(user_score(&st, 101).await, 0);
}

// spec/models/user_spec.rb

#[tokio::test]
async fn gamification_score_is_the_default_leaderboards_all_time_total() {
    let (db, st) = setup().await;
    fabricate_user(&db, 101, "current", "").await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    score(&db, lb, 101, 10, "CURRENT_DATE - 8").await;
    score(&db, lb, 101, 25, "CURRENT_DATE - 5").await;
    exec(
        &db,
        &format!(
            "UPDATE gamification_leaderboards SET from_date = CURRENT_DATE - 5 WHERE id = {lb}"
        ),
    )
    .await;
    // Not generated yet: 0, and the job enqueued.
    assert_eq!(user_score(&st, 101).await, 0);
    let jobs = scalar_i64(
        &db,
        "SELECT COUNT(*) FROM discourse_rs.jobs WHERE name = 'Jobs::GenerateLeaderboardPositions'",
    )
    .await;
    assert_eq!(jobs, 1);
    discourse_rs::jobs::drain(&st).await.unwrap();
    assert_eq!(user_score(&st, 101).await, 25);
}

// spec/lib/leaderboard_cached_view_spec.rb

async fn view_rows(db: &TestDb, lb: i64, period: &str) -> Vec<(i32, i64, i64)> {
    sqlx::query_as(&format!(
        "SELECT user_id, total_score::bigint, position FROM gamification_leaderboard_cache_{lb}_{period} ORDER BY user_id"
    ))
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

async fn mview_count(db: &TestDb, lb: i64) -> i64 {
    scalar_i64(
        db,
        &format!(
            "SELECT COUNT(*) FROM pg_matviews WHERE matviewname LIKE 'gamification_leaderboard_cache_{lb}_%'"
        ),
    )
    .await
}

#[tokio::test]
async fn cached_views_are_created_refreshed_purged_and_deleted() {
    let (db, st) = setup().await;
    fabricate_user(&db, 101, "user", "").await;
    fabricate_user(&db, 102, "other", "").await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    score(&db, lb, 101, 0, "CURRENT_DATE - 8").await;
    let mut conn = st.pool.acquire().await.unwrap();
    let s = settings(&st, &mut conn).await;
    cached_view::create(&mut conn, &s, lb).await.unwrap();
    assert_eq!(mview_count(&db, lb).await, 6);
    assert_eq!(view_rows(&db, lb, "all_time").await, vec![(101, 0, 1)]);

    score(&db, lb, 101, 10, "CURRENT_DATE").await;
    score(&db, lb, 1, 20, "CURRENT_DATE").await;
    score(&db, lb, 102, 1, "CURRENT_DATE - 5").await;
    score(&db, lb, 102, 4, "CURRENT_DATE - 3").await;
    cached_view::refresh(&mut conn, lb).await.unwrap();
    assert_eq!(
        view_rows(&db, lb, "all_time").await,
        vec![(1, 20, 1), (101, 10, 2), (102, 5, 3)]
    );

    // Not stale while the query's signature holds.
    assert!(!cached_view::is_stale(&mut conn, &s, lb).await.unwrap());
    cached_view::purge_stale(&mut conn, &s, lb).await.unwrap();
    assert_eq!(mview_count(&db, lb).await, 6);
    // A view built from another query is stale, and all of them go.
    exec(
        &db,
        &format!(
            "COMMENT ON MATERIALIZED VIEW gamification_leaderboard_cache_{lb}_weekly IS 'old'"
        ),
    )
    .await;
    assert!(cached_view::is_stale(&mut conn, &s, lb).await.unwrap());
    cached_view::purge_stale(&mut conn, &s, lb).await.unwrap();
    assert_eq!(mview_count(&db, lb).await, 0);

    cached_view::create(&mut conn, &s, lb).await.unwrap();
    cached_view::delete(&mut conn, lb).await.unwrap();
    assert_eq!(mview_count(&db, lb).await, 0);
}

#[tokio::test]
async fn a_view_in_another_schema_is_not_ready() {
    let (db, st) = setup().await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    exec(&db, "CREATE SCHEMA test_backup").await;
    exec(
        &db,
        &format!(
            "CREATE MATERIALIZED VIEW test_backup.gamification_leaderboard_cache_{lb}_all_time AS \
             SELECT 1 AS user_id, 100 AS total_score, 1 AS position"
        ),
    )
    .await;
    let mut conn = st.pool.acquire().await.unwrap();
    assert!(
        cached_view::scores(&mut conn, lb, "all_time", None, None, 0)
            .await
            .unwrap()
            .is_err()
    );
}

async fn period_totals(st: &AppState, lb: i64) -> Vec<Option<i64>> {
    let mut conn = st.pool.acquire().await.unwrap();
    let s = settings(st, &mut conn).await;
    cached_view::delete(&mut conn, lb).await.unwrap();
    cached_view::create(&mut conn, &s, lb).await.unwrap();
    let mut out = Vec::new();
    for period in plugin::PERIODS {
        let rows = cached_view::scores(&mut conn, lb, period, None, None, 0)
            .await
            .unwrap()
            .unwrap();
        out.push(rows.first().map(|r| r.total_score));
    }
    out
}

#[tokio::test]
async fn views_filter_scores_by_the_leaderboards_dates_and_period() {
    let (db, st) = setup().await;
    fabricate_user(&db, 101, "user", "").await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    // from = today - 45, to = today - 15
    for date in [
        "CURRENT_DATE - 60",
        "CURRENT_DATE - 50",
        "CURRENT_DATE - 46",
        "CURRENT_DATE - 45",
        "(CURRENT_DATE - interval '1 month')::date",
        "CURRENT_DATE - 15",
        "CURRENT_DATE - 14",
        "CURRENT_DATE",
        "CURRENT_DATE + 15",
    ] {
        score(&db, lb, 101, 10, date).await;
    }
    let set = |from: &str, to: &str| {
        format!(
            "UPDATE gamification_leaderboards SET from_date = {from}, to_date = {to} WHERE id = {lb}"
        )
    };
    exec(&db, &set("CURRENT_DATE - 45", "CURRENT_DATE - 15")).await;
    assert_eq!(
        period_totals(&st, lb).await,
        vec![Some(30), Some(30), Some(30), Some(20), None, None]
    );
    exec(&db, &set("CURRENT_DATE - 45", "NULL")).await;
    assert_eq!(
        period_totals(&st, lb).await,
        vec![Some(50), Some(50), Some(50), Some(40), Some(10), Some(10)]
    );
    exec(&db, &set("NULL", "CURRENT_DATE - 15")).await;
    assert_eq!(
        period_totals(&st, lb).await,
        vec![Some(60), Some(60), Some(60), Some(20), None, None]
    );
    exec(&db, &set("NULL", "NULL")).await;
    assert_eq!(
        period_totals(&st, lb).await,
        vec![Some(80), Some(80), Some(80), Some(40), Some(10), Some(10)]
    );
}

#[tokio::test]
async fn positions_follow_the_ranking_strategy() {
    let (db, st) = setup().await;
    fabricate_user(&db, 101, "user", "").await;
    fabricate_user(&db, 102, "other", "").await;
    fabricate_user(&db, 103, "moderator", "").await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    score(&db, lb, 101, 20, "CURRENT_DATE").await;
    score(&db, lb, 1, 50, "CURRENT_DATE").await;
    score(&db, lb, 102, 20, "CURRENT_DATE").await;
    score(&db, lb, 103, 10, "CURRENT_DATE").await;
    for (strategy, positions) in [
        ("rank", [1, 2, 2, 4]),
        ("dense_rank", [1, 2, 2, 3]),
        ("row_number", [1, 2, 3, 4]),
    ] {
        set_setting(&db.pool, "score_ranking_strategy", 7, strategy).await;
        let mut conn = st.pool.acquire().await.unwrap();
        let s = settings(&st, &mut conn).await;
        cached_view::delete(&mut conn, lb).await.unwrap();
        cached_view::create(&mut conn, &s, lb).await.unwrap();
        let rows = cached_view::scores(&mut conn, lb, "all_time", None, None, 0)
            .await
            .unwrap()
            .unwrap();
        let got: Vec<(i32, i64, i64)> = rows
            .iter()
            .map(|r| (r.id, r.total_score, r.position))
            .collect();
        assert_eq!(
            got,
            vec![
                (1, 50, positions[0]),
                (101, 20, positions[1]),
                (102, 20, positions[2]),
                (103, 10, positions[3])
            ],
            "{strategy}"
        );
        assert_eq!(rows[1].username, "user");
        assert_eq!(rows[1].name.as_deref(), Some("user name"));
    }
}

// spec/lib/directory_integration_spec.rb

async fn directory_score(db: &TestDb, user_id: i32) -> i64 {
    scalar_i64(
        db,
        &format!(
            "SELECT gamification_score::bigint FROM directory_items WHERE user_id = {user_id} AND period_type = 1"
        ),
    )
    .await
}

#[tokio::test]
async fn the_directory_column_sums_the_default_leaderboards_scores() {
    let (db, st) = setup().await;
    fabricate_user(&db, 101, "dir1", "").await;
    fabricate_user(&db, 102, "second", "").await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    score(&db, lb, 101, 10, "CURRENT_DATE - 8").await;
    score(&db, lb, 101, 40, "CURRENT_DATE - 3").await;
    score(&db, lb, 102, 25, "CURRENT_DATE - 5").await;
    score(&db, lb, 102, 5, "CURRENT_DATE - 2").await;
    let refresh = async |st: &AppState| {
        let mut conn = st.pool.acquire().await.unwrap();
        let s = settings(st, &mut conn).await;
        discourse_rs::directory::refresh_period(&mut conn, &s, "all", true)
            .await
            .unwrap();
    };
    refresh(&st).await;
    assert_eq!(directory_score(&db, 101).await, 50);
    assert_eq!(directory_score(&db, 102).await, 30);
    let set = |from: &str, to: &str| {
        format!(
            "UPDATE gamification_leaderboards SET from_date = {from}, to_date = {to} WHERE id = {lb}"
        )
    };
    exec(&db, &set("CURRENT_DATE - 5", "NULL")).await;
    refresh(&st).await;
    assert_eq!(directory_score(&db, 101).await, 40);
    assert_eq!(directory_score(&db, 102).await, 30);
    exec(&db, &set("NULL", "CURRENT_DATE - 4")).await;
    refresh(&st).await;
    assert_eq!(directory_score(&db, 101).await, 10);
    assert_eq!(directory_score(&db, 102).await, 25);
    exec(&db, &set("CURRENT_DATE - 5", "CURRENT_DATE - 3")).await;
    refresh(&st).await;
    assert_eq!(directory_score(&db, 101).await, 40);
    assert_eq!(directory_score(&db, 102).await, 25);
}

// spec/jobs

async fn leaderboard_score_on(db: &TestDb, lb: i64, user_id: i32, date: &str) -> Option<i64> {
    sqlx::query_scalar(&format!(
        "SELECT score::bigint FROM gamification_leaderboard_scores \
         WHERE leaderboard_id = {lb} AND user_id = {user_id} AND date = {date}"
    ))
    .fetch_optional(&db.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn update_scores_for_today_scores_today_and_generates_positions() {
    let (db, st) = setup().await;
    fabricate_user(&db, 101, "user", "").await;
    fabricate_user(&db, 102, "second", "").await;
    // a reply in a topic of the user's, and a topic of theirs
    let topic = fabricate_topic(&db, 101, Some(4), "").await;
    fabricate_post(&db, topic, 101, 2, "").await;
    fabricate_topic(&db, 101, Some(4), "").await;
    let older = fabricate_topic(&db, 102, Some(4), "").await;
    exec(
        &db,
        &format!("UPDATE topics SET created_at = now() - interval '2 days' WHERE id = {older}"),
    )
    .await;
    let lb1 = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    let lb2 = fabricate_leaderboard(&db, "leaderboard2", &[]).await;
    discourse_rs::jobs::run_scheduled(&st, plugin::jobs::UPDATE_SCORES_FOR_TODAY)
        .await
        .unwrap();
    assert_eq!(
        leaderboard_score_on(&db, lb1, 101, "CURRENT_DATE").await,
        Some(12)
    );
    assert_eq!(
        leaderboard_score_on(&db, lb1, 102, "CURRENT_DATE").await,
        None
    );
    assert_eq!(mview_count(&db, lb1).await, 6);
    assert_eq!(mview_count(&db, lb2).await, 6);
    // purges stale views and builds them again
    exec(
        &db,
        &format!(
            "COMMENT ON MATERIALIZED VIEW gamification_leaderboard_cache_{lb1}_daily IS 'old'"
        ),
    )
    .await;
    discourse_rs::jobs::run_scheduled(&st, plugin::jobs::UPDATE_SCORES_FOR_TODAY)
        .await
        .unwrap();
    let mut conn = st.pool.acquire().await.unwrap();
    let s = settings(&st, &mut conn).await;
    assert!(!cached_view::is_stale(&mut conn, &s, lb1).await.unwrap());
    let rows = cached_view::scores(&mut conn, lb1, "all_time", Some(101), None, 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((rows[0].total_score, rows[0].position), (12, 1));
}

#[tokio::test]
async fn update_scores_for_ten_days_scores_the_last_ten_days() {
    let (db, st) = setup().await;
    fabricate_user(&db, 101, "user", "").await;
    fabricate_user(&db, 102, "second", "").await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    let t1 = fabricate_topic(&db, 101, Some(4), "").await;
    let t2 = fabricate_topic(&db, 102, Some(4), "").await;
    exec(
        &db,
        &format!("UPDATE topics SET created_at = now() - interval '8 days' WHERE id = {t1}"),
    )
    .await;
    exec(
        &db,
        &format!("UPDATE topics SET created_at = now() - interval '12 days' WHERE id = {t2}"),
    )
    .await;
    discourse_rs::jobs::run_scheduled(&st, plugin::jobs::UPDATE_SCORES_FOR_TEN_DAYS)
        .await
        .unwrap();
    let sum = |u: i32| {
        format!(
            "SELECT COALESCE(SUM(score), 0)::bigint FROM gamification_leaderboard_scores WHERE leaderboard_id = {lb} AND user_id = {u}"
        )
    };
    assert_eq!(scalar_i64(&db, &sum(101)).await, 5);
    assert_eq!(scalar_i64(&db, &sum(102)).await, 0);
    // nothing while the plugin is off
    exec(&db, "DELETE FROM gamification_leaderboard_scores").await;
    set_setting(&db.pool, "discourse_gamification_enabled", 5, "f").await;
    discourse_rs::jobs::run_scheduled(&st, plugin::jobs::UPDATE_SCORES_FOR_TEN_DAYS)
        .await
        .unwrap();
    assert_eq!(scalar_i64(&db, &sum(101)).await, 0);
}

async fn enqueue(db: &TestDb, name: &str, args: Value) {
    let mut conn = db.pool.acquire().await.unwrap();
    discourse_rs::jobs::enqueue(&mut conn, name, args)
        .await
        .unwrap();
}

#[tokio::test]
async fn position_jobs_generate_refresh_update_and_delete() {
    let (db, st) = setup().await;
    fabricate_user(&db, 101, "user", "").await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    score(&db, lb, 101, 5, "CURRENT_DATE").await;
    // GenerateLeaderboardPositions: from the scores there are
    enqueue(
        &db,
        plugin::jobs::GENERATE_LEADERBOARD_POSITIONS,
        json!({ "leaderboard_id": lb }),
    )
    .await;
    discourse_rs::jobs::drain(&st).await.unwrap();
    assert_eq!(view_rows(&db, lb, "all_time").await, vec![(101, 5, 1)]);
    // RefreshLeaderboardPositions
    score(&db, lb, 101, 10, "CURRENT_DATE - 1").await;
    enqueue(
        &db,
        plugin::jobs::REFRESH_LEADERBOARD_POSITIONS,
        json!({ "leaderboard_id": lb }),
    )
    .await;
    discourse_rs::jobs::drain(&st).await.unwrap();
    assert_eq!(view_rows(&db, lb, "all_time").await, vec![(101, 15, 1)]);
    // UpdateStaleLeaderboardPositions
    exec(
        &db,
        &format!(
            "COMMENT ON MATERIALIZED VIEW gamification_leaderboard_cache_{lb}_all_time IS 'old'"
        ),
    )
    .await;
    enqueue(
        &db,
        plugin::jobs::UPDATE_STALE_LEADERBOARD_POSITIONS,
        json!({}),
    )
    .await;
    discourse_rs::jobs::drain(&st).await.unwrap();
    let mut conn = st.pool.acquire().await.unwrap();
    let s = settings(&st, &mut conn).await;
    assert!(!cached_view::is_stale(&mut conn, &s, lb).await.unwrap());
    assert_eq!(mview_count(&db, lb).await, 6);
    // DeleteLeaderboardPositions, of a leaderboard already gone too
    exec(
        &db,
        &format!("DELETE FROM gamification_leaderboards WHERE id = {lb}"),
    )
    .await;
    enqueue(
        &db,
        plugin::jobs::DELETE_LEADERBOARD_POSITIONS,
        json!({ "leaderboard_id": lb }),
    )
    .await;
    discourse_rs::jobs::drain(&st).await.unwrap();
    assert_eq!(mview_count(&db, lb).await, 0);
}

#[tokio::test]
async fn recalculate_scores_job_recalculates_and_tells_the_admin() {
    let (db, st) = setup().await;
    fabricate_user(&db, 101, "user", "").await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    fabricate_topic(&db, 101, Some(4), "").await;
    let from = st.bus.now().await.unwrap();
    enqueue(
        &db,
        plugin::jobs::RECALCULATE_SCORES,
        json!({ "since": discourse_rs::clock::now().format("%Y-%m-%dT00:00:00.000Z").to_string(), "user_id": 1 }),
    )
    .await;
    discourse_rs::jobs::drain(&st).await.unwrap();
    assert_eq!(view_rows(&db, lb, "all_time").await, vec![(101, 5, 1)]);
    let messages: Vec<Value> = common::bus_messages(&st, from, &["/recalculate_scores"])
        .await
        .into_iter()
        .map(|m| m.data)
        .collect();
    assert_eq!(
        messages,
        vec![json!({ "success": true, "remaining": 5, "user_id": [1] })]
    );
}

// spec/requests/gamification_leaderboard_controller_spec.rb

/// The controller spec's world: a group with current_user (user1), user2,
/// a staged, an anonymized, a suspended and a once suspended user, each
/// with a visit two days ago, and four leaderboards.
async fn leaderboard_world(db: &TestDb, st: &AppState) -> [i64; 4] {
    exec(db, "DELETE FROM user_visits").await;
    exec(
        db,
        "INSERT INTO groups (id, name, created_at, updated_at) VALUES (100, 'testers', now(), now())",
    )
    .await;
    exec(
        db,
        "INSERT INTO group_users (group_id, user_id, created_at, updated_at) VALUES (100, 3, now(), now())",
    )
    .await;
    fabricate_user(db, 102, "staged", ", 'staged', true").await;
    fabricate_user(db, 103, "anon", "").await;
    exec(
        db,
        "UPDATE user_emails SET email = 'john@anonymized.invalid' WHERE user_id = 103",
    )
    .await;
    fabricate_user(
        db,
        104,
        "suspended",
        ", 'suspended_at', now(), 'suspended_till', now() + interval '5 days'",
    )
    .await;
    fabricate_user(
        db,
        105,
        "wassuspended",
        ", 'suspended_at', now(), 'suspended_till', now() - interval '5 days'",
    )
    .await;
    for user in [3, 4, 102, 103, 104, 105] {
        exec(
            db,
            &format!(
                "INSERT INTO user_visits (user_id, visited_at) VALUES ({user}, CURRENT_DATE - 2)"
            ),
        )
        .await;
    }
    // a topic of current_user's today
    fabricate_topic(db, 3, Some(4), "").await;
    let lb = fabricate_leaderboard(db, "test", &[]).await;
    let lb2 = fabricate_leaderboard(
        db,
        "test_2",
        &[
            ("from_date", "CURRENT_DATE - 3"),
            ("to_date", "CURRENT_DATE - 1"),
        ],
    )
    .await;
    let lb3 = fabricate_leaderboard(
        db,
        "test_3",
        &[
            ("included_groups_ids", "'{100}'"),
            ("visible_to_groups_ids", "'{100}'"),
        ],
    )
    .await;
    let lb4 = fabricate_leaderboard(db, "test_4", &[("default_period", "5")]).await;
    let mut conn = st.pool.acquire().await.unwrap();
    let s = settings(st, &mut conn).await;
    let since = Since::Time(discourse_rs::clock::now_naive() - chrono::Duration::days(10));
    scorables::calculate_all(&mut conn, &s, &since)
        .await
        .unwrap();
    [lb, lb2, lb3, lb4]
}

async fn create_views(st: &AppState, lb: i64) {
    let mut conn = st.pool.acquire().await.unwrap();
    let s = settings(st, &mut conn).await;
    cached_view::create(&mut conn, &s, lb).await.unwrap();
}

#[tokio::test]
async fn respond_returns_users_their_scores_and_the_viewers_position() {
    let (db, st) = setup().await;
    let [lb, lb2, _, _] = leaderboard_world(&db, &st).await;
    create_views(&st, lb).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, doc) = user1.get(&format!("/leaderboard/{lb}.json")).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    let users = doc["users"].as_array().unwrap();
    assert_eq!(users[0]["username"], "user1");
    assert_eq!(users[0]["total_score"], user_score(&st, 3).await);
    assert_eq!(users[0]["total_score"], 6);
    assert!(
        users[0]["avatar_template"]
            .as_str()
            .unwrap()
            .starts_with('/')
    );
    assert_eq!(doc["personal"]["position"], 1);
    assert_eq!(doc["personal"]["user"]["username"], "user1");
    // no admin scoring configuration
    let keys: Vec<&String> = doc["leaderboard"].as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        vec![
            "id",
            "name",
            "created_by_id",
            "from_date",
            "to_date",
            "visible_to_groups_ids",
            "included_groups_ids",
            "excluded_groups_ids",
            "default_period",
            "updated_at",
            "period_filter_disabled"
        ]
    );
    // excludes staged, anonymized and currently suspended users
    let ids: Vec<i64> = users.iter().map(|u| u["id"].as_i64().unwrap()).collect();
    for id in [102, 103, 104] {
        assert!(!ids.contains(&id), "{ids:?}");
    }
    assert!(ids.contains(&105));
    // respects user_limit
    let (_, doc) = user1
        .get(&format!("/leaderboard/{lb}.json?user_limit=1"))
        .await;
    assert_eq!(doc["users"].as_array().unwrap().len(), 1);
    // only the date range's scores
    create_views(&st, lb2).await;
    let (_, doc) = user1.get(&format!("/leaderboard/{lb2}.json")).await;
    assert_eq!(doc["users"][0]["username"], "user1");
    assert_eq!(doc["users"][0]["total_score"], 1);
    // The first leaderboard without an id.
    let (status, doc) = user1.get("/leaderboard.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["leaderboard"]["id"], lb);
    // Anonymous: no personal position.
    let (_, doc) = Client::anonymous(st.clone())
        .get(&format!("/leaderboard/{lb}.json"))
        .await;
    assert_eq!(doc["personal"], json!({}));
}

#[tokio::test]
async fn respond_is_accepted_with_the_reason_while_positions_generate() {
    let (db, st) = setup().await;
    let [lb, ..] = leaderboard_world(&db, &st).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    // (Logging in serializes the user, whose score enqueues it too.)
    let before = job_args(&db, "Jobs::GenerateLeaderboardPositions")
        .await
        .len();
    let (status, doc) = user1.get(&format!("/leaderboard/{lb}.json")).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        doc["reason"],
        "We are generating your leaderboard. Try again in a few minutes."
    );
    assert_eq!(doc["users"], json!([]));
    assert_eq!(doc["name"], "test");
    assert_eq!(
        job_args(&db, "Jobs::GenerateLeaderboardPositions")
            .await
            .len(),
        before + 1
    );
    // The page shows the reason and Refresh.
    let (status, html) = user1.page(&format!("/leaderboard/{lb}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains("<div class=\"leaderboard__not-ready\"><p>We are generating"),
        "{html}"
    );
}

#[tokio::test]
async fn respond_scopes_users_and_visibility_to_groups() {
    let (db, st) = setup().await;
    let [_, _, lb3, _] = leaderboard_world(&db, &st).await;
    create_views(&st, lb3).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, doc) = user1.get(&format!("/leaderboard/{lb3}.json")).await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<i64> = doc["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, vec![3]);
    // not in a visible group: not found
    let mut user2 = Client::logged_in(st.clone(), "user2").await;
    assert_eq!(
        user2.get(&format!("/leaderboard/{lb3}.json")).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, body) = user2.page(&format!("/leaderboard/{lb3}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // admins see all leaderboards
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    assert_eq!(
        admin.get(&format!("/leaderboard/{lb3}.json")).await.0,
        StatusCode::OK
    );
    // an unknown one, or the plugin off
    assert_eq!(
        admin.get("/leaderboard/99999.json").await.0,
        StatusCode::NOT_FOUND
    );
    set_setting(&db.pool, "discourse_gamification_enabled", 5, "f").await;
    assert_eq!(
        admin.get(&format!("/leaderboard/{lb3}.json")).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn respond_uses_the_default_period() {
    let (db, st) = setup().await;
    let [lb, _, _, lb4] = leaderboard_world(&db, &st).await;
    create_views(&st, lb).await;
    create_views(&st, lb4).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let total = async |c: &mut Client, path: String| {
        c.get(&path).await.1["users"][0]["total_score"].clone()
    };
    let regular = total(&mut user1, format!("/leaderboard/{lb}.json")).await;
    let daily = total(&mut user1, format!("/leaderboard/{lb}.json?period=daily")).await;
    let default = total(&mut user1, format!("/leaderboard/{lb4}.json")).await;
    assert_eq!(default, daily);
    assert_ne!(default, regular);
    // `all` is all time
    let all = total(&mut user1, format!("/leaderboard/{lb4}.json?period=all")).await;
    assert_eq!(all, regular);
}

#[tokio::test]
async fn the_leaderboard_page_draws_the_podium_ranking_and_controls() {
    let (db, st) = setup().await;
    let [lb, ..] = leaderboard_world(&db, &st).await;
    create_views(&st, lb).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, html) = user1.page(&format!("/leaderboard/{lb}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains("<h1 class=\"page__title\">test</h1>"),
        "{html}"
    );
    assert!(html.contains("<div class=\"winner -position1\">"));
    assert!(html.contains("data-user-card=\"user1\""));
    assert!(html.contains("<div class=\"user -self\"><div class=\"user__rank\">1</div><div class=\"user__name\">You</div><div class=\"user__score\">6</div></div>"));
    assert!(html.contains("leaderboard__period-chooser"));
    assert!(html.contains("leaderboard-info-template"));
    assert!(html.contains("gamification.js"));
    // no settings link for members
    assert!(!html.contains("leaderboard__settings"));
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (_, html) = admin.page(&format!("/leaderboard/{lb}?period=all")).await;
    assert!(
        html.contains("<a class=\"leaderboard__settings\" href=\"/admin/plugins/gamification\">")
    );
    // a later page: its rows only (none here)
    let (_, html) = admin.page(&format!("/leaderboard/{lb}?page=1")).await;
    assert!(
        html.contains("<div class=\"leaderboard-page-rows\"></div>"),
        "{html}"
    );
}

// spec/requests/admin_gamification_leaderboard_controller_spec.rb

async fn job_args(db: &TestDb, name: &str) -> Vec<Value> {
    sqlx::query_scalar(&format!(
        "SELECT args FROM discourse_rs.jobs WHERE name = '{name}' ORDER BY id"
    ))
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn admins_create_update_show_and_destroy_leaderboards() {
    let (db, st) = setup().await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, doc) = admin
        .json(
            Method::POST,
            "/admin/plugins/gamification/leaderboard.json",
            "name=Test&created_by_id=1",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc["name"], "Test");
    assert_eq!(doc["created_by_id"], 1);
    assert_eq!(doc["score_overrides"], Value::Null);
    let id = doc["id"].as_i64().unwrap();
    assert_eq!(
        job_args(&db, plugin::jobs::RECALCULATE_LEADERBOARD_SCORES).await,
        vec![json!({ "leaderboard_id": id })]
    );
    // the reserved name, a missing param
    let (status, doc) = admin
        .json(
            Method::POST,
            "/admin/plugins/gamification/leaderboard.json",
            "name=new&created_by_id=1",
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(doc["errors"], json!(["Name new is reserved."]));
    assert_eq!(
        admin
            .json(
                Method::POST,
                "/admin/plugins/gamification/leaderboard.json",
                "name=x"
            )
            .await
            .0,
        StatusCode::BAD_REQUEST
    );

    // update: positions refreshed
    let (status, doc) = admin
        .json(
            Method::PUT,
            &format!("/admin/plugins/gamification/leaderboard/{id}.json"),
            "name=New+Name",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc, json!({ "success": "OK" }));
    let name: String = sqlx::query_scalar(&format!(
        "SELECT name FROM gamification_leaderboards WHERE id = {id}"
    ))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(name, "New Name");
    assert_eq!(
        job_args(&db, plugin::jobs::REFRESH_LEADERBOARD_POSITIONS).await,
        vec![json!({ "leaderboard_id": id })]
    );
    // scoring changed: scores recalculated
    let (status, _) = admin
        .json(
            Method::PUT,
            &format!("/admin/plugins/gamification/leaderboard/{id}.json"),
            "name=New+Name&score_overrides[like_received]=10&scorable_category_ids[]=123&included_groups_ids[]=1&default_period=3&period_filter_disabled=true&from_date=2026-01-02",
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        job_args(&db, plugin::jobs::RECALCULATE_LEADERBOARD_SCORES)
            .await
            .len(),
        2
    );
    // invalid overrides
    let (status, doc) = admin
        .json(
            Method::PUT,
            &format!("/admin/plugins/gamification/leaderboard/{id}.json"),
            "name=New+Name&score_overrides[bogus]=-1",
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        doc["errors"],
        json!([
            "Score overrides contains invalid scorable key: bogus",
            "Score overrides values must be non-negative integers"
        ])
    );

    // show: the admin scoring configuration
    let (status, doc) = admin
        .get(&format!(
            "/admin/plugins/discourse-gamification/leaderboards/{id}.json"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    let lb = &doc["leaderboard"];
    assert_eq!(lb["score_overrides"], json!({ "like_received": 10 }));
    assert_eq!(lb["scorable_category_ids"], json!([123]));
    assert_eq!(lb["included_groups_ids"], json!([1]));
    assert_eq!(lb["default_period"], 3);
    assert_eq!(lb["period_filter_disabled"], true);
    assert_eq!(lb["from_date"], "2026-01-02");

    // index
    let (status, doc) = admin.get("/admin/plugins/gamification.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["gamification_recalculate_scores_remaining"], 5);
    assert_eq!(doc["gamification_leaderboards"][0]["id"], id);
    assert!(doc["gamification_groups"].as_array().unwrap().len() > 5);

    // destroy
    let (status, _) = admin
        .json(
            Method::DELETE,
            &format!("/admin/plugins/gamification/leaderboard/{id}.json"),
            "",
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        scalar_i64(
            &db,
            &format!("SELECT COUNT(*) FROM gamification_leaderboards WHERE id = {id}")
        )
        .await,
        0
    );
    assert_eq!(
        job_args(&db, plugin::jobs::DELETE_LEADERBOARD_POSITIONS).await,
        vec![json!({ "leaderboard_id": id })]
    );
}

#[tokio::test]
async fn admin_endpoints_refuse_moderators_members_and_a_disabled_plugin() {
    let (db, st) = setup().await;
    exec(&db, "UPDATE users SET moderator = TRUE WHERE id = 4").await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    assert_eq!(
        user1.get("/admin/plugins/gamification.json").await.0,
        StatusCode::NOT_FOUND
    );
    let mut moddy = Client::logged_in(st.clone(), "user2").await;
    assert_eq!(
        moddy.get("/admin/plugins/gamification.json").await.0,
        StatusCode::FORBIDDEN
    );
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    set_setting(&db.pool, "discourse_gamification_enabled", 5, "f").await;
    assert_eq!(
        admin.get("/admin/plugins/gamification.json").await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn recalculate_scores_enqueues_from_a_past_date() {
    let (db, st) = setup().await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let ten_days_ago = discourse_rs::clock::now().date_naive() - chrono::Duration::days(10);
    let (status, _) = admin
        .json(
            Method::PUT,
            "/admin/plugins/gamification/recalculate-scores.json",
            &format!("from_date={ten_days_ago}"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let args = job_args(&db, plugin::jobs::RECALCULATE_SCORES).await;
    assert_eq!(args.len(), 1);
    assert_eq!(args[0]["since"], format!("{ten_days_ago}T00:00:00.000Z"));
    assert_eq!(args[0]["user_id"], 1);
    let tomorrow = discourse_rs::clock::now().date_naive() + chrono::Duration::days(1);
    let (status, _) = admin
        .json(
            Method::PUT,
            "/admin/plugins/gamification/recalculate-scores.json",
            &format!("from_date={tomorrow}"),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        job_args(&db, plugin::jobs::RECALCULATE_SCORES).await.len(),
        1
    );
}

// spec/requests/admin_gamification_score_event_controller_spec.rb

#[tokio::test]
async fn score_events_are_listed_created_and_updated() {
    let (db, st) = setup().await;
    fabricate_user(&db, 101, "another", "").await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    let mut ids = Vec::new();
    for (user, date, points) in [
        (1, "CURRENT_DATE", 7),
        (1, "CURRENT_DATE - 1", 17),
        (101, "CURRENT_DATE - 1", 27),
    ] {
        ids.push(
            scalar_i64(
                &db,
                &format!(
                    "INSERT INTO gamification_score_events (user_id, date, points, created_at, updated_at) \
                     VALUES ({user}, {date}, {points}, now(), now()) RETURNING id"
                ),
            )
            .await,
        );
    }
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, doc) = admin
        .get("/admin/plugins/gamification/score_events.json")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["events"].as_array().unwrap().len(), 3);
    assert_eq!(doc["events"][0]["points"], 7);
    let keys: Vec<&String> = doc["events"][0].as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        vec![
            "id",
            "user_id",
            "date",
            "points",
            "description",
            "created_at",
            "updated_at"
        ]
    );
    let today = discourse_rs::clock::now().date_naive();
    let (_, doc) = admin
        .get(&format!(
            "/admin/plugins/gamification/score_events.json?date={today}"
        ))
        .await;
    assert_eq!(doc["events"].as_array().unwrap().len(), 1);
    assert_eq!(doc["events"][0]["points"], 7);
    let (_, doc) = admin
        .get("/admin/plugins/gamification/score_events.json?user_id=1")
        .await;
    let points: i64 = doc["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["points"].as_i64().unwrap())
        .sum();
    assert_eq!(points, 24);
    let (_, doc) = admin
        .get(&format!(
            "/admin/plugins/gamification/score_events.json?user_id=101&date={today}"
        ))
        .await;
    assert_eq!(doc["events"].as_array().unwrap().len(), 0);
    let (_, doc) = admin
        .get(&format!(
            "/admin/plugins/gamification/score_events.json?id={}",
            ids[2]
        ))
        .await;
    assert_eq!(doc["events"][0]["id"], ids[2]);

    let total = async |st: &AppState| {
        let mut conn = st.pool.acquire().await.unwrap();
        let s = settings(st, &mut conn).await;
        let since = Since::Time(
            (discourse_rs::clock::now_naive() - chrono::Duration::days(10))
                .date()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        );
        scorables::calculate_all(&mut conn, &s, &since)
            .await
            .unwrap();
        sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COALESCE(SUM(score), 0)::bigint FROM gamification_leaderboard_scores WHERE user_id = 101 AND leaderboard_id = {lb}"
        ))
        .fetch_one(&st.pool)
        .await
        .unwrap()
    };
    // a created event affects the user's score
    let (status, doc) = admin
        .json(
            Method::POST,
            "/admin/plugins/gamification/score_events.json",
            &format!("points=10&user_id=101&date={today}"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc["points"], 10);
    assert_eq!(total(&st).await, 37);
    // an updated one too
    let (status, _) = admin
        .json(
            Method::PUT,
            "/admin/plugins/gamification/score_events.json",
            &format!("id={}&points=13", ids[2]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let new_id = doc["id"].as_i64().unwrap();
    exec(
        &db,
        &format!("DELETE FROM gamification_score_events WHERE id = {new_id}"),
    )
    .await;
    assert_eq!(total(&st).await, 13);
    // a user that doesn't exist
    let (status, doc) = admin
        .json(
            Method::POST,
            "/admin/plugins/gamification/score_events.json",
            &format!("points=10&user_id=99999&date={today}"),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(doc["errors"], json!(["User must exist"]));
}

// spec/plugin_spec.rb and the score connectors

#[tokio::test]
async fn the_card_profile_and_site_carry_the_score() {
    let (db, st) = setup().await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    score(&db, lb, 4, 1234, "CURRENT_DATE").await;
    create_views(&st, lb).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (_, card) = user1.get("/u/user2/card.json").await;
    assert_eq!(card["user"]["gamification_score"], 1234, "{card}");
    let (_, show) = user1.get("/u/user2.json").await;
    assert_eq!(show["user"]["gamification_score"], 1234);
    let (_, site) = user1.get("/site.json").await;
    assert_eq!(site["default_gamification_leaderboard_id"], lb);
    // the card's and profile's connectors
    let (status, html) = user1.page("/u/user2").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains(&format!(
            "<div><dt>Cheers</dt><dd><span class=\"gamification-score\"><a href=\"/leaderboard/{lb}\" class=\"gamification-score__link\">1,234</a></span></dd></div>"
        )),
        "{html}"
    );
    // plugin off: no keys
    set_setting(&db.pool, "discourse_gamification_enabled", 5, "f").await;
    let (_, card) = user1.get("/u/user2/card.json").await;
    assert!(card["user"].get("gamification_score").is_none());
    let (_, site) = user1.get("/site.json").await;
    assert!(site.get("default_gamification_leaderboard_id").is_none());
}

#[tokio::test]
async fn changing_the_ranking_strategy_regenerates_positions() {
    let (db, st) = setup().await;
    let mut conn = st.pool.acquire().await.unwrap();
    let s = settings(&st, &mut conn).await;
    discourse_rs::plugins::site_setting_changed(
        &mut conn,
        &st.site_setting_defs,
        &s,
        "score_ranking_strategy",
        &discourse_rs::site_settings::Value::Str("dense_rank".into()),
        &discourse_rs::site_settings::Value::Str("row_number".into()),
    )
    .await
    .unwrap();
    assert_eq!(
        job_args(&db, plugin::jobs::REGENERATE_LEADERBOARD_POSITIONS).await,
        vec![json!({})]
    );
}

#[tokio::test]
async fn destroying_a_user_takes_their_scores() {
    let (db, st) = setup().await;
    fabricate_user(&db, 101, "gone", "").await;
    let lb = fabricate_leaderboard(&db, "leaderboard1", &[]).await;
    score(&db, lb, 101, 5, "CURRENT_DATE").await;
    let mut conn = st.pool.acquire().await.unwrap();
    discourse_rs::plugins::user_destroying(&mut conn, 101)
        .await
        .unwrap();
    assert_eq!(
        scalar_i64(
            &db,
            "SELECT COUNT(*) FROM gamification_leaderboard_scores WHERE user_id = 101"
        )
        .await,
        0
    );
}

// The admin pages.

#[tokio::test]
async fn the_admin_pages_list_and_edit_leaderboards() {
    let (db, st) = setup().await;
    let lb = fabricate_leaderboard(&db, "Weekly race", &[("default_period", "4")]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin
        .page("/admin/plugins/discourse-gamification/leaderboards")
        .await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains(&format!("<tr id=\"leaderboard-admin__row-{lb}\"><td><a href=\"/leaderboard/{lb}\">Weekly race</a></td><td>Weekly</td>")), "{html}");
    assert!(html.contains("leaderboard-admin__btn-new"));
    assert!(html.contains("leaderboard-admin__btn-recalculate"));
    assert!(html.contains("recalculate-scores-template"));
    assert!(html.contains("5 daily recalculations left"));
    assert!(html.contains("d-nav-submenu__tabs"));
    let (status, html) = admin
        .page(&format!(
            "/admin/plugins/discourse-gamification/leaderboards/{lb}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains(&format!(
            "<form data-leaderboard-id=\"{lb}\" class=\"form-kit edit-create-leaderboard-form\""
        )),
        "{html}"
    );
    assert!(html.contains("name=\"score_override_chat_reaction_received\""));
    assert!(html.contains("name=\"included_groups_ids\""));
    // The plugin's settings page carries its configuration nav.
    let (status, html) = admin.page("/admin/plugins/discourse-gamification").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("/admin/plugins/discourse-gamification/leaderboards"));
    let (status, _) = admin.page("/admin/plugins/gamification").await;
    assert_eq!(status, StatusCode::OK);
    // Without leaderboards: the empty state.
    exec(&db, "DELETE FROM gamification_leaderboards").await;
    let (_, html) = admin
        .page("/admin/plugins/discourse-gamification/leaderboards")
        .await;
    assert!(html.contains("No leaderboards created yet."));
    assert!(!html.contains("leaderboard-admin__btn-recalculate"));
}
