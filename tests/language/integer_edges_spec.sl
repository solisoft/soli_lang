# Integers at the edges of i64, on whichever engine runs the spec: exact
# comparison past 2^53, and overflow raising instead of wrapping.

const INT_LARGEST = 9223372036854775807
const INT_SMALLEST = -9223372036854775807 - 1

describe("Integer comparison") do
  test("is exact past 2^53, where two Ints share one f64") do
    big = 9007199254740993
    assert(big > 9007199254740992)
    assert(9007199254740992 < big)
    assert(big >= 9007199254740993)
    assert_not(big <= 9007199254740992)
    assert_ne(big, 9007199254740992)
    assert(INT_LARGEST > 9223372036854775806)
  end

  test("the extremes print exactly") do
    assert_eq(INT_LARGEST.to_s, "9223372036854775807")
    assert_eq(INT_SMALLEST.to_s, "-9223372036854775808")
  end
end

describe("Integer negation") do
  test("negates every Int but the smallest") do
    assert_eq(-(INT_LARGEST), -9223372036854775807)
    assert_eq(-(INT_SMALLEST + 1), INT_LARGEST)
  end

  test("raises an overflow on the smallest Int, as subtraction does") do
    assert_raises("integer overflow: -(-9223372036854775808) does not fit in an Int") do
      -INT_SMALLEST
    end
    assert_raises("integer overflow: 0 - -9223372036854775808") do
      0 - INT_SMALLEST
    end
  end

  test("abs raises on the smallest Int as negation does") do
    # `(-9223372036854775807 - 1).abs` returns -9223372036854775808 on both
    # engines: it wraps where `-x` raises.
    pending("bug: Int#abs wraps on the smallest Int instead of raising an overflow")
    assert_raises("integer overflow") do
      INT_SMALLEST.abs
    end
  end
end

describe("Integer overflow") do
  test("addition past the largest Int raises") do
    assert_raises("integer overflow: 9223372036854775807 + 1 does not fit in an Int") do
      INT_LARGEST + 1
    end
  end

  test("multiplication past the largest Int raises") do
    assert_raises("integer overflow: 9223372036854775807 * 2") do
      INT_LARGEST * 2
    end
  end

  test("the smallest Int divided by -1 raises") do
    assert_raises("integer overflow: -9223372036854775808 / -1") do
      INT_SMALLEST / -1
    end
    assert_raises("integer overflow: -9223372036854775808 % -1") do
      INT_SMALLEST % -1
    end
  end

  test("the message names the Int range") do
    message = assert_raises() do
      INT_LARGEST + 1
    end
    assert_contains(message, "range -9223372036854775808..9223372036854775807")
  end

  test("arithmetic right up to the edge is exact") do
    assert_eq(INT_LARGEST - 1 + 1, INT_LARGEST)
    assert_eq(INT_SMALLEST + 1 - 1, INT_SMALLEST)
  end
end

describe("Integer division by zero") do
  test("division and modulo raise") do
    assert_raises("Division by zero") do
      1 / 0
    end
    assert_raises("Division by zero") do
      1 % 0
    end
  end
end
