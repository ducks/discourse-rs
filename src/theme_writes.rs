//! Theme writes: `Theme#set_field` and the theme's `after_save` (fields
//! saved, dependent fields invalidated, every field baked), the theme
//! settings and translations managers, `Theme#destroy` with its
//! dependents, and the staff logs with `log_theme_change`'s
//! `strip_duplicates`.
//!
//! Baking follows `ThemeField#ensure_baked!`: html fields through
//! Nokogiri's HTML5 fragment, translation fields into their
//! `javascript_caches` row, settings YAML validated, migrations and tests
//! marked "baked". The scss fields' value_baked is the SHA1 of the CSS a
//! dart-sass compile writes with the compile's temporary directory in its
//! source map, a digest no two compiles share; a rebake of an unchanged
//! scss field (a theme setting changed) writes a digest of its own and
//! keeps the field's error. Upload fields (theme_upload_var and
//! screenshots) keep their upload referenced, a `.js` upload in the field's
//! javascript cache and the `icons-sprite` upload in theme_svg_sprites.
//! Refused: new or changed scss values (their compile errors), extra_js
//! (ThemeJavascriptCompiler), inline scripts in html fields
//! (AssetProcessor), migrations, svg sprites that are not plain well-formed
//! XML (libxml2's error text).

use serde_json::{Map, Value, json};
use serde_yaml_ng::Value as Yaml;
use sha1::{Digest, Sha1};
use sqlx::PgConnection;

use crate::themes::{self, Theme};
use crate::{AppError, Unsupported};

/// `ThemeField::CSP_NONCE_PLACEHOLDER`
pub const CSP_NONCE_PLACEHOLDER: &str =
    "__CSP__NONCE__PLACEHOLDER__f72bff1b1768168a34ee092ce759f192__";

const HTML_FIELDS: [&str; 6] = [
    "body_tag",
    "head_tag",
    "header",
    "footer",
    "after_header",
    "embedded_header",
];
const SCSS_FIELDS: [&str; 3] = ["scss", "embedded_scss", "color_definitions"];
/// `ThemeField.basic_targets`: common, desktop, mobile.
const BASIC_TARGETS: [i32; 3] = [0, 1, 2];
const SETTINGS: i32 = 3;
const TRANSLATIONS: i32 = 4;
const EXTRA_SCSS: i32 = 5;
const EXTRA_JS: i32 = 6;
const TESTS_JS: i32 = 7;
const MIGRATIONS: i32 = 8;
/// `ThemeField.types[:theme_upload_var]`, the one `theme_var_type_ids`.
pub const UPLOAD_VAR: i32 = 2;
/// `ThemeField.types[:theme_screenshot_upload_var]`
pub const SCREENSHOT_UPLOAD_VAR: i32 = 7;
/// `SvgSprite.theme_sprite_variable_name`
const SPRITE_FIELD: &str = "icons-sprite";
/// `SvgSprite::MAX_THEME_SPRITE_SIZE`
const MAX_THEME_SPRITE_SIZE: usize = 1024 * 1024;

/// What baking needs beside the connection.
pub struct Env<'a> {
    pub cx: &'a themes::Ctx<'a>,
    /// `Theme.compiler_version`: a digest of the reference's environment
    /// (asset compiler, CDN, hostname) the port cannot derive, read from a
    /// baked field.
    pub compiler_version: String,
    /// `Discourse.current_hostname`, in the translation caches' urls.
    pub hostname: String,
    /// `Discourse.store`, for the upload fields' contents.
    pub store: crate::file_store::FileStore,
}

impl<'a> Env<'a> {
    pub async fn load(
        conn: &mut PgConnection,
        cx: &'a themes::Ctx<'a>,
    ) -> Result<Env<'a>, AppError> {
        let compiler_version: Option<String> = sqlx::query_scalar(
            "SELECT compiler_version FROM theme_fields WHERE compiler_version <> '0' \
             GROUP BY compiler_version ORDER BY count(*) DESC LIMIT 1",
        )
        .fetch_optional(&mut *conn)
        .await?;
        let compiler_version = compiler_version.ok_or(Unsupported(
            "Theme.compiler_version without a baked theme field to read it from",
        ))?;
        let hostname = cx.urls.current_hostname()?;
        let store = crate::file_store::FileStore::for_site(cx.urls.config, cx.settings)?;
        Ok(Env {
            cx,
            compiler_version,
            hostname,
            store,
        })
    }

    /// `upload.content`: the stored file of an upload, None when the
    /// upload or its file is gone.
    async fn upload_content(
        &self,
        conn: &mut PgConnection,
        upload_id: i32,
    ) -> Result<Option<Vec<u8>>, AppError> {
        let url: Option<String> = sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
            .bind(upload_id)
            .fetch_optional(&mut *conn)
            .await?;
        match url {
            Some(url) => Ok(self.store.read(&url).await?),
            None => Ok(None),
        }
    }
}

fn basic_html(target_id: i32, name: &str) -> bool {
    BASIC_TARGETS.contains(&target_id) && HTML_FIELDS.contains(&name)
}

fn basic_scss(target_id: i32, name: &str) -> bool {
    BASIC_TARGETS.contains(&target_id) && SCSS_FIELDS.contains(&name)
}

/// One field as the theme sets it (`set_field`'s keywords).
#[derive(Debug, Clone)]
pub struct FieldInput {
    pub target: String,
    pub name: String,
    pub value: Option<String>,
    pub type_id: Option<i32>,
    pub upload_id: Option<i32>,
}

impl FieldInput {
    /// A `theme_fields` param entry (`name target value upload_id type_id`).
    pub fn from_param(v: &Value) -> FieldInput {
        let s = |k: &str| v.get(k).and_then(crate::params::scalar);
        let int = |k: &str| {
            s(k).filter(|x| !crate::ruby::is_blank(x))
                .map(|x| crate::ruby::to_i(&x) as i32)
        };
        FieldInput {
            target: s("target").unwrap_or_default(),
            name: s("name").unwrap_or_default(),
            value: s("value"),
            type_id: int("type_id"),
            upload_id: int("upload_id"),
        }
    }
}

/// A field `set_field` leaves in `changed_fields` for the theme's save.
#[derive(Debug, Clone)]
pub enum FieldChange {
    Insert {
        target_id: i32,
        name: String,
        type_id: i32,
        value: String,
        upload_id: Option<i32>,
    },
    Update {
        id: i32,
        target_id: i32,
        name: String,
        type_id: i32,
        value: String,
        upload_id: Option<i32>,
        /// `will_save_change_to_upload_id?`
        upload_changed: bool,
    },
    Destroy {
        id: i32,
        target_id: i32,
        name: String,
        type_id: i32,
    },
}

impl FieldChange {
    fn target_id(&self) -> i32 {
        match self {
            FieldChange::Insert { target_id, .. }
            | FieldChange::Update { target_id, .. }
            | FieldChange::Destroy { target_id, .. } => *target_id,
        }
    }

    fn name(&self) -> &str {
        match self {
            FieldChange::Insert { name, .. }
            | FieldChange::Update { name, .. }
            | FieldChange::Destroy { name, .. } => name,
        }
    }

    fn type_id(&self) -> i32 {
        match self {
            FieldChange::Insert { type_id, .. }
            | FieldChange::Update { type_id, .. }
            | FieldChange::Destroy { type_id, .. } => *type_id,
        }
    }

    /// `(target_id, type_id, name)`: what identifies a theme's field.
    pub fn key(&self) -> (i32, i32, String) {
        (self.target_id(), self.type_id(), self.name().to_string())
    }
}

/// `validate_theme_fields`: the errors of the fields a save writes, as
/// ThemeField's validations word them (`theme_var_type_ids` names).
pub fn field_errors(changes: &[FieldChange]) -> Vec<String> {
    static NAME: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\A[a-zA-Z_][a-zA-Z0-9_-]*\z").expect("valid regex")
    });
    changes
        .iter()
        .filter(|c| !matches!(c, FieldChange::Destroy { .. }))
        .filter(|c| c.type_id() == UPLOAD_VAR && !NAME.is_match(c.name()))
        .map(|_| "Name is invalid".to_string())
        .collect()
}

/// `ThemeField.guess_type`
fn guess_type(name: &str, target: &str) -> Option<i32> {
    let basic = matches!(target, "common" | "desktop" | "mobile");
    if basic && HTML_FIELDS.contains(&name) {
        Some(0)
    } else if (basic && SCSS_FIELDS.contains(&name)) || target == "extra_scss" {
        Some(1)
    } else if matches!(target, "migrations" | "extra_js") {
        Some(6)
    } else if matches!(target, "settings" | "translations") {
        Some(5)
    } else {
        None
    }
}

/// `Theme#set_field`: the change the save will make, if any. Returns the
/// key of the field it leaves (`set_field`'s return value), None when it
/// leaves none.
pub async fn set_field(
    conn: &mut PgConnection,
    theme_id: i32,
    input: &FieldInput,
    changes: &mut Vec<FieldChange>,
) -> Result<Option<(i32, i32, String)>, AppError> {
    let target_id = themes::TARGETS
        .iter()
        .find(|(_, n)| *n == input.target)
        .map(|(id, _)| *id)
        .ok_or(Unsupported(
            "a theme field with an unknown target (InvalidFieldTargetError)",
        ))?;
    let type_id = match input.type_id {
        Some(t) => t,
        None => guess_type(&input.name, &input.target)
            .ok_or(Unsupported("a theme field whose type cannot be guessed"))?,
    };
    let value = input.value.clone().unwrap_or_default();
    let existing: Option<(i32, String, Option<i32>)> = sqlx::query_as(
        "SELECT id, value, upload_id FROM theme_fields WHERE theme_id = $1 AND name = $2 \
         AND target_id = $3 AND type_id = $4 LIMIT 1",
    )
    .bind(theme_id)
    .bind(&input.name)
    .bind(target_id)
    .bind(type_id)
    .fetch_optional(&mut *conn)
    .await?;
    let key = (target_id, type_id, input.name.clone());
    let (change, kept) = match existing {
        Some((id, _, _)) if crate::ruby::is_blank(&value) && input.upload_id.is_none() => (
            Some(FieldChange::Destroy {
                id,
                target_id,
                name: input.name.clone(),
                type_id,
            }),
            true,
        ),
        Some((id, current, current_upload))
            if current != value || current_upload != input.upload_id =>
        {
            (
                Some(FieldChange::Update {
                    id,
                    target_id,
                    name: input.name.clone(),
                    type_id,
                    value,
                    upload_id: input.upload_id,
                    upload_changed: current_upload != input.upload_id,
                }),
                true,
            )
        }
        Some(_) => (None, true),
        None if !crate::ruby::is_blank(&value) || input.upload_id.is_some() => (
            Some(FieldChange::Insert {
                target_id,
                name: input.name.clone(),
                type_id,
                value,
                upload_id: input.upload_id,
            }),
            true,
        ),
        None => (None, false),
    };
    if let Some(change) = change {
        refuse_unported(conn, theme_id, &change).await?;
        changes.push(change);
    }
    Ok(kept.then_some(key))
}

/// The fields whose save the port cannot bake as Rails does.
async fn refuse_unported(
    conn: &mut PgConnection,
    theme_id: i32,
    change: &FieldChange,
) -> Result<(), AppError> {
    let (target_id, name) = (change.target_id(), change.name());
    if matches!(change, FieldChange::Destroy { .. }) {
        return Ok(());
    }
    if basic_scss(target_id, name) {
        return Err(Unsupported("theme scss fields (stylesheet compile)").into());
    }
    if target_id == EXTRA_SCSS {
        let has_scss: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM theme_fields WHERE theme_id = $1 \
             AND target_id = ANY($2) AND name = ANY($3))",
        )
        .bind(theme_id)
        .bind(&BASIC_TARGETS[..])
        .bind(&SCSS_FIELDS[..])
        .fetch_one(&mut *conn)
        .await?;
        if has_scss {
            return Err(Unsupported("theme scss fields (stylesheet compile)").into());
        }
    }
    if target_id == EXTRA_JS {
        return Err(Unsupported("theme javascript (ThemeJavascriptCompiler)").into());
    }
    if target_id == MIGRATIONS {
        return Err(Unsupported("theme settings migrations").into());
    }
    if target_id == TRANSLATIONS && name != "en" {
        return Err(Unsupported("theme translations in other locales").into());
    }
    if let FieldChange::Insert { value, .. } | FieldChange::Update { value, .. } = change
        && value.len() > 1024 * 1024
    {
        return Err(Unsupported("a theme field over 1 MiB (its validation error)").into());
    }
    Ok(())
}

/// `ThemeField#dependent_fields` of a saved field, invalidated
/// (`update_all(value_baked: nil)`).
async fn invalidate_dependents(
    conn: &mut PgConnection,
    theme_id: i32,
    target_id: i32,
    name: &str,
) -> Result<(), sqlx::Error> {
    let names: Vec<&str> = if target_id == EXTRA_SCSS {
        SCSS_FIELDS.to_vec()
    } else if target_id == SETTINGS {
        SCSS_FIELDS
            .iter()
            .chain(HTML_FIELDS.iter())
            .copied()
            .collect()
    } else if target_id == TRANSLATIONS && name == "en" {
        sqlx::query(
            "UPDATE theme_fields SET value_baked = NULL WHERE theme_id = $1 AND target_id = $2 \
             AND name <> 'en'",
        )
        .bind(theme_id)
        .bind(TRANSLATIONS)
        .execute(&mut *conn)
        .await?;
        return Ok(());
    } else {
        return Ok(());
    };
    sqlx::query(
        "UPDATE theme_fields SET value_baked = NULL WHERE theme_id = $1 AND target_id = ANY($2) \
         AND name = ANY($3)",
    )
    .bind(theme_id)
    .bind(&BASIC_TARGETS[..])
    .bind(&names)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `ThemeField#invalidate_baked!`
pub async fn invalidate_baked(conn: &mut PgConnection, field_id: i32) -> Result<(), sqlx::Error> {
    let row: Option<(i32, i32, String)> = sqlx::query_as(
        "UPDATE theme_fields SET value_baked = NULL WHERE id = $1 RETURNING theme_id, target_id, name",
    )
    .bind(field_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((theme_id, target_id, name)) = row {
        invalidate_dependents(conn, theme_id, target_id, &name).await?;
    }
    Ok(())
}

/// The theme's `after_save`, with the theme row already written:
/// `changed_fields` saved or destroyed, the modifier set saved, html fields
/// invalidated on a rename, then the settings field and every field baked.
pub async fn after_save(
    conn: &mut PgConnection,
    env: &Env<'_>,
    theme_id: i32,
    changes: &[FieldChange],
    renamed: bool,
) -> Result<(), AppError> {
    let extra_js_changed = changes.iter().any(|c| c.target_id() == EXTRA_JS);
    for change in changes {
        let (field_id, upload_id, upload_changed) = match change {
            FieldChange::Insert {
                target_id,
                name,
                type_id,
                value,
                upload_id,
            } => {
                let id: i32 = sqlx::query_scalar(
                    "INSERT INTO theme_fields (theme_id, target_id, name, value, type_id, \
                     upload_id, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, now(), now()) \
                     RETURNING id",
                )
                .bind(theme_id)
                .bind(target_id)
                .bind(name)
                .bind(value)
                .bind(type_id)
                .bind(upload_id)
                .fetch_one(&mut *conn)
                .await?;
                (id, *upload_id, upload_id.is_some())
            }
            FieldChange::Update {
                id,
                value,
                upload_id,
                upload_changed,
                ..
            } => {
                // before_save: a changed value or upload drops the baked one.
                sqlx::query(
                    "UPDATE theme_fields SET value = $2, upload_id = $3, value_baked = NULL, \
                     updated_at = now() WHERE id = $1",
                )
                .bind(id)
                .bind(value)
                .bind(upload_id)
                .execute(&mut *conn)
                .await?;
                (*id, *upload_id, *upload_changed)
            }
            FieldChange::Destroy {
                id, type_id, name, ..
            } => {
                destroy_field(conn, *id).await?;
                // after_destroy: the theme's sprite goes with its field.
                if *type_id == UPLOAD_VAR && name == SPRITE_FIELD {
                    sqlx::query("DELETE FROM theme_svg_sprites WHERE theme_id = $1")
                        .bind(theme_id)
                        .execute(&mut *conn)
                        .await?;
                }
                continue;
            }
        };
        upload_callbacks(
            conn,
            env,
            field_id,
            change.type_id(),
            upload_id,
            upload_changed,
        )
        .await?;
        // after_save: dependent_fields invalidated.
        invalidate_dependents(conn, theme_id, change.target_id(), change.name()).await?;
        if let Some(upload_id) = upload_id
            && change.type_id() == UPLOAD_VAR
            && change.name() == SPRITE_FIELD
        {
            upsert_svg_sprite(conn, env, theme_id, upload_id).await?;
        }
    }
    // theme_modifier_set.save! (built for a new theme).
    sqlx::query(
        "INSERT INTO theme_modifier_sets (theme_id) SELECT $1 \
         WHERE NOT EXISTS (SELECT 1 FROM theme_modifier_sets WHERE theme_id = $1)",
    )
    .bind(theme_id)
    .execute(&mut *conn)
    .await?;
    if renamed {
        sqlx::query(
            "UPDATE theme_fields SET value_baked = NULL WHERE theme_id = $1 AND target_id = ANY($2) \
             AND name = ANY($3)",
        )
        .bind(theme_id)
        .bind(&BASIC_TARGETS[..])
        .bind(&HTML_FIELDS[..])
        .execute(&mut *conn)
        .await?;
    }
    // settings_field&.ensure_baked!, then every field.
    let settings_field: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM theme_fields WHERE theme_id = $1 AND target_id = $2 AND name = 'yaml' \
         ORDER BY id LIMIT 1",
    )
    .bind(theme_id)
    .bind(SETTINGS)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = settings_field {
        ensure_baked(conn, env, id).await?;
    }
    let stale_extra_js: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM theme_fields WHERE theme_id = $1 AND target_id = $2 \
         AND compiler_version <> $3)",
    )
    .bind(theme_id)
    .bind(EXTRA_JS)
    .bind(&env.compiler_version)
    .fetch_one(&mut *conn)
    .await?;
    let ids: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM theme_fields WHERE theme_id = $1 ORDER BY id")
            .bind(theme_id)
            .fetch_all(&mut *conn)
            .await?;
    for id in ids {
        ensure_baked(conn, env, id).await?;
    }
    if extra_js_changed || stale_extra_js {
        return Err(Unsupported("theme javascript (ThemeJavascriptCompiler)").into());
    }
    Ok(())
}

/// `ThemeField#destroy` with its dependents (javascript caches, upload
/// reference, settings migration).
async fn destroy_field(conn: &mut PgConnection, id: i32) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM javascript_caches WHERE theme_field_id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "DELETE FROM upload_references WHERE target_type = 'ThemeField' AND target_id = $1",
    )
    .bind(id)
    .execute(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM theme_settings_migrations WHERE theme_field_id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM theme_fields WHERE id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// ThemeField's save callbacks for its upload: before_save's javascript
/// cache of a `.js` upload (saved with the field), then after_save's
/// `UploadReference.ensure_exist!` for an upload field whose upload changed.
async fn upload_callbacks(
    conn: &mut PgConnection,
    env: &Env<'_>,
    field_id: i32,
    type_id: i32,
    upload_id: Option<i32>,
    upload_changed: bool,
) -> Result<(), AppError> {
    if let Some(upload_id) = upload_id {
        let extension: Option<Option<String>> =
            sqlx::query_scalar("SELECT extension FROM uploads WHERE id = $1")
                .bind(upload_id)
                .fetch_optional(&mut *conn)
                .await?;
        if extension.flatten().as_deref() == Some("js") {
            let cache: Option<i64> = sqlx::query_scalar(
                "SELECT id FROM javascript_caches WHERE theme_field_id = $1 AND name IS NULL",
            )
            .bind(field_id)
            .fetch_optional(&mut *conn)
            .await?;
            if upload_changed || cache.is_none() {
                let bytes = env
                    .upload_content(conn, upload_id)
                    .await?
                    .ok_or(Unsupported(
                        "a theme's javascript upload whose file is gone",
                    ))?;
                let content = String::from_utf8(bytes)
                    .map_err(|_| Unsupported("a theme's javascript upload that is not UTF-8"))?;
                let digest = javascript_digest(env, &content);
                match cache {
                    Some(id) => {
                        sqlx::query(
                            "UPDATE javascript_caches SET content = $2, digest = $3, \
                             updated_at = now() WHERE id = $1 AND content IS DISTINCT FROM $2",
                        )
                        .bind(id)
                        .bind(&content)
                        .bind(&digest)
                        .execute(&mut *conn)
                        .await?;
                    }
                    None => {
                        sqlx::query(
                            "INSERT INTO javascript_caches (theme_field_id, content, digest, \
                             created_at, updated_at) VALUES ($1, $2, $3, now(), now())",
                        )
                        .bind(field_id)
                        .bind(&content)
                        .bind(&digest)
                        .execute(&mut *conn)
                        .await?;
                    }
                }
            }
        }
    }
    if matches!(type_id, UPLOAD_VAR | SCREENSHOT_UPLOAD_VAR) && upload_changed {
        crate::upload_references::ensure_exist(
            conn,
            "ThemeField",
            i64::from(field_id),
            upload_id.map(i64::from),
        )
        .await?;
    }
    Ok(())
}

/// `ThemeField#upsert_svg_sprite!`: the sprite's content kept per theme
/// (none when its file can't be read or is over the limit).
async fn upsert_svg_sprite(
    conn: &mut PgConnection,
    env: &Env<'_>,
    theme_id: i32,
    upload_id: i32,
) -> Result<(), AppError> {
    let Some(content) = env.upload_content(conn, upload_id).await? else {
        return Ok(());
    };
    if content.len() > MAX_THEME_SPRITE_SIZE {
        return Ok(());
    }
    // upsert: CURRENT_TIMESTAMP for both timestamps on insert, the
    // updated one on conflict.
    sqlx::query(
        "INSERT INTO theme_svg_sprites (theme_id, upload_id, sprite, created_at, updated_at) \
         VALUES ($1, $2, $3, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP) \
         ON CONFLICT (theme_id) DO UPDATE SET upload_id = excluded.upload_id, \
         sprite = excluded.sprite, updated_at = (CASE WHEN theme_svg_sprites.upload_id \
         IS DISTINCT FROM excluded.upload_id OR theme_svg_sprites.sprite IS DISTINCT FROM \
         excluded.sprite THEN CURRENT_TIMESTAMP ELSE theme_svg_sprites.updated_at END)",
    )
    .bind(theme_id)
    .bind(upload_id)
    .bind(&content)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `ThemeField#validate_svg_sprite_xml`: None when the sprite parses.
async fn validate_svg_sprite(
    conn: &mut PgConnection,
    env: &Env<'_>,
    name: &str,
    upload_id: Option<i32>,
) -> Result<Option<String>, AppError> {
    let exists = match upload_id {
        Some(id) => {
            sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM uploads WHERE id = $1)")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?
        }
        None => false,
    };
    let Some(upload_id) = upload_id.filter(|_| exists) else {
        return Ok(Some(format!(
            "Error with {name}: No upload found for the provided upload_id"
        )));
    };
    let content = env
        .upload_content(conn, upload_id)
        .await?
        .ok_or(Unsupported(
            "a theme svg sprite whose file is gone (File.read's error text)",
        ))?;
    if content.len() > MAX_THEME_SPRITE_SIZE {
        return Ok(Some(format!(
            "Error with {name}: Icon sprite file is too large"
        )));
    }
    if !crate::theme_import::well_formed_xml(&content) {
        return Err(Unsupported(
            "a theme svg sprite that is not plain well-formed XML (libxml2's error text)",
        )
        .into());
    }
    Ok(None)
}

#[derive(sqlx::FromRow)]
struct BakeRow {
    theme_id: i32,
    target_id: i32,
    name: String,
    type_id: i32,
    value: String,
    value_baked: Option<String>,
    compiler_version: String,
    error: Option<String>,
    upload_id: Option<i32>,
}

/// `ThemeField#ensure_baked!`
pub async fn ensure_baked(
    conn: &mut PgConnection,
    env: &Env<'_>,
    field_id: i32,
) -> Result<(), AppError> {
    let Some(f): Option<BakeRow> = sqlx::query_as(
        "SELECT theme_id, target_id, name, type_id, value, value_baked, compiler_version, error, \
         upload_id FROM theme_fields WHERE id = $1",
    )
    .bind(field_id)
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(());
    };
    if f.value_baked.is_some() && f.compiler_version == env.compiler_version {
        return Ok(());
    }
    let cv = env.compiler_version.clone();
    let (baked, error, version): (Option<String>, Option<String>, String) =
        if basic_html(f.target_id, &f.name) {
            let (html, errors) = process_html(env, &f.value)?;
            (Some(html), errors, cv)
        } else if f.target_id == TRANSLATIONS {
            let html = process_translation(conn, env, field_id, f.theme_id, &f.name).await?;
            (Some(html), None, cv)
        } else if f.target_id == EXTRA_JS || f.target_id == TESTS_JS {
            (Some("baked".into()), None, cv)
        } else if basic_scss(f.target_id, &f.name) {
            // ensure_scss_compiles!: the compiled CSS's digest, which no
            // two compiles share, and the value's own checks.
            let error = if optimized_link(&f.value) {
                Some(t(env, "themes.errors.optimized_link"))
            } else if ember_selector(&f.value) {
                Some(t(env, "themes.ember_selector_error"))
            } else {
                f.error.clone()
            };
            let digest = format!(
                "{:x}",
                Sha1::digest(
                    format!(
                        "{}|{}|{}",
                        f.value,
                        field_id,
                        chrono::Utc::now().timestamp_micros()
                    )
                    .as_bytes(),
                )
            );
            (Some(digest), error, cv)
        } else if f.target_id == SETTINGS {
            let errors = if f.name == "yaml" {
                validate_yaml(env, &f.value)
            } else {
                f.error.clone()
            };
            (Some("baked".into()), errors, cv)
        } else if f.type_id == UPLOAD_VAR && f.name == SPRITE_FIELD {
            // svg_sprite_field?: the sprite's XML validated from its upload.
            let error = validate_svg_sprite(conn, env, &f.name, f.upload_id).await?;
            (Some("baked".into()), error, cv)
        } else if f.target_id == MIGRATIONS {
            (Some("baked".into()), f.error.clone(), cv)
        } else {
            return Ok(());
        };
    if baked != f.value_baked || version != f.compiler_version || error != f.error {
        sqlx::query(
            "UPDATE theme_fields SET value_baked = $2, compiler_version = $3, error = $4 WHERE id = $1",
        )
        .bind(field_id)
        .bind(&baked)
        .bind(&version)
        .bind(&error)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

fn t(env: &Env<'_>, key: &str) -> String {
    env.cx
        .i18n
        .t(key)
        .map(str::to_string)
        .unwrap_or_else(|| format!("Translation missing: en.{key}"))
}

/// `OptimizedImage::URL_REGEX` in a field's text.
fn optimized_link(text: &str) -> bool {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"/optimized/\dX[/.\w]*/[a-zA-Z0-9]+[.\w]*").expect("valid regex")
    });
    RE.is_match(text)
}

/// `contains_ember_css_selector?`
fn ember_selector(text: &str) -> bool {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"#ember\d+|[.]ember-view").expect("valid regex")
    });
    RE.is_match(text)
}

/// `ThemeField#process_html`: the fragment as Nokogiri's HTML5 parser
/// writes it back, scripts given the CSP nonce placeholder.
fn process_html(env: &Env<'_>, html: &str) -> Result<(String, Option<String>), AppError> {
    let mut inline = false;
    let out = crate::sanitize::html5_fragment_edit(html, |node| {
        if crate::sanitize::element_name(node).as_deref() != Some("script") {
            return;
        }
        let src = crate::sanitize::attr(node, "src").filter(|s| !crate::ruby::is_blank(s));
        let kind = crate::sanitize::attr(node, "type").filter(|s| !crate::ruby::is_blank(s));
        let javascript = match (&src, &kind) {
            (Some(_), _) => false,
            (None, Some(t)) => matches!(
                t.to_lowercase().as_str(),
                "text/javascript" | "application/javascript" | "application/ecmascript"
            ),
            (None, None) => true,
        };
        if javascript {
            inline = true;
        }
        crate::sanitize::set_attr(node, "nonce", CSP_NONCE_PLACEHOLDER);
    })
    .map_err(|_| Unsupported("theme html that does not parse"))?;
    if inline {
        return Err(Unsupported("inline scripts in theme html (AssetProcessor)").into());
    }
    let error = optimized_link(html).then(|| t(env, "themes.errors.optimized_link"));
    Ok((out, error))
}

/// `ThemeTranslationParser#load` for a field: `{locale => tree}`, the
/// internal `theme_metadata` left out.
fn translation_tree(name: &str, src: &str) -> Result<Map<String, Value>, AppError> {
    if crate::ruby::is_blank(src) {
        return Ok(Map::new());
    }
    let parsed: Yaml = serde_yaml_ng::from_str(src)
        .map_err(|_| Unsupported("theme translations that are not valid YAML (InvalidYaml)"))?;
    let root = parsed
        .as_mapping()
        .filter(|m| m.len() == 1 && m.keys().next().and_then(Yaml::as_str) == Some(name))
        .ok_or(Unsupported(
            "theme translations without their locale at the top (InvalidYaml)",
        ))?;
    let mut tree = match root.values().next() {
        Some(Yaml::Null) => Map::new(),
        Some(Yaml::Mapping(m)) => yaml_tree(m).ok_or(Unsupported(
            "theme translations that are not strings (InvalidYaml)",
        ))?,
        _ => {
            return Err(Unsupported("theme translations that are not a hash (InvalidYaml)").into());
        }
    };
    tree.remove("theme_metadata");
    let mut out = Map::new();
    out.insert(name.to_string(), Value::Object(tree));
    Ok(out)
}

fn yaml_tree(m: &serde_yaml_ng::Mapping) -> Option<Map<String, Value>> {
    let mut out = Map::new();
    for (k, v) in m {
        let key = match k {
            Yaml::String(s) => s.clone(),
            Yaml::Number(n) => n.to_string(),
            _ => return None,
        };
        let value = match v {
            Yaml::String(s) => Value::String(s.clone()),
            Yaml::Mapping(m) => Value::Object(yaml_tree(m)?),
            _ => return None,
        };
        out.insert(key, value);
    }
    Some(out)
}

/// Hash#deep_merge!
fn deep_merge(target: &mut Map<String, Value>, other: Map<String, Value>) {
    for (k, v) in other {
        match (target.get_mut(&k), v) {
            (Some(Value::Object(t)), Value::Object(o)) => deep_merge(t, o),
            (_, v) => {
                target.insert(k, v);
            }
        }
    }
}

/// `ThemeField#process_translation`: the locale's data with the theme's
/// overrides written to the field's javascript cache, and the script tag
/// that loads it.
async fn process_translation(
    conn: &mut PgConnection,
    env: &Env<'_>,
    field_id: i32,
    theme_id: i32,
    name: &str,
) -> Result<String, AppError> {
    // translation_data: the locale's fallbacks (I18n.fallbacks), each
    // field's tree merged in turn, then the overrides deep merged.
    let mut fallbacks = vec![name.to_string()];
    if name != "en" {
        fallbacks.push("en".into());
    }
    let mut data = Map::new();
    for locale in &fallbacks {
        let src: Option<String> = sqlx::query_scalar(
            "SELECT value FROM theme_fields WHERE theme_id = $1 AND target_id = $2 AND name = $3 \
             ORDER BY id LIMIT 1",
        )
        .bind(theme_id)
        .bind(TRANSLATIONS)
        .bind(locale)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(src) = src {
            for (k, v) in translation_tree(locale, &src)? {
                data.insert(k, v);
            }
        }
    }
    let overrides: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT locale, translation_key, value FROM theme_translation_overrides \
         WHERE theme_id = $1 ORDER BY id",
    )
    .bind(theme_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut hash = Map::new();
    for (locale, key, value) in overrides {
        let mut path: Vec<String> = vec![locale];
        path.extend(key.split('.').map(str::to_string));
        let mut cursor = &mut hash;
        for k in &path[..path.len() - 1] {
            let entry = cursor
                .entry(k.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            cursor = entry.as_object_mut().expect("an object");
        }
        cursor.insert(path[path.len() - 1].clone(), Value::String(value));
    }
    deep_merge(&mut data, hash);
    let content = format!(
        "/* Translation data for theme {theme_id} ({name})*/\n\
         const localeData = window._discourse_locale_data ??= {{}};\n\
         localeData.theme ??= {{}};\n\
         localeData.theme[{theme_id}] = {};\n",
        crate::web_hooks::to_json(&Value::Object(data))
    );
    let digest = javascript_digest(env, &content);
    let existing: Option<(i64, String)> = sqlx::query_as(
        "SELECT id, content FROM javascript_caches WHERE theme_field_id = $1 AND name IS NULL",
    )
    .bind(field_id)
    .fetch_optional(&mut *conn)
    .await?;
    match existing {
        Some((id, current)) => {
            if current != content {
                sqlx::query(
                    "UPDATE javascript_caches SET content = $2, digest = $3, updated_at = now() \
                     WHERE id = $1",
                )
                .bind(id)
                .bind(&content)
                .bind(&digest)
                .execute(&mut *conn)
                .await?;
            }
        }
        None => {
            sqlx::query(
                "INSERT INTO javascript_caches (theme_field_id, content, digest, created_at, \
                 updated_at) VALUES ($1, $2, $3, now(), now())",
            )
            .bind(field_id)
            .bind(&content)
            .bind(&digest)
            .execute(&mut *conn)
            .await?;
        }
    }
    let url = format!(
        "{}{}/theme-javascripts/{digest}.js?__ws={}",
        env.cx.urls.config.globals.cdn_url().unwrap_or(""),
        env.cx.urls.config.globals.relative_url_root(),
        env.hostname
    );
    Ok(format!(
        "<script type=\"module\" src=\"{url}\" data-theme-id=\"{theme_id}\" nonce=\"{CSP_NONCE_PLACEHOLDER}\"></script>\n"
    ))
}

/// `JavascriptCache#update_digest`
fn javascript_digest(env: &Env<'_>, content: &str) -> String {
    let salt = env
        .cx
        .urls
        .config
        .globals
        .get("asset_url_salt")
        .unwrap_or("");
    format!(
        "{:x}",
        Sha1::digest(format!("{content}||{salt}").as_bytes())
    )
}

/// `ThemeField#validate_yaml!`: the settings' errors, joined.
fn validate_yaml(env: &Env<'_>, src: &str) -> Option<String> {
    if crate::ruby::is_blank(src) {
        return None;
    }
    let tr = |key: &str, args: &[(&str, &str)]| {
        env.cx
            .i18n
            .t_with(&format!("themes.settings_errors.{key}"), args)
            .unwrap_or_default()
    };
    let parsed: Yaml = match serde_yaml_ng::from_str(src) {
        Ok(p) => p,
        Err(_) => return Some("theme settings YAML that does not parse".into()),
    };
    let Some(map) = parsed.as_mapping() else {
        return Some(tr("invalid_yaml", &[]));
    };
    let mut errors = Vec::new();
    for (k, v) in map {
        let name = k.as_str().unwrap_or_default().to_string();
        let (default, kind) = match v {
            Yaml::Mapping(h) => {
                let default = h.get("default").cloned().unwrap_or(Yaml::Null);
                let kind = match h.get("type") {
                    Some(t) => t.as_str().map(str::to_string),
                    None => guess_setting_type(&default),
                };
                (default, kind)
            }
            other => (other.clone(), guess_setting_type(other)),
        };
        if name.len() > 255 {
            errors.push(tr("name_too_long", &[("name", &name)]));
        }
        if kind.is_none() {
            errors.push(tr("data_type_inclusion", &[("name", &name)]));
        }
        if default.is_null() {
            errors.push(tr("default_value_missing", &[("name", &name)]));
            continue;
        }
        let matches_type = match kind.as_deref() {
            Some("integer") => matches!(&default, Yaml::Number(n) if n.is_i64() || n.is_u64()),
            Some("float") => default.is_number(),
            Some("bool") => default.is_bool(),
            Some("list") | Some("icon") => default.is_string(),
            _ => true,
        };
        if !matches_type {
            errors.push(tr("default_not_match_type", &[("name", &name)]));
        }
        if kind.as_deref() == Some("enum") {
            let choices: Vec<Yaml> = v
                .as_mapping()
                .and_then(|h| h.get("choices"))
                .and_then(Yaml::as_sequence)
                .cloned()
                .unwrap_or_default();
            if !choices.is_empty() && !choices.contains(&default) {
                // create_opts appends the default to the choices, so it
                // always validates.
            }
        }
    }
    (!errors.is_empty()).then(|| errors.join("\n"))
}

fn guess_setting_type(v: &Yaml) -> Option<String> {
    match v {
        Yaml::Number(n) if n.is_i64() || n.is_u64() => Some("integer".into()),
        Yaml::Number(_) => Some("float".into()),
        Yaml::String(_) => Some("string".into()),
        Yaml::Bool(_) => Some("bool".into()),
        _ => None,
    }
}

/// `ThemeSettingsManager#value=` failing.
pub enum SettingError {
    /// `Discourse::NotFound`: no setting of that name.
    NotFound,
    /// `Discourse::InvalidParameters` with the validator's messages.
    Invalid(String),
}

/// `cached_settings[name]`: the setting's value as Ruby has it.
pub fn setting_value(setting: &themes::Setting, stored: Option<&str>) -> Value {
    match setting.type_name {
        "bool" => {
            let raw = match stored {
                Some(s) => json!(s),
                None => themes::yaml_json(&setting.default),
            };
            json!(raw == json!(true) || raw == json!("true"))
        }
        _ => setting.value(stored),
    }
}

/// A Ruby value as the string `previous_value` column stores it.
pub fn log_string(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(if *b { "t" } else { "f" }.into()),
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        other => Some(other.to_string()),
    }
}

/// `Theme#update_setting`: the value validated, cast, and stored
/// (ThemeSetting's after_save refreshing the theme's setting modifiers and
/// invalidating its settings field). Returns whether the setting asks for
/// a refresh.
pub async fn update_setting(
    conn: &mut PgConnection,
    theme_id: i32,
    name: &str,
    new_value: Option<&Value>,
    i18n: &crate::i18n::I18n,
) -> Result<Result<bool, SettingError>, AppError> {
    let settings = themes::settings_of(conn, theme_id).await?;
    let Some(setting) = settings.into_iter().find(|s| s.name == name) else {
        return Ok(Err(SettingError::NotFound));
    };
    let text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    // ensure_is_valid_value! on the value as given (after the numeric and
    // boolean casts of their managers).
    let stored = match setting.type_name {
        "bool" => {
            let b = matches!(new_value, Some(Value::Bool(true)))
                || new_value.is_some_and(|v| *v == json!("true"));
            b.to_string()
        }
        "integer" => {
            let i = match new_value {
                Some(Value::Number(n)) if n.is_i64() => n.as_i64().unwrap_or(0),
                Some(v) => crate::ruby::to_i(&text(v)),
                None => 0,
            };
            if let Some(err) = range_error(i18n, "number", i as f64, &setting) {
                return Ok(Err(SettingError::Invalid(err)));
            }
            i.to_string()
        }
        "float" => {
            let f = match new_value {
                Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
                Some(v) => crate::ruby::to_f(&text(v)),
                None => 0.0,
            };
            if let Some(err) = range_error(i18n, "number", f, &setting) {
                return Ok(Err(SettingError::Invalid(err)));
            }
            crate::ruby::float_to_s(f)
        }
        "string" => {
            let s = new_value.map(text).unwrap_or_default();
            if new_value.is_some()
                && let Some(err) = range_error(i18n, "string", s.chars().count() as f64, &setting)
            {
                return Ok(Err(SettingError::Invalid(err)));
            }
            s
        }
        "enum" => {
            let s = new_value.map(text).unwrap_or_default();
            if new_value.is_some() {
                // choices.exclude?(value) && choices.map(&:to_s).exclude?(value)
                let found = setting.choices.iter().any(|c| {
                    themes::yaml_json(c) == *new_value.unwrap_or(&Value::Null)
                        || choice_to_s(c) == s
                });
                if !found {
                    let choices: Vec<String> = setting.choices.iter().map(choice_to_s).collect();
                    let message = i18n
                        .t_with(
                            "themes.settings_errors.enum_value_not_valid",
                            &[("choices", &choices.join(", "))],
                        )
                        .unwrap_or_default();
                    return Ok(Err(SettingError::Invalid(message)));
                }
            }
            s
        }
        _ => return Err(Unsupported("theme settings of this type").into()),
    };
    let existing: Option<(i64, Option<String>)> = sqlx::query_as(
        "SELECT id, value FROM theme_settings WHERE theme_id = $1 AND name = $2 AND data_type = $3 \
         ORDER BY id LIMIT 1",
    )
    .bind(theme_id)
    .bind(name)
    .bind(setting.type_id)
    .fetch_optional(&mut *conn)
    .await?;
    match existing {
        Some((id, current)) => {
            if current.as_deref() != Some(stored.as_str()) {
                sqlx::query(
                    "UPDATE theme_settings SET value = $2, updated_at = now() WHERE id = $1",
                )
                .bind(id)
                .bind(&stored)
                .execute(&mut *conn)
                .await?;
            }
        }
        None => {
            sqlx::query(
                "INSERT INTO theme_settings (name, data_type, theme_id, value, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, now(), now())",
            )
            .bind(name)
            .bind(setting.type_id)
            .bind(theme_id)
            .bind(&stored)
            .execute(&mut *conn)
            .await?;
        }
    }
    refresh_setting_modifiers(conn, theme_id, name, &stored).await?;
    // clear_settings_cache: the settings field invalidated.
    let settings_field: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM theme_fields WHERE theme_id = $1 AND target_id = $2 AND name = 'yaml' \
         ORDER BY id LIMIT 1",
    )
    .bind(theme_id)
    .bind(SETTINGS)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = settings_field {
        invalidate_baked(conn, id).await?;
    }
    Ok(Ok(setting.refresh))
}

/// An enum choice's `to_s`.
fn choice_to_s(c: &Yaml) -> String {
    match c {
        Yaml::String(x) => x.clone(),
        other => themes::yaml_json(other).to_string(),
    }
}

/// `validate_value_in_range!` with the setting's min and max.
fn range_error(
    i18n: &crate::i18n::I18n,
    prefix: &str,
    value: f64,
    setting: &themes::Setting,
) -> Option<String> {
    let num = |y: &Option<Yaml>| y.as_ref().and_then(Yaml::as_f64);
    let (min, max) = (num(&setting.min), num(&setting.max));
    let show = |y: &Option<Yaml>| {
        y.as_ref()
            .map(|v| match v {
                Yaml::Number(n) => n.to_string(),
                other => themes::yaml_json(other).to_string(),
            })
            .unwrap_or_default()
    };
    let key = |k: &str| format!("themes.settings_errors.{prefix}_value_not_valid_{k}");
    match (min, max) {
        (Some(lo), Some(hi)) if !(lo..=hi).contains(&value) => i18n.t_with(
            &key("min_max"),
            &[("min", &show(&setting.min)), ("max", &show(&setting.max))],
        ),
        (Some(lo), _) if value < lo => i18n.t_with(&key("min"), &[("min", &show(&setting.min))]),
        (_, Some(hi)) if value > hi => i18n.t_with(&key("max"), &[("max", &show(&setting.max))]),
        _ => None,
    }
}

/// `ThemeModifierSet#refresh_theme_setting_modifiers` for one setting,
/// saved when a modifier changed.
async fn refresh_setting_modifiers(
    conn: &mut PgConnection,
    theme_id: i32,
    setting: &str,
    value: &str,
) -> Result<(), AppError> {
    let modifiers: Option<Option<Value>> = sqlx::query_scalar(
        "SELECT theme_setting_modifiers FROM theme_modifier_sets WHERE theme_id = $1",
    )
    .bind(theme_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(Some(Value::Object(modifiers))) = modifiers else {
        return Ok(());
    };
    for (modifier, setting_name) in modifiers {
        if setting_name.as_str() != Some(setting) {
            continue;
        }
        let kind: Option<String> = sqlx::query_scalar(
            "SELECT data_type FROM information_schema.columns WHERE table_name = 'theme_modifier_sets' \
             AND column_name = $1",
        )
        .bind(&modifier)
        .fetch_optional(&mut *conn)
        .await?;
        let column = format!("\"{}\"", modifier.replace('"', ""));
        match kind.as_deref() {
            Some("boolean") => {
                sqlx::query(&format!(
                    "UPDATE theme_modifier_sets SET {column} = $2 WHERE theme_id = $1 \
                     AND {column} IS DISTINCT FROM $2"
                ))
                .bind(theme_id)
                .bind(value != "false")
                .execute(&mut *conn)
                .await?;
            }
            Some("ARRAY") => {
                let items: Vec<&str> = value.split('|').filter(|s| !s.is_empty()).collect();
                sqlx::query(&format!(
                    "UPDATE theme_modifier_sets SET {column} = $2::varchar[] WHERE theme_id = $1 \
                     AND {column} IS DISTINCT FROM $2::varchar[]"
                ))
                .bind(theme_id)
                .bind(&items)
                .execute(&mut *conn)
                .await?;
            }
            _ => return Err(Unsupported("a theme setting modifier of another type").into()),
        }
    }
    Ok(())
}

/// `Theme#update_translation` in English: the override written, updated
/// or (back to the default) removed, and the theme's translation fields
/// invalidated (ThemeTranslationOverride's after_commit). False when the
/// theme has no such translation (Discourse::NotFound).
pub async fn update_translation(
    conn: &mut PgConnection,
    theme_id: i32,
    key: &str,
    value: &str,
) -> Result<bool, AppError> {
    let entries = crate::site_texts::theme_translations(conn, theme_id).await?;
    let Some(entry) = entries.into_iter().find(|e| e.key == key) else {
        return Ok(false);
    };
    let existing: Option<(i64, String)> = sqlx::query_as(
        "SELECT id, value FROM theme_translation_overrides WHERE theme_id = $1 AND locale = 'en' \
         AND translation_key = $2 ORDER BY id LIMIT 1",
    )
    .bind(theme_id)
    .bind(key)
    .fetch_optional(&mut *conn)
    .await?;
    let committed = if value == entry.default {
        match existing {
            Some((id, _)) => {
                sqlx::query("DELETE FROM theme_translation_overrides WHERE id = $1")
                    .bind(id)
                    .execute(&mut *conn)
                    .await?;
                true
            }
            None => false,
        }
    } else {
        match existing {
            Some((id, current)) => {
                if current != value {
                    sqlx::query(
                        "UPDATE theme_translation_overrides SET value = $2, updated_at = now() \
                         WHERE id = $1",
                    )
                    .bind(id)
                    .bind(value)
                    .execute(&mut *conn)
                    .await?;
                }
                true
            }
            None => {
                sqlx::query(
                    "INSERT INTO theme_translation_overrides (theme_id, locale, translation_key, \
                     value, created_at, updated_at) VALUES ($1, 'en', $2, $3, now(), now())",
                )
                .bind(theme_id)
                .bind(key)
                .bind(value)
                .execute(&mut *conn)
                .await?;
                true
            }
        }
    };
    if committed {
        sqlx::query(
            "UPDATE theme_fields SET value_baked = NULL WHERE theme_id = $1 AND target_id = $2",
        )
        .bind(theme_id)
        .bind(TRANSLATIONS)
        .execute(&mut *conn)
        .await?;
    }
    Ok(true)
}

/// `Theme#destroy`: its dependents, then the row, then after_destroy's
/// color schemes (destroyed unless another theme uses them, else
/// detached). The default theme is never destroyed here (the services'
/// policy).
pub async fn destroy(conn: &mut PgConnection, theme: &Theme) -> Result<(), AppError> {
    let id = theme.id;
    let fields: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM theme_fields WHERE theme_id = $1 ORDER BY id")
            .bind(id)
            .fetch_all(&mut *conn)
            .await?;
    for field in fields {
        destroy_field(conn, field).await?;
    }
    let uploads: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM upload_references WHERE target_type = 'ThemeSetting' \
         AND target_id IN (SELECT id FROM theme_settings WHERE theme_id = $1))",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    if uploads {
        return Err(Unsupported("theme settings with uploads").into());
    }
    for sql in [
        "DELETE FROM theme_settings WHERE theme_id = $1",
        "DELETE FROM theme_translation_overrides WHERE theme_id = $1",
        "DELETE FROM child_themes WHERE parent_theme_id = $1",
        "DELETE FROM child_themes WHERE child_theme_id = $1",
        "DELETE FROM theme_modifier_sets WHERE theme_id = $1",
        "DELETE FROM theme_svg_sprites WHERE theme_id = $1",
        "DELETE FROM javascript_caches WHERE theme_id = $1",
        "DELETE FROM theme_site_settings WHERE theme_id = $1",
        "DELETE FROM theme_settings_migrations WHERE theme_id = $1",
        "DELETE FROM themes WHERE id = $1",
    ] {
        sqlx::query(sql).bind(id).execute(&mut *conn).await?;
    }
    // after_destroy: the theme's schemes no theme uses go, with their
    // colors; the rest are detached.
    let schemes: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM color_schemes WHERE theme_id = $1 AND id NOT IN \
         (SELECT color_scheme_id FROM themes WHERE color_scheme_id IS NOT NULL) ORDER BY id",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    if !schemes.is_empty() {
        return Err(Unsupported("destroying a theme's own color schemes").into());
    }
    sqlx::query("UPDATE color_schemes SET theme_id = NULL WHERE theme_id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Themes::ThemeSiteSettingManager for a themeable setting: the row
/// written (to_db_value of the value, the default when there is none) and
/// the change logged by `acting`. Returns the stored value, or the
/// InvalidParameters message of a value the setting refuses.
pub async fn set_theme_site_setting(
    conn: &mut PgConnection,
    defs: &crate::site_settings::Definitions,
    theme: &Theme,
    name: &str,
    raw_value: Option<String>,
    acting: i32,
) -> Result<Result<Option<String>, String>, AppError> {
    let def = defs.get(name).ok_or(Unsupported(
        "a themeable setting missing from site_settings.yml",
    ))?;
    // to_db_value / to_rb_value for the setting's type, the default when
    // nil.
    let (data_type, db_value, rb_value): (i32, String, Value) = match def.data_type {
        crate::site_settings::DataType::Bool => {
            let default = def.default.truthy();
            let value = match raw_value.as_deref() {
                None => default,
                Some(v) => matches!(v, "t" | "true"),
            };
            (5, if value { "t" } else { "f" }.into(), json!(value))
        }
        crate::site_settings::DataType::Enum => {
            let default = def.default.to_s();
            let value = raw_value.unwrap_or(default);
            let valid = crate::setting_enums::recorded(name)
                .and_then(|r| r.get("valid_values"))
                .and_then(Value::as_array)
                .is_some_and(|vs| vs.iter().any(|v| v["value"] == json!(value)));
            if !valid {
                return Ok(Err(format!("Invalid value `{value}` for `{name}`")));
            }
            (7, value.clone(), json!(value))
        }
        _ => return Err(Unsupported("themeable settings of other types").into()),
    };
    let existing: Option<(i32, i32, Option<String>)> = sqlx::query_as(
        "SELECT id::int, data_type, value FROM theme_site_settings WHERE theme_id = $1 \
         AND name = $2 LIMIT 1",
    )
    .bind(theme.id)
    .bind(name)
    .fetch_optional(&mut *conn)
    .await?;
    let rb = |t: i32, v: Option<&str>| match crate::site_settings::DataType::from_id(t) {
        Some(crate::site_settings::DataType::Bool) => json!(matches!(v, Some("t") | Some("true"))),
        _ => json!(v),
    };
    let previous = existing.as_ref().map(|(_, t, v)| rb(*t, v.as_deref()));
    let row_id: i32 =
        match &existing {
            Some((id, _, current)) => {
                if current.as_deref() != Some(db_value.as_str()) {
                    sqlx::query(
                    "UPDATE theme_site_settings SET value = $2, updated_at = now() WHERE id = $1",
                )
                .bind(id)
                .bind(&db_value)
                .execute(&mut *conn)
                .await?;
                }
                *id
            }
            None => sqlx::query_scalar(
                "INSERT INTO theme_site_settings (theme_id, name, data_type, value, created_at, \
                 updated_at) VALUES ($1, $2, $3, $4, now(), now()) RETURNING id::int",
            )
            .bind(theme.id)
            .bind(name)
            .bind(data_type)
            .bind(&db_value)
            .fetch_one(&mut *conn)
            .await?,
        };
    log(
        conn,
        "change_theme_site_setting",
        acting,
        &format!("{}: {name}", theme.name),
        None,
        previous.as_ref().and_then(log_string),
        log_string(&rb_value),
    )
    .await?;
    let stored: Option<String> =
        sqlx::query_scalar("SELECT value FROM theme_site_settings WHERE id = $1")
            .bind(row_id)
            .fetch_one(&mut *conn)
            .await?;
    Ok(Ok(stored))
}

/// `belongs_to :remote_theme, dependent: :destroy`, once the destroyed
/// theme is logged (its log still shows the remote theme it held).
pub async fn destroy_remote_theme(conn: &mut PgConnection, theme: &Theme) -> Result<(), AppError> {
    if let Some(remote) = theme.remote_theme_id {
        sqlx::query("DELETE FROM remote_themes WHERE id = $1")
            .bind(remote)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// StaffActionLogger#theme_json
pub async fn theme_json(
    conn: &mut PgConnection,
    cx: &themes::Ctx<'_>,
    theme: &Theme,
) -> Result<String, AppError> {
    let v = themes::serialize(conn, cx, theme, true).await?;
    Ok(crate::web_hooks::to_json(&v))
}

/// `strip_duplicates`: keys equal on both sides dropped, but `name` and
/// `id`.
fn strip_duplicates(old: &str, cur: &str) -> Result<(String, String), AppError> {
    let old: Map<String, Value> =
        serde_json::from_str(old).map_err(|_| Unsupported("a theme log without JSON"))?;
    let cur: Map<String, Value> =
        serde_json::from_str(cur).map_err(|_| Unsupported("a theme log without JSON"))?;
    let same: Vec<String> = old
        .iter()
        .filter(|(k, v)| *k != "name" && *k != "id" && cur.get(k.as_str()) == Some(*v))
        .map(|(k, _)| k.clone())
        .collect();
    // Hash#delete keeps the other keys in their order.
    let keep = |m: Map<String, Value>| -> Map<String, Value> {
        m.into_iter().filter(|(k, _)| !same.contains(k)).collect()
    };
    let (old, cur) = (keep(old), keep(cur));
    Ok((
        crate::web_hooks::to_json(&Value::Object(old)),
        crate::web_hooks::to_json(&Value::Object(cur)),
    ))
}

/// `UserHistory.create!` for a theme action.
#[allow(clippy::too_many_arguments)]
async fn log(
    conn: &mut PgConnection,
    action: &str,
    acting: i32,
    subject: &str,
    context: Option<String>,
    previous: Option<String>,
    new_value: Option<String>,
) -> Result<(), AppError> {
    let action = crate::staff_action_logs::action_id(action)
        .ok_or(Unsupported("a staff action missing from enums.json"))?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, subject, context, previous_value, \
         new_value, admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, TRUE, now(), now())",
    )
    .bind(action)
    .bind(acting)
    .bind(subject)
    .bind(context)
    .bind(previous)
    .bind(new_value)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The longest JSON `json_params` logs before it gives up on both.
const MAX_LOG_JSON: usize = 300_000;

/// `log_theme_change(old_json, theme)`
pub async fn log_theme_change(
    conn: &mut PgConnection,
    cx: &themes::Ctx<'_>,
    acting: i32,
    old_json: Option<&str>,
    theme: &Theme,
) -> Result<(), AppError> {
    let new_json = theme_json(conn, cx, theme).await?;
    let (old, new) = match old_json {
        Some(old) => {
            let (o, n) = strip_duplicates(old, &new_json)?;
            (Some(o), n)
        }
        None => (None, new_json),
    };
    if old.as_ref().is_some_and(|o| o.len() > MAX_LOG_JSON) || new.len() > MAX_LOG_JSON {
        return Err(Unsupported("theme logs over 300,000 characters (json_too_long)").into());
    }
    log(
        conn,
        "change_theme",
        acting,
        &theme.name,
        None,
        old,
        Some(new),
    )
    .await
}

/// `log_theme_destroy(theme)`, the theme serialized after its destroy.
pub async fn log_theme_destroy(
    conn: &mut PgConnection,
    cx: &themes::Ctx<'_>,
    acting: i32,
    theme: &Theme,
) -> Result<(), AppError> {
    let json = theme_json(conn, cx, theme).await?;
    if json.len() > MAX_LOG_JSON {
        return Err(Unsupported("theme logs over 300,000 characters (json_too_long)").into());
    }
    log(
        conn,
        "delete_theme",
        acting,
        &theme.name,
        None,
        Some(json),
        None,
    )
    .await
}

/// `log_theme_component_disabled` / `_enabled`
pub async fn log_component_toggle(
    conn: &mut PgConnection,
    acting: i32,
    theme: &Theme,
    enabled: bool,
) -> Result<(), AppError> {
    let action = if enabled {
        "enable_theme_component"
    } else {
        "disable_theme_component"
    };
    log(
        conn,
        action,
        acting,
        &theme.name,
        Some(theme.id.to_string()),
        None,
        None,
    )
    .await
}

/// `log_theme_setting_change`
pub async fn log_setting_change(
    conn: &mut PgConnection,
    acting: i32,
    theme: &Theme,
    setting: &str,
    previous: Option<String>,
    new_value: Option<String>,
) -> Result<(), AppError> {
    log(
        conn,
        "change_theme_setting",
        acting,
        &format!("{}: {setting}", theme.name),
        None,
        previous,
        new_value,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_duplicates_keeps_name_id_and_key_order() {
        let old = r#"{"id":1,"name":"A","description":null,"updated_at":"x","color_scheme_id":null,"user_selectable":false}"#;
        let cur = r#"{"id":1,"name":"A","description":null,"updated_at":"y","color_scheme_id":13,"user_selectable":false}"#;
        let (o, c) = strip_duplicates(old, cur).unwrap();
        assert_eq!(
            o,
            r#"{"id":1,"name":"A","updated_at":"x","color_scheme_id":null}"#
        );
        assert_eq!(
            c,
            r#"{"id":1,"name":"A","updated_at":"y","color_scheme_id":13}"#
        );
    }

    #[test]
    fn translation_trees_leave_out_theme_metadata_and_merge_overrides() {
        let mut data = translation_tree(
            "en",
            "en:\n  greeting: Hello\n  nested:\n    deep: Deep\n  theme_metadata:\n    description: D\n",
        )
        .unwrap();
        assert_eq!(
            Value::Object(data.clone()),
            json!({"en": {"greeting": "Hello", "nested": {"deep": "Deep"}}})
        );
        let overrides = json!({"en": {"nested": {"deep": "Deeper"}}, "fr": {"x": "y"}});
        let Value::Object(overrides) = overrides else {
            unreachable!()
        };
        deep_merge(&mut data, overrides);
        assert_eq!(
            crate::web_hooks::to_json(&Value::Object(data)),
            r#"{"en":{"greeting":"Hello","nested":{"deep":"Deeper"}},"fr":{"x":"y"}}"#
        );
    }

    #[test]
    fn a_translation_field_names_its_locale_at_the_top() {
        assert!(translation_tree("en", "fr:\n  a: b\n").is_err());
        assert!(translation_tree("en", "").unwrap().is_empty());
    }
}
