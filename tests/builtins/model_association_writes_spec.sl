# ============================================================================
# Association writers on has_many accessors (plain and polymorphic as:):
#   owner.rel << record          — stamps the FK (+ type pair) and saves
#   owner.rel.create({...})      — creates the child with the seed applied
# Both route through the regular save path: validations, callbacks, counter
# caches, and dirty tracking all apply. Error paths run without a database;
# behavior is gated behind the DB availability probe.
# ============================================================================
class AwAuthor < Model
  has_many "aw_books"
end

class AwBook < Model
  belongs_to "aw_author"
end

class AwStrictShelf < Model
  has_many "aw_strict_books"
end

class AwStrictBook < Model
  belongs_to "aw_strict_shelf"
  validates("title", {"presence": true})
end

# Polymorphic: the user-facing motivation — auto-set {name}_id + {name}_type.
class AwCustomer < Model
  has_many("aw_notes", {"as": "aw_notable"})
end

class AwSupplier < Model
  has_many("aw_notes", {"as": "aw_notable"})
end

class AwNote < Model
  belongs_to("aw_notable", {"polymorphic": true, "counter_cache": true})
end

# Detect DB availability
let __db_available = false
try
  let __probe = AwAuthor.create({"name": "__probe__"})
  if !__probe.nil? && !__probe._errors
    __db_available = true
    __probe.delete()
  end
catch e
  __db_available = false
end

describe("has_many shovel writes", fn() {
  test("pushing a persisted record adopts it", fn() {
    if __db_available
      let author = AwAuthor.create({"name": "a"})
      let book = AwBook.create({"title": "loose book"})

      author.aw_books << book

      assert_eq(author.aw_books.count(), 1)
      assert_eq(AwBook.find(book._key).aw_author_id, author._key)

      book.delete(); author.delete()
    end
  })

  test("pushing an unpersisted record creates it", fn() {
    if __db_available
      let author = AwAuthor.create({"name": "a"})
      let draft = AwBook.new({"title": "draft"})

      author.aw_books << draft

      assert_not_null(draft._key)
      assert_eq(author.aw_books.count(), 1)

      draft.delete(); author.delete()
    end
  })

  test("pushing an array adopts every record", fn() {
    if __db_available
      let author = AwAuthor.create({"name": "a"})
      let b1 = AwBook.new({"title": "one"})
      let b2 = AwBook.new({"title": "two"})

      author.aw_books << [b1, b2]

      assert_eq(author.aw_books.count(), 2)

      b1.delete(); b2.delete(); author.delete()
    end
  })

  test("pushing onto a polymorphic inverse auto-sets id and type", fn() {
    if __db_available
      let customer = AwCustomer.create({"name": "c"})
      let note = AwNote.create({"message": "bla"})

      customer.aw_notes << note

      let reloaded = AwNote.find(note._key)
      assert_eq(reloaded.aw_notable_id, customer._key)
      assert_eq(reloaded.aw_notable_type, "AwCustomer")
      assert_eq(customer.aw_notes.count(), 1)
      # Counter cache bumped through the FK-change path.
      assert_eq(AwCustomer.find(customer._key).aw_notes_count, 1)

      note.delete(); customer.delete()
    end
  })

  test("pushing a non-instance raises", fn() {
    if __db_available
      let author = AwAuthor.create({"name": "a"})
      let raised = false
      try
        author.aw_books << "some-key"
      catch e
        raised = true
        assert(str(e).includes?("model instance"))
      end
      assert(raised)
      author.delete()
    end
  })

  test("pushing onto an unpersisted owner raises", fn() {
    let raised = false
    try
      AwAuthor.new({}).aw_books << AwBook.new({})
    catch e
      raised = true
      assert(str(e).includes?("save the owner"))
    end
    assert(raised)
  })

  test("a failing save aborts the push loudly", fn() {
    if __db_available
      let shelf = AwStrictShelf.create({"name": "s"})
      let invalid = AwStrictBook.new({})  # missing required title
      let raised = false
      try
        shelf.aw_strict_books << invalid
      catch e
        raised = true
        assert(str(e).includes?("_errors"))
      end
      assert(raised)
      shelf.delete()
    end
  })
})

describe("has_many relation create", fn() {
  test("create seeds the foreign key", fn() {
    if __db_available
      let author = AwAuthor.create({"name": "a"})

      let book = author.aw_books.create({"title": "seeded"})

      assert_null(book._errors)
      assert_eq(book.aw_author_id, author._key)
      assert_eq(book.title, "seeded")
      assert_eq(author.aw_books.count(), 1)

      book.delete(); author.delete()
    end
  })

  test("create on a polymorphic inverse seeds id and type", fn() {
    if __db_available
      let customer = AwCustomer.create({"name": "c"})
      let supplier = AwSupplier.create({"name": "s"})

      let note = customer.aw_notes.create({"message": "bla"})

      assert_eq(note.aw_notable_id, customer._key)
      assert_eq(note.aw_notable_type, "AwCustomer")
      assert_eq(customer.aw_notes.count(), 1)
      assert_eq(supplier.aw_notes.count(), 0)
      assert_eq(AwCustomer.find(customer._key).aw_notes_count, 1)

      # The seed wins over caller-supplied values.
      let hijack = customer.aw_notes.create({
        "message": "sneaky",
        "aw_notable_id": supplier._key,
        "aw_notable_type": "AwSupplier"
      })
      assert_eq(hijack.aw_notable_id, customer._key)
      assert_eq(hijack.aw_notable_type, "AwCustomer")

      note.delete(); hijack.delete()
      customer.delete(); supplier.delete()
    end
  })

  test("a validation failure returns the instance with _errors", fn() {
    if __db_available
      let shelf = AwStrictShelf.create({"name": "s"})

      let invalid = shelf.aw_strict_books.create({})

      assert_not_null(invalid._errors)
      assert_null(invalid._key)
      assert_eq(shelf.aw_strict_books.count(), 0)

      shelf.delete()
    end
  })

  test("create on a plain where-QueryBuilder raises", fn() {
    let raised = false
    try
      AwBook.where("title == @t", {"t": "x"}).create({"title": "nope"})
    catch e
      raised = true
      assert(str(e).includes?("relation accessor"))
    end
    assert(raised)
  })
})
