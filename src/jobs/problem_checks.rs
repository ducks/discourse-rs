//! The admin dashboard's problem check jobs: Jobs::RunProblemChecks (the
//! scheduled checks' runs and the realtime checks), Jobs::RunProblemCheck
//! (one scheduled check with its retries), Jobs::NotifyAdminsOfProblems and
//! Jobs::CleanupProblemCheckTrackers.

use serde_json::{Value, json};

use crate::notification_create::{self, NewNotification};
use crate::problem_checks::{self, Env, NO_TARGET, Ran};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `Jobs::RunProblemChecks`, every 10 minutes.
pub const RUN_PROBLEM_CHECKS: &str = "run_problem_checks";
/// `Jobs::RunProblemCheck`
pub const RUN_PROBLEM_CHECK: &str = "run_problem_check";
/// `Jobs::NotifyAdminsOfProblems`, every 30 minutes.
pub const NOTIFY_ADMINS_OF_PROBLEMS: &str = "notify_admins_of_problems";
/// `Jobs::CleanupProblemCheckTrackers`, every 10 minutes.
pub const CLEANUP_PROBLEM_CHECK_TRACKERS: &str = "cleanup_problem_check_trackers";

/// `Notification.types[:admin_problems]`
const ADMIN_PROBLEMS: i32 = 38;
/// `Group::AUTO_GROUPS[:admins]`
const ADMINS_GROUP: i32 = 1;

fn env<'a>(state: &'a AppState, settings: &'a SiteSettings) -> Env<'a> {
    Env {
        settings,
        defs: &state.site_setting_defs,
        config: &state.config,
        i18n: &state.i18n,
    }
}

/// `Jobs::RunProblemChecks`: a run_problem_check for each scheduled check
/// and target that is due, then every realtime check.
pub async fn run_problem_checks(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let env = env(state, &settings);
    for check in problem_checks::CHECKS.iter().filter(|c| c.scheduled()) {
        for target in problem_checks::targets(&mut tx, &env, check).await? {
            if target == NO_TARGET && check.targeted {
                continue;
            }
            // ready_to_run?: the tracker (created if need be) is due.
            let tracker = problem_checks::tracker(&mut tx, check.identifier, &target).await?;
            let due = tracker
                .next_run_at
                .is_none_or(|at| at <= crate::clock::now_naive());
            if due {
                crate::jobs::enqueue(
                    &mut tx,
                    RUN_PROBLEM_CHECK,
                    json!({ "check_identifier": check.identifier, "target": target }),
                )
                .await?;
            }
        }
    }
    problem_checks::run_realtime(&mut tx, &env).await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::RunProblemCheck`: runs the check; a problem while retries are
/// left enqueues the job again in `retry_after` with the count one up
/// instead. Errors are logged, not raised (Discourse.warn_exception),
/// except what the port refuses.
pub async fn run_problem_check(state: &AppState, args: &Value) -> Result<(), AppError> {
    let retry_count = match &args["retry_count"] {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => crate::ruby::to_i(s),
        _ => 0,
    };
    let identifier = args["check_identifier"].as_str().unwrap_or_default();
    let target = match &args["target"] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    if crate::ruby::is_blank(&target) {
        return Ok(());
    }
    let Some(check) = problem_checks::find(identifier) else {
        tracing::warn!(
            identifier,
            "a scheduled admin dashboard problem check errored"
        );
        return Ok(());
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let env = env(state, &settings);
    match problem_checks::run(&mut tx, &env, check, &target, Some(retry_count)).await {
        Ok(Ran::Done) => {}
        Ok(Ran::Retry) => {
            let mut next = args.as_object().cloned().unwrap_or_default();
            next.insert("retry_count".into(), json!(retry_count + 1));
            crate::jobs::enqueue_in(
                &mut tx,
                problem_checks::Check::RETRY_AFTER,
                RUN_PROBLEM_CHECK,
                Value::Object(next),
            )
            .await?;
        }
        Err(e) if e.is_unsupported() => return Err(e),
        Err(e) => {
            tracing::warn!(identifier, error = %e, "a scheduled admin dashboard problem check errored");
            return Ok(());
        }
    }
    tx.commit().await?;
    Ok(())
}

/// `Jobs::NotifyAdminsOfProblems`: old admin_problems notifications go;
/// while a watched check has failed since before two days ago, each admin
/// without one gets an admin_problems notification.
pub async fn notify_admins_of_problems(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let gone: Vec<i32> = sqlx::query_scalar(
        "DELETE FROM notifications WHERE notification_type = $1 \
           AND created_at < now() - interval '7 days' RETURNING user_id",
    )
    .bind(ADMIN_PROBLEMS)
    .fetch_all(&mut *tx)
    .await?;
    let mut refreshed: Vec<i32> = Vec::new();
    for user_id in gone {
        if !refreshed.contains(&user_id) {
            refreshed.push(user_id);
            crate::bus::publish_notifications_state(&state.bus, &mut tx, &settings, user_id)
                .await?;
        }
    }
    let persistent: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM problem_check_trackers WHERE ignored_at IS NULL \
           AND last_problem_at > last_success_at AND last_success_at < now() - interval '2 days')",
    )
    .fetch_one(&mut *tx)
    .await?;
    if persistent {
        let users: Vec<i32> = sqlx::query_scalar(
            "SELECT gu.user_id FROM group_users gu WHERE gu.group_id = $1 \
               AND gu.user_id NOT IN (SELECT user_id FROM notifications WHERE notification_type = $2) \
             ORDER BY gu.id",
        )
        .bind(ADMINS_GROUP)
        .bind(ADMIN_PROBLEMS)
        .fetch_all(&mut *tx)
        .await?;
        for user_id in users {
            notification_create::create(
                &mut tx,
                &state.bus,
                &settings,
                &NewNotification::new(ADMIN_PROBLEMS, user_id, json!({})),
            )
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// `Jobs::CleanupProblemCheckTrackers`
pub async fn cleanup_problem_check_trackers(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let env = env(state, &settings);
    problem_checks::cleanup_trackers(&mut tx, &env).await?;
    tx.commit().await?;
    Ok(())
}
