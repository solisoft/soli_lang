# ============================================================================
# Scope and Shadowing Test Suite
# ============================================================================

describe("Scope and Shadowing", fn() {
  test("inner scope shadows outer variable", fn() {
    let x = 1
    {
      let x = 2
      assert_eq(x, 2)
    }
    assert_eq(x, 1)
  })

  test("inner scope can access outer variable", fn() {
    let outer = 10
    let result = 0
    {
      result = outer + 5
    }
    assert_eq(result, 15)
  })

  test("function has its own scope", fn() {
    let x = 100
    def setX
      let x = 50
      return x
    end
    assert_eq(setX(), 50)
    assert_eq(x, 100)
  })

  test("for loop scope", fn() {
    let sum = 0
    for i in range(1, 4)
      sum = sum + i
    end
    assert_eq(sum, 6)
  })

  test("while loop scope", fn() {
    let count = 0
    while (count < 3)
      let temp = count
      count = count + 1
    end
  })

  test("if block scope", fn() {
    let result = ""
    if (true)
      let message = "inside if"
      result = message
    end
    assert_eq(result, "inside if")
  })

  test("try/catch scope", fn() {
    let caught = false
    try
      let x = 1
    catch e
      caught = true
    end
    assert(caught == false)
  })

  test("lambda captures outer scope", fn() {
    let outer = 42
    let get_outer = fn() { outer }
    assert_eq(get_outer(), 42)
  })

  test("nested blocks", fn() {
    let a = 1
    {
      let b = 2
      {
        let c = 3
        assert_eq(a, 1)
        assert_eq(b, 2)
        assert_eq(c, 3)
      }
      assert_eq(a, 1)
      assert_eq(b, 2)
    }
    assert_eq(a, 1)
  })
})
