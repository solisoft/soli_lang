# ============================================================================
# validates(): inclusion / one_of, type, allow_nil, custom (method and closure)
# Every model here also has a failing `presence` rule on `title`, so no
# create reaches the database: the assertions read `_errors` only.
# ============================================================================
class TicketRule < Model
  validates("title", {"presence": true})
  validates("status", {"one_of": ["open", "closed"]})
  validates("priority", {"inclusion": [1, 2, 3], "allow_nil": true})
end

class TypedRule < Model
  validates("title", {"presence": true})
  validates("active", {"type": "boolean"})
  validates("tags", {"type": "array", "allow_null": true})
end

class CustomRule < Model
  validates("title", {"presence": true})
  validates("name", {"custom": "check_name"})
  validates("code", {"custom": fn(value) { value == "ok" || "must be ok" }})

  def check_name
    return nil unless @name == "bad"

    @_errors = @_errors ?? []
    @_errors.push({"field": "name", "message": "is reserved"})
  end
end

class LateMethodRule < Model
  validates("title", {"presence": true})
  validates("slug", {"custom": "slug_ok?"})

  # Declared after the `validates` line: still found.
  def slug_ok?
    @slug != "admin"
  end
end

def error_fields(record)
  errors = record._errors ?? []
  errors.map { |e| e["field"] }
end

def error_for(record, field)
  errors = record._errors ?? []
  found = errors.filter { |e| e["field"] == field }
  return nil if found.length == 0

  found[0]["message"]
end

describe("validates one_of / inclusion", fn() {
  test("an unknown value is rejected", fn() {
    record = TicketRule.create({"status": "bogus"})
    assert_eq(error_for(record, "status"), "is not included in the list")
  })

  test("an allowed value passes", fn() {
    record = TicketRule.create({"status": "open"})
    assert_eq(error_fields(record), ["title"])
  })

  test("values compare by type: 1 is not \"1\"", fn() {
    record = TicketRule.create({"status": "open", "priority": "1"})
    assert_eq(error_for(record, "priority"), "is not included in the list")
    record = TicketRule.create({"status": "open", "priority": 2})
    assert_eq(error_fields(record), ["title"])
  })

  test("allow_nil skips a nil value", fn() {
    record = TicketRule.create({"status": "open", "priority": nil})
    assert_eq(error_fields(record), ["title"])
  })
})

describe("validates type", fn() {
  test("a value of the wrong type is rejected", fn() {
    record = TypedRule.create({"active": "yes"})
    assert_eq(error_for(record, "active"), "must be a boolean")
  })

  test("the right type passes, a missing value is skipped", fn() {
    record = TypedRule.create({"active": false})
    assert_eq(error_fields(record), ["title"])
  })

  test("allow_null is an alias of allow_nil", fn() {
    record = TypedRule.create({"active": true, "tags": nil})
    assert_eq(error_fields(record), ["title"])
    record = TypedRule.create({"active": true, "tags": "a"})
    assert_eq(error_for(record, "tags"), "must be an array")
  })
})

describe("validates custom", fn() {
  test("a method pushing onto @_errors reports its error", fn() {
    record = CustomRule.create({"name": "bad", "code": "ok"})
    assert_eq(error_for(record, "name"), "is reserved")
  })

  test("a method that pushes nothing passes", fn() {
    record = CustomRule.create({"name": "fine", "code": "ok"})
    assert_eq(error_fields(record), ["title"])
  })

  test("a closure returning a String reports it as the message", fn() {
    record = CustomRule.create({"name": "fine", "code": "nope"})
    assert_eq(error_for(record, "code"), "must be ok")
  })

  test("a method returning false is invalid, and may be declared later", fn() {
    record = LateMethodRule.create({"slug": "admin"})
    assert_eq(error_for(record, "slug"), "is invalid")
    record = LateMethodRule.create({"slug": "home"})
    assert_eq(error_fields(record), ["title"])
  })
})
