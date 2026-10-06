# Hashes as a language feature: literals and key types, bracket and dot
# access, safe navigation, and the built-in Hash methods.

def make_response
  {"status": 200, "body": "ok"}
end

def empty_hash
  {}
end

describe("Hash literals and keys") do
  test("string keys read with brackets") do
    person = {"name": "Alice", "age": 30}
    assert_eq(person["name"], "Alice")
    assert_eq(person["age"], 30)
  end

  test("bracket assignment adds a key to hash()") do
    record = hash()
    assert_eq(record, {})
    record["key"] = "value"
    assert_eq(record, {"key": "value"})
  end

  test("integer keys") do
    numbers = {1: "one", 2: "two"}
    assert_eq(numbers[1], "one")
    assert_eq(numbers[2], "two")
  end

  test("an integer key and its string form are different keys") do
    mixed = {1: "int", "1": "string"}
    assert_eq(mixed.length, 2)
    assert_eq(mixed[1], "int")
    assert_eq(mixed["1"], "string")
  end

  test("boolean keys") do
    answers = {true: "yes", false: "no"}
    assert_eq(answers[true], "yes")
    assert_eq(answers[false], "no")
  end

  test("keys with spaces") do
    names = {"first name": "John", "last name": "Doe"}
    assert_eq(names["first name"], "John")
    assert_eq(names["last name"], "Doe")
  end

  test("keys keep insertion order") do
    assert_eq({"b": 1, "a": 2}.keys, ["b", "a"])
  end

  test("a missing key reads as nil") do
    assert_null({"a": 1}["zz"])
  end

  test("a key set to nil is kept") do
    record = {"a": 1}
    record["a"] = nil
    assert_eq(record, {"a": nil})
    assert(record.has_key("a"))
  end

  test("equality ignores insertion order") do
    assert_eq({"a": 1, "b": 2}, {"b": 2, "a": 1})
    assert_ne({"a": 1}, {"a": 2})
  end

  test("a hash literal as the last expression is the return value") do
    assert_eq(make_response(), {"status": 200, "body": "ok"})
    assert_eq(empty_hash(), {})
  end
end

describe("Nested hashes") do
  test("chained bracket access") do
    data = {"person": {"name": "Alice", "address": {"city": "NYC"}}}
    assert_eq(data["person"]["name"], "Alice")
    assert_eq(data["person"]["address"]["city"], "NYC")
  end

  test("controller-style nested params") do
    request = {
      "params": {"id": "42", "slug": "hello"},
      "query": {},
      "json": {"email": "a@b.c"}
    }
    assert_eq(request["params"]["id"], "42")
    assert_eq(request["params"]["slug"], "hello")
    assert_eq(request["json"]["email"], "a@b.c")
    assert_null(request["query"]["q"])
    assert_null(request["params"]["missing"])
  end
end

describe("Dot notation") do
  test("reads a string key") do
    person = {"name": "Alice", "age": 30}
    assert_eq(person.name, "Alice")
    assert_eq(person.age, 30)
  end

  test("reads nested keys") do
    user = {"profile": {"email": "alice@example.com"}}
    assert_eq(user.profile.email, "alice@example.com")
  end

  test("a missing key reads as nil") do
    assert_null({"name": "x"}.missing)
  end

  test("assigns an existing key") do
    person = {"name": "Alice"}
    person.name = "Bob"
    assert_eq(person, {"name": "Bob"})
  end

  test("adds a new key") do
    person = {"name": "Alice"}
    person.age = 30
    assert_eq(person, {"name": "Alice", "age": 30})
  end

  test("chains into a method on the value") do
    data = {"items": [1, 2, 3]}
    assert_eq(data.items.length, 3)
  end
end

describe("Safe navigation") do
  test("stops at a nil value") do
    user = {"profile": nil}
    assert_null(user&.profile&.email)
  end

  test("returns nil for a missing key") do
    user = {"name": "Alice"}
    assert_null(user&.email)
  end
end

describe("Hash methods") do
  context("size and membership") do
    test("length and len()") do
      assert_eq({"a": 1, "b": 2}.length, 2)
      assert_eq({"a": 1, "b": 2, "c": 3}.len(), 3)
      assert_eq({}.length, 0)
    end

    test("keys and values") do
      assert_eq({"a": 1, "b": 2}.keys, ["a", "b"])
      assert_eq({"a": 1, "b": 2}.values, [1, 2])
    end

    test("has_key") do
      letters = {"a": 1, "b": 2}
      assert_eq(letters.has_key("a"), true)
      assert_eq(letters.has_key("c"), false)
    end

    test("has_value? and its value? alias") do
      letters = {"a": 1, "b": 2}
      assert_eq(letters.has_value?(2), true)
      assert_eq(letters.has_value?(99), false)
      assert_eq(letters.value?(1), true)
      assert_eq(letters.value?(99), false)
    end
  end

  context("lookups") do
    test("key finds the first key holding a value") do
      assert_eq({"a": 1, "b": 2, "c": 3}.key(2), "b")
      assert_eq({"a": 1, "b": 1}.key(1), "a")
      assert_null({"a": 1}.key(99))
    end

    test("values_at returns nil for a missing key") do
      assert_eq({"a": 1, "b": 2, "c": 3}.values_at("a", "c"), [1, 3])
      assert_eq({"a": 1}.values_at("a", "missing"), [1, nil])
    end

    test("fetch_values returns the values in order") do
      assert_eq({"a": 1, "b": 2}.fetch_values("b", "a"), [2, 1])
    end

    test("fetch_values raises on a missing key") do
      assert_raises("key not found") do
        {"a": 1}.fetch_values("a", "zz")
      end
    end

    test("fetch returns the value, the default, or raises") do
      assert_eq({"a": 1}.fetch("a"), 1)
      assert_eq({"a": 1}.fetch("zz", 0), 0)
      assert_raises("key not found: zz") do
        {"a": 1}.fetch("zz")
      end
    end

    test("assoc finds a pair by key") do
      assert_eq({"a": 1, "b": 2}.assoc("b"), ["b", 2])
      assert_null({"a": 1}.assoc("missing"))
    end

    test("rassoc finds a pair by value") do
      assert_eq({"a": 1, "b": 2}.rassoc(2), ["b", 2])
      assert_null({"a": 1}.rassoc(99))
    end
  end

  context("mutation") do
    test("delete removes a key and returns its value") do
      letters = {"a": 1, "b": 2}
      assert_eq(letters.delete("a"), 1)
      assert_eq(letters, {"b": 2})
    end

    test("delete of a missing key returns nil and changes nothing") do
      letters = {"a": 1}
      assert_null(letters.delete("zz"))
      assert_eq(letters, {"a": 1})
    end

    test("clear empties the hash") do
      letters = {"a": 1, "b": 2}
      letters.clear
      assert_eq(letters, {})
    end

    test("shift removes and returns the first pair") do
      letters = {"a": 1, "b": 2}
      assert_eq(letters.shift, ["a", 1])
      assert_eq(letters, {"b": 2})
    end

    test("shift on an empty hash returns nil") do
      assert_null({}.shift)
    end
  end

  context("non-mutating transforms") do
    test("merge lets the right side win and leaves the receiver alone") do
      left = {"a": 1, "b": 2}
      merged = left.merge({"b": 3, "c": 4})
      assert_eq(merged, {"a": 1, "b": 3, "c": 4})
      assert_eq(left, {"a": 1, "b": 2})
    end

    test("update is an alias for merge, also non-mutating") do
      original = {"a": 1}
      updated = original.update({"b": 2})
      assert_eq(updated, {"a": 1, "b": 2})
      assert_eq(original, {"a": 1})
    end

    test("to_h returns an equal hash") do
      original = {"a": 1, "b": 2}
      assert_eq(original.to_h, {"a": 1, "b": 2})
    end

    test("flatten returns the [key, value] pairs") do
      assert_eq({"a": 1, "b": 2}.flatten, [["a", 1], ["b", 2]])
    end

    test("keep_if returns the matching entries") do
      numbers = {"a": 1, "b": 2, "c": 3}
      assert_eq(numbers.keep_if { |key, value| value >= 2 }, {"b": 2, "c": 3})
      assert_eq(numbers, {"a": 1, "b": 2, "c": 3})
    end

    test("delete_if returns the entries that do not match") do
      numbers = {"a": 1, "b": 2, "c": 3}
      assert_eq(numbers.delete_if { |key, value| value >= 2 }, {"a": 1})
      assert_eq(numbers, {"a": 1, "b": 2, "c": 3})
    end
  end

  context("iteration and predicates") do
    test("each_key visits the keys in order") do
      keys = []
      {"a": 1, "b": 2, "c": 3}.each_key { |key| keys.push(key) }
      assert_eq(keys, ["a", "b", "c"])
    end

    test("each_value visits the values in order") do
      values = []
      {"a": 1, "b": 2, "c": 3}.each_value { |value| values.push(value) }
      assert_eq(values, [1, 2, 3])
    end

    test("all? is true only when every entry matches") do
      assert_eq({"a": 2, "b": 4}.all? { |key, value| value % 2 == 0 }, true)
      assert_eq({"a": 2, "b": 3}.all? { |key, value| value % 2 == 0 }, false)
    end

    test("any? is true when one entry matches") do
      assert_eq({"a": 1, "b": 2}.any? { |key, value| value == 2 }, true)
      assert_eq({"a": 1, "b": 3}.any? { |key, value| value == 2 }, false)
    end

    test("all? on an empty hash is true, any? is false") do
      assert_eq({}.all? { |key, value| false }, true)
      assert_eq({}.any? { |key, value| true }, false)
    end
  end
end
