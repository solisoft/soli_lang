# Model search: the class-body index DSL (vector_index, fulltext_index,
# geo_index, index), __sync_model_indexes(), and the queries they power:
# similar(), search(), hybrid(), near() and within().

class SearchTestDoc < Model
  vector_index("vec", dimension: 4)
  fulltext_index("title", "body")
  index("email", unique: true)
end

class GeoTestShop < Model
  geo_index("location")
end

# Control model without any search declarations.
class SearchPlainDoc < Model
end

QUERY_VECTOR = [1.0, 0.0, 0.0, 0.0]

# Filled by the DB suite's before_all: the reports of two index syncs.
sync_reports = []

def seed_search_corpus
  SearchTestDoc.create({"title": "Database systems", "body": "All about database engines",
    "kind": "tech", "email": "a@example.com", "vec": [1.0, 0.0, 0.0, 0.0]})
  SearchTestDoc.create({"title": "Database handbook", "body": "A practical database guide",
    "kind": "tech", "email": "b@example.com", "vec": [0.9, 0.1, 0.0, 0.0]})
  SearchTestDoc.create({"title": "Cooking pasta", "body": "A recipe collection",
    "kind": "food", "email": "c@example.com", "vec": [0.0, 1.0, 0.0, 0.0]})

  GeoTestShop.create({"name": "louvre", "location": {"lat": 48.8606, "lon": 2.3376}})
  GeoTestShop.create({"name": "orsay", "location": {"lat": 48.86, "lon": 2.3266}})
  GeoTestShop.create({"name": "berlin", "location": {"lat": 52.52, "lon": 13.405}})
end

def titles(docs)
  docs.map { |doc| doc.title }
end

describe("Search declaration guards (no DB)") do
  test("search on a model without fulltext_index raises") do
    assert_raises("SearchPlainDoc.search requires a `fulltext_index` declaration") do
      SearchPlainDoc.search("anything")
    end
  end

  test("near on a model without geo_index raises") do
    assert_raises("SearchPlainDoc.near requires a `geo_index` declaration") do
      SearchPlainDoc.near(48.86, 2.33)
    end
  end

  test("within on a model without geo_index raises") do
    assert_raises("SearchPlainDoc.within requires a `geo_index` declaration") do
      SearchPlainDoc.within(48.86, 2.33, 1000.0)
    end
  end

  test("a search field not covered by the fulltext_index raises") do
    assert_raises("field 'nope' is not covered by the fulltext_index (declared: title, body)") do
      SearchTestDoc.search("database", {"field": "nope"})
    end
  end
end

describe("Hybrid declaration guards (no DB)") do
  test("hybrid on a model without vector_index raises") do
    assert_raises("SearchPlainDoc.hybrid requires a `vector_index` declaration") do
      SearchPlainDoc.hybrid("anything", {"vector": [1.0]})
    end
  end

  test("a hybrid fulltext field not covered raises") do
    assert_raises("field 'nope' is not covered by a fulltext_index") do
      SearchTestDoc.hybrid("database", {"vector": QUERY_VECTOR, "field": "nope"})
    end
  end

  test("an invalid fusion raises") do
    assert_raises("fusion must be \"weighted\" or \"rrf\", got \"bogus\"") do
      SearchTestDoc.hybrid("database", {"vector": QUERY_VECTOR, "fusion": "bogus"})
    end
  end

  test("an unknown option raises") do
    assert_raises("unknown/invalid option 'nope'") do
      SearchTestDoc.hybrid("database", {"vector": QUERY_VECTOR, "nope": 1})
    end
  end
end

describe("Search queries (DB)") do
  # One corpus for every test below, all of which only read. The docs are
  # seeded BEFORE the indexes are created, so the sync's backfill path is
  # what makes them searchable.
  before_all() do
    requires_solidb()
    seed_search_corpus()
    sync_reports = [__sync_model_indexes(), __sync_model_indexes()]
  end

  after_all() do
    SearchTestDoc.delete_all()
    GeoTestShop.delete_all()
    SearchPlainDoc.delete_all()
  end

  describe("__sync_model_indexes") do
    test("the first sync creates every declared index") do
      assert_contains(sync_reports[0], "created vector index idx_vec on search_test_docs")
      assert_contains(sync_reports[0], "created fulltext index ft_search_test_docs on search_test_docs")
      assert_contains(sync_reports[0], "created persistent index idx_search_test_docs_email on search_test_docs")
      assert_contains(sync_reports[0], "created geo index geo_location on geo_test_shops")
    end

    test("a second sync is a no-op") do
      assert_eq(sync_reports[1], [])
    end

    test("the unique index refuses a duplicate email") do
      duplicate = SearchTestDoc.create({"title": "dup", "email": "a@example.com"})

      assert_null(duplicate._key)
      assert_contains(str(duplicate._errors), "Unique constraint violated")
      assert_eq(SearchTestDoc.where({"email": "a@example.com"}).count, 1)
    end
  end

  describe("vector similarity") do
    test("similar with a vector literal ranks by similarity") do
      hits = SearchTestDoc.similar(QUERY_VECTOR, "vec", 2).all

      assert_eq(titles(hits), ["Database systems", "Database handbook"])
      assert_eq(hits[0]._similarity_score, 1)
      assert_gt(hits[1]._similarity_score, 0.99)
      assert_lt(hits[1]._similarity_score, 1)
    end

    test("docs seeded before the index was created are searchable (backfill)") do
      hits = SearchTestDoc.similar(QUERY_VECTOR, "vec", 3).all

      assert_eq(titles(hits), ["Database systems", "Database handbook", "Cooking pasta"])
    end

    test("exact: true returns the same ranking as the index path") do
      approximate = SearchTestDoc.similar(QUERY_VECTOR, "vec", 2).all
      exact = SearchTestDoc.similar(QUERY_VECTOR, "vec", 2, {"exact": true}).all

      assert_eq(titles(exact), titles(approximate))
    end

    test("similar chained after where() applies the filter") do
      hits = SearchTestDoc.where({"kind": "food"}).similar(QUERY_VECTOR, "vec", 3).all

      assert_eq(titles(hits), ["Cooking pasta"])
    end

    test("without a vector_index, similar falls back to client-side cosine") do
      SearchPlainDoc.create({"name": "east", "vec": [1.0, 0.0]})
      SearchPlainDoc.create({"name": "north", "vec": [0.0, 1.0]})

      hits = SearchPlainDoc.similar([1.0, 0.0], "vec", 1).all

      assert_eq(hits.map { |hit| hit.name }, ["east"])
      assert_eq(hits[0]._similarity_score, 1)
    end
  end

  describe("fulltext search") do
    test("search returns the matching instances with _search_score") do
      results = SearchTestDoc.search("database")

      assert_eq(titles(results).sort(), ["Database handbook", "Database systems"])
      assert_eq(results[0].class, "SearchTestDoc")
      assert_gt(results[0]._search_score, 0)
      assert_gt(results[1]._search_score, 0)
    end

    test("a term only in one doc finds only that doc") do
      assert_eq(titles(SearchTestDoc.search("pasta")), ["Cooking pasta"])
    end

    test("a term in no doc returns an empty array") do
      assert_eq(SearchTestDoc.search("zzzz"), [])
    end

    test("the field option restricts the search to one covered field") do
      assert_eq(titles(SearchTestDoc.search("recipe", {"field": "body"})), ["Cooking pasta"])
      assert_eq(SearchTestDoc.search("recipe", {"field": "title"}), [])
    end

    test("the limit option caps the results") do
      assert_eq(SearchTestDoc.search("database", {"limit": 1}).length, 1)
    end

    test("the highlight option adds _highlighted") do
      highlighted = SearchTestDoc.search("database", {"highlight": true}).map { |doc| doc._highlighted }

      assert_eq(highlighted.sort(), ["<b>Database</b> handbook", "<b>Database</b> systems"])
    end
  end

  describe("hybrid search") do
    test("hybrid with a vector literal fuses both legs") do
      # The vector leg favors "Database systems" ([1,0,0,0] exactly); the text
      # leg matches both database docs; "Cooking pasta" is vector-only.
      results = SearchTestDoc.hybrid("database", {"vector": QUERY_VECTOR})

      assert_eq(titles(results), ["Database systems", "Database handbook", "Cooking pasta"])
      assert_gt(results[0]._hybrid_score, results[1]._hybrid_score)
      assert_gt(results[1]._hybrid_score, results[2]._hybrid_score)
      assert_eq(results[0]._sources, ["vector", "fulltext"])
    end

    test("vector-only matches carry only the vector source") do
      results = SearchTestDoc.hybrid("database", {"vector": QUERY_VECTOR})
      pasta = results.filter { |doc| doc.title == "Cooking pasta" }

      assert_eq(pasta.length, 1)
      assert_eq(pasta[0]._sources, ["vector"])
    end

    test("text weighting overrides a hostile query vector") do
      # The query vector points at "Cooking pasta", but with text_weight 1.0
      # the two fulltext matches must still rank on top.
      results = SearchTestDoc.hybrid("database",
        {"vector": [0.0, 1.0, 0.0, 0.0], "vector_weight": 0.0, "text_weight": 1.0})

      assert_eq(titles(results).length, 3)
      assert_eq([results[0].title, results[1].title].sort(), ["Database handbook", "Database systems"])
      assert_eq(results[2].title, "Cooking pasta")
      assert_eq(results[2]._hybrid_score, 0)
    end

    test("rrf fusion and limit are honored") do
      results = SearchTestDoc.hybrid("database", {"vector": QUERY_VECTOR, "fusion": "rrf", "limit": 2})

      # The two text hits tie in the fulltext leg, so their RRF scores can tie
      # too and either may come first.
      assert_eq(titles(results).sort(), ["Database handbook", "Database systems"])
      assert(results[0]._hybrid_score >= results[1]._hybrid_score)
    end
  end

  describe("geo queries") do
    test("near returns the closest shops first, with _distance in meters") do
      shops = GeoTestShop.near(48.86, 2.33, {"limit": 2})

      assert_eq(shops.map { |shop| shop.name }, ["orsay", "louvre"])
      assert_gt(shops[0]._distance, 200)
      assert_lt(shops[0]._distance, 300)
      assert_gt(shops[1]._distance, 500)
      assert_lt(shops[1]._distance, 600)
    end

    test("near without a limit returns every shop, nearest first") do
      names = GeoTestShop.near(48.86, 2.33).map { |shop| shop.name }

      assert_eq(names, ["orsay", "louvre", "berlin"])
    end

    test("within excludes shops outside the radius") do
      names = GeoTestShop.within(48.86, 2.33, 5000.0).map { |shop| shop.name }

      assert_eq(names.sort(), ["louvre", "orsay"])
    end

    test("within a radius smaller than every distance is empty") do
      assert_eq(GeoTestShop.within(48.86, 2.33, 1.0), [])
    end
  end
end
