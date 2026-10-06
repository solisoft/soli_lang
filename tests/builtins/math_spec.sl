# The Math module and the numeric methods on Int and Float.

INFINITY = "inf".to_f
NAN = (-1.0).sqrt

describe("Math") do
  test("floor, ceil and round return Ints") do
    assert_eq(Math.floor(3.9), 3)
    assert_eq(Math.floor(-3.1), -4)
    assert_eq(Math.ceil(3.1), 4)
    assert_eq(Math.ceil(-3.1), -3)
    assert_eq(type(Math.floor(3.9)), "int")
  end

  test("round sends halves away from zero") do
    assert_eq(Math.round(3.4), 3)
    assert_eq(Math.round(3.5), 4)
    assert_eq(Math.round(2.5), 3)
    assert_eq(Math.round(-2.5), -3)
  end

  test("log and log10") do
    assert_eq(Math.log(Math.e), 1)
    assert_eq(Math.log(1.0), 0)
    assert_eq(Math.log10(100), 2.0)
    assert_eq(Math.log10(1000), 3)
  end

  test("log of zero is negative infinity") do
    assert_eq(Math.log(0.0), -INFINITY)
  end

  test("pi and e") do
    assert_eq(Math.pi, 3.141592653589793)
    assert_eq(Math.e, 2.718281828459045)
  end

  test("sin, cos and tan") do
    assert_eq(Math.sin(0.0), 0)
    assert_eq(Math.sin(Math.pi / 2), 1)
    assert_eq(Math.cos(0.0), 1)
    assert_eq(Math.tan(0.0), 0)
  end

  test("exp") do
    assert_eq(Math.exp(0.0), 1)
    assert_eq(Math.exp(1.0), Math.e)
  end

  test("random is a Float in [0, 1)") do
    draws = (0..200).map { |_i| Math.random() }
    assert(draws.all? { |draw| draw >= 0.0 && draw < 1.0 })
    assert_eq(type(draws[0]), "float")
  end

  test("random varies") do
    assert_gt(((0..20).map { |_i| Math.random() }).uniq.length, 1)
  end
end

describe("Int and Float magnitude") do
  test("abs") do
    assert_eq((-5).abs, 5)
    assert_eq(5.abs, 5)
    assert_eq(0.abs, 0)
    assert_eq((-3.14).abs, 3.14)
  end

  test("sqrt returns a Float") do
    assert_eq(4.sqrt, 2.0)
    assert_eq(2.sqrt, 1.4142135623730951)
    assert_eq(2.25.sqrt, 1.5)
    assert_eq(type(9.sqrt), "float")
  end

  test("sqrt of a negative is NaN") do
    assert((-4).sqrt.nan?)
  end

  test("pow") do
    assert_eq(2.pow(3), 8)
    assert_eq(10.pow(2), 100)
    assert_eq(2.pow(0), 1)
    assert_eq(2.pow(62), 4611686018427387904)
  end

  test("pow with a negative exponent is a Float") do
    assert_eq(2.pow(-1), 0.5)
  end

  test("pow raises when the Int overflows") do
    pending("bug: 2.pow(64) wraps to 0 instead of raising an integer overflow")
    assert_raises("overflow") do
      2.pow(64)
    end
  end

  test("min and max of an array") do
    assert_eq([3, 5].min, 3)
    assert_eq([-1, 1].min, -1)
    assert_eq([3, 5].max, 5)
    assert_eq([1, 0.5].min, 0.5)
  end

  test("min of an empty array is nil") do
    assert_null([].min)
  end

  test("Float rounding methods") do
    assert_eq(3.7.floor, 3)
    assert_eq(3.2.ceil, 4)
    assert_eq(3.5.round, 4)
    assert_eq(3.14159.round(2), 3.14)
    assert_eq((-3.7).truncate, -3)
    assert_eq(3.7.to_i, 3)
  end

  test("Int gcd, lcm, clamp and between?") do
    assert_eq(10.gcd(4), 2)
    assert_eq(10.lcm(4), 20)
    assert_eq(5.clamp(1, 3), 3)
    assert(5.between?(1, 10))
    assert_not(11.between?(1, 10))
  end
end

describe("Int predicates") do
  test("even? and odd?") do
    assert_eq([4.even?, 3.even?, 0.even?, (-2).even?], [true, false, true, true])
    assert_eq([3.odd?, 4.odd?, (-3).odd?], [true, false, true])
  end

  test("zero?, positive? and negative?") do
    assert_eq([0.zero?, 1.zero?], [true, false])
    assert_eq([1.positive?, 0.positive?, (-1).positive?], [true, false, false])
    assert_eq([(-1).negative?, 0.negative?, 1.negative?], [true, false, false])
  end
end

describe("Int succ, pred and chr") do
  test("succ, next and pred step by one") do
    assert_eq(5.succ, 6)
    assert_eq(5.next, 6)
    assert_eq(5.pred, 4)
    assert_eq(0.pred, -1)
  end

  test("chr is the character of a code point") do
    assert_eq(65.chr, "A")
    assert_eq(97.chr, "a")
    assert_eq(233.chr, "é")
    assert_eq(128512.chr, "😀")
  end

  test("chr raises outside Unicode") do
    assert_raises("-1 is not a valid character code") do
      (-1).chr
    end
    assert_raises("1114112 is not a valid character code") do
      1114112.chr
    end
  end
end

describe("Int#divmod") do
  test("returns the floored quotient and the remainder") do
    assert_eq(7.divmod(2), [3, 1])
    assert_eq(6.divmod(3), [2, 0])
  end

  test("floors a negative dividend") do
    assert_eq((-7).divmod(2), [-4, 1])
  end

  test("floors a negative divisor, the remainder taking its sign") do
    pending("bug: 7.divmod(-2) is [-3, 1]; floored division gives [-4, -1], as (-7).divmod(2) and Float#divmod do")
    assert_eq(7.divmod(-2), [-4, -1])
  end

  test("raises on a zero divisor") do
    assert_raises("divmod: division by zero") do
      7.divmod(0)
    end
  end

  test("raises on a Float divisor") do
    assert_raises("divmod expects integer, got float") do
      7.divmod(2.0)
    end
  end
end

describe("Float predicates") do
  test("nan?") do
    assert(NAN.nan?)
    assert_not(1.5.nan?)
    assert_not(INFINITY.nan?)
  end

  test("finite?") do
    assert(1.5.finite?)
    assert_not(INFINITY.finite?)
    assert_not(NAN.finite?)
  end

  test("infinite?") do
    assert(INFINITY.infinite?)
    assert((-INFINITY).infinite?)
    assert_not(1.5.infinite?)
  end

  test("zero?, positive? and negative?") do
    assert(0.0.zero?)
    assert_not(0.5.zero?)
    assert((-0.5).negative?)
    assert(0.5.positive?)
  end
end

describe("Float#divmod") do
  test("returns the floored quotient and the remainder") do
    assert_eq(7.5.divmod(2), [3, 1.5])
    assert_eq(7.5.divmod(2.5), [3, 0])
  end

  test("floors with a negative operand") do
    assert_eq((-7.5).divmod(2), [-4, 0.5])
    assert_eq(7.5.divmod(-2), [-4, -0.5])
  end

  test("raises on a zero divisor") do
    assert_raises("divmod: division by zero") do
      7.5.divmod(0)
    end
  end
end
