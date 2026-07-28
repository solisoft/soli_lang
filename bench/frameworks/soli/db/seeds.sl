# Exactly 50 rows, deterministic content — mirrored byte-for-byte in the Rails app.
for existing in Post.all() {
  Post.delete(existing._key)
}
for i in 0..50 {
  Post.create({ "id": i + 1, "title": "Post title #{i + 1}", "views": (i + 1) * 7 })
}
print("seeded: " + str(Post.all().length()))
