# frozen_string_literal: true
# Records, on a reference Discourse at the vendored commit, the admin
# sidebar's nav map (frontend/discourse/app/lib/sidebar/admin-nav-map.js,
# ADMIN_NAV_MAP) as JSON: each link's name, route, label, description,
# keywords, icon, settings area or category and its tabs' links, which
# the admin search indexes and resolves setting pages from.
# Run by scripts/record-admin-nav-map through `bin/rails runner`; the map
# is plain data, evaluated with MiniRacer.
require "json"

out = ARGV[0]
abort "usage: admin_nav_map_export.rb <file>" if out.blank?

src = File.read(Rails.root.join("frontend/discourse/app/lib/sidebar/admin-nav-map.js"))
ctx = MiniRacer::Context.new
ctx.eval(src.sub("export const ADMIN_NAV_MAP =", "var ADMIN_NAV_MAP ="))
map = JSON.parse(ctx.eval("JSON.stringify(ADMIN_NAV_MAP)"))

File.write(out, JSON.pretty_generate(map) + "\n")
puts "recorded #{map.sum { |s| s["links"].size }} nav links"
