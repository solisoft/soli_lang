# The pipeline operator: `x |> f(args)` calls `f(x, args)`, left to right.
# Do not run `soli fmt` here: it rewrites the `|x| { … }` lambda under test to
# `fn(x) { … }`.

def double(x)
  x * 2
end

def add_ten(x)
  x + 10
end

def square(x)
  x * x
end

def add(x, amount)
  x + amount
end

def subtract(x, amount)
  x - amount
end

def multiply(x, factor)
  x * factor
end

def to_text(x)
  str(x)
end

def text_length(text)
  text.length
end

describe("Pipeline operator") do
  context("into a function") do
    test("passes the left value as the only argument") do
      assert_eq(5 |> double(), 10)
    end

    test("works without parentheses on the function name") do
      assert_eq(5 |> double, 10)
    end

    test("chains left to right") do
      assert_eq(5 |> double() |> add_ten(), 20)
      assert_eq(5 |> add_ten() |> double(), 30)
    end

    test("passes the left value as the first argument before the others") do
      assert_eq(10 |> subtract(3), 7)
      assert_eq(2 |> add(3) |> multiply(4), 20)
    end

    test("carries the type each step returns") do
      result = 5 |> square() |> to_text()
      assert_eq(result, "25")
      assert_eq(type(result), "string")
      assert_eq("hello" |> text_length(), 5)
    end
  end

  context("into a lambda") do
    test("an inline fn receives the value") do
      assert_eq("hello" |> fn(text) { text.upcase }, "HELLO")
    end

    test("an inline pipe lambda can be called with ()") do
      assert_eq(100 |> |x| { subtract(x, 10) }(), 90)
    end

    test("a lambda stored in a variable, with or without ()") do
      shout = fn(text) { text.upcase + "!" }
      assert_eq("hi" |> shout(), "HI!")
      assert_eq("hi" |> shout, "HI!")
    end

    test("nil flows through like any other value") do
      assert(nil |> fn(value) { value.nil? })
    end
  end

  context("precedence") do
    test("binds looser than arithmetic on its left") do
      assert_eq(1 + 2 |> double(), 6)
    end

    test("binds tighter than comparison on its right") do
      assert(5 |> double() == 10)
    end
  end

  context("errors") do
    test("piping into a value that is not callable raises") do
      # The two engines word it differently.
      message = assert_raises() do
        5 |> 3
      end
      assert_match(message, "non-function value|must be a function")
    end
  end
end
