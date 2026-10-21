# Records what Discourse does for write requests, for discourse-rs to be
# measured against (run by scripts/record-writes through `bin/rails runner`).
#
# Each case is a list of requests made by one user through an integration
# session. They run on a pinned connection inside a transaction that is
# rolled back, as Rails' transactional tests do, so the site is left as it
# was. Recorded per case: each response, every row the requests inserted,
# updated or deleted (rows as Postgres' to_jsonb writes them), and the jobs
# they enqueued. A case with `run_jobs` also runs those of its enqueued
# jobs, and records what they enqueue in turn (`jobs_from_jobs`); its
# `enqueue` jobs are queued after its requests, for run_jobs to run.
#
# Before a case every id sequence is set to its table's max(id), so the
# rows a case inserts get the ids discourse-rs will give them.
require "json"
require "fileutils"
require "socket"

cases_file, out, files_dir = ARGV
abort "usage: record_writes.rb <cases json array> <dir> [files-dir]" if cases_file.blank? || out.blank?
FileUtils.mkdir_p(out)

ENQUEUED = []
module Jobs
  def self.enqueue(name, args = {}) = (ENQUEUED << [name.to_s, args.except(:current_site_id)]) && nil
  def self.enqueue_in(delay, name, args = {}) =
    (ENQUEUED << ["#{name} in #{delay.to_i}s", args.except(:current_site_id)]) && nil
  def self.enqueue_at(at, name, args = {}) = (ENQUEUED << ["#{name} at", args.except(:current_site_id)]) && nil
  def self.cancel_scheduled_job(name, args = {})
    ENQUEUED.reject! { |n, a| n.split(" ").first == name.to_s && a.to_h.stringify_keys.slice(*args.keys.map(&:to_s)) == args.stringify_keys }
    nil
  end
end
# BackupRestore.backup! and restore! spawn a detached process that backs
# up or restores outside the case's transaction: recorded as a job
# instead (the port queues the same), never run.
require "backup_restore"
# pg_dump connects as ENV["USER"] (BackupRestore.database_configuration),
# which `dv run` leaves unset: the process's own user, as a shell has it.
require "etc"
ENV["USER"] ||= Etc.getpwuid.name
BackupRestore.define_singleton_method(:spawn_process!) do |type, user_id, opts|
  ENQUEUED << ["spawn_backup_restore", { type: type.to_s, user_id: user_id, opts: opts }]
  nil
end
# The admin dashboard builds its sections and bulk reports on thread pools,
# each thread with a connection of its own that would not see the case's
# transaction (nor share it): build them on the case's connection, in order,
# as the port does.
AdminDashboardSectionLoader.prepend(Module.new { def build(parallel: true) = super(parallel: false) })
AdminDashboard::Reports::BulkFetch.singleton_class.prepend(
  Module.new do
    def fetch_in_parallel(items, guardian:, filters:)
      items.map do |item|
        fetch_item(item, guardian:, filters:)
      rescue StandardError
        AdminDashboard::Reports::BulkFetch::FETCH_FAILED
      end
    end
  end,
)
ActionController::Base.allow_forgery_protection = false
ActionMailer::Base.delivery_method = :test
ActionMailer::Base.perform_deliveries = true
RateLimiter.disable
# Scheduler::Defer runs its blocks on a thread of its own after the
# response, racing the case's snapshot: held here, and run after each
# request (and after the login and the jobs), so what they write lands in
# the case's transaction in order, as tests/writes.rs drains the port's.
Scheduler::Defer.pause

def db = ActiveRecord::Base.connection

# A delivered message as recordings keep it (its Date varies).
def email_record(m)
  {
    headers: m.header.fields.reject { |f| %w[Date].include?(f.name) }.map { |f| [f.name, f.value.to_s] },
    text: m.text_part&.decoded || (m.multipart? ? nil : m.decoded),
    html: m.html_part&.decoded,
  }
end

# `{"fixture": "name"}` in a case's params: that file from
# parity/writes/files, sent as a multipart upload.
def fixtures(value, files_dir)
  case value
  when Hash
    if value.keys == ["fixture"]
      path = File.join(files_dir, value["fixture"])
      Rack::Test::UploadedFile.new(path, Rack::Mime.mime_type(File.extname(path), "application/octet-stream"))
    else
      value.transform_values { |v| fixtures(v, files_dir) }
    end
  else
    value
  end
end

# A zip response (the custom emoji export) as its entries, by name: each
# one's size and SHA-256, and a CSV's text. Its bytes carry the archive's
# timestamps and compression, which the port needn't match.
def zip_entries(bytes)
  require "zip"
  entries = []
  Zip::File.open_buffer(StringIO.new(bytes)) do |zip|
    zip.each do |entry|
      next if entry.directory?
      data = entry.get_input_stream.read
      item = { "name" => entry.name, "size" => data.bytesize, "sha256" => Digest::SHA256.hexdigest(data) }
      item["text"] = data.force_encoding(Encoding::UTF_8) if entry.name.end_with?(".csv")
      entries << item
    end
  end
  { "zip" => entries.sort_by { |e| e["name"] } }
end

def multipart?(value)
  value.is_a?(Hash) && value.values.any? { |v| v.is_a?(Rack::Test::UploadedFile) || multipart?(v) }
end

# `{{path.to.value}}` (optionally `|reverse`) in a case's path or params:
# a value from the responses so far or the last job of a name enqueued so far,
# for honeypots, challenges and emailed tokens.
def resolve(value, state)
  case value
  when Hash
    value.transform_values { |v| resolve(v, state) }
  when Array
    value.map { |v| resolve(v, state) }
  when String
    # `{{totp:SECRET}}`: the code a base32 TOTP secret gives now.
    value =
      value.gsub(/\{\{totp:([A-Z2-7]+)\}\}/) do
        require "rotp"
        ROTP::TOTP.new(Regexp.last_match(1)).now
      end
    value.gsub(/\{\{([^}|]+)(\|reverse|\|last_segment|\|request_param|\|state_param|\|sso_nonce|\|totp)?\}\}/) do
      path, filter = Regexp.last_match(1), Regexp.last_match(2)
      found = path.split(".").reduce(state) { |acc, k| acc.is_a?(Array) ? acc[k.to_i] : acc&.[](k) }
      raise "{{#{path}}} resolved to nothing" if found.nil?
      case filter
      when "|reverse" then found.to_s.reverse
      when "|last_segment" then found.to_s.split("/").last
      # A url's `request` query value (the device flow's request token).
      when "|request_param" then CGI.unescape(found.to_s[/[?&]request=([^&#]*)/, 1].to_s)
      # A url's `state` query value (an OAuth authorize redirect's).
      when "|state_param" then CGI.unescape(found.to_s[/[?&]state=([^&#]*)/, 1].to_s)
      # The nonce in a DiscourseConnect redirect's signed payload.
      when "|sso_nonce"
        payload = CGI.unescape(found.to_s[/[?&]sso=([^&#]*)/, 1].to_s)
        Rack::Utils.parse_query(Base64.decode64(payload))["nonce"].to_s
      # The code a TOTP secret (spaced as the key is shown) gives now.
      when "|totp"
        require "rotp"
        ROTP::TOTP.new(found.to_s.delete(" ")).now
      else found.to_s
      end
    end
  else
    value
  end
end

# Tables with their primary key columns (none for a table without one).
TABLES =
  db
    .select_rows(<<~SQL)
      SELECT c.relname,
             COALESCE((SELECT string_agg(a.attname, ',' ORDER BY array_position(i.indkey, a.attnum))
                       FROM pg_index i JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey)
                       WHERE i.indrelid = c.oid AND i.indisprimary), '')
      FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = 'public' AND c.relkind = 'r'
      ORDER BY c.relname
    SQL
    .to_h { |name, pk| [name, pk.split(",")] }

# Written by the agent's own scheduler while a case runs, not by the case,
# unless the case names them in `include_tables` (a case running the job
# that writes them).
BACKGROUND_TABLES = %w[scheduler_stats top_topics]
INCLUDED_TABLES = []

# Redis is not rolled back: the duplicate post check keeps a key per user
# and post body, and Email::Processor one per address and rejection a day,
# which would change the same case recorded twice.
def clear_redis_state
  # user-last-seen: UserStat.update_time_read!, which would count the
  # seconds since the previous case read a topic.
  # __SEARCH__LOG_: SearchLog.log updates the log of a search made in the
  # last 5 seconds (a previous case's) instead of inserting one.
  # LAST_UPLOAD_CLEANUP: CleanUpUploads skips orphans for a day after the
  # reference's own scheduler ran it.
  # l-rate-limit3: the rate limiters' lists, for cases that turn them on.
  # unique-pm-post: the duplicate check's key for posts in messages.
  # grpmsg, last_notified_reviewable_id: what GroupMessage and the
  # reviewables reminder remember having sent.
  # badge_queue: BadgeGranter's queue, which a backlog case would drain
  # with earlier cases' payloads in it.
  # _CACHE:inline_onebox, _CACHE:onebox__, _CACHE:onebox_failed__: crawled
  # titles and oneboxes (InlineOneboxer, Oneboxer), kept up to a day.
  # about-stats: the about page's cached stats (About.fetch_cached_stats).
  # summary-new-users: the digest's new users count, kept a day.
  # directory_*: when the reference's own scheduler last refreshed the
  # user directory (DirectoryItem.last_updated_at, DirectoryRefresh).
  # _CACHE:watched-words-list: WordWatcher's word lists, cached a day
  # (empty ones too), which would hide a case's setup words from Rails.
  # otp_*: the user API one-time passwords a case hands out.
  # readonly_mode*: read-only mode's keys (a case's `readonly`, or the
  # admin toggle), which would refuse every later case's writes.
  # email-backup-token:*, backup_restore_operation_*, start_logs_message_id:
  # the backups' download tokens and running state (a case's `redis`).
  # _CACHE:reports:*, *-dashboard-data-*: cached reports and dashboard
  # stats (Report.cache, StatsCacheable), kept up to an hour.
  # ban_user_from_external_uploads_*: ExternalUploadManager's five
  # minute ban after a size mismatch, which would refuse later cases.
  # user-profile-view:*: UserProfileView.add's once-a-day marks, which
  # would leave out a profile an earlier case viewed.
  # user:<id>:<date>: User.should_update_last_seen?'s mark, kept
  # active_user_rate_limit_secs, which would leave out a page load's
  # last_seen_at and visit when an earlier case loaded one as the same user.
  # new_features*, latest_new_feature_created_at, refresh_latest_new_feature_lock,
  # last_viewed_feature_dates_for_users_hash: DiscourseUpdates' feed and
  # what each admin has seen of it (a case's `redis`, its requests).
  %w[ban_user_from_external_uploads_* user-profile-view:* user:*:????-??-?? readonly_mode* email-backup-token:* backup_restore_operation_* start_logs_message_id postgres:last_readonly otp_* unique-post-* unique-pm-post-* rejection_email:* user-last-seen:* hot_score_6_hourly __SEARCH__LOG_* LAST_UPLOAD_CLEANUP l-rate-limit3:* grpmsg:* last_notified_reviewable_id _CACHE:inline_onebox:* _CACHE:onebox__* _CACHE:onebox_failed__* about-stats badge_queue directory_* summary-new-users:* _CACHE:watched-words-list:* _CACHE:reports:* index-dashboard-data-* general-dashboard-data-* _CACHE:solved_admin_dashboard_kpi_enabled new_features new_features_last_seen_user_* latest_new_feature_created_at refresh_latest_new_feature_lock last_viewed_feature_dates_for_users_hash].each do |pattern|
    Discourse.redis.keys(pattern).each { |k| Discourse.redis.del(k) }
  end
  # Oneboxer's preferred strategies, outside the namespace.
  Discourse.redis.without_namespace.keys("ONEBOXER_STRATEGY_*").each { |k| Discourse.redis.without_namespace.del(k) }
  # FinalDestination's https domains, outside the namespace too.
  Discourse.redis.without_namespace.keys("HTTPS_DOMAIN_*").each { |k| Discourse.redis.without_namespace.del(k) }
  # Presence lives in Redis too, members and cached configs alike.
  PresenceChannel.clear_all!
  # MessageBus backlogs: responses carry channels' last ids (chat's
  # message_bus_last_ids), which an earlier case's publishes would raise.
  MessageBus.reset!
end

# Caches of rows a case's setup inserts (and the rollback removes): the
# serialized site categories, the custom emoji, and topic-voting's voting
# categories.
def reset_plugin_caches
  Site.clear_cache
  Emoji.clear_cache
  # The theme caches (the user selectable and parent theme ids) and the
  # stylesheet manager's, which settings (default_theme_id) and theme
  # writes expire after commit, so never within a case.
  Theme.clear_cache!
  Stylesheet::Manager.cache.clear
  ApplicationSerializer.expire_cache_fragment!("user_themes")
  Category.reset_voting_cache if Category.respond_to?(:reset_voting_cache)
  # discourse-solved's in-memory list of categories that take answers.
  DiscourseSolved::AcceptedAnswerCache.reset_accepted_answer_cache if defined?(DiscourseSolved::AcceptedAnswerCache)
  # WordWatcher's word lists (Discourse.cache, in Redis).
  WordWatcher.clear_cache!
  # PostActionType's flag caches (FlagSettings in process, Redis keys),
  # which a flags case's after_commit fills with a flag the rollback
  # removes.
  PostActionType.reload_types
  # The translation overrides I18n keeps per site (translate_accelerator),
  # which TranslationOverride's own writes reload but setup SQL and the
  # rollback do not.
  I18n.instance_variable_set(:@overrides_by_site, {})
  # Theme's in-process cache (user selectable themes, components, cached
  # settings), which setup SQL and the rollback leave stale.
  Theme.clear_cache!
end

# Every file and directory under public/backups.
def backup_files = Dir.glob(File.join(Rails.root, "public", "backups", "**", "*"))

# `s3`: the case runs on the S3 test server (MinIO at
# http://minio.localhost:9122, which scripts/record-writes starts in the
# agent; tests/writes.rs starts its own): its `buckets` made if missing
# and emptied before and after, its `objects` ({"bucket/key": fixture})
# put first, and the objects in the buckets afterwards recorded
# (`s3_objects`). The case's settings point the site at the server.
require "aws-sdk-s3"
S3_TEST = {
  endpoint: "http://minio.localhost:9122",
  region: "us-east-1",
  access_key_id: "discourse-rs-test",
  secret_access_key: "discourse-rs-test-secret",
  force_path_style: true,
}
def s3_test_client = (@s3_test_client ||= Aws::S3::Client.new(**S3_TEST))

def s3_buckets(c) = Array(c.dig("s3", "buckets"))

def s3_empty(buckets)
  client = s3_test_client
  buckets.each do |bucket|
    begin
      client.head_bucket(bucket:)
    rescue Aws::S3::Errors::NotFound
      client.create_bucket(bucket:)
      next
    end
    client.list_multipart_uploads(bucket:).uploads.each do |u|
      client.abort_multipart_upload(bucket:, key: u.key, upload_id: u.upload_id)
    end
    loop do
      keys = client.list_objects_v2(bucket:).contents.map(&:key)
      break if keys.empty?
      client.delete_objects(bucket:, delete: { objects: keys.map { |k| { key: k } } })
    end
    begin
      client.delete_bucket_lifecycle(bucket:)
    rescue Aws::S3::Errors::ServiceError
      nil
    end
  end
end

# `s3.lifecycle`: the buckets' expiry rules recorded too.
def s3_lifecycle(buckets)
  buckets.to_h do |bucket|
    rules =
      begin
        s3_test_client
          .get_bucket_lifecycle_configuration(bucket:)
          .rules
          .map do |r|
            { id: r.id, status: r.status, days: r.expiration&.days, prefix: r.filter&.prefix || r.prefix }
          end
      rescue Aws::S3::Errors::NoSuchLifecycleConfiguration
        []
      end
    [bucket, rules]
  end
end

def s3_objects(buckets)
  client = s3_test_client
  buckets.flat_map do |bucket|
    client
      .list_objects_v2(bucket:)
      .contents
      .map do |o|
        h = client.head_object(bucket:, key: o.key)
        {
          bucket:,
          key: o.key,
          size: h.content_length,
          etag: h.etag.to_s.delete('"'),
          content_type: h.content_type.presence,
          cache_control: h.cache_control.presence,
          content_disposition: h.content_disposition.presence,
          metadata: h.metadata,
        }
      end
  end
end

def checksums
  (TABLES.keys - (BACKGROUND_TABLES - INCLUDED_TABLES)).to_h do |t|
    [t, db.select_value("SELECT md5(COALESCE(string_agg(x::text, '|' ORDER BY x::text), '')) FROM #{db.quote_table_name(t)} x")]
  end
end

def rows(table)
  db.select_values("SELECT to_jsonb(x)::text FROM #{db.quote_table_name(table)} x").map { |r| JSON.parse(r) }
end

def reset_sequences
  db.select_rows(<<~SQL).each do |seq, table, column|
    SELECT s.relname, t.relname, a.attname
    FROM pg_class s
    JOIN pg_depend d ON d.objid = s.oid AND d.deptype = 'a'
    JOIN pg_class t ON t.oid = d.refobjid
    JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = d.refobjsubid
    WHERE s.relkind = 'S'
  SQL
    max = db.select_value("SELECT max(#{db.quote_column_name(column)}) FROM #{db.quote_table_name(table)}").to_i
    if max > 0
      db.execute("SELECT setval('#{seq}', #{max}, true)")
    else
      db.execute("SELECT setval('#{seq}', 1, false)")
    end
  end
end

# The id each table's sequence hands out next: where a case's new rows
# start, though rows it inserts and deletes again never show in its changes.
def next_ids
  db.select_rows(<<~SQL).to_h { |table, last| [table, last.nil? ? 1 : last.to_i + 1] }
    SELECT t.relname, ps.last_value
    FROM pg_class s
    JOIN pg_depend d ON d.objid = s.oid AND d.deptype = 'a'
    JOIN pg_class t ON t.oid = d.refobjid
    JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = d.refobjsubid AND a.attname = 'id'
    JOIN pg_sequences ps ON ps.schemaname = 'public' AND ps.sequencename = s.relname
    WHERE s.relkind = 'S'
  SQL
end

def diff(before_sums, before_rows)
  after = checksums
  changed = after.keys.select { |t| after[t] != before_sums[t] }
  changed.to_h do |t|
    pk = TABLES[t]
    key = ->(r) { pk.empty? ? r : pk.map { |c| r[c] } }
    old = before_rows.fetch(t).to_h { |r| [key.(r), r] }
    new = rows(t).to_h { |r| [key.(r), r] }
    [
      t,
      {
        inserted: (new.keys - old.keys).map { |k| new[k] },
        deleted: (old.keys - new.keys).map { |k| old[k] },
        updated: (new.keys & old.keys).filter_map { |k| { before: old[k], after: new[k] } if old[k] != new[k] },
      },
    ]
  end
end

cases = JSON.parse(File.read(cases_file))
pool = ActiveRecord::Base.connection_pool

# DB.after_commit defers to the open transaction, and the pinned one never
# commits, so callbacks like Topic.reset_highest would never run. Discourse's
# test environment compares against its test transaction instead
# (MiniSqlMultisiteConnection#transaction_open? with test_transaction); do the
# same with the pinned one.
PINNED = []
DB.define_singleton_method(:transaction_open?) do
  ActiveRecord::Base.connection.current_transaction != PINNED.last
end

# `remote`: the other site's pages a case fetches (inline onebox titles),
# on http://fixtures.localhost:8099 (loopback here, as on the host the
# replay runs on): request targets and what each answers, `{status,
# headers, body}`; anything else is a 404. tests/writes.rs serves the same.
REMOTE = {}
# A case's `env` values' previous ones, put back after it.
ENV_BEFORE = {}
if cases.any? { |c| c["remote"] }
  # "::" takes IPv4 too.
  server = TCPServer.new("::", 8099)
  Thread.new do
    loop do
      Thread.new(server.accept) do |s|
        head = +""
        head << s.readpartial(4096) until head.include?("\r\n\r\n")
        # The request's body is read too (a web hook's POST), so closing
        # the socket with it unread can't reset the connection.
        fields, rest = head.split("\r\n\r\n", 2)
        length = fields[/^content-length:\s*(\d+)/i, 1].to_i
        rest = rest.to_s
        rest << s.readpartial(4096) while rest.bytesize < length
        method, target = head.lines.first.to_s.split(" ")
        # A target given without a query answers whatever query is sent
        # (a date the request computes).
        page = REMOTE[target] || REMOTE[target.to_s.split("?", 2).first] || {}
        body = page["body"].to_s
        s.write "HTTP/1.1 #{page["status"] || 404} X\r\nConnection: close\r\nContent-Length: #{body.bytesize}\r\n"
        (page["headers"] || {}).each { |k, v| s.write "#{k}: #{v}\r\n" }
        s.write "\r\n"
        s.write body if method != "HEAD"
      rescue EOFError, IOError, SystemCallError
        nil
      ensure
        s.close
      end
    end
  end
end

# `remote_hosts`: a login provider's fixed endpoints (https://github.com,
# https://www.googleapis.com, ...) reached on the fixtures server instead,
# under the path each maps to. Faraday builds every URL the OAuth2
# strategies request through build_exclusive_url; tests/writes.rs sets
# the port's Config::auth_provider_hosts the same way.
REMOTE_HOSTS = {}
module RecordRemoteHosts
  def build_exclusive_url(*args)
    uri = super
    s = uri.to_s
    REMOTE_HOSTS.each do |from, to|
      if s.start_with?(from)
        return URI.parse("http://fixtures.localhost:8099#{to}#{s.delete_prefix(from)}")
      end
    end
    uri
  end
end
Faraday::Connection.prepend(RecordRemoteHosts)
# The oauth gem (Twitter) reaches its site through Net::HTTP: the site
# itself moved, the request paths kept.
require "oauth"
module RecordRemoteOAuthHosts
  def site
    s = super
    REMOTE_HOSTS.each do |from, to|
      return "http://fixtures.localhost:8099#{to}#{s.delete_prefix(from)}" if s.start_with?(from)
    end
    s
  end
end
OAuth::Consumer.prepend(RecordRemoteOAuthHosts)

cases.each do |c|
  ENQUEUED.clear
  REMOTE_HOSTS.replace(c["remote_hosts"] || {})
  backups_before = backup_files
  pool.pin_connection!(true)
  PINNED.replace([ActiveRecord::Base.connection.current_transaction])
  begin
    reset_sequences
    clear_redis_state
    session = ActionDispatch::Integration::Session.new(Rails.application)
    session.host! Discourse.current_hostname
    headers = { "X-Requested-With" => "XMLHttpRequest", "Accept" => "application/json" }
    if c["user"]
      session.post "/session.json", params: { login: c["user"], password: "password" }, headers: headers
      raise "login as #{c["user"]} failed: #{session.response.status}" if session.response.status != 200
      Scheduler::Defer.do_all_work
    end
    # `cookies`: set in the session's jar after the login (the
    # forced_color_mode cookie the interface color selector writes).
    (c["cookies"] || {}).each { |name, value| session.cookies[name] = value }
    # A case's `settings` are set inside the transaction, so they roll back
    # with it (the in-process cache is refreshed after).
    (c["settings"] || {}).each { |name, value| SiteSetting.set(name, value) }
    # A page's `{"fixture": name}` body: that file's text.
    REMOTE.replace(
      (c["remote"] || {}).transform_values do |page|
        next page if !page["body"].is_a?(Hash)
        page.merge("body" => File.read(File.join(files_dir, page["body"]["fixture"])))
      end,
    )
    if c["s3"]
      s3_empty(s3_buckets(c))
      (c.dig("s3", "objects") || {}).each do |path, name|
        bucket, key = path.split("/", 2)
        s3_test_client.put_object(bucket:, key:, body: File.binread(File.join(files_dir, name)))
      end
    end
    # `files`: fixture files stored at upload urls, for rows the setup
    # inserts (they are not rolled back: removed after the case).
    (c["files"] || {}).each do |url, name|
      path = File.join(Rails.root, "public", url)
      FileUtils.mkdir_p(File.dirname(path))
      FileUtils.cp(File.join(files_dir, name), path)
    end
    # `setup`: SQL for fixture rows a case needs (a reply key), run the same way.
    (c["setup"] || []).each { |sql| db.execute(sql) }
    # Always: a case's settings (emoji_deny_list) are cached too.
    reset_plugin_caches
    # `readonly`: read-only mode's Redis keys set for the case's requests
    # (readonly_mode:user, readonly_mode:staff_writes_only, ...), as
    # Discourse.enable_readonly_mode sets them, less the TTL keep-alive.
    (c["readonly"] || []).each { |key| Discourse.redis.set(key, 1) }
    # `redis`: other Redis values the case's requests read (a backup
    # download token, the backups' running flag, a `{"fixture": name}`'s
    # file: the new features feed), set without a TTL.
    (c["redis"] || {}).each do |key, value|
      value = File.read(File.join(files_dir, value["fixture"])) if value.is_a?(Hash)
      Discourse.redis.set(key, value)
    end
    # `env`: environment variables for the case (the new features
    # endpoint), restored after it.
    ENV_BEFORE.replace((c["env"] || {}).to_h { |k, _| [k, ENV[k]] })
    (c["env"] || {}).each { |k, v| ENV[k] = v }
    # `rate_limits`: the limiter on for the case's requests and jobs only
    # (after the login and setup, which would count otherwise).
    RateLimiter.enable if c["rate_limits"]
    INCLUDED_TABLES.replace(c["include_tables"] || [])
    ENQUEUED.clear
    before_sums = checksums
    before_rows = before_sums.keys.to_h { |t| [t, rows(t)] }
    first_ids = next_ids
    started_at = db.select_value("SELECT to_jsonb(clock_timestamp()::timestamp)::text")
    transaction_started_at = db.select_value("SELECT to_jsonb(transaction_timestamp()::timestamp)::text")
    responses = []
    ActionMailer::Base.deliveries.clear
    c["requests"].each do |r|
        state = { "responses" => responses.map { |x| JSON.parse(x.to_json) }, "jobs" => ENQUEUED.to_h { |n, a| [n.split(" ").first, JSON.parse(a.to_json)] } }
        # A GET with `as: :json` goes out as a POST with X-Http-Method-Override,
        # which the integration session writes into the headers it was given.
        params = fixtures(resolve(r["params"] || {}, state), files_dir)
        options = { params: params, headers: headers.merge(r["headers"] || {}) }
        options[:as] = :json if r["method"] != "GET" && !multipart?(params)
        path = resolve(r["path"], state)
        # `discourse_connect`: a DiscourseConnect provider's answer, its
        # `fields` signed with `secret` as DiscourseConnectBase#payload
        # signs them, added to the path's query.
        if (dc = r["discourse_connect"])
          dc = resolve(dc, state)
          payload = Base64.strict_encode64(Rack::Utils.build_query(dc["fields"]))
          sig = OpenSSL::HMAC.hexdigest("sha256", dc["secret"], payload)
          path = "#{path}#{path.include?("?") ? "&" : "?"}sso=#{CGI.escape(payload)}&sig=#{sig}"
        end
        # Unicorn sets REQUEST_PATH; log_on_user logs it as the token's path.
        options[:env] = { "REQUEST_PATH" => path.split("?").first }
        session.public_send(r["method"].downcase, path, **options)
        Scheduler::Defer.do_all_work
        # `stylesheet`: the page's stylesheet link of that data-target (or
        # class, for the color definitions) fetched in its place, by a
        # session of its own (StylesheetsController#show drops the
        # cookies); the response is the stylesheet's.
        last = session.response
        if (target = r["stylesheet"])
          link = last.body[/<link href="([^"]+)"(?:\s+[a-z-]+="[^"]*")*\s+(?:data-target|class)="#{Regexp.escape(target)}"/, 1]
          raise "no #{target} stylesheet on #{path}" if !link
          sheets = ActionDispatch::Integration::Session.new(Rails.application)
          sheets.host! Discourse.current_hostname
          sheets.get(CGI.unescapeHTML(link).sub(%r{\Ahttps?://[^/]+}, ""))
          last = sheets.response
        end
        body = last.body
        body =
          if last.media_type == "application/zip"
            zip_entries(body)
          else
            (JSON.parse(body) rescue body)
          end
        # `extract`: an HTML page kept as what the regexes capture, every
        # match's groups joined by `|`, a line a match (tests/writes.rs
        # extracts the same from the port's page).
        if c["extract"] && body.is_a?(String)
          body =
            c["extract"]
              .flat_map { |pattern| body.scan(Regexp.new(pattern)).map { |groups| Array(groups).map(&:to_s).join("|") } }
              .join("\n")
        end
        response = { status: last.status, body: body }
        # The rate limit headers, kept only when a response has them, and
        # those the case names in `keep_headers` (a redirect's Location, as
        # its path, the host being each side's own).
        kept = last.headers.to_h.slice("Retry-After", "Discourse-Rate-Limit-Error-Code", "Discourse-Actions-Remaining", "Discourse-Actions-Max", *(c["keep_headers"] || []))
        # A relative one (OmniAuth's failure redirect) is kept as sent.
        if kept["Location"]
          location = URI(kept["Location"])
          kept["Location"] = location.request_uri if location.respond_to?(:request_uri)
        end
        response[:headers] = kept if kept.any?
        responses << response
      end
    # run_jobs: the named jobs run in order (delayed ones too), as Sidekiq
    # would, including those the jobs themselves enqueue; what the jobs
    # enqueue is recorded apart, and mail is captured, not sent.
    from_requests = ENQUEUED.dup
    ENQUEUED.clear
    # Mail the requests send themselves (the admin's test email), apart
    # from the jobs'.
    request_emails = ActionMailer::Base.deliveries.map { |m| email_record(m) }
    ActionMailer::Base.deliveries.clear
    names = c["run_jobs"] || []
    pending = from_requests.dup
    # `enqueue`: jobs the case puts in the queue itself, after its requests
    # (a scheduled job, say), run as run_jobs runs the requests'.
    pending.concat((c["enqueue"] || []).map { |name, args| [name, args || {}] })
    while (job = pending.shift)
      name, args = job
      base = name.split(" ").first
      next if !names.include?(base)
      seen = ENQUEUED.size
      # Core jobs are enqueued by name, plugins' by class.
      klass = base.start_with?("Jobs::") ? base : "Jobs::#{base.camelize}"
      klass.constantize.new.execute(args.with_indifferent_access)
      pending.concat(ENQUEUED[seen..])
    end
    Scheduler::Defer.do_all_work
    emails = ActionMailer::Base.deliveries.map { |m| email_record(m) }
    record = {
      name: c["name"],
      user: c["user"],
      requests: c["requests"],
      started_at: JSON.parse(started_at),
      transaction_started_at: JSON.parse(transaction_started_at),
      responses: responses,
      changes: diff(before_sums, before_rows),
      jobs: from_requests,
    }
    record[:next_ids] = first_ids.slice(*record[:changes].select { |_, c| c[:inserted].any? }.keys)
    record[:settings] = c["settings"] if c["settings"]
    record[:setup] = c["setup"] if c["setup"]
    record[:readonly] = c["readonly"] if c["readonly"]
    record[:redis] = c["redis"] if c["redis"]
    record[:enqueue] = c["enqueue"] if c["enqueue"]
    record[:include_tables] = c["include_tables"] if c["include_tables"]
    record[:approximate] = c["approximate"] if c["approximate"]
    record[:digests] = c["digests"] if c["digests"]
    record[:unordered] = c["unordered"] if c["unordered"]
    record[:unordered_responses] = c["unordered_responses"] if c["unordered_responses"]
    record[:job_clock] = c["job_clock"] if c["job_clock"]
    record[:random_delays] = c["random_delays"] if c["random_delays"]
    record[:files] = c["files"] if c["files"]
    record[:remote] = c["remote"] if c["remote"]
    record[:env] = c["env"] if c["env"]
    record[:remote_hosts] = c["remote_hosts"] if c["remote_hosts"]
    record[:rate_limits] = c["rate_limits"] if c["rate_limits"]
    record[:keep_headers] = c["keep_headers"] if c["keep_headers"]
    record[:fragment] = c["fragment"] if c["fragment"]
    record[:extract] = c["extract"] if c["extract"]
    record[:cookies] = c["cookies"] if c["cookies"]
    record[:jobs_from_jobs] = ENQUEUED.dup if c["run_jobs"]
    if c["s3"]
      record[:s3] = c["s3"]
      record[:s3_objects] = s3_objects(s3_buckets(c))
      record[:s3_lifecycle] = s3_lifecycle(s3_buckets(c)) if c.dig("s3", "lifecycle")
    end
    record[:emails] = emails if emails.any?
    record[:request_emails] = request_emails if request_emails.any?
    File.write("#{out}/#{c["name"]}.json", JSON.pretty_generate(record) + "\n")
    puts "#{c["name"]}: #{responses.map { |r| r[:status] }.join(",")}, #{record[:changes].size} tables, #{from_requests.size} jobs"
  ensure
    # Stored files are not rolled back: the case's files go, and their
    # tombstone copies.
    (c["files"] || {}).each_key do |url|
      [url, url.sub("/uploads/", "/uploads/tombstone/")].each do |u|
        path = File.join(Rails.root, "public", u)
        File.delete(path) if File.file?(path)
      end
    end
    Array(record && record[:changes]&.dig("uploads", :inserted)).each do |u|
      path = File.join(Rails.root, "public", u["url"].to_s)
      File.delete(path) if u["url"].to_s.start_with?("/uploads/") && File.file?(path)
    end
    # Backups a case's requests or jobs wrote (uploaded chunks, merged
    # uploads) are not rolled back either.
    (backup_files - backups_before).sort.reverse.each { |p| File.directory?(p) ? Dir.rmdir(p) : File.delete(p) }
    # Nor are a case's objects.
    s3_empty(s3_buckets(c)) if c["s3"]
    RateLimiter.disable
    ENV_BEFORE.each { |k, v| ENV[k] = v }
    ENV_BEFORE.clear
    clear_redis_state
    pool.unpin_connection!
    # Always: a case's requests or jobs may set settings too
    # (SiteSetting.set_and_log), which the rollback undoes in the database
    # but not in the in-process cache.
    SiteSetting.refresh!
    # Always too: a case's requests may write rows these caches keep (the
    # admin watched words routes, custom emoji and the emoji deny list),
    # which the rollback removes.
    reset_plugin_caches
  end
end
