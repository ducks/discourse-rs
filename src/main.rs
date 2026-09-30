use std::error::Error;
use std::process::ExitCode;
use std::sync::Arc;

use discourse_rs::config::Config;
use discourse_rs::site_settings::Definitions;
use discourse_rs::{AppState, app, schema};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("discourse-rs: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    let site_setting_defs = Definitions::vendored()?;
    tracing::info!(settings = site_setting_defs.len(), env = ?config.rails_env, "loaded site setting definitions");
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await?;

    let report = schema::verify(&mut *pool.acquire().await?).await?;
    tracing::info!(
        discourse = schema::discourse_commit(),
        versions = report.expected,
        "schema verified"
    );
    if !report.extra.is_empty() {
        tracing::warn!(
            count = report.extra.len(),
            latest = report.extra.last().map(String::as_str),
            "database has schema versions newer than the vendored structure.sql"
        );
    }

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(addr = %config.bind, "listening");
    let state = AppState {
        pool,
        config,
        site_setting_defs: Arc::new(site_setting_defs),
    };
    axum::serve(listener, app(state))
        .with_graceful_shutdown(async {
            if let Err(e) = tokio::signal::ctrl_c().await {
                tracing::error!("installing ctrl-c handler: {e}");
            }
        })
        .await?;

    Ok(())
}
