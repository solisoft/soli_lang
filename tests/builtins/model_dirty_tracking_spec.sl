# ============================================================================
# Dirty tracking: changed?, changed, changes, previous_changes, attribute_was
# The baseline snapshot is seeded when a record is loaded from or persisted
# to the database; a new (never-loaded) record reports every attribute as
# changed. In-memory assertions run without a database; persistence paths
# are gated behind the DB availability probe, matching model_instances_spec.sl.
# ============================================================================
class DirtyDoc < Model
end

class DirtyValidated < Model
  validates("name", {"presence": true})
end

# Detect DB availability
let __db_available = false
try
  let __probe = DirtyDoc.create({"name": "__probe__"})
  if !__probe.nil? && !__probe._errors
    __db_available = true
    __probe.delete()
  end
catch e
  __db_available = false
end

describe("dirty tracking on new records", fn() {
  test("a blank instance is clean", fn() {
    let doc = DirtyDoc.new({})
    assert_eq(doc.changed?, false)
    assert_eq(doc.changed, [])
    assert_eq(doc.changes, {})
  })

  test("mass-assigned attributes count as changes", fn() {
    let doc = DirtyDoc.new({"title": "Hello", "views": 3})
    assert(doc.changed?)
    assert_eq(doc.changed, ["title", "views"])
    let ch = doc.changes
    assert_eq(ch["title"], [null, "Hello"])
    assert_eq(ch["views"], [null, 3])
  })

  test("changed is sorted alphabetically", fn() {
    let doc = DirtyDoc.new({"zeta": 1, "alpha": 2})
    assert_eq(doc.changed, ["alpha", "zeta"])
  })

  test("direct assignment counts as a change", fn() {
    let doc = DirtyDoc.new({})
    doc.title = "assigned"
    assert(doc.changed?)
    assert_eq(doc.changed, ["title"])
  })

  test("attribute_was is null on a new record", fn() {
    let doc = DirtyDoc.new({"title": "x"})
    assert_null(doc.attribute_was("title"))
  })

  test("previous_changes is empty before any persist", fn() {
    let doc = DirtyDoc.new({"title": "x"})
    assert_eq(doc.previous_changes, {})
  })

  test("attribute_was rejects a non-string name", fn() {
    let doc = DirtyDoc.new({})
    let raised = false
    try
      doc.attribute_was(42)
    catch e
      raised = true
    end
    assert(raised)
  })
})

describe("dirty tracking across persistence", fn() {
  test("create leaves the record clean and fills previous_changes", fn() {
    if __db_available
      let doc = DirtyDoc.create({"title": "fresh", "views": 0})
      assert_eq(doc.changed?, false)
      assert_eq(doc.changed, [])
      let prev = doc.previous_changes
      assert_eq(prev["title"], [null, "fresh"])
      assert_eq(prev["views"], [null, 0])
      doc.delete()
    end
  })

  test("update records exactly the delta in previous_changes", fn() {
    if __db_available
      let doc = DirtyDoc.create({"title": "before", "views": 1})
      doc.title = "after"
      assert(doc.changed?)
      assert_eq(doc.changed, ["title"])
      assert_eq(doc.attribute_was("title"), "before")
      assert(doc.update())
      assert_eq(doc.changed?, false)
      let prev = doc.previous_changes
      assert_eq(prev["title"], ["before", "after"])
      assert_null(prev["views"])
      doc.delete()
    end
  })

  test("assigning an equal value stays clean on a loaded record", fn() {
    if __db_available
      let doc = DirtyDoc.create({"title": "same"})
      let found = DirtyDoc.find(doc._key)
      assert_eq(found.changed?, false)
      found.title = "same"
      assert_eq(found.changed?, false)
      doc.delete()
    end
  })

  test("records loaded with find start clean", fn() {
    if __db_available
      let doc = DirtyDoc.create({"title": "loaded"})
      let found = DirtyDoc.find(doc._key)
      assert_eq(found.changed?, false)
      found.title = "edited"
      assert_eq(found.changed, ["title"])
      assert_eq(found.attribute_was("title"), "loaded")
      doc.delete()
    end
  })

  test("a failed validation keeps the record dirty", fn() {
    if __db_available
      let doc = DirtyValidated.create({"name": "valid"})
      doc.name = ""
      assert_eq(doc.update(), false)
      assert(doc.changed?)
      assert_eq(doc.attribute_was("name"), "valid")
      doc.delete()
    end
  })

  test("save on an existing record resets dirty state", fn() {
    if __db_available
      let doc = DirtyDoc.create({"title": "v1"})
      doc.title = "v2"
      assert(doc.save())
      assert_eq(doc.changed?, false)
      let prev = doc.previous_changes
      assert_eq(prev["title"], ["v1", "v2"])
      doc.delete()
    end
  })

  test("reload clears pending changes", fn() {
    if __db_available
      let doc = DirtyDoc.create({"title": "stored"})
      doc.title = "unsaved edit"
      assert(doc.changed?)
      doc.reload()
      assert_eq(doc.changed?, false)
      assert_eq(doc.title, "stored")
      doc.delete()
    end
  })

  test("increment does not leave the field dirty", fn() {
    if __db_available
      let doc = DirtyDoc.create({"title": "counted", "views": 1})
      doc.increment("views")
      assert_eq(doc.views, 2)
      assert_eq(doc.changed?, false)
      doc.delete()
    end
  })
})
