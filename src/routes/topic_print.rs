//! A topic's print view (`t/:slug/:topic_id/print`, or `?print=true`):
//! topics/show.html.erb in the crawler layout (layouts/crawler.html.erb),
//! every post of the print chunk drawn as a crawler post, then
//! js/print-page, which opens the browser's print dialog. The body from
//! #main-outlet through the noscript footer follows the ERB's output to
//! the whitespace (parity/writes/topic_print).
//!
//! Solved's QAPage microdata (DiscourseSolved::SchemaUtils) is drawn; its
//! JSON-LD in the head (BuildSchemaMarkup) is not.
//!
//! Refused: discourse-lazy-videos' and discourse-post-voting's crawler
//! post ends, a hidden first post's excerpt, secure post images and a site
//! locale other than English (`l(created_at, format: :long)`).

use axum::http::{HeaderMap, Uri};
use axum::response::Response;
use chrono::NaiveDateTime;
use serde_json::Value;
use sqlx::PgConnection;

use crate::email::notification::html_escape as h;
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// A post as the crawler view reads it.
#[derive(sqlx::FromRow)]
struct CrawlerPost {
    id: i32,
    post_number: i32,
    post_type: i32,
    cooked: String,
    hidden: bool,
    like_count: i32,
    action_code: Option<String>,
    version: i32,
    last_version_at: NaiveDateTime,
    created_at: NaiveDateTime,
    image_upload_id: Option<i64>,
    username: Option<String>,
    name: Option<String>,
    action_code_who: Option<String>,
    action_code_path: Option<String>,
}

/// `@breadcrumbs`: categories_breadcrumb's entries.
struct Crumb {
    url: String,
    name: String,
    color: String,
}

/// `Post.types[:regular]` and `[:small_action]`
const REGULAR: i32 = 1;
const SMALL_ACTION: i32 = 3;

/// DiscourseSolved::SchemaUtils.qa_page_schema? holds: the topic is a
/// QAPage, its accepted answers these posts.
struct Qa {
    /// `accepted_answer_visible?`
    accepted_visible: bool,
    answer_post_ids: Vec<i32>,
}

/// `SchemaUtils.eligible_answer?`: a visible regular reply with text.
fn eligible_answer(post: &CrawlerPost) -> bool {
    post.post_number != 1
        && post.post_type == REGULAR
        && !post.hidden
        && !post.cooked.is_empty()
        && !crate::excerpt::fragment_text(&post.cooked)
            .trim()
            .is_empty()
}

/// `crawler_post_schema_hash`, as the attributes it draws.
struct PostSchema {
    attributes: &'static str,
    itemscope: bool,
    /// `crawler_post_schema_overridden?`
    overridden: bool,
    /// solved's post_answer_meta: an accepted or suggested Answer.
    answer: bool,
}

fn post_schema(post: &CrawlerPost, qa: Option<&Qa>) -> PostSchema {
    let first = post.post_number == 1;
    let none = PostSchema {
        attributes: "",
        itemscope: false,
        overridden: false,
        answer: false,
    };
    let Some(qa) = qa else {
        if first {
            return none;
        }
        return PostSchema {
            attributes: "itemprop=\"comment\" itemscope=\"itemscope\" itemtype=\"http://schema.org/Comment\"",
            itemscope: true,
            ..none
        };
    };
    if first {
        return PostSchema {
            attributes: "data-qa-question=\"true\"",
            overridden: true,
            ..none
        };
    }
    if !eligible_answer(post) {
        return none;
    }
    let accepted = qa.accepted_visible && qa.answer_post_ids.contains(&post.id);
    PostSchema {
        attributes: if accepted {
            "itemprop=\"acceptedAnswer\" itemscope=\"itemscope\" itemtype=\"https://schema.org/Answer\""
        } else {
            "itemprop=\"suggestedAnswer\" itemscope=\"itemscope\" itemtype=\"https://schema.org/Answer\""
        },
        itemscope: true,
        overridden: true,
        answer: true,
    }
}

/// `SchemaUtils.qa_page_schema?(topic)`: solved's markup on (always, or
/// answered only with a visible answer), accepted answers allowed, and a
/// reply that could be one.
async fn qa_page(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
    category_id: Option<i32>,
    archetype: &str,
) -> Result<Option<Qa>, AppError> {
    if !settings.get("solved_enabled")?.truthy() {
        return Ok(None);
    }
    let markup = settings.get("solved_add_schema_markup")?.to_s().to_string();
    if markup == "never" {
        return Ok(None);
    }
    let (user_id, closed, archived, deleted): (Option<i32>, bool, bool, bool) = sqlx::query_as(
        "SELECT user_id, closed, archived, deleted_at IS NOT NULL FROM topics WHERE id = $1",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    let facts = crate::plugins::solved::TopicFacts {
        id: topic_id,
        user_id,
        category_id,
        archetype: archetype.to_string(),
        closed,
        archived,
        deleted,
    };
    // Guardian.new.allow_accepted_answers?
    if !crate::plugins::solved::allow_accepted_answers(&mut *conn, settings, &facts, None).await? {
        return Ok(None);
    }
    let answer_post_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT ta.answer_post_id::int FROM discourse_solved_solved_topics st \
         JOIN discourse_solved_topic_answers ta ON ta.solved_topic_id = st.id \
         WHERE st.topic_id = $1 ORDER BY ta.id",
    )
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?;
    // topic.solved.answer_posts, any an anonymous guardian may see.
    let accepted_visible: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM posts WHERE id = ANY($1) AND deleted_at IS NULL AND post_type <> 4)",
    )
    .bind(&answer_post_ids)
    .fetch_one(&mut *conn)
    .await?;
    if markup == "answered only" && !accepted_visible {
        return Ok(None);
    }
    // eligible_answers(topic).any? { eligible_answer? }
    let replies: Vec<String> = sqlx::query_scalar(
        "SELECT cooked FROM posts WHERE topic_id = $1 AND deleted_at IS NULL AND post_number <> 1 \
         AND post_type = 1 AND NOT hidden",
    )
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?;
    let any = replies
        .iter()
        .any(|c| !c.is_empty() && !crate::excerpt::fragment_text(c).trim().is_empty());
    Ok(any.then_some(Qa {
        accepted_visible,
        answer_post_ids,
    }))
}

/// `to_formatted_s(:iso8601)` of a UTC time.
fn iso8601(t: &NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// What the page is drawn from besides the request.
pub(super) struct Print<'a> {
    pub state: &'a AppState,
    pub settings: &'a SiteSettings,
    pub guardian: &'a Guardian,
    pub headers: &'a HeaderMap,
    pub uri: &'a Uri,
    /// The rendered TopicView (print chunk).
    pub view: &'a Value,
    /// `@page`
    pub page: i64,
    pub page_image: crate::topic_view::PageImage,
}

pub(super) async fn render(conn: &mut PgConnection, p: Print<'_>) -> Result<Response, AppError> {
    let Print {
        state,
        settings,
        guardian,
        headers,
        uri,
        view,
        page,
        page_image,
    } = p;
    if settings.get("default_locale")?.to_s() != "en" {
        return Err(Unsupported("the print view in a locale other than English").into());
    }
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let base_path = state.config.globals.relative_url_root().to_string();
    let base_url = urls.base_url()?;
    let topic_id = view["id"].as_i64().unwrap_or(0) as i32;

    let topic: (
        String,
        String,
        Option<i32>,
        NaiveDateTime,
        String,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT t.title, t.slug, t.category_id, t.created_at, t.archetype, t.subtype, u.username \
             FROM topics t LEFT JOIN users u ON u.id = t.user_id WHERE t.id = $1",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    let (title, slug, category_id, topic_created_at, archetype, subtype, author) = topic;
    let relative_url = format!("{base_path}/t/{slug}/{topic_id}");
    let topic_url = format!("{}{relative_url}", urls.base_url_no_prefix()?);

    // The plugins' crawler hooks this port does not draw.
    if crate::plugins::post_voting::is_post_voting(
        &mut *conn,
        settings,
        topic_id,
        &archetype,
        subtype.as_deref(),
    )
    .await?
    {
        return Err(Unsupported("the print view of a post voting topic").into());
    }
    let qa = qa_page(&mut *conn, settings, topic_id, category_id, &archetype).await?;

    // The view's posts, in the stream's order.
    let ids: Vec<i32> = view["post_stream"]["posts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["id"].as_i64().map(|id| id as i32))
        .collect();
    let rows: Vec<CrawlerPost> = sqlx::query_as(
        "SELECT p.id, p.post_number, p.post_type, p.cooked, p.hidden, p.like_count, p.action_code, \
         p.version, p.last_version_at, p.created_at, p.image_upload_id::bigint AS image_upload_id, \
         u.username, u.name, \
         (SELECT value FROM post_custom_fields f WHERE f.post_id = p.id AND f.name = 'action_code_who' ORDER BY f.id LIMIT 1) AS action_code_who, \
         (SELECT value FROM post_custom_fields f WHERE f.post_id = p.id AND f.name = 'action_code_path' ORDER BY f.id LIMIT 1) AS action_code_path \
         FROM posts p LEFT JOIN users u ON u.id = p.user_id WHERE p.id = ANY($1)",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut posts: Vec<CrawlerPost> = Vec::with_capacity(rows.len());
    let mut rows = rows;
    for id in &ids {
        if let Some(i) = rows.iter().position(|r| r.id == *id) {
            posts.push(rows.swap_remove(i));
        }
    }
    if settings.get("lazy_videos_enabled")?.truthy()
        && posts
            .iter()
            .any(|p| p.cooked.contains("lazy-video-container"))
    {
        return Err(Unsupported("the print view's lazy video schemas").into());
    }
    let link_counts = |post_id: i32| -> Vec<(String, String)> {
        view["post_stream"]["posts"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["id"].as_i64() == Some(i64::from(post_id)))
            .and_then(|p| p["link_counts"].as_array())
            .into_iter()
            .flatten()
            .filter(|l| l["reflection"] == true)
            .filter_map(|l| {
                let title = l["title"].as_str().filter(|t| !t.is_empty())?;
                Some((
                    l["url"].as_str().unwrap_or("").to_string(),
                    title.to_string(),
                ))
            })
            .collect()
    };

    // categories_breadcrumb
    let mut crumbs = Vec::new();
    let uncategorized = settings.get("uncategorized_category_id")?.to_i();
    let mut category_name = None;
    if let Some(id) = category_id.filter(|id| i64::from(*id) != uncategorized)
        && let Some(category) = crate::categories::model::Category::find(&mut *conn, id).await?
    {
        category_name = Some(category.name.clone());
        let mut current = Some(category);
        for _ in 0..4 {
            let Some(c) = current.take() else { break };
            crumbs.insert(
                0,
                Crumb {
                    url: c.url(&mut *conn, &base_path).await?,
                    name: c.name.clone(),
                    color: c.color.clone(),
                },
            );
            current = match c.parent_category_id {
                Some(parent) => {
                    crate::categories::model::Category::find(&mut *conn, parent).await?
                }
                None => None,
            };
        }
    } else if let Some(id) = category_id
        && let Some(category) = crate::categories::model::Category::find(&mut *conn, id).await?
    {
        category_name = Some(category.name);
    }

    // `visible_tags`: guardian.can_see_tags?(topic), topic.tags.visible.
    let tags: Vec<String> = if view.get("tags").is_some() {
        let sql = format!(
            "SELECT t.name FROM topic_tags tt JOIN tags t ON t.id = tt.tag_id \
             WHERE tt.topic_id = $1 AND t.id IN {} ORDER BY tt.tag_id",
            crate::tags::visible_tag_ids_subquery(guardian, settings)?
        );
        sqlx::query_scalar(&sql)
            .bind(topic_id)
            .fetch_all(&mut *conn)
            .await?
    } else {
        Vec::new()
    };

    let emojis = settings.emojis();
    let t = |key: &str| state.i18n.t(key).unwrap_or(key).to_string();

    // The view: topics/show.html.erb, as Erubi leaves it (a line holding
    // only a statement tag goes entirely).
    let mut out = String::new();
    out.push_str("  <div id=\"topic-title\">\n    <h1>\n");
    out.push_str(&format!(
        "      <a href=\"{}\">{}</a>\n",
        h(&relative_url),
        h(&emojis.gsub_to_unicode(&title))
    ));
    out.push_str("    </h1>\n\n");
    if !crumbs.is_empty() {
        out.push_str("      <div class=\"topic-category\" itemscope itemtype=\"http://schema.org/BreadcrumbList\">\n");
        for (i, c) in crumbs.iter().enumerate() {
            out.push_str(&format!(
                "          <span itemprop=\"itemListElement\" itemscope itemtype=\"http://schema.org/ListItem\">\n\
                 \x20           <a href=\"{}\" class=\"badge-wrapper bullet\" itemprop=\"item\">\n\
                 \x20             <span class='badge-category-bg' style='background-color: #{}'></span>\n\
                 \x20             <span class='badge-category clear-badge'>\n\
                 \x20               <span class='category-name' itemprop='name'>{}</span>\n\
                 \x20             </span>\n\
                 \x20           </a>\n\
                 \x20           <meta itemprop=\"position\" content=\"{}\" />\n\
                 \x20         </span>\n",
                h(&c.url),
                h(&c.color),
                h(&c.name),
                i + 1
            ));
        }
        out.push_str(&format!(
            "        <span itemprop=\"itemListElement\" itemscope itemtype=\"http://schema.org/ListItem\">\n\
             \x20         <meta itemprop=\"name\" content=\"{}\" />\n\
             \x20         <link itemprop=\"item\" href=\"{}\" />\n\
             \x20         <meta itemprop=\"position\" content=\"{}\" />\n\
             \x20       </span>\n\
             \x20     </div>\n",
            h(&emojis.gsub_to_unicode(&title)),
            h(&topic_url),
            crumbs.len() + 1
        ));
    }
    out.push('\n');
    if !tags.is_empty() {
        out.push_str("      <div class=\"topic-category\">\n        <div class='discourse-tags list-tags'>\n");
        for (i, tag) in tags.iter().enumerate() {
            out.push_str(&format!(
                "            <a href='{}' class='discourse-tag' rel=\"tag\">{}</a>{}",
                h(&format!("{base_url}/tag/{tag}")),
                h(tag),
                if i + 1 < tags.len() { ", " } else { "" }
            ));
            // `<%= tag.name -%>`'s line keeps its newline after the
            // trailing `<% end %>`s.
            out.push('\n');
        }
        out.push_str("        </div>\n      </div>\n");
    }
    out.push_str("  </div>\n\n  \n\n");

    // include_crawler_content?: the crawler layout shows it, but not to an
    // anonymous visitor of a login-required site.
    let crawler_content = !(guardian.is_anonymous() && settings.get("login_required")?.truthy());
    if crawler_content {
        // crawler_topic_container_schema: solved's QAPage when it applies.
        out.push_str(if qa.is_some() {
            "    <div itemscope=\"itemscope\" itemtype=\"https://schema.org/QAPage\">\n"
        } else {
            "    <div itemscope=\"itemscope\" itemtype=\"http://schema.org/DiscussionForumPosting\">\n"
        });
        out.push_str(&format!(
            "      <meta itemprop='{}' content='{}'>\n\
             \x20     <link itemprop='url' href='{}'>\n\
             \x20     <meta itemprop='datePublished' content='{}'>\n\
             \x20     <meta itemprop='keywords' content='{}'>\n",
            if qa.is_some() { "name" } else { "headline" },
            h(&title),
            h(&format!("{topic_url}/print")),
            iso8601(&topic_created_at),
            h(&tags.join(", "))
        ));
        // `unless main_entity_schema`: the article's section and publisher.
        if qa.is_none() {
            if let Some(name) = &category_name {
                out.push_str(&format!(
                    "          <meta itemprop='articleSection' content='{}'>\n",
                    h(name)
                ));
            }
            let company = settings.get("company_name")?.to_s().trim().to_string();
            let publisher = if company.is_empty() {
                settings.get("title")?.to_s().to_string()
            } else {
                settings.get("company_name")?.to_s().to_string()
            };
            out.push_str(&format!(
                "        <div itemprop='publisher' itemscope itemtype=\"http://schema.org/Organization\">\n\
                 \x20         <meta itemprop='name' content='{}'>\n",
                h(&publisher)
            ));
            // application_logo_url: the mobile logo on a mobile device.
            let logo_name = if crate::mobile::mobile_view() {
                "mobile_logo"
            } else {
                "logo"
            };
            let logo = crate::site_icons::site_url(&mut *conn, &urls, logo_name).await?;
            if !logo.is_empty() {
                out.push_str(&format!(
                    "            <div itemprop='logo' itemscope itemtype=\"http://schema.org/ImageObject\">\n\
                     \x20             <meta itemprop='url' content='{}'>\n\
                     \x20           </div>\n",
                    h(&logo)
                ));
            }
            out.push_str("        </div>\n");
        }
        // The before-posts builders (none), then solved's Question with its
        // main_entity_meta.
        out.push_str("\n      \n\n");
        if qa.is_some() {
            let answers = posts.iter().filter(|p| eligible_answer(p)).count();
            let first_likes: Option<i32> = sqlx::query_scalar(
                "SELECT like_count FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL",
            )
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?;
            out.push_str(&format!(
                "        <div itemprop=\"mainEntity\" itemscope=\"itemscope\" itemtype=\"https://schema.org/Question\">\n\
                 \x20         <meta itemprop='answerCount' content='{answers}'><meta itemprop='datePublished' content='{}'><meta itemprop='name' content='{}'><meta itemprop='upvoteCount' content='{}'>\n",
                iso8601(&topic_created_at),
                h(&title),
                first_likes.unwrap_or(0)
            ));
        }
        out.push('\n');

        // A later page starts past the first post: the topic's author and
        // excerpt stand in for it.
        if posts.first().is_some_and(|p| p.post_number != 1) {
            if let Some(author) = &author {
                out.push_str(&format!(
                    "          <span itemprop='author' itemscope itemtype=\"http://schema.org/Person\">\n\
                     \x20           <meta itemprop='name' content='{0}'>\n\
                     \x20           <link itemprop='url' rel='nofollow' href='{1}/u/{0}'>\n\
                     \x20         </span>\n",
                    h(author),
                    h(&base_url)
                ));
            }
            let first_hidden: Option<bool> = sqlx::query_scalar(
                "SELECT hidden FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL",
            )
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?;
            if first_hidden == Some(true) {
                return Err(Unsupported("the print view's excerpt of a hidden first post").into());
            }
            let stored: Option<String> =
                sqlx::query_scalar("SELECT excerpt FROM topics WHERE id = $1")
                    .bind(topic_id)
                    .fetch_one(&mut *conn)
                    .await?;
            let excerpt = stored
                .map(|e| {
                    html_escape::decode_html_entities(&super::topics::strip_tags(&e)).into_owned()
                })
                .unwrap_or_default();
            out.push_str(&format!(
                "        <meta itemprop='text' content='{}'>\n",
                h(&excerpt)
            ));
        }
        out.push('\n');

        let display_names = settings.get("display_name_on_posts")?.truthy()
            && settings.get("enable_names")?.truthy();
        for post in &posts {
            let Some(username) = &post.username else {
                continue;
            };
            // crawler_post_schema_skip?: solved leaves a QAPage's small
            // actions out.
            if qa.is_some() && post.post_type == SMALL_ACTION {
                continue;
            }
            if post.hidden || post.cooked.trim().is_empty() {
                continue;
            }
            let first = post.post_number == 1;
            let schema = post_schema(post, qa.as_ref());
            // crawler_post_emits_microdata? and _schema_overridden?
            let emit_md = first || schema.itemscope;
            let attr = |a: &'static str| if emit_md { a } else { "" };
            out.push_str(&format!(
                "          <div id='post_{}' {} class='topic-body crawler-post'>\n\
                 \x20           <div class='crawler-post-meta'>\n\
                 \x20             <span class=\"creator\" {}>\n\
                 \x20               <a rel='nofollow' href='{3}/u/{4}'><span {5}>{4}</span></a>\n",
                post.post_number,
                schema.attributes,
                attr(
                    "itemprop=\"author\" itemscope=\"itemscope\" itemtype=\"http://schema.org/Person\""
                ),
                h(&base_url),
                h(username),
                attr("itemprop=\"name\"")
            ));
            if emit_md {
                out.push_str(&format!(
                    "                  <meta itemprop='url' content='{}/u/{}'>\n",
                    h(&base_url),
                    h(username)
                ));
            }
            // `"(#{u.name})" if ...`
            let name = post.name.as_deref().filter(|n| !n.trim().is_empty());
            out.push_str("                ");
            if display_names && let Some(name) = name {
                out.push_str(&h(&format!("({name})")));
            }
            out.push('\n');
            if let Some(code) = &post.action_code {
                let who = h(post.action_code_who.as_deref().unwrap_or(""));
                let href = post.action_code_path.as_deref().unwrap_or("");
                let text = state
                    .i18n
                    .t_with(
                        &format!("js.action_codes.{code}"),
                        &[("when", ""), ("who", &who), ("href", href)],
                    )
                    .ok_or(Unsupported("an action code without its translation"))?;
                out.push_str(&format!("                  {text}\n"));
            }
            out.push_str("              </span>\n\n");
            if first && qa.is_none() {
                out.push_str(&format!(
                    "                <link itemprop=\"mainEntityOfPage\" href=\"{}\">\n",
                    h(&topic_url)
                ));
            }
            out.push('\n');
            if let Some(upload_id) = post.image_upload_id.filter(|_| emit_md) {
                let upload: Option<(String, bool)> =
                    sqlx::query_as("SELECT url, secure FROM uploads WHERE id = $1")
                        .bind(upload_id)
                        .fetch_optional(&mut *conn)
                        .await?;
                if let Some((url, secure)) = upload {
                    if secure && settings.get("secure_uploads")?.truthy() {
                        return Err(Unsupported("a secure post image in the print view").into());
                    }
                    let url = crate::topics::list::cook_topic_image(&urls, &url)?;
                    out.push_str(&format!(
                        "                <link itemprop=\"image\" href=\"{}\">\n",
                        h(&url)
                    ));
                }
            }
            let modified = if post.version > 1 {
                post.last_version_at
            } else {
                post.created_at
            };
            out.push_str(&format!(
                "\n              <span class=\"crawler-post-infos\">\n\
                 \x20                 <time {} datetime='{}' class='post-time'>\n\
                 \x20                   {}\n\
                 \x20                 </time>\n",
                if emit_md && !first {
                    "itemprop=\"datePublished\""
                } else {
                    ""
                },
                iso8601(&post.created_at),
                post.created_at.format("%B %-d, %Y, %l:%M%P"),
            ));
            if emit_md {
                out.push_str(&format!(
                    "                  <meta itemprop='dateModified' content='{}'>\n",
                    iso8601(&modified)
                ));
            }
            out.push_str(&format!(
                "              <span {}>{}</span>\n\
                 \x20             </span>\n\
                 \x20           </div>\n\
                 \x20           <div class='post' {}>\n\
                 \x20             {}\n\
                 \x20           </div>\n\n",
                attr("itemprop=\"position\""),
                post.post_number,
                attr("itemprop=\"text\""),
                post.cooked
            ));
            if !schema.overridden {
                let likes = if post.like_count > 0 {
                    state
                        .i18n
                        .t_count("post.has_likes", i64::from(post.like_count), &[])
                        .ok_or(Unsupported("post.has_likes without its translation"))?
                } else {
                    String::new()
                };
                out.push_str(&format!(
                    "              <div {}>\n",
                    attr("itemprop=\"interactionStatistic\" itemscope=\"itemscope\" itemtype=\"http://schema.org/InteractionCounter\"")
                ));
                if emit_md {
                    out.push_str(&format!(
                        "                  <meta itemprop=\"interactionType\" content=\"http://schema.org/LikeAction\"/>\n\
                         \x20                 <meta itemprop=\"userInteractionCount\" content=\"{}\" />\n",
                        post.like_count
                    ));
                }
                out.push_str(&format!(
                    "                <span class='post-likes'>{}</span>\n\
                     \x20             </div>\n",
                    h(&likes)
                ));
            }
            out.push('\n');
            let linkbacks = link_counts(post.id);
            if !linkbacks.is_empty() {
                out.push_str("                <div class='crawler-linkback-list'>\n");
                for (url, title) in &linkbacks {
                    out.push_str(&format!(
                        "                    <div>\n\
                         \x20                     <a href=\"{}\">{}</a>\n\
                         \x20                   </div>\n",
                        h(url),
                        h(title)
                    ));
                }
                out.push_str("                </div>\n");
            }
            // build_plugin_html "server:topic-show-crawler-post-end": the
            // bundled plugins' three builders joined by newlines, solved's
            // (its answer metas) last, the other two empty here.
            let answer_meta = if schema.answer {
                format!(
                    "<meta itemprop='upvoteCount' content='{}'><meta itemprop='url' content='{}/{}'>",
                    post.like_count, topic_url, post.post_number
                )
            } else {
                String::new()
            };
            out.push_str(&format!(
                "\n            \n\n{answer_meta}\n          </div>\n"
            ));
        }
        out.push('\n');
        if qa.is_some() {
            out.push_str("        </div>\n");
        }
        out.push_str("    </div>\n\n");

        // prev_page / next_page (TopicView): the print chunk's pages, a
        // next one while the stream runs past this chunk.
        let stream_len = view["post_stream"]["stream"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0) as i64;
        let shown_through = page.max(1) * crate::topic_view::PRINT_CHUNK_SIZE;
        let next_page =
            (stream_len > shown_through && !posts.is_empty()).then_some(page.max(1) + 1);
        let prev_page = (page > 1 && !posts.is_empty()).then_some(page - 1);
        if prev_page.is_some() || next_page.is_some() {
            out.push_str("      <div role='navigation' itemscope itemtype='http://schema.org/SiteNavigationElement' class=\"topic-body crawler-post\">\n");
            let page_path = |n: i64| format!("{relative_url}/print?page={n}");
            if let Some(n) = prev_page {
                out.push_str(&format!(
                    "            <span itemprop='name'><a rel=\"prev\" itemprop=\"url\" href=\"{}\">{}</a></span>\n",
                    h(&page_path(n)),
                    h(&t("prev_page"))
                ));
            }
            if let Some(n) = next_page {
                out.push_str(&format!(
                    "            <span itemprop='name'><b><a rel=\"next\" itemprop=\"url\" href=\"{}\">{}</a></b></span>\n",
                    h(&page_path(n)),
                    h(&t("next_page"))
                ));
            }
            out.push_str("      </div>\n");
        }
        out.push_str("\n    \n");
    } else {
        out.push_str(&format!("    {}\n", h(&t("crawler_content_hidden"))));
    }
    out.push_str("\n\n\n");

    // content_for(:title): page_title, then the site's.
    let mut page_title = title.clone();
    if page > 1 {
        page_title.push_str(&format!(
            " - {}",
            state
                .i18n
                .t_with("page_num", &[("num", &page.to_string())])
                .unwrap_or_default()
        ));
    }
    if settings.get("topic_page_title_includes_category")?.truthy() {
        if let Some(name) = category_id
            .filter(|id| i64::from(*id) != uncategorized)
            .and(category_name.as_ref())
        {
            page_title.push_str(&format!(" - {name}"));
        } else if !tags.is_empty() {
            return Err(Unsupported("a print view title with its most used tag").into());
        }
    }
    let site_title = settings.get("title")?.to_s().to_string();
    let document_title = format!("{} - {site_title}", emojis.gsub_to_unicode(&page_title));

    let crawler = super::topics::topic_crawler(
        &mut *conn,
        state,
        settings,
        uri,
        uri.path(),
        view,
        page_image,
    )
    .await?;
    let footer = noscript_footer(&mut *conn, state, settings, &base_path).await?;
    let html = layout(
        state,
        settings,
        &base_path,
        &document_title,
        &crawler,
        &format!("{topic_url}.rss"),
        &title,
        &out,
        &footer,
    )?;
    let vs = super::session::viewer_state(state, headers, settings, guardian)?;
    Ok(crate::html::crawler_response(
        html, &crawler, settings, &vs,
    )?)
}

/// layouts/_noscript_footer, chat's noscript_footer_nav connector in it.
async fn noscript_footer(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    base_path: &str,
) -> Result<String, AppError> {
    let t = |key: &str| h(state.i18n.t(key).unwrap_or(key));
    // One nav entry, `pad` the spaces before its <li>.
    let item = |pad: &str, href: &str, label: &str| {
        format!(
            "{pad}<li itemscope itemtype='http://schema.org/SiteNavigationElement'>\n\
             {pad}  <span itemprop='name'>\n\
             {pad}    <a href='{}' itemprop=\"url\">{label} </a>\n\
             {pad}  </span>\n\
             {pad}</li>\n",
            h(href)
        )
    };
    let mut out =
        String::from("<footer class=\"container wrap\">\n  <nav class='crawler-nav'>\n    <ul>\n");
    out.push_str(&item("      ", &format!("{base_path}/"), &t("home_title")));
    out.push_str(&item(
        "      ",
        &format!("{base_path}/categories"),
        &t("js.filters.categories.title"),
    ));
    out.push_str("      ");
    if crate::plugins::chat::enabled(settings)?
        && crate::plugins::chat::anonymous_public_channel_access_allowed(settings)?
    {
        out.push_str(&format!(
            "  <li itemscope itemtype='http://schema.org/SiteNavigationElement'>\n\
             \x20   <span itemprop='name'>\n\
             \x20     <a href='{}' itemprop=\"url\">{} </a>\n\
             \x20   </span>\n\
             \x20 </li>\n",
            h(&format!("{base_path}/chat")),
            t("chat.crawler.nav_title")
        ));
    }
    out.push('\n');
    out.push_str(&item(
        "      ",
        &format!("{base_path}/guidelines"),
        &t("guidelines_topic.guidelines_title"),
    ));
    if let Some(url) = super::invite_redeem::legal_url(
        &mut *conn,
        settings,
        base_path,
        "tos_url",
        "tos_topic_id",
        "/tos",
    )
    .await?
    {
        out.push_str(&item("        ", &url, &t("tos_topic.title")));
    }
    if let Some(url) = super::invite_redeem::legal_url(
        &mut *conn,
        settings,
        base_path,
        "privacy_policy_url",
        "privacy_topic_id",
        "/privacy",
    )
    .await?
    {
        out.push_str(&item("        ", &url, &t("privacy_topic.title")));
    }
    out.push_str("    </ul>\n  </nav>\n");
    out.push_str(&format!(
        "  <p class='powered-by-link'>{}</p>\n</footer>",
        state.i18n.t("powered_by_html").unwrap_or_default()
    ));
    Ok(out)
}

/// layouts/crawler.html.erb around the view. The head is the port's own
/// (its stylesheets, the page's meta tags); the body follows Rails'.
#[allow(clippy::too_many_arguments)]
fn layout(
    state: &AppState,
    settings: &SiteSettings,
    base_path: &str,
    title: &str,
    crawler: &crate::html::Crawler,
    rss_href: &str,
    topic_title: &str,
    view: &str,
    footer: &str,
) -> Result<String, AppError> {
    let site_title = settings.get("title")?.to_s().to_string();
    let lang = settings.get("default_locale")?.to_s().replace('_', "-");
    let rss_title = state
        .i18n
        .t_with("rss_posts_in_topic", &[("topic", topic_title)])
        .unwrap_or_default();
    let mut head = String::new();
    head.push_str(&format!(
        "    <meta charset=\"utf-8\">\n    <title>{}</title>\n    <meta name=\"description\" content=\"{}\">\n",
        h(title),
        h(&crawler.description)
    ));
    head.push_str("    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0, minimum-scale=1.0, viewport-fit=cover\">\n");
    if !crawler.canonical.is_empty() {
        head.push_str(&format!(
            "    <link rel=\"canonical\" href=\"{}\" />\n",
            h(&crawler.canonical)
        ));
    }
    head.push_str(&format!(
        "    <link href=\"{base_path}/assets/color_definitions_light.css\" media=\"all\" rel=\"stylesheet\" class=\"light-scheme\"/>\n\
         \x20   <link rel=\"stylesheet\" href=\"{}\">\n",
        crate::assets::url(base_path, "discourse.css")
    ));
    head.push_str(&format!(
        "    <link rel=\"alternate nofollow\" type=\"application/rss+xml\" title=\"{}\" href=\"{}\" />\n",
        h(&rss_title),
        h(rss_href)
    ));
    head.push_str(&format!(
        "    <meta property=\"og:site_name\" content=\"{0}\" />\n\
         \x20   <meta property=\"og:type\" content=\"website\" />\n\
         \x20   <meta property=\"og:url\" content=\"{1}\" />\n\
         \x20   <meta name=\"twitter:url\" content=\"{1}\" />\n\
         \x20   <meta property=\"og:title\" content=\"{2}\" />\n\
         \x20   <meta name=\"twitter:title\" content=\"{2}\" />\n",
        h(&site_title),
        h(&crawler.url),
        h(&crawler.title)
    ));
    if !crawler.og_description.is_empty() {
        head.push_str(&format!(
            "    <meta property=\"og:description\" content=\"{0}\" />\n\
             \x20   <meta name=\"twitter:description\" content=\"{0}\" />\n",
            h(&crawler.og_description)
        ));
    }
    if let Some(image) = &crawler.image {
        head.push_str(&format!(
            "    <meta name=\"twitter:image\" content=\"{0}\" />\n\
             \x20   <meta property=\"og:image\" content=\"{0}\" />\n",
            h(image)
        ));
    }
    Ok(format!(
        "<!DOCTYPE html>\n<html lang=\"{lang}\">\n  <head>\n{head}  </head>\n  <body class=\"crawler \">\n    \n    <header>\n  <a href=\"{base_path}/\">{}</a>\n</header>\n\n    <div id=\"main-outlet\" class=\"wrap\" role=\"main\">\n      {view}\n    </div>\n    {footer}\n\n    \n    \n  </body>\n  <script defer src=\"{}\"></script>\n</html>\n",
        h(&site_title),
        crate::assets::url(base_path, "print-page.js")
    ))
}
