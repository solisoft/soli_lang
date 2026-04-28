// ============================================================================
// has_many chainable / Enumerable behavior
// ----------------------------------------------------------------------------
// Verifies that `instance.<has_many_relation>` returns a QueryBuilder rather
// than a plain Array, so callers can chain Rails-style terminal operations
// (delete_all, count, where, ...) and still iterate / index it like an array.
// ============================================================================

class HmAuthor extends Model
    has_many("hm_books")
end

class HmBook extends Model
    belongs_to("hm_author")
end

// ----- DB availability probe (mirrors model_instances_spec.sl) ---------------
let __db_available = false;
try
    let __probe = HmAuthor.create({ "name": "__probe__" });
    if !__probe.nil? and !__probe._errors
        __db_available = true;
        __probe.delete();
    end
catch e
end

// ----------------------------------------------------------------------------
// No-DB tests: just verify the relation accessor returns a QueryBuilder with
// the right pre-applied FK filter — no rows are fetched.
// ----------------------------------------------------------------------------

describe("has_many returns chainable QueryBuilder (no DB)", fn() {
    test("class is query_builder, not array", fn() {
        let unsaved = HmAuthor.new();
        // Unsaved owners still produce a QueryBuilder (one that yields no
        // rows, so iteration / count / delete_all are all safe no-ops).
        assert_eq(unsaved.hm_books.class, "query_builder");
    });

    test("unsaved owner yields an always-empty filter", fn() {
        let unsaved = HmAuthor.new();
        let q = unsaved.hm_books.to_query;
        // Sentinel filter: never matches a real document.
        assert(q.contains("FILTER 1 == 0"));
    });

    test("where chaining ANDs onto the FK filter", fn() {
        let unsaved = HmAuthor.new();
        let q = unsaved.hm_books.where("title = @t", { "t": "x" }).to_query;
        // The seed filter and user filter are both present.
        assert(q.contains("1 == 0"));
        assert(q.contains("doc.title == @t"));
    });
});

// ----------------------------------------------------------------------------
// DB-backed tests: cover iteration, indexing, count, delete_all, and chained
// where(...).delete_all on a real has_many association.
// ----------------------------------------------------------------------------

if __db_available

describe("has_many chainable (DB)", fn() {
    test("count reflects child rows", fn() {
        let author = HmAuthor.create({ "name": "Octavia" });
        HmBook.create({ "title": "B1", "hm_author_id": author._key });
        HmBook.create({ "title": "B2", "hm_author_id": author._key });
        HmBook.create({ "title": "B3", "hm_author_id": author._key });

        assert_eq(author.hm_books.count, 3);

        author.hm_books.delete_all;
        author.delete();
    });

    test("len() works on the relation accessor", fn() {
        let author = HmAuthor.create({ "name": "Ursula" });
        HmBook.create({ "title": "L1", "hm_author_id": author._key });
        HmBook.create({ "title": "L2", "hm_author_id": author._key });

        assert_eq(len(author.hm_books), 2);

        author.hm_books.delete_all;
        author.delete();
    });

    test("for-loop iterates the relation", fn() {
        let author = HmAuthor.create({ "name": "Iain" });
        HmBook.create({ "title": "Loop1", "hm_author_id": author._key });
        HmBook.create({ "title": "Loop2", "hm_author_id": author._key });

        let count = 0;
        for book in author.hm_books
            assert(book.is_a?(HmBook));
            count = count + 1;
        end
        assert_eq(count, 2);

        author.hm_books.delete_all;
        author.delete();
    });

    test("indexing with [n] materializes and returns an instance", fn() {
        let author = HmAuthor.create({ "name": "Indexable" });
        HmBook.create({ "title": "Idx0", "hm_author_id": author._key });

        let first = author.hm_books[0];
        assert_not_null(first);
        assert(first.is_a?(HmBook));

        author.hm_books.delete_all;
        author.delete();
    });

    test("delete_all removes only this owner's children", fn() {
        let kept = HmAuthor.create({ "name": "Kept" });
        let dropped = HmAuthor.create({ "name": "Dropped" });
        HmBook.create({ "title": "Keep", "hm_author_id": kept._key });
        HmBook.create({ "title": "Gone1", "hm_author_id": dropped._key });
        HmBook.create({ "title": "Gone2", "hm_author_id": dropped._key });

        dropped.hm_books.delete_all;

        assert_eq(kept.hm_books.count, 1);
        assert_eq(dropped.hm_books.count, 0);

        kept.hm_books.delete_all;
        kept.delete();
        dropped.delete();
    });

    test("where(...).delete_all only deletes matching children", fn() {
        let author = HmAuthor.create({ "name": "Selective" });
        HmBook.create({ "title": "alpha", "hm_author_id": author._key });
        HmBook.create({ "title": "beta", "hm_author_id": author._key });
        HmBook.create({ "title": "alpha", "hm_author_id": author._key });

        author.hm_books.where("title = @t", { "t": "alpha" }).delete_all;

        assert_eq(author.hm_books.count, 1);
        let remaining = author.hm_books[0];
        assert_eq(remaining.title, "beta");

        author.hm_books.delete_all;
        author.delete();
    });

    test("each iterates with a block", fn() {
        let author = HmAuthor.create({ "name": "Each" });
        HmBook.create({ "title": "E1", "hm_author_id": author._key });
        HmBook.create({ "title": "E2", "hm_author_id": author._key });

        let titles = [];
        author.hm_books.each(fn(b) {
            titles.push(b.title);
        });
        assert_eq(len(titles), 2);

        author.hm_books.delete_all;
        author.delete();
    });

    test("map returns an array", fn() {
        let author = HmAuthor.create({ "name": "Map" });
        HmBook.create({ "title": "M1", "hm_author_id": author._key });
        HmBook.create({ "title": "M2", "hm_author_id": author._key });

        let titles = author.hm_books.map(fn(b) { b.title });
        assert_eq(len(titles), 2);

        author.hm_books.delete_all;
        author.delete();
    });
});

end
