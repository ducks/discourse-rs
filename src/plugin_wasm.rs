//! Third-party plugins as WebAssembly, in wasmtime (docs/plugin-wasm.md).
//! Spike: the topic list's lifecycle phases, and (api) every plugin_api hook.
//!
//! A plugin is a directory with `plugin.toml` (what it may read, its
//! phases, the keys it adds, its limits) and `plugin.wasm`, a core
//! WebAssembly module. The ABI is JSON over the guest's linear memory:
//!
//! - the guest exports `memory`, `dp_alloc(len) -> ptr` and one function
//!   per phase, named `<pipeline>.<phase>`, `(ptr, len) -> packed` where
//!   `packed` is the output's `ptr << 32 | len`. Input is
//!   `{"payload": ..., "guardian": ...}`, output `{"ok": ...}` or
//!   `{"error": "..."}`;
//! - the host defines, in the import module `discourse`, only the calls
//!   the manifest grants, each `(ptr, len) -> packed` with JSON in and
//!   out: `rows` (the plugin's own tables), `core_rows` (core columns the
//!   manifest lists), `setting` (site settings it lists), `t` (a server
//!   translation). There is no WASI: no clock, files, network or
//!   randomness.
//!
//! Every phase call gets a fresh instance from the pooling allocator, a
//! memory cap, a wall-clock budget (epoch interruption) and a host call
//! cap. Host calls are async host functions: the guest runs on a fiber,
//! and a call sends its request to a loop beside it on the request's task,
//! which answers on the request's own connection (as the Luau spike's
//! coroutine yield did), so a phase never takes a second connection.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;
use tokio::sync::{mpsc, oneshot};
use wasmtime::{
    Caller, Config, Engine, InstanceAllocationStrategy, InstancePre, Linker, Module,
    PoolingAllocationConfig, Store, StoreLimits, StoreLimitsBuilder,
};

use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::plugins::PosterInputs;
use crate::site_settings::{SettingError, SiteSettings};

mod api;
mod db;

pub use db::{register_site, role_name, schema_name};

/// The ABI version this host speaks (`api` in the manifest).
pub const API: u32 = 1;

/// How often the epoch advances: the wall-clock budget's resolution.
const TICK: Duration = Duration::from_millis(1);

/// The largest memory any plugin may ask for, and what the pool reserves
/// per slot.
const MAX_MEMORY_MIB: u64 = 64;

/// A plugin that could not be loaded.
#[derive(Debug)]
pub struct LoadError(pub String);

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "loading wasm plugins: {}", self.0)
    }
}

impl std::error::Error for LoadError {}

/// One wasmtime engine for every plugin: Cranelift, the pooling
/// allocator, epoch interruption, and a thread advancing the epoch.
pub struct Runtime {
    pub engine: Engine,
}

/// How the engine allocates instances; `Tuning::default()` is the
/// server's, the others are for measurements (docs/plugin-wasm.md).
#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    /// The pooling allocator, else on demand.
    pub pooling: bool,
    /// What a released memory slot keeps mapped and resets in place
    /// instead of handing back to the kernel (madvise), so the next
    /// instance does not fault it in again.
    pub keep_resident: usize,
    /// Reset only the pages a guest dirtied (Linux 6.7's PAGEMAP_SCAN)
    /// rather than memset all of `keep_resident`.
    pub pagemap_scan: bool,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            pooling: true,
            keep_resident: 1 << 20,
            pagemap_scan: true,
        }
    }
}

impl Runtime {
    pub fn new(tuning: Tuning) -> Result<Runtime, LoadError> {
        let mut config = Config::new();
        config.epoch_interruption(true);
        config.max_wasm_stack(512 << 10);
        config.async_stack_size(1 << 20);
        if tuning.pooling {
            let mut pool = PoolingAllocationConfig::default();
            // Concurrent phase calls across the process.
            pool.total_core_instances(256);
            pool.total_memories(256);
            pool.total_tables(256);
            pool.total_stacks(256);
            pool.max_memory_size((MAX_MEMORY_MIB << 20) as usize);
            pool.linear_memory_keep_resident(tuning.keep_resident);
            pool.async_stack_keep_resident(tuning.keep_resident.min(256 << 10));
            if tuning.pagemap_scan {
                pool.pagemap_scan(wasmtime::Enabled::Auto);
            }
            config.allocation_strategy(InstanceAllocationStrategy::Pooling(pool));
        }
        let engine = Engine::new(&config).map_err(|e| LoadError(e.to_string()))?;
        let ticking = engine.weak();
        std::thread::Builder::new()
            .name("wasm-epoch".into())
            .spawn(move || {
                while let Some(engine) = ticking.upgrade() {
                    engine.increment_epoch();
                    drop(engine);
                    std::thread::sleep(TICK);
                }
            })
            .map_err(|e| LoadError(e.to_string()))?;
        Ok(Runtime { engine })
    }
}

static RUNTIME: LazyLock<Result<Runtime, String>> =
    LazyLock::new(|| Runtime::new(Tuning::default()).map_err(|e| e.to_string()));

/// The process' engine.
pub fn runtime() -> Result<&'static Runtime, LoadError> {
    RUNTIME.as_ref().map_err(|e| LoadError(e.clone()))
}

/// A key's declared type, which the host coerces the phase's value to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyType {
    Bool,
    Int,
    String,
    Object,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    AfterLoad,
    BeforeSerialize,
    AfterSerialize,
    /// `route.handle`: one of the plugin's routes.
    Route,
    /// `job.run`: a regular or scheduled job.
    Job,
    /// `serializer.current_user` and `serializer.topic_view`.
    CurrentUser,
    TopicView,
    /// `outlet.topic_above_posts`.
    TopicAbovePosts,
    /// `outlet.post_menu`.
    PostMenu,
    /// `homepage.available`.
    HomepageAvailable,
}

impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Phase::AfterLoad => "after_load",
            Phase::BeforeSerialize => "before_serialize",
            Phase::AfterSerialize => "after_serialize",
            Phase::Route => "handle",
            Phase::Job => "run",
            Phase::CurrentUser => "current_user",
            Phase::TopicView => "topic_view",
            Phase::TopicAbovePosts => "topic_above_posts",
            Phase::PostMenu => "post_menu",
            Phase::HomepageAvailable => "available",
        }
    }

    /// What a phase may call. Reads only on the read pipelines; before
    /// serialize works on what after load kept, without the database, and
    /// homepage.available runs whenever `/` is resolved, so it is kept off
    /// the database too.
    fn allows(self, call: &str) -> bool {
        match self {
            Phase::BeforeSerialize | Phase::HomepageAvailable => matches!(call, "t" | "setting"),
            // Writes only where a write is the point: routes (the
            // transaction is read only under GET and HEAD) and jobs.
            _ if call == "execute" || call == "publish_post_change" => {
                matches!(self, Phase::Route | Phase::Job)
            }
            _ => true,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pipeline {
    phases: Vec<Phase>,
    /// The keys `after_serialize` may add, `[[key, type], ...]`, in order.
    #[serde(default)]
    keys: Vec<(String, KeyType)>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    plugin: ManifestPlugin,
    /// SQL of its own (query, execute) with no migrations to say so: on
    /// core's columns its `[core_reads]` grants.
    #[serde(default)]
    database: bool,
    #[serde(default)]
    limits: Limits,
    /// `read`: site settings the plugin may read with `setting`.
    #[serde(default)]
    settings: SettingsGrant,
    /// table => the columns the plugin may read with `rows`.
    #[serde(default)]
    tables: HashMap<String, Vec<String>>,
    /// core table => the columns the plugin may read with `core_rows`.
    #[serde(default)]
    core_reads: HashMap<String, Vec<String>>,
    /// `tables`: existing tables in `public`, not core's, that the plugin
    /// reads and writes as its own (a Rails plugin's, on a restored site).
    #[serde(default)]
    adopt: Adopt,
    #[serde(default)]
    topic_list: Option<Pipeline>,
    /// `[[routes]]`: mounted beside core's, answered by `route.handle`.
    #[serde(default)]
    routes: Vec<RouteDecl>,
    /// Regular jobs (`Jobs.enqueue`), run by `job.run`.
    #[serde(default)]
    jobs: Vec<String>,
    /// `[[scheduled]]`: scheduled jobs, run by `job.run` too.
    #[serde(default)]
    scheduled: Vec<ScheduledDecl>,
    /// Keys `serializer.current_user` adds to `/session/current.json`.
    #[serde(default)]
    current_user: Option<Keys>,
    /// Keys `serializer.topic_view` adds to the topic view.
    #[serde(default)]
    topic_view: Option<Keys>,
    #[serde(default)]
    outlets: Outlets,
    #[serde(default)]
    homepage: Option<HomepageDecl>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Adopt {
    tables: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsGrant {
    read: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RouteDecl {
    /// get, post, put, patch or delete.
    method: String,
    /// An axum path: `/echo/{id}`.
    path: String,
    /// A page with a form: its input carries the session's masked CSRF
    /// token (`form_authenticity_token`), its response the session cookie.
    #[serde(default)]
    form: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduledDecl {
    name: String,
    every_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Keys {
    /// `[[key, type], ...]`, in order.
    keys: Vec<(String, KeyType)>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Outlets {
    #[serde(default)]
    topic_above_posts: bool,
    /// Buttons in each post's menu (`api.addPostMenuButton`).
    #[serde(default)]
    post_menu: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct HomepageDecl {
    id: String,
    name: String,
    path: String,
    #[serde(default)]
    anonymous: bool,
    #[serde(default)]
    server_side: bool,
    /// The module exports `homepage.available`; without it the option is
    /// always available while the plugin is enabled.
    #[serde(default)]
    available: bool,
}

/// What a plugin's directory carries beside the module.
#[derive(Debug, Default)]
pub struct Files {
    /// `settings.yml`, of plugin_settings.yml's shape.
    pub settings_yml: Option<String>,
    /// `locales/*.yml`, by file name.
    pub locales: Vec<String>,
    /// `public/`, served at `/plugins/<name>/` as Rails serves a plugin's
    /// public directory.
    pub public: Option<std::path::PathBuf>,
    /// `migrations/*.sql`: (file stem, SQL), in file order.
    pub migrations: Vec<(String, String)>,
}

impl Files {
    pub fn read(dir: &Path) -> Result<Files, LoadError> {
        let fail = |path: &Path, e: std::io::Error| LoadError(format!("{}: {e}", path.display()));
        let settings = dir.join("settings.yml");
        let settings_yml = match std::fs::read_to_string(&settings) {
            Ok(src) => Some(src),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(fail(&settings, e)),
        };
        let mut locales = Vec::new();
        let locale_dir = dir.join("locales");
        if locale_dir.is_dir() {
            let mut paths: Vec<_> = std::fs::read_dir(&locale_dir)
                .map_err(|e| fail(&locale_dir, e))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "yml"))
                .collect();
            paths.sort();
            for path in paths {
                locales.push(std::fs::read_to_string(&path).map_err(|e| fail(&path, e))?);
            }
        }
        let mut migrations = Vec::new();
        let migration_dir = dir.join("migrations");
        if migration_dir.is_dir() {
            let mut paths: Vec<_> = std::fs::read_dir(&migration_dir)
                .map_err(|e| fail(&migration_dir, e))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "sql"))
                .collect();
            paths.sort();
            for path in paths {
                let version = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                migrations.push((
                    version,
                    std::fs::read_to_string(&path).map_err(|e| fail(&path, e))?,
                ));
            }
        }
        let public = dir.join("public");
        Ok(Files {
            settings_yml,
            locales,
            public: public.is_dir().then_some(public),
            migrations,
        })
    }

    /// The settings `settings.yml` declares, which the plugin reads
    /// without listing them under `[settings] read`.
    fn own_settings(&self) -> Result<Vec<String>, String> {
        let Some(src) = &self.settings_yml else {
            return Ok(Vec::new());
        };
        let doc: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(src).map_err(|e| format!("settings.yml: {e}"))?;
        let mut names = Vec::new();
        if let Some(categories) = doc.get("settings").and_then(|s| s.as_mapping()) {
            for (_, settings) in categories {
                if let Some(settings) = settings.as_mapping() {
                    names.extend(settings.keys().filter_map(|k| k.as_str().map(String::from)));
                }
            }
        }
        Ok(names)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestPlugin {
    name: String,
    api: u32,
    /// The site setting that turns the plugin on.
    enabled_setting: Option<String>,
    /// A bundled plugin (src/plugins) whose phases on the same pipelines
    /// this one takes over while enabled.
    replaces: Option<String>,
    /// `register_svg_icon`.
    #[serde(default)]
    svg_icons: Vec<String>,
    /// `register_asset` for a stylesheet: a file under `public/`.
    stylesheet: Option<String>,
}

/// Per plugin, from the manifest's `[limits]`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Limits {
    /// Linear memory, at most MAX_MEMORY_MIB.
    pub memory_mib: u64,
    /// Wall-clock budget of one phase call, host calls included.
    pub time_ms: u64,
    /// Host calls per phase call.
    pub host_calls: u32,
    /// Wall-clock budget of one job.
    pub job_time_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            memory_mib: 16,
            time_ms: 50,
            host_calls: 64,
            job_time_ms: 30_000,
        }
    }
}

/// What a plugin may read.
#[derive(Debug)]
struct Grants {
    settings: Vec<String>,
    tables: HashMap<String, Vec<String>>,
    core_reads: HashMap<String, Vec<String>>,
    adopted: Vec<String>,
}

pub struct Plugin {
    pub name: String,
    enabled_setting: Option<String>,
    replaces: Option<String>,
    svg_icons: Vec<String>,
    stylesheet: Option<String>,
    limits: Limits,
    grants: Grants,
    topic_list: Option<Pipeline>,
    routes: Vec<RouteDecl>,
    jobs: Vec<String>,
    scheduled: Vec<ScheduledDecl>,
    current_user: Option<Keys>,
    topic_view: Option<Keys>,
    outlets: Outlets,
    homepage: Option<HomepageDecl>,
    files: Files,
    /// Its own schema and SQL (db.rs).
    database: bool,
    pools: db::Pools,
    pre: InstancePre<HostState>,
}

/// The plugins of the process.
#[derive(Default)]
pub struct Registry {
    plugins: Vec<Arc<Plugin>>,
}

static REGISTRY: LazyLock<RwLock<Arc<Registry>>> =
    LazyLock::new(|| RwLock::new(Arc::new(Registry::default())));

/// The installed plugins (none until `install`).
pub fn registry() -> Arc<Registry> {
    REGISTRY.read().map(|r| r.clone()).unwrap_or_default()
}

/// Makes `registry` the process' plugins (at boot; tests).
pub fn install(registry: Registry) {
    if let Ok(mut current) = REGISTRY.write() {
        *current = Arc::new(registry);
    }
}

/// Each installed plugin's database on the site of `app` (its role,
/// schema and migrations), at boot: a site that can't give a plugin its
/// database stops the boot rather than failing the plugin's every hook.
pub async fn install_databases(app: &sqlx::PgPool) -> Result<(), String> {
    for plugin in &registry().plugins {
        if plugin.database {
            plugin.db_pool(app).await?;
        }
    }
    Ok(())
}

/// Loads `PLUGINS_DIR` when set, installs it for the topic list's phases,
/// and hands back the plugins for plugin_api's registry (every other
/// hook). A broken plugin stops the boot.
pub fn load_from_env() -> Result<Vec<Arc<dyn crate::plugin_api::Plugin>>, LoadError> {
    let Some(dir) = std::env::var_os("PLUGINS_DIR") else {
        return Ok(Vec::new());
    };
    let registry = Registry::load(Path::new(&dir))?;
    let plugins = registry.api_plugins();
    install(registry);
    Ok(plugins)
}

impl Registry {
    /// Every directory under `dir` with a `plugin.toml`, by name.
    pub fn load(dir: &Path) -> Result<Registry, LoadError> {
        let entries =
            std::fs::read_dir(dir).map_err(|e| LoadError(format!("{}: {e}", dir.display())))?;
        let mut dirs: Vec<_> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.join("plugin.toml").is_file())
            .collect();
        dirs.sort();
        let mut registry = Registry::default();
        for path in dirs {
            let read = |name: &str| {
                std::fs::read(path.join(name))
                    .map_err(|e| LoadError(format!("{}/{name}: {e}", path.display())))
            };
            registry.add(Plugin::load_dir(runtime()?, &path, &read("plugin.wasm")?)?);
        }
        Ok(registry)
    }

    pub fn add(&mut self, plugin: Plugin) {
        self.plugins.push(Arc::new(plugin));
    }

    /// Each plugin as plugin_api sees it.
    pub fn api_plugins(&self) -> Vec<Arc<dyn crate::plugin_api::Plugin>> {
        self.plugins
            .iter()
            .map(|p| {
                Arc::new(api::WasmPlugin::new(p.clone())) as Arc<dyn crate::plugin_api::Plugin>
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    fn enabled<'a>(
        &'a self,
        settings: &SiteSettings,
    ) -> Result<impl Iterator<Item = &'a Plugin>, SettingError> {
        let mut out = Vec::new();
        for plugin in &self.plugins {
            match &plugin.enabled_setting {
                Some(s) if !settings.get(s)?.truthy() => {}
                _ => out.push(plugin.as_ref()),
            }
        }
        Ok(out.into_iter())
    }

    /// The bundled plugins enabled plugins replace.
    pub fn replaced(&self, settings: &SiteSettings) -> Result<Vec<String>, SettingError> {
        Ok(self
            .enabled(settings)?
            .filter_map(|p| p.replaces.clone())
            .collect())
    }

    fn topic_list<'a>(
        &'a self,
        settings: &SiteSettings,
        phase: Phase,
    ) -> Result<Vec<(&'a Plugin, &'a Pipeline)>, SettingError> {
        Ok(self
            .enabled(settings)?
            .filter_map(|p| p.topic_list.as_ref().map(|l| (p, l)))
            .filter(|(_, l)| l.phases.contains(&phase))
            .collect())
    }

    /// Whether any enabled plugin runs on the topic list at all.
    pub fn on_topic_list(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        Ok(self
            .enabled(settings)?
            .any(|p| p.topic_list.as_ref().is_some_and(|l| !l.phases.is_empty())))
    }

    /// The topic list's after load and before serialize, for a page:
    /// `facts` are the topics' payloads, `posters` each topic's poster
    /// inputs (core's, after the bundled plugins' rewrites).
    pub async fn topic_list_load(
        &self,
        ctx: &mut Ctx<'_>,
        facts: Vec<Value>,
        mut posters: HashMap<i32, PosterInputs>,
        last_posters: &HashMap<i32, i32>,
    ) -> Result<TopicListPage, SettingError> {
        let mut page = TopicListPage {
            facts,
            ..TopicListPage::default()
        };
        if page.facts.is_empty() {
            return Ok(page);
        }
        for (plugin, _) in self.topic_list(ctx.settings, Phase::AfterLoad)? {
            let result = plugin
                .run(ctx, "topic_list", Phase::AfterLoad, json!(page.facts))
                .await
                .and_then(|v| {
                    serde_json::from_value::<AfterLoadResult>(v).map_err(|e| e.to_string())
                });
            match result {
                Ok(result) => {
                    page.user_ids.extend(result.user_ids);
                    page.data.insert(plugin.name.clone(), result.topics);
                }
                Err(e) => plugin.log_skip(Phase::AfterLoad, &e),
            }
        }
        let rewriting = self.topic_list(ctx.settings, Phase::BeforeSerialize)?;
        if rewriting.is_empty() {
            return Ok(page);
        }
        for (plugin, _) in rewriting {
            let data = page.data.get(&plugin.name);
            let mut payload = Map::new();
            for (id, inputs) in &posters {
                let key = id.to_string();
                let mut topic = json!({
                    "posters": inputs,
                    "last_post_user_id": last_posters.get(id),
                });
                if let Some(data) = data.and_then(|d| d.get(&key)) {
                    topic["data"] = data.clone();
                }
                payload.insert(key, topic);
            }
            let result = plugin
                .run(
                    ctx,
                    "topic_list",
                    Phase::BeforeSerialize,
                    Value::Object(payload),
                )
                .await
                .and_then(rewritten_posters);
            match result {
                Ok(rewritten) => {
                    for (id, inputs) in rewritten {
                        if let Some(slot) = posters.get_mut(&id) {
                            *slot = inputs;
                        }
                    }
                }
                Err(e) => plugin.log_skip(Phase::BeforeSerialize, &e),
            }
        }
        page.posters = posters;
        Ok(page)
    }

    /// The topic list's after serialize for the page, once: the keys each
    /// plugin adds to a topic, coerced and in declared order.
    pub async fn topic_list_keys(
        &self,
        ctx: &mut Ctx<'_>,
        page: &mut TopicListPage,
    ) -> Result<(), SettingError> {
        if page.keys.is_some() {
            return Ok(());
        }
        let mut keys: HashMap<String, Vec<(String, Value)>> = HashMap::new();
        for (plugin, pipeline) in self.topic_list(ctx.settings, Phase::AfterSerialize)? {
            let data = page.data.get(&plugin.name);
            let payload: Vec<Value> = page
                .facts
                .iter()
                .map(|f| {
                    let mut f = f.clone();
                    if let Some(data) = data.and_then(|d| d.get(&f["id"].to_string())) {
                        f["data"] = data.clone();
                    }
                    f
                })
                .collect();
            let result = plugin
                .run(ctx, "topic_list", Phase::AfterSerialize, json!(payload))
                .await
                .and_then(|v| coerce(&v, &pipeline.keys));
            match result {
                Ok(merged) => {
                    for (id, extra) in merged {
                        keys.entry(id).or_default().extend(extra);
                    }
                }
                Err(e) => plugin.log_skip(Phase::AfterSerialize, &e),
            }
        }
        page.keys = Some(keys);
        Ok(())
    }
}

/// The plugins' state for one page of the topic list.
#[derive(Debug, Default)]
pub struct TopicListPage {
    /// The topics' payloads, as after load and after serialize see them.
    facts: Vec<Value>,
    /// plugin => topic id => what its after load kept.
    data: HashMap<String, Map<String, Value>>,
    /// Users to load as well.
    pub user_ids: Vec<i32>,
    /// The poster inputs after every plugin's before serialize (empty when
    /// no plugin has one).
    pub posters: HashMap<i32, PosterInputs>,
    /// topic id => keys, once after serialize ran.
    keys: Option<HashMap<String, Vec<(String, Value)>>>,
}

impl TopicListPage {
    /// The keys for a topic, after `Registry::topic_list_keys`.
    pub fn keys(&self, topic_id: i32) -> &[(String, Value)] {
        self.keys
            .as_ref()
            .and_then(|k| k.get(&topic_id.to_string()))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn has(&self, topic_id: i32) -> bool {
        self.facts.iter().any(|f| f["id"] == topic_id)
    }
}

/// A topic as the topic list's after load and after serialize see it.
pub fn topic_facts(t: &crate::topic_query::TopicRow) -> Value {
    json!({
        "id": t.id,
        "user_id": t.user_id,
        "last_post_user_id": t.last_post_user_id,
        "category_id": t.category_id,
        "archetype": t.archetype,
        "closed": t.closed,
        "archived": t.archived,
    })
}

/// What a phase runs with: the request's connection and context.
pub struct Ctx<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub guardian: &'a Guardian,
    /// None where the hook has no translations at hand (homepage.available,
    /// which may call `setting` only).
    pub i18n: Option<&'a I18n>,
    /// The site's pool, where the caller has it; otherwise a plugin with a
    /// database finds it from the connection (db::site_pool).
    pub pool: Option<&'a sqlx::PgPool>,
    /// For URLs (a user's avatar_template), where the caller has it.
    pub config: Option<&'a crate::config::Config>,
    /// For `publish_post_change`, where the hook may publish (routes and
    /// jobs).
    pub bus: Option<&'a pg_bus::Bus>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AfterLoadResult {
    #[serde(default)]
    user_ids: Vec<i32>,
    #[serde(default)]
    topics: Map<String, Value>,
}

/// An instance's state: what its host calls may reach.
pub struct HostState {
    phase: Phase,
    calls: u32,
    max_calls: u32,
    requests: mpsc::UnboundedSender<Request>,
    limits: StoreLimits,
}

/// A host call on its way to the request's task.
struct Request {
    call: HostCall,
    reply: oneshot::Sender<Result<Vec<u8>, String>>,
}

#[derive(Debug)]
enum HostCall {
    Rows {
        core: bool,
        table: String,
        column: String,
        ids: Vec<i64>,
    },
    Setting(String),
    T {
        key: String,
        locale: Option<String>,
        args: Vec<(String, String)>,
    },
    /// Users' public cards: id, username, name, avatar_template.
    Users(Vec<i64>),
    /// `guardian.can_see_category?` for the hook's viewer.
    CanSeeCategory(i64),
    /// `post.publish_change_to_clients!(:acted)`, once the hook has
    /// committed: each viewer's page redraws the post as they see it.
    PublishPostChange(i32),
    /// SQL on the plugin's own connection (db.rs): rows, or rows affected.
    Sql {
        execute: bool,
        sql: String,
        params: Vec<Value>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TArgs {
    key: String,
    locale: Option<String>,
    #[serde(default)]
    args: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SqlArgs {
    sql: String,
    #[serde(default)]
    params: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RowsArgs {
    table: String,
    column: String,
    ids: Vec<i64>,
}

/// The import module's functions, each granted by the manifest.
const HOST_CALLS: [&str; 9] = [
    "rows",
    "core_rows",
    "setting",
    "t",
    "query",
    "execute",
    "users",
    "can_see_category",
    "publish_post_change",
];

impl Plugin {
    /// Compiles `wasm` and links it against the calls `manifest` grants;
    /// a module importing anything else does not load.
    pub fn new(runtime: &Runtime, manifest: &str, wasm: &[u8]) -> Result<Plugin, LoadError> {
        Plugin::build(runtime, manifest, wasm, Files::default())
    }

    /// A plugin directory's manifest and files, with `wasm` as its module.
    pub fn load_dir(runtime: &Runtime, dir: &Path, wasm: &[u8]) -> Result<Plugin, LoadError> {
        let manifest = std::fs::read_to_string(dir.join("plugin.toml"))
            .map_err(|e| LoadError(format!("{}/plugin.toml: {e}", dir.display())))?;
        Plugin::build(runtime, &manifest, wasm, Files::read(dir)?)
    }

    fn build(
        runtime: &Runtime,
        manifest: &str,
        wasm: &[u8],
        files: Files,
    ) -> Result<Plugin, LoadError> {
        let mut manifest: Manifest =
            toml::from_str(manifest).map_err(|e| LoadError(format!("plugin.toml: {e}")))?;
        let name = manifest.plugin.name.clone();
        // Its own settings, and the one that turns it on, need no grant.
        let own = files
            .own_settings()
            .map_err(|e| LoadError(format!("{name}: {e}")))?;
        for setting in own
            .into_iter()
            .chain(manifest.plugin.enabled_setting.clone())
        {
            if !manifest.settings.read.contains(&setting) {
                manifest.settings.read.push(setting);
            }
        }
        if manifest.plugin.stylesheet.is_some() && files.public.is_none() {
            return Err(LoadError(format!("{name}: a stylesheet but no public/")));
        }
        let fail = |e: wasmtime::Error| LoadError(format!("{name}: {e}"));
        if manifest.plugin.api != API {
            return Err(LoadError(format!(
                "{name}: api {} (this host speaks {API})",
                manifest.plugin.api
            )));
        }
        if manifest.limits.memory_mib > MAX_MEMORY_MIB {
            return Err(LoadError(format!(
                "{name}: memory_mib over {MAX_MEMORY_MIB}"
            )));
        }
        if let Some(route) = manifest
            .routes
            .iter()
            .find(|r| api::method_filter(&r.method).is_none())
        {
            return Err(LoadError(format!(
                "{name}: route {} has method {}",
                route.path, route.method
            )));
        }
        let database =
            manifest.database || !files.migrations.is_empty() || !manifest.adopt.tables.is_empty();
        let module = Module::new(&runtime.engine, wasm).map_err(fail)?;
        let mut linker: Linker<HostState> = Linker::new(&runtime.engine);
        for call in HOST_CALLS {
            let granted = match call {
                "rows" => !manifest.tables.is_empty(),
                "core_rows" => !manifest.core_reads.is_empty(),
                "setting" => !manifest.settings.read.is_empty(),
                "query" | "execute" => database,
                _ => true,
            };
            if granted {
                define(&mut linker, call).map_err(fail)?;
            }
        }
        let pre = linker.instantiate_pre(&module).map_err(fail)?;
        // Every phase the manifest declares is an export of the module.
        let exports: Vec<String> = module.exports().map(|e| e.name().to_string()).collect();
        let mut needed = vec!["memory".to_string(), "dp_alloc".to_string()];
        if let Some(list) = &manifest.topic_list {
            needed.extend(
                list.phases
                    .iter()
                    .map(|p| format!("topic_list.{}", p.name())),
            );
        }
        for (declared, export) in [
            (!manifest.routes.is_empty(), "route.handle"),
            (
                !manifest.jobs.is_empty() || !manifest.scheduled.is_empty(),
                "job.run",
            ),
            (manifest.current_user.is_some(), "serializer.current_user"),
            (manifest.topic_view.is_some(), "serializer.topic_view"),
            (
                manifest.outlets.topic_above_posts,
                "outlet.topic_above_posts",
            ),
            (manifest.outlets.post_menu, "outlet.post_menu"),
            (
                manifest.homepage.as_ref().is_some_and(|h| h.available),
                "homepage.available",
            ),
        ] {
            if declared {
                needed.push(export.to_string());
            }
        }
        if let Some(missing) = needed.iter().find(|n| !exports.contains(n)) {
            return Err(LoadError(format!("{name}: no export {missing}")));
        }
        Ok(Plugin {
            name,
            enabled_setting: manifest.plugin.enabled_setting,
            replaces: manifest.plugin.replaces,
            svg_icons: manifest.plugin.svg_icons,
            stylesheet: manifest.plugin.stylesheet,
            limits: manifest.limits,
            grants: Grants {
                settings: manifest.settings.read,
                tables: manifest.tables,
                core_reads: manifest.core_reads,
                adopted: manifest.adopt.tables,
            },
            topic_list: manifest.topic_list,
            routes: manifest.routes,
            jobs: manifest.jobs,
            scheduled: manifest.scheduled,
            current_user: manifest.current_user,
            topic_view: manifest.topic_view,
            outlets: manifest.outlets,
            homepage: manifest.homepage,
            files,
            database,
            pools: db::Pools::default(),
            pre,
        })
    }

    fn log_skip(&self, phase: Phase, error: &str) {
        tracing::error!(plugin = %self.name, phase = phase.name(), error, "plugin phase skipped");
    }

    /// `<pipeline>.<phase>(payload)` in a fresh instance, its host calls
    /// answered on the request's connection.
    pub async fn run(
        &self,
        ctx: &mut Ctx<'_>,
        pipeline: &str,
        phase: Phase,
        payload: Value,
    ) -> Result<Value, String> {
        let guardian = json!({
            "user_id": ctx.guardian.user_id(),
            "staff": ctx.guardian.is_staff(),
            "admin": ctx.guardian.is_admin(),
        });
        let route_writes = !matches!(
            payload.get("method").and_then(Value::as_str),
            Some("GET" | "HEAD")
        );
        let input = serde_json::to_vec(&json!({ "payload": payload, "guardian": guardian }))
            .map_err(|e| e.to_string())?;
        let (requests, mut incoming) = mpsc::unbounded_channel();
        let state = HostState {
            phase,
            calls: 0,
            max_calls: self.limits.host_calls,
            requests,
            limits: StoreLimitsBuilder::new()
                .memory_size((self.limits.memory_mib << 20) as usize)
                .instances(1)
                .memories(1)
                .tables(4)
                .build(),
        };
        // The site's install (role, schema, migrations) before the budget
        // starts: only the first call on a site waits for it.
        let databases = if self.database {
            let app = match ctx.pool {
                Some(pool) => pool.clone(),
                None => db::site_pool(&mut *ctx.conn).await?,
            };
            let pool = self.db_pool(&app).await?;
            Some((pool, app))
        } else {
            None
        };
        let writes = match phase {
            Phase::Job => true,
            Phase::Route => route_writes,
            _ => false,
        };
        let time_ms = match phase {
            Phase::Job => self.limits.job_time_ms,
            _ => self.limits.time_ms,
        };
        let started = std::time::Instant::now();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(time_ms);
        let export = format!("{pipeline}.{}", phase.name());
        let guest = self.call(state, export, input, time_ms);
        // Beside the guest on this task: the host calls it sends, until
        // its store (and the sender in it) is gone. SQL opens the hook's
        // transaction on the plugin's pool at its first statement.
        let serve = async {
            let mut tx: Option<db::HookTx> = None;
            let mut published: Vec<i32> = Vec::new();
            while let Some(request) = incoming.recv().await {
                let answer = match request.call {
                    HostCall::Sql {
                        execute,
                        sql,
                        params,
                    } => {
                        sql_call(
                            databases.as_ref(),
                            &mut tx,
                            writes,
                            deadline,
                            execute,
                            &sql,
                            &params,
                        )
                        .await
                    }
                    HostCall::PublishPostChange(post_id) => {
                        if !published.contains(&post_id) {
                            published.push(post_id);
                        }
                        Ok(b"true".to_vec())
                    }
                    call => answer(ctx, &self.grants, call).await,
                };
                let _ = request.reply.send(answer);
            }
            (tx, published)
        };
        let (output, (tx, published)) = tokio::join!(guest, serve);
        tracing::debug!(plugin = %self.name, phase = phase.name(), us = started.elapsed().as_micros() as u64, "plugin phase");
        let result = output.map_err(|e| format!("{e:#}")).and_then(|output| {
            let mut envelope: Value =
                serde_json::from_slice(&output).map_err(|e| format!("output: {e}"))?;
            if let Some(error) = envelope.get("error") {
                return Err(format!("plugin error: {error}"));
            }
            match envelope.get_mut("ok") {
                Some(ok) => Ok(ok.take()),
                None => Err("output is neither ok nor error".into()),
            }
        });
        // A hook's writes stand only if it succeeded; dropping the
        // transaction rolls it back.
        if let (Ok(_), Some(tx)) = (&result, tx) {
            tx.commit().await?;
        }
        // Its changes are published after they are committed, so a page
        // redrawing the post reads them.
        if result.is_ok() && !published.is_empty() {
            let bus = ctx
                .bus
                .ok_or("publish_post_change: no bus where this hook runs")?;
            for post_id in published {
                crate::bus::publish_post_change_message(
                    bus,
                    &mut *ctx.conn,
                    post_id,
                    "acted",
                    Map::new(),
                )
                .await
                .map_err(|e| e.to_string())?;
            }
        }
        result
    }

    async fn call(
        &self,
        state: HostState,
        export: String,
        input: Vec<u8>,
        time_ms: u64,
    ) -> wasmtime::Result<Vec<u8>> {
        let mut store = Store::new(self.pre.module().engine(), state);
        store.limiter(|s| &mut s.limits);
        store.set_epoch_deadline(time_ms.div_ceil(TICK.as_millis() as u64));
        let instance = self.pre.instantiate_async(&mut store).await?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| wasmtime::format_err!("no memory export"))?;
        let alloc = instance.get_typed_func::<u32, u32>(&mut store, "dp_alloc")?;
        let phase = instance.get_typed_func::<(u32, u32), u64>(&mut store, &export)?;
        let len = u32::try_from(input.len())?;
        let ptr = alloc.call_async(&mut store, len).await?;
        memory.write(&mut store, ptr as usize, &input)?;
        let packed = phase.call_async(&mut store, (ptr, len)).await?;
        read(&memory, &store, packed)
    }
}

fn read<T: 'static>(
    memory: &wasmtime::Memory,
    store: impl wasmtime::AsContext<Data = T>,
    packed: u64,
) -> wasmtime::Result<Vec<u8>> {
    let (ptr, len) = ((packed >> 32) as usize, (packed & 0xffff_ffff) as usize);
    let data = memory.data(&store);
    data.get(ptr..ptr.saturating_add(len))
        .map(|s| s.to_vec())
        .ok_or_else(|| wasmtime::format_err!("output outside memory"))
}

/// The host function `discourse.<call>`: reads the guest's request,
/// checks the phase and the call cap, has the request's task answer it,
/// and writes the answer into memory the guest allocates.
fn define(linker: &mut Linker<HostState>, call: &'static str) -> wasmtime::Result<()> {
    linker.func_wrap_async(
        "discourse",
        call,
        move |mut caller: Caller<'_, HostState>, (ptr, len): (u32, u32)| {
            Box::new(async move {
                let state = caller.data_mut();
                if !state.phase.allows(call) {
                    wasmtime::bail!("{call} is not allowed in {}", state.phase.name());
                }
                state.calls += 1;
                if state.calls > state.max_calls {
                    wasmtime::bail!("over {} host calls", state.max_calls);
                }
                let memory = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .ok_or_else(|| wasmtime::format_err!("no memory export"))?;
                let request = read(&memory, &caller, (u64::from(ptr) << 32) | u64::from(len))?;
                let call = parse(call, &request).map_err(wasmtime::Error::msg)?;
                let (reply, answer) = oneshot::channel();
                caller
                    .data()
                    .requests
                    .send(Request { call, reply })
                    .map_err(|_| wasmtime::format_err!("host gone"))?;
                let bytes = answer
                    .await
                    .map_err(|_| wasmtime::format_err!("host gone"))?
                    .map_err(wasmtime::Error::msg)?;
                let alloc = caller
                    .get_export("dp_alloc")
                    .and_then(|e| e.into_func())
                    .ok_or_else(|| wasmtime::format_err!("no dp_alloc export"))?
                    .typed::<u32, u32>(&caller)?;
                let len = u32::try_from(bytes.len())?;
                let out = alloc.call_async(&mut caller, len).await?;
                memory.write(&mut caller, out as usize, &bytes)?;
                Ok((u64::from(out) << 32) | u64::from(len))
            })
        },
    )?;
    Ok(())
}

fn parse(call: &str, request: &[u8]) -> Result<HostCall, String> {
    let text = || String::from_utf8(request.to_vec()).map_err(|e| e.to_string());
    Ok(match call {
        "rows" | "core_rows" => {
            let args: RowsArgs =
                serde_json::from_slice(request).map_err(|e| format!("{call}: {e}"))?;
            HostCall::Rows {
                core: call == "core_rows",
                table: args.table,
                column: args.column,
                ids: args.ids,
            }
        }
        "setting" => HostCall::Setting(text()?),
        // A key, or {"key", "locale", "args": {name: value}}.
        "t" => match serde_json::from_slice::<TArgs>(request) {
            Ok(args) => HostCall::T {
                key: args.key,
                locale: args.locale,
                args: args
                    .args
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            k,
                            v.as_str()
                                .map(String::from)
                                .unwrap_or_else(|| v.to_string()),
                        )
                    })
                    .collect(),
            },
            Err(_) => HostCall::T {
                key: text()?,
                locale: None,
                args: Vec::new(),
            },
        },
        "users" => {
            HostCall::Users(serde_json::from_slice(request).map_err(|e| format!("users: {e}"))?)
        }
        "can_see_category" => HostCall::CanSeeCategory(
            serde_json::from_slice(request).map_err(|e| format!("can_see_category: {e}"))?,
        ),
        "publish_post_change" => HostCall::PublishPostChange(
            serde_json::from_slice(request).map_err(|e| format!("publish_post_change: {e}"))?,
        ),
        "query" | "execute" => {
            let args: SqlArgs =
                serde_json::from_slice(request).map_err(|e| format!("{call}: {e}"))?;
            HostCall::Sql {
                execute: call == "execute",
                sql: args.sql,
                params: args.params,
            }
        }
        other => return Err(format!("no host call {other}")),
    })
}

/// `query` or `execute` on the plugin's own connection, in the hook's
/// transaction (begun at its first statement). The statement's own error
/// goes back to the guest as `{"error"}`; the time limit fails the hook.
async fn sql_call(
    databases: Option<&(sqlx::PgPool, sqlx::PgPool)>,
    tx: &mut Option<db::HookTx>,
    writes: bool,
    deadline: tokio::time::Instant,
    execute: bool,
    sql: &str,
    params: &[Value],
) -> Result<Vec<u8>, String> {
    let Some((pool, app)) = databases else {
        return Err("this plugin has no database".into());
    };
    let tx = match tx {
        Some(tx) => tx,
        None => tx.insert(db::HookTx::begin(pool, app, writes, deadline).await?),
    };
    let answer = if execute {
        tx.execute(sql, params, deadline)
            .await?
            .map(|n| json!({ "rows_affected": n }))
    } else {
        tx.query(sql, params, deadline).await?.map(Value::from)
    };
    let envelope = match answer {
        Ok(ok) => json!({ "ok": ok }),
        Err(error) => json!({ "error": error }),
    };
    serde_json::to_vec(&envelope).map_err(|e| e.to_string())
}

/// A host call, answered on the request's connection.
async fn answer(ctx: &mut Ctx<'_>, grants: &Grants, call: HostCall) -> Result<Vec<u8>, String> {
    match call {
        HostCall::Rows {
            core,
            table,
            column,
            ids,
        } => {
            let rows = if core {
                core_rows(ctx.conn, &grants.core_reads, &table, &column, &ids).await?
            } else {
                own_rows(ctx.conn, &grants.tables, &table, &column, &ids).await?
            };
            serde_json::to_vec(&rows).map_err(|e| e.to_string())
        }
        HostCall::Setting(name) => {
            if !grants.settings.contains(&name) {
                return Err(format!("setting {name} is not in the manifest"));
            }
            let value = ctx.settings.get(&name).map_err(|e| e.to_string())?;
            serde_json::to_vec(value).map_err(|e| e.to_string())
        }
        HostCall::T { key, locale, args } => {
            let args: Vec<(&str, &str)> =
                args.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let text = ctx.i18n.and_then(|i| match &locale {
                Some(locale) => i.locale(locale).t_with(&key, &args),
                None => i.t_with(&key, &args),
            });
            Ok(text.unwrap_or(key).into_bytes())
        }
        HostCall::Users(ids) => {
            let config = ctx.config.ok_or("users is not available here")?;
            let urls = crate::url::Urls {
                config,
                settings: ctx.settings,
            };
            let ids: Vec<i32> = ids
                .iter()
                .filter_map(|id| i32::try_from(*id).ok())
                .collect();
            let rows: Vec<(i32, String, Option<String>, Option<i32>)> = sqlx::query_as(
                "SELECT id, username, name, uploaded_avatar_id FROM users WHERE id = ANY($1) ORDER BY id",
            )
            .bind(&ids)
            .fetch_all(&mut *ctx.conn)
            .await
            .map_err(|e| e.to_string())?;
            let mut users = Vec::new();
            for (id, username, name, avatar) in rows {
                let template = crate::avatar::avatar_template(&urls, id, &username, avatar, None)
                    .map_err(|e| e.to_string())?;
                users.push(json!({
                    "id": id,
                    "username": username,
                    "name": name,
                    "avatar_template": template,
                }));
            }
            serde_json::to_vec(&users).map_err(|e| e.to_string())
        }
        HostCall::CanSeeCategory(id) => {
            let visible = can_see_category(ctx, id).await.map_err(|e| e.to_string())?;
            serde_json::to_vec(&visible).map_err(|e| e.to_string())
        }
        // Plugin::run answers SQL itself, on the plugin's connection.
        HostCall::Sql { .. } | HostCall::PublishPostChange(_) => {
            Err("answered by the hook, not here".into())
        }
    }
}

/// `guardian.can_see_category?`: the category exists and is not read
/// restricted, or the viewer is an admin (unless secured categories are
/// suppressed from admins) or in a group that may see it.
async fn can_see_category(ctx: &mut Ctx<'_>, id: i64) -> Result<bool, crate::AppError> {
    let Ok(id) = i32::try_from(id) else {
        return Ok(false);
    };
    let restricted: Option<bool> =
        sqlx::query_scalar("SELECT read_restricted FROM categories WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *ctx.conn)
            .await?;
    let Some(restricted) = restricted else {
        return Ok(false);
    };
    if ctx.guardian.is_admin()
        && !ctx
            .settings
            .get("suppress_secured_categories_from_admin")?
            .truthy()
    {
        return Ok(true);
    }
    if !restricted {
        return Ok(true);
    }
    Ok(ctx
        .guardian
        .secure_category_ids(&mut *ctx.conn, ctx.settings)
        .await?
        .contains(&id))
}

/// Identifiers come from the manifest, never from the guest.
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

async fn select_rows(
    conn: &mut PgConnection,
    table: &str,
    columns: &[String],
    column: &str,
    order: &str,
    ids: &[i64],
) -> Result<Vec<Value>, String> {
    if !columns.iter().any(|c| c == column) {
        return Err(format!("{table}.{column} is not in the manifest"));
    }
    let select = columns
        .iter()
        .map(|c| quote(c))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT row_to_json(r) FROM (SELECT {select} FROM {} WHERE {}::bigint = ANY($1) ORDER BY {}{order}) r",
        quote(table),
        quote(column),
        quote(column),
    );
    sqlx::query_scalar(&sql)
        .bind(ids)
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| format!("{table}: {e}"))
}

/// `rows`: rows of a table the plugin owns whose column is one of the
/// ids, with the columns the manifest lists.
async fn own_rows(
    conn: &mut PgConnection,
    tables: &HashMap<String, Vec<String>>,
    table: &str,
    column: &str,
    ids: &[i64],
) -> Result<Vec<Value>, String> {
    let columns = tables
        .get(table)
        .ok_or_else(|| format!("rows: {table} is not this plugin's"))?;
    let order = if columns.iter().any(|c| c == "id") {
        ", \"id\""
    } else {
        ""
    };
    select_rows(conn, table, columns, column, order, ids).await
}

/// The core tables `core_rows` may read, each with its order within a
/// key. Posts are read as an ActiveRecord join reads them (deleted ones
/// included: discourse-solved's answerers join).
const CORE_TABLES: &[(&str, &str)] = &[
    ("posts", ", \"id\""),
    ("category_custom_fields", ", \"id\""),
    ("topic_tags", ", \"id\""),
    ("tags", ""),
    ("topic_allowed_groups", ", \"id\""),
    ("topics", ""),
];

/// `core_rows`: a core table's rows, limited to the columns the manifest
/// lists.
async fn core_rows(
    conn: &mut PgConnection,
    reads: &HashMap<String, Vec<String>>,
    table: &str,
    column: &str,
    ids: &[i64],
) -> Result<Vec<Value>, String> {
    let columns = reads
        .get(table)
        .ok_or_else(|| format!("core_rows: {table} is not in core_reads"))?;
    let (_, order) = CORE_TABLES
        .iter()
        .find(|(t, _)| *t == table)
        .ok_or_else(|| format!("core_rows: {table} can't be read"))?;
    select_rows(conn, table, columns, column, order, ids).await
}

/// `before_serialize`'s result: `{ id: { "posters": ... } }`.
fn rewritten_posters(value: Value) -> Result<HashMap<i32, PosterInputs>, String> {
    let Value::Object(topics) = value else {
        return Err("before_serialize must return an object keyed by topic id".into());
    };
    let mut out = HashMap::new();
    for (id, mut topic) in topics {
        let id: i32 = id.parse().map_err(|_| format!("{id}: not a topic id"))?;
        let Some(posters) = topic.get_mut("posters").map(Value::take) else {
            continue;
        };
        let inputs: PosterInputs =
            serde_json::from_value(posters).map_err(|e| format!("{id}.posters: {e}"))?;
        out.insert(id, inputs);
    }
    Ok(out)
}

/// `after_serialize`'s result, `{ id: { key: value } }`, each key
/// checked against the declaration and coerced to its type, in declared
/// order.
fn coerce(
    result: &Value,
    keys: &[(String, KeyType)],
) -> Result<HashMap<String, Vec<(String, Value)>>, String> {
    let objects = result
        .as_object()
        .ok_or("after_serialize must return an object keyed by id")?;
    let mut out = HashMap::new();
    for (id, extra) in objects {
        let extra = extra
            .as_object()
            .ok_or_else(|| format!("{id}: not an object of keys"))?;
        if let Some(unknown) = extra.keys().find(|k| !keys.iter().any(|(d, _)| d == *k)) {
            return Err(format!("{id}: undeclared key {unknown}"));
        }
        let mut values = Vec::new();
        for (key, kind) in keys {
            let Some(value) = extra.get(key) else {
                continue;
            };
            let ok = match (kind, value) {
                (_, Value::Null) => true,
                (KeyType::Bool, Value::Bool(_)) => true,
                (KeyType::Int, Value::Number(n)) => n.is_i64() || n.is_u64(),
                (KeyType::String, Value::String(_)) => true,
                (KeyType::Object, Value::Object(_) | Value::Array(_)) => true,
                _ => false,
            };
            if !ok {
                return Err(format!("{id}.{key}: not a {kind:?}"));
            }
            values.push((key.clone(), value.clone()));
        }
        out.insert(id.clone(), values);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_checked_in_declared_order() {
        let keys = vec![
            ("has_accepted_answer".to_string(), KeyType::Bool),
            ("count".to_string(), KeyType::Int),
        ];
        let result = json!({ "35": { "count": 2, "has_accepted_answer": true } });
        let merged = coerce(&result, &keys).unwrap();
        assert_eq!(
            merged["35"],
            vec![
                ("has_accepted_answer".to_string(), Value::Bool(true)),
                ("count".to_string(), Value::from(2)),
            ]
        );
        assert!(coerce(&json!({ "35": { "other": 1 } }), &keys).is_err());
        assert!(coerce(&json!({ "35": { "count": 1.5 } }), &keys).is_err());
        assert!(coerce(&json!([]), &keys).is_err());
    }

    #[test]
    fn rewritten_posters_are_checked() {
        let value = json!({ "35": { "posters": {
            "user_ids": [2, 3, null],
            "descriptions": { "2": ["Original Poster"] },
            "last_poster_stays": true,
        } } });
        let posters = rewritten_posters(value).unwrap();
        assert_eq!(posters[&35].user_ids, vec![Some(2), Some(3), None]);
        assert_eq!(
            posters[&35].descriptions[&2],
            vec!["Original Poster".to_string()]
        );
        assert!(
            rewritten_posters(json!({ "35": { "posters": { "user_ids": [], "extra": 1 } } }))
                .is_err()
        );
    }

    #[test]
    fn a_module_importing_an_ungranted_call_does_not_load() {
        let runtime = Runtime::new(Tuning::default()).unwrap();
        let wat = r#"(module
            (import "discourse" "rows" (func (param i32 i32) (result i64)))
            (memory (export "memory") 1)
            (func (export "dp_alloc") (param i32) (result i32) i32.const 0))"#;
        let manifest = "[plugin]\nname = \"x\"\napi = 1\n";
        let Err(e) = Plugin::new(&runtime, manifest, &wat::parse_str(wat).unwrap()) else {
            panic!("loaded");
        };
        assert!(e.0.contains("rows"), "{e}");
    }
}
