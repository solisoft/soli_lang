# The nullish coalescing operator `??`: the right side only when the left is
# nil — false, 0, "" and empty collections are kept. `??=` is covered in
# compound_operators_spec.sl.

def greet(name)
  "Hello, " + (name ?? "Guest") + "!"
end

def value_if(flag)
  return "found" if flag

  nil
end

describe("Nullish coalescing ??") do
  context("choosing a side") do
    test("returns the right operand when the left is nil") do
      assert_eq(nil ?? "default", "default")
      assert_eq(nil ?? 42, 42)
    end

    test("returns the left operand when it is not nil") do
      assert_eq("value" ?? "default", "value")
    end

    test("keeps falsy values that are not nil") do
      assert_eq(0 ?? 100, 0)
      assert_eq(false ?? true, false)
      assert_eq("" ?? "default", "")
      assert_eq([] ?? [1, 2, 3], [])
      assert_eq({} ?? {"key": "value"}, {})
    end

    test("returns collections from the right") do
      assert_eq(nil ?? [1, 2, 3], [1, 2, 3])
      assert_eq(nil ?? {"key": "value"}, {"key": "value"})
    end

    test("gives nil when both sides are nil") do
      assert_null(nil ?? nil)
    end

    test("works on variables") do
      missing = nil
      found = "found"
      present = "value"
      assert_eq(missing ?? found, "found")
      assert_eq(present ?? found, "value")
    end

    test("a missing hash key reads nil, so ?? supplies the default") do
      settings = {"db": nil}
      assert_eq(settings["db"] ?? "sqlite", "sqlite")
      assert_eq(settings["absent"] ?? "none", "none")
    end
  end

  context("evaluation") do
    test("does not evaluate the right side when the left is not nil") do
      calls = []
      record = fn(value) {
        calls.push(value)
        value
      }
      assert_eq("left" ?? record("right"), "left")
      assert_eq(calls, [])
      assert_eq(nil ?? record("right"), "right")
      assert_eq(calls, ["right"])
    end
  end

  context("chaining") do
    test("returns the first non-nil value") do
      assert_eq(nil ?? nil ?? "final", "final")
      assert_eq(nil ?? nil ?? "third" ?? "fourth", "third")
    end

    test("stops at the first false") do
      assert_eq(nil ?? false ?? "later", false)
    end
  end

  context("precedence") do
    test("binds looser than arithmetic") do
      assert_eq(nil ?? 5 + 3, 8)
      assert_eq(1 ?? 5 + 3, 1)
      assert_eq(nil ?? 2 * 3, 6)
    end

    test("binds looser than ==") do
      missing = nil
      other = nil
      # (missing == other) ?? 1 is true; missing == (other ?? 1) would be false
      assert_eq(missing == other ?? 1, true)
    end

    test("binds looser than || and &&") do
      # false ?? (nil || "x"): false is not nil, so it is kept
      assert_eq(false ?? nil || "x", false)
      # (false || nil) ?? "y"
      assert_eq(false || nil ?? "y", "y")
      # (true && nil) ?? "z"
      assert_eq(true && nil ?? "z", "z")
    end

    test("a parenthesized ?? inside && and ||") do
      assert_eq(true && (nil ?? "fallback"), "fallback")
      assert_eq(false && (nil ?? "fallback"), false)
      assert_eq(false || (nil ?? "fallback"), "fallback")
      assert_eq(true || (nil ?? "fallback"), true)
      assert_eq(nil ?? (true && false), false)
    end
  end

  context("in expressions") do
    test("as a function argument default") do
      assert_eq(greet(nil), "Hello, Guest!")
      assert_eq(greet("Alice"), "Hello, Alice!")
    end

    test("on a function that may return nil") do
      assert_eq(value_if(false) ?? "default", "default")
      assert_eq(value_if(true) ?? "default", "found")
    end

    test("the result takes method calls") do
      assert_eq((nil ?? "default").upcase, "DEFAULT")
      assert_eq((nil ?? [1, 2, 3]).length, 3)
      assert_eq((["a", "b"] ?? [1, 2]).first, "a")
    end

    test("nested: a default hash, then a default field") do
      config = nil
      assert_eq((config ?? {"db": nil}).db ?? "sqlite", "sqlite")
      other_config = {"db": "postgresql"}
      assert_eq((other_config ?? {"db": nil}).db ?? "sqlite", "postgresql")
    end

    test("inside array literals") do
      assert_eq([1, nil ?? 2, 3], [1, 2, 3])
      assert_eq([1, "a" ?? 2, 3], [1, "a", 3])
    end

    test("inside hash literals") do
      settings = {"a": nil ?? 1, "b": "value" ?? 2}
      assert_eq(settings, {"a": 1, "b": "value"})
    end

    test("negating the result with not and !") do
      assert_eq(not (nil ?? "default"), false)
      assert_eq(not ("value" ?? "default"), false)
      assert_eq(!(nil ?? nil), true)
    end
  end

  context("safe navigation &.") do
    test("returns nil on a nil receiver instead of raising") do
      user = nil
      assert_null(user&.name)
      assert_eq(user&.name ?? "anonymous", "anonymous")
    end
  end
end
