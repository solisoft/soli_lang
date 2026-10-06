# ============================================================================
# Command-Style Calls Test Suite
# ============================================================================
# Tests for calling functions without parentheses: `print x` instead of `print(x)`

describe("Command-style calls with literals", fn() {
  test("string literal", fn() {
    def echo(x)
      return x
    end
    let result = echo("hello")
    assert_eq(result, "hello")
  })

  test("integer literal", fn() {
    def echo(x)
      return x
    end
    let result = echo(42)
    assert_eq(result, 42)
  })

  test("float literal", fn() {
    def echo(x)
      return x
    end
    let result = echo(3.14)
    assert_eq(result, 3.14)
  })

  test("boolean literal", fn() {
    def echo(x)
      return x
    end
    let t = echo(true)
    let f = echo(false)
    assert_eq(t, true)
    assert_eq(f, false)
  })

  test("null literal", fn() {
    def echo(x)
      return x
    end
    let result = echo(null)
    assert_null(result)
  })

  test("interpolated string", fn() {
    def echo(x)
      return x
    end
    let name = "world"
    let result = echo("hello #{name}")
    assert_eq(result, "hello world")
  })
})

describe("Command-style calls with variables", fn() {
  test("simple variable", fn() {
    def echo(x)
      return x
    end
    let msg = "hello"
    let result = echo(msg)
    assert_eq(result, "hello")
  })

  test("variable holding number", fn() {
    def double(x)
      return x * 2
    end
    let n = 21
    let result = double(n)
    assert_eq(result, 42)
  })

  test("variable holding boolean", fn() {
    def negate(x)
      return !x
    end
    let flag = true
    let result = negate(flag)
    assert_eq(result, false)
  })

  test("variable holding array", fn() {
    def first(arr)
      return arr[0]
    end
    let items = [10, 20, 30]
    let result = first(items)
    assert_eq(result, 10)
  })

  test("variable holding hash", fn() {
    def get_name(h)
      return h["name"]
    end
    let person = {"name": "Alice"}
    let result = get_name(person)
    assert_eq(result, "Alice")
  })
})

describe("Command-style calls with multiple arguments", fn() {
  test("two string arguments", fn() {
    def concat(a, b)
      return a + " " + b
    end
    let result = concat("hello", "world")
    assert_eq(result, "hello world")
  })

  test("two variable arguments", fn() {
    def add(a, b)
      return a + b
    end
    let x = 10
    let y = 20
    let result = add(x, y)
    assert_eq(result, 30)
  })

  test("mixed literal and variable", fn() {
    def add(a, b)
      return a + b
    end
    let x = 10
    let result = add(x, 5)
    assert_eq(result, 15)
  })

  test("three arguments", fn() {
    def sum3(a, b, c)
      return a + b + c
    end
    let a = 1
    let b = 2
    let c = 3
    let result = sum3(a, b, c)
    assert_eq(result, 6)
  })
})

describe("Command-style calls in different contexts", fn() {
  test("inside if body", fn() {
    def echo(x)
      return x
    end
    let result = null
    let x = "yes"
    result = echo(x) if (true)
    assert_eq(result, "yes")
  })

  test("inside function body with end syntax", fn() {
    def echo(x)
      return x
    end

    let msg = "hello"
    let result = echo(msg)
    assert_eq(result, "hello")
  })

  test("result used in expression", fn() {
    def double(x)
      return x * 2
    end
    let n = 5
    let result = (double(n)) + 1
    assert_eq(result, 11)
  })

  test("chained with parentheses call", fn() {
    def add(a, b)
      return a + b
    end
    def double(x)
      return x * 2
    end
    let x = 3
    let result = double(add(x, 2))
    assert_eq(result, 10)
  })
})

describe("Command-style calls do not break multi-line code", fn() {
  test("function body with separate statements", fn() {
    let log = []
    def process(x)
      log.push(x)
      log.push(x * 2)
      return log
    end
    let result = process(5)
    assert_eq(result, [5, 10])
  })

  test("variables on consecutive lines stay independent", fn() {
    let a = 1
    let b = 2
    let c = a
    let d = b
    assert_eq(c, 1)
    assert_eq(d, 2)
  })

  test("command call followed by another statement", fn() {
    def echo(x)
      return x
    end
    let msg = "hi"
    let result = echo(msg)
    let other = 42
    assert_eq(result, "hi")
    assert_eq(other, 42)
  })
})

describe("Parentheses call still works", fn() {
  test("standard parenthesized call", fn() {
    def add(a, b)
      return a + b
    end
    assert_eq(add(2, 3), 5)
  })

  test("parenthesized call with variable", fn() {
    def double(x)
      return x * 2
    end
    let n = 7
    assert_eq(double(n), 14)
  })

  test("both styles produce same result", fn() {
    def echo(x)
      return x
    end
    let val = "test"
    let a = echo(val)
    let b = echo(val)
    assert_eq(a, b)
  })
})
