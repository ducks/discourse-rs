# frozen_string_literal: true

# rails runner: AssetProcessor#post_css on each <name>.css of the directory
# given (as Stylesheet::Compiler.compile calls it, with no previous map and
# the source map file `x.css.map`), into <name>.out.css. Reads nothing
# else and writes nothing to the database. scripts/record-postcss runs it.

dir = ARGV.fetch(0)
count = 0
Dir[File.join(dir, "*.css")].sort.each do |path|
  next if path.end_with?(".out.css")
  css, _map = AssetProcessor.new.post_css(css: File.read(path), map: nil, source_map_file: "x.css.map")
  File.write(path.sub(/\.css\z/, ".out.css"), css)
  count += 1
end
puts "recorded #{count} stylesheets"
