# Plugins

How discourse-rs will be extended without writing Rust. Design, not yet
built; the open decisions are at the end.

## The constraint

Discourse's plugins are Ruby loaded into the Rails process. They reach
into everything: models, serializers, routes, jobs, the Ember client.
Porting that model would mean plugins in Rust, compiled into the binary,
which is the opposite of what a self-hoster who wants to tweak their
forum needs. The goal is a plugin written in whatever the author already
knows (Ruby, Python, TypeScript, Go, shell) that core treats as a black
box behind a stable, versioned protocol.

The parity ignore lists in `parity/cases` are the catalogue of plugin
surface the port has stepped around so far: `has_accepted_answer`,
`can_vote`, `reactions`, `valid_flag_applies_to_types`, `category_types`,
`pending_posts` (a plugin-registered NewPostManager handler), custom
fields on categories, `users_on_holiday`. A design that can't express
those isn't one.

## What a Discourse plugin actually does

Inventory of `discourse-solved/plugin.rb`, which is representative:

| plugin.rb call | what it is |
|---|---|
| `config/settings.yml` | site settings with defaults, types, groups |
| `register_asset`, `register_svg_icon` | CSS/JS/icons for the client |
| `add_to_serializer(:post, :accepted_answer) { ... }` | a key on a serializer, computed per object with the viewer (`scope`) in hand |
| `register_topic_preloader_associations`, `register_category_list_topics_preloader_associations` | batch-load the plugin's rows for a whole page so the serializer keys don't N+1 |
| `register_preloaded_category_custom_fields` | which `*_custom_fields` rows to load with the object |
| `register_modifier(:search_rank_sort_priorities) { ... }` | transform a value core computed before core uses it |
| `register_html_builder("server:before-head-close") { ... }` | HTML fragments at named outlets of the server-rendered page |
| `on(:post_destroyed) { ... }` | events, fire-and-forget |
| `app/controllers`, `config/routes.rb` | new endpoints |
| `db/migrate` | own tables |
| `Report.add_report`, admin dashboard sections, `register_mcp_tool` | admin and integration surface |
| `app/jobs` | scheduled and queued work |
| `add_to_class(:composer_messages_finder, ...)` | monkey patches of core; no protocol can offer this and that is fine |

Everything but the last row is a hook with a clear input and output.

## Design: three tiers, one manifest

A plugin is a directory with a `plugin.toml`. The manifest alone gives
tier 0; a program gives tier 1; the same program compiled to WASM gives
tier 2 later. Core loads plugins from `PLUGINS_DIR` (default `plugins/`)
at boot; `discourse-rs plugins` lists them with their state.

### Tier 0: declarative

Everything in the manifest, no code running:

```toml
[plugin]
name = "discourse-solved"
version = "20261001"
api = 1                          # protocol version this plugin speaks
enabled_setting = "solved_enabled"

[settings]                       # same shape as config/settings.yml
solved_enabled = { type = "bool", default = true, client = true }
allow_solved_on_all_topics = { type = "bool", default = false }

[preload]
category_custom_fields = ["enable_accepted_answers", "solved_topics_auto_close_hours"]
topic_custom_fields = []
post_custom_fields = []

[assets]
stylesheets = ["assets/solutions.css"]      # served under /plugins/<name>/
icons = ["far-square-check", "square-check"]

[i18n]
dir = "locales"                  # server.en.yml / client.en.yml, merged

[migrations]
dir = "db"                       # plain .sql, applied in order, tracked in plugin_schema_versions
```

Settings join the site settings table under the plugin's name, with the
same client/server split and the same admin API as core settings.
Custom fields are preloaded into the serialized object exactly as Rails
does (`custom_fields` key at the end). Migrations are SQL files, run by
core with the plugin's name as the lock, against the same database,
tables prefixed by convention (`solved_` as Rails plugins do).

Tier 0 covers settings, assets, custom fields and schema. It is also the
compatibility floor: a plugin that only wants a setting and a stylesheet
never starts a process.

### Tier 1: a process speaking JSON-RPC

```toml
[process]
command = ["ruby", "bin/plugin"]   # or python, node, a binary; anything on PATH
# or: socket = "/run/discourse-solved.sock"  for a plugin someone runs as its own service
hooks = ["serialize", "modify", "html", "route", "event"]   # what it implements
```

Core spawns the command at boot (or connects to the socket), keeps the
process alive, restarts it with backoff when it exits, and speaks
JSON-RPC 2.0 over stdin/stdout, newline-delimited: the shape MCP uses,
and the shape `mcp-stdio` already implements on the Rust side, so the
client library in core and the server libraries plugin authors reach for
exist today in every language. A plugin that implements MCP's
`initialize` handshake is halfway there.

The plugin gets `DATABASE_URL` in its environment and talks to Postgres
itself. Core does not proxy SQL: a plugin has the access a Rails plugin
has (all of it), and the same responsibility to respect permissions. The
`guardian` the hooks pass (user id, groups, staff flags, secure category
ids) is what it has to work with.

Hooks, all requests from core to the plugin:

**`serialize`** - the `add_to_serializer` equivalent, batched per response.

```json
{"method": "serialize", "params": {
  "serializer": "topic_list_item",
  "guardian": {"user_id": 3, "groups": [10, 11, 12], "staff": false, "secure_category_ids": []},
  "objects": [{"id": 35, "category_id": 4, "user_id": 2}, {"id": 38, ...}]
}}
```
```json
{"result": {"35": {"has_accepted_answer": true}, "38": {}}}
```

One call per (serializer, response), never per object: core collects the
page of topics, posts or users, calls once, and merges the returned keys
onto each object after core's own keys (which is where Rails emits plugin
keys too). The manifest declares which serializers the plugin touches so
core skips the call otherwise. Objects carry a fixed, documented column
subset per serializer plus the preloaded custom fields; anything else the
plugin looks up itself, in one query for the batch.

**`modify`** - `register_modifier`: `{"name": "search_rank_sort_priorities", "value": [...], "context": {...}}` returns the new value. Chained across plugins in load order.

**`html`** - `register_html_builder`: `{"outlet": "server:before-head-close", "context": {"path": "/t/x/35", "topic_id": 35, "guardian": ...}}` returns a string. Outlets are the names Rails uses plus ours for the server-rendered pages (`topic:after-posts`, `list:topic-row`, `layout:nav`). Batched per response like `serialize`: one call with every outlet the page has.

**`route`** - the plugin owns paths it declares (`routes = ["/solution/*", "/admin/plugins/solved/*"]`): core authenticates, resolves the guardian, checks CSRF for non-GET, then forwards `{method, path, query, headers subset, body, guardian}` and returns the plugin's `{status, headers, body}` as the response. Plugin controllers in any language, with core's session handling for free.

**`event`** - notifications, no reply expected: `post_created`, `topic_closed`, `user_logged_in`, ... with ids. Delivered after the response is sent.

**`job`** - the plugin declares schedules (`every = "10m"`) and core calls `{"method": "job", "params": {"name": ...}}` on time, serialized per plugin.

Every request carries `api: 1`. Adding a hook or a field is compatible;
removing one bumps the version and core refuses to load a plugin whose
`api` it no longer speaks.

### Failure policy

Core never 500s because a plugin is slow or down:

- `serialize`, `html`, `modify`: a timeout (default 50 ms, per plugin in
  the manifest) or a dead process means the plugin's keys are omitted,
  the outlet is empty, the value is unmodified. Logged with the plugin's
  name, counted in `/srv/status`.
- `route`: 502 with the plugin's name, since the page is the plugin's.
- `event`, `job`: retried with backoff, then dropped with a log line.

This is the one place the port guesses instead of refusing: a forum with
a broken solved plugin shows topics without checkmarks rather than
nothing. The same forum under Rails would be down.

### Tier 2: WASM, later

The same hooks, the same JSON payloads, but the plugin is a WASI
component loaded in-process (wasmtime). No process to supervise, no pipe
latency, sandboxed by construction. Not first because the tooling for
authors is less universal than "write a program that reads stdin", and
because tier 1 already answers the question that matters: can a plugin
be written in any language. A tier 1 plugin written against the JSON
hooks moves to tier 2 by recompiling.

## Cost

A pipe round-trip on this box is 50-100 us. A page with three hooked
serializers and two outlets makes five calls, batched, so under a
millisecond on top of the 3-5 ms the page takes today. The bench harness
will measure it with discourse-solved installed, logged in and out.

Per-object calls would have been the mistake: 30 topics times 3 keys is
90 round-trips. Batching per response is why `serialize` takes a list.

## The first plugin

discourse-solved's read side, ported to this protocol in Ruby (so the
reference plugin's author could recognise it), is the proof:

- tier 0: `solved_enabled` and friends, the category custom fields it
  preloads, its stylesheet
- `serialize`: `has_accepted_answer` on list items and search results,
  `accepted_answer`, `can_accept_answer`, `topic_accepted_answer` on
  posts, `accepted_answers` on user cards, `solved_count` on summaries
- `html`: the `server:before-head-close` schema.org fragment

Success is deleting those keys from the parity ignore lists: with the
plugin installed, the port's responses match Rails with the plugin
installed, byte for byte, through the same harness. Then the voting
plugin (`can_vote`, `vote_count`) the same way, in a different language.

## Where it sits in the roadmap

Reads are done (milestones 1-3 minus notifications); writes are next.
The plugin protocol is designed now and built before writes, for two
reasons: the `serialize`/`html`/`route` hooks are read-side and testable
today against the parity harness, and the write-side hooks (`modify` on
NewPostManager, `event` on post_created) should be designed into the
write path rather than bolted on after. The roadmap's "no plugin system
before writes" line goes; the ordering becomes: protocol and tier 0/1
read hooks with discourse-solved as the test, then writes with their
hooks, then tier 2.

## Decisions

1. **Where the frontend hooks go.** The server-rendered HTML takes
   outlets (`html` hook). When the Ember client runs against this port,
   plugin JS is the client's business (the plugin ships it as an asset);
   core only serves the files. Revisit if the port grows its own client.
2. **Settings ownership: shared.** Plugin settings live in the
   `site_settings` table like Rails', so a backup carries them and the
   admin API is one API. Decided 2026-10-01.
3. **No host permissions, ever (2026-10-01).** The subprocess tier above
   is rejected: a plugin with the host's permissions is the Drupal model.
   The direction instead is Shopify's: one scoped plugin API (API-key
   scopes, `plugin_store_rows`, custom fields), and two ways to run a
   plugin against it, in-process as a WASI component for installable
   plugins (drop a directory in, no host access) or as a hosted service
   over signed HTTP for plugins that are services anyway. Tier 1 as
   written is to be replaced by that split; parked while sessions slice
   3 is built.
4. **Admin UI.** Plugins that register admin pages do it through `route`
   with their own HTML. No admin framework from core until there is an
   admin.
