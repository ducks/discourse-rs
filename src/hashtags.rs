//! `HashtagAutocompleteService#search` (app/services/hashtag_autocomplete_service.rb)
//! over its data sources: categories (CategoryHashtagDataSource), tags
//! (TagHashtagDataSource) and chat's category channels
//! (Chat::ChannelHashtagDataSource). What the composers' `#` autocomplete
//! reads from /hashtags/search.json.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `HashtagAutocompleteService::HASHTAGS_PER_REQUEST`
const HASHTAGS_PER_REQUEST: usize = 20;
/// `HashtagAutocompleteService::SEARCH_MAX_LIMIT`
const SEARCH_MAX_LIMIT: usize = 50;
/// `Chat::ChannelFetcher::MAX_PUBLIC_CHANNEL_RESULTS`
const MAX_PUBLIC_CHANNEL_RESULTS: i64 = 100;
/// `Category::SLUG_REF_SEPARATOR`
const SLUG_REF_SEPARATOR: &str = ":";

#[derive(Clone, Copy, PartialEq)]
enum Condition {
    StartsWith,
    Contains,
}

/// `HashtagAutocompleteService::HashtagItem`, serialized as Rails does:
/// its instance variables, those `initialize` sets first, in that order,
/// then `secondary_text` when a data source set one.
#[derive(Clone)]
pub struct Item {
    relative_url: String,
    text: String,
    description: Option<String>,
    style_type: String,
    icon: String,
    emoji: Option<String>,
    colors: Option<Vec<String>>,
    kind: String,
    reference: String,
    slug: String,
    id: i64,
    secondary_text: Option<String>,
}

impl Item {
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("relative_url".into(), json!(self.relative_url));
        out.insert("text".into(), json!(self.text));
        out.insert("description".into(), json!(self.description));
        out.insert("style_type".into(), json!(self.style_type));
        out.insert("icon".into(), json!(self.icon));
        out.insert("emoji".into(), json!(self.emoji));
        out.insert("colors".into(), json!(self.colors));
        out.insert("type".into(), json!(self.kind));
        out.insert("ref".into(), json!(self.reference));
        out.insert("slug".into(), json!(self.slug));
        out.insert("id".into(), json!(self.id));
        if let Some(text) = &self.secondary_text {
            out.insert("secondary_text".into(), json!(text));
        }
        Value::Object(out)
    }
}

pub struct Search<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub guardian: &'a Guardian,
    pub base_path: &'a str,
}

impl Search<'_> {
    /// `data_source_types`: the enabled data sources.
    fn enabled(&self, kind: &str) -> Result<bool, AppError> {
        Ok(match kind {
            "category" => true,
            "tag" => self.settings.get("tagging_enabled")?.truthy(),
            "channel" => {
                crate::plugins::chat::enabled(self.settings)?
                    && self.settings.get("enable_public_channels")?.truthy()
            }
            _ => false,
        })
    }

    /// `search(term, types_in_priority_order, limit: 20)`
    pub async fn search(
        &mut self,
        term: Option<&str>,
        order: &[Value],
    ) -> Result<Vec<Item>, AppError> {
        let limit = 20.min(SEARCH_MAX_LIMIT);
        let mut types = Vec::new();
        for kind in order.iter().filter_map(Value::as_str) {
            if self.enabled(kind)? {
                types.push(kind.to_string());
            }
        }
        let term = term.filter(|t| !t.trim().is_empty());
        let Some(term) = term else {
            return self.search_without_term(&types, limit).await;
        };
        let term = term.to_lowercase();

        // Exact slug matches float to the top.
        let mut results: Vec<Item> = Vec::new();
        for kind in &types {
            let mut found = valid(self.lookup(kind, &term).await?);
            found.sort_by_key(|item| item.text.to_lowercase());
            results.extend(found);
            if results.len() >= limit {
                break;
            }
        }
        let mut top_ranked: Option<String> = None;
        for condition in [Condition::StartsWith, Condition::Contains] {
            if results.len() >= limit {
                break;
            }
            for kind in &types {
                self.search_using_condition(&mut results, &term, kind, limit, condition)
                    .await?;
                top_ranked.get_or_insert_with(|| kind.clone());
                if results.len() >= limit {
                    break;
                }
            }
        }
        Ok(append_types_to_conflicts(
            results,
            top_ranked.as_deref(),
            &types,
            limit,
        ))
    }

    async fn search_using_condition(
        &mut self,
        results: &mut Vec<Item>,
        term: &str,
        kind: &str,
        limit: usize,
        condition: Condition,
    ) -> Result<(), AppError> {
        let found = valid(
            self.search_for_type(kind, term, limit - results.len(), condition)
                .await?,
        );
        if found.is_empty() {
            return Ok(());
        }
        let mut found: Vec<Item> = found
            .into_iter()
            .filter(|item| {
                !results
                    .iter()
                    .any(|r| r.kind == kind && r.slug == item.slug)
            })
            .collect();
        // search_sort: categories put an exact slug first.
        if kind == "category" {
            found.sort_by_key(|item| (item.slug != term, item.text.to_lowercase()));
        } else {
            found.sort_by_key(|item| item.text.to_lowercase());
        }
        results.extend(found);
        Ok(())
    }

    async fn search_without_term(
        &mut self,
        types: &[String],
        limit: usize,
    ) -> Result<Vec<Item>, AppError> {
        if types.is_empty() {
            // (limit.to_f / 0.0).ceil raises FloatDomainError.
            return Err(Unsupported("hashtag search with no enabled type").into());
        }
        let split = limit.div_ceil(types.len()) as i64;
        let mut results = Vec::new();
        for kind in types {
            let found = match kind.as_str() {
                "category" => self.categories_without_term(split).await?,
                "tag" => self.tags_without_term(split).await?,
                _ => self.channels_without_term(split).await?,
            };
            results.extend(valid(found));
        }
        results.truncate(limit);
        Ok(results)
    }

    /// `lookup(slugs, types_in_priority_order)`: each type's hashtags for
    /// the slugs (at most HASHTAGS_PER_REQUEST + 1 of them), a slug without
    /// a `::type` suffix going to the first type that has it.
    pub async fn lookup_slugs(
        &mut self,
        slugs: &[String],
        order: &[String],
    ) -> Result<Map<String, Value>, AppError> {
        let all_types = ["category", "tag", "channel"];
        let mut enabled_types = Vec::new();
        for kind in all_types {
            if self.enabled(kind)? {
                enabled_types.push(kind);
            }
        }
        let types: Vec<&str> = order
            .iter()
            .map(String::as_str)
            .filter(|t| enabled_types.contains(t))
            .collect();
        let mut results: Vec<(String, Vec<Item>)> =
            types.iter().map(|t| (t.to_string(), Vec::new())).collect();
        let limited: Vec<&String> = slugs.iter().take(HASHTAGS_PER_REQUEST + 1).collect();
        let has_suffix = |slug: &str| {
            enabled_types
                .iter()
                .any(|t| slug.ends_with(&format!("::{t}")))
        };
        let mut without: Vec<String> = limited
            .iter()
            .filter(|s| !has_suffix(s))
            .map(|s| s.to_string())
            .collect();
        let with: Vec<String> = limited
            .iter()
            .filter(|s| has_suffix(s))
            .map(|s| s.to_string())
            .collect();
        if !without.is_empty() {
            for kind in &types {
                let to_lookup: Vec<String> = without
                    .iter()
                    .filter(|slug| !with.contains(&format!("{slug}::{kind}")))
                    .cloned()
                    .collect();
                let found = self.execute_lookup(&mut results, kind, &to_lookup).await?;
                let refs: Vec<String> = found.iter().map(|i| i.reference.clone()).collect();
                without.retain(|slug| !refs.contains(slug));
                if without.is_empty() {
                    break;
                }
            }
        }
        if !with.is_empty() {
            for kind in &types {
                let suffix = format!("::{kind}");
                let for_type: Vec<String> = with
                    .iter()
                    .filter(|slug| slug.ends_with(&suffix))
                    .map(|slug| slug.replace(&suffix, ""))
                    .collect();
                if for_type.is_empty() {
                    continue;
                }
                self.execute_lookup(&mut results, kind, &for_type).await?;
                // The refs go out as they came in.
                if let Some((_, items)) = results.iter_mut().find(|(t, _)| t == kind) {
                    for item in items.iter_mut() {
                        let suffixed = format!("{}{suffix}", item.reference);
                        if with.contains(&suffixed) {
                            item.reference = suffixed;
                        }
                    }
                }
            }
        }
        Ok(results
            .into_iter()
            .map(|(kind, items)| {
                (
                    kind,
                    Value::Array(items.iter().map(Item::to_json).collect()),
                )
            })
            .collect())
    }

    /// `execute_lookup!`: a type's hashtags for the slugs, by their text,
    /// added to the results.
    async fn execute_lookup(
        &mut self,
        results: &mut [(String, Vec<Item>)],
        kind: &str,
        slugs: &[String],
    ) -> Result<Vec<Item>, AppError> {
        // Category.query_loaded_from_slugs finds a category per slug (one
        // found twice comes twice); tags and channels are one IN query.
        let mut found: Vec<Item> = Vec::new();
        for slug in slugs {
            for item in valid(self.lookup(kind, slug).await?) {
                if kind == "category" || !found.iter().any(|f| f.id == item.id) {
                    found.push(item);
                }
            }
        }
        found.sort_by_key(|item| item.text.to_lowercase());
        if let Some((_, items)) = results.iter_mut().find(|(t, _)| t == kind) {
            items.extend(found.iter().cloned());
        }
        Ok(found)
    }

    /// `find_by_ids(ids_by_type)`: the categories and channels by id (tags
    /// have no find_by_ids), each type with any.
    pub async fn find_by_ids(
        &mut self,
        ids: &[(String, Vec<i64>)],
    ) -> Result<Map<String, Value>, AppError> {
        let mut out = Map::new();
        for kind in ["category", "tag", "channel"] {
            if !self.enabled(kind)? {
                continue;
            }
            let Some((_, wanted)) = ids.iter().find(|(t, list)| t == kind && !list.is_empty())
            else {
                continue;
            };
            let list = wanted
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let found = match kind {
                "category" => {
                    self.categories(&format!("categories.id IN ({list})"), "")
                        .await?
                }
                "channel" => {
                    self.allowed_channels(&format!("chat_channels.id IN ({list})"), "")
                        .await?
                }
                _ => continue,
            };
            if !found.is_empty() {
                out.insert(
                    kind.into(),
                    Value::Array(found.iter().map(Item::to_json).collect()),
                );
            }
        }
        Ok(out)
    }

    async fn lookup(&mut self, kind: &str, slug: &str) -> Result<Vec<Item>, AppError> {
        match kind {
            "category" => self.category_lookup(slug).await,
            "tag" => self.tag_lookup(slug).await,
            _ => self.channel_lookup(slug).await,
        }
    }

    async fn search_for_type(
        &mut self,
        kind: &str,
        term: &str,
        limit: usize,
        condition: Condition,
    ) -> Result<Vec<Item>, AppError> {
        match kind {
            "category" => self.category_search(term, limit as i64, condition).await,
            "tag" => self.tag_search(term, limit as i64, condition).await,
            _ => self.channel_search(term, limit as i64, condition).await,
        }
    }

    // Categories

    /// `Category.secured(guardian)` as a condition on `categories`.
    async fn secured_categories(&mut self) -> Result<String, AppError> {
        let ids = self
            .guardian
            .secure_category_ids(&mut *self.conn, self.settings)
            .await?;
        let ids: Vec<String> = ids.iter().map(i32::to_string).collect();
        Ok(if ids.is_empty() {
            "NOT categories.read_restricted".to_string()
        } else {
            format!(
                "(NOT categories.read_restricted OR categories.id IN ({}))",
                ids.join(", ")
            )
        })
    }

    async fn categories(&mut self, condition: &str, tail: &str) -> Result<Vec<Item>, AppError> {
        if self.settings.get("slug_generation_method")?.to_s() == "encoded" {
            return Err(Unsupported("category hashtags with encoded slugs").into());
        }
        let secured = self.secured_categories().await?;
        // The parent comes along unsecured (includes(:parent_category)),
        // the grandparent for its slug path.
        let sql = format!(
            "SELECT categories.id, categories.name, categories.slug, categories.description, \
               categories.color, categories.style_type, categories.icon, categories.emoji, \
               p.id AS parent_id, p.name AS parent_name, p.slug AS parent_slug, p.color AS parent_color, \
               g.id AS grandparent_id, g.slug AS grandparent_slug \
             FROM categories \
             LEFT JOIN categories p ON p.id = categories.parent_category_id \
             LEFT JOIN categories g ON g.id = p.parent_category_id \
             WHERE {secured} AND ({condition}) {tail}"
        );
        let rows: Vec<CategoryRow> = sqlx::query_as(&sql).fetch_all(&mut *self.conn).await?;
        rows.iter().map(|row| self.category_item(row)).collect()
    }

    /// `CategoryHashtagDataSource.category_to_hashtag_item`
    fn category_item(&self, c: &CategoryRow) -> Result<Item, AppError> {
        let slug_for_url = |id: i32, slug: &str| {
            if slug.is_empty() {
                format!("{id}-category")
            } else {
                slug.to_string()
            }
        };
        let mut path = Vec::new();
        if let (Some(id), Some(slug)) = (c.grandparent_id, &c.grandparent_slug) {
            if self.settings.get("max_category_nesting")?.to_i() > 3 {
                return Err(Unsupported("hashtags for categories nested past three levels").into());
            }
            path.push(slug_for_url(id, slug));
        }
        if let (Some(id), Some(slug)) = (c.parent_id, &c.parent_slug) {
            path.push(slug_for_url(id, slug));
        }
        path.push(slug_for_url(c.id, &c.slug));
        let style_type = match c.style_type {
            0 => "square",
            1 => "icon",
            2 => "emoji",
            _ => return Err(Unsupported("unknown category style_type").into()),
        };
        let mut colors = Vec::new();
        if let Some(color) = &c.parent_color {
            colors.push(color.clone());
        }
        colors.push(c.color.clone());
        Ok(Item {
            relative_url: format!("{}/c/{}/{}", self.base_path, path.join("/"), c.id),
            text: match &c.parent_name {
                Some(parent) => format!("{parent} > {}", c.name),
                None => c.name.clone(),
            },
            description: crate::categories::description_plain_text(c.description.as_deref())?,
            style_type: style_type.into(),
            icon: if style_type == "icon" {
                c.icon.clone().unwrap_or_default()
            } else {
                "folder".into()
            },
            emoji: if style_type == "emoji" {
                c.emoji.clone()
            } else {
                None
            },
            colors: Some(colors),
            kind: "category".into(),
            // slug_ref: one level of parent.
            reference: match &c.parent_slug {
                Some(parent) => format!("{parent}{SLUG_REF_SEPARATOR}{}", c.slug),
                None => c.slug.clone(),
            },
            slug: c.slug.clone(),
            id: c.id as i64,
            secondary_text: None,
        })
    }

    /// `Category.query_loaded_from_slugs([slug], user_categories)`: a
    /// top-level slug, or `parent:child`.
    async fn category_lookup(&mut self, slug: &str) -> Result<Vec<Item>, AppError> {
        let parts: Vec<&str> = slug.split(SLUG_REF_SEPARATOR).collect();
        let condition = match parts.as_slice() {
            [top] => format!(
                "lower(categories.slug) = lower({}) AND categories.parent_category_id IS NULL",
                quote(top)
            ),
            [parent, child] if !child.is_empty() => {
                // The parent must be one the user can see as well.
                let secured = self
                    .secured_categories()
                    .await?
                    .replace("categories.", "pc.");
                format!(
                    "lower(categories.slug) = lower({}) AND categories.parent_category_id IN \
                     (SELECT pc.id FROM categories pc WHERE lower(pc.slug) = lower({}) AND {secured})",
                    quote(child),
                    quote(parent)
                )
            }
            _ => return Ok(Vec::new()),
        };
        let mut found = self
            .categories(
                &condition,
                "ORDER BY categories.parent_category_id ASC NULLS FIRST, categories.id ASC",
            )
            .await?;
        found.truncate(1);
        Ok(found)
    }

    async fn category_search(
        &mut self,
        term: &str,
        limit: i64,
        condition: Condition,
    ) -> Result<Vec<Item>, AppError> {
        let t = quote(term);
        let condition = match condition {
            Condition::StartsWith => {
                format!("starts_with(lower(unaccent(categories.slug)), lower(unaccent({t})))")
            }
            Condition::Contains => format!(
                "position(lower(unaccent({t})) IN lower(unaccent(categories.name))) <> 0 \
                 OR position(lower(unaccent({t})) IN lower(unaccent(categories.slug))) <> 0"
            ),
        };
        self.categories(&condition, &format!("LIMIT {limit}")).await
    }

    async fn categories_without_term(&mut self, limit: i64) -> Result<Vec<Item>, AppError> {
        let Some(user_id) = self.guardian.user_id() else {
            return Err(Unsupported("hashtag search without a user").into());
        };
        // CategoryUser.default_notification_level: muted or regular.
        let default_level = if self
            .settings
            .get("mute_all_categories_by_default")?
            .truthy()
        {
            0
        } else {
            1
        };
        let nesting3 = self.settings.get("max_category_nesting")?.to_i() == 3;
        let muted = muted_category_ids_query(user_id, default_level, nesting3);
        self.categories(
            &format!("categories.id NOT IN ({muted})"),
            &format!("ORDER BY categories.topic_count DESC LIMIT {limit}"),
        )
        .await
    }

    // Tags

    fn topic_count_column(&self) -> Result<&'static str, AppError> {
        Ok(
            if self.guardian.is_staff()
                || self
                    .settings
                    .get("include_secure_categories_in_tag_counts")?
                    .truthy()
            {
                "staff_topic_count"
            } else {
                "public_topic_count"
            },
        )
    }

    /// `DiscourseTagging.filter_allowed_tags` without a category, selected
    /// tags or `for_input`: the visible tags matching, each once.
    async fn tags(&mut self, condition: &str, tail: &str) -> Result<Vec<Item>, AppError> {
        if self.settings.get("content_localization_enabled")?.truthy() {
            return Err(Unsupported("hashtag search over localized tags").into());
        }
        let count = self.topic_count_column()?;
        let visible = crate::tags::visible_tags_where(self.guardian, self.settings)?;
        let sql = format!(
            "SELECT tags.id, tags.name, tags.slug, tags.description, tags.{count} AS count \
             FROM tags WHERE {visible} AND ({condition}) {tail}"
        );
        let rows: Vec<TagRow> = sqlx::query_as(&sql).fetch_all(&mut *self.conn).await?;
        Ok(rows.into_iter().map(|row| self.tag_item(row)).collect())
    }

    /// `TagHashtagDataSource.tag_to_hashtag_item`
    fn tag_item(&self, row: TagRow) -> Item {
        let TagRow {
            id,
            name,
            slug,
            description,
            count,
        } = row;
        let slug_for_url = slug
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("{id}-tag"));
        Item {
            relative_url: format!("{}/tag/{slug_for_url}/{id}", self.base_path),
            text: name.clone(),
            description,
            style_type: "icon".into(),
            icon: "tag".into(),
            emoji: None,
            colors: None,
            kind: "tag".into(),
            reference: name.clone(),
            slug: name,
            id: id as i64,
            secondary_text: Some(format!("x{count}")),
        }
    }

    async fn tag_lookup(&mut self, name: &str) -> Result<Vec<Item>, AppError> {
        self.tags(&format!("lower(tags.name) = lower({})", quote(name)), "")
            .await
    }

    async fn tag_search(
        &mut self,
        term: &str,
        limit: i64,
        condition: Condition,
    ) -> Result<Vec<Item>, AppError> {
        let t = quote(term);
        let count = self.topic_count_column()?;
        let condition = match condition {
            Condition::StartsWith => format!("starts_with(lower(tags.name), lower({t}))"),
            Condition::Contains => format!("position(lower({t}) IN lower(tags.name)) <> 0"),
        };
        self.tags(
            &condition,
            &format!("ORDER BY lower(tags.name) = lower({t}) DESC, tags.{count} DESC, tags.name LIMIT {limit}"),
        )
        .await
    }

    async fn tags_without_term(&mut self, limit: i64) -> Result<Vec<Item>, AppError> {
        let count = self.topic_count_column()?;
        let muted = match self.guardian.user_id() {
            Some(id) => format!(
                "tags.name NOT IN (SELECT t.name FROM tag_users tu JOIN tags t ON t.id = tu.tag_id \
                 WHERE tu.user_id = {id} AND tu.notification_level = 0)"
            ),
            None => "TRUE".into(),
        };
        self.tags(
            &muted,
            &format!("ORDER BY tags.{count} DESC, tags.name LIMIT {limit}"),
        )
        .await
    }

    // Channels

    /// The category channels the user may post in, as
    /// `ChannelFetcher.secured_public_channel_search` and the slug lookup
    /// find them, when they can chat.
    async fn channels(&mut self, condition: &str, tail: &str) -> Result<Vec<Item>, AppError> {
        if !crate::plugins::chat::can_chat(&mut *self.conn, self.settings, self.guardian).await? {
            return Ok(Vec::new());
        }
        self.allowed_channels(condition, tail).await
    }

    /// The category channels among `generate_allowed_channel_ids_sql`'s
    /// (DM channels left out).
    async fn allowed_channels(
        &mut self,
        condition: &str,
        tail: &str,
    ) -> Result<Vec<Item>, AppError> {
        let allowed =
            crate::plugins::chat::allowed_channel_ids_sql(self.settings, self.guardian, true)?;
        let sql = format!(
            "SELECT chat_channels.id, COALESCE(NULLIF(chat_channels.name, ''), categories.name) AS title, \
               chat_channels.slug, chat_channels.description, chat_channels.emoji, categories.color \
             FROM chat_channels \
             INNER JOIN categories ON categories.id = chat_channels.chatable_id \
             WHERE chat_channels.deleted_at IS NULL AND chat_channels.chatable_type = 'Category' \
               AND chat_channels.id IN ({allowed}) AND ({condition}) {tail}"
        );
        let rows: Vec<ChannelRow> = sqlx::query_as(&sql).fetch_all(&mut *self.conn).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let ChannelRow {
                    id,
                    title,
                    slug,
                    description,
                    emoji,
                    color,
                } = row;
                let has_emoji = emoji.as_deref().is_some_and(|e| !e.trim().is_empty());
                Item {
                    relative_url: format!(
                        "{}/chat/c/{}/{id}",
                        self.base_path,
                        slug.as_deref().unwrap_or("-")
                    ),
                    text: title,
                    description,
                    style_type: if has_emoji { "emoji" } else { "icon" }.into(),
                    icon: "comment".into(),
                    emoji,
                    colors: Some(vec![color]),
                    kind: "channel".into(),
                    reference: slug.clone().unwrap_or_default(),
                    slug: slug.unwrap_or_default(),
                    id,
                    secondary_text: None,
                }
            })
            .collect())
    }

    async fn channel_lookup(&mut self, slug: &str) -> Result<Vec<Item>, AppError> {
        self.channels(&format!("chat_channels.slug = {}", quote(slug)), "")
            .await
    }

    async fn channel_search(
        &mut self,
        term: &str,
        limit: i64,
        condition: Condition,
    ) -> Result<Vec<Item>, AppError> {
        // secured_public_channel_search is empty with public channels off,
        // and clamps its limit.
        if !self.settings.get("enable_public_channels")?.truthy() {
            return Ok(Vec::new());
        }
        let like = like_escape(&term.to_lowercase());
        let pattern = quote(&match condition {
            Condition::StartsWith => format!("{like}%"),
            Condition::Contains => format!("%{like}%"),
        });
        let limit = limit.clamp(1, MAX_PUBLIC_CHANNEL_RESULTS);
        self.channels(
            &format!("chat_channels.name ILIKE {pattern} OR chat_channels.slug ILIKE {pattern}"),
            &format!("ORDER BY chat_channels.name ASC, categories.name ASC LIMIT {limit}"),
        )
        .await
    }

    async fn channels_without_term(&mut self, limit: i64) -> Result<Vec<Item>, AppError> {
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(Vec::new());
        };
        self.channels(
            &format!(
                "chat_channels.id IN (SELECT chat_channel_id FROM user_chat_channel_memberships \
                 WHERE user_id = {user_id} AND following = true)"
            ),
            &format!("ORDER BY chat_channels.messages_count DESC LIMIT {limit}"),
        )
        .await
    }
}

#[derive(sqlx::FromRow)]
struct TagRow {
    id: i32,
    name: String,
    slug: Option<String>,
    description: Option<String>,
    count: i32,
}

#[derive(sqlx::FromRow)]
struct ChannelRow {
    id: i64,
    title: String,
    slug: Option<String>,
    description: Option<String>,
    emoji: Option<String>,
    color: String,
}

#[derive(sqlx::FromRow)]
struct CategoryRow {
    id: i32,
    name: String,
    slug: String,
    description: Option<String>,
    color: String,
    style_type: i32,
    icon: Option<String>,
    emoji: Option<String>,
    parent_id: Option<i32>,
    parent_name: Option<String>,
    parent_slug: Option<String>,
    parent_color: Option<String>,
    grandparent_id: Option<i32>,
    grandparent_slug: Option<String>,
}

/// `filter_valid_data_items`
fn valid(items: Vec<Item>) -> Vec<Item> {
    items
        .into_iter()
        .filter(|item| !item.slug.trim().is_empty() && !item.text.trim().is_empty())
        .collect()
}

/// `append_types_to_conflicts`: an item not of the top-ranked type whose
/// slug a higher-ranked type also has gets `::type` on its ref.
fn append_types_to_conflicts(
    mut results: Vec<Item>,
    top_ranked: Option<&str>,
    types: &[String],
    limit: usize,
) -> Vec<Item> {
    let snapshot: Vec<(String, String)> = results
        .iter()
        .map(|r| (r.kind.clone(), r.slug.clone()))
        .collect();
    for item in &mut results {
        if Some(item.kind.as_str()) == top_ranked {
            continue;
        }
        let rank = types.iter().position(|t| *t == item.kind).unwrap_or(0);
        let higher = &types[..rank];
        if snapshot
            .iter()
            .any(|(kind, slug)| *kind != item.kind && higher.contains(kind) && *slug == item.slug)
        {
            item.reference = format!("{}::{}", item.reference, item.kind);
        }
    }
    results.truncate(limit);
    results
}

/// `CategoryUser.muted_category_ids_query(user, include_direct: true)`
fn muted_category_ids_query(user_id: i32, default_level: i64, nesting3: bool) -> String {
    let mut sql = format!(
        "SELECT mc.id FROM categories mc \
         LEFT JOIN categories mc2 ON mc2.id = mc.parent_category_id \
         LEFT JOIN category_users cu ON cu.category_id = mc.id AND cu.user_id = {user_id} \
         LEFT JOIN category_users cu2 ON cu2.category_id = mc2.id AND cu2.user_id = {user_id} "
    );
    let mut conditions = vec![
        format!("(cu.id IS NULL AND COALESCE(cu2.notification_level, {default_level}) = 0)"),
        format!("COALESCE(cu.notification_level, {default_level}) = 0"),
    ];
    if nesting3 {
        sql.push_str(&format!(
            "LEFT JOIN categories mc3 ON mc3.id = mc2.parent_category_id \
             LEFT JOIN category_users cu3 ON cu3.category_id = mc3.id AND cu3.user_id = {user_id} "
        ));
        conditions.push(format!(
            "(cu.id IS NULL AND cu2.id IS NULL AND COALESCE(cu3.notification_level, {default_level}) = 0)"
        ));
    }
    sql.push_str(&format!("WHERE {}", conditions.join(" OR ")));
    sql
}

/// A SQL string literal.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// `sanitize_sql_like`
fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// The `hashtag-css-generator` initializer's style: CategoryHashtagType's
/// default square color, then each site category's
/// (`generateColorCssClasses`): a square's background (split with the
/// parent's color for a subcategory), an icon's color, nothing for an
/// emoji. TagHashtagType preloads nothing.
pub fn generator_css(categories: &[Value]) -> String {
    let mut rules =
        vec![".hashtag-category-square { background-color: var(--primary-medium); }".to_string()];
    for c in categories {
        let color = c["color"].as_str().unwrap_or_default();
        let parent_color = c["parent_category_id"].as_i64().and_then(|parent| {
            categories
                .iter()
                .find(|p| p["id"].as_i64() == Some(parent))
                .and_then(|p| p["color"].as_str())
        });
        let style = match c["style_type"].as_str() {
            Some("square") => match parent_color {
                Some(parent) => {
                    format!("background: linear-gradient(-90deg, #{color} 50%, #{parent} 50%);")
                }
                None => format!("background-color: #{color};"),
            },
            Some("icon") => format!("color: #{color};"),
            _ => continue,
        };
        rules.push(format!(
            ".hashtag-color--category-{} {{ {style} }}",
            c["id"].as_i64().unwrap_or_default()
        ));
    }
    rules.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, slug: &str) -> Item {
        Item {
            relative_url: String::new(),
            text: slug.into(),
            description: None,
            style_type: "icon".into(),
            icon: String::new(),
            emoji: None,
            colors: None,
            kind: kind.into(),
            reference: slug.into(),
            slug: slug.into(),
            id: 1,
            secondary_text: None,
        }
    }

    #[test]
    fn lower_ranked_conflicts_name_their_type() {
        let types: Vec<String> = ["channel", "category", "tag"].map(String::from).to_vec();
        let out = append_types_to_conflicts(
            vec![
                item("channel", "general"),
                item("category", "general"),
                item("tag", "general"),
                item("tag", "other"),
            ],
            Some("channel"),
            &types,
            20,
        );
        let refs: Vec<&str> = out.iter().map(|i| i.reference.as_str()).collect();
        assert_eq!(
            refs,
            ["general", "general::category", "general::tag", "other"]
        );
    }

    #[test]
    fn like_patterns_are_escaped() {
        assert_eq!(like_escape(r"50%_a\b"), r"50\%\_a\\b");
        assert_eq!(quote("it's"), "'it''s'");
    }
}
