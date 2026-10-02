# HtmlToMarkdown's output for the HTML in Discourse's own spec
# (spec/lib/html_to_markdown_spec.rb), plus parity/html_to_markdown's
# extra samples, for discourse-rs to be measured against (run by
# scripts/record-html-to-markdown through `bin/rails runner`). Each input
# is converted with no options and with the ones Email::Receiver passes.
require "json"
require "prism"

spec, extra, output = ARGV
abort "usage: html_to_markdown_export.rb <spec.rb> <extra.json> <out.json>" if output.blank?

inputs = []
walk = ->(node) do
  return if node.nil?
  inputs << node.unescaped if node.is_a?(Prism::StringNode) && node.unescaped =~ /<[a-z!]/i
  node.compact_child_nodes.each { |c| walk.(c) }
end
walk.(Prism.parse(File.read(spec)).value)
inputs.concat(JSON.parse(File.read(extra)))
inputs.uniq!

receiver_opts = { keep_img_tags: true, keep_cid_imgs: true }
result =
  inputs.map do |html|
    {
      html: html,
      markdown: HtmlToMarkdown.new(html).to_markdown,
      receiver: HtmlToMarkdown.new(html, receiver_opts).to_markdown,
    }
  end

File.write(output, JSON.pretty_generate(result) + "\n")
puts "#{result.size} inputs"
