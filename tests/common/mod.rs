// Each test crate uses a different subset of these helpers.
#![allow(dead_code)]

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use discourse_rs::AppState;
use discourse_rs::config::{Config, GlobalSettings, RailsEnv};
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

        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(template_opts.database(&name))
            .await
            .expect("connecting to the cloned test database");

        TestDb { pool, name, admin }
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // Drop can't be async and may run inside the test's runtime, so the
        // cleanup gets its own thread and runtime.
        let name = self.name.clone();
        let admin = self.admin.clone();
        let result = std::thread::spawn(move || {
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
        })
        .join();
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
        globals: GlobalSettings::from_vars(globals.iter().copied()),
    }
}

pub fn state(pool: PgPool, config: Config) -> AppState {
    AppState {
        pool,
        config,
        site_setting_defs: Arc::new(Definitions::vendored().expect("vendored site_settings.yml")),
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
