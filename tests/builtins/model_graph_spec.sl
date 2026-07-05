# ============================================================================
# Model Graph (edge collections) Test Suite
# Tests for the `edge` declaration, endpoint coercion on create(),
# traverse() query building/execution, and shortest_path().
# ============================================================================

class GraphTestUser extends Model
end

class GraphFollow extends Model
    edge from: "graph_test_users", to: "graph_test_users"
end

# Detect DB availability
let __db_available = false;
try
    let __probe = GraphTestUser.create({ "name": "__probe__" });
    if !__probe.nil? and !__probe._errors
        __db_available = true;
        __probe.delete();
    end
catch e
end

# Seed a small follow chain: ga -> gb -> gc, plus isolated gd. Top-level
# (not describe-scope) because test closures don't see describe-scope
# definitions. Returns the users as a hash so each test picks what it needs.
fn seed_chain() {
    let ga = GraphTestUser.create({ "name": "ga" });
    let gb = GraphTestUser.create({ "name": "gb" });
    let gc = GraphTestUser.create({ "name": "gc" });
    let gd = GraphTestUser.create({ "name": "gd" });
    GraphFollow.create({ "from": ga, "to": gb, "since": 2020 });
    GraphFollow.create({ "from": gb, "to": gc, "since": 2024 });
    return { "ga": ga, "gb": gb, "gc": gc, "gd": gd };
}

# ============================================================================
# Tests that do NOT require a DB connection
# ============================================================================

describe("Edge model collection derivation", fn() {
    test("GraphFollow maps to the graph_follows collection", fn() {
        let q = GraphFollow.where("doc.since > 0").to_query;
        assert(q.contains("FOR doc IN graph_follows"));
    });

    test("GraphTestUser maps to the graph_test_users collection", fn() {
        let q = GraphTestUser.where("doc.name == @n", { "n": "x" }).to_query;
        assert(q.contains("FOR doc IN graph_test_users"));
    });
});

describe("Edge create endpoint validation (no DB)", fn() {
    test("missing both endpoints collects from and to errors", fn() {
        let f = GraphFollow.create({ "since": 2024 });
        assert_not_null(f._errors);
        assert_eq(len(f._errors), 2);
        assert_eq(f._errors[0]["field"], "from");
        assert(f._errors[0]["message"].contains("required"));
        assert_eq(f._errors[1]["field"], "to");
        assert(f._errors[1]["message"].contains("required"));
        assert_null(f._key);
    });

    test("named-arg form reaches the endpoint coercion", fn() {
        # Only to: given — from must be reported missing.
        let f = GraphFollow.create(to: "some_key");
        assert_not_null(f._errors);
        assert_eq(len(f._errors), 1);
        assert_eq(f._errors[0]["field"], "from");
        assert_null(f._key);
    });

    test("full id from the wrong collection is rejected", fn() {
        let f = GraphFollow.create({ "from": "other_coll/x", "to": "abc" });
        assert_not_null(f._errors);
        assert_eq(len(f._errors), 1);
        assert_eq(f._errors[0]["field"], "from");
        assert(f._errors[0]["message"].contains("does not belong"));
        assert_null(f._key);
    });

    test("full id with an empty key is rejected", fn() {
        let f = GraphFollow.create({ "from": "graph_test_users/", "to": "abc" });
        assert_not_null(f._errors);
        assert_eq(f._errors[0]["field"], "from");
        assert(f._errors[0]["message"].contains("missing a document key"));
    });

    test("empty-string endpoint is rejected as required", fn() {
        let f = GraphFollow.create({ "from": "", "to": "abc" });
        assert_not_null(f._errors);
        assert_eq(f._errors[0]["field"], "from");
        assert(f._errors[0]["message"].contains("required"));
    });

    test("unsaved model instance endpoint is rejected", fn() {
        let f = GraphFollow.create({ "from": GraphTestUser.new(), "to": "abc" });
        assert_not_null(f._errors);
        assert_eq(f._errors[0]["field"], "from");
        assert(f._errors[0]["message"].contains("saved record"));
    });
});

describe("traverse()/shortest_path() on unsaved records (no DB)", fn() {
    test("traverse on an unsaved instance raises", fn() {
        let u = GraphTestUser.new();
        let msg = "";
        try
            u.traverse(GraphFollow);
        catch e
            msg = str(e);
        end
        assert(msg.contains("saved record"));
    });

    test("shortest_path on an unsaved instance raises", fn() {
        let u = GraphTestUser.new();
        let target = GraphTestUser.new();
        let msg = "";
        try
            u.shortest_path(target, via: GraphFollow);
        catch e
            msg = str(e);
        end
        assert(msg.contains("saved record"));
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

describe("Edge create with valid endpoints (DB)", fn() {
    before_each(fn() {
        GraphFollow.delete_all() rescue null;
        GraphTestUser.delete_all() rescue null;
    });

    test("create(from:, to:) with instances writes _from/_to", fn() {
        if !__db_available
            return;
        end
        let alice = GraphTestUser.create({ "name": "e_alice" });
        let bob = GraphTestUser.create({ "name": "e_bob" });

        let follow = GraphFollow.create(from: alice, to: bob);

        assert_null(follow._errors);
        assert_not_null(follow._key);
        assert_eq(follow._from, "graph_test_users/" + alice._key);
        assert_eq(follow._to, "graph_test_users/" + bob._key);
    });

    test("hash form with full id + bare key persists extra fields", fn() {
        if !__db_available
            return;
        end
        let alice = GraphTestUser.create({ "name": "e_alice" });
        let bob = GraphTestUser.create({ "name": "e_bob" });

        let follow = GraphFollow.create({
            "from": "graph_test_users/" + alice._key,
            "to": bob._key,
            "since": 2024
        });

        assert_not_null(follow._key);
        assert_eq(follow._from, "graph_test_users/" + alice._key);
        assert_eq(follow._to, "graph_test_users/" + bob._key);

        # Reload from the DB — the edge is a real persisted document.
        let reloaded = GraphFollow.find(follow._key);
        assert_eq(reloaded._from, follow._from);
        assert_eq(reloaded._to, follow._to);
        assert_eq(reloaded.since, 2024);
    });
});

describe("traverse() traversal queries (DB)", fn() {
    before_each(fn() {
        GraphFollow.delete_all() rescue null;
        GraphTestUser.delete_all() rescue null;
    });

    test("to_query emits the traversal FOR-head", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let q = users["ga"].traverse(GraphFollow, depth: 3).to_query;
        assert(q.contains("OUTBOUND"));
        assert(q.contains("1..3"));
        assert(q.contains("@__soli_traverse_start"));
        assert(q.contains("graph_follows"));

        let q_in = users["ga"].traverse(GraphFollow, direction: "in").to_query;
        assert(q_in.contains("INBOUND"));
        assert(q_in.contains("1..1"));

        let q_any = users["ga"].traverse(GraphFollow, direction: "any", depth: [2, 3]).to_query;
        assert(q_any.contains("ANY"));
        assert(q_any.contains("2..3"));
    });

    test("default traversal is OUTBOUND depth 1..1", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let result = users["ga"].traverse(GraphFollow).all;
        assert_eq(len(result), 1);
        assert_eq(result[0].name, "gb");
    });

    test("depth [1, 2] reaches the friend-of-friend", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let result = users["ga"].traverse(GraphFollow, depth: [1, 2]).all;
        assert_eq(len(result), 2);
        let names = result.map(fn(v) v.name);
        assert(names.includes?("gb"));
        assert(names.includes?("gc"));
    });

    test("direction in walks edges backwards", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let result = users["gb"].traverse(GraphFollow, direction: "in").all;
        assert_eq(len(result), 1);
        assert_eq(result[0].name, "ga");
    });

    test("direction any sees both neighbors", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let result = users["gb"].traverse(GraphFollow, direction: "any").all;
        assert_eq(len(result), 2);
        let names = result.map(fn(v) v.name);
        assert(names.includes?("ga"));
        assert(names.includes?("gc"));
    });

    test("count terminal works on traversals", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        assert_eq(users["ga"].traverse(GraphFollow, depth: [1, 2]).count, 2);
        assert_eq(users["gd"].traverse(GraphFollow).count, 0);
    });

    test("where() filters on vertex fields", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let result = users["ga"].traverse(GraphFollow, depth: [1, 2])
            .where({ "name": "gc" })
            .all;
        assert_eq(len(result), 1);
        assert_eq(result[0].name, "gc");
    });

    test("where() filters on edge attributes via the edge variable", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let qb = users["ga"].traverse(GraphFollow, depth: [1, 2])
            .where("edge.since >= @y", { "y": 2024 });
        assert(qb.to_query.contains("FILTER edge.since >= @y"));

        let result = qb.all;
        assert_eq(len(result), 1);
        assert_eq(result[0].name, "gc");
    });

    test("order and limit compose with traversals", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let result = users["ga"].traverse(GraphFollow, depth: [1, 2])
            .order("name", "asc")
            .limit(1)
            .all;
        assert_eq(len(result), 1);
        assert_eq(result[0].name, "gb");
    });

    test("raw edge-collection name works in place of the model", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let result = users["ga"].traverse("graph_follows").all;
        assert_eq(len(result), 1);
        assert_eq(result[0].name, "gb");
    });

    test("invalid options raise", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let ga = users["ga"];

        let depth_msg = "";
        try
            ga.traverse(GraphFollow, depth: 0);
        catch e
            depth_msg = str(e);
        end
        assert(depth_msg.contains("depth must be >= 1"));

        let dir_msg = "";
        try
            ga.traverse(GraphFollow, direction: "sideways");
        catch e
            dir_msg = str(e);
        end
        assert(dir_msg.contains("invalid traversal direction"));

        let edge_msg = "";
        try
            ga.traverse(GraphTestUser);
        catch e
            edge_msg = str(e);
        end
        assert(edge_msg.contains("no `edge` declaration"));
    });

    test("traversals reject eager loading, group_by and bulk writes", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let qb = users["ga"].traverse(GraphFollow);

        let inc_msg = "";
        try
            qb.includes("posts");
        catch e
            inc_msg = str(e);
        end
        assert(inc_msg.contains("cannot be combined with traverse()"));

        let grp_msg = "";
        try
            qb.group_by("name", "sum", "since");
        catch e
            grp_msg = str(e);
        end
        assert(grp_msg.contains("cannot be combined with traverse()"));

        let del_msg = "";
        try
            qb.delete_all;
        catch e
            del_msg = str(e);
        end
        assert(del_msg.contains("cannot be combined with traverse()"));

        let upd_msg = "";
        try
            qb.update_all({ "x": 1 });
        catch e
            upd_msg = str(e);
        end
        assert(upd_msg.contains("cannot be combined with traverse()"));
    });
});

describe("shortest_path() (DB)", fn() {
    before_each(fn() {
        GraphFollow.delete_all() rescue null;
        GraphTestUser.delete_all() rescue null;
    });

    test("returns the vertices along the path, start first", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let path = users["ga"].shortest_path(users["gc"], via: GraphFollow);
        assert_eq(len(path), 3);
        assert_eq(path[0].name, "ga");
        assert_eq(path[1].name, "gb");
        assert_eq(path[2].name, "gc");
    });

    test("returns [] when the vertices are unconnected", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let path = users["ga"].shortest_path(users["gd"], via: GraphFollow);
        assert_eq(len(path), 0);
    });

    test("target accepts a full id or a bare key", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let by_id = users["ga"].shortest_path(
            "graph_test_users/" + users["gc"]._key,
            via: GraphFollow
        );
        assert_eq(len(by_id), 3);

        let by_key = users["ga"].shortest_path(users["gc"]._key, via: GraphFollow);
        assert_eq(len(by_key), 3);
    });

    test("default direction any finds the reverse path", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let path = users["gc"].shortest_path(users["ga"], via: GraphFollow);
        assert_eq(len(path), 3);
        assert_eq(path[0].name, "gc");
        assert_eq(path[2].name, "ga");
    });

    test("direction out from the sink finds nothing", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let path = users["gc"].shortest_path(users["ga"], via: GraphFollow, direction: "out");
        assert_eq(len(path), 0);
    });

    test("missing via: raises", fn() {
        if !__db_available
            return;
        end
        let users = seed_chain();
        let msg = "";
        try
            users["ga"].shortest_path(users["gc"]);
        catch e
            msg = str(e);
        end
        assert(msg.contains("via:"));
    });
});
