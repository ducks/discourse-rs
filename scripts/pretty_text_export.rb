# Records what Rails cooks on a reference Discourse, for discourse-rs to be
# measured against (run by scripts/record-pretty-text through
# `bin/rails runner`): the options PrettyText.markdown hands the renderer
# on this site, and a corpus of inputs with PrettyText.markdown's output.
require "json"
require "fileutils"

out = ARGV[0]
abort "usage: pretty_text_export.rb <dir> [raws.json]" if out.blank?
# Inputs to cook instead of this site's samples and posts
# (scripts/fetch-discourse-corpus): [{id, source, raw, topic_id, user_id}].
raws_file = ARGV[1]
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

# What CookedPostProcessor makes of a post: the html Jobs::ProcessPost
# writes to the column when it differs. The processor also writes (the
# post's image, badges, its uploads) and enqueues jobs, so it runs in a
# transaction that is rolled back, with enqueueing stubbed out. Posts with
# oneboxes would be fetched from the network; none are processed then.
module Jobs
  def self.enqueue(*) = nil
  def self.enqueue_in(*) = nil
end
def post_processed(post)
  cooked = post.cook(post.raw.dup, topic_id: post.topic_id)
  return nil if cooked.match?(/class="onebox"|inline-onebox-loading/)
  # Jobs::ProcessPost skips a post whose topic is gone.
  return nil if post.topic.blank?
  html = nil
  ActiveRecord::Base.transaction do
    post.cooked = cooked
    processor = CookedPostProcessor.new(post, {})
    processor.post_process
    html = processor.html
    raise ActiveRecord::Rollback
  end
  html
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
  # Nothing follows a table on a new line (table_close ends with </div>):
  # a paragraph after one, and one at the end of a blockquote.
  ["| a | b |\n|---|---|\n| 1 | 2 |\n\nAfter the table.\n\n> | q |\n> |---|\n> | 1 |\n\nAfter the quote.", replies.id, u1.id],
  # An entity joins the text beside it (text_join) before emoji: not an emoji
  # alone, drawn large.
  ["[:question:&nbsp;**Support**](https://example.com/support) and :smile:&amp;", replies.id, u1.id],
  # Lines that are only an <img> are a paragraph (html_img), not an HTML
  # block; indented four spaces they are code, and they cannot interrupt one.
  ["Before.\n\n<img src=\"https://example.com/a.png\" width=\"10\">\n<IMG src=\"https://example.com/b.png\" />\n\nText\n<img src=\"https://example.com/c.png\">\n\n    <img src=\"https://example.com/d.png\">", replies.id, u1.id],
  # Smart quotes pair within one inline block, whose ends count as space: a
  # quote closing a tight list item or a table cell, an item's open quote
  # that the next item does not close, and image alt text left alone.
  ["- no new posts since \"last visit\"\n- an \"open one\n- closed\" here\n\n| Default |\n|---|\n| \"Upcoming events\" |\n\n![it's \"alt\"](https://example.com/a.png) it's \"done\"", replies.id, u1.id],
  # Mentions and hashtags skip text inside an html link (textReplace's
  # skipAllLinks), not inside other html.
  ["<a class=\"mention-group\">@staff</a>, <A href=\"https://example.com\">**@admins** #support</A> but @staff and <b>@admins</b>", replies.id, u1.id],
  # Typography where JS scopes it: a block whose source has no quote is
  # skipped though linkify decodes a %27 into one, an inline footnote's
  # text is a block of its own, and alt text gets no replacements.
  ["https://example.com/Capture%20d%27%C3%A9cran.png\n\n| a |\n|---|\n| x^[the \"calendar's\" view] |\n\n![wait... it's](https://example.com/a.png) and wait...", replies.id, u1.id],
  # A fence or html block right after a tight item's text follows it on the
  # same line; one opening an item, or a blockquote, starts a new line.
  ["- a\n  ```\n  x\n  ```\n  after\n- b\n  <div>\n  y\n  </div>\n- ```\n  z\n  ```\n- c\n  > q", replies.id, u1.id],
  # The space after an <img> line's last tag stays in its paragraph.
  ["<img src=\"https://example.com/a.png\"> \n<img src=\"https://example.com/b.png\">\t\n\ntext\n\n- <img src=\"https://example.com/c.png\">  \n- d", replies.id, u1.id],
]
if upload
  samples << ["An upload that exists: ![image|64x64](#{upload.short_url}) and [a link|attachment](#{upload.short_url}).", replies.id, u1.id]
end
end

corpus = []
if raws_file.present?
  JSON
    .parse(File.read(raws_file))
    .each do |r|
      corpus << {
        id: r["id"],
        source: r["source"],
        raw: r["raw"],
        topic_id: r["topic_id"],
        user_id: r["user_id"],
      }
    end
  samples = []
end
samples.each_with_index do |sample, i|
  raw, topic_id, user_id = sample
  corpus << { id: "sample-#{i}", raw: raw, topic_id: topic_id, user_id: user_id }
end
(raws_file.present? ? [] : Post.with_deleted.order(:id)).each do |p|
  # What Post#cook passes, and the column the post processor wrote.
  corpus << {
    id: "post-#{p.id}",
    raw: p.raw,
    topic_id: p.topic_id,
    user_id: p.user_id,
    post: {
      post_id: p.id,
      user_id: p.last_editor_id,
      omit_nofollow: p.omit_nofollow?,
      cook_method: p.cook_method,
      post_cook: p.cook(p.raw.dup, topic_id: p.topic_id),
      stored: p.cooked,
      processed: post_processed(p),
    },
  }
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
