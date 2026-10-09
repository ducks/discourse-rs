# frozen_string_literal: true
# Rails runner script for scripts/record-chat-cooks: Chat::Message.cook for
# each sample, as user1 (id 3) sends it, into ARGV[0].
require "json"

samples = [
  "Hello world",
  "Hello **world** and _you_ and ~~gone~~",
  "# not a heading",
  "Setext\n===",
  "above\n\n---\n\nbelow",
  "* one\n* two\n\n1. first\n2. second",
  "inline `code` here",
  "```ruby\nputs 1\n```",
  "    indented code",
  "> quoted\n> text",
  "[a link](https://example.com) and <https://example.org>",
  "https://example.com",
  "visit example.com today",
  "![alt text](https://example.com/a.png)",
  "<kbd>Ctrl</kbd> + <mark>marked</mark>",
  "<b>bold?</b> <i>it</i> <span>x</span>",
  "<img src=\"https://example.com/b.png\" alt=\"b\">",
  "<div>block html</div>",
  "| a | b |\n|---|---|\n| 1 | 2 |",
  ":smile: :heart: :+1:",
  "inline emoji:smile:here",
  ":) :D <3",
  "@user0 and @user2 and @nobody",
  "@here @all",
  "#general and #guide",
  "[spoiler]hidden[/spoiler]",
  "[quote=\"user0, post:1, topic:35\"]\nquoted post\n[/quote]",
  "[date=2026-10-09 time=18:00:00 timezone=\"UTC\"]",
  "[details=\"summary\"]inside[/details]",
  "[poll]\n* a\n* b\n[/poll]",
  "[ ] todo [x] done",
  "a footnote[^1]\n\n[^1]: the note",
  "/me waves",
  "/shrug",
  "/shrug oh well",
  "line one\nline two",
  "\"quotes\" -- and ... (c)",
  "&amp; &copy; \\*not italic\\*",
  "[ref][1]\n\n[1]: https://example.com",
  "1) paren list",
  "   leading and trailing spaces   ",
]

out = samples.map do |raw|
  { raw: raw, cooked: Chat::Message.cook(raw, user_id: 3, author_username: "user1") }
end
FileUtils.mkdir_p(ARGV[0])
File.write(File.join(ARGV[0], "corpus.json"), JSON.pretty_generate(out) + "\n")
puts "#{out.size} samples"
