# Records, on a reference Discourse at the vendored commit, what SvgSprite
# builds its sprites from that does not live in the database: the core
# symbols (vendor/assets/svg-icons, each as SvgSprite.symbols_for
# serializes it), the bundled plugins' symbols (their svg-icons/*.svg),
# SVG_ICONS, and the icons the plugins register (register_svg_icon).
# Run by scripts/record-svg-sprite through `bin/rails runner`.
require "json"

out = ARGV[0]
abort "usage: svg_sprite_export.rb <file>" if out.blank?

record = {
  core: SvgSprite.core_svgs.to_a,
  plugin: SvgSprite.plugin_svgs.to_a,
  svg_icons: SvgSprite::SVG_ICONS.to_a.sort,
  plugin_icons: DiscoursePluginRegistry.svg_icons.to_a,
}

File.write(out, JSON.generate(record) + "\n")
puts "#{record[:core].size} core, #{record[:plugin].size} plugin symbols, " \
       "#{record[:svg_icons].size} core icons, #{record[:plugin_icons].size} plugin icons"
