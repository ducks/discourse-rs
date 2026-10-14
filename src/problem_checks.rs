//! ProblemCheck (app/models/problem_check.rb, app/services/problem_check/),
//! its trackers (ProblemCheckTracker) and the admin notices they raise
//! (AdminNotice). The core checks run here in CORE_PROBLEM_CHECKS' order;
//! no bundled plugin registers one.
//!
//! The checks that read Rails' environment answer about the port itself:
//! `rails_env` its configured RAILS_ENV, `host_names` the hostname it
//! serves, `ram` /proc/meminfo, the email checks its job queue.
//! `image_magick` and `landlock` never fire, since the port runs no
//! external commands (images are drawn in src/images.rs), and
//! `email_polling_errored_recently` and `github_onebox_backoff` neither,
//! since the port polls no mailbox and never backs off GitHub. What would
//! need the network (`twitter_login`'s health check, a group's SMTP
//! credentials) is refused when it would run.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::config::{Config, RailsEnv};
use crate::i18n::I18n;
use crate::site_settings::{ChangeStatus, Definitions, SiteSettings};
use crate::{AppError, Unsupported};

/// `ProblemCheck::NO_TARGET`
pub const NO_TARGET: &str = "__NULL__";

/// `AdminNotice.subjects[:problem]`
const SUBJECT_PROBLEM: i32 = 0;

/// A problem check class's attributes.
#[derive(Debug, Clone, Copy)]
pub struct Check {
    pub identifier: &'static str,
    /// `priority`: "low" or "high".
    pub priority: &'static str,
    /// `perform_every` in seconds: a scheduled check.
    pub perform_every: Option<i64>,
    /// An inline check: registered from application code, never run.
    pub inline: bool,
    /// Has a `targets` lambda of its own.
    pub targeted: bool,
}

impl Check {
    const fn new(identifier: &'static str, priority: &'static str) -> Check {
        Check {
            identifier,
            priority,
            perform_every: None,
            inline: false,
            targeted: false,
        }
    }

    const fn every(mut self, seconds: i64) -> Check {
        self.perform_every = Some(seconds);
        self
    }

    const fn inline(mut self) -> Check {
        self.inline = true;
        self
    }

    const fn targeted(mut self) -> Check {
        self.targeted = true;
        self
    }

    pub fn scheduled(&self) -> bool {
        self.perform_every.is_some()
    }

    pub fn realtime(&self) -> bool {
        !self.scheduled() && !self.inline
    }

    /// `max_retries`, the same for every core check.
    pub const MAX_RETRIES: i64 = 2;
    /// `retry_after`, in seconds.
    pub const RETRY_AFTER: i64 = 30;
    /// `max_blips`
    const MAX_BLIPS: i32 = 0;

    /// `AdminNotice.priorities`
    fn priority_id(&self) -> i32 {
        if self.priority == "high" { 1 } else { 0 }
    }
}

/// `ProblemCheck::CORE_PROBLEM_CHECKS`, in order.
pub const CHECKS: [Check; 35] = [
    Check::new("bad_favicon_url", "low").inline(),
    Check::new("content_security_policy_disabled", "low"),
    Check::new("email_sending_failures", "low").every(3600),
    Check::new("email_polling_errored_recently", "low"),
    Check::new("facebook_config", "low"),
    Check::new("failing_emails", "low"),
    Check::new("force_https", "low"),
    Check::new("github_config", "low"),
    Check::new("github_onebox_backoff", "low").every(600),
    Check::new("google_analytics_version", "low"),
    Check::new("google_oauth2_config", "low"),
    Check::new("group_email_credentials", "high")
        .every(1800)
        .targeted(),
    Check::new("host_names", "low"),
    Check::new("image_magick", "low"),
    Check::new("landlock", "high"),
    Check::new("missing_aws_sns_topic_arn", "low"),
    Check::new("missing_mailgun_api_key", "low"),
    Check::new("out_of_date_themes", "low"),
    Check::new("poll_pop3_timeout", "low").inline(),
    Check::new("poll_pop3_auth_error", "low").inline(),
    Check::new("qq_mail_smtp", "low"),
    Check::new("rails_env", "low"),
    Check::new("ram", "low"),
    Check::new("s3_backup_config", "low"),
    Check::new("s3_cdn", "low"),
    Check::new("s3_upload_config", "low"),
    Check::new("sidekiq_check", "low"),
    Check::new("subfolder_ends_in_slash", "low"),
    Check::new("starttls_disabled", "high"),
    Check::new("translation_overrides", "low"),
    Check::new("twitter_config", "low"),
    Check::new("twitter_login", "high").every(24 * 3600),
    Check::new("unreachable_themes", "low"),
    Check::new("watched_words", "low"),
    Check::new("upcoming_change_stable_opted_out", "low")
        .every(3600)
        .targeted(),
];

/// `ProblemCheck[identifier]`
pub fn find(identifier: &str) -> Option<&'static Check> {
    CHECKS.iter().find(|c| c.identifier == identifier)
}

/// What the checks read.
pub struct Env<'a> {
    pub settings: &'a SiteSettings,
    pub defs: &'a Definitions,
    pub config: &'a Config,
    pub i18n: &'a I18n,
}

impl Env<'_> {
    fn setting(&self, name: &str) -> Result<&crate::site_settings::Value, AppError> {
        Ok(self.settings.get(name)?)
    }

    fn on(&self, name: &str) -> Result<bool, AppError> {
        Ok(self.setting(name)?.truthy())
    }

    fn blank(&self, name: &str) -> Result<bool, AppError> {
        Ok(self.setting(name)?.is_blank())
    }

    /// `Discourse.base_path`
    pub fn base_path(&self) -> &str {
        self.config.globals.relative_url_root()
    }

    /// `ActionMailer::Base.smtp_settings[:address]`
    fn smtp_address(&self) -> String {
        self.config
            .globals
            .get("smtp_address")
            .unwrap_or_default()
            .to_string()
    }
}

/// `check.targets.call`
pub async fn targets(
    conn: &mut PgConnection,
    env: &Env<'_>,
    check: &Check,
) -> Result<Vec<String>, AppError> {
    Ok(match check.identifier {
        "group_email_credentials" => {
            sqlx::query_scalar("SELECT name FROM groups WHERE smtp_enabled")
                .fetch_all(conn)
                .await?
        }
        "upcoming_change_stable_opted_out" => env
            .defs
            .upcoming_change_names()
            .into_iter()
            .map(str::to_string)
            .collect(),
        _ => vec![NO_TARGET.to_string()],
    })
}

/// What `check.call` found: the notice's details (`Problem#details`).
pub type Problem = Map<String, Value>;

fn problem(pairs: &[(&str, Value)]) -> Option<Problem> {
    Some(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

/// `SiteSettings::LabelFormatter.setting_markers`
fn setting_markers(settings: &[&str]) -> String {
    settings
        .iter()
        .map(|s| format!("{{{{setting:{s}}}}}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `check.new(target).call`
pub async fn call(
    conn: &mut PgConnection,
    env: &Env<'_>,
    check: &Check,
    target: &str,
) -> Result<Option<Problem>, AppError> {
    let none = Ok(None);
    match check.identifier {
        "content_security_policy_disabled" => {
            if env.on("content_security_policy")? {
                return none;
            }
            Ok(problem(&[]))
        }
        "force_https" => {
            if env.on("force_https")? {
                return none;
            }
            Ok(problem(&[]))
        }
        "google_analytics_version" => {
            if env.setting("ga_version")?.to_s() != "v3_analytics" {
                return none;
            }
            Ok(problem(&[]))
        }
        "facebook_config" | "github_config" | "google_oauth2_config" | "twitter_config" => {
            let (enable, required) = authenticator(check.identifier);
            if !env.on(enable)? {
                return none;
            }
            let missing = missing(env, required)?;
            if missing.is_empty() {
                return none;
            }
            Ok(problem(&[(
                "missing_settings",
                json!(setting_markers(&missing)),
            )]))
        }
        "s3_backup_config" | "s3_upload_config" => {
            if env.config.globals.use_s3() {
                return none;
            }
            let (enabled, bucket) = if check.identifier == "s3_backup_config" {
                (
                    env.setting("backup_location")?.to_s() == "s3",
                    "s3_backup_bucket",
                )
            } else {
                (env.on("enable_s3_uploads")?, "s3_upload_bucket")
            };
            if !enabled {
                return none;
            }
            let mut required = if env.on("s3_use_iam_profile")? {
                Vec::new()
            } else {
                vec!["s3_access_key_id", "s3_secret_access_key"]
            };
            required.push(bucket);
            let missing = missing(env, &required)?;
            if missing.is_empty() {
                return none;
            }
            Ok(problem(&[(
                "missing_settings",
                json!(setting_markers(&missing)),
            )]))
        }
        "s3_cdn" => {
            if !env.config.globals.use_s3() && !env.on("enable_s3_uploads")? {
                return none;
            }
            // SiteSetting::Upload.s3_cdn_url: the global one first.
            let cdn = env
                .config
                .globals
                .s3_cdn_url()
                .map(str::to_string)
                .or_else(|| env.setting("s3_cdn_url").ok().and_then(|v| v.presence()));
            if cdn.is_some() {
                return none;
            }
            Ok(problem(&[]))
        }
        "email_sending_failures" => {
            if failing_email_jobs(conn).await? > 0 {
                return none;
            }
            // SkippedEmailLog.reason_types[:custom], in the last 24 hours.
            let failed: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM skipped_email_logs \
                 WHERE reason_type = 1 AND created_at >= now() - interval '24 hours'",
            )
            .fetch_one(&mut *conn)
            .await?;
            if failed == 0 {
                return none;
            }
            Ok(problem(&[("count", json!(failed))]))
        }
        "failing_emails" => {
            let failed = failing_email_jobs(conn).await?;
            if failed == 0 {
                return none;
            }
            Ok(problem(&[("num_failed_jobs", json!(failed))]))
        }
        // Jobs::PollMailbox's error count: the port polls no mailbox.
        "email_polling_errored_recently" => none,
        // GithubRateLimit: the port never backs off GitHub.
        "github_onebox_backoff" => none,
        "group_email_credentials" => {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM groups WHERE smtp_enabled AND name = $1)",
            )
            .bind(target)
            .fetch_one(&mut *conn)
            .await?;
            if !exists || !env.on("enable_smtp")? {
                return none;
            }
            Err(Unsupported("checking a group's SMTP credentials (EmailSettingsValidator)").into())
        }
        "host_names" => {
            let host = env.config.globals.hostname(env.config.rails_env);
            if !matches!(host, "localhost" | "production.localhost") {
                return none;
            }
            Ok(problem(&[]))
        }
        // The port draws images itself and runs no external command.
        "image_magick" | "landlock" => none,
        "missing_aws_sns_topic_arn" => {
            if !env.blank("aws_sns_topic_arn_allowlist")? {
                return none;
            }
            let ses = regex::RegexBuilder::new(r"(email-smtp|amazonses).*amazonaws\.com\z")
                .case_insensitive(true)
                .build()
                .expect("a literal pattern");
            if !ses.is_match(&env.smtp_address()) {
                return none;
            }
            Ok(problem(&[]))
        }
        "missing_mailgun_api_key" => {
            if !env.on("reply_by_email_enabled")? {
                return none;
            }
            if env.smtp_address() != "smtp.mailgun.org" {
                return none;
            }
            if !env.blank("mailgun_api_key")? {
                return none;
            }
            Ok(problem(&[]))
        }
        "out_of_date_themes" | "unreachable_themes" => {
            let condition = if check.identifier == "out_of_date_themes" {
                "(r.commits_behind > 0 OR r.remote_version <> r.local_version) AND t.enabled"
            } else {
                "r.last_error_text IS NOT NULL"
            };
            let themes: Vec<(String, i32)> = sqlx::query_as(&format!(
                "SELECT t.name, t.id FROM remote_themes r \
                 JOIN themes t ON t.remote_theme_id = r.id \
                 WHERE r.remote_url <> '' AND {condition}"
            ))
            .fetch_all(&mut *conn)
            .await?;
            if themes.is_empty() {
                return none;
            }
            let items: Vec<String> = themes
                .iter()
                .map(|(name, id)| {
                    format!(
                        "<li><a href=\"{}/admin/customize/themes/{id}\">{}</a></li>",
                        env.base_path(),
                        crate::admin_site_settings::escape_html(name)
                    )
                })
                .collect();
            // `<<~HTML.squish`: whitespace runs to one space.
            let list = format!("<ul>{}</ul>", items.join("\n"));
            let list = list.split_whitespace().collect::<Vec<_>>().join(" ");
            Ok(problem(&[("themes_list", json!(list))]))
        }
        "qq_mail_smtp" => {
            let address = env.smtp_address();
            let lower = address.to_lowercase();
            if !(lower == "qq.com" || (lower.ends_with(".qq.com") && lower.len() > ".qq.com".len()))
            {
                return none;
            }
            Ok(problem(&[("smtp_address", json!(address))]))
        }
        "rails_env" => {
            let env_name = match env.config.rails_env {
                RailsEnv::Production => return none,
                RailsEnv::Development => "development",
                RailsEnv::Test => "test",
            };
            Ok(problem(&[("env", json!(env_name))]))
        }
        "ram" => match mem_total() {
            Some(total) if total <= 950_000 => Ok(problem(&[])),
            _ => none,
        },
        "sidekiq_check" => {
            let queued = queued_jobs(conn).await?;
            if queued > 0 && !jobs_performed_recently(conn).await? {
                let mut p = problem(&[]).unwrap_or_default();
                p.insert(
                    TRANSLATION_KEY_DETAIL.into(),
                    json!("dashboard.problem.sidekiq_check"),
                );
                return Ok(Some(p));
            }
            if queued >= 100_000 {
                let mut p = problem(&[("queue_size", json!(queued))]).unwrap_or_default();
                p.insert(
                    TRANSLATION_KEY_DETAIL.into(),
                    json!("dashboard.problem.queue_size"),
                );
                return Ok(Some(p));
            }
            none
        }
        "subfolder_ends_in_slash" => {
            if !env.base_path().ends_with('/') {
                return none;
            }
            Ok(problem(&[]))
        }
        "starttls_disabled" => {
            // GlobalSetting.smtp_enable_start_tls (default true).
            let on = env
                .config
                .globals
                .get("smtp_enable_start_tls")
                .is_none_or(|v| v != "false");
            if on {
                return none;
            }
            Ok(problem(&[]))
        }
        "translation_overrides" => {
            // TranslationOverride.statuses: outdated 1, invalid_interpolation_keys 2.
            let any: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM translation_overrides WHERE status IN (1, 2))",
            )
            .fetch_one(&mut *conn)
            .await?;
            if !any {
                return none;
            }
            Ok(problem(&[]))
        }
        "twitter_login" => {
            // Auth::TwitterAuthenticator#enabled?, then #healthy? (a request
            // token from Twitter).
            if !env.on("enable_twitter_logins")?
                || !missing(env, &["twitter_consumer_key", "twitter_consumer_secret"])?.is_empty()
            {
                return none;
            }
            Err(Unsupported("checking Twitter's login health (an outbound request)").into())
        }
        "watched_words" => {
            // Every action's regexps compile, or the port can't tell an
            // invalid Ruby regexp from one it can't translate: each admin
            // regexp was refused at creation unless both dialects read it.
            for (_, action) in crate::word_watcher::ACTIONS {
                for c in
                    crate::word_watcher::compiled_for_action(conn, env.settings, action).await?
                {
                    c.regex()?;
                }
            }
            none
        }
        "upcoming_change_stable_opted_out" => upcoming_change_opted_out(env, target),
        // InlineProblemCheck#call is no problem; they are never run.
        _ if check.inline => none,
        _ => Err(Unsupported("an unknown problem check").into()),
    }
}

/// An authenticator's `enable_setting` and `required_settings`.
fn authenticator(identifier: &str) -> (&'static str, &'static [&'static str]) {
    match identifier {
        "facebook_config" => (
            "enable_facebook_logins",
            &["facebook_app_id", "facebook_app_secret"],
        ),
        "github_config" => (
            "enable_github_logins",
            &["github_client_id", "github_client_secret"],
        ),
        "google_oauth2_config" => (
            "enable_google_oauth2_logins",
            &["google_oauth2_client_id", "google_oauth2_client_secret"],
        ),
        _ => (
            "enable_twitter_logins",
            &["twitter_consumer_key", "twitter_consumer_secret"],
        ),
    }
}

/// The settings of `names` that are blank.
fn missing<'a>(env: &Env<'_>, names: &[&'a str]) -> Result<Vec<&'a str>, AppError> {
    let mut out = Vec::new();
    for name in names {
        if env.blank(name)? {
            out.push(*name);
        }
    }
    Ok(out)
}

/// `UpcomingChangeStableOptedOut#call`
fn upcoming_change_opted_out(env: &Env<'_>, target: &str) -> Result<Option<Problem>, AppError> {
    let Some(status) = env.defs.change_status(target) else {
        return Err(Unsupported("an upcoming change that is not declared").into());
    };
    // ConditionalDisplay.should_display?: the owning plugin's gates.
    if !env.defs.change_plugin_enabled(target, env.settings) {
        return Ok(None);
    }
    let enabled = env.on(target)?;
    // Only a stable change that is off is a problem.
    let would_fire = !enabled && status >= ChangeStatus::Stable;
    if !would_fire {
        return Ok(None);
    }
    // ConditionalDisplay's own conditions for a few changes.
    match target {
        "enable_local_logins_via_code" => {
            // `SiteSetting.current[:enable_local_logins_via_code]` is the
            // resolved value, false here.
            if !(env.on("enable_local_logins")?
                && env.on("enable_local_logins_via_email")?
                && !env.on("enable_discourse_connect")?)
            {
                return Ok(None);
            }
        }
        "update_pending_users_reminder_default" => {
            if !env.on("must_approve_users")? {
                return Ok(None);
            }
        }
        "enable_horizon_high_context_topic_cards" | "remove_and_replace_uncategorized" => {
            return Err(Unsupported(
                "the display conditions of an upcoming change that reads themes or categories",
            )
            .into());
        }
        _ => {}
    }
    Ok(problem(&[(
        "upcoming_change",
        json!(crate::admin_site_settings::humanized_name(target)),
    )]))
}

/// `Jobs.num_email_retry_jobs`: email jobs waiting for a retry (Sidekiq's
/// retry set): failed at least once, not yet failed for good.
async fn failing_email_jobs(conn: &mut PgConnection) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM discourse_rs.jobs \
         WHERE name LIKE '%\\_email' AND last_error IS NOT NULL AND failed_at IS NULL",
    )
    .fetch_one(conn)
    .await?)
}

/// `Jobs.queued`: jobs due and not waiting on a retry.
async fn queued_jobs(conn: &mut PgConnection) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM discourse_rs.jobs \
         WHERE failed_at IS NULL AND last_error IS NULL AND run_at <= clock_timestamp()",
    )
    .fetch_one(conn)
    .await?)
}

/// The key `Jobs::Base` stamps after each job (`last_job_perform_at`).
pub const LAST_JOB_PERFORMED_KEY: &str = "last_job_perform_at";

/// `Jobs.last_job_performed_at > 2.minutes.ago`
async fn jobs_performed_recently(conn: &mut PgConnection) -> Result<bool, AppError> {
    let at = crate::owned_schema::cached_get(conn, LAST_JOB_PERFORMED_KEY).await?;
    let Some(at) = at.and_then(|s| s.parse::<i64>().ok()) else {
        return Ok(false);
    };
    Ok(at > crate::clock::now().timestamp() - 120)
}

/// `MemInfo#mem_total` in kB; None when unknown.
fn mem_total() -> Option<i64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    info.lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
}

/// `AdminNotice::TRANSLATION_KEY_DETAIL`
const TRANSLATION_KEY_DETAIL: &str = "_translation_key";

/// A tracker row.
#[derive(Debug, sqlx::FromRow)]
pub struct Tracker {
    pub id: i64,
    pub identifier: String,
    pub target: String,
    pub blips: i32,
    pub details: Option<Value>,
    pub ignored_at: Option<chrono::NaiveDateTime>,
    pub last_run_at: Option<chrono::NaiveDateTime>,
    pub last_success_at: Option<chrono::NaiveDateTime>,
    pub last_problem_at: Option<chrono::NaiveDateTime>,
    pub next_run_at: Option<chrono::NaiveDateTime>,
}

impl Tracker {
    /// `failing?`
    pub fn failing(&self) -> bool {
        self.last_problem_at == self.last_run_at
    }

    /// `passing?`
    pub fn passing(&self) -> bool {
        self.last_success_at == self.last_run_at
    }
}

const TRACKER_COLUMNS: &str = "id, identifier, target, blips, details, ignored_at, last_run_at, \
                               last_success_at, last_problem_at, next_run_at";

pub async fn tracker_by_id(conn: &mut PgConnection, id: i64) -> Result<Option<Tracker>, AppError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {TRACKER_COLUMNS} FROM problem_check_trackers WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(conn)
    .await?)
}

/// `ProblemCheckTracker[identifier, target]`: find_or_create_by.
pub async fn tracker(
    conn: &mut PgConnection,
    identifier: &str,
    target: &str,
) -> Result<Tracker, AppError> {
    let found: Option<Tracker> = sqlx::query_as(&format!(
        "SELECT {TRACKER_COLUMNS} FROM problem_check_trackers \
         WHERE identifier = $1 AND target = $2 LIMIT 1"
    ))
    .bind(identifier)
    .bind(target)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(t) = found {
        return Ok(t);
    }
    Ok(sqlx::query_as(&format!(
        "INSERT INTO problem_check_trackers (identifier, target, blips, details) \
         VALUES ($1, $2, 0, '{{}}') RETURNING {TRACKER_COLUMNS}"
    ))
    .bind(identifier)
    .bind(target)
    .fetch_one(conn)
    .await?)
}

/// The tracker's problem notices (`admin_notice`): its identifier's, for
/// its target.
const NOTICE_SCOPE: &str = "subject = 0 AND identifier = $1 AND details->>'target' = $2";

/// `silence_the_alarm`
async fn silence_the_alarm(
    conn: &mut PgConnection,
    identifier: &str,
    target: &str,
) -> Result<(), AppError> {
    sqlx::query(&format!("DELETE FROM admin_notices WHERE {NOTICE_SCOPE}"))
        .bind(identifier)
        .bind(target)
        .execute(conn)
        .await?;
    Ok(())
}

/// `tracker.destroy` (before_destroy silences the alarm).
pub async fn destroy_tracker(conn: &mut PgConnection, t: &Tracker) -> Result<(), AppError> {
    silence_the_alarm(conn, &t.identifier, &t.target).await?;
    sqlx::query("DELETE FROM problem_check_trackers WHERE id = $1")
        .bind(t.id)
        .execute(conn)
        .await?;
    Ok(())
}

/// `details.merge(target:)`
fn with_target(details: &Option<Value>, target: &str) -> Value {
    let mut map = match details {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    map.insert("target".into(), json!(target));
    Value::Object(map)
}

/// `sound_the_alarm`: the notice, unless there is one.
async fn sound_the_alarm(
    conn: &mut PgConnection,
    check: &Check,
    t: &Tracker,
) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM admin_notices WHERE {NOTICE_SCOPE})"
    ))
    .bind(&t.identifier)
    .bind(&t.target)
    .fetch_one(&mut *conn)
    .await?;
    if exists {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO admin_notices (subject, priority, identifier, details, created_at, updated_at) \
         SELECT $1, $2, $3, $4::json, at, at FROM clock_timestamp() AS at",
    )
    .bind(SUBJECT_PROBLEM)
    .bind(check.priority_id())
    .bind(&t.identifier)
    .bind(with_target(&t.details, &t.target).to_string())
    .execute(conn)
    .await?;
    Ok(())
}

/// `sound_the_alarm?`: watched, failing and past the check's blips.
fn should_sound(t: &Tracker) -> bool {
    t.ignored_at.is_none() && t.failing() && t.blips > Check::MAX_BLIPS
}

/// `tracker.ignore!`
pub async fn ignore(conn: &mut PgConnection, t: &Tracker) -> Result<(), AppError> {
    if t.ignored_at.is_some() {
        return Ok(());
    }
    sqlx::query("UPDATE problem_check_trackers SET ignored_at = clock_timestamp() WHERE id = $1")
        .bind(t.id)
        .execute(&mut *conn)
        .await?;
    silence_the_alarm(conn, &t.identifier, &t.target).await
}

/// `tracker.watch!`
pub async fn watch(conn: &mut PgConnection, t: Tracker) -> Result<(), AppError> {
    if t.ignored_at.is_none() {
        return Ok(());
    }
    sqlx::query("UPDATE problem_check_trackers SET ignored_at = NULL WHERE id = $1")
        .bind(t.id)
        .execute(&mut *conn)
        .await?;
    let t = Tracker {
        ignored_at: None,
        ..t
    };
    let Some(check) = find(&t.identifier) else {
        return Err(Unsupported("watching the tracker of a check that is gone").into());
    };
    if should_sound(&t) {
        sound_the_alarm(conn, check, &t).await?;
    }
    Ok(())
}

/// `tracker.no_problem!(next_run_at:)`
async fn no_problem(
    conn: &mut PgConnection,
    t: &Tracker,
    every: Option<i64>,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE problem_check_trackers SET blips = 0, last_run_at = n.at, last_success_at = n.at, \
                next_run_at = n.at + make_interval(secs => $2) \
         FROM (SELECT clock_timestamp()::timestamp AS at) n WHERE id = $1",
    )
    .bind(t.id)
    .bind(every.map(|s| s as f64))
    .execute(&mut *conn)
    .await?;
    silence_the_alarm(conn, &t.identifier, &t.target).await
}

/// `tracker.problem!(next_run_at:, details:)`
async fn record_problem(
    conn: &mut PgConnection,
    check: &Check,
    t: Tracker,
    every: Option<i64>,
    details: Map<String, Value>,
) -> Result<(), AppError> {
    let t: Tracker = sqlx::query_as(&format!(
        "UPDATE problem_check_trackers SET blips = blips + 1, details = $2::json, \
                last_run_at = n.at, last_problem_at = n.at, \
                next_run_at = n.at + make_interval(secs => $3) \
         FROM (SELECT clock_timestamp()::timestamp AS at) n WHERE id = $1 \
         RETURNING {}",
        TRACKER_COLUMNS
            .split(", ")
            .map(|c| format!("problem_check_trackers.{}", c.trim()))
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .bind(t.id)
    .bind(Value::Object(details.clone()).to_string())
    .bind(every.map(|s| s as f64))
    .fetch_one(&mut *conn)
    .await?;
    // update_notice_details
    sqlx::query(&format!(
        "UPDATE admin_notices SET details = $3::json WHERE {NOTICE_SCOPE}"
    ))
    .bind(&t.identifier)
    .bind(&t.target)
    .bind(with_target(&Some(Value::Object(details)), &t.target).to_string())
    .execute(&mut *conn)
    .await?;
    if should_sound(&t) {
        sound_the_alarm(conn, check, &t).await?;
    }
    Ok(())
}

/// How a run ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Ran {
    Done,
    /// A problem while retries are left (`RetrySignal`): nothing recorded.
    Retry,
}

/// `check.new(target).run`, with run_problem_check's retry block when
/// `retry_count` is given.
pub async fn run(
    conn: &mut PgConnection,
    env: &Env<'_>,
    check: &Check,
    target: &str,
    retry_count: Option<i64>,
) -> Result<Ran, AppError> {
    if check.targeted {
        let current = targets(conn, env, check).await?;
        if target == NO_TARGET || !current.iter().any(|t| t == target) {
            let t = tracker(conn, check.identifier, target).await?;
            destroy_tracker(conn, &t).await?;
            return Ok(Ran::Done);
        }
    }
    let found = call(conn, env, check, target).await?;
    if found.is_some() && retry_count.is_some_and(|n| n < Check::MAX_RETRIES) {
        return Ok(Ran::Retry);
    }
    let t = tracker(conn, check.identifier, target).await?;
    match found {
        None => no_problem(conn, &t, check.perform_every).await?,
        Some(mut details) => {
            details.insert("base_path".into(), json!(env.base_path()));
            record_problem(conn, check, t, check.perform_every, details).await?;
        }
    }
    Ok(Ran::Done)
}

/// `ProblemCheck.realtime.run_all`: every enabled realtime check, for
/// each of its targets.
pub async fn run_realtime(conn: &mut PgConnection, env: &Env<'_>) -> Result<(), AppError> {
    for check in CHECKS.iter().filter(|c| c.realtime()) {
        for target in targets(conn, env, check).await? {
            run(conn, env, check, &target, None).await?;
        }
    }
    Ok(())
}

/// `AdminNotice.problem.order(:id)` with AdminNoticeSerializer.
pub async fn serialized_problems(
    conn: &mut PgConnection,
    env: &Env<'_>,
    urls: &crate::url::Urls<'_>,
) -> Result<Value, AppError> {
    let rows: Vec<(i64, i32, String, Value)> = sqlx::query_as(
        "SELECT id, priority, identifier, details::jsonb FROM admin_notices \
         WHERE subject = 0 ORDER BY id",
    )
    .fetch_all(conn)
    .await?;
    let mut out = Vec::new();
    for (id, priority, identifier, details) in rows {
        out.push(json!({
            "id": id,
            "priority": if priority == 1 { "high" } else { "low" },
            "message": message(env, urls, &identifier, &details)?,
            "identifier": identifier,
        }));
    }
    Ok(Value::Array(out))
}

/// `AdminNotice#message`: the translation with the details (and
/// base_path), setting links expanded, through the notice sanitizer.
pub fn message(
    env: &Env<'_>,
    urls: &crate::url::Urls<'_>,
    identifier: &str,
    details: &Value,
) -> Result<String, AppError> {
    let mut data: Vec<(String, String)> = Vec::new();
    let mut key = format!("dashboard.problem.{identifier}");
    let mut count = None;
    if let Some(map) = details.as_object() {
        for (k, v) in map {
            if k == TRANSLATION_KEY_DETAIL {
                if let Some(s) = v.as_str() {
                    key = s.to_string();
                }
                continue;
            }
            if k == "count" {
                count = v.as_i64();
            }
            let text = match v {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            data.push((k.clone(), text));
        }
    }
    data.retain(|(k, _)| k != "base_path");
    data.push(("base_path".into(), env.base_path().to_string()));
    let args: Vec<(&str, &str)> = data.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let translated = match (env.i18n.t(&key), count) {
        (Some(_), _) => env.i18n.t_with(&key, &args),
        (None, Some(n)) => env.i18n.t_count(&key, n, &args),
        (None, None) => None,
    }
    .ok_or(Unsupported("an admin notice without its translation"))?;
    let cx = crate::admin_site_settings::Context {
        defs: env.defs,
        settings: env.settings,
        i18n: env.i18n,
        globals: &env.config.globals,
        base_path: env.base_path(),
        urls,
    };
    let expanded = crate::admin_site_settings::expand_setting_links(&cx, &translated);
    Ok(sanitize_notice(&expanded)?)
}

/// `AdminNotice::MESSAGE_ALLOWED_TAGS`
const ALLOWED_TAGS: [&str; 4] = ["a", "pre", "ul", "li"];
/// `AdminNotice::MESSAGE_ALLOWED_ATTRIBUTES`
const ALLOWED_ATTRIBUTES: [&str; 8] = [
    "href",
    "target",
    "rel",
    "class",
    "data-setting-name",
    "data-setting-area",
    "data-setting-category",
    "data-setting-plugin",
];

/// `AdminNotice::MESSAGE_SANITIZER.sanitize(html, tags:, attributes:)`
/// (Loofah over libxml2's HTML4 parser) for the markup the locale's
/// notices hold: properly nested elements with quoted attributes. Tags
/// off the list go, their text stays; text is written back with `&`,
/// `<` and `>` escaped, attributes in double quotes.
pub fn sanitize_notice(html: &str) -> Result<String, Unsupported> {
    let mut out = String::with_capacity(html.len());
    let mut open: Vec<String> = Vec::new();
    let mut rest = html;
    loop {
        let next = rest.char_indices().find(|&(i, c)| {
            c == '<'
                && rest[i + 1..]
                    .chars()
                    .next()
                    .is_some_and(|n| n.is_ascii_alphabetic() || n == '/' || n == '!')
        });
        let Some((i, _)) = next else {
            out.push_str(&escape_text(&decode(rest)?));
            break;
        };
        out.push_str(&escape_text(&decode(&rest[..i])?));
        let after = &rest[i + 1..];
        if after.starts_with('!') {
            return Err(Unsupported("sanitizing comments in an admin notice"));
        }
        let Some(end) = after.find('>') else {
            return Err(Unsupported(
                "sanitizing an unterminated tag in an admin notice",
            ));
        };
        let inner = &after[..end];
        rest = &after[end + 1..];
        if let Some(closing) = inner.strip_prefix('/') {
            let name = closing.trim().to_ascii_lowercase();
            if open.pop().as_deref() != Some(name.as_str()) {
                return Err(Unsupported("sanitizing misnested tags in an admin notice"));
            }
            if ALLOWED_TAGS.contains(&name.as_str()) {
                out.push_str(&format!("</{name}>"));
            }
            continue;
        }
        let (name, attributes) = parse_tag(inner)?;
        let allowed = ALLOWED_TAGS.contains(&name.as_str());
        if allowed {
            out.push('<');
            out.push_str(&name);
            for (attr, value) in &attributes {
                if !ALLOWED_ATTRIBUTES.contains(&attr.as_str()) {
                    continue;
                }
                if attr == "href" && !safe_href(value) {
                    return Err(Unsupported("sanitizing a link scheme in an admin notice"));
                }
                let written = if attr == "href" {
                    value.replace('"', "%22")
                } else {
                    value.replace('&', "&amp;").replace('"', "&quot;")
                };
                out.push_str(&format!(" {attr}=\"{written}\""));
            }
            out.push('>');
        }
        open.push(name);
    }
    if !open.is_empty() {
        return Err(Unsupported("sanitizing unclosed tags in an admin notice"));
    }
    Ok(out)
}

/// A relative, fragment or http(s)/mailto link.
fn safe_href(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    url.starts_with('/')
        || url.starts_with('#')
        || lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
}

fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The XML entities and numeric references, as libxml2 reads text.
fn decode(text: &str) -> Result<String, Unsupported> {
    if !text.contains('&') {
        return Ok(text.to_string());
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(end) = after.find(';').filter(|&e| e > 0 && e < 10) else {
            out.push('&');
            rest = after;
            continue;
        };
        let name = &after[..end];
        let c = match name {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(hex) = name.strip_prefix("#x").or(name.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).ok()
                } else if let Some(dec) = name.strip_prefix('#') {
                    dec.parse().ok()
                } else {
                    return Err(Unsupported("named character references in an admin notice"));
                };
                code.and_then(char::from_u32)
                    .filter(|c| *c != '\u{a0}')
                    .ok_or(Unsupported("character references in an admin notice"))?
            }
        };
        out.push(c);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// A start tag's name and attributes (quoted values, decoded).
fn parse_tag(inner: &str) -> Result<(String, Vec<(String, String)>), Unsupported> {
    let inner = inner.trim_end().trim_end_matches('/');
    let name_end = inner
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(inner.len());
    let name = inner[..name_end].to_ascii_lowercase();
    let mut attributes = Vec::new();
    let mut rest = inner[name_end..].trim_start();
    while !rest.is_empty() {
        let eq = rest
            .find('=')
            .ok_or(Unsupported("valueless attributes in an admin notice"))?;
        let attr = rest[..eq].trim().to_ascii_lowercase();
        let value_part = rest[eq + 1..].trim_start();
        let quote = value_part
            .chars()
            .next()
            .filter(|q| *q == '"' || *q == '\'')
            .ok_or(Unsupported("unquoted attributes in an admin notice"))?;
        let close = value_part[1..]
            .find(quote)
            .ok_or(Unsupported("unterminated attributes in an admin notice"))?;
        let value = decode(&value_part[1..1 + close])?;
        if !attributes
            .iter()
            .any(|(a, _): &(String, String)| *a == attr)
        {
            attributes.push((attr, value));
        }
        rest = value_part[close + 2..].trim_start();
    }
    Ok((name, attributes))
}

/// `ProblemCheckTrackerSerializer`
pub fn tracker_json(t: &Tracker) -> Value {
    json!({
        "id": t.id,
        "identifier": t.identifier,
        "target": t.target,
        "last_run_at": t.last_run_at.map(|at| at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()),
        "ignored": t.ignored_at.is_some(),
        "status": if t.passing() { "passing" } else { "failing" },
    })
}

/// `ProblemCheckTracker.all.order(:identifier, :target)`
pub async fn all_trackers(conn: &mut PgConnection) -> Result<Vec<Tracker>, AppError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {TRACKER_COLUMNS} FROM problem_check_trackers ORDER BY identifier, target"
    ))
    .fetch_all(conn)
    .await?)
}

/// `ProblemCheck#cleanup_trackers` for every check, then the trackers and
/// notices of checks that no longer exist (Jobs::CleanupProblemCheckTrackers).
pub async fn cleanup_trackers(conn: &mut PgConnection, env: &Env<'_>) -> Result<(), AppError> {
    for check in &CHECKS {
        let current = targets(conn, env, check).await?;
        if current.is_empty() {
            continue;
        }
        let stale: Vec<Tracker> = sqlx::query_as(&format!(
            "SELECT {TRACKER_COLUMNS} FROM problem_check_trackers \
             WHERE identifier = $1 AND NOT (target = ANY($2)) ORDER BY id"
        ))
        .bind(check.identifier)
        .bind(&current)
        .fetch_all(&mut *conn)
        .await?;
        for t in stale {
            destroy_tracker(conn, &t).await?;
        }
    }
    let known: Vec<&str> = CHECKS.iter().map(|c| c.identifier).collect();
    let gone: Vec<Tracker> = sqlx::query_as(&format!(
        "SELECT {TRACKER_COLUMNS} FROM problem_check_trackers \
         WHERE NOT (identifier = ANY($1)) ORDER BY id"
    ))
    .bind(&known)
    .fetch_all(&mut *conn)
    .await?;
    for t in gone {
        destroy_tracker(conn, &t).await?;
    }
    sqlx::query("DELETE FROM admin_notices WHERE subject = 0 AND NOT (identifier = ANY($1))")
        .bind(&known)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_the_locale_notices() {
        assert_eq!(
            sanitize_notice(
                "Your website is using SSL. But `<a href='/admin/site_settings/category/all_results?filter=force_https'>force_https</a>` is not yet enabled in your site settings."
            )
            .unwrap(),
            "Your website is using SSL. But `<a href=\"/admin/site_settings/category/all_results?filter=force_https\">force_https</a>` is not yet enabled in your site settings."
        );
        assert_eq!(
            sanitize_notice("a <b>bold</b> &amp; <pre>X=false</pre> <span class=\"x\">y</span>")
                .unwrap(),
            "a bold &amp; <pre>X=false</pre> y"
        );
        assert_eq!(
            sanitize_notice("<ul><li><a href=\"/t/1\">Tom&#39;s</a></li></ul>").unwrap(),
            "<ul><li><a href=\"/t/1\">Tom's</a></li></ul>"
        );
    }
}
