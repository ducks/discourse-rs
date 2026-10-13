//! `Post#rebake!`: the post cooked again from its raw (as its last
//! editor, as PostAnalyzer cooks it), with what follows a new cooked: the
//! first post's excerpt, topic links, quoted posts, the post processor
//! and the clients told.

use std::collections::BTreeSet;

use sqlx::PgPool;

use super::Ctx;
use crate::{AppError, Unsupported};

/// `Post::BAKED_VERSION`
const BAKED_VERSION: i32 = 2;

/// `Post.cook_methods`
const COOK_REGULAR: i32 = 1;
const COOK_RAW_HTML: i32 = 2;

/// `Post#rebake!(priority: :ultra_low)` as Post.rebake_old runs it.
pub async fn rebake(pool: &PgPool, ctx: &Ctx<'_>, post_id: i32) -> Result<(), AppError> {
    let s = ctx.settings;
    #[derive(sqlx::FromRow)]
    struct Row {
        raw: String,
        topic_id: i32,
        post_number: i32,
        user_id: Option<i32>,
        last_editor_id: Option<i32>,
        cook_method: i32,
        reply_to_post_number: Option<i32>,
        reply_quoted: bool,
    }
    let mut conn = pool.acquire().await?;
    let post: Row = sqlx::query_as(
        "SELECT raw, topic_id, post_number, user_id, last_editor_id, cook_method, \
                reply_to_post_number, reply_quoted FROM posts WHERE id = $1",
    )
    .bind(post_id)
    .fetch_one(&mut *conn)
    .await?;
    if s.get("secure_uploads")?.truthy() {
        return Err(Unsupported("rebaking posts with secure uploads").into());
    }
    if s.get("content_localization_enabled")?.truthy() {
        return Err(Unsupported("rebaking post localizations").into());
    }
    // cook: raw HTML kept, email cooked by EmailCook (not ported), else
    // PrettyText with markdown_options (the last editor as the user).
    let cooked = match post.cook_method {
        COOK_RAW_HTML => post.raw.clone(),
        COOK_REGULAR => {
            let omit = match post.user_id {
                Some(uid) => super::create::omit_nofollow(&mut conn, ctx, uid).await?,
                None => false,
            };
            drop(conn);
            let cooked = crate::pretty_text::cook(
                ctx.host,
                &post.raw,
                &crate::pretty_text::MarkdownOptions {
                    topic_id: Some(i64::from(post.topic_id)),
                    post_id: Some(i64::from(post_id)),
                    user_id: post.last_editor_id.map(i64::from),
                    omit_nofollow: omit,
                    ..Default::default()
                },
            )
            .await?;
            conn = pool.acquire().await?;
            cooked
        }
        _ => return Err(Unsupported("rebaking emailed posts (EmailCook)").into()),
    };
    // Oneboxer.apply and the rest PostAnalyzer refuses.
    let base_path = ctx.config.globals.relative_url_root().to_string();
    super::validate::analyze(&cooked, &base_path)?;
    let definition: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE topic_id = $1)")
            .bind(post.topic_id)
            .fetch_one(&mut *conn)
            .await?;
    if definition && post.post_number == 1 {
        return Err(
            Unsupported("rebaking a category's definition (sync_category_description)").into(),
        );
    }
    drop(conn);

    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE posts SET cooked = $2, baked_at = clock_timestamp(), baked_version = $3 WHERE id = $1",
    )
    .bind(post_id)
    .bind(&cooked)
    .bind(BAKED_VERSION)
    .execute(&mut *tx)
    .await?;
    // sync_first_post_caches: the topic's excerpt.
    if post.post_number == 1 {
        let excerpt = crate::excerpt::excerpt(
            &cooked,
            s.get("topic_excerpt_maxlength")?.to_i().max(0) as usize,
            &crate::excerpt::Options {
                strip_links: true,
                image_mode: crate::excerpt::ImageMode::Strip,
                ..Default::default()
            },
        );
        sqlx::query("UPDATE topics SET excerpt = $2 WHERE id = $1")
            .bind(post.topic_id)
            .bind(excerpt)
            .execute(&mut *tx)
            .await?;
    }
    // TopicLink.extract_from
    if let Some(user_id) = post.user_id {
        let urls = crate::url::Urls {
            config: ctx.config,
            settings: s,
        };
        super::links::extract_from(
            &mut tx,
            &super::links::Site {
                hostname: &urls.current_hostname()?,
                base_path: &base_path,
                base_url_no_prefix: &urls.base_url_no_prefix()?,
                settings: s,
            },
            &super::links::LinkPost {
                id: post_id,
                user_id,
                topic_id: post.topic_id,
                cooked: &cooked,
            },
        )
        .await?;
    }
    extract_quoted_posts(
        &mut tx,
        post_id,
        post.topic_id,
        post.post_number,
        post.reply_to_post_number,
        post.reply_quoted,
        &cooked,
    )
    .await?;
    // trigger_post_process(bypass_bump: true, priority: :ultra_low)
    crate::jobs::enqueue(
        &mut tx,
        "process_post",
        serde_json::json!({
            "bypass_bump": true,
            "cooking_options": null,
            "new_post": false,
            "post_id": post_id,
            "skip_pull_hotlinked_images": false,
            "queue": "ultra_low",
        }),
    )
    .await?;
    // publish_change_to_clients!(:rebaked)
    crate::bus::publish_post_change(ctx, &mut tx, post_id, "rebaked", Default::default(), false)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// `QuotedPost.extract_from(post)`: the posts the cooked quotes, and
/// whether the post quotes the one it replies to (`reply_quoted`).
pub(crate) async fn extract_quoted_posts(
    conn: &mut sqlx::PgConnection,
    post_id: i32,
    topic_id: i32,
    post_number: i32,
    reply_to_post_number: Option<i32>,
    was_reply_quoted: bool,
    cooked: &str,
) -> Result<(), sqlx::Error> {
    let quotes = quoted(cooked, topic_id, post_number);
    if quotes.is_empty() {
        sqlx::query("DELETE FROM quoted_posts WHERE post_id = $1")
            .bind(post_id)
            .execute(&mut *conn)
            .await?;
    } else {
        let topic_ids: Vec<i32> = quotes.iter().map(|(t, _)| *t).collect();
        let post_numbers: Vec<i32> = quotes.iter().map(|(_, n)| *n).collect();
        sqlx::query(
            "INSERT INTO quoted_posts (post_id, quoted_post_id, created_at, updated_at) \
             SELECT $1, p.id, current_timestamp, current_timestamp FROM posts p \
             JOIN (SELECT unnest($2::int[]) topic_id, unnest($3::int[]) post_number) X \
               ON X.topic_id = p.topic_id AND X.post_number = p.post_number \
             LEFT JOIN quoted_posts q ON q.post_id = $1 AND q.quoted_post_id = p.id \
             WHERE q.id IS NULL",
        )
        .bind(post_id)
        .bind(&topic_ids)
        .bind(&post_numbers)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "DELETE FROM quoted_posts WHERE post_id = $1 AND id IN ( \
               SELECT q1.id FROM quoted_posts q1 LEFT JOIN posts p1 ON p1.id = q1.quoted_post_id \
               LEFT JOIN (SELECT unnest($2::int[]) topic_id, unnest($3::int[]) post_number) X \
                 ON X.topic_id = p1.topic_id AND X.post_number = p1.post_number \
               WHERE q1.post_id = $1 AND X.topic_id IS NULL)",
        )
        .bind(post_id)
        .bind(&topic_ids)
        .bind(&post_numbers)
        .execute(&mut *conn)
        .await?;
    }
    let reply_quoted = match reply_to_post_number {
        Some(n) => sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM quoted_posts q JOIN posts p ON p.id = q.quoted_post_id \
             WHERE q.post_id = $1 AND p.topic_id = $2 AND p.post_number = $3)",
        )
        .bind(post_id)
        .bind(topic_id)
        .bind(n)
        .fetch_one(&mut *conn)
        .await?,
        None => false,
    };
    if reply_quoted != was_reply_quoted {
        sqlx::query("UPDATE posts SET reply_quoted = $2 WHERE id = $1")
            .bind(post_id)
            .bind(reply_quoted)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// The posts a cooked post quotes (`aside.quote[data-topic]`), itself
/// left out, each once, in the order they come.
fn quoted(cooked: &str, topic_id: i32, post_number: i32) -> Vec<(i32, i32)> {
    use crate::pretty_text::cleanup::{attr, element_name, has_class, parse_document};
    let dom = parse_document(cooked);
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut stack = vec![dom.document.clone()];
    while let Some(node) = stack.pop() {
        if element_name(&node) == Some("aside") && has_class(&node, "quote") {
            let num = |name: &str| {
                attr(&node, name)
                    .map(|v| crate::ruby::to_i(&v) as i32)
                    .unwrap_or(0)
            };
            if attr(&node, "data-topic").is_some() {
                let (t, n) = (num("data-topic"), num("data-post"));
                if t != 0 && n != 0 && (t, n) != (topic_id, post_number) && seen.insert((t, n)) {
                    out.push((t, n));
                }
            }
        }
        for child in node.children.borrow().iter().rev() {
            stack.push(child.clone());
        }
    }
    out
}
