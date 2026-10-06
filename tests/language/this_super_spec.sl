# ============================================================================
# This and Super Keyword Test Suite
# ============================================================================

describe("This Keyword", fn() {
  test("this in instance method", fn() {
    class Counter
      value: Int = 0

      def get_value
        return this.value
      end

      def increment
        this.value = this.value + 1
      end
    end

    let c = new Counter()
    assert_eq(c.get_value(), 0)
    c.increment()
    assert_eq(c.get_value(), 1)
  })

  test("this in constructor", fn() {
    class Point
      x: Int
      y: Int

      new(x: Int, y: Int)
        this.x = x
        this.y = y
      end
    end

    let p = new Point(3, 4)
    assert_eq(p.x, 3)
    assert_eq(p.y, 4)
  })

  test("this in nested function", fn() {
    class Container
      value: Int = 42

      def get_value
        let helper = fn() { this.value }
        return helper()
      end
    end

    let c = new Container()
    assert_eq(c.get_value(), 42)
  })

  test("this chaining", fn() {
    class Chainer
      value: Int = 0

      def add(n: Int)
        this.value = this.value + n
        return this
      end

      def multiply(n: Int)
        this.value = this.value * n
        return this
      end
    end

    let c = new Chainer()
    c.add(5).multiply(2)
    assert_eq(c.value, 10)
  })

  test("this with different method calls", fn() {
    class Calculator
      value: Int = 0

      def add(n: Int)
        this.value = this.value + n
      end

      def get_value
        return this.value
      end
    end

    let calc = new Calculator()
    calc.add(10)
    calc.add(5)
    assert_eq(calc.get_value(), 15)
  })
})

describe("Super Keyword", fn() {
  test("super in method call", fn() {
    class Animal
      def speak
        return "sound"
      end
    end

    class Dog < Animal
      def speak
        return super.speak() + " bark"
      end
    end

    let d = new Dog()
    assert_eq(d.speak(), "sound bark")
  })

  test("super in constructor", fn() {
    class Base
      value: Int

      new(v)
        this.value = v
      end
    end

    class Derived < Base
      extra: Int

      new(a, b)
        this.value = a
        this.extra = b
      end
    end

    let d = new Derived(10, 20)
    assert_eq(d.value, 10)
    assert_eq(d.extra, 20)
  })

  test("super accessing parent method", fn() {
    class Adder
      def add(a: Int, b: Int) -> Int
        return a + b
      end
    end

    class Multiplier < Adder
      def multiply(a: Int, b: Int) -> Int
        return super.add(a, b) * 2
      end
    end

    let m = new Multiplier()
    assert_eq(m.multiply(3, 4), 14)
  })

  test("super in multiple levels of inheritance", fn() {
    class Level1
      def get_name
        return "Level1"
      end
    end

    class Level2 < Level1
      def get_name
        return super.get_name() + " -> Level2"
      end
    end

    class Level3 < Level2
      def get_name
        return super.get_name() + " -> Level3"
      end
    end

    let l3 = new Level3()
    assert_eq(l3.get_name(), "Level1 -> Level2 -> Level3")
  })

  test("super with field access", fn() {
    class Base
      value: Int = 100
    end

    class Derived < Base
      def get_base_value
        return super.value
      end
    end

    let d = new Derived()
    assert_eq(d.get_base_value(), 100)
  })
})

describe("This and Super Combined", fn() {
  test("this and super in same class", fn() {
    class Parent
      def greet
        return "Hello"
      end
    end

    class Child < Parent
      def greet
        return super.greet() + ", Child!"
      end

      def greet_verbose
        return this.greet()
      end
    end

    let c = new Child()
    assert_eq(c.greet_verbose(), "Hello, Child!")
  })
})
