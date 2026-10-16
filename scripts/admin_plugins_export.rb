# Records, on a reference Discourse at the vendored commit, what
# /admin/plugins.json says of each visible plugin (AdminPluginSerializer,
# in Discourse.plugins_sorted_by_name order) apart from `enabled`, which
# the port reads from the plugin's enabled setting when it serves the list.
# `config_nav` says whether the plugin's admin initializers add a
# configuration nav (addAdminPluginConfigurationNav), whose pages are the
# plugin's own.
# Run by scripts/record-admin-plugins through `bin/rails runner`.
require "json"

out = ARGV[0]
abort "usage: admin_plugins_export.rb <file>" if out.blank?

plugins =
  Discourse
    .plugins_sorted_by_name(enabled_only: false)
    .map do |plugin|
      json = AdminPluginSerializer.new(plugin, root: false).as_json
      json.delete(:enabled)
      assets = Dir.glob(File.join(plugin.directory, "{assets,admin/assets}", "javascripts", "**", "*.{js,gjs}"))
      json[:config_nav] = assets.any? do |file|
        File.read(file).include?("addAdminPluginConfigurationNav")
      end
      json
    end

File.write(out, JSON.pretty_generate(plugins) + "\n")
puts "#{plugins.size} plugins"
