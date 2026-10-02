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

# The feature samples need the parity fixtures (scripts/fixtures); on any
# other site only the posts are recorded.
samples = []
if Topic.exists?(title: "Parity fixture: replies and posters")
u1 = User.find_by!(username: "user1")
admin = User.find_by!(username: "admin")
replies = Topic.find_by!(title: "Parity fixture: replies and posters")
pinned = Topic.find_by!(title: "Parity fixture: pinned and closed")
staff_topic = Topic.where(category_id: Category.find_by!(slug: "staff").id).order(:id).first!
upload = Upload.where("id > 0").order(:id).first

# [raw, topic the post is cooked in, cooking user]
samples = [
  ["Plain paragraph with *emphasis*, **strong**, ~~strike~~ and `code`.", replies.id, u1.id],
  ["# Heading one\n\n## Heading two\n\nText under it.\n\n---\n\nAfter the rule.", replies.id, u1.id],
  ["- one\n- two\n  - nested\n  - nested two\n- three\n\n1. first\n2. second\n3. third", replies.id, u1.id],
  ["```ruby\ndef hello\n  puts \"hi\"\nend\n```\n\n    indented code\n\n```\nno language\n```", replies.id, u1.id],
  ["| a | b |\n|---|---:|\n| 1 | 2 |\n| three | four |", replies.id, u1.id],
  ["> a quote\n> on two lines\n\n> > nested", replies.id, u1.id],
  ["[quote=\"user1, post:2, topic:#{replies.id}\"]\nReply one from user1\n[/quote]\n\nI agree.", replies.id, u1.id],
  ["[quote=\"user0, post:1, topic:#{replies.id}, full:true\"]\nFirst post\n[/quote]", replies.id, u1.id],
  ["[quote=\"user0, post:1, topic:#{replies.id}\"]\nQuoted from another, public topic\n[/quote]", pinned.id, u1.id],
  ["[quote=\"admin, post:1, topic:#{staff_topic.id}\"]\nQuoted from a topic the public can't see\n[/quote]", replies.id, u1.id],
  ["[quote=\"admin, post:1, topic:999999\"]\nQuoted from a topic that does not exist\n[/quote]", replies.id, u1.id],
  ["[quote=\"nobody\"]\nA quote without a post\n[/quote]", replies.id, u1.id],
  ["Hello @user1 and @admin and @nobody, also @staff and @trust_level_2.", replies.id, u1.id],
  ["Categories #general and #site-feedback, tags #howto and #guide, and #nothing.", replies.id, u1.id],
  ["Typed #guide::tag and #general::category, nested #general:sub-general, and #staff.", replies.id, u1.id],
  ["The staff category for an admin: #staff and #howto.", replies.id, admin.id],
  ["Hashtags cooked for nobody: #general #staff #howto.", replies.id, nil],
  ["Emoji :smile: :+1: :wave:t3: and shortcuts :) ;) :D and :not_an_emoji:.", replies.id, u1.id],
  ["Unicode emoji 😀 and 👍🏽 inline.", replies.id, u1.id],
  ["A link https://example.com/path?q=1 and [named](https://example.org \"title\") and www.example.net.", replies.id, u1.id],
  ["https://example.com/onebox-candidate\n\nText after.", replies.id, u1.id],
  ["![alt text|100x200](upload://abcdefghij1234567890.png)\n\n![plain](https://example.com/a.png)", replies.id, u1.id],
  ["[file.pdf|attachment](upload://zyxwvutsrq0987654321.pdf) (12 KB)", replies.id, u1.id],
  ["Text with \"quotes\", 'single', dashes -- and --- and ellipsis... (c) (tm) 1/2.", replies.id, u1.id],
  ["<b>bold html</b> <script>alert(1)</script> <img src=x onerror=alert(1)> <kbd>Ctrl</kbd> <span style=\"color:red\">x</span>", replies.id, u1.id],
  ["<div align=\"center\">centered</div>\n\n<details>\n<summary>Summary</summary>\nHidden text\n</details>", replies.id, u1.id],
  ["<iframe src=\"https://www.google.com/maps/embed?pb=1\" width=\"600\" height=\"450\"></iframe>\n\n<iframe src=\"https://evil.example.com/\"></iframe>", replies.id, u1.id],
  ["[details=\"Click me\"]\nInside details with **bold**.\n[/details]", replies.id, u1.id],
  ["[spoiler]secret text[/spoiler] and inline [spoiler]x[/spoiler].", replies.id, u1.id],
  ["[poll type=regular results=always]\n* Option A\n* Option B\n[/poll]", replies.id, u1.id],
  ["[date=2026-10-02 time=10:00:00 timezone=\"Europe/Paris\"]", replies.id, u1.id],
  ["- [ ] unchecked\n- [x] checked\n- [ ] another", replies.id, u1.id],
  ["Footnote here[^1] and inline^[an inline note].\n\n[^1]: The footnote text.", replies.id, u1.id],
  ["Line one\nLine two with a soft break\\\nhard break\n\nNew paragraph.", replies.id, u1.id],
  ["[b]bbcode bold[/b] [i]italic[/i] [u]underline[/u] [s]strike[/s] [code]inline[/code]", replies.id, u1.id],
  ["[code]\nblock bbcode code\n  indented\n[/code]", replies.id, u1.id],
  ["Math $x^2 + y^2$ and\n\n$$\n\\int_0^1 x dx\n$$", replies.id, u1.id],
  ["```mermaid\ngraph TD; A-->B;\n```", replies.id, u1.id],
  ["Text <!-- a comment --> after comment.\n\n&amp; &lt; &copy; &#35; entities.", replies.id, u1.id],
  ["Term\n: Definition\n\nH~2~O and x^2^ and ==marked== and ++inserted++.", replies.id, u1.id],
  ["#{Discourse.base_url}/t/#{replies.slug}/#{replies.id}\n\n#{Discourse.base_url}/t/#{replies.slug}/#{replies.id}/2", replies.id, u1.id],
  ["a\n\n\n\nb with many blank lines\n\n   \n\nc", replies.id, u1.id],
  ["    \n\ttab indented\n* * *\n_ _ _\n+ plus list\n+ second", replies.id, u1.id],
  ["Setext heading\n==============\n\nAnother\n-------", replies.id, u1.id],
  ["Autolink <https://example.com> and email <a@example.com> and a@example.com plain.", replies.id, u1.id],
  ["Very long " + ("word " * 400), replies.id, u1.id],
  ["RTL text: مرحبا بالعالم and CJK: 你好世界 こんにちは 한국어.", replies.id, u1.id],
  ["Hidden direction marks in code: `a\u202Eb` and\n\n```\nx\u2066y\n```", replies.id, u1.id],
  ["Links: [here](/latest), [there](http://localhost:3042/about), [away](https://example.com/x), <a href=\"https://example.org\" target=\"_blank\">blank</a>.", replies.id, u1.id],
  ["Mentions in a list:\n\n- @user0\n- @admins @moderators @everyone\n- @USER1 and @trust_level_0", replies.id, u1.id],
  ["[wrap=foo bar=1]\nwrapped block\n[/wrap]\n\ninline [wrap=x]y[/wrap]", replies.id, u1.id],
  ["[grid]\n![a](upload://aaaaaaaaaaaaaaaaaaaa.png)\n![b](upload://bbbbbbbbbbbbbbbbbbbb.png)\n[/grid]", replies.id, u1.id],
  ["![video|video](upload://cccccccccccccccccccc.mp4)\n\n![audio|audio](upload://dddddddddddddddddddd.mp3)", replies.id, u1.id],
]
if upload
  samples << ["An upload that exists: ![image|64x64](#{upload.short_url}) and [a link|attachment](#{upload.short_url}).", replies.id, u1.id]
end
end

corpus = []
samples.each_with_index do |sample, i|
  raw, topic_id, user_id = sample
  corpus << { id: "sample-#{i}", raw: raw, topic_id: topic_id, user_id: user_id }
end
Post.with_deleted.order(:id).each do |p|
  corpus << { id: "post-#{p.id}", raw: p.raw, topic_id: p.topic_id, user_id: p.user_id }
end
corpus.each do |c|
  c[:markdown] = PrettyText.markdown(c[:raw].dup, topic_id: c[:topic_id], user_id: c[:user_id])
  # PrettyText.cook: the markdown, then PrettyText.cleanup.
  c[:cooked] = PrettyText.cook(c[:raw].dup, topic_id: c[:topic_id], user_id: c[:user_id])
end
File.write("#{out}/corpus.json", JSON.pretty_generate(corpus) + "\n")
calls = HELPER_CALLS.uniq { |c| [c[:method], c[:args]] }
File.write("#{out}/helpers.json", JSON.pretty_generate(calls) + "\n")
puts "#{corpus.size} corpus entries (#{samples.size} samples), #{calls.size} helper calls"
