# Decimal literals (`19.99D`): exact base-10 values for money. Covers the
# literal's normalized display, exact arithmetic, rounding and conversions.

describe("Decimal literals") do
  test("have the decimal type") do
    assert_eq(type(19.99D), "decimal")
    assert_eq(19.99D.class, "decimal")
  end

  context("display") do
    test("keeps the written digits of a fractional value") do
      assert_eq(str(19.99D), "19.99")
      assert_eq(str(0.0675D), "0.0675")
      assert_eq(str(0.000001D), "0.000001")
      assert_eq(str(9999999999.99D), "9999999999.99")
      assert_eq(str(5.5D), "5.5")
    end

    test("drops trailing zeros after a non-zero fraction") do
      assert_eq(str(1.50D), "1.5")
      assert_eq(str(100.50D), "100.5")
      assert_eq(str(3.140D), "3.14")
    end

    test("shows a whole value with two places") do
      assert_eq(str(100.00D), "100.00")
      assert_eq(str(1.0D), "1.00")
      assert_eq(str(0.00D), "0.00")
    end

    test("keeps the sign of a negative value") do
      assert_eq(str(-10.5D), "-10.5")
    end

    test("ignores underscores used as digit separators") do
      assert_eq(str(1_000.50D), "1000.5")
    end

    test("to_s matches str()") do
      assert_eq(19.99D.to_s, "19.99")
      assert_eq(100.00D.to_s, "100.00")
    end
  end
end

describe("Decimal arithmetic") do
  test("is exact where floats are not") do
    assert_eq(0.1 + 0.2 == 0.3, false)
    assert_eq(0.1D + 0.2D, 0.3D)
    assert_eq(str(0.1D + 0.2D), "0.3")
  end

  test("subtraction can cross zero") do
    assert_eq(str(10.00D - 0.01D), "9.99")
    assert_eq(str(19.99D - 20.00D), "-0.01")
  end

  test("multiplication by an Int stays a decimal") do
    assert_eq(type(19.99D * 3), "decimal")
    assert_eq(str(19.99D * 3), "59.97")
    assert_eq(str(-10.5D * -2), "21.0")
    assert_eq(str(1.25D * 2), "2.50")
  end

  test("adding an Int stays a decimal") do
    assert_eq(type(0.5D + 1), "decimal")
    assert_eq(str(19.99D + 1), "20.99")
  end

  test("adding a Float gives a Float") do
    assert_eq(type(0.5D + 1.5), "float")
    assert_eq(19.99D + 1.5, 21.49)
  end

  test("division carries 28 significant digits") do
    assert_eq(str(10.00D / 3), "3.3333333333333333333333333333")
  end

  test("division by zero raises") do
    assert_raises("Division by zero") do
      1.0D / 0
    end
  end
end

describe("Decimal rounding") do
  test("round(places) rounds half to even") do
    assert_eq(str(1.005D.round(2)), "1.00")
    assert_eq(str(1.015D.round(2)), "1.02")
    assert_eq(str(1.025D.round(2)), "1.02")
    assert_eq(str(2.675D.round(2)), "2.68")
    assert_eq(str(19.99D.round(1)), "20.0")
  end

  test("round with no places rounds half to even to a whole number") do
    assert_eq(str(2.5D.round), "2")
    assert_eq(str(3.5D.round), "4")
    assert_eq(str(-2.5D.round), "-2")
  end

  test("floor and ceil") do
    assert_eq(str(19.99D.floor), "19")
    assert_eq(str(19.01D.ceil), "20")
  end
end

describe("Decimal comparison and equality") do
  test("equal values compare equal whatever their written scale") do
    assert_eq(19.99D, 19.99D)
    assert_eq(100.00D, 100.0D)
    assert_eq(100.00D, 100.000D)
    assert_ne(19.99D, 20.00D)
  end

  test("a computed value equals a literal with fewer trailing zeros") do
    pending("bug: Decimal == compares scale, so 1.25D * 2 (2.50) != 2.5D and neither < nor > holds")
    assert_eq(1.25D * 2, 2.5D)
    assert_eq(100.00D + 8.5D, 108.5D)
  end

  test("ordering operators") do
    assert(19.99D > 19.98D)
    assert(-10.5D < 0.00D)
    assert(1.5D < 2)
  end

  test("a zero decimal is truthy") do
    assert_eq(0.00D ? "truthy" : "falsy", "truthy")
  end
end

describe("Decimal conversions") do
  test("to_f and to_i") do
    assert_eq(19.99D.to_f, 19.99)
    assert_eq(type(19.99D.to_f), "float")
    assert_eq(19.99D.to_i, 19)
  end
end

describe("Decimals in collections") do
  test("an array keeps them") do
    prices = [10.00D, 20.5D, 30.75D]
    assert_eq(prices.length, 3)
    assert_eq(prices.map { |price| price.to_s }, ["10.00", "20.5", "30.75"])
    assert(prices[0] < prices[2])
  end

  test("min and max compare them as numbers") do
    prices = [3.00D, 1.5D, 2.25D]
    assert_eq(str(prices.min), "1.5")
    assert_eq(str(prices.max), "3.00")
  end

  test("sort with a comparator block orders them") do
    sorted = [3.00D, 1.5D, 2.25D].sort { |a, b| a < b ? -1 : 1 }
    assert_eq(sorted.map { |price| price.to_s }, ["1.5", "2.25", "3.00"])
  end

  test("sort with no block orders them") do
    pending("bug: Array#sort with no block leaves Decimal elements unsorted")
    sorted = [3.00D, 1.5D, 2.25D].sort
    assert_eq(sorted.map { |price| price.to_s }, ["1.5", "2.25", "3.00"])
  end

  test("reduce keeps an exact decimal sum") do
    total = [1.10D, 2.20D].reduce(fn(acc, x) acc + x, 0.00D)
    assert_eq(str(total), "3.3")
  end

  test("sum keeps an exact decimal sum") do
    pending("bug: Array#sum converts Decimal elements to Float (3.3000000000000003)")
    total = [1.10D, 2.20D].sum
    assert_eq(type(total), "decimal")
    assert_eq(str(total), "3.3")
  end

  test("a hash keeps them") do
    order = {"subtotal": 100.00D, "tax": 8.5D, "total": 108.5D}
    assert_eq(str(order["subtotal"] + order["tax"]), "108.50")
    assert(order["subtotal"] < order["total"])
  end

  test("a nested hash keeps them") do
    product = {"pricing": {"base": 50.00D, "discount": 5.00D, "final": 45.00D}, "name": "Widget"}
    pricing = product["pricing"]
    assert_eq(pricing["base"] - pricing["discount"], pricing["final"])
    assert_eq(str(pricing["final"]), "45.00")
  end
end
