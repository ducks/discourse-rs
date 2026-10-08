# HtmlPrettify's output for the strings in Discourse's own spec
# (spec/lib/html_prettify_spec.rb), and Topic.fancy_title for the titles
# in parity/fancy_title/extra.json, for discourse-rs to be measured
# against (run by scripts/record-fancy-titles through `bin/rails runner`).
# Read-only; fancy_title follows the agent's emoji settings, recorded
# alongside.
require "json"
require "prism"

spec, extra, output = ARGV
abort "usage: fancy_title_export.rb <spec.rb> <extra.json> <out.json>" if output.blank?

inputs = []
walk = ->(node) do
  return if node.nil?
  inputs << node.unescaped if node.is_a?(Prism::StringNode)
  node.compact_child_nodes.each { |c| walk.(c) }
end
walk.(Prism.parse(File.read(spec)).value)
inputs.uniq!

result = {
  settings: {
    enable_emoji: SiteSetting.enable_emoji,
    enable_emoji_shortcuts: SiteSetting.enable_emoji_shortcuts,
    enable_inline_emoji_translation: SiteSetting.enable_inline_emoji_translation,
  },
  prettify: inputs.map { |html| { html: html, rendered: HtmlPrettify.render(html) } },
  fancy_title:
    JSON.parse(File.read(extra)).map { |title| { title: title, fancy: Topic.fancy_title(title) } },
}
File.write(output, JSON.pretty_generate(result) + "\n")
puts "#{inputs.size} prettify inputs, #{result[:fancy_title].size} titles"
