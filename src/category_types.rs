//! `Categories::TypeRegistry`: core's Discussion type and the types the
//! bundled plugins register whether or not they are enabled (in load
//! order: discourse-events' Events, discourse-solved's Support,
//! discourse-topic-voting's Ideas), with each type's metadata
//! (`Categories::Types::Base.metadata`) and its resolved configuration
//! schema.
//!
//! Configuring a category as a type (the category creator) isn't ported.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::admin_site_settings::{self, Context};
use crate::guardian::Guardian;
use crate::{AppError, Unsupported};

/// A registered type.
#[derive(Clone, Copy, PartialEq)]
enum Type {
    Discussion,
    Events,
    Support,
    Ideas,
}

const TYPES: [Type; 4] = [Type::Discussion, Type::Events, Type::Support, Type::Ideas];

/// One setting a type's schema configures.
struct SiteSettingConfig {
    key: &'static str,
    default: Value,
    type_name: Option<&'static str>,
    label: Option<String>,
    choices: Option<Value>,
    required: Option<bool>,
}

/// A category setting or custom field a type's schema declares.
struct FieldConfig {
    key: &'static str,
    default: Value,
    type_name: &'static str,
    subtype: Option<&'static str>,
    label: String,
    description: Option<String>,
    choices: Option<Value>,
    required: Option<bool>,
    show_on_create: Option<bool>,
    show_on_edit: Option<bool>,
}

struct SiteTextConfig {
    key: &'static str,
    label: String,
    description: Option<String>,
    depends_on: Option<&'static str>,
}

#[derive(Default)]
struct Schema {
    general: Vec<(&'static str, Value)>,
    site_settings: Vec<SiteSettingConfig>,
    category_settings: Vec<FieldConfig>,
    category_custom_fields: Vec<FieldConfig>,
    site_texts: Vec<SiteTextConfig>,
}

fn field(key: &'static str, default: Value, type_name: &'static str, label: String) -> FieldConfig {
    FieldConfig {
        key,
        default,
        type_name,
        subtype: None,
        label,
        description: None,
        choices: None,
        required: None,
        show_on_create: None,
        show_on_edit: None,
    }
}

fn setting(key: &'static str, default: Value) -> SiteSettingConfig {
    SiteSettingConfig {
        key,
        default,
        type_name: None,
        label: None,
        choices: None,
        required: None,
    }
}

pub struct Registry<'a> {
    pub cx: &'a Context<'a>,
}

impl Registry<'_> {
    fn t(&self, key: &str) -> String {
        self.cx.i18n.t(key).unwrap_or(key).to_string()
    }

    fn id(t: Type) -> &'static str {
        match t {
            Type::Discussion => "discussion",
            Type::Events => "events",
            Type::Support => "support",
            Type::Ideas => "ideas",
        }
    }

    fn icon(t: Type) -> &'static str {
        match t {
            Type::Discussion => "memo",
            Type::Events => "spiral_calendar",
            Type::Support => "person_raising_hand",
            Type::Ideas => "bulb",
        }
    }

    /// The plugin a type enables (`enables_plugin?`): its humanized name
    /// and `plugin_enabled?`.
    fn plugin(&self, t: Type) -> Result<Option<(&'static str, bool)>, AppError> {
        let on =
            |name: &str| -> Result<bool, AppError> { Ok(self.cx.settings.get(name)?.truthy()) };
        Ok(match t {
            Type::Discussion => None,
            Type::Events => Some((
                "Events",
                on("discourse_events_enabled")? && on("discourse_post_event_enabled")?,
            )),
            Type::Support => Some(("Solved", on("solved_enabled")?)),
            Type::Ideas => Some(("Topic Voting", on("topic_voting_enabled")?)),
        })
    }

    async fn schema(&self, conn: &mut PgConnection, t: Type) -> Result<Option<Schema>, AppError> {
        let general = |name: String, emoji: &str| {
            vec![
                ("name", json!(name)),
                ("style_type", json!("emoji")),
                ("emoji", json!(emoji)),
            ]
        };
        let choice = |key: &str, value: Value| json!({ "name": self.t(key), "value": value });
        Ok(Some(match t {
            Type::Discussion => return Ok(None),
            Type::Events => {
                let p = "discourse_events.category_type";
                Schema {
                    general: general(self.t("category_types.events.name"), "spiral_calendar"),
                    category_settings: vec![
                        FieldConfig {
                            required: Some(true),
                            choices: Some(json!(
                                ["day", "week", "month", "year"]
                                    .iter()
                                    .map(|v| choice(
                                        &format!("{p}.default_calendar_view.{v}"),
                                        json!(v)
                                    ))
                                    .collect::<Vec<_>>()
                            )),
                            ..field(
                                "events_calendar_default_view",
                                json!("month"),
                                "enum",
                                self.t(&format!("{p}.default_calendar_view.label")),
                            )
                        },
                        field(
                            "events_calendar_display_weekends",
                            json!(true),
                            "bool",
                            self.t(&format!("{p}.display_weekends.label")),
                        ),
                    ],
                    site_settings: vec![
                        SiteSettingConfig {
                            type_name: Some("group_list"),
                            label: Some(
                                self.t(&format!(
                                    "{p}.discourse_post_event_allowed_on_groups.label"
                                )),
                            ),
                            ..setting("discourse_post_event_allowed_on_groups", json!(""))
                        },
                        SiteSettingConfig {
                            type_name: Some("enum"),
                            required: Some(true),
                            choices: Some(json!([
                                choice(&format!("{p}.use_local_event_date.local"), json!(true)),
                                choice(&format!("{p}.use_local_event_date.relative"), json!(false)),
                            ])),
                            label: Some(self.t(&format!("{p}.use_local_event_date.label"))),
                            ..setting("use_local_event_date", json!(false))
                        },
                        SiteSettingConfig {
                            type_name: Some("enum"),
                            required: Some(true),
                            choices: Some(json!([
                                choice(
                                    &format!(
                                        "{p}.sort_categories_by_event_start_date_enabled.event_date"
                                    ),
                                    json!(true)
                                ),
                                choice(
                                    &format!(
                                        "{p}.sort_categories_by_event_start_date_enabled.latest_post"
                                    ),
                                    json!(false)
                                ),
                            ])),
                            label: Some(self.t(&format!(
                                "{p}.sort_categories_by_event_start_date_enabled.label"
                            ))),
                            ..setting("sort_categories_by_event_start_date_enabled", json!(true))
                        },
                        SiteSettingConfig {
                            type_name: Some("bool"),
                            label: Some(self.t(&format!("{p}.sidebar_show_upcoming_events.label"))),
                            ..setting("sidebar_show_upcoming_events", json!(true))
                        },
                    ],
                    ..Schema::default()
                }
            }
            Type::Support => {
                let p = "discourse_solved.category_type";
                let mut custom = vec![
                    FieldConfig {
                        required: Some(true),
                        show_on_create: Some(false),
                        show_on_edit: Some(false),
                        ..field(
                            "enable_accepted_answers",
                            json!(true),
                            "bool",
                            self.t(&format!("{p}.allow_accepted_answers.label")),
                        )
                    },
                    FieldConfig {
                        subtype: Some("duration"),
                        description: Some(self.t(&format!(
                            "{p}.solved_topics_auto_close_duration.description"
                        ))),
                        ..field(
                            "solved_topics_auto_close_hours",
                            json!(48),
                            "integer",
                            self.t(&format!("{p}.solved_topics_auto_close_duration.label")),
                        )
                    },
                    field(
                        "notify_on_staff_accept_solved",
                        json!(true),
                        "bool",
                        self.t(&format!("{p}.notify_on_staff_accept_solved.label")),
                    ),
                ];
                // Hidden when Horizon (theme -2) is the default theme.
                let horizon: bool =
                    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM themes WHERE id = -2)")
                        .fetch_one(&mut *conn)
                        .await?;
                if !horizon {
                    return Err(Unsupported("category types without the Horizon theme").into());
                }
                if self.cx.settings.get("default_theme_id")?.to_i() != -2 {
                    custom.push(field(
                        "empty_box_on_unsolved",
                        json!(true),
                        "bool",
                        self.t(&format!("{p}.empty_box_on_unsolved.label")),
                    ));
                }
                let mut site_texts = Vec::new();
                if self
                    .cx
                    .settings
                    .get("enable_solved_shared_issues")?
                    .truthy()
                {
                    custom.push(FieldConfig {
                        description: Some(self.t(&format!("{p}.enable_shared_issues.description"))),
                        ..field(
                            "enable_shared_issues",
                            json!(true),
                            "bool",
                            self.t(&format!("{p}.enable_shared_issues.label")),
                        )
                    });
                    site_texts.push(SiteTextConfig {
                        key: "js.solved.shared_issue.label",
                        label: self.t(&format!("{p}.shared_issue_label.label")),
                        description: Some(self.t(&format!("{p}.shared_issue_label.description"))),
                        depends_on: Some("enable_shared_issues"),
                    });
                }
                Schema {
                    general: general(self.t("category_types.support.name"), "red_question_mark"),
                    site_settings: vec![
                        setting("show_filter_by_solved_status", json!(true)),
                        setting("prioritize_solved_topics_in_search", json!(false)),
                        setting("show_who_marked_solved", json!(false)),
                    ],
                    category_custom_fields: custom,
                    site_texts,
                    ..Schema::default()
                }
            }
            Type::Ideas => {
                let p = "topic_voting.category_type";
                let labelled = |key: &'static str, default: Value, label: &str| SiteSettingConfig {
                    label: Some(self.t(&format!("{p}.{label}.label"))),
                    ..setting(key, default)
                };
                Schema {
                    general: general(self.t("category_types.ideas.name"), "bulb"),
                    site_settings: vec![
                        labelled("topic_voting_show_who_voted", json!(true), "show_who_voted"),
                        labelled(
                            "topic_voting_show_votes_on_profile",
                            json!(true),
                            "show_votes_on_profile",
                        ),
                        labelled(
                            "topic_voting_enable_vote_limits",
                            json!(true),
                            "enable_vote_limits",
                        ),
                        labelled("topic_voting_tl0_vote_limit", json!(2), "tl0_vote_limit"),
                        labelled("topic_voting_tl1_vote_limit", json!(4), "tl1_vote_limit"),
                        labelled("topic_voting_tl2_vote_limit", json!(6), "tl2_vote_limit"),
                        labelled("topic_voting_tl3_vote_limit", json!(8), "tl3_vote_limit"),
                        labelled("topic_voting_tl4_vote_limit", json!(10), "tl4_vote_limit"),
                        labelled(
                            "topic_voting_alert_votes_left",
                            json!(1),
                            "alert_votes_left",
                        ),
                    ],
                    ..Schema::default()
                }
            }
        }))
    }

    /// `resolved_configuration_schema`
    async fn resolved_schema(&self, conn: &mut PgConnection, t: Type) -> Result<Value, AppError> {
        let Some(schema) = self.schema(conn, t).await? else {
            return Ok(json!({}));
        };
        let general: Vec<Value> = schema
            .general
            .into_iter()
            .map(|(key, default)| {
                json!({
                    "key": key, "default": default, "type": "string", "required": null,
                    "show_on_create": true, "show_on_edit": true,
                })
            })
            .collect();
        let keys: Vec<&str> = schema.site_settings.iter().map(|s| s.key).collect();
        let overridden: Vec<String> =
            sqlx::query_scalar("SELECT name FROM site_settings WHERE name = ANY($1)")
                .bind(&keys)
                .fetch_all(&mut *conn)
                .await?;
        let mut site_settings = Vec::new();
        for s in schema.site_settings {
            let meta = admin_site_settings::setting_meta(self.cx, s.key)
                .ok_or(Unsupported("a category type setting that isn't defined"))?;
            let mut e = Map::new();
            e.insert("key".into(), json!(s.key));
            e.insert("default".into(), s.default);
            e.insert(
                "current".into(),
                serde_json::to_value(self.cx.settings.get(s.key)?).unwrap_or(Value::Null),
            );
            e.insert(
                "overridden".into(),
                json!(overridden.iter().any(|o| o == s.key)),
            );
            e.insert(
                "type".into(),
                json!(s.type_name.map(str::to_string).unwrap_or(meta.type_name)),
            );
            e.insert(
                "label".into(),
                json!(s.label.unwrap_or(meta.humanized_name)),
            );
            e.insert(
                "choices".into(),
                s.choices.or(meta.choices).unwrap_or(Value::Null),
            );
            e.insert("description".into(), json!(meta.description));
            e.insert("required".into(), json!(s.required));
            e.insert("show_on_create".into(), json!(true));
            e.insert("show_on_edit".into(), json!(true));
            if let Some(d) = meta.depends_on {
                e.insert("depends_on".into(), json!(d));
            }
            if let Some(min) = meta.min {
                e.insert("min".into(), min);
            }
            if let Some(max) = meta.max {
                e.insert("max".into(), max);
            }
            site_settings.push(Value::Object(e));
        }
        let fields = |list: Vec<FieldConfig>| -> Vec<Value> {
            list.into_iter()
                .map(|f| {
                    json!({
                        "key": f.key, "default": f.default, "type": f.type_name,
                        "subtype": f.subtype, "label": f.label, "description": f.description,
                        "choices": f.choices, "required": f.required,
                        "show_on_create": f.show_on_create.unwrap_or(true),
                        "show_on_edit": f.show_on_edit.unwrap_or(true),
                    })
                })
                .collect()
        };
        let mut site_texts = Vec::new();
        for text in schema.site_texts {
            let current = self.site_text(conn, text.key).await?;
            let mut e = json!({
                "key": text.key,
                "name": text.key.replace(|c: char| !c.is_alphanumeric() && c != '_', "_"),
                "label": text.label, "description": text.description, "current": current,
                "show_on_create": true, "show_on_edit": true,
            });
            if let Some(d) = text.depends_on {
                e["depends_on"] = json!(d);
            }
            site_texts.push(e);
        }
        Ok(json!({
            "general_category_settings": general,
            "site_settings": site_settings,
            "category_settings": fields(schema.category_settings),
            "category_custom_fields": fields(schema.category_custom_fields),
            "site_texts": site_texts,
        }))
    }

    /// `I18n.t(key)` in the default locale, translation overrides first.
    async fn site_text(&self, conn: &mut PgConnection, key: &str) -> Result<String, AppError> {
        let locale = self.cx.settings.get("default_locale")?.to_s();
        if locale != "en" {
            return Err(Unsupported("site texts in locales other than English").into());
        }
        let overridden: Option<String> = sqlx::query_scalar(
            "SELECT value FROM translation_overrides WHERE locale = 'en' AND translation_key = $1",
        )
        .bind(key)
        .fetch_optional(&mut *conn)
        .await?;
        Ok(overridden.unwrap_or_else(|| self.t(key)))
    }

    /// `metadata(guardian:)`
    async fn metadata(
        &self,
        conn: &mut PgConnection,
        t: Type,
        guardian: Option<&Guardian>,
    ) -> Result<Value, AppError> {
        let id = Self::id(t);
        let name = self
            .cx
            .i18n
            .t(&format!("category_types.{id}.name"))
            .map(str::to_string)
            .ok_or(Unsupported("a category type without a translated name"))?;
        let mut out = Map::new();
        out.insert("id".into(), json!(id));
        out.insert(
            "title".into(),
            json!(
                self.cx
                    .i18n
                    .t(&format!("category_types.{id}.title"))
                    .unwrap_or(&name)
            ),
        );
        out.insert("name".into(), json!(name));
        out.insert(
            "description".into(),
            json!(
                self.cx
                    .i18n
                    .t(&format!("category_types.{id}.description"))
                    .unwrap_or("")
            ),
        );
        out.insert("icon".into(), json!(Self::icon(t)));
        out.insert("available".into(), json!(true));
        out.insert("visible".into(), json!(true));
        out.insert(
            "configuration_schema".into(),
            self.resolved_schema(conn, t).await?,
        );
        if let Some((plugin, enabled)) = self.plugin(t)? {
            out.insert("required_plugin".into(), json!(plugin));
            // available_for?: a disabled plugin is for admins to enable.
            let can_enable = !(!enabled && guardian.is_some_and(|g| !g.is_admin()));
            out.insert("can_enable_plugin".into(), json!(can_enable));
            if !can_enable {
                let admin: Option<String> = sqlx::query_scalar(
                    "SELECT username FROM users u WHERE admin AND active AND id > 0 \
                       AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = u.id) \
                     ORDER BY last_seen_at DESC LIMIT 1",
                )
                .fetch_optional(&mut *conn)
                .await?;
                out.insert("contact_admin_username".into(), json!(admin));
            }
        }
        Ok(Value::Object(out))
    }

    /// `Categories::TypeRegistry.list(only_visible: true, guardian:)`
    pub async fn list(
        &self,
        conn: &mut PgConnection,
        guardian: &Guardian,
    ) -> Result<Value, AppError> {
        let mut out = Vec::new();
        for t in TYPES {
            out.push(self.metadata(conn, t, Some(guardian)).await?);
        }
        Ok(Value::Array(out))
    }

    /// `Category#category_types`: the metadata of each type the category
    /// matches, by id.
    pub async fn for_category(
        &self,
        conn: &mut PgConnection,
        category_id: i32,
    ) -> Result<Value, AppError> {
        let mut out = Map::new();
        for t in TYPES {
            if self.matches(conn, t, category_id).await? {
                out.insert(Self::id(t).into(), self.metadata(conn, t, None).await?);
            }
        }
        Ok(Value::Object(out))
    }

    /// `category_matches?`
    async fn matches(
        &self,
        conn: &mut PgConnection,
        t: Type,
        category_id: i32,
    ) -> Result<bool, AppError> {
        Ok(match t {
            Type::Discussion => true,
            // SiteSetting.events_calendar_categories_map
            Type::Events => self
                .cx
                .settings
                .get("events_calendar_categories")?
                .to_s()
                .split('|')
                .any(|id| id.trim().parse::<i32>() == Ok(category_id)),
            Type::Support => {
                sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM category_custom_fields \
                 WHERE category_id = $1 AND name = 'enable_accepted_answers' AND value = 'true')",
                )
                .bind(category_id)
                .fetch_one(&mut *conn)
                .await?
            }
            Type::Ideas => {
                crate::plugins::topic_voting::enabled(self.cx.settings)?
                    && crate::plugins::topic_voting::category_votes(conn, category_id).await?
            }
        })
    }
}
