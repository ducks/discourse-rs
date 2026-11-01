//! Chat::TranscriptService#generate_markdown: a `[chat]` block per run of
//! one author's messages (and, with threading on, per thread), its quote,
//! channel, multiQuote, chained, noLink, reactions, threadId and
//! threadTitle attributes, the messages as Chat::Message#to_markdown writes
//! them (their uploads after the text), and a thread's replies in blocks of
//! their own after its original message. POST /chat/:chat_channel_id/quote
//! (ChatController#quote_messages) asks for the messages by id;
//! ChannelArchiveService (archive.rs) hands its batches over as they are,
//! with noLink, the reactions and the ranges of the threads it splits.
//!
//! Refused: messages of deleted users (a NoMethodError in Rails).

use std::collections::HashMap;

use super::channels::{ChannelRow, Context};
use crate::{AppError, Unsupported};

/// `TranscriptService::QUOTATION_MARKS`
const QUOTATION_MARKS: [&str; 9] = ["\"\"", "''", "«»", "“”", "””", "‘’", "„“", "‚’", "‹›"];

/// `serialize_attr`: the value in the first pair of marks it contains
/// neither of, else in double quotes without its own.
fn serialize_attr(name: &str, value: &str) -> String {
    for pair in QUOTATION_MARKS {
        let mut chars = pair.chars();
        let (open, close) = (chars.next().unwrap_or('"'), chars.next().unwrap_or('"'));
        if value.contains(open) || value.contains(close) {
            continue;
        }
        return format!("{name}={open}{value}{close}");
    }
    format!("{name}=\"{}\"", value.replace('"', ""))
}

/// A message as the transcript reads it.
#[derive(sqlx::FromRow, Clone, Debug)]
pub struct TranscriptMessage {
    pub id: i64,
    pub user_id: Option<i32>,
    pub username: Option<String>,
    pub created_at: chrono::NaiveDateTime,
    pub message: Option<String>,
    pub thread_id: Option<i64>,
}

/// The message columns [`TranscriptMessage`] reads, joined to users.
pub(crate) const TRANSCRIPT_COLUMNS: &str = "chat_messages.id, chat_messages.user_id, users.username, \
     chat_messages.created_at, chat_messages.message, chat_messages.thread_id";

/// `opts:` the service takes.
#[derive(Default, Clone, Copy)]
pub struct TranscriptOpts {
    pub no_link: bool,
    pub include_reactions: bool,
}

/// A thread as `Chat::Thread.find` gives the transcript.
#[derive(sqlx::FromRow)]
struct ThreadRow {
    id: i64,
    original_message_id: i64,
    title: Option<String>,
    replies_count: i32,
}

/// `TranscriptBBCode`: one `[chat]` block being filled.
struct BBCode {
    channel: bool,
    multiquote: bool,
    chained: bool,
    no_link: bool,
    include_reactions: bool,
    thread_id: Option<i64>,
    messages: Vec<(TranscriptMessage, Vec<String>)>,
    threads_markdown: HashMap<i64, String>,
}

impl BBCode {
    fn new(
        channel: bool,
        multiquote: bool,
        chained: bool,
        opts: TranscriptOpts,
        thread_id: Option<i64>,
    ) -> BBCode {
        BBCode {
            channel,
            multiquote,
            chained,
            no_link: opts.no_link,
            include_reactions: opts.include_reactions,
            thread_id,
            messages: Vec::new(),
            threads_markdown: HashMap::new(),
        }
    }
}

/// What a transcript's blocks are drawn from: the channel's title and id,
/// each message's markdown and reactions, the threads, the split ranges.
struct Drawn<'a> {
    title: &'a str,
    channel_id: i64,
    markdown: &'a HashMap<i64, String>,
    threads: &'a HashMap<i64, ThreadRow>,
    thread_ranges: &'a HashMap<i64, Option<String>>,
    default_thread_title: &'a str,
}

impl Drawn<'_> {
    fn thread(&self, id: i64) -> Result<&ThreadRow, AppError> {
        self.threads.get(&id).ok_or_else(|| {
            Unsupported("a chat transcript of a thread gone (RecordNotFound)").into()
        })
    }

    /// `TranscriptBBCode#render`
    fn render(&self, tag: &BBCode) -> Result<String, AppError> {
        let first = &tag.messages[0].0;
        let username = first
            .username
            .as_deref()
            .ok_or(Unsupported("transcripts of deleted users' chat messages"))?;
        let mut attrs = vec![format!(
            "quote=\"{username};{};{}\"",
            first.id,
            first.created_at.format("%Y-%m-%dT%H:%M:%SZ")
        )];
        if tag.channel {
            attrs.push(serialize_attr("channel", self.title));
            attrs.push(format!("channelId=\"{}\"", self.channel_id));
        }
        if tag.multiquote {
            attrs.push("multiQuote=\"true\"".into());
        }
        if tag.chained {
            attrs.push("chained=\"true\"".into());
        }
        if tag.no_link {
            attrs.push("noLink=\"true\"".into());
        }
        if tag.include_reactions {
            // Array#join flattens each message's reactions into one list.
            let data: Vec<&str> = tag
                .messages
                .iter()
                .flat_map(|(_, reactions)| reactions.iter().map(String::as_str))
                .collect();
            if !data.is_empty() {
                attrs.push(format!("reactions=\"{}\"", data.join(";")));
            }
        }
        if let Some(thread_id) = tag.thread_id {
            let thread = self.thread(thread_id)?;
            if thread.replies_count > 0 {
                attrs.push(format!("threadId=\"{thread_id}\""));
                let mut title = thread
                    .title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| self.default_thread_title.to_string());
                if let Some(Some(range)) = self.thread_ranges.get(&first.id)
                    && !range.trim().is_empty()
                {
                    title.push_str(&format!(" ({range})"));
                }
                attrs.push(serialize_attr("threadTitle", &title));
            }
        }
        let messages: Vec<String> = tag
            .messages
            .iter()
            .map(|(m, _)| {
                let rendered = self.markdown.get(&m.id).cloned().unwrap_or_default();
                match m.thread_id.and_then(|t| tag.threads_markdown.get(&t)) {
                    Some(thread) if !thread.trim().is_empty() => {
                        format!("{rendered}\n\n{thread}")
                    }
                    _ => rendered,
                }
            })
            .collect();
        Ok(format!(
            "[chat {}]\n{}\n[/chat]\n",
            attrs.join(" "),
            messages.join("\n\n")
        ))
    }
}

/// `UploadMarkdown#to_markdown`: an image, playable media or an attachment.
fn upload_markdown(
    filename: &str,
    width: Option<i32>,
    height: Option<i32>,
    sha1: &str,
    extension: Option<&str>,
    filesize: i64,
) -> String {
    let base = crate::pretty_text::helpers::base62_sha1(sha1).unwrap_or_default();
    let ext = extension
        .filter(|e| !e.is_empty())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let short_url = format!("upload://{base}{ext}");
    let label = filename.replace(['[', ']', '|'], "");
    let size = |n: Option<i32>| n.map(|n| n.to_string()).unwrap_or_default();
    if crate::uploads::is_supported_image(filename) {
        format!("![{label}|{}x{}]({short_url})", size(width), size(height))
    } else if crate::file_store::is_supported_audio(filename) {
        format!("![{label}|audio]({short_url})")
    } else if crate::file_store::is_supported_video(filename) {
        format!("![{label}|video]({short_url})")
    } else {
        format!(
            "[{label}|attachment]({short_url}) ({})",
            crate::uploads::human_size(filesize)
        )
    }
}

impl Context<'_> {
    /// The transcript of the channel's messages among `message_ids`, as the
    /// guardian's user quotes them (`messages`: the live ones, by
    /// created_at).
    pub async fn transcript_markdown(
        &mut self,
        channel: &ChannelRow,
        message_ids: &[i64],
    ) -> Result<String, AppError> {
        let messages: Vec<TranscriptMessage> = sqlx::query_as(&format!(
            "SELECT {TRANSCRIPT_COLUMNS} FROM chat_messages \
             LEFT JOIN users ON users.id = chat_messages.user_id \
             WHERE chat_messages.id = ANY($1) AND chat_messages.chat_channel_id = $2 \
               AND chat_messages.deleted_at IS NULL \
             ORDER BY chat_messages.created_at, chat_messages.id"
        ))
        .bind(message_ids)
        .bind(channel.id)
        .fetch_all(&mut *self.conn)
        .await?;
        if messages.is_empty() {
            return Err(Unsupported("a transcript of no chat messages").into());
        }
        self.generate_transcript(
            channel,
            messages,
            &HashMap::new(),
            TranscriptOpts::default(),
        )
        .await
    }

    /// `TranscriptService.new(channel, acting_user, messages_or_ids:
    /// messages, thread_ranges:, opts:).generate_markdown` for messages in
    /// the order given.
    pub(crate) async fn generate_transcript(
        &mut self,
        channel: &ChannelRow,
        mut messages: Vec<TranscriptMessage>,
        thread_ranges: &HashMap<i64, Option<String>>,
        opts: TranscriptOpts,
    ) -> Result<String, AppError> {
        if messages.is_empty() {
            return Err(Unsupported("a transcript of no chat messages").into());
        }
        let threading = channel.threading_enabled;
        let mut threads = self
            .transcript_threads(messages.iter().filter_map(|m| m.thread_id))
            .await?;
        let mut thread_id = if threading {
            messages[0].thread_id
        } else {
            None
        };
        // Only a thread's original message: its replies come with it.
        if let Some(thread) = thread_id.and_then(|t| threads.get(&t))
            && messages.len() == 1
            && thread.original_message_id == messages[0].id
        {
            let replies: Vec<TranscriptMessage> = sqlx::query_as(&format!(
                "SELECT {TRANSCRIPT_COLUMNS} FROM chat_messages \
                 LEFT JOIN users ON users.id = chat_messages.user_id \
                 WHERE chat_messages.thread_id = $1 AND chat_messages.id <> $2 \
                   AND chat_messages.deleted_at IS NULL \
                 ORDER BY chat_messages.created_at, chat_messages.id"
            ))
            .bind(thread.id)
            .bind(thread.original_message_id)
            .fetch_all(&mut *self.conn)
            .await?;
            messages.extend(replies);
            threads = self
                .transcript_threads(messages.iter().filter_map(|m| m.thread_id))
                .await?;
        }
        let markdown = self.messages_markdown(&messages).await?;
        let mut reactions: HashMap<i64, Vec<String>> = HashMap::new();
        if opts.include_reactions {
            let ids: Vec<i64> = messages.iter().map(|m| m.id).collect();
            let rows: Vec<(String, Option<String>, i64)> = sqlx::query_as(
                "SELECT emoji, STRING_AGG(DISTINCT users.username, ',') AS usernames, chat_message_id \
                 FROM chat_message_reactions \
                 INNER JOIN users ON users.id = chat_message_reactions.user_id \
                 WHERE chat_message_id = ANY($1) \
                 GROUP BY emoji, chat_message_id \
                 ORDER BY chat_message_id, emoji",
            )
            .bind(&ids)
            .fetch_all(&mut *self.conn)
            .await?;
            for (emoji, usernames, message_id) in rows {
                reactions
                    .entry(message_id)
                    .or_default()
                    .push(format!("{emoji}:{}", usernames.unwrap_or_default()));
            }
        }
        let title = self.channel(channel, None, None, None).await?["title"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let default_thread_title = self
            .i18n
            .t("chat.transcript.default_thread_title")
            .unwrap_or("Thread")
            .to_string();
        let drawn = Drawn {
            title: &title,
            channel_id: channel.id,
            markdown: &markdown,
            threads: &threads,
            thread_ranges,
            default_thread_title: &default_thread_title,
        };
        let reactions_of = |m: &TranscriptMessage| -> Vec<String> {
            if opts.include_reactions {
                reactions.get(&m.id).cloned().unwrap_or_default()
            } else {
                Vec::new()
            }
        };

        let first_user = messages[0].user_id;
        let chained = !messages.iter().all(|m| m.user_id == first_user);
        let mut open = BBCode::new(true, messages.len() > 1, chained, opts, thread_id);
        // group_messages: by thread (else alone), in the order each first
        // appears.
        let groups: Vec<Vec<&TranscriptMessage>> = if threading {
            let mut keys: Vec<(bool, i64)> = Vec::new();
            let mut grouped: HashMap<(bool, i64), Vec<&TranscriptMessage>> = HashMap::new();
            for m in &messages {
                let key = match m.thread_id {
                    Some(t) => (true, t),
                    None => (false, m.id),
                };
                if !grouped.contains_key(&key) {
                    keys.push(key);
                }
                grouped.entry(key).or_default().push(m);
            }
            keys.iter()
                .map(|k| grouped.remove(k).unwrap_or_default())
                .collect()
        } else {
            messages.iter().map(|m| vec![m]).collect()
        };
        let mut rendered: Vec<String> = Vec::new();
        // Kept from one thread to the next as Rails keeps it: cleared only
        // for a group with replies.
        let mut rendered_thread: Vec<String> = Vec::new();
        let mut previous: Option<&TranscriptMessage> = None;
        for group in &groups {
            let message = group[0];
            if let Some(p) = previous {
                let user_changed = p.user_id != message.user_id;
                let thread_changed = threading && p.thread_id != message.thread_id;
                if user_changed || thread_changed {
                    rendered.push(drawn.render(&open)?);
                    thread_id = if threading { message.thread_id } else { None };
                    open = BBCode::new(false, false, chained, opts, thread_id);
                }
            }
            open.messages.push((message.clone(), reactions_of(message)));
            previous = Some(message);
            if !threading {
                continue;
            }
            if group.len() > 1 {
                rendered_thread.clear();
                let mut previous_reply: Option<&TranscriptMessage> = None;
                let mut tag = BBCode::new(false, false, chained, opts, None);
                for reply in &group[1..] {
                    if previous_reply.is_some_and(|p| p.user_id != reply.user_id) {
                        rendered_thread.push(drawn.render(&tag)?);
                        tag = BBCode::new(false, false, chained, opts, None);
                    }
                    tag.messages.push(((*reply).clone(), reactions_of(reply)));
                    previous_reply = Some(reply);
                }
                rendered_thread.push(drawn.render(&tag)?);
            }
            if let Some(t) = group[0].thread_id
                && drawn.thread(t)?.replies_count > 0
            {
                open.threads_markdown.insert(t, rendered_thread.join("\n"));
            }
        }
        rendered.push(drawn.render(&open)?);
        Ok(rendered.join("\n"))
    }

    async fn transcript_threads(
        &mut self,
        ids: impl Iterator<Item = i64>,
    ) -> Result<HashMap<i64, ThreadRow>, AppError> {
        let mut ids: Vec<i64> = ids.collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<ThreadRow> = sqlx::query_as(
            "SELECT id, original_message_id, title, replies_count FROM chat_threads WHERE id = ANY($1)",
        )
        .bind(&ids)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(rows.into_iter().map(|t| (t.id, t)).collect())
    }

    /// `Chat::Message#to_markdown` for each: the text, then its uploads by
    /// when they were referenced, one a line.
    async fn messages_markdown(
        &mut self,
        messages: &[TranscriptMessage],
    ) -> Result<HashMap<i64, String>, AppError> {
        let ids: Vec<i64> = messages.iter().map(|m| m.id).collect();
        #[allow(clippy::type_complexity)]
        let uploads: Vec<(
            i64,
            String,
            Option<i32>,
            Option<i32>,
            String,
            Option<String>,
            i64,
        )> = sqlx::query_as(
            "SELECT upload_references.target_id, uploads.original_filename, uploads.width, \
                        uploads.height, uploads.sha1, uploads.extension, uploads.filesize::bigint \
                 FROM upload_references JOIN uploads ON uploads.id = upload_references.upload_id \
                 WHERE upload_references.target_type = 'ChatMessage' \
                   AND upload_references.target_id = ANY($1) \
                 ORDER BY upload_references.created_at, upload_references.id",
        )
        .bind(&ids)
        .fetch_all(&mut *self.conn)
        .await?;
        let mut by_message: HashMap<i64, Vec<String>> = HashMap::new();
        for (id, filename, width, height, sha1, extension, size) in uploads {
            let md = upload_markdown(&filename, width, height, &sha1, extension.as_deref(), size);
            if !md.is_empty() {
                by_message.entry(id).or_default().push(md);
            }
        }
        Ok(messages
            .iter()
            .map(|m| {
                let text = m.message.clone().unwrap_or_default();
                let md = match by_message.get(&m.id) {
                    None => text,
                    Some(uploads) if !text.trim().is_empty() => {
                        let mut parts = vec![format!("{text}\n")];
                        parts.extend(uploads.iter().cloned());
                        parts.join("\n")
                    }
                    Some(uploads) => uploads.join("\n"),
                };
                (m.id, md)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{serialize_attr, upload_markdown};

    #[test]
    fn serialize_attr_picks_marks_the_value_lacks() {
        assert_eq!(serialize_attr("channel", "General"), "channel=\"General\"");
        assert_eq!(
            serialize_attr("channel", "say \"hi\""),
            "channel='say \"hi\"'"
        );
    }

    #[test]
    fn uploads_are_written_by_their_kind() {
        let sha1 = "a".repeat(40);
        assert!(
            upload_markdown("cat.png", Some(10), Some(20), &sha1, Some("png"), 5)
                .starts_with("![cat.png|10x20](upload://")
        );
        assert!(
            upload_markdown("song.mp3", None, None, &sha1, Some("mp3"), 5)
                .starts_with("![song.mp3|audio](upload://")
        );
        let doc = upload_markdown("a[b].pdf", None, None, &sha1, Some("pdf"), 5);
        assert!(doc.starts_with("[ab.pdf|attachment](upload://"), "{doc}");
        assert!(doc.ends_with(".pdf) (5 Bytes)"), "{doc}");
    }
}
