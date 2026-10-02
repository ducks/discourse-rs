# frozen_string_literal: true
#
# Rails runner: what PostRevisionSerializer shows as body_changes for edits
# between pretty_text corpus entries, for the DiscourseDiff port's tests.
#
# For each pair of texts, DiscourseDiff's three renderings. The pairs are
# consecutive corpus entries and each entry against a lightly edited copy
# of itself, the usual shape of an edit.
#
# Usage: bin/rails runner revision_fixtures.rb <corpus.json> <out.json>

corpus = JSON.parse(File.read(ARGV[0]))

def edited(text)
  text.sub(" the ", " a ").sub(/\.(\s|\z)/, "!\\1") + "\n\nEdited."
end

def diff_of(before, after)
  cooked = DiscourseDiff.new(before[:cooked], after[:cooked])
  raw = DiscourseDiff.new(before[:raw], after[:raw])
  {
    inline: cooked.inline_html,
    side_by_side: cooked.side_by_side_html,
    side_by_side_markdown: raw.side_by_side_markdown,
  }
rescue ONPDiff::DiffLimitExceeded
  { error: "diff_limit_exceeded" }
end

pairs = []
corpus.each_with_index do |entry, i|
  a = { raw: entry["raw"], cooked: entry["cooked"] }
  b = { raw: edited(entry["raw"]), cooked: edited(entry["cooked"]) }
  pairs << ["#{entry["id"]}-edited", a, b]
  if (succ = corpus[i + 1])
    pairs << ["#{entry["id"]}-#{succ["id"]}", a, { raw: succ["raw"], cooked: succ["cooked"] }]
  end
end

out = {
  pairs:
    pairs.map do |name, a, b|
      { name: name, before: a, after: b, diff: diff_of(a, b) }
    end,
}
File.write(ARGV[1], JSON.pretty_generate(out))
puts "wrote #{pairs.size} pairs"
