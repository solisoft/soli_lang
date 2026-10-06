# Anonymous functions: `fn(…) { … }`, `|…| { … }` and stabby `->(…) { … }`,
# closures over locals and over `this`, and lambdas as values.

def apply(callback, x)
  callback(x)
end

class ClosureOuter
  value: Int = 100
  base: Int = 10

  def get_closure
    return fn(x) { this.value + x }
  end

  def get_closure_via_at
    return fn(x) { @value + x }
  end

  def create_nested
    return fn(y) {
      return fn(z) { this.base + y + z }
    }
  end
end

class ClosureMultiplier
  factor: Int = 10

  def multiplier
    return fn(x) { x * this.factor }
  end
end

class ClosureProcessor
  multiplier: Int = 2

  def process(items: Array)
    items.map { |x| x * @multiplier }
  end

  def process_with_fn(items: Array)
    items.map(fn(x) { x * this.multiplier })
  end
end

describe("Anonymous functions") do
  context("fn syntax") do
    test("fn(a, b) { … }") do
      add = fn(a, b) { a + b }
      assert_eq(add(2, 3), 5)
    end

    test("several parameters") do
      sum = fn(a, b, c) { a + b + c }
      assert_eq(sum(1, 2, 3), 6)
    end

    test("no parameters") do
      get_five = fn() { 5 }
      assert_eq(get_five(), 5)
    end

    test("typed parameters") do
      add = fn(a: Int, b: Int) { a + b }
      assert_eq(add(10, 20), 30)
    end

    test("a typed return") do
      square = fn(x: Int) -> Int { x * x }
      assert_eq(square(7), 49)
    end

    test("a multi-line body returns its last expression") do
      add = fn(a, b) {
        sum = a + b
        sum * 10
      }
      assert_eq(add(2, 3), 50)
    end

    test("immediately invoked") do
      assert_eq((fn(x) { x + 1 })(5), 6)
    end
  end

  context("pipe syntax") do
    test("|x| { … }") do
      double = |x| { x * 2 }
      assert_eq(double(5), 10)
    end

    test("several parameters") do
      add = |a, b| { a + b }
      assert_eq(add(1, 2), 3)
    end

    test("no parameters with ||") do
      seven = || { 7 }
      assert_eq(seven(), 7)
    end
  end

  context("stabby syntax") do
    test("->(x) { … }") do
      double = ->(x) { x * 2 }
      assert_eq(double(3), 6)
    end

    test("several parameters") do
      multiply = ->(x, y) { x * y }
      assert_eq(multiply(4, 5), 20)
    end

    test("no parameters") do
      nine = -> { 9 }
      assert_eq(nine(), 9)
    end

    test("typed parameter and return") do
      increment = ->(a: Int) -> Int { a + 1 }
      assert_eq(increment(1), 2)
    end
  end

  context("arity") do
    test("too few arguments raise") do
      add = |a, b| { a + b }
      assert_raises("Wrong number of arguments: expected 2, got 1") do
        add(1)
      end
    end

    test("too many arguments raise") do
      add = |a, b| { a + b }
      assert_raises("Wrong number of arguments: expected 2, got 3") do
        add(1, 2, 3)
      end
    end
  end
end

describe("return in a lambda") do
  test("exits the lambda early") do
    sign = fn(x) {
      return "neg" if x < 0

      "pos"
    }
    assert_eq(sign(-1), "neg")
    assert_eq(sign(1), "pos")
  end
end

describe("Lambdas as values") do
  test("every syntax builds a Function") do
    assert_eq(type(fn(x) { x }), "Function")
    assert_eq(type(|x| { x }), "Function")
    assert_eq(type(->(x) { x }), "Function")
  end

  test("passed as a callback, in each syntax") do
    assert_eq(apply(fn(x) { x * x }, 4), 16)
    assert_eq(apply(|x| { x * 3 }, 2), 6)
    assert_eq(apply(->(x) { x - 1 }, 2), 1)
  end

  test("stored in an array") do
    operations = [fn(x) { x + 1 }, fn(x) { x * 2 }]
    assert_eq(operations[0](5), 6)
    assert_eq(operations[1](5), 10)
  end

  test("stored in a hash") do
    functions = {"double": fn(x) { x * 2 }, "triple": fn(x) { x * 3 }}
    assert_eq(functions["double"](5), 10)
    assert_eq(functions["triple"](5), 15)
  end

  test("composed by nesting calls") do
    double = fn(x) { x * 2 }
    add_one = fn(x) { x + 1 }
    assert_eq(double(add_one(5)), 12)
    assert_eq(add_one(double(5)), 11)
  end

  test("works on strings") do
    greet = fn(name) { "Hello, " + name }
    assert_eq(greet("World"), "Hello, World")
  end
end

describe("Closures") do
  test("capture a local and see later writes to it") do
    counter = 0
    increment = fn() { counter = counter + 1 }
    increment()
    increment()
    assert_eq(counter, 2)
  end

  test("a factory returns a lambda that keeps its argument") do
    make_adder = fn(n) { return fn(x) { x + n } }
    add_five = make_adder(5)
    add_ten = make_adder(10)
    assert_eq(add_five(1), 6)
    assert_eq(add_ten(1), 11)
  end
end

describe("Lambdas and this") do
  test("a lambda returned from a method captures this") do
    closure = new ClosureOuter().get_closure
    assert_eq(closure(5), 105)
  end

  test("@field inside a returned lambda reads the captured instance") do
    closure = new ClosureOuter().get_closure_via_at
    assert_eq(closure(5), 105)
  end

  test("a lambda captures this from another class") do
    closure = new ClosureMultiplier().multiplier
    assert_eq(closure(5), 50)
  end

  test("nested lambdas all see this") do
    inner = new ClosureOuter().create_nested()(5)
    assert_eq(inner(3), 18)
  end

  test("a block inside a method sees @field") do
    assert_eq(new ClosureProcessor().process([1, 2, 3]), [2, 4, 6])
  end

  test("an fn callback inside a method sees this") do
    assert_eq(new ClosureProcessor().process_with_fn([1, 2, 3]), [2, 4, 6])
  end
end
