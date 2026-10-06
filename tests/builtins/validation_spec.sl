# validate(data, schema) with the V validator builders: types and coercion,
# rules, optional/nullable/default, confirmation, nested schemas, and the
# password-rules string.
#
# Builder methods keep their `()` (`V.string().required()`), as in the docs:
# a validator is a hash of functions, so `.required` without parens reads the
# function instead of calling it (see the pending test at the end).

PASSWORD_PAIR = {
  "password": V.string().required(),
  "confirm_password": V.string().required().confirmation("password")
}

def valid?(data, schema)
  validate(data, schema)["valid"]
end

def errors_of(data, schema)
  validate(data, schema)["errors"]
end

def error_of(data, schema)
  errors_of(data, schema)[0]
end

describe("validate") do
  describe("result shape") do
    test("valid data gives valid, the data, and no errors") do
      result = validate({"name": "John"}, {"name": V.string().required()})
      assert_eq(result, {"valid": true, "data": {"name": "John"}, "errors": []})
    end

    test("invalid data gives the errors and no data") do
      result = validate({}, {"name": V.string().required()})
      assert_eq(result["valid"], false)
      assert_eq(result["data"], {})
      assert_eq(result["errors"], [{"field": "name", "message": "is required", "code": "required"}])
    end

    test("keys missing from the schema are dropped from the data") do
      result = validate({"a": 1, "extra": 2}, {"a": V.int()})
      assert_eq(result["data"], {"a": 1})
    end

    test("every failing field reports its own error") do
      schema = {"email": V.string().required().email(), "age": V.int().required().min(0)}
      errors = errors_of({"email": "bad", "age": -1}, schema)
      assert_eq(errors.length, 2)
      assert_contains(errors.map { |error| error["code"] }, "invalid_email")
      assert_contains(errors.map { |error| error["code"] }, "min")
    end

    test("chained validators all pass on good data") do
      schema = {"email": V.string().required().email(), "age": V.int().required().min(0).max(150)}
      assert(valid?({"email": "test@example.com", "age": 30}, schema))
    end
  end

  describe("types and coercion") do
    test("V.string accepts a string") do
      assert(valid?({"name": "John"}, {"name": V.string().required()}))
    end

    test("V.string coerces a number to a string") do
      data = validate({"name": 42}, {"name": V.string()})["data"]
      assert_eq(data["name"], "42")
    end

    test("V.int accepts an integer and coerces a numeric string") do
      assert_eq(validate({"age": 25}, {"age": V.int().required()})["data"]["age"], 25)
      assert_eq(validate({"age": "25"}, {"age": V.int()})["data"]["age"], 25)
    end

    test("V.int rejects a non-numeric string") do
      error = error_of({"age": "x"}, {"age": V.int().required()})
      assert_eq(error["code"], "type_error")
      assert_eq(error["message"], "cannot convert 'x' to int")
    end

    test("V.float accepts a float and coerces an int or a numeric string") do
      assert_eq(validate({"price": 9.99}, {"price": V.float().required()})["data"]["price"], 9.99)
      assert_eq(type(validate({"price": 9}, {"price": V.float()})["data"]["price"]), "float")
      assert_eq(validate({"price": "9.5"}, {"price": V.float()})["data"]["price"], 9.5)
    end

    test("V.bool accepts booleans and coerces true/false strings") do
      assert_eq(validate({"active": true}, {"active": V.bool().required()})["data"]["active"], true)
      assert_eq(validate({"active": "false"}, {"active": V.bool()})["data"]["active"], false)
    end

    test("V.bool rejects any other string") do
      assert_eq(error_of({"active": "x"}, {"active": V.bool()})["message"], "cannot convert 'x' to bool")
    end

    test("V.array accepts an array and rejects a string") do
      assert_eq(validate({"tags": ["a", "b", "c"]}, {"tags": V.array().required()})["data"]["tags"], ["a", "b", "c"])
      assert_eq(error_of({"tags": "x"}, {"tags": V.array()})["message"], "cannot convert string to array")
    end

    test("V.hash accepts a hash and rejects a string") do
      assert_eq(validate({"meta": {"key": "value"}}, {"meta": V.hash().required()})["data"]["meta"], {"key": "value"})
      assert_eq(error_of({"meta": "x"}, {"meta": V.hash()})["message"], "cannot convert string to hash")
    end
  end

  describe("presence") do
    test("a required field must be present") do
      assert_eq(error_of({}, {"name": V.string().required()})["code"], "required")
    end

    test("a required field may not be nil") do
      assert_eq(error_of({"name": nil}, {"name": V.string().required()})["code"], "required")
    end

    test("an empty string satisfies required") do
      assert(valid?({"name": ""}, {"name": V.string().required()}))
    end

    test("an optional field may be present or missing") do
      schema = {"nickname": V.string().optional()}
      assert(valid?({"nickname": "nick"}, schema))
      assert(valid?({}, schema))
    end

    test("a field is optional by default") do
      assert(valid?({}, {"nickname": V.string()}))
    end

    test("a nullable field accepts nil and a value") do
      schema = {"middle_name": V.string().nullable()}
      assert(valid?({"middle_name": nil}, schema))
      assert(valid?({"middle_name": "Marie"}, schema))
    end

    test("a default fills a missing field and leaves a present one") do
      schema = {"country": V.string().default("US")}
      assert_eq(validate({"country": "FR"}, schema)["data"]["country"], "FR")
      assert_eq(validate({}, schema)["data"]["country"], "US")
    end

    test("a default replaces nil") do
      assert_eq(validate({"country": nil}, {"country": V.string().default("US")})["data"]["country"], "US")
    end

    test("an int default applies to a missing field") do
      result = validate({}, {"attempts": V.int().default(0)})
      assert(result["valid"])
      assert_eq(result["data"]["attempts"], 0)
    end

    test("a default satisfies required") do
      assert_eq(validate({}, {"attempts": V.int().required().default(3)})["data"]["attempts"], 3)
    end
  end

  describe("rules") do
    test("min_length counts characters, bounds included") do
      schema = {"password": V.string().min_length(8)}
      assert(valid?({"password": "longpassword"}, schema))
      assert(valid?({"password": "exactly8"}, schema))
      assert_eq(error_of({"password": "short"}, schema), {
        "field": "password",
        "message": "must be at least 8 characters",
        "code": "min_length"
      })
    end

    test("max_length counts characters, bounds included") do
      schema = {"username": V.string().max_length(10)}
      assert(valid?({"username": "validuser"}, schema))
      assert(valid?({"username": "tencharsxx"}, schema))
      assert_eq(error_of({"username": "averylongusername"}, schema)["message"], "must be at most 10 characters")
    end

    test("min is inclusive") do
      schema = {"age": V.int().min(18)}
      assert(valid?({"age": 18}, schema))
      assert_eq(error_of({"age": 16}, schema), {"field": "age", "message": "must be at least 18", "code": "min"})
    end

    test("max is inclusive") do
      schema = {"quantity": V.int().max(100)}
      assert(valid?({"quantity": 100}, schema))
      expected = {"field": "quantity", "message": "must be at most 100", "code": "max"}
      assert_eq(error_of({"quantity": 101}, schema), expected)
    end

    test("pattern matches a regex") do
      schema = {"zip": V.string().pattern("^\\d{5}$")}
      assert(valid?({"zip": "12345"}, schema))
      assert_eq(error_of({"zip": "abc"}, schema)["code"], "pattern")
    end

    test("email checks the format") do
      schema = {"email": V.string().email()}
      assert(valid?({"email": "test@example.com"}, schema))
      assert_eq(error_of({"email": "nope"}, schema)["message"], "must be a valid email")
    end

    test("url checks the format") do
      schema = {"website": V.string().url()}
      assert(valid?({"website": "https://example.com"}, schema))
      assert_eq(error_of({"website": "not a url"}, schema)["code"], "invalid_url")
    end

    test("one_of restricts a string to the listed values") do
      schema = {"status": V.string().one_of(["active", "inactive", "pending"])}
      assert(valid?({"status": "active"}, schema))
      assert_eq(error_of({"status": "deleted"}, schema)["message"], "must be one of: active, inactive, pending")
    end

    test("one_of restricts an int to the listed values") do
      schema = {"priority": V.int().one_of([1, 2, 3])}
      assert(valid?({"priority": 2}, schema))
      assert_eq(error_of({"priority": 5}, schema)["code"], "one_of")
    end
  end

  describe("confirmation") do
    test("passes when the values match") do
      assert(valid?({"password": "Secret123!", "confirm_password": "Secret123!"}, PASSWORD_PAIR))
    end

    test("fails when the values differ") do
      error = error_of({"password": "Secret123!", "confirm_password": "Different!"}, PASSWORD_PAIR)
      assert_eq(error, {"field": "confirm_password", "message": "does not match", "code": "confirmation"})
    end

    test("fails when the confirmed field is missing") do
      errors = errors_of({"confirm_password": "Secret123!"}, PASSWORD_PAIR)
      assert_eq(errors.map { |error| error["code"] }, ["required", "confirmation"])
    end

    test("works on an email field") do
      schema = {"email": V.string().required().email(), "email_confirm": V.string().required().confirmation("email")}
      assert(valid?({"email": "user@example.com", "email_confirm": "user@example.com"}, schema))
    end

    test("works on an int field") do
      schema = {"user_id": V.int(), "confirm_id": V.int().confirmation("user_id")}
      assert(valid?({"user_id": 7, "confirm_id": 7}, schema))
      assert_not(valid?({"user_id": 7, "confirm_id": 8}, schema))
    end
  end

  describe("nested schemas") do
    test("V.hash(schema) validates the inner fields and reports a dotted path") do
      schema = {"address": V.hash({"city": V.string().required(), "street": V.string().required()}).required()}
      assert_eq(validate({"address": {"city": "Paris", "street": "Rue X"}}, schema)["data"]["address"]["city"], "Paris")
      assert_eq(error_of({"address": {"city": "Paris"}}, schema)["field"], "address.street")
    end

    test("V.array(validator) validates and coerces each element") do
      schema = {"items": V.array(V.hash({"id": V.int().required(), "name": V.string().required()})).required()}
      data = validate({"items": [{"id": "2", "name": "x"}]}, schema)["data"]
      assert_eq(data["items"], [{"id": 2, "name": "x"}])
      assert_eq(error_of({"items": [{"id": 1}]}, schema)["code"], "required")
    end

    test("an element error names the array field") do
      pending("bug: an array element error's field is \"[0].name\", without the array's name \"items\"")
      schema = {"items": V.array(V.hash({"id": V.int().required(), "name": V.string().required()})).required()}
      assert_eq(error_of({"items": [{"id": 1}]}, schema)["field"], "items[0].name")
    end

    test("a type error names its field") do
      pending("bug: a type_error carries field \"\" instead of the field name")
      assert_eq(error_of({"age": "x"}, {"age": V.int()})["field"], "age")
    end
  end

  describe("password rules") do
    test("letters rejects a value without letters") do
      error = error_of({"password": "12345"}, {"password": V.string().letters()})
      assert_eq(error["message"], "must contain at least one letter")
      assert(valid?({"password": "abc123"}, {"password": V.string().letters()}))
    end

    test("mixed_case needs both upper and lower case") do
      assert_eq(error_of({"password": "alllowercase"}, {"password": V.string().mixed_case()})["code"], "mixed_case")
      assert(valid?({"password": "MixedCase1"}, {"password": V.string().mixed_case()}))
    end

    test("numbers needs a digit") do
      assert_eq(error_of({"password": "abcdef"}, {"password": V.string().numbers()})["code"], "numbers")
      assert(valid?({"password": "abc123"}, {"password": V.string().numbers()}))
    end

    test("symbols needs a symbol") do
      assert_eq(error_of({"password": "abc123"}, {"password": V.string().symbols()})["code"], "symbols")
      assert(valid?({"password": "abc123!"}, {"password": V.string().symbols()}))
    end

    test("all the rules together pass on a strong password") do
      schema = {"password": V.string().letters().mixed_case().numbers().symbols()}
      assert(valid?({"password": "Aa1!"}, schema))
    end
  end

  describe("to_password_rules_string") do
    test("lists every password rule in order") do
      rules = V.string().min_length(12).max_length(64).mixed_case().numbers().symbols().to_password_rules_string()
      expected = "minlength: 12; maxlength: 64; required: lower; required: upper; required: digit; required: special;"
      assert_eq(rules, expected)
    end

    test("is empty when no password rule is set") do
      assert_eq(V.string().email().to_password_rules_string(), "")
    end

    test("maps letters to required lower and upper") do
      assert_eq(V.string().letters().to_password_rules_string(), "required: lower; required: upper;")
    end

    test("is available, and empty, on a non-string validator") do
      assert_eq(V.int().min(1).to_password_rules_string(), "")
    end
  end

  describe("builder syntax") do
    test("a zero-argument builder method works without parentheses") do
      pending("bug: V.string().required (no parens) yields the function, so validate reports invalid_schema")
      assert(valid?({"name": "John"}, {"name": V.string.required}))
    end
  end
end
