# Functions declared with `def`: parameters (typed, default, named), return
# values (explicit and implicit), recursion, first-class use, closures, and
# `#` comments around them.

def add(a, b)
  return a + b
end

def greet
  return "hello"
end

def greet_with_parens()
  "hello"
end

def five
  5
end

def multiply(a: Int, b: Int)
  a * b
end

def square(x: Int) -> Int
  x * x
end

def greet_name(name: String = "World")
  "Hello " + name
end

def configure(host: String = "localhost", port: Int = 8080, debug: Bool = false)
  host + ":" + str(port) + " (debug: " + str(debug) + ")"
end

def sum_with_defaults(a: Int, b: Int = 10, c: Int = 20)
  a + b + c
end

def ends_in_let
  let unused = 1
end

def ends_in_if_without_else(x)
  if x > 0
    "positive"
  end
end

def sign_of(x)
  return "negative" if x < 0

  return "non-negative"
end

def factorial(n)
  return 1 if n <= 1

  n * factorial(n - 1)
end

def fib(n)
  return n if n <= 1

  fib(n - 1) + fib(n - 2)
end

def double(x)
  x * 2
end

def triple(x)
  x * 3
end

def call_double(x)
  double(x)
end

def apply(callback, x)
  callback(x)
end

def absolute(x)
  if x < 0
    -x
  else
    x
  end
end

def with_local_helper
  def local_helper(x)
    x * 2
  end

  local_helper(4)
end

def reassign_param(list)
  list.push(9)
  list = [0]
  list
end

def make_adder(n)
  return fn(x) { x + n }
end

def make_counter
  count = 0
  return fn() {
    count = count + 1
    count
  }
end

class Calculator
  def add(a, b)
    return a + b
  end

  def multiply(a, b)
    a * b
  end

  def hello
    "hi"
  end
end

describe("Function declarations") do
  test("with parameters, called with arguments") do
    assert_eq(add(2, 3), 5)
  end

  test("without parentheses when there are no parameters") do
    assert_eq(greet(), "hello")
    assert_eq(five(), 5)
  end

  test("with empty parentheses") do
    assert_eq(greet_with_parens(), "hello")
  end

  test("with typed parameters") do
    assert_eq(multiply(3, 4), 12)
  end

  test("with a return type") do
    assert_eq(square(5), 25)
    assert_eq(square(-7), 49)
  end

  test("a def inside a function body is local to it") do
    assert_eq(with_local_helper(), 8)
    assert_raises("local_helper") do
      local_helper(1)
    end
  end
end

describe("Arguments") do
  test("a default parameter fills a missing argument") do
    assert_eq(greet_name(), "Hello World")
    assert_eq(greet_name("Alice"), "Hello Alice")
  end

  test("named arguments pick which defaults to override, in any order") do
    assert_eq(configure(), "localhost:8080 (debug: false)")
    assert_eq(configure(port: 3000), "localhost:3000 (debug: false)")
    assert_eq(configure(host: "example.com", port: 443), "example.com:443 (debug: false)")
    assert_eq(configure(port: 9000, debug: true, host: "api.example.com"), "api.example.com:9000 (debug: true)")
  end

  test("positional and named arguments mix") do
    assert_eq(sum_with_defaults(1), 31)
    assert_eq(sum_with_defaults(1, 2), 23)
    assert_eq(sum_with_defaults(1, c: 5), 16)
  end

  test("too few arguments raise") do
    assert_raises("Wrong number of arguments: expected 2, got 1") do
      add(1)
    end
  end

  test("too many arguments raise") do
    assert_raises("Wrong number of arguments: expected 2, got 3") do
      add(1, 2, 3)
    end
  end

  test("a mutated argument is shared, a reassigned parameter is not") do
    original = [1]
    assert_eq(reassign_param(original), [0])
    assert_eq(original, [1, 9])
  end
end

describe("Return values") do
  test("return with a postfix condition exits early") do
    assert_eq(sign_of(-5), "negative")
    assert_eq(sign_of(5), "non-negative")
  end

  test("the last expression is returned implicitly") do
    assert_eq(five(), 5)
    assert_eq(call_double(5), 10)
  end

  test("an if/else as the last expression returns its taken branch") do
    assert_eq(absolute(-5), 5)
    assert_eq(absolute(5), 5)
  end

  test("an if without else returns nil when not taken") do
    assert_eq(ends_in_if_without_else(1), "positive")
    assert_null(ends_in_if_without_else(-1))
  end

  test("a let as the last statement returns nil") do
    assert_null(ends_in_let())
  end

  test("a block's last expression is its value") do
    assert_eq([1, 2, 3].map { |x| x * 2 }, [2, 4, 6])
  end
end

describe("Recursion") do
  test("factorial") do
    assert_eq(factorial(5), 120)
    assert_eq(factorial(1), 1)
    assert_eq(factorial(0), 1)
  end

  test("fibonacci with two recursive calls") do
    assert_eq(fib(10), 55)
    assert_eq(fib(0), 0)
  end
end

describe("Functions as values") do
  test("a function can be assigned to a variable and called") do
    doubler = double
    assert_eq(doubler(5), 10)
  end

  test("a function can be passed to another function") do
    assert_eq(apply(triple, 4), 12)
    assert_eq(apply(double, 4), 8)
  end
end

describe("Methods") do
  test("explicit and implicit returns") do
    calc = new Calculator()
    assert_eq(calc.add(2, 3), 5)
    assert_eq(calc.multiply(4, 5), 20)
  end

  test("a parameterless method is called with or without parentheses") do
    calc = new Calculator()
    assert_eq(calc.hello, "hi")
    assert_eq(calc.hello(), "hi")
  end
end

describe("Closures") do
  test("capture an outer variable") do
    multiplier = 3
    multiply_by = fn(x) { x * multiplier }
    assert_eq(multiply_by(5), 15)
  end

  test("capture several variables") do
    first = 10
    second = 20
    total = fn() { first + second }
    assert_eq(total(), 30)
  end

  test("a factory keeps each call's argument") do
    add5 = make_adder(5)
    add10 = make_adder(10)
    assert_eq(add5(3), 8)
    assert_eq(add10(3), 13)
  end

  test("each closure from a factory keeps separate state") do
    counter1 = make_counter()
    counter2 = make_counter()
    assert_eq(counter1(), 1)
    assert_eq(counter1(), 2)
    assert_eq(counter2(), 1)
  end
end

describe("Hash comments (#)") do
  test("a comment on its own line") do
    # This is a comment
    answer = 42
    assert_eq(answer, 42)
  end

  test("a comment at the end of a line does not affect the next") do
    first = 1  # comment
    second = 2
    assert_eq(first + second, 3)
  end

  test("a comment after an interpolated string") do
    name = "Soli"
    message = "Hello #{name}!"  # interpolation inside string
    assert_eq(message, "Hello Soli!")
  end

  test("a # inside a string is not a comment") do
    text = "use # for comments"
    assert_eq(text, "use # for comments")
    assert_eq(text.length, 18)
  end

  test("consecutive comment lines") do
    # first comment
    # second comment
    answer = 99
    assert_eq(answer, 99)
  end
end
