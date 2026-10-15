use std::error::Error;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use axum::ServiceExt;
use discourse_rs::config::Config;
use discourse_rs::i18n::I18n;
use discourse_rs::multisite::{self, Shared, Sites};
use discourse_rs::site_settings::Definitions;
use tracing_subscriber::EnvFilter;

/// How far apart the sites' workers first run the scheduled jobs.
const STAGGER: Duration = Duration::from_secs(10);

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

    let mut config = Config::from_env()?;
    discourse_rs::readonly::force_from_globals(&config.globals);
    let site_setting_defs = Definitions::vendored()?;
    let i18n = I18n::vendored()?;
    tracing::info!(settings = site_setting_defs.len(), env = ?config.rails_env, "loaded site setting definitions");

    // multisite.yml, when DISCOURSE_MULTISITE_CONFIG_PATH names one.
    let specs = match config.globals.get("multisite_config_path") {
        Some(path) => {
            let src = std::fs::read_to_string(path)
                .map_err(|e| format!("DISCOURSE_MULTISITE_CONFIG_PATH {path}: {e}"))?;
            Some(multisite::parse(&src)?)
        }
        None => None,
    };
    config.site.multisite = specs.is_some();
    if let Some(spec) = specs.iter().flatten().next() {
        return Err(format!(
            "multisite.yml: site {}: only the default site is served yet",
            spec.db
        )
        .into());
    }

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
    let shared = Shared {
        site_setting_defs: Arc::new(site_setting_defs),
        i18n: Arc::new(i18n),
        keys: Arc::new(keys),
        mailer,
        global_rate_limits: Default::default(),
    };

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let default = multisite::boot(&shared, config).await?;
    let sites = match specs {
        None => Sites::single(default),
        Some(_) => Sites::multisite(vec![default])?,
    };
    tracing::info!(addr = %listener.local_addr()?, "listening");

    // A worker per site beside the web server unless DISCOURSE_RS_JOBS=off,
    // which leaves the queue to a separate process.
    let workers: Vec<_> = match std::env::var("DISCOURSE_RS_JOBS").as_deref() {
        Ok("off") => Vec::new(),
        _ => {
            tracing::info!("running background jobs");
            sites
                .states()
                .enumerate()
                .map(|(i, state)| {
                    tokio::spawn(discourse_rs::jobs::work(
                        state.clone(),
                        shutdown_signal(),
                        STAGGER * i as u32,
                    ))
                })
                .collect()
        }
    };
    let dispatcher = sites.into_service();
    axum::serve(
        listener,
        dispatcher
            .clone()
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    // The tracker's deferred work, then each site's lists and counts,
    // which Redis would keep.
    discourse_rs::request_tracking::drain().await;
    for state in dispatcher.sites().states() {
        multisite::shut_down(state).await?;
    }
    for worker in workers {
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
