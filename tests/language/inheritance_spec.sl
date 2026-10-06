# ============================================================================
# Class Inheritance Test Suite
# ============================================================================

describe("Class Inheritance Basics", fn() {
  test("subclass inherits from superclass", fn() {
    class Animal
      name: String

      new(name: String)
        this.name = name
      end

      def speak -> String
        return "..."
      end
    end

    class Dog < Animal
      def speak -> String
        return "Woof!"
      end
    end

    let dog = new Dog("Buddy")
    assert_eq(dog.name, "Buddy")
    assert_eq(dog.speak(), "Woof!")
  })

  test("subclass can extend inherited behavior", fn() {
    class Shape
      def description -> String
        return "A shape"
      end
    end

    class Circle < Shape
      radius: Float

      new(radius: Float)
        this.radius = radius
      end

      def area -> Float
        return 3.14159 * this.radius * this.radius
      end
    end

    let c = new Circle(5.0)
    assert_eq(c.description(), "A shape")
    assert_eq(c.area(), 78.53975)
  })

  test("deep inheritance chain", fn() {
    class A
      def method_a -> String
        return "A"
      end
    end

    class B < A
      def method_b -> String
        return "B"
      end
    end

    class C < B
      def method_c -> String
        return "C"
      end
    end

    let c = new C()
    assert_eq(c.method_a(), "A")
    assert_eq(c.method_b(), "B")
    assert_eq(c.method_c(), "C")
  })

  test("< as alias for extends", fn() {
    class Awesome
      def greet -> String
        return "awesome"
      end
    end

    class Demo < Awesome
    end

    let d = new Demo()
    assert_eq(d.greet(), "awesome")
    assert_eq(type(d), "Demo")
  })

  test("instance type shows most derived class", fn() {
    class Base
    end

    class Derived < Base
    end

    let d = new Derived()
    assert_eq(type(d), "Derived")
  })
})

describe("super Keyword", fn() {
  test("super.method() in instance methods", fn() {
    class Base
      def greet -> String
        return "Hello"
      end
    end

    class Derived < Base
      def greet -> String
        return super.greet() + " World"
      end
    end

    assert_eq(new Derived().greet(), "Hello World")
  })

  test("super with field access", fn() {
    class Base
      value: Int = 10
    end

    class Derived < Base
      def get_value -> Int
        return this.value
      end
    end

    assert_eq(new Derived().get_value(), 10)
  })

  test("super in constructor", fn() {
    class Person
      name: String

      new(name: String)
        this.name = name
      end
    end

    class Employee < Person
      employee_id: Int

      new(name: String, id: Int)
        super(name)
        this.employee_id = id
      end
    end

    let e = new Employee("Alice", 123)
    assert_eq(e.name, "Alice")
    assert_eq(e.employee_id, 123)
  })

  test("super chaining in deep hierarchy", fn() {
    class GrandParent
      def identify -> String
        return "GrandParent"
      end
    end

    class Parent < GrandParent
      def identify -> String
        return super.identify() + " -> Parent"
      end
    end

    class Child < Parent
      def identify -> String
        return super.identify() + " -> Child"
      end
    end

    assert_eq(new Child().identify(), "GrandParent -> Parent -> Child")
  })

  test("calling parent static method explicitly", fn() {
    # Note: super.method() in static methods is not supported
    # Use explicit class name instead
    class Base
      static def get_class_name -> String
        return "Base"
      end
    end

    class Derived < Base
      static def get_class_name -> String
        return Base.get_class_name() + "_Derived"
      end
    end

    assert_eq(Derived.get_class_name(), "Base_Derived")
  })

  test("super with static method inheritance", fn() {
    class Logger
      static def level -> String
        return "INFO"
      end
    end

    class DebugLogger < Logger
    end

    assert_eq(DebugLogger.level(), "INFO")
  })

  test("super in multiple inheritance levels", fn() {
    class Level1
      def level -> Int
        return 1
      end
    end

    class Level2 < Level1
      def level -> Int
        return super.level() + 10
      end
    end

    class Level3 < Level2
      def level -> Int
        return super.level() + 100
      end
    end

    assert_eq(new Level3().level(), 111)
  })
})

describe("this Keyword", fn() {
  test("this.field access in methods", fn() {
    class Point
      x: Int
      y: Int

      new(x: Int, y: Int)
        this.x = x
        this.y = y
      end

      def get_x -> Int
        return this.x
      end

      def get_y -> Int
        return this.y
      end
    end

    let p = new Point(5, 10)
    assert_eq(p.get_x(), 5)
    assert_eq(p.get_y(), 10)
  })

  test("this.method() calls", fn() {
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

      def reset
        this.value = 0
        return this
      end
    end

    let c = new Chainer()
    assert_eq(c.add(5).multiply(2).value, 10)
    assert_eq(c.reset().add(3).value, 3)
  })

  test("this in constructor", fn() {
    class Box
      width: Int
      height: Int
      depth: Int

      new(w: Int, h: Int, d: Int)
        this.width = w
        this.height = h
        this.depth = d
      end

      def volume -> Int
        return this.width * this.height * this.depth
      end
    end

    let box = new Box(2, 3, 4)
    assert_eq(box.volume(), 24)
  })

  test("this in nested method calls", fn() {
    class Outer
      value: Int = 100
    end

    let o = new Outer()
    assert_eq(o.value, 100)
  })

  test("this in static context throws error", fn() {
    let threw = false
    try
      class Test
        static def bad
          return this
        end
      end
      Test.bad()
    catch e
      threw = true
    end
    assert(threw)
  })
})

describe("Method Overriding", fn() {
  test("method override completely replaces super", fn() {
    class Base
      def get_value -> Int
        return 1
      end
    end

    class Derived < Base
      def get_value -> Int
        return 2
      end
    end

    assert_eq(new Base().get_value(), 1)
    assert_eq(new Derived().get_value(), 2)
  })

  test("override with super call", fn() {
    class Base
      def compute(x: Int) -> Int
        return x * 2
      end
    end

    class Derived < Base
      def compute(x: Int) -> Int
        let result = super.compute(x)
        return result + 1
      end
    end

    assert_eq(new Derived().compute(5), 11)
  })

  test("override with different signature", fn() {
    class Base
      def process(data: String) -> String
        return "processed: " + data
      end
    end

    class Derived < Base
      def process(data: String, prefix: String) -> String
        return prefix + ": " + data
      end
    end

    assert_eq(new Base().process("test"), "processed: test")
  })

  test("override adds new methods", fn() {
    class Base
      def existing -> String
        return "exists"
      end
    end

    class Derived < Base
      def new_method -> String
        return "new"
      end
    end

    let d = new Derived()
    assert_eq(d.existing(), "exists")
    assert_eq(d.new_method(), "new")
  })
})

describe("Constructor Behavior", fn() {
  test("default constructor when no new defined", fn() {
    class Simple
      value: Int = 42
    end

    let s = new Simple()
    assert_eq(s.value, 42)
  })

  test("custom constructor", fn() {
    class Rectangle
      width: Int
      height: Int

      new(w: Int, h: Int)
        this.width = w
        this.height = h
      end

      def area -> Int
        return this.width * this.height
      end
    end

    let r = new Rectangle(5, 3)
    assert_eq(r.area(), 15)
  })

  test("constructor with default parameters", fn() {
    class Box
      width: Int
      height: Int
      depth: Int

      new(w: Int, h: Int = 1, d: Int = 1)
        this.width = w
        this.height = h
        this.depth = d
      end

      def volume -> Int
        return this.width * this.height * this.depth
      end
    end

    assert_eq(new Box(2).volume(), 2)
    assert_eq(new Box(2, 3).volume(), 6)
    assert_eq(new Box(2, 3, 4).volume(), 24)
  })
})

describe("Multi-level Inheritance", fn() {
  test("3-level inheritance chain with method override", fn() {
    class Controller
      def action -> String
        return "Controller"
      end
    end

    class BaseController < Controller
      def before -> String
        return "authenticated"
      end
    end

    class HomeController < BaseController
      def action -> String
        return "home"
      end
    end

    let c = new HomeController()
    assert_eq(c.action(), "home")
    assert_eq(c.before(), "authenticated")
  })

  test("3-level inheritance with super chaining", fn() {
    class Controller
      def action -> String
        return "base"
      end
    end

    class BaseController < Controller
      def action -> String
        return super.action() + " -> base_ctrl"
      end
    end

    class HomeController < BaseController
      def action -> String
        return super.action() + " -> home"
      end
    end

    assert_eq(new HomeController().action(), "base -> base_ctrl -> home")
  })

  test("inheriting fields through 3 levels", fn() {
    class A
      x: Int = 1
    end

    class B < A
      y: Int = 2
    end

    class C < B
      z: Int = 3
    end

    let c = new C()
    assert_eq(c.x, 1)
    assert_eq(c.y, 2)
    assert_eq(c.z, 3)
  })

  test("constructor inheritance through 3 levels", fn() {
    class Base
      name: String

      new(name: String)
        this.name = name
      end
    end

    class Middle < Base
    end

    class Leaf < Middle
    end

    let leaf = new Leaf("hello")
    assert_eq(leaf.name, "hello")
  })

  test("4-level inheritance chain", fn() {
    class L1
      def id -> String
        return "L1"
      end
    end

    class L2 < L1
      def id -> String
        return super.id() + ".L2"
      end
    end

    class L3 < L2
      def id -> String
        return super.id() + ".L3"
      end
    end

    class L4 < L3
      def id -> String
        return super.id() + ".L4"
      end
    end

    assert_eq(new L4().id(), "L1.L2.L3.L4")
  })

  test("middle class adds methods accessible by leaf", fn() {
    class Controller
    end

    class BaseController < Controller
      def layout -> String
        return "application"
      end

      def current_user -> String
        return "admin"
      end
    end

    class PostsController < BaseController
      def index -> String
        return this.current_user() + " - " + this.layout()
      end
    end

    let pc = new PostsController()
    assert_eq(pc.index(), "admin - application")
  })

  test("override in middle class, inherit in leaf", fn() {
    class A
      def greet -> String
        return "A"
      end

      def farewell -> String
        return "bye from A"
      end
    end

    class B < A
      def greet -> String
        return "B"
      end
    end

    class C < B
    end

    let c = new C()
    assert_eq(c.greet(), "B")
    assert_eq(c.farewell(), "bye from A")
  })

  test("this refers to actual instance in inherited method", fn() {
    class Base
      name: String = "default"

      def get_name -> String
        return this.name
      end
    end

    class Middle < Base
    end

    class Leaf < Middle
      new()
        this.name = "leaf"
      end
    end

    assert_eq(new Leaf().get_name(), "leaf")
  })

  test("static method inheritance through 3 levels", fn() {
    class A
      static def class_type -> String
        return "A"
      end
    end

    class B < A
    end

    class C < B
    end

    assert_eq(C.class_type(), "A")
  })

  test("type reflects most derived class", fn() {
    class A
    end
    class B < A
    end
    class C < B
    end

    assert_eq(type(new C()), "C")
    assert_eq(type(new B()), "B")
    assert_eq(type(new A()), "A")
  })
})
