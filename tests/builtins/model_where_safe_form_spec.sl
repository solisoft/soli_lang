# Model.where Hash form (safe — SEC-005): each key is validated as a field
# identifier before any query is built, and every value travels as a bind
# parameter. The raw-string form `where("doc.x == @x", {...})` still works for
# developer-trusted call sites; only the Hash form is safe for untrusted input.

class WhereSafeItem < Model
end

describe("Model.where Hash form (safe)") do
  test("rejects an injection-shaped key") do
    assert_raises("invalid field name for SQL: \"name; REMOVE doc\"") do
      WhereSafeItem.where({"name; REMOVE doc": "x"})
    end
  end

  test("rejects a dotted key") do
    assert_raises("invalid field name for SQL: \"user.email\"") do
      WhereSafeItem.where({"user.email": "x"})
    end
  end

  test("rejects an empty key and one starting with a digit") do
    assert_raises("invalid field name for SQL: \"\"") do
      WhereSafeItem.where({"": "x"})
    end
    assert_raises("invalid field name for SQL: \"1abc\"") do
      WhereSafeItem.where({"1abc": "x"})
    end
  end

  test("rejects a second argument") do
    assert_raises("Model.where(Hash) takes a single argument") do
      WhereSafeItem.where({"name": "x"}, {"extra": 1})
    end
  end

  test("an empty hash adds no constraint") do
    assert_eq(WhereSafeItem.where({}).to_query, "FOR doc IN where_safe_items RETURN doc")
  end

  test("well-formed keys become filters with bound values") do
    query = WhereSafeItem.where({"name": "Alice", "active": true}).to_query

    assert_match(query, "^FOR doc IN where_safe_items FILTER doc.active == @active__eq_1 AND doc.name == @name__eq_2 ")
    assert_contains(query, "\"name__eq_2\": String(\"Alice\")")
  end

  test("an injection-shaped value stays a bind value") do
    query = WhereSafeItem.where({"name": "x; REMOVE doc IN where_safe_items"}).to_query

    assert_match(query, "^FOR doc IN where_safe_items FILTER doc.name == @name__eq_1 RETURN doc ")
    assert_contains(query, "String(\"x; REMOVE doc IN where_safe_items\")")
  end
end

describe("QueryBuilder.where Hash form (chain — SEC-005)") do
  test("rejects an injected key in the chain form") do
    assert_raises("invalid field name for SQL: \"x; REMOVE doc\"") do
      WhereSafeItem.order("name").where({"x; REMOVE doc": "y"})
    end
  end

  test("accepts a well-formed chained Hash filter") do
    assert_match(WhereSafeItem.order("name").where({"active": true}).to_query,
      "^FOR doc IN where_safe_items FILTER doc.active == @active__eq_1 SORT doc.name ASC RETURN doc")
  end
end

describe("Model.where String form still works for trusted call sites") do
  test("a string filter with binds") do
    assert_match(WhereSafeItem.where("doc.age >= @age", {"age": 18}).to_query,
      "^FOR doc IN where_safe_items FILTER doc.age >= @age RETURN doc")
  end

  test("a bare string filter with no binds") do
    assert_eq(WhereSafeItem.where("doc.active == true").to_query,
      "FOR doc IN where_safe_items FILTER doc.active == true RETURN doc")
  end
end

describe("Model.where Hash form against the database") do
  before_each() do
    requires_solidb()
    WhereSafeItem.create({"name": "Alice", "active": true})
    WhereSafeItem.create({"name": "Bob", "active": false})
  end

  after_each() do
    WhereSafeItem.delete_all()
  end

  test("matches on every key") do
    names = WhereSafeItem.where({"name": "Alice", "active": true}).all.map { |item| item.name }

    assert_eq(names, ["Alice"])
  end

  test("an injection-shaped value matches nothing and removes nothing") do
    assert_eq(WhereSafeItem.where({"name": "x\" || true || \"; REMOVE doc IN where_safe_items"}).count, 0)
    assert_eq(WhereSafeItem.count, 2)
  end
end
