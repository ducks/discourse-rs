# Fifth pass (2026-10-02): bookmarks for /u/:username/bookmarks.json and
# /u/:username/user-menu-bookmarks.json.
# Run inside the reference dv agent (dv copy + bin/rails runner) after
# scripts/fixtures/messages.rb. Idempotent: skips if the marker bookmark
# exists.
#
# user1 already has one unnamed post bookmark (topic 37's first post, from
# the third pass). Creates, through BookmarkManager so topic_users.bookmarked
# follows:
#   (a) user1: a topic bookmark on the replies-and-posters topic, named
#   (b) user1: a post bookmark on their own reply there, named, with a
#       reminder far in the future (reminder_at and the two ics keys)
#   (c) user1: a pinned post bookmark on user0's reply in the user1/user0 PM
#   (d) user1: a post bookmark on user0's mention reply whose reminder has
#       fired: BookmarkReminderNotificationHandler leaves an unread
#       bookmark_reminder notification and clears reminder_at
#   (e) admin: a post bookmark in the staff category and a topic bookmark on
#       the admin/user1/user2 PM
# user0 keeps no bookmarks (the empty document).

admin = User.find_by!(username: "admin")
u0, u1 = %w[user0 user1].map { |n| User.find_by!(username: n) }

marker = "Parity fixture: topic bookmark"

unless Bookmark.exists?(user_id: u1.id, name: marker)
  def bookmark!(user, bookmarkable, name: nil, reminder_at: nil)
    manager = BookmarkManager.new(user)
    bm =
      manager.create_for(
        bookmarkable_id: bookmarkable.id,
        bookmarkable_type: bookmarkable.class.name,
        name: name,
        reminder_at: reminder_at,
      )
    raise "bookmark on #{bookmarkable.class.name} #{bookmarkable.id}: #{manager.errors.full_messages}" if manager.errors.any?
    bm
  end

  replies = Topic.find_by!(title: "Parity fixture: replies and posters")
  pm = Topic.find_by!(title: "Parity fixture: PM user1 to user0")
  pm_admin = Topic.find_by!(title: "Parity fixture: PM admin to user1 and user2")
  staff_topic = Topic.where(category_id: Category.find_by!(slug: "staff").id).order(:id).first!

  a = bookmark!(u1, replies, name: marker)
  own_reply = replies.posts.where(user_id: u1.id).order(:post_number).first!
  b = bookmark!(u1, own_reply, name: "Parity fixture: with a reminder", reminder_at: Time.utc(2031, 1, 15, 9, 30))
  c = bookmark!(u1, pm.posts.find_by!(user_id: u0.id), name: "Parity fixture: pinned, in a PM")
  BookmarkManager.new(u1).toggle_pin(bookmark_id: c.id)
  mention = replies.posts.where(user_id: u0.id).order(:post_number).last!
  d = bookmark!(u1, mention, name: "Parity fixture: reminder sent", reminder_at: 1.hour.from_now)
  d.update_columns(reminder_at: 1.hour.ago)
  BookmarkReminderNotificationHandler.new(d.reload).send_notification

  e1 = bookmark!(admin, staff_topic.first_post)
  e2 = bookmark!(admin, pm_admin, name: "Parity fixture: admin's PM bookmark")

  puts "bookmarks: a=#{a.id} b=#{b.id} c=#{c.id} d=#{d.id} e1=#{e1.id} e2=#{e2.id}"
  puts "reminder notification: #{Notification.where(user_id: u1.id, notification_type: Notification.types[:bookmark_reminder]).pluck(:id, :data)}"
else
  puts "bookmark fixtures already present"
end
