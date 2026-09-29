// Each test crate uses a different subset of these helpers.
#![allow(dead_code)]

use discourse_rs::AppState;
use discourse_rs::config::Config;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// The test database, built by `make db-test` from schema/structure.sql.
pub fn test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must be set; run tests inside nix-shell after `make db-test`")
}

/// A pool that connects on first use, so routes that never touch the
/// database can be exercised without one.
pub fn lazy_pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect_lazy(&test_database_url())
        .expect("TEST_DATABASE_URL is not a valid postgres URL")
}

/// Mirrors a stock Discourse: no GlobalSetting overrides.
pub fn state(pool: PgPool) -> AppState {
    AppState {
        pool,
        config: Config {
            database_url: test_database_url(),
            bind: "127.0.0.1:0".parse().unwrap(),
            cluster_name: None,
        },
    }
}
