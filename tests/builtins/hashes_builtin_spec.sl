# hash(), Hash.from_entries, key types, indexing and Hash#dig.

describe("creating hashes") do
  test("hash() and {} are empty") do
    assert_eq(hash(), {})
    assert_eq(len(hash()), 0)
    assert_eq(len({}), 0)
  end

  test("a literal with entries") do
    assert_eq(len({"a": 1, "b": 2}), 2)
  end

  # `soli fmt` rewrites `=>` to `:`; keep the arrow, it is what this tests.
  test("a fat-arrow literal with string, Int and Bool keys") do
    mixed = {"string" => "value1", 123 => "value2", true => "value3"}
    assert_eq(mixed.keys, ["string", 123, true])
    assert_eq(mixed[123], "value2")
  end
end

describe("indexing") do
  test("reads a key") do
    person = {"name": "Alice", "age": 30}
    assert_eq(person["name"], "Alice")
    assert_eq(person["age"], 30)
  end

  test("assignment adds a key") do
    values = hash()
    values["a"] = 1
    values["b"] = 2
    assert_eq(values, {"a": 1, "b": 2})
  end

  test("assignment replaces a value") do
    values = {"a": 1}
    values["a"] = 2
    assert_eq(values, {"a": 2})
  end

  test("a missing key reads as nil") do
    assert_null(hash()["missing"])
  end

  test("a nil value is a present key") do
    values = {"a": nil, "b": 2}
    assert_null(values["a"])
    assert(values.has_key("a"))
    assert_eq(values["b"], 2)
  end

  test("nested hashes") do
    data = {"person": {"name": "Alice", "address": {"city": "NYC"}}}
    assert_eq(data["person"]["name"], "Alice")
    assert_eq(data["person"]["address"]["city"], "NYC")
  end

  test("keys with spaces") do
    names = {"first name": "John", "last name": "Doe"}
    assert_eq(names["first name"], "John")
    assert_eq(names["last name"], "Doe")
  end

  test("Bool keys") do
    answers = {true: "yes", false: "no"}
    assert_eq(answers[true], "yes")
    assert_eq(answers[false], "no")
  end

  test("Int keys are not their string form") do
    numbers = {1: "one", 2: "two"}
    assert_eq(numbers[1], "one")
    assert_eq(numbers[2], "two")
    assert_null(numbers["1"])
  end
end

describe("Hash.from_entries") do
  test("builds a hash from [key, value] pairs") do
    assert_eq(Hash.from_entries([["a", 1], ["b", 2]]), {"a": 1, "b": 2})
  end

  test("round-trips with entries") do
    original = {"x": 10, "y": 20, "z": 30}
    assert_eq(Hash.from_entries(original.entries), original)
  end

  test("an empty array gives an empty hash") do
    assert_eq(Hash.from_entries([]), {})
  end

  test("raises on an entry that is not a pair") do
    assert_raises("Hash.from_entries(): entry at index 0 must have exactly 2 elements, got 1") do
      Hash.from_entries([["a"]])
    end
  end
end

describe("Hash#dig") do
  test("follows nested keys") do
    assert_eq({"user": {"profile": {"name": "Alice"}}}.dig("user", "profile", "name"), "Alice")
  end

  test("a single key is a plain read") do
    assert_eq({"name": "Alice"}.dig("name"), "Alice")
  end

  test("is nil for a missing key, at any depth") do
    assert_null({"user": {"name": "Alice"}}.dig("user", "age"))
    assert_null({"user": {"profile": {"name": "Bob"}}}.dig("user", "settings", "theme"))
    assert_null({}.dig("missing"))
  end

  test("indexes into arrays") do
    data = {"items": [10, 20, 30]}
    assert_eq(data.dig("items", 0), 10)
    assert_eq(data.dig("items", 2), 30)
    assert_eq(data.dig("items", -1), 30)
  end

  test("mixes hashes and arrays") do
    data = {"users": [{"name": "Alice"}, {"name": "Bob"}]}
    assert_eq(data.dig("users", 0, "name"), "Alice")
    assert_eq(data.dig("users", 1, "name"), "Bob")
  end

  test("is nil past the end of an array") do
    assert_null({"items": [10, 20]}.dig("items", 5))
  end

  test("is nil when a step reaches a scalar") do
    assert_null({"a": 1}.dig("a", "b"))
  end
end
