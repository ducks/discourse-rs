# Records, on a reference Discourse at the vendored commit, the parts of
# SiteSettings::TypeSupervisor#type_hash that Ruby code computes: each
# enum class's valid values and translate_names, each setting's choices
# (evaluated from expressions like "TopMenu.choices" at load) and JSON
# schema.
# Run by scripts/record-setting-enums through `bin/rails runner`. Enum
# classes reading the database or other settings are left out; the port
# computes those when it serves the setting.
require "json"

out = ARGV[0]
abort "usage: setting_enums_export.rb <file>" if out.blank?

RUNTIME_CLASSES = %w[
  DiscourseAi::Configuration::AgentEnumerator
  DiscourseAi::Configuration::LlmEnumerator
  DiscourseAi::Configuration::EmbeddingDefsEnumerator
  ReactionForLikeSiteSettingEnum
  HomepageSiteSetting
]

supervisor = SiteSetting.type_supervisor
enums = supervisor.instance_variable_get(:@enums)
choices = supervisor.instance_variable_get(:@choices)
json_schemas = supervisor.instance_variable_get(:@json_schemas)

record = {}
SiteSetting.defaults.all.keys.sort.each do |name|
  klass = enums[name]
  next if klass.nil? && !choices.key?(name) && !json_schemas.key?(name)
  next if klass && RUNTIME_CLASSES.include?(klass.name)
  hash = supervisor.type_hash(name)
  record[name] = hash.slice(:valid_values, :translate_names, :choices, :json_schema)
end
# Settings plugins register from Ruby (discourse-ai's agent setting for each
# AI feature another plugin offers) rather than in a settings.yml, with
# what all_settings reads of them.
declared = YAML.safe_load_file(Rails.root.join("config/site_settings.yml"), aliases: true).values.flat_map(&:keys)
Dir[Rails.root.join("plugins/*/config/settings.yml")].each do |f|
  (YAML.safe_load_file(f, aliases: true) || {}).each_value { |s| declared.concat(s.keys) if s.is_a?(Hash) }
end
declared = declared.map(&:to_s).to_set
record[:_dynamic] = SiteSetting.defaults.all.keys.reject { |n| declared.include?(n.to_s) || n == :default_locale }.map do |name|
  {
    name: name,
    default: SiteSetting.defaults.get(name),
    category: SiteSetting.categories[name],
    area: Array.wrap(SiteSetting.areas[name]).join("|").presence,
    type: supervisor.get_type(name),
    enum: enums[name]&.name,
    depends_on: supervisor.dependencies[name],
    depends_behavior: supervisor.dependencies.behaviors[name],
    plugin: SiteSetting.plugins[name],
  }
end
# Areas a plugin set as a String rather than a list (all_settings then
# reports their first character as the primary area).
record[:_string_areas] = SiteSetting.areas.select { |_, v| v.is_a?(String) }
# SiteSettings::DeprecatedSettings with the plugins' additions: [old, new].
record[:_deprecated] = SiteSettings::DeprecatedSettings::SETTINGS.map { |s| s.first(2) }
# The stock images' seeded uploads (negative ids) with their measured size
# (Upload#width measures the file when the row has none), for sites
# without Discourse's public/images beside the port.
record[:_stock_upload_dimensions] = Upload.where("id < 0").to_h { |u| [u.url, [u.width, u.height]] }
record[:default_locale] = {
  valid_values: LocaleSiteSetting.values,
  translate_names: LocaleSiteSetting.translate_names?,
}

File.write(out, JSON.pretty_generate(record) + "\n")
puts "recorded #{record.size} settings' values"
