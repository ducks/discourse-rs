//! Custom emoji export and import: CustomEmoji::Export (a zip of the
//! images and an `emojis.csv` manifest), CustomEmoji::PreviewImport (the
//! zip read through Compression::SafeZipReader, each manifest row checked,
//! its image uploaded and compared with the emoji of that name, the rows
//! kept two hours where Rails keeps them in Redis) and
//! CustomEmoji::ConfirmImport (the rows applied in one transaction).

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::i18n::I18n;
use crate::{AppError, Unsupported};

const MANIFEST_FILENAME: &str = "emojis.csv";
const BOM: &str = "\u{FEFF}";
/// `ParseImportManifest::MAX_MANIFEST_BYTES`
const MAX_MANIFEST_BYTES: i64 = 1024 * 1024;
const SUPPORTED_EXTENSIONS: [&str; 3] = ["png", "gif", "svg"];
/// `StageImportRows` limits
const MAX_ENTRIES: usize = 1000;
const MAX_TOTAL_BYTES: i64 = 500 * 1024 * 1024;
const MAX_COMPRESSION_RATIO: f64 = 20.0;
/// `StageImportRow::UPLOAD_RETAIN_HOURS`
const UPLOAD_RETAIN_HOURS: i32 = 3;
/// `ImportPreviewCache::TTL`
const PREVIEW_TTL: i64 = 2 * 60 * 60;
/// `CustomEmoji::MAX_GROUP_LENGTH`
const MAX_GROUP_LENGTH: usize = 20;

/// An emoji's name and group with two of its upload's columns.
type EmojiRow = (String, Option<String>, Option<String>, Option<String>);

/// `CustomEmoji.normalize_group`: stripped, downcased, nil when blank or
/// "default".
pub fn normalize_group(group: Option<&str>) -> Option<String> {
    let g = crate::ruby::strip(group.unwrap_or("")).to_lowercase();
    if g.is_empty() || g == "default" {
        None
    } else {
        Some(g)
    }
}

/// A CSV field as Ruby's CSV writes it: quoted when it holds the quote,
/// the separator or a line break (and when empty), nil as nothing.
fn csv_field(value: Option<&str>) -> String {
    match value {
        None => String::new(),
        Some(v) if v.is_empty() || v.contains(['"', ',', '\r', '\n']) => {
            format!("\"{}\"", v.replace('"', "\"\""))
        }
        Some(v) => v.to_string(),
    }
}

pub enum Exported {
    /// The contract: no names.
    NoSelection,
    /// No emoji of those names.
    NotFound,
    Zip(Vec<u8>),
}

/// CustomEmoji::Export with BuildExportArchive: `<name>.<extension>` per
/// emoji and the manifest (BOM, `name,group,filename`).
pub async fn export(
    conn: &mut PgConnection,
    store: &crate::file_store::FileStore,
    names: &[String],
) -> Result<Exported, AppError> {
    let names: Vec<&String> = names.iter().filter(|n| !crate::ruby::is_blank(n)).collect();
    if names.is_empty() {
        return Ok(Exported::NoSelection);
    }
    let rows: Vec<EmojiRow> = sqlx::query_as(
        "SELECT e.name, e.\"group\", u.extension, u.url FROM custom_emojis e \
         LEFT JOIN uploads u ON u.id = e.upload_id WHERE e.name = ANY($1) ORDER BY e.id",
    )
    .bind(names.iter().map(|n| n.as_str()).collect::<Vec<_>>())
    .fetch_all(&mut *conn)
    .await?;
    if rows.is_empty() {
        return Ok(Exported::NotFound);
    }
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let zip_err = |_| Unsupported("an emoji export the zip writer refused");
    let mut manifest = format!("{BOM}name,group,filename\n");
    for (name, group, extension, url) in &rows {
        let (Some(extension), Some(url)) = (extension, url) else {
            return Err(Unsupported("exporting a custom emoji without its upload").into());
        };
        let filename = format!("{name}.{extension}");
        let bytes = store
            .read(url)
            .await?
            .ok_or(Unsupported("exporting a custom emoji whose file is gone"))?;
        zip.start_file(filename.as_str(), options)
            .map_err(zip_err)?;
        zip.write_all(&bytes)?;
        let group = normalize_group(group.as_deref());
        manifest.push_str(&format!(
            "{},{},{}\n",
            csv_field(Some(name)),
            csv_field(group.as_deref()),
            csv_field(Some(&filename))
        ));
    }
    zip.start_file(MANIFEST_FILENAME, options)
        .map_err(zip_err)?;
    zip.write_all(manifest.as_bytes())?;
    Ok(Exported::Zip(zip.finish().map_err(zip_err)?.into_inner()))
}

/// Compression::SafeZipReader's errors, and a CSV the parser refuses.
#[derive(Debug, PartialEq, Eq)]
pub enum ArchiveError {
    MissingEntry,
    TooManyEntries,
    EntryTooLarge,
    SuspiciousEntry,
    MalformedCsv,
}

impl ArchiveError {
    /// The controller's message for it.
    pub fn message_key(&self) -> &'static str {
        match self {
            ArchiveError::MissingEntry => "emoji.import.missing_csv",
            ArchiveError::TooManyEntries => "emoji.import.too_many_entries",
            ArchiveError::EntryTooLarge => "emoji.import.file_too_large",
            ArchiveError::SuspiciousEntry => "emoji.import.suspicious_entry",
            ArchiveError::MalformedCsv => "emoji.import.invalid_csv",
        }
    }
}

/// Compression::SafeZipReader over an uploaded zip.
struct SafeZip<'a> {
    archive: zip::ZipArchive<std::io::Cursor<&'a [u8]>>,
    remaining: i64,
    root_prefix: Option<String>,
}

/// `metadata_entry?`: macOS's `__MACOSX/` and dot files.
fn metadata_entry(name: &str) -> bool {
    let base = name.trim_end_matches('/');
    let base = base.rsplit('/').next().unwrap_or(base);
    name.starts_with("__MACOSX/") || base.starts_with('.')
}

impl<'a> SafeZip<'a> {
    fn open(bytes: &'a [u8]) -> Result<Result<Self, ArchiveError>, AppError> {
        let archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|_| Unsupported("an emoji import that isn't a zip (Zip::Error in Rails)"))?;
        if archive.len() > MAX_ENTRIES {
            return Ok(Err(ArchiveError::TooManyEntries));
        }
        let names: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.name_for_index(i).map(str::to_string))
            .filter(|n| !metadata_entry(n))
            .collect();
        let root_prefix = names.first().and_then(|first| {
            let (segment, _) = first.split_once('/')?;
            let prefix = format!("{segment}/");
            names
                .iter()
                .all(|n| n.starts_with(&prefix))
                .then_some(prefix)
        });
        Ok(Ok(SafeZip {
            archive,
            remaining: MAX_TOTAL_BYTES,
            root_prefix,
        }))
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.archive.index_for_name(name).or_else(|| {
            self.root_prefix
                .as_ref()
                .and_then(|p| self.archive.index_for_name(&format!("{p}{name}")))
        })
    }

    /// `read_entry` / `stream_entry_to_file`: None for a missing entry (or
    /// a directory).
    fn read(
        &mut self,
        name: &str,
        max_bytes: i64,
    ) -> Result<Result<Option<Vec<u8>>, ArchiveError>, AppError> {
        let Some(index) = self.find(name) else {
            return Ok(Ok(None));
        };
        let limit = max_bytes.min(self.remaining);
        let mut entry = self
            .archive
            .by_index(index)
            .map_err(|_| Unsupported("an emoji import entry the zip reader refused"))?;
        if entry.is_dir() {
            return Ok(Ok(None));
        }
        if limit <= 0 {
            return Ok(Err(ArchiveError::EntryTooLarge));
        }
        let size = entry.size() as i64;
        if size > limit {
            return Ok(Err(ArchiveError::EntryTooLarge));
        }
        let compressed = entry.compressed_size() as f64;
        if compressed > 0.0 && size as f64 / compressed > MAX_COMPRESSION_RATIO {
            return Ok(Err(ArchiveError::SuspiciousEntry));
        }
        let mut data = Vec::new();
        (&mut entry)
            .take(limit as u64 + 1)
            .read_to_end(&mut data)
            .map_err(|_| Unsupported("an emoji import entry the zip reader refused"))?;
        if data.len() as i64 > limit {
            return Ok(Err(ArchiveError::EntryTooLarge));
        }
        self.remaining -= data.len() as i64;
        Ok(Ok(Some(data)))
    }
}

/// Ruby's CSV.parse (default options, row separator detected): each row
/// its fields, None for an unquoted empty one.
pub(crate) fn parse_csv(text: &str) -> Result<Vec<Vec<Option<String>>>, ArchiveError> {
    let row_sep = match text.find(['\r', '\n']) {
        Some(i) if text[i..].starts_with("\r\n") => "\r\n",
        Some(i) if text[i..].starts_with('\r') => "\r",
        _ => "\n",
    };
    let mut rows = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let mut row: Vec<Option<String>> = Vec::new();
        // An empty line is a row without fields.
        if let Some(after) = rest.strip_prefix(row_sep) {
            rows.push(row);
            rest = after;
            continue;
        }
        loop {
            if let Some(quoted) = rest.strip_prefix('"') {
                let mut value = String::new();
                let mut chars = quoted.char_indices();
                let end = loop {
                    match chars.next() {
                        None => return Err(ArchiveError::MalformedCsv),
                        Some((i, '"')) => {
                            if quoted[i + 1..].starts_with('"') {
                                value.push('"');
                                chars.next();
                            } else {
                                break i + 1;
                            }
                        }
                        Some((_, c)) => value.push(c),
                    }
                };
                rest = &quoted[end..];
                row.push(Some(value));
            } else {
                let end = rest.find([',', '\r', '\n']).unwrap_or(rest.len());
                let field = &rest[..end];
                if field.contains('"') {
                    return Err(ArchiveError::MalformedCsv);
                }
                rest = &rest[end..];
                // A line break that isn't the row separator.
                if !rest.is_empty() && !rest.starts_with(',') && !rest.starts_with(row_sep) {
                    return Err(ArchiveError::MalformedCsv);
                }
                row.push((!field.is_empty()).then(|| field.to_string()));
            }
            if let Some(after) = rest.strip_prefix(',') {
                rest = after;
                if rest.is_empty() || rest.starts_with(row_sep) {
                    row.push(None);
                } else {
                    continue;
                }
            }
            if rest.is_empty() {
                break;
            }
            match rest.strip_prefix(row_sep) {
                Some(after) => {
                    rest = after;
                    break;
                }
                None => return Err(ArchiveError::MalformedCsv),
            }
        }
        rows.push(row);
    }
    Ok(rows)
}

/// CustomEmoji::ImportRow, as the preview cache keeps it (`to_h`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRow {
    pub index: usize,
    pub name: String,
    pub group: Option<String>,
    pub filename: String,
    pub errors: Vec<String>,
    pub category: Option<String>,
    pub incoming_url: Option<String>,
    pub existing_url: Option<String>,
    pub existing_group: Option<String>,
    pub upload_id: Option<i32>,
}

const INVALID: &str = "invalid";

impl ImportRow {
    fn invalid(&self) -> bool {
        self.category.as_deref() == Some(INVALID)
    }

    fn conflict(&self) -> bool {
        matches!(
            self.category.as_deref(),
            Some("conflict_group" | "conflict_image" | "conflict_both")
        )
    }

    fn mark_invalid(&mut self, messages: Vec<String>) {
        self.errors.extend(messages);
        self.category = Some(INVALID.into());
    }

    /// CustomEmoji::ImportRowSerializer
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("index".into(), json!(self.index));
        out.insert("name".into(), json!(self.name));
        out.insert(
            "group".into(),
            json!(self.group.as_deref().unwrap_or("default")),
        );
        out.insert("filename".into(), json!(self.filename));
        out.insert("category".into(), json!(self.category));
        if self.invalid() {
            out.insert("errors".into(), json!(self.errors));
        }
        if let Some(url) = self
            .incoming_url
            .as_deref()
            .filter(|u| !crate::ruby::is_blank(u))
        {
            out.insert("incoming_url".into(), json!(url));
        }
        if self.conflict()
            && let Some(url) = self
                .existing_url
                .as_deref()
                .filter(|u| !crate::ruby::is_blank(u))
        {
            out.insert("existing_url".into(), json!(url));
        }
        if self.conflict() {
            out.insert(
                "existing_group".into(),
                json!(self.existing_group.as_deref().unwrap_or("default")),
            );
        }
        Value::Object(out)
    }
}

/// `File.extname(name)` without its dot, downcased.
fn extension(filename: &str) -> String {
    let base = filename.rsplit('/').next().unwrap_or(filename);
    let base = base.trim_start_matches('.');
    match base.rfind('.') {
        Some(i) => base[i + 1..].to_lowercase(),
        None => String::new(),
    }
}

/// CustomEmoji::Action::ParseImportManifest over the manifest's text.
fn parse_manifest(i18n: &I18n, text: &str) -> Result<Vec<ImportRow>, ArchiveError> {
    let text = text.strip_prefix(BOM).unwrap_or(text);
    let mut csv = parse_csv(text)?.into_iter();
    let headers = csv.next().unwrap_or_default();
    let column = |row: &[Option<String>], name: &str| -> Option<String> {
        let at = headers.iter().position(|h| h.as_deref() == Some(name))?;
        row.get(at).cloned().flatten()
    };
    let t = |key: &str, args: &[(&str, &str)]| {
        let key = format!("emoji.import.validation.{key}");
        i18n.t_with(&key, args).unwrap_or(key)
    };
    let mut names_seen: Vec<String> = Vec::new();
    let mut filenames_seen: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    for (index, record) in csv.enumerate() {
        let name = column(&record, "name").unwrap_or_default();
        let mut row = ImportRow {
            index,
            name: crate::custom_emoji::sanitize_name(crate::ruby::strip(&name)),
            group: normalize_group(column(&record, "group").as_deref()),
            filename: crate::ruby::strip(&column(&record, "filename").unwrap_or_default())
                .to_string(),
            errors: Vec::new(),
            category: None,
            incoming_url: None,
            existing_url: None,
            existing_group: None,
            upload_id: None,
        };
        let mut errors = Vec::new();
        if crate::ruby::is_blank(&row.name) {
            errors.push(t("missing_name", &[]));
        } else if names_seen.contains(&row.name) {
            errors.push(t("duplicate_name", &[]));
        }
        if crate::ruby::is_blank(&row.filename) {
            errors.push(t("missing_filename", &[]));
        } else {
            let ext = extension(&row.filename);
            if !SUPPORTED_EXTENSIONS.contains(&ext.as_str()) {
                errors.push(t("unsupported_extension", &[("ext", &ext)]));
            }
            if filenames_seen.contains(&row.filename) {
                errors.push(t("duplicate_filename", &[]));
            }
        }
        if row
            .group
            .as_ref()
            .is_some_and(|g| g.chars().count() > MAX_GROUP_LENGTH)
        {
            errors.push(t("group_too_long", &[]));
        }
        if !errors.is_empty() {
            row.mark_invalid(errors);
        }
        filenames_seen.push(row.filename.clone());
        names_seen.push(row.name.clone());
        rows.push(row);
    }
    Ok(rows)
}

pub enum Previewed {
    /// The archive or its manifest refused.
    Refused(ArchiveError),
    /// `manifest_not_empty` failed.
    EmptyManifest,
    Rows {
        token: String,
        rows: Vec<ImportRow>,
    },
}

fn preview_key(user_id: i32, token: &str) -> String {
    format!("emoji_import_preview:{user_id}:{token}")
}

/// CustomEmoji::PreviewImport: the rows staged (each valid row's image
/// uploaded and kept three hours), then kept two hours under a token.
pub async fn preview(
    conn: &mut PgConnection,
    settings: &crate::site_settings::SiteSettings,
    i18n: &I18n,
    urls: &crate::url::Urls<'_>,
    store: &crate::file_store::FileStore,
    admin_id: i32,
    zip_bytes: &[u8],
) -> Result<Previewed, AppError> {
    let mut reader = match SafeZip::open(zip_bytes)? {
        Ok(r) => r,
        Err(e) => return Ok(Previewed::Refused(e)),
    };
    let manifest = match reader.read(MANIFEST_FILENAME, MAX_MANIFEST_BYTES)? {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(Previewed::Refused(ArchiveError::MissingEntry)),
        Err(e) => return Ok(Previewed::Refused(e)),
    };
    let text = String::from_utf8(manifest)
        .map_err(|_| Unsupported("an emoji manifest that isn't UTF-8"))?;
    let mut rows = match parse_manifest(i18n, &text) {
        Ok(rows) => rows,
        Err(e) => return Ok(Previewed::Refused(e)),
    };
    // existing_emojis_for: the valid rows' names.
    let names: Vec<String> = rows
        .iter()
        .filter(|r| !r.invalid())
        .map(|r| r.name.clone())
        .collect();
    let existing: Vec<EmojiRow> = sqlx::query_as(
        "SELECT e.name, e.\"group\", u.url, u.sha1 FROM custom_emojis e \
             LEFT JOIN uploads u ON u.id = e.upload_id WHERE e.name = ANY($1)",
    )
    .bind(&names)
    .fetch_all(&mut *conn)
    .await?;
    let max_image = settings.get("max_image_size_kb")?.to_i() * 1024;
    for row in rows.iter_mut().filter(|r| !r.invalid()) {
        let image = match reader.read(&row.filename, max_image)? {
            Ok(image) => image,
            Err(e) => return Ok(Previewed::Refused(e)),
        };
        let Some(image) = image else {
            let key = "emoji.import.validation.missing_image";
            row.mark_invalid(vec![i18n.t(key).unwrap_or(key).to_string()]);
            continue;
        };
        let outcome = crate::uploads::create(
            &mut *conn,
            settings,
            i18n,
            urls,
            store,
            &crate::uploads::NewUpload {
                user_id: admin_id,
                staff: true,
                upload_type: "custom_emoji",
                filename: &row.filename,
                bytes: &image,
                pasted: false,
                for_private_message: false,
                for_export: false,
                for_theme: false,
                skip_validations: false,
            },
        )
        .await?;
        let upload_id = match outcome {
            crate::uploads::Outcome::Created(upload) => upload["id"]
                .as_i64()
                .and_then(|id| i32::try_from(id).ok())
                .ok_or(Unsupported("an upload without its id"))?,
            crate::uploads::Outcome::Invalid(messages) => {
                row.mark_invalid(messages);
                continue;
            }
        };
        let (url, sha1): (String, String) = sqlx::query_as(
            "UPDATE uploads SET retain_hours = $2 WHERE id = $1 RETURNING url, sha1",
        )
        .bind(upload_id)
        .bind(UPLOAD_RETAIN_HOURS)
        .fetch_one(&mut *conn)
        .await?;
        row.upload_id = Some(upload_id);
        row.incoming_url = Some(url);
        // classify
        let category = match existing.iter().find(|(n, ..)| *n == row.name) {
            None => "new",
            Some((_, group, existing_url, existing_sha1)) => {
                row.existing_url = existing_url.clone();
                row.existing_group = normalize_group(group.as_deref());
                let group_changed = row.existing_group != row.group;
                let image_changed = existing_sha1.as_deref() != Some(sha1.as_str());
                match (group_changed, image_changed) {
                    (true, true) => "conflict_both",
                    (false, true) => "conflict_image",
                    (true, false) => "conflict_group",
                    (false, false) => "identical",
                }
            }
        };
        row.category = Some(category.into());
    }
    if rows.is_empty() {
        return Ok(Previewed::EmptyManifest);
    }
    let token = crate::accounts::random_hex();
    crate::owned_schema::cached_setex(
        &mut *conn,
        &preview_key(admin_id, &token),
        PREVIEW_TTL,
        &serde_json::to_string(&rows).map_err(std::io::Error::other)?,
    )
    .await?;
    Ok(Previewed::Rows { token, rows })
}

pub enum Confirmed {
    /// No rows under the token.
    Expired,
    Report(Value),
}

/// CustomEmoji::ConfirmImport: the staged rows applied (invalid,
/// identical and kept ones skipped), each logged as created, the preview
/// dropped.
pub async fn confirm(
    conn: &mut PgConnection,
    admin_id: i32,
    token: &str,
    resolutions: &Map<String, Value>,
) -> Result<Confirmed, AppError> {
    let key = preview_key(admin_id, token);
    let Some(payload) = crate::owned_schema::cached_get(&mut *conn, &key).await? else {
        return Ok(Confirmed::Expired);
    };
    if crate::ruby::is_blank(&payload) {
        return Ok(Confirmed::Expired);
    }
    let mut rows: Vec<ImportRow> = serde_json::from_str(&payload).map_err(std::io::Error::other)?;
    for row in &mut rows {
        row.group = normalize_group(row.group.as_deref());
    }
    let keep = |row: &ImportRow| {
        resolutions
            .get(&row.name)
            .is_some_and(|r| r.as_str() == Some("keep"))
    };
    let (mut created, mut updated, mut skipped) = (0, 0, 0);
    for row in &rows {
        if row.invalid() {
            continue;
        }
        if row.category.as_deref() == Some("identical") {
            skipped += 1;
            continue;
        }
        if keep(row) {
            continue;
        }
        let Some(upload_id) = row.upload_id else {
            continue;
        };
        let upload: Option<i32> = sqlx::query_scalar("SELECT id FROM uploads WHERE id = $1")
            .bind(upload_id)
            .fetch_optional(&mut *conn)
            .await?;
        if upload.is_none() {
            continue;
        }
        let existing: Option<(i32, i32, Option<String>)> =
            sqlx::query_as("SELECT id, upload_id, \"group\" FROM custom_emojis WHERE name = $1")
                .bind(&row.name)
                .fetch_optional(&mut *conn)
                .await?;
        let emoji_id = match existing {
            Some((id, old_upload, old_group)) => {
                // update!: saved (and dated) only when something changed.
                if old_upload != upload_id || old_group != row.group {
                    sqlx::query(
                        "UPDATE custom_emojis SET upload_id = $2, \"group\" = $3, \
                         updated_at = clock_timestamp() WHERE id = $1",
                    )
                    .bind(id)
                    .bind(upload_id)
                    .bind(&row.group)
                    .execute(&mut *conn)
                    .await?;
                }
                updated += 1;
                (old_upload != upload_id).then_some(id)
            }
            None => {
                let id: i32 = sqlx::query_scalar(
                    "INSERT INTO custom_emojis (name, upload_id, \"group\", user_id, created_at, updated_at) \
                     VALUES ($1, $2, $3, $4, clock_timestamp(), clock_timestamp()) RETURNING id",
                )
                .bind(&row.name)
                .bind(upload_id)
                .bind(&row.group)
                .bind(admin_id)
                .fetch_one(&mut *conn)
                .await?;
                created += 1;
                Some(id)
            }
        };
        // UploadReference.ensure_exist! when the upload changed.
        if let Some(id) = emoji_id {
            sqlx::query(
                "DELETE FROM upload_references WHERE target_type = 'CustomEmoji' AND target_id = $1 \
                 AND upload_id <> $2",
            )
            .bind(id)
            .bind(upload_id)
            .execute(&mut *conn)
            .await?;
            sqlx::query(
                "INSERT INTO upload_references (upload_id, target_type, target_id, created_at, updated_at) \
                 VALUES ($1, 'CustomEmoji', $2, clock_timestamp(), clock_timestamp()) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(upload_id)
            .bind(id)
            .execute(&mut *conn)
            .await?;
        }
        crate::custom_emoji::log_create(&mut *conn, admin_id, &row.name, row.group.as_deref())
            .await?;
    }
    crate::owned_schema::cached_del(&mut *conn, &key).await?;
    Ok(Confirmed::Report(
        json!({ "created": created, "updated": updated, "skipped": skipped }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_like_ruby() {
        assert_eq!(
            parse_csv("name,group,filename\na,,a.png\n\n\"b,\"\"c\",x,b.gif").unwrap(),
            vec![
                vec![
                    Some("name".into()),
                    Some("group".into()),
                    Some("filename".into())
                ],
                vec![Some("a".into()), None, Some("a.png".into())],
                vec![],
                vec![Some("b,\"c".into()), Some("x".into()), Some("b.gif".into())],
            ]
        );
        assert_eq!(
            parse_csv("a,b\r\nc,\r\n").unwrap(),
            vec![
                vec![Some("a".into()), Some("b".into())],
                vec![Some("c".into()), None]
            ]
        );
        assert_eq!(parse_csv("a\"b\n"), Err(ArchiveError::MalformedCsv));
        assert_eq!(parse_csv("\"ab\n"), Err(ArchiveError::MalformedCsv));
        assert_eq!(parse_csv("\"a\"b\n"), Err(ArchiveError::MalformedCsv));
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn safe_zip_limits() {
        let bytes = zip_of(&[
            ("wrap/emojis.csv", b"a"),
            ("__MACOSX/x", b"b"),
            ("wrap/a.gif", b"c"),
        ]);
        let mut z = SafeZip::open(&bytes).unwrap().unwrap();
        assert_eq!(z.read("emojis.csv", 10).unwrap(), Ok(Some(b"a".to_vec())));
        assert_eq!(
            z.read("emojis.csv", 0).unwrap(),
            Err(ArchiveError::EntryTooLarge)
        );
        let zeros = vec![0u8; 100_000];
        let bytes = zip_of(&[("emojis.csv", &zeros)]);
        let mut z = SafeZip::open(&bytes).unwrap().unwrap();
        assert_eq!(
            z.read("emojis.csv", 1 << 20).unwrap(),
            Err(ArchiveError::SuspiciousEntry)
        );
        let many: Vec<(String, &[u8])> =
            (0..1001).map(|i| (format!("{i}.gif"), &b"x"[..])).collect();
        let many: Vec<(&str, &[u8])> = many.iter().map(|(n, d)| (n.as_str(), *d)).collect();
        assert!(matches!(
            SafeZip::open(&zip_of(&many)).unwrap(),
            Err(ArchiveError::TooManyEntries)
        ));
    }

    #[test]
    fn extensions() {
        assert_eq!(extension("a.PNG"), "png");
        assert_eq!(extension("dir/a"), "");
        assert_eq!(extension(".png"), "");
        assert_eq!(extension("a.b.gif"), "gif");
    }

    #[test]
    fn fields_quoted_like_ruby() {
        assert_eq!(csv_field(None), "");
        assert_eq!(csv_field(Some("")), "\"\"");
        assert_eq!(csv_field(Some("a,b")), "\"a,b\"");
        assert_eq!(csv_field(Some("plain")), "plain");
    }
}
