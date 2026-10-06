# Trailing block syntax: passing a block to a method without wrapping it in
# the argument list. Every form is pinned here, so do not run `soli fmt` on
# this file — it rewrites all of them to `do |x| … end`:
#   obj.method |params| body end        bare pipe block
#   obj.method do |params| body end     Ruby do-block
#   obj.method { |params| body }        brace block
#   obj.method(arg) do |params| … end   after parenthesized arguments
#   obj.method(arg, &{ |params| … })    block passed as the last argument
# Each is the same as obj.method(fn(params) { body }).

describe("Bare pipe blocks: |params| body end") do
  test("map") do
    result = [1, 2, 3].map |x| x * 2 end
    assert_eq(result, [2, 4, 6])
  end

  test("filter") do
    result = [1, 2, 3, 4, 5].filter |x| x % 2 == 0 end
    assert_eq(result, [2, 4])
  end

  test("each mutates an outer variable") do
    sum = 0
    [1, 2, 3].each |x| sum = sum + x end
    assert_eq(sum, 6)
  end

  test("find returns the first match") do
    result = [10, 20, 30].find |x| x > 15 end
    assert_eq(result, 20)
  end

  test("any? and all?") do
    assert_eq([1, 2, 3].any? |x| x > 2 end, true)
    assert_eq([2, 4, 6].all? |x| x % 2 == 0 end, true)
  end

  test("sort with a two-parameter block") do
    result = [3, 1, 2].sort |a, b| a - b end
    assert_eq(result, [1, 2, 3])
  end

  test("a multi-statement body returns its last expression") do
    result = [1, 2, 3].map |x|
      doubled = x * 2
      doubled + 1
    end
    assert_eq(result, [3, 5, 7])
  end

  context("on Int") do
    test("times") do
      count = 0
      3.times |i| count = count + i end
      assert_eq(count, 3)
    end

    test("upto after parenthesized arguments") do
      sum = 0
      1.upto(3) |i| sum = sum + i end
      assert_eq(sum, 6)
    end

    test("downto after parenthesized arguments") do
      result = []
      3.downto(1) |i| result.push(i) end
      assert_eq(result, [3, 2, 1])
    end
  end

  context("on Hash") do
    test("each with one parameter receives [key, value] pairs") do
      keys = []
      {"a": 1, "b": 2}.each |pair| keys.push(pair[0]) end
      assert_eq(keys, ["a", "b"])
    end

    test("map with two parameters builds a hash") do
      result = {"a": 1, "b": 2}.map |k, v| [k, v * 10] end
      assert_eq(result, {"a": 10, "b": 20})
    end

    test("filter with one parameter") do
      result = {"a": 1, "b": 2, "c": 3}.filter |pair| pair[1] > 1 end
      assert_eq(result, {"b": 2, "c": 3})
    end
  end
end

describe("Ruby do-blocks: do |params| … end") do
  test("map") do
    result = [1, 2, 3].map do |x| x * 2 end
    assert_eq(result, [2, 4, 6])
  end

  test("hash map with two parameters") do
    curved = {"a": 10, "b": 20}.map do |k, v| [k, v + 5] end
    assert_eq(curved, {"a": 15, "b": 25})
  end

  test("filter") do
    result = [1, 2, 3, 4, 5].filter do |x| x % 2 == 0 end
    assert_eq(result, [2, 4])
  end

  test("hash each with two parameters") do
    pairs = []
    {"a": 1, "b": 2}.each do |k, v|
      pairs.push("#{k}=#{v}")
    end
    assert_eq(pairs, ["a=1", "b=2"])
  end

  test("with no parameters") do
    count = 0
    3.times do count = count + 1 end
    assert_eq(count, 3)
  end

  test("a multi-statement body") do
    log = []
    [1, 2, 3].each do |x|
      doubled = x * 2
      log.push(doubled)
    end
    assert_eq(log, [2, 4, 6])
  end

  test("after parenthesized arguments") do
    result = []
    1.upto(3) do |i| result.push(i) end
    assert_eq(result, [1, 2, 3])
  end

  test("nested do-blocks each see the outer variables") do
    total = 0
    [1, 2].each do |x|
      [10, 20].each do |y|
        total = total + x * y
      end
    end
    assert_eq(total, 90)
  end

  test("a method can be chained after end") do
    assert_eq([1, 2, 3].map do |x| x * 2 end.sum, 12)
  end
end

describe("Brace blocks: { |params| … }") do
  test("map after empty parentheses") do
    result = [1, 2].map() { |item| item + 1 }
    assert_eq(result, [2, 3])
  end

  test("map without parentheses") do
    result = [1, 2, 3].map { |x| x * x }
    assert_eq(result, [1, 4, 9])
  end

  test("filter") do
    result = [1, 2, 3, 4].filter { |x| x % 2 == 0 }
    assert_eq(result, [2, 4])
  end

  test("hash map with two parameters") do
    result = {"a": 1, "b": 2}.map { |k, v| [k, v * 10] }
    assert_eq(result, {"a": 10, "b": 20})
  end

  test("with no parameters") do
    count = 0
    3.times { count = count + 1 }
    assert_eq(count, 3)
  end

  test("after parenthesized arguments") do
    result = []
    1.upto(3) { |i| result.push(i) }
    assert_eq(result, [1, 2, 3])
  end

  test("a multi-line brace block can be chained") do
    total = [1, 2, 3].map { |x|
      x * 2
    }.sum
    assert_eq(total, 12)
  end

  test("nested brace blocks") do
    nested = [[1, 2], [3]].map { |row| row.map { |x| x * 10 } }
    assert_eq(nested, [[10, 20], [30]])
  end

  test("find returns nil when nothing matches") do
    assert_null([10, 20, 30].find { |x| x > 99 })
  end

  test("an empty receiver never runs the block") do
    assert_eq([].map { |x| x * 2 }, [])
  end
end

describe("Block passed as an argument: &{ … }") do
  test("as the only argument") do
    result = [1, 2, 3].map(&{ |x| x * 2 })
    assert_eq(result, [2, 4, 6])
  end

  test("after other arguments") do
    result = []
    1.upto(3, &{ |i| result.push(i) })
    assert_eq(result, [1, 2, 3])
  end

  test("with no parameters") do
    count = 0
    3.times(&{ count = count + 1 })
    assert_eq(count, 3)
  end
end

describe("Every form is equivalent to a lambda argument") do
  test("map") do
    expected = [1, 2, 3].map(fn(x) { x + 10 })
    assert_eq(expected, [11, 12, 13])
    assert_eq([1, 2, 3].map |x| x + 10 end, expected)
    assert_eq([1, 2, 3].map do |x| x + 10 end, expected)
    assert_eq([1, 2, 3].map { |x| x + 10 }, expected)
    assert_eq([1, 2, 3].map(&{ |x| x + 10 }), expected)
    assert_eq([1, 2, 3].map(|x| x + 10), expected)
  end

  test("filter") do
    expected = [1, 2, 3, 4].filter(fn(x) { x > 2 })
    assert_eq(expected, [3, 4])
    assert_eq([1, 2, 3, 4].filter |x| x > 2 end, expected)
    assert_eq([1, 2, 3, 4].filter { |x| x > 2 }, expected)
  end

  test("times") do
    with_block = 0
    with_lambda = 0
    3.times |i| with_block = with_block + i end
    3.times(fn(i) { with_lambda = with_lambda + i })
    assert_eq(with_block, 3)
    assert_eq(with_lambda, 3)
  end
end
