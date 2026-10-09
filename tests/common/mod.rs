// Each test crate uses a different subset of these helpers.
#![allow(dead_code)]

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use discourse_rs::AppState;
use discourse_rs::config::{Config, GlobalSettings, RailsEnv};
use discourse_rs::i18n::I18n;
use discourse_rs::site_settings::Definitions;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Connection, Executor, PgConnection, PgPool};

/// The template database, built by `make db-test` from the vendored schema
/// and seeds. Tests never connect to it directly: Postgres refuses to clone
/// a template that has open connections.
pub fn test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must be set; run tests inside nix-shell after `make db-test`")
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A private copy of the template database, dropped with the value (also on
/// panic), so tests can write freely and run in parallel.
pub struct TestDb {
    pub pool: PgPool,
    name: String,
    admin: PgConnectOptions,
}

impl TestDb {
    pub async fn new() -> TestDb {
        TestDb::create(&[]).await
    }

    /// A copy whose database clock can be pinned (`pin_clock`) or shifted
    /// (`shift_clock`): `now()` and `clock_timestamp()` resolve to functions
    /// in a `test_clock` schema, searched before pg_catalog, that answer the
    /// pinned time when there is one, else the real one shifted.
    pub async fn with_clock() -> TestDb {
        TestDb::create(&[
            "CREATE SCHEMA test_clock",
            "CREATE TABLE test_clock.pinned (at timestamptz NOT NULL)",
            "CREATE TABLE test_clock.shifted (by interval NOT NULL)",
            "CREATE FUNCTION test_clock.now() RETURNS timestamptz LANGUAGE sql STABLE AS \
             'SELECT COALESCE((SELECT at FROM test_clock.pinned LIMIT 1), \
                              pg_catalog.now() + COALESCE((SELECT by FROM test_clock.shifted LIMIT 1), interval ''0''))'",
            "CREATE FUNCTION test_clock.clock_timestamp() RETURNS timestamptz LANGUAGE sql VOLATILE AS \
             'SELECT COALESCE((SELECT at FROM test_clock.pinned LIMIT 1), \
                              pg_catalog.clock_timestamp() + COALESCE((SELECT by FROM test_clock.shifted LIMIT 1), interval ''0''))'",
            r#"ALTER DATABASE "{db}" SET search_path = test_clock, pg_catalog, "$user", public"#,
        ])
        .await
    }

    /// Clones the template, runs `setup` on the copy (`{db}` is its name)
    /// before anything else connects, so database settings apply to every
    /// pooled connection, then opens the pool and creates the port's own
    /// schema.
    async fn create(setup: &[&str]) -> TestDb {
        let template_opts = PgConnectOptions::from_str(&test_database_url())
            .expect("TEST_DATABASE_URL is not a valid postgres URL");
        let template = template_opts
            .get_database()
            .expect("TEST_DATABASE_URL has no database name")
            .to_string();
        let name = format!(
            "{template}_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let admin = template_opts.clone().database("postgres");

        let mut conn = PgConnection::connect_with(&admin)
            .await
            .expect("connecting to the postgres database");
        conn.execute(format!(r#"CREATE DATABASE "{name}" TEMPLATE "{template}""#).as_str())
            .await
            .unwrap_or_else(|e| panic!("cloning {template}; run `make db-test` first: {e}"));
        conn.close().await.ok();

        let opts = template_opts.database(&name);
        if !setup.is_empty() {
            let mut conn = PgConnection::connect_with(&opts)
                .await
                .expect("connecting to set up the test database");
            for sql in setup {
                let sql = sql.replace("{db}", &name);
                conn.execute(sql.as_str())
                    .await
                    .unwrap_or_else(|e| panic!("setting up the test database ({sql}): {e}"));
            }
            conn.close().await.ok();
        }

        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await
            .expect("connecting to the cloned test database");
        discourse_rs::owned_schema::migrate(&pool)
            .await
            .expect("creating the job queue");

        TestDb { pool, name, admin }
    }

    /// Pins the port's clock and the database's (None releases both).
    pub async fn pin_clock(&self, at: Option<chrono::DateTime<chrono::Utc>>) {
        discourse_rs::clock::pin(at);
        sqlx::query("DELETE FROM test_clock.pinned")
            .execute(&self.pool)
            .await
            .expect("unpinning the database clock");
        if let Some(at) = at {
            sqlx::query("INSERT INTO test_clock.pinned (at) VALUES ($1)")
                .bind(at)
                .execute(&self.pool)
                .await
                .expect("pinning the database clock");
        }
    }

    /// Shifts both clocks by `by`, still running: a write case replays as
    /// if from when Rails recorded it, with time passing as it did there.
    pub async fn shift_clock(&self, by: chrono::Duration) {
        discourse_rs::clock::shift(Some(by));
        sqlx::query("DELETE FROM test_clock.shifted")
            .execute(&self.pool)
            .await
            .expect("unshifting the database clock");
        sqlx::query("INSERT INTO test_clock.shifted (by) VALUES (make_interval(secs => $1))")
            .bind(by.num_microseconds().unwrap_or(0) as f64 / 1_000_000.0)
            .execute(&self.pool)
            .await
            .expect("shifting the database clock");
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // Drop can't be async and may run inside the test's runtime, so the
        // cleanup gets its own thread and runtime.
        //
        // DROP DATABASE WITH (FORCE) waits for every backend to take a
        // signal, and a backend still authenticating a new connection only
        // takes it once its client answers. On a multi-thread runtime that
        // client may be a task on this very worker: blocking the worker
        // would deadlock both until authentication_timeout. block_in_place
        // hands the worker's other tasks to another thread first.
        let name = self.name.clone();
        let admin = self.admin.clone();
        let multi_thread = tokio::runtime::Handle::try_current()
            .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread);
        let cleanup = move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("building cleanup runtime")
                .block_on(async move {
                    let mut conn = PgConnection::connect_with(&admin).await?;
                    conn.execute(
                        format!(r#"DROP DATABASE IF EXISTS "{name}" WITH (FORCE)"#).as_str(),
                    )
                    .await?;
                    Ok::<_, sqlx::Error>(())
                })
        };
        let join = move || std::thread::spawn(cleanup).join();
        let result = if multi_thread {
            tokio::task::block_in_place(join)
        } else {
            join()
        };
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => eprintln!("warning: failed to drop test database {}: {e}", self.name),
            Err(_) => eprintln!("warning: cleanup of test database {} panicked", self.name),
        }
    }
}

pub fn config(env: RailsEnv, globals: &[(&str, &str)]) -> Config {
    Config {
        database_url: test_database_url(),
        bind: "127.0.0.1:0".parse().unwrap(),
        rails_env: env,
        unicorn_port: "3000".into(),
        public_dir: "public".into(),
        discourse_src: None,
        tmp_dir: "tmp".into(),
        letter_avatar_cdn: discourse_rs::config::LETTER_AVATAR_CDN.into(),
        globals: GlobalSettings::from_vars(globals.iter().copied()),
    }
}

pub async fn state(pool: PgPool, config: Config) -> AppState {
    state_with_bus_on(pool.clone(), pool, config).await
}

/// `state` with the bus on another database, for an app pool that cannot
/// connect.
pub async fn state_with_bus_on(bus_pool: PgPool, pool: PgPool, config: Config) -> AppState {
    let bus = pg_bus::Bus::start(bus_pool, discourse_rs::bus::config())
        .await
        .expect("bus starts");
    AppState {
        bus,
        pool,
        anonymous_cache: Arc::new(discourse_rs::anonymous_cache::Cache::new(&config)),
        config,
        site_setting_defs: Arc::new(Definitions::vendored().expect("vendored site_settings.yml")),
        i18n: Arc::new(I18n::vendored().expect("vendored server.en.yml")),
        search_log_cache: Default::default(),
        keys: Arc::new(discourse_rs::session::current::Keys::ephemeral()),
        mailer: discourse_rs::email::Mailer::memory(),
    }
}

/// `SiteSetting.<name> = value`, as the row add_override! writes.
pub async fn set_setting(pool: &PgPool, name: &str, data_type: i32, value: &str) {
    sqlx::query("DELETE FROM site_settings WHERE name = $1")
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO site_settings (name, data_type, value, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now())",
    )
    .bind(name)
    .bind(data_type)
    .bind(value)
    .execute(pool)
    .await
    .unwrap();
}

/// A minimal user upload, like `Fabricate(:upload)`. Returns (id, url).
pub async fn fabricate_upload(pool: &PgPool) -> (i32, String) {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let sha1 = format!("{n:040x}");
    let url = format!("/uploads/default/original/1X/{sha1}.png");
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO uploads (user_id, original_filename, filesize, width, height, url, sha1, \
         extension, created_at, updated_at) \
         VALUES (-1, 'logo.png', 1234, 100, 200, $1, $2, 'png', now(), now()) RETURNING id",
    )
    .bind(&url)
    .bind(&sha1)
    .fetch_one(pool)
    .await
    .unwrap();
    (id, url)
}

/// The Config of the Discourse the golden files were recorded from
/// (parity/environment), pointed at the test database.
pub fn recorded_config() -> Config {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/environment");
    let mut vars = discourse_rs::parity::load_environment(&path).unwrap();
    vars.push(("DATABASE_URL".into(), test_database_url()));
    Config::from_vars(vars).unwrap()
}

/// Removes the resized icon copies SiteIconManager.ensure_optimized! made in
/// the snapshot, so icon URLs resolve to the original uploads.
pub async fn clear_optimized_images(pool: &PgPool) {
    sqlx::query("DELETE FROM optimized_images")
        .execute(pool)
        .await
        .unwrap();
}

/// Every bus message on `channels` after `from`, whoever it is for (all
/// users' and groups' tags held). Waits for delivery first: messages go out
/// once no older transaction is open anywhere on the server, and other
/// tests hold some, so a marker committed after the publishes is awaited.
pub async fn bus_messages(
    st: &AppState,
    from: pg_bus::Position,
    channels: &[&str],
) -> Vec<pg_bus::Message> {
    let pool = &st.pool;
    let mut tx = pool.begin().await.unwrap();
    st.bus
        .publish(&mut tx, "/test-marker", &serde_json::json!({}), None)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let marker = pg_bus::Filter {
        channels: vec!["/test-marker".into()],
        tags: vec![],
    };
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    while st.bus.backlog(from, &marker, 1).await.unwrap().is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the marker never arrived"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let users: Vec<i32> = sqlx::query_scalar("SELECT id FROM users")
        .fetch_all(pool)
        .await
        .unwrap();
    let groups: Vec<i32> = sqlx::query_scalar("SELECT id FROM groups")
        .fetch_all(pool)
        .await
        .unwrap();
    let mut tags: Vec<String> = users.into_iter().map(discourse_rs::bus::user_tag).collect();
    tags.extend(groups.into_iter().map(discourse_rs::bus::group_tag));
    st.bus
        .backlog(
            from,
            &pg_bus::Filter {
                channels: channels.iter().map(|c| c.to_string()).collect(),
                tags,
            },
            1000,
        )
        .await
        .unwrap()
}

/// Reads an event stream until an event called `name` arrives; its data
/// lines joined. What comes before it is dropped; what follows it stays in
/// `buffer` for the next call.
pub async fn next_sse_event(
    body: &mut axum::body::Body,
    buffer: &mut String,
    name: &str,
) -> String {
    use http_body_util::BodyExt;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    let marker = format!("event: {name}\n");
    loop {
        if let Some(start) = buffer.find(&marker)
            && let Some(end) = buffer[start..].find("\n\n")
        {
            let data = buffer[start..start + end]
                .lines()
                .filter_map(|l| l.strip_prefix("data: ").or(l.strip_prefix("data:")))
                .collect::<Vec<_>>()
                .join("\n");
            buffer.replace_range(..start + end + 2, "");
            return data;
        }
        let frame = tokio::time::timeout_at(deadline, body.frame())
            .await
            .unwrap_or_else(|_| panic!("a {name} event within 30 s"))
            .expect("the stream stays open")
            .unwrap();
        if let Ok(data) = frame.into_data() {
            buffer.push_str(&String::from_utf8_lossy(&data));
        }
    }
}
