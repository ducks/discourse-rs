//! The template variables (lib/replace-variables.js and the extractors
//! beside it): `%{name}` and `%{name,fallback:text}` in a template's title
//! and content, replaced as the template is inserted. Rails takes the
//! values from the client's models; here the server reads them from what
//! the insert's context names: the composer's topic and the post it
//! replies to or edits, or the chat channel and the message replied to.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use sqlx::PgConnection;

use crate::AppError;
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;

/// `TEMPLATES_ALLOWED_VARIABLES`, in its order (replacement runs in it).
pub const ALLOWED_VARIABLES: [&str; 16] = [
    "my_username",
    "my_name",
    "chat_channel_name",
    "chat_channel_url",
    "chat_thread_name",
    "chat_thread_url",
    "context_title",
    "context_url",
    "topic_title",
    "topic_url",
    "original_poster_username",
    "original_poster_name",
    "reply_to_username",
    "reply_to_name",
    "last_poster_username",
    "reply_to_or_last_poster_username",
];

struct Patterns {
    /// `%{key(,fallback:.[^}]*)?}`
    any: Regex,
    /// `%{key,fallback:(.[^}]*)}`
    fallback: Regex,
    /// `%{key}`
    bare: Regex,
}

static PATTERNS: LazyLock<Vec<Patterns>> = LazyLock::new(|| {
    ALLOWED_VARIABLES
        .iter()
        .map(|key| Patterns {
            any: Regex::new(&format!(r"%\{{{key}(,fallback:.[^}}]*)?\}}")).expect("variable regex"),
            fallback: Regex::new(&format!(r"%\{{{key},fallback:(.[^}}]*)\}}"))
                .expect("fallback regex"),
            bare: Regex::new(&format!(r"%\{{{key}\}}")).expect("bare regex"),
        })
        .collect()
});

/// `replaceVariables(title, content, variables)`: each allowed variable
/// with a value replaces its placeholders, fallback or not; one without
/// leaves its fallback text, or nothing. The value is a JS replacement
/// string: `$&`, `$1` and the like in it are expanded as String#replace
/// does.
pub fn replace_variables(
    title: &str,
    content: &str,
    variables: &HashMap<&'static str, String>,
) -> (String, String) {
    let mut title = title.to_string();
    let mut content = content.to_string();
    for (key, patterns) in ALLOWED_VARIABLES.iter().zip(PATTERNS.iter()) {
        let value = variables.get(key).filter(|v| !v.is_empty());
        for text in [&mut title, &mut content] {
            *text = match value {
                Some(value) => patterns
                    .any
                    .replace_all(text, |caps: &regex::Captures| js_expand(value, caps, text))
                    .into_owned(),
                None => {
                    let kept = patterns.fallback.replace_all(text, "$1").into_owned();
                    patterns.bare.replace_all(&kept, "").into_owned()
                }
            };
        }
    }
    (title, content)
}

/// GetSubstitution: a JS replacement string for one match. `$$` is `$`,
/// `$&` the match, `` $` `` and `$'` the text before and after it, `$n` and
/// `$nn` a group the pattern has (an unmatched one is empty); anything
/// else stays as written.
fn js_expand(replacement: &str, caps: &regex::Captures, haystack: &str) -> String {
    let whole = caps.get(0).expect("group 0");
    let groups = caps.len() - 1;
    let bytes = replacement.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' || i + 1 >= bytes.len() {
            let c = replacement[i..].chars().next().expect("char");
            out.push(c);
            i += c.len_utf8();
            continue;
        }
        match bytes[i + 1] {
            b'$' => {
                out.push('$');
                i += 2;
            }
            b'&' => {
                out.push_str(whole.as_str());
                i += 2;
            }
            b'`' => {
                out.push_str(&haystack[..whole.start()]);
                i += 2;
            }
            b'\'' => {
                out.push_str(&haystack[whole.end()..]);
                i += 2;
            }
            d @ b'0'..=b'9' => {
                let one = usize::from(d - b'0');
                let two = bytes
                    .get(i + 2)
                    .filter(|b| b.is_ascii_digit())
                    .map(|b| one * 10 + usize::from(b - b'0'));
                match two {
                    Some(n) if (1..=groups).contains(&n) => {
                        out.push_str(caps.get(n).map(|m| m.as_str()).unwrap_or_default());
                        i += 3;
                    }
                    _ if (1..=groups).contains(&one) => {
                        out.push_str(caps.get(one).map(|m| m.as_str()).unwrap_or_default());
                        i += 2;
                    }
                    _ => {
                        out.push('$');
                        i += 1;
                    }
                }
            }
            _ => {
                out.push('$');
                i += 1;
            }
        }
    }
    out
}

/// Where a template is inserted, as the request names it.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Context {
    /// A textarea outside the composers: the member's own variables only.
    #[default]
    None,
    /// The topic composer: its topic, and the post (by number) it replies
    /// to or edits.
    Composer {
        topic_id: Option<i32>,
        post_number: Option<i32>,
    },
    /// The chat channel composer, and the message it replies to.
    Chat {
        channel_id: i64,
        reply_to_id: Option<i64>,
    },
}

/// Ember's `User#displayName`: the name while names are on and it is set,
/// else the username.
fn display_name(
    settings: &SiteSettings,
    username: &str,
    name: Option<&str>,
) -> Result<String, AppError> {
    Ok(match name.filter(|n| !n.is_empty()) {
        Some(name) if settings.get("enable_names")?.truthy() => name.to_string(),
        _ => username.to_string(),
    })
}

/// A serialized user's `name`: included while enable_names is on.
fn serialized_name(settings: &SiteSettings, name: Option<String>) -> Result<String, AppError> {
    Ok(if settings.get("enable_names")?.truthy() {
        name.unwrap_or_default()
    } else {
        String::new()
    })
}

/// The values for a context: `my_username` and `my_name` always, the
/// rest from what the context names and the member can see (none of a
/// topic or channel they can't).
pub async fn load(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    config: &crate::config::Config,
    guardian: &Guardian,
    context: &Context,
) -> Result<HashMap<&'static str, String>, AppError> {
    let mut out: HashMap<&'static str, String> = HashMap::new();
    let base = config.globals.relative_url_root();
    match context {
        Context::None => {}
        Context::Composer {
            topic_id,
            post_number,
        } => {
            if let Some(topic_id) = topic_id {
                composer(
                    conn,
                    settings,
                    guardian,
                    base,
                    *topic_id,
                    *post_number,
                    &mut out,
                )
                .await?;
            }
        }
        Context::Chat {
            channel_id,
            reply_to_id,
        } => {
            chat(
                conn,
                settings,
                i18n,
                config,
                guardian,
                *channel_id,
                *reply_to_id,
                &mut out,
            )
            .await?;
        }
    }
    if let Some(user) = guardian.user() {
        out.insert("my_username", user.username.clone());
        out.insert(
            "my_name",
            display_name(settings, &user.username, user.name.as_deref())?,
        );
    }
    Ok(out)
}

/// extractVariablesFromComposerModel
async fn composer(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    base: &str,
    topic_id: i32,
    post_number: Option<i32>,
    out: &mut HashMap<&'static str, String>,
) -> Result<(), AppError> {
    let Some(ctx) =
        crate::topic_guardian::TopicCtx::load(&mut *conn, settings, guardian, topic_id).await?
    else {
        return Ok(());
    };
    let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
    if !guardian.can_see_topic(settings, &ctx, true, &secure)? {
        return Ok(());
    }
    type TopicRow = (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let (title, slug, op_username, op_name, last_poster): TopicRow = sqlx::query_as(
        "SELECT t.title, t.slug, op.username, op.name, lp.username FROM topics t \
         LEFT JOIN users op ON op.id = t.user_id \
         LEFT JOIN users lp ON lp.id = t.last_post_user_id \
         WHERE t.id = $1",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    // Topic#url
    let slug = slug
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "topic".into());
    let url = format!("{base}/t/{slug}/{topic_id}");
    let reply_to = match post_number {
        Some(number) => {
            let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT u.username, u.name FROM posts p LEFT JOIN users u ON u.id = p.user_id \
                 WHERE p.topic_id = $1 AND p.post_number = $2 AND p.deleted_at IS NULL",
            )
            .bind(topic_id)
            .bind(number)
            .fetch_optional(&mut *conn)
            .await?;
            row
        }
        None => None,
    };
    let (reply_username, reply_name) = reply_to.unwrap_or_default();
    let last_poster = last_poster.unwrap_or_default();
    let reply_username = reply_username.unwrap_or_default();
    out.insert("topic_title", title.clone());
    out.insert("topic_url", url.clone());
    out.insert("original_poster_username", op_username.unwrap_or_default());
    out.insert("original_poster_name", serialized_name(settings, op_name)?);
    out.insert(
        "reply_to_or_last_poster_username",
        if reply_username.is_empty() {
            last_poster.clone()
        } else {
            reply_username.clone()
        },
    );
    out.insert("reply_to_username", reply_username);
    out.insert("reply_to_name", serialized_name(settings, reply_name)?);
    out.insert("last_poster_username", last_poster);
    out.insert("context_title", title);
    out.insert("context_url", url);
    Ok(())
}

/// extractVariablesFromChatChannel
#[allow(clippy::too_many_arguments)]
async fn chat(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    config: &crate::config::Config,
    guardian: &Guardian,
    channel_id: i64,
    reply_to_id: Option<i64>,
    out: &mut HashMap<&'static str, String>,
) -> Result<(), AppError> {
    use crate::plugins::chat::channels::{CHANNEL_COLUMNS, ChannelRow, Context as ChatCx};
    if !crate::plugins::chat::enabled(settings)? {
        return Ok(());
    }
    let channel: Option<ChannelRow> = sqlx::query_as(&format!(
        "SELECT {CHANNEL_COLUMNS} FROM chat_channels WHERE id = $1 AND deleted_at IS NULL"
    ))
    .bind(channel_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(channel) = channel else {
        return Ok(());
    };
    let base = config.globals.relative_url_root();
    let mut cx = ChatCx {
        conn: &mut *conn,
        settings,
        i18n,
        guardian,
        base_path: base,
        config,
    };
    if !cx.can_preview(&channel).await? {
        return Ok(());
    }
    let json = cx.channel(&channel, None, None, None).await?;
    let title = json["title"].as_str().unwrap_or_default().to_string();
    // router.urlFor("chat.channel", ...channel.routeModels)
    let url = format!(
        "{base}/chat/c/{}/{}",
        crate::plugins::chat::page::slugified_title(&json),
        channel.id
    );
    if let Some(message_id) = reply_to_id {
        let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT u.username, u.name FROM chat_messages m LEFT JOIN users u ON u.id = m.user_id \
             WHERE m.id = $1 AND m.chat_channel_id = $2 AND m.deleted_at IS NULL",
        )
        .bind(message_id)
        .bind(channel.id)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some((username, name)) = row {
            out.insert("reply_to_username", username.unwrap_or_default());
            out.insert("reply_to_name", serialized_name(settings, name)?);
        }
    }
    out.insert("chat_channel_name", title.clone());
    out.insert("chat_channel_url", url.clone());
    out.insert("context_title", title);
    out.insert("context_url", url);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&'static str, &str)]) -> HashMap<&'static str, String> {
        pairs.iter().map(|(k, v)| (*k, v.to_string())).collect()
    }

    // replace-variables-test.js: simple replacement and with a fallback,
    // for every allowed variable, defined or not.
    #[test]
    fn replaces_every_allowed_variable_with_and_without_fallback() {
        let values = vars(&[
            ("my_username", "heisenberg"),
            ("my_name", "Walter White"),
            ("topic_title", "Villains"),
            ("topic_url", "/t/villains/6"),
            ("context_title", "Villains"),
            ("context_url", "/t/villains/6"),
            ("original_poster_username", "mr_hide"),
            ("original_poster_name", "Dr. Henry Jekyll"),
            ("reply_to_username", "dracula"),
            ("reply_to_name", "Vlad"),
            ("last_poster_username", "frankenstein"),
            ("reply_to_or_last_poster_username", "dracula"),
        ]);
        for key in ALLOWED_VARIABLES {
            let value = values.get(key).cloned().unwrap_or_default();
            let (title, content) = replace_variables(
                &format!("test title:%{{{key}}}"),
                &format!("test response:%{{{key}}}, %{{{key}}}, %{{{key}}}"),
                &values,
            );
            assert_eq!(title, format!("test title:{value}"), "{key}");
            assert_eq!(
                content,
                format!("test response:{value}, {value}, {value}"),
                "{key}"
            );
            let upper = key.to_uppercase();
            let (title, content) = replace_variables(
                &format!("test title:%{{{key},fallback:{upper}}}"),
                &format!("test response:%{{{key},fallback:{upper}}}, %{{{key},fallback:{upper}}}"),
                &values,
            );
            let expected = if value.is_empty() {
                upper.clone()
            } else {
                value.clone()
            };
            assert_eq!(title, format!("test title:{expected}"), "{key}");
            assert_eq!(
                content,
                format!("test response:{expected}, {expected}"),
                "{key}"
            );
        }
    }

    #[test]
    fn unknown_placeholders_stay() {
        let (title, content) = replace_variables(
            "%{unknown}",
            "%{my_username} %{other,fallback:x}",
            &vars(&[]),
        );
        assert_eq!(title, "%{unknown}");
        assert_eq!(content, " %{other,fallback:x}");
    }

    #[test]
    fn values_are_js_replacement_strings() {
        let values = vars(&[("my_name", "[$&|$1|$$|$2|$x]")]);
        let (_, content) = replace_variables("", "hi %{my_name,fallback:z}!", &values);
        assert_eq!(content, "hi [%{my_name,fallback:z}|,fallback:z|$|$2|$x]!");
        let (_, content) = replace_variables("", "a%{my_name}b", &vars(&[("my_name", "$`$'")]));
        assert_eq!(content, "aabb");
    }

    #[test]
    fn an_empty_fallback_does_not_match_the_fallback_form() {
        // `.[^}]*` needs at least one character: `%{key,fallback:}` is left
        // for neither pattern to take.
        let (_, content) = replace_variables("", "a%{topic_title,fallback:}b", &vars(&[]));
        assert_eq!(content, "a%{topic_title,fallback:}b");
    }
}
