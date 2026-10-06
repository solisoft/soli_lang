# JSON.parse, JSON.stringify, to_json and assert_json.

describe("JSON.parse") do
  test("parses an object") do
    assert_eq(JSON.parse("{\"name\": \"test\", \"value\": 42}"), {"name": "test", "value": 42})
  end

  test("parses an array") do
    assert_eq(JSON.parse("[1, 2, 3]"), [1, 2, 3])
  end

  test("parses nested structures") do
    parsed = JSON.parse("{\"data\": {\"items\": [1, 2, 3]}}")
    assert_eq(parsed["data"]["items"], [1, 2, 3])
  end

  test("parses booleans and null") do
    parsed = JSON.parse("{\"active\": true, \"deleted\": false, \"value\": null}")
    assert_eq(parsed["active"], true)
    assert_eq(parsed["deleted"], false)
    assert_null(parsed["value"])
    assert(parsed.has_key("value"))
  end

  test("parses a bare scalar") do
    assert_eq(JSON.parse("42"), 42)
    assert_eq(JSON.parse("1.5"), 1.5)
    assert_eq(JSON.parse("\"x\""), "x")
    assert_null(JSON.parse("null"))
  end

  test("decodes \\u escapes") do
    assert_eq(JSON.parse("\"\\u00e9\""), "é")
  end

  test("raises on malformed JSON") do
    assert_raises("Expected string key at position 1") do
      JSON.parse("{nope")
    end
  end

  test("raises on an empty string") do
    assert_raises("Unexpected end of JSON") do
      JSON.parse("")
    end
  end
end

describe("JSON.stringify") do
  test("renders an object compactly, in insertion order") do
    assert_eq(JSON.stringify({"b": 1, "a": 2}), "{\"b\":1,\"a\":2}")
  end

  test("renders an array") do
    assert_eq(JSON.stringify([1, 2, 3]), "[1,2,3]")
  end

  test("renders nested structures and every scalar") do
    data = {"name": "test", "items": [1, nil, true, 1.5]}
    assert_eq(JSON.stringify(data), "{\"name\":\"test\",\"items\":[1,null,true,1.5]}")
  end

  test("keeps a whole Float a Float") do
    assert_eq(JSON.stringify([1.0, 0.1]), "[1.0,0.1]")
  end

  test("escapes quotes and control characters in strings") do
    assert_eq(JSON.stringify("a\"b\n"), "\"a\\\"b\\n\"")
  end

  test("leaves non-ASCII characters as they are") do
    assert_eq(JSON.stringify("é"), "\"é\"")
  end

  test("renders nil as null") do
    assert_eq(JSON.stringify(nil), "null")
  end

  test("round-trips through JSON.parse") do
    data = {"data": {"items": [1, 2, 3]}, "ok": true}
    assert_eq(JSON.parse(JSON.stringify(data)), data)
  end
end

describe("to_json") do
  test("matches JSON.stringify") do
    data = {"a": [1]}
    assert_eq(data.to_json, "{\"a\":[1]}")
    assert_eq(data.to_json, JSON.stringify(data))
  end
end

describe("assert_json") do
  test("passes on valid JSON") do
    assert_json("{\"valid\": true}")
    assert_json("[1, 2, 3]")
  end
end
