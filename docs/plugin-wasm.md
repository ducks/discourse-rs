# Third-party plugins as WebAssembly (spike)

Branch `spike/plugin-wasm`, 2026-10-21. PLUGINS.md decision 6 leaves the
third-party runtime open; the Luau spike (`spike/plugin-runtime`, mlua)
and the QuickJS theme spike (`spike/theme-extensions`) both put C or C++
on the host, which is ruled out. This spike runs third-party plugins as
WebAssembly in wasmtime (Rust and Cranelift, with one 30-line C shim:
"C and C++ on the host" below), on the same lifecycle phases the
bundled plugins use, and ports the Luau spike's plugin
(discourse-solved on the topic list) to it.

What is here:

- `src/plugin_wasm.rs`: the host (registry, manifest, ABI, host calls,
  limits) and the topic list's three phases;
- `src/topic_list.rs`: the phases wired into the topic list next to the
  bundled plugins', on every route that serializes a topic list;
- `plugins/sdk`: the guest side of the ABI for Rust (`discourse-plugin`);
- `plugins/discourse-solved`: the Luau spike's solved port, in Rust,
  plus the bundled port's `can_have_answer`;
- `plugins/js-solved`: Boa compiled into a guest, running a JavaScript
  `plugin.js`;
- `tests/plugin_wasm.rs`: the behaviour tests and the measurements
  (`measure`, ignored by default).

On `spike/wasm-host` (2026-10-27) a guest is also a full
`plugin_api::Plugin`, the trait compiled-in plugins implement: see
"Every plugin_api hook" below.

## ABI: a core module, JSON over linear memory

The choice was between WIT and the component model (`wasmtime::component`,
cargo-component or wit-bindgen guests) and a plain core module with JSON
(or msgpack) over linear memory. This spike takes the core module with
JSON:

- The payloads are already JSON documents defined per phase (PLUGINS.md,
  the Luau spike, the theme spike's "one host contract": a named point,
  JSON in, validated JSON out, allowed host calls only). The host
  validates results against the manifest (declared keys and types,
  `deny_unknown_fields` on the poster inputs) whatever the transport, so
  WIT's types would be checked twice.
- A core module on `wasm32-unknown-unknown` imports exactly the host
  functions it calls, nothing else: the linker refuses a module importing
  anything the manifest did not grant, and `wasm-tools print` shows a
  plugin's whole reach in a few lines. On `wasm32-wasip2` (the component
  target) std is built on WASI, so modules generally import WASI
  interfaces the host would have to define or stub (not tried here).
- Any toolchain that emits a core module with a memory and two exports
  can write a plugin: Rust, Zig, C, TinyGo, AssemblyScript, and an
  interpreter compiled in (Boa below). No cargo-component, no
  wit-bindgen, no component adapter in the guest build.
- The host stays small: no `component-model` feature in wasmtime (its
  compile time and the async component machinery), and the glue is
  `src/plugin_wasm.rs`'s ~150 lines of pointer handling plus the
  140-line SDK.

What it costs: the pointer and length glue is hand written, and a plugin
author in a language without the SDK writes it again (it is three
functions). Strings and records cross as JSON text, parsed on both sides
(measured below: small next to the queries). If WIT is wanted later, each
phase is `func(input: string) -> result<string, string>` in a world with
the host calls as imports, and the host can load both kinds side by side.
msgpack was not tried: the payloads are small and JSON keeps them
readable in logs and tests.

### Guest exports

| export | signature | |
|---|---|---|
| `memory` | memory | the guest's linear memory |
| `dp_alloc` | `(len: i32) -> i32` | memory for `len` bytes the host writes (input, host call answers) |
| `<pipeline>.<phase>` | `(ptr: i32, len: i32) -> i64` | one per declared phase, e.g. `topic_list.after_load` |

A phase's input is `{"payload": ..., "guardian": {"user_id", "staff",
"admin"}}`; it returns `ptr << 32 | len` of `{"ok": result}` or
`{"error": "message"}`. A trap (a Rust panic is `unreachable` with
`panic = "abort"`) is an error too.

### Host calls

All in the import module `discourse`, each `(ptr: i32, len: i32) -> i64`
with the answer written into memory from `dp_alloc`:

| call | request | answer | granted by |
|---|---|---|---|
| `rows` | `{"table", "column", "ids"}` | rows of a table the plugin owns, the manifest's columns, ordered by `column` then `id` | `[tables]` |
| `core_rows` | the same | rows of a core table on a fixed list, the manifest's columns | `[core_reads]` |
| `setting` | a setting's name | its value as JSON | `[settings] read` |
| `t` | a translation key | the server translation (the key when missing) | always |
| `publish_post_change` | a post id | `true`; the post is published as `:acted` once the hook commits (routes and jobs) | always |

A call the manifest does not grant is not defined, so a module importing
it does not load. Per phase: `before_serialize` may call `t` and
`setting` only (it works on what `after_load` kept, without the
database); `after_load` and `after_serialize` may call all four. All are
reads; the write pipelines' phases and the API calls of decision 5 are
not built.

### Synchronous host calls on an async host

A host call is synchronous for the guest: it calls `rows` and gets rows
back. On the host it is an async host function (`func_wrap_async`):
wasmtime runs the guest on a fiber, and when the guest calls `rows` the
fiber suspends and the call's future sends the request over a channel to
a loop running beside the guest on the same task (`tokio::join!`), which
runs the query on the request's own `&mut PgConnection` and answers. So,
as with the Luau spike's coroutine yield:

- the query runs inside the request's connection (and transaction), never
  a second connection from the pool (the Luau spike's first version did
  that and stalled at ~3 req/s under 16 concurrent requests);
- the store holds no borrow of the connection (a `Store<T>` needs
  `T: 'static`), only the channel's sender;
- no blocking thread, no `block_in_place`: it works on the current-thread
  runtime the tests use as well as the server's.

No coroutine or promise is needed in the guest: Rust, and Boa's native
functions, call the host the way they call any function.

## The manifest

```toml
[plugin]
name = "discourse-solved-wasm"
api = 1                          # the ABI version; another one does not load
enabled_setting = "solved_enabled"
replaces = "discourse-solved"    # the bundled plugin whose phases it takes over

[limits]
memory_mib = 16                  # linear memory, at most 64
time_ms = 50                     # wall clock per phase call, host calls included
host_calls = 16                  # per phase call

[settings]
read = ["allow_solved_on_all_topics", "enable_solved_tags", "allow_solved_in_groups"]

[tables]                         # rows(): its own tables and columns
discourse_solved_solved_topics = ["id", "topic_id"]
discourse_solved_topic_answers = ["id", "solved_topic_id", "answer_post_id"]

[core_reads]                     # core_rows(): core columns
posts = ["id", "user_id"]
category_custom_fields = ["id", "category_id", "name", "value"]

[topic_list]
phases = ["after_load", "before_serialize", "after_serialize"]
keys = [["has_accepted_answer", "bool"], ["can_have_answer", "bool"]]
```

`replaces` is the spike's answer to running a third-party port of a
bundled plugin: while the guest is enabled, the bundled plugin's topic
list phases are skipped and the guest's keys go where the bundled
plugin's went. A plugin without it adds its keys after every bundled
plugin's.

## Limits and the security model

What a plugin can reach is what its module imports and its manifest
grants, nothing more:

- **No WASI.** No files, network, clock, environment or randomness. The
  guest's only way out is the four host calls above, each checked against
  the manifest and the phase.
- **Data.** `rows` reads only tables and columns the manifest lists as
  the plugin's; `core_rows` only the core tables on a fixed list
  (`CORE_TABLES`: posts, category_custom_fields, topic_tags, tags,
  topic_allowed_groups) and the columns the manifest lists. Identifiers
  come from the manifest, never the guest; ids are bound as `bigint[]`.
  Nothing is written.
- **Memory.** Each phase call gets a fresh instance from the pooling
  allocator, with its own linear memory capped by `memory_mib`
  (`StoreLimits`; a `memory.grow` past it returns -1) inside the engine's
  64 MiB slots, guard pages around them, and a 512 KiB wasm stack. Nothing
  survives the call: no state crosses requests or users.
- **Time.** Epoch interruption: a thread advances the engine's epoch every
  millisecond and a store's deadline is `time_ms` ticks, so an endless
  loop traps within about `time_ms`. The budget is wall clock and counts
  host calls, but a host call itself is not interrupted: a slow query
  runs to the end (see open questions).
- **Calls.** At most `host_calls` per phase call.
- **Output.** `after_serialize` may add only the keys it declares, of
  their declared types (all of a plugin's keys are dropped otherwise);
  `before_serialize` returns poster inputs that must parse as core's
  `PosterInputs`; `after_load` returns user ids and per-topic data.
- **Failure policy** (PLUGINS.md): a phase that traps, runs out of time,
  memory or calls, or returns what it may not, is skipped and logged with
  the plugin's name; the page is served without its keys. Tested for each
  case in `tests/plugin_wasm.rs`.

Not covered: Spectre-class side channels beyond wasmtime's own
mitigations; denial of service through queries that are expensive but
allowed (a `rows` call with many ids); the `guardian` the phase receives
(user id, staff, admin) is information the plugin sees.

## The port of the Luau plugin

`plugins/discourse-solved/src/lib.rs` is the Luau spike's
`plugin.luau` (109 lines) in Rust (about 290 lines, half of it the
`can_have_answer` the Luau spike did not do):

- `after_load`: the page's solved topics and answers (`rows` twice), the
  answer posts' authors (`core_rows` on posts, deleted ones included as
  Rails' join keeps them), handed back as users to load and per-topic
  data;
- `before_serialize`: the answerers inserted second among the posters
  with "Accepted Answer", and an answering last poster keeps their place;
- `after_serialize`: `has_accepted_answer`, and `can_have_answer` where
  the category has `empty_box_on_unsolved`, reading the settings, the
  category custom fields, the topic's tags and a private message's
  allowed groups as the bundled port does.

How it went: the port was mechanical. Two of the Luau spike's
workarounds disappeared: integers are integers (no doubles to turn back
into ids), and an empty object is `{}`, not an empty table that reads as
`[]`. JSON `null` is `Value::Null`, not a truthy sentinel. The one
contract change: `before_serialize` gets `last_post_user_id` in its
payload instead of reading it out of `user_ids[2]`.

What the tests show (`tests/plugin_wasm.rs`): on the four recorded
solved cases (`parity/writes/solved/latest`, `latest_answer_by_author`,
`latest_deleted_answer`, `latest_empty_box`), `/latest.json` with the
guest replacing the bundled port is the same document as with the bundled
port, which the write harness holds to Rails on those cases. So the
guest matches Rails where the Luau spike did (`has_accepted_answer`, the
posters) and on `can_have_answer`. A further test removes the guest's
`before_serialize` and one declared key to show the values come from the
guest and not from the bundled port underneath.

## Measurements

Release builds on the dev box (16 cores, shared: load average 9 to 12
from other work during every run, so absolute numbers move between runs
by 10 to 20%; the comparisons were run back to back). Microbenchmarks are
`cargo nextest run --release --test plugin_wasm --run-ignored only
--no-capture measure`; the route numbers are a release server on the
test fixtures plus the recorded `solved/latest` setup (5 topics on
`/latest.json`, one solved), anonymous, with the anonymous cache off,
`oha` for 10 s after a 3 s warmup.

### Compile, load, instantiate

| | discourse-solved (Rust) | Boa guest |
|---|---|---|
| module size | 131 KiB | 3.7 MiB (3.9 MiB pre-initialized) |
| compile with Cranelift (`Module::new`) | 27 to 37 ms | 430 to 530 ms |
| load precompiled (`Module::deserialize`) | 0.3 ms | 5 to 8 ms |
| instantiate, on-demand allocator | 14 to 22 us | 15 to 19 us |
| instantiate, pooling allocator | 3 to 6 us | 3.5 to 5 us |
| linear memory of an instance | 1088 KiB (1 MiB of it the shadow stack) | 1728 KiB (1984 KiB pre-initialized) |

Instantiation is from an `InstancePre` (imports resolved once); pooling
makes it a few microseconds. Pre-initialization (wizer) is about the
guest's own start-up, not instantiation: it matters for Boa (below) and
not for the Rust guest, which has nothing to initialize.

### Per phase call

What a call costs through `Plugin::run` (fresh instance, input written,
phase run, host calls answered on a real connection, output parsed), by
allocator tuning:

| | pooling, keep 1 MiB, pagemap scan (default) | pooling, keep 1 MiB, memset | pooling, keep 64 KiB | on demand |
|---|---|---|---|---|
| no-op phase (the fixed cost) | 11.5 us | 10.9 us | 10.9 us | 45 us |
| no-op plus one `t` host call | 15.5 us | 15.1 us | 15.3 us | 58 us |
| solved `before_serialize` (one `t`, no db) | 30 us | 58 us | 42 us | 79 us |
| solved `after_load` (3 `rows`/`core_rows`) | 168 us | 210 us | 207 us | 242 us |
| solved `after_serialize` (1 `rows`) | 79 us | 115 us | 106 us | 135 us |

The same three queries as `after_load` run directly on the connection
take 72 us, so `after_load`'s overhead is about 95 us: the fixed cost,
three host calls at about 4 us each, and the guest's JSON work and
first-touch of its memory. What decides the guest-side cost is how a
pooled memory slot is reset: `memset` of everything kept resident, or
`madvise` and fault the pages in again, both cost tens of microseconds;
Linux 6.7's PAGEMAP_SCAN resets only the pages the guest dirtied, which
is why it is the default (`Tuning`, falling back where unsupported).

### On the route

`/latest.json`, phases timed inside the server at one request at a time
(medians of 500): `after_load` 318 us, `before_serialize` 65 us,
`after_serialize` 123 us, so about 0.5 ms a list for the Rust guest
(including its 4 queries; the bundled port's own solved queries are
1 joined query plus the category fields), and 0.37 ms for the JavaScript
guest's one phase.

| | `/latest.json` req/s (16 conc.) | p50 | `/c/general/4.json` req/s | p50 | one at a time p50 | RSS after |
|---|---|---|---|---|---|---|
| solved disabled | 2600 | 6.0 ms | 2150 | 7.3 ms | 2.2 ms | 96 MB |
| bundled solved (Rust in the binary) | 2460 | 6.4 ms | 2080 | 7.5 ms | 2.2 ms | 95 MB |
| solved as a Rust WebAssembly guest | 1860 | 8.5 ms | 1640 | 9.6 ms | 2.9 ms | 131 MB |
| `has_accepted_answer` as a JavaScript guest | 2025 | 7.8 ms | 1680 | 9.4 ms | 2.9 ms | 288 MB |

(Two rounds each, averaged; the rounds agreed within 8%.)

Compared with the Luau spike: it measured /latest.json 3165 to 2928
req/s (-7.5%) and /c/general/4.json 2171 to 1925 (-11%) against no
plugin, about 0.4 ms a list, on another day's load. Here the Rust
guest is -28% and -24% against solved disabled, -24% and -21% against
the bundled port, about 0.5 ms a list. So per list the WebAssembly
runtime is in Luau's range, and under load it costs more. Where it
goes: a fresh instance per phase call (three a list) and three keyed
lookups where the bundled port makes one joined query; an instance per
request and a batched lookup would both cut it (open questions). The
memory cost is resident pool slots (up to 1 MiB kept per slot used) and
compiled code: +35 MB RSS for the Rust guest under 16-way load, +190 MB
for the Boa guest.

## JavaScript in the guest: Boa

Tried, and it works with no change to the host: `plugins/js-solved` is
Boa 0.22 (pure Rust; default features off, so no Intl, Temporal or
float16) compiled into a `wasm32-unknown-unknown` guest with a
`plugin.js` embedded at build time. The script sees `rows(table, column,
ids)` and `t(key)` as global functions; they are Boa native functions
calling the SDK, so they are synchronous, with no promise or `await`
(the theme spike's QuickJS plan needed one). The phase calls
`plugin.topic_list.after_serialize(topics)` with the payload converted
by `JsValue::from_json` and converts the result back with `to_json`; the
host validates it like any other guest's. The test runs
`has_accepted_answer` from `plugin.js` on `/latest.json`.

It took about an hour. Two things needed fixing, both because the guest
has no platform: Boa's default clock calls `Instant::now()`, which traps
on `wasm32-unknown-unknown`, so the context gets a fixed clock (`Date`
reads the epoch), and `rand` (for `Math.random`) needs getrandom's
`unsupported` backend to build (`Math.random` then fails).

| | Boa guest | wizer pre-initialized | Rust guest (solved) |
|---|---|---|---|
| module | 3.7 MiB | 3.9 MiB | 131 KiB |
| compile (Cranelift) | 430 to 530 ms | 430 to 450 ms | 27 to 37 ms |
| load precompiled | 5 to 8 ms | 5 to 8 ms | 0.3 ms |
| instantiate, pooled | 3.5 us | 4 us | 3 to 6 us |
| linear memory at start | 1728 KiB | 1984 KiB | 1088 KiB |
| `after_serialize`, 2 queries (1 for Rust), per call | 740 to 1000 us | 340 to 440 us | 79 us |

Because every phase call gets a fresh instance, the plain module builds
Boa's realm and parses the script on every call; wizer runs
`wizer.initialize` (realm and script) once at build time and snapshots
the memory, which halves the call. What remains is Boa's interpreter
and the JSON conversions: on the route the phase took 0.37 ms a list
(above).

For production the script should not be compiled into the module: one
`boa-runtime.wasm` for every JavaScript plugin, with the host
pre-initializing it per plugin at install time (the `wasmtime-wizer`
crate is wasmtime 50's own, usable as a library with the host's linker),
so a plugin author ships `plugin.js` and `plugin.toml` and never a
toolchain. Not built here.

## The guest toolchain under nix

- `rust-toolchain.toml` already carries the `wasm32-unknown-unknown`
  target (the composer preview builds the markdown crate with it), so the
  guests build with the same pinned rustc from rust-overlay; nothing
  global.
- `shell.nix` adds `wasm-tools` (inspecting modules: `wasm-tools print
  plugin.wasm | grep import`) and `wizer` (pre-initialization), both from
  nixpkgs.
- `plugins/` is its own cargo workspace (excluded from the server's),
  with its own `Cargo.lock` and a release profile for size (`opt-level =
  "s"`, LTO, `panic = "abort"`, stripped). `plugins/.cargo/config.toml`
  sets getrandom's `unsupported` backend: Boa pulls in `rand`, and the
  guest has no randomness to give it.
- `make wasm-plugins` builds every guest and leaves `plugin.wasm` next to
  each `plugin.toml` (pre-initializing the JavaScript one with wizer).
  The tests build the guests they need themselves.
- No C or C++ in the guests: the guest lockfile has no `cc` or `-sys`
  crate.

## C and C++ on the host

wasmtime is added with `default-features = false` and `cranelift`,
`runtime`, `std`, `pooling-allocator`, `async`, `parallel-compilation`:
no WASI, no component model, no profiling (`ittapi-sys`), no cache
(`zstd-sys`). What C remains:

- wasmtime's `runtime` feature compiles `src/runtime/vm/helpers.c` with
  `cc` on Unix: about 30 lines whose job is to look up libunwind's
  `__unw_add_dynamic_fde` as a weak symbol (Rust has no stable weak
  linkage). No logic, but it needs a C compiler at build time and it is C
  in the binary. It cannot be turned off without turning off the runtime;
  wasmtime's own build script carries a TODO to drop it on Unix. Removing
  it would mean a patched wasmtime (`[patch.crates-io]`, as the tree
  already does for grass) or an upstream change.
- `wasmtime-internal-fiber` compiles C only on Windows; on Linux its
  stack switching is inline assembly in Rust.
- Already in the tree before this spike, unrelated to it: `ring` (C and
  assembly, through rustls), `libdeflate-sys` (oxipng), `psm` (assembly,
  through stacker and markdown-it).

## Packaging and installing a third-party plugin

A plugin is a directory: `plugin.toml` and `plugin.wasm` (later
`locales/`, `assets/`, `db/` for tier 0). The author builds the module
(for Rust, `cargo build --release --target wasm32-unknown-unknown`
against the SDK) and ships the directory as a tarball or a git
repository with the built module in it; the module is the same file on
every host and architecture. Installing is dropping the directory under
`PLUGINS_DIR` and restarting: at boot the host checks `api`, compiles
the module, links it against the granted calls (refusing anything else)
and checks the declared phases are exported; a broken plugin stops the
boot, as in the Luau spike. An admin page would show the manifest's
grants (tables, core columns, settings, limits) for approval, the way a
mobile app lists permissions, since the manifest is the whole of what
the plugin can reach.

Compiled code can be cached: `Module::serialize` once per (module hash,
wasmtime version), `Module::deserialize` at boot (0.3 ms instead of
31 ms for solved, 5 to 8 ms instead of about 500 ms for Boa), wasmtime
refusing a cache from another version or CPU feature set.

## Open questions for Jake

1. **Long host calls.** The time budget stops wasm code, not a query in
   progress. Options: `SET LOCAL statement_timeout` around plugin
   queries (on the request's transaction), or cancel through Postgres'
   cancel request. Dropping the future mid-query would leave the
   connection unusable, so a tokio timeout around the phase is not one.
2. **Instance per call or per request.** A fresh instance per phase call
   is the simplest isolation (no state between users) and cheap for a
   Rust guest. Boa pays for its interpreter state on every call even
   pre-initialized; one instance per request across the three phases
   would let `after_load`'s data stay in the guest instead of crossing as
   JSON. Which matters more?
3. **`core_rows` versus API calls versus wider payloads** (carried over
   from the Luau spike): the solved port reads five core tables. Is a
   per-table allowlist with manifest columns acceptable, or should core
   put what plugins need into the payloads?
4. **JavaScript.** Boa inside the guest works with no host change, at a
   4 MB module and hundreds of microseconds a call. Is JavaScript worth
   that, or are Rust (and other compiled) guests enough for third-party
   plugins, with themes on the declarative path?
5. **WIT later?** The core-module ABI is chosen for reach and size; if
   typed bindings for many languages matter more, the component model
   can be added beside it.
6. **wasmtime's helpers.c.** Accept 30 lines of weak-symbol C from the
   runtime, or carry a patch?
7. **Who approves grants**, and whether a `replaces` of a bundled plugin
   should be allowed at all for third-party plugins.

## Every plugin_api hook (spike/wasm-host)

The Plugin trait (`src/plugin_api.rs`) is what a compiled-in plugin such
as the taper port implements. `src/plugin_wasm/api.rs`'s `WasmPlugin`
implements it for a loaded guest, so plugin_api's registry, its
enabled-setting gates and every call site serve both kinds, and
`cli::run` loads `PLUGINS_DIR` beside the compiled-in plugins.

A plugin directory:

```
plugins/echo/
  plugin.toml        manifest: grants, limits, the hooks below
  plugin.wasm        the module
  settings.yml       plugin_settings.yml's shape; readable without a grant
  locales/*.yml      server.en.yml, server.fr.yml, ...
  public/            served at /plugins/<name>/, as Rails serves it
```

| hook | manifest | export | input | output |
|---|---|---|---|---|
| routes | `[[routes]] method, path, form` | `route.handle` | method, path, route, params, query, content_type, body, csrf_token (form routes) | status, content_type, body, location |
| jobs | `jobs`, `[[scheduled]] name, every_secs` | `job.run` | name, args | anything; an error is the job's failure |
| current user keys | `[current_user] keys` | `serializer.current_user` | null | an object of the declared keys |
| topic view keys | `[topic_view] keys` | `serializer.topic_view` | topic_id | an object of the declared keys |
| outlet | `[outlets] topic_above_posts` | `outlet.topic_above_posts` | the topic view | HTML or null |
| post menu | `[outlets] post_menu` | `outlet.post_menu` | topic_id, base_path, posts (id, post_number, user_id, username, yours, deleted) | `{post_id: html}`, drawn first in each post's menu |
| homepage | `[homepage] id, name, path, anonymous, server_side, available` | `homepage.available` (with `available`) | null | bool |
| icons, stylesheet | `[plugin] svg_icons, stylesheet` | none | | |

What the host does around them:

- A write to a guest route (anything but GET and HEAD) is checked
  against the session's CSRF token before the guest runs, so a guest
  can't forget it. A `form = true` route gets the masked token in its
  input and the session cookie on its response.
- A route's answer reaches the response only as status, content type,
  body and (for a redirect) location: a guest sets no cookies or other
  headers. Bodies are UTF-8 text, at most 1 MiB in.
- The current user is resolved for every guest route (`AuthGuardian`)
  and crosses as `guardian` (user id, staff, admin).
- A failing serializer hook or outlet drops its keys or HTML and logs;
  a failing route answers 500; a failing job is retried with the
  guest's message in `last_error`, as core's are.
- `homepage.available` may call `setting` and `t` only: it runs
  whenever `/` is resolved.

`plugins/echo` is `tests/plugin_api.rs`' echo plugin as a guest (95 KiB,
importing `core_rows`, `t` and `setting`), and `tests/plugin_wasm_api.rs`
holds it to the same checks as the compiled-in one: settings and both
locales, routes, the token, `public/` and the stylesheet link, the icon,
jobs and their failure, scheduled jobs, both sets of keys, the outlet,
and the homepage with its availability.

Measured (release, `measure` in both test files, signed in, 2000
requests after 200 warm-up, three rounds):

| | compiled in, p50 | guest, p50 |
|---|---|---|
| `/echo.json` (settings, a translation) | 286 to 297 us | 368 to 395 us |
| `/t/1.json` (topic view hook, one query) | 4.8 to 4.9 ms | 4.9 to 5.0 ms |

The route's 80 to 100 us includes the session lookup the host makes for
every guest route, which the compiled-in echo route skips; the topic
view hook costs about 2% of the page.

Open, for this part:

- Outlet HTML goes into core's page as the guest wrote it: a guest can
  put a script on every topic page. Sanitize it, or accept it as part of
  what an admin approves when installing.
- Route bodies are text; uploads and binary answers are not covered.
- The outlet's guest is anonymous: plugin_api's outlets carry no
  guardian.

## A plugin's own tables and SQL (spike/wasm-host)

`src/plugin_wasm/db.rs`. A plugin with `migrations/*.sql` (or
`database = true`) writes SQL of its own, and Postgres, not the host,
keeps it in bounds.

Per site, at boot (`install_databases`, for every site before the server
listens; a site that can't stops the boot), or else at the first hook
call that needs it, before that call's time budget starts:

- a login role `dpl_<name>_<8 hex of md5(database)>` (roles are the
  cluster's; sites and test databases share one), `NOINHERIT`, with a
  random password in `discourse_rs.plugin_roles`;
- a schema `plugin_<name>` owned by the role and set as its search path
  in that database;
- `REVOKE ALL ON ALL TABLES IN SCHEMA public`, then a column-level
  `GRANT SELECT (cols) ON public.<table>` per `[core_reads]` entry.
  Credentials refuse the install (`DENIED`: `user_passwords`, API keys,
  auth tokens, email tokens, second factors, `web_hooks.secret`, the AI
  secrets, ...);
- `migrations/*.sql` in file order, each once, as the role, recorded in
  its own `schema_migrations`.

The guest calls `query(sql, params)` (rows as JSON objects; a write needs
RETURNING) and `execute(sql, params)` (rows affected). Both run on a pool
of up to 4 connections logged in as the role, never on the app's
connection: there, `SET ROLE` could be undone by `RESET ROLE` or
`set_config('role', ...)`, which Postgres checks against the session
user. Logged in as the role, `RESET ROLE` lands on the role.

Per hook call, one transaction, begun at the first statement:

- read only, except for jobs and routes other than GET and HEAD
  (`execute` is refused outside routes and jobs);
- committed only when the hook succeeds, so a route that fails after
  writing leaves nothing;
- `SET LOCAL statement_timeout` to what is left of the budget. The guest
  can lift it, so the host also races each statement against the
  deadline and cancels the backend (`pg_cancel_backend`; the app is
  granted the role for that) and waits for it to stop rather than
  dropping the connection mid-query;
- a statement's own error (a constraint, a denied table) goes back to
  the guest as an error it can handle; the time limit fails the hook.

Connections go back to the pool through `DISCARD ALL`, so nothing of a
guest's session outlives its hook: settings, advisory locks, LISTEN,
temporary tables, prepared statements. Statements are unnamed, and the
role's connections keep no statement cache. Parameters bind by JSON
type: integers as bigint, other numbers as double, strings as text,
booleans, null untyped (Postgres types it from where it goes), arrays and
objects as JSON text; anything else is cast in the statement
(`$1::date`, `$2::jsonb`).

Hooks that have no pool at hand (serializer keys, outlets, the homepage)
find the site's pool by `current_database()`, which `crate::app`
registers for every site: one more round trip for a plugin with a
database.

Tested in `tests/plugin_wasm_api.rs`: the echo guest migrates its
`echoes` table, a route inserts with the user's id, a failing route's
insert is rolled back, a job inserts, the schema is the role's, and the
topic view key reads `public.topics` through the column grant. The probe
routes show the role can't read `user_passwords`, an ungranted table or
column, `discourse_rs` or another plugin's schema; can't update a core
table or create in `public`; can't write under GET; can't `SET ROLE` or
`set_config('role')` to the app's user (and `RESET ROLE` stays on the
role); doesn't keep a `SET search_path`; and a `pg_sleep(5)` stops at the
500 ms budget either way. `scripts/drop-stale-test-dbs` sweeps the roles
of dropped test databases.

### Hardening (fix/wasm-hardening)

The app's role: a superuser, or `CREATEROLE` on PostgreSQL 16 or later,
plus what the database's owner holds (`discourse-rs setup --owner` makes
it the owner). Before 16, `CREATEROLE` lets a role grant itself
`pg_read_server_files` and the like, so the host doesn't ask for it
there and refuses to install a plugin database unless the app is a
superuser. Each refusal names the fix (`ALTER ROLE ... CREATEROLE`, or
the PostgreSQL version).

What `PUBLIC` reaches, checked on the vendored schema (PostgreSQL 16):

- No table, view or sequence in `public` is granted to `PUBLIC`, and the
  install revokes everything there from the role before its own grants.
  `discourse_functions` and `discourse_rs` give `PUBLIC` no `USAGE`.
- No `SECURITY DEFINER` function outside the system schemas. The 213
  functions in `public` (Discourse's, hstore, pg_trgm, unaccent, vector)
  run as the caller, so the role gets nothing from executing them.
- `CREATE` on `public`: revoked from `PUBLIC` by default since 15, but a
  database upgraded from 14 or older keeps it. There a plugin could put a
  function or operator in `public` that the app's SQL resolves to, and
  run it as the app. The install refuses, naming `REVOKE CREATE ON SCHEMA
  public FROM PUBLIC`.
- `pg_catalog` stays readable, as for every role: table, column, role
  and function names and definitions (all in Discourse's source), and
  row estimates (`pg_class.reltuples`, `pg_stat_user_tables`). Revoking
  catalog access per role isn't possible (privileges add up, and
  `PUBLIC` holds them), and revoking it from `PUBLIC` takes a superuser
  and breaks clients. Accepted: metadata, not rows.
- `NOTIFY` on any channel: pg-bus sends empty notifications as a wake-up
  and keeps its messages in its tables, so a plugin's NOTIFY is a
  spurious wake-up, not a message.

The transaction: a guest's `execute("COMMIT")` or `ROLLBACK` would end
the hook's transaction, so its next statement would commit on its own,
writable under GET and kept when the hook fails. After every successful
`execute` the host compares `now()` with the transaction's start and
fails the hook when they differ, before anything else runs. `query`
can't, as its statement sits in a CTE. What a COMMIT commits before the
check is the hook's own writes so far, in a write hook.

Multisite needs nothing more: the plugins are the process's, as Rails'
are, enabled per site by their settings, and each site's database gets
its own role, schema and pool.

Tested in `tests/plugin_wasm_api.rs`: COMMIT and ROLLBACK under POST
and GET fail the hook with nothing written after; a session advisory
lock is gone once the connection is back; an app role that isn't a
superuser installs with `CREATEROLE` and is refused without it; a
database where `PUBLIC` creates in `public` gets no role; two sites keep
their rows apart. The transaction and lock checks were confirmed by
breaking the code on purpose (the check disabled, `RESET ALL` restored).

Left as accepted: a guest can `SET work_mem` high for its statements
(bounded by its time budget), and can change its own role's password,
which breaks only that plugin's pool.

Open:

- Writes to core data (a post, a notification, a pg-bus message) are
  host calls through core's own code, not built.
- Uninstall (revoke, drop the schema and role) is not built.

## A Rails plugin's tables, core's read models, and a taper port

On a site restored from Rails, a plugin's tables already live in
`public`, created by its Ruby migrations. `[adopt] tables = [...]` names
them: install grants the role SELECT, INSERT, UPDATE and DELETE on each,
and USAGE on its id sequences, and refuses any table core's
`structure.sql` defines or `DENIED` names. The role's search path is its
own schema, then `public`, so a Rails plugin's unqualified SQL resolves
while every name still meets the grants. The tables stay where Rails put
them: a backup restores into either.

Two things a port needs that only core can answer are host calls every
plugin gets (read models, not tables):

- `users(ids)`: id, username, name and `avatar_template` (core's own
  computation, settings and CDN included);
- `can_see_category(id)`: `can_see_category?` for the hook's viewer
  (read restricted, admins and `suppress_secured_categories_from_admin`,
  group access).

`t` also takes `{"key", "locale", "args"}` for interpolation, a route's
input carries `base_path`, and a write with a bad token gets core's
`["BAD CSRF"]` answer.

The port: `discourse-taper-rs` branch `spike/wasm-guest`, `wasm/`, is
`POST /taper/suggest.json` and its band route as a guest, a port of
`suggest::submit`'s JSON path, `suggestions.rs` and the model queries it
uses: about 500 lines against the SDK, 185 KiB built. What changed in
the port:

- the SQL moved over as it was (taper's own tables, adopted; `uploads`
  through `[core_reads]`); typed rows became JSON objects, and dates
  cross as text (`date::text`, `$1::date`, `to_char` for Rails' JSON
  time);
- core calls became host calls: `can_see_archive` is
  `can_see_category(setting("taper_category_id"))`, the serializer's
  users come from `users`, the sequence label from `t` with args;
- the CSRF check left the plugin (the host does it before the call);
- regex (`\bthe\b`, the segue suffix) was hand-written to keep the
  module small; `chrono` stays for date parsing.

`tests/wasm.rs` in taper-rs installs the guest alone and holds it to
`tests/writes.rs`' JSON examples (correction, new source, new show with
setlist text and its nearby shows, a claim and both refusals, login,
token, bad kind, missing payload, an unknown show, an impossible date)
and a hidden archive (a member gets 404, an admin gets through). All
pass; the compiled-in suite still does (137).

What did not fit, or was left out:

- The HTML form and its thank-you page: they render in taper's layout
  with core's head (`PageHead`), which a guest has no call for. A
  `page_head` host call (core's icons, color schemes, theme CSS and
  head/body tags) would cover it.
- The reader's language (`?lang=` and the cookie): the guest answers in
  the default locale. The cookie reaches the guest only if the host
  passes cookies (it passes none), or a `reader_locale` in the input.
- `invalid_parameters`' `Cache-Control: no-cache, no-store`: a guest
  sets no headers but the content type.

## Plugin UI as htmx (spike/wasm-htmx)

Rails plugins draw their UI in Ember (45 of the 46 in core ship client
JS). discourse-rs renders on the server and updates pages with htmx, so a
guest draws HTML instead, and the page's own machinery does the rest:

- `outlet.post_menu` returns each post's button(s) for the viewer, as
  `api.addPostMenuButton` adds one. The topic page, a page loaded further
  down the stream (`/t/:id/posts`) and every `/live` redraw of a post
  call it, once per batch of posts.
- The button carries `hx-post` to the guest's own route. The body's
  `hx-headers` gives htmx the session's token, which the host checks
  before the guest runs. The route answers with the button redrawn, and
  htmx swaps it in place.
- `publish_post_change(post_id)` is `publish_change_to_clients!(:acted)`.
  It is queued and published after the hook's transaction commits, so
  every other page showing the post redraws it through `/live`, for that
  page's viewer, with the guest's `post_menu` run again. Routes and jobs
  only.

`plugins/helpful` is the loop end to end: a mark on another member's post
(its own table), the count alone for its author and for anonymous
readers. `tests/plugin_wasm_htmx.rs` checks the button per viewer, the
click's fragment, an anonymous reader's `/live` redraw, the refusals
(own post, no post, no token) and the plugin turned off. The guest is
about 150 lines and 96 KiB.

The same pattern gives the rest of the outlets (below a post, user card,
header icons, sidebar, composer fields, admin pages) and modals into
`modal.js`'s container. Not done: those outlets; `publish_post_change`
only as `:acted` (no other kinds or options); a `can_see_post` call, so
the guest checks only that the post stands; client state beyond a
fragment (a guest ships its own JS from `public/` for that).

## Productionizing estimate

About three to four weeks for one person, on top of what is here:

- the registry in AppState rather than a process global, per site under
  multisite, and hot reload (2 to 3 days);
- compiled-module cache and boot checks, the admin listing of plugins and
  their grants (2 to 3 days);
- the phases of the other read pipelines (topic view, posts, search, user
  card, current user) with their payloads (1 week);
- write pipelines and events (before: change or refuse; after: the
  event), plus the plugin API of decision 5 for writes (1 to 2 weeks);
- statement timeouts for plugin queries, per-plugin metrics (calls,
  time, skips) in `/srv/status` (2 days);
- SDK docs and a template plugin repository, a second real plugin (topic
  voting or a third-party one) to check the ABI (3 to 4 days).

## Not done

- No write phases or events; routes, jobs, outlets and settings are on
  spike/wasm-host, without writes.
- Migrations and tables of the plugin's own.
- Hot reload; the registry is process wide (`install` at boot).
- The JavaScript guest ports `has_accepted_answer` only, not the posters.
