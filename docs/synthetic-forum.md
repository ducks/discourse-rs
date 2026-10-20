# Public and synthetic forum datasets

## Public Meta baseline

Use the anonymous Discourse MCP collector for real writing rather than the
handwritten synthetic templates. It launches a separate stdio server tethered to
`https://meta.discourse.org`, with an empty authentication list, read-only mode,
and remote tool discovery disabled. The existing connected MCP configuration is
not used or changed. No cookies or API keys are supplied.

The 2026-10-19 collection contains 46 topics, 524 posts, 173 authors, and 17
source categories, with post dates from 2013-02-01 through 2026-10-19. There were
no failed topic reads. `backups/meta-public-mcp/summary.json` records the local
collection counts; reruns can select different topics as Meta changes.

```bash
node scripts/fetch-meta-mcp.mjs /path/to/built/discourse-mcp backups/meta-public-mcp
DV=/home/ducks/discourse/dv/dv scripts/build-synthetic-forum \
  rs-meta 12 discourse_synthetic_meta "$PWD/backups/meta-public-mcp/corpus.json"
```

The MCP path must contain current `dist/index.js` and installed dependencies.
An installed `@discourse/mcp` package directory also works. The numeric topic
argument is ignored in corpus mode; all sampled topics are imported.

Sampling combines all-time/yearly top lists with recent, pre-2020, short, long,
support, bug, development, feature, and how-to searches. This deliberately varied
sample is not a statistically representative sample of the whole forum. Search
results and category aliases can change. Discovery responses and selection labels
are saved alongside the corpus so the actual selection is inspectable.

The collector requests an opening window of 20 posts and, for longer topics, a
bounded recent window. Post IDs are deduplicated, and only regular, nonhidden,
nondeleted posts with complete raw text are retained. Exact-ID reads avoid
duplicate IDs in the upstream stream shrinking the opening sample. A topic must
have its public opening post. Failed reads are recorded explicitly. Completed
topic files can be reused on a rerun; the corpus records per-topic fetch times.

The importer preserves titles, dates, category hierarchy, tags, and reply
relationships when both posts were sampled. It consistently maps authors to
fictional local users with `.invalid` email addresses. Original usernames and
unaltered raw content remain in the local corpus for attribution. Each imported
post has an original-source link in the manifest, outside searchable text.
Quote headers lose source topic/post IDs so they cannot accidentally link to
unrelated local records.

This is a content baseline, not a site clone: source likes, permissions, topic
states, polls, solved records, and uploads are not reconstructed. Event creation
is disabled and event BBCode delimiters are escaped to display literally: the
pinned plugin still validates event blocks when its setting is off. Unmodified
event markup remains in the source corpus. Markdown can
still contain external links, mentions, and upload references. Assets are not
downloaded, so some images will be missing. No public content is committed;
`backups/` is ignored. The import and SQL export happen through the same isolated
Rails workflow described below, with email and background jobs disabled.

## Synthetic scenarios

Build a local browsing and correctness dataset without a production backup or
public-site crawl. Rails creates the data; discourse-rs can then consume the same
database. The source of truth is the generator, not a sequence of MCP calls.

From this checkout, with a running scratch DV agent at `vendor/discourse/DISCOURSE_REF`:

```bash
DV=/home/ducks/discourse/dv/dv scripts/build-synthetic-forum rs-meta
# Small smoke dataset, separate database:
DV=/home/ducks/discourse/dv/dv scripts/build-synthetic-forum rs-meta 12 discourse_synthetic_small
```

The builder refuses existing databases and output directories. It loads the
vendored schema and existing fresh-install fixtures into a separate database,
then runs `scripts/seed-synthetic-forum.rb`. It never replaces the parity seed or
the agent's normal development database. Failed builds leave the scratch database
for inspection; use a new name for another build.

Default additions:

- 40 fictional accounts with varied trust levels and random, undisclosed passwords.
- Three public categories, one subcategory, and a restricted group/category.
- 200 topics with 0, 1, 3, 8, 15, or 100 replies, distributed across fixed dates.
- Tags, likes, quotes, reply links, pinned, closed, archived, and unlisted topics.
- Code fences, tables, details, spoilers, Unicode, escaping, and nested lists.

Email and remote image downloads are disabled. Sidekiq jobs are captured in
process and not executed or published to Redis. Consequently this is not a test
of background processing, delivered notifications, or asynchronously derived data.
Post validation is skipped to permit fixed historical dates and bulk creation;
the real Rails creation/cooking paths still run. This does not test posting limits.
Content and scenario selection are fixed; password hashes, timestamps from
callbacks, and sequence IDs after failed transactions are not byte reproducible.

`backups/synthetic/<database>/` contains an ignored SQL snapshot, a manifest of
created IDs and expected states, and the Rails revision. Restore into a **new**
local database using the PostgreSQL environment from `shell.nix`:

```bash
db_start
createdb discourse_synthetic_local
psql -X -v ON_ERROR_STOP=1 -d discourse_synthetic_local \
  -f backups/synthetic/discourse_synthetic_forum/forum.sql
DATABASE_URL="postgresql://localhost:5442/discourse_synthetic_local?host=$PGHOST" \
  DISCOURSE_RS_JOBS=off cargo run --bin discourse-rs
```

Use the manifest for anonymous/authenticated topic checks and permission probes.
The generator verifies regular-post counts and restricted-category visibility;
Rails status changes may also add small-action posts. Deferred in-process work,
including search indexing, runs synchronously before the export.

Next coverage to add: uploads and local assets, polls and solved state, private
messages, bookmarks/read state, moderation transitions, and automated HTTP
comparisons against both implementations. This initial corpus supplies varied
structure, not Meta's vocabulary or production-scale query distributions.
