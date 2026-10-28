//! The command line (`discourse-rs`, `serve`, `setup`, `admin create`),
//! in the library so a binary built with plugins runs the same one.

use std::error::Error;
use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::i18n::I18n;
use crate::multisite::{self, Shared};
use crate::site_settings::Definitions;
use axum::ServiceExt;
use clap::{Parser, Subcommand};
use sqlx::Connection;
use tracing_subscriber::EnvFilter;

/// How far apart the sites' workers first run the scheduled jobs.
const STAGGER: Duration = Duration::from_secs(10);

/// A Rust port of Discourse. Configured by the environment as Discourse
/// is: DATABASE_URL, and every DISCOURSE_* variable is a GlobalSetting.
#[derive(Parser)]
#[command(name = "discourse-rs", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the forum (what running without a command does).
    Serve,
    /// Make a new site's database, as `rake db:migrate` does on a fresh
    /// install: the vendored schema and what db:seed creates, loaded into
    /// the empty database at DATABASE_URL in one transaction, then the
    /// port's own schema. A database with tables is refused. Creating the
    /// vector extension needs a superuser: run this as one with --owner
    /// naming the server's role, or create the extension beforehand.
    Setup {
        /// The role that owns the schema objects (the server's), when the
        /// connecting role is a superuser that should not.
        #[arg(long, value_name = "ROLE")]
        owner: Option<String>,
    },
    /// Administrator accounts.
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
}

#[derive(Subcommand)]
enum AdminCommand {
    /// Create an administrator, or make an existing account one with a new
    /// password, as `rake admin:create`. The password is read from
    /// --password-file, else from standard input (asked for twice on a
    /// terminal); never from the command line. No email is sent.
    Create {
        /// The account's email.
        #[arg(long)]
        email: String,
        /// The account's full name (needed when full_name_requirement is
        /// required_at_signup).
        #[arg(long)]
        name: Option<String>,
        /// A new account's username, checked as a signup's is; without
        /// it the one UserNameSuggester makes from the email (Rails' task
        /// takes none). An existing account keeps its own.
        #[arg(long)]
        username: Option<String>,
        /// A file holding the password (a trailing newline is dropped),
        /// such as a systemd credential.
        #[arg(long, value_name = "PATH")]
        password_file: Option<PathBuf>,
    },
}

/// The `discourse-rs` command line, with the plugins this binary is built
/// with (plugin_api): what a plugin's own binary calls from its main.
pub async fn run(plugins: Vec<Arc<dyn crate::plugin_api::Plugin>>) -> ExitCode {
    if let Err(e) = crate::plugin_api::install(plugins) {
        eprintln!("discourse-rs: {e}");
        return ExitCode::FAILURE;
    }
    let cli = Cli::parse();
    let result = match cli.command {
        None | Some(Command::Serve) => serve().await,
        Some(Command::Setup { owner }) => setup(owner.as_deref()).await,
        Some(Command::Admin {
            command:
                AdminCommand::Create {
                    email,
                    name,
                    username,
                    password_file,
                },
        }) => {
            admin_create(
                &email,
                name.as_deref(),
                username.as_deref(),
                password_file.as_deref(),
            )
            .await
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("discourse-rs: {e}");
            ExitCode::FAILURE
        }
    }
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

async fn setup(owner: Option<&str>) -> Result<(), Box<dyn Error>> {
    let config = Config::from_env()?;
    let defs = Definitions::vendored()?;
    let mut conn = sqlx::PgConnection::connect(&config.site.database_url).await?;
    let report = crate::setup::setup(&mut conn, &defs, &config, owner).await?;
    println!(
        "loaded {} tables and {} schema versions (Discourse {}) and the fresh install's seed",
        report.tables,
        report.versions,
        &crate::schema::discourse_commit()[..11],
    );
    Ok(())
}

/// The password, from the file or standard input.
fn read_password(path: Option<&std::path::Path>) -> Result<String, Box<dyn Error>> {
    let raw = match path {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| format!("reading the password from {}: {e}", path.display()))?,
        None if std::io::stdin().is_terminal() => {
            let password = prompt_hidden("Password: ")?;
            if prompt_hidden("Repeat password: ")? != password {
                return Err("the passwords don't match".into());
            }
            password
        }
        None => {
            let mut raw = String::new();
            std::io::stdin().read_to_string(&mut raw)?;
            raw
        }
    };
    let password = raw
        .strip_suffix('\n')
        .map(|p| p.strip_suffix('\r').unwrap_or(p))
        .unwrap_or(&raw);
    if password.is_empty() {
        return Err("no password given (--password-file, or standard input)".into());
    }
    Ok(password.to_string())
}

/// A line from the terminal with echo off.
fn prompt_hidden(prompt: &str) -> Result<String, Box<dyn Error>> {
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    let fd = libc::STDIN_FILENO;
    // SAFETY: tcgetattr/tcsetattr on stdin with a zeroed termios they fill.
    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut termios) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let echoing = termios;
    termios.c_lflag &= !libc::ECHO;
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) };
    let mut line = String::new();
    let read = std::io::stdin().read_line(&mut line);
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &echoing) };
    eprintln!();
    read?;
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

async fn admin_create(
    email: &str,
    name: Option<&str>,
    username: Option<&str>,
    password_file: Option<&std::path::Path>,
) -> Result<(), Box<dyn Error>> {
    let password = read_password(password_file)?;
    let config = Config::from_env()?;
    let shared = Shared {
        site_setting_defs: Arc::new(Definitions::vendored()?),
        i18n: Arc::new(I18n::vendored()?),
        keys: Arc::new(crate::session::current::Keys::ephemeral()),
        mailer: crate::email::Mailer::from_globals(&config.globals)?,
        global_rate_limits: Default::default(),
    };
    let state = multisite::connect_task(&shared, config).await?;
    let outcome = crate::admin_create::create(
        &state,
        &crate::admin_create::NewAdmin {
            email,
            password: &password,
            name,
            username,
        },
    )
    .await?;
    match outcome {
        crate::admin_create::Outcome::Created { username } => {
            println!("Account created successfully with username {username}; it is an admin.")
        }
        crate::admin_create::Outcome::Updated { username } => {
            println!("Account {username} updated successfully; it is an admin.")
        }
    }
    Ok(())
}

async fn serve() -> Result<(), Box<dyn Error>> {
    init_tracing();

    let config = Config::from_env()?;
    crate::readonly::force_from_globals(&config.globals);
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

    let keys = crate::session::current::Keys::new(secret_key_base(&config).await?);
    let mailer = crate::email::Mailer::from_globals(&config.globals)?;
    tracing::info!(mailer = ?mailer, "outgoing mail");
    let shared = Shared {
        site_setting_defs: Arc::new(site_setting_defs),
        i18n: Arc::new(i18n),
        keys: Arc::new(keys),
        mailer,
        global_rate_limits: Default::default(),
    };

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let sites = multisite::boot_all(&shared, config, specs).await?;
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
                    tokio::spawn(crate::jobs::work(
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
    crate::request_tracking::drain().await;
    for state in dispatcher.sites().states() {
        multisite::shut_down(state).await?;
    }
    for worker in workers {
        worker.await?;
    }

    Ok(())
}

/// `GlobalSetting.safe_secret_key_base`: DISCOURSE_SECRET_KEY_BASE when
/// it is valid, else the one Rails keeps in Redis, generated on first use
/// (here in the default site's database), so sessions survive a restart.
/// Without the database, a secret for this process only.
async fn secret_key_base(config: &Config) -> Result<String, Box<dyn Error>> {
    use crate::session::current::{stored_secret_key_base, valid_secret_key};
    let configured = config.globals.get("secret_key_base");
    if let Some(secret) = configured.filter(|s| valid_secret_key(s)) {
        return Ok(secret.to_string());
    }
    if configured.is_some() {
        tracing::warn!("WARNING: DISCOURSE_SECRET_KEY_BASE is invalid, it was re-generated");
    }
    let stored = async {
        let mut conn = sqlx::PgConnection::connect(&config.site.database_url).await?;
        crate::owned_schema::migrate_conn(&mut conn).await?;
        stored_secret_key_base(&mut conn).await
    }
    .await;
    match stored {
        Ok(secret) => Ok(secret),
        Err(e) => {
            tracing::warn!(
                "keeping a generated secret_key_base failed ({e}): sessions will not survive a restart"
            );
            Ok(crate::session::current::Keys::ephemeral().secret_key_base)
        }
    }
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
