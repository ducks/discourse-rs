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
