# Type annotations, part two: Void, return-type checks at runtime, function
# types, nullable types and array types. Part one is type_annotations_spec.sl.

def void_ending_in_let -> Void
  let unused = 1
end

def void_ending_in_assignment -> Void
  unused = 1
end

def void_explicit_nil -> Void
  return nil
end

class VoidCounter
  count: Int

  new()
    @count = 0
  end

  def bump -> Void
    @count = @count + 1
  end
end

def int_but_string -> Int
  "s"
end

def int_but_explicit_string -> Int
  return "x"
end

def int_but_nil -> Int
  nil
end

def float_but_int -> Float
  1
end

def optional_int -> Int?
  nil
end

describe("Void return type") do
  test("a body ending in a let statement returns nil") do
    assert_null(void_ending_in_let())
  end

  test("an explicit return nil is allowed") do
    assert_null(void_explicit_nil())
  end

  test("a body ending in an assignment returns nil") do
    pending("bug: a -> Void function whose last line is an assignment raises 'expected to return Void, got int'")
    assert_null(void_ending_in_assignment())
  end

  test("a method ending in an instance-variable assignment returns nil") do
    pending("bug: a -> Void method whose last line is `@count = @count + 1` raises 'expected to return Void, got int'")
    counter = new VoidCounter()
    assert_null(counter.bump)
    assert_eq(counter.count, 1)
  end
end

describe("Return types are checked at runtime") do
  test("an implicit return of the wrong type raises") do
    assert_raises("function 'int_but_string' expected to return Int, got string") do
      int_but_string()
    end
  end

  test("an explicit return of the wrong type raises") do
    assert_raises("expected to return Int, got string") do
      int_but_explicit_string()
    end
  end

  test("nil is not an Int") do
    assert_raises("function 'int_but_nil' expected to return Int, got null") do
      int_but_nil()
    end
  end

  test("an Int is not a Float") do
    assert_raises("expected to return Float, got int") do
      float_but_int()
    end
  end

  test("a nullable return type accepts nil") do
    assert_null(optional_int())
  end
end

describe("Function type annotations") do
  test("a two-parameter function type") do
    let adder: (Int, Int) -> Int
    adder = fn(a, b) { a + b }
    assert_eq(adder(2, 3), 5)
  end

  test("a function type with no parameters") do
    let greeter: () -> String
    greeter = fn() { "hello" }
    assert_eq(greeter(), "hello")
  end

  test("a single-parameter function type") do
    let doubler: (Int) -> Int
    doubler = fn(x) { x * 2 }
    assert_eq(doubler(5), 10)
  end

  test("a function type whose return differs from its parameters") do
    let joiner: (Int, Int) -> String
    joiner = fn(a, b) { str(a + b) }
    assert_eq(joiner(2, 3), "5")
  end

  test("declared and assigned on one line") do
    let adder: (Int, Int) -> Int = fn(a, b) { a + b }
    assert_eq(adder(2, 3), 5)
  end

  test("a lambda with typed parameters and return") do
    adder = fn(a: Int, b: Int) -> Int { a + b }
    assert_eq(adder(1, 2), 3)
  end
end

describe("Nullable types") do
  test("accept a value") do
    let count: Int? = 5
    assert_eq(count, 5)
  end

  test("accept nil, then a value") do
    let count: Int? = nil
    assert_null(count)
    count = 3
    assert_eq(count, 3)
  end
end

describe("Array type annotations") do
  test("an Int array") do
    let numbers: Int[] = [1, 2, 3]
    assert_eq(numbers, [1, 2, 3])
  end

  test("a String array") do
    let letters: String[] = ["a", "b", "c"]
    assert_eq(letters.join(""), "abc")
  end

  test("an empty typed array") do
    let empty: Int[] = []
    assert_eq(empty.length, 0)
  end
end
