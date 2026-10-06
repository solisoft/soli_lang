# `Set.new` and `Range.new`: the placeholder collection classes. Range literals
# such as `(1..5)` build plain Arrays and are covered in
# tests/language/range_spread_spec.sl.

describe("Set.new") do
  test("builds an instance of Set") do
    set = Set.new
    assert_eq(set.class, "Set")
    assert(set.is_a?("Set"))
  end

  test("each call builds a distinct instance") do
    assert_ne(Set.new, Set.new)
  end

  test("takes no arguments") do
    assert_raises("Wrong number of arguments: expected 0, got 1") do
      Set.new([1, 2])
    end
  end
end

describe("Range.new") do
  test("builds an instance of Range from two ints") do
    pending("bug: Range.new(1, 5) raises 'Range.new() requires integer end' (reads args[1]/args[2])")
    range = Range.new(1, 5)
    assert_eq(range.class, "Range")
  end

  test("refuses a non-int start") do
    pending("bug: Range.new(\"a\", 5) reports 'requires integer end', not the bad start")
    assert_raises("Range.new() requires integer start") do
      Range.new("a", 5)
    end
  end

  test("takes exactly two arguments") do
    assert_raises("Wrong number of arguments: expected 2, got 3") do
      Range.new(1, 5, 7)
    end
  end
end
