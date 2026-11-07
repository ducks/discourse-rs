//! What the post admin menu does to a post past the revisor: `Post#unhide!`,
//! PostLocker, the staff notice (PostsController#notice) and the rebake
//! posts#rebake asks for.

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::AppError;
use crate::posting::Ctx;
use crate::posting::post_types::{REGULAR, WHISPER};

/// `Topic.visibility_reasons`
const OP_FLAG_THRESHOLD_REACHED: i32 = 0;
const OP_UNHIDDEN: i32 = 1;

/// `Post#unhide!`: shown again; a first post hidden by its flags lists its
/// topic again (as the system user, op_unhidden), a last reply that isn't a
/// whisper bumps the topic back to it, the author's count comes back
/// (UserStatCountUpdater.increment!, which the relisting does itself), and
/// the clients are told (:acted).
pub async fn unhide(conn: &mut PgConnection, ctx: &Ctx<'_>, post_id: i32) -> Result<(), AppError> {
    let (topic_id, post_number, post_type, user_id, created_at): (
        i32,
        i32,
        i32,
        Option<i32>,
        chrono::NaiveDateTime,
    ) = sqlx::query_as(
        "SELECT topic_id, post_number, post_type, user_id, created_at FROM posts WHERE id = $1",
    )
    .bind(post_id)
    .fetch_one(&mut *conn)
    .await?;
    // update!(hidden: false)
    sqlx::query(
        "UPDATE posts SET hidden = FALSE, updated_at = clock_timestamp() WHERE id = $1 AND hidden",
    )
    .bind(post_id)
    .execute(&mut *conn)
    .await?;
    let (highest, archetype, visibility_reason_id): (i32, String, Option<i32>) = sqlx::query_as(
        "SELECT highest_post_number, archetype, visibility_reason_id FROM topics WHERE id = $1",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    let mut update_user_stat = true;
    // A topic's visibility_reason_id was nil before it was kept: that
    // counts as hidden by its flags too.
    let hidden_by_op_flagging = visibility_reason_id.is_none_or(|r| r == OP_FLAG_THRESHOLD_REACHED);
    if post_number == 1 && hidden_by_op_flagging {
        crate::topics::status::apply(
            &mut *conn,
            ctx,
            topic_id,
            -1,
            crate::topics::status::Status::Visible,
            true,
            &crate::topics::status::StatusOpts {
                visibility_reason_id: Some(OP_UNHIDDEN),
                ..Default::default()
            },
        )
        .await?;
        update_user_stat = false;
    }
    // reset_bumped_at(self) for the last reply.
    if highest == post_number && post_number != 1 && post_type != WHISPER {
        sqlx::query(
            "UPDATE topics SET bumped_at = $2, updated_at = clock_timestamp() \
             WHERE id = $1 AND bumped_at IS DISTINCT FROM $2",
        )
        .bind(topic_id)
        .bind(created_at)
        .execute(&mut *conn)
        .await?;
    }
    // UserStatCountUpdater.increment!: not in a message; a first post's
    // topic count, a regular reply's post count.
    let column = if post_number == 1 {
        Some("topic_count")
    } else if post_type == REGULAR {
        Some("post_count")
    } else {
        None
    };
    if update_user_stat
        && archetype != "private_message"
        && let (Some(column), Some(author)) = (column, user_id)
    {
        sqlx::query(&format!(
            "UPDATE user_stats SET {column} = {column} + 1 WHERE user_id = $1"
        ))
        .bind(author)
        .execute(&mut *conn)
        .await?;
    }
    crate::bus::publish_post_change(ctx, &mut *conn, post_id, "acted", Default::default(), false)
        .await?;
    Ok(())
}

/// `PostLocker#lock` and `#unlock` past the guardian: the locker kept on
/// the post and the post_locked or post_unlocked staff log.
pub async fn set_locked(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    post_id: i32,
    user_id: i32,
    locked: bool,
) -> Result<(), AppError> {
    sqlx::query("UPDATE posts SET locked_by_id = $2 WHERE id = $1")
        .bind(post_id)
        .bind(locked.then_some(user_id))
        .execute(&mut *conn)
        .await?;
    let action = if locked {
        "post_locked"
    } else {
        "post_unlocked"
    };
    log(conn, ctx, action, user_id, post_id, None, None).await
}

/// PostsController#notice past the guardian: the notice custom field set
/// to the cooked staff notice (Post.notices[:custom]) or removed, saved
/// (save_custom_fields), and log_post_staff_note. The cooked notice, for
/// the answer.
pub async fn set_notice(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    post_id: i32,
    user_id: i32,
    notice: Option<&str>,
) -> Result<Option<String>, AppError> {
    let old: Option<String> = sqlx::query_scalar(
        "SELECT value FROM post_custom_fields WHERE post_id = $1 AND name = 'notice' ORDER BY id LIMIT 1",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    // custom_fields[NOTICE]: the JSON field's value, parsed.
    let old_raw = old
        .as_deref()
        .and_then(|v| serde_json::from_str::<Value>(v).ok())
        .and_then(|v| v["raw"].as_str().map(str::to_string));
    let present = notice.filter(|n| !crate::ruby::is_blank(n));
    let cooked = match present {
        Some(raw) => {
            let cooked = crate::pretty_text::cook_without_onebox(ctx.host, raw).await?;
            let value = crate::web_hooks::to_json(&json!({
                "type": "custom",
                "raw": raw,
                "cooked": cooked,
                "created_by_user_id": user_id,
            }));
            // save_custom_fields: a row holding the value stays, the others
            // go, and the value is added when none held it.
            sqlx::query(
                "DELETE FROM post_custom_fields WHERE post_id = $1 AND name = 'notice' \
                   AND value IS DISTINCT FROM $2",
            )
            .bind(post_id)
            .bind(&value)
            .execute(&mut *conn)
            .await?;
            sqlx::query(
                "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
                 SELECT $1, 'notice', $2, clock_timestamp(), clock_timestamp() \
                 WHERE NOT EXISTS (SELECT 1 FROM post_custom_fields \
                                   WHERE post_id = $1 AND name = 'notice' AND value = $2)",
            )
            .bind(post_id)
            .bind(&value)
            .execute(&mut *conn)
            .await?;
            Some(cooked)
        }
        None => {
            sqlx::query("DELETE FROM post_custom_fields WHERE post_id = $1 AND name = 'notice'")
                .bind(post_id)
                .execute(&mut *conn)
                .await?;
            None
        }
    };
    let action = if present.is_some() {
        "post_staff_note_create"
    } else {
        "post_staff_note_destroy"
    };
    let previous = old_raw.filter(|r| !crate::ruby::is_blank(r));
    log(
        conn,
        ctx,
        action,
        user_id,
        post_id,
        previous.as_deref(),
        present,
    )
    .await?;
    Ok(cooked)
}

/// A StaffActionLogger entry about a post.
async fn log(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    action: &str,
    user_id: i32,
    post_id: i32,
    previous_value: Option<&str>,
    new_value: Option<&str>,
) -> Result<(), AppError> {
    let id = crate::staff_action_logs::action_id(action)
        .ok_or(crate::Unsupported("a staff action without its id"))?;
    let admin_only = crate::staff_action_logs::admin_only(action, ctx.settings)?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, post_id, previous_value, new_value, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, clock_timestamp(), clock_timestamp())",
    )
    .bind(id)
    .bind(user_id)
    .bind(post_id)
    .bind(previous_value)
    .bind(new_value)
    .bind(admin_only)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
