//! `UserNotifications#policy_email` (UserNotificationsExtension) with its
//! views, user_notifications/policy_email.html.erb and .text.erb: the
//! policy's post, a link to respond and the unsubscribe links, under an
//! unsubscribe key of the plugin's `policy_email` type.

use sqlx::PgConnection;

use crate::email::notification::{Built, html_escape, layout_view};
use crate::email::sender::Ctx;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// The unsubscribe key type and email type.
pub const EMAIL_TYPE: &str = "policy_email";

pub async fn build(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
    to: &str,
    post_id: i32,
) -> Result<Built, AppError> {
    let s = ctx.settings;
    let i18n = ctx.i18n;
    let t = |key: &str, args: &[(&str, &str)]| -> Result<String, AppError> {
        Ok(if args.is_empty() {
            i18n.t(key).map(str::to_string)
        } else {
            i18n.t_with(key, args)
        }
        .ok_or(Unsupported("a missing email translation"))?)
    };
    let urls = Urls {
        config: ctx.config,
        settings: s,
    };
    let base_url = urls.base_url()?;
    let preferences_path = format!("{base_url}/my/preferences/emails");
    let key = crate::email::unsubscribe::create_key_for(conn, user_id, EMAIL_TYPE, None).await?;
    let unsubscribe_link = format!("{base_url}/email/unsubscribe/{key}");

    #[derive(sqlx::FromRow)]
    struct PostRow {
        topic_id: i32,
        post_number: i32,
        cooked: String,
        title: String,
        slug: Option<String>,
    }
    let post: PostRow = sqlx::query_as(
        "SELECT p.topic_id, p.post_number, p.cooked, t.title, t.slug FROM posts p \
         JOIN topics t ON t.id = p.topic_id WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_one(&mut *conn)
    .await?;
    let locale: Option<String> = sqlx::query_scalar("SELECT locale FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    let private_email = s.get("private_email")?.truthy();
    // Post#url(without_slug: private_email)
    let url = if private_email {
        format!("/t/{}/{}", post.topic_id, post.post_number)
    } else {
        format!(
            "/t/{}/{}/{}",
            post.slug.as_deref().unwrap_or_default(),
            post.topic_id,
            post.post_number
        )
    };
    let post_url = format!(
        "/t/{}/{}/{}",
        post.slug.as_deref().unwrap_or_default(),
        post.topic_id,
        post.post_number
    );
    // Rtl.new(user).css_class
    let default_locale = s.get("default_locale")?.to_s();
    let reading = if s.get("allow_user_locale")?.truthy() {
        locale.as_deref().unwrap_or(&default_locale)
    } else {
        default_locale.as_str()
    };
    let classes = if matches!(reading, "ar" | "fa_IR" | "he" | "ug" | "ur") {
        "rtl"
    } else {
        ""
    };
    let subject = t(
        "user_notifications.policy_email.subject",
        &[("topic_title", &post.title)],
    )?;
    let visit_topic = t("user_notifications.policy_email.visit_topic", &[])?;
    let your_settings = t(
        "user_notifications.policy_email.your_policy_email_settings",
        &[],
    )?;
    let click_here = t("user_notifications.digest.click_here", &[])?;

    // policy_email.html.erb
    let content = if private_email {
        format!(
            "    <p>{}</p>\n",
            html_escape(&t("system_messages.contents_hidden", &[])?)
        )
    } else {
        let logo_small = crate::admin_users::logo_small_url(&mut *conn, s).await?;
        let partial = crate::email::notification::post_partial(
            &mut *conn,
            ctx,
            &urls,
            logo_small.as_deref(),
            post_id,
            s.get("post_excerpts_in_emails")?.truthy(),
        )
        .await?;
        format!("    {partial}\n")
    };
    let link_to = |text: &str, href: &str| {
        format!(
            "<a href=\"{}\">{}</a>",
            html_escape(href),
            html_escape(text)
        )
    };
    let visit_html = t(
        "user_notifications.policy_email.visit_link_to_respond",
        &[(
            "visit_topic_link",
            &link_to(&visit_topic, &format!("{base_url}{url}")),
        )],
    )?;
    let unsubscribe_html = t(
        "user_notifications.policy_email.unsubscribe",
        &[
            (
                "email_preferences_link",
                &link_to(&your_settings, &preferences_path),
            ),
            ("unsubscribe_link", &link_to(&click_here, &unsubscribe_link)),
        ],
    )?;
    let view = format!(
        "<div id='main' class={classes}>\n{content}\n  <hr>\n  <div class='footer undecorated-link-footer highlight'>\n    <p>\n      {visit_html}\n    </p>\n  </div>\n  <div class='footer'>\n    <p>\n      {unsubscribe_html}\n    </p>\n  </div>\n</div>\n\n<div itemscope itemtype=\"http://schema.org/EmailMessage\" style=\"display:none\">\n  <div itemprop=\"action\" itemscope itemtype=\"http://schema.org/ViewAction\">\n    <link itemprop=\"url\" href=\"{}{}\" />\n    <meta itemprop=\"name\" content=\"{}\"/>\n  </div>\n</div>\n",
        html_escape(&base_url),
        html_escape(&url),
        html_escape(&t("read_full_topic", &[])?),
    );
    let html_part = layout_view(
        &crate::email::notification::email_template(s)?,
        &view,
        &default_locale.replacen('_', "-", 1),
        "",
    );

    // policy_email.text.erb, its links numbered by MarkdownLinker (the
    // site link first, though the view never shows it).
    let absolute = |u: &str| {
        if u.starts_with(&base_url) {
            u.to_string()
        } else {
            format!("{base_url}{u}")
        }
    };
    let links = [
        absolute("/"),
        absolute(&format!("{base_url}{url}")),
        absolute(&preferences_path),
        absolute(&unsubscribe_link),
    ];
    let body = if private_email {
        t("system_messages.contents_hidden", &[])?
    } else {
        // format_for_email(@post, true): Post#excerpt.
        let excerpt = crate::excerpt::excerpt(
            &post.cooked,
            s.get("post_excerpt_maxlength")?.to_i().max(0) as usize,
            &crate::excerpt::Options {
                post_url: Some(post_url.clone()),
                ..Default::default()
            },
            crate::excerpt::Plugins::from_settings(s)?,
        );
        crate::email::notification::format_for_email(&excerpt, &urls, i18n, Some(&post_url))?
    };
    let visit_text = t(
        "user_notifications.policy_email.visit_link_to_respond",
        &[("visit_topic_link", &format!("[{visit_topic}][2]"))],
    )?;
    let unsubscribe_text = t(
        "user_notifications.policy_email.unsubscribe",
        &[
            ("email_preferences_link", &format!("[{your_settings}][3]")),
            ("unsubscribe_link", &format!("[{click_here}][4]")),
        ],
    )?;
    let references: String = links
        .iter()
        .enumerate()
        .map(|(i, l)| format!("[{}]: {l}\n", i + 1))
        .collect();
    let text = format!("\n{body}\n\n{visit_text}\n\n{unsubscribe_text}\n\n{references}\n");

    // Email::MessageBuilder's headers: the site's alias on the
    // notification address, the preferences as List-Unsubscribe.
    let site_title = s
        .get("email_site_title")?
        .presence()
        .unwrap_or_else(|| s.get("title").map(|t| t.to_s()).unwrap_or_default());
    let notification_email = s.get("notification_email")?.to_s();
    let cleanup = |name: &str| name.replace([':', '<', '>', ',', '"'], "");
    let from = if site_title.is_empty() {
        notification_email
    } else {
        format!("\"{}\" <{notification_email}>", cleanup(&site_title))
    };
    let mut headers = vec![
        ("To".to_string(), to.to_string()),
        ("Subject".to_string(), subject),
        ("From".to_string(), from.clone()),
        (
            "List-Unsubscribe".to_string(),
            format!("<{base_url}/my/preferences>"),
        ),
        ("X-Auto-Response-Suppress".to_string(), "All".to_string()),
        ("x-ms-reactions".to_string(), "disallow".to_string()),
        ("Reply-To".to_string(), from),
    ];
    for item in s.get("email_custom_headers")?.to_s().split('|') {
        if let Some((name, value)) = item.split_once(':') {
            let (name, value) = (name.trim(), value.trim());
            if !name.is_empty() && !value.is_empty() {
                headers.push((name.to_string(), value.to_string()));
            }
        }
    }
    Ok(Built {
        message: crate::email::Message {
            headers,
            text,
            html: None,
        },
        html_part,
        view_rendered: true,
    })
}
