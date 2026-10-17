//! `SeedData::Topics#update` for the welcome topic, as the title and
//! site_description settings' change handler runs it
//! (`SeedData::Topics.with_default_locale.update(site_setting_names:
//! ["welcome_topic_id"], skip_changed: true)`): the topic's first post
//! revised by the system user to the seeded title and body for the
//! site's title and description, unless someone else has edited it.

use sqlx::PgPool;

use crate::posting::Ctx;
use crate::posting::revise::{self, Changes};
use crate::posting::revise_topic::TopicFields;
use crate::session::current::SessionUser;
use crate::{AppError, Unsupported};

const SYSTEM_USER_ID: i32 = -1;

/// The welcome post: id, last editor, deleted at and by.
type SeedPost = (i32, Option<i32>, Option<chrono::NaiveDateTime>, Option<i32>);

/// The settings whose change rewrites the welcome topic.
pub fn rewrites_welcome_topic(name: &str) -> bool {
    matches!(name, "title" | "site_description")
}

/// `#{feedback_category}`: the meta category's hashtag.
async fn feedback_category_hashtag(pool: &PgPool, ctx: &Ctx<'_>) -> Result<String, AppError> {
    let id = ctx.settings.get("meta_category_id")?.to_i() as i32;
    let slug: Option<String> = sqlx::query_scalar("SELECT slug FROM categories WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(match slug {
        Some(slug) => format!("#{slug}"),
        None => "#site-feedback".to_string(),
    })
}

/// The welcome topic's seeded title and body.
async fn welcome_topic(pool: &PgPool, ctx: &Ctx<'_>) -> Result<(String, String), AppError> {
    let title = ctx.settings.get("title")?.to_s();
    let description = ctx.settings.get("site_description")?.to_s();
    let site_info_quote = if !crate::ruby::is_blank(&title) && !crate::ruby::is_blank(&description)
    {
        format!("> ## {title}\n>\n> {description}\n")
    } else {
        String::new()
    };
    let base_path = ctx.config.globals.relative_url_root();
    let feedback = feedback_category_hashtag(pool, ctx).await?;
    let topic_title = ctx
        .i18n
        .t_with("discourse_welcome_topic.title", &[("site_title", &title)])
        .ok_or(Unsupported("the welcome topic's title string"))?;
    let raw = ctx
        .i18n
        .t_with(
            "discourse_welcome_topic.body",
            &[
                ("base_path", base_path),
                ("site_title", &title),
                ("site_description", &description),
                ("site_info_quote", &site_info_quote),
                ("feedback_category", &feedback),
            ],
        )
        .ok_or(Unsupported("the welcome topic's body string"))?;
    Ok((topic_title, raw))
}

/// `update_topic(skip_changed: true)` for welcome_topic_id.
pub async fn update_welcome_topic(pool: &PgPool, ctx: &Ctx<'_>) -> Result<(), AppError> {
    let topic_id = ctx.settings.get("welcome_topic_id")?.to_i() as i32;
    if topic_id < 1 {
        return Ok(());
    }
    // find_post(deleted: true)
    let post: Option<SeedPost> = sqlx::query_as(
        "SELECT id, last_editor_id, deleted_at, deleted_by_id FROM posts \
             WHERE topic_id = $1 AND post_number = 1 ORDER BY id LIMIT 1",
    )
    .bind(topic_id)
    .fetch_optional(pool)
    .await?;
    let Some((post_id, last_editor_id, deleted_at, deleted_by_id)) = post else {
        return Ok(());
    };
    // unchanged?
    let unchanged = last_editor_id == Some(SYSTEM_USER_ID)
        && deleted_by_id.is_none_or(|id| id == SYSTEM_USER_ID);
    if !unchanged {
        return Ok(());
    }
    if deleted_at.is_some() {
        return Err(Unsupported("recovering a deleted welcome topic to rewrite it").into());
    }
    let (title, raw) = welcome_topic(pool, ctx).await?;
    let mut conn = pool.acquire().await?;
    let system = SessionUser::load(&mut conn, SYSTEM_USER_ID)
        .await?
        .ok_or(Unsupported("a site without its system user"))?;
    let guardian = crate::guardian::Guardian::for_user(&mut conn, &system).await?;
    drop(conn);
    let outcome = revise::revise(
        pool,
        ctx,
        &guardian,
        post_id,
        Changes {
            raw: Some(raw),
            skip_validations: true,
            topic: TopicFields {
                title: Some(title),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await?;
    match outcome {
        revise::Outcome::Revised => Ok(()),
        _ => Err(Unsupported("a welcome topic rewrite the revisor refused").into()),
    }
}
