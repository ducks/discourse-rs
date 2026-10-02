# Records what Discourse does for write requests, for discourse-rs to be
# measured against (run by scripts/record-writes through `bin/rails runner`).
#
# Each case is a list of requests made by one user through an integration
# session. They run on a pinned connection inside a transaction that is
# rolled back, as Rails' transactional tests do, so the site is left as it
# was. Recorded per case: each response, every row the requests inserted,
# updated or deleted (rows as Postgres' to_jsonb writes them), and the jobs
# they enqueued.
#
# Before a case every id sequence is set to its table's max(id), so the
# rows a case inserts get the ids discourse-rs will give them.
require "json"
require "fileutils"

cases_file, out = ARGV
abort "usage: record_writes.rb <cases.json> <dir>" if cases_file.blank? || out.blank?
FileUtils.mkdir_p(out)

ENQUEUED = []
module Jobs
  def self.enqueue(name, args = {}) = (ENQUEUED << [name.to_s, args.except(:current_site_id)]) && nil
  def self.enqueue_in(delay, name, args = {}) =
    (ENQUEUED << ["#{name} in #{delay.to_i}s", args.except(:current_site_id)]) && nil
  def self.enqueue_at(at, name, args = {}) = (ENQUEUED << ["#{name} at", args.except(:current_site_id)]) && nil
end
ActionController::Base.allow_forgery_protection = false
RateLimiter.disable

def db = ActiveRecord::Base.connection

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
# and post body, which would refuse the same case recorded twice.
def clear_redis_state
  Discourse.redis.keys("unique-post-*").each { |k| Discourse.redis.del(k) }
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
cases.each do |c|
  ENQUEUED.clear
  pool.pin_connection!(true)
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
    ENQUEUED.clear
    before_sums = checksums
    before_rows = before_sums.keys.to_h { |t| [t, rows(t)] }
    started_at = db.select_value("SELECT to_jsonb(clock_timestamp()::timestamp)::text")
    responses =
      c["requests"].map do |r|
        session.public_send(r["method"].downcase, r["path"], params: r["params"] || {}, headers: headers, as: :json)
        body = session.response.body
        { status: session.response.status, body: (JSON.parse(body) rescue body) }
      end
    record = {
      name: c["name"],
      user: c["user"],
      requests: c["requests"],
      started_at: JSON.parse(started_at),
      responses: responses,
      changes: diff(before_sums, before_rows),
      jobs: ENQUEUED.dup,
    }
    File.write("#{out}/#{c["name"]}.json", JSON.pretty_generate(record) + "\n")
    puts "#{c["name"]}: #{responses.map { |r| r[:status] }.join(",")}, #{record[:changes].size} tables, #{ENQUEUED.size} jobs"
  ensure
    clear_redis_state
    pool.unpin_connection!
  end
end
