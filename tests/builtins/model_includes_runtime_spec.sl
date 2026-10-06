# .includes() / .includes_count() at runtime.
#   1. After `.includes(rel)`, a habtm, belongs_to or has_one accessor returns
#      the preloaded rows without a fresh query. The specs prove it by deleting
#      the related rows after the eager fetch: a re-query would see them gone.
#      has_many stays a chainable QueryBuilder and is NOT served from the cache.
#   2. `.includes_count(rel)` exposes a `<rel>_count` field on each parent.

class IncRtAuthor < Model
  has_many("inc_rt_books")
  has_one("inc_rt_pet")
  has_and_belongs_to_many("inc_rt_tags")
end

class IncRtBook < Model
  belongs_to("inc_rt_author")
end

class IncRtTag < Model
  has_and_belongs_to_many("inc_rt_authors")
end

class IncRtPet < Model
  belongs_to("inc_rt_author")
end

def load_author(key, relation)
  IncRtAuthor.where("_key == @k", {"k": key}).includes(relation).first
end

def load_author_counting(key, relation)
  IncRtAuthor.where("_key == @k", {"k": key}).includes_count(relation).first
end

def tag_names(tags)
  tags.map { |tag| tag.name }.sort()
end

describe("includes_count - query structure") do
  test("has_many emits a LENGTH subquery and merges it as <rel>_count") do
    assert_eq(IncRtAuthor.includes_count("inc_rt_books").to_query,
      "FOR doc IN inc_rt_authors " +
      "LET _rel_inc_rt_books_count = LENGTH(" +
      "FOR rel IN inc_rt_books FILTER rel.inc_rt_author_id == doc._key RETURN 1) " +
      "RETURN MERGE(doc, {inc_rt_books_count: _rel_inc_rt_books_count})")
  end

  test("habtm counts rows of the join table") do
    assert_eq(IncRtAuthor.includes_count("inc_rt_tags").to_query,
      "FOR doc IN inc_rt_authors " +
      "LET _rel_inc_rt_tags_count = LENGTH(" +
      "FOR jt IN inc_rt_authors_inc_rt_tags FILTER jt.inc_rt_author_id == doc._key RETURN 1) " +
      "RETURN MERGE(doc, {inc_rt_tags_count: _rel_inc_rt_tags_count})")
  end

  test("rejects a singular relation") do
    assert_raises("only supported for has_many and has_and_belongs_to_many") do
      IncRtBook.includes_count("inc_rt_author").to_query
    end
  end

  test("combining .includes and .includes_count merges both") do
    query = IncRtAuthor.includes("inc_rt_books").includes_count("inc_rt_tags").to_query

    assert_contains(query, "LET _rel_inc_rt_books = ")
    assert_contains(query, "LET _rel_inc_rt_tags_count = LENGTH(")
    assert_contains(query,
      "RETURN MERGE(doc, {inc_rt_books: _rel_inc_rt_books, inc_rt_tags_count: _rel_inc_rt_tags_count})")
  end
end

describe("includes - runtime (DB)") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    IncRtAuthor.all.each do |author|
      author.remove_inc_rt_tag(author.inc_rt_tags)
    end
    IncRtBook.delete_all()
    IncRtPet.delete_all()
    IncRtTag.delete_all()
    IncRtAuthor.delete_all()
  end

  describe("has_and_belongs_to_many") do
    test("the accessor reads the cached preload after the join rows are deleted") do
      author = IncRtAuthor.create({"name": "Cache HABTM"})
      rust = IncRtTag.create({"name": "rust"})
      soli = IncRtTag.create({"name": "soli"})
      author.add_inc_rt_tag(rust, soli)

      loaded = load_author(author._key, "inc_rt_tags")
      author.remove_inc_rt_tag([rust, soli])

      # A fresh, non-preloaded read sees the join rows are gone...
      assert_eq(IncRtAuthor.find(author._key).inc_rt_tags, [])
      # ...while the preloaded instance still answers from its cache.
      assert_eq(tag_names(loaded.inc_rt_tags), ["rust", "soli"])
    end

    test("preloaded rows are model instances, not raw hashes") do
      author = IncRtAuthor.create({"name": "Instances"})
      author.add_inc_rt_tag(IncRtTag.create({"name": "rust"}))

      loaded = load_author(author._key, "inc_rt_tags")

      assert_eq(loaded.inc_rt_tags[0].class, "IncRtTag")
      assert_eq(loaded.inc_rt_tags[0].name, "rust")
    end

    test("an empty preload is an empty array") do
      author = IncRtAuthor.create({"name": "Cache empty"})

      assert_eq(load_author(author._key, "inc_rt_tags").inc_rt_tags, [])
    end

    test("a second access returns the same converted instances") do
      author = IncRtAuthor.create({"name": "Cache idempotent"})
      author.add_inc_rt_tag(IncRtTag.create({"name": "alpha"}))

      loaded = load_author(author._key, "inc_rt_tags")
      first_read = loaded.inc_rt_tags
      second_read = loaded.inc_rt_tags

      assert_eq(first_read.length, 1)
      assert_eq(second_read.length, 1)
      assert_eq(first_read[0].class, "IncRtTag")
      assert_eq(second_read[0].class, "IncRtTag")
      assert_eq(second_read[0]._key, first_read[0]._key)
    end
  end

  describe("belongs_to and has_one") do
    test("belongs_to reads the cached parent after the parent is deleted") do
      author = IncRtAuthor.create({"name": "Cached parent"})
      book = IncRtBook.create({"title": "B1", "inc_rt_author_id": author._key})

      loaded = IncRtBook.where("_key == @k", {"k": book._key}).includes("inc_rt_author").first
      author.delete

      assert_null(IncRtBook.find(book._key).inc_rt_author)
      assert_eq(loaded.inc_rt_author.class, "IncRtAuthor")
      assert_eq(loaded.inc_rt_author.name, "Cached parent")
    end

    test("has_one reads the cached child after the child is deleted") do
      author = IncRtAuthor.create({"name": "Pet owner"})
      pet = IncRtPet.create({"name": "Rex", "inc_rt_author_id": author._key})

      loaded = load_author(author._key, "inc_rt_pet")
      pet.delete

      assert_null(IncRtAuthor.find(author._key).inc_rt_pet)
      assert_eq(loaded.inc_rt_pet.class, "IncRtPet")
      assert_eq(loaded.inc_rt_pet.name, "Rex")
    end
  end

  describe("has_many") do
    test("includes keeps the chainable accessor, which re-queries") do
      author = IncRtAuthor.create({"name": "Chainable"})
      IncRtBook.create({"title": "B1", "inc_rt_author_id": author._key})
      IncRtBook.create({"title": "B2", "inc_rt_author_id": author._key})

      loaded = load_author(author._key, "inc_rt_books")
      assert_eq(loaded.inc_rt_books.class, "query_builder")
      assert_eq(loaded.inc_rt_books.length, 2)

      IncRtBook.find_by("title", "B1").delete

      assert_eq(loaded.inc_rt_books.length, 1)
    end
  end
end

describe("includes_count - runtime (DB)") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    IncRtAuthor.all.each do |author|
      author.remove_inc_rt_tag(author.inc_rt_tags)
    end
    IncRtBook.delete_all()
    IncRtTag.delete_all()
    IncRtAuthor.delete_all()
  end

  test("has_many count matches the related row count") do
    author = IncRtAuthor.create({"name": "Counter HM"})
    ["C1", "C2", "C3"].each do |title|
      IncRtBook.create({"title": title, "inc_rt_author_id": author._key})
    end

    assert_eq(load_author_counting(author._key, "inc_rt_books").inc_rt_books_count, 3)
  end

  test("habtm count matches the join row count") do
    author = IncRtAuthor.create({"name": "Counter HABTM"})
    author.add_inc_rt_tag(IncRtTag.create({"name": "x"}), IncRtTag.create({"name": "y"}))

    assert_eq(load_author_counting(author._key, "inc_rt_tags").inc_rt_tags_count, 2)
  end

  test("a parent with no related rows counts zero") do
    author = IncRtAuthor.create({"name": "Counter zero"})

    assert_eq(load_author_counting(author._key, "inc_rt_books").inc_rt_books_count, 0)
  end

  test("each parent gets its own count") do
    busy = IncRtAuthor.create({"name": "busy"})
    idle = IncRtAuthor.create({"name": "idle"})
    IncRtBook.create({"title": "only", "inc_rt_author_id": busy._key})

    counts = {}
    IncRtAuthor.includes_count("inc_rt_books").all.each do |author|
      counts[author.name] = author.inc_rt_books_count
    end

    assert_eq(counts, {"busy": 1, "idle": 0})
  end
end
