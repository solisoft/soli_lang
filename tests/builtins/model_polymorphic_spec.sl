# ============================================================================
# Polymorphic associations:
#   belongs_to "commentable", polymorphic: true    (child: {name}_id + {name}_type)
#   has_many "comments", as: "commentable"          (type-guarded inverse)
# The child accessor resolves the target class/collection from the type field
# at runtime. Eager-loading a polymorphic belongs_to raises (per-row dynamic
# collections don't fit one query) — the as: inverse eager-loads fine.
# DSL-validation and .to_query assertions run without a database; behavior
# is gated behind the DB availability probe.
# ============================================================================
class PolyComment < Model
  belongs_to("poly_commentable", {"polymorphic": true, "counter_cache": true})
end

class PolyPost < Model
  has_many("poly_comments", {"as": "poly_commentable", "dependent": "delete_all"})
end

class PolyPhoto < Model
  has_many("poly_comments", {"as": "poly_commentable"})
end

class PolyProduct < Model
  has_one("poly_image", {"as": "poly_imageable"})
end

class PolyImage < Model
  belongs_to "poly_imageable", polymorphic: true
end

class PolyNullifyOwner < Model
  has_many("poly_tags", {"as": "poly_taggable", "dependent": "nullify"})
end

class PolyTag < Model
  belongs_to "poly_taggable", polymorphic: true
end

class PolyDslProbe < Model
end

# Detect DB availability
let __db_available = false
try
  let __probe = PolyPost.create({"title": "__probe__"})
  if !__probe.nil? && !__probe._errors
    __db_available = true
    __probe.delete()
  end
catch e
  __db_available = false
end

describe("polymorphic DSL validation", fn() {
  test("polymorphic: true on has_many raises", fn() {
    let raised = false
    try
      PolyDslProbe.has_many("poly_things", {"polymorphic": true})
    catch e
      raised = true
      assert(str(e).includes?("belongs_to"))
    end
    assert(raised)
  })

  test("as: on belongs_to raises", fn() {
    let raised = false
    try
      PolyDslProbe.belongs_to("poly_thing", {"as": "poly_taggable"})
    catch e
      raised = true
      assert(str(e).includes?("has_many/has_one"))
    end
    assert(raised)
  })

  test("polymorphic: true with class_name raises", fn() {
    let raised = false
    try
      PolyDslProbe.belongs_to("poly_ref", {"polymorphic": true, "class_name": "PolyPost"})
    catch e
      raised = true
      assert(str(e).includes?("class_name"))
    end
    assert(raised)
  })

  test("polymorphic: with a non-boolean raises", fn() {
    let raised = false
    try
      PolyDslProbe.belongs_to("poly_ref2", {"polymorphic": "yes"})
    catch e
      raised = true
      assert(str(e).includes?("true or false"))
    end
    assert(raised)
  })
})

describe("polymorphic query shapes", fn() {
  test("the as: inverse carries the type guard", fn() {
    if __db_available
      let post = PolyPost.create({"title": "shape probe"})
      let q = post.poly_comments.to_query
      assert(q.includes?("poly_commentable_id == @__rel_fk"))
      assert(q.includes?("poly_commentable_type == @__rel_type"))
      post.delete()
    end
  })

  test("includes of an as: relation carries the type guard in the subquery", fn() {
    let q = PolyPost.includes("poly_comments").to_query
    assert(q.includes?("rel.poly_commentable_id == doc._key"))
    assert(q.includes?("rel.poly_commentable_type == \"PolyPost\""))
  })

  test("eager-loading a polymorphic belongs_to raises", fn() {
    let raised = false
    try
      PolyComment.includes("poly_commentable")
    catch e
      raised = true
      assert(str(e).includes?("polymorphic"))
    end
    assert(raised)
  })

  test("joining on a polymorphic belongs_to raises", fn() {
    let raised = false
    try
      PolyComment.join("poly_commentable")
    catch e
      raised = true
      assert(str(e).includes?("polymorphic"))
    end
    assert(raised)
  })
})

describe("polymorphic runtime behavior", fn() {
  test("the child accessor returns the right class per row", fn() {
    if __db_available
      let post = PolyPost.create({"title": "a post"})
      let photo = PolyPhoto.create({"caption": "a photo"})
      let on_post = PolyComment.create({
        "body": "on the post",
        "poly_commentable_id": post._key,
        "poly_commentable_type": "PolyPost"
      })
      let on_photo = PolyComment.create({
        "body": "on the photo",
        "poly_commentable_id": photo._key,
        "poly_commentable_type": "PolyPhoto"
      })

      assert_eq(on_post.poly_commentable.title, "a post")
      assert_eq(on_photo.poly_commentable.caption, "a photo")

      on_post.delete(); on_photo.delete()
      post.delete(); photo.delete()
    end
  })

  test("the accessor returns null when type or id is missing", fn() {
    if __db_available
      let orphan = PolyComment.create({"body": "unattached"})
      assert_null(orphan.poly_commentable)
      orphan.delete()
    end
  })

  test("an unknown type string raises naming it", fn() {
    if __db_available
      let bad = PolyComment.create({
        "body": "bad type",
        "poly_commentable_id": "whatever",
        "poly_commentable_type": "NoSuchClass"
      })
      let raised = false
      try
        bad.poly_commentable
      catch e
        raised = true
        assert(str(e).includes?("NoSuchClass"))
      end
      assert(raised)
      bad.delete()
    end
  })

  test("the inverse sees only its own typed children", fn() {
    if __db_available
      let post = PolyPost.create({"title": "p"})
      let photo = PolyPhoto.create({"caption": "ph"})
      # Same parent key shape on purpose: only the type guard separates them.
      PolyComment.create({"body": "c1", "poly_commentable_id": post._key, "poly_commentable_type": "PolyPost"})
      PolyComment.create({"body": "c2", "poly_commentable_id": photo._key, "poly_commentable_type": "PolyPhoto"})

      assert_eq(post.poly_comments.count(), 1)
      assert_eq(photo.poly_comments.count(), 1)

      PolyComment.where("poly_commentable_type != @x", {"x": ""}).delete_all()
      post.delete(); photo.delete()
    end
  })

  test("has_one with as: resolves through the type guard", fn() {
    if __db_available
      let product = PolyProduct.create({"name": "widget"})
      PolyImage.create({
        "url": "/w.png",
        "poly_imageable_id": product._key,
        "poly_imageable_type": "PolyProduct"
      })
      assert_eq(product.poly_image.url, "/w.png")
      PolyImage.where("poly_imageable_id == @k", {"k": product._key}).delete_all()
      product.delete()
    end
  })

  test("counter caches bump per parent type", fn() {
    if __db_available
      let post = PolyPost.create({"title": "counted post"})
      let photo = PolyPhoto.create({"caption": "counted photo"})
      let c1 = PolyComment.create({"body": "1", "poly_commentable_id": post._key, "poly_commentable_type": "PolyPost"})
      let c2 = PolyComment.create({"body": "2", "poly_commentable_id": post._key, "poly_commentable_type": "PolyPost"})
      let c3 = PolyComment.create({"body": "3", "poly_commentable_id": photo._key, "poly_commentable_type": "PolyPhoto"})

      assert_eq(PolyPost.find(post._key).poly_comments_count, 2)
      assert_eq(PolyPhoto.find(photo._key).poly_comments_count, 1)

      # Retargeting across TYPES moves the count between collections.
      c2.poly_commentable_id = photo._key
      c2.poly_commentable_type = "PolyPhoto"
      c2.save()
      assert_eq(PolyPost.find(post._key).poly_comments_count, 1)
      assert_eq(PolyPhoto.find(photo._key).poly_comments_count, 2)

      c3.delete()
      assert_eq(PolyPhoto.find(photo._key).poly_comments_count, 1)

      # reset_counters recounts with the type guard.
      PolyPost.update(post._key, {"poly_comments_count": 42})
      assert_eq(PolyPost.reset_counters(post._key, "poly_comments"), 1)

      c1.delete(); c2.delete()
      post.delete(); photo.delete()
    end
  })

  test("dependent: delete_all on an as: relation removes only own children", fn() {
    if __db_available
      let post = PolyPost.create({"title": "cascading"})
      let photo = PolyPhoto.create({"caption": "surviving"})
      PolyComment.create({"body": "goes", "poly_commentable_id": post._key, "poly_commentable_type": "PolyPost"})
      let survivor = PolyComment.create({"body": "stays", "poly_commentable_id": photo._key, "poly_commentable_type": "PolyPhoto"})

      post.delete()

      assert_eq(PolyComment.where("poly_commentable_type == @t", {"t": "PolyPost"}).count(), 0)
      assert_not_null(PolyComment.find_by("_key", survivor._key))

      survivor.delete()
      photo.delete()
    end
  })

  test("dependent: nullify clears both the fk and the type field", fn() {
    if __db_available
      let owner = PolyNullifyOwner.create({"name": "o"})
      let tag = PolyTag.create({
        "label": "t",
        "poly_taggable_id": owner._key,
        "poly_taggable_type": "PolyNullifyOwner"
      })

      owner.delete()

      let reloaded = PolyTag.find(tag._key)
      assert_null(reloaded.poly_taggable_id)
      assert_null(reloaded.poly_taggable_type)
      reloaded.delete()
    end
  })
})
