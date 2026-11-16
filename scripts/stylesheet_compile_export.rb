# frozen_string_literal: true

# rails runner: what Stylesheet::Compiler.compile feeds dart-sass and what
# comes out, for parity/stylesheets/compile (src/stylesheet/mod.rs,
# compile_tests). For each fixture the stylesheet is built as
# Stylesheet::Manager::Builder builds it, and the compile it ends in is
# caught: the SCSS (`input.scss`), dart-sass's output with the source map
# file `x.css.map` (`sass.css`) and AssetProcessor#post_css of that
# (`postcss.css`). The Horizon stylesheets' load paths (the theme's
# exported SCSS) are copied into `horizon_files`. The stylesheet cache
# rows the builds add are rolled back. scripts/record-stylesheet-compiles
# runs it.

out = ARGV.fetch(0)
FileUtils.rm_rf(out)
FileUtils.mkdir_p(out)

horizon = Theme.find(-2)
FIXTURES = {
  "color_definitions_base" => [:color_definitions, nil, nil],
  "color_definitions_2" => [:color_definitions, nil, 2],
  "color_definitions_13" => [:color_definitions, nil, 13],
  "color_definitions_14" => [:color_definitions, nil, 14],
  "color_definitions_16" => [:color_definitions, nil, 16],
  "color_definitions_17" => [:color_definitions, nil, 17],
  "color_definitions_1_horizon" => [:color_definitions, horizon, 1],
  "color_definitions_2_horizon" => [:color_definitions, horizon, 2],
  "horizon_common" => [:common_theme, horizon, nil],
  "horizon_desktop" => [:desktop_theme, horizon, nil],
  "horizon_mobile" => [:mobile_theme, horizon, nil],
}

$fixture_dir = nil
$theme_files = nil

# Stylesheet::Compiler.compile's engine and post_css, with `x.css.map`.
Stylesheet::Compiler.singleton_class.prepend(
  Module.new do
    def compile(stylesheet, filename, options = {})
      load_paths = [Stylesheet::Compiler::ASSET_ROOT] + (options[:load_paths] || [])
      engine =
        SassC::Engine.new(
          stylesheet,
          filename: filename,
          style: :compressed,
          source_map_file: "x.css.map",
          source_map_contents: true,
          load_paths: load_paths,
          silence_deprecations: %w[color-functions import global-builtin if-function],
          fatal_deprecations: [],
          quiet: true,
        )
      sass = engine.render
      map = engine.source_map.force_encoding("UTF-8")
      css, map = AssetProcessor.new.post_css(css: sass, map: map, source_map_file: "x.css.map")
      File.write(File.join($fixture_dir, "input.scss"), stylesheet)
      File.write(File.join($fixture_dir, "sass.css"), sass)
      File.write(File.join($fixture_dir, "postcss.css"), css)
      # The theme's export dir: the parent of its _entry_loadpath.
      entry = (options[:load_paths] || []).find { |p| p.end_with?("/_entry_loadpath") }
      $theme_files ||= File.dirname(entry).then { |d| [d, Dir.glob("#{d}/**/*").select { |f| File.file?(f) }.to_h { |f| [f.delete_prefix("#{d}/"), File.read(f)] }] } if entry
      [css, map]
    end
  end,
)

FIXTURES.each do |name, (target, theme, scheme_id)|
  $fixture_dir = File.join(out, name)
  FileUtils.mkdir_p($fixture_dir)
  manager = Stylesheet::Manager.new(theme_id: theme&.id)
  builder =
    Stylesheet::Manager::Builder.new(
      target: target,
      theme: theme,
      color_scheme: scheme_id && ColorScheme.find(scheme_id),
      manager: manager,
    )
  # Builder#compile adds the result to the stylesheet cache: rolled back.
  ActiveRecord::Base.transaction do
    builder.compile(force: true)
    raise ActiveRecord::Rollback
  end
end

if $theme_files
  _, files = $theme_files
  files.each do |path, content|
    dest = File.join(out, "horizon_files", path)
    FileUtils.mkdir_p(File.dirname(dest))
    File.write(dest, content)
  end
end
puts "recorded #{FIXTURES.size} stylesheets, #{$theme_files ? $theme_files[1].size : 0} theme files"
