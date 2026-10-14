//! UserAnonymizer (app/services/user_anonymizer.rb) and
//! Jobs::AnonymizeUser: the user renamed `anon<n>` (UsernameChanger
//! without its job), given a random password, stripped of name, title,
//! avatar, profile, status, keys, tokens, second factors and secondary
//! emails, their primary email made `<username>@anonymized.invalid`, the
//! staff log written, then the username job and the anonymize job (IPs,
//! invites, tokens, email logs, incoming mail, raw emails, user fields).
//!
//! Refused: watched words (user and profile saves), verbose auth token
//! logging, anonymize IPs that are not plain addresses, and the
//! `user_anonymized` and `user_updated` listeners the port doesn't have.

use rand::Rng;
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::posting::Ctx;
use crate::site_settings::SiteSettings;
use crate::username_changer::{self, Changed, Update};
use crate::{AppError, AppState, Unsupported};

/// `UserAnonymizer::EMAIL_SUFFIX`
pub const EMAIL_SUFFIX: &str = "@anonymized.invalid";
/// `Jobs::AnonymizeUser`
pub const ANONYMIZE_USER: &str = "anonymize_user";
/// `UserHistory.actions[:anonymize_user]`
const ANONYMIZE_USER_ACTION: i32 = 24;
/// `UserHistory.actions[:delete_user]`
const DELETE_USER_ACTION: i32 = 1;
/// `UserOption.email_level_types[:never]`
const NEVER: i32 = 2;
/// `Group::AUTO_GROUPS[:trust_level_0]`
const TRUST_LEVEL_0: i32 = 10;

/// `Guardian#can_anonymize_user?` for staff: not staff, not anonymized.
pub async fn can_anonymize(conn: &mut PgConnection, user_id: i32) -> Result<bool, AppError> {
    let row: Option<(bool, bool, Option<String>)> = sqlx::query_as(
        "SELECT admin, moderator, \
                (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\") \
         FROM users u WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(conn)
    .await?;
    Ok(match row {
        Some((admin, moderator, email)) => {
            !admin && !moderator && !email.is_some_and(|e| e.ends_with(EMAIL_SUFFIX))
        }
        None => false,
    })
}

/// `make_anon_username`: `anon` and a random number below 10^8, free.
async fn anon_username(conn: &mut PgConnection) -> Result<String, AppError> {
    for _ in 0..100 {
        let n: u32 = rand::thread_rng().gen_range(0..100_000_000);
        let candidate = format!("anon{n}");
        let taken: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE username_lower = $1)")
                .bind(&candidate)
                .fetch_one(&mut *conn)
                .await?;
        if !taken {
            return Ok(candidate);
        }
    }
    Err(std::io::Error::other("Failed to generate an anon username").into())
}

/// The user's after_save callbacks the anonymizer's saves reach:
/// `expire_tokens_if_password_changed` when the password was set, then
/// clear_global_notice, refresh_avatar, badge_grant and index_search.
pub(crate) async fn user_save_callbacks(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user_id: i32,
    password_changed: bool,
) -> Result<(), AppError> {
    if password_changed {
        expire_auth_tokens(&mut *conn, s, user_id).await?;
        sqlx::query("UPDATE email_tokens SET expired = TRUE WHERE user_id = $1 AND NOT expired")
            .bind(user_id)
            .execute(&mut *conn)
            .await?;
    }
    let (username_lower, name, admin_active): (String, Option<String>, bool) =
        sqlx::query_as("SELECT username_lower, name, admin AND active FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    // refresh_avatar: `user_avatar || create_user_avatar`
    let avatar: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_avatars WHERE user_id = $1)")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    if !avatar {
        sqlx::query(
            "INSERT INTO user_avatars (user_id, created_at, updated_at) \
             VALUES ($1, clock_timestamp(), clock_timestamp())",
        )
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    }
    crate::user_updater::after_save(
        conn,
        s,
        user_id,
        admin_active,
        &username_lower,
        name.as_deref(),
    )
    .await
}

/// `UserAuthToken.where(user_id:).destroy_all`: each row's before_destroy
/// logs only with verbose logging.
async fn expire_auth_tokens(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    let tokens: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_auth_tokens WHERE user_id = $1)")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    if tokens && s.get("verbose_auth_token_logging")?.truthy() {
        return Err(Unsupported("verbose auth token logging on destroy").into());
    }
    sqlx::query("DELETE FROM user_auth_tokens WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// The listeners of an event the port doesn't have.
async fn event_unsupported(conn: &mut PgConnection, event: &str) -> Result<(), AppError> {
    if crate::web_hooks::active(conn, event).await? {
        return Err(Unsupported("user web hooks").into());
    }
    let automations: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM discourse_automation_automations \
         WHERE enabled AND trigger = $1)",
    )
    .bind(event)
    .fetch_one(&mut *conn)
    .await?;
    if automations {
        return Err(Unsupported("automations on user events").into());
    }
    let workflows: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM discourse_workflows_workflows \
         WHERE active_version_id IS NOT NULL AND nodes::text LIKE '%trigger:' || $1 || '%')",
    )
    .bind(event)
    .fetch_one(&mut *conn)
    .await?;
    if workflows {
        return Err(Unsupported("discourse-workflows triggers on user events").into());
    }
    Ok(())
}

/// `UserAnonymizer.new(user, actor, opts).make_anonymous`: the new
/// username.
pub async fn make_anonymous(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
    actor: Option<i32>,
    anonymize_ip: Option<&str>,
) -> Result<String, AppError> {
    let s = ctx.settings;
    if let Some(ip) = anonymize_ip
        && ip.parse::<std::net::IpAddr>().is_err()
    {
        return Err(Unsupported("anonymizing with an IP that is not an address").into());
    }
    event_unsupported(&mut *conn, "user_anonymized").await?;
    // Watched words: the user's custom fields are clean on its save
    // (anonymize_user_fields deletes them after), and the profile's
    // location and bio become nil.

    // UserEmail.where(user_id:).pluck(:email) | the associated accounts'
    let mut prev_emails: Vec<String> =
        sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 ORDER BY id")
            .bind(user_id)
            .fetch_all(&mut *conn)
            .await?;
    let associated: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT info->>'email' FROM user_associated_accounts WHERE user_id = $1 ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    for email in associated.into_iter().flatten() {
        if !prev_emails.contains(&email) {
            prev_emails.push(email);
        }
    }
    let mut seen = Vec::new();
    prev_emails.retain(|e| {
        let first = !seen.contains(e);
        seen.push(e.clone());
        first
    });
    let (prev_username, old_name, old_title, old_avatar): (
        String,
        Option<String>,
        Option<String>,
        Option<i32>,
    ) = sqlx::query_as("SELECT username, name, title, uploaded_avatar_id FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;

    let username = anon_username(&mut *conn).await?;
    match username_changer::change(&mut *conn, ctx, user_id, &username, None, Update::Skip).await? {
        Changed::Changed => {}
        _ => return Err(std::io::Error::other("Failed to change username").into()),
    }

    // @user.save!
    let name =
        (s.get("full_name_requirement")?.to_s() == "required_at_signup").then(|| username.clone());
    if old_title.is_some() {
        // check_if_title_is_badged_granted: the profile's update! with
        // its callbacks (trigger_badges).
        sqlx::query(
            "UPDATE user_profiles SET granted_title_badge_id = NULL \
             WHERE user_id = $1 AND granted_title_badge_id IS NOT NULL",
        )
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
        crate::badge_granter::queue_user_change(&mut *conn, s, user_id).await?;
    }
    sqlx::query(
        "UPDATE users SET name = $2, date_of_birth = NULL, title = NULL, uploaded_avatar_id = NULL, \
                          ip_address = COALESCE($3::inet, ip_address), \
                          registration_ip_address = COALESCE($3::inet, registration_ip_address), \
                          updated_at = clock_timestamp() \
         WHERE id = $1",
    )
    .bind(user_id)
    .bind(&name)
    .bind(anonymize_ip)
    .execute(&mut *conn)
    .await?;
    crate::accounts::set_password(&mut *conn, user_id, &crate::accounts::random_hex()).await?;
    // after_update: trigger_user_updated_event (a human's avatar), then
    // change_display_name.
    if old_avatar.is_some() && user_id > 0 {
        event_unsupported(&mut *conn, "user_updated").await?;
    }
    if old_name != name {
        crate::jobs::enqueue(
            &mut *conn,
            "change_display_name",
            json!({ "user_id": user_id, "old_name": old_name, "new_name": name }),
        )
        .await?;
    }
    user_save_callbacks(&mut *conn, s, user_id, true).await?;
    if old_avatar.is_some() {
        // refresh_avatar: the quoted posts rebaked for the new avatar.
        crate::jobs::enqueue(
            &mut *conn,
            "rebake_quoted_posts_for_user",
            json!({ "user_id": user_id }),
        )
        .await?;
    }

    // primary_email.update_attribute(:email), then (:normalized_email):
    // the tokens of the old address go first.
    let email = format!("{username}{EMAIL_SUFFIX}");
    let old_email: Option<String> =
        sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(old_email) = old_email else {
        return Err(Unsupported("anonymizing a user without a primary email").into());
    };
    if old_email != email {
        sqlx::query("DELETE FROM email_tokens WHERE email = $1")
            .bind(&old_email)
            .execute(&mut *conn)
            .await?;
    }
    sqlx::query(
        "UPDATE user_emails SET email = $2, normalized_email = $2, updated_at = clock_timestamp() \
         WHERE user_id = $1 AND \"primary\"",
    )
    .bind(user_id)
    .bind(&email)
    .execute(&mut *conn)
    .await?;

    // user_option.save!
    sqlx::query(
        "UPDATE user_options SET mailing_list_mode = FALSE, email_digests = FALSE, \
                                 email_level = $2, email_messages_level = $2 \
         WHERE user_id = $1",
    )
    .bind(user_id)
    .bind(NEVER)
    .execute(&mut *conn)
    .await?;

    // user_profile.update!(...)
    #[derive(sqlx::FromRow)]
    struct Profile {
        bio_raw: Option<String>,
        profile_background_upload_id: Option<i32>,
        card_background_upload_id: Option<i32>,
    }
    let profile: Option<Profile> = sqlx::query_as(
        "SELECT bio_raw, profile_background_upload_id, card_background_upload_id \
         FROM user_profiles WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(profile) = profile {
        sqlx::query(
            "UPDATE user_profiles SET location = NULL, website = NULL, bio_raw = NULL, \
                                      bio_cooked = NULL, profile_background_upload_id = NULL, \
                                      card_background_upload_id = NULL \
             WHERE user_id = $1",
        )
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
        // trigger_badges
        crate::badge_granter::queue_user_change(&mut *conn, s, user_id).await?;
        // pull_hotlinked_image
        if profile.bio_raw.is_some() {
            let grace = s.get("editing_grace_period")?.to_i();
            crate::jobs::enqueue_in(
                &mut *conn,
                grace,
                "pull_user_profile_hotlinked_images",
                json!({ "user_id": user_id }),
            )
            .await?;
        }
        // UploadReference.ensure_exist!(upload_ids: [] ...): all of them go.
        if profile.bio_raw.is_some()
            || profile.profile_background_upload_id.is_some()
            || profile.card_background_upload_id.is_some()
        {
            sqlx::query(
                "DELETE FROM upload_references WHERE target_type = 'UserProfile' AND target_id = $1",
            )
            .bind(user_id)
            .execute(&mut *conn)
            .await?;
        }
    }

    // clear_status!
    sqlx::query("DELETE FROM user_statuses WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    let (hide_profile, silenced): (bool, bool) = sqlx::query_as(
        "SELECT COALESCE((SELECT hide_profile FROM user_options WHERE user_id = u.id), FALSE), \
                COALESCE(silenced_till > now(), FALSE) \
         FROM users u WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if !hide_profile && !silenced {
        let mut message = serde_json::Map::new();
        message.insert(user_id.to_string(), Value::Null);
        ctx.bus
            .publish(
                &mut *conn,
                "/user-status",
                &Value::Object(message),
                Some(&[crate::bus::group_tag(TRUST_LEVEL_0)]),
            )
            .await?;
    }

    for sql in [
        // user_avatar&.destroy!
        "DELETE FROM upload_references WHERE target_type = 'UserAvatar' AND target_id IN \
           (SELECT id FROM user_avatars WHERE user_id = $1)",
        "DELETE FROM user_avatars WHERE user_id = $1",
        "DELETE FROM single_sign_on_records WHERE user_id = $1",
        "DELETE FROM oauth2_user_infos WHERE user_id = $1",
        "DELETE FROM user_associated_accounts WHERE user_id = $1",
        "DELETE FROM api_keys WHERE user_id = $1",
        "DELETE FROM user_api_key_scopes WHERE user_api_key_id IN \
           (SELECT id FROM user_api_keys WHERE user_id = $1)",
        "DELETE FROM user_api_keys WHERE user_id = $1",
    ] {
        sqlx::query(sql).bind(user_id).execute(&mut *conn).await?;
    }
    expire_auth_tokens(&mut *conn, s, user_id).await?;
    for sql in [
        "DELETE FROM user_second_factors WHERE user_id = $1",
        "DELETE FROM user_security_keys WHERE user_id = $1",
        "DELETE FROM push_subscriptions WHERE user_id = $1",
        "DELETE FROM post_reply_keys WHERE user_id = $1",
        // user_emails.secondary.destroy_all: each one's tokens after.
        "WITH gone AS (DELETE FROM user_emails WHERE user_id = $1 AND NOT \"primary\" RETURNING email) \
         DELETE FROM email_tokens WHERE email IN (SELECT email FROM gone)",
    ] {
        sqlx::query(sql).bind(user_id).execute(&mut *conn).await?;
    }

    // log_action
    let details = s.get("log_anonymizer_details")?.truthy();
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, email, details, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(ANONYMIZE_USER_ACTION)
    .bind(actor.unwrap_or(user_id))
    .bind(user_id)
    .bind(details.then(|| prev_emails.first().cloned()).flatten())
    .bind(details.then(|| format!("username: {prev_username}")))
    .execute(&mut *conn)
    .await?;

    // UsernameChanger.update_username with `avatar_template`, not its url.
    let urls = crate::url::Urls {
        config: ctx.config,
        settings: s,
    };
    let avatar_template = crate::avatar::avatar_template(&urls, user_id, &username, None, None)
        .map_err(|e| match e {
            crate::avatar::AvatarError::Unsupported(u) => AppError::from(u),
            other => AppError::from(other),
        })?;
    crate::jobs::enqueue(
        &mut *conn,
        username_changer::UPDATE_USERNAME,
        json!({
            "user_id": user_id,
            "old_username": prev_username,
            "new_username": username,
            "avatar_template": avatar_template,
        }),
    )
    .await?;
    crate::jobs::enqueue(
        &mut *conn,
        ANONYMIZE_USER,
        json!({
            "user_id": user_id,
            "prev_emails": prev_emails,
            "prev_username": prev_username,
            "anonymize_ip": anonymize_ip,
        }),
    )
    .await?;
    Ok(username)
}

/// `Jobs::AnonymizeUser#execute`
pub async fn anonymize_user_job(state: &AppState, args: &Value) -> Result<(), AppError> {
    let user_id = args
        .get("user_id")
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .ok_or(Unsupported("anonymize_user without a user_id"))? as i32;
    let prev_emails: Vec<String> = match args.get("prev_emails") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str().map(str::to_string).ok_or(Unsupported(
                    "anonymize_user with emails that are not strings",
                ))
            })
            .collect::<Result<_, _>>()?,
        Some(_) => {
            return Err(Unsupported("anonymize_user with emails that are not a list").into());
        }
    };
    let prev_username = args.get("prev_username").and_then(Value::as_str);
    let anonymize_ip = args
        .get("anonymize_ip")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let mut tx = state.pool.begin().await?;
    let s = SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let conn = &mut *tx;

    if let Some(ip) = anonymize_ip {
        let ip_where = |column: &str| format!("{column} = $1 AND ip_address IS NOT NULL");
        for (table, column) in [
            ("incoming_links", "current_user_id"),
            ("search_logs", "user_id"),
            ("topic_link_clicks", "user_id"),
            ("topic_views", "user_id"),
            ("user_histories", "acting_user_id"),
            ("user_profile_views", "user_id"),
        ] {
            if table == "search_logs" {
                // ScreenedEmail.where(email: @prev_emails), in Rails' order.
                sqlx::query(
                    "UPDATE screened_emails SET ip_address = $1::inet WHERE email = ANY($2)",
                )
                .bind(ip)
                .bind(&prev_emails)
                .execute(&mut *conn)
                .await?;
            }
            sqlx::query(&format!(
                "UPDATE {table} SET ip_address = $2::inet WHERE {}",
                ip_where(column)
            ))
            .bind(user_id)
            .bind(ip)
            .execute(&mut *conn)
            .await?;
        }
        sqlx::query("DELETE FROM user_ip_address_histories WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *conn)
            .await?;
        sqlx::query(
            "UPDATE user_histories SET ip_address = $3::inet WHERE action = $1 AND details LIKE $2",
        )
        .bind(DELETE_USER_ACTION)
        .bind(format!("id: {user_id}\n%"))
        .bind(ip)
        .execute(&mut *conn)
        .await?;
    }

    if let Some(prev) = prev_username
        && !s.get("log_anonymizer_details")?.truthy()
    {
        // anonymize_username
        let escaped = prev
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        let reason = state.i18n.t("user.anonymized").unwrap_or("").to_string();
        sqlx::query(
            "UPDATE user_histories SET \
               context = CASE WHEN context LIKE $2 THEN $3 ELSE context END, \
               details = CASE WHEN details LIKE $2 THEN $3 ELSE details END, \
               previous_value = CASE WHEN previous_value LIKE $2 THEN $3 ELSE previous_value END, \
               new_value = CASE WHEN new_value LIKE $2 THEN $3 ELSE new_value END \
             WHERE target_user_id = $1 AND (context LIKE $2 OR details LIKE $2 \
               OR previous_value LIKE $2 OR new_value LIKE $2)",
        )
        .bind(user_id)
        .bind(&pattern)
        .bind(&reason)
        .execute(&mut *conn)
        .await?;
    }

    // Invite.where(email:).destroy_all (live ones, Trashable's scope)
    sqlx::query("DELETE FROM invites WHERE email = ANY($1) AND deleted_at IS NULL")
        .bind(&prev_emails)
        .execute(&mut *conn)
        .await?;
    // InvitedUser destroy: its invite's redemption count down.
    sqlx::query(
        "WITH gone AS (DELETE FROM invited_users WHERE user_id = $1 RETURNING invite_id) \
         UPDATE invites SET redemption_count = redemption_count - 1 \
         WHERE id IN (SELECT invite_id FROM gone) AND redemption_count > 0",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    for sql in [
        "DELETE FROM email_tokens WHERE user_id = $1",
        "DELETE FROM email_logs WHERE user_id = $1",
    ] {
        sqlx::query(sql).bind(user_id).execute(&mut *conn).await?;
    }
    sqlx::query("DELETE FROM incoming_emails WHERE user_id = $1 OR from_address = ANY($2)")
        .bind(user_id)
        .bind(&prev_emails)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE posts SET raw_email = NULL WHERE user_id = $1 AND raw_email IS NOT NULL")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;

    // anonymize_user_fields
    let field_ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM user_fields ORDER BY id")
        .fetch_all(&mut *conn)
        .await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    if !field_ids.is_empty() && exists {
        let names: Vec<String> = field_ids
            .iter()
            .map(|id| format!("user_field_{id}"))
            .collect();
        sqlx::query("DELETE FROM user_custom_fields WHERE user_id = $1 AND name = ANY($2)")
            .bind(user_id)
            .bind(&names)
            .execute(&mut *conn)
            .await?;
        // user.save!
        user_save_callbacks(&mut *conn, &s, user_id, false).await?;
    }
    tx.commit().await?;
    Ok(())
}
