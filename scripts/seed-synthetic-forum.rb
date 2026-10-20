# frozen_string_literal: true

# Run with Rails runner in a dedicated development database. See docs/synthetic-forum.md.
require "json"
require "sidekiq/testing"

db = ActiveRecord::Base.connection_db_config.database
abort "requires development and DISCOURSE_DEV_DB=discourse_synthetic_*" unless
  Rails.env.development? && db.start_with?("discourse_synthetic_") && ENV["DISCOURSE_DEV_DB"] == db

count = Integer(ENV.fetch("SYNTHETIC_TOPICS", "200"), 10)
abort "SYNTHETIC_TOPICS must be between 12 and 10000" unless (12..10_000).cover?(count)
abort "dataset already exists; use a fresh database" if User.exists?(username: "synthetic_01")

# Model callbacks can enqueue jobs even when PostCreator uses skip_jobs.
Sidekiq::Testing.fake!
Scheduler::Defer.async = false
RateLimiter.disable
SiteSetting.disable_emails = "yes"
SiteSetting.max_oneboxes_per_post = 0
SiteSetting.download_remote_images_to_local = false
SiteSetting.tagging_enabled = true

bodies = [
  "## Reproducing the problem\n\n1. Open the dashboard.\n2. Change the filter.\n3. Reload.\n\nExpected **saved preferences**, but the filter resets.",
  "## Deployment notes\n\n```ruby\nsettings = { retries: 3, timeout: 15 }\nputs settings.fetch(:timeout)\n```\n\nThe `timeout` is measured in seconds.",
  "## Measurements\n\n| Scenario | Requests | Duration |\n|---|---:|---:|\n| Cold | 10 | 420 ms |\n| Warm | 100 | 35 ms |\n\nThese are synthetic measurements, not production results.",
  "## A multilingual discussion\n\nPříliš žluťoučký kůň úpěl ďábelské ódy.\n\nこんにちは世界。 مرحباً بالعالم.\n\nDoes searching for **žluťoučký** find this post?",
  "## Troubleshooting\n\n[details=Steps and observations]\nThe first request worked. The second returned an empty list.\n\n- [x] Check configuration\n- [ ] Retry after restart\n[/details]\n\nA possible explanation is [spoiler]stale cached state[/spoiler].",
  "## Rendering boundaries\n\nLiteral text: `<script>alert(1)</script>` and `a & b < c`.\n\n> A quoted observation with *emphasis*.\n\n---\n\nNested list:\n\n- First\n  - Nested\n- Second :slightly_smiling_face:",
]

manifest = { version: 1, topics: [], users: [], categories: {} }
ActiveRecord::Base.transaction do
  users = (1..40).map do |i|
    username = format("synthetic_%02d", i)
    User.create!(username: username, name: "Synthetic Member #{i}",
                 email: "#{username}@example.invalid", password: SecureRandom.hex(32),
                 active: true, approved: true, trust_level: i % 5,
                 admin: i == 1).tap { |u| u.email_tokens.update_all(confirmed: true) }
  end
  admin = users.first
  manifest[:users] = users.map { |u| { id: u.id, username: u.username } }
  group = Group.create!(name: "synthetic_members")
  group.add(users[1])
  categories = %w[Support Development Community].map do |name|
    Category.create!(name: "Synthetic #{name}", user: admin, color: "0088CC", text_color: "FFFFFF")
  end
  categories << Category.create!(name: "Synthetic Hosting", user: admin,
                                 parent_category_id: categories[1].id, color: "0088CC", text_color: "FFFFFF")
  private_category = Category.new(name: "Synthetic Restricted", user: admin, color: "CC0000", text_color: "FFFFFF")
  private_category.set_permissions(group.name => :full)
  private_category.save!
  manifest[:categories] = (categories + [private_category]).to_h { |c| [c.name, c.id] }
  %w[synthetic-help synthetic-howto synthetic-performance].each { |name| Tag.create!(name: name) }

  count.times do |i|
    restricted = i % 12 == 11
    author = restricted ? users[1] : users[i % users.size]
    at = Time.utc(2025, 1, 1, 9) + i.hours
    post = PostCreator.create!(author, title: "Synthetic #{%w[question guide discussion investigation][i % 4]} #{i + 1}",
                               raw: bodies[i % bodies.size], category: restricted ? private_category.id : categories[i % categories.size].id,
                               tags: ["synthetic-#{%w[help howto performance][i % 3]}"],
                               created_at: at, skip_validations: true, skip_jobs: true)
    topic = post.topic
    replies = i % 20 == 0 ? 100 : [0, 1, 3, 8, 15][i % 5]
    replies.times do |j|
      replier = restricted ? users[1] : users[(i + j + 1) % users.size]
      raw = "Observation #{j + 1}: #{bodies[(i + j + 1) % bodies.size]}"
      raw += "\n\n[quote=\"#{author.username}, post:1, topic:#{topic.id}\"]\n#{bodies[i % bodies.size]}\n[/quote]" if j % 7 == 0
      reply = PostCreator.create!(replier, topic_id: topic.id, raw: raw,
                                  reply_to_post_number: 1, created_at: at + (j + 1).minutes,
                                  skip_validations: true, skip_jobs: true)
      if j % 3 == 0 && replier.id != admin.id
        result = PostActionCreator.like(admin, reply)
        raise "like failed: #{result.errors.full_messages.join(', ')}" if result.errors.any?
      end
    end
    case i % 12
    when 1 then topic.update_status("closed", true, admin)
    when 2 then topic.update_status("archived", true, admin)
    when 3 then topic.update_status("visible", false, admin)
    when 4 then topic.update_pinned(true, false)
    end
    raise "post count mismatch for #{topic.id}" unless topic.posts.where(post_type: Post.types[:regular]).count == replies + 1
    manifest[:topics] << { id: topic.id, posts: topic.posts.count, regular_posts: replies + 1, restricted: restricted,
                           closed: topic.reload.closed, archived: topic.archived, visible: topic.visible }
  end
  raise "restricted category is public" if Guardian.new.can_see?(private_category)
  raise "member cannot see restricted category" unless Guardian.new(users[1]).can_see?(private_category)
end

output = ENV.fetch("SYNTHETIC_MANIFEST", "/tmp/synthetic-forum.json")
File.write(output, JSON.pretty_generate(manifest))
puts "Created #{manifest[:users].size} users, #{count} topics, #{manifest[:topics].sum { |t| t[:posts] }} posts; manifest: #{output}"
