# frozen_string_literal: true
# Records, on a reference Discourse at the vendored commit, the MCP
# primitives core and the bundled plugins register (DiscourseMcp.registry):
# what the admin MCP pages list of each, and when it is available (its
# plugin enabled, and the site settings its availability lambda reads).
# Run by scripts/record-mcp-primitives through `bin/rails runner`.
require "json"

out = ARGV[0]
abort "usage: mcp_primitives_export.rb <file>" if out.blank?

# The site settings a lambda reads, from its source: each one must be
# truthy (every availability lambda is a conjunction of settings).
def settings_read(lambda_value)
  return [] if lambda_value.nil?
  file, line = lambda_value.source_location
  lines = File.readlines(file)[(line - 1)..]
  body = +""
  depth = 0
  lines.each do |l|
    body << l
    depth += l.scan(/\bdo\b|\{/).length
    depth -= l.scan(/\bend\b|\}/).length
    break if depth <= 0
  end
  body.scan(/SiteSetting\.(\w+)/).flatten.uniq
end

plugin_by_name = Discourse.plugins.index_by { |p| p.name }

primitives =
  DiscourseMcp::Primitive::KINDS.flat_map do |kind|
    DiscourseMcp.registry.all(kind).map do |p|
      availability = p.availability
      plugin = nil
      settings = []
      if availability
        binding = availability.binding
        if binding.local_variable_defined?(:configured_availability)
          plugin = binding.receiver.name
          settings = settings_read(binding.local_variable_get(:configured_availability))
        else
          settings = settings_read(availability)
        end
      end
      {
        kind: p.kind,
        identifier: p.identifier,
        title: p.title,
        description: p.description,
        provider: p.provider,
        risk: p.risk,
        required_scopes: p.required_scopes,
        schema_bytes: JSON.generate(p.input_schema).bytesize,
        consent_relevant: p.consent_relevant?,
        plugin: plugin,
        enabled_setting: plugin && plugin_by_name[plugin]&.enabled_site_setting,
        available_when: settings,
      }
    end
  end

File.write(out, JSON.pretty_generate(primitives) + "\n")
puts "recorded #{primitives.size} MCP primitives"
