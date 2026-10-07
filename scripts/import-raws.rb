# frozen_string_literal: true
# Build a forum from raw posts (backups/meta/raws.json: id, source
# "https://<site>/t/<topic>/<post>", raw) through PostCreator, so Rails
# cooks everything as it would. The authors, titles, categories, dates
# and likes are made up: the raws carry none of them. Nothing is fetched
# from the internet (oneboxes and remote images off, jobs not run).
#
# Run in a scratch dv agent, never the parity reference:
#   dv copy backups/meta/raws.json rs-meta:/tmp/raws.json
#   dv copy scripts/import-raws.rb rs-meta:/tmp/import-raws.rb
#   dv run --name rs-meta -- bin/rails runner /tmp/import-raws.rb /tmp/raws.json

path = ARGV[0] or abort "usage: import-raws.rb <raws.json>"
raws = JSON.parse(File.read(path))

USERS = 40
CATEGORIES = %w[Support Bug Feature UX Dev Plugin Hosting Community]
BASE_TIME = Time.utc(2025, 1, 6, 9)

RateLimiter.disable
SiteSetting.max_oneboxes_per_post = 0
SiteSetting.download_remote_images_to_local = false
SiteSetting.max_image_size_kb = 102_400
SiteSetting.min_post_length = 1
SiteSetting.min_first_post_length = 1
SiteSetting.min_topic_title_length = 1
SiteSetting.title_min_entropy = 0
SiteSetting.duplicate_topic_titles = "allowed"
SiteSetting.max_topics_per_day = 100_000
SiteSetting.max_replies_in_first_day = 100_000
SiteSetting.newuser_max_replies_per_topic = 100_000

users =
  (1..USERS).map do |i|
    username = "meta_user_#{i}"
    User.find_by(username: username) ||
      User
        .create!(
          username: username,
          name: "Meta User #{i}",
          email: "#{username}@example.com",
          password: "a-long-password-#{i}",
          active: true,
          approved: true,
          trust_level: [1, 1, 2, 2, 3, 4][i % 6],
        )
        .tap { |u| u.email_tokens.update_all(confirmed: true) }
  end

categories =
  CATEGORIES.map do |name|
    Category.find_by(name: name) ||
      Category.create!(name: name, user: Discourse.system_user, color: "0088CC", text_color: "FFFFFF")
  end

def title_for(raw, topic)
  line =
    raw.each_line.map(&:strip).find { |l| l.match?(/\A#+\s+\S/) } ||
      raw.each_line.map(&:strip).find { |l| l.present? && !l.start_with?("![", "<", "[quote", "|") } ||
      "Topic"
  text = line.sub(/\A#+\s*/, "").gsub(/[*_`>\[\]]/, "").gsub(/\(http[^)]*\)/, "").squish
  "#{text.truncate(80, separator: " ", omission: "")} (#{topic})"
end

by_topic =
  raws
    .map { |r| r.merge("t" => r["source"][%r{/t/(\d+)/}, 1].to_i, "p" => r["source"][%r{/(\d+)\z}, 1].to_i) }
    .group_by { |r| r["t"] }
    .sort_by { |t, _| t }

created = 0
by_topic.each_with_index do |(source_topic, posts), ti|
  # Resumable: a topic already imported (its title ends in the source
  # topic) is skipped whole.
  next if Topic.where("title LIKE ?", "% (#{source_topic})").exists?
  posts = posts.sort_by { |r| r["p"] }
  topic_at = BASE_TIME + ti.days
  first = posts.first
  op = users[source_topic % USERS]
  creator =
    PostCreator.new(
      op,
      raw: first["raw"],
      title: title_for(first["raw"], source_topic),
      category: categories[ti % categories.size].id,
      created_at: topic_at,
      skip_validations: true,
      skip_jobs: true,
    )
  post = creator.create
  if post.blank? || post.errors.any?
    puts "skip topic #{source_topic}: #{creator.errors.full_messages.join(", ")}"
    next
  end
  created += 1
  topic = post.topic
  posts.drop(1).each_with_index do |r, pi|
    author = users[(source_topic + r["p"] * 7) % USERS]
    reply =
      PostCreator.create(
        author,
        raw: r["raw"],
        topic_id: topic.id,
        created_at: topic_at + (pi + 1).hours,
        skip_validations: true,
        skip_jobs: true,
      )
    if reply.blank? || reply.errors.any?
      puts "skip post #{r["source"]}"
      next
    end
    created += 1
    # A few likes, from users other than the author.
    (r["p"] % 4).times do |k|
      liker = users[(source_topic * 3 + r["p"] + k * 11) % USERS]
      next if liker.id == author.id
      PostActionCreator.like(liker, reply)
    end
  end
end

Sidekiq::Queue.all.each(&:clear) if defined?(Sidekiq::Queue)
puts "created #{created} posts in #{Topic.where(archetype: "regular").count} topics"
