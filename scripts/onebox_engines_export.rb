# Which onebox engine Discourse picks for each URL (Oneboxer.engine: the
# matcher with the site's iframe allowlist), for the URLs in its onebox
# specs and parity/onebox_engines/extra.json, and what the engines that
# need no fetch render for them (Onebox.preview as
# Oneboxer.compute_external_onebox calls it), for discourse-rs to be
# measured against (run by scripts/record-onebox-engines through
# `bin/rails runner`). Read-only.
require "json"
require "prism"

extra_path, output = ARGV
abort "usage: onebox_engines_export.rb <extra.json> <out.json>" if output.blank?
extra = JSON.parse(File.read(extra_path))

urls = []
walk = ->(node) do
  return if node.nil?
  if node.is_a?(Prism::StringNode) && node.unescaped.match?(%r{\A(https?:)?//\S+\z})
    urls << node.unescaped
  end
  node.compact_child_nodes.each { |c| walk.(c) }
end
(Dir["spec/lib/onebox/**/*_spec.rb"] + ["spec/lib/oneboxer_spec.rb"]).sort.each do |f|
  walk.(Prism.parse(File.read(f)).value)
end
urls.concat(extra["urls"])
urls.uniq!

preview_engines = extra["preview_engines"]
options = {
  max_width: 695,
  sanitize_config: Onebox::SanitizeConfig::DISCOURSE_ONEBOX,
  allowed_iframe_origins: Oneboxer.allowed_iframe_origins,
  hostname: GlobalSetting.hostname,
  facebook_app_access_token: SiteSetting.facebook_app_access_token,
  disable_media_download_controls: SiteSetting.disable_onebox_media_download_controls,
}

engines =
  urls.map do |url|
    engine = Oneboxer.engine(url) rescue nil
    { url: url, engine: engine&.name&.split("::")&.last }
  end
previews =
  engines
    .select { |e| preview_engines.include?(e[:engine]) }
    .map do |e|
      preview = Onebox.preview(e[:url], options)
      engine = preview.send(:engine)
      raw = engine.to_html
      raw_placeholder = engine.placeholder_html
      {
        url: e[:url],
        engine: e[:engine],
        # The engine's own HTML, then as process_html leaves it, then
        # sanitized: what the preview gives.
        raw: raw,
        processed: preview.send(:process_html, raw),
        html: preview.to_s,
        raw_placeholder: raw_placeholder,
        processed_placeholder: preview.send(:process_html, raw_placeholder),
        placeholder: preview.placeholder_html,
      }
    end

result = {
  settings: {
    allowed_onebox_iframes: SiteSetting.allowed_onebox_iframes,
    allowed_iframes: SiteSetting.allowed_iframes,
    current_hostname: Discourse.current_hostname,
    disable_onebox_media_download_controls: SiteSetting.disable_onebox_media_download_controls,
  },
  engines: engines,
  previews: previews,
}
File.write(output, JSON.pretty_generate(result) + "\n")
puts "#{engines.size} urls, #{previews.size} previews"
