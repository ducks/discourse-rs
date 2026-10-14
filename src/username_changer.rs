//! Changing a username: UsernameChanger (app/services/username_changer.rb)
//! with the user's save (validations and after_save callbacks), the staff
//! log, Jobs::UpdateUsername (the old name rewritten in posts, revisions,
//! notifications and small actions, with QuoteRewriter's username half),
//! and Jobs::FixUserUsernamesAndGroupsNamesClash.
//!
//! The rewrites work as Rails' do: raw by regular expression, cooked
//! through an HTML5 fragment parse and serialization (Nokogiri there,
//! html5ever here), and `update_columns`, so posts keep their updated_at
//! and are not rebaked.
//!
//! Refused: non-ASCII usernames (Ruby's grapheme counts and Unicode
//! normalization), watched words (user fields), a category description
//! that a rewritten definition post would change (publish_category), the
//! system user's site-logo avatar, and the `user_updated` automations and
//! workflows the job's events would run.

use std::collections::HashSet;

use fancy_regex::Regex as FancyRegex;
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use regex::Regex;
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::i18n::I18n;
use crate::posting::Ctx;
use crate::pretty_text::cleanup::{
    all_elements, all_elements_within, attr, element_name, has_class, new_text, parse,
    replace_with, set_attr, text, to_html,
};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Jobs::UpdateUsername`
pub const UPDATE_USERNAME: &str = "update_username";
/// `Jobs::FixUserUsernamesAndGroupsNamesClash`, weekly.
pub const FIX_CLASH: &str = "fix_user_usernames_and_groups_names_clash";

/// `UserHistory.actions[:change_username]`
const CHANGE_USERNAME: i32 = 21;
/// `UserAction::MENTION`
const MENTION: i32 = 7;
/// `Discourse::SYSTEM_USER_ID`
const SYSTEM_USER_ID: i32 = -1;

/// What `UsernameChanger#change` did.
#[derive(Debug, PartialEq)]
pub enum Changed {
    Changed,
    /// The same username, byte for byte: false, with no errors.
    Same,
    /// `user.save` failed: the errors' full messages.
    Invalid(Vec<String>),
}

/// The user a change reads.
#[derive(sqlx::FromRow)]
struct User {
    id: i32,
    username: String,
    name: Option<String>,
    admin: bool,
    active: bool,
    uploaded_avatar_id: Option<i32>,
}

async fn load_user(conn: &mut PgConnection, user_id: i32) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, username, name, admin, active, uploaded_avatar_id FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(conn)
    .await
}

/// `User.normalize_username`, refused beyond ASCII.
fn normalize(username: &str) -> Result<String, Unsupported> {
    crate::accounts::normalize_username(username)
}

/// The user's validations as a save with `username` runs them: the
/// format, then uniqueness against users and groups and the password
/// check; the full messages, empty when valid.
async fn validate(
    conn: &mut PgConnection,
    s: &SiteSettings,
    i18n: &I18n,
    user_id: i32,
    username: &str,
) -> Result<Vec<String>, AppError> {
    if !username.is_ascii() {
        return Err(Unsupported("non-ASCII usernames").into());
    }
    // apply_watched_words and the user fields' watched words validation
    // skip a user whose custom fields are clean, as here.
    let has_email: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_emails WHERE user_id = $1 AND \"primary\")",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if !has_email {
        return Err(Unsupported("saving a user without a primary email").into());
    }
    let mut messages = Vec::new();
    if let Some(e) = crate::signup::username_format_error(s, i18n, username)? {
        messages.push(e);
    } else {
        let rows: Vec<(i32, bool)> = sqlx::query_as(
            "(SELECT users.id AS id, true AS is_user FROM users WHERE users.username_lower = $1) \
             UNION ALL \
             (SELECT groups.id, false AS is_user FROM groups WHERE lower(groups.name) = $1)",
        )
        .bind(normalize(username)?)
        .fetch_all(&mut *conn)
        .await?;
        let same_user = rows
            .iter()
            .find(|(_, is_user)| *is_user)
            .is_some_and(|(id, _)| *id == user_id);
        if !rows.is_empty() && !same_user {
            messages.push(i18n.t("user.username.unique").unwrap_or("").to_string());
        }
        for candidate in [username.to_string(), username.to_lowercase()] {
            let check = crate::session::token::confirm_password(&mut *conn, user_id, &candidate)
                .await
                .map_err(|e| match e {
                    crate::session::token::PasswordError::Db(e) => AppError::from(e),
                    crate::session::token::PasswordError::Unsupported(u) => AppError::from(u),
                })?;
            if check.is_some_and(|c| c.matches) {
                messages.push(
                    i18n.t("activerecord.errors.models.user.attributes.username.same_as_password")
                        .unwrap_or("")
                        .to_string(),
                );
                break;
            }
        }
    }
    let label = i18n
        .t("activerecord.attributes.user.username")
        .unwrap_or("Username")
        .to_string();
    Ok(messages
        .into_iter()
        .map(|m| format!("{label} {m}"))
        .collect())
}

/// `user.avatar_template_url`: the template, absolute, without `http:`.
fn avatar_template_url(
    ctx: &Ctx<'_>,
    user_id: i32,
    username: &str,
    uploaded_avatar_id: Option<i32>,
) -> Result<String, AppError> {
    let urls = crate::url::Urls {
        config: ctx.config,
        settings: ctx.settings,
    };
    if user_id == SYSTEM_USER_ID
        && ctx
            .settings
            .get("use_site_small_logo_as_system_avatar")?
            .truthy()
        && ctx.settings.get("logo_small")?.to_i() != 0
    {
        return Err(
            Unsupported("renaming the system user with the site logo as its avatar").into(),
        );
    }
    let template =
        crate::avatar::avatar_template(&urls, user_id, username, uploaded_avatar_id, None)
            .map_err(|e| match e {
                crate::avatar::AvatarError::Unsupported(u) => AppError::from(u),
                other => AppError::from(other),
            })?;
    let absolute = urls.absolute(&template)?;
    Ok(match absolute.get(..5) {
        Some(p) if p.eq_ignore_ascii_case("http:") => absolute[5..].to_string(),
        _ => absolute,
    })
}

/// How the update job runs after a change.
#[derive(Clone, Copy, PartialEq)]
pub enum Update {
    /// `Jobs.enqueue(:update_username, ...)`
    Enqueue,
    /// `Jobs::UpdateUsername.new.execute(...)` on this connection.
    Inline,
    /// `run_update_job: false`: the caller runs it.
    Skip,
}

/// `UsernameChanger.new(user, new_username, actor).change`
pub async fn change(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
    new_username: &str,
    actor: Option<i32>,
    update: Update,
) -> Result<Changed, AppError> {
    let user = load_user(&mut *conn, user_id)
        .await?
        .ok_or(Unsupported("changing the username of a missing user"))?;
    if user.username == new_username {
        return Ok(Changed::Same);
    }
    let errors = validate(&mut *conn, ctx.settings, ctx.i18n, user.id, new_username).await?;
    if !errors.is_empty() {
        return Ok(Changed::Invalid(errors));
    }
    let lower = normalize(new_username)?;
    sqlx::query(
        "UPDATE users SET username = $2, username_lower = $3, updated_at = clock_timestamp() \
         WHERE id = $1",
    )
    .bind(user.id)
    .bind(new_username)
    .bind(&lower)
    .execute(&mut *conn)
    .await?;
    crate::user_updater::after_save(
        &mut *conn,
        ctx.settings,
        user.id,
        user.admin && user.active,
        &lower,
        user.name.as_deref(),
    )
    .await?;
    if let Some(actor) = actor {
        // StaffActionLogger#log_username_change
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, target_user_id, previous_value, \
                                         new_value, admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, FALSE, clock_timestamp(), clock_timestamp())",
        )
        .bind(CHANGE_USERNAME)
        .bind(actor)
        .bind(user.id)
        .bind(&user.username)
        .bind(new_username)
        .execute(&mut *conn)
        .await?;
    }
    if update == Update::Skip {
        return Ok(Changed::Changed);
    }
    let avatar_template = avatar_template_url(ctx, user.id, new_username, user.uploaded_avatar_id)?;
    let args = json!({
        "user_id": user.id,
        "old_username": user.username,
        "new_username": new_username,
        "avatar_template": avatar_template,
    });
    match update {
        Update::Enqueue => crate::jobs::enqueue(&mut *conn, UPDATE_USERNAME, args).await?,
        Update::Inline => update_username(&mut *conn, ctx, &args).await?,
        Update::Skip => {}
    }
    Ok(Changed::Changed)
}

/// `UsernameChanger.override`'s first branch is not used here; this is
/// `Guardian#can_edit_username?`.
pub async fn can_edit_username(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    user_id: i32,
) -> Result<bool, AppError> {
    if s.get("auth_overrides_username")?.truthy() {
        return Ok(false);
    }
    if guardian.is_staff() {
        return Ok(true);
    }
    let period = s.get("username_change_period")?.to_i();
    if period <= 0 || guardian.is_anonymous() {
        return Ok(false);
    }
    if !guardian.is_me(user_id) {
        return Ok(false);
    }
    let recent: bool = sqlx::query_scalar(
        "SELECT created_at > now() - make_interval(days => $2) FROM users WHERE id = $1",
    )
    .bind(user_id)
    .bind(period as i32)
    .fetch_one(conn)
    .await?;
    Ok(recent)
}

/// The `user_updated` and `username_changed` listeners the port doesn't
/// have: automations and workflows.
async fn user_updated_unsupported(conn: &mut PgConnection) -> Result<(), AppError> {
    let automations: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM discourse_automation_automations \
         WHERE enabled AND trigger = 'user_updated')",
    )
    .fetch_one(&mut *conn)
    .await?;
    if automations {
        return Err(Unsupported("automations on user_updated").into());
    }
    let workflows: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM discourse_workflows_workflows \
         WHERE active_version_id IS NOT NULL AND nodes::text LIKE '%trigger:user_updated%')",
    )
    .fetch_one(&mut *conn)
    .await?;
    if workflows {
        return Err(Unsupported("discourse-workflows triggers on user updates").into());
    }
    Ok(())
}

/// The rewriting of one name to another, as Jobs::UpdateUsername sets it
/// up.
struct Rewriter {
    old: String,
    new: String,
    raw_mention: FancyRegex,
    quote_raw: Regex,
    cooked_mention: Regex,
    cooked_path: Regex,
    quote_title: FancyRegex,
    avatar_img: String,
}

/// Ruby's `\s`.
const SPACE: &str = r"[ \t\n\x0B\x0C\r]";
/// Onigmo's `\p{Alnum}` (Alphabetic or a decimal digit) and `\p{M}`.
const ALNUM_M: &str = r"\p{Alphabetic}\p{Nd}\p{M}";

impl Rewriter {
    fn new(old: &str, new: &str, avatar_img: String) -> Result<Rewriter, AppError> {
        let esc = regex::escape(old);
        let compile = |r: &str| Regex::new(r).map_err(|e| std::io::Error::other(e.to_string()));
        let fancy = |r: &str| FancyRegex::new(r).map_err(|e| std::io::Error::other(e.to_string()));
        // `$` is Ruby's end of line.
        let raw_mention = fancy(&format!(
            r"(?im)(?<![{ALNUM_M}`])@{esc}(?:(?![{ALNUM_M}_\-.`])|(?=[-_.](?:{SPACE}|$)))"
        ))?;
        // QuoteRewriter#rewrite_raw_username: the `username:` form first.
        let quote_raw = compile(&format!(
            r#"(?i)(\[quote{SPACE}*={SPACE}*["']?.*username:){esc}(,?[^\]]*\])|(\[quote{SPACE}*={SPACE}*["']?){esc}(,?[^\]]*\])"#
        ))?;
        let cooked_mention = compile(&format!(r"(?i)\A@{esc}\z"))?;
        let encoded = regex::escape(&crate::pretty_text::cleanup::encode_component(old));
        let cooked_path = compile(&format!(r"(?i)\A/u(?:sers)?/{encoded}\z"))?;
        let quote_title = fancy(&format!(r"(?i)(?<={SPACE}){esc}(?=:)"))?;
        Ok(Rewriter {
            old: old.to_string(),
            new: new.to_string(),
            raw_mention,
            quote_raw,
            cooked_mention,
            cooked_path,
            quote_title,
            avatar_img,
        })
    }

    /// `update_raw`
    fn raw(&self, raw: &str) -> Result<String, AppError> {
        let mention = format!("@{}", self.new);
        let mut out = String::with_capacity(raw.len());
        let mut last = 0;
        for m in self.raw_mention.find_iter(raw) {
            let m = m.map_err(|e| std::io::Error::other(e.to_string()))?;
            out.push_str(&raw[last..m.start()]);
            out.push_str(&mention);
            last = m.end();
        }
        out.push_str(&raw[last..]);
        let rewritten = self.quote_raw.replace_all(&out, |c: &regex::Captures| {
            let (pre, post) = match (c.get(1), c.get(2)) {
                (Some(pre), Some(post)) => (pre.as_str(), post.as_str()),
                _ => (&c[3], &c[4]),
            };
            format!("{pre}{}{post}", self.new)
        });
        Ok(rewritten.into_owned())
    }

    /// The `(data-topic, data-post)` of each quote in `cooked`, which
    /// `quotes_correct_user?` looks up.
    fn quoted(cooked: &str) -> Vec<(Option<String>, Option<String>)> {
        let dom = parse(cooked);
        all_elements(&dom)
            .iter()
            .filter(|e| element_name(e) == Some("aside") && has_class(e, "quote"))
            .map(|e| (attr(e, "data-topic"), attr(e, "data-post")))
            .collect()
    }

    /// `update_cooked`: `owned` says whether a quote's post is the user's.
    fn cooked(
        &self,
        cooked: &str,
        owned: &dyn Fn(Option<&str>, Option<&str>) -> bool,
    ) -> Result<String, AppError> {
        let dom = parse(cooked);
        let mention = format!("@{}", self.new);
        let path = format!(
            "/u/{}",
            crate::pretty_text::cleanup::encode_component(&self.new)
        );
        for a in all_elements(&dom) {
            if element_name(&a) != Some("a") || !has_class(&a, "mention") {
                continue;
            }
            let content = text(&a);
            let content = self.cooked_mention.replace_all(&content, mention.as_str());
            set_content(&a, &content);
            if let Some(href) = attr(&a, "href") {
                let href = self.cooked_path.replace_all(&href, path.as_str());
                set_attr(&a, "href", &href);
            }
        }
        self.rewrite_quotes(&dom, owned)?;
        Ok(to_html(&dom))
    }

    /// `QuoteRewriter#rewrite_cooked_username`
    fn rewrite_quotes(
        &self,
        dom: &RcDom,
        owned: &dyn Fn(Option<&str>, Option<&str>) -> bool,
    ) -> Result<(), AppError> {
        let asides: Vec<Handle> = all_elements(dom)
            .into_iter()
            .filter(|e| element_name(e) == Some("aside") && has_class(e, "quote"))
            .collect();
        for aside in asides {
            let Some(div) = all_elements_within(&aside)
                .into_iter()
                .find(|e| element_name(e) == Some("div") && has_class(e, "title"))
            else {
                continue;
            };
            if attr(&aside, "data-username").as_deref() == Some(self.old.as_str()) {
                set_attr(&aside, "data-username", &self.new);
            }
            let mut replaced = false;
            let children: Vec<Handle> = div.children.borrow().clone();
            for child in children {
                if let NodeData::Text { contents } = &child.data {
                    let content = contents.borrow().to_string();
                    let rewritten = self
                        .quote_title
                        .replace_all(&content, self.new.as_str())
                        .into_owned();
                    // `gsub!` returns nil when nothing matched.
                    replaced = self
                        .quote_title
                        .is_match(&content)
                        .map_err(|e| std::io::Error::other(e.to_string()))?;
                    if replaced {
                        *contents.borrow_mut() = rewritten.as_str().into();
                    }
                }
            }
            if replaced
                || owned(
                    attr(&aside, "data-topic").as_deref(),
                    attr(&aside, "data-post").as_deref(),
                )
            {
                let img = all_elements_within(&div)
                    .into_iter()
                    .find(|e| element_name(e) == Some("img") && has_class(e, "avatar"));
                if let Some(img) = img {
                    // `node.replace(html)`: the markup parsed in place.
                    let fragment = parse(&self.avatar_img);
                    let nodes: Vec<Handle> = crate::pretty_text::cleanup::fragment_root(&fragment)
                        .children
                        .borrow()
                        .clone();
                    replace_with(&img, nodes);
                }
            }
        }
        Ok(())
    }
}

/// Nokogiri's `node.content = text`: the children replaced by one text.
fn set_content(node: &Handle, content: &str) {
    let child = new_text(content);
    child.parent.set(Some(std::rc::Rc::downgrade(node)));
    *node.children.borrow_mut() = vec![child];
}

/// Rewrites one cooked string, looking up the quoted posts first.
async fn rewrite_cooked(
    conn: &mut PgConnection,
    rw: &Rewriter,
    user_id: i32,
    cooked: &str,
) -> Result<String, AppError> {
    let mut owned = HashSet::new();
    for (topic, post) in Rewriter::quoted(cooked) {
        // Post.exists?(topic_id:, post_number:, user_id:): nil matches no row.
        let (Some(topic), Some(post)) = (topic, post) else {
            continue;
        };
        let t = crate::ruby::to_i(&topic);
        let p = crate::ruby::to_i(&post);
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM posts WHERE topic_id = $1 AND post_number = $2 \
             AND user_id = $3 AND deleted_at IS NULL)",
        )
        .bind(t)
        .bind(p)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if exists {
            owned.insert((topic, post));
        }
    }
    rw.cooked(cooked, &|t, p| {
        matches!((t, p), (Some(t), Some(p)) if owned.contains(&(t.to_string(), p.to_string())))
    })
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Unsupported("update_username without its names").into())
}

/// `Jobs::UpdateUsername#execute`
pub async fn update_username(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    args: &Value,
) -> Result<(), AppError> {
    let user_id = args
        .get("user_id")
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .ok_or(Unsupported("update_username without a user_id"))? as i32;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Ok(());
    }
    let old = str_arg(args, "old_username")?;
    let new = str_arg(args, "new_username")?;
    if !old.is_ascii() || !new.is_ascii() {
        return Err(Unsupported("renaming non-ASCII usernames in posts").into());
    }
    user_updated_unsupported(&mut *conn).await?;
    let template = args
        .get("avatar_template")
        .and_then(Value::as_str)
        .unwrap_or("");
    let avatar_img = if template.is_empty() {
        String::new()
    } else {
        crate::onebox::avatar_img(ctx.settings, template, "tiny")?
    };
    let rw = Rewriter::new(old, new, avatar_img)?;

    update_posts(&mut *conn, ctx, &rw, user_id).await?;
    update_revisions(&mut *conn, &rw, user_id).await?;

    sqlx::query(
        "UPDATE notifications \
         SET data = (data :: JSONB || \
                     jsonb_strip_nulls( \
                         jsonb_build_object( \
                             'original_username', CASE data :: JSONB ->> 'original_username' \
                                                  WHEN $1 THEN $2 ELSE NULL END, \
                             'display_username', CASE data :: JSONB ->> 'display_username' \
                                                 WHEN $1 THEN $2 ELSE NULL END, \
                             'username', CASE data :: JSONB ->> 'username' \
                                         WHEN $1 THEN $2 ELSE NULL END, \
                             'username2', CASE data :: JSONB ->> 'username2' \
                                          WHEN $1 THEN $2 ELSE NULL END \
                         ) \
                     )) :: JSON \
         WHERE \
           data :: JSONB ->> 'original_username' = $1 OR \
           data :: JSONB ->> 'display_username' = $1 OR \
           data :: JSONB ->> 'username' = $1 OR \
           data :: JSONB ->> 'username2' = $1",
    )
    .bind(old)
    .bind(new)
    .execute(&mut *conn)
    .await?;

    sqlx::query(
        "UPDATE post_custom_fields SET value = $2 \
         WHERE name = 'action_code_who' AND value = $1",
    )
    .bind(old)
    .bind(new)
    .execute(&mut *conn)
    .await?;
    // DiscourseEvent :user_updated: its web hooks.
    crate::web_hooks::enqueue_user_hooks(
        &mut *conn,
        &crate::web_hooks::Payloads {
            settings: ctx.settings,
            i18n: ctx.i18n,
            config: ctx.config,
        },
        "user_updated",
        user_id,
    )
    .await?;
    Ok(())
}

/// `update_posts`: those mentioning the user, the user's own mentioning
/// the old name, those quoting the user, and category definitions
/// mentioning it, each once.
async fn update_posts(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    rw: &Rewriter,
    user_id: i32,
) -> Result<(), AppError> {
    let like = format!("%@{}%", rw.old);
    let mut updated: Vec<i32> = Vec::new();
    let mentioned: Vec<i32> = sqlx::query_scalar(
        "SELECT DISTINCT posts.id FROM posts \
         JOIN user_actions AS a ON (a.target_post_id = posts.id AND a.action_type = $2) \
         WHERE a.user_id = $1 ORDER BY posts.id",
    )
    .bind(user_id)
    .bind(MENTION)
    .fetch_all(&mut *conn)
    .await?;
    for id in mentioned {
        update_post(&mut *conn, ctx, rw, user_id, id).await?;
        updated.push(id);
    }
    let own: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE raw ILIKE $2 AND posts.user_id = $1 \
           AND NOT (id = ANY($3)) ORDER BY id",
    )
    .bind(user_id)
    .bind(&like)
    .bind(&updated)
    .fetch_all(&mut *conn)
    .await?;
    for id in own {
        update_post(&mut *conn, ctx, rw, user_id, id).await?;
        updated.push(id);
    }
    let quoting: Vec<i32> = sqlx::query_scalar(
        "SELECT DISTINCT posts.id FROM posts \
         JOIN quoted_posts AS q ON (q.post_id = posts.id) \
         JOIN posts AS p ON (q.quoted_post_id = p.id) \
         WHERE p.user_id = $1 AND NOT (posts.id = ANY($2)) ORDER BY posts.id",
    )
    .bind(user_id)
    .bind(&updated)
    .fetch_all(&mut *conn)
    .await?;
    for id in quoting {
        update_post(&mut *conn, ctx, rw, user_id, id).await?;
        updated.push(id);
    }
    let definitions: Vec<i32> = sqlx::query_scalar(
        "SELECT DISTINCT posts.id FROM posts \
         INNER JOIN categories ON categories.topic_id = posts.topic_id \
         WHERE posts.post_number = 1 AND raw ILIKE $1 AND NOT (posts.id = ANY($2)) \
         ORDER BY posts.id",
    )
    .bind(&like)
    .bind(&updated)
    .fetch_all(&mut *conn)
    .await?;
    for id in definitions {
        update_post(&mut *conn, ctx, rw, user_id, id).await?;
    }
    Ok(())
}

/// `update_post`: raw and cooked rewritten and written with
/// `update_columns`, the first post's caches synced, the post reindexed.
async fn update_post(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    rw: &Rewriter,
    user_id: i32,
    post_id: i32,
) -> Result<(), AppError> {
    let (raw, cooked, topic_id, post_number): (String, String, i32, i32) =
        sqlx::query_as("SELECT raw, cooked, topic_id, post_number FROM posts WHERE id = $1")
            .bind(post_id)
            .fetch_one(&mut *conn)
            .await?;
    let raw = rw.raw(&raw)?;
    let cooked = rewrite_cooked(&mut *conn, rw, user_id, &cooked).await?;
    sqlx::query("UPDATE posts SET raw = $2, cooked = $3 WHERE id = $1")
        .bind(post_id)
        .bind(&raw)
        .bind(&cooked)
        .execute(&mut *conn)
        .await?;
    // post.topic: live topics only (Trashable's default scope).
    let live_topic: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND deleted_at IS NULL)",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    let s = ctx.settings;
    if post_number == 1 {
        // sync_first_post_caches: topic&.update_excerpt(excerpt_for_topic)
        if live_topic {
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
                .bind(topic_id)
                .bind(excerpt)
                .execute(&mut *conn)
                .await?;
        }
        sync_category_description(&mut *conn, ctx, topic_id, &cooked).await?;
    }
    // SearchIndexer.index(post, force: true) if post.topic, for raw present.
    if live_topic && !crate::ruby::is_blank(&raw) {
        let urls = crate::url::Urls {
            config: ctx.config,
            settings: s,
        };
        let base_path = ctx.config.globals.relative_url_root().to_string();
        let base = crate::posting::search_index::BaseUrls {
            base_path: &base_path,
            base_url_no_prefix: urls.base_url_no_prefix()?,
            store: crate::file_store::FileStore::for_site(urls.config, s)?,
        };
        crate::posting::search_index::reindex_post(&mut *conn, s, &base, post_id).await?;
    }
    Ok(())
}

/// `Post#sync_category_description`: refused when the description would
/// change (publish_category and the site cache are not ported).
async fn sync_category_description(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    cooked: &str,
) -> Result<(), AppError> {
    let category: Option<Option<String>> =
        sqlx::query_scalar("SELECT description FROM categories WHERE topic_id = $1 LIMIT 1")
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(description) = category else {
        return Ok(());
    };
    let Some(html) = first_paragraph_description(cooked) else {
        return Ok(());
    };
    let template = ctx
        .i18n
        .t_with(
            "category.post_template",
            &[(
                "replace_paragraph",
                ctx.i18n.t("category.replace_paragraph").unwrap_or(""),
            )],
        )
        .ok_or(Unsupported("category.post_template"))?;
    let prefix: String = template.chars().take(51).collect();
    let new_description = (!html.starts_with(&prefix)).then_some(html);
    if description == new_description {
        return Ok(());
    }
    Err(Unsupported("changing a category description after a username change").into())
}

/// `Category.first_paragraph_description(cooked)`
pub(crate) fn first_paragraph_description(cooked: &str) -> Option<String> {
    let dom = parse(cooked);
    for img in all_elements(&dom)
        .into_iter()
        .filter(|e| element_name(e) == Some("img"))
    {
        replace_with(&img, Vec::new());
    }
    let p = all_elements(&dom)
        .into_iter()
        .find(|e| element_name(e) == Some("p"))?;
    let mut html = String::new();
    for child in p.children.borrow().iter() {
        html.push_str(&crate::pretty_text::cleanup::node_html(child));
    }
    Some(html.trim().to_string())
}

/// `update_revisions`: those whose changes mention the old name, then
/// those of posts quoting the user.
async fn update_revisions(
    conn: &mut PgConnection,
    rw: &Rewriter,
    user_id: i32,
) -> Result<(), AppError> {
    let similar: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM post_revisions WHERE modifications SIMILAR TO $1 ORDER BY id",
    )
    .bind(format!("%(raw|cooked)%@{}%", rw.old))
    .fetch_all(&mut *conn)
    .await?;
    for id in similar {
        update_revision(&mut *conn, rw, user_id, id).await?;
    }
    let quoting: Vec<i32> = sqlx::query_scalar(
        "SELECT DISTINCT post_revisions.id FROM post_revisions \
         JOIN quoted_posts AS q ON (q.post_id = post_revisions.post_id) \
         JOIN posts AS p ON (q.quoted_post_id = p.id) \
         WHERE p.user_id = $1 ORDER BY post_revisions.id",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    for id in quoting {
        update_revision(&mut *conn, rw, user_id, id).await?;
    }
    Ok(())
}

/// `update_revision`: the raw and cooked values rewritten, saved when
/// they changed. A value that is not text raises in Rails, which skips
/// the revision.
async fn update_revision(
    conn: &mut PgConnection,
    rw: &Rewriter,
    user_id: i32,
    revision_id: i32,
) -> Result<(), AppError> {
    let yaml: Option<String> =
        sqlx::query_scalar("SELECT modifications FROM post_revisions WHERE id = $1")
            .bind(revision_id)
            .fetch_one(&mut *conn)
            .await?;
    let Some(yaml) = yaml else {
        return Ok(());
    };
    let fields = crate::modifications::load(&yaml)?;
    if !fields.iter().any(|(k, _)| k == "raw" || k == "cooked") {
        return Ok(());
    }
    let mut out: Vec<(String, [Value; 2])> = Vec::with_capacity(fields.len());
    for (key, values) in &fields {
        let mut values = values.clone();
        if key == "raw" || key == "cooked" {
            for v in values.iter_mut() {
                let Some(s) = v.as_str() else {
                    // NoMethodError on nil, rescued.
                    return Ok(());
                };
                *v = Value::String(if key == "raw" {
                    rw.raw(s)?
                } else {
                    rewrite_cooked(&mut *conn, rw, user_id, s).await?
                });
            }
        }
        out.push((key.clone(), values));
    }
    if out == fields {
        return Ok(());
    }
    let refs: Vec<(&str, [Value; 2])> = out.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let dumped = crate::modifications::dump(&refs)?;
    sqlx::query(
        "UPDATE post_revisions SET modifications = $2, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(revision_id)
    .bind(dumped)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The job context from the state.
pub async fn run_update_username(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    update_username(&mut tx, &ctx, args).await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::FixUserUsernamesAndGroupsNamesClash`: each user whose name a
/// group has gets the first `<name><n>` that saves, changed without an
/// actor and updated inline.
pub async fn fix_clash(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let clashing: Vec<(i32, String)> = sqlx::query_as(
        "SELECT users.id, users.username FROM users \
         LEFT JOIN groups ON lower(groups.name) = users.username_lower \
         WHERE groups.id IS NOT NULL ORDER BY users.id",
    )
    .fetch_all(&mut *conn)
    .await?;
    drop(conn);
    for (user_id, username) in clashing {
        let mut tx = state.pool.begin().await?;
        let s =
            SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
        let host = crate::pretty_text::Host::from_state(state);
        let ctx = Ctx {
            host: &host,
            settings: &s,
            config: &state.config,
            i18n: &state.i18n,
            bus: &state.bus,
        };
        let mut suffix = 1;
        let candidate = loop {
            let candidate = format!("{username}{suffix}");
            suffix += 1;
            if validate(&mut tx, &s, &state.i18n, user_id, &candidate)
                .await?
                .is_empty()
            {
                break candidate;
            }
            if suffix > 1000 {
                return Err(Unsupported("a clashing username with no free suffix").into());
            }
        };
        change(&mut tx, &ctx, user_id, &candidate, None, Update::Inline).await?;
        tx.commit().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rw() -> Rewriter {
        Rewriter::new(
            "foo",
            "bar",
            "<img alt='' width='24' height='24' src='//x/48.png' class='avatar'>".into(),
        )
        .unwrap()
    }

    #[test]
    fn rewrites_mentions_in_raw_as_rails_does() {
        let r = rw();
        for (raw, want) in [
            (
                "There's no difference between @foo and @Foo",
                "There's no difference between @bar and @bar",
            ),
            (
                ".@foo -@foo %@foo _@foo ,@foo ;@foo @@foo",
                ".@bar -@bar %@bar _@bar ,@bar ;@bar @@bar",
            ),
            (r#""@foo" '@foo'"#, r#""@bar" '@bar'"#),
            (
                "**@foo** *@foo* _@foo_ ~~@foo~~",
                "**@bar** *@bar* _@bar_ ~~@bar~~",
            ),
            (
                "@foo. @foo, @foo: @foo; @foo_ @foo-",
                "@bar. @bar, @bar: @bar; @bar_ @bar-",
            ),
            ("@foo a@foo 2@foo", "@bar a@foo 2@foo"),
            ("@foo mail@foo.com", "@bar mail@foo.com"),
            (
                "@foo @foobar @foo-bar @foo_bar @foo1",
                "@bar @foobar @foo-bar @foo_bar @foo1",
            ),
            ("@foo.\nnext", "@bar.\nnext"),
        ] {
            assert_eq!(r.raw(raw).unwrap(), want, "{raw}");
        }
    }

    #[test]
    fn rewrites_quotes_in_raw_as_rails_does() {
        let r = rw();
        let raw = "[quote=\"foo, post:1, topic:5\"]\nq\n[/quote]\n\n[quote='foo']\nq\n[/quote]\n\n\
                   [quote=foo, post:1, topic:5]\nq\n[/quote]\n\n\
                   [quote=\"Foo Bar, post:1, topic:5, username:foo\"]\nq\n[/quote]";
        let want = "[quote=\"bar, post:1, topic:5\"]\nq\n[/quote]\n\n[quote='bar']\nq\n[/quote]\n\n\
                    [quote=bar, post:1, topic:5]\nq\n[/quote]\n\n\
                    [quote=\"Foo Bar, post:1, topic:5, username:bar\"]\nq\n[/quote]";
        assert_eq!(r.raw(raw).unwrap(), want);
    }

    #[test]
    fn rewrites_mentions_and_quotes_in_cooked() {
        let r = rw();
        let cooked = r#"<p>Hello <a class="mention" href="/users/foo">@foo</a> <a class="mention" href="/u/other">@other</a></p>
<aside class="quote no-group" data-username="foo" data-post="1" data-topic="5">
<div class="title">
<div class="quote-controls"></div>
<img loading="lazy" alt="" width="24" height="24" src="//old/48.png" class="avatar"> foo:</div>
<blockquote>
<p>q</p>
</blockquote>
</aside>"#;
        let want = r#"<p>Hello <a class="mention" href="/u/bar">@bar</a> <a class="mention" href="/u/other">@other</a></p>
<aside class="quote no-group" data-username="bar" data-post="1" data-topic="5">
<div class="title">
<div class="quote-controls"></div>
<img alt="" width="24" height="24" src="//x/48.png" class="avatar"> bar:</div>
<blockquote>
<p>q</p>
</blockquote>
</aside>"#;
        assert_eq!(r.cooked(cooked, &|_, _| false).unwrap(), want);
    }

    #[test]
    fn replaces_the_avatar_of_the_users_quoted_posts_only() {
        let r = rw();
        let cooked = r#"<aside class="quote" data-username="Foo Bar" data-post="2" data-topic="5"><div class="title"><img src="//old" class="avatar"> Foo Bar:</div></aside>"#;
        let mine = r
            .cooked(cooked, &|t, p| t == Some("5") && p == Some("2"))
            .unwrap();
        assert!(mine.contains(r#"src="//x/48.png""#), "{mine}");
        let theirs = r.cooked(cooked, &|_, _| false).unwrap();
        assert!(theirs.contains(r#"src="//old""#), "{theirs}");
    }
}
