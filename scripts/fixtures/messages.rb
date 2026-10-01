# Fourth pass (2026-10-01): private messages and notifications for the logged-in slice
# Run inside the reference dv agent (dv copy + bin/rails runner) after
# scripts/fixtures/topics.rb. Idempotent: skips if the marker PM exists.
#
# Creates:
#   (a) PM user1 -> user0, one reply from user0
#   (b) PM admin -> user1, user2 (three participants, no groups)
#   (c) PM user1 -> group "staff": skipped, the staff group's messageable_level
#       is 0 (nobody) on the reference, so no group PM exists
#   (d) PM user2 -> user1, archived by user1 (user_archived_messages)
#   (e) a reply on topic 35 by user0 that @mentions user1, and a like by user0
#       on user1's reply in topic 35 (post 36), so user1 has mentioned/liked
#       notifications
#   (f) user1's "Basic" badge notification (id 2) marked read
#
# PostAlerter is called directly after each post so the notifications exist
# when the script ends (the post_alert job is also enqueued for Sidekiq;
# PostAlerter skips a second unread notification of the same type on the same
# post, so running both is safe). Jobs.run_immediately! was tried first and
# lost jobs silently ("can't alloc thread" in the container).

admin = User.find_by!(username: "admin")
u0, u1, u2 = %w[user0 user1 user2].map { |n| User.find_by!(username: n) }

marker = "Parity fixture: PM user1 to user0"

unless Topic.exists?(title: marker)
  def alerted(post)
    PostAlerter.post_created(post)
    post
  end

  def pm(user, title, raw, targets)
    alerted(
      PostCreator.create!(
        user,
        title: title,
        raw: raw,
        archetype: Archetype.private_message,
        target_usernames: targets,
        skip_validations: true,
      ),
    ).topic
  end

  pm_a = pm(u1, marker, "A private message from user1 to user0, long enough to be a real post body.", "user0")
  alerted(PostCreator.create!(u0, topic_id: pm_a.id, raw: "A reply from user0 inside the private message, long enough too.", skip_validations: true))

  pm_b = pm(admin, "Parity fixture: PM admin to user1 and user2", "A private message from admin to two users, so the list shows three participants.", "user1,user2")

  staff = Group.find(Group::AUTO_GROUPS[:staff])
  if Group.messageable(u1).exists?(id: staff.id)
    pm_c = pm(u1, "Parity fixture: PM user1 to staff", "A private message from user1 to the staff group.", "staff")
  else
    puts "staff group not messageable by user1 (messageable_level=#{staff.messageable_level}); skipping (c)"
  end

  pm_d = pm(u2, "Parity fixture: PM user2 to user1, archived", "A private message from user2 to user1 that user1 archives.", "user1")
  UserArchivedMessage.archive!(u1.id, pm_d)

  mention = alerted(PostCreator.create!(u0, topic_id: 35, raw: "Reply from user0 mentioning @user1 so a mention notification exists.", skip_validations: true))
  like = PostActionCreator.like(u0, Post.find(36))

  Notification.read(u1, [2])

  puts "pms: a=#{pm_a.id} b=#{pm_b.id} c=#{pm_c&.id.inspect} d=#{pm_d.id}"
  puts "mention post: #{mention.id} (topic 35 post_number #{mention.post_number}); like post_action: #{like.post_action&.id.inspect}"
end

puts "notifications: #{Notification.order(:id).where(user_id: [1, 2, 3, 4]).pluck(:id, :user_id, :notification_type, :read, :high_priority, :topic_id, :post_number).inspect}"
puts "user_archived_messages: #{UserArchivedMessage.pluck(:user_id, :topic_id).inspect}"
puts "topic_allowed_users: #{TopicAllowedUser.order(:topic_id, :user_id).pluck(:topic_id, :user_id).inspect}"
