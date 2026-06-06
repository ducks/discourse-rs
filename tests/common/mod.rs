//! Shared test helpers: DB pool setup, table truncation between tests,
//! and fixture constructors for the common models.
//!
//! ## Usage
//!
//! Each integration test file does `mod common;` and then `common::setup()`
//! at the top of every test. `setup()` returns a [`TestCtx`] holding a
//! pooled connection plus a `Drop` impl that truncates all tables on the
//! way out — so tests stay isolated even when they panic mid-run.
//!
//! Tests are expected to run serially (`--test-threads=1`) because they
//! share one test database. The `nix-shell` exports a `TEST_DATABASE_URL`
//! pointing at `discourse_rs_test`; tests will not touch the dev DB.

use diesel::pg::PgConnection;
use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager, PooledConnection};
use std::env;
use std::sync::OnceLock;

use discourse_rs::models::{Category, NewCategory, NewPost, NewTopic, NewUser, Post, Topic, User};
use discourse_rs::schema::{categories, posts, topics, users};
use discourse_rs::DbPool;

/// Build (or reuse) the pool. r2d2 pools are cheap to clone, so one
/// process-wide pool is fine — each test checks out its own connection.
fn pool() -> &'static DbPool {
    static POOL: OnceLock<DbPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let url = env::var("TEST_DATABASE_URL")
            .expect("TEST_DATABASE_URL must be set (run `db_test_setup` in nix-shell)");
        let manager = ConnectionManager::<PgConnection>::new(url);
        r2d2::Pool::builder()
            .max_size(4)
            .build(manager)
            .expect("Failed to build test DB pool")
    })
}

/// Context handed to each test. Holds a pooled connection; truncates on Drop.
pub struct TestCtx {
    pub conn: PooledConnection<ConnectionManager<PgConnection>>,
}

impl TestCtx {
    // Route-level tests need a pool to hand to actix's `web::Data`.
    // Not yet used in any test, but kept here so route tests don't have
    // to reach into the private module to get one.
    #[allow(dead_code)]
    pub fn pool(&self) -> DbPool {
        pool().clone()
    }
}

// Listed explicitly (rather than relying on CASCADE alone) so adding a new
// table requires touching this list — easier than discovering leftover rows
// in a flaky test.
const TRUNCATE_SQL: &str = "TRUNCATE TABLE \
    notifications, \
    moderation_actions, \
    post_likes, \
    posts, \
    topic_views, \
    topics, \
    categories, \
    site_settings, \
    backie_tasks, \
    user_stats, \
    user_suspensions, \
    users \
    RESTART IDENTITY CASCADE";

impl Drop for TestCtx {
    fn drop(&mut self) {
        let _ = diesel::sql_query(TRUNCATE_SQL).execute(&mut self.conn);
    }
}

/// Entry point for tests. Truncates first (in case a prior test crashed
/// without running Drop) and returns a connection ready for use.
pub fn setup() -> TestCtx {
    let mut conn = pool().get().expect("Failed to check out test connection");
    diesel::sql_query(TRUNCATE_SQL)
        .execute(&mut conn)
        .expect("Failed to truncate test database");
    TestCtx { conn }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixture helpers
//
// Each `create_*` takes the connection and an optional config struct. The
// goal is "minimum-viable record" with sensible defaults — callers override
// only what the test cares about.

/// Defaults for [`create_user`]. Override fields per test as needed.
pub struct UserOpts {
    pub username: String,
    pub email: String,
    pub admin: bool,
    pub moderator: bool,
    pub trust_level: i32,
}

impl Default for UserOpts {
    fn default() -> Self {
        Self {
            username: format!("test_user_{}", random_suffix()),
            email: format!("user_{}@test.example.com", random_suffix()),
            admin: false,
            moderator: false,
            trust_level: 0,
        }
    }
}

pub fn create_user(conn: &mut PgConnection, opts: UserOpts) -> User {
    let new = NewUser {
        username: opts.username,
        email: opts.email,
        password_hash: "test_hash".to_string(),
        admin: opts.admin,
        moderator: opts.moderator,
        trust_level: opts.trust_level,
    };
    let user: User = diesel::insert_into(users::table)
        .values(&new)
        .returning(User::as_returning())
        .get_result(conn)
        .expect("create_user failed");
    // Mirror the route: every user gets a stats row. Tests that exercise
    // post/topic creation rely on this row already existing.
    discourse_rs::services::user_stats::ensure_for(conn, user.id)
        .expect("ensure_for failed");
    user
}

/// Defaults for [`create_category`].
pub struct CategoryOpts {
    pub name: String,
    pub slug: String,
}

impl Default for CategoryOpts {
    fn default() -> Self {
        let s = random_suffix();
        Self {
            name: format!("Test Category {s}"),
            slug: format!("test-category-{s}"),
        }
    }
}

pub fn create_category(conn: &mut PgConnection, opts: CategoryOpts) -> Category {
    let new = NewCategory {
        name: opts.name,
        slug: opts.slug,
        description: None,
        color: "0088CC".to_string(),
        position: 0,
    };
    diesel::insert_into(categories::table)
        .values(&new)
        .returning(Category::as_returning())
        .get_result(conn)
        .expect("create_category failed")
}

/// Defaults for [`create_topic`]. Requires a user_id; everything else has
/// sensible defaults.
pub struct TopicOpts {
    pub user_id: i32,
    pub title: String,
    pub slug: String,
    pub category_id: Option<i32>,
}

impl TopicOpts {
    pub fn for_user(user_id: i32) -> Self {
        let s = random_suffix();
        Self {
            user_id,
            title: format!("Test Topic {s}"),
            slug: format!("test-topic-{s}"),
            category_id: None,
        }
    }
}

pub fn create_topic(conn: &mut PgConnection, opts: TopicOpts) -> Topic {
    let new = NewTopic {
        title: opts.title,
        slug: opts.slug,
        user_id: opts.user_id,
        category_id: opts.category_id,
        views: 0,
        posts_count: 0,
        pinned: false,
        closed: false,
    };
    diesel::insert_into(topics::table)
        .values(&new)
        .returning(Topic::as_returning())
        .get_result(conn)
        .expect("create_topic failed")
}

/// Defaults for [`create_post`]. Requires a topic_id and user_id; the rest
/// has sensible defaults. post_number is caller-managed; tests creating
/// multiple posts in one topic should pass distinct numbers.
pub struct PostOpts {
    pub topic_id: i32,
    pub user_id: i32,
    pub post_number: i32,
    pub raw: String,
}

impl PostOpts {
    pub fn for_topic(topic_id: i32, user_id: i32) -> Self {
        Self {
            topic_id,
            user_id,
            post_number: 1,
            raw: "Test post content".to_string(),
        }
    }
}

pub fn create_post(conn: &mut PgConnection, opts: PostOpts) -> Post {
    let new = NewPost {
        topic_id: opts.topic_id,
        user_id: opts.user_id,
        post_number: opts.post_number,
        raw: opts.raw.clone(),
        cooked: opts.raw,
        reply_to_post_number: None,
    };
    diesel::insert_into(posts::table)
        .values(&new)
        .returning(Post::as_returning())
        .get_result(conn)
        .expect("create_post failed")
}

// ─────────────────────────────────────────────────────────────────────────────
// Route-level harness
//
// For tests that want to exercise the HTTP contract (status codes, JSON
// bodies, header handling), build an actix `App` via `test_app()` and use
// `auth_header_for(&user)` to mint a Bearer token. The app shares the same
// test pool, so fixture data inserted via TestCtx is visible to handlers.

#[allow(dead_code)]
pub fn auth_header_for(user: &User) -> (&'static str, String) {
    let token = discourse_rs::auth::generate_token(user.id, user.username.clone())
        .expect("generate_token failed");
    ("Authorization", format!("Bearer {token}"))
}

/// Build an actix `App` configured with the test pool and the same
/// `/api` route tree as production. Tests call this inline:
///
/// ```ignore
/// let app = actix_web::test::init_service(common::test_app_factory()).await;
/// ```
///
/// We return the `App` rather than the initialized `Service` so the test
/// owns the `init_service().await` step; this keeps the helper's return
/// type expressible without naming actix-http types.
#[allow(dead_code)]
pub fn test_app_factory()
-> actix_web::App<
    impl actix_web::dev::ServiceFactory<
        actix_web::dev::ServiceRequest,
        Config = (),
        Response = actix_web::dev::ServiceResponse,
        Error = actix_web::Error,
        InitError = (),
    >,
> {
    use actix_web::{App, web};
    let pool = pool().clone();
    App::new()
        .app_data(web::Data::new(pool))
        .service(web::scope("/api").configure(discourse_rs::routes::config))
}

// ─────────────────────────────────────────────────────────────────────────────
// Misc helpers

/// Small random string for unique usernames/slugs across tests within one
/// run. Not crypto-random; just enough to avoid UNIQUE collisions when
/// multiple fixtures get created in sequence within one test.
fn random_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{n:08x}")
}
