# frozen_string_literal: true

require "json"
require "sidekiq/testing"

db = ActiveRecord::Base.connection_db_config.database
abort "requires isolated development database" unless Rails.env.development? &&
  db.start_with?("discourse_synthetic_") && ENV["DISCOURSE_DEV_DB"] == db
version = ActiveRecord::Base.connection.select_value("SHOW server_version_num").to_i
abort "requires PostgreSQL 18" unless (180000...190000).cover?(version)

$stdout.sync = true
Sidekiq::Testing.fake!
Scheduler::Defer.async = false
SiteSetting.disable_emails = "yes"
SiteSetting.max_oneboxes_per_post = 0
SiteSetting.download_remote_images_to_local = false

footer = /\n\n\[Original post on Meta\]\((https:\/\/meta\.discourse\.org\/[^\s)]+)\)\s*\z/
posts = Post.joins(:user).where("users.username ~ '^corpus_([0-9]+|scale_[0-9]+)$'")
mapping = posts.order(:id).pluck(:id, :raw).filter_map do |id, raw|
  match = footer.match(raw)
  { local_id: id, source: match[1] } if match
end
abort "no attribution footers found" if mapping.empty?
manifest = ENV.fetch("ATTRIBUTION_MANIFEST")
File.open(manifest, "wx") { |f| f.write(JSON.pretty_generate(database: db, posts: mapping)) }

started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
mapping.each_with_index do |entry, index|
  Post.transaction do
    post = Post.find(entry.fetch(:local_id))
    post.update_columns(raw: post.raw.sub(footer, ""))
    post.rebake!(skip_publish_rebaked_changes: true)
    SearchIndexer.index(post, force: true)
    indexed_raw = PostSearchData.find_by!(post_id: post.id).raw_data
    raise "stale attribution for #{post.id}" if post.cooked.include?("Original post on Meta") ||
      indexed_raw.include?("Original post on Meta")
    raise "empty cooked post #{post.id}" if post.cooked.blank?
  end
  if (index + 1) % 500 == 0
    puts({ cleaned: index + 1, total: mapping.size,
           elapsed_seconds: Process.clock_gettime(Process::CLOCK_MONOTONIC) - started }.to_json)
  end
end
ActiveRecord::Base.connection.execute("ANALYZE")
result = { cleaned: mapping.size, database: db,
           seconds: Process.clock_gettime(Process::CLOCK_MONOTONIC) - started,
           indexed: PostSearchData.where(post_id: mapping.map { |p| p[:local_id] }).count }
File.write(ENV.fetch("CLEANUP_RESULT"), JSON.pretty_generate(result))
puts result.to_json
