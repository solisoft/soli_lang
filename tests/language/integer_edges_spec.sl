# Integers at the edges of i64, on whichever engine runs the spec.

def caught(block)
  try
    block()
  catch error
    # Lowercased: a spec earlier in the same process redefines String#to_s.
    return error.to_s.downcase
  end
  "no error"
end

describe("Integer comparison") do
  test("is exact past 2^53, where two Ints share one f64") do
    big = 9007199254740993
    assert(big > 9007199254740992)
    assert(9007199254740992 < big)
    assert(big >= 9007199254740993)
    assert(!(big <= 9007199254740992))
    assert(9223372036854775807 > 9223372036854775806)
  end
end

describe("Integer negation") do
  test("negates every Int but the smallest") do
    assert_eq(-(9223372036854775807), -9223372036854775807)
    smallest = -9223372036854775807 - 1
    assert_eq(-(smallest + 1), 9223372036854775807)
  end

  test("raises an overflow on the smallest Int, as subtraction does") do
    smallest = -9223372036854775807 - 1
    message = caught(fn() { -smallest })
    assert(message.includes?("integer overflow"))
    assert(caught(fn() { 0 - smallest }).includes?("integer overflow"))
  end
end
