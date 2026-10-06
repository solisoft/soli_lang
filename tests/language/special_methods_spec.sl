# Predicate methods (`?` suffix), the pipeline operator `|>` and the ternary
# `? :` — three uses of `?`/`|` the lexer must keep apart.

def double(x)
  x * 2
end

def add(x, y)
  x + y
end

class Account
  balance: Int

  new(balance: Int)
    @balance = balance
  end

  def overdrawn?
    @balance < 0
  end
end

describe("Predicate methods with ?") do
  test("empty? on an array") do
    assert([].empty?)
    assert_not([1, 2, 3].empty?)
  end

  test("empty? on a string") do
    assert("".empty?)
    assert_not("hello".empty?)
    assert_not(" ".empty?)
  end

  test("includes? on an array") do
    numbers = [1, 2, 3]
    assert(numbers.includes?(2))
    assert_not(numbers.includes?(5))
    assert([1, nil].includes?(nil))
  end

  test("includes? on a string") do
    text = "hello world"
    assert(text.includes?("world"))
    assert_not(text.includes?("xyz"))
  end

  test("starts_with?") do
    text = "hello world"
    assert(text.starts_with?("hello"))
    assert_not(text.starts_with?("world"))
    assert(text.starts_with?(""))
  end

  test("ends_with?") do
    text = "hello world"
    assert(text.ends_with?("world"))
    assert_not(text.ends_with?("hello"))
  end

  test("a predicate drives a condition") do
    items = [1, 2, 3]
    label = items.empty? ? "none" : "some"
    assert_eq(label, "some")
    label = "none" if [].empty?
    assert_eq(label, "none")
  end

  test("chained predicate checks") do
    text = "hello"
    assert(text.includes?("ell") && text.starts_with?("he"))
    assert_not(text.includes?("xyz") || text.ends_with?("xyz"))
  end

  test("a user-defined method may end in ?") do
    assert(new Account(-5).overdrawn?)
    assert_not(new Account(10).overdrawn?)
  end
end

describe("Pipeline operator") do
  test("feeds a value through lambdas") do
    assert_eq(5 |> fn(x) { x * 2 } |> fn(x) { x + 1 }, 11)
  end

  test("a lambda can call a method on the piped value") do
    assert_eq("hello" |> fn(text) { text.upcase }, "HELLO")
    assert_eq("hello" |> fn(text) { text.length }, 5)
  end

  test("chains several collection steps") do
    result = [1, 2, 3, 4, 5]
      |> fn(numbers) { numbers.filter { |x| x > 2 } }
      |> fn(numbers) { numbers.map { |x| x * 2 } }
    assert_eq(result, [6, 8, 10])
  end

  test("feeds a named function, as its first argument") do
    assert_eq(5 |> double(), 10)
    assert_eq(5 |> double() |> add(1), 11)
  end

  test("preserves the value's type") do
    result = 10 |> fn(x) { x + 5 }
    assert_eq(result, 15)
    assert_eq(type(result), "int")
  end

  test("string operations") do
    result = "  hello world  " |> fn(text) { text.trim } |> fn(text) { text.upcase }
    assert_eq(result, "HELLO WORLD")
  end
end

describe("Ternary operator") do
  test("true branch") do
    assert_eq(true ? "yes" : "no", "yes")
  end

  test("false branch") do
    assert_eq(false ? "yes" : "no", "no")
  end

  test("nil and 0 take the false branch") do
    assert_eq(nil ? "yes" : "no", "no")
    assert_eq(0 ? "yes" : "no", "no")
  end

  test("in an expression") do
    count = 5
    assert_eq(count > 0 ? "positive" : "non-positive", "positive")
  end

  test("nested in the else branch") do
    count = 0
    assert_eq(count > 0 ? "positive" : count < 0 ? "negative" : "zero", "zero")
  end

  test("nested in the then branch") do
    assert_eq(true ? false ? 1 : 2 : 3, 2)
  end
end
