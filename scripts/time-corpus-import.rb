# frozen_string_literal: true

# Rails runner microbenchmark against a disposable copy of the public corpus.
require "json"
require "sidekiq/testing"

db = ActiveRecord::Base.connection_db_config.database
abort "requires isolated development database" unless Rails.env.development? &&
  db.start_with?("discourse_synthetic_") && ENV["DISCOURSE_DEV_DB"] == db
abort "already timed; use a fresh database" if User.exists?(username: "import_timer_1")

Sidekiq::Testing.fake!
Scheduler::Defer.async = false
RateLimiter.disable
SiteSetting.disable_emails = "yes"
SiteSetting.max_oneboxes_per_post = 0
SiteSetting.download_remote_images_to_local = false
SiteSetting.discourse_post_event_enabled = false if SiteSetting.respond_to?(:discourse_post_event_enabled=)

source = Post.joins(:user).where("users.username LIKE 'corpus_%'").order(:id).pluck(:raw)
abort "public corpus missing" if source.empty?
clock = -> { Process.clock_gettime(Process::CLOCK_MONOTONIC) }
started = clock.call
users = 40.times.map do |i|
  User.create!(username: "import_timer_#{i + 1}", email: "import_timer_#{i + 1}@example.invalid",
               password: SecureRandom.hex(32), active: true, approved: true, trust_level: 2)
end
users_seconds = clock.call - started
category = Category.create!(name: "Import timing", user: Discourse.system_user, color: "0088CC", text_color: "FFFFFF")
ids = []
batches = []
created_at = Time.utc(2025, 1, 1)
posts_started = clock.call
50.times do |i|
  topic = nil
  20.times do |j|
    n = i * 20 + j
    raw = source[n % source.size].gsub(/\[quote="([^"]*)"\]/) do
      "[quote=\"#{Regexp.last_match(1).gsub(/,\s*(?:post|topic):\d+/, '')}\"]"
    end
    args = { raw: "Synthetic timing sample #{n + 1}\n\n#{raw}", created_at: created_at + n.minutes,
             skip_validations: true, skip_jobs: true }
    if topic
      args[:topic_id] = topic.id
      args[:reply_to_post_number] = 1
    else
      args[:title] = "Synthetic import timing topic #{i + 1}"
      args[:category] = category.id
    end
    post = PostCreator.create!(users[n % users.size], **args)
    topic ||= post.topic
    ids << post.id
  end
  if (i + 1) % 5 == 0
    batches << { posts: ids.size, elapsed_seconds: clock.call - posts_started }
    puts batches.last.to_json
    $stdout.flush
  end
end
seconds = clock.call - posts_started
indexed = PostSearchData.where(post_id: ids).count
empty_cooked = Post.where(id: ids).where("cooked IS NULL OR cooked = ''").count
raise "incomplete cooking or indexing" unless indexed == 1000 && empty_cooked.zero?
result = { users: users.size, users_seconds: users_seconds, topics: 50, posts: ids.size,
           posts_seconds: seconds, posts_per_second: ids.size / seconds,
           total_seconds: clock.call - started, indexed_posts: indexed, empty_cooked: empty_cooked,
           batches: batches, background_jobs: "fake", transaction: "per PostCreator call" }
File.write(ENV.fetch("TIMING_OUTPUT"), JSON.pretty_generate(result))
puts result.to_json
