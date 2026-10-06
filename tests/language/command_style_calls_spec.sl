# Command-style calls: calling a function without parentheses, `echo x`
# instead of `echo(x)`. The syntax under test is the paren-less call, so do
# not run `soli fmt` on this file — it adds the parentheses back.

def echo(x)
  x
end

def double(x)
  x * 2
end

def negate(x)
  !x
end

def first_of(list)
  list[0]
end

def get_name(person)
  person["name"]
end

def concat(a, b)
  a + " " + b
end

def add(a, b)
  a + b
end

def sum3(a, b, c)
  a + b + c
end

def record_twice(log, x)
  log.push(x)
  log.push(x * 2)
  log
end

describe("Command-style calls with literals") do
  test("a string") do
    result = echo "hello"
    assert_eq(result, "hello")
  end

  test("an integer") do
    result = echo 42
    assert_eq(result, 42)
  end

  test("a float") do
    result = echo 3.14
    assert_eq(result, 3.14)
  end

  test("booleans") do
    yes = echo true
    no = echo false
    assert_eq(yes, true)
    assert_eq(no, false)
  end

  test("nil") do
    result = echo nil
    assert_null(result)
  end

  test("an interpolated string") do
    name = "world"
    result = echo "hello #{name}"
    assert_eq(result, "hello world")
  end

  test("a symbol") do
    result = echo :sym
    assert_eq(result, :sym)
  end
end

describe("Command-style calls with variables") do
  test("a string variable") do
    message = "hello"
    result = echo message
    assert_eq(result, "hello")
  end

  test("a number variable") do
    n = 21
    result = double n
    assert_eq(result, 42)
  end

  test("a boolean variable") do
    flag = true
    result = negate flag
    assert_eq(result, false)
  end

  test("an array variable") do
    items = [10, 20, 30]
    result = first_of items
    assert_eq(result, 10)
  end

  test("a hash variable") do
    person = {"name": "Alice"}
    result = get_name person
    assert_eq(result, "Alice")
  end
end

describe("Command-style calls with several arguments") do
  test("two string literals") do
    result = concat "hello", "world"
    assert_eq(result, "hello world")
  end

  test("two variables") do
    x = 10
    y = 20
    result = add x, y
    assert_eq(result, 30)
  end

  test("a variable and a literal") do
    x = 10
    result = add x, 5
    assert_eq(result, 15)
  end

  test("three arguments") do
    result = sum3 1, 2, 3
    assert_eq(result, 6)
  end
end

describe("Command-style calls in expressions") do
  test("as the body of a postfix if") do
    result = nil
    result = echo "yes" if true
    assert_eq(result, "yes")
  end

  test("not run when the postfix if is false") do
    result = "unchanged"
    result = echo "yes" if false
    assert_eq(result, "unchanged")
  end

  test("wrapped in parentheses inside a larger expression") do
    n = 5
    result = (double n) + 1
    assert_eq(result, 11)
  end

  test("as the argument of a parenthesized call") do
    x = 3
    result = double(add x, 2)
    assert_eq(result, 10)
  end

  test("the whole operator expression after the name is the argument") do
    x = 10
    result = double x + 1
    assert_eq(result, 22)
  end

  test("nested command-style calls apply right to left") do
    result = echo double 4
    assert_eq(result, 8)
  end
end

describe("Command-style calls do not swallow the next line") do
  test("a call followed by another statement") do
    message = "hi"
    result = echo message
    other = 42
    assert_eq(result, "hi")
    assert_eq(other, 42)
  end

  test("variables on consecutive lines stay independent") do
    first = 1
    second = 2
    copy_of_first = first
    copy_of_second = second
    assert_eq(copy_of_first, 1)
    assert_eq(copy_of_second, 2)
  end

  test("a function whose body has several statements") do
    log = []
    result = record_twice log, 5
    assert_eq(result, [5, 10])
  end
end

describe("Command-style and parenthesized calls") do
  test("give the same result") do
    value = "test"
    bare = echo value
    assert_eq(bare, echo(value))
    total = add 2, 3
    assert_eq(total, add(2, 3))
  end
end

describe("Command-style method calls with positional arguments") do
  test("a method with one positional argument") do
    pending("bug: `log.push 5` parses as `log.push` then a separate `5`, silently pushing nothing")
    log = []
    log.push 5
    assert_eq(log, [5])
  end
end
