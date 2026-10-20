# frozen_string_literal: true

# Expand a disposable public corpus through real cooking and indexing.
require "json"
require "sidekiq/testing"

db = ActiveRecord::Base.connection_db_config.database
abort "requires isolated development database" unless Rails.env.development? &&
  db.start_with?("discourse_synthetic_") && ENV["DISCOURSE_DEV_DB"] == db
version = ActiveRecord::Base.connection.select_value("SHOW server_version_num").to_i
abort "requires PostgreSQL 18" unless (180000...190000).cover?(version)
abort "already expanded; use a fresh database" if User.exists?(username: "corpus_scale_1")
$stdout.sync = true

Sidekiq::Testing.fake!
Scheduler::Defer.async = false
RateLimiter.disable
SiteSetting.disable_emails = "yes"
SiteSetting.max_oneboxes_per_post = 0
SiteSetting.download_remote_images_to_local = false
SiteSetting.discourse_post_event_enabled = false if SiteSetting.respond_to?(:discourse_post_event_enabled=)

source = Post.joins(:user).where("users.username ~ '^corpus_[0-9]+$'").order(:id).pluck(:id, :raw)
abort "public corpus missing" if source.empty?
clock = -> { Process.clock_gettime(Process::CLOCK_MONOTONIC) }
started = clock.call
users = 40.times.map do |i|
  User.create!(username: "corpus_scale_#{i + 1}", email: "corpus_scale_#{i + 1}@example.invalid",
               password: SecureRandom.hex(32), active: true, approved: true, trust_level: 2)
end
users_seconds = clock.call - started
categories = %w[Support Development Hosting Community].map do |name|
  Category.create!(name: "Corpus #{name}", user: Discourse.system_user, color: "0088CC", text_color: "FFFFFF")
end
ids = []
source_mapping = []
batches = []
created_at = Time.utc(2025, 1, 1)
posts_started = clock.call
# 1,000 topics and 20,000 posts: mostly short threads, plus 100 long threads.
lengths = [5] * 400 + [10] * 300 + [25] * 200 + [100] * 100
lengths = lengths.shuffle(random: Random.new(20261019))
lengths.each_with_index do |length, i|
  topic = nil
  length.times do |j|
    n = ids.size
    source_id, source_raw = source[n % source.size]
    source_raw = source_raw.sub(/\n\n\[Original post on Meta\]\(https:\/\/meta\.discourse\.org\/[^\s)]+\)\s*\z/, "")
    raw = source_raw.gsub(/\[quote="([^"]*)"\]/) do
      "[quote=\"#{Regexp.last_match(1).gsub(/,\s*(?:post|topic):\d+/, '')}\"]"
    end
    args = { raw: "Synthetic corpus sample #{n + 1}\n\n#{raw}", created_at: created_at + n.minutes,
             skip_validations: true, skip_jobs: true }
    if topic
      args[:topic_id] = topic.id
      args[:reply_to_post_number] = 1
    else
      args[:title] = "Corpus #{%w[deployment search configuration troubleshooting][i % 4]} discussion #{i + 1}"
      args[:category] = categories[i % categories.size].id
    end
    post = PostCreator.create!(users[n % users.size], **args)
    topic ||= post.topic
    ids << post.id
    source_mapping << { local_id: post.id, source_local_id: source_id }
  end
  if (i + 1) % 5 == 0
    batches << { topics: i + 1, posts: ids.size, elapsed_seconds: clock.call - posts_started }
    puts batches.last.to_json
    $stdout.flush
  end
end
seconds = clock.call - posts_started
indexed = PostSearchData.where(post_id: ids).count
empty_cooked = Post.where(id: ids).where("cooked IS NULL OR cooked = ''").count
raise "incomplete cooking or indexing" unless ids.size == 20_000 && indexed == ids.size && empty_cooked.zero?
ActiveRecord::Base.connection.execute("ANALYZE")
result = { users: users.size, users_seconds: users_seconds, topics: lengths.size, posts: ids.size,
           source_mapping: source_mapping,
           thread_lengths: lengths.tally, database: db, postgres_version_num: version,
           posts_seconds: seconds, posts_per_second: ids.size / seconds,
           total_seconds: clock.call - started, indexed_posts: indexed, empty_cooked: empty_cooked,
           batches: batches, background_jobs: "fake", transaction: "per PostCreator call" }
File.write(ENV.fetch("TIMING_OUTPUT"), JSON.pretty_generate(result))
puts result.except(:source_mapping).to_json
