//! Port of lib/pretty_text/helpers.rb: the lookups Discourse's markdown rules
//! make while cooking, with HashtagAutocompleteService#lookup and its
//! category and tag data sources for `hashtag_lookup`.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::{CookError, Host};
use crate::Unsupported;
use crate::guardian::Guardian;
use crate::session::current::SessionUser;
use crate::site_settings::SiteSettings;
use crate::topic_guardian::TopicCtx;
use crate::topic_list::TopicListSerializer;
use crate::url::Urls;

/// `Base62::KEYS`
const BASE62: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
/// `Upload::MAX_BASE62_SHA1_LENGTH`
const MAX_BASE62_SHA1_LENGTH: usize = 27;
/// `Category::SLUG_REF_SEPARATOR`
const SLUG_REF_SEPARATOR: char = ':';

/// `t(key, opts)`: `I18n.t("js." + key)`, with the options interpolated
/// both as I18n does (`%{name}`, and `count` choosing the plural form) and
/// as the client templates write them (`{{name}}`).
pub fn translate(host: &Host, key: &Value, opts: &Value) -> String {
    let key = format!("js.{}", key.as_str().unwrap_or_default());
    let opts = opts.as_object().filter(|o| !o.is_empty());
    let missing = || format!("Translation missing: en.{key}");
    let Some(opts) = opts else {
        return host
            .i18n
            .t(&key)
            .map(str::to_string)
            .unwrap_or_else(missing);
    };
    let text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    // English has two plural forms.
    let plural = opts.get("count").map(|count| {
        let one = count.as_f64() == Some(1.0);
        format!("{key}.{}", if one { "one" } else { "other" })
    });
    let Some(template) = plural
        .as_deref()
        .and_then(|k| host.i18n.t(k))
        .or_else(|| host.i18n.t(&key))
    else {
        return missing();
    };
    let mut out = template.to_string();
    for (name, value) in opts {
        let value = text(value);
        out = out.replace(&format!("%{{{name}}}"), &value);
        out = out.replace(&format!("{{{{{name}}}}}"), &value);
    }
    out
}

/// `Base62.decode`, as big-endian bytes.
fn base62_decode(encoded: &str) -> Option<Vec<u8>> {
    let mut bytes: Vec<u8> = vec![0];
    for c in encoded.bytes() {
        let mut carry = BASE62.iter().position(|&k| k == c)? as u32;
        for byte in bytes.iter_mut().rev() {
            let value = u32::from(*byte) * 62 + carry;
            *byte = (value & 0xff) as u8;
            carry = value >> 8;
        }
        while carry > 0 {
            bytes.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    Some(bytes)
}

/// `Base62.encode(sha1.hex)`
pub(crate) fn base62_sha1(sha1: &str) -> Option<String> {
    let mut digits: Vec<u8> = sha1
        .chars()
        .map(|c| c.to_digit(16).map(|d| d as u8))
        .collect::<Option<_>>()?;
    let mut out = Vec::new();
    // Long division of the hex digits by 62.
    while digits.iter().any(|&d| d != 0) {
        let mut remainder = 0u32;
        for digit in digits.iter_mut() {
            let value = remainder * 16 + u32::from(*digit);
            *digit = (value / 62) as u8;
            remainder = value % 62;
        }
        out.push(BASE62[remainder as usize]);
    }
    if out.is_empty() {
        out.push(b'0');
    }
    out.reverse();
    String::from_utf8(out).ok()
}

/// `Upload.sha1_from_base62_encoded`
pub(crate) fn sha1_from_base62(encoded: &str) -> Option<String> {
    if encoded.len() > MAX_BASE62_SHA1_LENGTH {
        return None;
    }
    let bytes = base62_decode(encoded)?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let hex = hex.trim_start_matches('0');
    (hex.len() <= 40).then(|| format!("{hex:0>40}"))
}

/// `Upload.sha1_from_short_url`: the first run of `[a-zA-Z0-9]`, after
/// `upload://` when the url starts with it.
pub(crate) fn sha1_from_short_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("upload://").unwrap_or(url);
    let start = rest.find(|c: char| c.is_ascii_alphanumeric())?;
    let run: String = rest[start..]
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    sha1_from_base62(&run)
}

/// `Rack::Utils.escape_html` (CGI.escapeHTML).
fn escape_html(s: &str) -> String {
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

/// `Integer(value)` as the helpers check it (`is_a?(Integer)`).
fn integer(value: &Value) -> Option<i64> {
    value.as_i64()
}

#[derive(sqlx::FromRow)]
struct CategoryRow {
    id: i32,
    name: String,
    slug: String,
    description: Option<String>,
    color: String,
    parent_category_id: Option<i32>,
    style_type: i32,
    icon: Option<String>,
    emoji: Option<String>,
}

pub struct Helpers<'a> {
    pub host: &'a Host,
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
}

impl<'a> Helpers<'a> {
    /// Borrows the host and the settings, not `self`, so the connection
    /// stays free for the queries next to it.
    fn urls(&self) -> Urls<'a> {
        Urls {
            config: &self.host.config,
            settings: self.settings,
        }
    }

    /// `avatar_template(username)`: the user's template, absolute and
    /// without `http:`; empty for nobody.
    pub async fn avatar_template(&mut self, username: Option<&str>) -> Result<Value, CookError> {
        let Some(username) = username else {
            return Ok(json!(""));
        };
        let user: Option<(i32, String, Option<i32>)> = sqlx::query_as(
            "SELECT id, username, uploaded_avatar_id FROM users WHERE username_lower = $1",
        )
        .bind(username.to_lowercase())
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some((id, username, uploaded_avatar_id)) = user else {
            return Ok(json!(""));
        };
        let anonymous = Guardian::anonymous();
        let urls = self.urls();
        let logo_small_url = TopicListSerializer {
            conn: &mut *self.conn,
            settings: self.settings,
            i18n: &self.host.i18n,
            guardian: &anonymous,
            urls: &urls,
            more_topics_url: None,
            category_id: None,
            group_id: None,
            prefetched: Default::default(),
        }
        .logo_small_url()
        .await?;
        let template = crate::avatar::avatar_template(
            &urls,
            id,
            &username,
            uploaded_avatar_id,
            logo_small_url.as_deref(),
        )?;
        // UrlHelper.schemaless(UrlHelper.absolute(...))
        let absolute = urls.absolute(&template)?;
        let schemaless = match absolute.get(..5).map(|s| s.eq_ignore_ascii_case("http:")) {
            Some(true) => absolute[5..].to_string(),
            _ => absolute,
        };
        Ok(json!(schemaless))
    }

    /// `lookup_primary_user_group(username)`: the group's name, or empty.
    pub async fn primary_user_group(&mut self, username: Option<&str>) -> Result<Value, CookError> {
        let Some(username) = username else {
            return Ok(json!(""));
        };
        let name: Option<String> = sqlx::query_scalar(
            "SELECT g.name FROM users u JOIN groups g ON g.id = u.primary_group_id \
             WHERE u.username_lower = $1",
        )
        .bind(username.to_lowercase())
        .fetch_optional(&mut *self.conn)
        .await?;
        Ok(json!(name.unwrap_or_default()))
    }

    /// `get_current_user(user_id)`
    pub async fn current_user(&mut self, user_id: &Value) -> Result<Value, CookError> {
        let Some(id) = integer(user_id) else {
            return Ok(Value::Null);
        };
        let staff: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND (moderator OR admin))",
        )
        .bind(id)
        .fetch_one(&mut *self.conn)
        .await?;
        Ok(json!({ "staff": staff }))
    }

    /// `get_topic_info(topic_id)`: title and link of a topic anyone can
    /// see; a neutral title and a slugless link for any other.
    pub async fn topic_info(&mut self, topic_id: &Value) -> Result<Value, CookError> {
        let Some(id) = integer(topic_id).and_then(|id| i32::try_from(id).ok()) else {
            return Ok(Value::Null);
        };
        let topic: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT title, slug FROM topics WHERE id = $1 AND deleted_at IS NULL")
                .bind(id)
                .fetch_optional(&mut *self.conn)
                .await?;
        let Some((title, slug)) = topic else {
            return Ok(Value::Null);
        };
        let anonymous = Guardian::anonymous();
        let visible = match TopicCtx::load(&mut *self.conn, self.settings, &anonymous, id).await? {
            Some(ctx) => anonymous.can_see_topic(self.settings, &ctx, true, &[])?,
            None => false,
        };
        if visible {
            let slug = slug
                .filter(|s| !s.is_empty())
                .ok_or(Unsupported("topics without a stored slug (Slug.for)"))?;
            Ok(json!({
                "title": escape_html(&title),
                "href": format!("{}/t/{slug}/{id}", self.urls().base_url()?),
            }))
        } else {
            Ok(json!({
                "title": self.host.i18n.t("on_another_topic").unwrap_or("On another topic"),
                "href": format!("{}/t/{id}", self.host.config.globals.relative_url_root()),
            }))
        }
    }

    /// `lookup_upload_urls(urls)`: per short url of an upload that
    /// exists, its url, short path and base62 sha1.
    pub async fn upload_urls(&mut self, urls: &Value) -> Result<Value, CookError> {
        let mut found: Vec<(String, String)> = Vec::new();
        for url in urls
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let mut sha1 = sha1_from_short_url(url);
            // A video's short url without an extension stands for its
            // thumbnail, an upload named after the video's sha1.
            if url.split('.').nth(1).is_none() {
                let named = sha1.clone().or_else(|| Some(url.to_string()));
                if let Some(name) = named {
                    let pattern = format!(
                        "{}.%",
                        name.replace('\\', "\\\\")
                            .replace('%', "\\%")
                            .replace('_', "\\_")
                    );
                    let thumbnail: Option<String> = sqlx::query_scalar(
                        "SELECT sha1 FROM uploads WHERE original_filename LIKE $1 ORDER BY id DESC LIMIT 1",
                    )
                    .bind(pattern)
                    .fetch_optional(&mut *self.conn)
                    .await?;
                    if thumbnail.is_some() {
                        sha1 = thumbnail;
                    }
                }
            }
            if let Some(sha1) = sha1 {
                found.push((url.to_string(), sha1));
            }
        }
        let mut result = Map::new();
        if found.is_empty() {
            return Ok(Value::Object(result));
        }
        let sha1s: Vec<String> = found.iter().map(|(_, sha1)| sha1.clone()).collect();
        let uploads: Vec<(String, String, Option<String>, bool)> =
            sqlx::query_as("SELECT sha1, url, extension, secure FROM uploads WHERE sha1 = ANY($1)")
                .bind(&sha1s)
                .fetch_all(&mut *self.conn)
                .await?;
        let base_path = self.host.config.globals.relative_url_root();
        for (sha1, url, extension, secure) in uploads {
            if secure && self.settings.get("secure_uploads")?.truthy() {
                return Err(Unsupported("secure uploads in cooking").into());
            }
            // Discourse.store.cdn_url, which only rewrites behind a CDN.
            if self.urls().asset_host().is_some() {
                return Err(Unsupported("upload URLs behind a CDN in cooking").into());
            }
            let Some(base62) = base62_sha1(&sha1) else {
                continue;
            };
            // Upload.short_path: the route's optional `.extension`.
            let short_path = match extension.as_deref().filter(|e| !e.is_empty()) {
                Some(ext) => format!("{base_path}/uploads/short-url/{base62}.{ext}"),
                None => format!("{base_path}/uploads/short-url/{base62}"),
            };
            for (short_url, _) in found.iter().filter(|(_, s)| *s == sha1) {
                result.insert(
                    short_url.clone(),
                    json!({ "url": url, "short_path": short_path, "base62_sha1": base62 }),
                );
            }
        }
        Ok(Value::Object(result))
    }

    /// `hashtag_lookup(slug, cooking_user_id, types_in_priority_order)`:
    /// the first hashtag the slug resolves to for the cooking user, trying
    /// the types in order (or only the one a `::type` suffix names).
    pub async fn hashtag_lookup(
        &mut self,
        slug: &Value,
        cooking_user_id: &Value,
        types: &Value,
    ) -> Result<Value, CookError> {
        let Some(slug) = slug.as_str() else {
            return Ok(Value::Null);
        };
        // A missing or unknown user cooks with anonymous permissions.
        let user = match integer(cooking_user_id).and_then(|id| i32::try_from(id).ok()) {
            Some(id) => SessionUser::load(&mut *self.conn, id).await?,
            None => None,
        };
        let guardian = match &user {
            Some(user) => Guardian::for_user(&mut *self.conn, user).await?,
            None => Guardian::anonymous(),
        };
        let tagging = self.settings.get("tagging_enabled")?.truthy();
        let channels = self.settings.get("chat_enabled")?.truthy()
            && self.settings.get("enable_public_channels")?.truthy();
        let enabled = |ty: &str| match ty {
            "category" => true,
            "tag" => tagging,
            "channel" => channels,
            _ => false,
        };
        let types: Vec<&str> = types
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|ty| enabled(ty))
            .collect();

        // A `::type` suffix of an enabled type pins the lookup to it.
        let suffixed = ["category", "tag", "channel"]
            .into_iter()
            .filter(|ty| enabled(ty))
            .find(|ty| slug.ends_with(&format!("::{ty}")));
        for ty in &types {
            let (lookup, keep_suffix) = match suffixed {
                Some(pinned) if pinned == *ty => (slug.replace(&format!("::{ty}"), ""), true),
                Some(_) => continue,
                None => (slug.to_string(), false),
            };
            let item = match *ty {
                "category" => self.category_hashtag(&guardian, &lookup).await?,
                "channel" => self.channel_hashtag(&guardian, &lookup).await?,
                _ => self.tag_hashtag(&guardian, &lookup).await?,
            };
            if let Some(mut item) = item {
                item.insert("type".into(), json!(ty));
                if keep_suffix {
                    let reference = item["ref"].as_str().unwrap_or_default().to_string();
                    if format!("{reference}::{ty}") == slug {
                        item.insert("ref".into(), json!(slug));
                    }
                }
                return Ok(ordered(item));
            }
        }
        Ok(Value::Null)
    }

    /// Chat::ChannelHashtagDataSource.lookup for one slug: a category
    /// channel the cooking user may post in, while they can chat.
    async fn channel_hashtag(
        &mut self,
        guardian: &Guardian,
        slug: &str,
    ) -> Result<Option<Map<String, Value>>, CookError> {
        // guardian.can_chat?
        if !guardian.is_authenticated()
            || !guardian.in_setting_groups(self.settings, "chat_allowed_groups")?
        {
            return Ok(None);
        }
        // ChannelFetcher.secured_public_channel_slug_lookup
        let sql = format!(
            "SELECT ch.id, COALESCE(NULLIF(ch.name, ''), categories.name) AS title, ch.slug, ch.description, ch.emoji \
             FROM chat_channels ch \
             JOIN categories ON categories.id = ch.chatable_id AND ch.chatable_type = 'Category' \
             WHERE ch.deleted_at IS NULL AND ch.slug = $1 AND {} \
             ORDER BY ch.id LIMIT 1",
            crate::plugins::chat::categories_scoped_to(guardian, "1, 2")
        );
        #[derive(sqlx::FromRow)]
        struct ChannelRow {
            id: i64,
            title: String,
            slug: String,
            description: Option<String>,
            emoji: Option<String>,
        }
        let channel: Option<ChannelRow> = sqlx::query_as(&sql)
            .bind(slug.to_lowercase())
            .fetch_optional(&mut *self.conn)
            .await?;
        let Some(ChannelRow {
            id,
            title,
            slug,
            description,
            emoji,
        }) = channel
        else {
            return Ok(None);
        };
        let emoji = emoji.filter(|e| !e.is_empty());
        let mut item = Map::new();
        item.insert(
            "relative_url".into(),
            json!(format!(
                "{}/chat/c/{slug}/{id}",
                self.host.config.globals.relative_url_root()
            )),
        );
        item.insert("text".into(), json!(title));
        item.insert("description".into(), json!(description));
        item.insert("icon".into(), json!("comment"));
        item.insert("colors".into(), Value::Null);
        item.insert("ref".into(), json!(slug));
        item.insert("slug".into(), json!(slug));
        item.insert("id".into(), json!(id));
        item.insert(
            "style_type".into(),
            json!(if emoji.is_some() { "emoji" } else { "icon" }),
        );
        item.insert("emoji".into(), json!(emoji));
        Ok(Some(item))
    }

    /// CategoryHashtagDataSource.lookup for one slug, `parent:child` too.
    async fn category_hashtag(
        &mut self,
        guardian: &Guardian,
        slug: &str,
    ) -> Result<Option<Map<String, Value>>, CookError> {
        if self.settings.get("slug_generation_method")?.to_s() == "encoded" {
            return Err(Unsupported("category hashtags with encoded slugs").into());
        }
        let slug = slug.to_lowercase();
        let path: Vec<&str> = slug.split(SLUG_REF_SEPARATOR).collect();
        if slug.is_empty() || path.len() > 2 {
            return Ok(None);
        }
        // Category.secured(guardian)
        let secure = guardian
            .secure_category_ids(&mut *self.conn, self.settings)
            .await?;
        let categories: Vec<CategoryRow> = sqlx::query_as(
            "SELECT id, name, slug, description, color, parent_category_id, style_type, icon, emoji \
             FROM categories WHERE NOT read_restricted OR id = ANY($1) \
             ORDER BY parent_category_id ASC NULLS FIRST, id ASC",
        )
        .bind(&secure)
        .fetch_all(&mut *self.conn)
        .await?;
        let same = |a: &str, b: &str| a.to_lowercase() == b.to_lowercase();
        let found = match path.as_slice() {
            [parent, child] if !child.is_empty() => categories.iter().find(|c| {
                same(&c.slug, child)
                    && c.parent_category_id.is_some_and(|pid| {
                        categories
                            .iter()
                            .any(|p| p.id == pid && same(&p.slug, parent))
                    })
            }),
            [top, ..] => categories
                .iter()
                .find(|c| same(&c.slug, top) && c.parent_category_id.is_none()),
            [] => None,
        };
        let Some(category) = found else {
            return Ok(None);
        };
        // The parent is only shown when the viewer can see it as well.
        let parent = category
            .parent_category_id
            .and_then(|pid| categories.iter().find(|p| p.id == pid));
        if category.parent_category_id.is_some() && parent.is_none() {
            return Err(Unsupported("hashtag for a category under a hidden parent").into());
        }
        let text = match parent {
            Some(p) => format!("{} > {}", p.name, category.name),
            None => category.name.clone(),
        };
        let base_path = self.host.config.globals.relative_url_root();
        let (url, reference) = match parent {
            Some(p) => (
                format!("{base_path}/c/{}/{}/{}", p.slug, category.slug, category.id),
                format!("{}{SLUG_REF_SEPARATOR}{}", p.slug, category.slug),
            ),
            None => (
                format!("{base_path}/c/{}/{}", category.slug, category.id),
                category.slug.clone(),
            ),
        };
        // Category.style_types: square 0, icon 1, emoji 2.
        let style_type = match category.style_type {
            0 => "square",
            1 => "icon",
            2 => "emoji",
            _ => return Err(Unsupported("unknown category style_type").into()),
        };
        let mut colors: Vec<&str> = Vec::new();
        if let Some(p) = parent {
            colors.push(&p.color);
        }
        colors.push(&category.color);
        let mut item = Map::new();
        item.insert("relative_url".into(), json!(url));
        item.insert("text".into(), json!(text));
        item.insert(
            "description".into(),
            json!(crate::categories::description_plain_text(
                category.description.as_deref()
            )?),
        );
        item.insert(
            "icon".into(),
            json!(if style_type == "icon" {
                category.icon.as_deref()
            } else {
                Some("folder")
            }),
        );
        item.insert("colors".into(), json!(colors));
        item.insert("ref".into(), json!(reference));
        item.insert("slug".into(), json!(category.slug));
        item.insert("id".into(), json!(category.id));
        item.insert("style_type".into(), json!(style_type));
        item.insert(
            "emoji".into(),
            json!(if style_type == "emoji" {
                category.emoji.as_deref()
            } else {
                None
            }),
        );
        Ok(Some(item))
    }

    /// TagHashtagDataSource.lookup for one name.
    async fn tag_hashtag(
        &mut self,
        guardian: &Guardian,
        name: &str,
    ) -> Result<Option<Map<String, Value>>, CookError> {
        // DiscourseTagging.filter_visible(Tag.where_name(name), guardian)
        let sql = format!(
            "SELECT tags.id, tags.name, tags.slug, tags.description FROM tags \
             WHERE lower(tags.name) = lower($1) AND {} ORDER BY tags.id LIMIT 1",
            crate::tags::visible_tags_where(guardian, self.settings)?
        );
        let tag: Option<(i32, String, Option<String>, Option<String>)> = sqlx::query_as(&sql)
            .bind(name)
            .fetch_optional(&mut *self.conn)
            .await?;
        let Some((id, name, slug, description)) = tag else {
            return Ok(None);
        };
        let slug_for_url = slug
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("{id}-tag"));
        let mut item = Map::new();
        item.insert(
            "relative_url".into(),
            json!(format!(
                "{}/tag/{slug_for_url}/{id}",
                self.host.config.globals.relative_url_root()
            )),
        );
        item.insert("text".into(), json!(name));
        item.insert("description".into(), json!(description));
        item.insert("icon".into(), json!("tag"));
        item.insert("colors".into(), Value::Null);
        // The item's slug is the tag's name, and so is its ref.
        item.insert("ref".into(), json!(name));
        item.insert("slug".into(), json!(name));
        item.insert("id".into(), json!(id));
        item.insert("style_type".into(), json!("icon"));
        item.insert("emoji".into(), Value::Null);
        Ok(Some(item))
    }
}

/// `HashtagItem#to_h`'s key order.
fn ordered(mut item: Map<String, Value>) -> Value {
    let mut out = Map::new();
    for key in [
        "relative_url",
        "text",
        "description",
        "icon",
        "colors",
        "type",
        "ref",
        "slug",
        "id",
        "style_type",
        "emoji",
    ] {
        if let Some(value) = item.remove(key) {
            out.insert(key.into(), value);
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base62_round_trips_a_sha1() {
        let sha1 = "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567";
        let encoded = base62_sha1(sha1).unwrap();
        assert!(encoded.len() <= MAX_BASE62_SHA1_LENGTH);
        assert_eq!(sha1_from_base62(&encoded).as_deref(), Some(sha1));
        assert_eq!(
            sha1_from_short_url(&format!("upload://{encoded}.png")).as_deref(),
            Some(sha1)
        );
        // Base62.encode(255) and back, by hand: 255 = 4 * 62 + 7.
        assert_eq!(base62_sha1("ff").as_deref(), Some("47"));
        assert_eq!(base62_decode("47"), Some(vec![255]));
        // Longer than a sha1 can encode to.
        assert_eq!(sha1_from_base62(&"Z".repeat(28)), None);
    }
}
