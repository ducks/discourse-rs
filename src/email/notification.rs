//! `UserNotifications#notification_email` (app/mailers/user_notifications.rb)
//! with Email::MessageBuilder: the subject, text body, headers and HTML
//! part of a notification email, before Email::Sender finishes it.
//!
//! The views (email/notification, email/_post, layouts/email_template and
//! the default EmailStyle template) are reproduced as Rails renders them,
//! Erubi's line trimming included. Messages, context
//! posts, in-reply-to excerpts, translation overrides, private_email and
//! the daily limit notice are refused.

use serde_json::Value;
use sqlx::PgConnection;

use super::Message;
use crate::pretty_text::{self, MarkdownOptions};
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// What `user_email` hands the mailer for a notification.
pub struct Request<'a> {
    /// `user_replied`, `user_mentioned`, ...
    pub email_type: &'a str,
    /// `replied`, `mentioned`, ...
    pub notification_type: &'a str,
    pub data: &'a Value,
    pub user_id: i32,
    pub post_id: i32,
}

/// A message as `build_email` leaves it, for Email::Sender.
pub struct Built {
    pub message: Message,
    /// Email::Renderer works on this before the sender restyles it.
    pub html_part: String,
    /// A view rendered the body (ActionMailer's templates, not
    /// html_override): the multipart type carries the charset.
    pub view_rendered: bool,
}

/// `ERB::Util.html_escape`
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// `I18n.t(key, args)` with Rails' interpolation, where an argument the
/// string doesn't use is ignored.
pub(super) fn t(
    ctx: &super::sender::Ctx<'_>,
    key: &str,
    args: &[(&str, &str)],
) -> Result<String, AppError> {
    let raw = ctx
        .i18n
        .t(key)
        .ok_or(Unsupported("a missing email translation"))?;
    let mut out = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find("%{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let name = &after[..end];
        match args.iter().find(|(k, _)| *k == name) {
            Some((_, v)) => out.push_str(v),
            None => return Err(Unsupported("an email translation missing an argument").into()),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[derive(sqlx::FromRow)]
struct PostRow {
    id: i32,
    topic_id: i32,
    post_number: i32,
    post_type: i32,
    raw: String,
    cooked: String,
    created_at: chrono::NaiveDateTime,
    user_id: i32,
    username: String,
    username_lower: String,
    name: Option<String>,
    title: Option<String>,
    uploaded_avatar_id: Option<i32>,
    slug: Option<String>,
    archetype: String,
    category_id: Option<i32>,
    reply_to_post_number: Option<i32>,
}

#[derive(sqlx::FromRow)]
struct Recipient {
    username: String,
    email: Option<String>,
    staged: bool,
    suspended: bool,
    mailing_list_mode: bool,
    email_in_reply_to: bool,
    email_previous_replies: i32,
}

/// `user_notifications.user_<type>` (`UserNotifications.<email_type>`).
pub async fn build(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    ctx: &super::sender::Ctx<'_>,
    req: &Request<'_>,
) -> Result<Built, AppError> {
    let s = ctx.settings;
    let urls = Urls {
        config: ctx.config,
        settings: s,
    };
    let base_url = urls.base_url()?;
    // The per-type options of user_replied, user_mentioned, ...
    let add_re = match req.email_type {
        "user_replied" | "user_quoted" | "user_linked" | "user_mentioned" => false,
        "user_posted" | "user_watching_first_post" | "user_private_message" => true,
        // user_invited_to_private_message: the invite template.
        "user_invited_to_private_message" => false,
        other => {
            tracing::warn!(email_type = other, "notification email type not ported");
            return Err(Unsupported("this notification email type").into());
        }
    };
    let post: PostRow = sqlx::query_as(
        "SELECT p.id, p.topic_id, p.post_number, p.post_type, p.raw, p.cooked, p.created_at, p.user_id, \
                u.username, u.username_lower, u.name, u.title, u.uploaded_avatar_id, t.slug, t.archetype, \
                t.category_id, p.reply_to_post_number \
         FROM posts p JOIN topics t ON t.id = p.topic_id JOIN users u ON u.id = p.user_id WHERE p.id = $1",
    )
    .bind(req.post_id)
    .fetch_one(&mut *conn)
    .await?;
    let pm = post.archetype == "private_message";
    // user_private_message: the posted template, no category or tags in
    // the subject.
    let pm_email = req.email_type == "user_private_message";
    // post.topic.allowed_groups.first: what show_group_in_subject names.
    let first_group: Option<(String, Option<String>)> = if pm {
        sqlx::query_as(
            "SELECT g.name, g.full_name FROM topic_allowed_groups tag JOIN groups g ON g.id = tag.group_id \
             WHERE tag.topic_id = $1 ORDER BY tag.id LIMIT 1",
        )
        .bind(post.topic_id)
        .fetch_optional(&mut *conn)
        .await?
    } else {
        None
    };
    let user: Recipient = sqlx::query_as(
        "SELECT u.username, (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\" LIMIT 1) AS email, \
                u.staged, COALESCE(u.suspended_till > now(), FALSE) AS suspended, \
                o.mailing_list_mode, o.email_in_reply_to, o.email_previous_replies \
         FROM users u JOIN user_options o ON o.user_id = u.id WHERE u.id = $1",
    )
    .bind(req.user_id)
    .fetch_one(&mut *conn)
    .await?;
    // A staged user's email keeps the subject of the email that started
    // the topic.
    if user.staged {
        let incoming: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM incoming_emails ie JOIN posts p ON p.id = ie.post_id \
             WHERE p.topic_id = $1 AND p.post_number = 1)",
        )
        .bind(post.topic_id)
        .fetch_one(&mut *conn)
        .await?;
        if incoming {
            return Err(Unsupported("emails to staged users of a topic started by email").into());
        }
    }
    if s.get("private_email")?.truthy() {
        return Err(Unsupported("private_email").into());
    }
    let to = user
        .email
        .clone()
        .ok_or(Unsupported("a recipient without an email"))?;

    // notification_email
    let data_str = |k: &str| req.data.get(k).and_then(Value::as_str).map(str::to_string);
    let invite = req.email_type == "user_invited_to_private_message";
    let mut user_name = data_str("original_username").unwrap_or_default();
    if s.get("enable_names")?.truthy() && s.get("display_name_on_email_from")?.truthy() {
        // The name of original_user_id, else the post's author.
        let named = req
            .data
            .get("original_user_id")
            .and_then(Value::as_i64)
            .map_or(post.user_id, |id| id as i32);
        let name: Option<String> = sqlx::query_scalar("SELECT name FROM users WHERE id = $1")
            .bind(named)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
        if let Some(name) = name.filter(|n| !n.trim().is_empty()) {
            user_name = name;
        }
    }
    // user_invited_to_private_message: allow_reply_by_email false.
    let allow_reply_by_email = !invite && !user.suspended;
    let original_username = data_str("original_username")
        .or_else(|| Some(post.username.clone()))
        .unwrap_or_default();
    let topic_title = data_str("topic_title").unwrap_or_default();
    let site_title = s.get("title")?.to_s();
    let email_site_title = s
        .get("email_site_title")?
        .presence()
        .unwrap_or_else(|| site_title.clone());
    let from_alias = t(
        ctx,
        "email_from",
        &[("user_name", &user_name), ("site_name", &email_site_title)],
    )?;
    // group_name: the group a group's invitation is for.
    let group_name: Option<String> = match req.data.get("group_id").and_then(Value::as_i64) {
        Some(_) if !invite => return Err(Unsupported("group notification emails").into()),
        Some(id) => {
            sqlx::query_scalar("SELECT name FROM groups WHERE id = $1")
                .bind(id as i32)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };

    // send_notification_email
    let add_re_to_subject = add_re && post.post_number > 1;
    let notification_type = if pm_email {
        "posted"
    } else {
        req.notification_type
    };
    let template = if pm {
        let group = if group_name.is_some() {
            "_group"
        } else if user.staged {
            "_staged"
        } else {
            ""
        };
        format!("user_notifications.user_{notification_type}_pm{group}")
    } else {
        format!("user_notifications.user_{notification_type}")
    };
    let category: Option<(String, Option<i32>)> = match post.category_id {
        Some(id) => {
            sqlx::query_as("SELECT name, parent_category_id FROM categories WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    let uncategorized = s.get("uncategorized_category_id")?.to_i() as i32;
    let show_category_in_subject = match (&category, post.category_id) {
        _ if pm_email => None,
        (Some((name, parent)), Some(id)) if id != uncategorized => match parent {
            Some(parent) => {
                let parent_name: String =
                    sqlx::query_scalar("SELECT name FROM categories WHERE id = $1")
                        .bind(parent)
                        .fetch_one(&mut *conn)
                        .await?;
                Some(format!("{parent_name}/{name}"))
            }
            None => Some(name.clone()),
        },
        _ => None,
    };
    // tags the user may see, most used first
    let max_tags = if s.get("enable_max_tags_per_email_subject")?.truthy() {
        s.get("max_tags_per_email_subject")?.to_i()
    } else {
        s.get("max_tags_per_topic")?.to_i()
    };
    let restricted_tags: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_tags tt JOIN tag_group_memberships tgm ON tgm.tag_id = tt.tag_id \
           JOIN tag_group_permissions tgp ON tgp.tag_group_id = tgm.tag_group_id \
           WHERE tt.topic_id = $1 AND tgp.group_id <> 0)",
    )
    .bind(post.topic_id)
    .fetch_one(&mut *conn)
    .await?;
    if restricted_tags {
        return Err(Unsupported("restricted tags in email subjects").into());
    }
    let tags: Vec<String> = sqlx::query_scalar(
        "SELECT t.name FROM tags t JOIN topic_tags tt ON tt.tag_id = t.id WHERE tt.topic_id = $1 \
         ORDER BY t.public_topic_count DESC, t.name ASC LIMIT $2",
    )
    .bind(post.topic_id)
    .bind(max_tags)
    .fetch_all(&mut *conn)
    .await?;
    let show_tags_in_subject = (!tags.is_empty() && !pm_email).then(|| tags.join(" "));

    // get_context_posts: never (2) gives none; otherwise refused.
    if user.email_previous_replies != 2 {
        let earlier: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM posts WHERE topic_id = $1 AND post_number < $2 \
               AND NOT user_deleted AND NOT hidden AND post_type = 1 AND deleted_at IS NULL)",
        )
        .bind(post.topic_id)
        .bind(post.post_number)
        .fetch_one(&mut *conn)
        .await?;
        let context_limit = s.get("email_posts_context")?.to_i();
        if earlier && context_limit > 0 {
            let last_emailed: Option<i32> = sqlx::query_scalar(
                "SELECT last_emailed_post_number FROM topic_users WHERE topic_id = $1 AND user_id = $2",
            )
            .bind(post.topic_id)
            .bind(req.user_id)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
            let unless_emailed_excludes = user.email_previous_replies == 1
                && last_emailed.is_some_and(|n| n >= post.post_number - 1);
            if !unless_emailed_excludes {
                return Err(Unsupported("previous replies in notification emails").into());
            }
        }
    }
    // An overridden body is sent without the notification view (the
    // renderer cooks the text instead); an overridden subject gets
    // MessageBuilder's augmented arguments.
    let body_overridden = ctx
        .i18n
        .override_of(&format!("{template}.text_body_template"))
        .is_some();
    let subject_overridden = ctx
        .i18n
        .override_of(&format!("{template}.subject_template"))
        .is_some();
    let max_per_day = s.get("max_emails_per_day_per_user")?.to_i();
    if max_per_day > 0 {
        let sent: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM email_logs WHERE user_id = $1 AND created_at > now() - interval '1 day'",
        )
        .bind(req.user_id)
        .fetch_one(&mut *conn)
        .await?;
        if sent >= max_per_day - 1 {
            return Err(Unsupported("the daily email limit notice").into());
        }
    }
    if user.email_in_reply_to && post.reply_to_post_number.is_some() {
        return Err(Unsupported("in-reply-to excerpts in notification emails").into());
    }
    let message = if invite {
        // use_invite_template: the first post's excerpt and the topic.
        let excerpt = if post.post_number == 1 {
            crate::excerpt::excerpt(
                &post.cooked,
                s.get("post_excerpt_maxlength")?.to_i().max(0) as usize,
                &crate::excerpt::Options::default(),
            )
            .replace('\n', " ")
        } else {
            String::new()
        };
        let topic_url = format!(
            "{base_url}/t/{}/{}",
            post.slug.as_deref().unwrap_or_default(),
            post.topic_id
        );
        let body = if group_name.is_some() {
            "user_notifications.invited_group_to_private_message_body"
        } else {
            "user_notifications.invited_to_private_message_body"
        };
        t(
            ctx,
            body,
            &[
                ("username", &original_username),
                ("group_name", group_name.as_deref().unwrap_or_default()),
                ("topic_title", &s.emojis().gsub_to_unicode(&topic_title)),
                ("topic_excerpt", &excerpt),
                ("site_title", &site_title),
                ("site_description", &s.get("site_description")?.to_s()),
                ("topic_url", &topic_url),
            ],
        )?
    } else {
        format!("{}\n\n", post.raw)
    };
    let slug = post.slug.clone().unwrap_or_default();
    // Post#url: the post number even for the first post (only share_url
    // leaves it off).
    let url = format!(
        "{}/t/{slug}/{}/{}",
        ctx.config.globals.relative_url_root(),
        post.topic_id,
        post.post_number
    );

    // The views.
    let first_footer_classes = if user.suspended || user.staged {
        ""
    } else {
        "highlight"
    };
    let logo_small = crate::admin_users::logo_small_url(&mut *conn, s).await?;
    let avatar = crate::avatar::avatar_template(
        &urls,
        post.user_id,
        &post.username,
        post.uploaded_avatar_id,
        logo_small.as_deref(),
    )?
    .replace("{size}", "45");
    let avatar = urls.absolute(&avatar)?;
    let small_avatar_url = avatar
        .split_once(':')
        .map(|(_, rest)| rest.to_string())
        .unwrap_or(avatar);
    let post_html = render_post(
        ctx,
        &PostPartial {
            whisper: post.post_type == crate::posting::post_types::WHISPER,
            small_avatar_url: &small_avatar_url,
            username: &post.username,
            username_lower: &post.username_lower,
            name: post.name.as_deref(),
            title: post.title.as_deref(),
            created_at: post.created_at,
            body: &format_for_email(
                &post.cooked,
                &urls,
                ctx.i18n,
                Some(&format!(
                    "/t/{}/{}/{}",
                    post.slug.as_deref().unwrap_or_default(),
                    post.topic_id,
                    post.post_number
                )),
            )?,
            base_url: &base_url,
        },
    )?;
    let notification_html = format!(
        "<div id='main' class=>\n\n  <div class='email-preview'>%{{email_preview}}</div>\n\n  <div class='header-instructions'>%{{header_instructions}}</div>\n\n    {post_html}\n\n\n\n\n\n  <div class='footer undecorated-link-footer {first_footer_classes}'>%{{respond_instructions}}</div>\n  <div class='footer'>%{{unsubscribe_instructions}}</div>\n\n</div>\n\n<div itemscope itemtype=\"http://schema.org/EmailMessage\" style=\"display:none\">\n  <div itemprop=\"action\" itemscope itemtype=\"http://schema.org/ViewAction\">\n    <link itemprop=\"url\" href=\"{}{}\" />\n    <meta itemprop=\"name\" content=\"{}\"/>\n  </div>\n</div>\n",
        html_escape(&base_url),
        html_escape(&url),
        html_escape(&t(ctx, "read_full_topic", &[])?),
    );

    // TopicUser.change(last_emailed_post_number), and post.unsubscribe_url.
    crate::posting::change_topic_user(
        bus,
        &mut *conn,
        req.user_id,
        post.topic_id,
        &[crate::posting::TopicUserAttr::LastEmailedPostNumber(
            post.post_number,
        )],
    )
    .await?;
    let key = super::unsubscribe::create_key_for(
        &mut *conn,
        req.user_id,
        super::unsubscribe::TOPIC_TYPE,
        Some((post.topic_id, post.id)),
    )
    .await?;
    let unsubscribe_url = format!("{base_url}/email/unsubscribe/{key}");

    // Email::MessageBuilder
    let email_prefix = s
        .get("email_prefix")?
        .presence()
        .unwrap_or_else(|| site_title.clone());
    let preview = ctx
        .i18n
        .t(&format!("{template}.preview"))
        .map(str::to_string);
    let header_instructions = t(ctx, "user_notifications.header_instructions", &[])?;
    let reply_by_email = s.get("reply_by_email_enabled")?.truthy()
        && s.get("reply_by_email_address")?.presence().is_some()
        && allow_reply_by_email;
    let mut respond_key = if reply_by_email {
        "user_notifications.reply_by_email".to_string()
    } else {
        "user_notifications.visit_link_to_respond".to_string()
    };
    // A message's instructions name its participants (only a button
    // when the system user sent it).
    let participants = if pm {
        Some(
            pm_participants(
                conn,
                ctx,
                &base_url,
                post.topic_id,
                post.post_number,
                req.user_id,
            )
            .await?,
        )
    } else {
        None
    };
    if pm {
        // Discourse.system_user.username: a message from it only links.
        let system: String = sqlx::query_scalar("SELECT username FROM users WHERE id = -1")
            .fetch_one(&mut *conn)
            .await?;
        if original_username == system {
            respond_key.push_str("_pm_button_only");
        } else {
            respond_key.push_str("_pm");
        }
    }
    let respond_args: Vec<(&str, &str)> = match &participants {
        Some(p) => vec![("base_url", &base_url), ("url", &url), ("participants", p)],
        None => vec![("base_url", &base_url), ("url", &url)],
    };
    // include_respond_instructions: not for the suspended or staged.
    let respond_instructions = if user.suspended || user.staged {
        match &participants {
            Some(p) => t(
                ctx,
                "user_notifications.pm_participants",
                &[("participants", p)],
            )?,
            None => String::new(),
        }
    } else {
        format!("---\n{}", t(ctx, &respond_key, &respond_args)?)
    };
    // UserOption#mailing_list_mode reads false while the site disables it.
    let unsubscribe_key_name =
        if user.mailing_list_mode && !s.get("disable_mailing_list_mode")?.truthy() {
            "unsubscribe_mailing_list"
        } else if s.get("unsubscribe_via_email_footer")?.truthy() {
            "unsubscribe_link_and_mail"
        } else {
            "unsubscribe_link"
        };
    // add_unsubscribe_link: not for staged users.
    let unsubscribe_instructions = if user.staged {
        String::new()
    } else {
        t(
            ctx,
            unsubscribe_key_name,
            &[("unsubscribe_url", &unsubscribe_url)],
        )?
    };

    // subject (use_site_subject)
    // subject_pm: a group message's first group (its full name when it
    // has one) when emailed as a message with group_in_subject.
    let subject_pm = match &first_group {
        Some((name, full_name)) if pm_email && s.get("group_in_subject")?.truthy() => {
            let shown = full_name.as_deref().unwrap_or(name);
            if s.get("simple_email_subject")?.truthy() {
                format!("{shown}: ")
            } else {
                format!("[{shown}] ")
            }
        }
        _ if pm => t(ctx, "subject_pm", &[])?,
        _ => String::new(),
    };
    let format_category = show_category_in_subject
        .as_ref()
        .map(|c| format!("[{c}] "))
        .unwrap_or_default();
    let format_tags = show_tags_in_subject
        .as_ref()
        .map(|t| format!("{t} "))
        .unwrap_or_default();
    let subject_re = if add_re_to_subject {
        t(ctx, "subject_re", &[])?
    } else {
        String::new()
    };
    let topic_title_unicode = s.emojis().gsub_to_unicode(&topic_title);
    // MessageBuilder's template_args: the defaults, the email options and
    // the instructions. topic_title_url_encoded, site_title_url_encoded
    // and the tag hashtags are left out, so an override naming them is
    // refused as missing an argument.
    let hostname = urls.current_hostname()?;
    let user_preferences_url = format!("{base_url}/my/preferences");
    let site_description = s.get("site_description")?.to_s();
    let post_id = post.id.to_string();
    let topic_id = post.topic_id.to_string();
    let template_args: Vec<(&str, &str)> = vec![
        ("email_prefix", &email_prefix),
        ("base_url", &base_url),
        ("user_preferences_url", &user_preferences_url),
        ("hostname", &hostname),
        ("message", &message),
        ("url", &url),
        ("post_id", &post_id),
        ("topic_id", &topic_id),
        ("context", ""),
        ("username", &original_username),
        ("group_name", group_name.as_deref().unwrap_or_default()),
        ("unsubscribe_url", &unsubscribe_url),
        ("subject_pm", &subject_pm),
        ("participants", participants.as_deref().unwrap_or_default()),
        ("site_description", &site_description),
        ("site_title", &site_title),
        ("recipient_username", &user.username),
        ("email_preview", preview.as_deref().unwrap_or_default()),
        ("header_instructions", &header_instructions),
        ("respond_instructions", &respond_instructions),
        ("unsubscribe_instructions", &unsubscribe_instructions),
    ];
    // The invite's own subject template (no use_site_subject).
    let subject = if subject_overridden {
        let mut args: Vec<(&str, &str)> = vec![
            ("site_name", &email_prefix),
            ("optional_re", &subject_re),
            ("optional_pm", if pm { &subject_pm } else { "" }),
            ("optional_cat", &format_category),
            ("optional_tags", &format_tags),
            ("topic_title", &topic_title_unicode),
        ];
        args.extend(template_args.iter().copied());
        t(ctx, &format!("{template}.subject_template"), &args)?
    } else if invite {
        let key = format!("{template}.subject_template");
        let key = if s.get("simple_email_subject")?.truthy()
            && ctx.i18n.t(&format!("{key}_improved")).is_some()
        {
            format!("{key}_improved")
        } else {
            key
        };
        t(
            ctx,
            &key,
            &[
                ("email_prefix", &email_prefix),
                ("username", &original_username),
                ("group_name", group_name.as_deref().unwrap_or_default()),
                ("topic_title", &topic_title_unicode),
            ],
        )?
    } else {
        s.get("email_subject")?
            .to_s()
            .replace("%{site_name}", &email_prefix)
            .replace("%{optional_re}", &subject_re)
            .replace("%{optional_pm}", &subject_pm)
            .replace("%{optional_cat}", &format_category)
            .replace("%{optional_tags}", &format_tags)
            .replace("%{topic_title}", &topic_title_unicode)
    };

    // body: the text template, then the unsubscribe instructions.
    // MessageBuilder#body: topic_title escaped twice, the optional parts
    // of the subject blank but the category.
    let escaped_title = html_escape(&html_escape(&topic_title_unicode));
    let mut body_args: Vec<(&str, &str)> = vec![
        ("site_name", &site_title),
        ("topic_title", &escaped_title),
        ("optional_re", ""),
        ("optional_pm", ""),
        ("optional_cat", &format_category),
    ];
    body_args.extend(template_args.iter().copied());
    let mut text = t(ctx, &format!("{template}.text_body_template"), &body_args)?;
    if !unsubscribe_instructions.is_empty() {
        text.push('\n');
        text.push_str(&unsubscribe_instructions);
    }

    // html_part: the instructions cooked into the notification view, then
    // the layout around it.
    let cook = |raw: String| async move {
        Ok::<String, AppError>(
            pretty_text::cook(ctx.host, &raw, &MarkdownOptions::default()).await?,
        )
    };
    // The invite's html_override: its message cooked, nothing else.
    let mut html = if invite {
        cook(message.clone()).await?
    } else {
        notification_html
    };
    let unsubscribe_html = if unsubscribe_instructions.is_empty() {
        String::new()
    } else {
        cook(unsubscribe_instructions.clone()).await?
    };
    html = html.replace("%{unsubscribe_instructions}", &unsubscribe_html);
    let preview_html = match &preview {
        Some(p) if !p.trim().is_empty() => cook(p.clone()).await?,
        _ => String::new(),
    };
    html = html.replace("%{email_preview}", &preview_html);
    let header_html = if header_instructions.trim().is_empty() {
        String::new()
    } else {
        cook(header_instructions.clone()).await?
    };
    html = html.replace("%{header_instructions}", &header_html);
    let respond_html = if respond_instructions.trim().is_empty() {
        String::new()
    } else {
        cook(respond_instructions.clone()).await?
    };
    html = html.replace("%{respond_instructions}", &respond_html);
    let html_lang = s.get("default_locale")?.to_s().replacen('_', "-", 1);
    let html_part = if body_overridden {
        // Email::Renderer#html without an HTML part: the text, unescaped
        // and cooked for the recipient, in the layout with the preview.
        let unescaped = html_escape::decode_html_entities(&text).into_owned();
        let cooked = pretty_text::cook(
            ctx.host,
            &unescaped,
            &MarkdownOptions {
                user_id: Some(i64::from(req.user_id)),
                ..Default::default()
            },
        )
        .await?;
        layout(&email_template(s)?, &cooked, &html_lang, &preview_html)
    } else {
        layout(&email_template(s)?, &html, &html_lang, "")
    };

    // header_args, in MessageBuilder's order.
    let notification_email = s.get("notification_email")?.to_s();
    let cleanup = |name: &str| name.replace([':', '<', '>', ',', '"'], "");
    let from = format!("\"{}\" <{notification_email}>", cleanup(&from_alias));
    let mut headers: Vec<(String, String)> = Vec::new();
    if let Some(p) = preview.as_ref().filter(|p| !p.trim().is_empty()) {
        headers.push(("X-Discourse-Email-Preview".into(), p.clone()));
    }
    if !user.staged {
        headers.push(("List-Unsubscribe".into(), format!("<{unsubscribe_url}>")));
        headers.push((
            "List-Unsubscribe-Post".into(),
            "List-Unsubscribe=One-Click".into(),
        ));
    }
    headers.push(("X-Discourse-Post-Id".into(), post.id.to_string()));
    headers.push(("X-Discourse-Topic-Id".into(), post.topic_id.to_string()));
    if let Some(tags) = &show_tags_in_subject {
        headers.push(("X-Discourse-Tags".into(), tags.clone()));
    }
    if let Some(category) = &show_category_in_subject {
        headers.push(("X-Discourse-Category".into(), category.clone()));
    }
    if !original_username.is_empty() {
        headers.push(("X-Discourse-Sender".into(), original_username.clone()));
    }
    headers.push(("X-Auto-Response-Suppress".into(), "All".into()));
    headers.push(("x-ms-reactions".into(), "disallow".into()));
    if reply_by_email {
        // The sender swaps the key in and drops the marker header.
        headers.push(("X-Discourse-Allow-Reply-By-Email".into(), "true".into()));
        headers.push((
            "Reply-To".into(),
            format!(
                "\"{}\" <{}>",
                // alias_email for a private reply, the site alias otherwise
                cleanup(if pm { &from_alias } else { &email_site_title }),
                s.get("reply_by_email_address")?.to_s()
            ),
        ));
    } else {
        headers.push(("Reply-To".into(), from.clone()));
    }
    for item in s.get("email_custom_headers")?.to_s().split('|') {
        if let Some((name, value)) = item.split_once(':') {
            let (name, value) = (name.trim(), value.trim());
            if !name.is_empty() && !value.is_empty() {
                headers.push((name.to_string(), value.to_string()));
            }
        }
    }
    let mut all = vec![
        ("To".to_string(), to),
        ("Subject".to_string(), subject),
        ("From".to_string(), from),
    ];
    all.extend(headers);
    Ok(Built {
        message: Message {
            headers: all,
            text,
            html: None,
        },
        html_part,
        view_rendered: false,
    })
}

/// `PrettyText.format_for_email(cooked, post)`: reduce_cooked's changes
/// (discourse-details: each details its summary and a link to the post,
/// `post_url`, else the site), lightbox metadata out, links absolute.
pub(crate) fn format_for_email(
    cooked: &str,
    urls: &Urls<'_>,
    i18n: &crate::i18n::I18n,
    post_url: Option<&str>,
) -> Result<String, AppError> {
    use crate::pretty_text::cleanup::{
        all_elements, all_elements_within, append, attr, element_name, new_element, new_text,
        parse, replace_with, set_attr, text, to_html,
    };
    let dom = parse(cooked);
    let base_url = urls.base_url()?;
    // reduce_cooked: discourse-details' summary and link in place of each
    // details, outermost first (the inner ones go with it).
    let label = i18n
        .t("details.excerpt_details")
        .unwrap_or_default()
        .to_string();
    while let Some(details) = all_elements(&dom)
        .into_iter()
        .find(|e| element_name(e) == Some("details"))
    {
        let summary: String = all_elements_within(&details)
            .iter()
            .filter(|e| element_name(e) == Some("summary"))
            .map(text)
            .collect();
        let link = new_element("a", &[("href", post_url.unwrap_or(&base_url))]);
        append(&link, new_text(&label));
        replace_with(&details, vec![new_text(&format!("{summary} ")), link]);
    }
    // reduce_cooked: discourse-local-dates' email previews, after
    // discourse-details' (the plugins' load order).
    if crate::plugins::local_dates::enabled(urls.settings)? {
        crate::plugins::local_dates::reduce_cooked(&dom);
    }
    let base_no_prefix = urls.base_url_no_prefix()?;
    let base_path = urls.config.globals.relative_url_root();
    let mut stack = vec![dom.document.clone()];
    let mut links = Vec::new();
    while let Some(node) = stack.pop() {
        if crate::pretty_text::cleanup::has_class(&node, "lightbox-wrapper") {
            return Err(Unsupported("lightboxes in emails").into());
        }
        // convert_vimeo_iframes; Email::Styles makes the others links.
        if crate::pretty_text::cleanup::element_name(&node) == Some("iframe")
            && attr(&node, "src").is_some_and(|s| s.contains("player.vimeo.com"))
        {
            return Err(Unsupported("Vimeo iframes in emails").into());
        }
        if crate::pretty_text::cleanup::element_name(&node) == Some("a")
            && attr(&node, "href").is_some()
        {
            links.push(node.clone());
        }
        for child in node.children.borrow().iter().rev() {
            stack.push(child.clone());
        }
    }
    for a in links {
        let href = attr(&a, "href").unwrap_or_default();
        if href.trim().is_empty()
            || href.starts_with("mailto:")
            || href.starts_with(&base_url)
            || href.contains("://")
            || href.starts_with("//")
        {
            continue;
        }
        let absolute = if href.starts_with(base_path) {
            format!("{base_no_prefix}{href}")
        } else {
            format!("{base_url}{href}")
        };
        set_attr(&a, "href", &absolute);
    }
    Ok(to_html(&dom))
}

struct PostPartial<'a> {
    whisper: bool,
    small_avatar_url: &'a str,
    username: &'a str,
    username_lower: &'a str,
    name: Option<&'a str>,
    title: Option<&'a str>,
    created_at: chrono::NaiveDateTime,
    body: &'a str,
    base_url: &'a str,
}

/// `normalize_name`
fn normalize_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .collect()
}

/// email/_post.html.erb (use_excerpt false).
fn render_post(ctx: &super::sender::Ctx<'_>, p: &PostPartial<'_>) -> Result<String, AppError> {
    let s = ctx.settings;
    let names = s.get("enable_names")?.truthy() && s.get("display_name_on_posts")?.truthy();
    let mut out = format!(
        "<div class='post-wrapper {} '>\n  <table>\n    <tr>\n      <td class='user-avatar'>\n        <img src=\"{}\" title=\"{}\">\n      </td>\n      <td>\n",
        if p.whisper { "whisper" } else { "" },
        html_escape(p.small_avatar_url),
        html_escape(p.username),
    );
    let name = p.name.filter(|n| !n.trim().is_empty());
    let distinct_name = name.is_some_and(|n| normalize_name(n) != normalize_name(p.username));
    if s.get("prioritize_username_in_ux")?.truthy() {
        out.push_str(&format!(
            "          <a rel='nofollow' class=\"username username-link\" href=\"{}/u/{}\" target=\"_blank\">{}</a>\n",
            html_escape(p.base_url),
            html_escape(p.username_lower),
            html_escape(p.username)
        ));
        if names && distinct_name {
            out.push_str(&format!(
                "            <span class='user-name username-title'>{}</span>\n",
                html_escape(name.unwrap_or_default())
            ));
        }
    } else {
        return Err(Unsupported("prioritize_username_in_ux off in emails").into());
    }
    if let Some(title) = p.title.filter(|t| !t.trim().is_empty()) {
        out.push_str(&format!(
            "          <span class='user-title'>{}</span>\n",
            html_escape(title)
        ));
    }
    let now = crate::clock::now_naive();
    let date = if chrono::Datelike::year(&p.created_at) == chrono::Datelike::year(&now) {
        p.created_at.format("%B %-d").to_string()
    } else {
        p.created_at.format("%B %-d, %Y").to_string()
    };
    out.push_str(&format!(
        "        <br>\n        <span class='notification-date'>{}</span>\n      </td>\n    </tr>\n  </table>\n  <div class='body'>{}</div>\n</div>\n",
        html_escape(&date),
        p.body
    ));
    Ok(out)
}

/// `EmailStyle.new.html`: the admin's template (email_custom_template),
/// or the one Discourse ships.
pub fn email_template(s: &crate::site_settings::SiteSettings) -> Result<String, AppError> {
    Ok(s.get("email_custom_template")?
        .presence()
        .unwrap_or_else(|| DEFAULT_TEMPLATE.to_string()))
}

/// `email_html_template`: the template with the content in place.
fn fill_template(template: &str, content: &str, html_lang: &str, preview_html: &str) -> String {
    template
        .replacen("%{email_content}", content, 1)
        .replace("%{email_preview}", preview_html)
        .replace("%{html_lang}", html_lang)
        .replace("%{dark_mode_meta_tags}", DARK_MODE_META_TAGS)
        .replace("%{dark_mode_styles}", DARK_MODE_STYLES)
}

/// layouts/email_template with EmailStyle's template around the body.
pub(crate) fn layout(
    template: &str,
    html_body: &str,
    html_lang: &str,
    preview_html: &str,
) -> String {
    // `capture { yield }`: a block that wrote only whitespace gives its
    // value instead, the "\n" its last line appended.
    let content = if html_body.trim().is_empty() {
        "\n".to_string()
    } else {
        format!("\n    \n    {html_body}\n")
    };
    fill_template(template, &content, html_lang, preview_html)
}

/// layouts/email_template around a mailer's own view (ActionMailer's
/// layout, as chat's summary renders): the view as `yield` leaves it in
/// email_html_template's block.
pub(crate) fn layout_view(
    template: &str,
    view: &str,
    html_lang: &str,
    preview_html: &str,
) -> String {
    fill_template(
        template,
        &format!("\n    {view}\n    \n"),
        html_lang,
        preview_html,
    )
}

/// app/views/email/default_template.html, as Discourse ships it.
pub const DEFAULT_TEMPLATE: &str = include_str!("default_template.html");

const DARK_MODE_META_TAGS: &str = "\n    <meta name='color-scheme' content='light dark' />\n    <meta name='supported-color-schemes' content='light dark' />\n    ";

const DARK_MODE_STYLES: &str = include_str!("dark_mode_styles.html");

/// `UserNotifications.participants(post, recipient)`: the message's other
/// people, the latest posters first, as markdown links.
async fn pm_participants(
    conn: &mut PgConnection,
    ctx: &super::sender::Ctx<'_>,
    base_url: &str,
    topic_id: i32,
    post_number: i32,
    recipient: i32,
) -> Result<String, AppError> {
    let s = ctx.settings;
    let max = s.get("max_participant_names")?.to_i();
    // The groups first, largest first: name_full_preferred (user_count).
    let groups: Vec<(String, Option<String>, i32)> = sqlx::query_as(
        "SELECT g.name, g.full_name, g.user_count FROM topic_allowed_groups tag \
         JOIN groups g ON g.id = tag.group_id WHERE tag.topic_id = $1 \
         ORDER BY g.user_count DESC, tag.id",
    )
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut list: Vec<String> = Vec::new();
    for (name, full_name, user_count) in &groups {
        if !name.is_ascii() {
            return Err(Unsupported(
                "participants with unicode group names (UrlHelper.encode_component)",
            )
            .into());
        }
        let shown = full_name
            .as_deref()
            .filter(|f| !f.is_empty())
            .unwrap_or(name);
        list.push(format!("[{shown} ({user_count})]({base_url}/g/{name})"));
        if list.len() as i64 >= max {
            break;
        }
    }
    let users: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT u.username, u.name FROM topic_allowed_users tau JOIN users u ON u.id = tau.user_id \
         LEFT JOIN (SELECT user_id, MAX(post_number) AS post_number FROM posts \
                    WHERE topic_id = $1 AND post_type = 1 AND post_number <= $2 AND user_id <> $3 \
                      AND deleted_at IS NULL \
                    GROUP BY user_id ORDER BY post_number DESC LIMIT $4) pu ON pu.user_id = tau.user_id \
         WHERE tau.topic_id = $1 AND u.id <> $3 AND u.id > 0 \
         ORDER BY pu.post_number DESC NULLS LAST, u.id",
    )
    .bind(topic_id)
    .bind(post_number)
    .bind(recipient)
    .bind(max)
    .fetch_all(&mut *conn)
    .await?;
    let full_name_first = s.get("prioritize_full_name_in_ux")?.truthy();
    for (username, name) in &users {
        if list.len() as i64 >= max {
            break;
        }
        // User#display_name
        let display = match name
            .as_deref()
            .filter(|n| full_name_first && !n.trim().is_empty())
        {
            Some(n) => n.to_string(),
            None => username.clone(),
        };
        if !username.is_ascii() {
            return Err(Unsupported(
                "participants with unicode usernames (UrlHelper.encode_component)",
            )
            .into());
        }
        list.push(format!("[{display}]({base_url}/u/{username})"));
    }
    let participants = list.join(&t(ctx, "word_connector.comma", &[])?);
    let others = (groups.len() + users.len()) as i64 - list.len() as i64;
    if others > 0 {
        let form = if others == 1 { "one" } else { "other" };
        return t(
            ctx,
            &format!("user_notifications.more_pm_participants.{form}"),
            &[
                ("participants", &participants),
                ("count", &others.to_string()),
            ],
        );
    }
    Ok(participants)
}
