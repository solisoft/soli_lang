# ============================================================================
# Model Search Test Suite (vector + fulltext + geo + index DSL)
# Tests for the class-body index declarations (vector_index, fulltext_index,
# geo_index, index), __sync_model_indexes(), similar(), search(), near() and
# within().
# ============================================================================

class SearchTestDoc extends Model
    vector_index "vec", dimension: 4
    fulltext_index "title", "body"
    index "email", unique: true
end

class GeoTestShop extends Model
    geo_index "location"
end

# Control model without any search declarations.
class SearchPlainDoc extends Model
end

# Detect DB availability
let __db_available = false;
try
    let __probe = SearchTestDoc.create({ "title": "__probe__", "email": "probe@example.com" });
    if !__probe.nil? and !__probe._errors
        __db_available = true;
        __probe.delete();
    end
catch e
end

# One-time DB setup, top-level so every test sees the same corpus. Docs are
# seeded BEFORE the indexes are created, so the sync's backfill path is what
# makes them searchable. All tests below only read, so no per-test reseed.
let __sync_report_first = null;
let __sync_report_second = null;
if __db_available
    SearchTestDoc.delete_all() rescue null;
    GeoTestShop.delete_all() rescue null;

    SearchTestDoc.create({
        "title": "Database systems",
        "body": "All about database engines",
        "kind": "tech",
        "email": "a@example.com",
        "vec": [1.0, 0.0, 0.0, 0.0]
    });
    SearchTestDoc.create({
        "title": "Database handbook",
        "body": "A practical database guide",
        "kind": "tech",
        "email": "b@example.com",
        "vec": [0.9, 0.1, 0.0, 0.0]
    });
    SearchTestDoc.create({
        "title": "Cooking pasta",
        "body": "A recipe collection",
        "kind": "food",
        "email": "c@example.com",
        "vec": [0.0, 1.0, 0.0, 0.0]
    });

    GeoTestShop.create({ "name": "louvre", "location": { "lat": 48.8606, "lon": 2.3376 } });
    GeoTestShop.create({ "name": "orsay", "location": { "lat": 48.86, "lon": 2.3266 } });
    GeoTestShop.create({ "name": "berlin", "location": { "lat": 52.52, "lon": 13.405 } });

    __sync_report_first = __sync_model_indexes();
    __sync_report_second = __sync_model_indexes();
end

# ============================================================================
# Tests that do NOT require a DB connection. The declaration guards raise
# before any HTTP call, so they work without a reachable DB.
# ============================================================================

describe("Search declaration guards (no DB)", fn() {
    test("search on a model without fulltext_index raises", fn() {
        let msg = "";
        try
            SearchPlainDoc.search("anything");
        catch e
            msg = str(e);
        end
        assert(msg.contains("fulltext_index"));
    });

    test("near on a model without geo_index raises", fn() {
        let msg = "";
        try
            SearchPlainDoc.near(48.86, 2.33);
        catch e
            msg = str(e);
        end
        assert(msg.contains("geo_index"));
    });

    test("within on a model without geo_index raises", fn() {
        let msg = "";
        try
            SearchPlainDoc.within(48.86, 2.33, 1000.0);
        catch e
            msg = str(e);
        end
        assert(msg.contains("geo_index"));
    });

    test("search field not covered by the fulltext_index raises", fn() {
        let msg = "";
        try
            SearchTestDoc.search("database", { "field": "nope" });
        catch e
            msg = str(e);
        end
        assert(msg.contains("not covered"));
    });
});

describe("Hybrid declaration guards (no DB)", fn() {
    test("hybrid on a model without vector_index raises", fn() {
        let msg = "";
        try
            SearchPlainDoc.hybrid("anything", { "vector": [1.0] });
        catch e
            msg = str(e);
        end
        assert(msg.contains("vector_index"));
    });

    test("hybrid fulltext field not covered raises", fn() {
        let msg = "";
        try
            SearchTestDoc.hybrid("database", { "vector": [1.0, 0.0, 0.0, 0.0], "field": "nope" });
        catch e
            msg = str(e);
        end
        assert(msg.contains("not covered"));
    });

    test("hybrid invalid fusion raises", fn() {
        let msg = "";
        try
            SearchTestDoc.hybrid("database", { "vector": [1.0, 0.0, 0.0, 0.0], "fusion": "bogus" });
        catch e
            msg = str(e);
        end
        assert(msg.contains("fusion"));
    });

    test("hybrid unknown option raises", fn() {
        let msg = "";
        try
            SearchTestDoc.hybrid("database", { "vector": [1.0, 0.0, 0.0, 0.0], "nope": 1 });
        catch e
            msg = str(e);
        end
        assert(msg.contains("unknown"));
    });
});

# ============================================================================
# Tests that REQUIRE a DB connection.
#
# NOTE: suite extraction is static and only sees top-level describe() calls,
# so wrapping the describes in `if __db_available ... end` would silently
# skip them even WITH a live DB. Instead each test early-returns when no DB
# is reachable (they then pass trivially, contributing 0 assertions).
# ============================================================================

describe("__sync_model_indexes (DB)", fn() {
    test("returns a report array and is idempotent", fn() {
        if !__db_available
            return;
        end
        assert_eq(__sync_report_first.class, "array");
        assert_eq(__sync_report_second.class, "array");
        # The second sweep must not create anything new — every line (if any)
        # is an already-exists/warning line, never a fresh "created ...".
        for line in __sync_report_second
            assert(!line.starts_with("created"));
        end
    });
});

describe("Vector similarity (DB)", fn() {
    test("similar with a vector literal ranks by similarity", fn() {
        if !__db_available
            return;
        end
        let results = SearchTestDoc.similar([1.0, 0.0, 0.0, 0.0], "vec", 2);
        let hits = results.all;
        assert_eq(len(hits), 2);
        assert_eq(hits[0].title, "Database systems");
        assert_eq(hits[1].title, "Database handbook");
        assert(hits[0]._similarity_score > 0);
        assert(hits[1]._similarity_score > 0);
        assert(hits[0]._similarity_score >= hits[1]._similarity_score);
    });

    test("docs seeded before index creation are searchable (backfill)", fn() {
        if !__db_available
            return;
        end
        # All three docs predate the vector index (seeded before the sync),
        # so k=3 returning the full corpus proves the backfill.
        let hits = SearchTestDoc.similar([1.0, 0.0, 0.0, 0.0], "vec", 3).all;
        assert_eq(len(hits), 3);
    });

    test("exact: true returns the same top hits as the index path", fn() {
        if !__db_available
            return;
        end
        let ann = SearchTestDoc.similar([1.0, 0.0, 0.0, 0.0], "vec", 2).all;
        let exact = SearchTestDoc.similar([1.0, 0.0, 0.0, 0.0], "vec", 2, { "exact": true }).all;
        assert_eq(len(exact), 2);
        assert_eq(exact[0].title, ann[0].title);
        assert_eq(exact[1].title, ann[1].title);
    });

    test("similar chained after where() applies the filter", fn() {
        if !__db_available
            return;
        end
        let hits = SearchTestDoc.where({ "kind": "food" })
            .similar([1.0, 0.0, 0.0, 0.0], "vec", 3)
            .all;
        assert_eq(len(hits), 1);
        assert_eq(hits[0].title, "Cooking pasta");
    });
});

describe("Fulltext search (DB)", fn() {
    test("search returns ranked instances with _search_score", fn() {
        if !__db_available
            return;
        end
        let results = SearchTestDoc.search("database");
        assert_eq(len(results), 2);
        let titles = results.map(fn(r) r.title);
        assert(titles.includes?("Database systems"));
        assert(titles.includes?("Database handbook"));
        assert_not_null(results[0]._search_score);
        assert_not_null(results[1]._search_score);
    });

    test("limit option caps the results", fn() {
        if !__db_available
            return;
        end
        let results = SearchTestDoc.search("database", { "limit": 1 });
        assert_eq(len(results), 1);
    });

    test("highlight option adds _highlighted", fn() {
        if !__db_available
            return;
        end
        let results = SearchTestDoc.search("database", { "highlight": true });
        assert(len(results) >= 1);
        assert_not_null(results[0]._highlighted);
    });
});

describe("Hybrid search (DB)", fn() {
    test("hybrid with a vector literal fuses both legs", fn() {
        if !__db_available
            return;
        end
        # Vector leg favors "Database systems" ([1,0,0,0] exact); text leg
        # matches both database docs; "Cooking pasta" is vector-only.
        let results = SearchTestDoc.hybrid("database", { "vector": [1.0, 0.0, 0.0, 0.0] });
        assert_eq(len(results), 3);
        assert_eq(results[0].title, "Database systems");
        assert_eq(results[1].title, "Database handbook");
        assert(results[0]._hybrid_score >= results[1]._hybrid_score);
        assert(results[1]._hybrid_score >= results[2]._hybrid_score);
        assert(results[0]._sources.includes?("vector"));
        assert(results[0]._sources.includes?("fulltext"));
    });

    test("vector-only matches carry only the vector source", fn() {
        if !__db_available
            return;
        end
        let results = SearchTestDoc.hybrid("database", { "vector": [1.0, 0.0, 0.0, 0.0] });
        let pasta = results.filter(fn(r) r.title == "Cooking pasta");
        assert_eq(len(pasta), 1);
        assert(pasta[0]._sources.includes?("vector"));
        assert(!pasta[0]._sources.includes?("fulltext"));
    });

    test("text weighting overrides a hostile query vector", fn() {
        if !__db_available
            return;
        end
        # Query vector points at "Cooking pasta", but with text_weight 1.0
        # the two fulltext matches must still rank on top.
        let results = SearchTestDoc.hybrid("database", {
            "vector": [0.0, 1.0, 0.0, 0.0],
            "vector_weight": 0.0,
            "text_weight": 1.0
        });
        assert(len(results) >= 2);
        let top2 = [results[0].title, results[1].title];
        assert(top2.includes?("Database systems"));
        assert(top2.includes?("Database handbook"));
    });

    test("rrf fusion and limit are honored", fn() {
        if !__db_available
            return;
        end
        let results = SearchTestDoc.hybrid("database", {
            "vector": [1.0, 0.0, 0.0, 0.0],
            "fusion": "rrf",
            "limit": 2
        });
        assert_eq(len(results), 2);
        assert(results[0]._hybrid_score >= results[1]._hybrid_score);
    });
});

describe("Geo queries (DB)", fn() {
    test("near returns the closest shops with _distance", fn() {
        if !__db_available
            return;
        end
        let shops = GeoTestShop.near(48.86, 2.33, { "limit": 2 });
        assert_eq(len(shops), 2);
        let names = shops.map(fn(s) s.name);
        assert(names.includes?("louvre"));
        assert(names.includes?("orsay"));
        # Both Paris shops sit within ~1km of the query point.
        assert(shops[0]._distance >= 0.0);
        assert(shops[0]._distance < 10000.0);
        assert(shops[1]._distance < 10000.0);
    });

    test("within excludes shops outside the radius", fn() {
        if !__db_available
            return;
        end
        let shops = GeoTestShop.within(48.86, 2.33, 5000.0);
        assert_eq(len(shops), 2);
        let names = shops.map(fn(s) s.name);
        assert(names.includes?("louvre"));
        assert(names.includes?("orsay"));
        assert(!names.includes?("berlin"));
    });
});
