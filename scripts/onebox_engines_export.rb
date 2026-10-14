# Which onebox engine Discourse picks for each URL (Oneboxer.engine: the
# matcher with the site's iframe allowlist), for the URLs in its onebox
# specs and parity/onebox_engines/extra.json; what the engines that need
# no fetch render for them (Onebox.preview as
# Oneboxer.compute_external_onebox calls it); and what every engine
# renders for the pages in parity/onebox_engines/fixtures.json, with the
# fetch boundary (Onebox::Helpers.fetch_response, fetch_content_length,
# OpenURI's URI#open, FinalDestination#resolve) answered from those
# fixture files instead of the network. For discourse-rs to be measured
# against (run by scripts/record-onebox-engines through
# `bin/rails runner`). Read-only: nothing is written to the database or
# the cache.
require "json"
require "prism"

extra_path, output, fixtures_path, fixtures_dir = ARGV
if fixtures_dir.blank?
  abort "usage: onebox_engines_export.rb <extra.json> <out.json> <fixtures.json> <fixtures dir>"
end
extra = JSON.parse(File.read(extra_path))
FIXTURES = JSON.parse(File.read(fixtures_path))
FIXTURE_DIR = fixtures_dir
REQUESTS = []

# The fixture pages, keyed by URL without its fragment (what a request
# asks for).
module FixtureFetch
  def self.page(url)
    key = url.to_s.split("#", 2).first
    REQUESTS << key
    FIXTURES["pages"][key]
  end

  def self.status_error!(status)
    klass = Net::HTTPResponse::CODE_TO_OBJ[status.to_s] || Net::HTTPUnknownResponse
    klass.new("1.1", status.to_s, "Fixture").error!
  end

  def self.body(url)
    page = page(url)
    raise Errno::ECONNREFUSED, url.to_s if page.nil?
    status = page["status"] || 200
    status_error!(status) if status != 200
    page["file"] ? File.read(File.join(FIXTURE_DIR, page["file"])) : +""
  end

  def self.open_body(url)
    page = page(url)
    raise Errno::ECONNREFUSED, url.to_s if page.nil?
    status = page["status"] || 200
    if status != 200
      raise OpenURI::HTTPError.new("#{status} Fixture", StringIO.new)
    end
    page["file"] ? File.read(File.join(FIXTURE_DIR, page["file"])) : +""
  end
end

Onebox::Helpers.singleton_class.prepend(
  Module.new do
    def fetch_response(location, **_opts)
      FixtureFetch.body(location)
    end

    def fetch_content_length(location)
      page = FixtureFetch.page(location)
      raise Errno::ECONNREFUSED, location.to_s if page.nil?
      (page["status"] || 200) == 200 ? page["content_length"].presence : nil
    end
  end,
)

URI::HTTP.prepend(
  Module.new do
    def open(*_args, &block)
      io = StringIO.new(FixtureFetch.open_body(to_s))
      block ? block.call(io) : io
    end
  end,
)

FinalDestination.prepend(
  Module.new do
    def resolve
      return nil if @uri.nil?
      key = @uri.to_s
      REQUESTS << "resolve #{key}"
      page = FIXTURES["pages"][key.split("#", 2).first]
      to = page&.key?("resolves_to") ? page["resolves_to"] : key
      if to.blank?
        @status = :failure
        return nil
      end
      @status = :resolved
      @content_type = page && page["content_type"]
      @uri = URI(to)
    end
  end,
)


# The GitHub API client's request (no etag cache, no rate-limit state):
# the fixture's body, or the error its status raises.
Discourse::GithubApi.prepend(
  Module.new do
    private

    def request(_method, path, params: {}, body: nil, accept:, cache_etag: true)
      url = path.start_with?("http") ? path : "#{Discourse::GithubApi::API_ROOT}#{path}"
      host = URI.parse(url).host
      if Discourse::GithubApi::ALLOWED_HOSTS.exclude?(host)
        raise ArgumentError, "Refusing to call non-GitHub host: #{host.inspect}"
      end
      page = FixtureFetch.page(url)
      raise Discourse::GithubApi::Error.new("GitHub API request failed: refused") if page.nil?
      status = page["status"] || 200
      case status
      when 200..299
        page["file"] ? File.read(File.join(FIXTURE_DIR, page["file"])) : +""
      when 401
        raise Discourse::GithubApi::Unauthorized.new(status: 401)
      when 404
        raise Discourse::GithubApi::NotFound.new(status: 404)
      else
        raise Discourse::GithubApi::Error.new(status: status)
      end
    end
  end,
)

# FinalDestination::HTTP connections, for the HEAD requests GoogleMapsOnebox
# makes: the fixture's status and Location for the path.
FixtureHead = Struct.new(:code, :location) do
  def [](name)
    name.to_s.downcase == "location" ? location : nil
  end
end
FixtureHttp = Struct.new(:scheme, :host, :port) do
  def head(path)
    default = (scheme == "https" && port == 443) || (scheme == "http" && port == 80)
    url = default ? "#{scheme}://#{host}#{path}" : "#{scheme}://#{host}:#{port}#{path}"
    page = FixtureFetch.page(url)
    raise Errno::ECONNREFUSED, url if page.nil?
    FixtureHead.new((page["status"] || 200).to_s, page["location"])
  end

  def finish
  end
end
FinalDestination::HTTP.singleton_class.prepend(
  Module.new do
    def start(host, port, use_ssl: false, **_opts)
      FixtureHttp.new(use_ssl ? "https" : "http", host, port)
    end
  end,
)
Oneboxer.singleton_class.prepend(Module.new { def cache_preferred_strategy(*); end })

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

# A value, nil when what made it raised one of Preview's WEB_EXCEPTIONS
# (the preview is blank either way), or "raised" for anything else.
def attempt
  [yield, nil]
rescue *Onebox::Preview::WEB_EXCEPTIONS
  [nil, nil]
rescue StandardError => e
  [nil, "raised #{e.class}: #{e.message.to_s[0, 200]}"]
end

fetched =
  FIXTURES["cases"].map do |url|
    REQUESTS.clear
    content_type = FIXTURES["pages"].dig(url.split("#", 2).first, "content_type")
    engine_class = (Oneboxer.engine(url) rescue nil)
    preview = Onebox.preview(url, options.merge(content_type: content_type))
    engine = preview.send(:engine)
    record = { url: url, engine: engine&.class&.name&.split("::")&.last }
    if engine
      raw, raw_error = attempt { engine.to_html }
      raw_placeholder, placeholder_error = attempt { engine.placeholder_html }
      errors = engine.errors.keys.map(&:to_s)
      verified =
        if errors.any? && engine.respond_to?(:verified_data)
          attempt { engine.verified_data }.first
        end
      record.merge!(
        matcher_engine: engine_class&.name&.split("::")&.last,
        raw: raw,
        raw_error: raw_error,
        processed: raw.is_a?(String) ? preview.send(:process_html, raw) : nil,
        html: attempt { preview.to_s }.then { |v, err| err || v },
        raw_placeholder: raw_placeholder,
        placeholder_error: placeholder_error,
        processed_placeholder:
          raw_placeholder.is_a?(String) ? preview.send(:process_html, raw_placeholder) : nil,
        placeholder: attempt { preview.placeholder_html }.then { |v, err| err || v },
        errors: errors,
        verified_data: verified,
      )
    end
    external, external_error = attempt { Oneboxer.compute_external_onebox(url) }
    record[:external] = external_error || external
    record[:requests] = REQUESTS.uniq
    record
  end

# Date._parse and Time.parse on the date strings pages give, as the
# engines print them.
times =
  FIXTURES["times"].map do |s|
    hash = Date._parse(s).slice(:year, :mon, :mday, :yday, :hour, :min, :sec, :zone)
    time =
      begin
        t = Time.parse(s)
        [t.strftime("%-d %b %y"), t.strftime("%I:%M%p - %d %B %Y"), t.strftime("%H:%M")]
      rescue ArgumentError
        "raised"
      end
    { input: s, hash: hash, time: time }
  end

result = {
  settings: {
    allowed_onebox_iframes: SiteSetting.allowed_onebox_iframes,
    allowed_iframes: SiteSetting.allowed_iframes,
    current_hostname: Discourse.current_hostname,
    disable_onebox_media_download_controls: SiteSetting.disable_onebox_media_download_controls,
    enable_emoji: SiteSetting.enable_emoji,
    enable_inline_emoji_translation: SiteSetting.enable_inline_emoji_translation,
    emoji_set: SiteSetting.emoji_set,
    github_onebox_access_tokens: SiteSetting.github_onebox_access_tokens,
    github_pr_status_enabled:
      SiteSetting.respond_to?(:github_pr_status_enabled) && SiteSetting.github_pr_status_enabled,
    facebook_app_access_token: SiteSetting.facebook_app_access_token,
    twitter_credentials: !TwitterApi.twitter_credentials_missing?,
    base_path: Discourse.base_path,
    lazy_youtube: SiteSetting.lazy_videos_enabled && SiteSetting.lazy_youtube_enabled,
    lazy_vimeo: SiteSetting.lazy_videos_enabled && SiteSetting.lazy_vimeo_enabled,
    lazy_tiktok: SiteSetting.lazy_videos_enabled && SiteSetting.lazy_tiktok_enabled,
  },
  engines: engines,
  previews: previews,
  fetched: fetched,
  times: times,
}
File.write(output, JSON.pretty_generate(result) + "\n")
puts "#{engines.size} urls, #{previews.size} previews, #{fetched.size} fetched"
