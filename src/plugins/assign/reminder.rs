//! PendingAssignsReminder: a system message listing a user's newest and
//! oldest assigned topics they can see, once they hold
//! pending_assign_reminder_threshold of them, replacing the previous
//! reminder still unanswered.

use chrono::NaiveDateTime;
use sqlx::PgConnection;

use crate::{AppError, AppState, Unsupported};

/// `PendingAssignsReminder::REMINDED_AT`
pub const REMINDED_AT: &str = "last_reminded_at";
/// `PendingAssignsReminder::CUSTOM_FIELD_NAME`
pub const CUSTOM_FIELD_NAME: &str = "assigns_reminder";

/// `visible_topics(user)`: listable topics the user's guardian may see,
/// or their messages, as a condition over `topics`.
async fn visible_topics(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &crate::site_settings::SiteSettings,
    user_id: i32,
) -> Result<String, AppError> {
    let Some(guardian) = super::assigner::guardian_for(&mut *conn, user_id).await? else {
        return Err(Unsupported("a reminder for a user who is gone").into());
    };
    let _ = state;
    // Topic.listable_topics.secured(guardian)
    let secured = match guardian
        .visible_topic_clause(&mut *conn, settings, false)
        .await?
    {
        Some(clause) => clause,
        None => "TRUE".to_string(),
    };
    Ok(format!(
        "((topics.archetype <> 'private_message' AND topics.deleted_at IS NULL AND ({secured})) \
          OR (topics.archetype = 'private_message' AND topics.deleted_at IS NULL AND topics.id IN (\
            SELECT topic_id FROM topic_allowed_users WHERE user_id = {user_id} \
            UNION SELECT tg.topic_id FROM topic_allowed_groups tg \
              JOIN group_users gu ON gu.group_id = tg.group_id WHERE gu.user_id = {user_id})))"
    ))
}

#[derive(sqlx::FromRow)]
struct AssignedTopic {
    id: i32,
    slug: Option<String>,
    fancy_title: Option<String>,
    title: String,
    assigned_at: NaiveDateTime,
}

/// `assigned_topics(user, order:)`: three, by when they were assigned.
async fn assigned_topics(
    conn: &mut PgConnection,
    settings: &crate::site_settings::SiteSettings,
    visible: &str,
    user_id: i32,
    order: &str,
    except: &[i32],
) -> Result<Vec<AssignedTopic>, AppError> {
    // assigns_reminder_assigned_topics_query (discourse-solved's modifier)
    let solved = if crate::plugins::solved::enabled(settings)?
        && settings
            .get("ignore_solved_topics_in_assigned_reminder")?
            .truthy()
    {
        " AND topics.id NOT IN (SELECT topic_id FROM discourse_solved_solved_topics)"
    } else {
        ""
    };
    Ok(sqlx::query_as(&format!(
        "SELECT topics.id, topics.slug, topics.fancy_title, topics.title, \
                assignments.created_at AS assigned_at \
         FROM topics JOIN assignments ON assignments.target_id = topics.id \
           AND assignments.target_type = 'Topic' \
         WHERE assignments.assigned_to_id = $1 AND assignments.assigned_to_type = 'User' \
           AND assignments.active AND {visible}{solved} AND NOT (topics.id = ANY($2)) \
         ORDER BY assignments.created_at {order} LIMIT 3"
    ))
    .bind(user_id)
    .bind(except)
    .fetch_all(conn)
    .await?)
}

/// `remind(user)`
pub async fn remind(
    conn: &mut PgConnection,
    state: &AppState,
    user_id: i32,
) -> Result<(), AppError> {
    let settings = crate::site_settings::SiteSettings::load(
        &mut *conn,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    let visible = visible_topics(conn, state, &settings, user_id).await?;
    let newest = assigned_topics(conn, &settings, &visible, user_id, "DESC", &[]).await?;
    if (newest.len() as i64) < settings.get("pending_assign_reminder_threshold")?.to_i() {
        return Ok(());
    }
    let newest_ids: Vec<i32> = newest.iter().map(|t| t.id).collect();
    let oldest = assigned_topics(conn, &settings, &visible, user_id, "ASC", &newest_ids).await?;

    // assigned_count_for(user), with discourse-solved's
    // assigned_count_for_user_query modifier.
    let status_on_solve = settings.get("assignment_status_on_solve")?.to_s();
    let solved_status = if crate::plugins::solved::enabled(&settings)?
        && settings
            .get("ignore_solved_topics_in_assigned_reminder")?
            .truthy()
        && !status_on_solve.is_empty()
    {
        format!(
            " AND assignments.status IS DISTINCT FROM {}",
            crate::user_search::quote(&status_on_solve)
        )
    } else {
        String::new()
    };
    let count: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM assignments \
         INNER JOIN topics ON topics.id = assignments.target_id AND assignments.target_type = 'Topic' \
           AND topics.deleted_at IS NULL \
         WHERE assignments.assigned_to_id = $1 AND assignments.assigned_to_type = 'User' \
           AND assignments.active AND {visible}{solved_status}"
    ))
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;

    let (username_lower, frequency): (String, Option<String>) = sqlx::query_as(
        "SELECT u.username_lower, (SELECT value FROM user_custom_fields \
           WHERE user_id = u.id AND name = $2 ORDER BY id LIMIT 1) \
         FROM users u WHERE u.id = $1",
    )
    .bind(user_id)
    .bind(super::REMINDERS_FREQUENCY)
    .fetch_one(&mut *conn)
    .await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let now = crate::clock::now_naive();
    let base_path = state.config.globals.relative_url_root().to_string();
    let denied = settings.emojis().denied.clone();
    let list = |key: &str, topics: &[AssignedTopic]| -> String {
        if topics.is_empty() {
            return String::new();
        }
        let mut items = [String::new(), String::new(), String::new()];
        for (i, t) in topics.iter().enumerate() {
            let title = t.fancy_title.clone().unwrap_or_else(|| t.title.clone());
            let ago = crate::age_words::distance_of_time_in_words(
                &state.i18n,
                "datetime.distance_in_words_verbose",
                now,
                t.assigned_at,
                false,
            );
            items[i] = format!(
                "- [{}]({base_path}/t/{}/{}) - assigned {ago}",
                discourse_markdown::emoji::gsub_emoji_to_unicode(&title, &denied),
                t.slug.clone().unwrap_or_default(),
                t.id
            );
        }
        state
            .i18n
            .t_with(
                &format!("pending_assigns_reminder.{key}"),
                &[
                    ("topic_0", &items[0]),
                    ("topic_1", &items[1]),
                    ("topic_2", &items[2]),
                ],
            )
            .unwrap_or_default()
    };
    // frequency_in_words: the user's own, else the setting's.
    let minutes = match frequency {
        Some(f) => crate::ruby::to_i(&f),
        None => settings.get("remind_assigns_frequency")?.to_i(),
    };
    let frequency_name = match minutes {
        0 => "never",
        1440 => "daily",
        10080 => "weekly",
        43200 => "monthly",
        129600 => "quarterly",
        _ => {
            return Err(Unsupported("a reminder frequency that isn't one of the settings'").into());
        }
    };
    let frequency = state
        .i18n
        .t(&format!(
            "discourse_assign.reminders_frequency.{frequency_name}"
        ))
        .unwrap_or(frequency_name)
        .to_string();
    let count_s = count.to_string();
    let title = state
        .i18n
        .t_with(
            "pending_assigns_reminder.title",
            &[("pending_assignments", &count_s)],
        )
        .unwrap_or_default();
    let body = state
        .i18n
        .t_with(
            "pending_assigns_reminder.body",
            &[
                ("pending_assignments", &count_s),
                (
                    "assignments_link",
                    &format!("{}/u/{username_lower}/activity/assigned", urls.base_url()?),
                ),
                ("newest_assignments", &list("newest", &newest)),
                ("oldest_assignments", &list("oldest", &oldest)),
                ("frequency", &frequency),
            ],
        )
        .unwrap_or_default();

    delete_previous_reminders(conn, state, &settings, user_id).await?;

    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let (system, guardian) = super::assigner::system_user(&mut *conn).await?;
    let recipient: String = sqlx::query_scalar("SELECT username_lower FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    let _ = system;
    let outcome = crate::posting::create::create(
        &state.pool,
        &ctx,
        &guardian,
        crate::posting::create::NewPost {
            raw: body,
            title: Some(title),
            pm_recipients: Some(vec![recipient]),
            skip_validations: true,
            subtype: Some("system_message".to_string()),
            topic_custom_fields: vec![(CUSTOM_FIELD_NAME.to_string(), "true".to_string())],
            ..Default::default()
        },
    )
    .await?;
    let post_id = match outcome {
        crate::posting::create::Outcome::Created { post_id } => post_id,
        _ => return Err(Unsupported("a pending assigns reminder PostCreator refused").into()),
    };
    // PostCreator's custom_fields go on the post too.
    sqlx::query(
        "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
         VALUES ($1, $2, 'true', clock_timestamp(), clock_timestamp())",
    )
    .bind(post_id)
    .bind(CUSTOM_FIELD_NAME)
    .execute(&state.pool)
    .await?;

    // update_last_reminded: upsert_custom_fields
    let now = crate::clock::now()
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string();
    sqlx::query("DELETE FROM user_custom_fields WHERE user_id = $1 AND name = $2")
        .bind(user_id)
        .bind(REMINDED_AT)
        .execute(&state.pool)
        .await?;
    sqlx::query(
        "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) \
         VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
    )
    .bind(user_id)
    .bind(REMINDED_AT)
    .bind(now)
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// `delete_previous_reminders`: the system's one-post reminder messages to
/// the user, destroyed as the system user.
async fn delete_previous_reminders(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &crate::site_settings::SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    let posts: Vec<i32> = sqlx::query_scalar(
        "SELECT p.id FROM posts p JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
         JOIN topic_allowed_users tau ON tau.topic_id = t.id AND tau.user_id = $1 \
         JOIN topic_custom_fields tcf ON tcf.topic_id = t.id AND tcf.name = $2 \
         WHERE p.deleted_at IS NULL AND t.posts_count = 1 AND t.user_id = -1 \
           AND t.archetype = 'private_message' AND t.subtype = 'system_message' \
         ORDER BY p.id",
    )
    .bind(user_id)
    .bind(CUSTOM_FIELD_NAME)
    .fetch_all(&mut *conn)
    .await?;
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    for post_id in posts {
        crate::post_destroyer::destroy_as_system(
            &state.pool,
            &ctx,
            post_id,
            "cleaning up pending assigns reminder",
        )
        .await?;
    }
    Ok(())
}
