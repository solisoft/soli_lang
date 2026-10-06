# Edge collections: the `edge` declaration, endpoint coercion on create(),
# traverse() query building and execution, and shortest_path().

class GraphTestUser < Model
end

class GraphFollow < Model
  edge(from: "graph_test_users", to: "graph_test_users")
end

# A follow chain ga -> gb -> gc, plus an isolated gd.
def seed_chain
  ga = GraphTestUser.create({"name": "ga"})
  gb = GraphTestUser.create({"name": "gb"})
  gc = GraphTestUser.create({"name": "gc"})
  gd = GraphTestUser.create({"name": "gd"})
  GraphFollow.create({"from": ga, "to": gb, "since": 2020})
  GraphFollow.create({"from": gb, "to": gc, "since": 2024})
  {"ga": ga, "gb": gb, "gc": gc, "gd": gd}
end

def names(vertices)
  vertices.map { |vertex| vertex.name }
end

describe("Edge model collection derivation") do
  test("GraphFollow maps to the graph_follows collection") do
    assert_eq(GraphFollow.where("doc.since > 0").to_query, "FOR doc IN graph_follows FILTER doc.since > 0 RETURN doc")
  end

  test("GraphTestUser maps to the graph_test_users collection") do
    query = GraphTestUser.where("doc.name == @n", {"n": "x"}).to_query

    assert_contains(query, "FOR doc IN graph_test_users FILTER doc.name == @n RETURN doc")
  end
end

describe("Edge create endpoint validation (no DB)") do
  test("missing both endpoints collects from and to errors") do
    follow = GraphFollow.create({"since": 2024})

    assert_eq(follow._errors, [
      {"field": "from", "message": "from is required"},
      {"field": "to", "message": "to is required"}
    ])
    assert_null(follow._key)
  end

  test("named-arg form reaches the endpoint coercion") do
    # Only to: given — from must be reported missing.
    follow = GraphFollow.create(to: "some_key")

    assert_eq(follow._errors, [{"field": "from", "message": "from is required"}])
    assert_null(follow._key)
  end

  test("a full id from the wrong collection is rejected") do
    follow = GraphFollow.create({"from": "other_coll/x", "to": "abc"})

    assert_eq(follow._errors, [{
      "field": "from",
      "message": "from: 'other_coll/x' does not belong to the declared 'graph_test_users' collection"
    }])
    assert_null(follow._key)
  end

  test("a full id with an empty key is rejected") do
    follow = GraphFollow.create({"from": "graph_test_users/", "to": "abc"})

    assert_eq(follow._errors, [{"field": "from", "message": "from: 'graph_test_users/' is missing a document key"}])
  end

  test("an empty-string endpoint is rejected as required") do
    follow = GraphFollow.create({"from": "", "to": "abc"})

    assert_eq(follow._errors, [{"field": "from", "message": "from is required"}])
  end

  test("an unsaved model instance endpoint is rejected") do
    follow = GraphFollow.create({"from": GraphTestUser.new(), "to": "abc"})

    assert_eq(follow._errors, [{
      "field": "from",
      "message": "from: expected a saved record (GraphTestUser instance has no _key)"
    }])
  end

  test("an endpoint of the wrong type is rejected") do
    follow = GraphFollow.create({"from": 42, "to": "abc"})

    assert_eq(follow._errors[0]["field"], "from")
    assert_contains(follow._errors[0]["message"], "expected a model instance, \"coll/key\" id, or key string, got int")
  end
end

describe("traverse()/shortest_path() on unsaved records (no DB)") do
  test("traverse on an unsaved instance raises") do
    assert_raises("traverse() requires a saved record (GraphTestUser instance has no _key)") do
      GraphTestUser.new().traverse(GraphFollow)
    end
  end

  test("shortest_path on an unsaved instance raises") do
    assert_raises("shortest_path() requires a saved record (GraphTestUser instance has no _key)") do
      GraphTestUser.new().shortest_path(GraphTestUser.new(), via: GraphFollow)
    end
  end
end

describe("graphs (DB)") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    GraphFollow.delete_all()
    GraphTestUser.delete_all()
  end

  describe("Edge create with valid endpoints") do
    test("create(from:, to:) with instances writes _from/_to") do
      alice = GraphTestUser.create({"name": "e_alice"})
      bob = GraphTestUser.create({"name": "e_bob"})

      follow = GraphFollow.create(from: alice, to: bob)

      assert_null(follow._errors)
      assert_eq(follow._from, "graph_test_users/" + alice._key)
      assert_eq(follow._to, "graph_test_users/" + bob._key)
      assert_eq(GraphFollow.find(follow._key)._from, "graph_test_users/" + alice._key)
    end

    test("hash form with full id + bare key persists extra fields") do
      alice = GraphTestUser.create({"name": "e_alice"})
      bob = GraphTestUser.create({"name": "e_bob"})

      follow = GraphFollow.create({"from": "graph_test_users/" + alice._key, "to": bob._key, "since": 2024})

      assert_eq(follow._from, "graph_test_users/" + alice._key)
      assert_eq(follow._to, "graph_test_users/" + bob._key)
      # Reload from the DB — the edge is a real persisted document.
      reloaded = GraphFollow.find(follow._key)
      assert_eq(reloaded._from, follow._from)
      assert_eq(reloaded._to, follow._to)
      assert_eq(reloaded.since, 2024)
    end
  end

  describe("traverse() traversal queries") do
    test("to_query emits the traversal FOR-head") do
      users = seed_chain()

      assert_contains(
        users["ga"].traverse(GraphFollow, depth: 3).to_query,
        "FOR doc, edge IN 1..3 OUTBOUND @__soli_traverse_start graph_follows RETURN doc"
      )
      assert_contains(
        users["ga"].traverse(GraphFollow, direction: "in").to_query,
        "FOR doc, edge IN 1..1 INBOUND @__soli_traverse_start graph_follows RETURN doc"
      )
      assert_contains(
        users["ga"].traverse(GraphFollow, direction: "any", depth: [2, 3]).to_query,
        "FOR doc, edge IN 2..3 ANY @__soli_traverse_start graph_follows RETURN doc"
      )
    end

    test("the start vertex travels as a bind var") do
      users = seed_chain()

      assert_contains(
        users["ga"].traverse(GraphFollow).to_query,
        "\"__soli_traverse_start\": String(\"graph_test_users/#{users["ga"]._key}\")"
      )
    end

    test("default traversal is OUTBOUND depth 1..1") do
      users = seed_chain()

      assert_eq(names(users["ga"].traverse(GraphFollow).all), ["gb"])
    end

    test("traversals return model instances") do
      users = seed_chain()

      assert(users["ga"].traverse(GraphFollow).first.is_a?("GraphTestUser"))
    end

    test("depth [1, 2] reaches the friend-of-friend") do
      users = seed_chain()

      assert_eq(names(users["ga"].traverse(GraphFollow, depth: [1, 2]).all).sort(), ["gb", "gc"])
    end

    test("depth [2, 2] skips the direct neighbor") do
      users = seed_chain()

      assert_eq(names(users["ga"].traverse(GraphFollow, depth: [2, 2]).all), ["gc"])
    end

    test("direction in walks edges backwards") do
      users = seed_chain()

      assert_eq(names(users["gb"].traverse(GraphFollow, direction: "in").all), ["ga"])
    end

    test("direction any sees both neighbors") do
      users = seed_chain()

      assert_eq(names(users["gb"].traverse(GraphFollow, direction: "any").all).sort(), ["ga", "gc"])
    end

    test("count terminal works on traversals") do
      users = seed_chain()

      assert_eq(users["ga"].traverse(GraphFollow, depth: [1, 2]).count, 2)
      assert_eq(users["gd"].traverse(GraphFollow).count, 0)
    end

    test("where() filters on vertex fields") do
      users = seed_chain()

      result = users["ga"].traverse(GraphFollow, depth: [1, 2]).where({"name": "gc"}).all

      assert_eq(names(result), ["gc"])
    end

    test("where() filters on edge attributes via the edge variable") do
      users = seed_chain()

      query_builder = users["ga"].traverse(GraphFollow, depth: [1, 2]).where("edge.since >= @y", {"y": 2024})

      assert_contains(query_builder.to_query, "graph_follows FILTER edge.since >= @y RETURN doc")
      assert_eq(names(query_builder.all), ["gc"])
    end

    test("order and limit compose with traversals") do
      users = seed_chain()

      result = users["ga"].traverse(GraphFollow, depth: [1, 2]).order("name", "desc").limit(1).all

      assert_eq(names(result), ["gc"])
    end

    test("a raw edge-collection name works in place of the model") do
      users = seed_chain()

      assert_eq(names(users["ga"].traverse("graph_follows").all), ["gb"])
    end
  end

  describe("traverse() refusals") do
    test("a depth below 1 raises") do
      users = seed_chain()

      assert_raises("traverse() depth must be >= 1") do
        users["ga"].traverse(GraphFollow, depth: 0)
      end
    end

    test("an unknown direction raises") do
      users = seed_chain()

      assert_raises("invalid traversal direction 'sideways': expected \"out\", \"in\", or \"any\"") do
        users["ga"].traverse(GraphFollow, direction: "sideways")
      end
    end

    test("a model without an edge declaration raises") do
      users = seed_chain()

      assert_raises("GraphTestUser has no `edge` declaration") do
        users["ga"].traverse(GraphTestUser)
      end
    end

    test("traversals reject eager loading, group_by and bulk writes") do
      users = seed_chain()
      query_builder = users["ga"].traverse(GraphFollow)

      assert_raises("includes() cannot be combined with traverse()") do
        query_builder.includes("posts")
      end
      assert_raises("group_by() cannot be combined with traverse()") do
        query_builder.group_by("name", "sum", "since")
      end
      assert_raises("delete_all() cannot be combined with traverse()") do
        query_builder.delete_all
      end
      assert_raises("update_all() cannot be combined with traverse()") do
        query_builder.update_all({"x": 1})
      end
    end
  end

  describe("shortest_path()") do
    test("returns the vertices along the path, start first") do
      pending("bug (SoliDB): SHORTEST_PATH returns only the destination vertex, not every hop")
      users = seed_chain()

      assert_eq(names(users["ga"].shortest_path(users["gc"], via: GraphFollow)), ["ga", "gb", "gc"])
      # The default direction is any, so the reverse path exists too.
      assert_eq(names(users["gc"].shortest_path(users["ga"], via: GraphFollow)), ["gc", "gb", "ga"])
    end

    test("the path ends at the target, given as an instance, a full id or a bare key") do
      users = seed_chain()
      target_key = users["gc"]._key

      by_instance = users["ga"].shortest_path(users["gc"], via: GraphFollow)
      by_id = users["ga"].shortest_path("graph_test_users/" + target_key, via: GraphFollow)
      by_key = users["ga"].shortest_path(target_key, via: GraphFollow)

      assert_eq(by_instance.last.name, "gc")
      assert_eq(by_id.last.name, "gc")
      assert_eq(by_key.last.name, "gc")
    end

    test("the default direction any finds the reverse path") do
      users = seed_chain()

      assert_eq(users["gc"].shortest_path(users["ga"], via: GraphFollow).last.name, "ga")
    end

    test("returns [] when the vertices are unconnected") do
      users = seed_chain()

      assert_eq(users["ga"].shortest_path(users["gd"], via: GraphFollow), [])
    end

    test("direction out from the sink finds nothing") do
      users = seed_chain()

      assert_eq(users["gc"].shortest_path(users["ga"], via: GraphFollow, direction: "out"), [])
    end

    test("missing via: raises") do
      users = seed_chain()

      assert_raises("shortest_path() requires via: an edge model") do
        users["ga"].shortest_path(users["gc"])
      end
    end
  end
end
