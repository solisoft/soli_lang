# Cascade deletes: has_many/has_one dependent: "delete" | "delete_all" | "nullify".
# Cascades fire on hard instance deletes (and Model.delete(id) on classes that
# declare dependents), after before_delete and before the owner row goes.
# Soft-delete owners keep their children; bulk writes never cascade. A bare
# `delete` cascades exactly like `delete()`.
class CascAuthor < Model
  has_many("casc_posts", dependent: "delete")
end

class CascPost < Model
  belongs_to("casc_author")
  has_many("casc_comments", dependent: "delete")
end

class CascComment < Model
  belongs_to("casc_post")
end

class CascProfileOwner < Model
  has_one("casc_profile", dependent: "delete")
end

class CascProfile < Model
  belongs_to("casc_profile_owner")
end

class CascBulkOwner < Model
  has_many("casc_bulk_items", dependent: "delete_all")
end

class CascBulkItem < Model
  belongs_to("casc_bulk_owner")
  before_delete("veto")

  def veto
    # delete_all must bypass callbacks entirely — if this veto ever ran,
    # per-row deletion would fail and the rows would survive.
    false
  end
end

class CascNullifyOwner < Model
  has_many("casc_nullify_items", dependent: "nullify")
end

class CascNullifyItem < Model
  belongs_to("casc_nullify_owner")
end

class CascSoftOwner < Model
  soft_delete
  has_many("casc_soft_children", dependent: "delete")
end

class CascSoftChild < Model
  belongs_to("casc_soft_owner")
end

class CascVetoParent < Model
  has_many("casc_veto_children", dependent: "delete")
end

class CascVetoChild < Model
  belongs_to("casc_veto_parent")
  before_delete("refuse")

  def refuse
    false
  end
end

# Self-referential cycle: two nodes that are each other's parent.
class CascNode < Model
  has_many(
    "casc_nodes",
    {"dependent": "delete", "foreign_key": "parent_id"}
  )
end

# A symbol strategy given as a named argument.
class CascNamedArgOwner < Model
  has_many("casc_named_items", dependent: :delete_all)
end

class CascNamedItem < Model
  belongs_to("casc_named_arg_owner")
end

# "destroy" is accepted as an alias of "delete".
class CascAliasOwner < Model
  has_many("casc_alias_items", dependent: "destroy")
end

class CascAliasItem < Model
  belongs_to("casc_alias_owner")
end

# Throwaway class for post-hoc DSL error assertions.
class CascDslProbe < Model
end

const CASCADE_MODELS = [
  CascAuthor, CascPost, CascComment, CascProfileOwner, CascProfile, CascBulkOwner, CascBulkItem,
  CascNullifyOwner, CascNullifyItem, CascSoftOwner, CascSoftChild, CascVetoParent, CascVetoChild,
  CascNode, CascNamedArgOwner, CascNamedItem, CascAliasOwner, CascAliasItem
]

describe("dependent: option validation") do
  test("every strategy and the destroy alias declare the relation") do
    CascDslProbe.has_many("casc_probe_a", {"dependent": "delete"})
    CascDslProbe.has_many("casc_probe_b", {"dependent": "destroy"})
    CascDslProbe.has_many("casc_probe_c", {"dependent": "delete_all"})
    CascDslProbe.has_one("casc_probe_d", {"dependent": "nullify"})
    probe = CascDslProbe.new({})
    # An unsaved owner's has_many accessor matches nothing.
    assert_eq(probe.casc_probe_a.to_query, "FOR doc IN casc_probe_a FILTER 1 == 0 RETURN doc")
    assert_eq(probe.casc_probe_c.to_query, "FOR doc IN casc_probe_c FILTER 1 == 0 RETURN doc")
  end

  test("an unknown strategy is refused, naming the bad value") do
    assert_raises("`dependent:` expects \"delete\", \"delete_all\" or \"nullify\", got \"purge\"") do
      CascDslProbe.has_many("casc_probe_bad", {"dependent": "purge"})
    end
  end

  test("dependent: on belongs_to is refused") do
    assert_raises("`dependent:` is only supported on has_many/has_one relations") do
      CascDslProbe.belongs_to("casc_probe_parent", {"dependent": "delete"})
    end
  end

  test("dependent: combined with through: is refused") do
    assert_raises("`dependent:` cannot be combined with `through:`") do
      CascDslProbe.has_many("casc_probe_combo", {"dependent": "delete", "through": "casc_probe_a"})
    end
  end
end

describe("cascades against SoliDB") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    CASCADE_MODELS.each do |model|
      model.delete_all()
    end
  end

  describe("dependent: \"delete\"") do
    test("removes children and grandchildren") do
      author = CascAuthor.create({"name": "a"})
      post_one = CascPost.create({"casc_author_id": author._key, "title": "p1"})
      post_two = CascPost.create({"casc_author_id": author._key, "title": "p2"})
      CascComment.create({"casc_post_id": post_one._key, "body": "c1"})
      CascComment.create({"casc_post_id": post_two._key, "body": "c2"})
      CascPost.create({"casc_author_id": "someone-else", "title": "unrelated"})

      author.delete()

      assert_eq(CascAuthor.count, 0)
      assert_eq(CascPost.all.map { |post| post.title }, ["unrelated"])
      assert_eq(CascComment.count, 0)
    end

    test("has_one cascades too") do
      owner = CascProfileOwner.create({"name": "o"})
      CascProfile.create({"casc_profile_owner_id": owner._key})

      owner.delete()

      assert_eq(CascProfile.count, 0)
    end

    test("the destroy alias cascades like delete") do
      owner = CascAliasOwner.create({"name": "alias"})
      CascAliasItem.create({"casc_alias_owner_id": owner._key})

      owner.delete()

      assert_eq(CascAliasItem.count, 0)
    end

    test("a child before_delete veto aborts the owner delete") do
      parent = CascVetoParent.create({"name": "p"})
      child = CascVetoChild.create({"casc_veto_parent_id": parent._key})

      assert_raises("dependent: \"delete\" aborted: child casc_veto_children/#{child._key} could not be deleted") do
        parent.delete()
      end
      assert_eq(CascVetoParent.find_by("_key", parent._key).name, "p")
      assert_eq(CascVetoChild.find_by("_key", child._key).casc_veto_parent_id, parent._key)
    end

    test("a two-node parent cycle terminates") do
      node_one = CascNode.create({"label": "n1"})
      CascNode.create({"label": "n2", "parent_id": node_one._key})
      node_one.parent_id = CascNode.find_by("label", "n2")._key
      node_one.update()

      node_one.delete()

      assert_eq(CascNode.count, 0)
    end

    test("a bare delete cascades too") do
      author = CascAuthor.create({"name": "bare"})
      CascPost.create({"casc_author_id": author._key, "title": "orphaned"})
      author.delete
      assert_eq(CascPost.count, 0)
    end
  end

  describe("dependent: \"delete_all\" and \"nullify\"") do
    test("delete_all bulk-removes children without firing their callbacks") do
      owner = CascBulkOwner.create({"name": "b"})
      CascBulkItem.create({"casc_bulk_owner_id": owner._key})
      CascBulkItem.create({"casc_bulk_owner_id": owner._key})

      owner.delete()

      # Gone even though CascBulkItem's before_delete always vetoes: the bulk
      # REMOVE never consults callbacks.
      assert_eq(CascBulkItem.count, 0)
    end

    test("a symbol strategy given as a named argument works") do
      owner = CascNamedArgOwner.create({"name": "n"})
      CascNamedItem.create({"casc_named_arg_owner_id": owner._key})

      owner.delete()

      assert_eq(CascNamedItem.count, 0)
    end

    test("nullify clears the foreign key and keeps the rows") do
      owner = CascNullifyOwner.create({"name": "n"})
      item = CascNullifyItem.create({"casc_nullify_owner_id": owner._key, "tag": "casc-nullify"})

      owner.delete()

      reloaded = CascNullifyItem.find(item._key)
      assert_eq(reloaded.tag, "casc-nullify")
      assert_null(reloaded.casc_nullify_owner_id)
    end
  end

  describe("cascade boundaries") do
    test("a soft-delete owner keeps its children") do
      owner = CascSoftOwner.create({"name": "s"})
      CascSoftChild.create({"casc_soft_owner_id": owner._key})

      owner.delete()

      assert_eq(CascSoftOwner.find(owner._key).deleted_at.present?, true)
      assert_eq(CascSoftChild.where("casc_soft_owner_id == @k", {"k": owner._key}).count, 1)
    end

    test("the class form Model.delete(id) cascades") do
      author = CascAuthor.create({"name": "cf"})
      CascPost.create({"casc_author_id": author._key, "title": "cf-post"})

      CascAuthor.delete(author._key)

      assert_null(CascAuthor.find_by("_key", author._key))
      assert_eq(CascPost.count, 0)
    end

    test("QueryBuilder delete_all never cascades") do
      author = CascAuthor.create({"name": "qb"})
      CascPost.create({"casc_author_id": author._key, "title": "orphan"})

      CascAuthor.where("_key == @k", {"k": author._key}).delete_all

      assert_null(CascAuthor.find_by("_key", author._key))
      assert_eq(CascPost.where("casc_author_id == @k", {"k": author._key}).count, 1)
    end
  end
end
