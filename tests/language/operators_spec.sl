# Operators: arithmetic, comparison, logical, ternary and the shovel, plus the
# falsy set they all test against. Compound assignment and ++/-- live in
# compound_operators_spec.sl, ?? in nullish_coalescing_spec.sl.

def sign_of(x)
  x < 0 ? "negative" : x == 0 ? "zero" : "positive"
end

describe("Arithmetic operators") do
  test("addition") do
    assert_eq(2 + 3, 5)
    assert_eq(-1 + 1, 0)
    assert_eq(1.5 + 2.5, 4.0)
  end

  test("subtraction") do
    assert_eq(5 - 3, 2)
    assert_eq(0 - 5, -5)
    assert_eq(3.5 - 1.5, 2.0)
  end

  test("multiplication") do
    assert_eq(3 * 4, 12)
    assert_eq(-2 * 3, -6)
    assert_eq(2.5 * 2, 5.0)
  end

  test("Int division truncates toward zero") do
    assert_eq(10 / 2, 5)
    assert_eq(7 / 2, 3)
    assert_eq(-7 / 2, -3)
    assert_eq(7.0 / 2.0, 3.5)
  end

  test("modulo takes the sign of the dividend") do
    assert_eq(10 % 3, 1)
    assert_eq(15 % 5, 0)
    assert_eq(-7 % 3, -1)
    assert_eq(7 % -3, 1)
    assert_eq(7.5 % 2, 1.5)
  end

  test("mixing Int and Float gives a Float") do
    assert_eq(1 + 2.5, 3.5)
    assert_eq(type(1 + 2.5), "float")
  end

  test("unary negation") do
    x = 5
    assert_eq(-x, -5)
    assert_eq(-(-x), 5)
  end

  test("* and / bind tighter than + and -") do
    assert_eq(2 + 3 * 4, 14)
    assert_eq((2 + 3) * 4, 20)
    assert_eq(10 - 4 / 2, 8)
  end

  test("+ concatenates strings, converting a number on either side") do
    assert_eq("hello" + " " + "world", "hello world")
    assert_eq("a" + 1, "a1")
    assert_eq(1 + "a", "1a")
  end

  test("+ with nil raises") do
    assert_raises("Cannot add null and int") do
      nil + 1
    end
  end

  test("division by zero raises") do
    assert_raises("Division by zero") do
      1 / 0
    end
  end
end

describe("Comparison operators") do
  test("equality") do
    assert(1 == 1)
    assert("a" == "a")
    assert_not(1 == 2)
  end

  test("equality across Int and Float compares the value") do
    assert(1 == 1.0)
  end

  test("equality across unrelated types is false, not an error") do
    assert_not(1 == "1")
    assert_ne(nil, false)
  end

  test("arrays and hashes compare by content") do
    assert([1, [2]] == [1, [2]])
    assert([] == [])
    assert({"a": 1, "b": 2} == {"b": 2, "a": 1})
    assert_not([1, 2] == [2, 1])
  end

  test("inequality") do
    assert(1 != 2)
    assert("a" != "b")
    assert_not(1 != 1)
  end

  test("less than") do
    assert(1 < 2)
    assert_not(2 < 1)
    assert_not(1 < 1)
  end

  test("less than or equal") do
    assert(1 <= 2)
    assert(1 <= 1)
    assert_not(2 <= 1)
  end

  test("greater than") do
    assert(2 > 1)
    assert_not(1 > 2)
    assert_not(1 > 1)
  end

  test("greater than or equal") do
    assert(2 >= 1)
    assert(1 >= 1)
    assert_not(1 >= 2)
  end

  test("strings order lexicographically") do
    assert("a" < "b")
    assert_not("b" <= "a")
  end

  test("ordering unrelated types raises") do
    assert_raises("Cannot compare int and string") do
      1 < "a"
    end
  end
end

describe("Logical operators") do
  test("&&") do
    assert(true && true)
    assert_not(true && false)
    assert_not(false && true)
    assert_not(false && false)
  end

  test("||") do
    assert(true || true)
    assert(true || false)
    assert(false || true)
    assert_not(false || false)
  end

  test("!") do
    assert(!false)
    assert_not(!true)
  end

  test("&& does not evaluate its right side after a falsy left") do
    called = false
    result = false && (called = true)
    assert_eq(result, false)
    assert_not(called)
  end

  test("|| does not evaluate its right side after a truthy left") do
    called = false
    result = true || (called = true)
    assert_eq(result, true)
    assert_not(called)
  end

  test("&& and || evaluate the right side when they need it") do
    called = false
    result = true && (called = "right")
    assert_eq(result, "right")
    assert_eq(called, "right")
  end

  test("combined") do
    assert((true && true) || false)
    assert(!(false && true))
    assert((1 < 2) && (3 > 2))
  end
end

# `||` and `&&` answer an operand, and `*` repeats a string: forms `soli check`
# once refused although they ran. The spec runner does not type-check, so these
# pin the runtime half only.
describe("Operand-valued operators") do
  test("|| answers the first truthy operand, not a Bool") do
    let host: String = getenv("SOLI_SPEC_UNSET_HOST") || "localhost"
    assert_eq(host, "localhost")
    let fallback: Int = 0 || 42
    assert_eq(fallback, 42)
    let kept: Int = 7 || 42
    assert_eq(kept, 7)
  end

  test("&& answers the deciding operand too") do
    let last: Int = 1 && 5
    assert_eq(last, 5)
    assert_eq(false && 5, false)
    assert_null(1 && nil)
    assert_eq(nil || false, false)
  end

  test("|| still works as a condition") do
    seen = false
    seen = true if false || true
    assert(seen)
  end

  test("a string repeats by an int, either way round") do
    assert_eq("ab" * 3, "ababab")
    assert_eq(3 * "ab", "ababab")
    assert_eq("-" * 5, "-----")
    assert_eq("abc" * 0, "")
  end

  test("an empty string repeats to an empty string at once, however large the count") do
    # It used to append the empty string once per count: 10^11 passes, minutes.
    started = clock()
    assert_eq("" * 100000000000, "")
    assert_eq(100000000000 * "", "")
    assert_lt(clock() - started, 1.0)
  end

  test("a repeat too large for the limit raises instead of allocating") do
    assert_raises("over the") do
      "ab" * 100000000000
    end
  end

  test("a negative repeat count raises") do
    assert_raises("string * count needs a non-negative count, got -1") do
      "abc" * -1
    end
  end

  test("an array does not repeat") do
    assert_raises("Cannot multiply array and int") do
      [1, 2] * 2
    end
  end
end

describe("Ternary operator") do
  test("returns the true branch") do
    assert_eq(true ? "yes" : "no", "yes")
  end

  test("returns the false branch") do
    assert_eq(false ? "yes" : "no", "no")
  end

  test("takes an expression as its condition") do
    x = 10
    assert_eq(x > 5 ? "big" : "small", "big")
  end

  test("nests to the right") do
    assert_eq(sign_of(-1), "negative")
    assert_eq(sign_of(0), "zero")
    assert_eq(sign_of(5), "positive")
  end

  test("evaluates only the chosen branch") do
    calls = []
    result = true ? calls.push("then") : calls.push("else")
    assert_eq(calls, ["then"])
    assert_eq(result, ["then"])
  end
end

describe("Shovel operator <<") do
  test("appends to an array") do
    numbers = [1, 2, 3]
    numbers << 4
    assert_eq(numbers, [1, 2, 3, 4])
  end

  test("returns the array, so it chains") do
    numbers = []
    result = numbers << 1
    assert_eq(result, [1])
    numbers << 2 << 3
    assert_eq(numbers, [1, 2, 3])
  end

  test("appends any type") do
    mixed = [1, "two"]
    mixed << 3.0
    mixed << true
    assert_eq(mixed, [1, "two", 3.0, true])
  end

  test("raises on a non-array left side") do
    count = 5
    assert_raises("<< expects an array on the left, got int") do
      count << 1
    end
  end
end

# The falsy set, pinned: false, nil, 0, "", [] and {} — and nothing else.
describe("Truthiness") do
  test("the whole falsy set") do
    assert_eq(false || "f", "f")
    assert_eq(nil || "f", "f")
    assert_eq(0 || "f", "f")
    assert_eq("" || "f", "f")
    assert_eq([] || "f", "f")
    assert_eq({} || "f", "f")
  end

  test("0.0 is truthy even though 0 is not") do
    assert_eq(0.0 || "f", 0.0)
    assert_eq(0 || "f", "f")
  end

  test("everything else is truthy") do
    assert_eq(1 || "f", 1)
    assert_eq("x" || "f", "x")
    assert_eq([0] || "f", [0])
    assert_eq({"a": nil} || "f", {"a": nil})
    assert_eq(true || "f", true)
  end
end
