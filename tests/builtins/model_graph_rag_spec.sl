# ============================================================================
# Model Graph RAG Test Suite
# Tests for traverse().similar() composition and Model.graph_rag().
# ============================================================================

class GraphRagUser extends Model
    vector_index "vec", dimension: 4, metric: "cosine"
end

class GraphRagFollow extends Model
    edge from: "graph_rag_users", to: "graph_rag_users"
end

class GraphRagPlain extends Model
end

let __db_available = false;
try
    let __probe = GraphRagUser.create({ "name": "__probe__", "vec": [1.0, 0.0, 0.0, 0.0] });
    if !__probe.nil? and !__probe._errors
        __db_available = true;
        __probe.delete();
    end
catch e
end

describe("graph_rag declaration guards (no DB)", fn() {
    test("graph_rag without vector_index raises", fn() {
        let msg = "";
        try
            GraphRagPlain.graph_rag("query", { "via": GraphRagFollow });
        catch e
            msg = str(e);
        end
        assert(msg.contains("vector_index"));
    });

    test("graph_rag without via raises", fn() {
        let msg = "";
        try
            GraphRagUser.graph_rag("query", { "seed_k": 3 });
        catch e
            msg = str(e);
        end
        assert(msg.contains("via"));
    });

    test("graph_rag without options hash raises", fn() {
        let msg = "";
        try
            GraphRagUser.graph_rag("query");
        catch e
            msg = str(e);
        end
        assert(msg.contains("options hash"));
    });
});

describe("traverse().similar() composition (no DB)", fn() {
    test("unsaved instance traverse still raises before similar", fn() {
        let user = GraphRagUser.new();
        let msg = "";
        try
            user.traverse(GraphRagFollow).similar("friends", "vec", 3);
        catch e
            msg = str(e);
        end
        assert(msg.contains("saved record"));
    });
});

describe("traverse().similar() (DB)", fn() {
    before_each(fn() {
        GraphRagFollow.delete_all() rescue null;
        GraphRagUser.delete_all() rescue null;
    });

    test("similar ranks traversal reach with _similarity_score", fn() {
        if !__db_available
            return;
        end

        let alice = GraphRagUser.create({ "name": "alice", "vec": [1.0, 0.0, 0.0, 0.0] });
        let bob   = GraphRagUser.create({ "name": "bob",   "vec": [0.9, 0.1, 0.0, 0.0] });
        let carol = GraphRagUser.create({ "name": "carol", "vec": [0.0, 1.0, 0.0, 0.0] });
        GraphRagFollow.create({ "from": alice, "to": bob });
        GraphRagFollow.create({ "from": alice, "to": carol });

        let results = alice.traverse(GraphRagFollow)
            .similar([1.0, 0.0, 0.0, 0.0], "vec", 2)
            .all;

        assert_eq(len(results), 2);
        assert_not_null(results[0]._similarity_score);
        assert(results[0]._similarity_score >= results[1]._similarity_score);
        let names = results.map(fn(r) r.name);
        assert(names.includes?("bob"));
        assert(names.includes?("carol"));
    });
});

describe("graph_rag() (DB)", fn() {
    before_each(fn() {
        GraphRagFollow.delete_all() rescue null;
        GraphRagUser.delete_all() rescue null;
    });

    test("graph_rag expands seeds through edges and attaches metadata", fn() {
        if !__db_available
            return;
        end

        let alice = GraphRagUser.create({ "name": "alice", "vec": [1.0, 0.0, 0.0, 0.0] });
        let bob   = GraphRagUser.create({ "name": "bob",   "vec": [0.85, 0.15, 0.0, 0.0] });
        let carol = GraphRagUser.create({ "name": "carol", "vec": [0.0, 1.0, 0.0, 0.0] });
        GraphRagFollow.create({ "from": alice, "to": bob });
        GraphRagFollow.create({ "from": alice, "to": carol });

        __sync_model_indexes() rescue null;

        let results = GraphRagUser.graph_rag("alice network", {
            "via": GraphRagFollow,
            "vector": [1.0, 0.0, 0.0, 0.0],
            "field": "vec",
            "seed_k": 1,
            "limit": 5
        });

        assert(results.length >= 2);
        assert_not_null(results[0]._similarity_score);
        assert_not_null(results[0]._graph_seed);
        assert_not_null(results[0]._graph_hops);

        let seed_count = results.filter(fn(r) r._graph_seed).length;
        assert(seed_count >= 1);
    });
});