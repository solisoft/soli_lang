// ============================================================================
// rerank() builtin — client-side lexical reranking
// ============================================================================

describe("rerank builtin", fn() {
    test("reorders records by query-token overlap", fn() {
        let docs = [
            { "content": "the quick brown fox jumps over" },
            { "content": "vector databases store embeddings for search" },
            { "content": "graph traversal and vector search combined" }
        ];
        let ranked = rerank("vector search embeddings", docs);
        assert_eq(len(ranked), 3);
        // doc[1] has 3 matching tokens, doc[2] has 2, doc[0] has 0.
        assert_eq(ranked[0]["content"], "vector databases store embeddings for search");
        assert_eq(ranked[1]["content"], "graph traversal and vector search combined");
        assert_eq(ranked[2]["content"], "the quick brown fox jumps over");
    });

    test("limit truncates to the top-k after reordering", fn() {
        let docs = [
            { "content": "the quick brown fox" },
            { "content": "vector search embeddings" }
        ];
        let ranked = rerank("vector search embeddings", docs, { "limit": 1 });
        assert_eq(len(ranked), 1);
        assert_eq(ranked[0]["content"], "vector search embeddings");
    });

    test("explicit field selects the ranking text", fn() {
        let docs = [
            { "title": "unrelated topic", "body": "mentions vector search here" },
            { "title": "vector search", "body": "nope" }
        ];
        let ranked = rerank("vector search", docs, { "field": "title" });
        assert_eq(ranked[0]["title"], "vector search");
    });

    test("empty input returns empty", fn() {
        let ranked = rerank("anything", []);
        assert_eq(len(ranked), 0);
    });
});
