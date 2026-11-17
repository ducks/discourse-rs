//! `PlaintextMentions` (Discourse b1a0af17849): the @mentions in a review
//! queue note, which stays plain text. Its usernames are what the note
//! notifies; `render` is the note's `cooked`, the text escaped in a
//! paragraph with each mention of a known user linked to the profile.

use std::collections::HashSet;
use std::sync::LazyLock;

use fancy_regex::Regex;
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::categories::badge::html_escape;
use crate::notifications::create::NewNotification;
use crate::{AppError, Unsupported};

/// `Notification.types[:mentioned]`
const MENTIONED: i32 = 1;

/// `MENTION_PATTERN` (Ruby's `\w` is ASCII).
static MENTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?<![^\s\p{P}\p{S}])@([A-Za-z0-9_][A-Za-z0-9_.\-]{0,58}[A-Za-z0-9]|[A-Za-z0-9_])(?![^\s\p{P}\p{S}])",
    )
    .expect("mention pattern")
});

/// `UNICODE_MENTION_PATTERN`
static UNICODE_MENTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?<![^\s\p{P}\p{S}])@([\p{Alphabetic}\p{M}\p{Nd}_][\p{Alphabetic}\p{M}\p{Nd}._\-]{0,58}[\p{Alphabetic}\p{M}\p{Nd}]|[\p{Alphabetic}\p{M}\p{Nd}_])(?![^\s\p{P}\p{S}])",
    )
    .expect("unicode mention pattern")
});

/// One match: its byte range and the username as written.
struct Match {
    start: usize,
    end: usize,
    whole: String,
    name: String,
}

fn matches(content: &str, unicode: bool) -> Vec<Match> {
    let pattern = if unicode {
        &*UNICODE_MENTION
    } else {
        &*MENTION
    };
    pattern
        .captures_iter(content)
        .filter_map(Result::ok)
        .filter_map(|c| {
            let whole = c.get(0)?;
            Some(Match {
                start: whole.start(),
                end: whole.end(),
                whole: whole.as_str().to_string(),
                name: c.get(1)?.as_str().to_string(),
            })
        })
        .collect()
}

/// `#usernames`: each mentioned username normalized, once.
pub fn usernames(content: &str, unicode: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in matches(content, unicode) {
        let name = crate::avatar::normalize_username(&m.name);
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// `PlaintextMentions.known_usernames(contents)`: the mentioned usernames
/// that are real (non-staged) users.
pub async fn known_usernames(
    conn: &mut PgConnection,
    contents: &[&str],
    unicode: bool,
) -> Result<HashSet<String>, sqlx::Error> {
    let mut names: Vec<String> = Vec::new();
    for content in contents {
        for name in usernames(content, unicode) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    if names.is_empty() {
        return Ok(HashSet::new());
    }
    let known: Vec<String> = sqlx::query_scalar(
        "SELECT username_lower FROM users WHERE username_lower = ANY($1) AND NOT staged",
    )
    .bind(&names)
    .fetch_all(conn)
    .await?;
    Ok(known.into_iter().collect())
}

/// `#render(known_usernames:)`: None for `enable_mentions` off renders
/// the plain paragraph.
pub fn render(
    content: &str,
    known: Option<&HashSet<String>>,
    unicode: bool,
    base_path: &str,
) -> String {
    let Some(known) = known else {
        return format!("<p>{}</p>", html_escape(content));
    };
    let mut out = String::from("<p>");
    let mut cursor = 0;
    for m in matches(content, unicode) {
        out.push_str(&html_escape(&content[cursor..m.start]));
        let username = crate::avatar::normalize_username(&m.name);
        if known.contains(&username) {
            let href = format!(
                "{base_path}/u/{}",
                crate::pretty_text::cleanup::encode_component(&username)
            );
            out.push_str(&format!(
                "<a class=\"mention\" href=\"{}\">{}</a>",
                html_escape(&href),
                html_escape(&m.whole)
            ));
        } else {
            out.push_str(&html_escape(&m.whole));
        }
        cursor = m.end;
    }
    out.push_str(&html_escape(&content[cursor..]));
    out.push_str("</p>");
    out
}

/// A note's `cooked` as the serializer gives it, the known usernames read
/// for this note alone.
pub async fn cooked(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    base_path: &str,
    content: &str,
) -> Result<String, AppError> {
    let unicode = s.get("unicode_usernames")?.truthy();
    if !s.get("enable_mentions")?.truthy() {
        return Ok(render(content, None, unicode, base_path));
    }
    let known = known_usernames(conn, &[content], unicode).await?;
    Ok(render(content, Some(&known), unicode, base_path))
}

/// `Reviewable#title_for_notification(user)` (English, the locale the port
/// serves): ReviewableUser's username, chat's channel, else the topic's
/// title or the payload's.
async fn title_for_notification(
    conn: &mut PgConnection,
    i18n: &crate::i18n::I18n,
    reviewable_id: i64,
) -> Result<String, AppError> {
    let (kind, topic_id, target_id, payload): (String, Option<i32>, Option<i32>, Option<Value>) =
        sqlx::query_as("SELECT type, topic_id, target_id, payload FROM reviewables WHERE id = $1")
            .bind(reviewable_id)
            .fetch_one(&mut *conn)
            .await?;
    let payload_str = |key: &str| {
        payload
            .as_ref()
            .and_then(|p| p.get(key))
            .and_then(Value::as_str)
            .filter(|v| !crate::ruby::is_blank(v))
            .map(str::to_string)
    };
    let t = |key: &str| i18n.t(key).unwrap_or(key).to_string();
    if kind == "ReviewableUser" {
        // target&.username || payload&.dig("username") || user_label
        let username: Option<String> = match target_id {
            Some(id) => {
                sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut *conn)
                    .await?
            }
            None => None,
        };
        return Ok(username
            .or_else(|| {
                payload
                    .as_ref()
                    .and_then(|p| p.get("username"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| t("js.review.user_label")));
    }
    if kind == crate::reviewables::CHAT_MESSAGE {
        // channel&.name || channel&.title(user) || super
        let channel: Option<(Option<String>, String, Option<String>)> = sqlx::query_as(
            "SELECT c.name, c.chatable_type, cat.name FROM chat_messages m \
             JOIN chat_channels c ON c.id = m.chat_channel_id AND c.deleted_at IS NULL \
             LEFT JOIN categories cat ON c.chatable_type = 'Category' AND cat.id = c.chatable_id \
             WHERE m.id = $1",
        )
        .bind(target_id)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some((name, chatable_type, category)) = channel {
            if let Some(name) = name.filter(|n| !n.is_empty()) {
                return Ok(name);
            }
            if chatable_type == "Category" {
                if let Some(category) = category {
                    return Ok(category);
                }
            } else {
                return Err(Unsupported("a direct message channel's title in a mention").into());
            }
        }
    }
    // topic&.title.presence || payload&.dig("title").presence || ...
    let title: Option<String> = match topic_id {
        Some(id) => {
            sqlx::query_scalar("SELECT title FROM topics WHERE id = $1 AND deleted_at IS NULL")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    Ok(title
        .filter(|t| !crate::ruby::is_blank(t))
        .or_else(|| payload_str("title"))
        .unwrap_or_else(|| {
            t(if topic_id.is_some() {
                "js.review.topics.deleted"
            } else {
                "js.review.title"
            })
        }))
}

/// The note's author, as notify_mentions reads them.
pub struct Author<'a> {
    pub id: i32,
    pub username: &'a str,
    pub staff: bool,
}

/// `ReviewableNote#notify_mentions` (after_create): a mention notification
/// for each mentioned user who can see the reviewable, unless they ignore
/// or mute the author; the usernames of those who can't see it.
pub async fn notify_mentions(
    conn: &mut PgConnection,
    ctx: &crate::posting::Ctx<'_>,
    author: &Author<'_>,
    reviewable_id: i64,
    note_id: i64,
    content: &str,
) -> Result<Vec<String>, AppError> {
    let s = ctx.settings;
    let mut unnotified = Vec::new();
    if !s.get("enable_mentions")?.truthy() {
        return Ok(unnotified);
    }
    let names = usernames(content, s.get("unicode_usernames")?.truthy());
    if names.is_empty() {
        return Ok(unnotified);
    }
    // User.where(username_lower:, staged: false).where.not(id:): no ORDER BY.
    let users: Vec<(i32, String, bool)> = sqlx::query_as(
        "SELECT id, username, admin OR moderator FROM users \
         WHERE username_lower = ANY($1) AND NOT staged AND id <> $2",
    )
    .bind(&names)
    .bind(author.id)
    .fetch_all(&mut *conn)
    .await?;
    let ids: Vec<i32> = users.iter().map(|u| u.0).collect();
    let screener = crate::user_comm_screener::UserCommScreener::new(
        &mut *conn,
        crate::user_comm_screener::Actor {
            id: author.id,
            staff: author.staff,
        },
        &ids,
    )
    .await?;
    for (id, username, staff) in users {
        // bot?
        if id <= 0 {
            continue;
        }
        // guardian.can_see_review_queue?, then viewable_by(user).exists?
        if !staff && s.get("enable_category_group_moderation")?.truthy() {
            return Err(
                Unsupported("mentioning a category group moderator in a review note").into(),
            );
        }
        let can_see = staff && {
            let guardian = crate::plugins::assign::assigner::guardian_for(&mut *conn, id)
                .await?
                .ok_or(Unsupported("a mentioned user without a session row"))?;
            crate::reviewables::is_viewable(&mut *conn, s, &guardian, reviewable_id).await?
        };
        if !can_see {
            unnotified.push(username);
            continue;
        }
        if screener.ignoring_or_muting_actor(id)? {
            continue;
        }
        let title = title_for_notification(&mut *conn, ctx.i18n, reviewable_id).await?;
        let mut n = NewNotification::new(
            MENTIONED,
            id,
            json!({
                "reviewable_id": reviewable_id,
                "reviewable_note_id": note_id,
                "topic_title": title,
                "display_username": author.username,
            }),
        );
        n.skip_send_email = true;
        crate::notifications::create::create(&mut *conn, ctx.bus, s, &n).await?;
    }
    Ok(unnotified)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_mentions_between_spaces_and_punctuation() {
        assert_eq!(
            usernames("hi @Bob, and (@sam.) not a@b or @_x_", false),
            vec!["bob", "sam", "_x"]
        );
        assert_eq!(usernames("@a.b- end", false), vec!["a.b"]);
    }

    #[test]
    fn renders_known_mentions_as_links() {
        let known: HashSet<String> = ["bob".to_string()].into_iter().collect();
        assert_eq!(
            render("<b> @Bob & @nobody", Some(&known), false, ""),
            "<p>&lt;b&gt; <a class=\"mention\" href=\"/u/bob\">@Bob</a> &amp; @nobody</p>"
        );
        assert_eq!(render("x", None, false, ""), "<p>x</p>");
    }
}
