# Ranges and the array spread operator. `a..b` is exclusive and evaluates to an
# Array, as does `range(start, stop, step)`; `...xs` splices an array into an
# array literal.

describe("Range literal") do
  test("is exclusive of its end") do
    assert_eq(1..5, [1, 2, 3, 4])
    assert_eq((1..51).length, 50)
    assert_eq((1..51).last, 50)
  end

  test("is an ordinary Array value") do
    numbers = 1..4
    assert_eq(type(numbers), "array")
    assert_eq(numbers.length, 3)
    assert_eq(numbers.first, 1)
    assert_eq(numbers.sum, 6)
    assert_eq(numbers.reverse, [3, 2, 1])
  end

  test("takes variables and negative numbers as bounds") do
    low = -2
    high = 2
    assert_eq(low..high, [-2, -1, 0, 1])
  end

  test("is empty when start equals end") do
    assert_eq(3..3, [])
  end

  test("is empty when start is past end: it never counts down") do
    assert_eq(5..1, [])
  end

  test("membership excludes the end") do
    assert((1..5).includes?(4))
    assert_not((1..5).includes?(5))
    assert_not((1..5).includes?(0))
  end

  test("iterates with blocks") do
    assert_eq((1..5).map { |i| i * 2 }, [2, 4, 6, 8])
    assert_eq((0..10).filter { |i| i % 3 == 0 }, [0, 3, 6, 9])
  end

  test("drives a for loop") do
    sum = 0
    for i in 1..4
      sum = sum + i
    end
    assert_eq(sum, 6)
  end

  test("refuses non-integer bounds") do
    assert_raises("range (..) expects two integers, got float and int") do
      1.0..3
    end
    assert_raises("got string and string") do
      "a".."c"
    end
  end

  test("is not a slice: indexing an array with a range raises") do
    assert_raises("cannot index array with array") do
      [10, 20, 30][0..2]
    end
  end
end

describe("range()") do
  test("builds an exclusive array from start to stop") do
    numbers = range(1, 6)
    assert_eq(numbers, [1, 2, 3, 4, 5])
    assert_eq(range(1, 6), 1..6)
  end

  test("with a single element") do
    assert_eq(range(3, 4), [3])
  end

  test("with negative numbers") do
    assert_eq(range(-3, 4), [-3, -2, -1, 0, 1, 2, 3])
  end

  test("is empty when start equals stop") do
    assert_eq(range(3, 3), [])
  end

  test("takes a step") do
    assert_eq(range(0, 10, 2), [0, 2, 4, 6, 8])
  end

  test("counts down with a negative step") do
    assert_eq(range(10, 0, -3), [10, 7, 4, 1])
  end

  test("refuses a zero step") do
    assert_raises("range() step cannot be zero") do
      range(1, 10, 0)
    end
  end

  test("refuses a single argument") do
    assert_raises("range() expects 2 or 3 arguments, got 1") do
      range(5)
    end
  end

  test("drives a for loop") do
    sum = 0
    for i in range(1, 4)
      sum = sum + i
    end
    assert_eq(sum, 6)
  end
end

describe("Spread operator") do
  test("splices an array into another") do
    head = [1, 2, 3]
    assert_eq([...head, 4, 5], [1, 2, 3, 4, 5])
  end

  test("splices several arrays") do
    first = [1, 2]
    second = [3, 4]
    assert_eq([...first, ...second], [1, 2, 3, 4])
  end

  test("splices in the middle") do
    middle = [2, 3]
    assert_eq([1, ...middle, 4], [1, 2, 3, 4])
  end

  test("an empty array contributes nothing") do
    empty = []
    assert_eq([...empty, 1], [1])
    assert_eq([...empty, ...empty], [])
  end

  test("splices one level only: nested arrays stay nested") do
    nested = [[1], [2]]
    assert_eq([...nested, 3], [[1], [2], 3])
  end

  test("makes a new array: the source is not changed") do
    source = [1, 2]
    copy = [...source]
    copy.push(3)
    assert_eq(source, [1, 2])
    assert_eq(copy, [1, 2, 3])
  end

  test("splices a range and a range() result") do
    assert_eq([...(1..4), 4, 5], [1, 2, 3, 4, 5])
    assert_eq([...range(1, 3), ...range(3, 5)], [1, 2, 3, 4])
  end

  test("refuses a value that is not an array") do
    assert_raises("cannot spread non-array value") do
      [...5]
    end
    assert_raises("cannot spread non-array value") do
      [...nil]
    end
  end
end
