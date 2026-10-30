//! EmailController's unsubscribe flow: UnsubscribeKey, the strategies
//! (EmailControllerHelper::{Base,Digest,Topic}EmailUnsubscriber) and the
//! views email/unsubscribe.html.erb and email/unsubscribed.html.erb.
//!
//! The views are written out by hand with Erubi's trimming: a line holding
//! only a `<% %>` statement leaves nothing, everything else is kept as is.

use sqlx::PgConnection;

use crate::category_badge::html_escape;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `UnsubscribeKey`'s types (plugins add theirs).
pub const DIGEST_TYPE: &str = "digest";
pub const TOPIC_TYPE: &str = "topic";
pub const ALL_TYPE: &str = "all";

/// `UnsubscribeKey.create_key_for(user, type, post:)`: a random key
/// (`SecureRandom.hex(32)`), the post and its topic for a topic key.
pub async fn create_key_for(
    conn: &mut PgConnection,
    user_id: i32,
    key_type: &str,
    post: Option<(i32, i32)>,
) -> Result<String, AppError> {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let key: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let (topic_id, post_id) = match post {
        Some((topic_id, post_id)) if key_type == TOPIC_TYPE => (Some(topic_id), Some(post_id)),
        _ => (None, None),
    };
    sqlx::query(
        "INSERT INTO unsubscribe_keys (key, user_id, unsubscribe_key_type, topic_id, post_id, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, clock_timestamp(), clock_timestamp())",
    )
    .bind(&key)
    .bind(user_id)
    .bind(key_type)
    .bind(topic_id)
    .bind(post_id)
    .execute(&mut *conn)
    .await?;
    Ok(key)
}

/// An unsubscribe key row.
#[derive(sqlx::FromRow)]
pub struct Key {
    pub user_id: i32,
    pub unsubscribe_key_type: Option<String>,
    pub topic_id: Option<i32>,
    pub post_id: Option<i32>,
}

pub async fn find_key(conn: &mut PgConnection, key: &str) -> Result<Option<Key>, AppError> {
    Ok(sqlx::query_as(
        "SELECT user_id, unsubscribe_key_type, topic_id, post_id FROM unsubscribe_keys WHERE key = $1",
    )
    .bind(key)
    .fetch_optional(&mut *conn)
    .await?)
}

/// The key's owner and the user options the flow reads.
#[derive(sqlx::FromRow)]
pub struct Owner {
    pub id: i32,
    pub name: Option<String>,
    pub email: Option<String>,
    pub mailing_list_mode: bool,
    pub email_digests: Option<bool>,
    pub digest_after_minutes: Option<i32>,
    pub email_level: i32,
    pub email_messages_level: i32,
}

pub async fn owner(conn: &mut PgConnection, user_id: i32) -> Result<Option<Owner>, AppError> {
    Ok(sqlx::query_as(
        "SELECT u.id, u.name, \
           (SELECT email FROM user_emails e WHERE e.user_id = u.id AND e.primary) AS email, \
           o.mailing_list_mode, o.email_digests, o.digest_after_minutes, o.email_level, o.email_messages_level \
         FROM users u JOIN user_options o ON o.user_id = u.id WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?)
}

/// The strategy a key's type picks (`get_unsubscribe_strategy_for`).
#[derive(Clone, Copy, PartialEq)]
pub enum Strategy {
    Base,
    Digest,
    Topic,
    /// discourse-policy's PolicyEmailUnsubscriber.
    Policy,
    /// chat's ChatSummaryUnsubscriber, registered while chat is on.
    ChatSummary,
}

/// `UnsubscribeKey::CHAT_SUMMARY`'s type (chat's register_email_unsubscriber).
pub const CHAT_SUMMARY_TYPE: &str = "chat_summary";

pub fn strategy_for(key: &Key, settings: &SiteSettings) -> Result<Strategy, AppError> {
    match key.unsubscribe_key_type.as_deref() {
        Some(DIGEST_TYPE) => Ok(Strategy::Digest),
        Some(TOPIC_TYPE) => Ok(Strategy::Topic),
        Some(ALL_TYPE) => Ok(Strategy::Base),
        Some(crate::plugins::policy::email::EMAIL_TYPE) => Ok(Strategy::Policy),
        Some(CHAT_SUMMARY_TYPE) if crate::plugins::chat::enabled(settings)? => {
            Ok(Strategy::ChatSummary)
        }
        // chat_summary with chat off, or a type nothing handles (Rails
        // then fails rendering the page).
        _ => Err(Unsupported("unsubscribe keys of plugins' email types").into()),
    }
}

/// A topic as the views link it.
#[derive(sqlx::FromRow)]
pub struct KeyTopic {
    pub id: i32,
    pub title: String,
    pub slug: Option<String>,
    pub category_id: Option<i32>,
}

/// `associated_topic`: the key's topic, else its post's (Trashable's
/// default scope leaves deleted ones out).
pub async fn associated_topic(
    conn: &mut PgConnection,
    key: &Key,
) -> Result<Option<KeyTopic>, AppError> {
    const COLUMNS: &str = "t.id, t.title, t.slug, t.category_id";
    if let Some(id) = key.topic_id {
        let topic = sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM topics t WHERE t.id = $1 AND t.deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
        if topic.is_some() {
            return Ok(topic);
        }
    }
    let Some(post_id) = key.post_id else {
        return Ok(None);
    };
    Ok(sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM posts p JOIN topics t ON t.id = p.topic_id \
         WHERE p.id = $1 AND p.deleted_at IS NULL AND t.deleted_at IS NULL"
    ))
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?)
}

/// `render_topic_title(topic)`: a link to the topic with its title's emoji
/// as unicode.
pub fn render_topic_title(
    base_path: &str,
    emojis: &crate::emojis::Emojis,
    topic: &KeyTopic,
) -> String {
    format!(
        "<a href=\"{}\">{}</a>",
        html_escape(&format!(
            "{base_path}/t/{}/{}",
            topic.slug.as_deref().unwrap_or(""),
            topic.id
        )),
        html_escape(&emojis.gsub_to_unicode(&topic.title))
    )
}

/// What the page's form shows, as the strategy's
/// `prepare_unsubscribe_options` sets it.
pub struct Options {
    pub key_type: String,
    pub email: String,
    pub mailing_list_mode: bool,
    pub unsubscribed_from_all: bool,
    pub digest_unsubscribe: bool,
    pub digest_frequencies: Option<DigestFrequencies>,
    pub topic: Option<KeyTopic>,
    pub watching_topic: bool,
    pub watched_category: bool,
    /// discourse-policy's PolicyEmailUnsubscriber: the frequencies
    /// (label, name) and the owner's.
    pub policy_email_frequencies: Option<(Vec<(String, String)>, String)>,
    /// chat's ChatSummaryUnsubscriber: the same for chat summary emails.
    pub chat_email_frequencies: Option<(Vec<(String, String)>, String)>,
}

/// The digest frequencies the page offers: the user's, the choices
/// (label, minutes) and the one selected.
pub struct DigestFrequencies {
    pub current: String,
    pub frequencies: Vec<(String, i32)>,
    pub selected: i32,
}

/// DigestEmailSiteSetting.values: (name, minutes).
pub const DIGEST_FREQUENCIES: [(&str, i32); 7] = [
    ("never", 0),
    ("every_30_minutes", 30),
    ("every_hour", 60),
    ("daily", 1440),
    ("weekly", 10_080),
    ("every_month", 43_200),
    ("every_six_months", 259_200),
];

fn translated(i18n: &I18n, key: &str) -> Result<String, AppError> {
    Ok(i18n
        .t(key)
        .ok_or(Unsupported("a missing unsubscribe translation"))?
        .to_string())
}

fn translated_with(i18n: &I18n, key: &str, args: &[(&str, &str)]) -> Result<String, AppError> {
    Ok(i18n
        .t_with(key, args)
        .ok_or(Unsupported("a missing unsubscribe translation"))?)
}

/// DigestEmailUnsubscriber's frequencies: the allowed ones (never last),
/// the one the user is on, and the next allowed one after it, a step down.
fn digest_frequencies(i18n: &I18n, owner: &Owner) -> Result<DigestFrequencies, AppError> {
    const ALLOWED: [&str; 4] = ["never", "weekly", "every_month", "every_six_months"];
    let email_digests = owner.email_digests == Some(true);
    let mut current: Option<&str> = None;
    let mut selected: Option<i32> = None;
    // Ruby's `!m[:take_next] && m[:current]`: truthy when current is set.
    let mut take_next = false;
    let mut frequencies = Vec::new();
    for (name, value) in &DIGEST_FREQUENCIES[1..] {
        if owner.digest_after_minutes == Some(*value) && email_digests {
            current = Some(name);
        }
        if !ALLOWED.contains(name) {
            continue;
        }
        if take_next && email_digests {
            selected = Some(*value);
        }
        frequencies.push((
            translated(i18n, &format!("unsubscribe.digest_frequency.{name}"))?,
            *value,
        ));
        take_next = !take_next && current.is_some();
    }
    frequencies.push((translated(i18n, "unsubscribe.digest_frequency.never")?, 0));
    Ok(DigestFrequencies {
        current: current.unwrap_or("never").to_string(),
        frequencies,
        selected: selected.unwrap_or(0),
    })
}

/// `prepare_unsubscribe_options` of the key's strategy.
pub async fn prepare(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    key: &Key,
    owner: &Owner,
) -> Result<Options, AppError> {
    let strategy = strategy_for(key, settings)?;
    // UserOption#mailing_list_mode reads false while the site disables it.
    let mailing_list_mode =
        owner.mailing_list_mode && !settings.get("disable_mailing_list_mode")?.truthy();
    let mut options = Options {
        key_type: key.unsubscribe_key_type.clone().unwrap_or_default(),
        email: owner.email.clone().unwrap_or_default(),
        mailing_list_mode,
        // UserOption#unsubscribed_from_all?: email_level never is 2.
        unsubscribed_from_all: !mailing_list_mode
            && owner.email_digests != Some(true)
            && owner.email_level == 2
            && owner.email_messages_level == 2,
        digest_unsubscribe: false,
        digest_frequencies: None,
        topic: None,
        watching_topic: false,
        watched_category: false,
        policy_email_frequencies: None,
        chat_email_frequencies: None,
    };
    match strategy {
        Strategy::Base => {}
        Strategy::ChatSummary => {
            let current: i32 = sqlx::query_scalar(
                "SELECT chat_email_frequency FROM user_options WHERE user_id = $1",
            )
            .bind(owner.id)
            .fetch_one(&mut *conn)
            .await?;
            let names = crate::plugins::chat::site_integration::EMAIL_FREQUENCIES;
            let mut frequencies = Vec::new();
            for name in names {
                frequencies.push((
                    translated(i18n, &format!("unsubscribe.chat_summary.{name}"))?,
                    name.to_string(),
                ));
            }
            let current = names
                .get(usize::try_from(current).unwrap_or(usize::MAX))
                .copied()
                .unwrap_or_default()
                .to_string();
            options.chat_email_frequencies = Some((frequencies, current));
        }
        Strategy::Policy => {
            let current: i32 = sqlx::query_scalar(
                "SELECT policy_email_frequency FROM user_options WHERE user_id = $1",
            )
            .bind(owner.id)
            .fetch_one(&mut *conn)
            .await?;
            let names = crate::plugins::policy::EMAIL_FREQUENCIES;
            let mut frequencies = Vec::new();
            for name in names {
                frequencies.push((
                    translated(i18n, &format!("unsubscribe.policy_emails.{name}"))?,
                    name.to_string(),
                ));
            }
            let current = names
                .get(usize::try_from(current).unwrap_or(usize::MAX))
                .copied()
                .unwrap_or_default()
                .to_string();
            options.policy_email_frequencies = Some((frequencies, current));
        }
        Strategy::Digest => {
            options.digest_unsubscribe = !settings.get("disable_digest_emails")?.truthy();
            options.digest_frequencies = Some(digest_frequencies(i18n, owner)?);
        }
        Strategy::Topic => {
            let Some(topic) = associated_topic(conn, key).await? else {
                return Ok(options);
            };
            options.watching_topic = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM topic_users WHERE user_id = $1 AND topic_id = $2 AND notification_level = 3)",
            )
            .bind(owner.id)
            .bind(topic.id)
            .fetch_one(&mut *conn)
            .await?;
            // @watched_count: shown when set, even at 0.
            if let Some(category_id) = topic.category_id {
                options.watched_category = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM category_users WHERE user_id = $1 AND category_id = $2 \
                       AND notification_level IN (3, 4))",
                )
                .bind(owner.id)
                .bind(category_id)
                .fetch_one(&mut *conn)
                .await?;
            }
            options.topic = Some(topic);
        }
    }
    Ok(options)
}

/// `check_box_tag(name, value = "1", checked = false)`
fn check_box(name: &str, checked: bool) -> String {
    format!(
        "<input type=\"checkbox\" name=\"{name}\" id=\"{name}\" value=\"1\"{} />",
        if checked { " checked=\"checked\"" } else { "" }
    )
}

/// `submit_tag(text, class: 'btn btn-danger')`
fn submit(text: &str) -> String {
    let text = html_escape(text);
    format!(
        "<input type=\"submit\" name=\"commit\" value=\"{text}\" class=\"btn btn-danger\" data-disable-with=\"{text}\" />"
    )
}

/// The token form_tag adds (`token_tag`), empty when forgery protection
/// is off.
fn token_tag(csrf: Option<&str>) -> String {
    csrf.map(|t| {
        format!(
            "<input type=\"hidden\" name=\"authenticity_token\" value=\"{}\" />",
            html_escape(t)
        )
    })
    .unwrap_or_default()
}

/// What email/unsubscribe.html.erb shows.
pub enum Page<'a> {
    NotFound,
    UserNotFound,
    /// Logged in as someone else: the log out form (its action the current
    /// user's session, the return url this page).
    DifferentUser {
        username_lower: &'a str,
        return_url: &'a str,
    },
    Form(&'a Options),
}

/// email/unsubscribe.html.erb
pub fn render_unsubscribe(
    settings: &SiteSettings,
    i18n: &I18n,
    urls: &crate::url::Urls<'_>,
    key: &str,
    page: &Page<'_>,
    csrf: Option<&str>,
    category_badge: &str,
) -> Result<String, AppError> {
    let base_path = urls.config.globals.relative_url_root();
    let mut out = String::from("<div class='container unsubscribe'>\n");
    let options = match page {
        Page::NotFound => {
            out.push_str(&format!(
                "\n      <p>{}</p>\n",
                html_escape(&translated(i18n, "unsubscribe.not_found_description")?)
            ));
            None
        }
        Page::UserNotFound => {
            out.push_str(&format!(
                "\n      <p>{}</p>\n",
                html_escape(&translated(i18n, "unsubscribe.user_not_found_description")?)
            ));
            None
        }
        Page::DifferentUser {
            username_lower,
            return_url,
        } => {
            out.push_str(&format!(
                "\n      <p>{}</p>\n",
                translated(i18n, "unsubscribe.different_user_description")?
            ));
            out.push_str(&format!(
                "      <form action=\"{}\" accept-charset=\"UTF-8\" method=\"post\">\
                 <input type=\"hidden\" name=\"_method\" value=\"delete\" />{}\n",
                html_escape(&format!("{base_path}/session/{username_lower}")),
                token_tag(csrf)
            ));
            out.push_str(&format!(
                "        <input type=\"hidden\" name=\"return_url\" id=\"return_url\" value=\"{}\" />\n",
                html_escape(return_url)
            ));
            out.push_str(&format!(
                "        {}\n</form>",
                submit(&translated(i18n, "unsubscribe.log_out")?)
            ));
            None
        }
        Page::Form(options) => Some(*options),
    };
    let Some(o) = options else {
        out.push_str("\n</div>\n");
        return Ok(out);
    };
    out.push_str("    <br/>\n");
    out.push_str(&format!(
        "    <h2>{}</h2>\n",
        html_escape(&translated_with(
            i18n,
            "unsubscribe.title",
            &[("email", &o.email)]
        )?)
    ));
    out.push_str("    <br/>\n");
    out.push_str(&format!(
        "    <form action=\"{}\" accept-charset=\"UTF-8\" method=\"post\">{}\n",
        html_escape(&format!("{base_path}/email/unsubscribe/{key}")),
        token_tag(csrf)
    ));
    if let Some(topic) = &o.topic {
        let link = render_topic_title(base_path, settings.emojis(), topic);
        let (name, text) = if o.watching_topic {
            ("unwatch_topic", "unsubscribe.stop_watching_topic")
        } else {
            ("mute_topic", "unsubscribe.mute_topic")
        };
        out.push_str("          <p>\n          <label>\n");
        out.push_str(&format!("            {}\n", check_box(name, true)));
        out.push_str(&format!(
            "            {}\n",
            translated_with(i18n, text, &[("link", &link)])?
        ));
        out.push_str("          </label>\n          </p>\n");
        out.push_str("\n\n");
        if o.watched_category {
            out.push_str("          <p>\n          <label>\n");
            out.push_str(&format!(
                "            {}\n",
                check_box("unwatch_category", false)
            ));
            out.push_str(&format!(
                "            {}\n",
                translated_with(
                    i18n,
                    "unsubscribe.unwatch_category",
                    &[("category", category_badge)]
                )?
            ));
            out.push_str("          </label>\n          </p>\n");
        }
    }
    out.push('\n');
    if o.mailing_list_mode {
        out.push_str("        <p>\n        <label>\n");
        out.push_str(&format!(
            "        {}\n",
            check_box("disable_mailing_list", false)
        ));
        out.push_str(&format!(
            "        {}\n",
            html_escape(&translated(i18n, "unsubscribe.mailing_list_mode")?)
        ));
        out.push_str("        </label>\n        </p>\n");
    }
    out.push('\n');
    if o.digest_unsubscribe
        && let Some(DigestFrequencies {
            current,
            frequencies,
            selected,
        }) = &o.digest_frequencies
    {
        out.push_str("          <p>\n\n");
        out.push_str("            <h3>\n");
        if current == "never" {
            out.push_str(&format!(
                "                {} \n",
                html_escape(&translated(
                    i18n,
                    "unsubscribe.digest_frequency.never_title"
                )?)
            ));
        } else {
            let frequency = translated(i18n, &format!("unsubscribe.digest_frequency.{current}"))?;
            out.push_str(&format!(
                "                {}\n",
                html_escape(&translated_with(
                    i18n,
                    "unsubscribe.digest_frequency.title",
                    &[("frequency", &frequency)]
                )?)
            ));
        }
        out.push_str("            </h3>\n            <br/>\n\n");
        out.push_str(&format!(
            "          <label>{}</label>\n",
            html_escape(&translated(
                i18n,
                "unsubscribe.digest_frequency.select_title"
            )?)
        ));
        // select_tag with options_for_select, the options joined by "\n".
        let options: Vec<String> = frequencies
            .iter()
            .map(|(label, value)| {
                format!(
                    "<option {}value=\"{value}\">{}</option>",
                    if value == selected {
                        "selected=\"selected\" "
                    } else {
                        ""
                    },
                    html_escape(label)
                )
            })
            .collect();
        out.push_str(&format!(
            "          <select name=\"digest_after_minutes\" id=\"digest_after_minutes\" class=\"combobox\">{}</select>\n",
            options.join("\n")
        ));
        out.push_str("          </p>\n");
    }
    // server_plugin_outlet "unsubscribe_options": chat's connector, then
    // discourse-policy's, each drawing its frequencies for its own keys.
    let frequency_select = |name: &str,
                            title_key: &str,
                            (frequencies, current): &(Vec<(String, String)>, String)|
     -> Result<String, AppError> {
        let options: Vec<String> = frequencies
            .iter()
            .map(|(label, value)| {
                format!(
                    "<option {}value=\"{}\">{}</option>",
                    if value == current {
                        "selected=\"selected\" "
                    } else {
                        ""
                    },
                    html_escape(value),
                    html_escape(label)
                )
            })
            .collect();
        Ok(format!(
            "  <p>\n    <label>{}</label>\n    <select name=\"{name}\" id=\"{name}\" class=\"combobox\">{}</select>\n  </p>\n",
            html_escape(&translated(i18n, title_key)?),
            options.join("\n")
        ))
    };
    let mut outlet = String::new();
    if let Some(chat) = &o.chat_email_frequencies {
        outlet.push_str(&frequency_select(
            "chat_email_frequency",
            "unsubscribe.chat_summary.select_title",
            chat,
        )?);
    }
    if let Some(policy) = &o.policy_email_frequencies {
        outlet.push_str(&frequency_select(
            "policy_email_frequency",
            "unsubscribe.policy_emails.select_title",
            policy,
        )?);
    }
    out.push_str(&format!("\n      {outlet}\n\n"));
    if !o.unsubscribed_from_all {
        out.push_str("        <p>\n        <label>\n");
        out.push_str(&format!(
            "          {}\n",
            check_box("unsubscribe_all", o.key_type == ALL_TYPE)
        ));
        out.push_str(&format!(
            "          {}\n",
            html_escape(&translated_with(
                i18n,
                "unsubscribe.all",
                &[("sitename", &settings.get("title")?.to_s())]
            )?)
        ));
        out.push_str("        </label>\n        </p>\n");
    }
    out.push_str("\n      <br/>\n");
    let text = if o.key_type == DIGEST_TYPE {
        translated(i18n, "unsubscribe.submit")?
    } else {
        translated(i18n, "unsubscribe.unsubscribe")?
    };
    out.push_str(&format!("      {}\n</form>\n\n\n</div>\n", submit(&text)));
    Ok(out)
}

/// email/unsubscribed.html.erb; `topic` the topic when anyone may see
/// it, `topic_id` whether one was named.
pub fn render_unsubscribed(
    i18n: &I18n,
    settings: &SiteSettings,
    base_path: &str,
    email: &str,
    topic: Option<&KeyTopic>,
    topic_id: bool,
) -> Result<String, AppError> {
    let mut out = String::from("<div class='container unsubscribe'>\n  <br>\n");
    out.push_str(&format!(
        "  <h2>{}</h2>\n  <br>\n  <p>\n",
        html_escape(&translated(i18n, "unsubscribed.title")?)
    ));
    out.push_str(&format!(
        "    {}\n  </p>\n\n",
        translated_with(
            i18n,
            "unsubscribed.description",
            &[
                ("email", email),
                ("url", &format!("{base_path}/my/preferences"))
            ]
        )?
    ));
    if let Some(topic) = topic {
        out.push_str(&format!(
            "    <p>{}</p>\n",
            translated_with(
                i18n,
                "unsubscribed.topic_description",
                &[(
                    "link",
                    &render_topic_title(base_path, settings.emojis(), topic)
                )]
            )?
        ));
    } else if topic_id {
        out.push_str(&format!(
            "    <p>{}</p>\n",
            html_escape(&translated(i18n, "unsubscribed.private_topic_description")?)
        ));
    }
    out.push_str("</div>\n");
    Ok(out)
}

/// A param as Ruby tests it: present and neither nil nor false.
fn given(params: &serde_json::Map<String, serde_json::Value>, name: &str) -> bool {
    !matches!(
        params.get(name),
        None | Some(serde_json::Value::Null) | Some(serde_json::Value::Bool(false))
    )
}

/// The strategy's `unsubscribe(params)`, after One-Click's params are
/// resolved: whether any preference changed. Every write is
/// update_columns or update_all (no callbacks, no updated_at), but chat's
/// update!, whose callbacks leave this column alone (user_options has no
/// timestamps).
pub async fn unsubscribe(
    conn: &mut PgConnection,
    key: &Key,
    owner_id: i32,
    params: &serde_json::Map<String, serde_json::Value>,
    settings: &SiteSettings,
) -> Result<bool, AppError> {
    let strategy = strategy_for(key, settings)?;
    let mut updated = false;
    // BaseEmailUnsubscriber
    if given(params, "disable_mailing_list") {
        sqlx::query("UPDATE user_options SET mailing_list_mode = false WHERE user_id = $1")
            .bind(owner_id)
            .execute(&mut *conn)
            .await?;
        updated = true;
    }
    if given(params, "unsubscribe_all") {
        sqlx::query(
            "UPDATE user_options SET email_digests = false, email_level = 2, \
             email_messages_level = 2, mailing_list_mode = false WHERE user_id = $1",
        )
        .bind(owner_id)
        .execute(&mut *conn)
        .await?;
        updated = true;
    }
    match strategy {
        Strategy::Base => {}
        Strategy::ChatSummary => {
            // `user_option.update!(chat_email_frequency:)`: the enum
            // raises on anything else (a blank one breaks NOT NULL).
            if given(params, "chat_email_frequency") {
                let value = params
                    .get("chat_email_frequency")
                    .and_then(crate::params::scalar)
                    .unwrap_or_default();
                let Some(frequency) = crate::plugins::chat::site_integration::EMAIL_FREQUENCIES
                    .iter()
                    .position(|f| *f == value)
                else {
                    return Err(std::io::Error::other(format!(
                        "ArgumentError: '{value}' is not a valid chat_email_frequency"
                    ))
                    .into());
                };
                sqlx::query("UPDATE user_options SET chat_email_frequency = $2 WHERE user_id = $1")
                    .bind(owner_id)
                    .bind(frequency as i32)
                    .execute(&mut *conn)
                    .await?;
                updated = true;
            }
        }
        Strategy::Policy => {
            if given(params, "policy_email_frequency") {
                let value = params
                    .get("policy_email_frequency")
                    .and_then(crate::params::scalar)
                    .unwrap_or_default();
                // The enum refuses anything else (ArgumentError).
                let Some(frequency) = crate::plugins::policy::EMAIL_FREQUENCIES
                    .iter()
                    .position(|f| *f == value)
                else {
                    return Err(Unsupported("a policy email frequency that is not one").into());
                };
                sqlx::query(
                    "UPDATE user_options SET policy_email_frequency = $2 WHERE user_id = $1",
                )
                .bind(owner_id)
                .bind(frequency as i32)
                .execute(&mut *conn)
                .await?;
                updated = true;
            }
        }
        Strategy::Digest => {
            if given(params, "digest_after_minutes") {
                let minutes = crate::params::integer(params, "digest_after_minutes").unwrap_or(0);
                let minutes = i32::try_from(minutes)
                    .map_err(|_| Unsupported("a digest frequency out of range"))?;
                sqlx::query(
                    "UPDATE user_options SET digest_after_minutes = $2, email_digests = $3 WHERE user_id = $1",
                )
                .bind(owner_id)
                .bind(minutes)
                .bind(minutes > 0)
                .execute(&mut *conn)
                .await?;
                updated = true;
            }
        }
        Strategy::Topic => {
            let Some(topic) = associated_topic(conn, key).await? else {
                return Ok(updated);
            };
            if given(params, "unwatch_topic") {
                sqlx::query(
                    "UPDATE topic_users SET notification_level = 2 WHERE topic_id = $1 AND user_id = $2",
                )
                .bind(topic.id)
                .bind(owner_id)
                .execute(&mut *conn)
                .await?;
                updated = true;
            }
            if given(params, "unwatch_category")
                && let Some(category_id) = topic.category_id
            {
                // joins(:topic): the topics' default scope applies.
                sqlx::query(
                    "UPDATE topic_users tu SET notification_level = 2 FROM topics t \
                     WHERE t.id = tu.topic_id AND t.deleted_at IS NULL AND tu.user_id = $1 \
                       AND tu.notification_level = 3 AND t.category_id = $2",
                )
                .bind(owner_id)
                .bind(category_id)
                .execute(&mut *conn)
                .await?;
                sqlx::query(
                    "DELETE FROM category_users WHERE user_id = $1 AND category_id = $2 \
                     AND notification_level IN (3, 4)",
                )
                .bind(owner_id)
                .bind(category_id)
                .execute(&mut *conn)
                .await?;
                updated = true;
            }
            if given(params, "mute_topic") {
                sqlx::query(
                    "UPDATE topic_users SET notification_level = 0 WHERE topic_id = $1 AND user_id = $2",
                )
                .bind(topic.id)
                .bind(owner_id)
                .execute(&mut *conn)
                .await?;
                updated = true;
            }
        }
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(minutes: Option<i32>, digests: bool) -> Owner {
        Owner {
            id: 1,
            name: None,
            email: None,
            mailing_list_mode: false,
            email_digests: Some(digests),
            digest_after_minutes: minutes,
            email_level: 1,
            email_messages_level: 0,
        }
    }

    #[test]
    fn the_selected_frequency_is_a_step_down() {
        let i18n = I18n::vendored().unwrap();
        let pick = |m, d| {
            let DigestFrequencies {
                current,
                frequencies: list,
                selected,
            } = digest_frequencies(&i18n, &owner(m, d)).unwrap();
            (
                current,
                list.iter().map(|(_, v)| *v).collect::<Vec<_>>(),
                selected,
            )
        };
        assert_eq!(
            pick(Some(10080), true),
            ("weekly".into(), vec![10080, 43200, 259200, 0], 43200)
        );
        assert_eq!(pick(Some(1440), true).0, "daily");
        assert_eq!(pick(Some(1440), true).2, 43200);
        assert_eq!(pick(Some(43200), true).2, 259200);
        assert_eq!(pick(Some(259200), true).2, 0);
        assert_eq!(
            pick(Some(10080), false),
            ("never".into(), vec![10080, 43200, 259200, 0], 0)
        );
    }
}
