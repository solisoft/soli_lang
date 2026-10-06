# `scope(name, fn)` in a Model class body. Inside the closure `this` is a fresh
# QueryBuilder for the model, and `Model.<name>` runs the closure and returns
# the refined QueryBuilder; scopes compose by chaining.

class Article < Model
  scope("published", fn() { this.where("status = @s", {"s": "published"}) })
  scope("recent", fn() { this.order("created_at", "desc").limit(10) })
  scope("identity", fn() { this })
end

describe("scope (class-body DSL)") do
  describe("the query it builds") do
    test("a where scope filters with its bind variables") do
      assert_eq(Article.published.class, "query_builder")
      assert_eq(Article.published.to_query,
        "FOR doc IN articles FILTER doc.status == @s RETURN doc | bind_vars: {\"s\": String(\"published\")}")
    end

    test("an order + limit scope sorts and limits") do
      assert_eq(Article.recent.to_query, "FOR doc IN articles SORT doc.created_at DESC LIMIT 10 RETURN doc")
    end

    test("a scope returning `this` is the unrefined QueryBuilder") do
      assert_eq(Article.identity.class, "query_builder")
      assert_eq(Article.identity.to_query, "FOR doc IN articles RETURN doc")
    end

    test("scopes compose in either order") do
      expected = "FOR doc IN articles FILTER doc.status == @s SORT doc.created_at DESC LIMIT 10 RETURN doc"

      assert_match(Article.published.recent.to_query, "^" + expected)
      assert_match(Article.recent.published.to_query, "^" + expected)
    end

    test("a scope chains after where, and where chains after a scope") do
      assert_match(Article.where({"x": 1}).published.to_query,
        "^FOR doc IN articles FILTER \\(doc.x == @x__eq_1\\) AND \\(doc.status == @s\\) RETURN doc")
      assert_match(Article.published.where({"x": 1}).to_query,
        "^FOR doc IN articles FILTER \\(doc.status == @s\\) AND \\(doc.x == @x__eq_1\\) RETURN doc")
    end

    test("an undeclared scope name raises") do
      assert_raises("Cannot access property 'unpublished' on Article") do
        Article.unpublished
      end
    end
  end

  describe("against the database") do
    before_each() do
      requires_solidb()
      ["2024-01", "2024-03", "2024-02"].each do |month|
        Article.create({"title": "post #{month}", "status": "published", "created_at": month})
      end
      Article.create({"title": "draft", "status": "draft", "created_at": "2024-04"})
    end

    after_each() do
      Article.delete_all()
    end

    test("a where scope returns only the matching rows") do
      titles = Article.published.all.map { |article| article.title }.sort()

      assert_eq(titles, ["post 2024-01", "post 2024-02", "post 2024-03"])
    end

    test("composed scopes filter and sort") do
      titles = Article.published.recent.all.map { |article| article.title }

      assert_eq(titles, ["post 2024-03", "post 2024-02", "post 2024-01"])
    end

    test("a scope works with count") do
      assert_eq(Article.published.count, 3)
      assert_eq(Article.identity.count, 4)
    end
  end
end
