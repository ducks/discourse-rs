# Run inside the reference dv agent (dv copy + bin/rails runner) before
# scripts/snapshot-dv, to give topic lists something to compare.
# Fixture topics for parity testing; idempotent-ish (skips if marker topic exists).
unless Topic.exists?(title: "Parity fixture: replies and posters")

admin = User.find_by!(username: "admin")
u0, u1, u2 = %w[user0 user1 user2].map { |n| User.find_by!(username: n) }
general = Category.find_by!(name: "General")
feedback = Category.find_by!(name: "Site Feedback")

sub = Category.create!(name: "Sub General", slug: "sub-general", user: admin, parent_category_id: general.id, color: "AB9364", text_color: "FFFFFF")

def mk(user, title, raw, **opts)
  PostCreator.create!(user, title: title, raw: raw, skip_validations: true, **opts)
end

t1 = mk(u0, "Parity fixture: replies and posters", "First post of a topic with several repliers, long enough to be a real post body for the excerpt.", category: general.id).topic
PostCreator.create!(u1, topic_id: t1.id, raw: "Reply one from user1, also long enough to pass the minimum length check.", skip_validations: true)
PostCreator.create!(u2, topic_id: t1.id, raw: "Reply two from user2, also long enough to pass the minimum length check.", skip_validations: true)
PostCreator.create!(admin, topic_id: t1.id, raw: "Reply three from admin, also long enough to pass the minimum length check.", skip_validations: true)

t2 = mk(admin, "Parity fixture: pinned and closed", "A pinned, closed topic in Site Feedback with an excerpt that should be shown in the list.", category: feedback.id).topic
t2.update_pinned(true, false)
t2.update_status("closed", true, admin)

tag = Tag.create!(name: "howto")
t3 = mk(u1, "Parity fixture: tagged in a subcategory", "A topic in a subcategory carrying a tag, so tags and category ids show up.", category: sub.id, tags: [tag.name]).topic

t4 = mk(admin, "Parity fixture: unlisted topic", "This topic is unlisted and must not appear in latest for anonymous users.", category: general.id).topic
t4.update_status("visible", false, admin)

t5 = mk(u2, "Parity fixture: deleted topic", "This topic is deleted and must not appear anywhere.", category: general.id).topic
PostDestroyer.new(admin, t5.first_post).destroy

t6 = mk(u0, "Parity fixture: liked and archived", "An archived topic whose first post got a like, for like_count and op_like_count.", category: general.id).topic
PostActionCreator.like(u1, t6.first_post)
t6.update_status("archived", true, admin)

puts "created topics #{[t1, t2, t3, t4, t5, t6].map(&:id).inspect}, category #{sub.id}, tag #{tag.id}"
end
admin = User.find_by!(username: "admin")

# Second pass (2026-09-30): more tag coverage. Idempotent.
guide = Tag.find_or_create_by!(name: "guide")
{ 35 => %w[guide], 41 => %w[howto guide] }.each do |topic_id, names|
  topic = Topic.find(topic_id)
  DiscourseTagging.tag_topic_by_names(topic, Guardian.new(admin), (topic.tags.pluck(:name) + names).uniq)
  topic.save!
end
puts "tags: #{Tag.pluck(:name, :public_topic_count).inspect}"
