//! Bulk invites from a CSV file: `Jobs::BulkInvite` (each row's email
//! invited to the forum, its groups and topic, an existing user added to
//! the groups instead, then a system message with what happened) and
//! `Jobs::ProcessBulkInviteEmails` (the emails of a large upload, two
//! hundred a minute).

use serde_json::{Value, json};

use crate::guardian::Guardian;
use crate::invites::{self, Generate, Generated, Inviter, Limits, emailed_status};
use crate::session::current::SessionUser;
use crate::{AppError, AppState, Unsupported};

pub const BULK_INVITE: &str = "bulk_invite";
pub const PROCESS_BULK_INVITE_EMAILS: &str = "process_bulk_invite_emails";

/// `Invite::BULK_INVITE_EMAIL_LIMIT`
const BULK_INVITE_EMAIL_LIMIT: usize = 200;

/// What the job counts and logs.
#[derive(Default)]
struct Tally {
    logs: Vec<String>,
    sent: i64,
    skipped: i64,
    skipped_emails: Vec<String>,
    warnings: i64,
    failed: i64,
    failed_emails: Vec<String>,
}

impl Tally {
    /// `save_log`: `[Time.now] message`.
    fn log(&mut self, message: String) {
        let now = crate::clock::now_naive().format("%Y-%m-%d %H:%M:%S +0000");
        self.logs.push(format!("[{now}] {message}"));
    }
}

fn arg<'a>(invite: &'a Value, key: &str) -> Option<&'a str> {
    invite
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !crate::ruby::is_blank(v))
}

/// `Jobs::BulkInvite`
pub async fn bulk_invite(state: &AppState, args: &Value) -> Result<(), AppError> {
    let invites = args
        .get("invites")
        .and_then(Value::as_array)
        .filter(|i| !i.is_empty())
        .ok_or(Unsupported(
            "bulk_invite without invites (InvalidParameters)",
        ))?;
    let user_id = args
        .get("current_user_id")
        .and_then(Value::as_i64)
        .ok_or(Unsupported(
            "bulk_invite without a user (InvalidParameters)",
        ))? as i32;
    let mut conn = state.pool.acquire().await?;
    let s = crate::site_settings::SiteSettings::load(
        &mut conn,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    let Some(user) = SessionUser::load(&mut conn, user_id).await? else {
        return Err(Unsupported("bulk_invite for a user that is gone (InvalidParameters)").into());
    };
    let guardian = Guardian::for_user(&mut conn, &user).await?;
    let inviter = Inviter::load(&mut conn, user_id)
        .await?
        .ok_or(Unsupported("an inviter who is gone"))?;
    drop(conn);
    let skip_email = s.get("skip_email_bulk_invites")?.truthy();
    let large = invites.len() > BULK_INVITE_EMAIL_LIMIT;
    let host = crate::pretty_text::Host::from_state(state);
    let promotion = crate::promotion::Ctx {
        settings: &s,
        i18n: &state.i18n,
        host: &host,
        config: &state.config,
    };
    let cx = crate::group_manager::Cx {
        state,
        promotion: &promotion,
    };
    let limits = Limits {
        pool: &state.pool,
        enabled: state.config.rate_limits,
    };
    let mut tally = Tally::default();
    // fetch_group and can_edit_group?, cached by lowercased name.
    let mut groups_seen: Vec<(String, Option<crate::groups::BasicGroup>, bool)> = Vec::new();
    for invite in invites {
        let email = arg(invite, "email").unwrap_or_default().to_string();
        if !crate::accounts::valid_email(&email) {
            tally.log(format!("Invalid Email '{email}"));
            tally.failed += 1;
            tally.failed_emails.push(email);
            continue;
        }
        let mut tx = state.pool.begin().await?;
        // get_groups
        let mut groups = Vec::new();
        if let Some(names) = arg(invite, "groups") {
            for name in names.split(';') {
                let lower = name.to_lowercase();
                let cached = groups_seen.iter().find(|(n, _, _)| *n == lower).cloned();
                let (group, editable) = match cached {
                    Some((_, group, editable)) => (group, editable),
                    None => {
                        let group: Option<crate::groups::BasicGroup> = sqlx::query_as(&format!(
                            "SELECT {} FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id \
                             WHERE lower(g.name) = $1 ORDER BY g.id LIMIT 1",
                            crate::groups::BASIC_GROUP_COLUMNS
                        ))
                        .bind(&lower)
                        .fetch_optional(&mut *tx)
                        .await?;
                        let editable = match &group {
                            Some(g) => {
                                let membership =
                                    crate::groups::membership(&mut tx, &guardian, g.id).await?;
                                crate::groups::can_edit_group(&guardian, &s, g, membership)?
                            }
                            None => false,
                        };
                        groups_seen.push((lower.clone(), group.clone(), editable));
                        (group, editable)
                    }
                };
                match group {
                    Some(g) if editable => groups.push(g),
                    _ => {
                        tally.log(format!("Invalid Group '{name}' for '{email}'"));
                        tally.warnings += 1;
                    }
                }
            }
        }
        // get_topic
        let mut topic_id = None;
        if let Some(raw) = arg(invite, "topic_id") {
            let id = crate::ruby::to_i(raw) as i32;
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND deleted_at IS NULL)",
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
            if exists {
                topic_id = Some(id);
            } else {
                tally.log(format!("Invalid Topic ID '{raw}' for '{email}'"));
                tally.warnings += 1;
            }
        }
        let extra = invite.as_object().is_some_and(|o| {
            o.keys()
                .any(|k| !["email", "groups", "topic_id"].contains(&k.as_str()))
        });
        if extra {
            return Err(Unsupported("user fields and locales in bulk invites").into());
        }
        match invites::find_user_by_email(&mut tx, &email).await? {
            Some(existing) => {
                for group in &groups {
                    let Some(managed) = crate::group_manager::find(&mut tx, group.id).await? else {
                        continue;
                    };
                    crate::group_manager::add(&mut tx, &cx, &managed, existing, user_id).await?;
                }
                tally.skipped += 1;
                tally.skipped_emails.push(email.clone());
            }
            None => {
                let opts = Generate {
                    email: Some(email.clone()),
                    topic_id,
                    group_ids: groups.iter().map(|g| g.id).collect(),
                    skip_email,
                    emailed_status: large.then_some(emailed_status::BULK_PENDING),
                    ..Generate::default()
                };
                match invites::generate(&mut tx, &s, &state.i18n, &limits, &inviter, &opts).await? {
                    Generated::Invite(_) => tally.sent += 1,
                    Generated::UserExists(message) | Generated::Invalid(message) => {
                        tally.log(format!("Error inviting '{email}' -- {message}"));
                        tally.failed += 1;
                        tally.failed_emails.push(email.clone());
                    }
                    Generated::RateLimited(refused) => {
                        tally.log(format!(
                            "Error inviting '{email}' -- {}",
                            refused.description(&state.i18n)
                        ));
                        tally.failed += 1;
                        tally.failed_emails.push(email.clone());
                    }
                }
            }
        }
        tx.commit().await?;
    }
    if large {
        let mut conn = state.pool.acquire().await?;
        crate::jobs::enqueue(&mut conn, PROCESS_BULK_INVITE_EMAILS, json!({})).await?;
    }
    // notify_user
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let mut params = vec![
        ("sent".to_string(), tally.sent.to_string()),
        ("skipped".to_string(), tally.skipped.to_string()),
        (
            "skipped_emails".to_string(),
            tally.skipped_emails.join("\n"),
        ),
        ("warnings".to_string(), tally.warnings.to_string()),
    ];
    let message_type = if tally.sent > 0 && tally.failed == 0 {
        "bulk_invite_succeeded"
    } else {
        params.push(("failed".to_string(), tally.failed.to_string()));
        params.push(("failed_emails".to_string(), tally.failed_emails.join("\n")));
        "bulk_invite_failed"
    };
    params.push(("logs".to_string(), tally.logs.join("\n")));
    crate::system_message::create_from_system_user(
        &state.pool,
        &ctx,
        user_id,
        message_type,
        &params,
    )
    .await?;
    Ok(())
}

/// `Jobs::ProcessBulkInviteEmails`
pub async fn process_bulk_invite_emails(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM invites WHERE emailed_status = $1 AND deleted_at IS NULL LIMIT $2",
    )
    .bind(emailed_status::BULK_PENDING)
    .bind(BULK_INVITE_EMAIL_LIMIT as i64)
    .fetch_all(&mut *tx)
    .await?;
    if !ids.is_empty() {
        sqlx::query("UPDATE invites SET emailed_status = $2 WHERE id = ANY($1)")
            .bind(&ids)
            .bind(emailed_status::SENDING)
            .execute(&mut *tx)
            .await?;
        for id in &ids {
            crate::jobs::enqueue(
                &mut tx,
                crate::invite_mailer::INVITE_EMAIL,
                json!({"invite_id": id}),
            )
            .await?;
        }
        crate::jobs::enqueue_in(&mut tx, 60, PROCESS_BULK_INVITE_EMAILS, json!({})).await?;
    }
    tx.commit().await?;
    Ok(())
}
