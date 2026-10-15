//! Theme bundles in and out: ThemeStore::ZipImporter (zip, tar, tar.gz
//! and gz archives unpacked as Compression::Engine does, the size budget
//! and the root directory stripped), RemoteTheme.update_theme with
//! update_from_remote (assets and screenshots uploaded, about.json's
//! metadata, modifiers, color schemes and theme site settings, every file
//! set as its field and the fields the bundle lost destroyed),
//! ThemeStore::ZipExporter with Theme#generate_metadata_hash, the remote
//! placeholders, and ThemeStore::GitImporter up to its clone (the url
//! normalized, its scheme and key checked).
//!
//! Refused: cloning a git repository (http(s) or ssh: a network fetch),
//! so remote imports, update checks, updates and source changes of git
//! themes; child components about.json names (each one a git import);
//! pax tar headers.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::theme_writes::{self, FieldChange, FieldInput};
use crate::themes::{self, Theme};
use crate::{AppError, Unsupported};

/// `RemoteTheme::ImportError`: rendered as a 422 with its message.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportError(pub String);

/// A Ruby exception the controller doesn't rescue: a 500.
fn crash(what: &str) -> AppError {
    std::io::Error::other(what.to_string()).into()
}

/// `Discourse::MAX_METADATA_FILE_SIZE`
const MAX_METADATA_FILE_SIZE: i64 = 64 * 1024;
/// `RemoteTheme::MAX_ASSET_FILE_SIZE`
const MAX_ASSET_FILE_SIZE: i64 = 8 * 1024 * 1024;
/// `RemoteTheme::MAX_THEME_FILE_COUNT`
const MAX_THEME_FILE_COUNT: usize = 1024;
/// `RemoteTheme::MAX_THEME_SIZE`
const MAX_THEME_SIZE: i64 = 256 * 1024 * 1024;
/// `Compression::Zip::MAX_ZIP_ENTRIES`
const MAX_ZIP_ENTRIES: usize = 10_000;
/// `Compression::Strategy#chunk_size`
const CHUNK: f64 = 2.0 * 1024.0 * 1024.0;
/// `ThemeScreenshotsHandler` limits.
const MAX_SCREENSHOT_FILE_SIZE: usize = 1024 * 1024;
const MAX_SCREENSHOT_DIMENSIONS: (u32, u32) = (3840, 2160);
const MAX_SCREENSHOT_COUNT: usize = 2;
const SCREENSHOT_TYPES: [&str; 4] = [".jpeg", ".jpg", ".png", ".webp"];
/// `Admin::ThemesController::MAX_REMOTE_LENGTH`
pub const MAX_REMOTE_LENGTH: usize = 10_000;

fn t(i18n: &crate::i18n::I18n, key: &str, args: &[(&str, &str)]) -> String {
    i18n.t_with(key, args)
        .unwrap_or_else(|| format!("Translation missing: en.{key}"))
}

// The unpacked bundle: a directory tree in memory.

#[derive(Debug, Clone)]
enum Node {
    Dir,
    File(Vec<u8>),
}

/// The importer's temp folder, paths relative to it.
#[derive(Debug, Default)]
pub struct Bundle {
    nodes: BTreeMap<String, Node>,
}

/// `File.expand_path(File.join(dest, name))` kept under `dest`: the
/// relative path, None when it leaves it.
fn safe_join(name: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

fn parent(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

fn basename(path: &str) -> &str {
    path.rfind('/').map_or(path, |i| &path[i + 1..])
}

/// `File.extname`
fn extname(path: &str) -> &str {
    let base = basename(path);
    let trimmed = base.trim_start_matches('.');
    match trimmed.rfind('.') {
        Some(i) => &trimmed[i..],
        None => "",
    }
}

impl Bundle {
    fn mkdir_p(&mut self, path: &str) {
        let mut prefix = String::new();
        for part in path.split('/').filter(|p| !p.is_empty()) {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            self.nodes.entry(prefix.clone()).or_insert(Node::Dir);
        }
    }

    fn add_file(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), AppError> {
        if path.is_empty() || self.nodes.contains_key(path) {
            return Err(crash("Compression::Strategy::DestinationFileExistsError"));
        }
        self.mkdir_p(parent(path));
        self.nodes.insert(path.to_string(), Node::File(bytes));
        Ok(())
    }

    /// `real_path`: the path an existing entry resolves to, None when it
    /// is missing or resolves outside the folder.
    pub fn real_path(&self, relative: &str) -> Option<String> {
        let mut parts: Vec<&str> = Vec::new();
        for part in relative.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    // Every component before `..` must be a directory.
                    let current = parts.join("/");
                    if !current.is_empty() && !matches!(self.nodes.get(&current), Some(Node::Dir)) {
                        return None;
                    }
                    parts.pop()?;
                }
                other => parts.push(other),
            }
        }
        let path = parts.join("/");
        if path.is_empty() || self.nodes.contains_key(&path) {
            Some(path)
        } else {
            None
        }
    }

    /// `importer[path]`: a file's content.
    pub fn read(&self, relative: &str) -> Option<&[u8]> {
        match self.nodes.get(&self.real_path(relative)?) {
            Some(Node::File(bytes)) => Some(bytes),
            _ => None,
        }
    }

    /// `file_size`: -1 for a missing file.
    pub fn file_size(&self, relative: &str) -> i64 {
        match self.real_path(relative).and_then(|p| self.nodes.get(&p)) {
            Some(Node::File(bytes)) => bytes.len() as i64,
            Some(Node::Dir) => 4096,
            None => -1,
        }
    }

    /// The entries directly under a directory, sorted by name.
    fn children(&self, dir: &str) -> Vec<String> {
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let mut names: Vec<String> = self
            .nodes
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .filter(|rest| !rest.is_empty() && !rest.contains('/'))
            .map(str::to_string)
            .collect();
        names.sort();
        names
    }

    /// `all_files`: `Dir.glob("**/**")` less directories, in glob's order
    /// (each directory's entries sorted, dot files left out).
    pub fn all_files(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.walk("", &mut out);
        out
    }

    fn walk(&self, dir: &str, out: &mut Vec<String>) {
        for name in self.children(dir) {
            if name.starts_with('.') {
                continue;
            }
            let path = if dir.is_empty() {
                name
            } else {
                format!("{dir}/{name}")
            };
            match self.nodes.get(&path) {
                Some(Node::Dir) => self.walk(&path, out),
                Some(Node::File(_)) => out.push(path),
                None => {}
            }
        }
    }

    /// `File.rename` of an entry (and what is under it).
    fn rename(&mut self, from: &str, to: &str) {
        let moved: Vec<String> = self
            .nodes
            .keys()
            .filter(|k| *k == from || k.starts_with(&format!("{from}/")))
            .cloned()
            .collect();
        for key in moved {
            if let Some(node) = self.nodes.remove(&key) {
                let new_key = format!("{to}{}", &key[from.len()..]);
                self.nodes.insert(new_key, node);
            }
        }
    }

    /// `strip_root_directory`: a lone root directory's entries moved up.
    fn strip_root_directory(&mut self) {
        let roots: Vec<String> = self
            .children("")
            .into_iter()
            .filter(|n| !n.starts_with('.'))
            .collect();
        if roots.len() != 1 || !matches!(self.nodes.get(&roots[0]), Some(Node::Dir)) {
            return;
        }
        let root = roots[0].clone();
        for child in self.children(&root) {
            if child.starts_with('.') || self.nodes.contains_key(&child) {
                continue;
            }
            self.rename(&format!("{root}/{child}"), &child);
        }
    }
}

/// Why unpacking stopped short of an archive.
enum UnpackError {
    /// RuntimeError: `themes.import_error.unpack_failed`.
    Failed,
    /// Compression::Strategy::ExtractFailed: `file_too_big`.
    TooBig,
}

/// `calculate_available_size`: megabytes as mebibytes.
fn available(max_mb: f64) -> f64 {
    1024.0 * 1024.0 * (max_mb / 1.049)
}

/// Bytes read `CHUNK` at a time, each read counted as a whole chunk.
fn chunked(len: usize) -> f64 {
    (len as f64 / CHUNK).ceil() * CHUNK
}

/// `ThemeStore::ZipImporter#import!`: the archive unpacked by the
/// strategy its original filename names.
fn unpack(
    bytes: &[u8],
    original_filename: &str,
    max_mb: f64,
) -> Result<Result<Bundle, UnpackError>, AppError> {
    let mut bundle = Bundle::default();
    let outcome = if original_filename.contains(".zip") {
        unzip(bytes, max_mb, &mut bundle)?
    } else if original_filename.contains(".tar.gz") {
        match gunzip(bytes, max_mb)? {
            Ok(tar) => untar(&tar, max_mb, &mut bundle)?,
            Err(e) => Err(e),
        }
    } else if original_filename.contains(".gz") {
        // The decompressed file is named for the upload's temp file, which
        // no matcher (nor about.json) can be.
        match gunzip(bytes, max_mb)? {
            Ok(data) => bundle.add_file("RackMultipart", data).map(Ok)?,
            Err(e) => Err(e),
        }
    } else if original_filename.contains(".tar") {
        untar(bytes, max_mb, &mut bundle)?
    } else {
        return Err(crash("Compression::Engine::UnsupportedFileExtension"));
    };
    if let Err(e) = outcome {
        return Ok(Err(e));
    }
    bundle.strip_root_directory();
    Ok(Ok(bundle))
}

/// Compression::Zip#decompress
fn unzip(
    bytes: &[u8],
    max_mb: f64,
    bundle: &mut Bundle,
) -> Result<Result<(), UnpackError>, AppError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| crash("Zip::Error (not a zip archive)"))?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(crash("Compression::SafeZipReader::TooManyEntriesError"));
    }
    let mut remaining = available(max_mb);
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|_| crash("Zip::Error (an entry it can't read)"))?;
        let symlink = entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000);
        if symlink {
            continue;
        }
        let Some(path) = safe_join(entry.name()) else {
            continue;
        };
        if entry.is_dir() {
            bundle.mkdir_p(&path);
            continue;
        }
        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .map_err(|_| crash("Zip::Error (an entry it can't inflate)"))?;
        remaining -= chunked(data.len());
        if remaining < 0.0 {
            return Ok(Err(UnpackError::TooBig));
        }
        bundle.add_file(&path, data)?;
    }
    Ok(Ok(()))
}

/// Compression::Gzip#decompress of one member.
fn gunzip(bytes: &[u8], max_mb: f64) -> Result<Result<Vec<u8>, UnpackError>, AppError> {
    let mut data = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut data)
        .map_err(|_| crash("Zlib::GzipFile::Error"))?;
    if available(max_mb) - chunked(data.len()) < 0.0 {
        return Ok(Err(UnpackError::TooBig));
    }
    Ok(Ok(data))
}

/// An octal header field, None when it isn't one.
fn octal(field: &[u8]) -> Option<u64> {
    let text: String = field
        .iter()
        .take_while(|b| **b != 0)
        .map(|b| *b as char)
        .collect();
    let text = text.trim();
    if text.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(text, 8).ok()
}

fn c_string(field: &[u8]) -> String {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// Compression::Tar#decompress over Gem::Package::TarReader: regular
/// files and directories, GNU long names.
fn untar(
    data: &[u8],
    max_mb: f64,
    bundle: &mut Bundle,
) -> Result<Result<(), UnpackError>, AppError> {
    let mut remaining = available(max_mb);
    let mut offset = 0;
    let mut long_name: Option<String> = None;
    while offset + 512 <= data.len() {
        let header = &data[offset..offset + 512];
        if header.iter().all(|b| *b == 0) {
            break;
        }
        // The header's checksum: its bytes with the checksum field as
        // spaces.
        let stored = octal(&header[148..156]);
        let sum: u64 = header
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if (148..156).contains(&i) {
                    32
                } else {
                    u64::from(*b)
                }
            })
            .sum();
        let (Some(stored), Some(size)) = (stored, octal(&header[124..136])) else {
            return Ok(Err(UnpackError::Failed));
        };
        if stored != sum {
            return Ok(Err(UnpackError::Failed));
        }
        let size = size as usize;
        let body_start = offset + 512;
        let body_end = body_start + size;
        if body_end > data.len() {
            return Ok(Err(UnpackError::Failed));
        }
        let body = &data[body_start..body_end];
        offset = body_start + size.div_ceil(512) * 512;
        let typeflag = header[156];
        if typeflag == b'L' {
            long_name = Some(c_string(body));
            continue;
        }
        if matches!(typeflag, b'x' | b'g') {
            return Err(Unsupported("theme tar archives with pax headers").into());
        }
        let name = match long_name.take() {
            Some(name) => name,
            None => {
                let name = c_string(&header[0..100]);
                let prefix = if &header[257..262] == b"ustar" {
                    c_string(&header[345..500])
                } else {
                    String::new()
                };
                if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                }
            }
        };
        let file = matches!(typeflag, b'0' | 0);
        let dir = typeflag == b'5';
        if !file && !dir {
            continue;
        }
        let Some(path) = safe_join(&name) else {
            continue;
        };
        // mkdir_p(File.dirname(entry_path)): a directory entry makes only
        // its parent (Tar's extract_folder does nothing).
        let trimmed = name.trim_end_matches('/');
        if dir {
            if let Some(p) = safe_join(parent(trimmed)) {
                bundle.mkdir_p(&p);
            }
            continue;
        }
        remaining -= body.len() as f64;
        if remaining < 0.0 {
            return Ok(Err(UnpackError::TooBig));
        }
        bundle.add_file(&path, body.to_vec())?;
    }
    Ok(Ok(()))
}

// about.json

/// `RemoteTheme.extract_theme_info`
fn extract_theme_info(
    bundle: &Bundle,
    i18n: &crate::i18n::I18n,
) -> Result<Result<Map<String, Value>, ImportError>, AppError> {
    if bundle.file_size("about.json") > MAX_METADATA_FILE_SIZE {
        return Ok(Err(ImportError(t(
            i18n,
            "themes.import_error.about_json_too_big",
            &[("limit", &crate::uploads::human_size(MAX_METADATA_FILE_SIZE))],
        ))));
    }
    let invalid = || {
        Ok(Err(ImportError(t(
            i18n,
            "themes.import_error.about_json",
            &[],
        ))))
    };
    let Some(bytes) = bundle.read("about.json") else {
        return invalid();
    };
    let Ok(parsed) = serde_json::from_slice::<Value>(bytes) else {
        return invalid();
    };
    match parsed {
        Value::Object(map) if map.contains_key("name") => Ok(Ok(map)),
        Value::Object(_) | Value::Array(_) => invalid(),
        _ => Err(crash("NoMethodError: fetch on about.json's value")),
    }
}

/// ActiveModel's string cast: true "t", false "f", other scalars as Ruby
/// writes them.
fn cast_string(value: Option<&Value>) -> Result<Option<String>, AppError> {
    Ok(match value {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(if *b { "t" } else { "f" }.to_string()),
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(_) => {
            return Err(Unsupported("about.json metadata that is not a string or number").into());
        }
    })
}

/// Ruby's `to_s` of a JSON value, for `theme_info["component"].to_s`.
fn ruby_to_s(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// `presence` of a JSON value.
fn present(value: Option<&Value>) -> Option<&Value> {
    value.filter(|v| match v {
        Value::Null | Value::Bool(false) => false,
        Value::String(s) => !crate::ruby::is_blank(s),
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
        _ => true,
    })
}

/// `Discourse::VERSION_REGEXP`
fn valid_version(version: &str) -> bool {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\A\d+\.\d+\.\d+(\.beta\d+|-latest(\.\d+)?)?\z").expect("valid regex")
    });
    RE.is_match(version)
}

/// RemoteTheme's `METADATA_PROPERTIES`, as about.json sets them.
#[derive(Debug, Default, Clone)]
struct Metadata {
    license_url: Option<String>,
    about_url: Option<String>,
    authors: Option<String>,
    theme_version: Option<String>,
    minimum_discourse_version: Option<String>,
    maximum_discourse_version: Option<String>,
}

impl Metadata {
    fn from_info(info: &Map<String, Value>) -> Result<Metadata, AppError> {
        Ok(Metadata {
            license_url: cast_string(info.get("license_url"))?,
            about_url: cast_string(info.get("about_url"))?,
            authors: cast_string(info.get("authors"))?,
            theme_version: cast_string(info.get("theme_version"))?,
            minimum_discourse_version: cast_string(info.get("minimum_discourse_version"))?,
            maximum_discourse_version: cast_string(info.get("maximum_discourse_version"))?,
        })
    }

    /// RemoteTheme's validations' full messages.
    fn errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self
            .minimum_discourse_version
            .as_deref()
            .is_some_and(|v| !valid_version(v))
        {
            errors.push("Minimum discourse version is invalid".to_string());
        }
        if self
            .maximum_discourse_version
            .as_deref()
            .is_some_and(|v| !valid_version(v))
        {
            errors.push("Maximum discourse version is invalid".to_string());
        }
        errors
    }
}

// Theme modifiers

/// A `theme_modifier_sets` column and its value.
#[derive(Debug, Clone, PartialEq)]
enum Modifier {
    Bool(Option<bool>),
    Strings(Option<Vec<Option<String>>>),
}

/// `ThemeModifierSet.modifiers`: the table's boolean and string array
/// columns, in its order.
async fn modifier_columns(conn: &mut PgConnection) -> Result<Vec<(String, bool)>, AppError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT column_name::text, data_type::text FROM information_schema.columns \
         WHERE table_name = 'theme_modifier_sets' AND table_schema = 'public' \
         AND column_name NOT IN ('id', 'theme_id', 'theme_setting_modifiers') \
         ORDER BY ordinal_position",
    )
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter()
        .map(|(name, kind)| match kind.as_str() {
            "boolean" => Ok((name, true)),
            "ARRAY" => Ok((name, false)),
            _ => Err(Unsupported("a theme modifier column of another type").into()),
        })
        .collect()
}

/// A theme's modifier set, as the import changes it.
#[derive(Debug, Clone)]
struct ModifierSet {
    values: Vec<(String, Modifier)>,
    setting_modifiers: Option<Map<String, Value>>,
}

impl ModifierSet {
    async fn load(
        conn: &mut PgConnection,
        theme_id: Option<i32>,
        columns: &[(String, bool)],
    ) -> Result<ModifierSet, AppError> {
        let row: Option<Value> = match theme_id {
            Some(id) => {
                sqlx::query_scalar(
                    "SELECT to_jsonb(m) FROM theme_modifier_sets m WHERE theme_id = $1",
                )
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
            }
            None => None,
        };
        let row = row.unwrap_or(Value::Null);
        let values = columns
            .iter()
            .map(|(name, boolean)| {
                let v = &row[name.as_str()];
                let value = if *boolean {
                    Modifier::Bool(v.as_bool())
                } else {
                    Modifier::Strings(
                        v.as_array()
                            .map(|a| a.iter().map(|s| s.as_str().map(str::to_string)).collect()),
                    )
                };
                (name.clone(), value)
            })
            .collect();
        Ok(ModifierSet {
            values,
            setting_modifiers: row["theme_setting_modifiers"].as_object().cloned(),
        })
    }

    /// The column's writer for an about.json value (ActiveRecord's casts,
    /// and topic_thumbnail_sizes' own writer).
    fn assign(&mut self, name: &str, value: Option<&Value>) -> Result<(), AppError> {
        let Some(slot) = self.values.iter_mut().find(|(n, _)| n == name) else {
            return Ok(());
        };
        slot.1 = match (&slot.1, value) {
            (Modifier::Bool(_), None | Some(Value::Null)) => Modifier::Bool(None),
            (Modifier::Bool(_), Some(Value::String(s))) => {
                Modifier::Bool(crate::ruby::cast_boolean(Some(s)))
            }
            (Modifier::Bool(_), Some(Value::Bool(b))) => Modifier::Bool(Some(*b)),
            (Modifier::Bool(_), Some(Value::Number(n))) => {
                Modifier::Bool(Some(n.as_f64() != Some(0.0)))
            }
            (Modifier::Bool(_), Some(_)) => Modifier::Bool(Some(true)),
            (Modifier::Strings(_), None | Some(Value::Null)) => Modifier::Strings(None),
            (Modifier::Strings(_), Some(Value::Array(items))) => {
                let pairs = name == "topic_thumbnail_sizes"
                    && items
                        .iter()
                        .all(|i| i.as_array().is_some_and(|a| a.len() == 2));
                let cast = |v: &Value| -> Result<Option<String>, AppError> {
                    match v {
                        Value::Null => Ok(None),
                        Value::Array(_) | Value::Object(_) => {
                            Err(Unsupported("a theme modifier list holding lists or hashes").into())
                        }
                        other => cast_string(Some(other)),
                    }
                };
                let list: Result<Vec<Option<String>>, AppError> = if pairs && !items.is_empty() {
                    items
                        .iter()
                        .map(|pair| {
                            let pair = pair.as_array().map(Vec::as_slice).unwrap_or(&[]);
                            Ok(Some(format!(
                                "{}x{}",
                                ruby_to_s(pair.first()),
                                ruby_to_s(pair.get(1))
                            )))
                        })
                        .collect()
                } else {
                    items.iter().map(cast).collect()
                };
                Modifier::Strings(Some(list?))
            }
            (Modifier::Strings(_), Some(_)) => {
                return Err(Unsupported("a theme modifier list that is not a list").into());
            }
        };
        Ok(())
    }

    /// `add_theme_setting_modifier`
    fn add_setting_modifier(&mut self, name: &str, setting: Option<&Value>) {
        self.setting_modifiers
            .get_or_insert_with(Map::new)
            .insert(name.to_string(), setting.cloned().unwrap_or(Value::Null));
    }

    /// `type_validator`'s full messages.
    fn errors(&self) -> Vec<String> {
        self.values
            .iter()
            .filter(|(_, v)| matches!(v, Modifier::Strings(Some(items)) if items.iter().any(Option::is_none)))
            .map(|(name, _)| format!("{} is invalid", humanize(name)))
            .collect()
    }

    fn get(&self, name: &str) -> Option<&Modifier> {
        self.values.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// The row written: inserted for a theme without one.
    async fn save(&self, conn: &mut PgConnection, theme_id: i32) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO theme_modifier_sets (theme_id) SELECT $1 \
             WHERE NOT EXISTS (SELECT 1 FROM theme_modifier_sets WHERE theme_id = $1)",
        )
        .bind(theme_id)
        .execute(&mut *conn)
        .await?;
        for (name, value) in &self.values {
            let column = format!("\"{}\"", name.replace('"', ""));
            let cast = match value {
                Modifier::Bool(_) => "boolean",
                Modifier::Strings(_) => "varchar[]",
            };
            let sql = format!(
                "UPDATE theme_modifier_sets SET {column} = $2::{cast} WHERE theme_id = $1 \
                 AND {column} IS DISTINCT FROM $2::{cast}"
            );
            let query = sqlx::query(&sql).bind(theme_id);
            match value {
                Modifier::Bool(b) => query.bind(*b).execute(&mut *conn).await?,
                Modifier::Strings(list) => query.bind(list.clone()).execute(&mut *conn).await?,
            };
        }
        sqlx::query(
            "UPDATE theme_modifier_sets SET theme_setting_modifiers = $2 WHERE theme_id = $1 \
             AND theme_setting_modifiers IS DISTINCT FROM $2",
        )
        .bind(theme_id)
        .bind(self.setting_modifiers.clone().map(Value::Object))
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// `generate_metadata_hash`'s modifiers: the set ones, as their
    /// readers return them.
    fn metadata(&self) -> Map<String, Value> {
        let mut out = Map::new();
        for (name, value) in &self.values {
            let v = match value {
                Modifier::Bool(None) | Modifier::Strings(None) => continue,
                Modifier::Bool(Some(b)) => json!(b),
                Modifier::Strings(Some(items)) if name == "topic_thumbnail_sizes" => {
                    // topic_thumbnail_sizes' reader: "WxH" as [w, h].
                    let sizes: Vec<Value> = items
                        .iter()
                        .flatten()
                        .filter_map(|d| {
                            let parts: Vec<&str> = d.split('x').collect();
                            (parts.len() == 2).then(|| {
                                json!([crate::ruby::to_i(parts[0]), crate::ruby::to_i(parts[1])])
                            })
                        })
                        .collect();
                    Value::Array(sizes)
                }
                Modifier::Strings(Some(items)) => json!(items),
            };
            out.insert(name.clone(), v);
        }
        out
    }
}

/// ActiveSupport's `humanize` of an attribute name.
fn humanize(name: &str) -> String {
    let text = name.replace('_', " ");
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `ThemeModifierSet#refresh_theme_setting_modifiers` after an import:
/// each setting modifier from its setting's value. True when one changed.
async fn refresh_setting_modifiers(
    conn: &mut PgConnection,
    theme_id: i32,
    set: &mut ModifierSet,
) -> Result<bool, AppError> {
    let Some(modifiers) = set.setting_modifiers.clone().filter(|m| !m.is_empty()) else {
        return Ok(false);
    };
    let settings = themes::settings_of(conn, theme_id).await?;
    let stored = themes::stored_settings(conn, theme_id).await?;
    let mut changed = false;
    for (modifier, setting) in modifiers {
        let setting_name = ruby_to_s(Some(&setting));
        let value = settings.iter().find(|s| s.name == setting_name).map(|s| {
            let row = stored
                .iter()
                .find(|(n, t, _)| *n == s.name && *t == s.type_id)
                .and_then(|(_, _, v)| v.as_deref());
            theme_writes::setting_value(s, row)
        });
        let new = match set.get(&modifier) {
            Some(Modifier::Bool(_)) => Modifier::Bool(Some(ruby_to_s(value.as_ref()) != "false")),
            Some(Modifier::Strings(_)) => Modifier::Strings(Some(match &value {
                Some(Value::Array(items)) => {
                    items.iter().map(|i| Some(ruby_to_s(Some(i)))).collect()
                }
                other => ruby_to_s(other.as_ref())
                    .split('|')
                    .filter(|s| !s.is_empty())
                    .map(|s| Some(s.to_string()))
                    .collect(),
            })),
            None => continue,
        };
        if set.get(&modifier) != Some(&new)
            && let Some(slot) = set.values.iter_mut().find(|(n, _)| *n == modifier)
        {
            slot.1 = new;
            changed = true;
        }
    }
    Ok(changed)
}

// Color schemes

/// `normalize_override`: six hex digits, downcased, else nothing.
fn normalize_override(hex: Option<&Value>) -> Result<Option<String>, AppError> {
    match hex {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let lower = s.to_lowercase();
            Ok((lower.len() == 6 && lower.bytes().all(|b| b.is_ascii_hexdigit())).then_some(lower))
        }
        Some(_) => Err(crash(
            "NoMethodError: downcase on a color that is not a string",
        )),
    }
}

#[derive(Debug, Clone)]
struct ExistingScheme {
    id: i32,
    name: String,
    remote_copy: bool,
    base_scheme_id: Option<i32>,
}

/// `update_theme_color_schemes`: the theme's schemes from about.json, the
/// ones it no longer names destroyed. Returns the schemes in about.json's
/// order.
async fn update_color_schemes(
    conn: &mut PgConnection,
    theme_id: i32,
    existing_theme: bool,
    schemes: Option<&Value>,
) -> Result<Vec<i32>, AppError> {
    let existing: Vec<ExistingScheme> = if existing_theme {
        sqlx::query_as::<_, (i32, String, bool, Option<i32>)>(
            "SELECT id, name, remote_copy, base_scheme_id FROM color_schemes WHERE theme_id = $1 \
             ORDER BY id",
        )
        .bind(theme_id)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|(id, name, remote_copy, base_scheme_id)| ExistingScheme {
            id,
            name,
            remote_copy,
            base_scheme_id,
        })
        .collect()
    } else {
        Vec::new()
    };
    // missing_scheme_names: a name's last non-copy scheme.
    let mut missing: Vec<(String, ExistingScheme)> = Vec::new();
    for cs in existing.iter().filter(|cs| !cs.remote_copy) {
        match missing.iter_mut().find(|(n, _)| *n == cs.name) {
            Some(entry) => entry.1 = cs.clone(),
            None => missing.push((cs.name.clone(), cs.clone())),
        }
    }
    let entries: Vec<(String, Value)> = match schemes {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Some(_) => return Err(Unsupported("about.json color schemes that are not a hash").into()),
    };
    let mut ordered = Vec::new();
    for (name, colors) in &entries {
        missing.retain(|(n, _)| n != name);
        let colors = match colors {
            Value::Object(m) => m.clone(),
            Value::Null => Map::new(),
            _ => return Err(Unsupported("about.json colors that are not a hash").into()),
        };
        let found = existing
            .iter()
            .find(|cs| cs.name == *name && cs.remote_copy)
            .or_else(|| existing.iter().find(|cs| cs.name == *name));
        let (scheme_id, is_new) = match found {
            Some(cs) => (cs.id, false),
            None => {
                let id: i32 = sqlx::query_scalar(
                    "INSERT INTO color_schemes (name, version, theme_id, created_at, updated_at) \
                     VALUES ($1, 1, $2, now(), now()) RETURNING id",
                )
                .bind(name)
                .bind(theme_id)
                .fetch_one(&mut *conn)
                .await?;
                (id, true)
            }
        };
        let current: Vec<(i32, String, String)> = sqlx::query_as(
            "SELECT id, name, hex FROM color_scheme_colors WHERE color_scheme_id = $1 ORDER BY id",
        )
        .bind(scheme_id)
        .fetch_all(&mut *conn)
        .await?;
        let mut changed = false;
        for (color, base_hex) in crate::color_scheme::base_colors() {
            let hex = normalize_override(colors.get(color))?.unwrap_or_else(|| base_hex.clone());
            changed |= set_color(conn, scheme_id, &current, color, &hex).await?;
        }
        for variable in crate::color_scheme::COLOR_TRANSFORMATION_VARIABLES {
            let row = current.iter().find(|(_, n, _)| n == variable);
            match normalize_override(colors.get(variable))? {
                Some(hex) => {
                    changed |= set_color(conn, scheme_id, &current, variable, &hex).await?
                }
                None => {
                    if let Some((id, _, _)) = row {
                        sqlx::query("DELETE FROM color_scheme_colors WHERE id = $1")
                            .bind(id)
                            .execute(&mut *conn)
                            .await?;
                        changed = true;
                    }
                }
            }
        }
        // The theme's save saves the schemes it notified (bump_version).
        if changed && !is_new {
            sqlx::query(
                "UPDATE color_schemes SET version = version + 1, updated_at = now() WHERE id = $1",
            )
            .bind(scheme_id)
            .execute(&mut *conn)
            .await?;
        }
        ordered.push(scheme_id);
    }
    if !missing.is_empty() {
        let mut doomed = Vec::new();
        for (_, cs) in &missing {
            let copy = existing
                .iter()
                .find(|s| Some(s.id) == cs.base_scheme_id && s.remote_copy);
            doomed.push(copy.map_or(cs.id, |s| s.id));
        }
        destroy_color_schemes(conn, &doomed).await?;
    }
    Ok(ordered)
}

/// A scheme's color set to a hex, written when it changed.
async fn set_color(
    conn: &mut PgConnection,
    scheme_id: i32,
    current: &[(i32, String, String)],
    name: &str,
    hex: &str,
) -> Result<bool, AppError> {
    match current.iter().find(|(_, n, _)| n == name) {
        Some((_, _, existing)) if existing == hex => Ok(false),
        Some((id, _, _)) => {
            sqlx::query(
                "UPDATE color_scheme_colors SET hex = $2, updated_at = now() WHERE id = $1",
            )
            .bind(id)
            .bind(hex)
            .execute(&mut *conn)
            .await?;
            Ok(true)
        }
        None => {
            sqlx::query(
                "INSERT INTO color_scheme_colors (name, hex, color_scheme_id, created_at, updated_at) \
                 VALUES ($1, $2, $3, now(), now())",
            )
            .bind(name)
            .bind(hex)
            .bind(scheme_id)
            .execute(&mut *conn)
            .await?;
            Ok(true)
        }
    }
}

/// `ColorScheme.unscoped.where(id:).destroy_all`: each with its colors,
/// then after_destroy's remote original.
async fn destroy_color_schemes(conn: &mut PgConnection, ids: &[i32]) -> Result<(), AppError> {
    let rows: Vec<(i32, Option<i32>, Option<i32>)> = sqlx::query_as(
        "SELECT id, theme_id, base_scheme_id FROM color_schemes WHERE id = ANY($1) ORDER BY id",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    for (id, theme_id, base_scheme_id) in rows {
        sqlx::query("DELETE FROM color_scheme_colors WHERE color_scheme_id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM color_schemes WHERE id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
        if let (Some(theme_id), Some(base)) = (theme_id, base_scheme_id.filter(|b| *b >= 0)) {
            let copies: Vec<i32> = sqlx::query_scalar(
                "SELECT id FROM color_schemes WHERE theme_id = $1 AND id = $2 AND remote_copy",
            )
            .bind(theme_id)
            .bind(base)
            .fetch_all(&mut *conn)
            .await?;
            if !copies.is_empty() {
                Box::pin(destroy_color_schemes(conn, &copies)).await?;
            }
        }
    }
    Ok(())
}

// Uploads

/// `FileHelper.sanitize_filename`
fn sanitize_filename(name: &str) -> String {
    let stripped = crate::ruby::strip(name);
    let base = stripped
        .rfind(['/', '\\'])
        .map_or(stripped, |i| &stripped[i + 1..]);
    let replaced: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut out = String::new();
    for c in replaced.chars() {
        if c == '_' && out.ends_with('_') {
            continue;
        }
        out.push(c);
    }
    out
}

/// `SecureRandom.hex`
fn random_hex() -> String {
    use rand::Rng;
    let bytes: [u8; 16] = rand::thread_rng().r#gen();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What an import uploads with.
pub struct Uploader<'a> {
    pub cx: &'a themes::Ctx<'a>,
    pub staff: bool,
}

impl Uploader<'_> {
    /// `RemoteTheme.create_upload`: the file renamed in the folder, then
    /// uploaded for the theme under its relative path's basename. The
    /// upload's id, or its errors' full messages.
    async fn create_upload(
        &self,
        conn: &mut PgConnection,
        bundle: &mut Bundle,
        user_id: i32,
        path: &str,
        relative_path: &str,
        skip_validations: bool,
    ) -> Result<Result<i32, Vec<String>>, AppError> {
        let renamed = format!(
            "{}{}{}{}",
            parent(path),
            if parent(path).is_empty() { "" } else { "/" },
            random_hex(),
            extname(path)
        );
        bundle.rename(path, &renamed);
        let bytes = match bundle.nodes.get(&renamed) {
            Some(Node::File(bytes)) => bytes.clone(),
            _ => return Err(Unsupported("a theme asset that is a directory").into()),
        };
        let store = crate::file_store::FileStore::for_site(self.cx.urls.config, self.cx.settings)?;
        let outcome = crate::uploads::create(
            conn,
            self.cx.settings,
            self.cx.i18n,
            self.cx.urls,
            &store,
            &crate::uploads::NewUpload {
                user_id,
                staff: self.staff,
                upload_type: "theme",
                filename: basename(relative_path),
                bytes: &bytes,
                pasted: false,
                for_private_message: false,
                for_export: false,
                for_theme: true,
                skip_validations,
            },
        )
        .await?;
        Ok(match outcome {
            crate::uploads::Outcome::Created(upload) => Ok(upload["id"]
                .as_i64()
                .and_then(|id| i32::try_from(id).ok())
                .ok_or(Unsupported("an upload without its id"))?),
            crate::uploads::Outcome::Invalid(errors) => Err(errors),
        })
    }
}

// Theme files

/// `ThemeField.opts_from_file_path`: target, name and type of a bundle
/// file, by FILE_MATCHERS.
pub fn opts_from_file_path(path: &str) -> Option<(&'static str, String, i32)> {
    let basic = ["mobile", "desktop", "common"];
    if let Some((target, rest)) = path.split_once('/')
        && let Some(target) = basic.iter().find(|t| **t == target)
    {
        if let Some(name) = rest.strip_suffix(".html")
            && ["head_tag", "header", "after_header", "body_tag", "footer"].contains(&name)
        {
            return Some((target, name.to_string(), 0));
        }
        if rest == format!("{target}.scss") {
            return Some((target, "scss".to_string(), 1));
        }
    }
    match path {
        "common/embedded.scss" => return Some(("common", "embedded_scss".into(), 1)),
        "common/color_definitions.scss" => {
            return Some(("common", "color_definitions".into(), 1));
        }
        "about.json" => return Some(("about", "about".into(), 8)),
        "settings.yml" | "settings.yaml" => return Some(("settings", "yaml".into(), 5)),
        _ => {}
    }
    let one_line = |s: &str| !s.is_empty() && !s.contains('\n');
    for prefix in ["scss/", "stylesheets/"] {
        if let Some(name) = path
            .strip_prefix(prefix)
            .and_then(|r| r.strip_suffix(".scss"))
            .filter(|n| one_line(n))
        {
            return Some(("extra_scss", name.to_string(), 1));
        }
    }
    if let Some(name) = path.strip_prefix("javascripts/").filter(|n| one_line(n)) {
        return Some(("extra_js", name.to_string(), 6));
    }
    if let Some(name) = path.strip_prefix("test/").filter(|n| one_line(n)) {
        return Some(("tests_js", name.to_string(), 6));
    }
    if let Some(name) = path
        .strip_prefix("locales/")
        .and_then(|r| r.strip_suffix(".yml"))
        .filter(|n| themes::locale_available(n))
    {
        return Some(("translations", name.to_string(), 5));
    }
    if let Some(name) = path
        .strip_prefix("migrations/settings/")
        .and_then(|r| r.strip_suffix(".js"))
        .filter(|n| !n.is_empty() && !n.contains('/'))
    {
        return Some(("migrations", name.to_string(), 6));
    }
    None
}

/// What the import is asked to do.
pub struct ZipImport<'a> {
    pub bytes: &'a [u8],
    pub original_filename: &'a str,
    pub user_id: i32,
    /// `params[:theme_id]`
    pub theme_id: Option<String>,
    /// `params[:components]`
    pub update_components: Option<String>,
}

/// ActiveModel's integer cast of a param: nil when blank or not numeric.
fn cast_id(raw: &str) -> Option<i32> {
    let trimmed = raw.trim_start();
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    if crate::ruby::is_blank(raw) || !digits.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    i32::try_from(crate::ruby::to_i(raw)).ok()
}

/// `RemoteTheme.update_zipped_theme`: the bundle imported as a new theme,
/// or into the theme `theme_id` names. Runs in the caller's transaction,
/// which an ImportError rolls back.
pub async fn update_zipped_theme(
    conn: &mut PgConnection,
    uploader: &Uploader<'_>,
    import: &ZipImport<'_>,
) -> Result<Result<Theme, ImportError>, AppError> {
    let cx = uploader.cx;
    let i18n = cx.i18n;
    let max_mb = cx
        .settings
        .get("decompressed_theme_max_file_size_mb")?
        .to_f();
    let mut bundle = match unpack(import.bytes, import.original_filename, max_mb)? {
        Ok(bundle) => bundle,
        Err(UnpackError::Failed) => {
            return Ok(Err(ImportError(t(
                i18n,
                "themes.import_error.unpack_failed",
                &[],
            ))));
        }
        Err(UnpackError::TooBig) => {
            return Ok(Err(ImportError(t(
                i18n,
                "themes.import_error.file_too_big",
                &[],
            ))));
        }
    };
    let info = match extract_theme_info(&bundle, i18n)? {
        Ok(info) => info,
        Err(e) => return Ok(Err(e)),
    };
    let requested_id = import.theme_id.as_deref().and_then(cast_id);
    let existing = match (import.theme_id.as_ref(), requested_id) {
        (Some(_), Some(id)) => Theme::find(conn, id).await?,
        _ => None,
    };
    let component = ruby_to_s(info.get("component")) == "true";
    let child_components = present(info.get("components")).cloned();
    let skip_child_components = import.update_components.as_deref() == Some("none");

    // A new theme has no row (nor fields) until its save: its fields are
    // looked up under an id no theme has.
    let (lookup_id, theme_user_id, is_new) = match &existing {
        Some(t) => (t.id, t.user_id, false),
        None => (i32::MIN, import.user_id, true),
    };

    // update_from_remote(importer, skip_update: true)
    let info = match extract_theme_info(&bundle, i18n)? {
        Ok(info) => info,
        Err(e) => return Ok(Err(e)),
    };
    let mut changes: Vec<FieldChange> = Vec::new();
    let mut kept: Vec<(i32, i32, String)> = Vec::new();
    let mut set = |key: Option<(i32, i32, String)>| {
        if let Some(key) = key {
            kept.push(key);
        }
    };
    match info.get("assets") {
        None | Some(Value::Null) => {}
        Some(Value::Object(assets)) => {
            for (name, relative) in assets {
                let Value::String(relative) = relative else {
                    return Err(Unsupported("about.json assets that are not paths").into());
                };
                let Some(path) = bundle.real_path(relative) else {
                    continue;
                };
                match uploader
                    .create_upload(conn, &mut bundle, theme_user_id, &path, relative, false)
                    .await?
                {
                    Ok(upload_id) => {
                        let input = FieldInput {
                            target: "common".into(),
                            name: name.clone(),
                            value: None,
                            type_id: Some(theme_writes::UPLOAD_VAR),
                            upload_id: Some(upload_id),
                        };
                        set(theme_writes::set_field(conn, lookup_id, &input, &mut changes).await?);
                    }
                    Err(errors) => {
                        return Ok(Err(ImportError(t(
                            i18n,
                            "themes.import_error.upload",
                            &[("name", name), ("errors", &errors.join(","))],
                        ))));
                    }
                }
            }
        }
        Some(_) => return Err(Unsupported("about.json assets that are not a hash").into()),
    }
    // ThemeScreenshotsHandler#parse_screenshots_as_theme_fields!
    let screenshots: Vec<Value> = match info.get("screenshots") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(other) => vec![other.clone()],
    };
    for relative in screenshots.iter().take(MAX_SCREENSHOT_COUNT) {
        let Value::String(relative) = relative else {
            return Err(Unsupported("about.json screenshots that are not paths").into());
        };
        let Some(path) = bundle.real_path(relative) else {
            continue;
        };
        let filename = basename(&path).to_string();
        let extension = extname(&path).to_string();
        if !SCREENSHOT_TYPES.contains(&extension.as_str()) {
            return Ok(Err(ImportError(t(
                i18n,
                "themes.import_error.screenshot_invalid_type",
                &[
                    ("file_name", &filename),
                    ("accepted_formats", &SCREENSHOT_TYPES.join(",")),
                ],
            ))));
        }
        let bytes = bundle.read(&path).map(<[u8]>::to_vec).unwrap_or_default();
        if bytes.len() > MAX_SCREENSHOT_FILE_SIZE {
            return Ok(Err(ImportError(t(
                i18n,
                "themes.import_error.screenshot_invalid_size",
                &[
                    ("file_name", &filename),
                    (
                        "max_size",
                        &crate::uploads::human_size(MAX_SCREENSHOT_FILE_SIZE as i64),
                    ),
                ],
            ))));
        }
        let size = crate::images::info(&bytes).map(|i| (i.width, i.height));
        let (width, height) = size.unwrap_or((0, 0));
        if size.is_none()
            || width > MAX_SCREENSHOT_DIMENSIONS.0
            || height > MAX_SCREENSHOT_DIMENSIONS.1
        {
            return Ok(Err(ImportError(t(
                i18n,
                "themes.import_error.screenshot_invalid_dimensions",
                &[
                    ("file_name", &filename),
                    ("width", &width.to_string()),
                    ("height", &height.to_string()),
                    ("max_width", &MAX_SCREENSHOT_DIMENSIONS.0.to_string()),
                    ("max_height", &MAX_SCREENSHOT_DIMENSIONS.1.to_string()),
                ],
            ))));
        }
        match uploader
            .create_upload(conn, &mut bundle, theme_user_id, &path, relative, true)
            .await?
        {
            Ok(upload_id) => {
                let clean = sanitize_filename(&filename.replace(&extension, ""));
                let input = FieldInput {
                    target: "common".into(),
                    name: format!("screenshot_{clean}"),
                    value: None,
                    type_id: Some(theme_writes::SCREENSHOT_UPLOAD_VAR),
                    upload_id: Some(upload_id),
                };
                set(theme_writes::set_field(conn, lookup_id, &input, &mut changes).await?);
            }
            Err(errors) => {
                return Ok(Err(ImportError(t(
                    i18n,
                    "themes.import_error.screenshot",
                    &[("errors", &errors.join(","))],
                ))));
            }
        }
    }
    let metadata = Metadata::from_info(&info)?;
    let errors = metadata.errors();
    if !errors.is_empty() {
        return Ok(Err(ImportError(t(
            i18n,
            "themes.import_error.about_json_values",
            &[("errors", &errors.join(","))],
        ))));
    }
    let columns = modifier_columns(conn).await?;
    let mut modifiers = ModifierSet::load(conn, existing.as_ref().map(|t| t.id), &columns).await?;
    let modifier_info = match info.get("modifiers") {
        None | Some(Value::Null) => None,
        Some(Value::Object(m)) => Some(m.clone()),
        Some(_) => return Err(crash("TypeError: dig on about.json's modifiers")),
    };
    for (name, _) in &columns {
        let value = modifier_info.as_ref().and_then(|m| m.get(name));
        match value {
            Some(Value::Object(h)) if h.get("type") == Some(&json!("setting")) => {
                modifiers.add_setting_modifier(name, h.get("value"));
            }
            other => modifiers.assign(name, other)?,
        }
    }
    let errors = modifiers.errors();
    if !errors.is_empty() {
        return Ok(Err(ImportError(t(
            i18n,
            "themes.import_error.modifier_values",
            &[("errors", &errors.join(","))],
        ))));
    }
    let files = bundle.all_files();
    if files.len() > MAX_THEME_FILE_COUNT {
        return Ok(Err(ImportError(t(
            i18n,
            "themes.import_error.too_many_files",
            &[
                ("count", &files.len().to_string()),
                ("limit", &MAX_THEME_FILE_COUNT.to_string()),
            ],
        ))));
    }
    let mut theme_size = 0;
    for filename in &files {
        let Some((target, name, type_id)) = opts_from_file_path(filename) else {
            continue;
        };
        let file_size = bundle.file_size(filename);
        if file_size > MAX_ASSET_FILE_SIZE {
            return Ok(Err(ImportError(t(
                i18n,
                "themes.import_error.asset_too_big",
                &[
                    ("filename", filename),
                    ("limit", &crate::uploads::human_size(MAX_ASSET_FILE_SIZE)),
                ],
            ))));
        }
        theme_size += file_size;
        if theme_size > MAX_THEME_SIZE {
            return Ok(Err(ImportError(t(
                i18n,
                "themes.import_error.theme_too_big",
                &[("limit", &crate::uploads::human_size(MAX_THEME_SIZE))],
            ))));
        }
        let bytes = bundle.read(filename).unwrap_or_default();
        let value = String::from_utf8(bytes.to_vec())
            .map_err(|_| crash("ArgumentError: a theme file that is not UTF-8"))?;
        let input = FieldInput {
            target: target.into(),
            name,
            value: Some(value),
            type_id: Some(type_id),
            upload_id: None,
        };
        set(theme_writes::set_field(conn, lookup_id, &input, &mut changes).await?);
    }

    // The transaction block: the fields the bundle lost destroyed.
    if !is_new {
        let fields: Vec<(i32, i32, i32, String)> = sqlx::query_as(
            "SELECT id, target_id, type_id, name FROM theme_fields WHERE theme_id = $1 ORDER BY id",
        )
        .bind(lookup_id)
        .fetch_all(&mut *conn)
        .await?;
        for (id, target_id, type_id, name) in fields {
            let key = (target_id, type_id, name.clone());
            if kept.contains(&key) || changes.iter().any(|c| c.key() == key) {
                continue;
            }
            changes.push(FieldChange::Destroy {
                id,
                target_id,
                name,
                type_id,
            });
        }
    }
    // save!: the remote theme, never linked to the theme.
    sqlx::query(
        "INSERT INTO remote_themes (remote_url, about_url, license_url, authors, theme_version, \
         minimum_discourse_version, maximum_discourse_version, created_at, updated_at) \
         VALUES ('', $1, $2, $3, $4, $5, $6, now(), now())",
    )
    .bind(&metadata.about_url)
    .bind(&metadata.license_url)
    .bind(&metadata.authors)
    .bind(&metadata.theme_version)
    .bind(&metadata.minimum_discourse_version)
    .bind(&metadata.maximum_discourse_version)
    .execute(&mut *conn)
    .await?;
    // theme.save!: its validations, then the row and its after_save.
    let mut errors = Vec::new();
    if let Some(t) = &existing
        && component
    {
        let default = cx.settings.get("default_theme_id")?.to_i() == i64::from(t.id);
        if t.color_scheme_id.is_some() || t.user_selectable || default {
            errors.push("component");
        }
    }
    if !errors.is_empty() || !theme_writes::field_errors(&changes).is_empty() {
        return Err(crash("ActiveRecord::RecordInvalid on the imported theme"));
    }
    let theme_id = match &existing {
        Some(t) => {
            if t.component != component {
                sqlx::query("UPDATE themes SET component = $2, updated_at = now() WHERE id = $1")
                    .bind(t.id)
                    .bind(component)
                    .execute(&mut *conn)
                    .await?;
            }
            t.id
        }
        None => insert_theme(conn, &info, requested_id, import.user_id, component).await?,
    };
    modifiers.save(conn, theme_id).await?;
    let ordered_schemes = if component {
        Vec::new()
    } else {
        update_color_schemes(conn, theme_id, !is_new, info.get("color_schemes")).await?
    };
    if is_new && !ordered_schemes.is_empty() {
        let only_theme = matches!(
            modifiers.get("only_theme_color_schemes"),
            Some(Modifier::Bool(Some(true)))
        );
        let (light, dark) = if only_theme {
            let mut darkness = Vec::new();
            for id in &ordered_schemes {
                let scheme = crate::color_scheme::ColorScheme::find(conn, *id)
                    .await?
                    .ok_or(Unsupported("a color scheme gone after its import"))?;
                darkness.push((*id, scheme.is_dark()?.unwrap_or(false)));
            }
            let light = darkness
                .iter()
                .find(|(_, d)| !d)
                .map_or(ordered_schemes[0], |(i, _)| *i);
            let dark = darkness
                .iter()
                .find(|(_, d)| *d)
                .map_or(ordered_schemes[0], |(i, _)| *i);
            (Some(light), Some(dark))
        } else {
            (Some(ordered_schemes[0]), None)
        };
        sqlx::query(
            "UPDATE themes SET color_scheme_id = $2, dark_color_scheme_id = $3, updated_at = now() \
             WHERE id = $1",
        )
        .bind(theme_id)
        .bind(light)
        .bind(dark)
        .execute(&mut *conn)
        .await?;
    }
    {
        let env = theme_writes::Env::load(conn, cx).await?;
        theme_writes::after_save(conn, &env, theme_id, &changes, false).await?;
    }
    if is_new && !component && !skip_child_components && child_components.is_some() {
        return Err(Unsupported(
            "child components named by about.json (each a git import over the network)",
        )
        .into());
    }
    let theme = Theme::find(conn, theme_id)
        .await?
        .ok_or(Unsupported("a theme gone after its import"))?;
    create_theme_site_settings(conn, cx, &theme, info.get("theme_site_settings")).await?;
    if refresh_setting_modifiers(conn, theme_id, &mut modifiers).await? {
        modifiers.save(conn, theme_id).await?;
    }
    // update_components on an existing theme: the child components by url.
    if !is_new
        && let Some(mode) = import
            .update_components
            .as_deref()
            .filter(|m| !m.is_empty() && *m != "none")
    {
        let urls: Vec<String> = match &child_components {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|u| normalize_git_url(crate::ruby::strip(&ruby_to_s(Some(u)))))
                .collect(),
            Some(_) => return Err(crash("NoMethodError: map on about.json's components")),
        };
        if mode == "sync" {
            sqlx::query(
                "DELETE FROM child_themes WHERE child_theme_id IN (SELECT t.id FROM themes t \
                 JOIN remote_themes r ON r.id = t.remote_theme_id WHERE NOT (r.remote_url = ANY($1)))",
            )
            .bind(&urls)
            .execute(&mut *conn)
            .await?;
        }
        let present_urls: Vec<String> = sqlx::query_scalar(
            "SELECT r.remote_url FROM child_themes c JOIN themes t ON t.id = c.child_theme_id \
             JOIN remote_themes r ON r.id = t.remote_theme_id WHERE c.parent_theme_id = $1 \
             AND r.remote_url = ANY($2)",
        )
        .bind(theme_id)
        .bind(&urls)
        .fetch_all(&mut *conn)
        .await?;
        let remaining = urls.iter().any(|u| !present_urls.contains(u));
        if !theme.component && remaining {
            return Err(Unsupported(
                "child components named by about.json (each a git import over the network)",
            )
            .into());
        }
    }
    Ok(Ok(theme))
}

/// The new theme's row: `Theme.new(id: theme_id, name:, auto_update:
/// false)` saved.
async fn insert_theme(
    conn: &mut PgConnection,
    info: &Map<String, Value>,
    requested_id: Option<i32>,
    user_id: i32,
    component: bool,
) -> Result<i32, AppError> {
    let name = match info.get("name") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => {
            return Err(crash("PG::NotNullViolation: a theme without a name"));
        }
        Some(other) => other.to_string(),
    };
    let id: i32 = match requested_id {
        Some(id) => {
            sqlx::query_scalar(
                "INSERT INTO themes (id, name, user_id, auto_update, component, created_at, \
                 updated_at) VALUES ($1, $2, $3, FALSE, $4, now(), now()) RETURNING id",
            )
            .bind(id)
            .bind(&name)
            .bind(user_id)
            .bind(component)
            .fetch_one(&mut *conn)
            .await?
        }
        None => {
            sqlx::query_scalar(
                "INSERT INTO themes (name, user_id, auto_update, component, created_at, \
                 updated_at) VALUES ($1, $2, FALSE, $3, now(), now()) RETURNING id",
            )
            .bind(&name)
            .bind(user_id)
            .bind(component)
            .fetch_one(&mut *conn)
            .await?
        }
    };
    Ok(id)
}

/// `RemoteTheme#create_theme_site_settings`: the themeable settings
/// about.json sets that the theme doesn't have yet, by the system user.
async fn create_theme_site_settings(
    conn: &mut PgConnection,
    cx: &themes::Ctx<'_>,
    theme: &Theme,
    settings: Option<&Value>,
) -> Result<(), AppError> {
    let entries = match settings {
        None | Some(Value::Null) => return Ok(()),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => {
            return Err(crash(
                "NoMethodError: keys on about.json's theme_site_settings",
            ));
        }
    };
    for (name, value) in entries {
        if !themes::THEMEABLE_SITE_SETTINGS.contains(&name.as_str()) {
            continue;
        }
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM theme_site_settings WHERE theme_id = $1 AND name = $2)",
        )
        .bind(theme.id)
        .bind(&name)
        .fetch_one(&mut *conn)
        .await?;
        if exists {
            continue;
        }
        let raw = match value {
            Value::Null => None,
            Value::String(s) => Some(s),
            other => Some(other.to_string()),
        };
        theme_writes::set_theme_site_setting(conn, cx.defs, theme, &name, raw, -1)
            .await?
            .map_err(|message| crash(&format!("Discourse::InvalidParameters: {message}")))?;
    }
    Ok(())
}

// Git

/// `GitUrl.normalize`
pub fn normalize_git_url(url: &str) -> String {
    static SSH: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\A([A-Za-z0-9_]+@[A-Za-z0-9_]+(?:\.[A-Za-z0-9_]+)*):([^\n]*)\z")
            .expect("valid regex")
    });
    let mut url = match SSH.captures(url) {
        Some(c) => format!("ssh://{}/{}", &c[1], &c[2]),
        None => url.to_string(),
    };
    if url.starts_with("https://github.com/") && !url.ends_with(".git") {
        if url.ends_with('/') {
            url.pop();
        }
        url.push_str(".git");
    }
    url
}

/// `URI.parse`'s scheme, Err when Ruby can't parse the url.
fn uri_scheme(url: &str) -> Result<Option<&str>, ()> {
    let mut chars = url.bytes().peekable();
    let mut hashes = 0;
    while let Some(b) = chars.next() {
        match b {
            b'%' => {
                let ok = matches!(chars.next(), Some(h) if h.is_ascii_hexdigit())
                    && matches!(chars.next(), Some(h) if h.is_ascii_hexdigit());
                if !ok {
                    return Err(());
                }
            }
            b'#' => hashes += 1,
            b if b.is_ascii_alphanumeric() || b"-._~:/?[]@!$&'()*+,;=".contains(&b) => {}
            _ => return Err(()),
        }
    }
    if hashes > 1 {
        return Err(());
    }
    let end = url.find([':', '/', '?', '#']);
    Ok(match end {
        Some(i) if url.as_bytes()[i] == b':' && i > 0 => {
            let scheme = &url[..i];
            let valid = scheme.as_bytes()[0].is_ascii_alphabetic()
                && scheme
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'));
            if valid { Some(scheme) } else { return Err(()) }
        }
        _ => None,
    })
}

/// `ThemeStore::GitImporter#import!` up to the clone: the ImportError of a
/// url it can't clone. A url it would clone is refused (a network fetch).
pub fn git_import(
    i18n: &crate::i18n::I18n,
    url: &str,
    private_key: Option<&str>,
) -> Result<ImportError, AppError> {
    let url = normalize_git_url(url);
    let git_error = || ImportError(t(i18n, "themes.import_error.git", &[]));
    match uri_scheme(&url) {
        Err(()) => Ok(git_error()),
        Ok(Some("http" | "https")) => {
            Err(Unsupported("cloning a remote theme over http(s) (a network fetch)").into())
        }
        Ok(Some("ssh")) => {
            if private_key.is_none_or(crate::ruby::is_blank) {
                Ok(git_error())
            } else {
                Err(Unsupported("cloning a remote theme over ssh (a network fetch)").into())
            }
        }
        Ok(_) => Ok(ImportError(t(
            i18n,
            "themes.import_error.git_unsupported_scheme",
            &[],
        ))),
    }
}

/// Jobs::CheckOutOfDateThemes
pub const CHECK_OUT_OF_DATE_THEMES: &str = "check_out_of_date_themes";

/// Jobs::CheckOutOfDateThemes: each git theme's `update_remote_version`,
/// a fetch of its repository, refused (nothing to do without one).
pub async fn check_out_of_date_themes(state: &crate::AppState) -> Result<(), AppError> {
    let git_themes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM remote_themes r JOIN themes t ON t.remote_theme_id = r.id \
         WHERE r.remote_url <> ''",
    )
    .fetch_one(&state.pool)
    .await?;
    if git_themes > 0 {
        return Err(Unsupported(
            "checking git themes for updates (update_remote_version fetches each repository)",
        )
        .into());
    }
    Ok(())
}

/// `create_remote_theme_placeholder`: a remote theme and its theme named
/// for the repository, nothing cloned.
pub async fn create_placeholder(
    conn: &mut PgConnection,
    remote: &str,
    branch: Option<&str>,
    private_key: Option<&str>,
    user_id: i32,
) -> Result<Theme, AppError> {
    let remote_id: i32 = sqlx::query_scalar(
        "INSERT INTO remote_themes (remote_url, branch, private_key, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now()) RETURNING id",
    )
    .bind(remote)
    .bind(branch)
    .bind(private_key)
    .fetch_one(&mut *conn)
    .await?;
    let trimmed = remote.strip_suffix(".git").unwrap_or(remote);
    // String#split drops trailing empty fields.
    let name = trimmed
        .split('/')
        .rev()
        .find(|s| !s.is_empty())
        .ok_or_else(|| crash("PG::NotNullViolation: a placeholder without a name"))?;
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO themes (name, user_id, remote_theme_id, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now()) RETURNING id",
    )
    .bind(name)
    .bind(user_id)
    .bind(remote_id)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query("INSERT INTO theme_modifier_sets (theme_id) VALUES ($1)")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Theme::find(conn, id)
        .await?
        .ok_or_else(|| Unsupported("a theme gone after its create").into())
}

/// `SSHKey.generate`: a 2048 bit RSA key, its OpenSSH public key
/// (`ssh-rsa <wire format>`) and its PKCS#1 PEM private key.
pub fn generate_ssh_key() -> Result<(String, String), AppError> {
    use rsa::pkcs1::EncodeRsaPrivateKey;
    let key = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048)
        .map_err(|_| crash("SSHKey.generate failed"))?;
    let private = key
        .to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)
        .map_err(|_| crash("SSHKey#private_key failed"))?
        .to_string();
    Ok((ssh_public_key(&key.to_public_key()), private))
}

/// The OpenSSH line of an RSA public key.
fn ssh_public_key(key: &rsa::RsaPublicKey) -> String {
    use base64::Engine;
    use rsa::traits::PublicKeyParts;
    fn put_string(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(bytes);
    }
    // mpint: big-endian, a zero byte ahead of a set high bit.
    fn put_mpint(out: &mut Vec<u8>, bytes: &[u8]) {
        let trimmed: &[u8] = {
            let start = bytes.iter().position(|b| *b != 0).unwrap_or(bytes.len());
            &bytes[start..]
        };
        if trimmed.first().is_some_and(|b| b & 0x80 != 0) {
            let mut padded = vec![0];
            padded.extend_from_slice(trimmed);
            put_string(out, &padded);
        } else {
            put_string(out, trimmed);
        }
    }
    let mut wire = Vec::new();
    put_string(&mut wire, b"ssh-rsa");
    put_mpint(&mut wire, &key.e().to_bytes_be());
    put_mpint(&mut wire, &key.n().to_bytes_be());
    format!(
        "ssh-rsa {}",
        base64::engine::general_purpose::STANDARD.encode(wire)
    )
}

// Export

/// `ThemeStore::ZipExporter#export_name`
pub fn export_name(theme_name: &str) -> String {
    let name: String = theme_name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_digit() || c.is_ascii_lowercase() || c == '.' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if name.starts_with("discourse") {
        name
    } else {
        format!("discourse-{name}")
    }
}

/// `Theme#generate_metadata_hash`
async fn metadata_hash(conn: &mut PgConnection, theme: &Theme) -> Result<Value, AppError> {
    let mut meta = Map::new();
    meta.insert("name".into(), json!(theme.name));
    meta.insert("component".into(), json!(theme.component));
    let remote = themes::RemoteTheme::find(conn, theme.remote_theme_id).await?;
    let property = |v: Option<&Option<String>>| -> Value {
        match v.and_then(|v| v.as_deref()) {
            Some("URL") | None => Value::Null,
            Some(s) => json!(s),
        }
    };
    let r = remote.as_ref();
    meta.insert("license_url".into(), property(r.map(|r| &r.license_url)));
    meta.insert("about_url".into(), property(r.map(|r| &r.about_url)));
    meta.insert("authors".into(), property(r.map(|r| &r.authors)));
    meta.insert(
        "theme_version".into(),
        property(r.map(|r| &r.theme_version)),
    );
    meta.insert(
        "minimum_discourse_version".into(),
        property(r.map(|r| &r.minimum_discourse_version)),
    );
    meta.insert(
        "maximum_discourse_version".into(),
        property(r.map(|r| &r.maximum_discourse_version)),
    );
    // The unique index's order (theme_id, target_id, type_id, name).
    let assets: Vec<(i32, i32, i32, String, String, Option<i32>)> = sqlx::query_as(
        "SELECT id, target_id, type_id, name, value, upload_id FROM theme_fields \
         WHERE theme_id = $1 AND type_id = $2 ORDER BY target_id, type_id, name",
    )
    .bind(theme.id)
    .bind(theme_writes::UPLOAD_VAR)
    .fetch_all(&mut *conn)
    .await?;
    let mut asset_map = Map::new();
    for (id, target_id, type_id, name, value, upload_id) in assets {
        let field = themes::Field {
            id,
            name: name.clone(),
            target_id,
            type_id,
            value,
            error: None,
            upload_id,
        };
        asset_map.insert(name, json!(themes::field_file_path(conn, &field).await?));
    }
    meta.insert("assets".into(), Value::Object(asset_map));
    let mut scheme_ids: Vec<i32> = Vec::new();
    if let Some(id) = theme.color_scheme_id {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM color_schemes WHERE id = $1 AND NOT remote_copy)",
        )
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
        if exists {
            scheme_ids.push(id);
        }
    }
    let own: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM color_schemes WHERE theme_id = $1 AND NOT remote_copy ORDER BY id",
    )
    .bind(theme.id)
    .fetch_all(&mut *conn)
    .await?;
    for id in own {
        if !scheme_ids.contains(&id) {
            scheme_ids.push(id);
        }
    }
    let mut schemes = Map::new();
    for id in scheme_ids {
        let name: String = sqlx::query_scalar("SELECT name FROM color_schemes WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
        let colors: Vec<(String, String)> = sqlx::query_as(
            "SELECT name, hex FROM color_scheme_colors WHERE color_scheme_id = $1 ORDER BY id",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
        let mut map = Map::new();
        for (n, hex) in colors {
            map.insert(n, json!(hex));
        }
        schemes.insert(name, Value::Object(map));
    }
    meta.insert("color_schemes".into(), Value::Object(schemes));
    let columns = modifier_columns(conn).await?;
    let modifiers = ModifierSet::load(conn, Some(theme.id), &columns).await?;
    meta.insert("modifiers".into(), Value::Object(modifiers.metadata()));
    meta.insert(
        "learn_more".into(),
        json!("https://meta.discourse.org/t/beginners-guide-to-using-discourse-themes/91966"),
    );
    Ok(Value::Object(meta))
}

/// `ThemeStore::ZipExporter#package_filename`: the zip's name and bytes,
/// every field with a file path and the generated about.json.
pub async fn export(
    conn: &mut PgConnection,
    store: &crate::file_store::FileStore,
    theme: &Theme,
) -> Result<(String, Vec<u8>), AppError> {
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for field in themes::fields(conn, theme.id).await? {
        let Some(path) = themes::field_file_path(conn, &field).await? else {
            continue;
        };
        // Belt and braces: a path that leaves the export directory.
        let Some(path) = safe_join(&path).filter(|p| !p.is_empty()) else {
            return Err(crash("Theme exporter tried to leave directory"));
        };
        let content = if field.type_id == theme_writes::UPLOAD_VAR {
            let upload: Option<String> = match field.upload_id {
                Some(id) => {
                    sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
                        .bind(id)
                        .fetch_optional(&mut *conn)
                        .await?
                }
                None => None,
            };
            let url = upload.ok_or_else(|| crash("NoMethodError: content on a missing upload"))?;
            store
                .read(&url)
                .await?
                .ok_or_else(|| crash("Errno::ENOENT: an exported upload's file"))?
        } else {
            field.value.into_bytes()
        };
        files.insert(path, content);
    }
    let about = serde_json::to_string_pretty(&metadata_hash(conn, theme).await?)
        .map_err(std::io::Error::other)?;
    files.insert("about.json".into(), about.into_bytes());

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let zip_err = |_| crash("the theme export's zip writer failed");
    let mut dirs: Vec<String> = Vec::new();
    for path in files.keys() {
        let mut prefix = String::new();
        let parts: Vec<&str> = path.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            prefix.push_str(part);
            prefix.push('/');
            if !dirs.contains(&prefix) {
                dirs.push(prefix.clone());
            }
        }
    }
    for dir in &dirs {
        zip.add_directory(dir.as_str(), options).map_err(zip_err)?;
    }
    for (path, content) in &files {
        zip.start_file(path.as_str(), options).map_err(zip_err)?;
        zip.write_all(content)?;
    }
    let bytes = zip.finish().map_err(zip_err)?.into_inner();
    Ok((format!("{}.zip", export_name(&theme.name)), bytes))
}

/// Whether a sprite is XML libxml2 reads without an error: one root
/// element, balanced tags, quoted attributes, known entities. Conservative:
/// a construct it doesn't read counts as not well-formed.
pub fn well_formed_xml(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let name_start = |c: char| c.is_ascii_alphabetic() || c == '_' || c == ':';
    let name_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '-' | '.');
    let read_name = |s: &str| -> Option<usize> {
        let mut chars = s.char_indices();
        match chars.next() {
            Some((_, c)) if name_start(c) => {}
            _ => return None,
        }
        Some(
            chars
                .find(|(_, c)| !name_char(*c))
                .map_or(s.len(), |(i, _)| i),
        )
    };
    let entities_ok = |s: &str| -> bool {
        let mut rest = s;
        while let Some(i) = rest.find('&') {
            let after = &rest[i + 1..];
            let Some(end) = after.find(';') else {
                return false;
            };
            let entity = &after[..end];
            let ok = matches!(entity, "lt" | "gt" | "amp" | "quot" | "apos")
                || entity
                    .strip_prefix("#x")
                    .is_some_and(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()))
                || entity
                    .strip_prefix('#')
                    .is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()));
            if !ok {
                return false;
            }
            rest = &after[end + 1..];
        }
        true
    };
    let mut stack: Vec<&str> = Vec::new();
    let mut roots = 0;
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<?") {
            let Some(end) = after.find("?>") else {
                return false;
            };
            rest = &after[end + 2..];
        } else if let Some(after) = rest.strip_prefix("<!--") {
            let Some(end) = after.find("-->") else {
                return false;
            };
            rest = &after[end + 3..];
        } else if let Some(after) = rest.strip_prefix("<![CDATA[") {
            if stack.is_empty() {
                return false;
            }
            let Some(end) = after.find("]]>") else {
                return false;
            };
            rest = &after[end + 3..];
        } else if rest.starts_with("<!") {
            // A doctype (and its internal subset) is not read here.
            return false;
        } else if let Some(after) = rest.strip_prefix("</") {
            let Some(len) = read_name(after) else {
                return false;
            };
            let name = &after[..len];
            let tail = after[len..].trim_start();
            let Some(tail) = tail.strip_prefix('>') else {
                return false;
            };
            if stack.pop() != Some(name) {
                return false;
            }
            rest = tail;
        } else if let Some(after) = rest.strip_prefix('<') {
            let Some(len) = read_name(after) else {
                return false;
            };
            let name = &after[..len];
            let mut tail = &after[len..];
            let mut seen: Vec<&str> = Vec::new();
            loop {
                let trimmed = tail.trim_start_matches([' ', '\t', '\r', '\n']);
                let spaced = trimmed.len() != tail.len();
                tail = trimmed;
                if let Some(t) = tail.strip_prefix("/>") {
                    if stack.is_empty() {
                        roots += 1;
                    }
                    tail = t;
                    break;
                }
                if let Some(t) = tail.strip_prefix('>') {
                    if stack.is_empty() {
                        roots += 1;
                    }
                    stack.push(name);
                    tail = t;
                    break;
                }
                if !spaced {
                    return false;
                }
                let Some(alen) = read_name(tail) else {
                    return false;
                };
                let attr = &tail[..alen];
                if seen.contains(&attr) {
                    return false;
                }
                seen.push(attr);
                let t = tail[alen..].trim_start_matches([' ', '\t', '\r', '\n']);
                let Some(t) = t.strip_prefix('=') else {
                    return false;
                };
                let t = t.trim_start_matches([' ', '\t', '\r', '\n']);
                let quote = match t.chars().next() {
                    Some(q @ ('"' | '\'')) => q,
                    _ => return false,
                };
                let body = &t[1..];
                let Some(end) = body.find(quote) else {
                    return false;
                };
                let value = &body[..end];
                if value.contains('<') || !entities_ok(value) {
                    return false;
                }
                tail = &body[end + 1..];
            }
            rest = tail;
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            let chunk = &rest[..end];
            if stack.is_empty() {
                if !chunk.chars().all(|c| matches!(c, ' ' | '\t' | '\r' | '\n')) {
                    return false;
                }
            } else if !entities_ok(chunk) || chunk.contains("]]>") {
                return false;
            }
            rest = &rest[end..];
        }
    }
    stack.is_empty() && roots == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(files: &[&str]) -> Bundle {
        let mut b = Bundle::default();
        for f in files {
            b.add_file(f, b"x".to_vec()).unwrap();
        }
        b
    }

    #[test]
    fn all_files_walks_sorted_directories_and_skips_dot_files() {
        let b = bundle(&[
            "settings.yml",
            "about.json",
            "README.md",
            "a/b",
            "a.txt",
            ".hidden",
            "common/.DS_Store",
            "common/header.html",
        ]);
        assert_eq!(
            b.all_files(),
            vec![
                "README.md",
                "a/b",
                "a.txt",
                "about.json",
                "common/header.html",
                "settings.yml"
            ]
        );
    }

    #[test]
    fn a_lone_root_directory_is_stripped() {
        let mut b = bundle(&["theme/about.json", "theme/common/header.html", "theme/.git"]);
        b.strip_root_directory();
        assert!(b.read("about.json").is_some());
        assert!(b.read("common/header.html").is_some());
        let mut flat = bundle(&["about.json", "common/header.html"]);
        flat.strip_root_directory();
        assert!(flat.read("about.json").is_some());
    }

    #[test]
    fn real_paths_stay_in_the_folder() {
        let b = bundle(&["assets/logo.png", "about.json"]);
        assert_eq!(
            b.real_path("assets/../about.json").as_deref(),
            Some("about.json")
        );
        assert_eq!(b.real_path("../etc/passwd"), None);
        assert_eq!(b.real_path("nope/../about.json"), None);
        assert_eq!(b.file_size("missing.png"), -1);
    }

    #[test]
    fn file_paths_map_to_fields() {
        assert_eq!(
            opts_from_file_path("common/header.html"),
            Some(("common", "header".into(), 0))
        );
        assert_eq!(
            opts_from_file_path("desktop/desktop.scss"),
            Some(("desktop", "scss".into(), 1))
        );
        assert_eq!(opts_from_file_path("desktop/mobile.scss"), None);
        assert_eq!(
            opts_from_file_path("settings.yaml"),
            Some(("settings", "yaml".into(), 5))
        );
        assert_eq!(
            opts_from_file_path("javascripts/discourse/api.js"),
            Some(("extra_js", "discourse/api.js".into(), 6))
        );
        assert_eq!(opts_from_file_path("assets/logo.png"), None);
        assert_eq!(opts_from_file_path("README.md"), None);
    }

    #[test]
    fn git_urls_normalize_like_git_url() {
        assert_eq!(
            normalize_git_url("git@github.com:discourse/x.git"),
            "ssh://git@github.com/discourse/x.git"
        );
        assert_eq!(
            normalize_git_url("https://github.com/discourse/x/"),
            "https://github.com/discourse/x.git"
        );
        assert_eq!(uri_scheme("git://example.com/x"), Ok(Some("git")));
        assert_eq!(uri_scheme("not a url"), Err(()));
        assert_eq!(uri_scheme(""), Ok(None));
    }

    #[test]
    fn export_names_are_prefixed() {
        assert_eq!(export_name("Fancy Theme"), "discourse-fancy-theme");
        assert_eq!(export_name("Plain Exported!"), "discourse-plain-exported-");
        assert_eq!(export_name("Discourse Kit"), "discourse-kit");
    }

    #[test]
    fn sprites_are_checked_for_well_formed_xml() {
        assert!(well_formed_xml(
            br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"><symbol id="a"><path d="M0 0"/></symbol></svg>"#
        ));
        assert!(!well_formed_xml(b"<svg><symbol></svg>"));
        assert!(!well_formed_xml(b"<svg a=b></svg>"));
        assert!(!well_formed_xml(b"<svg/><svg/>"));
        assert!(!well_formed_xml(b"<svg>&nbsp;</svg>"));
    }

    #[test]
    fn ssh_public_keys_use_the_openssh_wire_format() {
        use rsa::BigUint;
        let key = rsa::RsaPublicKey::new(
            BigUint::from_bytes_be(&[0xc1; 128]),
            BigUint::from(65537u32),
        )
        .unwrap();
        let line = ssh_public_key(&key);
        assert!(line.starts_with("ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAAgQDB"));
    }

    #[test]
    fn screenshot_names_are_sanitized() {
        assert_eq!(sanitize_filename("my light shot"), "my_light_shot");
        assert_eq!(sanitize_filename("a__b"), "a_b");
        assert_eq!(extname("screenshots/light.png"), ".png");
        assert_eq!(extname("screenshots/.png"), "");
    }
}
