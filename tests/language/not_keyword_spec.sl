# ============================================================================
# Not Keyword and Bang Suffix Test Suite
# ============================================================================

describe("Not Keyword", fn() {
  test("not true equals false", fn() { assert_eq(!true, false) })

  test("not false equals true", fn() { assert_eq(!false, true) })

  test("not null equals true", fn() { assert_eq(!null, true) })

  test("not with variables", fn() {
    let flag = true
    assert_eq(!flag, false)

    let flag2 = false
    assert_eq(!flag2, true)
  })

  test("not with expressions", fn() {
    assert_eq(!(1 == 2), true)
    assert_eq(!(1 != 1), true)
    assert_eq(!(5 > 3), false)
  })

  test("double negation", fn() {
    assert_eq(!!true, true)
    assert_eq(!!false, false)
  })

  test("not with logical operators", fn() {
    assert(!(true && false))
    assert(!(false || false))
    assert(!(true && true) == false)
  })

  test("not with function calls", fn() {
    let non_empty = fn() { [
      1,
      2,
      3
    ] }
    # non_empty().empty?() is false, so not false is true
    assert(!non_empty().empty?())
    # Double negation: not not false = false, which equals empty?() result
    assert(non_empty().empty?() == !!non_empty().empty?())
  })
})

describe("Bang Suffix for Methods", fn() {
  test("method names can end with bang", fn() {
    def insert!
      return "insert called"
    end

    let result = insert!()
    assert_eq(result, "insert called")
  })

  test("bang method with parameters", fn() {
    def delete!(id)
      return "deleted " + id
    end

    let result = delete!("123")
    assert_eq(result, "deleted 123")
  })

  test("bang methods in classes", fn() {
    class FileHelper
      def save!
        return "saved"
      end

      def delete!
        return "deleted"
      end
    end

    let fh = new FileHelper()
    assert_eq(fh.save!(), "saved")
    assert_eq(fh.delete!(), "deleted")
  })

  test("multiple bang methods", fn() {
    def fail!
      return "failed"
    end
    def insert!
      return "inserted"
    end
    def update!
      return "updated"
    end
    def delete!
      return "deleted"
    end

    assert_eq(fail!(), "failed")
    assert_eq(insert!(), "inserted")
    assert_eq(update!(), "updated")
    assert_eq(delete!(), "deleted")
  })

  test("bang suffix with predicate", fn() {
    def validate!
      return false
    end

    assert_eq(validate!(), false)
  })
})

describe("Not Keyword vs Bang Operator Equivalence", fn() {
  test("not and ! produce same results", fn() {
    let values = [true, false, null, 1 == 1, 1 != 2]

    for v in values
      assert_eq(!v, !v)
    end
  })

  test("not and ! have same precedence", fn() {
    let x = 5
    # Test with explicit parentheses to ensure consistent behavior
    let result1 = !(x > 3)
    let result2 = !(x > 3)
    assert_eq(result1, result2)

    # Also test with boolean values directly
    assert_eq(!true, !true)
    assert_eq(!false, !false)
  })
})

describe("Edge Cases", fn() {
  test("not with empty string", fn() {
    assert_eq(!"", true)
    assert_eq(!!"", false)
  })

  test("not with zero", fn() {
    assert_eq(!0, true)
    assert_eq(!!0, false)
  })

  test("not with empty array", fn() {
    assert_eq(![], true)
    assert_eq(!![], false)
  })

  test("not with empty hash", fn() {
    assert_eq(!{}, true)
    assert_eq(!!{}, false)
  })

  test("chained bang methods", fn() {
    def step1!
      return 1
    end
    def step2!
      return 2
    end

    let a = step1!()
    let b = step2!()
    assert_eq(a + b, 3)
  })
})
