# rerank(query, records, {field:, limit:}) — client-side lexical reranking:
# most query-token matches first, original order on ties. Offline, no LLM.

class Snippet
  content: String

  new(content: String)
    @content = content
  end
end

def contents(records)
  records.map { |record| record["content"] }
end

describe("rerank") do
  describe("ordering") do
    test("orders records by query-token overlap") do
      docs = [
        {"content": "the quick brown fox jumps over"},
        {"content": "vector databases store embeddings for search"},
        {"content": "graph traversal and vector search combined"}
      ]
      # 3 matching tokens, then 2, then 0.
      assert_eq(contents(rerank("vector search embeddings", docs)), [
        "vector databases store embeddings for search",
        "graph traversal and vector search combined",
        "the quick brown fox jumps over"
      ])
    end

    test("keeps the original order between equal scores") do
      docs = [{"content": "alpha one"}, {"content": "beta"}, {"content": "alpha two"}, {"content": "gamma"}]
      assert_eq(contents(rerank("alpha", docs)), ["alpha one", "alpha two", "beta", "gamma"])
    end

    test("matches case-insensitively, on word boundaries") do
      docs = [{"content": "nothing here"}, {"content": "VECTOR-Search!"}]
      assert_eq(rerank("vector search", docs)[0]["content"], "VECTOR-Search!")
    end

    test("ignores one-character tokens") do
      docs = [{"content": "a b c"}, {"content": "cat"}]
      assert_eq(contents(rerank("a b cat", docs)), ["cat", "a b c"])
    end

    test("a query with no usable token keeps the input order") do
      docs = [{"content": "first"}, {"content": "second"}]
      assert_eq(contents(rerank("?", docs)), ["first", "second"])
    end

    test("does not modify the input array") do
      docs = [{"content": "unrelated"}, {"content": "match"}]
      rerank("match", docs)
      assert_eq(contents(docs), ["unrelated", "match"])
    end
  end

  describe("ranking text") do
    test("an explicit field selects the text") do
      docs = [
        {"title": "unrelated topic", "body": "mentions vector search here"},
        {"title": "vector search", "body": "nope"}
      ]
      assert_eq(rerank("vector search", docs, {"field": "title"})[0]["title"], "vector search")
    end

    test("without a field it falls back through text, summary, body and title") do
      docs = [{"title": "nothing"}, {"summary": "about rust"}, {"text": "rust and rust"}]
      ranked = rerank("rust", docs)
      assert_eq(ranked[0], {"text": "rust and rust"})
      assert_eq(ranked[1], {"summary": "about rust"})
      assert_eq(ranked[2], {"title": "nothing"})
    end

    test("ranks object instances by their fields") do
      docs = [new Snippet("plain words"), new Snippet("soli language")]
      assert_eq(rerank("soli", docs)[0].content, "soli language")
    end
  end

  describe("limit") do
    test("truncates to the top-k after reordering") do
      docs = [{"content": "the quick brown fox"}, {"content": "vector search embeddings"}]
      ranked = rerank("vector search embeddings", docs, {"limit": 1})
      assert_eq(contents(ranked), ["vector search embeddings"])
    end

    test("a limit of 0 returns nothing, a large one returns everything") do
      docs = [{"content": "one"}, {"content": "two"}]
      assert_eq(rerank("one", docs, {"limit": 0}), [])
      assert_eq(rerank("one", docs, {"limit": 10}).length, 2)
    end
  end

  describe("input checks") do
    test("an empty array returns an empty array") do
      assert_eq(rerank("anything", []), [])
    end

    test("the query must be a string") do
      assert_raises("rerank expects (query_string, docs_array[, options])") do
        rerank(42, [])
      end
    end

    test("the records must be an array") do
      assert_raises("rerank: second argument must be an array of records") do
        rerank("query", "not an array")
      end
    end

    test("an unknown or invalid option is refused") do
      assert_raises("rerank() unknown/invalid option 'top'") do
        rerank("query", [], {"top": 3})
      end
      assert_raises("unknown/invalid option 'limit'") do
        rerank("query", [], {"limit": -1})
      end
    end
  end
end
