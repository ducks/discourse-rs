# Records what Discourse does for write requests, for discourse-rs to be
# measured against (run by scripts/record-writes through `bin/rails runner`).
#
# Each case is a list of requests made by one user through an integration
# session. They run on a pinned connection inside a transaction that is
# rolled back, as Rails' transactional tests do, so the site is left as it
# was. Recorded per case: each response, every row the requests inserted,
# updated or deleted (rows as Postgres' to_jsonb writes them), and the jobs
# they enqueued. A case with `run_jobs` also runs those of its enqueued
# jobs, and records what they enqueue in turn (`jobs_from_jobs`).
#
# Before a case every id sequence is set to its table's max(id), so the
# rows a case inserts get the ids discourse-rs will give them.
require "json"
require "fileutils"

cases_file, out, files_dir = ARGV
abort "usage: record_writes.rb <cases.json> <dir> [files-dir]" if cases_file.blank? || out.blank?
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

# Written by the agent's own scheduler while a case runs, not by the case.
BACKGROUND_TABLES = %w[scheduler_stats top_topics]

# Redis is not rolled back: the duplicate post check keeps a key per user
# and post body, and Email::Processor one per address and rejection a day,
# which would change the same case recorded twice.
def clear_redis_state
  # user-last-seen: UserStat.update_time_read!, which would count the
  # seconds since the previous case read a topic.
  %w[unique-post-* rejection_email:* user-last-seen:*].each do |pattern|
    Discourse.redis.keys(pattern).each { |k| Discourse.redis.del(k) }
  end
end

def checksums
  (TABLES.keys - BACKGROUND_TABLES).to_h do |t|
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
    # `setup`: SQL for fixture rows a case needs (a reply key), run the same way.
    (c["setup"] || []).each { |sql| db.execute(sql) }
    ENQUEUED.clear
    before_sums = checksums
    before_rows = before_sums.keys.to_h { |t| [t, rows(t)] }
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
        session.public_send(r["method"].downcase, resolve(r["path"], state), **options)
        body = session.response.body
        responses << { status: session.response.status, body: (JSON.parse(body) rescue body) }
      end
    # run_jobs: the named jobs run in order (delayed ones too), as Sidekiq
    # would, including those the jobs themselves enqueue; what the jobs
    # enqueue is recorded apart, and mail is captured, not sent.
    from_requests = ENQUEUED.dup
    ENQUEUED.clear
    ActionMailer::Base.deliveries.clear
    names = c["run_jobs"] || []
    pending = from_requests.dup
    while (job = pending.shift)
      name, args = job
      base = name.split(" ").first
      next if !names.include?(base)
      seen = ENQUEUED.size
      "Jobs::#{base.camelize}".constantize.new.execute(args.with_indifferent_access)
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
    record[:settings] = c["settings"] if c["settings"]
    record[:setup] = c["setup"] if c["setup"]
    record[:jobs_from_jobs] = ENQUEUED.dup if c["run_jobs"]
    record[:emails] = emails if emails.any?
    File.write("#{out}/#{c["name"]}.json", JSON.pretty_generate(record) + "\n")
    puts "#{c["name"]}: #{responses.map { |r| r[:status] }.join(",")}, #{record[:changes].size} tables, #{from_requests.size} jobs"
  ensure
    # Stored files are not rolled back: the case's uploads go.
    Array(record && record[:changes]&.dig("uploads", :inserted)).each do |u|
      path = File.join(Rails.root, "public", u["url"].to_s)
      File.delete(path) if u["url"].to_s.start_with?("/uploads/") && File.file?(path)
    end
    clear_redis_state
    pool.unpin_connection!
    SiteSetting.refresh! if c["settings"]
  end
end
