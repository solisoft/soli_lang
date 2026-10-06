# Counter caches: belongs_to ..., counter_cache: true | "column" | :column.
# The child maintains a <children>_count column on its parent via the CAS
# increment loop. Bumps are best-effort and skip bulk writes;
# Model.reset_counters(id, relation) recounts.

class CcPost < Model
  has_many("cc_comments")
end

class CcComment < Model
  belongs_to("cc_post", counter_cache: true)
end

# Custom column name.
class CcArticle < Model
  has_many("cc_reviews")
end

class CcReview < Model
  belongs_to("cc_article", {"counter_cache": "review_tally"})
end

# Soft-deleting child: counters track default-scope-visible children.
class CcNotebook < Model
  has_many("cc_notes")
end

class CcNote < Model
  soft_delete
  belongs_to("cc_notebook", counter_cache: true)
end

# Column name given as a symbol.
class CcShelf < Model
  has_many("cc_volumes")
end

class CcVolume < Model
  belongs_to("cc_shelf", counter_cache: :volume_tally)
end

class CcDslProbe < Model
end

def comment_count(post)
  CcPost.find(post._key).cc_comments_count
end

describe("counter_cache: option validation") do
  test("counter_cache: on has_many is refused") do
    assert_raises("`counter_cache:` is only supported on belongs_to relations") do
      CcDslProbe.has_many("cc_probe_kids", {"counter_cache": true})
    end
  end

  test("a value that is neither true nor a column name is refused") do
    assert_raises("`counter_cache:` expects true or a column name") do
      CcDslProbe.belongs_to("cc_probe_bad", {"counter_cache": 42})
    end
  end
end

describe("counter cache maintenance") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    [CcPost, CcComment, CcArticle, CcReview, CcNotebook, CcNote, CcShelf, CcVolume].each do |model|
      model.delete_all()
    end
  end

  test("a parent without children has no counter yet") do
    assert_null(CcPost.create({"title": "empty"}).cc_comments_count)
  end

  test("create and delete keep the parent count current") do
    post = CcPost.create({"title": "p"})
    first = CcComment.create({"cc_post_id": post._key, "body": "one"})
    second = CcComment.create({"cc_post_id": post._key, "body": "two"})
    assert_eq(comment_count(post), 2)

    first.delete
    assert_eq(comment_count(post), 1)

    second.delete
    assert_eq(comment_count(post), 0)
  end

  test("save on a new instance increments too") do
    post = CcPost.create({"title": "p"})
    comment = CcComment.new({"cc_post_id": post._key, "body": "via save"})
    assert_eq(comment.save, true)
    assert_eq(comment_count(post), 1)
  end

  test("reassigning the foreign key moves the count between parents") do
    post_a = CcPost.create({"title": "a"})
    post_b = CcPost.create({"title": "b"})
    comment = CcComment.create({"cc_post_id": post_a._key, "body": "mover"})
    assert_eq(comment_count(post_a), 1)

    comment.cc_post_id = post_b._key
    comment.save
    assert_eq(comment_count(post_a), 0)
    assert_eq(comment_count(post_b), 1)
  end

  test("setting the foreign key to nil only decrements") do
    post = CcPost.create({"title": "p"})
    comment = CcComment.create({"cc_post_id": post._key, "body": "detach"})
    comment.cc_post_id = nil
    comment.update
    assert_eq(comment_count(post), 0)
  end

  test("the class form Model.update moves the count, Model.delete drops it") do
    post_a = CcPost.create({"title": "a"})
    post_b = CcPost.create({"title": "b"})
    comment = CcComment.create({"cc_post_id": post_a._key, "body": "cf"})

    CcComment.update(comment._key, {"cc_post_id": post_b._key, "body": "cf"})
    assert_eq(comment_count(post_a), 0)
    assert_eq(comment_count(post_b), 1)

    CcComment.delete(comment._key)
    assert_eq(comment_count(post_b), 0)
  end

  test("a custom column name given as a string is honored") do
    article = CcArticle.create({"title": "art"})
    CcReview.create({"cc_article_id": article._key, "stars": 5})
    found = CcArticle.find(article._key)
    assert_eq(found.review_tally, 1)
    assert_null(found.cc_reviews_count)
  end

  test("a custom column name given as a symbol is honored") do
    shelf = CcShelf.create({"name": "s"})
    CcVolume.create({"cc_shelf_id": shelf._key})
    assert_eq(CcShelf.find(shelf._key).volume_tally, 1)
  end

  test("soft delete decrements and restore re-increments") do
    notebook = CcNotebook.create({"name": "nb"})
    note = CcNote.create({"cc_notebook_id": notebook._key, "body": "n"})
    assert_eq(CcNotebook.find(notebook._key).cc_notes_count, 1)

    note.delete
    assert_eq(CcNotebook.find(notebook._key).cc_notes_count, 0)

    note.restore
    assert_eq(CcNotebook.find(notebook._key).cc_notes_count, 1)
  end

  test("bulk writes skip the bumps") do
    post = CcPost.create({"title": "bulk"})
    CcComment.create({"cc_post_id": post._key})
    CcComment.create_many([{"cc_post_id": post._key}])
    assert_eq(comment_count(post), 1)

    CcComment.where("cc_post_id == @k", {"k": post._key}).delete_all
    assert_eq(comment_count(post), 1)
  end

  test("reset_counters repairs drift and returns the fresh count") do
    post = CcPost.create({"title": "drift"})
    CcComment.create({"cc_post_id": post._key, "body": "1"})
    CcComment.create({"cc_post_id": post._key, "body": "2"})
    CcPost.update(post._key, {"cc_comments_count": 99})
    assert_eq(comment_count(post), 99)

    assert_eq(CcPost.reset_counters(post._key, "cc_comments"), 2)
    assert_eq(comment_count(post), 2)
  end

  test("reset_counters refuses an unknown relation and lists the known ones") do
    post = CcPost.create({"title": "p"})
    assert_raises("reset_counters: CcPost has no relation \"nope\" (available: cc_comments)") do
      CcPost.reset_counters(post._key, "nope")
    end
  end
end
