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
ActionController::Base.allow_forgery_protection = false
ActionMailer::Base.delivery_method = :test
ActionMailer::Base.perform_deliveries = true
RateLimiter.disable

def db = ActiveRecord::Base.connection

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
    value.gsub(/\{\{([^}|]+)(\|reverse|\|last_segment)?\}\}/) do
      path, filter = Regexp.last_match(1), Regexp.last_match(2)
      found = path.split(".").reduce(state) { |acc, k| acc.is_a?(Array) ? acc[k.to_i] : acc&.[](k) }
      raise "{{#{path}}} resolved to nothing" if found.nil?
      case filter
      when "|reverse" then found.to_s.reverse
      when "|last_segment" then found.to_s.split("/").last
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
  # directory_*: when the reference's own scheduler last refreshed the
  # user directory (DirectoryItem.last_updated_at, DirectoryRefresh).
  %w[unique-post-* unique-pm-post-* rejection_email:* user-last-seen:* hot_score_6_hourly __SEARCH__LOG_* LAST_UPLOAD_CLEANUP l-rate-limit3:* grpmsg:* last_notified_reviewable_id _CACHE:inline_onebox:* _CACHE:onebox__* _CACHE:onebox_failed__* about-stats badge_queue directory_*].each do |pattern|
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
  Category.reset_voting_cache if Category.respond_to?(:reset_voting_cache)
  # discourse-solved's in-memory list of categories that take answers.
  DiscourseSolved::AcceptedAnswerCache.reset_accepted_answer_cache if defined?(DiscourseSolved::AcceptedAnswerCache)
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
if cases.any? { |c| c["remote"] }
  # "::" takes IPv4 too.
  server = TCPServer.new("::", 8099)
  Thread.new do
    loop do
      Thread.new(server.accept) do |s|
        head = +""
        head << s.readpartial(4096) until head.include?("\r\n\r\n")
        method, target = head.lines.first.to_s.split(" ")
        page = REMOTE[target] || {}
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

cases.each do |c|
  ENQUEUED.clear
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
    end
    # A case's `settings` are set inside the transaction, so they roll back
    # with it (the in-process cache is refreshed after).
    (c["settings"] || {}).each { |name, value| SiteSetting.set(name, value) }
    REMOTE.replace(c["remote"] || {})
    # `files`: fixture files stored at upload urls, for rows the setup
    # inserts (they are not rolled back: removed after the case).
    (c["files"] || {}).each do |url, name|
      path = File.join(Rails.root, "public", url)
      FileUtils.mkdir_p(File.dirname(path))
      FileUtils.cp(File.join(files_dir, name), path)
    end
    # `setup`: SQL for fixture rows a case needs (a reply key), run the same way.
    (c["setup"] || []).each { |sql| db.execute(sql) }
    reset_plugin_caches if c["setup"]
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
    c["requests"].each do |r|
        state = { "responses" => responses.map { |x| JSON.parse(x.to_json) }, "jobs" => ENQUEUED.to_h { |n, a| [n.split(" ").first, JSON.parse(a.to_json)] } }
        # A GET with `as: :json` goes out as a POST with X-Http-Method-Override,
        # which the integration session writes into the headers it was given.
        params = fixtures(resolve(r["params"] || {}, state), files_dir)
        options = { params: params, headers: headers.merge(r["headers"] || {}) }
        options[:as] = :json if r["method"] != "GET" && !multipart?(params)
        path = resolve(r["path"], state)
        # Unicorn sets REQUEST_PATH; log_on_user logs it as the token's path.
        options[:env] = { "REQUEST_PATH" => path.split("?").first }
        session.public_send(r["method"].downcase, path, **options)
        body = session.response.body
        response = { status: session.response.status, body: (JSON.parse(body) rescue body) }
        # The rate limit headers, kept only when a response has them, and
        # those the case names in `keep_headers` (a redirect's Location, as
        # its path, the host being each side's own).
        kept = session.response.headers.to_h.slice("Retry-After", "Discourse-Rate-Limit-Error-Code", "Discourse-Actions-Remaining", "Discourse-Actions-Max", *(c["keep_headers"] || []))
        kept["Location"] = URI(kept["Location"]).request_uri if kept["Location"]
        response[:headers] = kept if kept.any?
        responses << response
      end
    # run_jobs: the named jobs run in order (delayed ones too), as Sidekiq
    # would, including those the jobs themselves enqueue; what the jobs
    # enqueue is recorded apart, and mail is captured, not sent.
    from_requests = ENQUEUED.dup
    ENQUEUED.clear
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
    emails =
      ActionMailer::Base.deliveries.map do |m|
        {
          headers: m.header.fields.reject { |f| %w[Date].include?(f.name) }.map { |f| [f.name, f.value.to_s] },
          text: m.text_part&.decoded || (m.multipart? ? nil : m.decoded),
          html: m.html_part&.decoded,
        }
      end
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
    record[:enqueue] = c["enqueue"] if c["enqueue"]
    record[:include_tables] = c["include_tables"] if c["include_tables"]
    record[:approximate] = c["approximate"] if c["approximate"]
    record[:unordered] = c["unordered"] if c["unordered"]
    record[:unordered_responses] = c["unordered_responses"] if c["unordered_responses"]
    record[:job_clock] = c["job_clock"] if c["job_clock"]
    record[:files] = c["files"] if c["files"]
    record[:remote] = c["remote"] if c["remote"]
    record[:rate_limits] = c["rate_limits"] if c["rate_limits"]
    record[:keep_headers] = c["keep_headers"] if c["keep_headers"]
    record[:fragment] = c["fragment"] if c["fragment"]
    record[:jobs_from_jobs] = ENQUEUED.dup if c["run_jobs"]
    record[:emails] = emails if emails.any?
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
    RateLimiter.disable
    clear_redis_state
    pool.unpin_connection!
    # Always: a case's requests or jobs may set settings too
    # (SiteSetting.set_and_log), which the rollback undoes in the database
    # but not in the in-process cache.
    SiteSetting.refresh!
    reset_plugin_caches if c["setup"]
  end
end
