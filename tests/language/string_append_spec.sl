# In-place growth of strings (`+=`, `<<`) and arrays mutated while a block
# iterates them.

describe("in-place string append") do
  test("+= grows a string") do
    buffer = ""
    buffer += "ab"
    buffer += "c"
    assert_eq(buffer, "abc")
  end

  test("+= with an empty string leaves it unchanged") do
    buffer = ""
    buffer += ""
    assert_eq(buffer, "")
  end

  test("+= with an Int appends its text") do
    buffer = "x"
    buffer += 1
    assert_eq(buffer, "x1")
  end

  test("assigning a string plus a string grows the same binding") do
    buffer = "a"
    buffer = buffer + "b"
    buffer = buffer + "c"
    assert_eq(buffer, "abc")
  end

  test("<< appends to a string variable and returns it") do
    buffer = "hi"
    returned = buffer << "!"
    assert_eq(buffer, "hi!")
    assert_eq(returned, "hi!")
  end

  test("<< refuses a non-string right side and leaves the string intact") do
    buffer = "a"
    assert_raises("string << expects a string, got int") do
      buffer << 1
    end
    assert_eq(buffer, "a")
  end

  test("<< refuses a number on the left") do
    count = 5
    assert_raises("<< expects an array on the left, got int") do
      count << "x"
    end
  end

  test("appending does not change another variable that already aliased it") do
    original = "ab"
    alias = original
    original = original + "c"
    assert_eq(original, "abc")
    assert_eq(alias, "ab")
  end

  test("<< still pushes onto an array") do
    items = [1]
    items << 2
    assert_eq(items, [1, 2])
  end
end

describe("mutating an array while iterating it") do
  test("each stops when the block shrinks the array") do
    items = [1, 2, 3]
    visited = []
    items.each do |x|
      visited.push(x)
      items.pop
    end
    assert_eq(items, [1])
    assert_eq(visited, [1, 2])
  end

  test("map keeps the elements visited before the array shrinks past the index") do
    items = [1, 2, 3]
    mapped = items.map do |x|
      items.pop
      x
    end
    assert_eq(mapped, [1, 2])
    assert_eq(items, [1])
  end

  test("reduce adds only the elements visited") do
    items = [1, 2, 3]
    total = items.reduce(fn(acc, x) {
      items.pop
      acc + x
    }, 0)
    assert_eq(total, 3)
    assert_eq(items, [1])
  end
end
