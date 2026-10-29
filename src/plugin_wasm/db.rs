//! A plugin's own database: a schema of its own and SQL of its own,
//! sandboxed by Postgres rather than by the host.
//!
//! Per site, a plugin with a database gets:
//!
//! - a login role, `dpl_<name>_<hash of the database name>`, with a random
//!   password kept in `discourse_rs.plugin_roles`. It owns nothing but its
//!   schema and is granted nothing but the core columns its manifest lists
//!   under `[core_reads]` (column-level `GRANT SELECT`), minus the
//!   credentials in [`DENIED`];
//! - a schema, `plugin_<name>`, owned by that role and on its search path,
//!   where `migrations/*.sql` run as the role, in file order, once each;
//! - a small pool of connections logged in as the role. The guest's SQL
//!   runs there and never on the app's connection: after `SET ROLE` on the
//!   app's connection, `RESET ROLE` (or `set_config('role', ...)`) would
//!   hand the app's own privileges back, as Postgres checks a role switch
//!   against the session user. Logged in as the role, there is nothing to
//!   switch back to.
//!
//! Each hook call that queries gets one transaction on that pool: read
//! only except for jobs and the routes that write (anything but GET and
//! HEAD), committed only when the hook succeeds. A query past the hook's
//! time budget is cancelled with `pg_cancel_backend` (the guest can raise
//! its own `statement_timeout`, so the setting is not the limit), and
//! session settings a guest changes are reset when its connection goes
//! back to the pool.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde_json::Value;
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::postgres::types::Oid;
use sqlx::postgres::{PgArgumentBuffer, PgPoolOptions, PgTypeInfo};
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use tokio::time::Instant;

use super::Plugin;

/// Core tables and columns no grant reaches: credentials and tokens.
const DENIED: &[(&str, Option<&str>)] = &[
    ("user_passwords", None),
    ("groups", Some("email_password")),
    ("invites", Some("email_token")),
    ("web_hooks", Some("secret")),
    ("ai_secrets", None),
    ("ai_mcp_oauth_tokens", None),
    ("mcp_oauth_access_tokens", None),
    ("mcp_oauth_refresh_tokens", None),
    ("api_keys", None),
    ("user_api_keys", None),
    ("user_api_key_clients", None),
    ("user_auth_tokens", None),
    ("user_auth_token_logs", None),
    ("email_tokens", None),
    ("user_second_factors", None),
    ("user_security_keys", None),
    ("single_sign_on_records", None),
    ("user_associated_accounts", None),
    ("user_emails", None),
];

/// The most rows `query` hands back.
const MAX_ROWS: usize = 10_000;

/// Connections per plugin per site.
const POOL_SIZE: u32 = 4;

/// What one of a plugin's statements may spill to disk. Superuser-only,
/// so set on the role it holds where a guest's own SET can't lift it.
const TEMP_FILE_LIMIT: &str = "1GB";

/// The sites of the process, by database name: hooks that have no pool
/// at hand (serializer keys, outlets, the homepage) find their site's
/// here from their connection's `current_database()`.
static SITES: LazyLock<Mutex<HashMap<String, PgPool>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Records a site's pool (`crate::app` calls it for every site).
pub fn register_site(pool: &PgPool) {
    let Some(db) = pool.connect_options().get_database().map(String::from) else {
        return;
    };
    if let Ok(mut sites) = SITES.lock() {
        sites.entry(db).or_insert_with(|| pool.clone());
    }
}

/// The app pool of the site `conn` is connected to.
pub(super) async fn site_pool(conn: &mut PgConnection) -> Result<PgPool, String> {
    let db: String = sqlx::query_scalar("SELECT current_database()::text")
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| e.to_string())?;
    SITES
        .lock()
        .ok()
        .and_then(|s| s.get(&db).cloned())
        .ok_or_else(|| format!("no site registered for database {db}"))
}

/// A plugin's pools, one per site, each installed once.
#[derive(Default)]
pub(super) struct Pools(Mutex<HashMap<String, Arc<tokio::sync::Mutex<Option<PgPool>>>>>);

fn ident(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `plugin_<name>`.
pub fn schema_name(plugin: &str) -> String {
    format!("plugin_{}", ident(plugin))
}

/// `dpl_<name>_<first 8 hex of md5(database)>`: roles are the cluster's
/// and sites (and test databases) share a cluster.
pub fn role_name(plugin: &str, database: &str) -> String {
    let hash = format!("{:x}", md5::compute(database.as_bytes()));
    let mut name = ident(plugin);
    name.truncate(40);
    format!("dpl_{name}_{}", &hash[..8])
}

impl Plugin {
    /// The plugin's pool on the site of `app`, installing it there first:
    /// role, password, schema, grants, migrations.
    pub(super) async fn db_pool(&self, app: &PgPool) -> Result<PgPool, String> {
        let db = app
            .connect_options()
            .get_database()
            .map(String::from)
            .ok_or("the site's pool names no database")?;
        let cell = {
            let mut pools = self.pools.0.lock().map_err(|e| e.to_string())?;
            pools.entry(db.clone()).or_default().clone()
        };
        let mut slot = cell.lock().await;
        if let Some(pool) = slot.as_ref() {
            return Ok(pool.clone());
        }
        let install: std::pin::Pin<Box<dyn Future<Output = Result<PgPool, String>> + Send + '_>> =
            Box::pin(self.install(app, &db));
        let pool = install.await?;
        *slot = Some(pool.clone());
        Ok(pool)
    }

    async fn install(&self, app: &PgPool, db: &str) -> Result<PgPool, String> {
        let s = |e: sqlx::Error| format!("installing {}: {e}", self.name);
        let role = role_name(&self.name, db);
        let schema = schema_name(&self.name);
        for (table, columns) in &self.grants.core_reads {
            if DENIED
                .iter()
                .any(|(t, c)| t == table && c.is_none_or(|c| columns.iter().any(|col| col == c)))
            {
                return Err(format!("{}: core_reads may not name {table}", self.name));
            }
        }
        // Adopted tables are a plugin's own that live in `public` (a Rails
        // plugin's, on a restored site): never core's.
        let structure = crate::schema::structure_sql();
        if let Some(table) = self.grants.adopted.iter().find(|t| {
            DENIED.iter().any(|(d, _)| d == t)
                || structure.contains(&format!("CREATE TABLE public.{t} ("))
        }) {
            return Err(format!("{}: may not adopt core's {table}", self.name));
        }
        can_create_roles(app).await?;
        let mut tx = app.begin().await.map_err(s)?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(format!("plugin role {role}"))
            .execute(&mut *tx)
            .await
            .map_err(s)?;
        run_sql(
            &mut tx,
            "CREATE TABLE IF NOT EXISTS discourse_rs.plugin_roles \
             (role text PRIMARY KEY, password text NOT NULL)",
        )
        .await
        .map_err(s)?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)")
                .bind(&role)
                .fetch_one(&mut *tx)
                .await
                .map_err(s)?;
        let stored: Option<String> =
            sqlx::query_scalar("SELECT password FROM discourse_rs.plugin_roles WHERE role = $1")
                .bind(&role)
                .fetch_optional(&mut *tx)
                .await
                .map_err(s)?;
        let password = match (exists, stored) {
            (true, Some(password)) => password,
            _ => {
                // Hex: safe to put in the statement, which takes no binds.
                let password: String = (0..32)
                    .map(|_| format!("{:02x}", rand::random::<u8>()))
                    .collect();
                sqlx::query(
                    "INSERT INTO discourse_rs.plugin_roles (role, password) VALUES ($1, $2) \
                     ON CONFLICT (role) DO UPDATE SET password = EXCLUDED.password",
                )
                .bind(&role)
                .bind(&password)
                .execute(&mut *tx)
                .await
                .map_err(s)?;
                password
            }
        };
        // Every boot, not only the first: any role may change its own
        // password and session defaults, so what a guest set there lasts
        // until the next.
        let verb = if exists { "ALTER" } else { "CREATE" };
        let grants = [
            format!(
                "{verb} ROLE {} LOGIN NOINHERIT NOCREATEDB NOCREATEROLE PASSWORD '{password}'",
                quote(&role)
            ),
            format!("ALTER ROLE {} RESET ALL", quote(&role)),
            format!(
                "ALTER ROLE {} IN DATABASE {} RESET ALL",
                quote(&role),
                quote(db)
            ),
            format!(
                "ALTER ROLE {} SET temp_file_limit = '{TEMP_FILE_LIMIT}'",
                quote(&role)
            ),
            // The app may cancel the role's queries (pg_cancel_backend
            // wants the privileges of the backend's role).
            format!("GRANT {} TO CURRENT_USER", quote(&role)),
            format!(
                "CREATE SCHEMA IF NOT EXISTS {} AUTHORIZATION {}",
                quote(&schema),
                quote(&role)
            ),
            // Its own schema first, where its migrations create; then
            // public, where every name still meets the grants below.
            format!(
                "ALTER ROLE {} IN DATABASE {} SET search_path = {}, public",
                quote(&role),
                quote(db),
                quote(&schema)
            ),
            format!(
                "REVOKE ALL ON ALL TABLES IN SCHEMA public FROM {}",
                quote(&role)
            ),
            format!(
                "REVOKE ALL ON ALL SEQUENCES IN SCHEMA public FROM {}",
                quote(&role)
            ),
        ];
        for sql in grants {
            run_sql(&mut tx, sql.as_str()).await.map_err(s)?;
        }
        // A database upgraded from PostgreSQL 14 or older keeps CREATE on
        // public for everyone: a plugin could put functions and operators
        // there that the app's own SQL would resolve to, and run them as
        // the app.
        let creates_in_public: bool =
            sqlx::query_scalar("SELECT has_schema_privilege($1, 'public', 'CREATE')")
                .bind(&role)
                .fetch_one(&mut *tx)
                .await
                .map_err(s)?;
        if creates_in_public {
            return Err(format!(
                "installing {}: every role may create in the public schema; run \
                 REVOKE CREATE ON SCHEMA public FROM PUBLIC (the default since PostgreSQL 15)",
                self.name
            ));
        }
        let mut core: Vec<_> = self.grants.core_reads.iter().collect();
        core.sort();
        for (table, columns) in core {
            let columns: Vec<String> = columns.iter().map(|c| quote(c)).collect();
            let sql = format!(
                "GRANT SELECT ({}) ON public.{} TO {}",
                columns.join(", "),
                quote(table),
                quote(&role)
            );
            run_sql(&mut tx, sql.as_str()).await.map_err(s)?;
        }
        for table in &self.grants.adopted {
            let sql = format!(
                "GRANT SELECT, INSERT, UPDATE, DELETE ON public.{} TO {}",
                quote(table),
                quote(&role)
            );
            run_sql(&mut tx, sql.as_str()).await.map_err(s)?;
            // Its id sequences, for nextval on insert.
            let sequences: Vec<String> = sqlx::query_scalar(
                "SELECT s.relname::text FROM pg_class s \
                 JOIN pg_depend d ON d.objid = s.oid AND d.deptype IN ('a', 'i') \
                 JOIN pg_class t ON t.oid = d.refobjid \
                 JOIN pg_namespace n ON n.oid = t.relnamespace \
                 WHERE s.relkind = 'S' AND n.nspname = 'public' AND t.relname = $1",
            )
            .bind(table)
            .fetch_all(&mut *tx)
            .await
            .map_err(s)?;
            for sequence in sequences {
                let sql = format!(
                    "GRANT USAGE, SELECT ON SEQUENCE public.{} TO {}",
                    quote(&sequence),
                    quote(&role)
                );
                run_sql(&mut tx, sql.as_str()).await.map_err(s)?;
            }
        }
        tx.commit().await.map_err(s)?;

        let options = (*app.connect_options())
            .clone()
            .username(&role)
            .password(&password)
            // DISCARD ALL on release drops prepared statements, so none
            // are kept for reuse.
            .statement_cache_capacity(0);
        let pool = PgPoolOptions::new()
            .max_connections(POOL_SIZE)
            .min_connections(0)
            .idle_timeout(Duration::from_secs(60))
            // What a guest left in its session goes when the connection
            // comes back (reset_session).
            .after_release(reset_session)
            .connect_lazy_with(options);
        self.migrate(&pool).await?;
        Ok(pool)
    }

    /// `migrations/*.sql`, each once, as the role: whatever they create is
    /// the role's, in its schema.
    async fn migrate(&self, pool: &PgPool) -> Result<(), String> {
        let s = |e: sqlx::Error| format!("migrating {}: {e}", self.name);
        let mut tx = pool.begin().await.map_err(s)?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(format!("plugin migrations {}", self.name))
            .execute(&mut *tx)
            .await
            .map_err(s)?;
        run_sql(
            &mut tx,
            "CREATE TABLE IF NOT EXISTS schema_migrations (version text PRIMARY KEY)",
        )
        .await
        .map_err(s)?;
        let done: Vec<String> = sqlx::query_scalar("SELECT version FROM schema_migrations")
            .fetch_all(&mut *tx)
            .await
            .map_err(s)?;
        for (version, sql) in &self.files.migrations {
            if done.contains(version) {
                continue;
            }
            sqlx::Executor::execute(&mut *tx, sql.as_str())
                .await
                .map_err(|e| format!("migrating {} ({version}): {e}", self.name))?;
            sqlx::query("INSERT INTO schema_migrations (version) VALUES ($1)")
                .bind(version)
                .execute(&mut *tx)
                .await
                .map_err(s)?;
        }
        tx.commit().await.map_err(s)
    }
}

/// `after_release`: RESET ALL.
fn reset_session(
    conn: &mut PgConnection,
    _: sqlx::pool::PoolConnectionMetadata,
) -> futures_util::future::BoxFuture<'_, Result<bool, sqlx::Error>> {
    Box::pin(async move {
        sqlx::Executor::execute(&mut *conn, "DISCARD ALL").await?;
        Ok(true)
    })
}

/// Whether the app's role may make plugin roles: a superuser, or a role
/// with CREATEROLE on PostgreSQL 16 or later. Before 16, CREATEROLE lets a
/// role grant itself the predefined roles that read and write the
/// server's files, so the app is not asked to hold it there.
async fn can_create_roles(app: &PgPool) -> Result<(), String> {
    let (user, superuser, createrole, version): (String, bool, bool, i32) = sqlx::query_as(
        "SELECT rolname::text, rolsuper, rolcreaterole, \
                current_setting('server_version_num')::int \
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(app)
    .await
    .map_err(|e| format!("checking the app's role: {e}"))?;
    if superuser {
        return Ok(());
    }
    if version < 160000 {
        return Err(format!(
            "plugins with a database need PostgreSQL 16 or later when the app's role ({user}) \
             is not a superuser"
        ));
    }
    if !createrole {
        return Err(format!(
            "plugins with a database need the app's role to create their roles: \
             ALTER ROLE {} CREATEROLE",
            quote(&user)
        ));
    }
    let sets_temp_file_limit: bool =
        sqlx::query_scalar("SELECT has_parameter_privilege('temp_file_limit', 'SET')")
            .fetch_one(app)
            .await
            .map_err(|e| format!("checking the app's role: {e}"))?;
    if !sets_temp_file_limit {
        return Err(format!(
            "plugins with a database need the app's role to cap their temporary files: \
             GRANT SET ON PARAMETER temp_file_limit TO {}",
            quote(&user)
        ));
    }
    Ok(())
}

/// A statement of no binds (DDL, SET), by the simple protocol.
async fn run_sql(conn: &mut PgConnection, sql: &str) -> Result<(), sqlx::Error> {
    sqlx::Executor::execute(conn, sql).await.map(|_| ())
}

/// A hook call's transaction on the plugin's pool.
pub(super) struct HookTx {
    tx: Transaction<'static, Postgres>,
    pid: i32,
    /// `now()` as the transaction began: a statement that sees another
    /// ran outside it.
    began: String,
    /// The site's app pool, which cancels a query past the budget.
    app: PgPool,
}

impl HookTx {
    pub(super) async fn begin(
        pool: &PgPool,
        app: &PgPool,
        writes: bool,
        deadline: Instant,
    ) -> Result<HookTx, String> {
        let s = |e: sqlx::Error| e.to_string();
        let begin = tokio::time::timeout_at(deadline, pool.begin());
        let mut tx = begin
            .await
            .map_err(|_| "no plugin connection within the time limit".to_string())?
            .map_err(s)?;
        if !writes {
            run_sql(&mut tx, "SET TRANSACTION READ ONLY")
                .await
                .map_err(s)?;
        }
        let left = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .max(1);
        run_sql(
            &mut tx,
            format!("SET LOCAL statement_timeout = {left}").as_str(),
        )
        .await
        .map_err(s)?;
        let (pid, began): (i32, String) = sqlx::query_as("SELECT pg_backend_pid(), now()::text")
            .fetch_one(&mut *tx)
            .await
            .map_err(s)?;
        Ok(HookTx {
            tx,
            pid,
            began,
            app: app.clone(),
        })
    }

    pub(super) async fn commit(self) -> Result<(), String> {
        self.tx.commit().await.map_err(|e| e.to_string())
    }

    /// `query`: the rows of a statement (a SELECT, or a write with
    /// RETURNING) as JSON objects.
    pub(super) async fn query(
        &mut self,
        sql: &str,
        params: &[Value],
        deadline: Instant,
    ) -> Result<Result<Vec<Value>, String>, String> {
        let wrapped = format!("WITH q AS ({sql}) SELECT row_to_json(q)::text FROM q");
        let pid = self.pid;
        let app = self.app.clone();
        let query = Box::pin(
            bind(sqlx::query_scalar::<_, String>(&wrapped), params).fetch_all(&mut *self.tx),
        );
        let rows = match within(query, deadline, &app, pid).await? {
            Ok(rows) => rows,
            Err(e) => return Ok(Err(e.to_string())),
        };
        if rows.len() > MAX_ROWS {
            return Ok(Err(format!("more than {MAX_ROWS} rows")));
        }
        Ok(Ok(rows
            .iter()
            .map(|r| serde_json::from_str(r).unwrap_or(Value::Null))
            .collect()))
    }

    /// `execute`: a statement's rows affected.
    pub(super) async fn execute(
        &mut self,
        sql: &str,
        params: &[Value],
        deadline: Instant,
    ) -> Result<Result<u64, String>, String> {
        let pid = self.pid;
        let app = self.app.clone();
        let query = Box::pin(bind_query(sqlx::query(sql), params).execute(&mut *self.tx));
        let result = within(query, deadline, &app, pid).await?;
        // A statement that failed leaves the transaction aborted, not
        // ended (a COMMIT after it rolls back, and succeeds).
        if result.is_ok() {
            self.still_open().await?;
        }
        Ok(result.map(|r| r.rows_affected()).map_err(|e| e.to_string()))
    }

    /// Fails the hook when the guest's statement ended the transaction
    /// (COMMIT, ROLLBACK): what it ran next would commit on its own,
    /// writable under GET and kept when the hook fails. `query` can't, as
    /// its statement sits in a CTE.
    async fn still_open(&mut self) -> Result<(), String> {
        let now: String = sqlx::query_scalar("SELECT now()::text")
            .fetch_one(&mut *self.tx)
            .await
            .map_err(|e| e.to_string())?;
        if now != self.began {
            return Err("a plugin's statement ended its transaction".into());
        }
        Ok(())
    }
}

/// Runs `query` until `deadline`; past it, cancels the backend and waits
/// for the query to give up (dropping it mid-flight would leave the
/// connection unusable). The outer error is the time limit, which fails
/// the hook; the inner one is the statement's, which the guest sees.
async fn within<T>(
    mut query: std::pin::Pin<Box<dyn Future<Output = Result<T, sqlx::Error>> + Send + '_>>,
    deadline: Instant,
    app: &PgPool,
    pid: i32,
) -> Result<Result<T, sqlx::Error>, String> {
    match tokio::time::timeout_at(deadline, &mut query).await {
        Ok(result) => Ok(result),
        Err(_) => {
            let _ = sqlx::query("SELECT pg_cancel_backend($1)")
                .bind(pid)
                .execute(app)
                .await;
            let _ = query.await;
            Err("a query ran past the time limit".into())
        }
    }
}

/// A NULL of no declared type, which Postgres types from where it goes
/// (an int column, a text comparison), as an unknown literal is.
struct Untyped;

impl sqlx::Type<Postgres> for Untyped {
    fn type_info() -> PgTypeInfo {
        PgTypeInfo::with_oid(Oid(0))
    }
}

impl sqlx::Encode<'_, Postgres> for Untyped {
    fn encode_by_ref(&self, _: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
        Ok(IsNull::Yes)
    }
}

/// Binds JSON parameters by their JSON type: integers as bigint, other
/// numbers as double precision, strings as text, booleans as boolean,
/// null untyped, arrays and objects as their JSON text (cast with
/// `$1::jsonb`). A statement casts what it needs otherwise (`$2::date`).
macro_rules! bind_params {
    ($query:expr, $params:expr) => {{
        let mut query = $query.persistent(false);
        for param in $params {
            query = match param {
                Value::Null => query.bind(Untyped),
                Value::Bool(b) => query.bind(*b),
                Value::Number(n) => match n.as_i64() {
                    Some(i) => query.bind(i),
                    None => query.bind(n.as_f64()),
                },
                Value::String(text) => query.bind(text.clone()),
                other => query.bind(other.to_string()),
            };
        }
        query
    }};
}

fn bind<'q>(
    query: sqlx::query::QueryScalar<'q, Postgres, String, sqlx::postgres::PgArguments>,
    params: &[Value],
) -> sqlx::query::QueryScalar<'q, Postgres, String, sqlx::postgres::PgArguments> {
    bind_params!(query, params)
}

fn bind_query<'q>(
    query: sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments>,
    params: &[Value],
) -> sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments> {
    bind_params!(query, params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_identifiers_and_per_database() {
        assert_eq!(schema_name("discourse-taper"), "plugin_discourse_taper");
        let a = role_name("discourse-taper", "site_a");
        assert!(a.starts_with("dpl_discourse_taper_"), "{a}");
        assert_ne!(a, role_name("discourse-taper", "site_b"));
        assert!(role_name(&"x".repeat(100), "db").len() < 63);
    }
}
