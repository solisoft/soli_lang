# Auto-invoke: a zero-argument method is called by naming it, Ruby-style —
# `arr.length`, `str.upcase`, `dog.bark` — while fields and lambda-valued
# fields are read, not called. The parenthesized form keeps working.

class AutoDog
  name: String

  new(name: String)
    @name = name
  end

  def bark
    "Woof!"
  end

  def greet(person)
    "Hello #{person}, I'm #{@name}"
  end
end

class AutoBox
  value: Int

  new(value: Int)
    @value = value
  end
end

class ActionBox
  action: Function

  new(action: Function)
    @action = action
  end
end

class AutoGreeter
  def hello(name = "World")
    "Hello #{name}!"
  end
end

describe("Auto-invoke on Array methods") do
  test("length, with or without parentheses") do
    numbers = [1, 2, 3]
    assert_eq(numbers.length, 3)
    assert_eq(numbers.length(), 3)
  end

  test("first and last") do
    numbers = [10, 20, 30]
    assert_eq(numbers.first, 10)
    assert_eq(numbers.last, 30)
  end

  test("first and last of an empty array are nil") do
    assert_null([].first)
    assert_null([].last)
  end

  test("empty?") do
    assert_eq([].empty?, true)
    assert_eq([1].empty?, false)
  end

  test("reverse, sort and uniq") do
    assert_eq([1, 2, 3].reverse, [3, 2, 1])
    assert_eq([3, 1, 2].sort, [1, 2, 3])
    assert_eq([1, 2, 2, 3, 3].uniq, [1, 2, 3])
  end

  test("compact drops nils") do
    assert_eq([1, nil, 2, nil, 3].compact, [1, 2, 3])
  end

  test("flatten flattens every level") do
    assert_eq([[1, 2], [3, 4]].flatten, [1, 2, 3, 4])
    assert_eq([1, [2, [3]]].flatten, [1, 2, 3])
  end

  test("sum, min and max") do
    assert_eq([1, 2, 3, 4].sum, 10)
    assert_eq([3, 1, 2].min, 1)
    assert_eq([3, 1, 2].max, 3)
  end

  test("to_string") do
    assert_eq([1, 2, 3].to_string, "[1, 2, 3]")
  end

  test("join with its separator omitted") do
    pending("bug: arr.join without parens returns a bound Method instead of calling join()")
    assert_eq(["a", "b"].join, "ab")
  end

  test("a method with arguments takes them in parentheses") do
    numbers = [1, 2, 3]
    assert_eq(numbers.map { |x| x * 2 }, [2, 4, 6])
    assert_eq(numbers.includes?(2), true)
    assert_eq(numbers.includes?(4), false)
  end
end

describe("Auto-invoke on String methods") do
  test("length") do
    assert_eq("hello".length, 5)
    assert_eq("".length, 0)
  end

  test("upcase, downcase and capitalize") do
    assert_eq("hello".upcase, "HELLO")
    assert_eq("hello".upcase(), "HELLO")
    assert_eq("HELLO".downcase, "hello")
    assert_eq("hello".capitalize, "Hello")
  end

  test("trim and reverse") do
    assert_eq("  hello  ".trim, "hello")
    assert_eq("hello".reverse, "olleh")
  end

  test("empty?") do
    assert_eq("".empty?, true)
    assert_eq("hello".empty?, false)
  end

  test("chars and bytes") do
    assert_eq("abc".chars, ["a", "b", "c"])
    assert_eq("abc".bytes, [97, 98, 99])
  end

  test("split with its separator omitted splits on whitespace") do
    assert_eq("hello world".split, ["hello", "world"])
  end

  test("a method with arguments takes them in parentheses") do
    assert_eq("hello world".split(" "), ["hello", "world"])
  end
end

describe("Auto-invoke on number methods") do
  test("round with its precision omitted") do
    assert_eq(3.7.round, 4)
    assert_eq(3.14159.round(2), 3.14)
  end

  test("to_s") do
    assert_eq(5.to_s, "5")
    assert_eq(1.5.to_s, "1.5")
  end
end

describe("Auto-invoke on Hash methods") do
  test("length") do
    assert_eq({"a": 1, "b": 2, "c": 3}.length, 3)
  end

  test("keys and values") do
    assert_eq({"a": 1, "b": 2}.keys, ["a", "b"])
    assert_eq({"a": 1, "b": 2}.values, [1, 2])
  end

  test("empty?") do
    assert_eq({}.empty?, true)
    assert_eq({"a": 1}.empty?, false)
  end

  test("to_string") do
    assert_eq({"a": 1}.to_string, "{a => 1}")
  end

  test("compact drops nil values") do
    assert_eq({"a": 1, "b": nil, "c": 3}.compact, {"a": 1, "c": 3})
  end

  test("a dot name that is a key reads the key") do
    config = {"name": "Soli", "version": 1}
    assert_eq(config.name, "Soli")
    assert_eq(config.version, 1)
  end
end

describe("Auto-invoke on user-defined methods") do
  test("a zero-argument method is called by naming it") do
    dog = new AutoDog("Rex")
    assert_eq(dog.bark, "Woof!")
  end

  test("a zero-argument method still accepts parentheses") do
    dog = new AutoDog("Rex")
    assert_eq(dog.bark(), "Woof!")
  end

  test("a method whose parameters all have defaults is called by naming it") do
    greeter = new AutoGreeter()
    assert_eq(greeter.hello, "Hello World!")
    assert_eq(greeter.hello("Ann"), "Hello Ann!")
  end

  test("a method with required arguments takes them in parentheses") do
    dog = new AutoDog("Rex")
    assert_eq(dog.greet("Alice"), "Hello Alice, I'm Rex")
  end

  test("a field is read, not invoked") do
    box = new AutoBox(42)
    assert_eq(box.value, 42)
  end

  test("a lambda-valued field with parameters is read, not invoked") do
    box = new ActionBox(fn(x) { x * 2 })
    action = box.action
    assert_eq(type(action), "Function")
    assert_eq(action(4), 8)
  end

  test("a zero-parameter lambda field is read, not invoked") do
    pending("bug: `action = box.action` calls a zero-parameter lambda field; only type(box.action) sees the Function")
    box = new ActionBox(fn() { "called" })
    assert_eq(type(box.action), "Function")
    action = box.action
    assert_eq(type(action), "Function")
  end
end

describe("Method chaining with auto-invoke") do
  test("on arrays") do
    assert_eq([3, 1, 2].sort.first, 1)
    assert_eq([3, 1, 2].sort.last, 3)
    assert_eq([3, 1, 2].sort.reverse, [3, 2, 1])
    assert_eq([1, 2, 3].reverse.first, 3)
  end

  test("on strings") do
    assert_eq("hello".upcase.reverse, "OLLEH")
    assert_eq("  hello  ".trim.upcase, "HELLO")
  end

  test("on hashes") do
    assert_eq({"a": 1, "b": 2, "c": 3}.keys.length, 3)
    assert_eq({"a": 3, "b": 1, "c": 2}.values.sort, [1, 2, 3])
  end

  test("an auto-invoked call followed by a call with a block") do
    assert_eq([3, 1, 2].sort.map { |x| x * 10 }, [10, 20, 30])
  end
end

describe("Safe navigation with auto-invoke") do
  test("nil&.method returns nil") do
    missing = nil
    assert_null(missing&.length)
  end

  test("a nil in the middle of a chain short-circuits the rest") do
    missing = nil
    assert_null(missing&.upcase&.length)
  end

  test("a non-nil receiver auto-invokes") do
    numbers = [1, 2, 3]
    assert_eq(numbers&.length, 3)
  end

  test("a chain of safe calls") do
    numbers = [3, 1, 2]
    assert_eq(numbers&.sort&.first, 1)
  end
end
