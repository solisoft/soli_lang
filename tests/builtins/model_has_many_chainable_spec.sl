# has_many returns a chainable QueryBuilder, not a plain Array: callers chain
# Rails-style terminals (delete_all, count, where, update_all, ...) and can
# still iterate and index it like an array.

class HmAuthor < Model
  has_many("hm_books")
end

class HmBook < Model
  belongs_to("hm_author")
end

def add_books(author, titles)
  titles.each do |title|
    HmBook.create({"title": title, "hm_author_id": author._key})
  end
end

# No rows are fetched: an unsaved owner's relation is a QueryBuilder whose
# seed filter can never match.
describe("has_many returns a chainable QueryBuilder (no DB)") do
  test("class is query_builder, not array") do
    assert_eq(HmAuthor.new().hm_books.class, "query_builder")
  end

  test("an unsaved owner yields an always-empty filter") do
    assert_eq(HmAuthor.new().hm_books.to_query, "FOR doc IN hm_books FILTER 1 == 0 RETURN doc")
  end

  test("where chaining ANDs onto the seed filter") do
    query = HmAuthor.new().hm_books.where("title = @t", {"t": "x"}).to_query

    assert_contains(query, "FILTER (1 == 0) AND (doc.title == @t) RETURN doc")
  end

  test("update_all on an unsaved owner is a safe no-op") do
    assert_null(HmAuthor.new().hm_books.update_all({"title": "x"}))
  end
end

describe("has_many chainable (DB)") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    HmBook.delete_all()
    HmAuthor.delete_all()
  end

  test("a saved owner filters on its foreign key") do
    author = HmAuthor.create({"name": "Octavia"})

    assert_contains(author.hm_books.to_query, "FILTER doc.hm_author_id == @__rel_fk RETURN doc")
  end

  test("count reflects child rows") do
    author = HmAuthor.create({"name": "Octavia"})
    add_books(author, ["B1", "B2", "B3"])

    assert_eq(author.hm_books.count, 3)
  end

  test("count is 0 for an owner without children") do
    author = HmAuthor.create({"name": "Childless"})

    assert_eq(author.hm_books.count, 0)
    assert_eq(len(author.hm_books), 0)
  end

  test("len() works on the relation accessor") do
    author = HmAuthor.create({"name": "Ursula"})
    add_books(author, ["L1", "L2"])

    assert_eq(len(author.hm_books), 2)
  end

  test("for-loop iterates the relation") do
    author = HmAuthor.create({"name": "Iain"})
    add_books(author, ["Loop1", "Loop2"])

    titles = []
    for book in author.hm_books
      assert(book.is_a?("HmBook"))
      titles.push(book.title)
    end

    assert_eq(titles.sort(), ["Loop1", "Loop2"])
  end

  test("indexing with [n] materializes and returns an instance") do
    author = HmAuthor.create({"name": "Indexable"})
    add_books(author, ["Idx0"])

    first = author.hm_books[0]

    assert(first.is_a?("HmBook"))
    assert_eq(first.title, "Idx0")
  end

  test("delete_all removes only this owner's children") do
    kept = HmAuthor.create({"name": "Kept"})
    dropped = HmAuthor.create({"name": "Dropped"})
    add_books(kept, ["Keep"])
    add_books(dropped, ["Gone1", "Gone2"])

    dropped.hm_books.delete_all

    assert_eq(kept.hm_books.count, 1)
    assert_eq(dropped.hm_books.count, 0)
  end

  test("where(...).delete_all only deletes matching children") do
    author = HmAuthor.create({"name": "Selective"})
    add_books(author, ["alpha", "beta", "alpha"])

    author.hm_books.where("title = @t", {"t": "alpha"}).delete_all

    assert_eq(author.hm_books.count, 1)
    assert_eq(author.hm_books[0].title, "beta")
  end

  test("where(...).update_all patches only matching children") do
    author = HmAuthor.create({"name": "Patcher"})
    add_books(author, ["draft", "draft", "published"])

    author.hm_books.where("title = @t", {"t": "draft"}).update_all({"title": "archived"})

    assert_eq(author.hm_books.where("title = @t", {"t": "archived"}).count, 2)
    assert_eq(author.hm_books.where("title = @t", {"t": "published"}).count, 1)
    assert_eq(author.hm_books.where("title = @t", {"t": "draft"}).count, 0)
  end

  test("update_all scopes to this owner's children only") do
    kept = HmAuthor.create({"name": "KeptOwner"})
    touched = HmAuthor.create({"name": "TouchedOwner"})
    add_books(kept, ["orig"])
    add_books(touched, ["orig"])

    touched.hm_books.update_all({"title": "renamed"})

    assert_eq(kept.hm_books.where("title = @t", {"t": "orig"}).count, 1)
    assert_eq(touched.hm_books.where("title = @t", {"t": "renamed"}).count, 1)
  end

  test("each iterates with a block") do
    author = HmAuthor.create({"name": "Each"})
    add_books(author, ["E1", "E2"])

    titles = []
    author.hm_books.each do |book|
      titles.push(book.title)
    end

    assert_eq(titles.sort(), ["E1", "E2"])
  end

  test("map returns an array") do
    author = HmAuthor.create({"name": "Map"})
    add_books(author, ["M1", "M2"])

    titles = author.hm_books.map { |book| book.title }

    assert_eq(type(titles), "array")
    assert_eq(titles.sort(), ["M1", "M2"])
  end
end
