# Relation DSL (has_many, has_one, belongs_to, has_and_belongs_to_many) and the
# SDBQL it compiles to for includes, join, filtered includes and select. Every
# test reads `.to_query`; none needs a database.

class Writer < Model
  has_many("essays")
  has_one("biography")
end

class Essay < Model
  belongs_to("writer")
  has_many("remarks")
end

class Remark < Model
  belongs_to("essay")
end

class Label < Model
  has_and_belongs_to_many("essays")
end

class Journal < Model
  has_and_belongs_to_many("stickers", {"class_name": "Label"})
end

class Magazine < Model
  has_and_belongs_to_many("labels")
end

const ESSAYS_SUBQUERY = "LET _rel_essays = (FOR rel IN essays FILTER rel.writer_id == doc._key RETURN rel)"
const BIOGRAPHY_SUBQUERY = "LET _rel_biography = " +
  "(FOR rel IN biographies FILTER rel.writer_id == doc._key LIMIT 1 RETURN rel)"

# The SDBQL text of a builder, without the bind-variable dump that follows it.
def query_of(builder)
  builder.to_query.split(" | bind_vars:")[0]
end

describe("includes") do
  test("has_many loads the children through the foreign key") do
    expected = "FOR doc IN writers #{ESSAYS_SUBQUERY} RETURN MERGE(doc, {essays: _rel_essays})"
    assert_eq(query_of(Writer.includes("essays")), expected)
  end

  test("has_one stops at the first child and unwraps it") do
    expected = "FOR doc IN writers #{BIOGRAPHY_SUBQUERY} RETURN MERGE(doc, {biography: FIRST(_rel_biography)})"
    assert_eq(query_of(Writer.includes("biography")), expected)
  end

  test("belongs_to matches the parent key against the document's foreign key") do
    expected = "FOR doc IN essays " +
      "LET _rel_writer = (FOR rel IN writers FILTER rel._key == doc.writer_id LIMIT 1 RETURN rel) " +
      "RETURN MERGE(doc, {writer: FIRST(_rel_writer)})"
    assert_eq(query_of(Essay.includes("writer")), expected)
  end

  test("several relations share one MERGE") do
    expected = "FOR doc IN writers #{ESSAYS_SUBQUERY} #{BIOGRAPHY_SUBQUERY} " +
      "RETURN MERGE(doc, {essays: _rel_essays, biography: FIRST(_rel_biography)})"
    assert_eq(query_of(Writer.includes("essays", "biography")), expected)
  end

  test("a second-level model resolves its own relations") do
    expected = "FOR doc IN remarks " +
      "LET _rel_essay = (FOR rel IN essays FILTER rel._key == doc.essay_id LIMIT 1 RETURN rel) " +
      "RETURN MERGE(doc, {essay: FIRST(_rel_essay)})"
    assert_eq(query_of(Remark.includes("essay")), expected)
    remarks_query = query_of(Essay.includes("remarks"))
    assert_contains(remarks_query, "FOR rel IN remarks FILTER rel.essay_id == doc._key RETURN rel")
  end

  test("an undeclared relation is refused") do
    assert_raises("nope") do
      Writer.includes("nope").to_query
    end
  end
end

describe("join") do
  test("keeps parents with at least one child") do
    expected = "FOR doc IN writers " +
      "FILTER LENGTH(FOR rel IN essays FILTER rel.writer_id == doc._key LIMIT 1 RETURN 1) > 0 RETURN doc"
    assert_eq(query_of(Writer.join("essays")), expected)
  end

  test("ANDs a filter onto the child match") do
    query = query_of(Writer.join("essays", "published = @p", {"p": true}))
    assert_contains(query, "FILTER rel.writer_id == doc._key AND rel.published == @p LIMIT 1 RETURN 1")
  end
end

describe("chaining") do
  test("where before or after includes gives the same query") do
    includes_first = query_of(Writer.includes("essays").where("active = @a", {"a": true}))
    where_first = query_of(Writer.where("active = @a", {"a": true}).includes("essays"))
    assert_eq(includes_first, where_first)
    expected = "FOR doc IN writers FILTER doc.active == @a #{ESSAYS_SUBQUERY} RETURN MERGE(doc, {essays: _rel_essays})"
    assert_eq(where_first, expected)
  end

  test("a where after join adds a second FILTER") do
    query = query_of(Writer.join("essays").where("active = @a", {"a": true}))
    assert_contains(query, "LIMIT 1 RETURN 1) > 0 FILTER doc.active == @a RETURN doc")
  end

  test("the where filter text survives into the query") do
    query = query_of(Writer.includes("essays").where("name = @n", {"n": "Alice"}))
    assert_contains(query, "FILTER doc.name == @n LET _rel_essays")
  end
end

describe("filtered includes") do
  test("a filter narrows the children") do
    query = query_of(Writer.includes("essays", "published = @p", {"p": true}))
    assert_contains(query, "(FOR rel IN essays FILTER rel.writer_id == doc._key AND rel.published == @p RETURN rel)")
    assert_contains(Writer.includes("essays", "published = @p", {"p": true}).to_query, "bind_vars")
  end

  test("a fields key projects the children") do
    query = query_of(Writer.includes("essays", "published = @p", {"p": true, "fields": ["title", "body"]}))
    assert_contains(query, "AND rel.published == @p RETURN {title: rel.title, body: rel.body})")
  end

  test("a hash argument projects without a filter") do
    expected = "FOR doc IN writers LET _rel_essays = (FOR rel IN essays FILTER rel.writer_id == doc._key " +
      "RETURN {title: rel.title, body: rel.body}) RETURN MERGE(doc, {essays: _rel_essays})"
    assert_eq(query_of(Writer.includes({"essays": ["title", "body"]})), expected)
  end

  test("a hash argument with nil fields loads whole children") do
    assert_eq(query_of(Writer.includes({"essays": nil})), query_of(Writer.includes("essays")))
  end

  test("a filtered include chains with a plain one") do
    query = query_of(Writer.includes("essays", "published = @p", {"p": true}).includes("biography"))
    assert_contains(query, "AND rel.published == @p RETURN rel) #{BIOGRAPHY_SUBQUERY}")
    assert_contains(query, "RETURN MERGE(doc, {essays: _rel_essays, biography: FIRST(_rel_biography)})")
  end
end

describe("select and fields") do
  test("select projects the main collection and keeps _key") do
    expected = "FOR doc IN writers RETURN {name: doc.name, email: doc.email, _key: doc._key}"
    assert_eq(query_of(Writer.select("name", "email")), expected)
  end

  test("fields is an alias of select") do
    assert_eq(query_of(Writer.fields("name", "email")), query_of(Writer.select("name", "email")))
  end

  test("select merges the projection with includes") do
    query = query_of(Writer.select("name", "email").includes("essays"))
    assert_contains(query, "RETURN MERGE({name: doc.name, email: doc.email, _key: doc._key}, {essays: _rel_essays})")
  end

  test("select chains after where") do
    query = query_of(Writer.where("active = @a", {"a": true}).select("name"))
    assert_eq(query, "FOR doc IN writers FILTER doc.active == @a RETURN {name: doc.name, _key: doc._key}")
  end

  test("select with a filtered, projected include") do
    query = query_of(Writer.select("name").includes("essays", "published = @p", {"p": true, "fields": ["title"]}))
    assert_contains(query, "AND rel.published == @p RETURN {title: rel.title})")
    assert_contains(query, "RETURN MERGE({name: doc.name, _key: doc._key}, {essays: _rel_essays})")
  end
end

describe("has_and_belongs_to_many") do
  test("includes walks the join table, then the related collection") do
    expected = "FOR doc IN magazines LET _rel_labels = (FOR jt IN labels_magazines FILTER jt.magazine_id == doc._key " +
      "FOR rel IN labels FILTER rel._key == jt.label_id RETURN rel) RETURN MERGE(doc, {labels: _rel_labels})"
    assert_eq(query_of(Magazine.includes("labels")), expected)
  end

  test("join checks existence through the join table") do
    expected = "FOR doc IN magazines FILTER LENGTH(FOR jt IN labels_magazines FILTER jt.magazine_id == doc._key " +
      "FOR rel IN labels FILTER rel._key == jt.label_id LIMIT 1 RETURN 1) > 0 RETURN doc"
    assert_eq(query_of(Magazine.join("labels")), expected)
  end

  test("a filter applies to the related collection") do
    query = query_of(Magazine.includes("labels", "active = @a", {"a": true}))
    assert_contains(query, "FOR rel IN labels FILTER rel._key == jt.label_id AND rel.active == @a RETURN rel")
  end

  test("both sides name the join table alphabetically") do
    query = query_of(Label.includes("essays"))
    assert_contains(query, "FOR jt IN essays_labels FILTER jt.label_id == doc._key")
    assert_contains(query, "FOR rel IN essays FILTER rel._key == jt.essay_id")
  end

  test("class_name points the relation at another model") do
    expected = "FOR doc IN journals LET _rel_stickers = (FOR jt IN journals_labels FILTER jt.journal_id == doc._key " +
      "FOR rel IN labels FILTER rel._key == jt.label_id RETURN rel) RETURN MERGE(doc, {stickers: _rel_stickers})"
    assert_eq(query_of(Journal.includes("stickers")), expected)
  end
end
