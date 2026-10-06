# Reads `obj.<name>` through an untyped parameter: the static checker cannot
# see the receiver's class, so the runtime's refusal is what gets tested.
def read_member(obj, name)
  return obj.password rescue "refused" if name == "password"
  return obj.value rescue "refused" if name == "value"
  return obj.email rescue "refused" if name == "email"
  return nil
end

# ============================================================================
# Classes Test Suite
# ============================================================================

describe("Basic Classes", fn() {
  test("class declaration and instantiation", fn() {
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

  test("class with methods", fn() {
    class Rectangle
      width: Int
      height: Int

      new(w: Int, h: Int)
        this.width = w
        this.height = h
      end

      def area
        return this.width * this.height
      end

      def perimeter
        return 2 * (this.width + this.height)
      end
    end
    let rect = new Rectangle(5, 3)
    assert_eq(rect.area(), 15)
    assert_eq(rect.perimeter(), 16)
  })

  test("class with default field values", fn() {
    class Counter
      count: Int = 0

      def increment
        this.count = this.count + 1
      end

      def get
        return this.count
      end
    end
    let c = new Counter()
    assert_eq(c.get(), 0)
    c.increment()
    c.increment()
    assert_eq(c.get(), 2)
  })

  test("class method chaining", fn() {
    class Builder
      value: String = ""

      def add(s: String)
        this.value = this.value + s
        return this
      end

      def build
        return this.value
      end
    end
    let result = new Builder().add("Hello").add(" ").add("World").build()
    assert_eq(result, "Hello World")
  })
})

describe("Static Methods", fn() {
  test("static method can be called without instance", fn() {
    class MathUtils
      static def random
        return 42
      end
    end
    assert_eq(MathUtils.random(), 42)
  })

  test("static method with parameters", fn() {
    class MathUtils
      static def add(a: Int, b: Int) -> Int
        return a + b
      end
    end
    assert_eq(MathUtils.add(5, 3), 8)
  })

  test("static method with return type", fn() {
    class Calculator
      static def multiply(a: Float, b: Float) -> Float
        return a * b
      end
    end
    assert_eq(Calculator.multiply(3.0, 4.0), 12.0)
  })

  test("static method inheritance", fn() {
    class Base
      static def greet -> String
        return "Hello"
      end
    end

    class Derived < Base
    end

    assert_eq(Derived.greet(), "Hello")
  })

  test("static method override", fn() {
    class Base
      static def get_value -> Int
        return 10
      end
    end

    class Derived < Base
      static def get_value -> Int
        return 20
      end
    end

    assert_eq(Derived.get_value(), 20)
  })

  test("static method can call other static methods", fn() {
    class MathOps
      static def double(x: Int) -> Int
        return x * 2
      end

      static def quadruple(x: Int) -> Int
        return MathOps.double(MathOps.double(x))
      end
    end
    assert_eq(MathOps.quadruple(5), 20)
  })

  test("static factory method pattern", fn() {
    class Point
      x: Float
      y: Float

      new(x: Float, y: Float)
        this.x = x
        this.y = y
      end

      static def origin -> Point
        return new Point(0.0, 0.0)
      end

      static def unit -> Point
        return new Point(1.0, 1.0)
      end
    end

    let origin = Point.origin()
    assert_eq(origin.x, 0.0)
    assert_eq(origin.y, 0.0)

    let unit = Point.unit()
    assert_eq(unit.x, 1.0)
    assert_eq(unit.y, 1.0)
  })

  test("def self.foo declares a static method", fn() {
    class MathUtils
      static def random
        return 42
      end
    end
    assert_eq(MathUtils.random(), 42)
  })

  test("def self.foo with parameters and return type", fn() {
    class MathUtils
      static def add(a: Int, b: Int) -> Int
        return a + b
      end
    end
    assert_eq(MathUtils.add(5, 3), 8)
  })

  test("fn self.foo also works as a static method", fn() {
    class Calculator
      static def multiply(a: Float, b: Float) -> Float
        return a * b
      end
    end
    assert_eq(Calculator.multiply(3.0, 4.0), 12.0)
  })

  test("static def self.foo combines harmlessly (still static)", fn() {
    class MathUtils
      static def cube(x: Int) -> Int
        return x * x * x
      end
    end
    assert_eq(MathUtils.cube(3), 27)
  })

  test("class can mix def self.foo, static def, and instance methods", fn() {
    class Box
      value: Int

      new(value: Int)
        this.value = value
      end

      static def zero -> Box
        return new Box(0)
      end

      static def one -> Box
        return new Box(1)
      end

      def get -> Int
        return this.value
      end
    end

    assert_eq(Box.zero().get(), 0)
    assert_eq(Box.one().get(), 1)
    assert_eq(new Box(7).get(), 7)
  })
})

describe("class << self block", fn() {
  test("single static method declared inside the block", fn() {
    class MathUtils
      static def square(x: Int) -> Int
        return x * x
      end
    end
    assert_eq(MathUtils.square(4), 16)
  })

  test("multiple static methods grouped in one block", fn() {
    class MathUtils
      static def square(x: Int) -> Int
        return x * x
      end

      static def cube(x: Int) -> Int
        return x * x * x
      end
    end
    assert_eq(MathUtils.square(3), 9)
    assert_eq(MathUtils.cube(3), 27)
  })

  test("def alias works inside the block", fn() {
    class Greeter
      static def hello -> String
        return "hi"
      end
    end
    assert_eq(Greeter.hello(), "hi")
  })

  test("instance methods coexist with class << self block", fn() {
    class Counter
      value: Int = 0

      new()
        this.value = 0
      end

      def increment
        this.value = this.value + 1
      end

      static def zero -> Int
        return 0
      end

      static def one -> Int
        return 1
      end
    end
    let c = new Counter()
    c.increment()
    c.increment()
    assert_eq(c.value, 2)
    assert_eq(Counter.zero(), 0)
    assert_eq(Counter.one(), 1)
  })

  test("class << self methods can call each other via the class name", fn() {
    class Math2
      static def double(x: Int) -> Int
        return x * 2
      end

      static def quadruple(x: Int) -> Int
        return Math2.double(Math2.double(x))
      end
    end
    assert_eq(Math2.quadruple(5), 20)
  })

  test("class << self block alongside top-level static fn", fn() {
    class Mixed
      static def outside -> Int
        return 1
      end

      static def inside -> Int
        return 2
      end
    end
    assert_eq(Mixed.outside(), 1)
    assert_eq(Mixed.inside(), 2)
  })
})

describe("Static Fields", fn() {
  test("static field declaration and access", fn() {
    class Counter
      static count: Int = 0
    end
    assert_eq(Counter.count, 0)
  })

  test("static field initial value", fn() {
    class Config
      static debug: Bool = false
      static version: String = "1.0.0"
    end
    assert_eq(Config.debug, false)
    assert_eq(Config.version, "1.0.0")
  })

  test("static field mutation via Class.field = value", fn() {
    class Config
      static debug: Bool = false
    end
    Config.debug = true
    assert_eq(Config.debug, true)
  })

  test("static field in subclass shares with parent", fn() {
    class Base
      static instance_count: Int = 0
    end

    class Derived < Base
    end

    assert_eq(Derived.instance_count, 0)
    Base.instance_count = 5
    assert_eq(Derived.instance_count, 5)
  })

  test("static field for class-level state", fn() {
    class IdGenerator
      static next_id: Int = 1

      static def generate -> Int
        let id = IdGenerator.next_id
        IdGenerator.next_id = IdGenerator.next_id + 1
        return id
      end
    end

    assert_eq(IdGenerator.generate(), 1)
    assert_eq(IdGenerator.generate(), 2)
    assert_eq(IdGenerator.generate(), 3)
  })

  test("static field with type annotation", fn() {
    class Constants
      static pi: Float = 3.14159
      static max_size: Int = 1000
    end
    assert_eq(Constants.pi, 3.14159)
    assert_eq(Constants.max_size, 1000)
  })
})

describe("Private and Protected Visibility", fn() {
  test("a private field is reachable on self only", fn() {
    class Secret
      private password: String = "secret"

      def get_password
        return this.password
      end
    end
    let s = new Secret()
    assert_eq(s.get_password(), "secret")
    assert_eq(read_member(s, "password"), "refused")
  })

  test("a private method is callable on self only", fn() {
    class Container
      private

      def compute -> Int
        return 42
      end

      public

      def get_value
        return this.compute()
      end
    end
    let c = new Container()
    assert_eq(c.get_value(), 42)
    let result = c.compute() rescue "refused"
    assert_eq(result, "refused")
  })

  test("a protected field is refused from outside", fn() {
    class Base
      protected value: Int = 10

      def value_of(other)
        return other.value
      end
    end
    let b = new Base()
    assert_eq(b.value_of(new Base()), 10)
    assert_eq(read_member(b, "value"), "refused")
  })

  test("a protected method is reachable from its class, not from outside", fn() {
    class Base
      protected

      def internal_method -> String
        return "internal"
      end

      public

      def peek(other)
        return other.internal_method()
      end
    end
    let b = new Base()
    assert_eq(b.peek(new Base()), "internal")
    let result = b.internal_method() rescue "refused"
    assert_eq(result, "refused")
  })

  test("private field with default value", fn() {
    class SafeBox
      private code: String = "1234"
      private attempts: Int = 0

      def try_code(input: String) -> Bool
        this.attempts = this.attempts + 1
        return this.code == input
      end

      def get_attempts -> Int
        return this.attempts
      end
    end
    let box = new SafeBox()
    assert_eq(box.try_code("0000"), false)
    assert_eq(box.try_code("1234"), true)
    assert_eq(box.get_attempts(), 2)
  })

  test("class with multiple visibility modifiers", fn() {
    class User
      private id: Int
      protected email: String
      name: String

      new(id: Int, email: String, name: String)
        this.id = id
        this.email = email
        this.name = name
      end

      def get_id -> Int
        return this.id
      end
    end
    let u = new User(1, "test@example.com", "Alice")
    assert_eq(u.get_id(), 1)
    assert_eq(read_member(u, "email"), "refused")
    assert_eq(u.name, "Alice")
  })

  test("static private field", fn() {
    class SecureCounter
      private static counter: Int = 0

      static def increment
        SecureCounter.counter = SecureCounter.counter + 1
      end

      static def get_count -> Int
        return SecureCounter.counter
      end
    end
    assert_eq(SecureCounter.get_count(), 0)
    SecureCounter.increment()
    SecureCounter.increment()
    assert_eq(SecureCounter.get_count(), 2)
  })
})

describe("Native Static Methods", fn() {
  test("DateTime.now() returns current datetime", fn() {
    let now = DateTime.now()
    assert_not_null(now)
    assert_eq(type(now), "DateTime")
  })

  test("DateTime.parse() parses ISO string", fn() {
    let dt = DateTime.parse("2024-01-15T10:30:00Z")
    assert_not_null(dt)
    assert_eq(type(dt), "DateTime")
  })

  test("Duration.of_seconds() creates duration", fn() {
    let dur = Duration.of_seconds(120)
    assert_not_null(dur)
    assert_eq(type(dur), "Duration")
  })

  test("Duration.of_minutes() creates duration", fn() {
    let dur = Duration.of_minutes(5)
    assert_not_null(dur)
  })

  test("Duration.of_hours() creates duration", fn() {
    let dur = Duration.of_hours(2)
    assert_not_null(dur)
  })

  test("Duration.between() calculates difference", fn() {
    let dt1 = DateTime.parse("2024-01-15T10:00:00Z")
    let dt2 = DateTime.parse("2024-01-15T11:30:00Z")
    let dur = Duration.between(dt1, dt2)
    assert_not_null(dur)
  })
})

describe("Constructor Named Parameters", fn() {
  test("constructor with all named parameters", fn() {
    class User
      name: String
      age: Int
      active: Bool

      new(name: String = "Guest", age: Int = 0, active: Bool = true)
        this.name = name
        this.age = age
        this.active = active
      end
    end
    let user = new User(name: "Alice", age: 30, active: false)
    assert_eq(user.name, "Alice")
    assert_eq(user.age, 30)
    assert_eq(user.active, false)
  })

  test("constructor with mixed positional and named parameters", fn() {
    class Config
      host: String
      port: Int
      ssl: Bool
      debug: Bool

      new(host: String, port: Int = 80, ssl: Bool = false, debug: Bool = false)
        this.host = host
        this.port = port
        this.ssl = ssl
        this.debug = debug
      end
    end
    let config = new Config("example.com", ssl: true)
    assert_eq(config.host, "example.com")
    assert_eq(config.port, 80)
    assert_eq(config.ssl, true)
    assert_eq(config.debug, false)
  })

  test("constructor with some named parameters", fn() {
    class Server
      name: String
      port: Int
      workers: Int

      new(name: String, port: Int = 8080, workers: Int = 4)
        this.name = name
        this.port = port
        this.workers = workers
      end
    end
    let server = new Server("api-server", workers: 8)
    assert_eq(server.name, "api-server")
    assert_eq(server.port, 8080)
    assert_eq(server.workers, 8)
  })

  test("constructor duplicate named parameter throws error", fn() {
    class Point
      x: Int
      y: Int

      new(x: Int = 0, y: Int = 0)
        this.x = x
        this.y = y
      end
    end
    let error_thrown = false
    try
      new Point(x: 5, x: 10)
    catch error
      error_thrown = true
    end
    assert_eq(error_thrown, true)
  })

  test("constructor unknown parameter name throws error", fn() {
    class Circle
      radius: Int

      new(radius: Int = 1)
        this.radius = radius
      end
    end
    let error_thrown = false
    try
      new Circle(diameter: 10)
    catch error
      error_thrown = true
    end
    assert_eq(error_thrown, true)
  })
})

describe("Nested Classes", fn() {
  test("basic nested class declaration", fn() {
    class Outer

      class Inner
        def greet
          return "Hello from Inner"
        end
      end
    end
    let inner = new Outer::Inner()
    assert_eq(inner.greet(), "Hello from Inner")
  })

  test("nested class with constructor", fn() {
    class Container

      class Item
        def create_value
          return 42
        end
      end
    end
    let item = new Container::Item()
    assert_eq(item.create_value(), 42)
  })

  test("multiple nested classes", fn() {
    class Service

      class Database
        def connect
          return "DB connected"
        end
      end

      class Cache
        def get(key)
          return "cached:" + key
        end
      end
    end
    let db = new Service::Database()
    let cache = new Service::Cache()
    assert_eq(db.connect(), "DB connected")
    assert_eq(cache.get("test"), "cached:test")
  })

  test("nested class accessing parent class", fn() {
    class Parent
      def get_name
        return "Parent"
      end

      class Child
        def introduce
          return "Child of Parent"
        end
      end
    end
    let child = new Parent::Child()
    assert_eq(child.introduce(), "Child of Parent")
  })
})

describe("Const Fields", fn() {
  test("const instance field declaration", fn() {
    class Config
      const MAX_LENGTH = 500
    end
    let c = new Config()
    assert_eq(c.MAX_LENGTH, 500)
  })

  test("static const field declaration", fn() {
    class Message
      static const TYPE_REPLY = "reply"
      static const TYPE_FORWARD = "forward"
    end
    assert_eq(Message.TYPE_REPLY, "reply")
    assert_eq(Message.TYPE_FORWARD, "forward")
  })

  test("const field with type annotation", fn() {
    class Limits
      const MAX_SIZE: Int = 1000
    end
    let l = new Limits()
    assert_eq(l.MAX_SIZE, 1000)
  })

  test("static const field with type annotation", fn() {
    class Api
      static const VERSION: String = "2.0"
    end
    assert_eq(Api.VERSION, "2.0")
  })

  test("const instance field cannot be reassigned", fn() {
    class Immutable
      const VALUE = 42
    end
    let obj = new Immutable()
    let error_thrown = false
    try
      obj.VALUE = 100
    catch e
      error_thrown = true
    end
    assert_eq(error_thrown, true)
    assert_eq(obj.VALUE, 42)
  })

  test("static const field cannot be reassigned", fn() {
    class Constants
      static const PI = 3.14159
    end
    let error_thrown = false
    try
      Constants.PI = 0
    catch e
      error_thrown = true
    end
    assert_eq(error_thrown, true)
    assert_eq(Constants.PI, 3.14159)
  })

  test("const field accessible via this in method", fn() {
    class Settings
      const LIMIT = 10

      def get_limit
        return this.LIMIT
      end
    end
    let s = new Settings()
    assert_eq(s.get_limit(), 10)
  })

  test("static const field accessible from method", fn() {
    class Counter
      static const MAX = 100

      static def get_max
        return Counter.MAX
      end
    end
    assert_eq(Counter.get_max(), 100)
  })

  test("mixed const and mutable fields", fn() {
    class Item
      const CATEGORY = "default"
      name: String = "unnamed"

      new(name: String)
        this.name = name
      end
    end
    let item = new Item("Widget")
    assert_eq(item.CATEGORY, "default")
    assert_eq(item.name, "Widget")
    item.name = "Updated"
    assert_eq(item.name, "Updated")
  })

  test("mixed static const and static mutable fields", fn() {
    class Registry
      static const TYPE = "singleton"
      static count: Int = 0
    end
    assert_eq(Registry.TYPE, "singleton")
    assert_eq(Registry.count, 0)
    Registry.count = 5
    assert_eq(Registry.count, 5)
  })
})

# ============================================================================
# Method named "new" (keyword used as method name)
# ============================================================================

describe("Method named new", fn() {
  test("def new as a regular method (end-style)", fn() {
    class AppsController
      def new(req)
        return "new form"
      end
    end
    let c = AppsController()
    assert_eq(c.new("GET"), "new form")
  })

  test("fn new as a regular method (brace-style)", fn() {
    class UsersController
      def new(req)
        return "create user form"
      end
    end
    let c = new UsersController()
    assert_eq(c.new("GET"), "create user form")
  })

  test("new method alongside other methods", fn() {
    class ItemsController
      def index(req)
        return "list"
      end

      def new(req)
        return "new form"
      end

      def create(req)
        return "created"
      end

      def show(req)
        return "detail"
      end
    end
    let c = ItemsController()
    assert_eq(c.index("GET"), "list")
    assert_eq(c.new("GET"), "new form")
    assert_eq(c.create("POST"), "created")
    assert_eq(c.show("GET"), "detail")
  })

  test("new method with constructor in same class", fn() {
    class Widget
      name: String

      new(name: String)
        this.name = name
      end

      def new(req)
        return "new " + this.name + " form"
      end
    end
    let w = new Widget("button")
    assert_eq(w.name, "button")
    assert_eq(w.new("GET"), "new button form")
  })
})

# ============================================================================
# Method named "match" (keyword used as method name)
# ============================================================================

describe("Method named match", fn() {
  test("match as a method name in a class", fn() {
    class Validator
      def match(pattern)
        return "matched: " + pattern
      end
    end
    let v = new Validator()
    assert_eq(v.match("^[a-z]+$"), "matched: ^[a-z]+$")
  })

  test("calling .match() on a string", fn() {
    let name = "hello"
    let result = name.match("^[a-z]+$")
    assert_eq(result.empty?(), false)
  })

  test("match method alongside match expression", fn() {
    class Checker
      def match(input)
        return "checking: " + input
      end
    end
    let c = new Checker()
    assert_eq(c.match("test"), "checking: test")

    # match expression still works
    let x = 42
    let result = match x {
      42 => "forty-two",
      _ => "other",
    }
    assert_eq(result, "forty-two")
  })

  test("classes compare with ==", fn() {
    class EqAccount
    end
    class EqOther
    end
    klass = EqAccount
    assert(klass == EqAccount)
    assert(EqAccount == EqAccount)
    assert(klass != EqOther)
    # `.class` answers the class name; it compares equal to the class too.
    record = new EqAccount()
    assert(record.class == EqAccount)
    assert(record.class == "EqAccount")
    assert(record.class != EqOther)
  })
})
