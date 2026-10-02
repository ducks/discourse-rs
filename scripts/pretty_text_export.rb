# Records what Rails cooks on a reference Discourse, for discourse-rs to be
# measured against (run by scripts/record-pretty-text through
# `bin/rails runner`): the options PrettyText.markdown hands the renderer
# on this site, and a corpus of inputs with PrettyText.markdown's output.
require "json"
require "fileutils"

out = ARGV[0]
abort "usage: pretty_text_export.rb <dir>" if out.blank?
FileUtils.mkdir_p(out)

# Every call the cooking JavaScript makes back into Ruby
# (PrettyText::Helpers), with its result: wrapped before the context is
# built, since the context captures the methods when it attaches them.
HELPER_CALLS = []
PrettyText::Helpers.instance_methods.each do |m|
  original = PrettyText::Helpers.method(m)
  PrettyText::Helpers.define_singleton_method(m) do |*args|
    result = original.call(*args)
    HELPER_CALLS << { method: m, args: args, result: result }
    result
  end
end

# PrettyText.markdown's opt_input, without the per-call ids.
custom_emoji = {}
Emoji.custom.map { |e| custom_emoji[e.name] = e.cdn_url }
opt_input = {
  siteSettings: SiteSetting.client_settings_hash,
  allowedIframes: DiscoursePluginRegistry.apply_modifier(:pretty_text_allowed_iframes, SiteSetting.allowed_iframes.split("|")),
  paths: PrettyText.paths,
  customEmoji: custom_emoji,
  customEmojiTranslation: Plugin::CustomEmoji.translations,
  emojiDenyList: Emoji.denied,
  censoredRegexp: WordWatcher.serialized_regexps_for_action(:censor),
  watchedWordsReplace: WordWatcher.regexps_for_action(:replace),
  watchedWordsLink: WordWatcher.regexps_for_action(:link),
  additionalOptions: Site.markdown_additional_options,
  avatar_sizes: SiteSetting.avatar_sizes,
  hashtagTypesInPriorityOrder: HashtagAutocompleteService.ordered_types_for_context("topic-composer"),
  hashtagIcons: HashtagAutocompleteService.data_source_icon_map,
}
File.write("#{out}/opt_input.json", JSON.pretty_generate(opt_input) + "\n")

u1 = User.find_by!(username: "user1")
admin = User.find_by!(username: "admin")
replies = Topic.find_by!(title: "Parity fixture: replies and posters")
pinned = Topic.find_by!(title: "Parity fixture: pinned and closed")
staff_topic = Topic.where(category_id: Category.find_by!(slug: "staff").id).order(:id).first!
upload = Upload.where("id > 0").order(:id).first

# [raw, topic the post is cooked in, cooking user]
samples = [
  "Plain paragraph with *emphasis*, **strong**, ~~strike~~ and `code`.",
  "# Heading one\n\n## Heading two\n\nText under it.\n\n---\n\nAfter the rule.",
  "- one\n- two\n  - nested\n  - nested two\n- three\n\n1. first\n2. second\n3. third",
  "```ruby\ndef hello\n  puts \"hi\"\nend\n```\n\n    indented code\n\n```\nno language\n```",
  "| a | b |\n|---|---:|\n| 1 | 2 |\n| three | four |",
  "> a quote\n> on two lines\n\n> > nested",
  "[quote=\"user1, post:2, topic:#{replies.id}\"]\nReply one from user1\n[/quote]\n\nI agree.",
  "[quote=\"user0, post:1, topic:#{replies.id}, full:true\"]\nFirst post\n[/quote]",
  ["[quote=\"user0, post:1, topic:#{replies.id}\"]\nQuoted from another, public topic\n[/quote]", pinned.id, u1.id],
  "[quote=\"admin, post:1, topic:#{staff_topic.id}\"]\nQuoted from a topic the public can't see\n[/quote]",
  "[quote=\"admin, post:1, topic:999999\"]\nQuoted from a topic that does not exist\n[/quote]",
  "[quote=\"nobody\"]\nA quote without a post\n[/quote]",
  "Hello @user1 and @admin and @nobody, also @staff and @trust_level_2.",
  "Categories #general and #site-feedback, tags #howto and #guide, and #nothing.",
  "Typed #guide::tag and #general::category, nested #general:sub-general, and #staff.",
  ["The staff category for an admin: #staff and #howto.", replies.id, admin.id],
  ["Hashtags cooked for nobody: #general #staff #howto.", replies.id, nil],
  "Emoji :smile: :+1: :wave:t3: and shortcuts :) ;) :D and :not_an_emoji:.",
  "Unicode emoji 😀 and 👍🏽 inline.",
  "A link https://example.com/path?q=1 and [named](https://example.org \"title\") and www.example.net.",
  "https://example.com/onebox-candidate\n\nText after.",
  "![alt text|100x200](upload://abcdefghij1234567890.png)\n\n![plain](https://example.com/a.png)",
  "[file.pdf|attachment](upload://zyxwvutsrq0987654321.pdf) (12 KB)",
  "Text with \"quotes\", 'single', dashes -- and --- and ellipsis... (c) (tm) 1/2.",
  "<b>bold html</b> <script>alert(1)</script> <img src=x onerror=alert(1)> <kbd>Ctrl</kbd> <span style=\"color:red\">x</span>",
  "<div align=\"center\">centered</div>\n\n<details>\n<summary>Summary</summary>\nHidden text\n</details>",
  "<iframe src=\"https://www.google.com/maps/embed?pb=1\" width=\"600\" height=\"450\"></iframe>\n\n<iframe src=\"https://evil.example.com/\"></iframe>",
  "[details=\"Click me\"]\nInside details with **bold**.\n[/details]",
  "[spoiler]secret text[/spoiler] and inline [spoiler]x[/spoiler].",
  "[poll type=regular results=always]\n* Option A\n* Option B\n[/poll]",
  "[date=2026-10-02 time=10:00:00 timezone=\"Europe/Paris\"]",
  "- [ ] unchecked\n- [x] checked\n- [ ] another",
  "Footnote here[^1] and inline^[an inline note].\n\n[^1]: The footnote text.",
  "Line one\nLine two with a soft break\\\nhard break\n\nNew paragraph.",
  "[b]bbcode bold[/b] [i]italic[/i] [u]underline[/u] [s]strike[/s] [code]inline[/code]",
  "[code]\nblock bbcode code\n  indented\n[/code]",
  "Math $x^2 + y^2$ and\n\n$$\n\\int_0^1 x dx\n$$",
  "```mermaid\ngraph TD; A-->B;\n```",
  "Text <!-- a comment --> after comment.\n\n&amp; &lt; &copy; &#35; entities.",
  "Term\n: Definition\n\nH~2~O and x^2^ and ==marked== and ++inserted++.",
  "#{Discourse.base_url}/t/#{replies.slug}/#{replies.id}\n\n#{Discourse.base_url}/t/#{replies.slug}/#{replies.id}/2",
  "a\n\n\n\nb with many blank lines\n\n   \n\nc",
  "    \n\ttab indented\n* * *\n_ _ _\n+ plus list\n+ second",
  "Setext heading\n==============\n\nAnother\n-------",
  "Autolink <https://example.com> and email <a@example.com> and a@example.com plain.",
  "Very long " + ("word " * 400),
  "RTL text: مرحبا بالعالم and CJK: 你好世界 こんにちは 한국어.",
  "[wrap=foo bar=1]\nwrapped block\n[/wrap]\n\ninline [wrap=x]y[/wrap]",
  "[grid]\n![a](upload://aaaaaaaaaaaaaaaaaaaa.png)\n![b](upload://bbbbbbbbbbbbbbbbbbbb.png)\n[/grid]",
  "![video|video](upload://cccccccccccccccccccc.mp4)\n\n![audio|audio](upload://dddddddddddddddddddd.mp3)",
]
if upload
  samples << "An upload that exists: ![image|64x64](#{upload.short_url}) and [a link|attachment](#{upload.short_url})."
end

corpus = []
samples.each_with_index do |sample, i|
  raw, topic_id, user_id = sample.is_a?(Array) ? sample : [sample, replies.id, u1.id]
  corpus << { id: "sample-#{i}", raw: raw, topic_id: topic_id, user_id: user_id }
end
Post.with_deleted.order(:id).each do |p|
  corpus << { id: "post-#{p.id}", raw: p.raw, topic_id: p.topic_id, user_id: p.user_id }
end
corpus.each do |c|
  c[:markdown] = PrettyText.markdown(c[:raw].dup, topic_id: c[:topic_id], user_id: c[:user_id])
end
File.write("#{out}/corpus.json", JSON.pretty_generate(corpus) + "\n")
calls = HELPER_CALLS.uniq { |c| [c[:method], c[:args]] }
File.write("#{out}/helpers.json", JSON.pretty_generate(calls) + "\n")
puts "#{corpus.size} corpus entries (#{samples.size} samples), #{calls.size} helper calls"
