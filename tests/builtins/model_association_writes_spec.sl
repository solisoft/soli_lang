# Association writers on has_many accessors (plain and polymorphic as:):
#   owner.rel << record          — stamps the FK (+ type pair) and saves
#   owner.rel.create({...})      — creates the child with the seed applied
# Both route through the regular save path: validations, callbacks, counter
# caches and dirty tracking all apply.

class AwAuthor < Model
  has_many("aw_books")
end

class AwBook < Model
  belongs_to("aw_author")
end

class AwStrictShelf < Model
  has_many("aw_strict_books")
end

class AwStrictBook < Model
  belongs_to("aw_strict_shelf")
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
  belongs_to(
    "aw_notable",
    {"polymorphic": true, "counter_cache": true}
  )
end

def titles_of(relation)
  relation.order("title").all.map { |book| book.title }
end

describe("association writes without a database") do
  test("pushing onto an unpersisted owner is refused") do
    assert_raises("cannot push to \"aw_books\": save the owner record first") do
      AwAuthor.new({}).aw_books << AwBook.new({})
    end
  end

  test("create on a plain where-QueryBuilder is refused") do
    assert_raises("create() is only available on a has_many relation accessor of a persisted record") do
      AwBook.where("title == @t", {"t": "x"}).create({"title": "nope"})
    end
  end
end

describe("association writes against SoliDB") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    [AwAuthor, AwBook, AwStrictShelf, AwStrictBook, AwCustomer, AwSupplier, AwNote].each do |model|
      model.delete_all()
    end
  end

  describe("<<") do
    test("pushing a persisted record adopts it") do
      author = AwAuthor.create({"name": "a"})
      book = AwBook.create({"title": "loose book"})

      author.aw_books << book

      assert_eq(titles_of(author.aw_books), ["loose book"])
      assert_eq(AwBook.find(book._key).aw_author_id, author._key)
    end

    test("pushing an unpersisted record creates it") do
      author = AwAuthor.create({"name": "a"})
      draft = AwBook.new({"title": "draft"})

      author.aw_books << draft

      assert_eq(AwBook.find(draft._key).aw_author_id, author._key)
      assert_eq(AwBook.count, 1)
    end

    test("pushing an array adopts every record") do
      author = AwAuthor.create({"name": "a"})
      author.aw_books << [AwBook.new({"title": "one"}), AwBook.new({"title": "two"})]
      assert_eq(titles_of(author.aw_books), ["one", "two"])
    end

    test("an empty array is a no-op") do
      author = AwAuthor.create({"name": "a"})
      author.aw_books << []
      assert_eq(author.aw_books.count, 0)
    end

    test("pushing onto a polymorphic inverse sets id, type and the counter") do
      customer = AwCustomer.create({"name": "c"})
      note = AwNote.create({"message": "bla"})

      customer.aw_notes << note

      reloaded = AwNote.find(note._key)
      assert_eq(reloaded.aw_notable_id, customer._key)
      assert_eq(reloaded.aw_notable_type, "AwCustomer")
      assert_eq(customer.aw_notes.count, 1)
      assert_eq(AwCustomer.find(customer._key).aw_notes_count, 1)
    end

    test("pushing something that is not an instance is refused") do
      author = AwAuthor.create({"name": "a"})
      assert_raises("\"aw_books\" << expects a model instance (or an array of them), got string") do
        author.aw_books << "some-key"
      end
    end

    test("a failing save aborts the push loudly") do
      shelf = AwStrictShelf.create({"name": "s"})
      assert_raises("\"aw_strict_books\" << failed: the record did not save (check its _errors)") do
        shelf.aw_strict_books << AwStrictBook.new({})
      end
      assert_eq(AwStrictBook.count, 0)
    end
  end

  describe("relation create") do
    test("seeds the foreign key") do
      author = AwAuthor.create({"name": "a"})

      book = author.aw_books.create({"title": "seeded"})

      assert_null(book._errors)
      assert_eq(book.aw_author_id, author._key)
      assert_eq(book.title, "seeded")
      assert_eq(titles_of(author.aw_books), ["seeded"])
    end

    test("on a polymorphic inverse seeds id and type and bumps the counter") do
      customer = AwCustomer.create({"name": "c"})
      supplier = AwSupplier.create({"name": "s"})

      note = customer.aw_notes.create({"message": "bla"})

      assert_eq(note.aw_notable_id, customer._key)
      assert_eq(note.aw_notable_type, "AwCustomer")
      assert_eq(customer.aw_notes.count, 1)
      assert_eq(supplier.aw_notes.count, 0)
      assert_eq(AwCustomer.find(customer._key).aw_notes_count, 1)
    end

    test("the seed wins over caller-supplied foreign keys") do
      customer = AwCustomer.create({"name": "c"})
      supplier = AwSupplier.create({"name": "s"})

      forged = {"message": "sneaky", "aw_notable_id": supplier._key, "aw_notable_type": "AwSupplier"}
      hijack = customer.aw_notes.create(forged)

      assert_eq(hijack.aw_notable_id, customer._key)
      assert_eq(hijack.aw_notable_type, "AwCustomer")
      assert_eq(supplier.aw_notes.count, 0)
    end

    test("a validation failure returns the instance with _errors") do
      shelf = AwStrictShelf.create({"name": "s"})

      invalid = shelf.aw_strict_books.create({})

      assert_eq(invalid._errors, [{"field": "title", "message": "can't be blank"}])
      assert_null(invalid._key)
      assert_eq(shelf.aw_strict_books.count, 0)
    end
  end
end
