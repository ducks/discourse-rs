# Records, on a reference Discourse at the vendored commit, the English
# translation tree as Admin::SiteTextsController searches it: every string
# leaf of I18n.backend's `en` translations (server, client, admin, wizard,
# the plugins' and the gems' files: rails-i18n's dates, numbers, errors...)
# in the order DiscourseI18n#find_results walks them, and the keys of the
# other leaves (I18n.exists? is true for them too).
# Run by scripts/record-i18n-tree through `bin/rails runner`.
require "json"

out = ARGV[0]
abort "usage: i18n_tree_export.rb <file>" if out.blank?

I18n.ensure_loaded!(:en)
tree = I18n.backend.send(:translations)[:en]

strings = []
others = []
seen = {}
walk =
  lambda do |node, path|
    node.each do |k_sym, v|
      key = path ? "#{path}.#{k_sym}" : k_sym.to_s
      if v.is_a?(String)
        next if seen.key?(key)
        seen[key] = true
        strings << [key, v]
      elsif v.is_a?(Hash)
        walk.call(v, key)
      else
        others << key
      end
    end
  end
# Faker's locales load only in development (translate_accelerator.rb's
# `Rails.env.local?`); production searches without them.
walk.call(tree.except(:faker), nil)

File.write(out, JSON.generate({ strings: strings, other_leaves: others }))
puts "#{strings.size} strings, #{others.size} other leaves"
