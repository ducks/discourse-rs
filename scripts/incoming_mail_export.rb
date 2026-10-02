# What Discourse makes of each sample in parity/incoming_mail/emails, for
# discourse-rs's mail parsing to be measured against (run by
# scripts/record-incoming-mail through `bin/rails runner`). Read-only: it
# builds Email::Receivers and asks them, without processing anything.
require "json"

input, output = ARGV
abort "usage: incoming_mail_export.rb <emails.json> <out.json>" if input.blank? || output.blank?

result =
  JSON
    .parse(File.read(input))
    .sort
    .to_h do |name, raw|
      r = Email::Receiver.new(raw)
      mail = r.mail
      from_email, from_name = r.send(:parse_from_field, mail)
      text_part = mail.multipart? ? mail.text_part : (mail.content_type.to_s["text/html"] ? nil : mail)
      html_part = mail.multipart? ? mail.html_part : (mail.content_type.to_s["text/html"] ? mail : nil)
      body, elided, format = r.select_body
      [
        name,
        {
          cleaned: Email::Cleaner.new(r.raw_email).execute,
          message_id: r.message_id,
          from: [from_email, from_name],
          to: Array.wrap(mail.to),
          cc: Array.wrap(mail.cc),
          subject: r.send(:subject),
          date: mail.date&.to_time&.utc&.iso8601,
          reply_message_ids: Email::Receiver.extract_reply_message_ids(mail, max_message_id_count: 5),
          destinations: r.send(:all_destinations).to_a,
          text: r.send(:fix_charset, text_part),
          html: r.send(:fix_charset, html_part),
          attachments: r.send(:attachments).size,
          auto_generated: !!r.send(:is_auto_generated?),
          body: body,
          elided: elided,
          format: format,
        },
      ]
    end

File.write(output, JSON.pretty_generate(result) + "\n")
puts "#{result.size} emails"
