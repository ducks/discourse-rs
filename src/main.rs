use std::error::Error;
use std::process::ExitCode;
use std::sync::Arc;

use discourse_rs::config::Config;
use discourse_rs::i18n::I18n;
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
    let i18n = I18n::vendored()?;
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

    discourse_rs::owned_schema::migrate(&pool).await?;

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(addr = %config.bind, "listening");
    // Rails keeps secret_key_base in redis when it is not configured; a
    // process without one gets a random secret, so sessions work but do
    // not survive a restart and Rails cookies are unreadable.
    let keys = match config.globals.get("secret_key_base") {
        Some(secret) => discourse_rs::session::current::Keys::new(secret.to_string()),
        None => {
            tracing::warn!(
                "DISCOURSE_SECRET_KEY_BASE not set: sessions will not survive a restart"
            );
            discourse_rs::session::current::Keys::ephemeral()
        }
    };
    let mailer = discourse_rs::email::Mailer::from_globals(&config.globals)?;
    tracing::info!(mailer = ?mailer, "outgoing mail");
    let state = AppState {
        pool,
        config,
        site_setting_defs: Arc::new(site_setting_defs),
        i18n: Arc::new(i18n),
        search_log_cache: Default::default(),
        keys: Arc::new(keys),
        mailer,
    };
    // A worker beside the web server unless DISCOURSE_RS_JOBS=off, which
    // leaves the queue to a separate process.
    let worker = match std::env::var("DISCOURSE_RS_JOBS").as_deref() {
        Ok("off") => None,
        _ => {
            tracing::info!("running background jobs");
            Some(tokio::spawn(discourse_rs::jobs::work(
                state.clone(),
                shutdown_signal(),
            )))
        }
    };
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    if let Some(worker) = worker {
        worker.await?;
    }

    Ok(())
}

/// Resolves on SIGINT (ctrl-c) or SIGTERM (what `systemctl stop` sends),
/// so in-flight requests finish before the process exits.
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(term) => term,
        Err(e) => {
            tracing::error!("installing SIGTERM handler: {e}");
            return std::future::pending().await;
        }
    };
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(e) = result {
                tracing::error!("installing ctrl-c handler: {e}");
            }
        }
        _ = term.recv() => {}
    }
}
