# Polymorphic associations:
#   belongs_to "commentable", polymorphic: true    (child: {name}_id + {name}_type)
#   has_many "comments", as: "commentable"          (type-guarded inverse)
# The child accessor resolves the target class from the type field at runtime.
# Eager-loading a polymorphic belongs_to raises (the collection varies per
# row); the as: inverse eager-loads fine.

class PolyComment < Model
  belongs_to(
    "poly_commentable",
    {"polymorphic": true, "counter_cache": true}
  )
end

class PolyPost < Model
  has_many(
    "poly_comments",
    {"as": "poly_commentable", "dependent": "delete_all"}
  )
end

class PolyPhoto < Model
  has_many("poly_comments", {"as": "poly_commentable"})
end

class PolyProduct < Model
  has_one("poly_image", {"as": "poly_imageable"})
end

class PolyImage < Model
  belongs_to("poly_imageable", polymorphic: true)
end

class PolyNullifyOwner < Model
  has_many(
    "poly_tags",
    {"as": "poly_taggable", "dependent": "nullify"}
  )
end

class PolyTag < Model
  belongs_to("poly_taggable", polymorphic: true)
end

class PolyDslProbe < Model
end

const NOT_EAGER = "a polymorphic belongs_to can't be eager-loaded or join-filtered"

# The SDBQL text of a builder, without the bind-variable dump that follows it.
def query_of(builder)
  builder.to_query.split(" | bind_vars:")[0]
end

def comment_on(parent, body)
  PolyComment.create({"body": body, "poly_commentable_id": parent._key, "poly_commentable_type": parent.class})
end

describe("polymorphic DSL validation") do
  test("polymorphic: true on has_many is refused") do
    assert_raises("`polymorphic:` is only supported on belongs_to relations") do
      PolyDslProbe.has_many("poly_things", {"polymorphic": true})
    end
  end

  test("as: on belongs_to is refused") do
    assert_raises("`as:` is only supported on has_many/has_one relations") do
      PolyDslProbe.belongs_to("poly_thing", {"as": "poly_taggable"})
    end
  end

  test("polymorphic: true with class_name is refused") do
    assert_raises("`polymorphic: true` cannot be combined with `class_name:`") do
      PolyDslProbe.belongs_to("poly_ref", {"polymorphic": true, "class_name": "PolyPost"})
    end
  end

  test("polymorphic: with a non-boolean is refused") do
    assert_raises("`polymorphic:` expects true or false") do
      PolyDslProbe.belongs_to("poly_ref2", {"polymorphic": "yes"})
    end
  end
end

describe("polymorphic query shapes") do
  test("includes of an as: relation carries the type guard in the subquery") do
    expected = "FOR doc IN poly_posts LET _rel_poly_comments = (FOR rel IN poly_comments " +
      "FILTER rel.poly_commentable_id == doc._key AND rel.poly_commentable_type == \"PolyPost\" RETURN rel) " +
      "RETURN MERGE(doc, {poly_comments: _rel_poly_comments})"
    assert_eq(query_of(PolyPost.includes("poly_comments")), expected)
  end

  test("eager-loading a polymorphic belongs_to is refused") do
    assert_raises(NOT_EAGER) do
      PolyComment.includes("poly_commentable")
    end
  end

  test("joining on a polymorphic belongs_to is refused") do
    assert_raises(NOT_EAGER) do
      PolyComment.join("poly_commentable")
    end
  end
end

describe("polymorphic runtime behavior") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    [PolyComment, PolyPost, PolyPhoto, PolyProduct, PolyImage, PolyNullifyOwner, PolyTag].each do |model|
      model.delete_all()
    end
  end

  test("the as: inverse binds the owner's key and class") do
    post = PolyPost.create({"title": "shape probe"})
    query = post.poly_comments.to_query
    assert_contains(query, "FILTER doc.poly_commentable_id == @__rel_fk AND doc.poly_commentable_type == @__rel_type")
    bind_vars = query.split(" | bind_vars:")[1]
    assert_contains(bind_vars, "PolyPost")
    assert_contains(bind_vars, post._key)
  end

  test("the child accessor returns the right class per row") do
    post = PolyPost.create({"title": "a post"})
    photo = PolyPhoto.create({"caption": "a photo"})
    on_post = comment_on(post, "on the post")
    on_photo = comment_on(photo, "on the photo")

    assert(on_post.poly_commentable.is_a?("PolyPost"))
    assert_eq(on_post.poly_commentable.title, "a post")
    assert(on_photo.poly_commentable.is_a?("PolyPhoto"))
    assert_eq(on_photo.poly_commentable.caption, "a photo")
  end

  test("the accessor returns nil when type or id is missing") do
    assert_null(PolyComment.create({"body": "unattached"}).poly_commentable)
    assert_null(PolyComment.create({"body": "no type", "poly_commentable_id": "k"}).poly_commentable)
  end

  test("an unknown type string raises naming it") do
    bad = PolyComment.create({"body": "bad", "poly_commentable_id": "whatever", "poly_commentable_type": "NoSuchClass"})
    assert_raises("poly_commentable_type == \"NoSuchClass\" does not name a known model class") do
      bad.poly_commentable
    end
  end

  test("the inverse sees only its own typed children") do
    post = PolyPost.create({"title": "p"})
    photo = PolyPhoto.create({"caption": "ph"})
    comment_on(post, "c1")
    comment_on(photo, "c2")
    # Same key on a different type: only the type guard separates them.
    PolyComment.create({"body": "c3", "poly_commentable_id": post._key, "poly_commentable_type": "PolyPhoto"})

    assert_eq(post.poly_comments.all.map { |comment| comment.body }, ["c1"])
    assert_eq(photo.poly_comments.all.map { |comment| comment.body }, ["c2"])
  end

  test("includes loads only the owner's typed children") do
    post = PolyPost.create({"title": "eager"})
    comment_on(post, "mine")
    PolyComment.create({"body": "other type", "poly_commentable_id": post._key, "poly_commentable_type": "PolyPhoto"})
    loaded = PolyPost.includes("poly_comments").first
    assert_eq(loaded.poly_comments.map { |comment| comment["body"] }, ["mine"])
  end

  test("has_one with as: resolves through the type guard") do
    product = PolyProduct.create({"name": "widget"})
    PolyImage.create({"url": "/w.png", "poly_imageable_id": product._key, "poly_imageable_type": "PolyProduct"})
    assert_eq(product.poly_image.url, "/w.png")
    assert_null(PolyProduct.create({"name": "bare"}).poly_image)
  end

  describe("counter caches") do
    test("each parent type counts its own children") do
      post = PolyPost.create({"title": "counted post"})
      photo = PolyPhoto.create({"caption": "counted photo"})
      comment_on(post, "1")
      comment_on(post, "2")
      comment_on(photo, "3")
      assert_eq(PolyPost.find(post._key).poly_comments_count, 2)
      assert_eq(PolyPhoto.find(photo._key).poly_comments_count, 1)
    end

    test("retargeting across types moves the count between collections") do
      post = PolyPost.create({"title": "from"})
      photo = PolyPhoto.create({"caption": "to"})
      moved = comment_on(post, "moves")
      moved.poly_commentable_id = photo._key
      moved.poly_commentable_type = "PolyPhoto"
      moved.save
      assert_eq(PolyPost.find(post._key).poly_comments_count, 0)
      assert_eq(PolyPhoto.find(photo._key).poly_comments_count, 1)
    end

    test("delete decrements the owner's count") do
      photo = PolyPhoto.create({"caption": "shrinks"})
      comment_on(photo, "stays")
      comment_on(photo, "goes").delete
      assert_eq(PolyPhoto.find(photo._key).poly_comments_count, 1)
    end

    test("reset_counters recounts with the type guard") do
      post = PolyPost.create({"title": "recount"})
      comment_on(post, "real")
      PolyComment.create({"body": "other type", "poly_commentable_id": post._key, "poly_commentable_type": "PolyPhoto"})
      PolyPost.update(post._key, {"poly_comments_count": 42})
      assert_eq(PolyPost.reset_counters(post._key, "poly_comments"), 1)
      assert_eq(PolyPost.find(post._key).poly_comments_count, 1)
    end
  end

  describe("dependent cascades") do
    test("delete_all on an as: relation removes only the owner's children") do
      post = PolyPost.create({"title": "cascading"})
      photo = PolyPhoto.create({"caption": "surviving"})
      comment_on(post, "goes")
      survivor = comment_on(photo, "stays")

      post.delete()

      assert_eq(PolyComment.where("poly_commentable_type == @t", {"t": "PolyPost"}).count, 0)
      assert_eq(PolyComment.find_by("_key", survivor._key).body, "stays")
    end

    test("nullify clears both the foreign key and the type field") do
      owner = PolyNullifyOwner.create({"name": "o"})
      tag = PolyTag.create({"label": "t", "poly_taggable_id": owner._key, "poly_taggable_type": "PolyNullifyOwner"})

      owner.delete()

      reloaded = PolyTag.find(tag._key)
      assert_eq(reloaded.label, "t")
      assert_null(reloaded.poly_taggable_id)
      assert_null(reloaded.poly_taggable_type)
    end
  end
end
