# ============================================================================
# Interfaces Test Suite
# ============================================================================

describe("Interface Declaration", fn() {
  test("interface with method signature", fn() {
    interface Greetable {
      fn greet()
    }

    class Person implements Greetable
      def greet
        return "Hello"
      end
    end

    let p = new Person()
    assert_eq(p.greet(), "Hello")
  })

  test("interface with multiple methods", fn() {
    interface CRUD {
      fn create()
      fn read()
      fn update(data)
      fn delete()
    }

    class SimpleStore implements CRUD
      data: String = ""

      def create
        this.data = "created"
      end

      def read
        return this.data
      end

      def update(data)
        this.data = data
      end

      def delete
        this.data = ""
      end
    end

    let store = new SimpleStore()
    store.create()
    assert_eq(store.read(), "created")
    store.update("updated")
    assert_eq(store.read(), "updated")
    store.delete()
    assert_eq(store.read(), "")
  })

  test("interface with typed parameters", fn() {
    interface Calculator {
      fn add(a, b)
      fn multiply(a, b)
    }

    class SimpleCalc implements Calculator
      def add(a, b)
        return a + b
      end

      def multiply(a, b)
        return a * b
      end
    end

    let calc = new SimpleCalc()
    assert_eq(calc.add(3, 4), 7)
    assert_eq(calc.multiply(2, 5), 10)
  })

  test("empty interface", fn() {
    interface Empty {
    }

    class EmptyImpl implements Empty
    end

    let e = new EmptyImpl()
    assert_not_null(e)
  })
})

describe("Interface Implementation", fn() {
  test("class implements single interface", fn() {
    interface Printable {
      fn print()
    }

    class Document implements Printable
      content: String

      new(content: String)
        this.content = content
      end

      def print
        return this.content
      end
    end

    let doc = new Document("Hello World")
    assert_eq(doc.print(), "Hello World")
  })

  test("class implements multiple interfaces", fn() {
    interface Loggable {
      fn log()
    }

    interface Serializable {
      fn serialize()
    }

    class Data implements Loggable, Serializable
      value: Int

      new(value: Int)
        this.value = value
      end

      def log
        return "Data: " + str(this.value)
      end

      def serialize
        return "{\"value\":" + str(this.value) + "}"
      end
    end

    let d = new Data(42)
    assert_eq(d.log(), "Data: 42")
    assert_eq(d.serialize(), "{\"value\":42}")
  })

  test("class with interface and inheritance", fn() {
    class Base
      base_value: Int = 10
    end

    interface Incrementable {
      fn increment()
    }

    class Derived < Base implements Incrementable
      def increment
        this.base_value = this.base_value + 1
        return this.base_value
      end
    end

    let d = new Derived()
    assert_eq(d.base_value, 10)
    assert_eq(d.increment(), 11)
    assert_eq(d.increment(), 12)
  })

  test("implementation order does not matter", fn() {
    interface A {
      fn method_a()
    }

    interface B {
      fn method_b()
    }

    class AB implements A, B
      def method_a
        return "A"
      end

      def method_b
        return "B"
      end
    end

    let ab = new AB()
    assert_eq(ab.method_a(), "A")
    assert_eq(ab.method_b(), "B")
  })

  test("interface with return type", fn() {
    interface Summable {
      fn sum(a: Int, b: Int) -> Int
    }

    class Adder implements Summable
      def sum(a: Int, b: Int) -> Int
        return a + b
      end
    end

    let adder = new Adder()
    assert_eq(adder.sum(3, 4), 7)
    assert_eq(adder.sum(10, 20), 30)
  })

  test("interface with typed parameters and return type", fn() {
    interface Formatter {
      fn format(name: String, age: Int) -> String
    }

    class PersonFormatter implements Formatter
      def format(name: String, age: Int) -> String
        return name + " is " + str(age) + " years old"
      end
    end

    let pf = new PersonFormatter()
    assert_eq(pf.format("Alice", 30), "Alice is 30 years old")
    assert_eq(pf.format("Bob", 25), "Bob is 25 years old")
  })

  test("interface in array type annotation", fn() {
    interface Drawable {
      fn draw() -> String
    }

    class Circle implements Drawable
      def draw -> String
        return "Circle"
      end
    end

    class Square implements Drawable
      def draw -> String
        return "Square"
      end
    end

    let shapes = [new Circle(), new Square()]
    assert_eq(len(shapes), 2)
    assert_eq(shapes[0].draw(), "Circle")
    assert_eq(shapes[1].draw(), "Square")
  })
})

describe("Interface `~` Shorthand", fn() {
  test("~ replaces `implements` for a single interface", fn() {
    interface Greetable {
      fn greet()
    }

    class Person implements Greetable
      def greet
        return "Hello"
      end
    end

    let p = new Person()
    assert_eq(p.greet(), "Hello")
  })

  test("~ accepts a comma-separated list", fn() {
    interface Drawable {
      fn draw() -> String
    }
    interface Resizable {
      fn resize(factor: Float) -> Float
    }

    class Box implements Drawable, Resizable
      size: Float = 1.0

      def draw -> String
        return "Box"
      end

      def resize(factor: Float) -> Float
        this.size = this.size * factor
        return this.size
      end
    end

    let b = new Box()
    assert_eq(b.draw(), "Box")
    assert_eq(b.resize(3.0), 3.0)
  })

  test("extends combined with ~", fn() {
    interface Greetable {
      fn greet()
    }

    class Animal
      def breathe -> String
        return "breathing"
      end
    end

    class Dog < Animal implements Greetable
      def greet
        return "woof"
      end
    end

    let d = new Dog()
    assert_eq(d.breathe(), "breathing")
    assert_eq(d.greet(), "woof")
  })

  test("< combined with ~", fn() {
    interface Greetable {
      fn greet()
    }

    class Animal
      def breathe -> String
        return "breathing"
      end
    end

    class Cat < Animal implements Greetable
      def greet
        return "meow"
      end
    end

    let c = new Cat()
    assert_eq(c.breathe(), "breathing")
    assert_eq(c.greet(), "meow")
  })
})
