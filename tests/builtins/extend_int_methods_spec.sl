# Int.define_method and Float.define_method add methods to number values.
# Extensions outlive this file, so every name starts with `spec_`.

describe("Int.define_method") do
  test("adds a method callable on any Int") do
    Int.define_method("spec_triple", fn() { this * 3 })
    assert_eq(4.spec_triple, 12)
    assert_eq((-2).spec_triple, -6)
  end

  test("a method with arguments") do
    Int.define_method("spec_plus_n", fn(addend) { this + addend })
    assert_eq(10.spec_plus_n(5), 15)
    assert_eq(0.spec_plus_n(7), 7)
  end

  test("a zero-argument method runs with or without parentheses") do
    Int.define_method("spec_squared", fn() { this * this })
    assert_eq(5.spec_squared, 25)
    assert_eq(5.spec_squared(), 25)
  end

  test("accepts a symbol as the name") do
    Int.define_method(:spec_increment, fn() { this + 1 })
    assert_eq(1.spec_increment, 2)
  end

  test("builtin methods still work alongside") do
    Int.define_method("spec_noop", fn() { this })
    assert_eq((-7).abs, 7)
    assert_eq(3.spec_noop, 3)
  end

  test("does not reach Float") do
    Int.define_method("spec_int_only", fn() { this })
    assert_raises("spec_int_only") do
      3.0.spec_int_only
    end
  end
end

describe("Float.define_method") do
  test("adds a method callable on any Float") do
    Float.define_method("spec_half", fn() { this / 2 })
    assert_eq(3.0.spec_half, 1.5)
  end
end
