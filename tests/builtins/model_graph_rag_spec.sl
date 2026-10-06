# Graph RAG: traverse().similar() ranks what a traversal reaches by vector
# similarity, and Model.graph_rag() expands vector-search seeds through edges.

class GraphRagUser < Model
  vector_index("vec", dimension: 4, metric: "cosine")
end

class GraphRagFollow < Model
  edge(from: "graph_rag_users", to: "graph_rag_users")
end

class GraphRagPlain < Model
end

# alice follows bob (a near neighbour in vector space) and carol (orthogonal).
def seed_network
  alice = GraphRagUser.create({"name": "alice", "vec": [1.0, 0.0, 0.0, 0.0]})
  bob = GraphRagUser.create({"name": "bob", "vec": [0.9, 0.1, 0.0, 0.0]})
  carol = GraphRagUser.create({"name": "carol", "vec": [0.0, 1.0, 0.0, 0.0]})
  GraphRagFollow.create({"from": alice, "to": bob})
  GraphRagFollow.create({"from": alice, "to": carol})
  # graph_rag's seed search needs the declared vector index to exist.
  __sync_model_indexes()
  alice
end

describe("graph_rag declaration guards (no DB)") do
  test("graph_rag without vector_index raises") do
    assert_raises("GraphRagPlain.graph_rag requires a `vector_index` declaration") do
      GraphRagPlain.graph_rag("query", {"via": GraphRagFollow})
    end
  end

  test("graph_rag without via raises") do
    assert_raises("GraphRagUser.graph_rag requires via: EdgeModel in the options hash") do
      GraphRagUser.graph_rag("query", {"seed_k": 3})
    end
  end

  test("graph_rag without an options hash raises") do
    assert_raises("GraphRagUser.graph_rag requires an options hash with via: EdgeModel") do
      GraphRagUser.graph_rag("query")
    end
  end
end

describe("traverse().similar() composition (no DB)") do
  test("an unsaved instance's traverse raises before similar") do
    assert_raises("traverse() requires a saved record (GraphRagUser instance has no _key)") do
      GraphRagUser.new().traverse(GraphRagFollow).similar("friends", "vec", 3)
    end
  end
end

describe("graph RAG (DB)") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    GraphRagFollow.delete_all()
    GraphRagUser.delete_all()
  end

  describe("traverse().similar()") do
    test("ranks the traversal's reach by _similarity_score") do
      alice = seed_network()

      results = alice.traverse(GraphRagFollow).similar([1.0, 0.0, 0.0, 0.0], "vec", 2).all

      assert_eq(results.map { |user| user.name }, ["bob", "carol"])
      assert_gt(results[0]._similarity_score, 0.99)
      assert_eq(results[1]._similarity_score, 0)
    end

    test("the limit keeps only the closest matches") do
      alice = seed_network()

      results = alice.traverse(GraphRagFollow).similar([1.0, 0.0, 0.0, 0.0], "vec", 1).all

      assert_eq(results.map { |user| user.name }, ["bob"])
    end
  end

  describe("graph_rag()") do
    test("expands seeds through edges and attaches metadata") do
      seed_network()

      results = GraphRagUser.graph_rag("alice network", {
        "via": GraphRagFollow,
        "vector": [1.0, 0.0, 0.0, 0.0],
        "field": "vec",
        "seed_k": 1,
        "limit": 5
      })

      # alice is the one seed (hop 0); bob and carol are reached in one hop.
      assert_eq(results.map { |user| [user.name, user._graph_seed, user._graph_hops] }, [
        ["alice", true, 0],
        ["bob", false, 1],
        ["carol", false, 1]
      ])
      assert_eq(results[0]._similarity_score, 1)
      assert_gt(results[1]._similarity_score, results[2]._similarity_score)
    end

    test("limit caps the expanded results") do
      seed_network()

      results = GraphRagUser.graph_rag("alice network", {
        "via": GraphRagFollow,
        "vector": [1.0, 0.0, 0.0, 0.0],
        "field": "vec",
        "seed_k": 1,
        "limit": 2
      })

      assert_eq(results.map { |user| user.name }, ["alice", "bob"])
    end
  end
end
