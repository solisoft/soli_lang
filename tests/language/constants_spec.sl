# const: immutable bindings (not immutable values), and const_get, which
# resolves a constant, class, function or variable by name.

const CONSTANTS_SPEC_LIMIT = 10

class ConstGetUser
  def greet
    "hi"
  end
end

def const_greet(name)
  "hello " + name
end

def limit_plus_one
  CONSTANTS_SPEC_LIMIT + 1
end

def radius_area
  const PI = 3.14159
  PI * 10
end

describe("Constants") do
  context("declaring") do
    test("holds a float") do
      const PI = 3.14159
      assert_eq(PI, 3.14159)
    end

    test("holds an integer") do
      const MAX_SIZE = 1000
      assert_eq(MAX_SIZE, 1000)
    end

    test("holds a string") do
      const GREETING = "Hello, World!"
      assert_eq(GREETING, "Hello, World!")
    end

    test("holds booleans") do
      const ENABLED = true
      const DISABLED = false
      assert_eq(ENABLED, true)
      assert_eq(DISABLED, false)
    end

    test("holds an array") do
      const COLORS = ["red", "green", "blue"]
      assert_eq(COLORS, ["red", "green", "blue"])
    end

    test("holds a hash") do
      const CONFIG = {"host": "localhost", "port": 3000}
      assert_eq(CONFIG["host"], "localhost")
      assert_eq(CONFIG["port"], 3000)
    end

    test("takes a type annotation") do
      const PI: Float = 3.14159
      assert_eq(PI, 3.14159)
      assert_eq(type(PI), "float")
    end

    test("several in one scope are independent") do
      const A = 1
      const B = 2
      const C = 3
      assert_eq([A, B, C], [1, 2, 3])
    end

    test("evaluates its initializer once") do
      const SUM = 10 + 20
      assert_eq(SUM, 30)
    end

    test("can be built from another constant") do
      const VALUE = 100
      const DOUBLED = VALUE * 2
      assert_eq(DOUBLED, 200)
    end
  end

  context("visibility") do
    test("a top-level constant is readable from a function") do
      assert_eq(limit_plus_one(), 11)
    end

    test("a constant declared in a function is local to it") do
      assert_eq(radius_area(), 31.4159)
    end

    test("a const of the same name replaces a let binding") do
      let value = 10
      const value = 20
      assert_eq(value, 20)
    end

    test("a const in a nested block shadows the outer one") do
      const LEVEL = 1
      inner = 0
      if true
        const LEVEL = 2
        inner = LEVEL
      end
      assert_eq(inner, 2)
      assert_eq(LEVEL, 1)
    end
  end

  context("reassigning") do
    test("assignment raises and leaves the value") do
      const VALUE = 42
      assert_raises("cannot reassign constant 'VALUE'") do
        VALUE = 100
      end
      assert_eq(VALUE, 42)
    end

    test("a compound assignment raises too") do
      const VALUE = 42
      assert_raises("cannot reassign constant 'VALUE'") do
        VALUE += 1
      end
      assert_eq(VALUE, 42)
    end

    test("array elements stay mutable: const protects the binding only") do
      const NUMBERS = [1, 2, 3]
      NUMBERS[0] = 100
      NUMBERS.push(4)
      assert_eq(NUMBERS, [100, 2, 3, 4])
    end

    test("hash values stay mutable") do
      const SETTINGS = {"key": "value"}
      SETTINGS["key"] = "new"
      SETTINGS["other"] = 1
      assert_eq(SETTINGS, {"key": "new", "other": 1})
    end
  end
end

describe("const_get") do
  test("resolves a const by name") do
    const MY_VALUE = 42
    assert_eq(const_get("MY_VALUE"), 42)
  end

  test("resolves a top-level const by name") do
    assert_eq(const_get("CONSTANTS_SPEC_LIMIT"), 10)
  end

  test("resolves a class by name and instantiates it") do
    user_class = const_get("ConstGetUser")
    assert_eq(type(user_class), "Class")
    instance = user_class.new()
    assert_eq(type(instance), "ConstGetUser")
    assert_eq(instance.greet, "hi")
  end

  test("resolves a function by name and calls it") do
    greet = const_get("const_greet")
    assert_eq(greet("world"), "hello world")
  end

  test("resolves a variable by name") do
    value = 99
    assert_eq(value, 99)
    assert_eq(const_get("value"), 99)
  end

  test("returns nil for an undefined name") do
    assert_null(const_get("DefinitelyNotDefined"))
  end

  test("raises when passed a non-string argument") do
    assert_raises("const_get() expects a string, got int") do
      const_get(123)
    end
  end

  test("raises without an argument") do
    assert_raises("expected 1, got 0") do
      const_get()
    end
  end
end
