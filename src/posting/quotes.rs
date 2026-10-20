//! What a post's quotes link it to: `Post#extract_quoted_post_numbers`
//! (its `quote_count`) and `Post#save_reply_relationships` (a PostReply,
//! and a reply count, for each post it quotes in its topic and the one it
//! replies to). `QuotedPost.extract_from` is `rebake::extract_quoted_posts`.

use sqlx::PgConnection;

/// `Post#extract_quoted_post_numbers`: the post numbers of the raw's
/// `[quote="..."]` tags in this topic (nil for a quote without one), each
/// once. Their count is the post's `quote_count`.
pub fn quoted_post_numbers(raw: &str, topic_id: i32, post_number: i32) -> Vec<Option<i32>> {
    let mut out: Vec<Option<i32>> = Vec::new();
    let mut rest = raw;
    while let Some(start) = rest.find("[quote=\"") {
        let after = &rest[start + 8..];
        let Some(end) = after.find('"') else {
            break;
        };
        let inner = &after[..end];
        rest = &after[end..];
        if !after[end..].starts_with("\"]") {
            continue;
        }
        if inner.is_empty() {
            continue;
        }
        // quote.first.scan(/([a-z]+)\:(\d+)/): the last of each name wins.
        let mut post = None;
        let mut topic = None;
        let bytes = inner.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_lowercase() {
                let name_start = i;
                while i < bytes.len() && bytes[i].is_ascii_lowercase() {
                    i += 1;
                }
                if i + 1 < bytes.len() && bytes[i] == b':' && bytes[i + 1].is_ascii_digit() {
                    let name = &inner[name_start..i];
                    let digits_start = i + 1;
                    let mut j = digits_start;
                    while j < bytes.len() && bytes[j].is_ascii_digit() {
                        j += 1;
                    }
                    let value = crate::ruby::to_i(&inner[digits_start..j]) as i32;
                    match name {
                        "post" => post = Some(value),
                        "topic" => topic = Some(value),
                        _ => {}
                    }
                    i = j;
                }
            } else {
                i += 1;
            }
        }
        if !(topic.is_some() && topic != Some(topic_id)) && post != Some(post_number) {
            out.push(post);
        }
    }
    let mut unique: Vec<Option<i32>> = Vec::new();
    for n in out {
        if !unique.contains(&n) {
            unique.push(n);
        }
    }
    unique
}

/// The post whose reply relationships are saved.
#[derive(Clone, Copy)]
pub struct ReplyingPost {
    pub id: i32,
    pub topic_id: i32,
    pub post_type: i32,
    pub reply_to_post_number: Option<i32>,
    pub deleted: bool,
}

/// `Post#save_reply_relationships`: the posts quoted (`quoted`, from
/// `quoted_post_numbers`) then the one replied to, each found in the
/// topic among the posts not deleted, get a PostReply to this post
/// (`create_reply_relationship_with`); a new one counts as a reply of
/// theirs when this post is of a type the topic shows everyone
/// (`Topic.visible_post_types`). Nothing for a deleted post.
pub async fn save_reply_relationships(
    conn: &mut PgConnection,
    post: ReplyingPost,
    quoted: &[Option<i32>],
) -> Result<(), sqlx::Error> {
    if post.deleted {
        return Ok(());
    }
    let mut numbers: Vec<Option<i32>> = quoted.to_vec();
    if let Some(n) = post.reply_to_post_number {
        numbers.push(Some(n));
    }
    let visible = [
        super::post_types::REGULAR,
        super::post_types::MODERATOR_ACTION,
        super::post_types::SMALL_ACTION,
    ]
    .contains(&post.post_type);
    for n in numbers.into_iter().flatten() {
        let parent: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM posts WHERE topic_id = $1 AND post_number = $2 AND deleted_at IS NULL \
             ORDER BY id LIMIT 1",
        )
        .bind(post.topic_id)
        .bind(n)
        .fetch_optional(&mut *conn)
        .await?;
        let Some(parent) = parent else {
            continue;
        };
        let inserted = sqlx::query(
            "INSERT INTO post_replies (post_id, reply_post_id, created_at, updated_at) \
             VALUES ($1, $2, clock_timestamp(), clock_timestamp()) ON CONFLICT DO NOTHING",
        )
        .bind(parent)
        .bind(post.id)
        .execute(&mut *conn)
        .await?
        .rows_affected();
        if inserted > 0 && visible {
            sqlx::query("UPDATE posts SET reply_count = reply_count + 1 WHERE id = $1")
                .bind(parent)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::quoted_post_numbers;

    #[test]
    fn quoted_numbers() {
        let raw = "[quote=\"user1, post:2, topic:35\"]\nx\n[/quote]\n[quote=\"user2, post:3, topic:9\"]\ny\n[/quote]\n[quote=\"user3\"]z[/quote]\n[quote=\"user1, post:2, topic:35\"]again[/quote]";
        assert_eq!(quoted_post_numbers(raw, 35, 4), vec![Some(2), None]);
        assert_eq!(quoted_post_numbers(raw, 35, 2), vec![None]);
    }
}
