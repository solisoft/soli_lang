# ============================================================================
# Cascade deletes: has_many/has_one dependent: "delete" | "delete_all" | "nullify"
# Cascades fire on HARD instance deletes (and Model.delete(id) on classes that
# declare dependents), after before_delete and before the owner row is removed.
# Soft-delete owners keep their children. Bulk writes never cascade.
# DSL-validation assertions run without a database; behavior assertions are
# gated behind the DB availability probe, matching model_instances_spec.sl.
# ============================================================================
class CascAuthor < Model
  has_many "casc_posts", dependent: "delete"
end

class CascPost < Model
  belongs_to "casc_author"
  has_many "casc_comments", dependent: "delete"
end

class CascComment < Model
  belongs_to "casc_post"
end

class CascProfileOwner < Model
  has_one "casc_profile", dependent: "delete"
end

class CascProfile < Model
  belongs_to "casc_profile_owner"
end

class CascBulkOwner < Model
  has_many "casc_bulk_items", dependent: "delete_all"
end

class CascBulkItem < Model
  belongs_to "casc_bulk_owner"

  before_delete("veto")

  def veto
    # delete_all must bypass callbacks entirely — if this veto ever ran,
    # per-row deletion would fail and the rows would survive.
    false
  end
end

class CascNullifyOwner < Model
  has_many "casc_nullify_items", dependent: "nullify"
end

class CascNullifyItem < Model
  belongs_to "casc_nullify_owner"
end

class CascSoftOwner < Model
  soft_delete
  has_many "casc_soft_children", dependent: "delete"
end

class CascSoftChild < Model
  belongs_to "casc_soft_owner"
end

class CascVetoParent < Model
  has_many "casc_veto_children", dependent: "delete"
end

class CascVetoChild < Model
  belongs_to "casc_veto_parent"

  before_delete("refuse")

  def refuse
    false
  end
end

# Self-referential cycle: two nodes that are each other's parent.
class CascNode < Model
  has_many("casc_nodes", {"dependent": "delete", "foreign_key": "parent_id"})
end

# Named-arg symbol value parses (class loading is the assertion).
class CascNamedArgOwner < Model
  has_many "casc_named_items", dependent: :delete_all
end

# Throwaway class for post-hoc DSL error assertions.
class CascDslProbe < Model
end

# Detect DB availability
let __db_available = false
try
  let __probe = CascAuthor.create({"name": "__probe__"})
  if !__probe.nil? && !__probe._errors
    __db_available = true
    __probe.delete()
  end
catch e
  __db_available = false
end

describe("dependent: option validation", fn() {
  test("all three strategies and the destroy alias parse", fn() {
    CascDslProbe.has_many("casc_probe_a", {"dependent": "delete"})
    CascDslProbe.has_many("casc_probe_b", {"dependent": "destroy"})
    CascDslProbe.has_many("casc_probe_c", {"dependent": "delete_all"})
    CascDslProbe.has_one("casc_probe_d", {"dependent": "nullify"})
    assert(true)
  })

  test("an unknown strategy raises naming the bad value", fn() {
    let raised = false
    try
      CascDslProbe.has_many("casc_probe_bad", {"dependent": "purge"})
    catch e
      raised = true
      assert(str(e).includes?("purge"))
    end
    assert(raised)
  })

  test("dependent: on belongs_to raises", fn() {
    let raised = false
    try
      CascDslProbe.belongs_to("casc_probe_parent", {"dependent": "delete"})
    catch e
      raised = true
      assert(str(e).includes?("has_many/has_one"))
    end
    assert(raised)
  })

  test("dependent: combined with through: raises", fn() {
    let raised = false
    try
      CascDslProbe.has_many("casc_probe_combo", {"dependent": "delete", "through": "casc_probe_a"})
    catch e
      raised = true
      assert(str(e).includes?("through"))
    end
    assert(raised)
  })
})

describe("dependent: \"delete\"", fn() {
  test("removes children and grandchildren through their callbacks", fn() {
    if __db_available
      let author = CascAuthor.create({"name": "a"})
      let post_one = CascPost.create({"casc_author_id": author._key, "title": "p1"})
      let post_two = CascPost.create({"casc_author_id": author._key, "title": "p2"})
      CascComment.create({"casc_post_id": post_one._key, "body": "c1"})
      CascComment.create({"casc_post_id": post_two._key, "body": "c2"})

      author.delete()

      assert_eq(CascPost.where("casc_author_id == @k", {"k": author._key}).count(), 0)
      assert_eq(CascComment.where("casc_post_id == @k", {"k": post_one._key}).count(), 0)
      assert_eq(CascComment.where("casc_post_id == @k", {"k": post_two._key}).count(), 0)
    end
  })

  test("has_one cascades too", fn() {
    if __db_available
      let owner = CascProfileOwner.create({"name": "o"})
      CascProfile.create({"casc_profile_owner_id": owner._key})

      owner.delete()

      assert_eq(CascProfile.where("casc_profile_owner_id == @k", {"k": owner._key}).count(), 0)
    end
  })

  test("a child before_delete veto aborts the owner delete", fn() {
    if __db_available
      let parent = CascVetoParent.create({"name": "p"})
      let child = CascVetoChild.create({"casc_veto_parent_id": parent._key})

      let raised = false
      try
        parent.delete()
      catch e
        raised = true
      end
      assert(raised)
      # Both rows survive: the cascade aborted before the owner row delete.
      assert_not_null(CascVetoParent.find_by("_key", parent._key))
      assert_not_null(CascVetoChild.find_by("_key", child._key))

      # Clean up (child veto blocks its instance delete; bypass via class bulk).
      CascVetoChild.where("_key == @k", {"k": child._key}).delete_all()
      parent.delete()
    end
  })

  test("a two-node parent cycle terminates", fn() {
    if __db_available
      let node_one = CascNode.create({"label": "n1"})
      let node_two = CascNode.create({"label": "n2", "parent_id": node_one._key})
      node_one.parent_id = node_two._key
      node_one.update()

      node_one.delete()

      assert_null(CascNode.find_by("_key", node_one._key))
      assert_null(CascNode.find_by("_key", node_two._key))
    end
  })
})

describe("dependent: \"delete_all\" and \"nullify\"", fn() {
  test("delete_all bulk-removes children without firing callbacks", fn() {
    if __db_available
      let owner = CascBulkOwner.create({"name": "b"})
      CascBulkItem.create({"casc_bulk_owner_id": owner._key})
      CascBulkItem.create({"casc_bulk_owner_id": owner._key})

      owner.delete()

      # Rows are gone even though CascBulkItem's before_delete always vetoes:
      # the bulk REMOVE never consults callbacks.
      assert_eq(CascBulkItem.where("casc_bulk_owner_id == @k", {"k": owner._key}).count(), 0)
    end
  })

  test("nullify clears the foreign key and keeps the rows", fn() {
    if __db_available
      let owner = CascNullifyOwner.create({"name": "n"})
      let item = CascNullifyItem.create({"casc_nullify_owner_id": owner._key, "tag": "casc-nullify"})

      owner.delete()

      let reloaded = CascNullifyItem.find(item._key)
      assert_null(reloaded.casc_nullify_owner_id)
      reloaded.delete()
    end
  })
})

describe("cascade boundaries", fn() {
  test("a soft-delete owner keeps its children", fn() {
    if __db_available
      let owner = CascSoftOwner.create({"name": "s"})
      let child = CascSoftChild.create({"casc_soft_owner_id": owner._key})

      owner.delete()  # soft delete: sets deleted_at, no cascade

      assert_eq(CascSoftChild.where("casc_soft_owner_id == @k", {"k": owner._key}).count(), 1)
      child.delete()
    end
  })

  test("the class form Model.delete(id) cascades", fn() {
    if __db_available
      let author = CascAuthor.create({"name": "cf"})
      let post = CascPost.create({"casc_author_id": author._key, "title": "cf-post"})

      CascAuthor.delete(author._key)

      assert_null(CascAuthor.find_by("_key", author._key))
      assert_eq(CascPost.where("casc_author_id == @k", {"k": author._key}).count(), 0)
    end
  })

  test("QueryBuilder delete_all never cascades", fn() {
    if __db_available
      let author = CascAuthor.create({"name": "qb"})
      let post = CascPost.create({"casc_author_id": author._key, "title": "orphan"})

      CascAuthor.where("_key == @k", {"k": author._key}).delete_all()

      assert_null(CascAuthor.find_by("_key", author._key))
      # The child survives as an orphan — bulk writes skip cascades.
      assert_eq(CascPost.where("casc_author_id == @k", {"k": author._key}).count(), 1)
      post.delete()
    end
  })
})
