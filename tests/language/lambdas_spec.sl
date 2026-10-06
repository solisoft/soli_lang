# ============================================================================
# Anonymous Functions (Lambdas) Test Suite
# ============================================================================

describe("Anonymous Functions", fn() {
  test("fn() syntax", fn() {
    let add = fn(a, b) { a + b }
    assert_eq(add(2, 3), 5)
  })

  test("lambda with pipe syntax", fn() {
    let double = fn(x) { x * 2 }
    assert_eq(double(5), 10)
  })

  test("lambda with multiple parameters", fn() {
    let sum = fn(a, b, c) { a + b + c }
    assert_eq(sum(1, 2, 3), 6)
  })

  test("lambda as callback", fn() {
    def apply(f, x)
      return f(x)
    end
    let result = apply(fn(x) { x * x }, 4)
    assert_eq(result, 16)
  })

  test("immediately invoked lambda", fn() {
    let result = (fn(x) { x + 1 })(5)
    assert_eq(result, 6)
  })
})

describe("Lambdas and this Keyword", fn() {
  test("arrow lambda captures this from enclosing scope", fn() {
    class Outer
      value: Int = 100

      def get_closure
        return fn(x) { this.value + x }
      end
    end
    let o = new Outer()
    let closure = o.get_closure()
    assert_eq(closure(5), 105)
  })

  test("fn() {} lambda captures this", fn() {
    class Container
      factor: Int = 10

      def multiplier
        return fn(x) { x * this.factor }
      end
    end
    let c = new Container()
    let closure = c.multiplier()
    assert_eq(closure(5), 50)
  })

  test("nested lambdas with this", fn() {
    class Outer
      base: Int = 10

      def create_nested
        return fn(y) {
          return fn(z) { this.base + y + z }
        }
      end
    end
    let o = new Outer()
    let inner = o.create_nested()(5)
    assert_eq(inner(3), 18)
  })

  test("this in method callback", fn() {
    class Processor
      multiplier: Int = 2

      def process(items: Array)
        return items.map(fn(x) { x * this.multiplier })
      end
    end
    let p = new Processor()
    let result = p.process([
      1,
      2,
      3
    ])
    assert_eq(result[0], 2)
    assert_eq(result[1], 4)
    assert_eq(result[2], 6)
  })
})

describe("Lambda Edge Cases", fn() {
  test("lambda with no parameters", fn() {
    let getFive = fn() { 5 }
    assert_eq(getFive(), 5)
  })

  test("lambda with typed parameters", fn() {
    let add = fn(a: Int, b: Int) { a + b }
    assert_eq(add(10, 20), 30)
  })

  test("lambda with typed return", fn() {
    let square = fn(x: Int) -> Int { x * x }
    assert_eq(square(7), 49)
  })

  test("lambda stored in array", fn() {
    let ops = [fn(x) { x + 1 }, fn(x) { x * 2 }]
    assert_eq(ops[0](5), 6)
    assert_eq(ops[1](5), 10)
  })

  test("lambda stored in hash", fn() {
    let funcs = {"double": fn(x) { x * 2 }, "triple": fn(x) { x * 3 }}
    assert_eq(funcs["double"](5), 10)
    assert_eq(funcs["triple"](5), 15)
  })
})

describe("Stabby Lambda (->)", fn() {
  test("stabby lambda with pipe params and block body", fn() {
    let double = fn(x) { x * 2 }
    assert_eq(double(5), 10)
  })

  test("stabby lambda with block body", fn() {
    let add = fn(a, b) {
      let sum = a + b
      return sum
    }
    assert_eq(add(2, 3), 5)
  })

  test("stabby lambda with identifier params", fn() {
    let multiply = fn(x, y) { x * y }
    assert_eq(multiply(4, 5), 20)
  })

  test("stabby lambda with parenthesized params", fn() {
    let multiply = fn(x, y) { x * y }
    assert_eq(multiply(4, 5), 20)
  })

  test("stabby lambda with typed params", fn() {
    let add = fn(a: Int, b: Int) { a + b }
    assert_eq(add(10, 20), 30)
  })

  test("stabby lambda with no params using ||", fn() {
    let getTime = fn() { clock() }
    let t = getTime()
    assert(t > 0)
  })

  test("stabby lambda as callback", fn() {
    def apply(f, x)
      return f(x)
    end
    let result = apply(fn(x) { x * x }, 4)
    assert_eq(result, 16)
  })

  test("stabby lambda with string operations", fn() {
    let greet = fn(name) { "Hello, " + name }
    assert_eq(greet("World"), "Hello, World")
  })

  test("chained stabby lambdas", fn() {
    let double = fn(x) { x * 2 }
    let addOne = fn(x) { x + 1 }
    let result = double(addOne(5))
    assert_eq(result, 12)
  })

  test("stabby lambda returns value without return keyword", fn() {
    let square = fn(x) { x * x }
    assert_eq(square(7), 49)
  })
})
