# frozen_string_literal: true
# Records, on a reference Discourse at the vendored commit, the commits of
# its history made in the year before it: what GitUtils.has_commit?
# answers true for (`git merge-base --is-ancestor <sha> HEAD`), which
# DiscourseUpdates.new_features asks of each feed entry's
# discourse_version. A year covers the feed, which only lists features
# released in the last five months.
# Run by scripts/record-discourse-commits through `bin/rails runner`.
out = ARGV[0]
abort "usage: discourse_commits_export.rb <file>" if out.blank?

since = `git -C #{Rails.root} log -1 --format=%cI HEAD`.strip
since = (Time.zone.parse(since) - 1.year).iso8601
shas = `git -C #{Rails.root} rev-list --since=#{since} HEAD`.split("\n")
abort "git rev-list failed" if !$?.success? || shas.empty?

File.write(out, shas.sort.join("\n") + "\n")
puts "recorded #{shas.size} commits since #{since}"
