# Primitive method dispatch: Bool, Nil, Int, Float and Decimal methods that
# take arguments (`is_a?`, `round(n)`, `between?`, `clamp`, `gcd`, `lcm`,
# `pow`, `to_s(base)`), and the errors each raises on a bad argument. These
# are the `call_<type>_method` paths in `interpreter/executor/calls/`.

describe("Bool method dispatch") do
  test("is_a? is true for bool and object") do
    assert(true.is_a?("bool"))
    assert(false.is_a?("bool"))
    assert(true.is_a?("object"))
    assert(false.is_a?("object"))
  end

  test("is_a? is false for unrelated types") do
    assert_not(true.is_a?("int"))
    assert_not(false.is_a?("string"))
    assert_not(true.is_a?("float"))
  end

  test("is_a? with a non-string argument raises") do
    assert_raises("is_a? expects a string argument") do
      true.is_a?(42)
    end
  end

  test("is_a? with two arguments raises") do
    assert_raises("Wrong number of arguments: expected 1, got 2") do
      true.is_a?("bool", "extra")
    end
  end

  test("an unknown method raises") do
    assert_raises("Cannot access property 'frobnicate' on bool") do
      true.frobnicate(1)
    end
  end
end

describe("Nil method dispatch") do
  test("is_a? is true for null and object") do
    assert(nil.is_a?("null"))
    assert(nil.is_a?("object"))
  end

  test("is_a? is false for unrelated types") do
    assert_not(nil.is_a?("int"))
    assert_not(nil.is_a?("string"))
    assert_not(nil.is_a?("bool"))
  end

  test("is_a? with a non-string argument raises") do
    assert_raises("is_a? expects a string argument") do
      nil.is_a?(0)
    end
  end

  test("is_a? with no argument raises") do
    assert_raises("Wrong number of arguments: expected 1, got 0") do
      nil.is_a?()
    end
  end
end

describe("Int method dispatch") do
  context("gcd and lcm") do
    test("gcd returns the greatest common divisor") do
      assert_eq((12).gcd(8), 4)
      assert_eq((17).gcd(5), 1)
      assert_eq((0).gcd(7), 7)
      assert_eq((0).gcd(0), 0)
    end

    test("gcd ignores signs") do
      assert_eq((-12).gcd(8), 4)
      assert_eq((12).gcd(-8), 4)
    end

    test("gcd with a non-integer argument raises") do
      assert_raises("gcd expects an integer argument") do
        (12).gcd("eight")
      end
    end

    test("lcm returns the least common multiple") do
      assert_eq((4).lcm(6), 12)
      assert_eq((3).lcm(5), 15)
      assert_eq((-4).lcm(6), 12)
    end

    test("lcm of 0 and 0 is 0") do
      assert_eq((0).lcm(0), 0)
    end

    test("lcm with a float argument raises") do
      assert_raises("lcm expects an integer argument") do
        (4).lcm(6.0)
      end
    end
  end

  context("between? and clamp") do
    test("between? is inclusive at both ends") do
      assert((5).between?(1, 10))
      assert((1).between?(1, 10))
      assert((10).between?(1, 10))
    end

    test("between? is false outside the range") do
      assert_not((0).between?(1, 10))
      assert_not((11).between?(1, 10))
    end

    test("between? accepts float bounds") do
      assert((5).between?(1.5, 10.5))
      assert_not((1).between?(1.5, 10.5))
    end

    test("between? with a non-numeric lower bound raises") do
      assert_raises("between? expects numeric arguments") do
        (5).between?("low", 10)
      end
    end

    test("between? with a non-numeric upper bound raises") do
      assert_raises("between? expects numeric arguments") do
        (5).between?(1, "high")
      end
    end

    test("clamp restricts the value to the range") do
      assert_eq((5).clamp(1, 10), 5)
      assert_eq((0).clamp(1, 10), 1)
      assert_eq((20).clamp(1, 10), 10)
    end

    test("clamp with a float lower bound raises") do
      assert_raises("clamp expects integer arguments") do
        (5).clamp(1.0, 10)
      end
    end

    test("clamp with a float upper bound raises") do
      assert_raises("clamp expects integer arguments") do
        (5).clamp(1, 10.0)
      end
    end
  end

  context("is_a?") do
    test("is true for int, numeric and object") do
      assert((42).is_a?("int"))
      assert((42).is_a?("numeric"))
      assert((42).is_a?("object"))
    end

    test("is false for unrelated types") do
      assert_not((42).is_a?("string"))
      assert_not((42).is_a?("bool"))
    end
  end

  context("pow") do
    test("a non-negative integer exponent gives an int") do
      assert_eq((2).pow(10), 1024)
      assert_eq((2).pow(0), 1)
    end

    test("a negative exponent gives a float") do
      assert_eq((2).pow(-2), 0.25)
    end

    test("a float exponent gives a float") do
      assert_eq((4).pow(0.5), 2.0)
    end

    test("a non-numeric exponent raises") do
      assert_raises("pow expects a numeric argument") do
        (2).pow("three")
      end
    end
  end

  context("to_s") do
    test("without a base gives the decimal string, with or without parens") do
      assert_eq((255).to_s, "255")
      assert_eq((255).to_s(), "255")
      assert_eq((-7).to_string, "-7")
    end

    test("with a base converts the radix") do
      assert_eq((255).to_s(16), "ff")
      assert_eq((255).to_s(2), "11111111")
      assert_eq((255).to_s(8), "377")
      assert_eq((255).to_s(10), "255")
      assert_eq((35).to_s(36), "z")
      assert_eq((0).to_s(16), "0")
      assert_eq((255).to_string(16), "ff")
    end

    test("with a base keeps the sign of a negative number") do
      assert_eq((-255).to_s(16), "-ff")
      assert_eq((-9223372036854775807 - 1).to_s(16), "-8000000000000000")
    end

    test("round-trips with String#hex") do
      assert_eq("ff".hex.to_s(16), "ff")
    end

    test("a base below 2 raises") do
      assert_raises("to_s base must be between 2 and 36, got 1") do
        (255).to_s(1)
      end
    end

    test("a base above 36 raises") do
      assert_raises("to_s base must be between 2 and 36, got 37") do
        (255).to_s(37)
      end
    end

    test("a non-integer base raises") do
      assert_raises("to_s expects an integer base, got string") do
        (255).to_s("x")
      end
    end
  end
end

describe("Int iterators with a bad argument") do
  test("times with a non-function argument raises") do
    assert_raises("times expects a function argument") do
      (3).times(42)
    end
  end

  test("upto with a non-integer limit raises") do
    assert_raises("upto expects an integer limit") do
      (1).upto("ten", fn(i) { i })
    end
  end

  test("downto with a non-function body raises") do
    assert_raises("downto expects a function argument") do
      (3).downto(1, "noop")
    end
  end
end

# Every type answers `to_i`, so the one type that already IS an int was the
# only one on which `x.to_i` could raise — which is exactly where a caller
# reaching for it would never expect it to. `to_f` on a float was the same
# gap, mirrored. Both are what let a value be normalised in one call,
# whatever arrived.
describe("A conversion to a type's own type is the identity") do
  test("an int converts to an int") do
    assert_eq((42).to_i, 42)
    assert_eq((42).to_int, 42)
    assert_eq((-7).to_i, -7)
    assert_eq((0).to_i, 0)
    assert_eq((42).to_i(), 42)
  end

  test("a float converts to a float") do
    assert_eq((3.5).to_f, 3.5)
    assert_eq((3.5).to_float, 3.5)
    assert_eq((-0.25).to_f, -0.25)
    assert_eq((3.5).to_f(), 3.5)
  end

  # The point of the pair: one call normalises whatever arrived, so the
  # caller stops writing `?? 0` around it.
  test("to_i is total over the scalars a param can hold") do
    seen = [7, "7", 7.9, nil, true].map { |value| value.to_i }
    assert_eq(seen, [7, 7, 7, 0, 1])
  end

  test("to_f is total over the same") do
    assert_eq((7).to_f, 7.0)
    assert_eq("7.5".to_f, 7.5)
    assert_eq((7.5).to_f, 7.5)
    assert_eq(nil.to_f, 0.0)
  end
end

describe("Float method dispatch") do
  context("round") do
    test("with no argument returns an int, rounding half away from zero") do
      assert_eq((3.4).round, 3)
      assert_eq((3.6).round, 4)
      assert_eq((-2.5).round, -3)
    end

    test("with digits returns a float") do
      assert_eq((3.14159).round(2), 3.14)
      assert_eq((3.14159).round(4), 3.1416)
      assert_eq((1.5).round(0), 2.0)
    end

    test("honors the decimal value, not the binary representation") do
      # 38.995 stored as f64 is ~38.99499999999999744, so naive
      # `(n * 100).round() / 100` would yield 38.99. Soli rounds via
      # the shortest round-trip decimal so the answer is 39.0 — matching
      # Ruby 2.4+ and user intent.
      assert_eq((38.995).round(2), 39.0)
      assert_eq((2.675).round(2), 2.68)
      assert_eq((1.235).round(2), 1.24)
      assert_eq((38.985).round(2), 38.99)
      assert_eq((-38.995).round(2), -39.0)
    end

    test("with negative digits rounds to a power of 10") do
      assert_eq((12345.0).round(-2), 12300.0)
      assert_eq((12345.0).round(-3), 12000.0)
      assert_eq((1234.0).round(-4), 0.0)
    end

    test("with a non-integer argument raises") do
      assert_raises("round expects an integer argument") do
        (3.14).round("two")
      end
    end

    test("with two arguments raises") do
      assert_raises("Wrong number of arguments: expected 1, got 2") do
        (3.14).round(2, 3)
      end
    end
  end

  context("between? and clamp") do
    test("between? is inclusive") do
      assert((3.14).between?(1.0, 5.0))
      assert((1.0).between?(1.0, 5.0))
    end

    test("between? is false outside the range") do
      assert_not((0.5).between?(1.0, 5.0))
      assert_not((6.0).between?(1.0, 5.0))
    end

    test("between? accepts integer bounds") do
      assert((3.14).between?(1, 5))
      assert_not((5.5).between?(1, 5))
    end

    test("between? with a non-numeric lower bound raises") do
      assert_raises("between? expects numeric arguments") do
        (3.14).between?("low", 5.0)
      end
    end

    test("between? with a non-numeric upper bound raises") do
      assert_raises("between? expects numeric arguments") do
        (3.14).between?(1.0, "high")
      end
    end

    test("clamp restricts the value") do
      assert_eq((3.14).clamp(0.0, 2.0), 2.0)
      assert_eq((3.14).clamp(5.0, 10.0), 5.0)
      assert_eq((3.14).clamp(0.0, 5.0), 3.14)
    end

    test("clamp accepts integer bounds and returns a float") do
      assert_eq((3.14).clamp(0, 2), 2.0)
      assert_eq(type((3.14).clamp(0, 2)), "float")
    end

    test("clamp with a non-numeric lower bound raises") do
      assert_raises("clamp expects numeric arguments") do
        (3.14).clamp("zero", 10.0)
      end
    end

    test("clamp with a non-numeric upper bound raises") do
      assert_raises("clamp expects numeric arguments") do
        (3.14).clamp(0.0, "ten")
      end
    end
  end

  context("is_a? and unknown methods") do
    test("is_a? is true for float, numeric and object") do
      assert((3.14).is_a?("float"))
      assert((3.14).is_a?("numeric"))
      assert((3.14).is_a?("object"))
    end

    test("is_a? is false for unrelated types") do
      assert_not((3.14).is_a?("int"))
      assert_not((3.14).is_a?("string"))
    end

    test("is_a? with a non-string argument raises") do
      assert_raises("is_a? expects a string argument") do
        (3.14).is_a?(1)
      end
    end

    test("an unknown method raises") do
      assert_raises("Cannot access property 'frobnicate' on float") do
        (3.14).frobnicate(1)
      end
    end
  end
end

describe("Decimal method dispatch") do
  context("round") do
    test("with no argument returns an int") do
      assert_eq(3.7D.round, 4)
      assert_eq(3.4D.round, 3)
      assert_eq(type(3.7D.round), "int")
    end

    test("with digits returns a decimal") do
      rounded = 3.14159D.round(2)
      assert_eq(type(rounded), "decimal")
      assert_eq(str(rounded), "3.14")
    end

    test("with a non-integer argument raises") do
      assert_raises("round expects an integer argument") do
        3.14D.round("two")
      end
    end

    test("with two arguments raises") do
      assert_raises("Wrong number of arguments: expected 1, got 2") do
        3.14D.round(2, 3)
      end
    end
  end

  context("between? and clamp") do
    test("between? accepts int, float and decimal bounds") do
      price = 3.14D
      assert(price.between?(1, 5))
      assert(price.between?(1.0, 5.0))
      assert(price.between?(1.00D, 5.00D))
    end

    test("between? is false outside the range") do
      assert_not(10.00D.between?(0, 5))
    end

    test("between? with a non-numeric lower bound raises") do
      assert_raises("between? expects numeric arguments") do
        3.14D.between?("low", 5)
      end
    end

    test("between? with a non-numeric upper bound raises") do
      assert_raises("between? expects numeric arguments") do
        3.14D.between?(1, "high")
      end
    end

    test("clamp restricts the value") do
      assert_eq(str(3.14D.clamp(0.00D, 2.00D)), "2.00")
      assert_eq(str(3.14D.clamp(0.00D, 5.00D)), "3.14")
    end

    test("clamp accepts integer bounds") do
      clamped = 3.14D.clamp(0, 2)
      assert_eq(type(clamped), "decimal")
      assert_eq(str(clamped), "2")
    end

    test("clamp with a non-numeric lower bound raises") do
      assert_raises("clamp expects decimal or integer arguments") do
        3.14D.clamp("zero", 10.00D)
      end
    end

    test("clamp with a non-numeric upper bound raises") do
      assert_raises("clamp expects decimal or integer arguments") do
        3.14D.clamp(0.00D, "ten")
      end
    end
  end

  context("is_a? and unknown methods") do
    test("is_a? is true for decimal, numeric and object") do
      price = 3.14D
      assert(price.is_a?("decimal"))
      assert(price.is_a?("numeric"))
      assert(price.is_a?("object"))
    end

    test("is_a? is false for unrelated types") do
      assert_not(3.14D.is_a?("float"))
      assert_not(3.14D.is_a?("int"))
    end

    test("is_a? with a non-string argument raises") do
      assert_raises("is_a? expects a string argument") do
        3.14D.is_a?(1)
      end
    end

    test("an unknown method raises") do
      assert_raises("Cannot access property 'frobnicate' on decimal") do
        3.14D.frobnicate(1)
      end
    end
  end
end
