# Sixth pass (2026-11-15): the state rs-parity's seed carried beyond the
# fixture passes, from browsing and its scheduler, made explicitly so a
# new reference agent reproduces it. Run inside the reference dv agent
# (dv copy + bin/rails runner) after scripts/fixtures/bookmarks.rb, with
# Sidekiq still running, then apply scripts/reference-env and snapshot.
#
#   - login_required stored off (it was turned on and off) and
#     bypass_wizard_check on
#   - TopTopic.refresh!, which the top lists' goldens need
#   - user2 demoted from trust level 3 by Tl3Promotions
#   - the first visits of user0, user1 and admin (last and first seen,
#     visit rows, chat's auto join) and the IPs the seed keeps for them
#   - Jobs::ReindexSearch, so the fixture posts carry the current search
#     index version (search refuses blurbs from older ones)
#   - user0's timezone, America/Denver
Jobs.run_immediately!

SiteSetting.login_required = true
SiteSetting.login_required = false
SiteSetting.bypass_wizard_check = true
TopTopic.refresh!

u2 = User.find_by!(username: "user2")
Promotion.new(u2).change_trust_level!(TrustLevel[2]) if u2.trust_level == TrustLevel[3]
%w[user0 user1 admin].each do |name|
  User.find_by!(username: name).update_last_seen!(Time.zone.now, force: true)
end
User.where(username: %w[user0 user1]).update_all(ip_address: "172.17.0.1")
User.where(username: "admin").update_all(ip_address: "127.0.0.1")

Jobs::ReindexSearch.new.execute({})
User.find_by!(username: "user0").user_option.update!(timezone: "America/Denver")

puts User.where("id > 0").order(:id).pluck(:id, :username, :trust_level, :ip_address).inspect
puts PostSearchData.group(:version).count.inspect

# Seventh pass (2026-11-21): what the scheduler had done on rs-parity-20261115
# before its snapshot, run here so the seed does not depend on how long
# Sidekiq ran first: badges and their notifications, automatic groups,
# featured topics, the directory (whose refresh creates the plugin columns),
# problem checks' notices, hot scores, sitemaps and fancy titles.
# BadgeGranter.backfill rather than Jobs::BadgeGrant: its inline backfill_badge
# jobs try to start threads and the runner hangs.
Badge.enabled.each { |badge| BadgeGranter.backfill(badge) }
Group.refresh_automatic_groups!
CategoryFeaturedTopic.feature_topics
DirectoryItem.period_types.each_key { |period| DirectoryItem.refresh_period!(period, force: true) }
Jobs::RunProblemChecks.new.execute({})
TopicHotScore.update_scores
Sitemap.regenerate_sitemaps
# Topic#title= nulls fancy_title and the getter writes it back on first read;
# the fixtures reassign titles, and browsing had filled them on the old agent.
Topic.with_deleted.find_each(&:fancy_title)

puts({
  user_badges: UserBadge.count,
  directory_columns: DirectoryColumn.count,
  directory_items: DirectoryItem.count,
  admin_notices: AdminNotice.count,
  topic_hot_scores: TopicHotScore.count,
  category_featured_topics: CategoryFeaturedTopic.count,
  sitemaps: Sitemap.count,
}.inspect)
