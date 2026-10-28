//! Third-party plugins in WebAssembly (src/plugin_wasm.rs,
//! docs/plugin-wasm.md): discourse-solved's topic list as a guest
//! (plugins/discourse-solved) answers /latest.json exactly as the bundled
//! port does on the recorded solved cases (parity/writes/solved/latest*,
//! which hold the bundled port to Rails), and a misbehaving plugin costs
//! the page its keys, never the page.

mod common;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;
use discourse_rs::plugin_wasm::{self, Plugin, Registry, Runtime, Tuning};
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

const BOOL: i32 = 5;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// A guest of plugins/ (its package) built for wasm32-unknown-unknown
/// (cargo serializes concurrent builds).
fn build_guest(package: &str) -> Vec<u8> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "-p",
            package,
        ])
        .current_dir(root().join("plugins"))
        .env_remove("RUSTFLAGS")
        .status()
        .expect("cargo runs");
    assert!(status.success(), "building {package}");
    let path: PathBuf = root().join(format!(
        "plugins/target/wasm32-unknown-unknown/release/{}.wasm",
        package.replace('-', "_")
    ));
    std::fs::read(path).unwrap()
}

/// plugins/discourse-solved, once per test process.
fn solved_wasm() -> &'static [u8] {
    static WASM: OnceLock<Vec<u8>> = OnceLock::new();
    WASM.get_or_init(|| build_guest("discourse-solved-wasm"))
}

fn solved_manifest() -> String {
    std::fs::read_to_string(root().join("plugins/discourse-solved/plugin.toml")).unwrap()
}

fn runtime() -> &'static Runtime {
    plugin_wasm::runtime().unwrap()
}

fn install(plugins: Vec<Plugin>) {
    let mut registry = Registry::default();
    for plugin in plugins {
        registry.add(plugin);
    }
    plugin_wasm::install(registry);
}

async fn get(pool: &PgPool, path: &str) -> (StatusCode, Value) {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])).await);
    let response = app
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A recorded solved case's settings and setup.
async fn setup_case(pool: &PgPool, name: &str) {
    let path = root().join(format!("parity/writes/solved/{name}.case.json"));
    let case: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    if let Some(settings) = case["settings"].as_object() {
        for (name, value) in settings {
            set_setting(pool, name, BOOL, value.as_str().unwrap()).await;
        }
    }
    for sql in case["setup"].as_array().unwrap() {
        sqlx::query(sql.as_str().unwrap())
            .execute(pool)
            .await
            .unwrap();
    }
}

fn topic(json: &Value, id: i64) -> &Value {
    json["topic_list"]["topics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap()
}

/// The case's /latest.json with the bundled port, then with the guest
/// replacing it: the same document.
async fn matches_the_bundled_port(case: &str) -> Value {
    let db = TestDb::new().await;
    setup_case(&db.pool, case).await;
    install(Vec::new());
    let (status, bundled) = get(&db.pool, "/latest.json").await;
    assert_eq!(status, StatusCode::OK);
    install(vec![
        Plugin::new(runtime(), &solved_manifest(), solved_wasm()).unwrap(),
    ]);
    let (status, wasm) = get(&db.pool, "/latest.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::to_string_pretty(&wasm).unwrap(),
        serde_json::to_string_pretty(&bundled).unwrap()
    );
    wasm
}

#[tokio::test]
async fn solved_latest_matches_the_bundled_port() {
    let json = matches_the_bundled_port("latest").await;
    let solved = topic(&json, 35);
    assert_eq!(solved["has_accepted_answer"], true);
    assert_eq!(topic(&json, 41)["has_accepted_answer"], false);
    // The answerer, second among the posters.
    let posters = solved["posters"].as_array().unwrap();
    assert!(
        posters.iter().any(|p| p["description"]
            .as_str()
            .unwrap()
            .contains("Accepted Answer")),
        "{posters:?}"
    );
}

#[tokio::test]
async fn solved_latest_answer_by_author_matches_the_bundled_port() {
    matches_the_bundled_port("latest_answer_by_author").await;
}

#[tokio::test]
async fn solved_latest_deleted_answer_matches_the_bundled_port() {
    matches_the_bundled_port("latest_deleted_answer").await;
}

#[tokio::test]
async fn solved_latest_empty_box_matches_the_bundled_port() {
    let json = matches_the_bundled_port("latest_empty_box").await;
    assert!(
        json["topic_list"]["topics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t.get("can_have_answer").is_some())
    );
}

/// The keys and posters come from the guest, not the bundled port it
/// replaces: without its before_serialize the answerer is not among the
/// posters, and a key it no longer declares costs it all its keys.
#[tokio::test]
async fn the_guest_replaces_the_bundled_port() {
    let db = TestDb::new().await;
    setup_case(&db.pool, "latest_empty_box").await;
    let manifest = solved_manifest().replace(
        "phases = [\"after_load\", \"before_serialize\", \"after_serialize\"]",
        "phases = [\"after_load\", \"after_serialize\"]",
    );
    install(vec![
        Plugin::new(runtime(), &manifest, solved_wasm()).unwrap(),
    ]);
    let (_, json) = get(&db.pool, "/latest.json").await;
    let solved = topic(&json, 35);
    assert_eq!(solved["has_accepted_answer"], true);
    assert!(!solved["posters"].to_string().contains("Accepted Answer"));

    let manifest = solved_manifest().replace(", [\"can_have_answer\", \"bool\"]", "");
    install(vec![
        Plugin::new(runtime(), &manifest, solved_wasm()).unwrap(),
    ]);
    let (status, json) = get(&db.pool, "/latest.json").await;
    assert_eq!(status, StatusCode::OK);
    assert!(topic(&json, 35).get("has_accepted_answer").is_none());
}

#[tokio::test]
async fn solved_disabled_adds_nothing() {
    let db = TestDb::new().await;
    setup_case(&db.pool, "latest").await;
    set_setting(&db.pool, "solved_enabled", BOOL, "f").await;
    install(vec![
        Plugin::new(runtime(), &solved_manifest(), solved_wasm()).unwrap(),
    ]);
    let (status, json) = get(&db.pool, "/latest.json").await;
    assert_eq!(status, StatusCode::OK);
    assert!(topic(&json, 35).get("has_accepted_answer").is_none());
}

/// A guest written by hand: `body` is the after_serialize function's,
/// returning a packed pointer; `data` is placed at 1024.
fn wat_plugin(manifest_extra: &str, imports: &str, body: &str, data: &str) -> Plugin {
    wat_plugin_on(runtime(), manifest_extra, imports, body, data)
}

fn wat_plugin_on(
    runtime: &Runtime,
    manifest_extra: &str,
    imports: &str,
    body: &str,
    data: &str,
) -> Plugin {
    let wat = format!(
        r#"(module
            {imports}
            (memory (export "memory") 1)
            (global $next (mut i32) (i32.const 4096))
            (func (export "dp_alloc") (param $len i32) (result i32)
                (local $at i32)
                global.get $next
                local.set $at
                global.get $next
                local.get $len
                i32.add
                global.set $next
                local.get $at)
            (func (export "topic_list.after_serialize") (param i32 i32) (result i64)
                {body})
            (data (i32.const 1024) "{data}"))"#
    );
    let manifest = format!(
        "[plugin]\nname = \"misbehaving\"\napi = 1\n{manifest_extra}\n[topic_list]\nphases = [\"after_serialize\"]\nkeys = [[\"misbehaved\", \"bool\"]]\n"
    );
    Plugin::new(runtime, &manifest, &wat::parse_str(wat).unwrap()).unwrap()
}

/// The packed pointer to `len` bytes at 1024.
fn at_1024(len: usize) -> String {
    format!("i64.const {}", (1024i64 << 32) | len as i64)
}

async fn page_survives(plugin: Plugin) -> Duration {
    let db = TestDb::new().await;
    install(vec![plugin]);
    let started = Instant::now();
    let (status, json) = get(&db.pool, "/latest.json").await;
    let took = started.elapsed();
    assert_eq!(status, StatusCode::OK);
    for t in json["topic_list"]["topics"].as_array().unwrap() {
        assert!(t.get("misbehaved").is_none(), "{t}");
    }
    took
}

#[tokio::test]
async fn a_well_behaved_hand_written_guest_adds_its_key() {
    let ok = r#"{\"ok\":{\"35\":{\"misbehaved\":false}}}"#;
    let len = ok.len() - ok.matches('\\').count();
    let plugin = wat_plugin("", "", &at_1024(len), ok);
    let db = TestDb::new().await;
    install(vec![plugin]);
    let (_, json) = get(&db.pool, "/latest.json").await;
    assert_eq!(topic(&json, 35)["misbehaved"], false);
    assert!(topic(&json, 41).get("misbehaved").is_none());
}

#[tokio::test]
async fn an_endless_loop_is_cut_at_its_time_limit() {
    let plugin = wat_plugin(
        "[limits]\ntime_ms = 20",
        "",
        "(loop $forever (br $forever)) unreachable",
        "",
    );
    // Each topic's keys run the phase once for the page.
    assert!(page_survives(plugin).await < Duration::from_secs(5));
}

#[tokio::test]
async fn memory_past_the_cap_is_refused() {
    // memory.grow by 100 pages (6.4 MB) over a 1 MiB cap returns -1.
    let plugin = wat_plugin(
        "[limits]\nmemory_mib = 1",
        "",
        "(if (i32.eq (memory.grow (i32.const 100)) (i32.const -1)) (then unreachable)) i64.const 0",
        "",
    );
    page_survives(plugin).await;
}

#[tokio::test]
async fn an_undeclared_key_drops_the_plugins_keys() {
    let out = r#"{\"ok\":{\"35\":{\"misbehaved\":true,\"other\":1}}}"#;
    let len = out.len() - out.matches('\\').count();
    page_survives(wat_plugin("", "", &at_1024(len), out)).await;
}

#[tokio::test]
async fn host_calls_past_the_cap_fail_the_phase() {
    // t("k") in a loop, forever, under a cap of 3 calls.
    let plugin = wat_plugin(
        "[limits]\nhost_calls = 3",
        r#"(import "discourse" "t" (func $t (param i32 i32) (result i64)))"#,
        "(loop $again (drop (call $t (i32.const 1024) (i32.const 1))) (br $again)) unreachable",
        "k",
    );
    page_survives(plugin).await;
}

#[tokio::test]
async fn a_phase_may_not_make_calls_its_phase_does_not_allow() {
    // before_serialize may not read rows.
    let wat = r#"(module
        (import "discourse" "rows" (func $rows (param i32 i32) (result i64)))
        (memory (export "memory") 1)
        (func (export "dp_alloc") (param i32) (result i32) i32.const 4096)
        (func (export "topic_list.before_serialize") (param i32 i32) (result i64)
            (call $rows (i32.const 1024) (i32.const 2)))
        (data (i32.const 1024) "{}"))"#;
    let manifest = "[plugin]\nname = \"reader\"\napi = 1\n[tables]\nx = [\"id\"]\n[topic_list]\nphases = [\"before_serialize\"]\n";
    let plugin = Plugin::new(runtime(), manifest, &wat::parse_str(wat).unwrap()).unwrap();
    let db = TestDb::new().await;
    install(Vec::new());
    let (_, before) = get(&db.pool, "/latest.json").await;
    install(vec![plugin]);
    let (status, after) = get(&db.pool, "/latest.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after, before);
}

#[test]
fn a_plugin_for_another_api_version_does_not_load() {
    let manifest = solved_manifest().replace("api = 1", "api = 2");
    assert!(Plugin::new(runtime(), &manifest, solved_wasm()).is_err());
}

/// The numbers in docs/plugin-wasm.md. `PLUGIN_WASM_BOA` names the Boa
/// guest's module (and `PLUGIN_WASM_BOA_WIZER` its pre-initialized one)
/// to measure it too. Run in release:
/// `cargo nextest run --release --test plugin_wasm --run-ignored only --no-capture`.
#[tokio::test]
#[ignore]
async fn measure() {
    use wasmtime::{
        Config, Engine, InstanceAllocationStrategy, Linker, Module, PoolingAllocationConfig, Store,
    };

    fn engine(pooling: bool) -> Engine {
        let mut config = Config::new();
        config.epoch_interruption(true);
        if pooling {
            let mut pool = PoolingAllocationConfig::default();
            pool.total_core_instances(16);
            pool.total_memories(16);
            pool.max_memory_size(64 << 20);
            config.allocation_strategy(InstanceAllocationStrategy::Pooling(pool));
        }
        Engine::new(&config).unwrap()
    }
    fn per(total: Duration, n: u32) -> String {
        format!("{:.1} us", total.as_secs_f64() * 1e6 / f64::from(n))
    }
    async fn instantiation(name: &str, wasm: &[u8]) {
        let started = Instant::now();
        let module = Module::new(&engine(false), wasm).unwrap();
        println!(
            "{name}: {} KiB, compile {:.1} ms",
            wasm.len() / 1024,
            started.elapsed().as_secs_f64() * 1e3
        );
        let bytes = module.serialize().unwrap();
        for pooling in [false, true] {
            let engine = engine(pooling);
            let started = Instant::now();
            // SAFETY: bytes are this process' own serialization.
            let module = unsafe { Module::deserialize(&engine, &bytes) }.unwrap();
            let load = started.elapsed();
            let mut linker: Linker<()> = Linker::new(&engine);
            for import in module.imports() {
                if let wasmtime::ExternType::Func(ty) = import.ty() {
                    linker
                        .func_new(import.module(), import.name(), ty.clone(), |_, _, _| Ok(()))
                        .unwrap();
                }
            }
            let pre = linker.instantiate_pre(&module).unwrap();
            let n = 2000;
            let started = Instant::now();
            let mut pages = 0;
            for _ in 0..n {
                let mut store = Store::new(&engine, ());
                store.set_epoch_deadline(1000);
                let instance = pre.instantiate_async(&mut store).await.unwrap();
                pages = instance
                    .get_memory(&mut store, "memory")
                    .unwrap()
                    .size(&store);
            }
            println!(
                "  pooling {pooling}: deserialize {:.2} ms, instantiate {}, memory {} KiB",
                load.as_secs_f64() * 1e3,
                per(started.elapsed(), n),
                pages * 64
            );
        }
    }

    instantiation("discourse-solved (Rust)", solved_wasm()).await;
    for var in ["PLUGIN_WASM_BOA", "PLUGIN_WASM_BOA_WIZER"] {
        if let Ok(path) = std::env::var(var) {
            instantiation(var, &std::fs::read(path).unwrap()).await;
        }
    }

    // Phase calls through the host, on a real connection.
    let db = TestDb::new().await;
    setup_case(&db.pool, "latest").await;
    let mut conn = db.pool.acquire().await.unwrap();
    let state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let settings = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await
    .unwrap();
    let guardian = discourse_rs::guardian::Guardian::anonymous();
    let facts: Vec<Value> = [5, 35, 41, 38, 37]
        .iter()
        .map(|id| serde_json::json!({"id": id, "category_id": 4, "archetype": "regular", "closed": false, "archived": false, "last_post_user_id": 2, "user_id": 2}))
        .collect();
    let mut ctx = plugin_wasm::Ctx {
        conn: &mut conn,
        settings: &settings,
        guardian: &guardian,
        i18n: Some(&state.i18n),
        pool: None,
    };
    let posters = serde_json::json!({"35": {"posters": {"user_ids": [2, 2], "descriptions": {"2": ["Original Poster", "Most Recent Poster"]}, "last_poster_stays": true}, "last_post_user_id": 2, "data": {"answerers": [3], "answered": true}}});
    let tunings = [
        ("pooling, keep 1 MiB, pagemap scan", Tuning::default()),
        (
            "pooling, keep 1 MiB, memset",
            Tuning {
                pagemap_scan: false,
                ..Tuning::default()
            },
        ),
        (
            "pooling, keep 64 KiB, memset",
            Tuning {
                keep_resident: 64 << 10,
                pagemap_scan: false,
                ..Tuning::default()
            },
        ),
        (
            "on demand",
            Tuning {
                pooling: false,
                ..Tuning::default()
            },
        ),
    ];
    for (tuning_name, tuning) in tunings {
        println!("{tuning_name}:");
        let runtime = Runtime::new(tuning).unwrap();
        // The fixed cost of a phase call: a guest that returns at once;
        // then the same with one host call (t).
        let noop = wat_plugin_on(&runtime, "", "", &at_1024(9), r#"{\"ok\":{}}"#);
        let one_call = wat_plugin_on(
            &runtime,
            "",
            r#"(import "discourse" "t" (func $t (param i32 i32) (result i64)))"#,
            &format!(
                "(drop (call $t (i32.const 1033) (i32.const 1))) {}",
                at_1024(9)
            ),
            r#"{\"ok\":{}}k"#,
        );
        for (name, plugin) in [("no-op phase", &noop), ("one t host call", &one_call)] {
            let n = 2000;
            let started = Instant::now();
            for _ in 0..n {
                plugin
                    .run(
                        &mut ctx,
                        "topic_list",
                        plugin_wasm::Phase::AfterSerialize,
                        Value::from(facts.clone()),
                    )
                    .await
                    .unwrap();
            }
            println!("  {name}: {} per call", per(started.elapsed(), n));
        }
        let plugin = Plugin::new(&runtime, &solved_manifest(), solved_wasm()).unwrap();
        let cases: [(&str, plugin_wasm::Phase, Value); 3] = [
            (
                "solved before_serialize (one t call, no db)",
                plugin_wasm::Phase::BeforeSerialize,
                posters.clone(),
            ),
            (
                "solved after_load (3 queries)",
                plugin_wasm::Phase::AfterLoad,
                Value::from(facts.clone()),
            ),
            (
                "solved after_serialize (1 query)",
                plugin_wasm::Phase::AfterSerialize,
                Value::from(facts.clone()),
            ),
        ];
        for (name, phase, payload) in cases {
            let n = 500;
            let started = Instant::now();
            for _ in 0..n {
                plugin
                    .run(&mut ctx, "topic_list", phase, payload.clone())
                    .await
                    .unwrap();
            }
            println!("  {name}: {} per call", per(started.elapsed(), n));
        }
    }
    // The same queries without the plugin: what the host calls cost on top.
    let n = 500;
    let started = Instant::now();
    for _ in 0..n {
        for sql in [
            "SELECT row_to_json(r) FROM (SELECT id, topic_id FROM discourse_solved_solved_topics WHERE topic_id::bigint = ANY($1) ORDER BY topic_id, id) r",
            "SELECT row_to_json(r) FROM (SELECT id, solved_topic_id, answer_post_id FROM discourse_solved_topic_answers WHERE solved_topic_id::bigint = ANY($1) ORDER BY solved_topic_id, id) r",
            "SELECT row_to_json(r) FROM (SELECT id, user_id FROM posts WHERE id::bigint = ANY($1) ORDER BY id) r",
        ] {
            let _: Vec<Value> = sqlx::query_scalar(sql)
                .bind(vec![5i64, 35, 41, 38, 37])
                .fetch_all(&mut *ctx.conn)
                .await
                .unwrap();
        }
    }
    println!(
        "after_load's 3 queries alone: {} per call",
        per(started.elapsed(), n)
    );
    if let Ok(path) = std::env::var("PLUGIN_WASM_BOA_WIZER") {
        let manifest =
            std::fs::read_to_string(root().join("plugins/js-solved/plugin.toml")).unwrap();
        for (name, path) in [
            ("plain", std::env::var("PLUGIN_WASM_BOA").unwrap()),
            ("wizer", path),
        ] {
            let js = Plugin::new(runtime(), &manifest, &std::fs::read(path).unwrap()).unwrap();
            let n = 50;
            let started = Instant::now();
            let mut out = Value::Null;
            for _ in 0..n {
                out = js
                    .run(
                        &mut ctx,
                        "topic_list",
                        plugin_wasm::Phase::AfterSerialize,
                        Value::from(facts.clone()),
                    )
                    .await
                    .unwrap();
            }
            println!(
                "boa ({name}) after_serialize (2 queries): {} per call: {out}",
                per(started.elapsed(), n)
            );
        }
    }
}

/// plugins/js-solved: Boa inside the guest running plugin.js.
#[tokio::test]
async fn a_javascript_plugin_runs_in_boa_inside_the_guest() {
    let db = TestDb::new().await;
    setup_case(&db.pool, "latest").await;
    let manifest = std::fs::read_to_string(root().join("plugins/js-solved/plugin.toml")).unwrap();
    let wasm = build_guest("js-solved-wasm");
    install(vec![Plugin::new(runtime(), &manifest, &wasm).unwrap()]);
    let (status, json) = get(&db.pool, "/latest.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(topic(&json, 35)["has_accepted_answer"], true);
    assert_eq!(topic(&json, 41)["has_accepted_answer"], false);
}
