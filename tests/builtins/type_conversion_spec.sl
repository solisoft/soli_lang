# The conversion functions str(), int(), float() and type().

describe("str") do
  test("renders scalars") do
    assert_eq(str(42), "42")
    assert_eq(str(3.14), "3.14")
    assert_eq(str(-0.5), "-0.5")
    assert_eq(str(true), "true")
    assert_eq(str(false), "false")
    assert_eq(str("text"), "text")
  end

  # The printed name of nil is "null".
  test("renders nil as null") do
    assert_eq(str(nil), "null")
  end

  test("drops the fraction of a whole Float") do
    assert_eq(str(1.0), "1")
  end

  test("renders collections") do
    assert_eq(str([1, "a"]), "[1, a]")
    assert_eq(str({"a": 1}), "{a => 1}")
  end
end

describe("int") do
  test("parses an integer string") do
    assert_eq(int("42"), 42)
    assert_eq(int("-7"), -7)
  end

  test("truncates a Float toward zero") do
    assert_eq(int(3.14), 3)
    assert_eq(int(3.99), 3)
    assert_eq(int(-3.14), -3)
    assert_eq(int(-3.99), -3)
  end

  test("maps true to 1") do
    assert_eq(int(true), 1)
  end

  test("raises on a string that is not an integer") do
    assert_raises("cannot convert 'abc' to int") do
      int("abc")
    end
    assert_raises("cannot convert '42.7' to int") do
      int("42.7")
    end
    assert_raises("cannot convert ' 42 ' to int") do
      int(" 42 ")
    end
  end

  test("raises on nil") do
    assert_raises("cannot convert null to int") do
      int(nil)
    end
  end
end

describe("float") do
  test("parses a numeric string") do
    assert_eq(float("3.14"), 3.14)
    assert_eq(float("-1"), -1.0)
  end

  test("widens an Int") do
    assert_eq(float(42), 42.0)
    assert_eq(type(float(42)), "float")
  end

  test("raises on a string that is not a number") do
    assert_raises("cannot convert 'x' to float") do
      float("x")
    end
  end

  test("raises on nil and booleans") do
    assert_raises("cannot convert null to float") do
      float(nil)
    end
    assert_raises("cannot convert bool to float") do
      float(true)
    end
  end
end

describe("type") do
  test("names the primitive types") do
    assert_eq(type(42), "int")
    assert_eq(type(3.14), "float")
    assert_eq(type("hello"), "string")
    assert_eq(type(true), "bool")
    assert_eq(type(nil), "null")
    assert_eq(type(:name), "symbol")
  end

  test("names the collections; a range is an array") do
    assert_eq(type([1, 2, 3]), "array")
    assert_eq(type(hash()), "hash")
    assert_eq(type({}), "hash")
    assert_eq(type(1..3), "array")
  end

  test("names functions and the time classes") do
    assert_eq(type(fn() { 1 }), "Function")
    assert_eq(type(DateTime.epoch), "DateTime")
    assert_eq(type(Duration.seconds(1)), "Duration")
  end
end
