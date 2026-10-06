# Core global functions: clock, debug, len, print/println.

describe("clock") do
  test("returns a Float") do
    assert_eq(type(clock()), "float")
  end

  test("returns a Unix timestamp in seconds") do
    # Later than 2023-11-14 and earlier than 2100-01-01.
    now = clock()
    assert_gt(now, 1700000000)
    assert_lt(now, 4102444800)
  end

  test("never goes backwards between two calls") do
    first = clock()
    second = clock()
    assert(second >= first)
  end
end

describe("debug") do
  test("returns a Breakpoint value") do
    # Used as a value (not a statement), debug() hands back the breakpoint
    # marker instead of pausing.
    assert_eq(type(debug()), "Breakpoint")
  end
end

describe("len") do
  test("counts the characters of a string") do
    assert_eq(len("hello"), 5)
    assert_eq(len(""), 0)
  end

  test("counts the elements of an array") do
    assert_eq(len([1, 2, 3]), 3)
    assert_eq(len([]), 0)
  end

  test("counts the keys of a hash") do
    assert_eq(len({"a": 1, "b": 2}), 2)
    assert_eq(len({}), 0)
  end

  test("rejects a value that has no length") do
    assert_raises("len() expects array, string, or hash, got int") do
      len(42)
    end
  end
end

describe("print and println") do
  test("print takes several arguments and returns nil") do
    assert_null(print("hello", "world"))
  end

  test("println takes several arguments and returns nil") do
    assert_null(println("hello", "world"))
  end
end
