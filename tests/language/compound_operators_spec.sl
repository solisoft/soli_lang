# Compound assignment (`+= -= *= /= %=`), logical assignment (`||= &&= ??=`)
# and the postfix `++` / `--` operators, on variables, elements and fields.

class CompoundCounter
  count: Int

  new()
    @count = 0
  end

  def bump
    @count += 1
    @count++
    @count
  end
end

describe("Arithmetic compound assignment") do
  test("+= adds") do
    total = 1
    total += 1
    assert_eq(total, 2)
    total += 10
    assert_eq(total, 12)
  end

  test("-= subtracts") do
    total = 10
    total -= 3
    assert_eq(total, 7)
    total -= 7
    assert_eq(total, 0)
    total -= -1
    assert_eq(total, 1)
  end

  test("*= multiplies") do
    total = 3
    total *= 4
    assert_eq(total, 12)
    total *= 0
    assert_eq(total, 0)
  end

  test("/= divides, truncating an Int") do
    total = 10
    total /= 2
    assert_eq(total, 5)
    total /= 2
    assert_eq(total, 2)
  end

  test("/= on a Float keeps the fraction") do
    total = 7.0
    total /= 2
    assert_eq(total, 3.5)
  end

  test("%= takes the remainder") do
    total = 10
    total %= 3
    assert_eq(total, 1)
    other = 15
    other %= 5
    assert_eq(other, 0)
  end

  test("works on floats") do
    value = 1.5
    value += 0.5
    assert_eq(value, 2.0)
    value *= 3.0
    assert_eq(value, 6.0)
    value -= 1.0
    assert_eq(value, 5.0)
    value /= 2.0
    assert_eq(value, 2.5)
  end

  test("+= concatenates strings, *= repeats them") do
    text = "hello"
    text += " world"
    assert_eq(text, "hello world")
    pattern = "ab"
    pattern *= 3
    assert_eq(pattern, "ababab")
  end

  test("+= on a string converts the right side") do
    text = "a"
    text += 1
    assert_eq(text, "a1")
  end

  test("+= appends an array") do
    items = [1, 2]
    items += [3]
    assert_eq(items, [1, 2, 3])
  end

  test("applies in sequence") do
    total = 1
    total += 1
    total *= 3
    total -= 1
    total /= 2
    assert_eq(total, 2)
  end

  context("on elements and fields") do
    test("an array element") do
      items = [1, 2]
      items[0] += 10
      assert_eq(items, [11, 2])
    end

    test("a hash value, by bracket and by dot") do
      counts = {"n": 1}
      counts["n"] += 1
      counts.n += 1
      assert_eq(counts, {"n": 3})
    end

    test("an instance field via @") do
      assert_eq(new CompoundCounter().bump, 2)
    end
  end

  context("errors") do
    test("/= and %= by zero raise") do
      assert_raises("Division by zero") do
        total = 1
        total /= 0
      end
      assert_raises("Division by zero") do
        total = 5
        total %= 0
      end
    end

    test("+= past the largest Int raises an overflow") do
      assert_raises("integer overflow: 9223372036854775807 + 1") do
        total = 9223372036854775807
        total += 1
      end
    end

    test("+= on nil raises") do
      assert_raises("Cannot add null and int") do
        total = nil
        total += 1
      end
    end

    test("+= of a non-array onto an array raises") do
      assert_raises("Cannot add array and int") do
        items = [1]
        items += 2
      end
    end
  end
end

describe("Logical compound assignment") do
  context("||=") do
    test("assigns when the target is nil") do
      value = nil
      value ||= 42
      assert_eq(value, 42)
    end

    test("assigns when the target is false") do
      value = false
      value ||= "fallback"
      assert_eq(value, "fallback")
    end

    test("assigns over 0 and an empty string, which are falsy") do
      count = 0
      count ||= 10
      assert_eq(count, 10)
      text = ""
      text ||= "x"
      assert_eq(text, "x")
    end

    test("keeps an existing truthy value") do
      value = 7
      value ||= 99
      assert_eq(value, 7)
      text = "hello"
      text ||= "ignored"
      assert_eq(text, "hello")
    end

    test("on a hash member, by bracket and by dot") do
      person = {"name": "Alice", "nickname": nil}
      person["age"] ||= 30
      person["name"] ||= "Bob"
      person.nickname ||= "Al"
      assert_eq(person, {"name": "Alice", "nickname": "Al", "age": 30})
    end
  end

  context("&&=") do
    test("assigns when the target is truthy") do
      value = 1
      value &&= 99
      assert_eq(value, 99)
    end

    test("keeps a falsy value") do
      empty = nil
      empty &&= 99
      assert_null(empty)
      flag = false
      flag &&= 99
      assert_eq(flag, false)
      count = 0
      count &&= 5
      assert_eq(count, 0)
    end
  end

  context("??=") do
    test("assigns only when the target is nil") do
      empty = nil
      empty ??= 42
      assert_eq(empty, 42)
      empty ??= 9
      assert_eq(empty, 42)
    end

    test("keeps false, 0 and other values") do
      flag = false
      flag ??= 99
      assert_eq(flag, false)
      count = 0
      count ??= 99
      assert_eq(count, 0)
      text = "x"
      text ??= "y"
      assert_eq(text, "x")
    end

    test("on a hash member holding nil") do
      settings = {"key": nil}
      settings["key"] ??= "set"
      assert_eq(settings, {"key": "set"})
    end
  end
end

describe("Postfix ++ and --") do
  context("++") do
    test("increments") do
      count = 1
      count++
      assert_eq(count, 2)
    end

    test("evaluates to the old value") do
      count = 5
      previous = count++
      assert_eq(previous, 5)
      assert_eq(count, 6)
    end

    test("inside a larger expression, contributes the old value") do
      count = 5
      result = count++ + 10
      assert_eq(result, 15)
      assert_eq(count, 6)
    end

    test("on a Float") do
      value = 5.0
      previous = value++
      assert_eq(previous, 5.0)
      assert_eq(value, 6.0)
    end

    test("repeats") do
      count = 0
      count++
      count++
      count++
      assert_eq(count, 3)
    end

    test("on an array element and a hash value") do
      items = [1]
      items[0]++
      counts = {"k": 1}
      counts["k"]++
      assert_eq(items, [2])
      assert_eq(counts, {"k": 2})
    end
  end

  context("--") do
    test("decrements") do
      count = 5
      count--
      assert_eq(count, 4)
    end

    test("evaluates to the old value") do
      count = 10
      previous = count--
      assert_eq(previous, 10)
      assert_eq(count, 9)
    end

    test("inside a larger expression, contributes the old value") do
      count = 5
      result = count-- + 10
      assert_eq(result, 15)
      assert_eq(count, 4)
    end

    test("goes below zero") do
      count = 0
      count--
      assert_eq(count, -1)
    end
  end

  context("mixed with compound assignment") do
    test("in sequence") do
      count = 1
      count++
      count += 10
      assert_eq(count, 12)
      count--
      count -= 5
      assert_eq(count, 6)
    end

    test("in a loop") do
      sum = 0
      i = 0
      while i < 5
        sum += i
        i++
      end
      assert_eq(sum, 10)
      assert_eq(i, 5)
    end
  end
end
