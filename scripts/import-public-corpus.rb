# frozen_string_literal: true

require "json"
require "sidekiq/testing"

db = ActiveRecord::Base.connection_db_config.database
abort "requires isolated development database" unless Rails.env.development? &&
  db.start_with?("discourse_synthetic_") && ENV["DISCOURSE_DEV_DB"] == db
corpus = JSON.parse(File.read(ARGV.fetch(0)))
abort "expected anonymous Meta MCP corpus" unless corpus.values_at("site", "access", "transport") ==
  ["https://meta.discourse.org", "anonymous", "mcp-stdio"]
abort "corpus is empty" if corpus.fetch("topics").empty?
abort "already imported; use a fresh database" if User.exists?(username: "corpus_1")

Sidekiq::Testing.fake!
Scheduler::Defer.async = false
RateLimiter.disable
SiteSetting.disable_emails = "yes"
SiteSetting.max_oneboxes_per_post = 0
SiteSetting.download_remote_images_to_local = false
SiteSetting.tagging_enabled = true
SiteSetting.duplicate_topic_titles = "allowed"
# Import event markup as content, without recreating events or invitees.
SiteSetting.discourse_post_event_enabled = false if SiteSetting.respond_to?(:discourse_post_event_enabled=)

manifest = { source: corpus.fetch("site"), access: "anonymous", topics: [], users: [] }
ActiveRecord::Base.transaction do
  authors = corpus.fetch("topics").flat_map { |t| t.fetch("posts").map { |p| p.fetch("username") } }.uniq.sort
  users = authors.each_with_index.to_h do |author, i|
    user = User.create!(username: "corpus_#{i + 1}", name: "Corpus Member #{i + 1}",
                        email: "corpus_#{i + 1}@example.invalid", password: SecureRandom.hex(32),
                        active: true, approved: true, trust_level: 2)
    user.email_tokens.update_all(confirmed: true)
    manifest[:users] << { source_username: author, local_id: user.id, local_username: user.username }
    [author, user]
  end
  definitions = corpus.fetch("categories").index_by { |c| c.fetch("id") }
  categories = {}
  create_category = lambda do |id|
    return categories[id] if categories.key?(id)
    definition = definitions.fetch(id)
    raise "restricted source category" if definition.fetch("read_restricted")
    parent_id = definition["parent_category_id"]
    parent = parent_id ? create_category.call(parent_id) : nil
    categories[id] = Category.create!(name: "Meta #{definition.fetch('name')}", user: Discourse.system_user,
                                      parent_category_id: parent&.id, color: "0088CC", text_color: "FFFFFF")
  end
  corpus.fetch("topics").each { |t| create_category.call(t.fetch("category_id")) }
  corpus.fetch("topics").each do |source|
    posts = source.fetch("posts").sort_by { |p| p.fetch("post_number") }
    raise "missing opening post: #{source['id']}" unless posts.first.fetch("post_number") == 1
    topic = nil
    numbers = {}
    mapping = []
    posts.each do |post|
      raise "incomplete or non-public post" if post["truncated"] || post["hidden"] || post["deleted_at"] || post["post_type"] != 1
      # Source IDs must not turn into links to unrelated local topics. Keep the
      # quoted text/author; source links live in the manifest, outside the index.
      raw = post.fetch("raw").gsub(/\[quote="([^"]*)"\]/) do
        header = Regexp.last_match(1).gsub(/,\s*(?:post|topic):\d+/, "")
        "[quote=\"#{header}\"]"
      end
      # The pinned event validator still parses BBCode with events disabled.
      # Render its delimiters literally instead of creating plugin records.
      raw = raw.gsub(/\[(\/?event)(?=[\s\]])/i, '&#91;\1')
      args = { raw: raw, created_at: Time.iso8601(post.fetch("created_at")),
               skip_validations: true, skip_jobs: true }
      if topic
        args[:topic_id] = topic.id
        args[:reply_to_post_number] = numbers[post["reply_to_post_number"]]
      else
        args[:title] = source.fetch("title")
        args[:category] = categories.fetch(source.fetch("category_id")).id
      end
      created = PostCreator.create!(users.fetch(post.fetch("username")), **args)
      topic ||= created.topic
      numbers[post.fetch("post_number")] = created.post_number
      mapping << { source_id: post.fetch("id"), source_post_number: post.fetch("post_number"),
                   local_id: created.id, local_post_number: created.post_number,
                   source: "#{source.fetch('source')}/#{post.fetch('post_number')}" }
    end
    names = source.fetch("tags", []).map { |tag| tag.is_a?(Hash) ? tag.fetch("name") : tag }
    if names.any?
      DiscourseTagging.tag_topic_by_names(topic, Guardian.new(Discourse.system_user), names)
      topic.save!
    end
    raise "post count mismatch" unless topic.posts.where(post_type: Post.types[:regular]).count == posts.size
    manifest[:topics] << { source_id: source.fetch("id"), local_id: topic.id,
                           source: source.fetch("source"), sampled_posts: mapping,
                           source_posts_count: source.fetch("posts_count"), strata: source.fetch("strata") }
    puts "Imported source topic #{source.fetch('id')}: #{posts.size} posts"
  end
end
File.write(ENV.fetch("SYNTHETIC_MANIFEST"), JSON.pretty_generate(manifest))
puts "Imported #{manifest[:topics].size} public topics from #{manifest[:users].size} authors"
