//! Chat's summary emails: Jobs::Chat::EmailNotifications every five
//! minutes (Chat::Mailer.send_unread_mentions_summary: who has unread
//! mentions, direct messages or watched threads while away), and
//! UserNotifications#chat_summary, the email Jobs::UserEmail sends them.

use serde_json::json;
use sqlx::PgConnection;

use crate::email::notification::{Built, html_escape, layout_view};
use crate::email::sender::Ctx;
use crate::guardian::Guardian;
use crate::session::current::SessionUser;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// The scheduled job's name, as the queue knows it.
pub const EMAIL_NOTIFICATIONS_JOB: &str = "Jobs::Chat::EmailNotifications";

/// `UserOption.chat_email_frequencies[:when_away]`
const WHEN_AWAY: i32 = 1;
/// `UserOption.email_level_types[:never]`
const EMAIL_NEVER: i32 = 2;
/// `Chat::NotificationLevels.all[:watching]`
const WATCHING: i32 = 3;

/// Jobs::Chat::EmailNotifications: Chat::Mailer.send_unread_mentions_summary,
/// a chat_summary user_email for each real, active, unstaged, unsuspended
/// user with unreads.
pub async fn send_unread_mentions_summary(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let settings = crate::site_settings::SiteSettings::load(
        &mut tx,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    if !super::enabled(&settings)? {
        return Ok(());
    }
    let allowed = settings.group_ids("chat_allowed_groups")?;
    let groups_join = if allowed.contains(&crate::guardian::auto_groups::EVERYONE)
        || allowed.contains(&crate::guardian::auto_groups::LOGGED_IN_USERS)
    {
        String::new()
    } else {
        let ids: Vec<String> = allowed.iter().map(i64::to_string).collect();
        format!(
            "JOIN group_users gu ON gu.user_id = u.id AND gu.group_id IN ({})",
            if ids.is_empty() {
                "NULL".to_string()
            } else {
                ids.join(",")
            }
        )
    };
    let sql = format!(
        "WITH eligible_users AS NOT MATERIALIZED ( \
           SELECT u.id, uo.allow_private_messages, uo.email_level, uo.chat_email_frequency \
           FROM users u \
           JOIN user_options uo ON uo.user_id = u.id AND uo.chat_enabled \
           {groups_join} \
           WHERE u.last_seen_at < now() - interval '15 minutes' \
         ), unread_dms AS ( \
           SELECT DISTINCT uccm.user_id \
           FROM user_chat_channel_memberships uccm \
           JOIN eligible_users eu ON eu.id = uccm.user_id AND eu.allow_private_messages \
           JOIN chat_messages cm ON cm.chat_channel_id = uccm.chat_channel_id \
             AND cm.deleted_at IS NULL \
             AND (cm.thread_id IS NULL OR cm.thread_id IN (SELECT id FROM chat_threads WHERE original_message_id = cm.id)) \
             AND NOT cm.created_by_sdk \
             AND cm.created_at > now() - interval '1 day' \
           JOIN users sender ON sender.id = cm.user_id \
           JOIN chat_channels cc ON cc.id = cm.chat_channel_id AND cc.deleted_at IS NULL \
             AND cc.chatable_type = 'DirectMessage' \
           WHERE NOT uccm.muted \
             AND (uccm.last_read_message_id IS NULL OR uccm.last_read_message_id < cm.id) \
             AND (uccm.last_unread_mention_when_emailed_id IS NULL OR uccm.last_unread_mention_when_emailed_id < cm.id) \
             AND eu.chat_email_frequency = {WHEN_AWAY} \
         ), unread_mentions AS ( \
           SELECT DISTINCT n.user_id \
           FROM notifications n \
           JOIN eligible_users eu ON eu.id = n.user_id \
           JOIN chat_mention_notifications cmn ON cmn.notification_id = n.id \
           JOIN chat_mentions mn ON mn.id = cmn.chat_mention_id \
           JOIN chat_messages cm ON cm.id = mn.chat_message_id \
             AND cm.deleted_at IS NULL \
             AND cm.thread_id IS NULL \
             AND NOT cm.created_by_sdk \
             AND cm.created_at > now() - interval '1 day' \
           JOIN users sender ON sender.id = cm.user_id \
           JOIN chat_channels cc ON cc.id = cm.chat_channel_id AND cc.deleted_at IS NULL \
             AND cc.chatable_type = 'Category' \
           JOIN user_chat_channel_memberships uccm ON uccm.chat_channel_id = cc.id \
             AND uccm.user_id = n.user_id \
             AND NOT uccm.muted \
             AND uccm.following \
             AND (uccm.last_read_message_id IS NULL OR uccm.last_read_message_id < cm.id) \
             AND (uccm.last_unread_mention_when_emailed_id IS NULL OR uccm.last_unread_mention_when_emailed_id < cm.id) \
           WHERE NOT n.read \
             AND eu.email_level <> {EMAIL_NEVER} \
         ), unread_threads AS ( \
           SELECT DISTINCT uctm.user_id \
           FROM user_chat_thread_memberships uctm \
           JOIN eligible_users eu ON eu.id = uctm.user_id \
           JOIN chat_threads ct ON ct.id = uctm.thread_id \
           JOIN chat_messages cm ON cm.thread_id = ct.id \
             AND cm.deleted_at IS NULL \
             AND NOT cm.created_by_sdk \
             AND cm.created_at > now() - interval '1 day' \
           JOIN users sender ON sender.id = cm.user_id \
           JOIN chat_channels cc ON cc.id = ct.channel_id AND cc.deleted_at IS NULL \
           WHERE uctm.notification_level = {WATCHING} \
             AND (uctm.last_read_message_id IS NULL OR uctm.last_read_message_id < cm.id) \
             AND eu.email_level <> {EMAIL_NEVER} \
         ) \
         SELECT user_id::int FROM unread_dms \
         UNION SELECT user_id::int FROM unread_mentions \
         UNION SELECT user_id::int FROM unread_threads"
    );
    let with_unreads: Vec<i32> = sqlx::query_scalar(&sql).fetch_all(&mut *tx).await?;
    // User.real.activated.not_staged.not_suspended, find_each by id.
    let users: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT u.id, (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\" LIMIT 1) \
         FROM users u WHERE u.id = ANY($1) AND u.id > 0 AND u.active AND NOT u.staged \
           AND (u.suspended_till IS NULL OR u.suspended_till <= now()) ORDER BY u.id",
    )
    .bind(&with_unreads)
    .fetch_all(&mut *tx)
    .await?;
    for (user_id, email) in users {
        crate::jobs::enqueue(
            &mut tx,
            "user_email",
            json!({
                "type": "chat_summary",
                "user_id": user_id,
                "to_address": email,
                "force_respect_seen_recently": true,
            }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// A message the summary shows.
#[derive(sqlx::FromRow)]
struct SummaryMessage {
    id: i64,
    chat_channel_id: i64,
    user_id: Option<i32>,
    cooked: Option<String>,
    created_at: chrono::NaiveDateTime,
}

/// The summary's recipient.
#[derive(sqlx::FromRow)]
struct Recipient {
    last_seen_at: Option<chrono::NaiveDateTime>,
    email_level: i32,
    allow_private_messages: bool,
    chat_email_frequency: i32,
    timezone: Option<String>,
}

/// UserNotifications#chat_summary: None when there's nothing to send.
pub async fn build(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
    to: &str,
) -> Result<Option<Built>, AppError> {
    let s = ctx.settings;
    let Some(session) = SessionUser::load(&mut *conn, user_id).await? else {
        return Ok(None);
    };
    let guardian = Guardian::for_user(&mut *conn, &session).await?;
    if !super::can_chat(&mut *conn, s, &guardian).await? {
        return Ok(None);
    }
    let user: Recipient = sqlx::query_as(
        "SELECT u.last_seen_at, o.email_level, o.allow_private_messages, \
                o.chat_email_frequency, o.timezone \
         FROM users u JOIN user_options o ON o.user_id = u.id WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    let now = crate::clock::now_naive();
    if user
        .last_seen_at
        .is_some_and(|seen| seen > now - chrono::Duration::minutes(15))
    {
        return Ok(None);
    }
    let allow_mentions = user.email_level != EMAIL_NEVER;
    let allow_dms = user.allow_private_messages && user.chat_email_frequency != 0;
    if !allow_mentions && !allow_dms {
        return Ok(None);
    }
    let channels = if allow_mentions {
        let mentions = unread_mentions(conn, user_id).await?;
        messages_for(conn, ctx, &guardian, &mentions).await?
    } else {
        Vec::new()
    };
    let dms = if allow_dms {
        let unread = unread_messages(conn, user_id).await?;
        messages_for(conn, ctx, &guardian, &unread).await?
    } else {
        Vec::new()
    };
    if allow_mentions {
        let watched: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_chat_thread_memberships WHERE user_id = $1 AND notification_level = $2)",
        )
        .bind(user_id)
        .bind(WATCHING)
        .fetch_one(&mut *conn)
        .await?;
        if watched {
            return Err(Unsupported("chat threads").into());
        }
    }
    let count: usize = channels
        .iter()
        .chain(dms.iter())
        .map(|(_, m)| m.len())
        .sum();
    if count == 0 {
        return Ok(None);
    }

    // build_summary_for
    let site_name = s
        .get("email_prefix")?
        .presence()
        .unwrap_or_else(|| s.get("title").map(|t| t.to_s()).unwrap_or_default());
    let urls = Urls {
        config: ctx.config,
        settings: s,
    };
    let base_url = urls.base_url()?;
    let base_url_no_prefix = urls.base_url_no_prefix()?;
    let preferences_path = format!("{base_url}/my/preferences/emails");
    let i18n = ctx.i18n;
    let t = |key: &str, args: &[(&str, &str)]| i18n.t_with(key, args).unwrap_or_default();
    let tc = |key: &str, n: usize, args: &[(&str, &str)]| {
        i18n.t_count(key, n as i64, args).unwrap_or_default()
    };

    // The subject (chat_summary_subject): DMs of more than two members
    // count as channels; watched threads are refused above.
    let subject_key = |kind: &str| {
        let key = format!("user_notifications.chat_summary.subject.{kind}");
        let improved = format!("{key}_improved");
        if s.get("simple_email_subject").is_ok_and(|v| v.truthy()) && i18n.t(&improved).is_some() {
            improved
        } else {
            key
        }
    };
    let private_email = s.get("private_email")?.truthy();
    let subject = if private_email {
        tc(
            &subject_key("private_email"),
            count,
            &[("site_name", &site_name)],
        )
    } else {
        let (one_to_one, group_dms): (Vec<_>, Vec<_>) =
            dms.iter().partition(|(c, _)| c.memberships == 2);
        let as_channels: Vec<&(SummaryChannel, Vec<SummaryMessage>)> =
            channels.iter().chain(group_dms).collect();
        if !as_channels.is_empty() {
            if let Some((dm, _)) = one_to_one.first() {
                t(
                    &subject_key("chat_channel_and_dm"),
                    &[
                        ("site_name", &site_name),
                        ("channel", &as_channels[0].0.title),
                        ("name", &dm.title),
                    ],
                )
            } else {
                match as_channels.len() {
                    1 => tc(
                        &subject_key("chat_channel_1"),
                        as_channels[0].1.len(),
                        &[
                            ("site_name", &site_name),
                            ("channel", &as_channels[0].0.title),
                        ],
                    ),
                    2 => t(
                        &subject_key("chat_channel_2"),
                        &[
                            ("site_name", &site_name),
                            ("channel_1", &as_channels[0].0.title),
                            ("channel_2", &as_channels[1].0.title),
                        ],
                    ),
                    // A plain string: count only interpolates.
                    n => t(
                        &subject_key("chat_channel_3_or_more"),
                        &[
                            ("site_name", &site_name),
                            ("channel", &as_channels[0].0.title),
                            ("count", &(n - 1).to_string()),
                        ],
                    ),
                }
            }
        } else {
            match one_to_one.len() {
                1 => tc(
                    &subject_key("chat_dm_1"),
                    one_to_one[0].1.len(),
                    &[("site_name", &site_name), ("name", &one_to_one[0].0.title)],
                ),
                2 => t(
                    &subject_key("chat_dm_2"),
                    &[
                        ("site_name", &site_name),
                        ("name_1", &one_to_one[0].0.title),
                        ("name_2", &one_to_one[1].0.title),
                    ],
                ),
                n => t(
                    &subject_key("chat_dm_3_or_more"),
                    &[
                        ("site_name", &site_name),
                        ("name", &one_to_one[0].0.title),
                        ("count", &(n - 1).to_string()),
                    ],
                ),
            }
        }
    };

    // chat_summary.text.erb, its links numbered as MarkdownLinker numbers
    // them.
    let link = |url: &str| -> String {
        if url.starts_with(&base_url) {
            url.to_string()
        } else {
            format!("{base_url}{url}")
        }
    };
    let description = tc("user_notifications.chat_summary.description", count, &[]);
    let view_messages = tc(
        "user_notifications.chat_summary.view_messages",
        count,
        &[("count", &count.to_string())],
    );
    let your_settings = t("user_notifications.chat_summary.your_chat_settings", &[]);
    let refs = [link("/"), link("/chat"), link(&preferences_path)];
    let text = format!(
        "{description}\n[{view_messages}][2]\n       {}\n[1]: {}\n[2]: {}\n[3]: {}\n",
        t(
            "user_notifications.chat_summary.unsubscribe_no_link",
            &[
                ("site_link", &format!("[{site_name}][1]")),
                ("email_preferences_link", &format!("[{your_settings}][3]")),
            ],
        ),
        refs[0],
        refs[1],
        refs[2],
    );

    // chat_summary.html.erb, as Erubi renders it.
    // ColorScheme.hex_for_name; nil interpolates as nothing.
    let header_color = crate::color_scheme::hex_for_name(&mut *conn, s, "header_primary")
        .await?
        .unwrap_or_default();
    let header_bgcolor = crate::color_scheme::hex_for_name(&mut *conn, s, "header_background")
        .await?
        .unwrap_or_default();
    let title = s.get("title")?.to_s();
    let logo_url = {
        let digest = crate::site_icons::site_url(&mut *conn, &urls, "digest_logo").await?;
        let logo = if digest.is_empty() || digest.to_lowercase().ends_with(".svg") {
            crate::site_icons::site_url(&mut *conn, &urls, "logo").await?
        } else {
            digest
        };
        Some(logo).filter(|l| !l.is_empty() && !l.to_lowercase().ends_with(".svg"))
    };
    let mut html = format!(
        "<div class=\"summary-email\">\n  <table class=\"chat-summary-header text-header with-dir\" style=\"background-color:#{header_bgcolor};width:100%;min-width:100%;\">\n    <tr>\n      <td align=\"center\" style=\"text-align: center;padding: 20px 0; font-family:Arial,sans-serif;\">\n        <a href=\"{base_url}\" style=\"color:#{header_color};font-size:22px;text-decoration:none;\">\n"
    );
    match &logo_url {
        None => html.push_str(&format!("            {}\n", html_escape(&title))),
        Some(logo) => html.push_str(&format!(
            "            <img src=\"{}\" height=\"40\" style=\"clear:both;display:block;height:40px;margin:auto;max-width:100%;outline:0;text-decoration:none;\" alt=\"{}\">\n",
            html_escape(logo),
            html_escape(&title)
        )),
    }
    html.push_str(&format!(
        "        </a>\n      </td>\n    </tr>\n    <tr>\n      <td align=\"center\" style=\"font-weight:bold;font-size:22px;color:#{header_color}\">\n        {}\n      </td>\n    </tr>\n  </table>\n\n",
        html_escape(&description)
    ));
    let tz: chrono_tz::Tz = user
        .timezone
        .as_deref()
        .and_then(|z| z.parse().ok())
        .unwrap_or(chrono_tz::UTC);
    let prioritize_username = s.get("prioritize_username_in_ux")?.truthy();
    let enable_names = s.get("enable_names")?.truthy();
    let logo_small = crate::admin_users::logo_small_url(&mut *conn, s).await?;
    for (channel, messages) in channels.iter().chain(dms.iter()) {
        let other = messages.len() as i64 - 2;
        html.push_str("      <table class=\"chat-summary-content\" style=\"padding:1em;margin-top:20px;width:100%;min-width:100%;background-color:#f7f7f7;\">\n        <tbody>\n          <tr>\n            <td colspan=\"100%\">\n              <h5 style=\"margin:0.5em 0 0.5em 0;font-size:0.9em;\">\n");
        let heading = if private_email {
            t(
                "system_messages.private_channel_title",
                &[("id", &channel.id.to_string())],
            )
        } else {
            channel.title.clone()
        };
        html.push_str(&format!(
            "                  {}\n              </h5>\n            </td>\n          </tr>\n\n",
            html_escape(&heading)
        ));
        if !private_email {
            for m in messages.iter().take(2) {
                let (name, avatar) = match m.user_id {
                    Some(id) => {
                        let (username, name, uploaded): (String, Option<String>, Option<i32>) =
                            sqlx::query_as(
                                "SELECT username, name, uploaded_avatar_id FROM users WHERE id = $1",
                            )
                            .bind(id)
                            .fetch_one(&mut *conn)
                            .await?;
                        // User#display_name
                        let display = match name.filter(|n| !n.trim().is_empty()) {
                            Some(n) if enable_names && !prioritize_username => n,
                            _ => username.clone(),
                        };
                        let template = crate::avatar::avatar_template(
                            &urls,
                            id,
                            &username,
                            uploaded,
                            logo_small.as_deref(),
                        )?;
                        let template = if template.starts_with('/') && !template.starts_with("//") {
                            format!("{base_url_no_prefix}{template}")
                        } else {
                            template
                        };
                        (display, template.replace("{size}", "45"))
                    }
                    None => return Err(Unsupported("chat messages of deleted users").into()),
                };
                let at = chrono::TimeZone::from_utc_datetime(&tz, &m.created_at);
                let when = at.format("%B %-d, %Y, %l:%M%P").to_string();
                html.push_str(&format!(
                    "              <tr class=\"message-row\">\n                <td style=\"white-space:nowrap;vertical-align:top;padding:1em 0 0 2em\">\n                  <img src=\"{}\" style=\"height:20px;width:20px;margin:0 5px 0 0;border-radius:50%;clear:both;display:inline-block;outline:0;text-decoration:none;vertical-align:top;\" alt=\"{}\">\n                  <span style=\"display:inline-block;color:#0a0a0a;vertical-align:top;font-weight:bold;\">\n                    {}                  </span>\n                  <span style=\"display:inline-block;color:#0a0a0a;font-size:0.8em;\">\n                    {}                  </span>\n                </td>\n              </tr>\n              <tr>\n                <td style=\"width:99%;margin:0;padding:0 0 0 2em;vertical-align:top;\">\n                  {}\n                </td>\n              </tr>\n",
                    html_escape(&avatar),
                    html_escape(&name),
                    html_escape(&name),
                    html_escape(&when),
                    crate::email::digest::email_excerpt(
                        s,
                        &urls,
                        ctx.i18n,
                        m.cooked.as_deref().unwrap_or_default(),
                        None
                    )?
                ));
            }
        }
        let more = if private_email || other <= 0 {
            tc(
                "user_notifications.chat_summary.view_messages",
                messages.len(),
                &[("count", &messages.len().to_string())],
            )
        } else {
            tc(
                "user_notifications.chat_summary.view_more",
                other as usize,
                &[("count", &other.to_string())],
            )
        };
        // Chat::Message#full_url
        html.push_str(&format!(
            "\n          <tr>\n            <td colspan=\"100%\" style=\"padding:2em 0 0 2em\">\n              <a class=\"more-messages-link\" href=\"{base_url_no_prefix}{}/chat/c/-/{}/{}\">\n                  {}\n              </a>\n            </td>\n          </tr>\n        </tbody>\n      </table>\n",
            ctx.config.globals.relative_url_root(),
            channel.id,
            messages[0].id,
            html_escape(&more)
        ));
    }
    html.push_str(&format!(
        "</div>\n\n<table class='summary-footer with-dir' style=\"margin-top:2em;width:100%\">\n  <tr>\n    <td align=\"center\">\n         {}\n    </td>\n  </tr>\n</table>\n",
        t(
            "user_notifications.chat_summary.unsubscribe_no_link",
            &[
                ("site_link", &format!("<a href='{base_url}'>{site_name}</a>")),
                (
                    "email_preferences_link",
                    &format!(
                        "<a href=\"{}\">{}</a>",
                        html_escape(&preferences_path),
                        html_escape(&your_settings)
                    )
                ),
            ],
        )
    ));
    let html_part = layout_view(
        &crate::email::notification::email_template(s)?,
        &html,
        &s.get("default_locale")?.to_s().replacen('_', "-", 1),
        "",
    );

    // Email::MessageBuilder's headers, with the chat_summary.from alias.
    let notification_email = s.get("notification_email")?.to_s();
    let cleanup = |name: &str| name.replace([':', '<', '>', ',', '"'], "");
    let from_alias = t(
        "user_notifications.chat_summary.from",
        &[("site_name", &site_name)],
    );
    let from = format!("\"{}\" <{notification_email}>", cleanup(&from_alias));
    let mut headers = vec![
        ("To".to_string(), to.to_string()),
        ("Subject".to_string(), subject),
        ("From".to_string(), from.clone()),
        ("X-Auto-Response-Suppress".to_string(), "All".to_string()),
        ("x-ms-reactions".to_string(), "disallow".to_string()),
        ("Reply-To".to_string(), from),
    ];
    for item in s.get("email_custom_headers")?.to_s().split('|') {
        if let Some((name, value)) = item.split_once(':') {
            let (name, value) = (name.trim(), value.trim());
            if !name.is_empty() && !value.is_empty() {
                headers.push((name.to_string(), value.to_string()));
            }
        }
    }
    Ok(Some(Built {
        message: crate::email::Message {
            headers,
            text,
            html: None,
        },
        html_part,
        view_rendered: true,
    }))
}

/// A channel the summary shows messages of.
struct SummaryChannel {
    id: i64,
    title: String,
    /// Its memberships (`user_chat_channel_memberships.count`).
    memberships: i64,
}

/// `#unread_messages`: per unmuted DM membership, the first unread message
/// of the last day from someone else not yet emailed (thread replies
/// aside), the membership marked emailed up to the last.
async fn unread_messages(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<Vec<(i64, i64)>, AppError> {
    Ok(sqlx::query_as(
        "WITH unread_messages AS ( \
           SELECT uccm.id membership_id, uccm.chat_channel_id, MIN(chat_messages.id) first_chat_message_id, MAX(chat_messages.id) last_chat_message_id \
           FROM user_chat_channel_memberships uccm \
           JOIN chat_channels ON chat_channels.id = uccm.chat_channel_id \
           JOIN chat_messages ON chat_messages.chat_channel_id = chat_channels.id \
           JOIN users ON users.id = chat_messages.user_id \
           LEFT JOIN chat_threads om ON om.original_message_id = chat_messages.id \
           WHERE uccm.user_id = $1 \
           AND NOT uccm.muted \
           AND chat_channels.deleted_at IS NULL \
           AND chat_channels.chatable_type = 'DirectMessage' \
           AND chat_messages.deleted_at IS NULL \
           AND chat_messages.user_id != uccm.user_id \
           AND chat_messages.created_at > now() - interval '1 day' \
           AND (uccm.last_read_message_id IS NULL OR uccm.last_read_message_id < chat_messages.id) \
           AND (uccm.last_unread_mention_when_emailed_id IS NULL OR uccm.last_unread_mention_when_emailed_id < chat_messages.id) \
           AND (chat_messages.thread_id IS NULL OR chat_messages.thread_id IN (SELECT id FROM chat_threads WHERE original_message_id = chat_messages.id)) \
           GROUP BY uccm.id \
         ) \
         UPDATE user_chat_channel_memberships uccm \
         SET last_unread_mention_when_emailed_id = um.last_chat_message_id \
         FROM unread_messages um \
         WHERE uccm.id = um.membership_id \
         AND uccm.user_id = $1 \
         RETURNING um.chat_channel_id, um.first_chat_message_id",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?)
}

/// `#unread_mentions`: per followed, unmuted category channel, the first
/// unread mention of the user in the last day not yet emailed (the
/// membership marked emailed up to the last).
async fn unread_mentions(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<Vec<(i64, i64)>, AppError> {
    let group_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT groups.id FROM groups JOIN group_users ON group_users.group_id = groups.id \
         WHERE group_users.user_id = $1 AND groups.mentionable_level <> 0",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let group_sql = if group_ids.is_empty() {
        String::new()
    } else {
        let ids: Vec<String> = group_ids.iter().map(i32::to_string).collect();
        format!(
            "WHEN 'Chat::GroupMention' THEN chat_mentions.target_id IN ({})",
            ids.join(",")
        )
    };
    let sql = format!(
        "WITH unread_mentions AS ( \
           SELECT uccm.id membership_id, uccm.chat_channel_id, MIN(chat_messages.id) first_chat_message_id, MAX(chat_messages.id) last_chat_message_id \
           FROM user_chat_channel_memberships uccm \
           JOIN chat_channels ON chat_channels.id = uccm.chat_channel_id \
           JOIN chat_messages ON chat_messages.chat_channel_id = chat_channels.id \
           JOIN chat_mentions ON chat_mentions.chat_message_id = chat_messages.id \
           JOIN chat_mention_notifications cmn ON cmn.chat_mention_id = chat_mentions.id \
           JOIN notifications ON notifications.id = cmn.notification_id \
           JOIN users ON users.id = chat_messages.user_id \
           WHERE uccm.user_id = $1 \
           AND NOT uccm.muted \
           AND uccm.following \
           AND chat_channels.deleted_at IS NULL \
           AND chat_channels.chatable_type = 'Category' \
           AND chat_messages.deleted_at IS NULL \
           AND chat_messages.user_id != uccm.user_id \
           AND chat_messages.created_at > now() - interval '1 day' \
           AND (uccm.last_read_message_id IS NULL OR uccm.last_read_message_id < chat_messages.id) \
           AND (uccm.last_unread_mention_when_emailed_id IS NULL OR uccm.last_unread_mention_when_emailed_id < chat_messages.id) \
           AND ( \
             CASE chat_mentions.type \
             WHEN 'Chat::UserMention' THEN chat_mentions.target_id = $1 \
             WHEN 'Chat::AllMention' THEN chat_channels.allow_channel_wide_mentions = true \
             {group_sql} END \
           ) \
           AND NOT notifications.read \
           GROUP BY uccm.id \
         ) \
         UPDATE user_chat_channel_memberships uccm \
         SET last_unread_mention_when_emailed_id = um.last_chat_message_id \
         FROM unread_mentions um \
         WHERE uccm.id = um.membership_id \
         AND uccm.user_id = $1 \
         RETURNING um.chat_channel_id, um.first_chat_message_id"
    );
    Ok(sqlx::query_as(&sql)
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?)
}

/// `#messages_for`: each channel's messages from its first unread on,
/// oldest first, by channel in the order of their first message, the
/// channels the user can join.
async fn messages_for(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    data: &[(i64, i64)],
) -> Result<Vec<(SummaryChannel, Vec<SummaryMessage>)>, AppError> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let channel_ids: Vec<i64> = data.iter().map(|(c, _)| *c).collect();
    let mins: Vec<i64> = data.iter().map(|(_, m)| *m).collect();
    let messages: Vec<SummaryMessage> = sqlx::query_as(
        "SELECT m.id, m.chat_channel_id, m.user_id, m.cooked, m.created_at FROM chat_messages m \
         JOIN UNNEST($1::bigint[], $2::bigint[]) AS t(channel_id, min_unread_id) ON t.channel_id = m.chat_channel_id \
         WHERE m.deleted_at IS NULL AND m.id >= t.min_unread_id ORDER BY m.created_at ASC",
    )
    .bind(&channel_ids)
    .bind(&mins)
    .fetch_all(&mut *conn)
    .await?;
    let mut out: Vec<(SummaryChannel, Vec<SummaryMessage>)> = Vec::new();
    for m in messages {
        match out.iter_mut().find(|(c, _)| c.id == m.chat_channel_id) {
            Some((_, list)) => list.push(m),
            None => {
                let sql = format!(
                    "SELECT {} FROM chat_channels WHERE id = $1",
                    super::channels::CHANNEL_COLUMNS
                );
                let row: super::channels::ChannelRow = sqlx::query_as(&sql)
                    .bind(m.chat_channel_id)
                    .fetch_one(&mut *conn)
                    .await?;
                let mut cx = super::channels::Context {
                    conn: &mut *conn,
                    settings: ctx.settings,
                    i18n: ctx.i18n,
                    guardian,
                    base_path: ctx.config.globals.relative_url_root(),
                    config: ctx.config,
                };
                // Channel#title(user): a category channel's name or its
                // category's; a DM's name (a group's) or its title for the
                // recipient.
                let title = if row.chatable_type == "DirectMessage" {
                    let dm = cx.direct_message(row.chatable_id).await?;
                    match row.name.clone().filter(|n| !n.is_empty()) {
                        Some(name) if dm.group => name,
                        _ => match cx.dm_title(&row, &dm)? {
                            serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        },
                    }
                } else {
                    match row.name.clone().filter(|n| !n.is_empty()) {
                        Some(name) => name,
                        None => {
                            sqlx::query_scalar("SELECT name FROM categories WHERE id = $1")
                                .bind(row.chatable_id as i32)
                                .fetch_one(&mut *cx.conn)
                                .await?
                        }
                    }
                };
                let joinable = cx.can_join(&row, None).await?;
                let memberships: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM user_chat_channel_memberships WHERE chat_channel_id = $1",
                )
                .bind(row.id)
                .fetch_one(&mut *conn)
                .await?;
                let channel = SummaryChannel {
                    id: row.id,
                    title,
                    memberships,
                };
                let mut list = vec![m];
                if !joinable {
                    list.clear();
                }
                out.push((channel, list));
            }
        }
    }
    // The channels the user can't join, left out.
    out.retain(|(_, list)| !list.is_empty());
    Ok(out)
}
