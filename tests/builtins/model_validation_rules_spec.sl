# validates(): inclusion / one_of, type, allow_nil, custom (method and
# closure), and strict option checking. Every model here also has a failing
# `presence` rule on `title`, so no create reaches the database: the
# assertions read `_errors` only.

class TicketRule < Model
  validates("title", {"presence": true})
  validates("status", {"one_of": ["open", "closed"]})
  validates("priority", {"inclusion": [1, 2, 3], "allow_nil": true})
end

class TypedRule < Model
  validates("title", {"presence": true})
  validates("active", {"type": "boolean"})
  validates("tags", {"type": "array", "allow_null": true})
  validates("count", {"type": "int"})
  validates("label", {"type": "string"})
  validates("price", {"type": "float"})
  validates("meta", {"type": "hash"})
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

class PrivateRule < Model
  validates("title", {"presence": true})
  validates("name", {"custom": "_no_shouting"})
  validates("code", {"custom": fn(value, record) { record["name"] == value ? "must differ from name" : true }})
  validates("flag", {"custom": fn(value) { value != "off" }})

  private

  # One parameter: receives the field's value.
  def _no_shouting(name)
    return "can't be all caps" if name == name.upcase

    true
  end
end

def error_fields(record)
  (record._errors ?? []).map { |error| error["field"] }
end

def error_for(record, field)
  found = (record._errors ?? []).filter { |error| error["field"] == field }
  return nil if found.length == 0

  found[0]["message"]
end

describe("validates one_of / inclusion") do
  test("an unknown value is rejected") do
    record = TicketRule.create({"status": "bogus"})

    assert_eq(error_for(record, "status"), "is not included in the list")
  end

  test("an allowed value passes") do
    assert_eq(error_fields(TicketRule.create({"status": "open"})), ["title"])
  end

  test("values compare by type: 1 is not \"1\"") do
    assert_eq(error_for(TicketRule.create({"status": "open", "priority": "1"}), "priority"),
      "is not included in the list")
    assert_eq(error_fields(TicketRule.create({"status": "open", "priority": 2})), ["title"])
  end

  test("allow_nil skips a nil value") do
    assert_eq(error_fields(TicketRule.create({"status": "open", "priority": nil})), ["title"])
  end

  test("a nil or absent value is skipped even without allow_nil") do
    assert_eq(error_fields(TicketRule.create({})), ["title"])
    assert_eq(error_fields(TicketRule.create({"status": nil})), ["title"])
  end
end

describe("validates type") do
  test("a value of the wrong type is rejected") do
    assert_eq(error_for(TypedRule.create({"active": "yes"}), "active"), "must be a boolean")
  end

  test("the right type passes, a missing value is skipped") do
    assert_eq(error_fields(TypedRule.create({"active": false})), ["title"])
  end

  test("allow_null is an alias of allow_nil") do
    assert_eq(error_fields(TypedRule.create({"active": true, "tags": nil})), ["title"])
    assert_eq(error_for(TypedRule.create({"active": true, "tags": "a"}), "tags"), "must be an array")
  end

  test("int, string, float and hash name their type in the message") do
    record = TypedRule.create({"count": "1", "label": 3, "price": 1, "meta": []})

    assert_eq(error_for(record, "count"), "must be an int")
    assert_eq(error_for(record, "label"), "must be a string")
    assert_eq(error_for(record, "price"), "must be a float")
    assert_eq(error_for(record, "meta"), "must be a hash")
  end

  test("int, string, float and hash accept their own type") do
    record = TypedRule.create({"count": 1, "label": "x", "price": 1.5, "meta": {}})

    assert_eq(error_fields(record), ["title"])
  end
end

describe("validates custom") do
  test("a method pushing onto @_errors reports its error") do
    record = CustomRule.create({"name": "bad", "code": "ok"})

    assert_eq(error_for(record, "name"), "is reserved")
  end

  test("a method that pushes nothing passes") do
    assert_eq(error_fields(CustomRule.create({"name": "fine", "code": "ok"})), ["title"])
  end

  test("a closure returning a String reports it as the message") do
    assert_eq(error_for(CustomRule.create({"name": "fine", "code": "nope"}), "code"), "must be ok")
  end

  test("a method returning false is invalid, and may be declared later") do
    assert_eq(error_for(LateMethodRule.create({"slug": "admin"}), "slug"), "is invalid")
    assert_eq(error_fields(LateMethodRule.create({"slug": "home"})), ["title"])
  end

  test("a private one-parameter method receives the field value") do
    assert_eq(error_for(PrivateRule.create({"name": "LOUD", "code": "x"}), "name"), "can't be all caps")
    assert_eq(error_fields(PrivateRule.create({"name": "quiet", "code": "x"})), ["title"])
  end

  test("a two-parameter closure receives the attribute hash as the record") do
    record = PrivateRule.create({"name": "quiet", "code": "quiet"})

    assert_eq(error_for(record, "code"), "must differ from name")
  end

  test("a closure returning false is reported as invalid") do
    assert_eq(error_for(PrivateRule.create({"name": "quiet", "flag": "off"}), "flag"), "is invalid")
  end
end

describe("validates options are strict") do
  test("a misspelt option raises when the class loads") do
    assert_raises("unknown option `presense`") do
      class TypoRule < Model
        validates("title", {"presense": true})
      end
    end
  end

  test("a wrong-typed option value raises when the class loads") do
    assert_raises("`min_length:` expects a non-negative Int, got string") do
      class WrongTypeRule < Model
        validates("title", {"min_length": "3"})
      end
    end
  end
end
