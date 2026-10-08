# Records, on a reference Discourse at the vendored commit, enums Ruby
# builds at runtime (core's plus what the bundled plugins add), for
# discourse-rs to read from vendor/discourse/config/enums.json.
# Run by scripts/record-enums through `bin/rails runner`.
require "json"

out = ARGV[0]
abort "usage: enums_export.rb <file>" if out.blank?

result = {
  user_history: {
    actions: UserHistory.actions.to_h.transform_keys(&:to_s),
    staff_actions: UserHistory.staff_actions.map(&:to_s),
    moderator_visible_actions: UserHistory.moderator_visible_actions.map(&:to_s),
  },
}
File.write(out, JSON.pretty_generate(result) + "\n")
puts "#{result[:user_history][:actions].size} user history actions"
