# Run inside a reference dv agent by scripts/lightbox-fixtures, with the
# images of parity/screens/uploads copied to /tmp/lightbox-fixtures: a
# topic (9230) whose post (92300) shows two uploads (92300, 92301) in
# lightboxes, cooked and post-processed by Rails (CookedPostProcessor makes
# the thumbnails and the div.lightbox-wrapper markup). Idempotent: skips
# what exists. The thumbnails' optimized_images rows are moved to ids
# 92300 and up.
dir = "/tmp/lightbox-fixtures"
files = { 92_300 => "lightbox-one.jpeg", 92_301 => "lightbox-two.png" }

files.each do |id, name|
  next if Upload.exists?(id: id)
  path = File.join(dir, name)
  width, height = FastImage.size(path)
  upload =
    Upload.new(
      id: id,
      user_id: 1,
      original_filename: name,
      filesize: File.size(path),
      width: width,
      height: height,
      sha1: Upload.generate_digest(path),
      extension: File.extname(name).delete_prefix("."),
    )
  upload.url = Discourse.store.store_upload(File.open(path), upload)
  upload.save!(validate: false)
end

unless Post.exists?(id: 92_300)
  one, two = files.keys.map { |id| Upload.find(id) }
  raw = <<~RAW.strip
    Two photos in lightboxes:

    ![Lightbox one|#{one.width}x#{one.height}](#{one.short_url})

    ![Lightbox two|#{two.width}x#{two.height}](#{two.short_url})
  RAW
  DB.exec(<<~SQL, raw: raw)
    INSERT INTO topics (id, title, fancy_title, slug, user_id, last_post_user_id, archetype, category_id, created_at, updated_at, bumped_at, last_posted_at, posts_count, highest_post_number, visible)
    VALUES (9230, 'Images in lightboxes for the gallery', 'Images in lightboxes for the gallery', 'images-in-lightboxes-for-the-gallery', 1, 1, 'regular', 4, now() - interval '1 hour', now() - interval '1 hour', now() - interval '1 hour', now() - interval '1 hour', 1, 1, true);
    INSERT INTO posts (id, topic_id, user_id, post_number, raw, cooked, created_at, updated_at, last_version_at, sort_order)
    VALUES (92300, 9230, 1, 1, :raw, '', now() - interval '1 hour', now() - interval '1 hour', now() - interval '1 hour', 1);
  SQL
end

post = Post.find(92_300)
unless post.cooked.include?("lightbox-wrapper")
  post.update_columns(cooked: post.cook(post.raw, topic_id: post.topic_id))
  processor = CookedPostProcessor.new(post.reload)
  processor.post_process
  # The runner has no request, so the urls name the agent's host
  # (//localhost:<port>); relative, they work on either side.
  cooked = processor.html.gsub(%r{//localhost:\d+/uploads/}, "/uploads/")
  post.update_columns(cooked: cooked, baked_at: Time.zone.now, baked_version: Post::BAKED_VERSION)
end

# Without the topic's image (update_post_image), whose thumbnails the
# topic page serializes and discourse-rs does not port yet.
Topic.where(id: 9230).update_all(image_upload_id: nil)
Post.where(id: 92_300).update_all(image_upload_id: nil)
TopicThumbnail.where(upload_id: files.keys).delete_all

OptimizedImage
  .where(upload_id: files.keys)
  .where("id < 92300")
  .order(:id)
  .each do |optimized|
    free = (92_300..92_399).find { |i| !OptimizedImage.exists?(id: i) }
    optimized.update_columns(id: free)
  end
