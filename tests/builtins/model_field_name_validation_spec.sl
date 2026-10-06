# AQL injection guard on field-name arguments. Every Model method that
# interpolates a field name into a query validates it against
# [A-Za-z_][A-Za-z0-9_]* and raises before the query is built, so a
# controller passing request params straight through cannot inject SDBQL.

class FieldGuardItem < Model
end

const WORD_CHARS_ONLY = "field name may only contain letters, digits, and underscores"
const ASC_OR_DESC = "direction must be one of asc/desc/ascending/descending"

describe("Field-name validator — find_by / first_by / find_or_create_by") do
  test("rejects a name with a space") do
    assert_raises("find_by() #{WORD_CHARS_ONLY} — got \"name ; REMOVE\"") do
      FieldGuardItem.find_by("name ; REMOVE", "x")
    end
  end

  test("rejects a name with a quote") do
    assert_raises("first_by() #{WORD_CHARS_ONLY}") do
      FieldGuardItem.first_by("name'", "x")
    end
  end

  test("rejects a name with a dot (defense-in-depth)") do
    assert_raises("find_or_create_by() #{WORD_CHARS_ONLY}") do
      FieldGuardItem.find_or_create_by("user.email", "x", {})
    end
  end

  test("rejects an empty name") do
    assert_raises("find_by() field name must start with a letter or underscore — got \"\"") do
      FieldGuardItem.find_by("", "x")
    end
  end

  test("rejects a name that starts with a digit") do
    assert_raises("find_by() field name must start with a letter or underscore — got \"1col\"") do
      FieldGuardItem.find_by("1col", "x")
    end
  end
end

describe("Field-name validator — order / select / pluck / aggregations / group_by") do
  test("order rejects parens") do
    assert_raises("order() #{WORD_CHARS_ONLY}") do
      FieldGuardItem.order("name); REMOVE doc")
    end
  end

  test("select rejects spaces") do
    assert_raises("select() #{WORD_CHARS_ONLY}") do
      FieldGuardItem.select("a b")
    end
  end

  test("pluck rejects semicolons") do
    assert_raises("pluck() #{WORD_CHARS_ONLY}") do
      FieldGuardItem.pluck("a;b")
    end
  end

  test("sum rejects dashes") do
    assert_raises("sum() #{WORD_CHARS_ONLY}") do
      FieldGuardItem.sum("a-b")
    end
  end

  test("group_by rejects an injected agg field") do
    assert_raises("group_by() #{WORD_CHARS_ONLY} — got \"amount; REMOVE doc\"") do
      FieldGuardItem.group_by("status", "sum", "amount; REMOVE doc")
    end
  end

  test("group_by rejects an injected group field") do
    assert_raises("group_by() #{WORD_CHARS_ONLY} — got \"status; REMOVE doc\"") do
      FieldGuardItem.group_by("status; REMOVE doc", "sum", "amount")
    end
  end
end

describe("Field-name validator — well-formed names still work") do
  test("snake_case and underscore-led names pass") do
    assert_eq(
      FieldGuardItem.select("user_id", "_internal", "x9").to_query,
      "FOR doc IN field_guard_items RETURN {user_id: doc.user_id, _internal: doc._internal, x9: doc.x9, _key: doc._key}"
    )
  end

  test("PascalCase passes") do
    assert_eq(FieldGuardItem.pluck("UserId").to_query, "FOR doc IN field_guard_items RETURN doc.UserId")
  end
end

# SEC-004a: the order direction is restricted to asc/desc/ascending/descending,
# case-insensitively — on the static entry and on the chain form.
describe("Order direction validator — Model.order entry") do
  test("rejects an injected direction") do
    assert_raises("order() #{ASC_OR_DESC} — got \"ASC; REMOVE doc IN x\"") do
      FieldGuardItem.order("name", "ASC; REMOVE doc IN x")
    end
  end

  test("rejects an arbitrary unknown direction") do
    assert_raises("order() #{ASC_OR_DESC} — got \"sideways\"") do
      FieldGuardItem.order("name", "sideways")
    end
  end

  test("accepts asc/desc in any case and the long forms") do
    assert_eq(FieldGuardItem.order("name", "asc").to_query, "FOR doc IN field_guard_items SORT doc.name ASC RETURN doc")
    assert_eq(
      FieldGuardItem.order("name", "DESC").to_query,
      "FOR doc IN field_guard_items SORT doc.name DESC RETURN doc"
    )
    assert_eq(
      FieldGuardItem.order("name", "Ascending").to_query,
      "FOR doc IN field_guard_items SORT doc.name ASC RETURN doc"
    )
    assert_eq(
      FieldGuardItem.order("name", "descending").to_query,
      "FOR doc IN field_guard_items SORT doc.name DESC RETURN doc"
    )
  end

  test("defaults to ascending") do
    assert_eq(FieldGuardItem.order("name").to_query, "FOR doc IN field_guard_items SORT doc.name ASC RETURN doc")
  end
end

describe("Order direction validator — QueryBuilder.order chain") do
  test("rejects an injected direction in the chain form") do
    assert_raises("order() #{ASC_OR_DESC} — got \"; REMOVE doc\"") do
      FieldGuardItem.order("name").order("name", "; REMOVE doc")
    end
  end
end

# SEC-004b: the chain form `Model.where(...).order(field, dir)` shares the SORT
# sink with the static `Model.order`, so the field-name check applies there too.
describe("Field-name validator — QueryBuilder.order chain (SEC-004b)") do
  test("rejects an injected field name in the chain form") do
    assert_raises("order() #{WORD_CHARS_ONLY}") do
      FieldGuardItem.order("name").order("name; REMOVE doc IN x", "asc")
    end
  end

  test("rejects a dotted field name in the chain form") do
    assert_raises("order() #{WORD_CHARS_ONLY} — got \"user.email\"") do
      FieldGuardItem.order("name").order("user.email", "asc")
    end
  end

  test("accepts a well-formed field name in the chain form") do
    assert_eq(
      FieldGuardItem.order("name").order("created_at", "desc").to_query,
      "FOR doc IN field_guard_items SORT doc.created_at DESC RETURN doc"
    )
  end
end

# SEC-004c: the other chain methods that take field names
# (select/pluck/aggregate/group_by) share sinks with their static
# counterparts, so the same validator applies on the chain side.
describe("Field-name validator — QueryBuilder.select chain (SEC-004c)") do
  test("rejects an injected field name in the chain form") do
    assert_raises("select() #{WORD_CHARS_ONLY}") do
      FieldGuardItem.order("name").select("name; REMOVE doc")
    end
  end

  test("accepts a well-formed name in the chain form") do
    assert_eq(
      FieldGuardItem.order("name").select("user_id", "email").to_query,
      "FOR doc IN field_guard_items SORT doc.name ASC RETURN {user_id: doc.user_id, email: doc.email, _key: doc._key}"
    )
  end
end

describe("Field-name validator — QueryBuilder.pluck chain (SEC-004c)") do
  test("rejects an injected field name in the chain form") do
    assert_raises("pluck() #{WORD_CHARS_ONLY}") do
      FieldGuardItem.order("name").pluck("a; REMOVE doc IN x")
    end
  end
end

describe("Field-name validator — QueryBuilder.{sum,avg,min,max} chain (SEC-004c)") do
  test("sum rejects an injected field") do
    assert_raises("aggregate() #{WORD_CHARS_ONLY}") do
      FieldGuardItem.order("name").sum("amount; REMOVE doc")
    end
  end

  test("avg rejects an injected field") do
    assert_raises("aggregate() #{WORD_CHARS_ONLY} — got \"a-b\"") do
      FieldGuardItem.order("name").avg("a-b")
    end
  end

  test("min rejects an injected field") do
    assert_raises("aggregate() #{WORD_CHARS_ONLY} — got \"a b\"") do
      FieldGuardItem.order("name").min("a b")
    end
  end

  test("max rejects an injected field") do
    assert_raises("aggregate() #{WORD_CHARS_ONLY} — got \"a)\"") do
      FieldGuardItem.order("name").max("a)")
    end
  end
end

describe("Field-name validator — QueryBuilder.group_by chain (SEC-004c)") do
  test("rejects an injected group field in the chain form") do
    assert_raises("group_by() #{WORD_CHARS_ONLY} — got \"status; REMOVE doc\"") do
      FieldGuardItem.order("name").group_by("status; REMOVE doc", "sum", "amount")
    end
  end

  test("rejects an injected agg field in the chain form") do
    assert_raises("group_by() #{WORD_CHARS_ONLY} — got \"amount; REMOVE doc\"") do
      FieldGuardItem.order("name").group_by("status", "sum", "amount; REMOVE doc")
    end
  end
end
