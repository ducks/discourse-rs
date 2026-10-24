//! `discourse-rs setup`: a new site's database, as `rake db:migrate` makes
//! one on a fresh install. The vendored structure.sql, then what db:seed
//! creates (seed/clean_install.sql, snapshotted from the reference right
//! after its own migrate by `scripts/snapshot-dv --clean`), then the parts
//! of db:seed that depend on the site or are random, which the snapshot
//! leaves out:
//!
//! - db/fixtures/009_users.rb and discourse-narrative-bot's 001_discobot.rb:
//!   the system and discobot users' random passwords, and the signup email
//!   token each user's after_create makes;
//! - 990_settings.rb: notification_email from the hostname,
//!   api_key_last_used_epoch;
//! - 991_vapid_keys.rb and 992_push_api_secret_key.rb ([`ensure_push_keys`],
//!   which every start runs too, as every Rails deploy's db:seed does);
//! - 993_login_hint.rb: the finish installation notice while there is no
//!   admin.
//!
//! The snapshot was seeded under [`SEED_HOSTNAME`]: the admin guide topic
//! links to that Discourse.base_url and topic_links keep its domain, so the
//! site's own is put in its place before loading.
//!
//! All of it in one transaction, into a database without tables; anything
//! else is refused. Then the port's own `discourse_rs` schema, as the
//! server makes it at start.

use std::fmt;

use sqlx::{Connection, PgConnection};

use crate::AppError;
use crate::config::Config;
use crate::site_settings::{Definitions, SiteSettings};

/// What db:seed creates on an empty database at the vendored commit.
pub const CLEAN_SEED: &str = include_str!("../seed/clean_install.sql");

/// `Discourse.current_hostname` while the clean seed was made.
pub const SEED_HOSTNAME: &str = "setup.discourse-rs.invalid";
/// Its `Discourse.base_url` (development's port 3000).
const SEED_BASE_URL: &str = "http://setup.discourse-rs.invalid:3000";
const SEED_HOST_WITH_PORT: &str = "setup.discourse-rs.invalid:3000";

/// The users db:seed gives a random password: system (009_users.rb) and
/// discobot (discourse-narrative-bot's 001_discobot.rb).
const SEEDED_USERS: [i32; 2] = [-1, -2];

/// `site_settings.data_type` for strings and booleans
/// (SiteSettings::TypeSupervisor.types).
const STRING: i32 = 1;
const BOOL: i32 = 5;

#[derive(Debug)]
pub enum SetupError {
    /// The database has tables already.
    NotEmpty {
        tables: i64,
    },
    /// The connecting role can't create an extension structure.sql needs.
    Extensions(String),
    /// `--owner` names a role the connecting one can't act as.
    Owner(String),
    App(AppError),
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetupError::NotEmpty { tables } => write!(
                f,
                "the database already has {tables} tables; setup only loads an empty database \
                 (nothing was changed). To start over, drop and recreate it."
            ),
            SetupError::Extensions(message) | SetupError::Owner(message) => f.write_str(message),
            SetupError::App(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SetupError {}

impl From<AppError> for SetupError {
    fn from(e: AppError) -> Self {
        SetupError::App(e)
    }
}

impl From<sqlx::Error> for SetupError {
    fn from(e: sqlx::Error) -> Self {
        SetupError::App(e.into())
    }
}

impl From<crate::site_settings::SettingError> for SetupError {
    fn from(e: crate::site_settings::SettingError) -> Self {
        SetupError::App(e.into())
    }
}

impl From<crate::url::UrlError> for SetupError {
    fn from(e: crate::url::UrlError) -> Self {
        SetupError::App(e.into())
    }
}

/// What was loaded.
#[derive(Debug)]
pub struct Report {
    pub tables: i64,
    pub versions: usize,
}

/// The extensions structure.sql creates, in its order.
pub fn extensions() -> Vec<&'static str> {
    crate::schema::structure_sql()
        .lines()
        .filter_map(|l| l.strip_prefix("CREATE EXTENSION IF NOT EXISTS "))
        .filter_map(|rest| rest.split_whitespace().next())
        .collect()
}

/// What the connecting role knows about an extension it lacks.
#[derive(Debug, PartialEq)]
pub enum Availability {
    /// Not installed on the server at all.
    Missing,
    /// Installed; a database owner may create it (trusted).
    Trusted,
    /// Installed; only a superuser may create it.
    Untrusted,
}

/// Why a role that is not a superuser can't load the schema, if it can't:
/// each extension the database lacks must be one it may create.
pub fn privilege_problem(
    superuser: bool,
    database: &str,
    lacking: &[(&str, Availability)],
) -> Option<String> {
    let missing: Vec<&str> = lacking
        .iter()
        .filter(|(_, a)| *a == Availability::Missing)
        .map(|(n, _)| *n)
        .collect();
    if !missing.is_empty() {
        return Some(format!(
            "the PostgreSQL server lacks the {} extension{} structure.sql needs (vector is pgvector); \
             install {} on the server first. Nothing was changed.",
            missing.join(", "),
            if missing.len() == 1 { "" } else { "s" },
            if missing.len() == 1 { "it" } else { "them" },
        ));
    }
    if superuser {
        return None;
    }
    let untrusted: Vec<&str> = lacking
        .iter()
        .filter(|(_, a)| *a == Availability::Untrusted)
        .map(|(n, _)| *n)
        .collect();
    if untrusted.is_empty() {
        return None;
    }
    let creates = untrusted
        .iter()
        .map(|n| format!("CREATE EXTENSION {n};"))
        .collect::<Vec<_>>()
        .join(" ");
    Some(format!(
        "only a superuser can create the {} extension{} in {database}, and this role is not one. \
         Either create {} first as a superuser (sudo -u postgres psql -d {database} -c '{creates}'), \
         or run setup as a superuser with --owner <the server's role>. Nothing was changed.",
        untrusted.join(", "),
        if untrusted.len() == 1 { "" } else { "s" },
        if untrusted.len() == 1 { "it" } else { "them" },
    ))
}

/// Puts the site's host and base url where the seed has the placeholder's.
/// The seed's data is COPY text, so a value holding a backslash, tab or
/// newline (which no hostname does) would need escaping: refused.
pub fn localize_seed(seed: &str, base_url: &str, host_with_port: &str, host: &str) -> String {
    for value in [base_url, host_with_port, host] {
        assert!(
            !value.contains(['\\', '\t', '\n', '\r']),
            "a hostname or base url with a control character: {value:?}"
        );
    }
    seed.replace(SEED_BASE_URL, base_url)
        .replace(SEED_HOST_WITH_PORT, host_with_port)
        .replace(SEED_HOSTNAME, host)
}

/// A pg_dump data-only script: plain statements, and COPY blocks with
/// their data.
#[derive(Debug, PartialEq)]
pub enum Piece<'a> {
    Sql(String),
    Copy { statement: &'a str, data: String },
}

pub fn pieces(dump: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut lines = dump.lines();
    let mut sql = String::new();
    while let Some(line) = lines.next() {
        if sql.is_empty() && (line.is_empty() || line.starts_with("--")) {
            continue;
        }
        if sql.is_empty() && line.starts_with("COPY ") && line.ends_with(" FROM stdin;") {
            let mut data = String::new();
            for row in lines.by_ref() {
                if row == "\\." {
                    break;
                }
                data.push_str(row);
                data.push('\n');
            }
            out.push(Piece::Copy {
                statement: line.trim_end_matches(';'),
                data,
            });
            continue;
        }
        if !sql.is_empty() {
            sql.push('\n');
        }
        sql.push_str(line);
        if line.trim_end().ends_with(';') {
            out.push(Piece::Sql(std::mem::take(&mut sql)));
        }
    }
    out
}

async fn run_pieces(conn: &mut PgConnection, dump: &str) -> Result<(), SetupError> {
    for piece in pieces(dump) {
        match piece {
            Piece::Sql(sql) => {
                sqlx::raw_sql(&sql).execute(&mut *conn).await?;
            }
            Piece::Copy { data, .. } if data.is_empty() => {}
            Piece::Copy { statement, data } => {
                let mut copy = conn.copy_in_raw(statement).await?;
                copy.send(data.as_bytes()).await?;
                copy.finish().await?;
            }
        }
    }
    Ok(())
}

async fn table_count(conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE schemaname = 'public'")
        .fetch_one(&mut *conn)
        .await
}

/// A role name as an SQL identifier.
fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Loads the schema and seed into the empty database `conn` is connected
/// to, in one transaction. With `owner`, the schema objects belong to that
/// role (the server's), as Rails' database user owns its tables; without,
/// to the connecting role.
pub async fn setup(
    conn: &mut PgConnection,
    defs: &Definitions,
    config: &Config,
    owner: Option<&str>,
) -> Result<Report, SetupError> {
    let tables = table_count(conn).await?;
    if tables > 0 {
        return Err(SetupError::NotEmpty { tables });
    }
    let (database, superuser): (String, bool) = sqlx::query_as(
        "SELECT current_database(), rolsuper FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(&mut *conn)
    .await?;
    let mut lacking = Vec::new();
    for name in extensions() {
        let installed: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = $1)")
                .bind(name)
                .fetch_one(&mut *conn)
                .await?;
        if installed {
            continue;
        }
        let trusted: Option<bool> = sqlx::query_scalar(
            "SELECT v.trusted FROM pg_available_extensions a \
             JOIN pg_available_extension_versions v ON v.name = a.name AND v.version = a.default_version \
             WHERE a.name = $1",
        )
        .bind(name)
        .fetch_optional(&mut *conn)
        .await?;
        let availability = match trusted {
            None => Availability::Missing,
            Some(true) => Availability::Trusted,
            Some(false) => Availability::Untrusted,
        };
        lacking.push((name, availability));
    }
    if let Some(problem) = privilege_problem(superuser, &database, &lacking) {
        return Err(SetupError::Extensions(problem));
    }
    if let Some(owner) = owner {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)")
                .bind(owner)
                .fetch_one(&mut *conn)
                .await?;
        if !exists {
            return Err(SetupError::Owner(format!(
                "there is no role {owner} to own the schema; create it first \
                 (sudo -u postgres createuser {owner}). Nothing was changed."
            )));
        }
        let member: bool = sqlx::query_scalar("SELECT pg_has_role(current_user, $1, 'MEMBER')")
            .bind(owner)
            .fetch_one(&mut *conn)
            .await?;
        if !member {
            return Err(SetupError::Owner(format!(
                "this role can't act as {owner}; run setup as a superuser, or as {owner} itself \
                 without --owner. Nothing was changed."
            )));
        }
    }
    // structure.sql creates the discourse_functions schema and the tables
    // in public: the database's owner may.
    let (creates, owns_public): (bool, bool) = sqlx::query_as(
        "SELECT has_database_privilege(COALESCE($1, current_user::text), current_database(), 'CREATE'), \
                has_schema_privilege(COALESCE($1, current_user::text), 'public', 'CREATE')",
    )
    .bind(owner)
    .fetch_one(&mut *conn)
    .await?;
    if !creates || !owns_public {
        let who = owner.unwrap_or("this role");
        return Err(SetupError::Owner(format!(
            "{who} can't create schemas and tables in {database}; make it the database's owner \
             (ALTER DATABASE {database} OWNER TO <role>), as Rails' database user owns its database. \
             Nothing was changed."
        )));
    }

    let mut tx = conn.begin().await?;
    if let Some(owner) = owner {
        // The extensions as the connecting role (vector needs a superuser),
        // the rest as the owner.
        for name in extensions() {
            sqlx::raw_sql(&format!(
                "CREATE EXTENSION IF NOT EXISTS {name} WITH SCHEMA public"
            ))
            .execute(&mut *tx)
            .await?;
        }
        sqlx::raw_sql(&format!("SET ROLE {}", ident(owner)))
            .execute(&mut *tx)
            .await?;
    }
    sqlx::raw_sql(crate::schema::structure_sql())
        .execute(&mut *tx)
        .await?;
    sqlx::raw_sql("RESET search_path").execute(&mut *tx).await?;

    // The settings the urls depend on are not in the seed: defaults and
    // globals, read from the empty site_settings.
    let settings = SiteSettings::load(&mut tx, defs, &config.globals).await?;
    let urls = crate::url::Urls {
        config,
        settings: &settings,
    };
    let seed = localize_seed(
        CLEAN_SEED,
        &urls.base_url()?,
        &urls.current_hostname_with_port()?,
        &urls.current_hostname()?,
    );
    run_pieces(&mut tx, &seed).await?;
    sqlx::raw_sql("RESET search_path").execute(&mut *tx).await?;

    seed_site(&mut tx, defs, config).await?;
    crate::owned_schema::migrate_conn(&mut tx).await?;
    if owner.is_some() {
        sqlx::raw_sql("RESET ROLE").execute(&mut *tx).await?;
    }
    let tables = table_count(&mut tx).await?;
    tx.commit().await?;
    Ok(Report {
        tables,
        versions: crate::schema::expected_versions().len(),
    })
}

/// db:seed's fixtures that depend on the site or are random, on a database
/// just loaded from the clean seed.
pub async fn seed_site(
    conn: &mut PgConnection,
    defs: &Definitions,
    config: &Config,
) -> Result<(), AppError> {
    // `u.password = SecureRandom.hex`, and User's after_create
    // create_email_token.
    for user_id in SEEDED_USERS {
        let email: Option<String> =
            sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
                .bind(user_id)
                .fetch_optional(&mut *conn)
                .await?;
        let Some(email) = email else {
            continue;
        };
        crate::accounts::set_password(&mut *conn, user_id, &crate::accounts::random_hex()).await?;
        crate::accounts::create_email_token(
            &mut *conn,
            user_id,
            &email,
            crate::accounts::token_scopes::SIGNUP,
        )
        .await?;
    }

    let settings = SiteSettings::load(&mut *conn, defs, &config.globals).await?;
    let urls = crate::url::Urls {
        config,
        settings: &settings,
    };
    // 990_settings.rb: the default notification_email becomes
    // noreply@<hostname>, unless that is not a valid address (as in
    // development's localhost).
    let default_email = defs
        .get("notification_email")
        .map(|d| d.default.to_s())
        .unwrap_or_default();
    if settings.get("notification_email")?.to_s() == default_email {
        let email = format!("noreply@{}", urls.current_hostname()?);
        if crate::accounts::valid_email(&email) {
            set_raw(&mut *conn, "notification_email", STRING, &email).await?;
        }
    }
    if settings.get("api_key_last_used_epoch")?.is_blank() {
        // Time.now as a string.
        let now = crate::clock::now().format("%Y-%m-%d %H:%M:%S +0000");
        set_raw(
            &mut *conn,
            "api_key_last_used_epoch",
            STRING,
            &now.to_string(),
        )
        .await?;
    }
    ensure_push_keys(&mut *conn, defs, config).await?;
    if config.site.db == crate::multisite::DEFAULT {
        login_hint(&mut *conn, &settings, config).await?;
    }
    Ok(())
}

/// 991_vapid_keys.rb and 992_push_api_secret_key.rb, which every db:seed
/// (so every Rails deploy) runs and every start of the port does: a VAPID
/// key pair when one is missing or `vapid_base_url` is not
/// `Discourse.base_url` (push subscriptions are bound to both, so the old
/// ones go), and a push_api_secret_key unless one of 32 characters is set.
/// The names of the settings it set.
pub async fn ensure_push_keys(
    conn: &mut PgConnection,
    defs: &Definitions,
    config: &Config,
) -> Result<Vec<&'static str>, AppError> {
    let settings = SiteSettings::load(&mut *conn, defs, &config.globals).await?;
    let base_url = crate::url::Urls {
        config,
        settings: &settings,
    }
    .base_url()?;
    let mut set = Vec::new();
    if settings.get("vapid_public_key")?.is_blank()
        || settings.get("vapid_private_key")?.is_blank()
        || settings.get("vapid_public_key_bytes")?.is_blank()
        || settings.get("vapid_base_url")?.to_s() != base_url
    {
        let keys = VapidKeys::generate();
        for (name, value) in [
            ("vapid_public_key", keys.public_key.as_str()),
            ("vapid_private_key", keys.private_key.as_str()),
            ("vapid_public_key_bytes", keys.public_key_bytes.as_str()),
            ("vapid_base_url", base_url.as_str()),
        ] {
            set_raw(&mut *conn, name, STRING, value).await?;
            set.push(name);
        }
        sqlx::query("DELETE FROM push_subscriptions")
            .execute(&mut *conn)
            .await?;
    }
    if settings.get("push_api_secret_key")?.to_s().chars().count() != 32 {
        set_raw(
            &mut *conn,
            "push_api_secret_key",
            STRING,
            &crate::accounts::random_hex(),
        )
        .await?;
        set.push("push_api_secret_key");
    }
    Ok(set)
}

/// 993_login_hint.rb: with fewer than 20 users and no human admin, the
/// global notice that points at the finish installation page (or says
/// there is no developer email), and has_login_hint on.
async fn login_hint(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    config: &Config,
) -> Result<(), AppError> {
    let (users, admins): (i64, bool) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM (SELECT 1 FROM users LIMIT 20) u), \
                EXISTS (SELECT 1 FROM users WHERE admin AND id > 0)",
    )
    .fetch_one(&mut *conn)
    .await?;
    if users >= 20 || admins {
        return Ok(());
    }
    let notice = login_hint_notice(config.globals.get("developer_emails"));
    if settings.get("global_notice")?.to_s() != notice {
        set_raw(&mut *conn, "global_notice", STRING, &notice).await?;
        set_raw(&mut *conn, "has_login_hint", BOOL, "t").await?;
    }
    Ok(())
}

/// The notice 993_login_hint.rb sets (English in Rails too).
pub fn login_hint_notice(developer_emails: Option<&str>) -> String {
    let Some(emails) = developer_emails.filter(|e| !crate::ruby::is_blank(e)) else {
        return "Congratulations, you installed Discourse! Unfortunately, no administrator emails were defined during setup, so finalizing the configuration <a href='https://meta.discourse.org/t/create-admin-account-from-console/17274'>may be difficult</a>.".to_string();
    };
    let emails: Vec<&str> = emails.split(',').collect();
    let named = match emails.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last} ", rest.join(", ")),
        _ => emails[0].to_string(),
    };
    format!(
        "Congratulations, you installed Discourse! Register a new admin account with {named} to finalize configuration."
    )
}

/// `User#clear_global_notice_if_needed`, User's after_save: an active
/// admin saved while the login hint is on turns it and the notice off.
pub async fn clear_global_notice_if_needed(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<(), AppError> {
    if user_id < 0 {
        return Ok(());
    }
    let hint: Option<Option<String>> =
        sqlx::query_scalar("SELECT value FROM site_settings WHERE name = 'has_login_hint'")
            .fetch_optional(&mut *conn)
            .await?;
    if !matches!(hint, Some(Some(ref v)) if v == "t") {
        return Ok(());
    }
    let active_admin: bool = sqlx::query_scalar("SELECT admin AND active FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&mut *conn)
        .await?
        .unwrap_or(false);
    if !active_admin {
        return Ok(());
    }
    set_raw(&mut *conn, "has_login_hint", BOOL, "f").await?;
    set_raw(&mut *conn, "global_notice", STRING, "").await?;
    Ok(())
}

/// `SiteSetting.<name> = value` for the plain settings above (DbProvider's
/// save: a new row, else the value when it changed).
async fn set_raw(
    conn: &mut PgConnection,
    name: &str,
    data_type: i32,
    value: &str,
) -> Result<(), sqlx::Error> {
    let updated = sqlx::query(
        "UPDATE site_settings SET value = $2, data_type = $3, updated_at = clock_timestamp() \
         WHERE name = $1 AND (value IS DISTINCT FROM $2 OR data_type <> $3)",
    )
    .bind(name)
    .bind(value)
    .bind(data_type)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if updated == 0 {
        sqlx::query(
            "INSERT INTO site_settings (name, data_type, value, created_at, updated_at) \
             VALUES ($1, $3, $2, clock_timestamp(), clock_timestamp()) ON CONFLICT (name) DO NOTHING",
        )
        .bind(name)
        .bind(value)
        .bind(data_type)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// A VAPID key pair as `WebPush.generate_key` gives it and 991_vapid_keys.rb
/// stores it: the uncompressed P-256 public point and the private scalar,
/// URL-safe base64 with padding, and `vapid_public_key_bytes`, the public
/// key's bytes joined by `|`.
pub struct VapidKeys {
    pub public_key: String,
    pub private_key: String,
    pub public_key_bytes: String,
}

impl VapidKeys {
    pub fn generate() -> Self {
        use base64::Engine;
        use p256::elliptic_curve::sec1::ToEncodedPoint;
        let secret = p256::SecretKey::random(&mut rand::rngs::OsRng);
        let public = secret.public_key().to_encoded_point(false);
        let b64 = base64::engine::general_purpose::URL_SAFE;
        VapidKeys {
            public_key: b64.encode(public.as_bytes()),
            private_key: b64.encode(secret.to_bytes()),
            public_key_bytes: public
                .as_bytes()
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join("|"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn vapid_keys_are_webpush_shaped() {
        let keys = VapidKeys::generate();
        let b64 = base64::engine::general_purpose::URL_SAFE;
        let public = b64.decode(&keys.public_key).unwrap();
        assert_eq!(public.len(), 65);
        assert_eq!(public[0], 4, "an uncompressed point");
        assert_eq!(b64.decode(&keys.private_key).unwrap().len(), 32);
        let bytes: Vec<u8> = keys
            .public_key_bytes
            .split('|')
            .map(|b| b.parse().unwrap())
            .collect();
        assert_eq!(bytes, public);
        assert_ne!(keys.public_key, VapidKeys::generate().public_key);
    }

    #[test]
    fn the_extensions_are_structure_sqls() {
        assert_eq!(extensions(), ["hstore", "pg_trgm", "unaccent", "vector"]);
    }

    #[test]
    fn a_superuser_may_create_any_installed_extension() {
        let lacking = [
            ("pg_trgm", Availability::Trusted),
            ("vector", Availability::Untrusted),
        ];
        assert_eq!(privilege_problem(true, "forum", &lacking), None);
    }

    #[test]
    fn an_owner_may_create_trusted_extensions_only() {
        assert_eq!(
            privilege_problem(false, "forum", &[("pg_trgm", Availability::Trusted)]),
            None
        );
        let problem =
            privilege_problem(false, "forum", &[("vector", Availability::Untrusted)]).unwrap();
        assert!(problem.contains("only a superuser can create the vector extension in forum"));
        assert!(problem.contains("CREATE EXTENSION vector;"), "{problem}");
        assert!(problem.contains("--owner"), "{problem}");
    }

    #[test]
    fn an_extension_missing_from_the_server_is_named() {
        let problem =
            privilege_problem(true, "forum", &[("vector", Availability::Missing)]).unwrap();
        assert!(problem.contains("lacks the vector extension"), "{problem}");
    }

    #[test]
    fn the_seed_gets_the_sites_urls() {
        let seed = "1\thttp://setup.discourse-rs.invalid:3000/t/-/5/\tsetup.discourse-rs.invalid\n\
                    2\tsetup.discourse-rs.invalid:3000/admin\t?__ws=setup.discourse-rs.invalid\n";
        let localized = localize_seed(
            seed,
            "https://forum.example.com",
            "forum.example.com",
            "forum.example.com",
        );
        assert_eq!(
            localized,
            "1\thttps://forum.example.com/t/-/5/\tforum.example.com\n\
             2\tforum.example.com/admin\t?__ws=forum.example.com\n"
        );
        assert!(
            !localize_seed(CLEAN_SEED, "https://f.test", "f.test", "f.test")
                .contains("discourse-rs.invalid")
        );
    }

    #[test]
    fn the_dump_splits_into_statements_and_copies() {
        let dump = "-- a comment\n\nSET statement_timeout = 0;\n\
                    SELECT pg_catalog.set_config('search_path', '', false);\n\n\
                    COPY public.t (a, b) FROM stdin;\n1\tx\n2\t\\N\n\\.\n\n\
                    COPY public.empty (a) FROM stdin;\n\\.\n\
                    SELECT pg_catalog.setval('public.t_id_seq', 2,\n true);\n";
        assert_eq!(
            pieces(dump),
            [
                Piece::Sql("SET statement_timeout = 0;".into()),
                Piece::Sql("SELECT pg_catalog.set_config('search_path', '', false);".into()),
                Piece::Copy {
                    statement: "COPY public.t (a, b) FROM stdin",
                    data: "1\tx\n2\t\\N\n".into()
                },
                Piece::Copy {
                    statement: "COPY public.empty (a) FROM stdin",
                    data: String::new()
                },
                Piece::Sql("SELECT pg_catalog.setval('public.t_id_seq', 2,\n true);".into()),
            ]
        );
    }

    #[test]
    fn the_clean_seed_has_no_secrets_or_users_beyond_the_seeded() {
        for name in [
            "vapid_public_key",
            "vapid_private_key",
            "push_api_secret_key",
            "has_login_hint",
            "global_notice",
            "api_key_last_used_epoch",
        ] {
            assert!(!CLEAN_SEED.contains(&format!("\t{name}\t")), "{name}");
        }
        assert!(!CLEAN_SEED.contains("COPY public.user_passwords "));
        assert!(!CLEAN_SEED.contains("COPY public.email_tokens "));
        let pieces = pieces(CLEAN_SEED);
        let users = pieces
            .iter()
            .find_map(|p| match p {
                Piece::Copy { statement, data } if statement.starts_with("COPY public.users ") => {
                    Some(data)
                }
                _ => None,
            })
            .unwrap();
        let ids: Vec<&str> = users
            .lines()
            .map(|l| l.split('\t').next().unwrap())
            .collect();
        assert_eq!(ids, ["-1", "-2"]);
    }

    #[test]
    fn the_login_hint_names_the_developer_emails() {
        assert!(login_hint_notice(None).contains("no administrator emails were defined"));
        assert_eq!(
            login_hint_notice(Some("a@example.com")),
            "Congratulations, you installed Discourse! Register a new admin account with a@example.com to finalize configuration."
        );
        assert_eq!(
            login_hint_notice(Some("a@example.com,b@example.com,c@example.com")),
            "Congratulations, you installed Discourse! Register a new admin account with a@example.com, b@example.com or c@example.com  to finalize configuration."
        );
    }
}
