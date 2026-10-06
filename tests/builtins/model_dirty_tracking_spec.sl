# Dirty tracking: changed?, changed, changes, previous_changes, attribute_was.
# The baseline snapshot is seeded when a record is loaded from or persisted to
# the database; a new (never-loaded) record reports every attribute as changed.

class DirtyDoc < Model
end

class DirtyValidated < Model
  validates("name", {"presence": true})
end

describe("dirty tracking on new records") do
  test("a blank instance is clean") do
    doc = DirtyDoc.new({})

    assert_eq(doc.changed?, false)
    assert_eq(doc.changed, [])
    assert_eq(doc.changes, {})
  end

  test("mass-assigned attributes count as changes") do
    doc = DirtyDoc.new({"title": "Hello", "views": 3})

    assert(doc.changed?)
    assert_eq(doc.changed, ["title", "views"])
    assert_eq(doc.changes, {"title": [nil, "Hello"], "views": [nil, 3]})
  end

  test("changed is sorted alphabetically") do
    doc = DirtyDoc.new({"zeta": 1, "alpha": 2})

    assert_eq(doc.changed, ["alpha", "zeta"])
  end

  test("direct assignment counts as a change") do
    doc = DirtyDoc.new({})
    doc.title = "assigned"

    assert(doc.changed?)
    assert_eq(doc.changed, ["title"])
    assert_eq(doc.changes, {"title": [nil, "assigned"]})
  end

  test("assigning nil back to a new attribute leaves it clean") do
    doc = DirtyDoc.new({"title": "x"})
    doc.title = nil

    assert_eq(doc.changed?, false)
    assert_eq(doc.changes, {})
  end

  test("attribute_was is nil on a new record") do
    doc = DirtyDoc.new({"title": "x"})

    assert_null(doc.attribute_was("title"))
    assert_null(doc.attribute_was("never_set"))
  end

  test("previous_changes is empty before any persist") do
    doc = DirtyDoc.new({"title": "x"})

    assert_eq(doc.previous_changes, {})
  end

  test("attribute_was rejects a non-string name") do
    doc = DirtyDoc.new({})

    assert_raises("attribute_was() expects a string attribute name") do
      doc.attribute_was(42)
    end
  end
end

describe("dirty tracking across persistence") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    DirtyDoc.delete_all()
    DirtyValidated.delete_all()
  end

  test("create leaves the record clean and fills previous_changes") do
    doc = DirtyDoc.create({"title": "fresh", "views": 0})

    assert_eq(doc.changed?, false)
    assert_eq(doc.changed, [])
    previous = doc.previous_changes
    assert_eq(previous["title"], [nil, "fresh"])
    assert_eq(previous["views"], [nil, 0])
  end

  test("update records exactly the delta in previous_changes") do
    doc = DirtyDoc.create({"title": "before", "views": 1})
    doc.title = "after"

    assert(doc.changed?)
    assert_eq(doc.changed, ["title"])
    assert_eq(doc.attribute_was("title"), "before")

    assert(doc.update)
    assert_eq(doc.changed?, false)
    assert_eq(doc.previous_changes, {"title": ["before", "after"]})
  end

  test("assigning an equal value stays clean on a loaded record") do
    doc = DirtyDoc.create({"title": "same"})
    found = DirtyDoc.find(doc._key)

    assert_eq(found.changed?, false)
    found.title = "same"
    assert_eq(found.changed?, false)
  end

  test("records loaded with find start clean") do
    doc = DirtyDoc.create({"title": "loaded"})
    found = DirtyDoc.find(doc._key)

    assert_eq(found.changed?, false)
    found.title = "edited"
    assert_eq(found.changed, ["title"])
    assert_eq(found.changes, {"title": ["loaded", "edited"]})
    assert_eq(found.attribute_was("title"), "loaded")
  end

  test("a failed validation keeps the record dirty") do
    doc = DirtyValidated.create({"name": "valid"})
    doc.name = ""

    assert_eq(doc.update, false)
    assert(doc.changed?)
    assert_eq(doc.attribute_was("name"), "valid")
  end

  test("save on an existing record resets dirty state") do
    doc = DirtyDoc.create({"title": "v1"})
    doc.title = "v2"

    assert(doc.save)
    assert_eq(doc.changed?, false)
    assert_eq(doc.previous_changes, {"title": ["v1", "v2"]})
  end

  test("reload clears pending changes") do
    doc = DirtyDoc.create({"title": "stored"})
    doc.title = "unsaved edit"
    assert(doc.changed?)

    doc.reload

    assert_eq(doc.changed?, false)
    assert_eq(doc.title, "stored")
  end

  test("increment does not leave the field dirty") do
    doc = DirtyDoc.create({"title": "counted", "views": 1})

    doc.increment("views")

    assert_eq(doc.views, 2)
    assert_eq(doc.changed?, false)
  end
end
