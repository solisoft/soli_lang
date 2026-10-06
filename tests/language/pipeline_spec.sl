# ============================================================================
# Pipeline Operator Test Suite
# ============================================================================

describe("Pipeline Operator", fn() {
  test("basic pipeline", fn() {
    def double(x)
      return x * 2
    end
    def addOne(x)
      return x + 1
    end

    let result = 5 |> double()
    assert_eq(result, 10)
  })

  test("chained pipeline", fn() {
    def double(x)
      return x * 2
    end
    def addTen(x)
      return x + 10
    end

    let result = 5 |> double() |> addTen()
    assert_eq(result, 20)
  })

  test("pipeline with method call", fn() {
    let result = "hello" |> fn(s) { s.upcase() }
    assert_eq(result, "HELLO")
  })

  test("pipeline with custom function", fn() {
    def square(x)
      return x * x
    end
    def toString(x)
      return str(x)
    end

    let result = 5 |> square() |> toString()
    assert_eq(result, "25")
  })

  test("pipeline with multiple transformations", fn() {
    def add(x, n)
      return x + n
    end
    def multiply(x, n)
      return x * n
    end

    let result = 2 |> add(3) |> multiply(4)
    assert_eq(result, 20)
  })

  test("pipeline preserves types", fn() {
    def get_len(s)
      return len(s)
    end

    let result = "hello" |> get_len()
    assert_eq(result, 5)
    assert_eq(type(result), "int")
  })
})
