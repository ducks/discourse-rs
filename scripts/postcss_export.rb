# frozen_string_literal: true

# rails runner: AssetProcessor#post_css on each <name>.css of the directory
# given (as Stylesheet::Compiler.compile calls it, with no previous map and
# the source map file `x.css.map`), and for an `rtl_` name Rtlcss.flip_css
# of that (as it flips a right-to-left target), into <name>.out.css. Reads
# nothing else and writes nothing to the database. scripts/record-postcss
# runs it.

require "rtlcss"

dir = ARGV.fetch(0)
count = 0
Dir[File.join(dir, "*.css")].sort.each do |path|
  next if path.end_with?(".out.css")
  css, _map = AssetProcessor.new.post_css(css: File.read(path), map: nil, source_map_file: "x.css.map")
  css = Rtlcss.flip_css(css) if File.basename(path).start_with?("rtl_")
  File.write(path.sub(/\.css\z/, ".out.css"), css)
  count += 1
end
puts "recorded #{count} stylesheets"
