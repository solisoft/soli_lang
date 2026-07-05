# ============================================================================
# Counter caches: belongs_to ..., counter_cache: true | "column"
# The child maintains a <children>_count column on its parent via the CAS
# increment loop. Bumps are best-effort and skip bulk writes;
# Model.reset_counters(id, relation) recounts. DSL-validation assertions run
# without a database; behavior assertions are gated behind the DB probe.
# ============================================================================
class CcPost < Model
  has_many "cc_comments"
end

class CcComment < Model
  belongs_to "cc_post", counter_cache: true
end

# Custom column name.
class CcArticle < Model
  has_many "cc_reviews"
end

class CcReview < Model
  belongs_to("cc_article", {"counter_cache": "review_tally"})
end

# Soft-deleting child: counters track default-scope-visible children.
class CcNotebook < Model
  has_many "cc_notes"
end

class CcNote < Model
  soft_delete
  belongs_to "cc_notebook", counter_cache: true
end

class CcDslProbe < Model
end

# Detect DB availability
let __db_available = false
try
  let __probe = CcPost.create({"title": "__probe__"})
  if !__probe.nil? && !__probe._errors
    __db_available = true
    __probe.delete()
  end
catch e
  __db_available = false
end

describe("counter_cache: option validation", fn() {
  test("true, string, and symbol forms parse", fn() {
    CcDslProbe.belongs_to("cc_probe_a", {"counter_cache": true})
    CcDslProbe.belongs_to("cc_probe_b", {"counter_cache": "my_tally"})
    CcDslProbe.belongs_to("cc_probe_c", {"counter_cache": :tally_col})
    assert(true)
  })

  test("counter_cache: on has_many raises", fn() {
    let raised = false
    try
      CcDslProbe.has_many("cc_probe_kids", {"counter_cache": true})
    catch e
      raised = true
      assert(str(e).includes?("belongs_to"))
    end
    assert(raised)
  })

  test("a non-boolean, non-string value raises", fn() {
    let raised = false
    try
      CcDslProbe.belongs_to("cc_probe_bad", {"counter_cache": 42})
    catch e
      raised = true
      assert(str(e).includes?("true or a column name"))
    end
    assert(raised)
  })
})

describe("counter cache maintenance", fn() {
  test("create and delete keep the parent count current", fn() {
    if __db_available
      let post = CcPost.create({"title": "p"})
      let c1 = CcComment.create({"cc_post_id": post._key, "body": "one"})
      let c2 = CcComment.create({"cc_post_id": post._key, "body": "two"})
      assert_eq(CcPost.find(post._key).cc_comments_count, 2)

      c1.delete()
      assert_eq(CcPost.find(post._key).cc_comments_count, 1)

      c2.delete()
      assert_eq(CcPost.find(post._key).cc_comments_count, 0)
      post.delete()
    end
  })

  test("save on a new instance increments too", fn() {
    if __db_available
      let post = CcPost.create({"title": "p"})
      let comment = CcComment.new({"cc_post_id": post._key, "body": "via save"})
      comment.save()
      assert_eq(CcPost.find(post._key).cc_comments_count, 1)
      comment.delete()
      post.delete()
    end
  })

  test("FK reassignment moves the count between parents", fn() {
    if __db_available
      let post_a = CcPost.create({"title": "a"})
      let post_b = CcPost.create({"title": "b"})
      let comment = CcComment.create({"cc_post_id": post_a._key, "body": "mover"})
      assert_eq(CcPost.find(post_a._key).cc_comments_count, 1)

      comment.cc_post_id = post_b._key
      comment.save()
      assert_eq(CcPost.find(post_a._key).cc_comments_count, 0)
      assert_eq(CcPost.find(post_b._key).cc_comments_count, 1)

      comment.delete()
      post_a.delete(); post_b.delete()
    end
  })

  test("setting the FK to null only decrements", fn() {
    if __db_available
      let post = CcPost.create({"title": "p"})
      let comment = CcComment.create({"cc_post_id": post._key, "body": "detach"})
      comment.cc_post_id = null
      comment.update()
      assert_eq(CcPost.find(post._key).cc_comments_count, 0)
      comment.delete()
      post.delete()
    end
  })

  test("the class form Model.update moves the count", fn() {
    if __db_available
      let post_a = CcPost.create({"title": "a"})
      let post_b = CcPost.create({"title": "b"})
      let comment = CcComment.create({"cc_post_id": post_a._key, "body": "cf"})

      CcComment.update(comment._key, {"cc_post_id": post_b._key, "body": "cf"})
      assert_eq(CcPost.find(post_a._key).cc_comments_count, 0)
      assert_eq(CcPost.find(post_b._key).cc_comments_count, 1)

      CcComment.delete(comment._key)
      assert_eq(CcPost.find(post_b._key).cc_comments_count, 0)
      post_a.delete(); post_b.delete()
    end
  })

  test("a custom column name is honored", fn() {
    if __db_available
      let article = CcArticle.create({"title": "art"})
      let review = CcReview.create({"cc_article_id": article._key, "stars": 5})
      assert_eq(CcArticle.find(article._key).review_tally, 1)
      review.delete()
      article.delete()
    end
  })

  test("soft delete decrements and restore re-increments", fn() {
    if __db_available
      let notebook = CcNotebook.create({"name": "nb"})
      let note = CcNote.create({"cc_notebook_id": notebook._key, "body": "n"})
      assert_eq(CcNotebook.find(notebook._key).cc_notes_count, 1)

      note.delete()  # soft
      assert_eq(CcNotebook.find(notebook._key).cc_notes_count, 0)

      note.restore()
      assert_eq(CcNotebook.find(notebook._key).cc_notes_count, 1)

      CcNote.where("_key == @k", {"k": note._key}).delete_all()
      notebook.delete()
    end
  })

  test("reset_counters repairs drift", fn() {
    if __db_available
      let post = CcPost.create({"title": "drift"})
      CcComment.create({"cc_post_id": post._key, "body": "1"})
      CcComment.create({"cc_post_id": post._key, "body": "2"})

      # Corrupt the counter directly (bulk-style write, no bumps).
      CcPost.update(post._key, {"cc_comments_count": 99})
      assert_eq(CcPost.find(post._key).cc_comments_count, 99)

      let fresh = CcPost.reset_counters(post._key, "cc_comments")
      assert_eq(fresh, 2)
      assert_eq(CcPost.find(post._key).cc_comments_count, 2)

      let raised = false
      try
        CcPost.reset_counters(post._key, "nope")
      catch e
        raised = true
        assert(str(e).includes?("cc_comments"))
      end
      assert(raised)

      CcComment.where("cc_post_id == @k", {"k": post._key}).delete_all()
      post.delete()
    end
  })
})
