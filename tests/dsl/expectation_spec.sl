# expect(actual).<matcher>(expected): every matcher, on its passing path and
# on its failing path, with the message the failure carries.

class Point
  x: Int

  new(x: Int)
    @x = x
  end
end

# The message an expectation fails with, without its " at line:col" suffix,
# or nil when it passes. assert_raises lets a failed assertion through on
# purpose (a failing spec inside its block must still fail the test), so the
# failure path of a matcher is read with try/catch.
def failure_of(check)
  try
    check()
  catch error
    return Regex.replace(" at \\d+:\\d+$", "#{error}", "")
  end

  nil
end

describe("expect()") do
  test("holds the value under test in .actual") do
    assert_eq(expect(42).actual, 42)
    assert_eq(expect("hello").actual, "hello")
    assert_null(expect(nil).actual)
  end

  test("builds an Expectation") do
    assert_eq(type(expect(42)), "Expectation")
  end

  test("a passing matcher returns true and counts as an assertion") do
    assert_eq(expect(1).to_be(1), true)
  end

  test("a matcher takes no message argument") do
    assert_eq(failure_of(fn() { expect(1).to_be(1, "why") }), "Wrong number of arguments: expected 1, got 2")
  end
end

describe("to_be and to_equal") do
  test("pass on equal values of any type") do
    expect(42).to_be(42)
    expect("hello").to_be("hello")
    expect(false).to_be(false)
    expect([1, 2]).to_equal([1, 2])
    expect({"a": 1}).to_equal({"a": 1})
  end

  test("compare by value, not identity") do
    expect([1, [2]]).to_be([1, [2]])
    expect(1).to_be(1.0)
    expect(1.0).to_equal(1)
  end

  test("fail with both values in the message") do
    assert_eq(failure_of(fn() { expect(42).to_be(100) }), "Expected 42 to be 100")
    assert_eq(failure_of(fn() { expect(42).to_equal(100) }), "Expected 42 to equal 100")
    assert_eq(failure_of(fn() { expect("a").to_equal("b") }), "Expected \"a\" to equal \"b\"")
  end

  test("show an instance by its fields") do
    assert_eq(failure_of(fn() { expect(new Point(1)).to_equal(2) }), "Expected <Point x: 1> to equal 2")
  end

  test("truncate a long string in the message") do
    message = failure_of(fn() { expect("x" * 100).to_equal("y") })
    assert_eq(message, "Expected \"#{"x" * 80}\"… (100 chars) to equal \"y\"")
  end
end

describe("to_not_be and to_not_equal") do
  test("pass on different values") do
    expect(42).to_not_be(100)
    expect(42).to_not_equal(100)
    expect(nil).to_not_be(false)
  end

  test("fail on equal values") do
    assert_eq(failure_of(fn() { expect(42).to_not_be(42) }), "Expected 42 to not be 42")
    assert_eq(failure_of(fn() { expect(42).to_not_equal(42) }), "Expected 42 to not equal 42")
  end
end

describe("to_be_null and to_not_be_null") do
  test("to_be_null passes for nil only") do
    expect(nil).to_be_null
    assert_eq(failure_of(fn() { expect(42).to_be_null }), "Expected 42 to be null")
    assert_eq(failure_of(fn() { expect(false).to_be_null }), "Expected false to be null")
  end

  test("to_not_be_null passes for anything else, falsy values included") do
    expect(42).to_not_be_null
    expect(false).to_not_be_null
    expect("").to_not_be_null
    assert_eq(failure_of(fn() { expect(nil).to_not_be_null }), "Expected value to not be null")
  end
end

describe("numeric comparisons") do
  test("to_be_greater_than is strict") do
    expect(100).to_be_greater_than(50)
    assert_eq(failure_of(fn() { expect(50).to_be_greater_than(100) }), "Expected 50 to be greater than 100")
    assert_eq(failure_of(fn() { expect(50).to_be_greater_than(50) }), "Expected 50 to be greater than 50")
  end

  test("to_be_less_than is strict") do
    expect(50).to_be_less_than(100)
    assert_eq(failure_of(fn() { expect(100).to_be_less_than(50) }), "Expected 100 to be less than 50")
    assert_eq(failure_of(fn() { expect(50).to_be_less_than(50) }), "Expected 50 to be less than 50")
  end

  test("to_be_greater_than_or_equal accepts equality") do
    expect(100).to_be_greater_than_or_equal(50)
    expect(50).to_be_greater_than_or_equal(50)
    assert_eq(failure_of(fn() { expect(50).to_be_greater_than_or_equal(100) }), "Expected 50 to be >= 100")
  end

  test("to_be_less_than_or_equal accepts equality") do
    expect(50).to_be_less_than_or_equal(100)
    expect(50).to_be_less_than_or_equal(50)
    assert_eq(failure_of(fn() { expect(100).to_be_less_than_or_equal(50) }), "Expected 100 to be <= 50")
  end

  test("mix Int and Float") do
    expect(10.5).to_be_greater_than(10)
    expect(10).to_be_greater_than(9.5)
    expect(9.5).to_be_less_than(10)
    expect(10).to_be_greater_than_or_equal(10.0)
    expect(10.0).to_be_less_than_or_equal(10)
    assert_eq(failure_of(fn() { expect(10).to_be_less_than(9.5) }), "Expected 10 to be less than 9.5")
  end

  test("refuse anything but numbers") do
    assert_raises("to_be_greater_than expects two numbers, got string and string") do
      expect("b").to_be_greater_than("a")
    end
    assert_raises("to_be_less_than expects two numbers, got int and null") do
      expect(1).to_be_less_than(nil)
    end
  end
end

describe("to_have_key") do
  test("passes when the hash has the key") do
    expect({"name": "Ada", "age": nil}).to_have_key("name")
    expect({"name": "Ada", "age": nil}).to_have_key("age")
  end

  test("fails when the key is missing") do
    assert_eq(failure_of(fn() { expect({"a": 1}).to_have_key("b") }), "Expected {a => 1} to have key \"b\"")
    assert_eq(failure_of(fn() { expect({}).to_have_key("a") }), "Expected {} to have key \"a\"")
  end

  test("refuses anything but a hash") do
    assert_raises("to_have_key expects a Hash, got array") do
      expect(["a"]).to_have_key("a")
    end
  end
end

describe("to_contain") do
  test("finds an element in an array") do
    expect([1, 2]).to_contain(2)
    assert_eq(failure_of(fn() { expect([1, 2]).to_contain(5) }), "Expected [1, 2] to contain 5")
  end

  test("compares array elements by value") do
    expect([[1], [2]]).to_contain([2])
    assert_eq(failure_of(fn() { expect([1]).to_contain([1]) }), "Expected [1] to contain [1]")
  end

  test("finds a substring in a string") do
    expect("hello world").to_contain("world")
    expect("hello").to_contain("")
    assert_eq(failure_of(fn() { expect("hello").to_contain("bye") }), "Expected \"hello\" to contain \"bye\"")
  end

  test("looks at a hash's values, not its keys") do
    expect({"a": 1}).to_contain(1)
    assert_eq(failure_of(fn() { expect({"a": 1}).to_contain("a") }), "Expected {a => 1} to contain \"a\"")
  end

  test("refuses a non-string needle in a string and other actuals") do
    assert_raises("to_contain expects string argument") do
      expect("123").to_contain(1)
    end
    assert_raises("to_contain expects string, array, or hash") do
      expect(123).to_contain(1)
    end
  end
end

describe("to_match") do
  test("matches a substring, not a regular expression") do
    expect("hello world").to_match("o w")
    assert_eq(failure_of(fn() { expect("hello").to_match("^h.*o$") }), "Expected \"hello\" to match \"^h.*o$\"")
  end

  test("fails when the substring is absent") do
    assert_eq(failure_of(fn() { expect("abc").to_match("z") }), "Expected \"abc\" to match \"z\"")
  end

  test("refuses anything but strings") do
    assert_raises("to_match expects string actual and string pattern") do
      expect(1).to_match("1")
    end
  end
end

describe("to_be_valid_json") do
  test("passes for any JSON document") do
    expect("{\"name\": \"test\"}").to_be_valid_json
    expect("[1, 2, 3]").to_be_valid_json
    expect("\"string\"").to_be_valid_json
    expect("null").to_be_valid_json
  end

  test("fails for text that does not parse") do
    assert_eq(failure_of(fn() { expect("not json").to_be_valid_json }), "Expected valid JSON, got: not json")
    assert_eq(failure_of(fn() { expect("").to_be_valid_json }), "Expected valid JSON, got: ")
  end

  test("refuses anything but a string") do
    assert_raises("to_be_valid_json expects string") do
      expect(123).to_be_valid_json
    end
  end
end
