# ============================================================================
# Edge Cases Test Suite
# ============================================================================

describe("Edge Cases", fn() {
  test("empty braces is empty hash", fn() {
    let result = {}
    # Empty braces {} is now an empty hash, not an empty block
    assert_eq(type(result), "hash")
    assert_eq(len(result), 0)
  })

  test("chained comparisons", fn() {
    let x = 5
    assert(x > 0 && x < 10)
  })

  test("deeply nested expressions", fn() {
    let result = ((1 + 2) * (3 + 4)) + ((5 - 2) * (8 / 4))
    assert_eq(result, 27)
  })

  test("array of functions", fn() {
    let funcs = [fn(x) { x + 1 }, fn(x) { x * 2 }, fn(x) { x - 3 }]
    assert_eq(funcs[0](10), 11)
    assert_eq(funcs[1](10), 20)
    assert_eq(funcs[2](10), 7)
  })

  test("hash of functions", fn() {
    let ops = {"add": fn(a, b) { a + b }, "sub": fn(a, b) { a - b }}
    assert_eq(ops["add"](5, 3), 8)
    assert_eq(ops["sub"](5, 3), 2)
  })

  test("method on literal", fn() {
    assert_eq(len("hello"), 5)
    assert_eq(len([1, 2, 3]), 3)
  })

  test("boolean coercion in conditions", fn() {
    let result = ""
    result = "truthy" if (1)
    assert_eq(result, "truthy")

    result = ""
    result = "truthy" if ("non-empty")
    assert_eq(result, "truthy")
  })

  test("zero and empty string are truthy in conditions", fn() {
    let x = 0
    let s = ""
    let executed = false
    executed = true if (x)
    assert_not(executed)
    executed = false
    executed = true if (s)
    assert_not(executed)
  })

  test("null handling", fn() {
    let n = null
    assert_null(n)
    assert_not_null(42)
  })

  test("negative index handling", fn() {
    let arr = [1, 2, 3]
    assert_eq(arr[-1], 3)
    assert_eq(arr[-2], 2)
    assert_eq(arr[-3], 1)
  })

  test("large numbers", fn() {
    let large = 1000000
    let very_large = large * large
    assert_eq(very_large, 1000000000000)
  })

  test("floating point precision", fn() {
    let a = 0.1 + 0.2
    assert(a > 0.29 && a < 0.31)
  })

  test("unicode in strings", fn() {
    let unicode_str = "Hello 世界 🌍"
    assert_eq(len(unicode_str), 10)
    assert_contains(unicode_str, "世界")
  })

  test("empty array and hash", fn() {
    let empty_arr = []
    let empty_hash = hash()
    assert_eq(len(empty_arr), 0)
    assert_eq(len(empty_hash), 0)
  })

  test("shadowing with same name", fn() {
    let x = 1
    let x = 2
    assert_eq(x, 2)
  })
})
