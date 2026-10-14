# Records, on a reference Discourse at the vendored commit, what admin API
# key scopes are made of at runtime: ApiKeyScope.scope_mappings (core's and
# the bundled plugins', with the urls find_urls lists for each), and the
# route table in recognition order (mounted engines expanded where they are
# mounted), from which discourse-rs tells which controller#action a request
# reaches. Written to vendor/discourse/config/api_key_scopes.json.
# Run by scripts/record-api-key-scopes through `bin/rails runner`.
require "json"

out = ARGV[0]
abort "usage: api_key_scopes_export.rb <file>" if out.blank?

def action_name(action)
  action == Logster::Web ? "logster" : action.to_s
end

scopes =
  ApiKeyScope.scope_mappings.to_h do |resource, actions|
    [
      resource.to_s,
      actions.to_h do |name, mapping|
        [
          name.to_s,
          {
            actions: mapping[:actions]&.map { |a| action_name(a) },
            methods: mapping[:methods]&.map(&:to_s),
            formats: mapping[:formats] && Array(mapping[:formats]).map(&:to_s),
            params: mapping[:params]&.map(&:to_s),
            path_params: mapping[:path_params]&.map(&:to_s),
            aliases: mapping[:aliases]&.to_h { |k, v| [k.to_s, v.to_s] },
            urls: mapping[:urls],
          }.compact,
        ]
      end,
    ]
  end

def requirement(value)
  value.is_a?(Regexp) ? value.source : value.to_s
end

def routes_of(set, prefix, out)
  set.routes.each do |route|
    app = route.app
    app = app.app if app.respond_to?(:app) && !app.respond_to?(:routes) && app.app.respond_to?(:routes)
    if app.respond_to?(:routes) && app.routes.is_a?(ActionDispatch::Routing::RouteSet) &&
         app.routes != set
      mount = route.path.spec.to_s.sub(/\(\.:format\)\z/, "")
      mount = "" if mount == "/"
      routes_of(app.routes, "#{prefix}#{mount}", out)
      next
    end
    defaults = route.defaults
    next if defaults[:controller].blank? || defaults[:action].blank?
    constraint = route.app.respond_to?(:constraints) ? route.app.constraints : []
    out << {
      verb: route.verb.to_s,
      path: "#{prefix}#{route.path.spec}",
      requirements: route.path.requirements.to_h { |k, v| [k.to_s, requirement(v)] },
      action: "#{defaults[:controller]}##{defaults[:action]}",
      format: defaults[:format]&.to_s,
      constraints: Array(constraint).map { |c| c.is_a?(Proc) ? "proc" : c.class.name },
    }.compact
  end
end

routes = []
routes_of(Rails.application.routes, "", routes)

File.write(out, JSON.pretty_generate({ scopes: scopes, routes: routes }) + "\n")
puts "#{scopes.values.sum(&:size)} scopes, #{routes.size} routes"
